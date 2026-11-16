//! The run-policy popover (HV-EXEC-08; M3b Task 24, `design/m3b-safe`): the console's error
//! policy (Stop on error / Ask) and Run as one, from the console toolbar.
//!
//! ## m3b-t01b-seams: the seam (Task 1b). STUB: renders nothing, so the toolbar stays M3a's.
//! Task 24 fills it.

use hacivat_ui_core::gpui_surface::{AnyElement, App};

use crate::console::ConsoleId;

/// The popover's toolbar item for console `console`. STUB: `None`.
pub fn render(console: ConsoleId, cx: &App) -> Option<AnyElement> {
    let _ = (console, cx);
    None
}
