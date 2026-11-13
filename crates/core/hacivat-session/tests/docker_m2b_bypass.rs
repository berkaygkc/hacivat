//! M2b conformance over Docker (Task 23; exit criterion E4; Review Focus 4): the SQL-SAFE-06
//! bypass catalogue against real servers with no classifier. Hacivat has none in M2b: the
//! session runs every read-only statement through the guard (Task 18) — the transaction-keyword
//! layer 1a, the read-only transaction (L3), the session settings (L4), the least-privilege
//! principal (L5), the post-checks and the reset (L6). Each row must be stopped by those layers,
//! or neutralised by the request's end, and a side channel (an admin connection, or the session
//! itself after the request) must find nothing left. Run with `cargo xtask docker test --set
//! pr|full -p hacivat-session`.
//!
//! - Every row runs as `hv_ro` (least privilege). The server-blocked rows also run as the test
//!   account, a superuser on PostgreSQL and the owner of every fixture on MySQL, where no
//!   privilege can stop them: only the read-only transaction and settings do (L3, L4 stand
//!   alone).
//! - PostgreSQL fixtures live in a database of their own (`hv_t23`, dropped first and last);
//!   MySQL's in `hv_t23_*` objects of the test database (dropped first and last). The matrix
//!   volumes keep no trace (Q7).
//! - Every row of every target runs and is recorded before the test judges: one run shows the
//!   whole matrix. Each target's results are written to the Docker artifact
//!   `t23-bypass-<target>.tsv` (ADR 0004 `m2b-t23`). A statement that holds a password is never
//!   printed or recorded.
//! - MariaDB's results are recorded, not judged: its label stays guarded (C19).
//! - `sqlite_bypass_rows_blocked_without_classifier` needs no server: it runs on every PR.

#![allow(
    clippy::result_large_err,
    reason = "Reply is the session's fixed interface"
)]

mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use common::conformance::*;
use hacivat_proto::{AccessLabel, DbmsKind};
use hacivat_session::LiveState;
use hacivat_session::pipeline::{Interaction, LiveConnection, connect};
use hacivat_session::profile::Profile;
use hacivat_session::session::{DenyLayer, ExecErrorKind, Reply};
use hacivat_testkit::{DockerTarget, targets};

// ---------------------------------------------------------------------------------------------
// Rows

/// One SQL-SAFE-06 row: its id, the statement, and the outcomes that count as stopped — a
/// server code (the SQLSTATE on PostgreSQL, the vendor code on MySQL and MariaDB), `1a` (the
/// transaction-keyword layer, before any I/O), `protocol` (MySQL will not prepare it on a
/// read-only connection) or `gap` (L6 found the escape and discarded the connection).
type Row = (&'static str, &'static str, &'static [&'static str]);

/// PostgreSQL's read-only transaction refusal.
const PG_RO: &str = "25006";
/// "transaction read-write mode must be set before any query": the snapshot is pinned.
const PG_PINNED: &str = "25001";
/// PostgreSQL's privilege refusal.
const PG_PRIV: &str = "42501";

/// The fixture large object (`lo_from_bytea(LO, 'hv')`).
const LO: u32 = 230_023;

/// The guard's canary found an assigned transaction ID after the statement: L6, a gap, the
/// connection discarded (after the `ROLLBACK`, which undid the write).
const GAP: &str = "gap";

/// Rows the read-only transaction (or 1a, the extended protocol, or L6's canary with the
/// rollback) stops whoever runs them: P1 P2 P3 P4 P5 P7 P8 P14 (`COPY FROM`) P26 (temp table)
/// P37 P38, and the large-object writes (T5 review m-5). `hv_ro` may meet its missing
/// privilege first (`42501`).
///
/// Observed: the read-only transaction does **not** refuse the large-object writes
/// (`lo_create`, `lo_from_bytea`, and for an owner `lo_put`, `lo_unlink`, `lo_import`) on
/// PostgreSQL 13–18. They run, assign a transaction ID, and L6's canary reports the gap after
/// the guard's `ROLLBACK` has undone them: nothing persists (the side channel), and the
/// connection is discarded. `query_to_xml` cannot run a `DELETE` at all (`0A000`), so a
/// PL/pgSQL `EXECUTE` stands for P38's dynamic SQL.
const PG_SERVER_BLOCKED: &[Row] = &[
    (
        "P1",
        "WITH d AS (DELETE FROM hv_t23 RETURNING *) SELECT * FROM d",
        &[PG_RO],
    ),
    ("P2", "SELECT * INTO hv_t23_p2 FROM hv_t23", &[PG_RO]),
    ("P3", "EXPLAIN (ANALYZE) DELETE FROM hv_t23", &[PG_RO]),
    ("P4", "SELECT 1; DROP TABLE hv_t23", &["42601"]),
    ("P4", "COMMIT; DROP TABLE hv_t23", &["1a"]),
    ("P5", "SET TRANSACTION READ WRITE", &[PG_PINNED]),
    ("P5", "SET transaction_read_only = off", &[PG_PINNED]),
    (
        "P5",
        "SELECT set_config('transaction_read_only', 'off', true)",
        &[PG_PINNED],
    ),
    ("P7", "COMMIT", &["1a"]),
    ("P7", "COMMIT AND CHAIN", &["1a"]),
    ("P7", "/* x */ END", &["1a"]),
    ("P7", "ROLLBACK", &["1a"]),
    ("P8", "CALL hv_t23_commit()", &["2D000"]),
    ("P14", "COPY hv_t23 FROM '/etc/hostname'", &[PG_RO, PG_PRIV]),
    (
        "P26",
        "CREATE TEMP TABLE hv_t23_tmp (a int)",
        &[PG_RO, PG_PRIV],
    ),
    ("P37", "SELECT nextval('hv_t23_seq')", &[PG_RO, PG_PRIV]),
    ("P37", "SELECT setval('hv_t23_seq', 100)", &[PG_RO, PG_PRIV]),
    ("P38", "SELECT hv_t23_sd_write()", &[PG_RO]),
    ("P38", "SELECT hv_t23_dyn_write()", &[PG_RO]),
    (
        "P38",
        "SELECT query_to_xml('DELETE FROM hv_t23 RETURNING a', true, true, '')",
        &[PG_RO, "0A000"],
    ),
    ("P38", "SELECT lo_from_bytea(0, '\\x6876')", &[PG_RO, GAP]),
    ("P38", "SELECT lo_create(0)", &[PG_RO, GAP]),
    (
        "P38",
        "SELECT lo_put(230023, 0, '\\x00')",
        &[PG_RO, PG_PRIV, GAP],
    ),
    ("P38", "SELECT lo_unlink(230023)", &[PG_RO, PG_PRIV, GAP]),
    (
        "P38",
        "SELECT lo_import('/etc/hostname')",
        &[PG_RO, PG_PRIV, GAP],
    ),
    ("P35", "SELECT txid_current()", &[GAP]),
];

