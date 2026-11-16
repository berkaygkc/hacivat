// ## m3b-t01c-core-seams
//! Smart mode (HV-EXEC-10, D-036; Task 19): Auto, but a unit whose effects open a write
//! (`SessionEffects::OPENS_WRITE`, from the exec gate) opens the user's transaction first,
//! through `manual_begin`.
//!
//! ## m3b-t19-confirm-tx (C2; ADR 0004 `m3b-t19`)
//! - Before a unit of a run whose verdict is not `Read`, while no transaction is open, the
//!   session opens one (owner `User`: the user chose Smart). A `Read` verdict never opens one;
//!   a unit without a verdict (`Unknown`, every M3a run) opens one, the safe direction. A
//!   `TxThenConfirm` unit then takes the savepoint form (`confirm.rs`).
//! - The user ends it (`COMMIT`, `ROLLBACK`, the badge); the mode stays Smart and the next
//!   non-`Read` unit opens again.
//! - Not opened: before the user's own transaction control (`BEGIN`, `COMMIT`, …), before an
//!   `EXPLAIN ANALYZE` that the D-062 wrap rolls back anyway, and on MySQL/MariaDB before a
//!   statement that commits an open transaction first (`SessionEffects::COMMITS_FIRST`: it
//!   would end the new transaction at once, and the implicit-commit backstop would refuse it).
//! - What opens it: PostgreSQL `BEGIN` (with the isolation level), SQLite `BEGIN IMMEDIATE`
//!   unless the statement reads (as Manual), MySQL/MariaDB `START TRANSACTION` (Smart keeps
//!   `autocommit` on, so nothing is open before it; one the user's `autocommit = 0` keeps open
//!   counts as open and is never started again).
//! - `TxCommand::SetMode(Smart)` is accepted like Auto (refused while a transaction is open;
//!   a Manual MySQL session turns `autocommit` back on), and a profile's Smart mode starts as
//!   Smart. `Request::Exec` of the user (no verdict) opens before any statement but the user's
//!   transaction control.
//! - Fix round 1 (controller rulings m4, m5): Smart never opens before a statement that cannot
//!   run inside a transaction ([`outside_tx_only`]: PostgreSQL `VACUUM`, `CREATE`/`DROP
//!   DATABASE`, `CREATE`/`DROP INDEX CONCURRENTLY`, `REINDEX … CONCURRENTLY`, `ALTER SYSTEM`,
//!   `CREATE`/`DROP TABLESPACE`, `CLUSTER` without a table; MySQL/MariaDB `SET TRANSACTION`,
//!   and `LOCK`/`UNLOCK TABLES`, which end any transaction themselves; SQLite `VACUUM`): it
//!   runs as in Auto. On MySQL/MariaDB, while the user's `LOCK TABLES`
//!   locks may be held, Smart's `START TRANSACTION` (which would release them) is refused
//!   (`TxError::TableLocksHeld`), nothing sent.

use crate::session::actor::Core;
use crate::session::schema::{Family, family};
use crate::session::tx::begin_sql;
use crate::session::verdict::{SessionEffects, UnitVerdict};
use crate::session::{RunUnit, TxMode};

impl Core {
    /// Whether Smart opens the user's transaction before `unit` (the module docs).
    /// `control`: the unit is the user's own transaction control.
    pub(crate) fn smart_opens_before(&self, unit: &RunUnit, control: bool) -> bool {
        self.tx.mode == TxMode::Smart
            && !control
            && !outside_tx_only(self.kind, &unit.sql)
            && !self.tx_open_now()
            && !matches!(
                unit.verdict,
                UnitVerdict::Read | UnitVerdict::ExplainAnalyzeRollback
            )
            && !(family(self.kind) == Family::MySql
                && unit.effects.contains(SessionEffects::COMMITS_FIRST))
    }

    /// What Smart sends to open the user's transaction before a statement whose first keyword
    /// is `keyword` (the module docs).
    pub(crate) fn smart_begin(&self, keyword: &str) -> Vec<String> {
        match family(self.kind) {
            Family::MySql => begin_sql(self.kind, self.tx.isolation, false),
            _ => self.manual_begin(keyword),
        }
    }
}

