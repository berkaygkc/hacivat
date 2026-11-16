//! The transaction mode and isolation chip in the console toolbar (D-014, D-036; M3b Task 22,
//! `design/m3b-safe` surface 8).
//!
//! ## m3b-t01b-seams: the slot (Task 1b). STUB: renders nothing; the M3a transaction segment
//! stays. Task 22 fills it.

use hacivat_ui_core::gpui_surface::{AnyElement, App};

use crate::console::ConsoleId;

/// The chip of console `console`. STUB: `None`.
pub fn render(console: ConsoleId, cx: &App) -> Option<AnyElement> {
    let _ = (console, cx);
    None
}
