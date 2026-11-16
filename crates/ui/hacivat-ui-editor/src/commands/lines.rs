//! Line commands (M3b Task 34): toggle comment, duplicate, delete and move lines, normalise
//! line endings, and history recall (`editor.history_prev`/`_next`).
//!
//! ## m3b-t01b-seams: the rows are registered (`commands/mod.rs`) and every body is a no-op
//! until Task 34 fills it. A no-op never edits, so a stub can never change the text or what Run
//! sends.

use hacivat_ui_core::gpui_surface::{Context, Window};

use crate::view::EditorView;

type Cx<'a> = Context<'a, EditorView>;

pub(super) fn toggle_comment(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let _ = (v, cx);
}

pub(super) fn duplicate_line(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let _ = (v, cx);
}

pub(super) fn delete_line(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let _ = (v, cx);
}

pub(super) fn move_line_up(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let _ = (v, cx);
}

pub(super) fn move_line_down(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let _ = (v, cx);
}

pub(super) fn normalize_line_endings(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let _ = (v, cx);
}

pub(super) fn history_prev(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let _ = (v, cx);
}

pub(super) fn history_next(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let _ = (v, cx);
}
