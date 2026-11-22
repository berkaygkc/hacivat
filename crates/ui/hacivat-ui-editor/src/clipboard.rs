//! The clipboard (UI-EDIT-14, M3a scope; Task 27): copy and cut (line-wise without a
//! selection), and a paste read asynchronously and applied to the selection current when it
//! lands, as one `InputPaste` transaction.
//!
//! - **Copy** takes the selection; without one (and with `copy_line_without_selection`)
//!   the caret's whole line with its break, a break of the dominant kind added to a last line
//!   that has none. The clipboard string carries the piece metadata of UI-EDIT-14 step 1
//!   (`{"v":1,"src":"hacivat-editor","pieces":[{"len":…,"line_wise":…}]}`, one piece per range),
//!   so a line-wise copy pastes above the caret's line with the caret's column kept. Metadata
//!   that does not describe the text exactly is ignored.
//! - **Cut** copies, then deletes what it copied in one `DeleteCut` transaction; a line-wise
//!   cut of the last line takes the break before it.
//! - **Paste** reads with `read_from_clipboard_async` (never waiting on the UI thread) and
//!   normalises on the background executor: one leading U+FEFF stripped, every CRLF, CR and LF
//!   converted to the document's dominant ending (U+2028 is not a break), NULs kept, where the
//!   element draws each as a visible `NUL` chip, and counted for a warning toast. Only string
//!   entries are text: file paths and images are not inserted. A paste that would take the
//!   document past T1's 20 MB is refused with `EditorEvent::PasteRefused { bytes }` and a toast
//!   (R-M3-49); a paste whose normalised length alone is past it is refused on the background
//!   executor before anything that size is built, and the UI thread never copies the text. A
//!   failed read shows the clipboard-unavailable toast with Try again. ## m4b-t11-editor
//!   (UI-EDIT-14 step 5): when the host takes the question (`EditorConfig::
//!   confirm_large_paste`), a paste that makes the document 20–100 MB asks first
//!   (`EditorEvent::PasteNeedsConfirm`, the notes at the end of this file) and only one past
//!   100 MB is refused.
//! - **Never inside a composition:** paste and cut are refused while composing (no binding
//!   runs then either, F15), and a read that lands while composing waits until the composition
//!   ends (`ime::defer_while_composing`, the corrected R-M3-7 ruling). Copy changes no text and
//!   always runs.
//! - A read-only editor copies, and neither cuts nor pastes.
//!
//! ## m3c-t31-multi: several carets (UI-EDIT-14 steps 1, 2, 4 and 6)
//! - **Copy** takes every non-empty range as a piece, the pieces joined with the dominant line
//!   ending; with no selection, each caret's line once, line-wise. The metadata lists every
//!   piece. **Cut** deletes every copied range in one transaction.
//! - **Paste** cuts the text into its pieces when the metadata describes it exactly, or, when
//!   the platform dropped the metadata, when the text is the last copy this app made (its hash;
//!   the copy record is app-wide, so a copy in one console pastes into another). N pieces into
//!   N carets go one each; otherwise text of exactly N lines (a trailing break aside) goes one
//!   line per caret; anything else goes whole to every caret. One `InputPaste` group.
//! - **The primary selection (Linux):** 100 ms after a selection change, a non-empty main
//!   selection is written to it; a middle click pastes it at the pointer, the selections staying
//!   where they are (`mouse.rs`). Elsewhere neither happens.

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use hacivat_text::{
    ChangeSet, LineEnding, Origin, SelectionRange, SelectionSet, TextBuffer, Transaction, UserEvent,
};
use hacivat_ui::actions::ActionFn;
// ## m4b-t11-editor
use hacivat_ui::button::ButtonKind;
use hacivat_ui::dialog::{DialogButton, DialogSize, DialogSpec};
use hacivat_ui::icon::IconName;
use hacivat_ui::l10n::UiArg;
use hacivat_ui::text::TextFn;
use hacivat_ui::toast::{Toast, ToastAction, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, ClipboardEntry, ClipboardItem, Context, Global, Window};
use twox_hash::XxHash3_128;

use crate::commands::message;
use crate::ime::{self, AppEdit};
// ## m4b-t11-editor
use crate::large_file;
use crate::view::{EditorEvent, EditorView};

/// The module-owned state (P11): the clipboard reads in flight, and the primary selection's
/// pending write (Linux).
#[derive(Debug, Clone, Default)]
pub struct PasteState {
    reads: u32,
    // ## m3c-t31-multi
    /// Bumped by every selection change: a pending primary-selection write runs only if it is
    /// still the newest (the 100 ms debounce).
    primary_generation: u64,
    // ## m4b-t11-editor
    /// The 20–100 MB paste waiting for the host's answer (`EditorView::confirm_paste`).
    pub(crate) pending: Option<PendingPaste>,
}

impl PasteState {
    /// Paste reads asked for whose text has not been applied (or refused) yet.
    pub fn reads_in_flight(&self) -> u32 {
        self.reads
    }
}

// ## m3c-t31-multi
/// One copied piece (UI-EDIT-14 step 1): its bytes in the clipboard text and whether it is a
/// whole line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Piece {
    len: usize,
    line_wise: bool,
}

/// The last copy made in this app (any editor): the clipboard text's hash and its pieces. A
/// platform that drops the metadata (UI-EDIT-14 verification: Windows and Linux clipboards
/// may) still pastes N pieces into N carets while the clipboard holds exactly that text.
/// The hash cannot tell who put the text there: the same bytes copied in another app are taken
/// for this copy (accepted, fix round 1 review M9).
#[derive(Debug, Clone, Default)]
struct CopyRecord {
    hash: u128,
    pieces: Vec<Piece>,
}

impl Global for CopyRecord {}

fn hash_of(text: &str) -> u128 {
    let mut h = XxHash3_128::new();
    h.write(text.as_bytes());
    h.finish_128()
}

/// Normalised pasted text, ready to apply.
// ## m4b-t11-editor: `Clone` (its texts are `Arc`s) for the pending question.
#[derive(Debug, Clone)]
pub(crate) struct Pasted {
    text: Arc<str>,
    nuls: usize,
    /// The pieces, normalised, when the clipboard described them (its metadata, or the copy
    /// record): one per copied range, with whether it is line-wise. Empty for foreign text.
    pieces: Vec<(Arc<str>, bool)>,
    // ## m4b-t11-editor
    /// The host answered "Paste" to the 20–100 MB question: the paste may make the document
    /// up to `TierLimits::paste_max_bytes` (kept while the paste is held for a composition).
    confirmed: bool,
}