// ## m3b-t19-confirm-tx (fix round 1, m4)
/// Whether `sql` is a statement `kind` refuses inside a transaction block, so Smart lets it run
/// as in Auto (the module docs). Reads the leading words only; a unit carries no leading
/// comment.
pub(crate) fn outside_tx_only(kind: hacivat_proto::DbmsKind, sql: &str) -> bool {
    let words: Vec<String> = sql
        .split(|c: char| c.is_whitespace() || matches!(c, ';' | '(' | ')' | ','))
        .filter(|w| !w.is_empty())
        .take(8)
        .map(str::to_ascii_uppercase)
        .collect();
    let w = |i: usize| words.get(i).map_or("", String::as_str);
    match family(kind) {
        Family::Postgres => match (w(0), w(1)) {
            ("VACUUM", _) | ("ALTER", "SYSTEM") => true,
            ("CREATE" | "DROP", "DATABASE" | "TABLESPACE") => true,
            ("CREATE", _) => {
                let at = if w(1) == "UNIQUE" { 2 } else { 1 };
                w(at) == "INDEX" && w(at + 1) == "CONCURRENTLY"
            }
            ("DROP", "INDEX") => w(2) == "CONCURRENTLY",
            ("REINDEX", _) => words.iter().any(|w| w == "CONCURRENTLY"),
            ("CLUSTER", _) => words.iter().skip(1).all(|w| w == "VERBOSE"),
            _ => false,
        },
        // `LOCK`/`UNLOCK TABLES` end any transaction themselves (and Smart's own start would
        // release the user's locks before an `UNLOCK TABLES` could).
        Family::MySql => matches!(
            (w(0), w(1)),
            ("SET", "TRANSACTION") | ("LOCK" | "UNLOCK", "TABLE" | "TABLES")
        ),
        Family::Sqlite => w(0) == "VACUUM",
        Family::Other => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacivat_proto::DbmsKind;

    #[test]
    fn statements_that_cannot_run_in_a_transaction() {
        let pg = |sql| outside_tx_only(DbmsKind::Postgres, sql);
        for sql in [
            "VACUUM",
            "vacuum analyze t",
            "CREATE DATABASE d",
            "DROP DATABASE d",
            "CREATE INDEX CONCURRENTLY i ON t (a)",
            "CREATE UNIQUE INDEX CONCURRENTLY i ON t (a)",
            "DROP INDEX CONCURRENTLY i",
            "REINDEX TABLE CONCURRENTLY t",
            "REINDEX (VERBOSE) INDEX CONCURRENTLY i",
            "ALTER SYSTEM SET work_mem = '64MB'",
            "CREATE TABLESPACE s LOCATION '/x'",
            "DROP TABLESPACE s",
            "CLUSTER",
            "CLUSTER VERBOSE",
        ] {
            assert!(pg(sql), "{sql}");
        }
        for sql in [
            "CREATE INDEX i ON t (a)",
            "DROP INDEX i",
            "REINDEX TABLE t",
            "CLUSTER t USING i",
            "ALTER TABLE t ADD a int",
            "UPDATE t SET a = 1",
            "CREATE TABLE concurrently (a int)",
        ] {
            assert!(!pg(sql), "{sql}");
        }
        assert!(outside_tx_only(
            DbmsKind::MySql,
            "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE"
        ));
        assert!(!outside_tx_only(
            DbmsKind::MySql,
            "SET SESSION TRANSACTION ISOLATION LEVEL SERIALIZABLE"
        ));
        assert!(outside_tx_only(DbmsKind::MySql, "UNLOCK TABLES"));
        assert!(outside_tx_only(DbmsKind::MySql, "LOCK TABLES t WRITE"));
        assert!(outside_tx_only(DbmsKind::Sqlite, "VACUUM"));
        assert!(!outside_tx_only(DbmsKind::Sqlite, "UPDATE t SET a = 1"));
    }
}
