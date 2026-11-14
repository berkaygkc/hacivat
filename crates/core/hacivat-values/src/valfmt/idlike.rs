//! The ID heuristic (Task 16): ID-like columns stay ungrouped.

use crate::logical::LogicalType;

/// Whether a column is ID-like: an integer column named `id` or ending in `_id` (ASCII match),
/// or a PostgreSQL `oid`/`reg*` column.
pub fn id_like(name: &str, logical: &LogicalType, native_name: &str) -> bool {
    let _ = (name, logical, native_name);
    unimplemented!("valfmt::id_like: Task 16")
}
