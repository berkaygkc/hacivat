// ## m3b-t01c-core-seams
//! "Count rows" (R-M3-76, P-M3b-27; Task 16): `SELECT COUNT(*) FROM (\n<q>\n) hv_c`, with a
//! trailing `;` and a top-level `ORDER BY` without `LIMIT`/`FETCH` stripped by lexer tokens and
//! the trailing trivia trimmed; classified again and refused unless `Read`.
//!
//! STUB until Task 16: no count statement (`None`); `ExecService::count` answers `Internal`.

use sql_split::DocSnapshot;

use crate::units::ExecUnit;

/// The count statement of `unit` in `doc`, or `None` when it cannot be counted. STUB: `None`.
pub fn count_sql(doc: &DocSnapshot, unit: &ExecUnit) -> Option<String> {
    let _ = (doc, unit);
    None
}
