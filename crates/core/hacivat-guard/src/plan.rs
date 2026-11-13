//! Guard plans: the statements a read-only session runs around each user statement, per engine
//! (SQL-SAFE-05). Working since Task 1 (Q8): Task 5's exact values, tested by Task 5.

use hacivat_proto::DbmsKind;

/// How a pooled or guarded connection is returned to a clean state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResetKind {
    /// PostgreSQL: the `DISCARD ALL` sequence without `DEALLOCATE ALL` (Task 10, Q10).
    DiscardAll,
    /// MySQL and MariaDB: `COM_RESET_CONNECTION`.
    ResetConnection,
    None,
}

/// What a read-only session runs: once per session (`session_setup`), around every user
/// statement (`before`, the optional `canary`, `after`), and how it resets (SQL-SAFE-05).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GuardPlan {
    pub session_setup: Vec<&'static str>,
    pub before: Vec<&'static str>,
    pub canary: Option<&'static str>,
    pub after: Vec<&'static str>,
    pub reset: ResetKind,
}

/// The guard plan of `kind`: PostgreSQL, MySQL and MariaDB, and SQLite (whose layers are
/// connection-level, `sqlite_hardening`). `None` for an engine without a plan: such an engine
/// has no read-only guard, so a read-only session on it must not run.
pub fn plan_for(kind: DbmsKind) -> Option<GuardPlan> {
    match kind {
        DbmsKind::Postgres => Some(GuardPlan {
            session_setup: vec![
                "SET default_transaction_read_only = on",
                "SET standard_conforming_strings = on",
                "SET client_encoding = 'UTF8'",
                "SET idle_in_transaction_session_timeout = '30s'",
            ],
            before: vec![
                "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
                "SELECT 1",
            ],
            canary: Some("SELECT txid_current_if_assigned()"),
            after: vec!["ROLLBACK"],
            reset: ResetKind::DiscardAll,
        }),
        DbmsKind::MySql | DbmsKind::MariaDb => Some(GuardPlan {
            session_setup: vec!["SET SESSION TRANSACTION READ ONLY", "SET NAMES utf8mb4"],
            before: vec!["START TRANSACTION READ ONLY"],
            canary: None,
            after: vec!["ROLLBACK"],
            reset: ResetKind::ResetConnection,
        }),
        DbmsKind::Sqlite => Some(GuardPlan {
            session_setup: Vec::new(),
            before: Vec::new(),
            canary: None,
            after: Vec::new(),
            reset: ResetKind::None,
        }),
        _ => None,
    }
}
