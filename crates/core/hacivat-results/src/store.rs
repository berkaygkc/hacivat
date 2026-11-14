//! `ResultStore` (Task 7): the published snapshot (one atomic load), the change `watch` and the
//! "Fetch more anyway" cap raise.
//!
//! The store publishes on every append and state change (R-M3-48): its single writer builds a
//! whole new [`StoreSnapshot`], swaps it in, and only then sends `(generation, version)` on the
//! `watch`, so a reader woken by a change always loads a snapshot at least that new. Readers
//! never lock: `snapshot` is one `ArcSwap` load.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use arc_swap::ArcSwap;
use hacivat_values::ResultSchema;
use tokio::sync::watch;

use crate::budget::MemoryBudget;
use crate::snapshot::StoreSnapshot;
use crate::writer::ResultWriter;

/// A result's ID, unique in the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResultId(pub u64);

/// One result set's rows, read through snapshots.
pub struct ResultStore {
    id: ResultId,
    published: ArcSwap<StoreSnapshot>,
    changes: watch::Sender<(u64, u64)>,
    /// "Fetch more anyway": the writer's `admit` reads it (R-M3-33).
    cap_raised: AtomicBool,
}

impl fmt::Debug for ResultStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResultStore")
            .field("id", &self.id)
            .field("cap_raised", &self.cap_raised())
            .finish_non_exhaustive()
    }
}

impl ResultStore {
    /// A store for `schema` on `budget`, and its single writer. `generation` counts re-runs
    /// into the same result tab. The first snapshot is empty and `Streaming`, at version 0.
    pub fn new(
        id: ResultId,
        schema: Arc<ResultSchema>,
        budget: Arc<MemoryBudget>,
        generation: u64,
    ) -> (Arc<ResultStore>, ResultWriter) {
        let first = StoreSnapshot::empty(generation, schema);
        let (changes, _) = watch::channel((generation, first.version));
        let store = Arc::new(ResultStore {
            id,
            published: ArcSwap::from_pointee(first),
            changes,
            cap_raised: AtomicBool::new(false),
        });
        let writer = ResultWriter::new(store.clone(), budget, &store.snapshot());
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

    /// Swaps `snapshot` in, then tells the `watch` (the writer's only way to publish).
    pub(crate) fn publish(&self, snapshot: StoreSnapshot) {
        let key = (snapshot.generation, snapshot.version);
        self.published.store(Arc::new(snapshot));
        self.changes.send_replace(key);
    }
}
