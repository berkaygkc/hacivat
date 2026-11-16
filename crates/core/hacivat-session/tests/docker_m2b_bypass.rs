//! M2b conformance over Docker (Task 23; exit criterion E4; Review Focus 4): the SQL-SAFE-06
//! bypass catalogue against real servers with no classifier. Hacivat has none in M2b: the
//! session runs every read-only statement through the guard (Task 18) — the transaction-keyword
//! layer 1a, the refusal of calls no rollback undoes (1b, finding F1), the read-only
//! transaction (L3), the session settings (L4), the least-privilege
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
//!   MySQL's in `hv_t23_*` objects of the test database (dropped first and last). A row whose
//!   escape would leave something on the server (a replication slot, a server file, a global
//!   variable) is undone at teardown. The matrix volumes keep no trace (Q7).
//! - The fixture names are fixed: two runs at once from different worktrees would drop each
//!   other's fixtures. The Docker runs are serialised by the controller's lock wrapper
//!   (`target/docker/run-docker-tests.sh`).
//! - Every row of every target runs and is recorded before the test judges: one run shows the
//!   whole matrix. Each target's results are written to the Docker artifact
//!   `t23-bypass-<target>.tsv` (ADR 0004 `m2b-t23`). A statement that holds a password is never
//!   printed or recorded.
//! - MariaDB's results are recorded; its label stays guarded (C19). ## m3b-t18-safe06 (fix round
//!   1, m7): the rows of `mariadb.toml` are judged by their recorded outcome (`observed`); run
//!   B's inline rows (M9 with its follow-up, `GET_LOCK('hv_m5')`, n-3, M7/I-2) stay recorded
//!   only.
//! - `sqlite_bypass_rows_blocked_without_classifier` needs no server: it runs on every PR.
//! - ## m3b-t18-safe06: this is SQL-SAFE-06 run B. Its rows live in
//!   `crates/sql/sql-testkit/bypass/{pg,mysql,mariadb,sqlite}.toml` (the rows an account runs,
//!   by `kind`, in file order), read by path with no crate edge (P-M3b-13); the sessions still
//!   have no classifier (`ManagerConfig::m2b()`). MariaDB's file records each row's outcome
//!   here (`observed`, C19). Run A is `sql-safety/tests/bypass_run_a.rs`, run A′
//!   `hacivat-exec/tests/docker_m3b_bypass.rs`.

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
use hacivat_session::pipeline::{Interaction, LiveConnection, connect};
use hacivat_session::profile::Profile;
use hacivat_session::session::{DenyLayer, ExecErrorKind, Reply};
use hacivat_session::{ConnectionEvent, LiveState, RoleWarning};
use hacivat_testkit::{DockerTarget, targets};
use tokio::sync::broadcast;

// ---------------------------------------------------------------------------------------------
// Rows

/// PostgreSQL's read-only transaction refusal.
const PG_RO: &str = "25006";

/// The fixture large object (`lo_from_bytea(LO, 'hv')`).
const LO: u32 = 230_023;

/// L6 found the statement's effect after it ran (the canary saw a transaction ID, or the
/// transaction had ended) and discarded the connection. A row passes with it only when the side
/// channel is unchanged: the `ROLLBACK` undid the effect.
const GAP: &str = "gap";

// ## m2b-t23-f1
/// The guard's text layer refused a call no rollback undoes, before any I/O (1b).
const FUNCTION: &str = "function";

/// The connection was lost (the session recovered on its own).
const LOST: &str = "lost";

/// MySQL's read-only transaction refusal (`ER_CANT_EXECUTE_IN_READ_ONLY_TRANSACTION`).
const MY_RO: &str = "1792";
/// Table-level privilege refusals: `ER_TABLEACCESS_DENIED_ERROR`.
const MY_TABLE_PRIV: &str = "1142";

// ## m3b-t18-safe06 (named exception: the rows moved to `sql-testkit/bypass/*.toml`, read by
// path with no crate edge, P-M3b-13). Run B is unchanged: the same rows, in the same order, with
// the same outcomes allowed, and no classifier.

