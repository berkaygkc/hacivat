//! `ResultStore::from_rows` and `from_rows_paused` (feature `test-support`; Task 7): finished,
//! immutable stores for shots and UI tests with no core runtime (R-M3-15).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use hacivat_values::{Chunk, ResultSchema, Value};

use crate::budget::MemoryBudget;
use crate::cursor::{Completeness, CursorInfo};
use crate::store::{ResultId, ResultStore};
use crate::window::FetchPolicy;
use crate::writer::{Admission, PauseReason, ResultWriter};

/// Fixture IDs count down from the top, far from the run service's, so a fixture and a live
/// result never share one.
static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(u64::MAX);

/// `rows` cut as a stream arrives: the default policy's first rows, then whole chunks.
fn stream_chunks(schema: &ResultSchema, rows: Vec<Vec<Value>>) -> Vec<Chunk> {
    let policy = FetchPolicy::default();
    let mut rows = rows.into_iter();
    let mut size = policy.first_rows as usize;
    let mut chunks = Vec::new();
    loop {
        let part: Vec<Vec<Value>> = rows.by_ref().take(size).collect();
        if part.is_empty() {
            return chunks;
        }
        chunks.push(Chunk::from_values(schema, part));
        size = policy.chunk_rows as usize;
    }
}

/// A generation-1 store on `budget` holding `chunks`, and its writer.
fn fixture_store(
    schema: Arc<ResultSchema>,
    chunks: Vec<Chunk>,
    budget: Arc<MemoryBudget>,
) -> (Arc<ResultStore>, ResultWriter) {
    let id = ResultId(NEXT_FIXTURE_ID.fetch_sub(1, Ordering::Relaxed));
    let (store, mut writer) = ResultStore::new(id, schema, budget, 1);
    for chunk in chunks {
        writer.append(chunk);
    }
    (store, writer)
}

impl ResultStore {
    /// A store holding `rows` of `schema`, in state `completeness`: chunked as a stream
    /// is (the default policy's first rows, then whole chunks), generation 1, on a budget of
    /// its own, and with its writer gone, so it never changes. The writer's drop guard is
    /// disarmed, so a `Streaming` or `Suspended` fixture keeps that state.
    ///
    /// # Panics
    /// When a row has more or fewer values than `schema` has columns.
    pub fn from_rows(
        schema: Arc<ResultSchema>,
        rows: Vec<Vec<Value>>,
        completeness: Completeness,
    ) -> Arc<ResultStore> {
        let chunks = stream_chunks(&schema, rows);
        let budget = MemoryBudget::new(u64::MAX, u64::MAX);
        let (store, mut writer) = fixture_store(schema, chunks, budget);
        writer.set_state(completeness);
        writer.freeze();
        store
    }

    /// The memory banner's state (R-M3-33; the "Memory limit" strip shot): `rows` of
    /// `schema`, `Suspended` with `cursor`, and paused for `reason`, as a result at its limit
    /// is. Its own budget is sized to the rows: for `ResultCap` the per-result cap equals
    /// them, for `GlobalBudget` the global budget does, so the writer's last `admit` really
    /// paused for that reason. The cap is not raised.
    ///
    /// # Panics
    /// As [`from_rows`](Self::from_rows).
    pub fn from_rows_paused(
        schema: Arc<ResultSchema>,
        rows: Vec<Vec<Value>>,
        cursor: CursorInfo,
        reason: PauseReason,
    ) -> Arc<ResultStore> {
        let chunks = stream_chunks(&schema, rows);
        let held: u64 = chunks.iter().map(Chunk::bytes).sum();
        let budget = match reason {
            PauseReason::ResultCap => MemoryBudget::new(u64::MAX, held),
            PauseReason::GlobalBudget => MemoryBudget::new(held, u64::MAX),
        };
        let (store, mut writer) = fixture_store(schema, chunks, budget);
        assert_eq!(writer.admit(1), Admission::Pause(reason));
        writer.set_state(Completeness::Suspended(cursor));
        writer.freeze();
        store
    }
}