/// The piece metadata of the copied pieces (UI-EDIT-14 step 1).
fn metadata(pieces: &[Piece]) -> String {
    let list: Vec<String> = pieces
        .iter()
        .map(|p| format!(r#"{{"len":{},"line_wise":{}}}"#, p.len, p.line_wise))
        .collect();
    format!(
        r#"{{"v":1,"src":"hacivat-editor","pieces":[{}]}}"#,
        list.join(",")
    )
}

/// The pieces of `meta` when it is exactly the format [`metadata`] writes (another version or
/// source is ignored).
fn parse_metadata(meta: &str) -> Option<Vec<Piece>> {
    let list = meta
        .strip_prefix(r#"{"v":1,"src":"hacivat-editor","pieces":["#)?
        .strip_suffix("]}")?;
    list.strip_prefix('{')?
        .strip_suffix('}')?
        .split("},{")
        .map(|p| {
            let rest = p.strip_prefix(r#""len":"#)?;
            let (len, lw) = rest.split_once(r#","line_wise":"#)?;
            let line_wise = match lw {
                "true" => true,
                "false" => false,
                _ => return None,
            };
            Some(Piece {
                len: len.parse().ok()?,
                line_wise,
            })
        })
        .collect()
}

/// `text` cut into `pieces` (a line break between two pieces that are not line-wise, as Copy
/// joins them); `None` when they do not describe the text exactly.
fn split_pieces<'a>(text: &'a str, pieces: &[Piece]) -> Option<Vec<(&'a str, bool)>> {
    let mut out = Vec::with_capacity(pieces.len());
    let mut at = 0usize;
    for (i, p) in pieces.iter().enumerate() {
        let end = at.checked_add(p.len)?;
        if end > text.len() || !text.is_char_boundary(end) {
            return None;
        }
        out.push((&text[at..end], p.line_wise));
        at = end;
        if i + 1 < pieces.len() && !p.line_wise {
            let rest = &text[at..];
            at += if rest.starts_with("\r\n") {
                2
            } else if rest.starts_with(['\n', '\r']) {
                1
            } else {
                return None;
            };
        }
    }
    (at == text.len()).then_some(out)
}

/// What Copy takes: the text, its pieces, and the bytes Cut deletes (sorted, merged). Every
/// non-empty range is a piece (pieces joined with the dominant line ending); with none, each
/// caret's line once, line-wise, with its break (a break of the dominant kind added to a last
/// line that has none, whose cut takes the break before it).
fn copied(v: &EditorView) -> Option<(String, Vec<Piece>, Vec<Range<usize>>)> {
    let buf = v.model.buffer();
    let sel = v.model.selection();
    let nl = buf.dominant_line_ending().as_str();
    let selected: Vec<Range<usize>> = sel
        .ranges()
        .iter()
        .map(|r| r.range())
        .filter(|r| !r.is_empty())
        .collect();
    if !selected.is_empty() {
        let mut text = String::new();
        let mut pieces = Vec::with_capacity(selected.len());
        for (i, r) in selected.iter().enumerate() {
            if i > 0 {
                text.push_str(nl);
            }
            let piece = buf.slice(r.clone());
            pieces.push(Piece {
                len: piece.len(),
                line_wise: false,
            });
            text.push_str(&piece);
        }
        return Some((text, pieces, selected));
    }
    if !v.config.copy_line_without_selection {
        return None;
    }
    let mut lines: Vec<usize> = sel
        .ranges()
        .iter()
        .map(|r| buf.byte_to_line(r.head))
        .collect();
    lines.dedup();
    let mut text = String::new();
    let mut pieces = Vec::with_capacity(lines.len());
    let mut cuts: Vec<Range<usize>> = Vec::with_capacity(lines.len());
    for line in lines {
        let whole = buf.line_range(line, true);
        let content = buf.line_range(line, false);
        let mut piece = buf.slice(whole.clone()).into_owned();
        let cut = if whole.end > content.end {
            whole
        } else {
            // The last line has no break: copy it with one, and cut the break before it.
            piece.push_str(nl);
            let from = if line > 0 {
                buf.line_range(line - 1, false).end
            } else {
                whole.start
            };
            from..whole.end
        };
        pieces.push(Piece {
            len: piece.len(),
            line_wise: true,
        });
        text.push_str(&piece);
        match cuts.last_mut() {
            Some(prev) if cut.start <= prev.end => prev.end = prev.end.max(cut.end),
            _ => cuts.push(cut),
        }
    }
    Some((text, pieces, cuts))
}

fn write(text: String, pieces: Vec<Piece>, cx: &mut App) {
    let meta = metadata(&pieces);
    cx.set_global(CopyRecord {
        hash: hash_of(&text),
        pieces,
    });
    cx.write_to_clipboard(ClipboardItem::new_string_with_metadata(text, meta));
}

pub fn copy(v: &mut EditorView, window: &mut Window, cx: &mut Context<EditorView>) {
    let _ = window;
    if let Some((text, pieces, _)) = copied(v) {
        write(text, pieces, cx);
    }
}

pub fn cut(v: &mut EditorView, window: &mut Window, cx: &mut Context<EditorView>) {
    let _ = window;
    if v.config.read_only || v.ime.is_composing() {
        return;
    }
    let Some((text, pieces, ranges)) = copied(v) else {
        return;
    };
    write(text, pieces, cx);
    let len = v.model.buffer().len_bytes();
    let Ok(changes) = ChangeSet::from_edits(len, ranges.into_iter().map(|r| (r, Arc::from(""))))
    else {
        return;
    };
    let ann = v.annotations(UserEvent::DeleteCut, Origin::User, cx);
    // Every caret inside a cut range maps to where that range was.
    v.commit(
        Transaction {
            changes,
            selection: None,
            ann,
        },
        cx,
    );
}

pub fn paste(v: &mut EditorView, window: &mut Window, cx: &mut Context<EditorView>) {
    let _ = window;
    if v.config.read_only || v.ime.is_composing() {
        return;
    }
    let read = cx.read_from_clipboard_async();
    v.paste.reads += 1;
    cx.spawn(async move |this, cx| {
        let result = read.await.map_err(|_| ());
        let _ = this.update(cx, |v, cx| {
            v.paste.reads = v.paste.reads.saturating_sub(1);
            landed(v, result, cx);
        });
    })
    .detach();
}

/// A clipboard read finished.
fn landed(
    v: &mut EditorView,
    result: Result<Option<ClipboardItem>, ()>,
    cx: &mut Context<EditorView>,
) {
    match result {
        Ok(Some(item)) => paste_item(v, item, cx),
        Ok(None) => {}
        Err(()) => {
            let weak = cx.entity().downgrade();
            let retry: ActionFn = Rc::new(move |window: &mut Window, cx: &mut App| {
                let _ = weak.update(cx, |v, cx| paste(v, window, cx));
            });
            ToastQueue::push(cx, unavailable_toast(retry));
        }
    }
}

/// What a paste's background work hands back.
#[derive(Debug)]
pub(crate) enum Landing {
    Text(Pasted),
    /// The normalised text alone would be this many bytes, past the limit: nothing was built.
    TooBig(u64),
}

/// Pastes `item` (a landed read, or the platform's own paste). Nothing the size of the text
/// is touched on the UI thread (review I2): the item moves to the background executor, which
/// takes its text and its pieces (the metadata, else the copy record when the text is the one
/// copied), measures the normalised length without allocating, refuses a paste that is alone
/// past the limit, and only then normalises.
pub(crate) fn paste_item(v: &mut EditorView, item: ClipboardItem, cx: &mut Context<EditorView>) {
    if v.config.read_only {
        return;
    }
    let ending = v.model.buffer().dominant_line_ending();
    // ## m4b-t11-editor: 100 MB when the host takes the 20–100 MB question.
    let limit = refused_past(v);
    let record = cx.try_global::<CopyRecord>().cloned();
    v.paste.reads += 1;
    let work = cx.background_executor().spawn(async move {
        let (text, meta) = text_of(item)?;
        let pieces = meta
            .as_deref()
            .and_then(parse_metadata)
            .filter(|p| split_pieces(&text, p).is_some())
            .or_else(|| {
                record
                    .filter(|r| r.hash == hash_of(&text))
                    .map(|r| r.pieces)
                    .filter(|p| split_pieces(&text, p).is_some())
            })
            .unwrap_or_default();
        Some(prepare(text, ending, &pieces, limit))
    });
    cx.spawn(async move |this, cx| {
        let landing = work.await;
        let _ = this.update(cx, |v, cx| {
            v.paste.reads = v.paste.reads.saturating_sub(1);
            match landing {
                Some(Landing::Text(p)) => apply(v, p, cx),
                Some(Landing::TooBig(bytes)) => refuse(v, bytes, cx),
                None => {}
            }
        });
    })
    .detach();
}

/// The text of `item`'s string entries (moved out when there is one, joined when there are
/// several) and the metadata of a single one; `None` when it holds no text.
fn text_of(item: ClipboardItem) -> Option<(String, Option<String>)> {
    let mut strings: Vec<_> = item
        .into_entries()
        .filter_map(|e| match e {
            ClipboardEntry::String(s) => Some(s),
            _ => None,
        })
        .collect();
    let (text, meta) = if strings.len() == 1 {
        let s = strings.pop()?;
        (s.text, s.metadata)
    } else {
        (
            strings.into_iter().map(|s| s.text).collect::<String>(),
            None,
        )
    };
    (!text.is_empty()).then_some((text, meta))
}

/// The length [`normalize`] gives `text`, counted without building anything.
fn normalized_len(text: &str, ending: LineEnding) -> usize {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let nl = ending.as_str().len();
    let mut len = 0;
    let mut rest = text;
    while let Some(i) = rest.find(['\r', '\n']) {
        len += i + nl;
        let skip = if rest[i..].starts_with("\r\n") { 2 } else { 1 };
        rest = &rest[i + skip..];
    }
    len + rest.len()
}

/// The background half of a paste: refused by its normalised length before anything is built
/// when it alone is past `limit` (the document after it is at least that long), else
/// normalised, with its `pieces` (already checked against the text) normalised one by one.
fn prepare(text: String, ending: LineEnding, pieces: &[Piece], limit: u64) -> Landing {
    let len = normalized_len(&text, ending) as u64;
    if len > limit {
        return Landing::TooBig(len);
    }
    let (whole, nuls) = normalize(&text, ending);
    let pieces = split_pieces(&text, pieces)
        .unwrap_or_default()
        .into_iter()
        .map(|(piece, line_wise)| (normalize(piece, ending).0, line_wise))
        .collect();
    Landing::Text(Pasted {
        text: whole,
        nuls,
        pieces,
        confirmed: false,
    })
}

#[cfg(test)]
thread_local! {
    /// How many times [`normalize`] ran on this thread (the early-refusal test).
    static NORMALIZED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// UI-EDIT-14 step 3: one leading BOM stripped, endings converted to `ending`, NULs counted.
fn normalize(text: &str, ending: LineEnding) -> (Arc<str>, usize) {
    #[cfg(test)]
    NORMALIZED.with(|n| n.set(n.get() + 1));
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let nl = ending.as_str();
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(['\r', '\n']) {
        out.push_str(&rest[..i]);
        out.push_str(nl);
        let skip = if rest[i..].starts_with("\r\n") { 2 } else { 1 };
        rest = &rest[i + skip..];
    }
    out.push_str(rest);
    let nuls = out.bytes().filter(|&b| b == 0).count();
    (Arc::from(out), nuls)
}

/// A paste past the limit: refused with `PasteRefused { bytes }` and the toast (R-M3-49).
/// ## m4b-t11-editor: the limit the toast names is [`refused_past`]'s.
fn refuse(v: &mut EditorView, bytes: u64, cx: &mut Context<EditorView>) {
    let limit = refused_past(v);
    cx.emit(EditorEvent::PasteRefused { bytes });
    ToastQueue::push(cx, refused_toast(bytes, limit));
}

// ## m4b-t11-editor (named items: the size guard of `apply` and `insert_at`)
/// The largest document a paste may make before it is refused: T1's 20 MB, or 100 MB when
/// the host takes the 20–100 MB question (`EditorConfig::confirm_large_paste`).
fn refused_past(v: &EditorView) -> u64 {
    if v.config.confirm_large_paste {
        v.config.tiers.paste_max_bytes
    } else {
        v.config.tiers.reduced_max_bytes
    }
}

/// The size guard of every paste (UI-EDIT-14 step 5): `true` when `p`, inserting `inserted`
/// bytes and making the document `after` bytes, goes ahead now. Within 20 MB it does; past
/// it, a paste the host confirmed does up to 100 MB; otherwise, when the host takes the
/// question and the document stays within 100 MB, the paste waits for the answer
/// (`PasteNeedsConfirm`; a newer question replaces an older one) and nothing is pasted; past
/// that it is refused. `at`: the insertion point of a middle click's paste, with the version
/// it is for (`None`: the selection current at the answer).
fn size_guard(
    v: &mut EditorView,
    p: &Pasted,
    inserted: usize,
    after: usize,
    at: Option<(u64, usize)>,
    cx: &mut Context<EditorView>,
) -> bool {
    let (bytes, total) = (inserted as u64, after as u64);
    let tiers = v.config.tiers;
    if total <= tiers.reduced_max_bytes || (p.confirmed && total <= tiers.paste_max_bytes) {
        return true;
    }
    if !p.confirmed && v.config.confirm_large_paste && total <= tiers.paste_max_bytes {
        v.paste.pending = Some(PendingPaste {
            pasted: p.clone(),
            at,
            bytes,
            total,
        });
        cx.emit(EditorEvent::PasteNeedsConfirm { bytes, total });
    } else {
        refuse(v, bytes, cx);
    }
    false
}

/// What each caret of `n` gets (UI-EDIT-14 step 4): piece `i` at caret `i` when there are `n`
/// pieces; else, with several carets and text of exactly `n` lines (a trailing break aside),
/// one line each; else the whole text at every caret.
fn distribution(p: &Pasted, n: usize, nl: &str) -> Vec<(Arc<str>, bool)> {
    if p.pieces.len() == n {
        return p.pieces.clone();
    }
    if n > 1 && p.pieces.len() <= 1 {
        let body = p.text.strip_suffix(nl).unwrap_or(&p.text);
        let lines: Vec<&str> = body.split(nl).collect();
        if lines.len() == n {
            return lines.into_iter().map(|l| (Arc::from(l), false)).collect();
        }
    }
    // ## m3c-t31-multi (fix round 1, I3): whole lines stay whole lines, whatever the count.
    let line_wise = !p.pieces.is_empty() && p.pieces.iter().all(|x| x.1);
    vec![(p.text.clone(), line_wise); n]
}

/// The selection now, each caret's edit, and where each caret lands (`Some(i)`: after edit
/// `i`'s text; `None`: its own position moved by the edits at or before it, for a line-wise
/// piece). ## m4b-t11-editor: `apply`'s plan, shared with "Paste as preview".
#[allow(
    clippy::type_complexity,
    reason = "a private plan of three parts, named at its one destructuring call site"
)]
fn edits_for(
    v: &EditorView,
    p: &Pasted,
) -> (
    SelectionSet,
    Vec<(Range<usize>, Arc<str>)>,
    Vec<Option<usize>>,
) {
    let buf = v.model.buffer();
    let sel = v.model.selection().clone();
    let plan = distribution(p, sel.ranges().len(), buf.dominant_line_ending().as_str());
    let mut edits: Vec<(Range<usize>, Arc<str>)> = Vec::with_capacity(plan.len());
    let mut after_edit: Vec<Option<usize>> = Vec::with_capacity(plan.len());
    for (r, (text, line_wise)) in sel.ranges().iter().zip(plan) {
        let range = r.range();
        if line_wise && range.is_empty() {
            let start = buf.line_to_byte(buf.byte_to_line(range.start));
            edits.push((start..start, text));
            after_edit.push(None);
        } else {
            edits.push((range, text));
            after_edit.push(Some(edits.len() - 1));
        }
    }
    (sel, edits, after_edit)
}

/// Applies normalised pasted text to the selection current now (## m3c-t31-multi: one piece,
/// one line or the whole text per caret, [`distribution`]): a line-wise piece above its caret's
/// line when the caret has no selection (its column kept), anything else over the range. Held
/// while a composition is open (or held edits wait before it); refused past the size limit,
/// with the selections it replaces counted out (R-M3-49). One `InputPaste` group.
pub(crate) fn apply(v: &mut EditorView, p: Pasted, cx: &mut Context<EditorView>) {
    if v.config.read_only {
        return;
    }
    if ime::must_hold(v) {
        ime::hold(v, AppEdit::Paste(p));
        return;
    }
    let len = v.model.buffer().len_bytes();
    let (sel, edits, after_edit) = edits_for(v, &p);
    let inserted: usize = edits.iter().map(|(_, t)| t.len()).sum();
    let removed: usize = edits.iter().map(|(r, _)| r.len()).sum();
    // ## m4b-t11-editor: past 20 MB, asked or refused (`size_guard`).
    if !size_guard(v, &p, inserted, len - removed + inserted, None, cx) {
        return;
    }
    // Where each edit's text starts in the new text (in the order the change set applies them).
    let mut order: Vec<usize> = (0..edits.len()).collect();
    order.sort_by_key(|&i| (edits[i].0.start, edits[i].0.end));
    let mut starts = vec![0; edits.len()];
    // ## m3c-t31-multi (fix round 1, M7): each edit's old start with the length change of it
    // and every edit before it, so a caret's shift is one binary search (O(N log N) in all).
    let mut shifts: Vec<(usize, isize)> = Vec::with_capacity(edits.len());
    let mut delta: isize = 0;
    for &i in &order {
        let (r, t) = &edits[i];
        starts[i] = r.start.saturating_add_signed(delta);
        delta += t.len() as isize - r.len() as isize;
        shifts.push((r.start, delta));
    }
    let carets: Vec<SelectionRange> = sel
        .ranges()
        .iter()
        .zip(&after_edit)
        .map(|(r, e)| match e {
            Some(i) => SelectionRange::caret(starts[*i] + edits[*i].1.len()),
            None => {
                let k = shifts.partition_point(|(start, _)| *start <= r.head);
                let shift = k.checked_sub(1).map_or(0, |k| shifts[k].1);
                SelectionRange::caret(r.head.saturating_add_signed(shift))
            }
        })
        .collect();
    let Ok(changes) = ChangeSet::from_edits(len, edits) else {
        return;
    };
    let ann = v.annotations(UserEvent::InputPaste, Origin::User, cx);
    let done = v.commit(
        Transaction {
            changes,
            selection: Some(SelectionSet::normalized(carets, sel.main_index())),
            ann,
        },
        cx,
    );
    if done.is_some() && p.nuls > 0 {
        ToastQueue::push(cx, nul_toast(p.nuls));
    }
}

// ## m3c-t31-multi: the Linux primary selection (UI-EDIT-14 step 6).
/// The text the primary selection takes now: the main selection's, when it is not empty.
#[cfg_attr(
    not(target_os = "linux"),
    allow(dead_code, reason = "the primary selection exists on Linux only")
)]
pub(crate) fn primary_text(v: &EditorView) -> Option<String> {
    let main = v.model.selection().main().range();
    (!main.is_empty()).then(|| v.model.buffer().slice(main).into_owned())
}

/// A selection change: on Linux, 100 ms after the last one, the main selection's text goes to
/// the primary selection (nothing when it is empty). Elsewhere nothing happens.
pub(crate) fn selection_changed(v: &mut EditorView, cx: &mut Context<EditorView>) {
    v.paste.primary_generation = v.paste.primary_generation.wrapping_add(1);
    #[cfg(target_os = "linux")]
    {
        // ## m3c-t31-multi (fix round 1, M4): nothing while composing; the commit writes.
        if v.model.selection().main().is_empty() || v.ime.is_composing() {
            return;
        }
        let generation = v.paste.primary_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(100))
                .await;
            let _ = this.update(cx, |v, cx| {
                if v.paste.primary_generation == generation
                    && !v.ime.is_composing()
                    && let Some(text) = primary_text(v)
                {
                    cx.write_to_primary(ClipboardItem::new_string(text));
                }
            });
        })
        .detach();
    }
    #[cfg(not(target_os = "linux"))]
    let _ = cx;
}

