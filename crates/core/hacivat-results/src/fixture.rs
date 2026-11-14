//! `ResultStore::from_rows` (feature `test-support`; Task 7): a complete, immutable store for
//! shots and UI tests with no core runtime (R-M3-15).

use std::sync::Arc;

use hacivat_values::{ResultSchema, Value};

use crate::cursor::Completeness;
use crate::store::ResultStore;

impl ResultStore {
    /// A store holding `rows` of `schema`, in state `completeness`.
    pub fn from_rows(
        schema: Arc<ResultSchema>,
        rows: Vec<Vec<Value>>,
        completeness: Completeness,
    ) -> Arc<ResultStore> {
        let _ = (schema, rows, completeness);
        unimplemented!("ResultStore::from_rows: Task 7")
    }
}