/// Rows the server accepts inside the read-only transaction, which the request's end
/// neutralises (L6: `ROLLBACK`, then `DISCARD ALL` and the environment replay): P5 (`RESET`:
/// it restores L4's `default_transaction_read_only = on`, so it changes nothing) P6 P10 P15 P17
/// P23 P25 P26 (`LISTEN`, `WITH HOLD`). A refusal is fine too. Nothing may linger.
const PG_NEUTRALISED: &[(&str, &str)] = &[
    ("P5", "RESET transaction_read_only"),
    (
        "P6",
        "SET SESSION CHARACTERISTICS AS TRANSACTION READ WRITE",
    ),
    ("P6", "SET default_transaction_read_only = off"),
    ("P6", "RESET ALL"),
    ("P10", "PREPARE hv_t23_s AS SELECT 1"),
    ("P15", "SELECT pg_advisory_lock(230023)"),
    ("P15", "LOCK TABLE hv_t23 IN ACCESS EXCLUSIVE MODE"),
    ("P17", "SET search_path = pg_catalog"),
    ("P23", "SET standard_conforming_strings = off"),
    ("P25", "NOTIFY hv_t23, 'x'"),
    ("P25", "SELECT pg_notify('hv_t23', 'x')"),
    ("P26", "LISTEN hv_t23"),
    ("P26", "DECLARE hv_t23_c CURSOR WITH HOLD FOR SELECT 1"),
];

/// Rows the read-only transaction does not stop: only the principal's privileges (L5) do. Run
/// as `hv_ro` only. P11 P13 P14 (read, `COPY TO` a file) P16 P31 P32 P33 P34.
const PG_PRIVILEGE_ONLY: &[Row] = &[
    ("P11", "SELECT pg_terminate_backend({victim})", &[PG_PRIV]),
    ("P13", "COPY (SELECT 1) TO PROGRAM 'true'", &[PG_PRIV]),
    ("P14", "SELECT pg_read_file('/etc/hostname')", &[PG_PRIV]),
    ("P14", "COPY hv_t23 TO '/tmp/hv_t23_copy'", &[PG_PRIV]),
    ("P16", "SET ROLE hv_test", &[PG_PRIV]),
    ("P16", "SET SESSION AUTHORIZATION hv_test", &[PG_PRIV]),
    (
        "P31",
        "SELECT pg_create_physical_replication_slot('hv_t23_slot')",
        &[PG_PRIV],
    ),
    ("P32", "SELECT pg_switch_wal()", &[PG_PRIV]),
    (
        "P32",
        "SELECT pg_logical_emit_message(true, 'hv_t23', 'x')",
        &[PG_PRIV, PG_RO, GAP],
    ),
    (
        "P33",
        "SELECT lo_export(230023, '/tmp/hv_t23_lo')",
        &[PG_PRIV],
    ),
    ("P34", "CHECKPOINT", &[PG_PRIV]),
    ("P34", "SELECT pg_stat_reset()", &[PG_PRIV]),
];

/// The connection was lost (the session recovered on its own).
const LOST: &str = "lost";

/// MySQL's read-only transaction refusal (`ER_CANT_EXECUTE_IN_READ_ONLY_TRANSACTION`).
const MY_RO: &str = "1792";
/// Table-level privilege refusals: `ER_TABLEACCESS_DENIED_ERROR`, `ER_DBACCESS_DENIED_ERROR`.
const MY_TABLE_PRIV: &str = "1142";
const MY_DB_PRIV: &str = "1044";
/// Global privilege refusals: `ER_SPECIFIC_ACCESS_DENIED_ERROR`, `ER_ACCESS_DENIED_ERROR`.
const MY_PRIV: &str = "1227";
const MY_ACCESS: &str = "1045";

/// Rows the read-only transaction or session setting (or the prepared-statement protocol)
/// stops whoever runs them: M1 M2 M5 (`FOR UPDATE`, `LOCK TABLES`) M6 M8 M12 M13 M17. `hv_ro`
/// may meet its missing privilege first.
const MYSQL_SERVER_BLOCKED: &[Row] = &[
    (
        "M1",
        "/*!50000 DROP TABLE hv_t23_bypass */",
        &[MY_RO, MY_TABLE_PRIV],
    ),
    (
        "M1",
        "/*!50000 INSERT INTO hv_t23_bypass VALUES (50) */",
        &[MY_RO, MY_TABLE_PRIV],
    ),
    ("M2", "SELECT 1; DROP TABLE hv_t23_bypass", &["1064"]),
    (
        "M5",
        "SELECT * FROM hv_t23_bypass FOR UPDATE",
        &[MY_RO, MY_TABLE_PRIV],
    ),
    ("M5", "LOCK TABLES hv_t23_bypass WRITE", &["protocol"]),
    (
        "M6",
        "PREPARE hv_t23_s FROM 'DELETE FROM hv_t23_bypass'",
        &["protocol"],
    ),
    (
        "M8",
        "CREATE TABLE hv_t23_m8 (a int)",
        &[MY_RO, MY_TABLE_PRIV],
    ),
    (
        "M8",
        "TRUNCATE TABLE hv_t23_bypass",
        &[MY_RO, MY_TABLE_PRIV],
    ),
    ("M8", "DELETE FROM hv_t23_bypass", &[MY_RO, MY_TABLE_PRIV]),
    ("M12", "HANDLER hv_t23_bypass OPEN", &["protocol"]),
    ("M13", "XA START 'hv_t23'", &["protocol"]),
    (
        "M17",
        "CREATE TEMPORARY TABLE hv_t23_tmp (a int)",
        &[MY_RO, MY_DB_PRIV],
    ),
];