/// One SQL-SAFE-06 row: its id, the statement, and the outcomes that count as stopped — a
/// server code (the SQLSTATE on PostgreSQL, the vendor code on MySQL and MariaDB), `1a` (the
/// transaction-keyword layer, before any I/O), `protocol` (MySQL will not prepare it on a
/// read-only connection) or `gap` (L6 found the escape and discarded the connection).
struct Row {
    id: String,
    sql: String,
    allowed: Vec<String>,
    // ## m3b-t18-safe06 (fix round 1, m7)
    /// MariaDB: run B's recorded outcome per account (`hv_ro`, `owner`), as `BypassResult::label`
    /// writes it (C19, R-M3-58).
    observed: Vec<(String, String)>,
}

impl Row {
    fn allowed(&self) -> Vec<&str> {
        self.allowed.iter().map(String::as_str).collect()
    }

    // ## m3b-t18-safe06 (fix round 1, m7)
    /// MariaDB's judgement: the row ended as it was recorded for `who` (`hv_ro`, or the test
    /// account, possibly through SSH).
    fn as_recorded(&self, who: &str, result: &BypassResult) -> bool {
        let account = if who.starts_with("hv_ro") {
            "hv_ro"
        } else {
            "owner"
        };
        self.observed
            .iter()
            .any(|(a, o)| a == account && *o == result.label())
    }
}

