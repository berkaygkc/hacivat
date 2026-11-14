//! The fetch window: how many rows come first, per chunk and ahead of the viewport, and the
//! cursor timings (HV-RES-05, R-M3-16, R-M3-20; Task 7). [`FetchPolicy::rows_to_fetch`] is the
//! one decision `StoreSource::ensure_rows` takes from it.

use std::time::Duration;

/// How a run fetches. `Default` is the spec's numbers; tests inject `idle_cap` (2 s).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FetchPolicy {
    /// Rows of the first chunk (shown before the rest is asked for).
    pub first_rows: u32,
    /// Rows per later chunk.
    pub chunk_rows: u32,
    /// A suspendable result reads ahead this many rows, or `read_ahead_bytes`, then suspends.
    pub read_ahead_rows: u32,
    pub read_ahead_bytes: u64,
    /// `ensure_rows` asks for more this close to the loaded end.
    pub prefetch_margin_rows: u32,
    /// How long a read-write PostgreSQL cursor may stay idle (R-M3-20).
    pub idle_timeout: Duration,
    /// How long a release may drain before it closes the engine's way.
    pub release_drain: Duration,
    /// The most bytes kept of one cell; longer values are truncated.
    pub cell_cap: u32,
    /// The bytes of one chunk.
    pub chunk_bytes: u64,
    /// A chunk this late sends the server cancel (R-M3-16).
    pub overdue: Duration,
    /// Caps every engine's idle bound (tests inject 2 s).
    pub idle_cap: Option<Duration>,
}

impl Default for FetchPolicy {
    /// 500 first rows, 4,096 per chunk, 10,000 rows or 16 MiB ahead, a 1,000-row prefetch
    /// margin, 10 min idle, 250 ms release drain, 1 MiB cells, 4 MiB chunks, 1 s overdue, no
    /// idle cap.
    fn default() -> Self {
        FetchPolicy {
            first_rows: 500,
            chunk_rows: 4_096,
            read_ahead_rows: 10_000,
            read_ahead_bytes: 16 << 20,
            prefetch_margin_rows: 1_000,
            idle_timeout: Duration::from_secs(600),
            release_drain: Duration::from_millis(250),
            cell_cap: 1 << 20,
            chunk_bytes: 4 << 20,
            overdue: Duration::from_secs(1),
            idle_cap: None,
        }
    }
}

/// The most rows one ask (`rows_to_fetch`, hence one `fetch_more` from a source) names:
/// `u32::MAX`, about 4.3 billion. Why this bound:
/// - it never limits a real reader: a viewport plus the margin is a few thousand rows, and
///   even the largest jump a reader can ask for in M3b (go to row, a thumb drag) stays far
///   below it; the memory budget, not the ask, decides how many rows are kept;
/// - a sentinel such as `u64::MAX` can no longer reach the session as a row count;
/// - the session may add up queued asks ("two queued `FetchMore`s coalesce into one fetch of
///   the sum", Task 18): 2^32 asks of this size still fit in a `u64`, so the sum cannot
///   overflow.
pub const MAX_FETCH_ROWS: u64 = u32::MAX as u64;

impl FetchPolicy {
    /// How many rows to ask for when a reader needs row `upto` of `loaded` rows (HV-RES-03
    /// step 3): nothing while `upto` plus the prefetch margin is loaded, else the read-ahead or
    /// the shortfall, whichever is larger, and never more than [`MAX_FETCH_ROWS`].
    pub fn rows_to_fetch(&self, upto: u64, loaded: u64) -> Option<u64> {
        let wanted = upto.saturating_add(u64::from(self.prefetch_margin_rows));
        (wanted > loaded).then(|| {
            (wanted - loaded)
                .max(u64::from(self.read_ahead_rows))
                .min(MAX_FETCH_ROWS)
        })
    }
}
