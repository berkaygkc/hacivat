//! The selection model (Task 17): anchor and focus, rectangles in view coordinates, row and
//! column sets, select-all; cell counts in `u128`; copy normalisation (a union that is neither
//! stacked nor concatenable is refused; more than 1,000,000 cells is refused).

use std::ops::Range;

use crate::source::{ColId, ModelCell};

/// A grid's selection.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Selection {
    anchor: Option<ModelCell>,
    focus: Option<ModelCell>,
    rows: Vec<Range<u64>>,
    cols: Vec<ColId>,
    all: bool,
}

impl Selection {
    /// The focused cell.
    pub fn focus(&self) -> Option<ModelCell> {
        self.focus
    }

    /// The anchor of a range selection.
    pub fn anchor(&self) -> Option<ModelCell> {
        self.anchor
    }

    /// Selected row ranges (model rows).
    pub fn rows(&self) -> &[Range<u64>] {
        &self.rows
    }

    /// Selected columns.
    pub fn cols(&self) -> &[ColId] {
        &self.cols
    }

    /// Everything is selected.
    pub fn is_all(&self) -> bool {
        self.all
    }
}

/// What a copy takes: row ranges and columns, in view order.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CopyRegion {
    pub rows: Vec<Range<u64>>,
    pub cols: Vec<ColId>,
}

/// Why a copy was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CopyRefusal {
    /// Disjoint rectangles that neither stack nor concatenate.
    MultipleSelection,
    TooMany {
        cells: u128,
    },
}
