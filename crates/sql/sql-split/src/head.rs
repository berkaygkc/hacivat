//! Cursor eligibility from a statement's head (R-M3-23; Task 8). This is a fetch strategy, not
//! a safety layer: read-only safety stays with the guard.

use sql_text::TextRange;

use crate::document::DocSnapshot;

/// Whether the unit at `range` may stay suspended as a console cursor: its head reads
/// (`SELECT`, `VALUES`, `TABLE`, MySQL `SHOW`, or `WITH` without a DML keyword), with no `INTO`,
/// no locking clause and no executable comment (Task 8).
///
/// Task 1b stub (fail closed): `false`, so every unit streams to completion.
pub fn cursor_eligible(doc: &DocSnapshot, range: TextRange) -> bool {
    let _ = (doc, range);
    false
}
