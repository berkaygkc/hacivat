//! The value viewer (HV-GRID-12, R-M3-82; M3c Task 38): a 480 px popover with the column and
//! type, the wrapped value, a pretty toggle for JSON and XML, hex for binary, Copy raw value,
//! and "Re-run with a 64 MiB value limit" for a truncated value.
//!
//! ## m3b-t01b-seams: the seam (Task 1b). STUB: opens nothing (`GridEvent::ValueRequested` is
//! not answered yet). Task 38 fills it.

use hacivat_ui_core::gpui_surface::App;
use hacivat_ui_grid::ModelCell;

use crate::console::ConsoleId;

/// Opens the viewer on `cell` of console `console`'s active result (the tile calls it on
/// `GridEvent::ValueRequested`). STUB: nothing.
pub fn open(console: ConsoleId, cell: ModelCell, cx: &mut App) {
    let _ = (console, cell, cx);
}
