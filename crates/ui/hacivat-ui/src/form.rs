//! Form layout (NOTES "NEW primitives"): a label column and a control, a hint row under a
//! control, and the field well that restores text-3 inside dialogs (NOTES cross-cutting #4).
//! Task 6 draws them; the Task 1 stubs are empty `div`s.

use hacivat_ui_core::gpui_surface::{Div, IntoElement, SharedString, div};
use hacivat_ui_core::theme::Theme;

/// A labelled form row.
pub fn form_row(_theme: &Theme, _label: SharedString, _control: impl IntoElement) -> Div {
    div()
}

/// A hint under a control, aligned with the control column.
pub fn hint_row(_theme: &Theme, _content: impl IntoElement) -> Div {
    div()
}

/// The well behind a field: its own fill, so text-3 is allowed on it.
pub fn field_well(_theme: &Theme) -> Div {
    div()
}
