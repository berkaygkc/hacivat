//! Anchors: positions that follow edits (UI-EDIT-03).
//!
//! An [`AnchorSet`] keeps its anchors sorted by offset, `Before` ahead of `After` at one
//! offset, and maps them all through a change in one forward pass: O(k + s) for k anchors and
//! s sections (each anchor's mapping resumes where the last one stopped). An anchor whose mode
//! tracks a deletion that took it is dropped.

use crate::buffer::Bias;
use crate::change::{ChangeDesc, MapMode};

/// Values anchored at byte offsets, kept sorted by offset (`Before` first at one offset).
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

/// The sort key: offset, then `Before` ahead of `After` (so one forward pass maps them).
fn key(at: usize, bias: Bias) -> (usize, bool) {
    (at, bias == Bias::After)
}

impl<T> AnchorSet<T> {
    /// Anchors `v` at `at`, keeping to `bias` and mapped by `mode`; after the anchors already
    /// there with the same offset and bias.
    pub fn insert(&mut self, at: usize, bias: Bias, mode: MapMode, v: T) {
        let k = key(at, bias);
        let i = self.anchors.partition_point(|(o, b, ..)| key(*o, *b) <= k);
        self.anchors.insert(i, (at, bias, mode, v));
    }

    /// Maps every anchor through `c` in one pass; an anchor whose mapping fails (a tracked
    /// deletion) goes. Anchors that mapping reorders (both sides of a deleted range) are
    /// re-sorted, stably.
    pub fn map(&mut self, c: &ChangeDesc) {
        let mut m = c.mapper();
        let mut sorted = true;
        let mut last = (0, false);
        let mut kept: Vec<(usize, Bias, MapMode, T)> = Vec::with_capacity(self.anchors.len());
        for (at, bias, mode, v) in self.anchors.drain(..) {
            if let Some(new) = m.map(at, bias, mode) {
                let k = key(new, bias);
                sorted &= k >= last;
                last = k;
                kept.push((new, bias, mode, v));
            }
        }
        if !sorted {
            kept.sort_by_key(|(o, b, ..)| key(*o, *b));
        }
        self.anchors = kept;
    }

    /// The anchors in order.
    pub fn iter(&self) -> impl Iterator<Item = (usize, &T)> {
        self.anchors.iter().map(|(o, _, _, v)| (*o, v))
    }

    pub fn len(&self) -> usize {
        self.anchors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.anchors.is_empty()
    }

    /// Keeps the anchors whose value passes `f`.
    pub fn retain(&mut self, mut f: impl FnMut(&T) -> bool) {
        self.anchors.retain(|(.., v)| f(v));
    }
}
