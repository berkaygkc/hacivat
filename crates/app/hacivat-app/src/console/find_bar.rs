//! The editor's find bar (UI-EDIT-22 without replace, R-M3-10, D-074; M3c Task 33): the query
//! field on `TextInput`, Match case, Regex, Words, Turkish-tolerant, "k of N", and the
//! `regex`-backed `FindMatcher` the editor steps with (Task 37's grid find reuses the builder).
//!
//! ## m3b-t01b-seams: the slot (Task 1b). STUB: renders nothing (the editor's
//! `EditorEvent::FindRequested` is not answered yet). Task 33 fills it.

use hacivat_ui_core::gpui_surface::{AnyElement, App};

use crate::console::ConsoleId;

/// The bar of console `console`, while it is open. STUB: `None`.
pub fn render(console: ConsoleId, cx: &App) -> Option<AnyElement> {
    let _ = (console, cx);
    None
}
