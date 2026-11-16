//! Find commands (UI-EDIT-22 without replace; M3b Task 33): open the find bar
//! (`EditorEvent::FindRequested`), step to the next or previous match, close.
//!
//! ## m3b-t01b-seams: the rows are registered (`commands/mod.rs`) and every body is a no-op
//! until Task 33 fills it over `crate::find`.

use hacivat_ui_core::gpui_surface::{Context, Window};

use crate::view::EditorView;

type Cx<'a> = Context<'a, EditorView>;

pub(super) fn find(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let _ = (v, cx);
}

pub(super) fn find_next(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let _ = (v, cx);
}

pub(super) fn find_prev(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let _ = (v, cx);
}

pub(super) fn find_close(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let _ = (v, cx);
}
