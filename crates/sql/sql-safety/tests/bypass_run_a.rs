//! SQL-SAFE-06 run A (M3b Task 18; `alg/13` SQL-SAFE-06 step 2): the classifier on, offline.
//! Every row of the bypass catalogue (`sql-testkit/bypass/{pg,mysql,mariadb,sqlite}.toml`: P1–P39,
//! M1–M19, L1–L9, X1–X5 and the SQL-SAFE-02 test table) is classified as the read-only guard's
//! step 0b classifies a statement (one statement, no client flavor, the session's lexing modes
//! known) and must come to its recorded verdict under `ReadOnlyPlusExplain` for a human
//! (R-M3-56) with exactly its recorded reason codes. A row's verdict is `allow` when the read-only
//! guard lets it through to the server's layers, else `deny` (a `Confirm` is a denial on a
//! read-only session).
//!
//! The rows are data read by path: `sql-testkit` has no edge to this crate (P-M3b-13), and the
//! Docker runs (A′ in `hacivat-exec`, B in `hacivat-session`) read the same files.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, LexModes};
use sql_safety::{Actor, ClassifyCtx, Expect, Profile, SafetyReport, Verdict, classify, decide};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    row: Vec<Row>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    ServerBlocked,
    Neutralised,
    PrivilegeOnly,
    NonTransactional,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expected {
    verdict: String,
    reasons: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    id: String,
    sql: String,
    #[serde(default)]
    modes: Option<String>,
    #[serde(default)]
    search_path: Option<Vec<String>>,
    #[serde(default)]
    allowlist: Option<Vec<String>>,
    classifier: Expected,
    server_off: Vec<String>,
    accounts: Vec<String>,
    kind: Kind,
    anchor: String,
    #[serde(default)]
    observed: Option<BTreeMap<String, String>>,
    #[serde(default)]
    note: Option<String>,
}

const FILES: [(&str, DialectId); 4] = [
    ("pg", DialectId::Postgres),
    ("mysql", DialectId::MySql),
    ("mariadb", DialectId::MariaDb),
    ("sqlite", DialectId::Sqlite),
];

fn bypass_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sql-testkit/bypass")
}

