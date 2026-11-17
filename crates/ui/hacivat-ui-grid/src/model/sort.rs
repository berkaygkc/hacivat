//! Local sort keys (HV-GRID-07; M3b Task 36). The grid only names what to sort by
//! (`GridEvent::SortRequested`); the app sorts off the UI thread with the column's collation
//! and publishes the order as `ViewMap::Permuted`.
//!
//! ## m3b-t01b-seams: the types (Task 1b). Task 36 fills the module.
//!
//! ## m3c-t36: [`stable_order`], the sort the app's job runs on the background executor: a
//! stable merge sort of row numbers under the job's comparator (ties keep model order),
//! checking for cancellation every [`CANCEL_EVERY`] comparisons, split over up to
//! [`MAX_THREADS`] threads and merged above [`PARALLEL_ABOVE`] rows. It knows nothing of
//! values: the comparator does (`hacivat-values::valfmt::compare` in the app).

use std::cmp::Ordering;

use smallvec::SmallVec;

use crate::source::ColId;

/// One sort key: a column and its direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SortKey {
    pub col: ColId,
    pub desc: bool,
}

/// The keys of a sort, most significant first (empty: no sort, source order).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct SortSpec {
    pub keys: SmallVec<[SortKey; 3]>,
}

// ## m3c-t36
impl SortSpec {
    /// A sort by `col` alone.
    pub fn by(col: ColId, desc: bool) -> Self {
        let mut keys = SmallVec::new();
        keys.push(SortKey { col, desc });
        SortSpec { keys }
    }

    /// No keys: the source order.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// `col`'s level (0: the most significant key) and its key, when it is sorted.
    pub fn level_of(&self, col: ColId) -> Option<(usize, SortKey)> {
        self.keys
            .iter()
            .position(|k| k.col == col)
            .map(|ix| (ix, self.keys[ix]))
    }
}

/// Comparisons between two looks at the cancellation flag (the plan's 64k).
pub const CANCEL_EVERY: u64 = 1 << 16;

/// Above this many rows the sort is split over several threads, then merged.
pub const PARALLEL_ABOVE: u64 = 1_000_000;

/// The most threads one sort uses.
pub const MAX_THREADS: usize = 4;

/// Runs shorter than this are sorted by insertion before the merges.
const RUN: usize = 32;

/// The sort stopped: its cancellation flag was raised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cancelled;

/// Counts comparisons and looks at the cancellation flag every [`CANCEL_EVERY`] of them.
struct Checker<'a, K: ?Sized> {
    count: u64,
    cancelled: &'a K,
}

impl<K: Fn() -> bool + ?Sized> Checker<'_, K> {
    #[inline]
    fn tick(&mut self) -> Result<(), Cancelled> {
        self.count += 1;
        if self.count.is_multiple_of(CANCEL_EVERY) && (self.cancelled)() {
            return Err(Cancelled);
        }
        Ok(())
    }
}

/// The rows `0..rows` in the order `cmp` gives them (a stable sort: rows `cmp` finds equal keep
/// their order), or [`Cancelled`] once `cancelled` answers `true` (looked at every
/// [`CANCEL_EVERY`] comparisons, and before each merge pass). Above [`PARALLEL_ABOVE`] rows the
/// rows are split into up to [`MAX_THREADS`] runs sorted on their own threads (scoped, so they
/// borrow `cmp`), then merged pairwise, left run first.
///
/// Call it off the UI thread: it is O(n log n) comparisons.
pub fn stable_order<C, K>(rows: u64, cmp: &C, cancelled: &K) -> Result<Vec<u64>, Cancelled>
where
    C: Fn(u64, u64) -> Ordering + Sync + ?Sized,
    K: Fn() -> bool + Sync + ?Sized,
{
    let threads = if rows > PARALLEL_ABOVE {
        std::thread::available_parallelism()
            .map_or(1, usize::from)
            .clamp(1, MAX_THREADS)
    } else {
        1
    };
    stable_order_on(rows, threads, cmp, cancelled)
}

