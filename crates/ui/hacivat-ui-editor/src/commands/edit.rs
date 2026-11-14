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
//! Every edit refuses on a read-only editor (`super::change`; undo, redo, cut and paste check
//! by themselves).

use std::ops::Range;
use std::sync::Arc;

use hacivat_text::units::{SCAN_LIMIT, grapheme_column};
use hacivat_text::{
    Bias, Dir, SelectionSet, TextBuffer, TokenOracle, Unit, UserEvent, move_offset,
};
use hacivat_ui_core::gpui_surface::{Context, Window};
use unicode_segmentation::UnicodeSegmentation as _;

use super::{change, selection, words};
use crate::clipboard;
use crate::view::EditorView;

type Cx<'a> = Context<'a, EditorView>;

/// How far Enter looks for the line's indentation, in bytes.
const INDENT_SCAN: usize = 4096;

/// The display column of `at` on its line, tabs expanded to `tab` columns (grapheme columns on
/// a line longer than the scan limit).
fn display_column(buf: &TextBuffer, at: usize, tab: usize) -> usize {
    let start = buf.line_to_byte(buf.byte_to_line(at));
    if at - start > SCAN_LIMIT {
        return grapheme_column(buf, at);
    }
    buf.slice(start..at).graphemes(true).fold(0, |col, g| {
        if g == "\t" {
            col + tab - col % tab
        } else {
            col + 1
        }
    })
}

/// The lines the main selection touches: a selection that ends at a line's start does not
/// touch that line.
fn selected_lines(v: &EditorView) -> Range<usize> {
    let buf = v.model.buffer();
    let r = v.model.selection().main().range();
    let first = buf.byte_to_line(r.start);
    let mut last = buf.byte_to_line(r.end);
    if last > first && buf.line_to_byte(last) == r.end {
        last -= 1;
    }
    first..last + 1
}

pub(super) fn newline(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let buf = v.model.buffer();
    let r = v.model.selection().main().range();
    let line_start = buf.line_to_byte(buf.byte_to_line(r.start));
    let scan_end = buf.snap(r.start.min(line_start + INDENT_SCAN), Bias::Before);
    let before = buf.slice(line_start..scan_end.max(line_start));
    let indent = before
        .find(|c: char| c != ' ' && c != '\t')
        .map_or(&*before, |i| &before[..i]);
    let text = format!("{}{indent}", buf.dominant_line_ending().as_str());
    let caret = r.start + text.len();
    change(
        v,
        vec![(r, Arc::from(text))],
        Some(SelectionSet::cursor(caret)),
        UserEvent::InputType,
        cx,
    );
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
    let lines = selected_lines(v);
    let buf = v.model.buffer();
    let r = v.model.selection().main().range();
    if buf.byte_to_line(r.start) != buf.byte_to_line(r.end) {
        // Several lines: indent each that has text.
        let unit: Arc<str> = Arc::from(indent_unit(v));
        let edits = lines
            .map(|l| buf.line_range(l, false))
            .filter(|body| !body.is_empty())
            .map(|body| (body.start..body.start, unit.clone()))
            .collect();
        change(v, edits, None, UserEvent::Command("indent"), cx);
        return;
    }
    let text = if v.config.insert_spaces {
        let tab = usize::from(v.config.tab_size.max(1));
        " ".repeat(tab - display_column(buf, r.start, tab) % tab)
    } else {
        "\t".to_string()
    };
    let caret = r.start + text.len();
    change(
        v,
        vec![(r, Arc::from(text))],
        Some(SelectionSet::cursor(caret)),
        UserEvent::InputType,
        cx,
    );
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

/// Deletes the selection, or from the caret to where `target` puts it.
fn delete(
    v: &mut EditorView,
    target: impl Fn(&TextBuffer, usize, &dyn TokenOracle) -> usize,
    event: UserEvent,
    cx: &mut Cx<'_>,
) {
    let now = *v.model.selection().main();
    let (r, event) = if now.is_empty() {
        let to = target(v.model.buffer(), now.head, words(v).as_ref());
        (now.head.min(to)..now.head.max(to), event)
    } else {
        (now.range(), UserEvent::DeleteSelection)
    };
    if r.is_empty() {
        return;
    }
    let caret = r.start;
    change(
        v,
        vec![(r, Arc::from(""))],
        Some(SelectionSet::cursor(caret)),
        event,
        cx,
    );
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
