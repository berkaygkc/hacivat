//! The window-free IME core (UI-EDIT-12 model; Task 10). The view's `EntityInputHandler`
//! (Task 27) delegates here and hands every `Committed` this returns to
//! `EditorView::apply_committed`.
//!
//! - **Marked text is real buffer text** (UI-EDIT-07 step 4): a composition update replaces
//!   its target with the marked text through an ordinary transaction (`InputTypeCompose`,
//!   `Origin::Ime`), so every offset the platform sees is the buffer's own. The target is the
//!   platform's range when it gives one, else the marked text, else the main selection.
//! - **One composition, one undo group:** the first mark opens a history bracket
//!   (`InputTypeCompose`); the commit, an unmark or an empty mark (a cancelled composition)
//!   ends it. A cancelled composition leaves no group, because its change composes to nothing.
//! - **UTF-16 through the buffer:** platform ranges are UTF-16 code units over the whole text
//!   ([`utf16_to_bytes`]); both ends snap outward, never splitting a surrogate pair or a CRLF,
//!   and past the end they clamp. The selection inside the marked text is relative to its
//!   start (AppKit's `setMarkedText:selectedRange:`).
//! - **The blur rule** (the M0 spike's, kept): an app switch leaves the composition with the
//!   platform, which resumes it on return; a focus move inside the active window commits it
//!   locally ([`ImeCore::blur`]).
//! - **A programmatic edit commits the composition first** (UI-EDIT-12 step 9): every edit
//!   that is not the IME's own goes through [`ImeCore::dispatch`] (undo and redo call
//!   [`ImeCore::unmark`] first), so it never joins the composition's undo group. An edit that
//!   bypassed the core anyway is mapped over at the next call: the marked range follows its
//!   text, and the composition ends when that edit touched it ([`ImeCore::marked`] and
//!   [`ImeCore::marked_utf16`] report the followed range even before that call).
//! - **The caller's clock:** `mark` and `commit` take `now`, the time the transaction is
//!   stamped with (the view passes its executor's clock, so a test on the fake clock drives
//!   the 500 ms typing window). The core never reads the wall clock.
//!
//! ## m3c-t31-multi: several carets (D-072, UI-EDIT-12 step 5c and 6)
//! - **Typing goes to every caret:** a commit outside a composition replaces every range of the
//!   selection with the text, a caret after each, in one transaction (one undo group per
//!   typing run, as for one caret).
//! - **A composition is mirrored to every caret:** the first mark takes every range of the
//!   selection; each update and the commit replace every marked range in one transaction, and
//!   the selection inside the marked text is set at each. The main caret's composition is the
//!   one the platform sees ([`ImeCore::marked`], [`ImeCore::marked_utf16`]); the underline
//!   paints them all ([`ImeCore::marked_all`]).
//! - **The platform's range moves the mirrors with it:** when it gives a range, the mirrors'
//!   targets move by the same distances from their marked text (or their selection) as the
//!   platform's range from the main one; a mirror that would then overlap another target, or
//!   that an edit outside the core deleted, leaves the composition.
//! - Whether to mirror at all is the view's choice (R-M3-64: only when every caret is in plain
//!   text, `multi::carets_in_plain_text`); otherwise it collapses the selection to the main
//!   caret before the first mark.

use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use smallvec::SmallVec;

use crate::buffer::{Bias, TextBuffer};
use crate::change::{ChangeSet, RangeKind};
use crate::changelog::Rebased;
use crate::model::{BracketId, Committed, EditorModel, Veto};
use crate::selection::{SelectionRange, SelectionSet};
use crate::transaction::{Annotations, Origin, Transaction, UserEvent};

/// Byte ranges, one per caret, sorted.
type Targets = SmallVec<[Range<usize>; 1]>;

/// The composition state of one editor.
#[derive(Debug, Clone, Default)]
pub struct ImeCore {
    composition: Option<Composition>,
}

