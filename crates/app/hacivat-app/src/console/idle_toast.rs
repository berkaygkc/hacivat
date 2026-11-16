//! The idle-transaction toast (D-014, C1; M3b Task 22, `design/m3b-safe` surface 10): a user
//! transaction left open past the idle threshold.
//!
//! ## m3b-t01b-seams: the slot (Task 1b). STUB: renders nothing. Task 22 fills it.

use hacivat_ui_core::gpui_surface::{AnyElement, App};

use crate::console::ConsoleId;

/// The toast of console `console`. STUB: `None`.
pub fn render(console: ConsoleId, cx: &App) -> Option<AnyElement> {
    let _ = (console, cx);
    None
}
