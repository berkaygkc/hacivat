//! The editor element (UI-EDIT-07 paint, -08 scroll; Task 19): one custom element for the text
//! and the gutter area, in the harman paint order, calling `gutter::paint`, `marks::paint`,
//! `preview::paint_band`, `scrollbar::paint` and `a11y::declare` at their paint points; each
//! frame's layout is kept for `EditorView::last_layout` (P11).

use hacivat_ui_core::gpui_surface::{Bounds, Pixels, Point};

/// The painted rows of one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisibleRows {
    pub bounds: Bounds<Pixels>,
    pub line_height: Pixels,
    pub first_line: u32,
    pub last_line: u32,
    /// The top of `first_line`, in window coordinates.
    pub first_y: Pixels,
}

/// The last painted frame (Task 19 fills it): what IME bounds and hit testing read.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameLayout {
    rows: VisibleRows,
}

impl FrameLayout {
    /// The rows the frame painted.
    pub fn rows(&self) -> &VisibleRows {
        &self.rows
    }

    /// The x of byte `offset` on a painted line; `None` off the painted lines (Task 19).
    pub fn x_for_offset(&self, offset: usize) -> Option<Pixels> {
        let _ = offset;
        None
    }

    /// The byte offset at `p`; `None` off the painted lines (Task 19).
    pub fn offset_for_point(&self, p: Point<Pixels>) -> Option<usize> {
        let _ = p;
        None
    }
}
