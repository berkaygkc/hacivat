//! The height map (UI-EDIT-08; Task 11). M3a: uniform rows; M3b adds a chunked map for folds
//! and wrapped lines. Absolute y is `f64`: a million 18 px lines is 1.8e7 px, past what `f32`
//! keeps exact to the pixel.
//!
//! ## m3b-t01b-seams: the [`HeightMap::Chunked`] variant and [`HeightMap::chunked`] are Task
//! 1b's seam (P-M3b-24: the map is no longer `Copy`). The constructor is a stub that answers a
//! uniform map (no line hidden), and nothing builds `Chunked` yet; Task 32 owns the variant's
//! shape and its bodies.

use std::ops::Range;
use std::sync::Arc;

/// Row heights of a document.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum HeightMap {
    Uniform {
        lines: u32,
        line_height: f64,
    },
    // ## m3b-t01b-seams (Task 32 fills): rows of `line_height` with the `hidden` line ranges
    // (folds) taking no height. Until Task 32 its arms read it as a uniform map.
    Chunked {
        lines: u32,
        line_height: f64,
        hidden: Arc<[Range<u32>]>,
    },
}

impl HeightMap {
    /// `lines` rows of `line_height` pixels each.
    pub fn uniform(lines: u32, line_height: f64) -> Self {
        HeightMap::Uniform { lines, line_height }
    }

    // ## m3b-t01b-seams
    /// `lines` rows of `line_height` pixels each, the `hidden` line ranges (sorted, merged)
    /// taking no height. STUB (Task 32): the uniform map; no line is hidden.
    pub fn chunked(lines: u32, line_height: f64, hidden: &[Range<u32>]) -> Self {
        let _ = hidden;
        HeightMap::uniform(lines, line_height)
    }

    /// Rows in the document.
    pub fn lines(&self) -> u32 {
        match self {
            HeightMap::Uniform { lines, .. } | HeightMap::Chunked { lines, .. } => *lines,
        }
    }

    /// The top y of `line`; a line past the end answers the document's height.
    pub fn y_of(&self, line: u32) -> f64 {
        match self {
            HeightMap::Uniform { lines, line_height }
            | HeightMap::Chunked {
                lines, line_height, ..
            } => f64::from(line.min(*lines)) * line_height,
        }
    }

    /// The height of the whole document.
    pub fn total_height(&self) -> f64 {
        self.y_of(self.lines())
    }

    /// The line under `y`, clamped to `0..lines` (an empty map answers 0).
    pub fn line_at_y(&self, y: f64) -> u32 {
        match *self {
            HeightMap::Uniform { lines, line_height }
            | HeightMap::Chunked {
                lines, line_height, ..
            } => {
                if lines == 0 || line_height <= 0.0 || y.is_nan() || y <= 0.0 {
                    return 0;
                }
                let l = (y / line_height).floor();
                if l >= f64::from(lines - 1) {
                    lines - 1
                } else {
                    l as u32
                }
            }
        }
    }

    /// How far into its row `y` lies.
    pub fn offset_in_row(&self, y: f64) -> f64 {
        (y - self.y_of(self.line_at_y(y))).max(0.0)
    }
}