/// Rows only `hv_ro`'s missing privileges stop (L5), including T18's carries: `SET GLOBAL`,
/// `SET PERSIST`, `RESET PERSIST` (no implicit commit; administrative privileges) and a
/// `CALL` of a procedure that commits and writes (no `EXECUTE`). M4 M7 M10 M11.
const MYSQL_PRIVILEGE_ONLY: &[Row] = &[
    (
        "M4",
        "SELECT a FROM hv_t23_bypass INTO OUTFILE '/tmp/hv_t23_out'",
        &[MY_ACCESS, MY_PRIV, "1290"],
    ),
    // MySQL 8 ends the connection when an account without SYSTEM_VARIABLES_ADMIN sends
    // `SET GLOBAL`/`SET PERSIST` as a prepared statement (finding F2, `zz_docker_m2b_findings`):
    // the session sees a lost connection and reconnects; the side channel shows nothing changed.
    ("M10", "SET GLOBAL max_connections = 999", &[MY_PRIV, LOST]),
    (
        "M10",
        "SET PERSIST max_connections = 999",
        &[MY_PRIV, "1064", LOST],
    ),
    // 3615: the variable is not persisted; nothing to reset, nothing reset.
    (
        "M10",
        "RESET PERSIST max_connections",
        &[MY_PRIV, "1064", "3615"],
    ),
    ("M10", "FLUSH TABLES", &[MY_PRIV]),
    ("M10", "KILL {victim}", &["1095", "protocol"]),
    (
        "M11",
        "CREATE FUNCTION hv_t23_udf RETURNS STRING SONAME 'hv_t23.so'",
        &[MY_PRIV, MY_DB_PRIV, MY_RO, "1126", "protocol"],
    ),
    ("M7", "CALL hv_t23_escape()", &["1370"]),
];

/// Rows the server accepts, which the request's end neutralises (`ROLLBACK`,
/// `COM_RESET_CONNECTION` and the read-only setup again): M1 (a lock in an executable
/// comment) M3 M5 (`GET_LOCK`) M9 M14. `{tro}` is the engine's transaction read-only variable.
const MYSQL_NEUTRALISED: &[(&str, &str)] = &[
    (
        "M1",
        "SELECT 1 /*!50000 , (SELECT GET_LOCK('hv_t23_m1', 0)) */",
    ),
    ("M3", "SET sql_mode = 'NO_BACKSLASH_ESCAPES'"),
    ("M5", "SELECT GET_LOCK('hv_t23_m5', 0)"),
    ("M9", "SET SESSION TRANSACTION READ WRITE"),
    ("M9", "SET @@session.{tro} = 0"),
    ("M14", "SET @hv_t23 = 1"),
];

/// The name of the transaction read-only variable: `transaction_read_only` on MySQL 8 and
/// MariaDB 11.1+, `tx_read_only` on MariaDB 10.
fn tro(t: &DockerTarget) -> &'static str {
    if t.engine == DbmsKind::MariaDb && t.major.starts_with("10.") {
        "tx_read_only"
    } else {
        "transaction_read_only"
    }
}

// ---------------------------------------------------------------------------------------------
// Outcomes

/// What a row came to (recorded in ADR 0004; Q32: none may stay `Unknown`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum BypassResult {
    /// Stopped: a server code, `1a` or `protocol`.
    Blocked(String),
    /// It ran; L6 found the escape and discarded the connection.
    Gap,
    /// It ran, and nothing was left after the request.
    Neutralised,
    /// It ran and left an effect the side channel saw.
    Escaped(String),
    /// The outcome could not be read, or was not one the row allows.
    Unknown(String),
}

impl BypassResult {
    fn label(&self) -> String {
        match self {
            BypassResult::Blocked(code) => format!("blocked {code}"),
            BypassResult::Gap => "gap (L6, connection discarded)".into(),
            BypassResult::Neutralised => "ran, neutralised".into(),
            BypassResult::Escaped(what) => format!("ESCAPED: {what}"),
            BypassResult::Unknown(what) => format!("unknown: {what}"),
        }
    }
}

/// How a reply ended: a server code, `1a`, `protocol`, `gap`, `ran`, or the other kind.
fn outcome(reply: &Reply) -> String {
    match reply {
        Ok(_) => "ran".into(),
        Err(e) => match &e.kind {
            ExecErrorKind::Db => code_of(e.server.as_ref()).unwrap_or_else(|| "db".into()),
            ExecErrorKind::ReadOnlyDenied(DenyLayer::Transaction) => "1a".into(),
            ExecErrorKind::ReadOnlyDenied(DenyLayer::Protocol) => "protocol".into(),
            ExecErrorKind::EnforcementGap(_) => "gap".into(),
            ExecErrorKind::ConnectionLost { .. } => LOST.into(),
            other => format!("{other:?} {:?}", code_of(e.server.as_ref())),
        },
    }
}

/// Whether `got` is a stop: a server code (an SQLSTATE or a vendor code), `1a` or `protocol`.
fn is_code(got: &str) -> bool {
    !got.is_empty() && got.chars().all(|c| c.is_ascii_alphanumeric()) && got != "ran"
}

/// What one row came to, whatever it was expected to do: `Escaped` when the side channel
/// changed, `Gap`, `Neutralised` (it ran and left nothing), `Blocked` (a server code, `1a`,
/// `protocol`), else `Unknown`.
fn judge(reply: &Reply, before: &str, after: &str) -> BypassResult {
    if before != after {
        return BypassResult::Escaped(format!("side channel changed: {before} → {after}"));
    }
    let got = outcome(reply);
    match got.as_str() {
        "gap" => BypassResult::Gap,
        "ran" => BypassResult::Neutralised,
        _ if is_code(&got) => BypassResult::Blocked(got),
        _ => BypassResult::Unknown(got),
    }
}

/// Whether a stopped row's result is one its row allows.
fn stopped(result: &BypassResult, allowed: &[&str]) -> bool {
    match result {
        BypassResult::Blocked(code) => allowed.contains(&code.as_str()),
        BypassResult::Gap => allowed.contains(&"gap"),
        _ => false,
    }
}

