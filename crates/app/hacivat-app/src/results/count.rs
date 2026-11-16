//! Σ Count rows (HV-RES-10; M3b Task 25): the result toolbar's count of the statement's rows
//! (estimate vs exact, a cancellable spinner, the refusal reason).
//!
//! ## m3b-t01b-seams: the slot (Task 1b). STUB: renders nothing, so the result toolbar stays
//! M3a's. Task 25 fills it.

use hacivat_ui_core::gpui_surface::{AnyElement, App};

use crate::console::ConsoleId;

/// The toolbar item of console `console`'s active result. STUB: `None`.
pub fn render(console: ConsoleId, cx: &App) -> Option<AnyElement> {
    let _ = (console, cx);
    None
}
