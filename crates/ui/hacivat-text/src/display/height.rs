//! The height map (UI-EDIT-08; Task 11). M3a: uniform rows; M3c (Task 32) adds the chunked map
//! for folds. Absolute y is `f64`: a million 18 px lines is 1.8e7 px, past what `f32` keeps
//! exact to the pixel.
//!
//! ## m3b-t01b-seams: the [`HeightMap::Chunked`] variant and [`HeightMap::chunked`] (Task 1b;
//! P-M3b-24: the map is no longer `Copy`). ## m3c-t32-fold: the variant's shape and bodies.
//!
//! **Chunked.** The document is cut into chunks at the hidden-line runs the folds make: each
//! run takes no height, every other line `line_height`. The runs are kept sorted and merged
//! with the count of hidden lines before each (a prefix sum), so `y_of` and `line_at_y` are one
//! binary search each, O(log runs) ≤ O(log n), whatever the document's size. Without a run the
//! map answers exactly what the uniform map does.

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
    // ## m3c-t32-fold (the shape is Task 32's): rows of `line_height`, the `hidden` runs taking
    // no height; `before[i]` is the number of hidden lines in `hidden[..i]` (one more entry
    // than `hidden`: the last is the total).
    Chunked {
        lines: u32,
        line_height: f64,
        hidden: Arc<[Range<u32>]>,
        before: Arc<[u32]>,
    },
}

impl HeightMap {
    /// `lines` rows of `line_height` pixels each.
    pub fn uniform(lines: u32, line_height: f64) -> Self {
        HeightMap::Uniform { lines, line_height }
    }

    // ## m3c-t32-fold
    /// `lines` rows of `line_height` pixels each, the `hidden` line ranges taking no height
    /// (clamped to the document, sorted and merged here; empty ones dropped).
    pub fn chunked(lines: u32, line_height: f64, hidden: &[Range<u32>]) -> Self {
        let mut runs: Vec<Range<u32>> = hidden
            .iter()
            .map(|r| r.start.min(lines)..r.end.min(lines))
            .filter(|r| r.start < r.end)
            .collect();
        runs.sort_by_key(|r| r.start);
        let mut merged: Vec<Range<u32>> = Vec::with_capacity(runs.len());
        for r in runs {
            match merged.last_mut() {
                Some(prev) if r.start <= prev.end => prev.end = prev.end.max(r.end),
                _ => merged.push(r),
            }
        }
        let mut before = Vec::with_capacity(merged.len() + 1);
        let mut n = 0u32;
        before.push(0);
        for r in &merged {
            n += r.end - r.start;
            before.push(n);
        }
        HeightMap::Chunked {
            lines,
            line_height,
            hidden: merged.into(),
            before: before.into(),
        }
    }

    /// Rows in the document (hidden ones included).
    pub fn lines(&self) -> u32 {
        match self {
            HeightMap::Uniform { lines, .. } | HeightMap::Chunked { lines, .. } => *lines,
        }
    }

    fn line_height(&self) -> f64 {
        match self {
            HeightMap::Uniform { line_height, .. } | HeightMap::Chunked { line_height, .. } => {
                *line_height
            }
        }
    }

    /// The hidden-line runs, sorted and merged (none for a uniform map).
    pub fn hidden(&self) -> &[Range<u32>] {
        match self {
            HeightMap::Uniform { .. } => &[],
            HeightMap::Chunked { hidden, .. } => hidden,
        }
    }

    /// The index of the run holding `line`, or of the first run after it (`Err`).
    fn run_of(&self, line: u32) -> Result<usize, usize> {
        let hidden = self.hidden();
        let i = hidden.partition_point(|r| r.end <= line);
        match hidden.get(i) {
            Some(r) if r.start <= line => Ok(i),
            _ => Err(i),
        }
    }

    /// Hidden lines before `line`.
    fn hidden_before(&self, line: u32) -> u32 {
        let HeightMap::Chunked { hidden, before, .. } = self else {
            return 0;
        };
        match self.run_of(line) {
            Ok(i) => before[i] + (line - hidden[i].start),
            Err(i) => before[i],
        }
    }

    /// The rows the view shows: the lines less the hidden ones.
    pub fn visible_rows(&self) -> u32 {
        match self {
            HeightMap::Uniform { lines, .. } => *lines,
            HeightMap::Chunked { lines, before, .. } => lines - before.last().copied().unwrap_or(0),
        }
    }

    /// `line` is hidden by a fold.
    pub fn is_hidden(&self, line: u32) -> bool {
        self.run_of(line).is_ok()
    }

    /// The visible line that shows `line`: itself, or the header above a hidden run.
    pub fn visible_line(&self, line: u32) -> u32 {
        match self.run_of(line) {
            Ok(i) => self.hidden()[i].start.saturating_sub(1),
            Err(_) => line,
        }
    }

    /// `line`, or the first line after the hidden run holding it (the document's line count
    /// when the run reaches its end).
    pub fn next_visible(&self, line: u32) -> u32 {
        match self.run_of(line) {
            Ok(i) => self.hidden()[i].end,
            Err(_) => line,
        }
    }

    /// The visible row `line` is drawn on (its header's for a hidden line), from 0.
    pub fn row_of(&self, line: u32) -> u32 {
        let line = self.visible_line(line.min(self.lines()));
        line - self.hidden_before(line)
    }

    /// The line drawn on visible row `row`, clamped to the last visible row.
    pub fn line_of_row(&self, row: u32) -> u32 {
        let rows = self.visible_rows();
        if rows == 0 {
            return 0;
        }
        let row = row.min(rows - 1);
        match self {
            HeightMap::Uniform { .. } => row,
            HeightMap::Chunked { hidden, before, .. } => {
                // The runs that start at or before visible row `row` (a run's start is at the
                // visible row `start - before`).
                let (mut lo, mut hi) = (0, hidden.len());
                while lo < hi {
                    let mid = lo + (hi - lo) / 2;
                    if hidden[mid].start - before[mid] <= row {
                        lo = mid + 1;
                    } else {
                        hi = mid;
                    }
                }
                row + before[lo]
            }
        }
    }

    /// The top y of `line` (a hidden line: where the next visible one starts); a line past the
    /// end answers the document's height.
    pub fn y_of(&self, line: u32) -> f64 {
        let line = line.min(self.lines());
        f64::from(line - self.hidden_before(line)) * self.line_height()
    }

    /// The height of the whole document.
    pub fn total_height(&self) -> f64 {
        self.y_of(self.lines())
    }

    /// The visible line under `y`, clamped to the visible rows (an empty map answers 0).
    pub fn line_at_y(&self, y: f64) -> u32 {
        let lh = self.line_height();
        let rows = self.visible_rows();
        if rows == 0 || lh <= 0.0 || y.is_nan() || y <= 0.0 {
            return self.line_of_row(0);
        }
        let r = (y / lh).floor();
        let row = if r >= f64::from(rows - 1) {
            rows - 1
        } else {
            r as u32
        };
        self.line_of_row(row)
    }

    /// How far into its row `y` lies.
    pub fn offset_in_row(&self, y: f64) -> f64 {
        (y - self.y_of(self.line_at_y(y))).max(0.0)
    }
}
