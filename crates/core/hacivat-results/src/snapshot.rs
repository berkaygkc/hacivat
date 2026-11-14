//! `StoreSnapshot` (Task 7): an immutable view of a result's chunks; a held snapshot keeps its
//! chunks alive and never sees later rows.

use std::fmt;
use std::sync::Arc;

use hacivat_values::{CellFlags, ResultSchema, ValueRef};

use crate::cursor::Completeness;

/// One published state of a result.
pub struct StoreSnapshot {
    schema: Arc<ResultSchema>,
    completeness: Completeness,
}

impl fmt::Debug for StoreSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoreSnapshot")
            .field("completeness", &self.completeness)
            .finish_non_exhaustive()
    }
}

impl StoreSnapshot {
    /// Bumped when a re-run replaces the rows.
    pub fn generation(&self) -> u64 {
        unimplemented!("StoreSnapshot::generation: Task 7")
    }

    /// Bumped on every publish.
    pub fn version(&self) -> u64 {
        unimplemented!("StoreSnapshot::version: Task 7")
    }

    pub fn schema(&self) -> &Arc<ResultSchema> {
        &self.schema
    }

    pub fn row_count(&self) -> u64 {
        unimplemented!("StoreSnapshot::row_count: Task 7")
    }

    pub fn completeness(&self) -> &Completeness {
        &self.completeness
    }

    pub fn bytes(&self) -> u64 {
        unimplemented!("StoreSnapshot::bytes: Task 7")
    }

    /// The cell at (`row`, `col`): a binary search over the chunk starts, then the chunk.
    pub fn cell(&self, row: u64, col: u32) -> ValueRef<'_> {
        let _ = (row, col);
        unimplemented!("StoreSnapshot::cell: Task 7")
    }

    pub fn flags(&self, row: u64, col: u32) -> CellFlags {
        let _ = (row, col);
        unimplemented!("StoreSnapshot::flags: Task 7")
    }
}