/// [`stable_order`] on `threads` threads (1: the calling thread only).
pub fn stable_order_on<C, K>(
    rows: u64,
    threads: usize,
    cmp: &C,
    cancelled: &K,
) -> Result<Vec<u64>, Cancelled>
where
    C: Fn(u64, u64) -> Ordering + Sync + ?Sized,
    K: Fn() -> bool + Sync + ?Sized,
{
    let all: Vec<u64> = (0..rows).collect();
    if threads <= 1 || rows < 2 * RUN as u64 {
        return merge_sort(all, cmp, cancelled);
    }
    let per = all.len().div_ceil(threads);
    let parts: Vec<Vec<u64>> = all.chunks(per).map(<[u64]>::to_vec).collect();
    let mut runs = std::thread::scope(|s| {
        let handles: Vec<_> = parts
            .into_iter()
            .map(|part| s.spawn(move || merge_sort(part, cmp, cancelled)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or(Err(Cancelled)))
            .collect::<Result<Vec<_>, _>>()
    })?;
    // Merge neighbours pairwise (left first: stability), each level on its own threads.
    while runs.len() > 1 {
        if cancelled() {
            return Err(Cancelled);
        }
        let mut pairs = Vec::with_capacity(runs.len().div_ceil(2));
        let mut it = runs.into_iter();
        while let Some(left) = it.next() {
            pairs.push((left, it.next()));
        }
        runs = std::thread::scope(|s| {
            let handles: Vec<_> = pairs
                .into_iter()
                .map(|(left, right)| {
                    s.spawn(move || match right {
                        None => Ok(left),
                        Some(right) => {
                            let mut out = vec![0; left.len() + right.len()];
                            let mut check = Checker {
                                count: 0,
                                cancelled,
                            };
                            merge(&left, &right, &mut out, cmp, &mut check)?;
                            Ok(out)
                        }
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or(Err(Cancelled)))
                .collect::<Result<Vec<_>, _>>()
        })?;
    }
    Ok(runs.pop().unwrap_or_default())
}

/// A bottom-up stable merge sort of `a`: runs of [`RUN`] by insertion, then merge passes
/// between `a` and one buffer.
fn merge_sort<C, K>(mut a: Vec<u64>, cmp: &C, cancelled: &K) -> Result<Vec<u64>, Cancelled>
where
    C: Fn(u64, u64) -> Ordering + ?Sized,
    K: Fn() -> bool + ?Sized,
{
    let mut check = Checker {
        count: 0,
        cancelled,
    };
    let n = a.len();
    for run in a.chunks_mut(RUN) {
        insertion_sort(run, cmp, &mut check)?;
    }
    if n <= RUN {
        return Ok(a);
    }
    let mut b = vec![0u64; n];
    let mut width = RUN;
    while width < n {
        if cancelled() {
            return Err(Cancelled);
        }
        let mut start = 0;
        while start < n {
            let mid = (start + width).min(n);
            let end = (start + 2 * width).min(n);
            merge(
                &a[start..mid],
                &a[mid..end],
                &mut b[start..end],
                cmp,
                &mut check,
            )?;
            start = end;
        }
        std::mem::swap(&mut a, &mut b);
        width *= 2;
    }
    Ok(a)
}

/// Sorts `v` by insertion, stably.
fn insertion_sort<C, K>(v: &mut [u64], cmp: &C, check: &mut Checker<'_, K>) -> Result<(), Cancelled>
where
    C: Fn(u64, u64) -> Ordering + ?Sized,
    K: Fn() -> bool + ?Sized,
{
    for i in 1..v.len() {
        let x = v[i];
        let mut j = i;
        while j > 0 {
            check.tick()?;
            if cmp(v[j - 1], x) != Ordering::Greater {
                break;
            }
            v[j] = v[j - 1];
            j -= 1;
        }
        v[j] = x;
    }
    Ok(())
}

/// Merges the sorted `left` and `right` into `out`, stably (a tie takes `left`'s row first).
fn merge<C, K>(
    left: &[u64],
    right: &[u64],
    out: &mut [u64],
    cmp: &C,
    check: &mut Checker<'_, K>,
) -> Result<(), Cancelled>
where
    C: Fn(u64, u64) -> Ordering + ?Sized,
    K: Fn() -> bool + ?Sized,
{
    // Already in order (common for presorted data): one comparison, two copies.
    if let (Some(&l), Some(&r)) = (left.last(), right.first()) {
        check.tick()?;
        if cmp(l, r) != Ordering::Greater {
            out[..left.len()].copy_from_slice(left);
            out[left.len()..].copy_from_slice(right);
            return Ok(());
        }
    }
    let (mut i, mut j, mut k) = (0, 0, 0);
    while i < left.len() && j < right.len() {
        check.tick()?;
        if cmp(right[j], left[i]) == Ordering::Less {
            out[k] = right[j];
            j += 1;
        } else {
            out[k] = left[i];
            i += 1;
        }
        k += 1;
    }
    out[k..k + left.len() - i].copy_from_slice(&left[i..]);
    k += left.len() - i;
    out[k..].copy_from_slice(&right[j..]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

    use super::*;

    /// A pseudo-random key per row (a fixed LCG), with many ties.
    fn keys(n: u64, distinct: u64) -> Vec<u64> {
        let mut x = 0x2545_f491_4f6c_dd1du64;
        (0..n)
            .map(|_| {
                x = x
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (x >> 33) % distinct
            })
            .collect()
    }

    fn expected(keys: &[u64]) -> Vec<u64> {
        let mut rows: Vec<u64> = (0..keys.len() as u64).collect();
        rows.sort_by_key(|r| keys[*r as usize]);
        rows
    }

    #[test]
    fn sorts_stably_like_the_standard_sort() {
        for n in [0u64, 1, 2, 31, 32, 33, 100, 1000, 4097] {
            let k = keys(n, 17);
            let cmp = |a: u64, b: u64| k[a as usize].cmp(&k[b as usize]);
            assert_eq!(stable_order(n, &cmp, &|| false), Ok(expected(&k)), "{n}");
        }
    }

    #[test]
    fn several_threads_give_the_same_stable_order() {
        let n = 50_000;
        let k = keys(n, 97);
        let cmp = |a: u64, b: u64| k[a as usize].cmp(&k[b as usize]);
        for threads in [2, 3, 4] {
            assert_eq!(
                stable_order_on(n, threads, &cmp, &|| false),
                Ok(expected(&k)),
                "{threads}"
            );
        }
    }

    #[test]
    fn a_raised_flag_stops_the_sort_within_one_check_interval() {
        let n = 200_000;
        let k = keys(n, 1_000_000);
        let calls = AtomicU64::new(0);
        let cmp = |a: u64, b: u64| {
            calls.fetch_add(1, AtomicOrdering::Relaxed);
            k[a as usize].cmp(&k[b as usize])
        };
        // Raised from the start: the first look stops it.
        assert_eq!(stable_order(n, &cmp, &|| true), Err(Cancelled));
        assert!(calls.load(AtomicOrdering::Relaxed) <= CANCEL_EVERY);
        // On several threads too.
        assert_eq!(stable_order_on(n, 4, &cmp, &|| true), Err(Cancelled));
    }

    #[test]
    fn spec_levels() {
        let spec = SortSpec {
            keys: [
                SortKey {
                    col: ColId(3),
                    desc: false,
                },
                SortKey {
                    col: ColId(1),
                    desc: true,
                },
            ]
            .into_iter()
            .collect(),
        };
        assert_eq!(
            spec.level_of(ColId(1)).map(|(l, k)| (l, k.desc)),
            Some((1, true))
        );
        assert_eq!(spec.level_of(ColId(2)), None);
        assert_eq!(SortSpec::by(ColId(2), true).keys.len(), 1);
        assert!(SortSpec::default().is_empty());
    }
}
