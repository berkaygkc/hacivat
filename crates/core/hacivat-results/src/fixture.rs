//! `ResultStore::from_rows` (feature `test-support`; Task 7): a complete, immutable store for
//! shots and UI tests with no core runtime (R-M3-15).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use hacivat_values::{Chunk, ResultSchema, Value};

use crate::budget::MemoryBudget;
use crate::cursor::Completeness;
use crate::store::{ResultId, ResultStore};
use crate::window::FetchPolicy;

/// Fixture IDs count down from the top, far from the run service's, so a fixture and a live
/// result never share one.
static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(u64::MAX);

impl ResultStore {
    /// A store holding `rows` of `schema`, in state `completeness`: chunked as a stream
    /// is (the default policy's first rows, then whole chunks), generation 1, on a budget of
    /// its own, and with its writer gone, so it never changes.
    ///
    /// # Panics
    /// When a row has more or fewer values than `schema` has columns.
    pub fn from_rows(
        schema: Arc<ResultSchema>,
        rows: Vec<Vec<Value>>,
        completeness: Completeness,
    ) -> Arc<ResultStore> {
        let id = ResultId(NEXT_FIXTURE_ID.fetch_sub(1, Ordering::Relaxed));
        let budget = MemoryBudget::new(u64::MAX, u64::MAX);
        let (store, mut writer) = ResultStore::new(id, schema.clone(), budget, 1);
        let policy = FetchPolicy::default();
        let mut rows = rows.into_iter();
        let mut size = policy.first_rows as usize;
        loop {
            let part: Vec<Vec<Value>> = rows.by_ref().take(size).collect();
            if part.is_empty() {
                break;
            }
            writer.append(Chunk::from_values(&schema, part));
            size = policy.chunk_rows as usize;
        }
        writer.set_state(completeness);
        store
    }
}
