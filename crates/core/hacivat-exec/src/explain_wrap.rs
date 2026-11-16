// ## m3b-t01c-core-seams
//! The D-062 wrap (R-M3-21, R-M3-55; Task 15): `EXPLAIN ANALYZE` of a statement that is not a
//! read runs inside `BEGIN … ROLLBACK` (or `SAVEPOINT hv_explain … ROLLBACK TO` inside a user
//! transaction), and is refused for `Routine`/`Admin` inner classes.
//!
//! STUB until Task 15: no statement is recognised as an `EXPLAIN ANALYZE` (`None`); the gate
//! stub denies every unit anyway.

use sql_safety::{GateClass, StmtReport};

/// The gate class of the statement an `EXPLAIN ANALYZE` in `report` runs, or `None` when
/// `report` is no `EXPLAIN ANALYZE`. STUB: `None`.
pub fn explain_analyze_inner(report: &StmtReport) -> Option<GateClass> {
    let _ = report;
    None
}