/// A middle click's paste of the primary selection at byte `at` (Linux): `text` goes through
/// the paste's background half (fix round 1, I2: refused past the size limit before anything
/// is built, then normalised, never on the UI thread) and lands at `at`, followed through
/// whatever changed meanwhile.
#[cfg_attr(
    not(target_os = "linux"),
    allow(
        dead_code,
        reason = "the middle click pastes the primary selection on Linux only"
    )
)]
pub(crate) fn paste_text_at(
    v: &mut EditorView,
    at: usize,
    text: String,
    cx: &mut Context<EditorView>,
) {
    if v.config.read_only || text.is_empty() {
        return;
    }
    let ending = v.model.buffer().dominant_line_ending();
    // ## m4b-t11-editor: 100 MB when the host takes the 20–100 MB question.
    let limit = refused_past(v);
    let version = v.model.version();
    v.paste.reads += 1;
    let work = cx
        .background_executor()
        .spawn(async move { prepare(text, ending, &[], limit) });
    cx.spawn(async move |this, cx| {
        let landing = work.await;
        let _ = this.update(cx, |v, cx| {
            v.paste.reads = v.paste.reads.saturating_sub(1);
            match landing {
                Landing::Text(p) => insert_at(v, version, at, p, cx),
                Landing::TooBig(bytes) => refuse(v, bytes, cx),
            }
        });
    })
    .detach();
}

