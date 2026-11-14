//! Decoration range sets (Task 11: O(log n + k) queries). Task 1b: a sorted vector scanned in
//! full.

use std::ops::Range;

/// Values on byte ranges, sorted by start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeSet<T> {
    items: Vec<(Range<usize>, T)>,
}

impl<T> Default for RangeSet<T> {
    fn default() -> Self {
        RangeSet { items: Vec::new() }
    }
}

impl<T: Clone> RangeSet<T> {
    /// A set from `(range, value)` pairs sorted by range start.
    pub fn from_sorted(v: Vec<(Range<usize>, T)>) -> Self {
        RangeSet { items: v }
    }

    /// The entries overlapping `r` (an empty entry at `r`'s start counts), in order.
    pub fn query(&self, r: Range<usize>) -> impl Iterator<Item = (Range<usize>, &T)> {
        self.items
            .iter()
            .filter(move |(range, _)| {
                range.start < r.end.max(r.start + 1) && r.start < range.end.max(range.start + 1)
            })
            .map(|(range, v)| (range.clone(), v))
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}
