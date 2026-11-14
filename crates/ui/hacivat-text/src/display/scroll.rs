//! Row-anchored scrolling (UI-EDIT-08; Task 11): edits above the anchor keep the visible
//! content still; the absolute `f64` y is for the scrollbar only.

use crate::buffer::{Bias, TextBuffer};
use crate::change::{ChangeDesc, MapMode};
use crate::display::HeightMap;

/// The first visible row: its buffer line, its wrap row, and how far into it the view starts.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScrollAnchor {
    pub line: u32,
    pub wrap_index: u16,
    pub offset_in_row: f32,
}

impl ScrollAnchor {
    /// The anchor for an absolute scrollbar position.
    pub fn from_y(map: &HeightMap, y: f64) -> Self {
        ScrollAnchor {
            line: map.line_at_y(y),
            wrap_index: 0,
            offset_in_row: map.offset_in_row(y) as f32,
        }
    }

    /// The absolute y of the view's top, for the scrollbar only.
    pub fn y(&self, map: &HeightMap) -> f64 {
        map.y_of(self.line) + f64::from(self.offset_in_row)
    }

    /// Re-pins the anchor after `change` (a `desc()` of the committed change, applied to
    /// `old`, producing `new`): the anchor follows the start of its line, so lines inserted
    /// or removed above it leave the visible content where it was. O(log n).
    pub fn rebase(&mut self, old: &TextBuffer, new: &TextBuffer, change: &ChangeDesc) {
        let line = (self.line as usize).min(old.len_lines().saturating_sub(1));
        let at = old.line_to_byte(line);
        let mapped = change
            .map_pos(at, Bias::After, MapMode::Simple)
            .unwrap_or(0)
            .min(new.len_bytes());
        let moved = new.byte_to_line(mapped);
        if moved != self.line as usize {
            self.wrap_index = 0;
        }
        self.line = moved as u32;
    }
}
