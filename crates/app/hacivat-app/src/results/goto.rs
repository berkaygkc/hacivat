//! Go to row (HV-GRID-14; M3c Task 37): the go-to field over `hacivat_ui_grid::parse_goto`,
//! fetching past the loaded rows with a confirmation above a million.
//!
//! ## m3b-t01b-seams: the seam (Task 1b). STUB: opens nothing (`GridEvent::GotoRequested` is
//! not answered yet). Task 37 fills it.

use hacivat_ui_core::gpui_surface::App;

use crate::console::ConsoleId;

/// Opens the go-to field over console `console`'s active result (the tile calls it on
/// `GridEvent::GotoRequested`). STUB: nothing.
pub fn open(console: ConsoleId, cx: &mut App) {
    let _ = (console, cx);
}
