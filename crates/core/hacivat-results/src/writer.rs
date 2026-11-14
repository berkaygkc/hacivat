//! `ResultWriter` (Task 7): the single writer (the session actor) admits, appends and sets the
//! state; every append and state change publishes a new snapshot (R-M3-48).
//!
//! Each publish clones the chunk list and the chunk starts into new `Arc` slices (O(chunks):
//! about 250 `Arc` clones for 1M rows), so a published snapshot never changes and a held one
//! keeps exactly the chunks it showed. Consumers coalesce changes through the store's `watch`.

use std::fmt;
use std::iter;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use hacivat_values::{Chunk, ResultSchema};

use crate::budget::MemoryBudget;
use crate::cursor::{CloseReason, Completeness};
use crate::snapshot::{StoreSnapshot, StoredChunk};
use crate::store::{ResultId, ResultStore};

/// Why a writer must pause before its next chunk (R-M3-33).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PauseReason {
    /// The per-result cap (256 MiB unless raised).
    ResultCap,
    /// The global budget, shared by every result.
    GlobalBudget,
}

/// Whether the next chunk may be appended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Admission {
    Ok,
    Pause(PauseReason),
}

impl Admission {
    fn encode(self) -> u8 {
        match self {
            Admission::Ok => 0,
            Admission::Pause(PauseReason::ResultCap) => 1,
            Admission::Pause(PauseReason::GlobalBudget) => 2,
        }
    }

    fn pause_of(code: u8) -> Option<PauseReason> {
        match code {
            1 => Some(PauseReason::ResultCap),
            2 => Some(PauseReason::GlobalBudget),
            _ => None,
        }
    }
}

/// The one writer of a result.
pub struct ResultWriter {
    id: ResultId,
    store: Arc<ResultStore>,
    budget: Arc<MemoryBudget>,
    schema: Arc<ResultSchema>,
    generation: u64,
    version: u64,
    chunks: Arc<[Arc<StoredChunk>]>,
    starts: Arc<[u64]>,
    rows: u64,
    bytes: u64,
    completeness: Completeness,
    /// The last `admit`'s answer ([`Admission::encode`]); an atomic so `admit` takes `&self`
    /// and the writer stays `Sync`.
    last_admission: AtomicU8,
    /// The drop guard closes a `Streaming` or `Suspended` result; fixtures disarm it.
    armed: bool,
}

impl fmt::Debug for ResultWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResultWriter")
            .field("id", &self.id)
            .field("generation", &self.generation)
            .field("version", &self.version)
            .field("rows", &self.rows)
            .field("bytes", &self.bytes)
            .field("completeness", &self.completeness)
            .finish_non_exhaustive()
    }
}

impl ResultWriter {
    /// The writer of `store`, which has just published `first`.
    pub(crate) fn new(
        store: Arc<ResultStore>,
        budget: Arc<MemoryBudget>,
        first: &StoreSnapshot,
    ) -> ResultWriter {
        ResultWriter {
            id: store.id(),
            store,
            budget,
            schema: first.schema().clone(),
            generation: first.generation(),
            version: first.version(),
            chunks: first.chunks.clone(),
            starts: first.starts.clone(),
            rows: first.row_count(),
            bytes: first.bytes(),
            completeness: *first.completeness(),
            last_admission: AtomicU8::new(Admission::Ok.encode()),
            armed: true,
        }
    }

    /// Drops the writer without the drop guard, leaving the last published state as it is
    /// (`from_rows` fixtures that show a `Streaming` or `Suspended` result).
    #[cfg(feature = "test-support")]
    pub(crate) fn freeze(mut self) {
        self.armed = false;
    }

    pub fn id(&self) -> ResultId {
        self.id
    }

    /// Whether a chunk of about `next_chunk_estimate` bytes fits the budgets: past this
    /// result's cap (256 MiB, or the global budget once raised) it answers
    /// `Pause(ResultCap)`, past the global budget `Pause(GlobalBudget)`. A chunk that lands
    /// exactly on a limit fits. The answer is kept for the next publish (`paused`).
    pub fn admit(&self, next_chunk_estimate: u64) -> Admission {
        let raised = self.store.cap_raised();
        let answer = if !raised
            && self.bytes.saturating_add(next_chunk_estimate) > self.budget.per_result()
        {
            Admission::Pause(PauseReason::ResultCap)
        } else if self.budget.used().saturating_add(next_chunk_estimate) > self.budget.global() {
            Admission::Pause(PauseReason::GlobalBudget)
        } else {
            Admission::Ok
        };
        self.last_admission
            .store(answer.encode(), Ordering::Relaxed);
        answer
    }

    /// Appends `chunk` (charging its bytes to the budget) and publishes. A chunk of no rows
    /// adds nothing, but the call still publishes.
    pub fn append(&mut self, chunk: Chunk) {
        let rows = u64::from(chunk.rows());
        if rows > 0 {
            let stored = Arc::new(StoredChunk::charge(chunk, &self.budget));
            self.bytes += stored.chunk.bytes();
            self.chunks = self
                .chunks
                .iter()
                .cloned()
                .chain(iter::once(stored))
                .collect();
            self.starts = self
                .starts
                .iter()
                .copied()
                .chain(iter::once(self.rows))
                .collect();
            self.rows += rows;
        }
        self.publish();
    }

    /// Sets the completeness and publishes.
    pub fn set_state(&mut self, c: Completeness) {
        self.completeness = c;
        self.publish();
    }

    fn publish(&mut self) {
        self.version += 1;
        let paused = match self.completeness {
            Completeness::Suspended(_) => {
                Admission::pause_of(self.last_admission.load(Ordering::Relaxed))
            }
            _ => None,
        };
        self.store.publish(StoreSnapshot {
            generation: self.generation,
            version: self.version,
            schema: self.schema.clone(),
            chunks: self.chunks.clone(),
            starts: self.starts.clone(),
            row_count: self.rows,
            bytes: self.bytes,
            completeness: self.completeness,
            paused,
        });
    }
}

/// The drop guard (review I1). A writer that goes away while its result is `Streaming` or
/// `Suspended` (an early return in the actor, an actor that exits on a lost connection, a
/// panic that unwinds it) publishes `Closed { reason: Cancelled }` ("fetching was stopped"),
/// so no result claims a fetch or an open cursor that no longer exists. The rows stay
/// browsable and charged. A terminal state the session set first (its own close reason)
/// wins: the guard then publishes nothing. The guard adds no assertion, so a drop during an
/// unwind cannot turn into an abort.
impl Drop for ResultWriter {
    fn drop(&mut self) {
        if self.armed
            && matches!(
                self.completeness,
                Completeness::Streaming | Completeness::Suspended(_)
            )
        {
            self.completeness = Completeness::Closed {
                reason: CloseReason::Cancelled,
            };
            self.publish();
        }
    }
}
