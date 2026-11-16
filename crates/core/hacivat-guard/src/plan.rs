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
    // ## m3b-t18-safe06 (named exception; R-M3-57, P-M3b-33)
    /// Session setup built from what the session observed ([`plan_for_with`]), run right after
    /// `session_setup` wherever it runs (after every reset): MySQL and MariaDB
    /// `SET SESSION sql_mode = '<observed>'`. Empty from [`plan_for`].
    pub pinned: Vec<String>,
}

// ## m3b-t18-safe06 (named exception; R-M3-57, P-M3b-33)
/// What a session observed that its guard plan pins ([`plan_for_with`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct GuardInputs {
    /// MySQL/MariaDB: the session's `sql_mode` as read at connect, verbatim.
    pub sql_mode: Option<String>,
}

/// Whether `mode` can be pinned as it is: a `sql_mode` value is a comma-separated list of
/// mode names (ASCII letters, digits, `_`), possibly empty. Anything else is never put into a
/// statement.
pub fn pinnable_sql_mode(mode: &str) -> bool {
    mode.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b',')
}

/// [`plan_for`] with what the session observed (R-M3-57): on MySQL and MariaDB a pinnable
/// `sql_mode` ([`pinnable_sql_mode`]) becomes `SET SESSION sql_mode = '<mode>'` in `pinned`, so
/// the server lexes after `COM_RESET_CONNECTION` (which restores the global default) as the
/// classifier does. A value that is not pinnable pins nothing (the classifier then reads the
/// text under every mode, fail-closed). Other engines ignore `inputs`.
pub fn plan_for_with(kind: DbmsKind, inputs: &GuardInputs) -> Option<GuardPlan> {
    let mut plan = plan_for(kind)?;
    if matches!(kind, DbmsKind::MySql | DbmsKind::MariaDb)
        && let Some(mode) = inputs.sql_mode.as_deref().filter(|m| pinnable_sql_mode(m))
    {
        plan.pinned.push(format!("SET SESSION sql_mode = '{mode}'"));
    }
    Some(plan)
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
            // T18 review m-3: qualified, so a `search_path` that names another schema before
            // `pg_catalog` cannot shadow the tripwire.
            canary: Some("SELECT pg_catalog.txid_current_if_assigned()"),
            after: vec!["ROLLBACK"],
            reset: ResetKind::DiscardAll,
            pinned: Vec::new(),
        }),
        DbmsKind::MySql | DbmsKind::MariaDb => Some(GuardPlan {
            session_setup: vec!["SET SESSION TRANSACTION READ ONLY", "SET NAMES utf8mb4"],
            before: vec!["START TRANSACTION READ ONLY"],
            canary: None,
            after: vec!["ROLLBACK"],
            reset: ResetKind::ResetConnection,
            pinned: Vec::new(),
        }),
        DbmsKind::Sqlite => Some(GuardPlan {
            session_setup: Vec::new(),
            before: Vec::new(),
            canary: None,
            after: Vec::new(),
            reset: ResetKind::None,
            pinned: Vec::new(),
        }),
        _ => None,
    }
}

// ## m3b-t18-safe06
#[cfg(test)]
mod m3b_t18_tests {
    use super::*;

    #[test]
    fn plan_for_with_pins_an_observed_sql_mode_on_mysql_only() {
        let inputs = GuardInputs {
            sql_mode: Some("ANSI_QUOTES,NO_BACKSLASH_ESCAPES".into()),
        };
        for k in [DbmsKind::MySql, DbmsKind::MariaDb] {
            let p = plan_for_with(k, &inputs).unwrap();
            assert_eq!(
                p.pinned,
                ["SET SESSION sql_mode = 'ANSI_QUOTES,NO_BACKSLASH_ESCAPES'"]
            );
            assert_eq!(p.session_setup, plan_for(k).unwrap().session_setup);
        }
        assert!(
            plan_for_with(DbmsKind::Postgres, &inputs)
                .unwrap()
                .pinned
                .is_empty()
        );
        assert!(plan_for(DbmsKind::MySql).unwrap().pinned.is_empty());
        // An empty mode is a mode (no flags); text that is no mode list is never sent.
        let empty = GuardInputs {
            sql_mode: Some(String::new()),
        };
        assert_eq!(
            plan_for_with(DbmsKind::MySql, &empty).unwrap().pinned,
            ["SET SESSION sql_mode = ''"]
        );
        for hostile in ["x'; DROP TABLE t; --", "A B", "A\\", "ANSI\u{0}"] {
            let p = plan_for_with(
                DbmsKind::MySql,
                &GuardInputs {
                    sql_mode: Some(hostile.into()),
                },
            )
            .unwrap();
            assert!(p.pinned.is_empty(), "{hostile:?}");
        }
    }
}