/// An open composition.
#[derive(Debug, Clone)]
struct Composition {
    /// The marked text's bytes at every caret (never empty), sorted.
    marked: Targets,
    /// The main caret's index in `marked`.
    main: usize,
    /// The model version `marked` is for.
    version: u64,
    bracket: BracketId,
}

impl ImeCore {
    /// `replace_and_mark_text_in_range`: replaces the targets (`r16` for the main caret, else
    /// the marked text, else the selection; module docs for the mirrors) with the marked `text`
    /// and selects `sel16` inside each (relative to its start; `None`: a caret after it). The
    /// first mark opens the composition; an empty `text` cancels it. A veto changes nothing: a
    /// first mark then opens no composition. The transaction is stamped `now`.
    pub fn mark(
        &mut self,
        m: &mut EditorModel,
        r16: Option<Range<usize>>,
        text: &str,
        sel16: Option<Range<usize>>,
        now: Instant,
    ) -> Result<Committed, Veto> {
        self.sync(m);
        let (targets, main) = self.targets(m, r16);
        let inner = match sel16 {
            Some(s) => {
                let anchor = byte_in(text, s.start, Bias::Before);
                let head = if s.end <= s.start {
                    anchor
                } else {
                    byte_in(text, s.end, Bias::After)
                };
                (anchor, head)
            }
            None => (text.len(), text.len()),
        };
        let (bracket, opened) = match &self.composition {
            Some(c) => (c.bracket, false),
            None => (m.begin_bracket(UserEvent::InputTypeCompose), true),
        };
        let version = m.version();
        match replace(m, &targets, main, text, inner, compose(now)) {
            Ok((c, placed)) => {
                match landed(m, version, &placed, main, text) {
                    Some((marked, main)) => {
                        self.composition = Some(Composition {
                            marked,
                            main,
                            version: m.version(),
                            bracket,
                        });
                    }
                    // Cancelled (an empty mark), or a filter or an observer moved the text out
                    // of reach: the composition ends with what is there.
                    None => {
                        self.composition = None;
                        m.end_bracket(bracket);
                    }
                }
                Ok(c)
            }
            Err(v) => {
                if opened {
                    m.end_bracket(bracket);
                }
                Err(v)
            }
        }
    }

    /// `replace_text_in_range`: commits `text` over the targets (as [`ImeCore::mark`]: every
    /// marked range, or every range of the selection) with a caret after each. Inside a
    /// composition this ends it (its undo group closes); outside one it is plain typing
    /// (`InputType`), which joins the typing group within the 500 ms window from `now`. A veto
    /// inside a composition still ends it, keeping the marked text: the platform has ended its
    /// side.
    pub fn commit(
        &mut self,
        m: &mut EditorModel,
        r16: Option<Range<usize>>,
        text: &str,
        now: Instant,
    ) -> Result<Committed, Veto> {
        self.sync(m);
        let (targets, main) = self.targets(m, r16);
        let ann = if self.composition.is_some() {
            compose(now)
        } else {
            Annotations {
                user_event: UserEvent::InputType,
                add_to_history: true,
                time: now,
                origin: Origin::User,
            }
        };
        let result = replace(m, &targets, main, text, (text.len(), text.len()), ann);
        self.end(m);
        result.map(|(c, _)| c)
    }

    /// `unmark_text`: ends the composition, keeping its text.
    pub fn unmark(&mut self, m: &mut EditorModel) {
        self.sync(m);
        self.end(m);
    }

    /// The editor lost the focus: `true` when the composition was committed (a focus move
    /// inside the still-active window: the editor commits locally first, then the platform
    /// drops its copy, which the GPUI fork does on a real text-input focus loss), `false` when
    /// there was none or it stays with the platform (an app switch: the platform resumes it on
    /// return, and a local commit would leave its next update landing beside the committed
    /// copy). While it stays, the view keeps the history unsealed, or the composition becomes
    /// two undo groups.
    pub fn blur(&mut self, m: &mut EditorModel, window_active: bool) -> bool {
        self.sync(m);
        if window_active && self.composition.is_some() {
            self.end(m);
            return true;
        }
        false
    }

