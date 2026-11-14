//! `StoreSnapshot` (Task 7): an immutable view of a result's chunks; a held snapshot keeps its
//! chunks alive and never sees later rows.
//!
//! A snapshot is built whole by the writer before it is published: its chunk list, the first
//! row of every chunk, the row count and the bytes all describe the same chunks, so a reader
//! never sees a chunk without its start or a row count past its chunks.

use std::fmt;
use std::sync::Arc;

use hacivat_values::{CellFlags, Chunk, ResultSchema, ValueRef};

use crate::budget::MemoryBudget;
use crate::cursor::Completeness;
use crate::writer::PauseReason;

/// One appended chunk and the budget it is charged to: its last holder gives the bytes back.
pub(crate) struct StoredChunk {
    pub(crate) chunk: Chunk,
    budget: Arc<MemoryBudget>,
}

impl StoredChunk {
    /// Charges `chunk` to `budget` until the returned value drops.
    pub(crate) fn charge(chunk: Chunk, budget: &Arc<MemoryBudget>) -> StoredChunk {
        budget.charge(chunk.bytes());
        StoredChunk {
            chunk,
            budget: budget.clone(),
        }
    }
}

impl Drop for StoredChunk {
    fn drop(&mut self) {
        self.budget.release(self.chunk.bytes());
    }
}

/// One published state of a result.
pub struct StoreSnapshot {
    pub(crate) generation: u64,
    pub(crate) version: u64,
    pub(crate) schema: Arc<ResultSchema>,
    /// Every non-empty chunk, in row order.
    pub(crate) chunks: Arc<[Arc<StoredChunk>]>,
    /// The first row of each chunk: strictly increasing, `starts[0] == 0`.
    pub(crate) starts: Arc<[u64]>,
    pub(crate) row_count: u64,
    pub(crate) bytes: u64,
    pub(crate) completeness: Completeness,
    pub(crate) paused: Option<PauseReason>,
}

impl fmt::Debug for StoreSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoreSnapshot")
            .field("generation", &self.generation)
            .field("version", &self.version)
            .field("row_count", &self.row_count)
            .field("chunks", &self.chunks.len())
            .field("bytes", &self.bytes)
            .field("completeness", &self.completeness)
            .field("paused", &self.paused)
            .finish_non_exhaustive()
    }
}

impl StoreSnapshot {
    /// The empty first snapshot of a result.
    pub(crate) fn empty(generation: u64, schema: Arc<ResultSchema>) -> StoreSnapshot {
        StoreSnapshot {
            generation,
            version: 0,
            schema,
            chunks: Arc::from(Vec::new()),
            starts: Arc::from(Vec::new()),
            row_count: 0,
            bytes: 0,
            completeness: Completeness::Streaming,
            paused: None,
        }
    }

    /// Bumped when a re-run replaces the rows.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Bumped on every publish.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn schema(&self) -> &Arc<ResultSchema> {
        &self.schema
    }

    pub fn row_count(&self) -> u64 {
        self.row_count
    }

    pub fn completeness(&self) -> &Completeness {
        &self.completeness
    }

    /// The bytes of this snapshot's chunks (what they charge the memory budget).
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Why a suspended result stopped fetching (R-M3-33's memory banner): the reason of the
    /// writer's last `admit` when it answered `Pause`. `None` once an `admit` answers `Ok`, and
    /// in every state but `Suspended`.
    pub fn paused(&self) -> Option<PauseReason> {
        self.paused
    }

    /// The cell at (`row`, `col`): a binary search over the chunk starts, then the chunk.
    ///
    /// # Panics
    /// When `row` is not below [`row_count`](Self::row_count) or `col` is outside the schema.
    pub fn cell(&self, row: u64, col: u32) -> ValueRef<'_> {
        let (chunk, local) = self.locate(row);
        chunk.cell(local, col)
    }

    /// The flags of the cell at (`row`, `col`).
    ///
    /// # Panics
    /// As [`cell`](Self::cell).
    pub fn flags(&self, row: u64, col: u32) -> CellFlags {
        let (chunk, local) = self.locate(row);
        chunk.flags(local, col)
    }

    /// The chunk that holds `row`, and the row's index inside it.
    #[inline]
    fn locate(&self, row: u64) -> (&Chunk, u32) {
        assert!(
            row < self.row_count,
            "row {row} outside a result of {} rows",
            self.row_count
        );
        // `starts[0] == 0 <= row`, so at least one start is not after `row`.
        let i = self.starts.partition_point(|&start| start <= row) - 1;
        let local = row - self.starts[i];
        // A chunk holds at most `u32::MAX` rows, so `local` fits.
        (&self.chunks[i].chunk, local as u32)
    }
}
