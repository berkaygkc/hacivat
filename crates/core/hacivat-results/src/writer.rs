//! `ResultWriter` (Task 7): the single writer (the session actor) admits, appends and sets the
//! state; every append and state change publishes a new snapshot (R-M3-48).
//!
//! Each publish clones the chunk list and the chunk starts into new `Arc` slices (O(chunks):
//! about 250 `Arc` clones for 1M rows), so a published snapshot never changes and a held one
//! keeps exactly the chunks it showed. Consumers coalesce changes through the store's `watch`.
//!
//! Since M3b Task 8, the rows the writer builds from live in the store's tail (`store.rs`), shared with
//! eviction: `admit` evicts background results before it answers `Pause(GlobalBudget)`
//! (R-M3-68, P-M3b-29), and once eviction has cut this writer's result, its later appends and
//! states are dropped (an evicted result never reads as anything else). Eviction takes only
//! results whose run has ended, so a live run never meets this.

// ## m3b-t08-store (module docs above, the tail, eviction in `admit`)
use std::fmt;
use std::iter;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use hacivat_values::Chunk;

use crate::budget::MemoryBudget;
use crate::cursor::{CloseReason, Completeness};
use crate::snapshot::StoredChunk;
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
    /// The last `admit`'s answer ([`Admission::encode`]); an atomic so `admit` takes `&self`
    /// and the writer stays `Sync`.
    last_admission: AtomicU8,
    /// The drop guard closes a `Streaming` or `Suspended` result; fixtures disarm it.
    armed: bool,
}

impl fmt::Debug for ResultWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tail = self.store.tail();
        f.debug_struct("ResultWriter")
            .field("id", &self.id)
            .field("generation", &tail.generation)
            .field("version", &tail.version)
            .field("rows", &tail.rows)
            .field("bytes", &tail.bytes)
            .field("completeness", &tail.completeness)
            .finish_non_exhaustive()
    }
}

impl ResultWriter {
    /// The writer of `store`, which has just published its first snapshot.
    pub(crate) fn new(store: Arc<ResultStore>, budget: Arc<MemoryBudget>) -> ResultWriter {
        ResultWriter {
            id: store.id(),
            store,
            budget,
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
    ///
    /// Past the global budget it first asks the budget to evict background results until
    /// the chunk fits (R-M3-68), and pauses only when that did not make room. The budget
    /// re-checks the room under its eviction lock, so two runs over the budget at once evict
    /// only what both need together (review M1).
    pub fn admit(&self, next_chunk_estimate: u64) -> Admission {
        let raised = self.store.cap_raised();
        let bytes = self.store.tail().bytes;
        let answer =
            if !raised && bytes.saturating_add(next_chunk_estimate) > self.budget.per_result() {
                Admission::Pause(PauseReason::ResultCap)
            } else if self.budget.over_global(next_chunk_estimate) {
                // ## m3b-t08-store
                self.budget.evict_to_fit(next_chunk_estimate);
                if self.budget.over_global(next_chunk_estimate) {
                    Admission::Pause(PauseReason::GlobalBudget)
                } else {
                    Admission::Ok
                }
            } else {
                Admission::Ok
            };
        self.last_admission
            .store(answer.encode(), Ordering::Relaxed);
        answer
    }

    /// Appends `chunk` (charging its bytes to the budget) and publishes. A chunk of no rows
    /// adds nothing, but the call still publishes. After an eviction the chunk is dropped
    /// uncharged and nothing is published.
    pub fn append(&mut self, chunk: Chunk) {
        let mut tail = self.store.tail();
        if tail.evicted() {
            return;
        }
        let rows = u64::from(chunk.rows());
        if rows > 0 {
            let stored = Arc::new(StoredChunk::charge(chunk, &self.budget));
            tail.bytes += stored.chunk.bytes();
            tail.chunks = tail
                .chunks
                .iter()
                .cloned()
                .chain(iter::once(stored))
                .collect();
            let start = tail.rows;
            tail.starts = tail
                .starts
                .iter()
                .copied()
                .chain(iter::once(start))
                .collect();
            tail.rows += rows;
        }
        let paused = self.paused(&tail.completeness);
        self.store.publish(&mut tail, paused);
    }

    /// Sets the completeness and publishes (nothing after an eviction).
    pub fn set_state(&mut self, c: Completeness) {
        let mut tail = self.store.tail();
        if tail.evicted() {
            return;
        }
        tail.completeness = c;
        let paused = self.paused(&c);
        self.store.publish(&mut tail, paused);
    }

    /// The pause a snapshot in state `c` reports: the last `admit`'s, while `Suspended`.
    fn paused(&self, c: &Completeness) -> Option<PauseReason> {
        match c {
            Completeness::Suspended(_) => {
                Admission::pause_of(self.last_admission.load(Ordering::Relaxed))
            }
            _ => None,
        }
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
        if !self.armed {
            return;
        }
        let mut tail = self.store.tail();
        if !tail.evicted()
            && matches!(
                tail.completeness,
                Completeness::Streaming | Completeness::Suspended(_)
            )
        {
            tail.completeness = Completeness::Closed {
                reason: CloseReason::Cancelled,
            };
            self.store.publish(&mut tail, None);
        }
    }
}
