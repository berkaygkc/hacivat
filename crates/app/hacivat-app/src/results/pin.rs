//! Pinned result tabs (HV-EXEC-07 UI; M3b Task 25): a pinned tab is never reused by the next
//! run; pin and unpin from the tab's context menu.
//!
//! ## m3b-t01b-seams: the seam (Task 1b). STUB: no tab is pinned, so every run reuses tabs as
//! in M3a. Task 25 fills it.

use hacivat_ui_core::gpui_surface::App;

use crate::console::ConsoleId;

/// Whether console `console`'s result tab `ix` is pinned. STUB: `false`.
pub fn is_pinned(console: ConsoleId, ix: usize, cx: &App) -> bool {
    let _ = (console, ix, cx);
    false
}