/// The run B rows of `file` (`pg`, `mysql`, `mariadb`, `sqlite`) of `kind`
/// (`server_blocked`, `neutralised`, `privilege_only`, `non_transactional`), in file order: the
/// rows an account runs (`accounts` not empty), with their `server_off` outcomes.
fn rows(file: &str, kind: &str) -> Vec<Row> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../sql/sql-testkit/bypass")
        .join(format!("{file}.toml"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    let doc: toml_edit::DocumentMut = text.parse().unwrap_or_else(|e| panic!("{path:?}: {e}"));
    let strings = |v: Option<&toml_edit::Item>| -> Vec<String> {
        v.and_then(toml_edit::Item::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    doc["row"]
        .as_array_of_tables()
        .unwrap_or_else(|| panic!("{path:?}: no rows"))
        .iter()
        .filter(|r| r.get("kind").and_then(toml_edit::Item::as_str) == Some(kind))
        .filter(|r| !strings(r.get("accounts")).is_empty())
        .map(|r| Row {
            id: r["id"].as_str().expect("an id").to_string(),
            sql: r["sql"].as_str().expect("a statement").to_string(),
            allowed: strings(r.get("server_off")),
            observed: r
                .get("observed")
                .and_then(toml_edit::Item::as_inline_table)
                .map(|t| {
                    t.iter()
                        .filter_map(|(k, v)| v.as_str().map(|v| (k.to_string(), v.to_string())))
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect()
}

/// The MySQL-family rows of `t`: MariaDB's file for a MariaDB line (the same rows, with their
/// recorded outcomes, C19).
fn my_file(t: &DockerTarget) -> &'static str {
    if t.engine == DbmsKind::MariaDb {
        "mariadb"
    } else {
        "mysql"
    }
}

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
    /// It ran; L6 found the escape and discarded the connection, and the side channel is
    /// unchanged (the `ROLLBACK` undid it).
    Gap,
    /// L6 found it and discarded the connection, **but the change was committed** (the side
    /// channel saw it): detected, not stopped (review I2).
    CommittedGap(String),
    /// The connection ended (the session recovered on its own); nothing changed.
    Lost,
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
            BypassResult::CommittedGap(what) => {
                format!("committed; L6 gap (connection discarded): {what}")
            }
            BypassResult::Lost => "connection lost".into(),
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
            ExecErrorKind::ReadOnlyDenied(DenyLayer::Function) => FUNCTION.into(),
            ExecErrorKind::EnforcementGap(_) => "gap".into(),
            ExecErrorKind::ConnectionLost { .. } => LOST.into(),
            other => format!("{other:?} {:?}", code_of(e.server.as_ref())),
        },
    }
}

/// Whether `got` is a stop: a server code (an SQLSTATE or a vendor code), `1a` or `protocol`
/// (never `ran`, `gap` or `lost`, which have results of their own).
fn is_code(got: &str) -> bool {
    !got.is_empty()
        && got.chars().all(|c| c.is_ascii_alphanumeric())
        && !["ran", GAP, LOST].contains(&got)
}

/// What one row came to, whatever it was expected to do: `Escaped` when the side channel
/// changed, `Gap`, `Neutralised` (it ran and left nothing), `Blocked` (a server code, `1a`,
/// `protocol`), else `Unknown`.
fn judge(reply: &Reply, before: &str, after: &str) -> BypassResult {
    let got = outcome(reply);
    if before != after {
        let change = format!("side channel changed: {before} → {after}");
        return if got == GAP {
            BypassResult::CommittedGap(change)
        } else {
            BypassResult::Escaped(change)
        };
    }
    match got.as_str() {
        GAP => BypassResult::Gap,
        LOST => BypassResult::Lost,
        "ran" => BypassResult::Neutralised,
        _ if is_code(&got) => BypassResult::Blocked(got),
        _ => BypassResult::Unknown(got),
    }
}

/// Whether a stopped row's result is one its row allows (a committed gap never is).
fn stopped(result: &BypassResult, allowed: &[&str]) -> bool {
    match result {
        BypassResult::Blocked(code) => allowed.contains(&code.as_str()),
        BypassResult::Gap => allowed.contains(&GAP),
        BypassResult::Lost => allowed.contains(&LOST),
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
    if !lingering.is_empty() {
        return BypassResult::Escaped(format!("lingering: {}", lingering.join("; ")));
    }
    // A gap (committed or not) or a lost connection keeps its own result, which fails a
    // neutralised row.
    judge(reply, before, after)
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

    // ## m2b-t23-f1
    /// The role probe's warnings (empty when not connected).
    fn warnings(&self) -> Vec<RoleWarning> {
        match self.on.m.state(self.on.id()) {
            LiveState::Connected(info) => info.access_warnings.clone(),
            _ => Vec::new(),
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
// The lines

/// The lines of `engine` a bypass test runs (the twins with a `-` in their id are left out):
/// those of the active set. Under the full set every one of `expected` must be there (review
/// m5: a missing line fails, and a run of none fails); a line a narrower set leaves out skips
/// with its line on stderr (Q6).
fn lines_of(engine: DbmsKind, expected: &[&str]) -> Vec<DockerTarget> {
    let found: Vec<DockerTarget> = targets(Some(engine))
        .into_iter()
        .filter(|t| !t.id.contains('-'))
        .collect();
    let set = std::env::var("HACIVAT_DOCKER_SET").ok();
    for id in expected {
        if !found.iter().any(|t| t.id == *id) {
            assert!(
                !must_exist(set.as_deref()),
                "{id} is missing from the {} set",
                set.as_deref().unwrap_or("full")
            );
            eprintln!(
                "skip {id}: not in set {}",
                set.as_deref().unwrap_or("<all>")
            );
        }
    }
    assert!(
        !must_exist(set.as_deref()) || !found.is_empty(),
        "no {engine:?} line ran"
    );
    found
}

// ---------------------------------------------------------------------------------------------
// PostgreSQL

const PG_DB: &str = "hv_t23";

/// `hv_t23` on `t`, made afresh: the fixture table, sequence, security-definer writer,
/// committing procedure and large object, `hv_ro` able to read them, TEMP and `CREATE` on
/// `public` revoked from `PUBLIC` as the matrix does.
async fn pg_fixture(t: &DockerTarget) {
    let mut a = admin(t).await;
    // What an interrupted earlier run may have left.
    pg_undo_escapes(t, &mut a).await;
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

/// Drops the fixture database and undoes what an escaped row would leave outside it; returns
/// what it had to undo (each one a failure: the row escaped).
async fn pg_drop_fixture(t: &DockerTarget) -> Vec<String> {
    let mut a = admin(t).await;
    let undone = pg_undo_escapes(t, &mut a).await;
    batch(
        &mut a,
        &t.id,
        &format!("DROP DATABASE IF EXISTS {PG_DB} WITH (FORCE)"),
    )
    .await;
    a.close().await;
    undone
}

/// The server files P14 (`COPY TO`) and P33 (`lo_export`) would write.
const PG_SERVER_FILES: [&str; 2] = ["/tmp/hv_t23_copy", "/tmp/hv_t23_lo"];

/// Undoes what an escaped row leaves outside the fixture database (review m6), as the
/// superuser: P31's replication slot (a slot keeps WAL forever, and `DROP DATABASE` does not
/// drop a physical one) and the server files of P14 and P33. Returns what it found.
async fn pg_undo_escapes(t: &DockerTarget, a: &mut LiveConnection) -> Vec<String> {
    let mut found = Vec::new();
    let slot = "SELECT count(*) FROM pg_replication_slots WHERE slot_name = 'hv_t23_slot'";
    if count(a, slot).await > 0 {
        batch(a, &t.id, "SELECT pg_drop_replication_slot('hv_t23_slot')").await;
        found.push(format!(
            "{}: the replication slot hv_t23_slot (dropped)",
            t.id
        ));
    }
    for file in PG_SERVER_FILES {
        let exists = scalar(
            a,
            &format!("SELECT pg_stat_file('{file}', true) IS NOT NULL"),
        )
        .await;
        if exists == "t" {
            batch(
                a,
                &t.id,
                &format!("COPY (SELECT 1) TO PROGRAM 'rm -f {file}'"),
            )
            .await;
            found.push(format!("{}: the server file {file} (removed)", t.id));
        }
    }
    found
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
    let server_blocked = rows("pg", "server_blocked");
    for user in ["hv_ro", t.user.as_str()] {
        let ro = Ro::open_on(t, path, user, PG_DB).await;
        let baseline = pg_snapshot(a, ro.app()).await;
        let who = leg(user, path);
        for row in &server_blocked {
            let reply = ro.try_row(&row.sql).await;
            let after = pg_snapshot(a, ro.app()).await;
            let result = judge(&reply, &baseline, &after);
            let ok = stopped(&result, &row.allowed());
            record.row(t, &who, &row.id, &row.sql, result, ok);
        }
        ro.close().await;
    }
}

// ## m2b-t23-f1
/// Task 23 finding F1 on `t`, in `hv_t23` as created (`PUBLIC` holds `EXECUTE` on
/// `pg_logical_emit_message`, every cluster's default): `hv_ro` reads guarded with
/// `NonTransactionalWrite`, and the guard refuses every spelling of the call before any I/O.
/// Then the remediation's optional `REVOKE` for this version (its exact line, so the run also
/// checks that it is valid here), after which `hv_ro` reads enforced, as the rest of the suite
/// asserts.
async fn pg_non_transactional_rows(t: &DockerTarget, a: &mut LiveConnection, record: &mut Record) {
    let ro = Ro::open(t, "hv_ro", PG_DB).await;
    let (label, warnings) = (ro.label(), ro.warnings());
    if label != Some(AccessLabel::ReadOnlyGuarded)
        || !warnings.contains(&RoleWarning::NonTransactionalWrite)
    {
        record.fail(format!(
            "{} hv_ro with EXECUTE on pg_logical_emit_message: {label:?} {warnings:?}",
            t.id
        ));
    }
    let baseline = pg_snapshot(a, ro.app()).await;
    for row in rows("pg", "non_transactional") {
        let reply = ro.try_row(&row.sql).await;
        let after = pg_snapshot(a, ro.app()).await;
        let result = judge(&reply, &baseline, &after);
        let ok = stopped(&result, &row.allowed());
        record.row(t, "hv_ro (PUBLIC EXECUTE)", &row.id, &row.sql, result, ok);
    }
    ro.close().await;
    batch(a, &t.id, &revoke_emit_message(t)).await;
    let revoked = Ro::open(t, "hv_ro", PG_DB).await;
    if revoked
        .warnings()
        .contains(&RoleWarning::NonTransactionalWrite)
    {
        record.fail(format!(
            "{} hv_ro after the remediation's REVOKE: {:?}",
            t.id,
            revoked.warnings()
        ));
    }
    revoked.close().await;
}

/// The PostgreSQL rows on `t`: server-blocked as `hv_ro` and as the superuser, directly and
/// (the `pr` line) through `sshd-a`; neutralised and privilege-only as `hv_ro`. First (F1) the
/// calls no rollback undoes, while `PUBLIC` may still run `pg_logical_emit_message`.
async fn pg_bypass_rows(t: &DockerTarget) -> Record {
    let _one = fixture_lock(t).await;
    pg_fixture(t).await;
    let mut record = Record::default();
    let mut a = admin_on(t, PG_DB).await;
    let search_path = scalar(&mut a, "SHOW search_path").await;
    let checks = pg_linger_checks(&search_path);

    pg_non_transactional_rows(t, &mut a, &mut record).await;

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
    for Row { id, sql, .. } in &rows("pg", "neutralised") {
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
    for row in rows("pg", "privilege_only") {
        let run_sql = row.sql.replace("{victim}", &victim);
        let reply = ro.try_row(&run_sql).await;
        let after = pg_snapshot(&mut a, ro.app()).await;
        let result = judge(&reply, &baseline, &after);
        let ok = stopped(&result, &row.allowed());
        record.row(t, "hv_ro", &row.id, &shown(&row.sql), result, ok);
    }
    // P11's victim is alive (the side channel itself).
    assert_eq!(scalar(&mut a, "SELECT 1").await, "1", "{} P11", t.id);
    ro.close().await;
    a.close().await;
    for undone in pg_drop_fixture(t).await {
        record.fail(format!("escaped: {undone}"));
    }
    record
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_bypass_rows_blocked_without_classifier() {
    if !docker_enabled() {
        return;
    }
    // pg13–pg18 (Q6); the TLS, auth and Turkish twins are pg17 again.
    let lines = lines_of(
        DbmsKind::Postgres,
        &["pg13", "pg14", "pg15", "pg16", "pg17", "pg18"],
    );
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
    for row in rows(my_file(t), "server_blocked") {
        let reply = ro.try_row(&row.sql).await;
        let after = mysql_snapshot(a, t).await;
        let result = judge(&reply, &baseline, &after);
        // ## m3b-t18-safe06 (fix round 1, m7): MariaDB is judged by its recorded outcome.
        let ok = if judged {
            stopped(&result, &row.allowed())
        } else {
            row.as_recorded(who, &result)
        };
        record.row(t, who, &row.id, &row.sql, result, ok);
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
    let max_connections = scalar(&mut a, "SELECT @@global.max_connections").await;
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
        // Judged by the side channel (review m1): `hv_m9` must not exist afterwards.
        let _ = ro.try_row("SET SESSION TRANSACTION READ WRITE").await;
        let before = mysql_snapshot(&mut a, t).await;
        let m9 = ro.try_row("CREATE TABLE hv_m9 (a int)").await;
        let after = mysql_snapshot(&mut a, t).await;
        let result = judge(&m9, &before, &after);
        let ok = required(stopped(&result, &[MY_RO, MY_TABLE_PRIV]), &result);
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
    for row in &rows(my_file(t), "neutralised") {
        let (id, sql) = (&row.id, &row.sql);
        let run_sql = sql.replace("{tro}", tro(t));
        let reply = ro.try_row(&run_sql).await;
        let mut lingering = ro.lingering(&checks).await;
        let ddl = outcome(&ro.run("CREATE TABLE hv_t23_m9 (a int)").await);
        if ![MY_RO, MY_TABLE_PRIV].contains(&ddl.as_str()) {
            lingering.push(format!("DDL after it → {ddl}"));
        }
        let after = mysql_snapshot(&mut a, t).await;
        let result = judge_neutralised(&reply, &baseline, &after, &lingering);
        // ## m3b-t18-safe06 (fix round 1, m7): MariaDB is judged by its recorded outcome.
        let ok = if judged {
            matches!(result, BypassResult::Neutralised | BypassResult::Blocked(_))
        } else {
            row.as_recorded("hv_ro", &result)
        };
        record.row(t, "hv_ro", id, &shown(sql), result, ok);
    }

    let victim = scalar(&mut a, "SELECT CONNECTION_ID()").await;
    for row in rows(my_file(t), "privilege_only") {
        let run_sql = row.sql.replace("{victim}", &victim);
        let reply = ro.try_row(&run_sql).await;
        let after = mysql_snapshot(&mut a, t).await;
        let result = judge(&reply, &baseline, &after);
        // ## m3b-t18-safe06 (fix round 1, m7): MariaDB is judged by its recorded outcome.
        let ok = if judged {
            stopped(&result, &row.allowed())
        } else {
            row.as_recorded("hv_ro", &result)
        };
        record.row(t, "hv_ro", &row.id, &shown(&row.sql), result, ok);
    }
    // M10's victim is alive (the side channel itself).
    assert_eq!(scalar(&mut a, "SELECT 1").await, "1", "{} M10 KILL", t.id);

    // T18 review n-3: an account may always change its own password, and L5 cannot prevent it;
    // in a guarded read-only session every form must be refused (`1792`) or caught (a gap):
    // MySQL's `USER()` form, the `CURRENT_USER()` one MariaDB also parses, and `SET PASSWORD`.
    // The statement holds a password, so neither it nor its server message is printed. The
    // side channel: `hv_ro` still signs in with the matrix password.
    for form in [
        "ALTER USER USER() IDENTIFIED BY",
        "ALTER USER CURRENT_USER() IDENTIFIED BY",
        "SET PASSWORD =",
    ] {
        let changed = "hv_t23_changed";
        let reply = ro.try_row(&format!("{form} '{changed}'")).await;
        let mut p = profile_for(t, auto_or_require(t)).user("hv_ro");
        p.read_only = true;
        let still = connect(&deps(), req(p), Interaction::Headless, token(), progress()).await;
        let result = match still {
            Ok(live) => {
                live.close().await;
                // Unchanged: hv_ro still signs in with the matrix password.
                judge(&reply, "", "")
            }
            Err(_) => {
                restore_hv_ro_password(t, changed).await;
                BypassResult::Escaped("the password changed (restored)".into())
            }
        };
        let ok = required(stopped(&result, &[MY_RO, GAP]), &result);
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
        // The side channel decides (review I2): a gap after a committed row is detected, not
        // stopped.
        let result = match (got.as_str(), wrote) {
            (GAP, 0) => BypassResult::Gap,
            (GAP, n) => BypassResult::CommittedGap(format!("{n} row committed")),
            (_, 0) if is_code(&got) => BypassResult::Blocked(got),
            (_, 0) => BypassResult::Unknown(got),
            _ => BypassResult::Escaped(format!("committed a row ({got}); the label is guarded")),
        };
        // The first must be caught by L6 (stopped or detected); the second is the documented
        // residual (L5 only).
        let ok = if value == 100 {
            required(
                matches!(result, BypassResult::Gap | BypassResult::CommittedGap(_)),
                &result,
            )
        } else {
            !matches!(result, BypassResult::Unknown(_))
        };
        record.row(t, &t.user, "M7/I-2", sql, result, ok);
    }
    ro.close().await;
    // A `SET GLOBAL`/`SET PERSIST` that escaped is undone (review m6): only an account that could
    // change it can put it back, and `hv_ro` is the one that tried.
    let now = scalar(&mut a, "SELECT @@global.max_connections").await;
    if now != max_connections {
        restore_max_connections(t, &max_connections).await;
        record.fail(format!(
            "{}: max_connections escaped ({max_connections} → {now}; restored)",
            t.id
        ));
    }
    a.close().await;
    mysql_drop_fixture(t).await;
    record
}

/// Puts `max_connections` back to `value` and drops a persisted setting of it, as `hv_ro` on a
/// read-write connection (it changed it, so it may), over `COM_QUERY`.
async fn restore_max_connections(t: &DockerTarget, value: &str) {
    let p = profile_for(t, auto_or_require(t)).user("hv_ro");
    let mut live = connect(&deps(), req(p), Interaction::Headless, token(), progress())
        .await
        .unwrap_or_else(|e| panic!("{}: hv_ro to restore: {:?}", t.id, e.class));
    batch(
        &mut live,
        &t.id,
        &format!("SET GLOBAL max_connections = {value}"),
    )
    .await;
    if t.engine == DbmsKind::MySql {
        batch(&mut live, &t.id, "RESET PERSIST IF EXISTS max_connections").await;
    }
    live.close().await;
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
    let mut failures = Vec::new();
    for t in lines_of(DbmsKind::MySql, &["mysql80", "mysql84"]) {
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
    for t in lines_of(
        DbmsKind::MariaDb,
        &["mariadb106", "mariadb1011", "mariadb114"],
    ) {
        let record = mysql_bypass_rows(&t, false).await;
        record.write(&t);
        failures.extend(record.failures);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// ## m2b-fix-f2
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn mysql_refused_set_global_is_a_refusal_on_a_live_session() {
    // F2 (moved from `zz_docker_m2b_findings`; fixed in the MySQL driver). MySQL 8.0 and 8.4
    // answer the prepare of a `SET GLOBAL …`/`SET PERSIST …` that an account without
    // SYSTEM_VARIABLES_ADMIN sends with a bare OK packet, which `mysql_async` cannot read as a
    // prepare reply: the connection ended, the user was told "connection lost, outcome
    // unknown" for a statement the server refused, and the session reconnected. The driver now
    // runs every `SET` asked for as a prepared statement over `COM_QUERY`, where the server
    // answers `1227`: the refusal, with the server's message, on a session that stays up (no
    // `SessionLost`, the next statement runs), read-write and read-only.
    if !docker_enabled() {
        return;
    }
    let mut failures = Vec::new();
    for t in lines_of(DbmsKind::MySql, &["mysql80", "mysql84"]) {
        // The bypass rows may change `hv_ro`'s password for a moment (n-3).
        let _one = fixture_lock(&t).await;
        for read_only in [false, true] {
            let mut p = profile_for(&t, auto_or_require(&t)).user("hv_ro");
            p.read_only = read_only;
            let on = OnManager::connect(deps(), p).await;
            let mut events = on.m.events();
            for sql in [
                "SET GLOBAL max_connections = 999",
                "SET PERSIST max_connections = 999",
            ] {
                let reply = run(&on.s, sql).await;
                let refused = matches!(
                    &reply,
                    Err(e) if e.kind == ExecErrorKind::Db
                        && code_of(e.server.as_ref()).as_deref() == Some("1227")
                        && e.server.as_ref().is_some_and(|m| !m.text.is_empty())
                );
                if !refused {
                    failures.push(format!(
                        "{} read_only={read_only} `{sql}`: {:?}",
                        t.id,
                        reply.map(|s| s.row_count).map_err(|e| e.kind)
                    ));
                }
            }
            // The next statement runs on the same session, which never lost its connection.
            let next = run(&on.s, "SELECT 1").await;
            if let Err(e) = next {
                failures.push(format!("{} read_only={read_only}: {:?}", t.id, e.kind));
            }
            loop {
                match events.try_recv() {
                    Ok(ConnectionEvent::SessionLost { lost, .. }) => failures.push(format!(
                        "{} read_only={read_only}: the session was lost ({lost:?})",
                        t.id
                    )),
                    Ok(_) | Err(broadcast::error::TryRecvError::Lagged(_)) => {}
                    Err(_) => break,
                }
            }
            if !matches!(on.m.state(on.id()), LiveState::Connected(_)) {
                failures.push(format!(
                    "{} read_only={read_only}: {:?}",
                    t.id,
                    on.m.state(on.id())
                ));
            }
            on.shutdown().await;
        }
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
    // ## m3b-t18-safe06: the rows from `sql-testkit/bypass/sqlite.toml` (`{d}`: this directory).
    let rows: Vec<(String, String)> = rows("sqlite", "server_blocked")
        .into_iter()
        .map(|r| (r.id, r.sql.replace("{d}", &d.to_string())))
        .collect();
    assert_eq!(rows.len(), 11, "run B's SQLite rows");
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
