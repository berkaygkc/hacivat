//! Completeness, console cursors and the result source the grid adapter reads (HV-RES-05,
//! HV-RES-07; Task 7).

use std::fmt;
use std::sync::Arc;
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
pub struct StoreSource {
    store: Arc<ResultStore>,
    policy: FetchPolicy,
}

impl fmt::Debug for StoreSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoreSource")
            .field("store", &self.store)
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl StoreSource {
    pub fn new(
        store: Arc<ResultStore>,
        control: Arc<dyn FetchControl>,
        policy: FetchPolicy,
    ) -> Arc<Self> {
        let _ = (store, control, policy);
        unimplemented!("StoreSource::new: Task 7")
    }
}

impl ResultSource for StoreSource {
    fn snapshot(&self) -> Arc<StoreSnapshot> {
        self.store.snapshot()
    }

    fn changes(&self) -> tokio::sync::watch::Receiver<(u64, u64)> {
        self.store.changes()
    }

    fn ensure_rows(&self, upto: u64) {
        let _ = upto;
        unimplemented!("StoreSource::ensure_rows: Task 7")
    }

    fn caps(&self) -> SourceCaps {
        unimplemented!("StoreSource::caps: Task 7")
    }

    fn fetch_all(&self) {
        unimplemented!("StoreSource::fetch_all: Task 7")
    }

    fn close_cursor(&self) {
        unimplemented!("StoreSource::close_cursor: Task 7")
    }

    fn raise_cap(&self) {
        unimplemented!("StoreSource::raise_cap: Task 7")
    }
}
