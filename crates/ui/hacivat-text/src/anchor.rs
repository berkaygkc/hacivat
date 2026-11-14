//! Anchors: positions that follow edits (UI-EDIT-03; Task 5 maps them in O(k + s)).
//!
//! Task 1b: a sorted vector mapped position by position through `ChangeDesc::map_pos`.

use crate::buffer::Bias;
use crate::change::{ChangeDesc, MapMode};

/// Values anchored at byte offsets, kept sorted by offset.
#[derive(Debug, Clone, PartialEq)]
pub struct AnchorSet<T> {
    anchors: Vec<(usize, Bias, MapMode, T)>,
}

impl<T> Default for AnchorSet<T> {
    fn default() -> Self {
        AnchorSet {
            anchors: Vec::new(),
        }
    }
}

impl<T> AnchorSet<T> {
    /// Anchors `v` at `at`, keeping to `bias` and mapped by `mode`.
    pub fn insert(&mut self, at: usize, bias: Bias, mode: MapMode, v: T) {
        let i = self.anchors.partition_point(|(o, ..)| *o <= at);
        self.anchors.insert(i, (at, bias, mode, v));
    }

    /// Maps every anchor through `c`; an anchor whose mapping fails (a tracked deletion) goes.
    pub fn map(&mut self, c: &ChangeDesc) {
        let mut kept: Vec<(usize, Bias, MapMode, T)> = Vec::with_capacity(self.anchors.len());
        for (at, bias, mode, v) in self.anchors.drain(..) {
            if let Some(new) = c.map_pos(at, bias, mode) {
                kept.push((new, bias, mode, v));
            }
        }
        kept.sort_by_key(|(o, ..)| *o);
        self.anchors = kept;
    }

    /// The anchors in offset order.
    pub fn iter(&self) -> impl Iterator<Item = (usize, &T)> {
        self.anchors.iter().map(|(o, _, _, v)| (*o, v))
    }
}