/// Judges a neutralised row: it ran (or the server refused it) and left nothing.
fn judge_neutralised(
    reply: &Reply,
    before: &str,
    after: &str,
    lingering: &[String],
) -> BypassResult {
    if before != after {
        return BypassResult::Escaped(format!("side channel changed: {before} → {after}"));
    }
    if !lingering.is_empty() {
        return BypassResult::Escaped(format!("lingering: {}", lingering.join("; ")));
    }
    let got = outcome(reply);
    if got == "ran" {
        BypassResult::Neutralised
    } else if is_code(&got) {
        BypassResult::Blocked(got)
    } else {
        BypassResult::Unknown(got)
    }
}

/// One target's recorded rows and what failed.
#[derive(Default)]
struct Record {
    /// `(user, id, statement, result)`.
    rows: Vec<(String, String, String, BypassResult)>,
    failures: Vec<String>,
}

impl Record {
    /// Records a row; `ok` says whether its result is what the suite requires.
    fn row(
        &mut self,
        t: &DockerTarget,
        user: &str,
        id: &str,
        sql: &str,
        result: BypassResult,
        ok: bool,
    ) {
        if !ok {
            self.failures
                .push(format!("{} {user} {id} `{sql}`: {}", t.id, result.label()));
        }
        self.rows.push((user.into(), id.into(), sql.into(), result));
    }

    fn fail(&mut self, what: String) {
        self.failures.push(what);
    }

    /// Writes `t23-bypass-<target>.tsv` (`user`, `id`, `result`, `statement`).
    fn write(&self, t: &DockerTarget) {
        let mut text = format!("# {} {}\n", t.id, t.major);
        for (user, id, sql, result) in &self.rows {
            let _ = writeln!(text, "{user}\t{id}\t{}\t{sql}", result.label());
        }
        let path = hacivat_testkit::artifact(&format!("t23-bypass-{}.tsv", t.id));
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).expect("the artifact directory");
        }
        std::fs::write(&path, text).expect("the bypass record is written");
        eprintln!("bypass results: {}", path.display());
    }
}

// ---------------------------------------------------------------------------------------------
// Read-only sessions

/// One run at a time per target: the direct and the SSH legs share its fixtures (the tests of
/// this binary run in parallel).
async fn fixture_lock(t: &DockerTarget) -> tokio::sync::OwnedMutexGuard<()> {
    static LOCKS: std::sync::Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<()>>>> =
        std::sync::Mutex::new(BTreeMap::new());
    let lock = LOCKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .entry(t.id.clone())
        .or_default()
        .clone();
    lock.lock_owned().await
}

/// A row's statement as the record shows it, the same on every target: the victim's backend
/// (it differs per run) by its role, and the engine's transaction read-only variable by
/// MySQL's name (MariaDB 10 runs `tx_read_only`).
fn shown(sql: &str) -> String {
    sql.replace("{victim}", "<admin's backend>")
        .replace("{tro}", "transaction_read_only")
}

/// The record's user column: the user, and the path when it is not direct.
fn leg(user: &str, path: Path) -> String {
    match path {
        Path::Direct => user.to_string(),
        Path::Ssh(b) => format!("{user} via {b}"),
        Path::Chain(a, b) => format!("{user} via {a}, {b}"),
    }
}

/// A read-only default session of `t` as `user` on `db`, on its own M2b manager, that comes
/// back after a gap discarded its connection.
struct Ro {
    on: OnManager,
}

impl Ro {
    async fn open(t: &DockerTarget, user: &str, db: &str) -> Ro {
        Self::open_on(t, Path::Direct, user, db).await
    }

    /// [`Ro::open`] on `path` (through SSH: the in-network endpoint, the bastion's plain key).
    async fn open_on(t: &DockerTarget, path: Path, user: &str, db: &str) -> Ro {
        let mut p: Profile = profile_on(t, path, mode_on(t, path))
            .unwrap_or_else(|| panic!("{path:?}: a bastion is not in the active set"))
            .user(user)
            .database(db);
        p.read_only = true;
        p.allow_unsupported_server = true;
        p.application_name = unique("hv_t23_ro");
        let on = OnManager::connect(matrix_deps(), p).await;
        Ro { on }
    }

    fn app(&self) -> &str {
        &self.on.profile.application_name
    }

    fn label(&self) -> Option<AccessLabel> {
        match self.on.m.state(self.on.id()) {
            LiveState::Connected(info) => Some(info.access),
            _ => None,
        }
    }

    async fn run(&self, sql: &str) -> Reply {
        run(&self.on.session(), sql).await
    }

    /// Runs `sql`. After a gap (the connection was discarded) the profile connects again, as
    /// the capsule's Retry does; after a lost connection the session's own recovery is waited
    /// for (and a Retry made if it ended `Lost`), so the next row starts on a live session.
    async fn try_row(&self, sql: &str) -> Reply {
        let reply = self.run(sql).await;
        let ended = matches!(
            &reply,
            Err(e) if matches!(
                e.kind,
                ExecErrorKind::EnforcementGap(_) | ExecErrorKind::ConnectionLost { .. }
            )
        );
        if ended {
            let (m, id) = (&self.on.m, self.on.id());
            if matches!(&reply, Err(e) if matches!(e.kind, ExecErrorKind::EnforcementGap(_))) {
                wait_state_of(m, id, secs(10), |s| !matches!(s, LiveState::Connected(_))).await;
            }
            let state = wait_state_of(m, id, CONNECT_LIMIT, |s| {
                !matches!(s, LiveState::Reconnecting { .. } | LiveState::Connecting(_))
            })
            .await;
            if !matches!(state, LiveState::Connected(_)) {
                connect_on(m, &self.on.profile).await;
            }
        }
        reply
    }

    /// The checks (each a statement that answers one row when all is well) that do not: they
    /// show what a row left behind in the session.
    async fn lingering(&self, checks: &[String]) -> Vec<String> {
        let mut found = Vec::new();
        for check in checks {
            match self.run(check).await {
                Ok(summary) if summary.row_count == 1 => {}
                Ok(summary) => found.push(format!("{check} → {} rows", summary.row_count)),
                Err(e) => found.push(format!("{check} → {}", outcome(&Err(e)))),
            }
        }
        found
    }

    async fn close(self) {
        self.on.shutdown().await;
    }
}

// ---------------------------------------------------------------------------------------------
// PostgreSQL

const PG_DB: &str = "hv_t23";

