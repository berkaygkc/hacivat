//! Decoration range sets: O(log n + k) queries over ranges sorted by start.

use std::ops::Range;

/// Values on byte ranges, sorted by start. Ranges may overlap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeSet<T> {
    items: Vec<(Range<usize>, T)>,
    /// `reach[i]` = the largest effective end among `items[..=i]`; non-decreasing, so the
    /// first item that can overlap a query is found by binary search.
    reach: Vec<usize>,
}

/// What one query examined (the test counter for the O(log n + k) bound).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QueryStats {
    /// Binary-search steps plus entries visited.
    pub probes: usize,
    /// Entries returned.
    pub hits: usize,
}

impl<T> Default for RangeSet<T> {
    fn default() -> Self {
        RangeSet {
            items: Vec::new(),
            reach: Vec::new(),
        }
    }
}

/// An empty range occupies one byte for overlap purposes (so an empty entry at the start of a
/// query counts).
fn eff_end(r: &Range<usize>) -> usize {
    r.end.max(r.start.saturating_add(1))
}

impl<T: Clone> RangeSet<T> {
    /// A set from `(range, value)` pairs sorted by range start.
    pub fn from_sorted(v: Vec<(Range<usize>, T)>) -> Self {
        debug_assert!(v.windows(2).all(|w| w[0].0.start <= w[1].0.start));
        let mut reach = Vec::with_capacity(v.len());
        let mut m = 0;
        for (r, _) in &v {
            m = m.max(eff_end(r));
            reach.push(m);
        }
        RangeSet { items: v, reach }
    }

    /// The entries overlapping `r` (an empty entry at `r`'s start counts), in order.
    pub fn query(&self, r: Range<usize>) -> impl Iterator<Item = (Range<usize>, &T)> {
        self.run(r).0
    }

    /// `query` collected, with what it examined.
    pub fn query_with_stats(&self, r: Range<usize>) -> (Vec<(Range<usize>, &T)>, QueryStats) {
        let (it, first, steps) = self.run(r.clone());
        let q_end = eff_end(&r);
        let hits: Vec<_> = it.collect();
        let visited = self.items[first..]
            .iter()
            .take_while(|(range, _)| range.start < q_end)
            .count();
        let stats = QueryStats {
            probes: steps + visited,
            hits: hits.len(),
        };
        (hits, stats)
    }

    fn run(&self, r: Range<usize>) -> (impl Iterator<Item = (Range<usize>, &T)>, usize, usize) {
        let first = self.reach.partition_point(|&m| m <= r.start);
        let steps = (usize::BITS - self.reach.len().leading_zeros()) as usize;
        let q_end = eff_end(&r);
        let it = self.items[first..]
            .iter()
            .take_while(move |(range, _)| range.start < q_end)
            .filter(move |(range, _)| r.start < eff_end(range))
            .map(|(range, v)| (range.clone(), v));
        (it, first, steps)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}