/// Byte `at` of version `version` in the current text; `None` when that text is gone (or the
/// change log no longer reaches back).
fn at_now(v: &EditorView, version: u64, at: usize) -> Option<usize> {
    match v
        .model
        .changelog()
        .rebase(version, at..at, hacivat_text::RangeKind::Content)
    {
        hacivat_text::Rebased::Mapped(r) => Some(r.start),
        hacivat_text::Rebased::Deleted | hacivat_text::Rebased::TooOld => None,
    }
}

/// Inserts prepared text at byte `at` of version `version` (mapped to the current text; dropped
/// when that text is gone) as one `InputPaste` group; the selections stay where they are
/// (mapped). Refused on a read-only editor, while composing, or past the size limit.
fn insert_at(v: &mut EditorView, version: u64, at: usize, p: Pasted, cx: &mut Context<EditorView>) {
    if v.config.read_only || ime::must_hold(v) {
        return;
    }
    let Some(at) = at_now(v, version, at) else {
        return;
    };
    let len = v.model.buffer().len_bytes();
    // ## m4b-t11-editor: past 20 MB, asked or refused (`size_guard`).
    let now = Some((v.model.version(), at));
    if !size_guard(v, &p, p.text.len(), len + p.text.len(), now, cx) {
        return;
    }
    let at = v
        .model
        .buffer()
        .snap(at.min(len), hacivat_text::Bias::Before);
    let Ok(changes) = ChangeSet::from_edits(len, [(at..at, p.text.clone())]) else {
        return;
    };
    let ann = v.annotations(UserEvent::InputPaste, Origin::User, cx);
    let done = v.commit(
        Transaction {
            changes,
            selection: None,
            ann,
        },
        cx,
    );
    if done.is_some() && p.nuls > 0 {
        ToastQueue::push(cx, nul_toast(p.nuls));
    }
}

