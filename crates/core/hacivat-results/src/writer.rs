//! `ResultWriter` (Task 7): the single writer (the session actor) admits, appends and sets the
//! state; every append and state change publishes a new snapshot (R-M3-48).

use std::fmt;

use hacivat_values::Chunk;

use crate::cursor::Completeness;
use crate::store::ResultId;

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

/// The one writer of a result.
pub struct ResultWriter {
    id: ResultId,
}

impl fmt::Debug for ResultWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResultWriter")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl ResultWriter {
    pub fn id(&self) -> ResultId {
        self.id
    }

    /// Whether a chunk of about `next_chunk_estimate` bytes fits the budgets.
    pub fn admit(&self, next_chunk_estimate: u64) -> Admission {
        let _ = next_chunk_estimate;
        unimplemented!("ResultWriter::admit ({:?}): Task 7", self.id)
    }

    /// Appends `chunk` and publishes.
    pub fn append(&mut self, chunk: Chunk) {
        let _ = chunk;
        unimplemented!("ResultWriter::append ({:?}): Task 7", self.id)
    }

    /// Sets the completeness and publishes.
    pub fn set_state(&mut self, c: Completeness) {
        let _ = c;
        unimplemented!("ResultWriter::set_state ({:?}): Task 7", self.id)
    }
}
