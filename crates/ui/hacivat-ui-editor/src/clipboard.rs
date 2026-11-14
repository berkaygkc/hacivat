//! The clipboard (UI-EDIT-14; Task 27): copy and cut (line-wise without a selection), and an
//! asynchronous paste applied to the selection current when it lands, as one `InputPaste`
//! transaction: a leading BOM stripped, line endings converted, a NUL shown and warned about,
//! a paste past 20 MB refused (`EditorEvent::PasteRefused`, R-M3-49).
//!
//! Task 1b: no-ops.

use hacivat_ui_core::gpui_surface::{Context, Window};

use crate::view::EditorView;

/// The module-owned state (P11): a paste in flight (Task 27).
#[derive(Debug, Clone, Default)]
pub struct PasteState {}

pub fn copy(v: &mut EditorView, window: &mut Window, cx: &mut Context<EditorView>) {
    let _ = (v, window, cx);
}

pub fn cut(v: &mut EditorView, window: &mut Window, cx: &mut Context<EditorView>) {
    let _ = (v, window, cx);
}

pub fn paste(v: &mut EditorView, window: &mut Window, cx: &mut Context<EditorView>) {
    let _ = (v, window, cx);
}
