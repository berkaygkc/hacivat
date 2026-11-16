//! The grid's context menu (HV-GRID-04 menu; M3c Task 39): cell, header and row-number variants
//! with disabled items and their reasons.
//!
//! ## m3b-t01b-seams: the seam (Task 1b). STUB: opens nothing
//! (`GridEvent::ContextMenuRequested` is not answered yet). Task 39 fills it.

use hacivat_ui_core::gpui_surface::{App, Pixels, Point};
use hacivat_ui_grid::{ColId, ModelCell};

use crate::console::ConsoleId;

/// Opens the menu at `at` (window coordinates) over `cell` or `header` of console `console`'s
/// active result (the tile calls it on `GridEvent::ContextMenuRequested`). STUB: nothing.
pub fn open(
    console: ConsoleId,
    at: Point<Pixels>,
    cell: Option<ModelCell>,
    header: Option<ColId>,
    cx: &mut App,
) {
    let _ = (console, at, cell, header, cx);
}
