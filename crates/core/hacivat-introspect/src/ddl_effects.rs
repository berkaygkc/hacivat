// ## m4-t01c-core
//! Refresh planning after DDL (HV-INTRO-12; Task 21): from the catalog effects of the
//! statements a run executed (`sql_safety::StmtReport.ddl`), the session's state when they ran
//! and its transaction state, the namespaces or objects to refresh now and those pending until
//! `COMMIT` (R-M4-42).
//!
//! **Fail-closed stub (M4 Task 1c).** Every statement with any catalog effect (opaque or not)
//! widens to [`Target::Namespace`] of the session's current namespace: refresh more, never
//! less. Inside a transaction on an engine whose DDL is transactional the target waits for the
//! commit; otherwise it is refreshed now.

use hacivat_meta::{NsKey, ObjectKey};
use sql_safety::{DdlEffect, StmtKind};

use crate::introspector::IntroCaps;

/// One statement a run executed successfully.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutedStmt {
    pub kind: StmtKind,
    /// Its catalog effects (`StmtReport.ddl`).
    pub ddl: Vec<DdlEffect>,
}

/// The session's state when the statements ran (captured at execution time, never later).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStateAtExec {
    /// The namespace the session was in (PostgreSQL `[db, current_schema]`; MySQL/MariaDB
    /// `[database]`; SQLite `[main]`).
    pub current: NsKey,
    /// The search path, for resolving unqualified names (Task 21).
    pub search_path: Vec<NsKey>,
}

/// The session's transaction state when the statements ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TxnState {
    AutoCommit,
    InTransaction,
}

/// What to refresh.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    /// A whole namespace.
    Namespace(NsKey),
    /// Some objects of one namespace.
    Objects { ns: NsKey, keys: Vec<ObjectKey> },
    /// The list of namespaces (a schema created, dropped or renamed).
    ScopeRefresh,
}

/// The plan for one run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RefreshPlan {
    /// Refresh at once.
    pub now: Vec<Target>,
    /// Refresh on `COMMIT`, drop on `ROLLBACK` (R-M4-42; the coordinator keeps them per
    /// session, R-M4-11).
    pub pending: Vec<Target>,
}

/// Plans the refresh after `stmts` (alg/02 HV-INTRO-12 steps 1–4; Task 21).
///
/// Stub: any statement with a catalog effect → `Target::Namespace(session.current)`, once.
pub fn plan_refresh(
    stmts: &[ExecutedStmt],
    session: &SessionStateAtExec,
    txn: TxnState,
    caps: &IntroCaps,
) -> RefreshPlan {
    let mut plan = RefreshPlan::default();
    if stmts.iter().all(|s| s.ddl.is_empty()) {
        return plan;
    }
    let target = Target::Namespace(session.current.clone());
    if txn == TxnState::InTransaction && caps.transactional_ddl {
        plan.pending.push(target);
    } else {
        plan.now.push(target);
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacivat_proto::DbmsKind;

    fn session() -> SessionStateAtExec {
        SessionStateAtExec {
            current: NsKey::schema(["db", "public"]),
            search_path: Vec::new(),
        }
    }

    fn stmt(ddl: Vec<DdlEffect>) -> ExecutedStmt {
        ExecutedStmt {
            kind: StmtKind::Other,
            ddl,
        }
    }

    #[test]
    fn no_effect_no_refresh_and_transactions_wait_where_ddl_is_transactional() {
        let pg = IntroCaps::planned(DbmsKind::Postgres);
        let my = IntroCaps::planned(DbmsKind::MySql);
        assert_eq!(
            plan_refresh(&[stmt(vec![])], &session(), TxnState::AutoCommit, &pg),
            RefreshPlan::default()
        );
        let ns = Target::Namespace(NsKey::schema(["db", "public"]));
        let opaque = [stmt(vec![DdlEffect::opaque()])];
        let p = plan_refresh(&opaque, &session(), TxnState::InTransaction, &pg);
        assert_eq!((p.now, p.pending), (vec![], vec![ns.clone()]));
        // MySQL commits before DDL: refreshed at once even inside a transaction.
        let p = plan_refresh(&opaque, &session(), TxnState::InTransaction, &my);
        assert_eq!((p.now, p.pending), (vec![ns], vec![]));
    }
}