// ## m3b-t27-history (named exception): the app's programmatic insert (the history dialog's
// Insert, `exec_ui::insert_at_caret`), beside the paste whose rules it shares.
impl EditorView {
    /// Replaces the main selection with `text`, its line endings made the document's dominant
    /// one, as one undo group of its own (a command transaction, `Origin::Program`) through
    /// [`EditorView::commit`] (`ImeCore::dispatch`, then `apply_committed`): the caret lands
    /// after it. Returns the bytes it now covers; `None`, with nothing changed, while an IME
    /// composition is open or held edits wait for it (the caller tries again later, never
    /// queued: the M3a T27 rule), on a read-only editor, or past the document size limit
    /// (R-M3-49).
    pub fn replace_selection(
        &mut self,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        if self.config.read_only || ime::must_hold(self) {
            return None;
        }
        let (text, _) = normalize(text, self.model.buffer().dominant_line_ending());
        let len = self.model.buffer().len_bytes();
        let main = self.model.selection().main().range();
        let after = (len - main.len() + text.len()) as u64;
        if after > self.config.tiers.reduced_max_bytes {
            return None;
        }
        let end = main.start + text.len();
        let changes = ChangeSet::from_edits(len, [(main.clone(), text)]).ok()?;
        let ann = self.annotations(UserEvent::Command("insert_text"), Origin::Program, cx);
        self.commit(
            Transaction {
                changes,
                selection: Some(SelectionSet::cursor(end)),
                ann,
            },
            cx,
        )?;
        Some(main.start..end)
    }
}

/// Whole megabytes, rounded up (a size shown as "{ $n } MB").
fn megabytes(bytes: u64) -> u64 {
    bytes.div_ceil(1 << 20).max(1)
}

fn size_text(cx: &App, bytes: u64) -> UiArg {
    UiArg::Text(message(
        cx,
        "hui-editor-size-mb",
        &[("n", UiArg::Count(megabytes(bytes)))],
    ))
}

/// A wide toast in ink (Hue Is Meaning: errors keep the ink, never a new hue).
fn notice(icon: IconName, title: TextFn, body: TextFn) -> Toast {
    let mut toast = Toast::new(ToastKind::Info, body);
    toast.title = Some(title);
    toast.icon = Some(icon);
    toast.wide = true;
    toast
}

// ## m3a-t35-shots (named exception): the three clipboard notices are public so the app's
// shots render them (NOTES surface 7) without a paste through the machine's clipboard.
/// "Couldn't paste": a paste of `bytes` would take the document past `limit` (R-M3-49).
pub fn refused_toast(bytes: u64, limit: u64) -> Toast {
    notice(
        IconName::CircleAlert,
        Rc::new(|cx: &App| message(cx, "hui-editor-paste-refused-title", &[])),
        Rc::new(move |cx: &App| {
            message(
                cx,
                "hui-editor-paste-refused-body",
                &[
                    ("size", size_text(cx, bytes)),
                    ("limit", size_text(cx, limit)),
                ],
            )
        }),
    )
}

/// The `n` NUL characters a paste inserted, shown as chips.
pub fn nul_toast(n: usize) -> Toast {
    let n = n as u64;
    notice(
        IconName::TriangleAlert,
        Rc::new(|cx: &App| message(cx, "hui-editor-nul-title", &[])),
        Rc::new(move |cx: &App| message(cx, "hui-editor-nul-body", &[("n", UiArg::Count(n))])),
    )
}

/// The read failed: a sticky toast (it carries an action) with Try again.
pub fn unavailable_toast(retry: ActionFn) -> Toast {
    let mut toast = notice(
        IconName::ClipboardX,
        Rc::new(|cx: &App| message(cx, "hui-editor-clipboard-title", &[])),
        Rc::new(|cx: &App| message(cx, "hui-editor-clipboard-body", &[])),
    );
    toast.sticky = true;
    toast.actions = vec![ToastAction {
        label: Rc::new(|cx: &App| message(cx, "hui-editor-try-again", &[])),
        run: retry,
    }];
    toast
}

// ## m4b-t11-editor: the 20–100 MB paste confirmation (UI-EDIT-14 step 5, R-M4b-20;
// design/m4b-scripts surface 9).
//
// A paste that would make the document larger than T1's 20 MB but at most 100 MB, in an editor
// whose host takes the question (`EditorConfig::confirm_large_paste`), is held as the one
// pending paste and the host hears `PasteNeedsConfirm { bytes, total }`. The host shows
// `paste_confirm_dialog` (a 420 px alertdialog: "Paste as preview" Default, Cancel Primary,
// focused and Escape's answer) and answers with `EditorView::confirm_paste`:
// - Cancel: dropped;
// - AsPreview: the whole document as the paste would make it is built on the background
//   executor and handed to the host (`PasteAsPreview`), which shows it as a T2 preview; the
//   editor's own text stays;
// - Paste: pasted as it would have been, editable in reduced mode (up to 100 MB).
// Past 100 MB a paste is refused (`PasteRefused`, the toast names 100 MB). A host that does not
// take the question keeps M3's refusal past 20 MB, so no paste ever waits for an answer that
// never comes.

/// The host's answer to `EditorEvent::PasteNeedsConfirm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PasteAnswer {
    /// Paste it, editable in reduced mode (not offered by surface 9's dialog).
    Paste,
    /// "Paste as preview": the host gets the document as the paste would make it.
    AsPreview,
    /// Paste nothing.
    Cancel,
}

/// The document "Paste as preview" makes (`EditorEvent::PasteAsPreview`), 20–100 MB of text:
/// its `Debug` gives the size only.
#[derive(Clone)]
pub struct PastedDocument(Arc<str>);

impl PastedDocument {
    /// The whole text.
    pub fn text(&self) -> &str {
        &self.0
    }

