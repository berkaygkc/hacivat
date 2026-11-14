//! `ResultStore` (Task 7): the published snapshot (one atomic load), the change `watch` and the
//! "Fetch more anyway" cap raise.

use std::fmt;
use std::sync::Arc;

use hacivat_values::ResultSchema;

use crate::budget::MemoryBudget;
use crate::snapshot::StoreSnapshot;
use crate::writer::ResultWriter;

/// A result's ID, unique in the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResultId(pub u64);

/// One result set's rows, read through snapshots.
pub struct ResultStore {
    id: ResultId,
}

impl fmt::Debug for ResultStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResultStore")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl ResultStore {
    /// A store for `schema` on `budget`, and its single writer. `generation` counts re-runs
    /// into the same result tab.
    pub fn new(
        id: ResultId,
        schema: Arc<ResultSchema>,
        budget: Arc<MemoryBudget>,
        generation: u64,
    ) -> (Arc<ResultStore>, ResultWriter) {
        let _ = (schema, budget, generation);
        unimplemented!("ResultStore::new ({id:?}): Task 7")
    }

    /// The current snapshot: one atomic load, never a lock.
    pub fn snapshot(&self) -> Arc<StoreSnapshot> {
        unimplemented!("ResultStore::snapshot ({:?}): Task 7", self.id)
    }

    /// `(generation, version)` after every publish.
    pub fn changes(&self) -> tokio::sync::watch::Receiver<(u64, u64)> {
        unimplemented!("ResultStore::changes ({:?}): Task 7", self.id)
    }

    /// "Fetch more anyway": the writer's next `admit` uses the global budget as this result's
    /// cap (R-M3-33).
    pub fn raise_cap_to_global(&self) {
        unimplemented!("ResultStore::raise_cap_to_global ({:?}): Task 7", self.id)
    }
}
