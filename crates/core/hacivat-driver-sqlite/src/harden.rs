//! Read-only hardening of a SQLite connection (Task 11, B21).
//!
//! A read-only profile (`DriverConnect.read_only`) opens the file with `SQLITE_OPEN_READ_ONLY`
//! (`open.rs`), then [`harden_read_only`] applies `hacivat_guard::sqlite_hardening`, in this
//! order: `SQLITE_DBCONFIG_DEFENSIVE`; `SQLITE_DBCONFIG_ENABLE_ATTACH_CREATE` and
//! `_ATTACH_WRITE` off; `SQLITE_LIMIT_ATTACHED` 0 with the length, SQL-length and variable
//! limits; `PRAGMA query_only = 1`; and last the default-deny authorizer, which maps every
//! action SQLite reports to a [`SqliteAction`] and answers `hacivat_guard::sqlite_authorize`'s
//! verdict. Every setting is read back, so a setting the engine ignored fails the open.
//!
//! A read-write profile gets none of this (M2a's defensive and untrusted-schema settings
//! apply to every connection).

use std::sync::{Arc, Mutex, PoisonError};

use hacivat_driver::{RawError, RawErrorKind};
use hacivat_guard::{Authorization as Verdict, SqliteAction, sqlite_authorize, sqlite_hardening};
use rusqlite::config::DbConfig;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::limits::Limit;
use rusqlite::{Connection, ffi};

use crate::raw_error;

/// The pragma whose plain query `sqlite3_stmt_readonly` reports as a write (Q16 lists it as
/// readable: its query form compiles to `OP_JournalMode` with the "query" mode).
const JOURNAL_MODE: &str = "journal_mode";

/// What the authorizer saw while the current statement was prepared: how many actions it
/// was asked about, and how many of them were the plain journal-mode query. `execute` clears
/// it before preparing and reads it before running (see [`AuthTrail::only_journal_mode_query`]).
#[derive(Debug, Default)]
pub(crate) struct AuthTrail {
    seen: Mutex<(usize, usize)>,
}

impl AuthTrail {
    pub(crate) fn clear(&self) {
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner) = (0, 0);
    }

    fn note(&self, action: &SqliteAction<'_>) {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        seen.0 += 1;
        if matches!(action, SqliteAction::Pragma { name, arg: None } if name.eq_ignore_ascii_case(JOURNAL_MODE))
        {
            seen.1 += 1;
        }
    }

    /// Whether the statement prepared since [`clear`](Self::clear) asked about exactly one
    /// action, `PRAGMA journal_mode` without an argument. That statement only reads the mode;
    /// every form that sets it has an argument, which the authorizer denies.
    pub(crate) fn only_journal_mode_query(&self) -> bool {
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner) == (1, 1)
    }
}

/// The [`SqliteAction`] for an action rusqlite reports. Every action code (0–33) has a
/// mapping: the `CREATE_*` and `DROP_*` families (temp, view, trigger, index, virtual table
/// included) become `Create` and `Drop`; `REINDEX`, and any code rusqlite reports as
/// `Unknown` (0, `SQLITE_COPY`, which SQLite no longer emits; a code past 33; a known code
/// with arguments rusqlite did not expect), become `Other(code)`. An action a later rusqlite
/// adds becomes `Other(-1)`. `sqlite_authorize` denies every `Other`.
pub(crate) fn sqlite_action<'a>(action: &AuthAction<'a>) -> SqliteAction<'a> {
    match *action {
        AuthAction::Read {
            table_name,
            column_name,
        } => SqliteAction::Read {
            table: table_name,
            column: column_name,
        },
        AuthAction::Select => SqliteAction::Select,
        AuthAction::Recursive => SqliteAction::Recursive,
        AuthAction::Function { function_name } => SqliteAction::Function {
            name: function_name,
        },
        AuthAction::Pragma {
            pragma_name,
            pragma_value,
        } => SqliteAction::Pragma {
            name: pragma_name,
            arg: pragma_value,
        },
        AuthAction::Attach { filename } => SqliteAction::Attach { file: filename },
        AuthAction::Detach { .. } => SqliteAction::Detach,
        AuthAction::Insert { table_name } => SqliteAction::Insert { table: table_name },
        AuthAction::Update { table_name, .. } => SqliteAction::Update { table: table_name },
        AuthAction::Delete { table_name } => SqliteAction::Delete { table: table_name },
        AuthAction::CreateIndex { .. }
        | AuthAction::CreateTable { .. }
        | AuthAction::CreateTempIndex { .. }
        | AuthAction::CreateTempTable { .. }
        | AuthAction::CreateTempTrigger { .. }
        | AuthAction::CreateTempView { .. }
        | AuthAction::CreateTrigger { .. }
        | AuthAction::CreateView { .. }
        | AuthAction::CreateVtable { .. } => SqliteAction::Create,
        AuthAction::DropIndex { .. }
        | AuthAction::DropTable { .. }
        | AuthAction::DropTempIndex { .. }
        | AuthAction::DropTempTable { .. }
        | AuthAction::DropTempTrigger { .. }
        | AuthAction::DropTempView { .. }
        | AuthAction::DropTrigger { .. }
        | AuthAction::DropView { .. }
        | AuthAction::DropVtable { .. } => SqliteAction::Drop,
        AuthAction::AlterTable { .. } => SqliteAction::Alter,
        AuthAction::Transaction { .. } => SqliteAction::Transaction,
        AuthAction::Savepoint { .. } => SqliteAction::Savepoint,
        AuthAction::Analyze { .. } => SqliteAction::Analyze,
        AuthAction::Reindex { .. } => SqliteAction::Other(ffi::SQLITE_REINDEX),
        AuthAction::Unknown { code, .. } => SqliteAction::Other(code),
        _ => SqliteAction::Other(-1),
    }
}

