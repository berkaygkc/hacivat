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
//!   text, and the composition ends when that edit touched it.
//!
//! Single cursor in M3a: the marked text replaces the main range only; mirroring a
//! composition to every cursor (D-072) is M3b's.

use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use crate::buffer::{Bias, TextBuffer};
use crate::change::{ChangeSet, RangeKind};
use crate::changelog::Rebased;
use crate::model::{BracketId, Committed, EditorModel, Veto};
use crate::selection::{SelectionRange, SelectionSet};
use crate::transaction::{Annotations, Origin, Transaction, UserEvent};

/// The composition state of one editor.
#[derive(Debug, Clone, Default)]
pub struct ImeCore {
    composition: Option<Composition>,
}

/// An open composition.
#[derive(Debug, Clone)]
struct Composition {
    /// The marked text's bytes (never empty).
    marked: Range<usize>,
    /// The model version `marked` is for.
    version: u64,
    bracket: BracketId,
}

impl ImeCore {
    /// `replace_and_mark_text_in_range`: replaces the target (`r16`, else the marked text, else
    /// the main selection) with the marked `text` and selects `sel16` inside it (relative to
    /// its start; `None`: a caret after it). The first mark opens the composition; an empty
    /// `text` cancels it. A veto changes nothing: a first mark then opens no composition.
    pub fn mark(
        &mut self,
        m: &mut EditorModel,
        r16: Option<Range<usize>>,
        text: &str,
        sel16: Option<Range<usize>>,
    ) -> Result<Committed, Veto> {
        self.sync(m);
        let target = self.target(m, r16);
        let start = target.start;
        let end = start + text.len();
        let selection = match sel16 {
            Some(s) => {
                let anchor = start + byte_in(text, s.start, Bias::Before);
                let head = if s.end <= s.start {
                    anchor
                } else {
                    start + byte_in(text, s.end, Bias::After)
                };
                SelectionRange::new(anchor, head)
            }
            None => SelectionRange::caret(end),
        };
        let (bracket, opened) = match &self.composition {
            Some(c) => (c.bracket, false),
            None => (m.begin_bracket(UserEvent::InputTypeCompose), true),
        };
        let version = m.version();
        match replace(
            m,
            target,
            text,
            selection,
            UserEvent::InputTypeCompose,
            Origin::Ime,
        ) {
            Ok(c) => {
                match landed(m, version, start..end, text) {
                    Some(marked) => {
                        self.composition = Some(Composition {
                            marked,
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

    /// `replace_text_in_range`: commits `text` over the target (`r16`, else the marked text,
    /// else the main selection) with a caret after it. Inside a composition this ends it (its
    /// undo group closes); outside one it is plain typing (`InputType`), which joins the
    /// typing group. A veto inside a composition still ends it, keeping the marked text: the
    /// platform has ended its side.
    pub fn commit(
        &mut self,
        m: &mut EditorModel,
        r16: Option<Range<usize>>,
        text: &str,
    ) -> Result<Committed, Veto> {
        self.sync(m);
        let target = self.target(m, r16);
        let caret = SelectionRange::caret(target.start + text.len());
        let result = if self.composition.is_some() {
            replace(
                m,
                target,
                text,
                caret,
                UserEvent::InputTypeCompose,
                Origin::Ime,
            )
        } else {
            replace(m, target, text, caret, UserEvent::InputType, Origin::User)
        };
        self.end(m);
        result
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

    /// The marked text as UTF-16, while composing (as of the last call into the core, clamped
    /// to `b`).
    pub fn marked_utf16(&self, b: &TextBuffer) -> Option<Range<usize>> {
        let c = self.composition.as_ref()?;
        let len = b.len_bytes();
        let start = b.snap(c.marked.start.min(len), Bias::Before);
        let end = b.snap(c.marked.end.min(len), Bias::After).max(start);
        Some(bytes_to_utf16(b, start..end))
    }

    /// The marked text's bytes in `m`'s current text (for the underline): mapped over any edit
    /// since the last call into the core; `None` when not composing or such an edit took it.
    pub fn marked(&self, m: &EditorModel) -> Option<Range<usize>> {
        let c = self.composition.as_ref()?;
        if c.version == m.version() {
            return Some(c.marked.clone());
        }
        match m
            .changelog()
            .rebase(c.version, c.marked.clone(), RangeKind::Content)
        {
            Rebased::Mapped(r) => Some(r),
            Rebased::Deleted | Rebased::TooOld => None,
        }
    }

    pub fn is_composing(&self) -> bool {
        self.composition.is_some()
    }

    /// The bytes an IME call replaces.
    fn target(&self, m: &EditorModel, r16: Option<Range<usize>>) -> Range<usize> {
        match (r16, &self.composition) {
            (Some(r16), _) => utf16_to_bytes(m.buffer(), r16),
            (None, Some(c)) => c.marked.clone(),
            (None, None) => m.selection().main().range(),
        }
    }

    /// Catches up with an edit that bypassed the core (module docs).
    fn sync(&mut self, m: &mut EditorModel) {
        let Some(c) = &mut self.composition else {
            return;
        };
        if c.version == m.version() {
            return;
        }
        match m
            .changelog()
            .rebase(c.version, c.marked.clone(), RangeKind::Content)
        {
            Rebased::Mapped(r) if !r.is_empty() => {
                c.marked = r;
                c.version = m.version();
            }
            _ => self.end(m),
        }
    }

    /// Ends the composition, keeping its text; its undo group closes.
    fn end(&mut self, m: &mut EditorModel) {
        if let Some(c) = self.composition.take() {
            m.end_bracket(c.bracket);
        }
    }
}

/// Replaces `target` with `text` in one history transaction, with `sel` after it.
fn replace(
    m: &mut EditorModel,
    target: Range<usize>,
    text: &str,
    sel: SelectionRange,
    user_event: UserEvent,
    origin: Origin,
) -> Result<Committed, Veto> {
    let changes = ChangeSet::from_edits(m.buffer().len_bytes(), [(target, Arc::from(text))])
        .map_err(|_| Veto("editor-veto-stale-change"))?;
    m.dispatch(Transaction {
        changes,
        selection: Some(SelectionSet::single(sel)),
        ann: Annotations {
            user_event,
            add_to_history: true,
            time: Instant::now(),
            origin,
        },
    })
}

/// Where the marked `text`, inserted at `r` over version `version`, is now: `r`, or `r` mapped
/// over what observers committed after it; `None` when it is empty or not there as given (a
/// filter rewrote it).
fn landed(m: &EditorModel, version: u64, r: Range<usize>, text: &str) -> Option<Range<usize>> {
    if text.is_empty() {
        return None;
    }
    let r = if m.version() > version + 1 {
        match m.changelog().rebase(version + 1, r, RangeKind::Content) {
            Rebased::Mapped(r) => r,
            Rebased::Deleted | Rebased::TooOld => return None,
        }
    } else {
        r
    };
    let b = m.buffer();
    let fits = r.end <= b.len_bytes() && b.is_char_boundary(r.start) && b.is_char_boundary(r.end);
    (fits && b.slice(r.clone()) == text).then_some(r)
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