    /// Dispatches an edit that is not the IME's own (a command, a paste, `set_text`, a click's
    /// selection): an open composition is committed first, so the edit never lands in it or
    /// joins its undo group (UI-EDIT-12 step 9). The view then tells the platform
    /// (`notify_text_input_changed`, R-M3-7).
    pub fn dispatch(&mut self, m: &mut EditorModel, tr: Transaction) -> Result<Committed, Veto> {
        self.sync(m);
        self.end(m);
        m.dispatch(tr)
    }

    // ## m4b-t11-editor (fix round 1, I2; named item)
    /// [`ImeCore::dispatch`] with a text built elsewhere
    /// ([`EditorModel::dispatch_prebuilt`]): a reload from disk swaps it in.
    pub fn dispatch_prebuilt(
        &mut self,
        m: &mut EditorModel,
        tr: Transaction,
        after: crate::TextBuffer,
    ) -> Result<Committed, Veto> {
        self.sync(m);
        self.end(m);
        m.dispatch_prebuilt(tr, after)
    }

    /// The text of the UTF-16 range `r16`, snapped outward and clamped ([`utf16_to_bytes`]);
    /// `adjusted` gets the UTF-16 range actually returned.
    pub fn text_for_range(
        &self,
        b: &TextBuffer,
        r16: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
    ) -> String {
        let r = utf16_to_bytes(b, r16);
        *adjusted = Some(bytes_to_utf16(b, r.clone()));
        b.slice(r).into_owned()
    }

    /// The main caret's marked text as UTF-16 in `m`'s current text (`marked_text_range`, which
    /// the platform polls): [`ImeCore::marked`] converted, so it follows an edit that bypassed
    /// the core too.
    pub fn marked_utf16(&self, m: &EditorModel) -> Option<Range<usize>> {
        self.marked(m).map(|r| bytes_to_utf16(m.buffer(), r))
    }

    /// The main caret's marked text's bytes in `m`'s current text: mapped over any edit since
    /// the last call into the core; `None` when not composing or such an edit took it.
    pub fn marked(&self, m: &EditorModel) -> Option<Range<usize>> {
        let c = self.composition.as_ref()?;
        follow(m, c.version, c.marked[c.main].clone())
    }

    // ## m3c-t31-multi
    /// Every marked range (the main caret's and its mirrors'), sorted and followed like
    /// [`ImeCore::marked`] (for the underline); empty when not composing.
    pub fn marked_all(&self, m: &EditorModel) -> SmallVec<[Range<usize>; 1]> {
        let Some(c) = self.composition.as_ref() else {
            return SmallVec::new();
        };
        if self.marked(m).is_none() {
            return SmallVec::new();
        }
        c.marked
            .iter()
            .filter_map(|r| follow(m, c.version, r.clone()))
            .collect()
    }

    pub fn is_composing(&self) -> bool {
        self.composition.is_some()
    }