/// `hv_t23` on `t`, made afresh: the fixture table, sequence, security-definer writer,
/// committing procedure and large object, `hv_ro` able to read them, TEMP and `CREATE` on
/// `public` revoked from `PUBLIC` as the matrix does.
async fn pg_fixture(t: &DockerTarget) {
    let mut a = admin(t).await;
    batch(
        &mut a,
        &t.id,
        &format!("DROP DATABASE IF EXISTS {PG_DB} WITH (FORCE)"),
    )
    .await;
    batch(&mut a, &t.id, &format!("CREATE DATABASE {PG_DB}")).await;
    let password = t.password.replace('\'', "''''");
    batch(
        &mut a,
        &t.id,
        &format!(
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'hv_ro') THEN \
             EXECUTE 'CREATE ROLE hv_ro LOGIN PASSWORD ''{password}'''; END IF; END $$"
        ),
    )
    .await;
    a.close().await;
    let mut d = admin_on(t, PG_DB).await;
    for sql in [
        format!("REVOKE TEMP ON DATABASE {PG_DB} FROM PUBLIC"),
        "REVOKE CREATE ON SCHEMA public FROM PUBLIC".into(),
        "CREATE TABLE hv_t23 (a int)".into(),
        "INSERT INTO hv_t23 VALUES (1), (2), (3)".into(),
        "CREATE SEQUENCE hv_t23_seq".into(),
        "CREATE FUNCTION hv_t23_sd_write() RETURNS int LANGUAGE sql SECURITY DEFINER \
         AS $$ INSERT INTO hv_t23 VALUES (99) RETURNING a $$"
            .into(),
        "CREATE FUNCTION hv_t23_dyn_write() RETURNS void LANGUAGE plpgsql \
         AS $$ BEGIN EXECUTE 'INSERT INTO hv_t23 VALUES (98)'; END $$"
            .into(),
        "CREATE PROCEDURE hv_t23_commit() LANGUAGE plpgsql \
         AS $$ BEGIN COMMIT; INSERT INTO hv_t23 VALUES (8); COMMIT; END $$"
            .into(),
        format!("SELECT lo_from_bytea({LO}, '\\x6876')"),
        // PostgreSQL 13 has no `pg_read_all_data`.
        "GRANT SELECT ON ALL TABLES IN SCHEMA public TO hv_ro".into(),
    ] {
        batch(&mut d, &t.id, &sql).await;
    }
    d.close().await;
}

async fn pg_drop_fixture(t: &DockerTarget) {
    let mut a = admin(t).await;
    batch(
        &mut a,
        &t.id,
        &format!("DROP DATABASE IF EXISTS {PG_DB} WITH (FORCE)"),
    )
    .await;
    a.close().await;
}

/// What the admin side channel sees in `hv_t23`: the table's rows, the relations of `public`,
/// the sequence, the large objects and the fixture's content, replication slots, prepared
/// transactions, temporary relations, and the advisory locks of session `app`.
async fn pg_snapshot(admin: &mut LiveConnection, app: &str) -> String {
    scalar(
        admin,
        &format!(
            "SELECT concat_ws('|', \
             (SELECT count(*) || ':' || coalesce(sum(a), 0) FROM hv_t23), \
             (SELECT string_agg(relname, ',' ORDER BY relname) FROM pg_class \
              WHERE relnamespace = 'public'::regnamespace), \
             (SELECT last_value || ':' || is_called FROM hv_t23_seq), \
             (SELECT count(*) FROM pg_largeobject_metadata), \
             (SELECT encode(lo_get({LO}), 'hex')), \
             (SELECT count(*) FROM pg_replication_slots), \
             (SELECT count(*) FROM pg_prepared_xacts), \
             (SELECT count(*) FROM pg_class WHERE relpersistence = 't'), \
             (SELECT count(*) FROM pg_locks l JOIN pg_stat_activity s ON s.pid = l.pid \
              WHERE l.locktype = 'advisory' AND s.application_name = '{app}'))"
        ),
    )
    .await
}

/// After a neutralised row nothing of it may be left in the session: no channel, cursor or
/// prepared statement, the read-only default, the lexical settings and `search_path` as they
/// were. Each check answers one row when all is well.
fn pg_linger_checks(search_path: &str) -> Vec<String> {
    vec![
        "SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM pg_listening_channels())".into(),
        // The protocol's own portal and prepared statements (the check's) are not counted.
        "SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM pg_cursors WHERE name <> '')".into(),
        "SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM pg_prepared_statements WHERE from_sql)".into(),
        "SELECT 1 WHERE current_setting('default_transaction_read_only') = 'on'".into(),
        "SELECT 1 WHERE current_setting('standard_conforming_strings') = 'on'".into(),
        format!(
            "SELECT 1 WHERE current_setting('search_path') = '{}'",
            search_path.replace('\'', "''")
        ),
    ]
}

/// The server-blocked PostgreSQL rows on `t` over `path`, as `hv_ro` and as the superuser.
async fn pg_server_blocked(
    t: &DockerTarget,
    path: Path,
    a: &mut LiveConnection,
    record: &mut Record,
) {
    for user in ["hv_ro", t.user.as_str()] {
        let ro = Ro::open_on(t, path, user, PG_DB).await;
        let baseline = pg_snapshot(a, ro.app()).await;
        let who = leg(user, path);
        for (id, sql, allowed) in PG_SERVER_BLOCKED {
            let reply = ro.try_row(sql).await;
            let after = pg_snapshot(a, ro.app()).await;
            let result = judge(&reply, &baseline, &after);
            let ok = stopped(&result, allowed);
            record.row(t, &who, id, sql, result, ok);
        }
        ro.close().await;
    }
}

