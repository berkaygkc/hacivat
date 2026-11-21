//! `ResultStore` (Task 7): the published snapshot (one atomic load), the change `watch` and the
//! "Fetch more anyway" cap raise.
//!
//! The store publishes on every append and state change (R-M3-48): its single writer builds a
//! whole new [`StoreSnapshot`], swaps it in, and only then sends `(generation, version)` on the
//! `watch`, so a reader woken by a change always loads a snapshot at least that new. Readers
//! never lock: `snapshot` is one `ArcSwap` load.
//!
//! Since M3b Task 8:
//! - **The tail.** The rows and state the writer builds snapshots from live here, behind a
//!   mutex the writer takes for each append or state change, so eviction (the one other
//!   publisher, `MemoryBudget::evict_for`) can cut a background result to its first chunk and
//!   publish `Closed { reason: Evicted }` without a second copy of the chunk list keeping the
//!   rows alive. Both publish while holding it, so versions only grow. Readers still never
//!   lock.
//! - **The ask ledger.** Every [`StoreSource`](crate::StoreSource) over this store asks for
//!   rows through one ledger, so sibling sources never ask twice for the same rows (ledger 109
//!   M4).
//! - **Visibility.** The store registers itself with its budget's
//!   [`EvictionRegistry`] when it is made; [`ResultStore::set_visible`] tells it whether the
//!   result's tab is on screen.

// ## m3b-t08-store (module docs above, the tail, the ask ledger, eviction)
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use arc_swap::ArcSwap;
use hacivat_values::ResultSchema;
use tokio::sync::watch;

use crate::budget::MemoryBudget;
use crate::cursor::{CloseReason, Completeness};
use crate::evict::{Eviction, EvictionRegistry};
use crate::snapshot::{StoreSnapshot, StoredChunk};
use crate::window::FetchPolicy;
use crate::writer::{PauseReason, ResultWriter};

/// A result's ID, unique in the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResultId(pub u64);

// ## m3b-t08-store
/// The rows and state a result's snapshots are built from (the module docs: the writer and
/// eviction share it).
pub(crate) struct Tail {
    pub(crate) generation: u64,
    pub(crate) schema: Arc<ResultSchema>,
    /// The last published version.
    pub(crate) version: u64,
    pub(crate) chunks: Arc<[Arc<StoredChunk>]>,
    pub(crate) starts: Arc<[u64]>,
    pub(crate) rows: u64,
    pub(crate) bytes: u64,
    pub(crate) completeness: Completeness,
    /// `Some(the state it had ended in)` once eviction cut the result to its first chunk: the
    /// writer's later appends and states are dropped, so an evicted result never reads as
    /// anything else.
    pub(crate) before_eviction: Option<Completeness>,
}

impl Tail {
    /// Whether eviction has cut this result.
    pub(crate) fn evicted(&self) -> bool {
        self.before_eviction.is_some()
    }

    /// The next snapshot: one version on, the tail's chunks, `paused` as given.
    pub(crate) fn next_snapshot(&mut self, paused: Option<PauseReason>) -> StoreSnapshot {
        self.version += 1;
        StoreSnapshot {
            generation: self.generation,
            version: self.version,
            schema: self.schema.clone(),
            chunks: self.chunks.clone(),
            starts: self.starts.clone(),
            row_count: self.rows,
            bytes: self.bytes,
            completeness: self.completeness,
            paused,
            before_eviction: self.before_eviction,
        }
    }
}

/// What the sources of a store have asked the session for (ledger 109 M4).
#[derive(Debug, Default)]
struct Asked {
    /// The snapshot version the asks were made at.
    version: u64,
    /// The rows loaded at that version plus every row asked since.
    through: u64,
}

/// Whether a result in `c` may be evicted: its run has ended (no writer appends to it, no
/// cursor is open on the server) and it still holds rows (not given back, not evicted).
fn evictable(c: &Completeness) -> bool {
    match c {
        Completeness::Complete | Completeness::Cancelled | Completeness::Truncated(_) => true,
        Completeness::Closed { reason } => *reason != CloseReason::Evicted,
        Completeness::Streaming | Completeness::Suspended(_) | Completeness::Released => false,
    }
}

/// One result set's rows, read through snapshots.
pub struct ResultStore {
    id: ResultId,
    published: ArcSwap<StoreSnapshot>,
    changes: watch::Sender<(u64, u64)>,
    /// "Fetch more anyway": the writer's `admit` reads it (R-M3-33).
    cap_raised: AtomicBool,
    // ## m3b-t08-store
    tail: Mutex<Tail>,
    asked: Mutex<Asked>,
    registry: Arc<EvictionRegistry>,
}

impl fmt::Debug for ResultStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResultStore")
            .field("id", &self.id)
            .field("cap_raised", &self.cap_raised())
            .finish_non_exhaustive()
    }
}

