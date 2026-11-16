//! The grid find bar (HV-GRID-09, R-M3-10; M3c Task 37): it replaces the result toolbar's left
//! group, scans display strings off the UI thread, highlights matches
//! (`GridView::set_highlights`) and filters rows (`ViewMap::Filtered`).
//!
//! ## m3b-t01b-seams: the seam (Task 1b). STUB: opens nothing (`GridEvent::FindRequested` is
//! not answered yet). Task 37 fills it.

use hacivat_ui_core::gpui_surface::App;

use crate::console::ConsoleId;

/// Opens the find bar over console `console`'s active result (the tile calls it on
/// `GridEvent::FindRequested`). STUB: nothing.
pub fn open(console: ConsoleId, cx: &mut App) {
    let _ = (console, cx);
}
