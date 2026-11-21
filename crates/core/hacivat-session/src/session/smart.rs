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
/// as in Auto (the module docs). Reads the leading words ([`leading_words`]: comments skipped,
/// fix round 2, R1-m4).
pub(crate) fn outside_tx_only(kind: hacivat_proto::DbmsKind, sql: &str) -> bool {
    let words = leading_words(sql, 8);
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

// ## m3b-t19-confirm-tx (fix round 2, R1-m1, R1-m4)
/// The first `n` words of `sql`, ASCII-uppercased: leading whitespace and comments (`--`, `#`,
/// `/* */`) are skipped, and a MySQL/MariaDB executable comment (`/*!…`, `/*!50000 …`,
/// `/*M!…`) is read as code (its marker and version skipped, its `*/` a separator). `(`, `)`,
/// `,` and `;` separate words; any other character ends the scan as a one-character word of its
/// own, so "something else follows" is never mistaken for the end of the statement.
pub(crate) fn leading_words(sql: &str, n: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = sql;
    while out.len() < n {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if let Some(r) = rest.strip_prefix("--").or_else(|| rest.strip_prefix('#')) {
            rest = r.find('\n').map_or("", |i| &r[i + 1..]);
        } else if let Some(r) = rest
            .strip_prefix("/*!")
            .or_else(|| rest.strip_prefix("/*M!"))
        {
            rest = r.trim_start_matches(|c: char| c.is_ascii_digit());
        } else if let Some(r) = rest.strip_prefix("/*") {
            match r.find("*/") {
                Some(i) => rest = &r[i + 2..],
                None => break,
            }
        } else if let Some(r) = rest.strip_prefix("*/") {
            rest = r;
        } else if let Some(r) = rest.strip_prefix(['(', ')', ',', ';']) {
            rest = r;
        } else {
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '$')))
                .unwrap_or(rest.len());
            if end == 0 {
                let c = rest.chars().next().unwrap_or_default();
                out.push(c.to_string());
                break;
            }
            out.push(rest[..end].to_ascii_uppercase());
            rest = &rest[end..];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacivat_proto::DbmsKind;

    #[test]
    fn leading_words_skip_comments_and_read_executable_comments() {
        assert_eq!(
            leading_words("/* c */ -- x\n  lock tables t write", 2),
            ["LOCK", "TABLES"]
        );
        assert_eq!(
            leading_words("/*!40000 LOCK TABLES t */", 2),
            ["LOCK", "TABLES"]
        );
        assert_eq!(
            leading_words("/*M!100000 UNLOCK TABLES */", 2),
            ["UNLOCK", "TABLES"]
        );
        assert_eq!(leading_words("# c\nVACUUM", 2), ["VACUUM"]);
        assert_eq!(leading_words("CLUSTER \"t\"", 4), ["CLUSTER", "\""]);
        assert_eq!(
            leading_words("REINDEX (VERBOSE) INDEX CONCURRENTLY i", 8),
            ["REINDEX", "VERBOSE", "INDEX", "CONCURRENTLY", "I"]
        );
        assert!(leading_words("/* never closed", 2).is_empty());
    }

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
            "/* maintenance */ VACUUM",
            "-- nightly\nCREATE INDEX CONCURRENTLY i ON t (a)",
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
            "CLUSTER \"t\"",
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