    /// The text, shared.
    pub fn shared(&self) -> Arc<str> {
        self.0.clone()
    }
}

impl std::fmt::Debug for PastedDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PastedDocument")
            .field("bytes", &self.0.len())
            .finish()
    }
}

/// The paste waiting for the host's answer.
#[derive(Clone)]
pub(crate) struct PendingPaste {
    pasted: Pasted,
    /// A middle click's insertion point and the version it is for; `None`: the selection
    /// current at the answer.
    at: Option<(u64, usize)>,
    bytes: u64,
    total: u64,
}

impl std::fmt::Debug for PendingPaste {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingPaste")
            .field("bytes", &self.bytes)
            .field("total", &self.total)
            .field("at", &self.at)
            .finish_non_exhaustive()
    }
}

/// The confirmation's "Paste as preview" button (`DialogEvent::Pressed`).
pub const PASTE_CONFIRM_PREVIEW: usize = 0;
/// The confirmation's Cancel button: the initial focus and Escape's answer.
pub const PASTE_CONFIRM_CANCEL: usize = 1;

/// The answer a press of the confirmation's `button` gives; anything else is Cancel (fail
/// closed: a dialog closed some other way pastes nothing).
pub fn paste_answer(button: usize) -> PasteAnswer {
    match button {
        PASTE_CONFIRM_PREVIEW => PasteAnswer::AsPreview,
        _ => PasteAnswer::Cancel,
    }
}

/// The question for a paste of `bytes` that would make the document `total` bytes
/// (design/m4b-scripts surface 9): "Paste 42.6 MB?" / "That makes this document 61.0 MB, too
/// large to edit. Paste it as a read-only preview, or cancel." with "Paste as preview"
/// (Default) and Cancel (Primary: the initial focus, and what Escape answers). The host opens
/// it as a modal layer with `Role::AlertDialog` and maps the press with [`paste_answer`].
/// Sizes are the format locale's (`large_file::SizeText`).
pub fn paste_confirm_dialog(bytes: u64, total: u64) -> DialogSpec {
    let button = |label: TextFn, kind| DialogButton {
        label,
        kind,
        accesskey: None,
        english_accesskey: None,
    };
    DialogSpec {
        title: Rc::new(move |cx: &App| {
            message(
                cx,
                "hui-editor-paste-confirm-title",
                &[("size", UiArg::Text(large_file::size_text(bytes, cx)))],
            )
        }),
        body: Rc::new(move |cx: &App| {
            message(
                cx,
                "hui-editor-paste-confirm-body",
                &[("total", UiArg::Text(large_file::size_text(total, cx)))],
            )
        }),
        buttons: vec![
            button(
                Rc::new(|cx: &App| message(cx, "hui-editor-paste-confirm-preview", &[])),
                ButtonKind::Default,
            ),
            button(
                Rc::new(|cx: &App| message(cx, "hui-editor-paste-confirm-cancel", &[])),
                ButtonKind::Primary,
            ),
        ],
        cancel: PASTE_CONFIRM_CANCEL,
        primary: PASTE_CONFIRM_CANCEL,
        size: DialogSize::Standard,
    }
}

/// `buf` with `edits` applied (sorted, not overlapping: checked by the caller), built off the
/// UI thread.
fn document_after(buf: &TextBuffer, mut edits: Vec<(Range<usize>, Arc<str>)>) -> Arc<str> {
    edits.sort_by_key(|(r, _)| (r.start, r.end));
    let len = buf.len_bytes();
    let removed: usize = edits.iter().map(|(r, _)| r.len()).sum();
    let inserted: usize = edits.iter().map(|(_, t)| t.len()).sum();
    let mut out = String::with_capacity(len - removed + inserted);
    let mut at = 0;
    for (r, t) in &edits {
        out.push_str(&buf.slice(at..r.start));
        out.push_str(t);
        at = r.end;
    }
    out.push_str(&buf.slice(at..len));
    Arc::from(out)
}

/// "Paste as preview": the document as `pending` would make it now, handed to the host.
/// Nothing when its place is gone, or when the document would now be past 100 MB (refused).
fn as_preview(v: &mut EditorView, pending: PendingPaste, cx: &mut Context<EditorView>) {
    let p = pending.pasted;
    let edits = match pending.at {
        None => edits_for(v, &p).1,
        Some((version, at)) => match at_now(v, version, at) {
            Some(at) => vec![(at..at, p.text.clone())],
            None => return,
        },
    };
    let buf = v.model.buffer().clone();
    let len = buf.len_bytes();
    let inserted: usize = edits.iter().map(|(_, t)| t.len()).sum();
    let removed: usize = edits.iter().map(|(r, _)| r.len()).sum();
    if (len - removed + inserted) as u64 > v.config.tiers.paste_max_bytes {
        refuse(v, inserted as u64, cx);
        return;
    }
    // The edits must make one change set (sorted, not overlapping), as `apply` requires.
    if ChangeSet::from_edits(len, edits.clone()).is_err() {
        return;
    }
    let work = cx
        .background_executor()
        .spawn(async move { document_after(&buf, edits) });
    cx.spawn(async move |this, cx| {
        let doc = work.await;
        let _ = this.update(cx, |_, cx| {
            cx.emit(EditorEvent::PasteAsPreview {
                text: PastedDocument(doc),
            });
        });
    })
    .detach();
}

impl EditorView {
    /// The host's answer to the pending `PasteNeedsConfirm` (the module's m4b-t11 notes):
    /// Cancel drops it, AsPreview hands the host the document it would make
    /// (`PasteAsPreview`), Paste pastes it (held while a composition is open, like any paste;
    /// the size is checked again, against 100 MB). Nothing happens when no paste waits.
    pub fn confirm_paste(&mut self, answer: PasteAnswer, cx: &mut Context<Self>) {
        let Some(pending) = self.paste.pending.take() else {
            return;
        };
        if self.config.read_only {
            return;
        }
        match answer {
            PasteAnswer::Cancel => {}
            PasteAnswer::AsPreview => as_preview(self, pending, cx),
            PasteAnswer::Paste => {
                let mut p = pending.pasted;
                p.confirmed = true;
                match pending.at {
                    None => apply(self, p, cx),
                    Some((version, at)) => insert_at(self, version, at, p, cx),
                }
            }
        }
    }

    /// The paste waiting for the host's answer, as `(bytes, total)` (`PasteNeedsConfirm`).
    pub fn pending_paste(&self) -> Option<(u64, u64)> {
        self.paste.pending.as_ref().map(|p| (p.bytes, p.total))
    }
}

// ## m3a-t27-ime-clipboard: the parts no window reaches (the read failure, the toasts' text).
#[cfg(test)]
mod t27_tests {
    use std::cell::RefCell;
    use std::sync::Arc as StdArc;

