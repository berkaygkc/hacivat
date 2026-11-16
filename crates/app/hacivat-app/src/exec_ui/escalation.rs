//! Cancel escalation (HV-EXEC-09; M3b Task 23, `design/m3b-safe` surface 6): a cancel that is
//! slow to land offers Terminate (and Deactivate where the engine has it); Escape never sends
//! either (R-M3-38).
//!
//! ## m3b-t01b-seams: the seams (Task 1b). STUB: both are no-ops (no badge, no dialog; the M3a
//! "Cancelling…" state stays). Task 23 fills them.

use hacivat_ui_core::gpui_surface::{App, Window};

use crate::console::ConsoleId;

/// The session reported a slow cancel on console `console` (`SessionEvent::SlowCancel`).
/// STUB: nothing.
pub fn on_slow_cancel(console: ConsoleId, cx: &mut App) {
    let _ = (console, cx);
}

/// Opens the escalation dialog for console `console` (a second `exec.cancel` while
/// cancelling). STUB: nothing.
pub fn open_dialog(console: ConsoleId, window: &mut Window, cx: &mut App) {
    let _ = (console, window, cx);
}