/// The authorizer's answer to SQLite for `ctx`: the guard's verdict on the mapped action.
pub(crate) fn authorize(ctx: &AuthContext<'_>) -> Authorization {
    match sqlite_authorize(&sqlite_action(&ctx.action)) {
        Verdict::Allow => Authorization::Allow,
        Verdict::Ignore => Authorization::Ignore,
        Verdict::Deny => Authorization::Deny,
    }
}

/// A hardening setting the engine did not take: the open fails rather than run unguarded.
fn not_applied() -> RawError {
    RawError {
        kind: RawErrorKind::Other,
        server: None,
    }
}

fn db_config(conn: &Connection, config: DbConfig, on: bool) -> Result<(), RawError> {
    let now = conn.set_db_config(config, on).map_err(raw_error)?;
    if now == on && conn.db_config(config).map_err(raw_error)? == on {
        Ok(())
    } else {
        Err(not_applied())
    }
}

fn limit(conn: &Connection, limit: Limit, value: i32) -> Result<(), RawError> {
    conn.set_limit(limit, value).map_err(raw_error)?;
    if conn.limit(limit).map_err(raw_error)? == value {
        Ok(())
    } else {
        Err(not_applied())
    }
}

/// Hardens a connection opened `SQLITE_OPEN_READ_ONLY` for a read-only profile (B21), in the
/// order of the module notes. The authorizer notes each action in `trail`.
pub(crate) fn harden_read_only(
    conn: &mut Connection,
    trail: Arc<AuthTrail>,
) -> Result<(), RawError> {
    let h = sqlite_hardening();
    db_config(conn, DbConfig::SQLITE_DBCONFIG_DEFENSIVE, h.defensive)?;
    db_config(
        conn,
        DbConfig::SQLITE_DBCONFIG_ENABLE_ATTACH_CREATE,
        h.attach_create,
    )?;
    db_config(
        conn,
        DbConfig::SQLITE_DBCONFIG_ENABLE_ATTACH_WRITE,
        h.attach_write,
    )?;
    limit(conn, Limit::SQLITE_LIMIT_ATTACHED, h.limit_attached)?;
    limit(conn, Limit::SQLITE_LIMIT_LENGTH, h.limit_length)?;
    limit(conn, Limit::SQLITE_LIMIT_SQL_LENGTH, h.limit_sql_length)?;
    limit(
        conn,
        Limit::SQLITE_LIMIT_VARIABLE_NUMBER,
        h.limit_variable_number,
    )?;
    let query_only = if h.query_only { 1 } else { 0 };
    conn.execute_batch(&format!("PRAGMA query_only = {query_only}"))
        .map_err(raw_error)?;
    let now: i64 = conn
        .query_row("PRAGMA query_only", [], |r| r.get(0))
        .map_err(raw_error)?;
    if now != query_only {
        return Err(not_applied());
    }
    conn.authorizer(Some(move |ctx: AuthContext<'_>| {
        trail.note(&sqlite_action(&ctx.action));
        authorize(&ctx)
    }))
    .map_err(raw_error)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use rusqlite::hooks::TransactionOperation;

    use super::*;

    /// The action rusqlite reports for SQLite action `code` with typical arguments: the table
    /// of `sqlite3.h` (`SQLITE_COPY` 0 through `SQLITE_RECURSIVE` 33). Code 0 has no rusqlite
    /// variant, so it arrives as `Unknown`.
    fn reported(code: i32) -> AuthAction<'static> {
        use AuthAction as A;
        let (i, t, v, tr) = ("i", "t", "v", "tr");
        match code {
            0 => A::Unknown {
                code,
                arg1: Some(t),
                arg2: None,
            },
            1 => A::CreateIndex {
                index_name: i,
                table_name: t,
            },
            2 => A::CreateTable { table_name: t },
            3 => A::CreateTempIndex {
                index_name: i,
                table_name: t,
            },
            4 => A::CreateTempTable { table_name: t },
            5 => A::CreateTempTrigger {
                trigger_name: tr,
                table_name: t,
            },
            6 => A::CreateTempView { view_name: v },
            7 => A::CreateTrigger {
                trigger_name: tr,
                table_name: t,
            },
            8 => A::CreateView { view_name: v },
            9 => A::Delete { table_name: t },
            10 => A::DropIndex {
                index_name: i,
                table_name: t,
            },
            11 => A::DropTable { table_name: t },
            12 => A::DropTempIndex {
                index_name: i,
                table_name: t,
            },
            13 => A::DropTempTable { table_name: t },
            14 => A::DropTempTrigger {
                trigger_name: tr,
                table_name: t,
            },
            15 => A::DropTempView { view_name: v },
            16 => A::DropTrigger {
                trigger_name: tr,
                table_name: t,
            },
            17 => A::DropView { view_name: v },
            18 => A::Insert { table_name: t },
            19 => A::Pragma {
                pragma_name: "writable_schema",
                pragma_value: Some("1"),
            },
            20 => A::Read {
                table_name: t,
                column_name: "a",
            },
            21 => A::Select,
            22 => A::Transaction {
                operation: TransactionOperation::Begin,
            },
            23 => A::Update {
                table_name: t,
                column_name: "a",
            },
            24 => A::Attach { filename: "x.db" },
            25 => A::Detach { database_name: "x" },
            26 => A::AlterTable {
                database_name: "main",
                table_name: t,
            },
            27 => A::Reindex { index_name: i },
            28 => A::Analyze { table_name: t },
            29 => A::CreateVtable {
                table_name: t,
                module_name: "fts5",
            },
            30 => A::DropVtable {
                table_name: t,
                module_name: "fts5",
            },
            31 => A::Function {
                function_name: "upper",
            },
            32 => A::Savepoint {
                operation: TransactionOperation::Begin,
                savepoint_name: "s",
            },
            33 => A::Recursive,
            _ => unreachable!("not an SQLite action code: {code}"),
        }
    }

    fn verdict(a: &AuthAction<'_>) -> Verdict {
        sqlite_authorize(&sqlite_action(a))
    }

    /// Controller carry-in (T5 review): every action code 0–33 through this adapter against
    /// the guard's rules. Only READ, SELECT, FUNCTION (an allowed name) and RECURSIVE are
    /// allowed (PRAGMA here carries a setting, so it is denied); every other code is denied.
    #[test]
    fn every_action_code_maps_and_only_reads_are_allowed() {
        let allowed = [20, 21, 31, 33];
        for code in 0..=33 {
            let want = if allowed.contains(&code) {
                Verdict::Allow
            } else {
                Verdict::Deny
            };
            assert_eq!(verdict(&reported(code)), want, "code {code}");
        }
    }

    /// The same codes, as rusqlite reports them when the arguments do not fit its variant
    /// (`Unknown`), and codes past 33 and negative ones: `Other(code)`, so denied. Even the
    /// read codes are denied in this form.
    #[test]
    fn unknown_and_unlisted_codes_are_denied() {
        for code in (-1..=40).chain([i32::MAX, i32::MIN]) {
            let a = AuthAction::Unknown {
                code,
                arg1: Some("t"),
                arg2: Some("a"),
            };
            assert_eq!(sqlite_action(&a), SqliteAction::Other(code));
            assert_eq!(verdict(&a), Verdict::Deny, "code {code}");
        }
        assert_eq!(
            sqlite_action(&reported(27)),
            SqliteAction::Other(ffi::SQLITE_REINDEX)
        );
    }

    #[test]
    fn the_mapping_keeps_the_names_the_guard_reads() {
        let f = |name| AuthAction::Function {
            function_name: name,
        };
        for name in [
            "ReadFile",
            "LOAD_EXTENSION",
            "writefile",
            "Edit",
            "FTS3_Tokenizer",
        ] {
            assert_eq!(verdict(&f(name)), Verdict::Deny, "{name}");
        }
        assert_eq!(verdict(&f("Upper")), Verdict::Allow);
        let p = |name, value| AuthAction::Pragma {
            pragma_name: name,
            pragma_value: value,
        };
        assert_eq!(verdict(&p("query_only", None)), Verdict::Allow);
        assert_eq!(verdict(&p("query_only", Some("0"))), Verdict::Deny);
        assert_eq!(verdict(&p("table_info", Some("t"))), Verdict::Allow);
        assert_eq!(verdict(&p("writable_schema", None)), Verdict::Deny);
        assert_eq!(
            sqlite_action(&AuthAction::Attach { filename: "f.db" }),
            SqliteAction::Attach { file: "f.db" }
        );
    }

    /// The adapter on a real connection: each statement's actions are recorded with the
    /// verdict the read-only authorizer would give, while the statement itself is allowed (a
    /// shadow authorizer), so every code a statement asks about is seen.
    fn shadow(sql: &str, conn: &mut Connection) -> Vec<(i32, Verdict)> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        conn.authorizer(Some(move |ctx: AuthContext<'_>| {
            let v = sqlite_authorize(&sqlite_action(&ctx.action));
            log.lock().unwrap().push((code_of(&ctx.action), v));
            Authorization::Allow
        }))
        .unwrap();
        conn.execute_batch(sql)
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
        conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .unwrap();
        let mut log = seen.lock().unwrap();
        std::mem::take(&mut *log)
    }

    /// The SQLite action code of a reported action.
    fn code_of(a: &AuthAction<'_>) -> i32 {
        if let AuthAction::Unknown { code, .. } = a {
            return *code;
        }
        (1..=33)
            .find(|c| std::mem::discriminant(&reported(*c)) == std::mem::discriminant(a))
            .unwrap_or(-1)
    }

    /// Every action code SQLite still emits (1–33; 0 is `SQLITE_COPY`, gone since SQLite 3)
    /// arrives through real statements, and each gets the verdict the table above expects.
    #[test]
    fn real_statements_reach_every_emitted_code() {
        let mut c = Connection::open_in_memory().unwrap();
        let script = [
            ("CREATE TABLE t(a)", 2),
            ("CREATE INDEX i ON t(a)", 1),
            ("CREATE TEMP TABLE tt(a)", 4),
            ("CREATE INDEX temp.ti ON tt(a)", 3),
            (
                "CREATE TEMP TRIGGER ttr AFTER INSERT ON t BEGIN SELECT 1; END",
                5,
            ),
            ("CREATE TEMP VIEW tv AS SELECT 1", 6),
            ("CREATE TRIGGER tr AFTER INSERT ON t BEGIN SELECT 1; END", 7),
            ("CREATE VIEW v AS SELECT 1", 8),
            ("INSERT INTO t VALUES (1)", 18),
            ("UPDATE t SET a = 2", 23),
            ("DELETE FROM t", 9),
            ("PRAGMA user_version = 1", 19),
            ("SELECT a FROM t", 20),
            ("SELECT 1", 21),
            ("BEGIN; COMMIT", 22),
            ("ATTACH ':memory:' AS m", 24),
            ("DETACH m", 25),
            ("ALTER TABLE t ADD COLUMN b", 26),
            ("REINDEX", 27),
            ("ANALYZE", 28),
            ("CREATE VIRTUAL TABLE vt USING fts5(x)", 29),
            ("DROP TABLE vt", 30),
            ("SELECT upper('a')", 31),
            ("SAVEPOINT s; RELEASE s", 32),
            (
                "WITH RECURSIVE r(i) AS (SELECT 1 UNION ALL SELECT i FROM r LIMIT 1) SELECT i FROM r",
                33,
            ),
            ("DROP VIEW tv", 15),
            ("DROP TRIGGER ttr", 14),
            ("DROP INDEX temp.ti", 12),
            ("DROP TABLE tt", 13),
            ("DROP TRIGGER tr", 16),
            ("DROP VIEW v", 17),
            ("DROP INDEX i", 10),
            ("DROP TABLE t", 11),
        ];
        let mut covered = Vec::new();
        for (sql, code) in script {
            let seen = shadow(sql, &mut c);
            let hit = seen.iter().find(|(c, _)| *c == code);
            let (_, v) = hit.unwrap_or_else(|| panic!("{sql}: code {code} not in {seen:?}"));
            let want = if [20, 21, 31, 33].contains(&code) {
                Verdict::Allow
            } else {
                Verdict::Deny
            };
            assert_eq!(*v, want, "{sql}");
            covered.push(code);
        }
        covered.sort_unstable();
        covered.dedup();
        assert_eq!(covered, (1..=33).collect::<Vec<_>>());
    }

    #[test]
    fn the_journal_mode_query_is_the_one_exemption() {
        let trail = AuthTrail::default();
        let p = |name, arg| SqliteAction::Pragma { name, arg };
        trail.note(&p("JOURNAL_MODE", None));
        assert!(trail.only_journal_mode_query());
        trail.note(&SqliteAction::Select);
        assert!(!trail.only_journal_mode_query());
        trail.clear();
        trail.note(&p("journal_mode", Some("wal")));
        assert!(!trail.only_journal_mode_query());
        trail.clear();
        assert!(!trail.only_journal_mode_query());
        trail.note(&p("main.journal_mode", None));
        assert!(!trail.only_journal_mode_query());
    }
}
