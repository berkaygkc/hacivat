// ## m3b-t01c-core-seams
//! The D-062 wrap (R-M3-21, R-M3-55; Task 15): `EXPLAIN ANALYZE` of a statement that is not a
//! read runs inside `BEGIN … ROLLBACK` (or `SAVEPOINT hv_explain … ROLLBACK TO` inside a user
//! transaction; the session builds it, Task 19), and is refused for `Routine`/`Admin` inner
//! classes.
//!
//! ## m3b-t15-gate
//! [`explain_analyze_inner`] reads one statement report; the gate ([`crate::gate`]) decides.
//! An `Unknown` inner class gets the wrap (the rollback is the point: Hacivat cannot tell what
//! the inner statement changes); `Routine` and `Admin` inners are refused (a routine may commit
//! or reach outside the transaction, an administrative statement is not undone by a rollback),
//! and so is a DDL inner on MySQL, MariaDB and TiDB, whose implicit commit would end the
//! wrapping transaction before the rollback.
//!
//! Fix round 1 (I2, M7): a rollback does not undo everything. Sequence advances, session-level
//! advisory locks, `dblink` and other file or network I/O survive it, and on MySQL/MariaDB/TiDB
//! so do writes to non-transactional (MyISAM) tables. On production the gate therefore asks
//! before wrapping an `Unknown` inner, and before wrapping any inner on those engines.

use hacivat_proto::DbmsKind;
use sql_safety::{GateClass, ReasonCode, RiskFlags, StmtKind, StmtReport, gate_class};

/// The gate class of the statement an `EXPLAIN ANALYZE` in `report` runs, or `None` when
/// `report` is no `EXPLAIN ANALYZE` (its kind, its `EXPLAIN_ANALYZE` flag or its
/// `ExplainAnalyze` reason says so). The backends give such a statement its inner statement's
/// effects, so its gate class is the inner class (an empty inner effect set is `Unknown`).
pub fn explain_analyze_inner(report: &StmtReport) -> Option<GateClass> {
    let explain = matches!(report.kind, StmtKind::Explain { analyze: true })
        || report.flags.contains(RiskFlags::EXPLAIN_ANALYZE)
        || report
            .reasons
            .iter()
            .any(|r| r.code == ReasonCode::ExplainAnalyze);
    explain.then(|| gate_class(report))
}

/// Whether an `EXPLAIN ANALYZE` whose inner statement has class `inner` may run wrapped on
/// `engine` (the module docs).
pub(crate) fn wrap_allowed(inner: GateClass, engine: DbmsKind) -> bool {
    let ddl = matches!(inner, GateClass::DdlOther | GateClass::DdlDestructive);
    let commits_on_ddl = matches!(engine, DbmsKind::MySql | DbmsKind::MariaDb | DbmsKind::TiDb);
    !matches!(inner, GateClass::Routine | GateClass::Admin) && !(ddl && commits_on_ddl)
}