    use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext, div, px};
    use hacivat_text::TextBuffer;
    use hacivat_ui::l10n::{EnglishDefaults, UiText};
    use hacivat_ui_core::gpui_surface::{IntoElement, ParentElement as _, Render, Styled as _};

    use super::*;
    use crate::config::{DocId, EditorConfig};
    use crate::service::FailClosedService;

    #[test]
    fn normalisation_strips_one_leading_bom_and_converts_every_ending() {
        let crlf = TextBuffer::from_str("a\r\nb\r\n").dominant_line_ending();
        let (text, nuls) = normalize("\u{FEFF}\u{FEFF}a\nb\r\nc\rd\u{2028}e\0", crlf);
        assert_eq!(&*text, "\u{FEFF}a\r\nb\r\nc\r\nd\u{2028}e\0");
        assert_eq!(nuls, 1);
        let cr = TextBuffer::from_str("a\rb\rc").dominant_line_ending();
        assert_eq!(&*normalize("x\r\ny\nz", cr).0, "x\ry\rz");
        // A lone CR at the end and a CRLF split across nothing else.
        let lf = TextBuffer::from_str("").dominant_line_ending();
        assert_eq!(&*normalize("\r\r\n\n", lf).0, "\n\n\n");
    }

    /// Review I2: the length scan allocates nothing and agrees with what `normalize` builds.
    #[test]
    fn the_length_scan_agrees_with_normalize() {
        let samples = [
            "",
            "\u{FEFF}",
            "\u{FEFF}\u{FEFF}x",
            "a\nb",
            "a\r\nb",
            "a\rb",
            "\r\r\n\n\r",
            "x\u{2028}y\0z",
            "trailing\r",
            "ş\r\n😀\n",
        ];
        let endings = [
            TextBuffer::from_str("").dominant_line_ending(),
            TextBuffer::from_str("a\r\nb").dominant_line_ending(),
            TextBuffer::from_str("a\rb\rc").dominant_line_ending(),
        ];
        for s in samples {
            for e in endings {
                assert_eq!(normalized_len(s, e), normalize(s, e).0.len(), "{s:?} {e:?}");
            }
        }
    }

    /// Review I2: a paste whose normalised length alone is past the limit is refused before
    /// anything the size of the paste is built; one within it is normalised.
    #[test]
    fn an_oversized_paste_is_refused_before_normalizing() {
        let lf = TextBuffer::from_str("").dominant_line_ending();
        let crlf = TextBuffer::from_str("a\r\nb").dominant_line_ending();
        let before = NORMALIZED.with(|n| n.get());
        // 8 bytes, but 12 with CRLF endings: past a 10-byte limit.
        let landing = prepare("a\nb\nc\nd\n".into(), crlf, &[], 10);
        assert!(matches!(landing, Landing::TooBig(12)), "{landing:?}");
        assert_eq!(NORMALIZED.with(|n| n.get()), before, "normalize never ran");
        let landing = prepare("a\nb".into(), lf, &[], 10);
        assert!(matches!(landing, Landing::Text(ref p) if &*p.text == "a\nb"));
        assert_eq!(NORMALIZED.with(|n| n.get()), before + 1);
    }

    // ## m3c-t31-multi (named exception): metadata of N pieces; it counts only when it
    // describes its own text exactly.
    #[test]
    fn metadata_marks_line_wise_only_for_its_own_text() {
        let one = |len, line_wise| [Piece { len, line_wise }];
        let pieces_of = |text: &str, meta: String| {
            parse_metadata(&meta)
                .and_then(|p| split_pieces(text, &p).map(|s| s.iter().map(|x| x.1).collect()))
        };
        assert_eq!(pieces_of("ab\n", metadata(&one(3, true))), Some(vec![true]));
        assert_eq!(pieces_of("ab\n", metadata(&one(4, true))), None);
        assert_eq!(pieces_of("ab", metadata(&one(2, false))), Some(vec![false]));
        let three = [
            Piece {
                len: 1,
                line_wise: false,
            },
            Piece {
                len: 2,
                line_wise: false,
            },
            Piece {
                len: 0,
                line_wise: false,
            },
        ];
        assert_eq!(parse_metadata(&metadata(&three)), Some(three.to_vec()));
        assert_eq!(
            split_pieces("a\r\nbc\n", &three),
            Some(vec![("a", false), ("bc", false), ("", false)])
        );
        assert_eq!(split_pieces("a bc\n", &three), None, "no break between");
        assert_eq!(split_pieces("a\nbc\nx", &three), None, "text left over");
        assert_eq!(
            parse_metadata(r#"{"v":2,"src":"hacivat-editor","pieces":[]}"#),
            None
        );
        let item = |text: &str, meta: String| {
            ClipboardItem::new_string_with_metadata(text.to_string(), meta)
        };
        assert_eq!(
            text_of(item("ab", metadata(&one(2, false)))),
            Some(("ab".into(), Some(metadata(&one(2, false)))))
        );
        assert_eq!(text_of(ClipboardItem::new_string(String::new())), None);
    }

    /// Several carets: N pieces go one per caret, N lines one per caret, anything else whole.
    #[test]
    fn distribution_follows_pieces_then_lines_then_whole() {
        let pasted = |text: &str, pieces: &[(&str, bool)]| Pasted {
            text: Arc::from(text),
            nuls: 0,
            pieces: pieces.iter().map(|(p, l)| (Arc::from(*p), *l)).collect(),
            confirmed: false,
        };
        let plan = |p: &Pasted, n| {
            distribution(p, n, "\n")
                .into_iter()
                .map(|(t, l)| (t.to_string(), l))
                .collect::<Vec<_>>()
        };
        let p = pasted("a\nb\nc", &[("a", false), ("b", false), ("c", false)]);
        assert_eq!(
            plan(&p, 3),
            [
                ("a".into(), false),
                ("b".into(), false),
                ("c".into(), false)
            ]
        );
        // Three pieces, two carets: whole at each.
        assert_eq!(plan(&p, 2), vec![("a\nb\nc".to_string(), false); 2]);
        // Foreign text of three lines (a trailing break aside), three carets: a line each.
        let p = pasted("x\ny\nz\n", &[]);
        assert_eq!(
            plan(&p, 3),
            [
                ("x".into(), false),
                ("y".into(), false),
                ("z".into(), false)
            ]
        );
        // One caret, or a line count that does not fit: whole.
        assert_eq!(plan(&p, 1), [("x\ny\nz\n".into(), false)]);
        assert_eq!(plan(&p, 2), vec![("x\ny\nz\n".to_string(), false); 2]);
        // One line-wise piece at three carets: whole, line-wise, at each.
        let p = pasted("x\n", &[("x\n", true)]);
        assert_eq!(plan(&p, 3), vec![("x\n".to_string(), true); 3]);
    }

    /// The toasts' texts come from `hui.ftl` (block `m3a-t27-clipboard`) with their sizes and
    /// counts as Fluent arguments.
    #[gpui::test]
    fn the_toasts_read_their_fluent_texts(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(UiText(StdArc::new(EnglishDefaults::new())));
            let r = refused_toast(31_400_000, 20 << 20);
            assert_eq!(r.title.as_ref().unwrap()(cx), "Couldn't paste");
            assert_eq!(
                (r.text)(cx),
                "The pasted text is \u{2068}\u{2068}30\u{2069} MB\u{2069}, which would take \
                 this document past \u{2068}\u{2068}20\u{2069} MB\u{2069}. Paste a smaller part."
            );
            assert_eq!(
                (r.icon, r.wide, r.kind),
                (Some(IconName::CircleAlert), true, ToastKind::Info)
            );
            let one = nul_toast(1);
            assert_eq!(
                (one.text)(cx),
                "\u{2068}1\u{2069} NUL character is marked in the text so you can find it. \
                 Most servers reject it."
            );
            let two = nul_toast(2);
            assert_eq!(
                two.title.as_ref().unwrap()(cx),
                "The pasted text contains NUL characters"
            );
            assert!((two.text)(cx).starts_with("\u{2068}2\u{2069} NUL characters are marked"));
            let u = unavailable_toast(Rc::new(|_: &mut Window, _: &mut App| {}));
            assert!(u.sticky);
            assert_eq!(u.icon, Some(IconName::ClipboardX));
            assert_eq!(u.title.as_ref().unwrap()(cx), "Couldn't read the clipboard");
            assert_eq!((u.actions[0].label)(cx), "Try again");
        });
    }

    struct Host {
        editor: Entity<EditorView>,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(800.0)).h(px(600.0)).child(self.editor.clone())
        }
    }

    fn open(cx: &mut TestAppContext) -> (Entity<EditorView>, &mut VisualTestContext) {
        let slot: Rc<RefCell<Option<Entity<EditorView>>>> = Rc::default();
        let out = slot.clone();
        let (_, vcx) = cx.add_window_view(move |window, cx| {
            let editor = cx.new(|cx| {
                EditorView::new(
                    DocId(14),
                    "SELECT 1",
                    EditorConfig::default(),
                    Rc::new(FailClosedService),
                    Rc::new(|_: &App| "Console 1".into()),
                    window,
                    cx,
                )
            });
            *out.borrow_mut() = Some(editor.clone());
            Host { editor }
        });
        let editor = slot.borrow().clone().unwrap();
        (editor, vcx)
    }

    /// A failed read (the clipboard denied or unavailable) changes nothing and shows the
    /// clipboard-unavailable toast; an empty clipboard shows nothing.
    #[gpui::test]
    fn a_failed_read_shows_the_unavailable_toast(cx: &mut TestAppContext) {
        let (e, vcx) = open(cx);
        e.update(vcx, |v, cx| landed(v, Ok(None), cx));
        assert!(vcx.read(|cx| ToastQueue::visible_ids(cx).is_empty()));
        e.update(vcx, |v, cx| landed(v, Err(()), cx));
        vcx.run_until_parked();
        assert_eq!(vcx.read(|cx| ToastQueue::visible_ids(cx).len()), 1);
        assert_eq!(
            vcx.read(|cx| e.read(cx).snapshot().buffer.to_string()),
            "SELECT 1"
        );
        assert_eq!(vcx.read(|cx| e.read(cx).paste.reads_in_flight()), 0);
    }

    // ## m3c-t31-multi: the primary selection's text and the middle click's insert (the
    // platform calls are Linux-only; these parts run everywhere).
    #[gpui::test]
    fn the_primary_selection_takes_the_main_selection_and_inserts_at_a_point(
        cx: &mut TestAppContext,
    ) {
        let (e, vcx) = open(cx);
        e.update(vcx, |v, cx| {
            assert_eq!(primary_text(v), None, "a caret: nothing to write");
            v.select_range(0..6, cx);
            assert_eq!(primary_text(v).as_deref(), Some("SELECT"));
            // Fix round 1 (I2): the text goes through the background half; byte 7 is followed
            // through an edit made before it lands.
            paste_text_at(v, 7, "a\r\nb ".to_string(), cx);
            v.select_range(0..0, cx);
            v.replace_selection("-- ", cx);
            v.select_range(3..9, cx);
        });
        vcx.run_until_parked();
        e.update(vcx, |v, _| {
            // Inserted at byte 7 (now 10), normalised; the selection is mapped, not moved.
            assert_eq!(v.snapshot().buffer.to_string(), "-- SELECT a\nb 1");
            assert_eq!(v.snapshot().selection.main().range(), 3..9);
            assert_eq!(v.model.undo_kind(), Some(UserEvent::InputPaste));
            assert_eq!(v.paste.reads_in_flight(), 0);
        });
        // Past the limit: refused on the background executor, nothing inserted.
        e.update(vcx, |v, cx| {
            let mut cfg = v.config;
            cfg.tiers.reduced_max_bytes = 16;
            v.set_config(cfg, cx);
            paste_text_at(v, 0, "x".repeat(64), cx);
        });
        vcx.run_until_parked();
        assert_eq!(
            vcx.read(|cx| e.read(cx).snapshot().buffer.to_string()),
            "-- SELECT a\nb 1"
        );
    }

    // ## m4b-t11-editor
    /// A middle click's paste asks like any other (small limits stand in for 20 and 100 MB):
    /// the answer "Paste" inserts it at the clicked point, followed through an edit made while
    /// the question was open; "Paste as preview" hands the host the document with it.
    #[gpui::test]
    fn a_middle_click_paste_asks_and_lands_at_its_point(cx: &mut TestAppContext) {
        let (e, vcx) = open(cx);
        let asked: Rc<RefCell<Vec<(u64, u64)>>> = Rc::default();
        let docs: Rc<RefCell<Vec<String>>> = Rc::default();
        let (a, d) = (asked.clone(), docs.clone());
        vcx.update(|_, cx| {
            cx.subscribe(&e, move |_, ev: &EditorEvent, _| match ev {
                EditorEvent::PasteNeedsConfirm { bytes, total } => {
                    a.borrow_mut().push((*bytes, *total))
                }
                EditorEvent::PasteAsPreview { text } => d.borrow_mut().push(text.text().into()),
                _ => {}
            })
            .detach();
        });
        e.update(vcx, |v, cx| {
            let mut cfg = v.config;
            cfg.tiers.reduced_max_bytes = 16;
            cfg.tiers.paste_max_bytes = 64;
            cfg.confirm_large_paste = true;
            v.set_config(cfg, cx);
            // "SELECT 1" is 8 bytes; 12 more make 20: past 16, within 64.
            paste_text_at(v, 7, "abcdefghijkl".to_string(), cx);
        });
        vcx.run_until_parked();
        assert_eq!(*asked.borrow(), [(12, 20)]);
        e.update(vcx, |v, cx| {
            assert_eq!(v.snapshot().buffer.to_string(), "SELECT 1");
            v.confirm_paste(PasteAnswer::AsPreview, cx);
        });
        vcx.run_until_parked();
        assert_eq!(*docs.borrow(), ["SELECT abcdefghijkl1"]);
        e.update(vcx, |v, cx| {
            paste_text_at(v, 7, "abcdefghijkl".to_string(), cx)
        });
        vcx.run_until_parked();
        e.update(vcx, |v, cx| {
            v.select_range(0..0, cx);
            v.replace_selection("-- ", cx);
            v.confirm_paste(PasteAnswer::Paste, cx);
        });
        vcx.run_until_parked();
        assert_eq!(
            vcx.read(|cx| e.read(cx).snapshot().buffer.to_string()),
            "-- SELECT abcdefghijkl1"
        );
        // Past 64: refused, nothing asks.
        e.update(vcx, |v, cx| paste_text_at(v, 0, "x".repeat(64), cx));
        vcx.run_until_parked();
        assert_eq!(asked.borrow().len(), 2);
        assert_eq!(vcx.read(|cx| e.read(cx).pending_paste()), None);
    }
}
