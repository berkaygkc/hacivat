//! Row-anchored scrolling (UI-EDIT-08; Task 11): edits above the anchor keep the visible
//! content still; the absolute `f64` y is for the scrollbar only.

/// The first visible row: its buffer line, its wrap row, and how far into it the view starts.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScrollAnchor {
    pub line: u32,
    pub wrap_index: u16,
    pub offset_in_row: f32,
}
