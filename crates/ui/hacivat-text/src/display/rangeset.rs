//! Decoration range sets: O(log n + k) queries over ranges sorted by start, however long
//! and nested the ranges are (an implicit interval tree over the sorted array).

use std::ops::Range;

/// Values on byte ranges, sorted by start. Ranges may overlap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeSet<T> {
    items: Vec<(Range<usize>, T)>,
    /// The implicit tree: the node of `lo..hi` is `mid = (lo + hi) / 2`; `sub_end[mid]` is the
    /// largest effective end in `items[lo..hi]`.
    sub_end: Vec<usize>,
}

/// What one query did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QueryStats {
    /// Tree nodes visited by the query itself (the same walk `query` runs).
    pub probes: usize,
    /// Entries returned.
    pub hits: usize,
}

impl<T> Default for RangeSet<T> {
    fn default() -> Self {
        RangeSet {
            items: Vec::new(),
            sub_end: Vec::new(),
        }
    }
}

/// An empty range occupies one byte for overlap purposes (so an empty entry at the start of a
/// query counts).
fn eff_end(r: &Range<usize>) -> usize {
    r.end.max(r.start.saturating_add(1))
}

fn build<T>(items: &[(Range<usize>, T)], sub_end: &mut [usize], lo: usize, hi: usize) -> usize {
    if lo >= hi {
        return 0;
    }
    let mid = (lo + hi) / 2;
    let m = eff_end(&items[mid].0)
        .max(build(items, sub_end, lo, mid))
        .max(build(items, sub_end, mid + 1, hi));
    sub_end[mid] = m;
    m
}

impl<T: Clone> RangeSet<T> {
    /// A set from `(range, value)` pairs sorted by range start.
    pub fn from_sorted(v: Vec<(Range<usize>, T)>) -> Self {
        debug_assert!(v.windows(2).all(|w| w[0].0.start <= w[1].0.start));
        let mut sub_end = vec![0; v.len()];
        build(&v, &mut sub_end, 0, v.len());
        RangeSet { items: v, sub_end }
    }

    /// The entries overlapping `r` (an empty entry at `r`'s start counts), in order.
    pub fn query(&self, r: Range<usize>) -> impl Iterator<Item = (Range<usize>, &T)> {
        self.walk(&r, &mut 0).into_iter()
    }

    /// `query` collected, with the work the walk did.
    pub fn query_with_stats(&self, r: Range<usize>) -> (Vec<(Range<usize>, &T)>, QueryStats) {
        let mut probes = 0;
        let hits = self.walk(&r, &mut probes);
        let stats = QueryStats {
            probes,
            hits: hits.len(),
        };
        (hits, stats)
    }

    fn walk(&self, r: &Range<usize>, probes: &mut usize) -> Vec<(Range<usize>, &T)> {
        let mut out = Vec::new();
        self.visit(r, eff_end(r), 0, self.items.len(), probes, &mut out);
        out
    }

    fn visit<'a>(
        &'a self,
        r: &Range<usize>,
        q_end: usize,
        lo: usize,
        hi: usize,
        probes: &mut usize,
        out: &mut Vec<(Range<usize>, &'a T)>,
    ) {
        if lo >= hi {
            return;
        }
        let mid = (lo + hi) / 2;
        *probes += 1;
        if self.sub_end[mid] <= r.start {
            return; // nothing in this subtree reaches the query
        }
        self.visit(r, q_end, lo, mid, probes, out);
        let (range, v) = &self.items[mid];
        if range.start >= q_end {
            return; // this and every later start is past the query
        }
        if r.start < eff_end(range) {
            out.push((range.clone(), v));
        }
        self.visit(r, q_end, mid + 1, hi, probes, out);
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}