    /// The bytes an IME call replaces, one range per caret, and the main caret's index: the
    /// marked text (else the selection), the main one replaced by `r16` when the platform gives
    /// it, the others moved by the same distances (module docs).
    fn targets(&self, m: &EditorModel, r16: Option<Range<usize>>) -> (Targets, usize) {
        let (base, main): (Targets, usize) = match &self.composition {
            Some(c) => (c.marked.clone(), c.main),
            None => {
                let sel = m.selection();
                (
                    sel.ranges().iter().map(SelectionRange::range).collect(),
                    sel.main_index(),
                )
            }
        };
        let Some(r16) = r16 else {
            return (base, main);
        };
        let buf = m.buffer();
        let target = utf16_to_bytes(buf, r16);
        let from = &base[main];
        let ds = target.start as isize - from.start as isize;
        let de = target.end as isize - from.end as isize;
        if (ds, de) == (0, 0) {
            return (base, main);
        }
        let len = buf.len_bytes() as isize;
        let moved = |r: &Range<usize>| {
            let s = buf.snap((r.start as isize + ds).clamp(0, len) as usize, Bias::Before);
            let e = buf.snap((r.end as isize + de).clamp(0, len) as usize, Bias::After);
            (s <= e).then_some(s..e)
        };
        // The main target always stays; a mirror that would overlap a kept target leaves.
        let mut left: Targets = SmallVec::new();
        let mut bound = target.start;
        for r in base[..main].iter().rev() {
            if let Some(t) = moved(r).filter(|t| t.end <= bound) {
                bound = t.start;
                left.push(t);
            }
        }
        left.reverse();
        let kept_main = left.len();
        let mut out = left;
        let mut bound = target.end;
        out.push(target);
        for r in &base[main + 1..] {
            if let Some(t) = moved(r).filter(|t| t.start >= bound) {
                bound = t.end;
                out.push(t);
            }
        }
        (out, kept_main)
    }

    /// Catches up with an edit that bypassed the core (module docs): every marked range is
    /// followed; the composition ends when the main one is gone, a mirror that is gone leaves.
    fn sync(&mut self, m: &mut EditorModel) {
        let Some(c) = &mut self.composition else {
            return;
        };
        if c.version == m.version() {
            return;
        }
        let Some(main) = follow(m, c.version, c.marked[c.main].clone()) else {
            self.end(m);
            return;
        };
        let mut kept: Targets = SmallVec::new();
        let mut at = 0;
        for (i, r) in c.marked.iter().enumerate() {
            if i == c.main {
                at = kept.len();
                kept.push(main.clone());
            } else if let Some(r) = follow(m, c.version, r.clone()) {
                kept.push(r);
            }
        }
        c.marked = kept;
        c.main = at;
        c.version = m.version();
    }

    /// Ends the composition, keeping its text; its undo group closes.
    fn end(&mut self, m: &mut EditorModel) {
        if let Some(c) = self.composition.take() {
            m.end_bracket(c.bracket);
        }
    }
}

/// `r`, marked text of version `version`, in `m`'s current text; `None` when an edit since
/// deleted it (or the log no longer reaches back).
fn follow(m: &EditorModel, version: u64, r: Range<usize>) -> Option<Range<usize>> {
    if version == m.version() {
        return Some(r);
    }
    match m.changelog().rebase(version, r, RangeKind::Content) {
        Rebased::Mapped(r) if !r.is_empty() => Some(r),
        Rebased::Mapped(_) | Rebased::Deleted | Rebased::TooOld => None,
    }
}

/// The annotations of a composition's transactions, stamped `now`.
fn compose(now: Instant) -> Annotations {
    Annotations {
        user_event: UserEvent::InputTypeCompose,
        add_to_history: true,
        time: now,
        origin: Origin::Ime,
    }
}

/// Replaces every target (sorted, never overlapping) with `text` in one history transaction,
/// the range `inner` (anchor, head; relative to the text's start) selected in each, the main
/// one at `main`; also answers where each copy of the text was placed.
fn replace(
    m: &mut EditorModel,
    targets: &[Range<usize>],
    main: usize,
    text: &str,
    inner: (usize, usize),
    ann: Annotations,
) -> Result<(Committed, Targets), Veto> {
    let ins: Arc<str> = Arc::from(text);
    let changes = ChangeSet::from_edits(
        m.buffer().len_bytes(),
        targets.iter().map(|t| (t.clone(), ins.clone())),
    )
    .map_err(|_| Veto("editor-veto-stale-change"))?;
    let mut placed: Targets = SmallVec::with_capacity(targets.len());
    let mut delta: isize = 0;
    for t in targets {
        let start = t.start.saturating_add_signed(delta);
        placed.push(start..start + text.len());
        delta += text.len() as isize - t.len() as isize;
    }
    let selection = if placed.len() == 1 {
        let s = placed[0].start;
        SelectionSet::single(SelectionRange::new(s + inner.0, s + inner.1))
    } else {
        SelectionSet::normalized(
            placed
                .iter()
                .map(|p| SelectionRange::new(p.start + inner.0, p.start + inner.1)),
            main,
        )
    };
    let c = m.dispatch(Transaction {
        changes,
        selection: Some(selection),
        ann,
    })?;
    Ok((c, placed))
}

