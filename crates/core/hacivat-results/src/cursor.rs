//! Completeness, console cursors and the result source the grid adapter reads (HV-RES-05,
//! HV-RES-07; Task 7).

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::snapshot::StoreSnapshot;
use crate::store::{ResultId, ResultStore};
use crate::window::FetchPolicy;

/// Why a cursor (or a stream) ended before its result completed. Rows already fetched stay
/// browsable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CloseReason {
    /// Another request needed the session (R-M3-23: Hacivat's own transaction rolls back).
    SessionNeeded,
    IdleTimeout,
    TabClosed,
    /// The user transaction that owned the cursor ended.
    TxEnded,
    UserClosed,
    MemoryLimit,
    ConnectionLost,
    ServerTerminated,
    Cancelled,
    /// The read-only guard found a write (the connection was discarded).
    EnforcementGap,
}

/// Why a result kept fewer rows than the server sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TruncReason {
    /// Rows past the per-result cap were counted, not kept.
    ResultCap,
    /// Result sets after the first were drained and counted (R-M3-25).
    FollowingResultSets,
}

/// An open cursor's bookkeeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CursorInfo {
    pub opened_at: Instant,
    pub last_fetch: Instant,
    /// The cursor holds a transaction (Hacivat's own, or the read-only guard's).
    pub holds_tx: bool,
    pub idle_bound: Duration,
}

/// Where a result is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Completeness {
    Streaming,
    /// A cursor is open; more rows come on `fetch_more`.
    Suspended(CursorInfo),
    Complete,
    Closed {
        reason: CloseReason,
    },
    Cancelled,
    Truncated(TruncReason),
    /// The rows were given back (a closed tab).
    Released,
}

/// What a result source sends its session (implemented for `SessionHandle`; Task 1b).
pub trait FetchControl: Send + Sync + 'static {
    fn fetch_more(&self, result: ResultId, rows: u64);
    fn close(&self, result: ResultId, reason: CloseReason);
    fn fetch_all(&self, result: ResultId);
}

/// What a result offers the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SourceCaps {
    pub has_cursor: bool,
    pub editable: bool,
}

/// A result the grid adapter reads: snapshots, changes, and cursor commands. `ensure_rows`
/// coalesces: one `fetch_more` per snapshot version while suspended, nothing otherwise.
pub trait ResultSource: Send + Sync + 'static {
    fn snapshot(&self) -> Arc<StoreSnapshot>;
    fn changes(&self) -> tokio::sync::watch::Receiver<(u64, u64)>;
    fn ensure_rows(&self, upto: u64);
    fn caps(&self) -> SourceCaps;
    fn fetch_all(&self);
    fn close_cursor(&self);
    /// "Fetch more anyway": the store's cap rises to the global budget, then `fetch_all`.
    fn raise_cap(&self);
}

/// The [`ResultSource`] over a store and its session (Task 7).
///
/// `ensure_rows` coalesces without a timer: while the result is `Suspended` and the reader
/// needs rows within the prefetch margin of the loaded end, it sends **one** `fetch_more` for
/// the snapshot version it saw, then nothing until a later version (the session's reply moves
/// the version: rows, a state change, or both).
pub struct StoreSource {
    store: Arc<ResultStore>,
    control: Arc<dyn FetchControl>,
    policy: FetchPolicy,
    /// One more than the snapshot version of the last `fetch_more` (0: none yet). Only grows,
    /// so a caller holding an older snapshot can never ask again.
    asked_after: AtomicU64,
}

impl fmt::Debug for StoreSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoreSource")
            .field("store", &self.store)
            .field("policy", &self.policy)
            .field("asked_after", &self.asked_after.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl StoreSource {
    pub fn new(
        store: Arc<ResultStore>,
        control: Arc<dyn FetchControl>,
        policy: FetchPolicy,
    ) -> Arc<Self> {
        Arc::new(StoreSource {
            store,
            control,
            policy,
            asked_after: AtomicU64::new(0),
        })
    }
}

impl ResultSource for StoreSource {
    fn snapshot(&self) -> Arc<StoreSnapshot> {
        self.store.snapshot()
    }

    fn changes(&self) -> tokio::sync::watch::Receiver<(u64, u64)> {
        self.store.changes()
    }

    /// One `fetch_more` per snapshot version while `Suspended`; nothing while `Streaming`,
    /// `Complete`, `Closed`, `Cancelled`, `Truncated` or `Released`, and nothing while the
    /// result is paused at a memory limit (the banner's "Fetch more anyway" is the way on,
    /// R-M3-33).
    fn ensure_rows(&self, upto: u64) {
        let snap = self.store.snapshot();
        if !matches!(snap.completeness(), Completeness::Suspended(_)) || snap.paused().is_some() {
            return;
        }
        let Some(rows) = self.policy.rows_to_fetch(upto, snap.row_count()) else {
            return;
        };
        let mark = snap.version().saturating_add(1);
        if self.asked_after.fetch_max(mark, Ordering::AcqRel) >= mark {
            return;
        }
        self.control.fetch_more(self.store.id(), rows);
    }

    /// A cursor while the result is `Suspended`; never editable in M3a.
    fn caps(&self) -> SourceCaps {
        SourceCaps {
            has_cursor: matches!(
                self.store.snapshot().completeness(),
                Completeness::Suspended(_)
            ),
            editable: false,
        }
    }

    fn fetch_all(&self) {
        self.control.fetch_all(self.store.id());
    }

    fn close_cursor(&self) {
        self.control.close(self.store.id(), CloseReason::UserClosed);
    }

    /// The cap rises first, so the session's `fetch_all` is admitted past it.
    fn raise_cap(&self) {
        self.store.raise_cap_to_global();
        self.control.fetch_all(self.store.id());
    }
}
