//! Editing commands (Task 25): newline, tab, outdent, the deletes, undo and redo, and the
//! clipboard commands, which delegate to `clipboard::*` (Task 27).
//!
//! - **Enter** inserts the text's dominant line ending and the current line's indentation (the
//!   spaces and tabs before the caret, at most [`INDENT_SCAN`] bytes of them).
//! - **Tab** inserts spaces up to the next tab stop (`tab_size` columns, 4 by default; a whole
//!   stop at a line's start), or one `\t` with `insert_spaces = false`. A selection over
//!   several lines is indented instead, one unit per line that has text. Shift+Tab takes one
//!   unit of indentation from each line of the selection.
//! - **Deletes** take the selection when there is one, else a grapheme cluster (a CRLF is
//!   one), a word (the language service's tokens), or the line up to the caret.
//! - **Undo and redo** end an open composition first (`ImeCore::unmark`), then hand the
//!   model's `Committed` to `EditorView::apply_committed`.
//!
//! ## m3b-t31-multi: Enter, Tab, Shift+Tab and the deletes act at every range in one
//! transaction (`edit_each`); a multi-line indent or outdent takes each touched line once.
//!
//! Every edit refuses on a read-only editor (`super::change`; undo, redo, cut and paste check
//! by themselves).

use std::ops::Range;
use std::sync::Arc;

use hacivat_text::display::{Fragment, VisualLine};
use hacivat_text::units::{SCAN_LIMIT, grapheme_column};
use hacivat_text::{
    Bias, Dir, SelectionRange, SelectionSet, TextBuffer, TokenOracle, Unit, UserEvent, move_offset,
};
use hacivat_ui_core::gpui_surface::{Context, Window};
use smallvec::smallvec;

use super::{change, selection, words};
use crate::clipboard;
use crate::view::EditorView;

type Cx<'a> = Context<'a, EditorView>;

/// How far Enter looks for the line's indentation, in bytes.
const INDENT_SCAN: usize = 4096;

/// The cells the text `r` takes in the monospace grid, as the display model counts them
/// (`VisualLine::cell_widths`, what `compose_line` places tab stops by): a wide character or an
/// emoji cluster is 2, a zero-width one 0.
fn cells(buf: &TextBuffer, r: Range<usize>) -> usize {
    if r.is_empty() {
        return 0;
    }
    let run = VisualLine {
        first_line: 0,
        last_line: 0,
        fragments: smallvec![Fragment::Text { bytes: r }],
    };
    run.cell_widths(buf).first().map_or(0, |w| *w as usize)
}

/// The display column of `at` on its line, in cells, tabs expanded to `tab` columns (grapheme
/// columns on a line longer than the scan limit). A control character counts no cell here,
/// though the element draws it as a chip wider than one: a tab stop after one is approximate.
fn display_column(buf: &TextBuffer, at: usize, tab: usize) -> usize {
    let start = buf.line_to_byte(buf.byte_to_line(at));
    if at - start > SCAN_LIMIT {
        return grapheme_column(buf, at);
    }
    let text = buf.slice(start..at);
    let (mut col, mut run) = (0, start);
    for (i, _) in text.match_indices('\t') {
        col += cells(buf, run..start + i);
        col += tab - col % tab;
        run = start + i + 1;
    }
    col + cells(buf, run..at)
}

/// The lines a range touches: one that ends at a line's start does not touch that line.
fn lines_of(buf: &TextBuffer, r: Range<usize>) -> Range<usize> {
    let first = buf.byte_to_line(r.start);
    let mut last = buf.byte_to_line(r.end);
    if last > first && buf.line_to_byte(last) == r.end {
        last -= 1;
    }
    first..last + 1
}

// ## m3b-t31-multi
/// The lines every range touches, each once, in order.
fn selected_lines(v: &EditorView) -> Vec<usize> {
    let buf = v.model.buffer();
    let mut out: Vec<usize> = Vec::new();
    for r in v.model.selection().ranges() {
        for l in lines_of(buf, r.range()) {
            if out.last().is_none_or(|&last| last < l) {
                out.push(l);
            }
        }
    }
    out
}

/// What one range's edit is: the bytes it replaces, the text, and where its caret lands inside
/// the text.
type RangeEdit = (Range<usize>, String, usize);