/// Locks `m`; a panic elsewhere while it was held leaves data that is still whole (every
/// update is a plain field store), so a poisoned lock is used as it is.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ResultStore {
    /// A store for `schema` on `budget`, and its single writer. `generation` counts re-runs
    /// into the same result tab. The first snapshot is empty and `Streaming`, at version 0.
    /// The store joins the budget's eviction registry, visible until
    /// [`set_visible`](Self::set_visible) says otherwise.
    pub fn new(
        id: ResultId,
        schema: Arc<ResultSchema>,
        budget: Arc<MemoryBudget>,
        generation: u64,
    ) -> (Arc<ResultStore>, ResultWriter) {
        let first = StoreSnapshot::empty(generation, schema.clone());
        let (changes, _) = watch::channel((generation, first.version));
        let tail = Tail {
            generation,
            schema,
            version: first.version,
            chunks: first.chunks.clone(),
            starts: first.starts.clone(),
            rows: 0,
            bytes: 0,
            completeness: first.completeness,
            before_eviction: None,
        };
        let registry = budget.registry().clone();
        let store = Arc::new(ResultStore {
            id,
            published: ArcSwap::from_pointee(first),
            changes,
            cap_raised: AtomicBool::new(false),
            tail: Mutex::new(tail),
            asked: Mutex::new(Asked::default()),
            registry,
        });
        store.registry.register(&store);
        let writer = ResultWriter::new(store.clone(), budget);
        (store, writer)
    }

    pub fn id(&self) -> ResultId {
        self.id
    }

    /// Whether "Fetch more anyway" raised this result's cap.
    pub fn cap_raised(&self) -> bool {
        self.cap_raised.load(Ordering::Acquire)
    }

    /// The current snapshot: one atomic load, never a lock.
    pub fn snapshot(&self) -> Arc<StoreSnapshot> {
        self.published.load_full()
    }

    /// `(generation, version)` after every publish.
    ///
    /// **Subscribe before the first [`snapshot`](Self::snapshot).** A new receiver counts the
    /// current value as seen, so a consumer that loads a snapshot first and subscribes second
    /// misses any publish in between, and then sleeps on a stale snapshot; for a result that
    /// has finished, no later publish ever wakes it.
    pub fn changes(&self) -> watch::Receiver<(u64, u64)> {
        self.changes.subscribe()
    }

    /// "Fetch more anyway": the writer's next `admit` uses the global budget as this result's
    /// cap (R-M3-33).
    pub fn raise_cap_to_global(&self) {
        self.cap_raised.store(true, Ordering::Release);
    }

    // ## m3b-t08-store
    /// The result's tab is on screen (`true`) or in the background (`false`). A visible
    /// result is never evicted (R-M3-68); a result counts as visible until this says
    /// otherwise, so a console that never reports visibility loses nothing.
    pub fn set_visible(&self, visible: bool) {
        self.registry.set_visible(self.id, visible);
    }

    /// The tail, locked (the writer's and eviction's only way in).
    pub(crate) fn tail(&self) -> MutexGuard<'_, Tail> {
        lock(&self.tail)
    }

    /// Swaps in the tail's next snapshot, then tells the `watch`. Called with the tail locked,
    /// so two publishers never interleave.
    pub(crate) fn publish(&self, tail: &mut Tail, paused: Option<PauseReason>) {
        let snapshot = tail.next_snapshot(paused);
        let key = (snapshot.generation, snapshot.version);
        self.published.store(Arc::new(snapshot));
        self.changes.send_replace(key);
    }

    /// Cuts a finished result to its first chunk and publishes `Closed { reason: Evicted }`
    /// (R-M3-68), keeping the state it had ended in (`before_eviction`). Answers what
    /// happened and the cut chunks nobody else holds: the caller drops those (releasing their
    /// bytes) once it has let go of the registry lock. A chunk a held snapshot still shows is
    /// released when that snapshot drops (P-M3b-29). A result still being written, one with a
    /// cursor open, one already evicted or given back, and one with nothing past its first
    /// chunk are left as they are (`None`).
    pub(crate) fn evict(&self) -> Option<(Eviction, Vec<StoredChunk>)> {
        let (eviction, cut) = {
            let mut tail = self.tail();
            if tail.evicted() || !evictable(&tail.completeness) || tail.chunks.len() < 2 {
                return None;
            }
            let first = tail.chunks[0].clone();
            let cut = tail.chunks[1..].to_vec();
            let previous = tail.completeness;
            tail.rows = tail.starts[1];
            tail.bytes = first.chunk.bytes();
            tail.chunks = Arc::from(vec![first]);
            tail.starts = Arc::from(vec![0]);
            tail.completeness = Completeness::Closed {
                reason: CloseReason::Evicted,
            };
            tail.before_eviction = Some(previous);
            self.publish(&mut tail, None);
            let eviction = Eviction {
                result: self.id,
                kept_rows: tail.rows,
                previous,
            };
            (eviction, cut)
        };
        // Whoever drops a chunk last releases its bytes: the caller, unless a snapshot holds it.
        Some((
            eviction,
            cut.into_iter().filter_map(Arc::into_inner).collect(),
        ))
    }

    /// How many rows to ask the session for so a reader of `snap` sees row `upto` (ledger 109
    /// M4): per snapshot version, every source's asks add up to what the farthest reader
    /// needs, and a need already asked for asks nothing. `None` for a snapshot older than one
    /// already asked at (a later answer has moved on).
    pub(crate) fn ask(&self, snap: &StoreSnapshot, upto: u64, policy: &FetchPolicy) -> Option<u64> {
        let mut asked = lock(&self.asked);
        let version = snap.version();
        if version < asked.version {
            return None;
        }
        if version > asked.version {
            *asked = Asked {
                version,
                through: snap.row_count(),
            };
        }
        let rows = policy.rows_to_fetch(upto, asked.through)?;
        asked.through = asked.through.saturating_add(rows);
        Some(rows)
    }
}