/// Where the marked `text`, placed at `placed` over version `version`, is now, and the main
/// one's index: as placed, or mapped over what observers committed after it; `None` when it
/// is empty or the main one is not there as given (a filter rewrote it). A mirror not there as
/// given leaves the composition.
fn landed(
    m: &EditorModel,
    version: u64,
    placed: &[Range<usize>],
    main: usize,
    text: &str,
) -> Option<(Targets, usize)> {
    if text.is_empty() {
        return None;
    }
    let b = m.buffer();
    let at = |r: &Range<usize>| -> Option<Range<usize>> {
        let r = if m.version() > version + 1 {
            match m
                .changelog()
                .rebase(version + 1, r.clone(), RangeKind::Content)
            {
                Rebased::Mapped(r) => r,
                Rebased::Deleted | Rebased::TooOld => return None,
            }
        } else {
            r.clone()
        };
        let fits =
            r.end <= b.len_bytes() && b.is_char_boundary(r.start) && b.is_char_boundary(r.end);
        (fits && b.slice(r.clone()) == text).then_some(r)
    };
    at(&placed[main])?;
    let mut out: Targets = SmallVec::new();
    let mut main_at = 0;
    for (i, r) in placed.iter().enumerate() {
        if let Some(r) = at(r) {
            if i == main {
                main_at = out.len();
            }
            out.push(r);
        }
    }
    Some((out, main_at))
}

/// The byte offset `u16` UTF-16 units into `text` (clamped), on `bias`'s side of a surrogate
/// pair it falls inside.
fn byte_in(text: &str, u16: usize, bias: Bias) -> usize {
    let mut units = 0;
    for (i, c) in text.char_indices() {
        if units >= u16 {
            return i;
        }
        units += c.len_utf16();
        if units > u16 {
            return match bias {
                Bias::Before => i,
                Bias::After => i + c.len_utf8(),
            };
        }
    }
    text.len()
}

/// The byte offset of the UTF-16 offset `u` (clamped to the text), moved to `bias`'s side of a
/// surrogate pair or a CRLF it falls inside.
fn byte_at_utf16(b: &TextBuffer, u: usize, bias: Bias) -> usize {
    let u = u.min(b.len_utf16());
    let at = b.utf16_to_byte(u);
    let at = if bias == Bias::After && b.byte_to_utf16(at) < u {
        // Inside a surrogate pair: `at` is its start; its end is the next char boundary.
        b.snap_to_char(at + 1, Bias::After)
    } else {
        at
    };
    b.snap(at, bias)
}

/// The bytes of the UTF-16 range `r16` (UI-EDIT-12 step 4): clamped to the text, the start
/// snapped back and the end forward out of a surrogate pair or a CRLF; an empty (or reversed)
/// range is a caret at its start, snapped back.
pub fn utf16_to_bytes(b: &TextBuffer, r16: Range<usize>) -> Range<usize> {
    let start = byte_at_utf16(b, r16.start, Bias::Before);
    if r16.end <= r16.start {
        return start..start;
    }
    let end = byte_at_utf16(b, r16.end, Bias::After);
    start..end.max(start)
}

/// The UTF-16 range of the bytes `r` (char boundaries).
pub fn bytes_to_utf16(b: &TextBuffer, r: Range<usize>) -> Range<usize> {
    b.byte_to_utf16(r.start)..b.byte_to_utf16(r.end)
}