/// One edit per range of the selection, from `f(buffer, range)` (`None`: that range has no
/// edit and its caret stays where the others' edits put it). Edits that would overlap (two
/// deletions reaching into each other) merge into one, their carets at its start. Applied as
/// one transaction of `event` with a caret per range, the main one kept.
fn edit_each(
    v: &mut EditorView,
    event: UserEvent,
    cx: &mut Cx<'_>,
    mut f: impl FnMut(&TextBuffer, &SelectionRange) -> Option<RangeEdit>,
) {
    let sel = v.model.selection().clone();
    let buf = v.model.buffer().clone();
    let mut edits: Vec<(Range<usize>, Arc<str>)> = Vec::new();
    // Per range: the edit its caret follows and where inside its text.
    let mut at: Vec<Option<(usize, usize)>> = Vec::with_capacity(sel.ranges().len());
    for range in sel.ranges() {
        let Some((r, text, caret)) = f(&buf, range) else {
            at.push(None);
            continue;
        };
        match edits.last_mut() {
            Some((prev, _)) if r.start < prev.end => {
                *prev = prev.start.min(r.start)..prev.end.max(r.end);
                at.push(Some((edits.len() - 1, 0)));
            }
            _ => {
                let caret = caret.min(text.len());
                edits.push((r, Arc::from(text)));
                at.push(Some((edits.len() - 1, caret)));
            }
        }
    }
    if edits.is_empty() {
        return;
    }
    // Where each edit's text starts in the new text.
    let mut starts = Vec::with_capacity(edits.len());
    let mut delta: isize = 0;
    for (r, ins) in &edits {
        starts.push(r.start.saturating_add_signed(delta));
        delta += ins.len() as isize - r.len() as isize;
    }
    // An unedited caret moves by the edits before it.
    let shifted = |p: usize| -> usize {
        let i = edits.partition_point(|(r, _)| r.end <= p);
        let shift: isize = edits[..i]
            .iter()
            .map(|(r, ins)| ins.len() as isize - r.len() as isize)
            .sum();
        p.saturating_add_signed(shift)
    };
    let carets = sel.ranges().iter().zip(&at).map(|(r, a)| match a {
        Some((i, inside)) => SelectionRange::caret(starts[*i] + inside),
        None => SelectionRange::caret(shifted(r.head)),
    });
    let after = SelectionSet::normalized(carets, sel.main_index());
    change(v, edits, Some(after), event, cx);
}

pub(super) fn newline(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    edit_each(v, UserEvent::InputType, cx, |buf, sel| {
        let r = sel.range();
        let line_start = buf.line_to_byte(buf.byte_to_line(r.start));
        let scan_end = buf.snap(r.start.min(line_start + INDENT_SCAN), Bias::Before);
        let before = buf.slice(line_start..scan_end.max(line_start));
        let indent = before
            .find(|c: char| c != ' ' && c != '\t')
            .map_or(&*before, |i| &before[..i]);
        let text = format!("{}{indent}", buf.dominant_line_ending().as_str());
        let caret = text.len();
        Some((r, text, caret))
    });
}

/// One unit of indentation at a line's start.
fn indent_unit(v: &EditorView) -> String {
    if v.config.insert_spaces {
        " ".repeat(usize::from(v.config.tab_size.max(1)))
    } else {
        "\t".to_string()
    }
}

pub(super) fn tab(v: &mut EditorView, window: &mut Window, cx: &mut Cx<'_>) {
    if selection::tab_leaves(v) {
        v.commands.released = None;
        window.focus_next(cx);
        return;
    }
    let buf = v.model.buffer();
    let multi_line = v.model.selection().ranges().iter().any(|r| {
        let r = r.range();
        buf.byte_to_line(r.start) != buf.byte_to_line(r.end)
    });
    if multi_line {
        // Several lines: indent each that has text.
        let unit: Arc<str> = Arc::from(indent_unit(v));
        let edits = selected_lines(v)
            .into_iter()
            .map(|l| buf.line_range(l, false))
            .filter(|body| !body.is_empty())
            .map(|body| (body.start..body.start, unit.clone()))
            .collect();
        change(v, edits, None, UserEvent::Command("indent"), cx);
        return;
    }
    let spaces = v.config.insert_spaces;
    let size = usize::from(v.config.tab_size.max(1));
    edit_each(v, UserEvent::InputType, cx, |buf, sel| {
        let r = sel.range();
        let text = if spaces {
            " ".repeat(size - display_column(buf, r.start, size) % size)
        } else {
            "\t".to_string()
        };
        let caret = text.len();
        Some((r, text, caret))
    });
}

pub(super) fn outdent(v: &mut EditorView, window: &mut Window, cx: &mut Cx<'_>) {
    if selection::tab_leaves(v) {
        v.commands.released = None;
        window.focus_prev(cx);
        return;
    }
    let tab = usize::from(v.config.tab_size.max(1));
    let buf = v.model.buffer();
    let edits = selected_lines(v)
        .into_iter()
        .filter_map(|l| {
            let body = buf.line_range(l, false);
            let end = buf.snap((body.start + tab).min(body.end), Bias::Before);
            let lead = buf.slice(body.start..end.max(body.start));
            let take = if lead.starts_with('\t') {
                1
            } else {
                lead.bytes().take_while(|b| *b == b' ').count()
            };
            (take > 0).then(|| (body.start..body.start + take, Arc::from("")))
        })
        .collect();
    change(v, edits, None, UserEvent::Command("outdent"), cx);
}

