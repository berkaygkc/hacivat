//! Multi-cursor commands (UI-EDIT-05 multi; M3b Task 31): add a caret above or below, select
//! the next or every occurrence, split the selection into lines, and the selection-only undo
//! and redo (Ctrl/⌘+U).
//!
//! ## m3b-t01b-seams: the rows are registered (`commands/mod.rs`) and every body is a no-op
//! until Task 31 fills it over `hacivat_text::multi`. A no-op never edits, so a stub can never
//! change the text or what Run sends.

use hacivat_ui_core::gpui_surface::{Context, Window};

use crate::view::EditorView;

type Cx<'a> = Context<'a, EditorView>;

pub(super) fn add_caret_above(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let _ = (v, cx);
}

pub(super) fn add_caret_below(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let _ = (v, cx);
}

pub(super) fn select_next_occurrence(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let _ = (v, cx);
}

pub(super) fn select_all_occurrences(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let _ = (v, cx);
}

pub(super) fn split_selection_into_lines(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let _ = (v, cx);
}

pub(super) fn undo_selection(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let _ = (v, cx);
}

pub(super) fn redo_selection(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let _ = (v, cx);
}
