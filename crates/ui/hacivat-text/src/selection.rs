//! Selections (UI-EDIT-05 model; Task 10). Many ranges can be held; M3a's single-cursor
//! commands use the main one.
//!
//! Task 1b: construction, the main range and mapping through a change (both ends keep `After`,
//! so typing at a caret moves it past the text). Task 10 adds normalisation and merging.

use smallvec::{SmallVec, smallvec};

use crate::buffer::Bias;
use crate::change::{ChangeDesc, MapMode};

/// One selection: the anchor stays, the head moves; `goal_x` keeps the column of vertical
/// moves; `assoc` the side a caret keeps to at a wrap or an inlay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectionRange {
    pub anchor: usize,
    pub head: usize,
    pub goal_x: Option<f32>,
    pub assoc: Bias,
}

impl SelectionRange {
    /// A caret at `at`.
    pub fn caret(at: usize) -> Self {
        SelectionRange {
            anchor: at,
            head: at,
            goal_x: None,
            assoc: Bias::After,
        }
    }

    /// The selected bytes, low to high.
    pub fn range(&self) -> std::ops::Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// The selections of a document and which is the main one.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionSet {
    ranges: SmallVec<[SelectionRange; 1]>,
    main: usize,
}

impl SelectionSet {
    /// One caret at `at`.
    pub fn cursor(at: usize) -> Self {
        Self::single(SelectionRange::caret(at))
    }

    pub fn single(r: SelectionRange) -> Self {
        SelectionSet {
            ranges: smallvec![r],
            main: 0,
        }
    }

    /// The set of `r` with `main` as the main range (Task 10 sorts and merges overlaps; Task
    /// 1b keeps the order, and an empty `r` is a caret at 0).
    pub fn normalized(r: impl IntoIterator<Item = SelectionRange>, main: usize) -> Self {
        let ranges: SmallVec<[SelectionRange; 1]> = r.into_iter().collect();
        if ranges.is_empty() {
            return Self::cursor(0);
        }
        let main = main.min(ranges.len() - 1);
        SelectionSet { ranges, main }
    }

    /// Every range mapped through `c`.
    pub fn map(&self, c: &ChangeDesc) -> Self {
        let map = |at: usize| c.map_pos(at, Bias::After, MapMode::Simple).unwrap_or(at);
        SelectionSet {
            ranges: self
                .ranges
                .iter()
                .map(|r| SelectionRange {
                    anchor: map(r.anchor),
                    head: map(r.head),
                    ..*r
                })
                .collect(),
            main: self.main,
        }
    }

    pub fn main(&self) -> &SelectionRange {
        &self.ranges[self.main]
    }

    pub fn ranges(&self) -> &[SelectionRange] {
        &self.ranges
    }
}