/// Deletes each selection, or from each caret to where `target` puts it (## m3b-t31-multi: at
/// every range, in one transaction).
fn delete(
    v: &mut EditorView,
    target: impl Fn(&TextBuffer, usize, &dyn TokenOracle) -> usize,
    event: UserEvent,
    cx: &mut Cx<'_>,
) {
    let tok = words(v);
    let any_selected = v.model.selection().ranges().iter().any(|r| !r.is_empty());
    let event = if any_selected {
        UserEvent::DeleteSelection
    } else {
        event
    };
    edit_each(v, event, cx, |buf, now| {
        let r = if now.is_empty() {
            let to = target(buf, now.head, tok.as_ref());
            now.head.min(to)..now.head.max(to)
        } else {
            now.range()
        };
        (!r.is_empty()).then(|| (r, String::new(), 0))
    });
}

pub(super) fn delete_backward(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    delete(
        v,
        |buf, at, tok| move_offset(buf, at, Dir::Back, Unit::Grapheme, tok),
        UserEvent::DeleteBackward,
        cx,
    );
}

pub(super) fn delete_forward(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    delete(
        v,
        |buf, at, tok| move_offset(buf, at, Dir::Forward, Unit::Grapheme, tok),
        UserEvent::DeleteForward,
        cx,
    );
}

pub(super) fn delete_word_backward(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    delete(
        v,
        |buf, at, tok| move_offset(buf, at, Dir::Back, Unit::Word, tok),
        UserEvent::DeleteBackward,
        cx,
    );
}

pub(super) fn delete_word_forward(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    delete(
        v,
        |buf, at, tok| move_offset(buf, at, Dir::Forward, Unit::Word, tok),
        UserEvent::DeleteForward,
        cx,
    );
}

/// To the line's start; at the line's start, the break before it (the macOS convention).
pub(super) fn delete_to_line_start(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    delete(
        v,
        |buf, at, tok| {
            let start = buf.line_to_byte(buf.byte_to_line(at));
            if at > start {
                start
            } else {
                move_offset(buf, at, Dir::Back, Unit::Grapheme, tok)
            }
        },
        UserEvent::DeleteBackward,
        cx,
    );
}

/// The Edit menu's Undo is enabled: the editor is editable and has a group to undo.
pub fn can_undo(v: &EditorView) -> bool {
    !v.config.read_only && v.model.undo_depth() > 0
}

/// The Edit menu's Redo is enabled.
pub fn can_redo(v: &EditorView) -> bool {
    !v.config.read_only && v.model.redo_depth() > 0
}

/// What the Edit menu's Undo would take back (its label, "Undo Typing", is keyed by the
/// kind); `None` when there is nothing to undo or the editor is read-only.
pub fn undo_kind(v: &EditorView) -> Option<UserEvent> {
    can_undo(v).then(|| v.model.undo_kind()).flatten()
}

/// What the Edit menu's Redo would apply again; `None` as for [`undo_kind`].
pub fn redo_kind(v: &EditorView) -> Option<UserEvent> {
    can_redo(v).then(|| v.model.redo_kind()).flatten()
}

/// Undo (`redo`: redo). A read-only editor refuses: the view's `read_only` is no model filter,
/// so the model itself would apply it.
fn history(v: &mut EditorView, redo: bool, cx: &mut Cx<'_>) {
    super::acted(v);
    if v.config.read_only {
        return;
    }
    // Not the IME's edit: the composition ends first, keeping its text as its own undo group.
    if v.ime.is_composing() {
        v.ime.unmark(&mut v.model);
        v.composition_changed(false, cx);
    }
    let (committed, event) = if redo {
        (v.model.redo(), UserEvent::Redo)
    } else {
        (v.model.undo(), UserEvent::Undo)
    };
    if let Some(committed) = committed {
        v.apply_committed(committed, event, cx);
    }
}

pub(super) fn undo(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    history(v, false, cx);
}

pub(super) fn redo(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    history(v, true, cx);
}

pub(super) fn copy(v: &mut EditorView, window: &mut Window, cx: &mut Cx<'_>) {
    super::acted(v);
    clipboard::copy(v, window, cx);
}

pub(super) fn cut(v: &mut EditorView, window: &mut Window, cx: &mut Cx<'_>) {
    super::acted(v);
    if v.config.read_only {
        return;
    }
    clipboard::cut(v, window, cx);
}

pub(super) fn paste(v: &mut EditorView, window: &mut Window, cx: &mut Cx<'_>) {
    super::acted(v);
    if v.config.read_only {
        return;
    }
    clipboard::paste(v, window, cx);
}