/// The PostgreSQL rows on `t`: server-blocked as `hv_ro` and as the superuser, directly and
/// (the `pr` line) through `sshd-a`; neutralised and privilege-only as `hv_ro`.
async fn pg_bypass_rows(t: &DockerTarget) -> Record {
    let _one = fixture_lock(t).await;
    pg_fixture(t).await;
    let mut record = Record::default();
    let mut a = admin_on(t, PG_DB).await;
    let search_path = scalar(&mut a, "SHOW search_path").await;
    let checks = pg_linger_checks(&search_path);

    pg_server_blocked(t, Path::Direct, &mut a, &mut record).await;
    let ssh = Path::Ssh("sshd-a");
    if in_pr_set(t) && ssh.available() {
        pg_server_blocked(t, ssh, &mut a, &mut record).await;
    }

    let ro = Ro::open(t, "hv_ro", PG_DB).await;
    let label = ro.label();
    if label != Some(AccessLabel::ReadOnlyEnforced) {
        record.fail(format!("{} hv_ro: the label is {label:?}", t.id));
    }
    let baseline = pg_snapshot(&mut a, ro.app()).await;
    for (id, sql) in PG_NEUTRALISED {
        let reply = ro.try_row(sql).await;
        let mut lingering = ro.lingering(&checks).await;
        // A write is still refused by the read-only transaction.
        let write = outcome(&ro.run("INSERT INTO hv_t23 VALUES (7)").await);
        if write != PG_RO {
            lingering.push(format!("a write after it → {write}"));
        }
        let after = pg_snapshot(&mut a, ro.app()).await;
        let result = judge_neutralised(&reply, &baseline, &after, &lingering);
        let ok = matches!(result, BypassResult::Neutralised | BypassResult::Blocked(_));
        record.row(t, "hv_ro", id, sql, result, ok);
    }

    // A backend of another role for P11: the admin's own.
    let victim = scalar(&mut a, "SELECT pg_backend_pid()").await;
    for (id, sql, allowed) in PG_PRIVILEGE_ONLY {
        let run_sql = sql.replace("{victim}", &victim);
        let reply = ro.try_row(&run_sql).await;
        let after = pg_snapshot(&mut a, ro.app()).await;
        let result = judge(&reply, &baseline, &after);
        let ok = stopped(&result, allowed);
        record.row(t, "hv_ro", id, &shown(sql), result, ok);
    }
    // P11's victim is alive (the side channel itself).
    assert_eq!(scalar(&mut a, "SELECT 1").await, "1", "{} P11", t.id);
    ro.close().await;
    a.close().await;
    pg_drop_fixture(t).await;
    record
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_bypass_rows_blocked_without_classifier() {
    if !docker_enabled() {
        return;
    }
    // pg13–pg18 of the active set (Q6); the TLS, auth and Turkish twins are pg17 again.
    let lines: Vec<DockerTarget> = targets(Some(DbmsKind::Postgres))
        .into_iter()
        .filter(|t| !t.id.contains('-'))
        .collect();
    assert!(!lines.is_empty(), "the active set has no PostgreSQL line");
    let mut failures = Vec::new();
    for t in lines {
        let record = pg_bypass_rows(&t).await;
        record.write(&t);
        failures.extend(record.failures);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// ---------------------------------------------------------------------------------------------
// MySQL and MariaDB

/// The MySQL fixtures in the test database, made afresh: the table, and two procedures that
/// commit, open a read-write transaction, write and commit — one ending there, one inside a
/// new read-only transaction (T18 review I-2).
async fn mysql_fixture(t: &DockerTarget) {
    let mut a = admin(t).await;
    for sql in [
        "DROP PROCEDURE IF EXISTS hv_t23_escape",
        "DROP PROCEDURE IF EXISTS hv_t23_escape_ro",
        "DROP TABLE IF EXISTS hv_t23_bypass, hv_t23_m8, hv_t23_m9, hv_m9",
        "CREATE TABLE hv_t23_bypass (a int)",
        "INSERT INTO hv_t23_bypass VALUES (1), (2), (3)",
        "CREATE PROCEDURE hv_t23_escape() BEGIN COMMIT; START TRANSACTION READ WRITE; \
         INSERT INTO hv_t23_bypass VALUES (100); COMMIT; END",
        "CREATE PROCEDURE hv_t23_escape_ro() BEGIN COMMIT; START TRANSACTION READ WRITE; \
         INSERT INTO hv_t23_bypass VALUES (200); COMMIT; START TRANSACTION READ ONLY; END",
    ] {
        batch(&mut a, &t.id, sql).await;
    }
    a.close().await;
}

async fn mysql_drop_fixture(t: &DockerTarget) {
    let mut a = admin(t).await;
    for sql in [
        "DROP PROCEDURE IF EXISTS hv_t23_escape",
        "DROP PROCEDURE IF EXISTS hv_t23_escape_ro",
        "DROP TABLE IF EXISTS hv_t23_bypass, hv_t23_m8, hv_t23_m9, hv_m9",
    ] {
        batch(&mut a, &t.id, sql).await;
    }
    a.close().await;
}

/// What the admin side channel sees: the table's rows, the test database's tables and
/// routines, the two user locks, and the global variables the rows try to change.
async fn mysql_snapshot(admin: &mut LiveConnection, t: &DockerTarget) -> String {
    scalar(
        admin,
        &format!(
            "SELECT CONCAT_WS('|', \
             (SELECT CONCAT(COUNT(*), ':', COALESCE(SUM(a), 0)) FROM hv_t23_bypass), \
             (SELECT GROUP_CONCAT(table_name ORDER BY table_name) FROM information_schema.tables \
              WHERE table_schema = '{db}'), \
             (SELECT COUNT(*) FROM information_schema.routines WHERE routine_schema = '{db}'), \
             IS_FREE_LOCK('hv_t23_m1'), IS_FREE_LOCK('hv_t23_m5'), \
             @@global.max_connections, @@global.read_only)",
            db = t.database
        ),
    )
    .await
}

/// After a neutralised row nothing of it may be left in the session: the read-only setting,
/// no user variable, the pinned `sql_mode`. Each check answers one row when all is well.
fn mysql_linger_checks(t: &DockerTarget) -> Vec<String> {
    vec![
        format!("SELECT 1 FROM DUAL WHERE @@session.{} = 1", tro(t)),
        "SELECT 1 FROM DUAL WHERE @hv_t23 IS NULL".into(),
        "SELECT 1 FROM DUAL WHERE @@session.sql_mode NOT LIKE '%NO_BACKSLASH_ESCAPES%'".into(),
    ]
}

/// The server-blocked MySQL-family rows on `t` in `ro` (`who`: the user and the path).
async fn mysql_server_blocked(
    t: &DockerTarget,
    ro: &Ro,
    who: &str,
    judged: bool,
    a: &mut LiveConnection,
    record: &mut Record,
) {
    let baseline = mysql_snapshot(a, t).await;
    for (id, sql, allowed) in MYSQL_SERVER_BLOCKED {
        let reply = ro.try_row(sql).await;
        let after = mysql_snapshot(a, t).await;
        let result = judge(&reply, &baseline, &after);
        let ok = if judged {
            stopped(&result, allowed)
        } else {
            !matches!(result, BypassResult::Unknown(_))
        };
        record.row(t, who, id, sql, result, ok);
    }
}

/// The MySQL-family rows on `t`. The server-blocked rows run directly and (the `pr` line)
/// through `sshd-a`. `judged` (MySQL): a row that is not stopped (or neutralised)
/// fails. Not judged (MariaDB): only a row without a result (`Unknown`) fails (Q32).
async fn mysql_bypass_rows(t: &DockerTarget, judged: bool) -> Record {
    let _one = fixture_lock(t).await;
    mysql_fixture(t).await;
    let mut record = Record::default();
    let mut a = admin(t).await;
    let required = |ok: bool, result: &BypassResult| {
        if judged {
            ok
        } else {
            !matches!(result, BypassResult::Unknown(_))
        }
    };
    let checks = mysql_linger_checks(t);

    let ssh = Path::Ssh("sshd-a");
    if in_pr_set(t) && ssh.available() {
        for user in ["hv_ro", t.user.as_str()] {
            let ro = Ro::open_on(t, ssh, user, &t.database).await;
            mysql_server_blocked(t, &ro, &leg(user, ssh), judged, &mut a, &mut record).await;
            ro.close().await;
        }
    }
    for user in ["hv_ro", t.user.as_str()] {
        let ro = Ro::open(t, user, &t.database).await;
        mysql_server_blocked(t, &ro, user, judged, &mut a, &mut record).await;
        // The brief's M9 and M5: a session set back to read-write is read-only again at the
        // next request (the test account has `CREATE`); a user lock does not survive it.
        let _ = ro.try_row("SET SESSION TRANSACTION READ WRITE").await;
        let m9 = ro.run("CREATE TABLE hv_m9 (a int)").await;
        let result = if m9.is_err() {
            BypassResult::Blocked(outcome(&m9))
        } else {
            BypassResult::Escaped("CREATE TABLE ran".into())
        };
        let ok = required(m9.is_err(), &result);
        record.row(
            t,
            user,
            "M9",
            "SET SESSION TRANSACTION READ WRITE, then CREATE TABLE",
            result,
            ok,
        );
        let _ = ro.try_row("SELECT GET_LOCK('hv_m5', 0)").await;
        let free = scalar(&mut a, "SELECT IS_FREE_LOCK('hv_m5')").await;
        let result = if free == "1" {
            BypassResult::Neutralised
        } else {
            BypassResult::Escaped("the lock is held".into())
        };
        let ok = required(free == "1", &result);
        record.row(t, user, "M5", "SELECT GET_LOCK('hv_m5', 0)", result, ok);
        ro.close().await;
    }

    let ro = Ro::open(t, "hv_ro", &t.database).await;
    let label = ro.label();
    let expected = if t.engine == DbmsKind::MySql {
        AccessLabel::ReadOnlyEnforced
    } else {
        // C19: MariaDB stays guarded whatever these rows show.
        AccessLabel::ReadOnlyGuarded
    };
    if label != Some(expected) {
        record.fail(format!("{} hv_ro: the label is {label:?}", t.id));
    }
    let baseline = mysql_snapshot(&mut a, t).await;
    for (id, sql) in MYSQL_NEUTRALISED {
        let run_sql = sql.replace("{tro}", tro(t));
        let reply = ro.try_row(&run_sql).await;
        let mut lingering = ro.lingering(&checks).await;
        let ddl = outcome(&ro.run("CREATE TABLE hv_t23_m9 (a int)").await);
        if ![MY_RO, MY_TABLE_PRIV].contains(&ddl.as_str()) {
            lingering.push(format!("DDL after it → {ddl}"));
        }
        let after = mysql_snapshot(&mut a, t).await;
        let result = judge_neutralised(&reply, &baseline, &after, &lingering);
        let ok = required(
            matches!(result, BypassResult::Neutralised | BypassResult::Blocked(_)),
            &result,
        );
        record.row(t, "hv_ro", id, &shown(sql), result, ok);
    }

    let victim = scalar(&mut a, "SELECT CONNECTION_ID()").await;
    for (id, sql, allowed) in MYSQL_PRIVILEGE_ONLY {
        let run_sql = sql.replace("{victim}", &victim);
        let reply = ro.try_row(&run_sql).await;
        let after = mysql_snapshot(&mut a, t).await;
        let result = judge(&reply, &baseline, &after);
        let ok = required(stopped(&result, allowed), &result);
        record.row(t, "hv_ro", id, &shown(sql), result, ok);
    }
    // M10's victim is alive (the side channel itself).
    assert_eq!(scalar(&mut a, "SELECT 1").await, "1", "{} M10 KILL", t.id);

    // T18 review n-3: an account may always change its own password, and L5 cannot prevent it;
    // in a guarded read-only session every form must be refused (`1792`, or `1227`) or caught
    // (a gap): MySQL's `USER()` form, the `CURRENT_USER()` one MariaDB also parses, and
    // `SET PASSWORD`.
    // The statement holds a password, so neither it nor its server message is printed. The
    // side channel: `hv_ro` still signs in with the matrix password.
    for form in [
        "ALTER USER USER() IDENTIFIED BY",
        "ALTER USER CURRENT_USER() IDENTIFIED BY",
        "SET PASSWORD =",
    ] {
        let changed = "hv_t23_changed";
        let reply = ro.try_row(&format!("{form} '{changed}'")).await;
        let got = outcome(&reply);
        let mut p = profile_for(t, auto_or_require(t)).user("hv_ro");
        p.read_only = true;
        let still = connect(&deps(), req(p), Interaction::Headless, token(), progress()).await;
        let result = match still {
            Ok(live) => {
                live.close().await;
                if got == "gap" {
                    BypassResult::Gap
                } else if is_code(&got) {
                    BypassResult::Blocked(got)
                } else {
                    BypassResult::Unknown(got)
                }
            }
            Err(_) => {
                restore_hv_ro_password(t, changed).await;
                BypassResult::Escaped("the password changed (restored)".into())
            }
        };
        let ok = required(
            matches!(&result, BypassResult::Gap)
                || matches!(&result, BypassResult::Blocked(code) if code == MY_RO || code == MY_PRIV),
            &result,
        );
        record.row(t, "hv_ro", "n-3", &format!("{form} '…'"), result, ok);
    }
    ro.close().await;

    // T18 review I-2: `CALL` of a procedure that commits, opens a read-write transaction, writes
    // and commits. `hv_ro` has no `EXECUTE` (M7 above). The test account has: L3 and L4 cannot
    // stop it (a procedure may override the session's read-only setting), L6 catches the one
    // that leaves no transaction (a gap, the connection discarded), and the one that ends inside
    // a new read-only transaction escapes every runtime layer. Only L5 is honest about it: the
    // account is labelled guarded. The writes are recorded as they are.
    let ro = Ro::open(t, &t.user, &t.database).await;
    let label = ro.label();
    if label != Some(AccessLabel::ReadOnlyGuarded) {
        record.fail(format!("{} {}: the label is {label:?}", t.id, t.user));
    }
    for (sql, value) in [
        ("CALL hv_t23_escape()", 100),
        ("CALL hv_t23_escape_ro()", 200),
    ] {
        let reply = ro.try_row(sql).await;
        let got = outcome(&reply);
        let wrote = count(
            &mut a,
            &format!("SELECT COUNT(*) FROM hv_t23_bypass WHERE a = {value}"),
        )
        .await;
        let result = match (got.as_str(), wrote) {
            ("gap", _) => BypassResult::Gap,
            (_, 0) if is_code(&got) => BypassResult::Blocked(got),
            (_, 0) => BypassResult::Unknown(got),
            _ => BypassResult::Escaped(format!("committed a row ({got}); the label is guarded")),
        };
        // The first must be caught; the second is the documented residual (L5 only).
        let ok = if value == 100 {
            required(result == BypassResult::Gap, &result)
        } else {
            !matches!(result, BypassResult::Unknown(_))
        };
        record.row(t, &t.user, "M7/I-2", sql, result, ok);
    }
    ro.close().await;
    a.close().await;
    mysql_drop_fixture(t).await;
    record
}

/// Puts `hv_ro`'s matrix password back after a row changed it to `changed` (as `hv_ro` itself,
/// on a read-write connection).
async fn restore_hv_ro_password(t: &DockerTarget, changed: &str) {
    let p = profile_for(t, auto_or_require(t)).user("hv_ro");
    let mut live = connect(
        &deps(),
        req_pw(p, changed),
        Interaction::Headless,
        token(),
        progress(),
    )
    .await
    .unwrap_or_else(|e| panic!("{}: hv_ro with the changed password: {:?}", t.id, e.class));
    let password = t.password.replace('\\', "\\\\").replace('\'', "''");
    batch(&mut live, &t.id, &format!("SET PASSWORD = '{password}'")).await;
    live.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn mysql_bypass_rows_blocked_without_classifier() {
    if !docker_enabled() {
        return;
    }
    let mysql = targets(Some(DbmsKind::MySql));
    assert!(!mysql.is_empty(), "the active set has no MySQL line");
    let mut failures = Vec::new();
    for t in mysql {
        let record = mysql_bypass_rows(&t, true).await;
        record.write(&t);
        failures.extend(record.failures);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn mariadb_bypass_rows_recorded() {
    if !docker_enabled() {
        return;
    }
    // Q32: every row has a result (none `Unknown`); ADR 0004 records them. The label stays
    // guarded (C19).
    let mut failures = Vec::new();
    for t in targets(Some(DbmsKind::MariaDb)) {
        let record = mysql_bypass_rows(&t, false).await;
        record.write(&t);
        failures.extend(record.failures);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// ---------------------------------------------------------------------------------------------
// SQLite (no server)

/// The files in `dir`, sorted.
fn files_in(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("the directory lists")
        .map(|e| {
            e.expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_bypass_rows_blocked_without_classifier() {
    // L1–L4, L8, L9 with no classifier: the read-only open, the hardening and the authorizer
    // (Task 11). Every path is absolute, so a file a row could create would land in `dir`.
    let dir = tempfile::tempdir().expect("a temp directory");
    let path = dir.path().join("db.sqlite");
    {
        let rw = OnManager::connect(deps(), sqlite_profile(&path)).await;
        for sql in ["CREATE TABLE t (a int)", "INSERT INTO t VALUES (1)"] {
            let reply = run(&rw.s, sql).await;
            assert!(reply.is_ok(), "{sql}: {:?}", reply.map_err(|e| e.kind));
        }
        rw.shutdown().await;
    }
    let before = std::fs::read(&path).expect("the database file");
    let mut p = sqlite_profile(&path);
    p.read_only = true;
    p.endpoint = hacivat_session::profile::ProfileEndpoint::File {
        path: path.clone(),
        create_if_missing: false,
    };
    let ro = OnManager::connect(deps(), p).await;
    let d = dir.path().display();
    let rows = [
        ("L9", "INSERT INTO t VALUES (2)".to_string()),
        ("L2", format!("ATTACH '{d}/n.db' AS n")),
        ("L2", format!("VACUUM INTO '{d}/c.db'")),
        ("L1", "PRAGMA query_only = 0".to_string()),
        ("L8", "CREATE TEMP TABLE x (a)".to_string()),
        ("L3", format!("SELECT writefile('{d}/w', 'x')")),
        ("L3", "SELECT load_extension('hv_t23')".to_string()),
        ("L4", "PRAGMA writable_schema = 1".to_string()),
        ("L9", "PRAGMA user_version = 7".to_string()),
        ("L9", "PRAGMA journal_mode = WAL".to_string()),
        ("L6", "BEGIN IMMEDIATE".to_string()),
    ];
    for (id, sql) in &rows {
        let reply = run(&ro.s, sql).await;
        assert!(reply.is_err(), "{id} `{sql}` ran");
        // The session stays usable after each refusal.
        let read = run(&ro.s, "SELECT count(*) FROM t").await;
        assert!(read.is_ok(), "{id}: {:?}", read.map_err(|e| e.kind));
    }
    ro.shutdown().await;
    // The side channel: no file was created, and the database is byte-identical.
    assert_eq!(files_in(dir.path()), ["db.sqlite"]);
    assert_eq!(std::fs::read(&path).expect("the database file"), before);
}