fn rows_of(name: &str) -> Vec<Row> {
    let path = bypass_dir().join(format!("{name}.toml"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    toml::from_str::<File>(&text)
        .unwrap_or_else(|e| panic!("{path:?}: {e}"))
        .row
}

/// Every row with its dialect.
fn bypass_rows() -> Vec<(DialectId, Row)> {
    FILES
        .iter()
        .flat_map(|(name, d)| rows_of(name).into_iter().map(move |r| (*d, r)))
        .collect()
}

/// The statement as it runs: the placeholders the Docker runs fill in, filled.
fn sql_of(row: &Row) -> String {
    row.sql
        .replace("{victim}", "4242")
        .replace("{tro}", "transaction_read_only")
        .replace("{d}", "/tmp/hv_t18")
}

/// Step 0b's context for `row` on `d` (the read-only guard's: one statement, no client
/// flavor), with the session's lexing modes known: the dialect's defaults, or `row.modes` (a
/// PostgreSQL `standard_conforming_strings` value or a MySQL `sql_mode`; `unknown`: not
/// known); the row's search path and function allowlist.
fn ctx_for(d: DialectId, row: &Row) -> ClassifyCtx {
    let mut ctx = ClassifyCtx::new(
        DialectCtx::new(d, None),
        ClientFlavor::None,
        Expect::SingleStatement,
    );
    let modes = match row.modes.as_deref() {
        Some("unknown") => None,
        None => Some(LexModes::default_for(d)),
        Some(m) if d == DialectId::Postgres => Some(LexModes {
            standard_conforming_strings: m.eq_ignore_ascii_case("on"),
            ..LexModes::default_for(d)
        }),
        Some(m) => Some(LexModes::from_mysql_sql_mode(m)),
    };
    if let Some(modes) = modes {
        ctx = ctx.with_known_modes(modes);
    }
    if let Some(path) = &row.search_path {
        ctx.search_path = path.iter().map(|s| Arc::from(s.as_str())).collect();
    }
    if let Some(allow) = &row.allowlist {
        ctx.fn_allowlist = allow.iter().map(|s| Arc::from(s.as_str())).collect();
    }
    ctx
}

/// The read-only guard's answer: `allow` only when `decide` allows.
fn verdict_of(r: &SafetyReport, p: Profile) -> &'static str {
    match decide(r, p, Actor::Human).verdict {
        Verdict::Allow => "allow",
        Verdict::Confirm(_) | Verdict::Deny(_) => "deny",
    }
}

/// The reason codes the read-only guard's denial names (`decide`'s, which step 0b hands on as
/// `SafetyDenial.reasons`, Fluent IDs), sorted and unique; none for an allowed row.
fn reason_codes(r: &SafetyReport) -> Vec<String> {
    let reasons = match decide(r, Profile::ReadOnlyPlusExplain, Actor::Human).verdict {
        Verdict::Allow => Vec::new(),
        Verdict::Confirm(reasons) | Verdict::Deny(reasons) => reasons,
    };
    let mut codes: Vec<String> = reasons.iter().map(|x| format!("{:?}", x.code)).collect();
    codes.sort();
    codes.dedup();
    codes
}

#[test]
fn run_a_every_row_matches_verdict_and_reasons() {
    let mut failures = Vec::new();
    let rows = bypass_rows();
    for (d, row) in &rows {
        let r = classify(&sql_of(row), &ctx_for(*d, row));
        let (verdict, reasons) = (
            verdict_of(&r, Profile::ReadOnlyPlusExplain),
            reason_codes(&r),
        );
        if verdict != row.classifier.verdict || reasons != row.classifier.reasons {
            failures.push(format!(
                "{d:?} {} {:?}: {verdict} {reasons:?}, recorded {} {:?}",
                row.id, row.sql, row.classifier.verdict, row.classifier.reasons
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(rows.len() >= 300, "{}", rows.len());
}

#[test]
fn every_denial_names_its_reasons_and_only_reads_are_allowed() {
    // A row recorded `deny` names at least one reason (never "denied" with nothing to say), and
    // every row the guard lets through is one the catalogue expects to be a read (the documented
    // allow rows) or an allowlisted builtin.
    for (d, row) in bypass_rows() {
        let deny = row.classifier.verdict == "deny";
        assert!(
            deny || row.classifier.verdict == "allow",
            "{d:?} {}: {}",
            row.id,
            row.classifier.verdict
        );
        if deny {
            assert!(
                !row.classifier.reasons.is_empty(),
                "{d:?} {} {:?}",
                row.id,
                row.sql
            );
        }
    }
    let allowed: Vec<(String, String)> = bypass_rows()
        .into_iter()
        .filter(|(_, r)| r.classifier.verdict == "allow")
        .map(|(_, r)| (r.id, r.sql))
        .collect();
    let documented: Vec<(String, String)> = ALLOW_ROWS
        .iter()
        .map(|(id, sql)| (id.to_string(), sql.to_string()))
        .collect();
    assert_eq!(allowed, documented);
}

/// The rows the read-only guard lets through, in file order: P17's run B form (`SET search_path
/// = pg_catalog`, on the harmless list), P18 with `pg_catalog` first, P20 and P27 (reads; the
/// server's layers stop what the view or the long read does), P39 (a principal row: nothing to
/// classify), the SQL-SAFE-02 table's allow rows (1, 4, 33, 34, 15 with the profile's allowlist,
/// 31's `table_info`), L5 (no host function), L7's recursive read, and SQLite's comment that
/// runs to the end of the text (its server accepts it too). M14's in-statement assignment and
/// SAFE-02 row 27 (`SELECT @v := 1`), which `alg/13` S-13 would allow flagged, are denied: the
/// T4 policy refuses `SESSION_STATE` on a read-only session (stricter; reported).
const ALLOW_ROWS: [(&str, &str); 14] = [
    ("P17", "SET search_path = pg_catalog"),
    ("P18", "SELECT lower('A')"),
    ("P20", "SELECT * FROM hv_t23_v"),
    ("P27", "SELECT * FROM t"),
    ("P39", "SELECT 1"),
    ("SAFE02-1", "SELECT * FROM t"),
    ("SAFE02-4", "EXPLAIN SELECT 1"),
    ("SAFE02-33", "SELECT \"DELETE\" FROM t"),
    ("SAFE02-34", "SELECT 1;;"),
    ("SAFE02-15", "SELECT my_stable_sql_fn(x) FROM t"),
    ("L5", "SELECT 1"),
    (
        "L7",
        "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT x FROM c",
    ),
    ("SAFE02-31", "PRAGMA table_info(t)"),
    ("X1", "SELECT 1 /* never closed"),
];

#[test]
fn every_catalogue_row_is_there() {
    let ids: Vec<String> = bypass_rows().into_iter().map(|(_, r)| r.id).collect();
    let has = |id: &str| ids.iter().any(|i| i == id);
    let mut missing: Vec<String> = Vec::new();
    for (prefix, n) in [("P", 39), ("M", 19), ("L", 9), ("X", 5)] {
        for i in 1..=n {
            let id = format!("{prefix}{i}");
            if !has(&id) {
                missing.push(id);
            }
        }
    }
    // The SQL-SAFE-02 rows of the dialects with a backend (SQL Server, ClickHouse and Oracle
    // have none: `UnsupportedDialect`, M5+); 16 is 15 with `dblink` installed (a catalog fact,
    // M4).
    for n in (1..=15).chain(17..=27).chain([31, 33, 34]) {
        let id = format!("SAFE02-{n}");
        if !has(&id) {
            missing.push(id);
        }
    }
    assert!(missing.is_empty(), "missing rows: {missing:?}");
}

#[test]
fn docker_rows_say_who_runs_them_and_what_stops_them() {
    for (name, _) in FILES {
        for row in rows_of(name) {
            for a in &row.accounts {
                assert!(
                    ["hv_ro", "owner", "read_only_file"].contains(&a.as_str()),
                    "{name} {}: account {a}",
                    row.id
                );
            }
            // A row run B judges by its stop names at least one; a neutralised one may run.
            if !row.accounts.is_empty() && row.kind != Kind::Neutralised {
                assert!(
                    !row.server_off.is_empty(),
                    "{name} {} {:?}",
                    row.id,
                    row.sql
                );
            }
            if row.kind == Kind::Neutralised {
                assert!(row.server_off.is_empty(), "{name} {} {:?}", row.id, row.sql);
            }
            assert!(!row.anchor.is_empty(), "{name} {}", row.id);
            // C19 (R-M3-58): every MariaDB row run B runs carries its recorded outcome.
            if name == "mariadb" && !row.accounts.is_empty() {
                assert!(row.observed.is_some(), "mariadb {} {:?}", row.id, row.sql);
            }
            let _ = &row.note;
        }
    }
}

/// Run B's rows as the M2b suite held them as Rust constants (`docker_m2b_bypass.rs` before
/// Task 18 moved them here): `(engine file, kind, id, statement)`.
const M2B_ROWS: &[(&str, Kind, &str, &str)] = &[
    (
        "pg",
        Kind::ServerBlocked,
        "P1",
        "WITH d AS (DELETE FROM hv_t23 RETURNING *) SELECT * FROM d",
    ),
    (
        "pg",
        Kind::ServerBlocked,
        "P2",
        "SELECT * INTO hv_t23_p2 FROM hv_t23",
    ),
    (
        "pg",
        Kind::ServerBlocked,
        "P3",
        "EXPLAIN (ANALYZE) DELETE FROM hv_t23",
    ),
    (
        "pg",
        Kind::ServerBlocked,
        "P4",
        "SELECT 1; DROP TABLE hv_t23",
    ),
    ("pg", Kind::ServerBlocked, "P4", "COMMIT; DROP TABLE hv_t23"),
    (
        "pg",
        Kind::ServerBlocked,
        "P5",
        "SET TRANSACTION READ WRITE",
    ),
    (
        "pg",
        Kind::ServerBlocked,
        "P5",
        "SET transaction_read_only = off",
    ),
    (
        "pg",
        Kind::ServerBlocked,
        "P5",
        "SELECT set_config('transaction_read_only', 'off', true)",
    ),
    ("pg", Kind::ServerBlocked, "P7", "COMMIT"),
    ("pg", Kind::ServerBlocked, "P7", "COMMIT AND CHAIN"),
    ("pg", Kind::ServerBlocked, "P7", "/* x */ END"),
    ("pg", Kind::ServerBlocked, "P7", "ROLLBACK"),
    ("pg", Kind::ServerBlocked, "P8", "CALL hv_t23_commit()"),
    (
        "pg",
        Kind::ServerBlocked,
        "P9",
        "DO $$ BEGIN INSERT INTO hv_t23 VALUES (9); END $$",
    ),
    (
        "pg",
        Kind::ServerBlocked,
        "P14",
        "COPY hv_t23 FROM '/etc/hostname'",
    ),
    (
        "pg",
        Kind::ServerBlocked,
        "P26",
        "CREATE TEMP TABLE hv_t23_tmp (a int)",
    ),
    (
        "pg",
        Kind::ServerBlocked,
        "P37",
        "SELECT nextval('hv_t23_seq')",
    ),
    (
        "pg",
        Kind::ServerBlocked,
        "P37",
        "SELECT setval('hv_t23_seq', 100)",
    ),
    ("pg", Kind::ServerBlocked, "P38", "SELECT hv_t23_sd_write()"),
    (
        "pg",
        Kind::ServerBlocked,
        "P38",
        "SELECT hv_t23_dyn_write()",
    ),
    (
        "pg",
        Kind::ServerBlocked,
        "P38",
        "SELECT query_to_xml('DELETE FROM hv_t23 RETURNING a', true, true, '')",
    ),
    (
        "pg",
        Kind::ServerBlocked,
        "P38",
        "SELECT lo_from_bytea(0, '\\x6876')",
    ),
    ("pg", Kind::ServerBlocked, "P38", "SELECT lo_create(0)"),
    (
        "pg",
        Kind::ServerBlocked,
        "P38",
        "SELECT lo_put(230023, 0, '\\x00')",
    ),
    ("pg", Kind::ServerBlocked, "P38", "SELECT lo_unlink(230023)"),
    (
        "pg",
        Kind::ServerBlocked,
        "P38",
        "SELECT lo_import('/etc/hostname')",
    ),
    ("pg", Kind::ServerBlocked, "P35", "SELECT txid_current()"),
    ("pg", Kind::Neutralised, "P5", "RESET transaction_read_only"),
    (
        "pg",
        Kind::Neutralised,
        "P6",
        "SET SESSION CHARACTERISTICS AS TRANSACTION READ WRITE",
    ),
    (
        "pg",
        Kind::Neutralised,
        "P6",
        "SET default_transaction_read_only = off",
    ),
    ("pg", Kind::Neutralised, "P6", "RESET ALL"),
    (
        "pg",
        Kind::Neutralised,
        "P10",
        "PREPARE hv_t23_s AS SELECT 1",
    ),
    (
        "pg",
        Kind::Neutralised,
        "P15",
        "SELECT pg_advisory_lock(230023)",
    ),
    (
        "pg",
        Kind::Neutralised,
        "P15",
        "LOCK TABLE hv_t23 IN ACCESS EXCLUSIVE MODE",
    ),
    (
        "pg",
        Kind::Neutralised,
        "P17",
        "SET search_path = pg_catalog",
    ),
    (
        "pg",
        Kind::Neutralised,
        "P23",
        "SET standard_conforming_strings = off",
    ),
    ("pg", Kind::Neutralised, "P25", "NOTIFY hv_t23, 'x'"),
    (
        "pg",
        Kind::Neutralised,
        "P25",
        "SELECT pg_notify('hv_t23', 'x')",
    ),
    ("pg", Kind::Neutralised, "P26", "LISTEN hv_t23"),
    (
        "pg",
        Kind::Neutralised,
        "P26",
        "DECLARE hv_t23_c CURSOR WITH HOLD FOR SELECT 1",
    ),
    (
        "pg",
        Kind::NonTransactional,
        "P32",
        "SELECT pg_logical_emit_message(false, 'hv_t23', 'x')",
    ),
    (
        "pg",
        Kind::NonTransactional,
        "P32",
        "SELECT pg_catalog.pg_logical_emit_message(false, 'hv_t23', 'x'::bytea)",
    ),
    (
        "pg",
        Kind::NonTransactional,
        "P32",
        "SELECT \"pg_catalog\".\"pg_logical_emit_message\"(false, 'hv_t23', 'x')",
    ),
    (
        "pg",
        Kind::NonTransactional,
        "P32",
        "SELECT PG_LOGICAL_EMIT_MESSAGE /* c */ (false, 'hv_t23', 'x')",
    ),
    (
        "pg",
        Kind::NonTransactional,
        "P32",
        "SELECT U&\"pg\\005flogical_emit_message\"(false, 'hv_t23', 'x')",
    ),
    (
        "pg",
        Kind::NonTransactional,
        "P32",
        "DO $$ BEGIN PERFORM pg_logical_emit_message(false, 'hv_t23', 'x'); END $$",
    ),
    (
        "pg",
        Kind::NonTransactional,
        "P32",
        "SELECT pg_logical_emit_message(true, 'hv_t23', 'x')",
    ),
    (
        "pg",
        Kind::PrivilegeOnly,
        "P11",
        "SELECT pg_terminate_backend({victim})",
    ),
    (
        "pg",
        Kind::PrivilegeOnly,
        "P13",
        "COPY (SELECT 1) TO PROGRAM 'true'",
    ),
    (
        "pg",
        Kind::PrivilegeOnly,
        "P14",
        "SELECT pg_read_file('/etc/hostname')",
    ),
    (
        "pg",
        Kind::PrivilegeOnly,
        "P14",
        "COPY hv_t23 TO '/tmp/hv_t23_copy'",
    ),
    ("pg", Kind::PrivilegeOnly, "P16", "SET ROLE hv_test"),
    (
        "pg",
        Kind::PrivilegeOnly,
        "P16",
        "SET SESSION AUTHORIZATION hv_test",
    ),
    (
        "pg",
        Kind::PrivilegeOnly,
        "P31",
        "SELECT pg_create_physical_replication_slot('hv_t23_slot')",
    ),
    (
        "pg",
        Kind::PrivilegeOnly,
        "P31",
        "DO $$ BEGIN EXECUTE 'SELECT pg_create_physical_' || 'replication_slot(''hv_t23_slot'')'; END $$",
    ),
    ("pg", Kind::PrivilegeOnly, "P32", "SELECT pg_switch_wal()"),
    (
        "pg",
        Kind::PrivilegeOnly,
        "P32",
        "DO $$ BEGIN EXECUTE 'SELECT pg_switch' || '_wal()'; END $$",
    ),
    (
        "pg",
        Kind::PrivilegeOnly,
        "P32",
        "SELECT pg_logical_emit_message(true, 'hv_t23', 'x')",
    ),
    (
        "pg",
        Kind::PrivilegeOnly,
        "P32",
        "SELECT pg_logical_emit_message(false, 'hv_t23', 'x')",
    ),
    (
        "pg",
        Kind::PrivilegeOnly,
        "P32",
        "DO $$ BEGIN EXECUTE 'SELECT pg_logical_' || 'emit_message(true, ''hv_t23'', ''x'')'; END $$",
    ),
    (
        "pg",
        Kind::PrivilegeOnly,
        "P32",
        "DO $$ BEGIN EXECUTE 'SELECT pg_logical_' || 'emit_message(false, ''hv_t23'', ''x'')'; END $$",
    ),
    (
        "pg",
        Kind::PrivilegeOnly,
        "P33",
        "SELECT lo_export(230023, '/tmp/hv_t23_lo')",
    ),
    ("pg", Kind::PrivilegeOnly, "P34", "CHECKPOINT"),
    ("pg", Kind::PrivilegeOnly, "P34", "SELECT pg_stat_reset()"),
    (
        "mysql",
        Kind::ServerBlocked,
        "M1",
        "/*!50000 DROP TABLE hv_t23_bypass */",
    ),
    (
        "mysql",
        Kind::ServerBlocked,
        "M1",
        "/*!50000 INSERT INTO hv_t23_bypass VALUES (50) */",
    ),
    (
        "mysql",
        Kind::ServerBlocked,
        "M2",
        "SELECT 1; DROP TABLE hv_t23_bypass",
    ),
    (
        "mysql",
        Kind::ServerBlocked,
        "M5",
        "SELECT * FROM hv_t23_bypass FOR UPDATE",
    ),
    (
        "mysql",
        Kind::ServerBlocked,
        "M5",
        "LOCK TABLES hv_t23_bypass WRITE",
    ),
    (
        "mysql",
        Kind::ServerBlocked,
        "M6",
        "PREPARE hv_t23_s FROM 'DELETE FROM hv_t23_bypass'",
    ),
    (
        "mysql",
        Kind::ServerBlocked,
        "M8",
        "CREATE TABLE hv_t23_m8 (a int)",
    ),
    (
        "mysql",
        Kind::ServerBlocked,
        "M8",
        "TRUNCATE TABLE hv_t23_bypass",
    ),
    (
        "mysql",
        Kind::ServerBlocked,
        "M8",
        "DELETE FROM hv_t23_bypass",
    ),
    (
        "mysql",
        Kind::ServerBlocked,
        "M12",
        "HANDLER hv_t23_bypass OPEN",
    ),
    ("mysql", Kind::ServerBlocked, "M13", "XA START 'hv_t23'"),
    (
        "mysql",
        Kind::ServerBlocked,
        "M17",
        "CREATE TEMPORARY TABLE hv_t23_tmp (a int)",
    ),
    (
        "mysql",
        Kind::PrivilegeOnly,
        "M4",
        "SELECT a FROM hv_t23_bypass INTO OUTFILE '/tmp/hv_t23_out'",
    ),
    (
        "mysql",
        Kind::PrivilegeOnly,
        "M10",
        "SET GLOBAL max_connections = 999",
    ),
    (
        "mysql",
        Kind::PrivilegeOnly,
        "M10",
        "SET PERSIST max_connections = 999",
    ),
    (
        "mysql",
        Kind::PrivilegeOnly,
        "M10",
        "RESET PERSIST max_connections",
    ),
    ("mysql", Kind::PrivilegeOnly, "M10", "FLUSH TABLES"),
    ("mysql", Kind::PrivilegeOnly, "M10", "KILL {victim}"),
    (
        "mysql",
        Kind::PrivilegeOnly,
        "M11",
        "CREATE FUNCTION hv_t23_udf RETURNS STRING SONAME 'hv_t23.so'",
    ),
    ("mysql", Kind::PrivilegeOnly, "M7", "CALL hv_t23_escape()"),
    (
        "mysql",
        Kind::Neutralised,
        "M1",
        "SELECT 1 /*!50000 , (SELECT GET_LOCK('hv_t23_m1', 0)) */",
    ),
    (
        "mysql",
        Kind::Neutralised,
        "M3",
        "SET sql_mode = 'NO_BACKSLASH_ESCAPES'",
    ),
    (
        "mysql",
        Kind::Neutralised,
        "M5",
        "SELECT GET_LOCK('hv_t23_m5', 0)",
    ),
    (
        "mysql",
        Kind::Neutralised,
        "M9",
        "SET SESSION TRANSACTION READ WRITE",
    ),
    ("mysql", Kind::Neutralised, "M9", "SET @@session.{tro} = 0"),
    ("mysql", Kind::Neutralised, "M14", "SET @hv_t23 = 1"),
    (
        "sqlite",
        Kind::ServerBlocked,
        "L9",
        "INSERT INTO t VALUES (2)",
    ),
    (
        "sqlite",
        Kind::ServerBlocked,
        "L2",
        "ATTACH '{d}/n.db' AS n",
    ),
    (
        "sqlite",
        Kind::ServerBlocked,
        "L2",
        "VACUUM INTO '{d}/c.db'",
    ),
    ("sqlite", Kind::ServerBlocked, "L1", "PRAGMA query_only = 0"),
    (
        "sqlite",
        Kind::ServerBlocked,
        "L8",
        "CREATE TEMP TABLE x (a)",
    ),
    (
        "sqlite",
        Kind::ServerBlocked,
        "L3",
        "SELECT writefile('{d}/w', 'x')",
    ),
    (
        "sqlite",
        Kind::ServerBlocked,
        "L3",
        "SELECT load_extension('hv_t23')",
    ),
    (
        "sqlite",
        Kind::ServerBlocked,
        "L4",
        "PRAGMA writable_schema = 1",
    ),
    (
        "sqlite",
        Kind::ServerBlocked,
        "L9",
        "PRAGMA user_version = 7",
    ),
    (
        "sqlite",
        Kind::ServerBlocked,
        "L9",
        "PRAGMA journal_mode = WAL",
    ),
    ("sqlite", Kind::ServerBlocked, "L6", "BEGIN IMMEDIATE"),
];

#[test]
fn the_row_set_equals_the_m2b_constants() {
    // One-time equality: every M2b row (id and statement) is in the TOML, with the kind of the
    // group it ran in and an account that runs it, in run B's order within its group; and the
    // TOML's Docker rows are exactly these (MariaDB runs MySQL's).
    for (name, _) in FILES {
        let file = if name == "mariadb" { "mysql" } else { name };
        let expected: Vec<(Kind, &str, &str)> = M2B_ROWS
            .iter()
            .filter(|(f, ..)| *f == file)
            .map(|(_, k, id, sql)| (*k, *id, *sql))
            .collect();
        let rows = rows_of(name);
        let docker: Vec<(Kind, &str, &str)> = rows
            .iter()
            .filter(|r| !r.accounts.is_empty())
            .map(|r| (r.kind, r.id.as_str(), r.sql.as_str()))
            .collect();
        assert_eq!(docker, expected, "{name}");
    }
}
