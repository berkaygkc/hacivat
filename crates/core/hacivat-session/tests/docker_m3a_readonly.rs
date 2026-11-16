//! M3a Task 22 over Docker: the typed read-only guard (HV-SESS-04 on `Request::Run`, R-M3-17)
//! against real servers. The server-blocked rows of the M2b bypass catalogue
//! (`docker_m2b_bypass.rs`, run B) are sent through `Run` instead of `Exec`, with no
//! classifier, as `hv_ro` and as the test account: each must end as it did there, and the
//! admin side channel must find nothing changed. A read-only PostgreSQL cursor is released by
//! its 20 s bound before the server's 30 s `idle_in_transaction_session_timeout` would end the
//! connection. Run with `<main>/target/docker/run-docker-tests.sh --set pr|full -p
//! hacivat-session --test docker_m3a_readonly`.
//!
//! - Targets come only from `targets(..)` and `require_target(..)` (Q6). Fixtures carry this
//!   task's names (`hv_t22…`; PostgreSQL's in a database of their own), are made afresh and
//!   dropped at the end (Q7).
//! - Waits are on conditions bounded by real time. The cursor test waits the real 20 s bound
//!   (the subject of the test); nothing asserts a wall-clock budget.
//! - One difference from run B, by design and pinned: `COPY` is refused by the typed
//!   PostgreSQL driver before anything is sent (Task 12), so P14 ends as `protocol` (run B:
//!   `25006`/`42501` from the server).
//! - Every row's outcome is written to the Docker artifact `t22-run-bypass.tsv`.
//! - Fix round 1 (review I1, M6): run B's neutralised and privilege-only rows through `Run`
//!   (`t22-run-neutralised.tsv`), and locks taken by a read-only cursor's rows released with it
//!   (PostgreSQL; MySQL and MariaDB, which also prove the real release keeps the connection).

#![allow(
    clippy::result_large_err,
    reason = "Reply is the session's fixed interface"
)]

mod common;

use std::sync::Arc;

use common::conformance::*;
use common::runs::*;
use hacivat_driver::TxStatus;
use hacivat_proto::DbmsKind;
use hacivat_results::{CloseReason, Completeness, FetchPolicy};
use hacivat_session::LiveState;
use hacivat_session::pipeline::LiveConnection;
use hacivat_session::profile::Profile;
use hacivat_session::session::{DenyLayer, ExecErrorKind, Reply, SessionHandle, TxOwner};
use hacivat_testkit::{DockerTarget, require_target, targets};

/// One row: its id in the catalogue, the statement, the outcomes that count as stopped (a
/// server code, `1a`, `protocol`, `function` or `gap`).
type Row = (&'static str, &'static str, &'static [&'static str]);

const PG_RO: &str = "25006";
const PG_PINNED: &str = "25001";
const PG_PRIV: &str = "42501";
const GAP: &str = "gap";
const PROTOCOL: &str = "protocol";
// ## m3a-f5-demo (DEMO-7): refused before anything is sent (a first keyword that changes data
// or schema).
const INPUT: &str = "input";

/// The fixture large object.
const LO: u32 = 220_022;

/// Run B's PostgreSQL server-blocked rows on this task's fixtures.
const PG_SERVER_BLOCKED: &[Row] = &[
    (
        "P1",
        "WITH d AS (DELETE FROM hv_t22 RETURNING *) SELECT * FROM d",
        &[PG_RO],
    ),
    ("P2", "SELECT * INTO hv_t22_p2 FROM hv_t22", &[PG_RO]),
    ("P3", "EXPLAIN (ANALYZE) DELETE FROM hv_t22", &[PG_RO]),
    ("P4", "SELECT 1; DROP TABLE hv_t22", &["42601"]),
    ("P4", "COMMIT; DROP TABLE hv_t22", &["1a"]),
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
    ("P8", "CALL hv_t22_commit()", &[INPUT]),
    (
        "P9",
        "DO $$ BEGIN INSERT INTO hv_t22 VALUES (9); END $$",
        &[INPUT],
    ),
    // Pinned difference from run B (`[PG_RO, PG_PRIV]` there): the typed PostgreSQL driver
    // refuses `COPY` before anything is sent (Task 12), so `Run` never reaches the server.
    ("P14", "COPY hv_t22 FROM '/etc/hostname'", &[PROTOCOL]),
    ("P26", "CREATE TEMP TABLE hv_t22_tmp (a int)", &[INPUT]),
    ("P37", "SELECT nextval('hv_t22_seq')", &[PG_RO, PG_PRIV]),
    ("P37", "SELECT setval('hv_t22_seq', 100)", &[PG_RO, PG_PRIV]),
    ("P38", "SELECT hv_t22_sd_write()", &[PG_RO]),
    ("P38", "SELECT hv_t22_dyn_write()", &[PG_RO]),
    (
        "P38",
        "SELECT query_to_xml('DELETE FROM hv_t22 RETURNING a', true, true, '')",
        &[PG_RO, "0A000"],
    ),
    ("P38", "SELECT lo_from_bytea(0, '\\x6876')", &[PG_RO, GAP]),
    ("P38", "SELECT lo_create(0)", &[PG_RO, GAP]),
    (
        "P38",
        "SELECT lo_put(220022, 0, '\\x00')",
        &[PG_RO, PG_PRIV, GAP],
    ),
    ("P38", "SELECT lo_unlink(220022)", &[PG_RO, PG_PRIV, GAP]),
    (
        "P38",
        "SELECT lo_import('/etc/hostname')",
        &[PG_RO, PG_PRIV, GAP],
    ),
    ("P35", "SELECT txid_current()", &[GAP]),
    (
        "P32",
        "SELECT pg_logical_emit_message(false, 'hv_t22', 'x')",
        &["function"],
    ),
];

const MY_RO: &str = "1792";
const MY_TABLE_PRIV: &str = "1142";

/// Run B's MySQL server-blocked rows on this task's fixture table.
const MYSQL_SERVER_BLOCKED: &[Row] = &[
    ("M1", "/*!50000 DROP TABLE hv_t22_bypass */", &[INPUT]),
    (
        "M1",
        "/*!50000 INSERT INTO hv_t22_bypass VALUES (50) */",
        &[INPUT],
    ),
    ("M2", "SELECT 1; DROP TABLE hv_t22_bypass", &["1064"]),
    (
        "M5",
        "SELECT * FROM hv_t22_bypass FOR UPDATE",
        &[MY_RO, MY_TABLE_PRIV],
    ),
    ("M5", "LOCK TABLES hv_t22_bypass WRITE", &[PROTOCOL]),
    (
        "M6",
        "PREPARE hv_t22_s FROM 'DELETE FROM hv_t22_bypass'",
        &[PROTOCOL],
    ),
    ("M8", "CREATE TABLE hv_t22_m8 (a int)", &[INPUT]),
    ("M8", "TRUNCATE TABLE hv_t22_bypass", &[INPUT]),
    ("M8", "DELETE FROM hv_t22_bypass", &[INPUT]),
    (
        "M8",
        "WITH x AS (SELECT 1 AS a) DELETE FROM hv_t22_bypass WHERE a = 1",
        &[MY_RO, MY_TABLE_PRIV],
    ),
    ("M12", "HANDLER hv_t22_bypass OPEN", &[PROTOCOL]),
    ("M13", "XA START 'hv_t22'", &[PROTOCOL]),
    ("M17", "CREATE TEMPORARY TABLE hv_t22_tmp (a int)", &[INPUT]),
    ("M7", "/*!50000 COMMIT */", &["1a"]),
];

// ## m3b-t18-safe06 (named exception: the C19 rows judged, R-M3-58)
/// MariaDB's server-blocked rows through `Run`: MySQL's, with the outcomes run B recorded for
/// MariaDB where it differs (`sql-testkit/bypass/mariadb.toml`, `observed`): `LOCK TABLES` is
/// caught by L6 (`gap`: its implicit commit ended the read-only transaction; the side channel
/// shows nothing persisted), `HANDLER … OPEN` runs and the reset closes it (`ran`, judged by the
/// side channel), `XA START` is refused by the server (`1400`). Judged since Task 18 (C19).
const MARIADB_SERVER_BLOCKED: &[Row] = &[
    ("M1", "/*!50000 DROP TABLE hv_t22_bypass */", &[INPUT]),
    (
        "M1",
        "/*!50000 INSERT INTO hv_t22_bypass VALUES (50) */",
        &[INPUT],
    ),
    ("M2", "SELECT 1; DROP TABLE hv_t22_bypass", &["1064"]),
    (
        "M5",
        "SELECT * FROM hv_t22_bypass FOR UPDATE",
        &[MY_RO, MY_TABLE_PRIV],
    ),
    ("M5", "LOCK TABLES hv_t22_bypass WRITE", &[PROTOCOL, GAP]),
    (
        "M6",
        "PREPARE hv_t22_s FROM 'DELETE FROM hv_t22_bypass'",
        &[PROTOCOL],
    ),
    ("M8", "CREATE TABLE hv_t22_m8 (a int)", &[INPUT]),
    ("M8", "TRUNCATE TABLE hv_t22_bypass", &[INPUT]),
    ("M8", "DELETE FROM hv_t22_bypass", &[INPUT]),
    // MariaDB has no `WITH … DELETE`: the server refuses its syntax (`1064`) before it runs.
    (
        "M8",
        "WITH x AS (SELECT 1 AS a) DELETE FROM hv_t22_bypass WHERE a = 1",
        &[MY_RO, MY_TABLE_PRIV, "1064"],
    ),
    ("M12", "HANDLER hv_t22_bypass OPEN", &[PROTOCOL, "ran"]),
    ("M13", "XA START 'hv_t22'", &[PROTOCOL, "1400"]),
    ("M17", "CREATE TEMPORARY TABLE hv_t22_tmp (a int)", &[INPUT]),
    ("M7", "/*!50000 COMMIT */", &["1a"]),
];

// ---------------------------------------------------------------------------------------------
// Outcomes

/// How a reply ended: a server code, `1a`, `protocol`, `function`, `gap`, `lost`, `ran`, or
/// the other kind.
fn outcome(reply: &Reply) -> String {
    match reply {
        Ok(_) => "ran".into(),
        Err(e) => match &e.kind {
            ExecErrorKind::Db => code_of(e.server.as_ref()).unwrap_or_else(|| "db".into()),
            ExecErrorKind::ReadOnlyDenied(DenyLayer::Transaction) => "1a".into(),
            ExecErrorKind::ReadOnlyDenied(DenyLayer::Input) => INPUT.into(),
            ExecErrorKind::ReadOnlyDenied(DenyLayer::Protocol) => PROTOCOL.into(),
            ExecErrorKind::ReadOnlyDenied(DenyLayer::Function) => "function".into(),
            ExecErrorKind::EnforcementGap(_) => GAP.into(),
            ExecErrorKind::ConnectionLost { .. } => "lost".into(),
            other => format!("{other:?} {:?}", code_of(e.server.as_ref())),
        },
    }
}

/// Whether a console would keep `sql` as a cursor (R-M3-23's read head, by its first keyword:
/// `SELECT`, `VALUES`, `TABLE`, or `WITH` without a data-changing verb). Both paths of the
/// guard run: the named portal with the canary after the first chunk, and `Execute(0)`.
fn read_head(sql: &str) -> bool {
    let upper = sql.to_ascii_uppercase();
    let first = upper.split_whitespace().next().unwrap_or_default();
    match first {
        "SELECT" | "VALUES" | "TABLE" => true,
        "WITH" => !["INSERT", "UPDATE", "DELETE", "MERGE"]
            .iter()
            .any(|verb| upper.contains(verb)),
        _ => false,
    }
}

/// A read-only default session of `t` as `user` on `db`, on its own manager, that comes back
/// after a gap discarded its connection (as run B's).
struct Ro {
    on: OnManager,
}

impl Ro {
    async fn open(t: &DockerTarget, user: &str, db: &str) -> Ro {
        let mut p: Profile = profile_on(t, Path::Direct, mode_on(t, Path::Direct))
            .expect("a direct profile")
            .user(user)
            .database(db);
        p.read_only = true;
        p.allow_unsupported_server = true;
        p.application_name = unique("hv_t22_ro");
        Ro {
            on: OnManager::connect(matrix_deps(), p).await,
        }
    }

    fn app(&self) -> &str {
        &self.on.profile.application_name
    }

    fn session(&self) -> SessionHandle {
        self.on.session()
    }

    /// Runs `sql` through `Request::Run` (one unit, suspendable when it is a read head).
    /// After a gap or a lost connection the profile is connected again, so the next row
    /// starts on a live session.
    async fn try_row(&self, sql: &str) -> Reply {
        self.try_row_with(sql, read_head(sql)).await.0
    }

    // ## fix round 1 (I1)
    /// The checks (each a statement that answers one row when all is well), run through `Run`
    /// as units that do not suspend (a named portal would count among `pg_cursors`), that do
    /// not: they show what a row left behind in the session.
    async fn lingering(&self, checks: &[String]) -> Vec<String> {
        let mut found = Vec::new();
        for check in checks {
            match self.try_row_with(check, false).await {
                (Ok(_), o) if o.rows() == 1 => {}
                (Ok(_), o) => found.push(format!("{check} → {} rows", o.rows())),
                (reply, _) => found.push(format!("{check} → {}", outcome(&reply))),
            }
        }
        found
    }

    /// [`Ro::try_row`] with the unit's `suspendable` flag given, and its recorder.
    async fn try_row_with(&self, sql: &str, suspendable: bool) -> (Reply, Arc<Recorder>) {
        let o = Recorder::new();
        let reply = run_spec(
            &self.session(),
            spec_with(&o, &[(sql, suspendable)], FetchPolicy::default()),
        )
        .await;
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
        (reply, o)
    }

    async fn close(self) {
        self.on.shutdown().await;
    }
}

/// The active lines of `engine` (the twins with a `-` in their id are left out); the full set
/// must hold one.
fn lines_of(engine: DbmsKind) -> Vec<DockerTarget> {
    let found: Vec<DockerTarget> = targets(Some(engine))
        .into_iter()
        .filter(|t| !t.id.contains('-'))
        .collect();
    let set = std::env::var("HACIVAT_DOCKER_SET").ok();
    assert!(
        !must_exist(set.as_deref()) || !found.is_empty(),
        "no {engine:?} line ran"
    );
    found
}

/// Runs `rows` in `ro` and judges each: an allowed outcome, and the side channel unchanged.
async fn judge_rows<F, Fut>(
    t: &DockerTarget,
    who: &str,
    ro: &Ro,
    rows: &[Row],
    mut snapshot: F,
    failures: &mut Vec<String>,
    record: &mut String,
) where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = String>,
{
    let baseline = snapshot().await;
    for (id, sql, allowed) in rows {
        let reply = ro.try_row(sql).await;
        let got = outcome(&reply);
        let after = snapshot().await;
        record.push_str(&format!("{}\t{who}\t{id}\t{got}\t{sql}\n", t.id));
        if after != baseline {
            failures.push(format!(
                "{} {who} {id} `{sql}`: {got}, and the side channel changed: {baseline} → \
                 {after}",
                t.id
            ));
        } else if !allowed.contains(&got.as_str()) {
            failures.push(format!(
                "{} {who} {id} `{sql}`: {got}, not one of {allowed:?}",
                t.id
            ));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// PostgreSQL fixtures

const PG_DB: &str = "hv_t22";

/// The tests of this binary run in parallel; the ones that make and drop the fixtures
/// (`hv_t22` and the MySQL `hv_t22_*` objects) take turns.
static FIXTURES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn pg_fixture(t: &DockerTarget) {
    let mut a = admin(t).await;
    batch(
        &mut a,
        &t.id,
        &format!("DROP DATABASE IF EXISTS {PG_DB} WITH (FORCE)"),
    )
    .await;
    batch(&mut a, &t.id, &format!("CREATE DATABASE {PG_DB}")).await;
    a.close().await;
    let mut d = admin_on(t, PG_DB).await;
    for sql in [
        format!("REVOKE TEMP ON DATABASE {PG_DB} FROM PUBLIC"),
        "REVOKE CREATE ON SCHEMA public FROM PUBLIC".into(),
        "CREATE TABLE hv_t22 (a int)".into(),
        "INSERT INTO hv_t22 VALUES (1), (2), (3)".into(),
        "CREATE SEQUENCE hv_t22_seq".into(),
        "CREATE FUNCTION hv_t22_sd_write() RETURNS int LANGUAGE sql SECURITY DEFINER \
         AS $$ INSERT INTO hv_t22 VALUES (99) RETURNING a $$"
            .into(),
        "CREATE FUNCTION hv_t22_dyn_write() RETURNS void LANGUAGE plpgsql \
         AS $$ BEGIN EXECUTE 'INSERT INTO hv_t22 VALUES (98)'; END $$"
            .into(),
        "CREATE PROCEDURE hv_t22_commit() LANGUAGE plpgsql \
         AS $$ BEGIN COMMIT; INSERT INTO hv_t22 VALUES (8); COMMIT; END $$"
            .into(),
        format!("SELECT lo_from_bytea({LO}, '\\x6876')"),
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

/// What the admin side channel sees in `hv_t22`: the table's rows, the relations of `public`,
/// the sequence, the large objects and the fixture's content, prepared transactions and
/// temporary relations.
async fn pg_snapshot(admin: &mut LiveConnection) -> String {
    scalar(
        admin,
        &format!(
            "SELECT concat_ws('|', \
             (SELECT count(*) || ':' || coalesce(sum(a), 0) FROM hv_t22), \
             (SELECT string_agg(relname, ',' ORDER BY relname) FROM pg_class \
              WHERE relnamespace = 'public'::regnamespace), \
             (SELECT last_value || ':' || is_called FROM hv_t22_seq), \
             (SELECT count(*) FROM pg_largeobject_metadata), \
             (SELECT encode(lo_get({LO}), 'hex')), \
             (SELECT count(*) FROM pg_prepared_xacts), \
             (SELECT count(*) FROM pg_class WHERE relpersistence = 't'))"
        ),
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn server_blocked_bypass_rows_are_blocked_through_run() {
    if !docker_enabled() {
        return;
    }
    let _fixtures = FIXTURES.lock().await;
    let mut failures = Vec::new();
    let mut record = String::new();
    for t in lines_of(DbmsKind::Postgres) {
        pg_fixture(&t).await;
        let a = tokio::sync::Mutex::new(admin_on(&t, PG_DB).await);
        for user in ["hv_ro", t.user.as_str()] {
            let ro = Ro::open(&t, user, PG_DB).await;
            judge_rows(
                &t,
                user,
                &ro,
                PG_SERVER_BLOCKED,
                || async { pg_snapshot(&mut *a.lock().await).await },
                &mut failures,
                &mut record,
            )
            .await;
            ro.close().await;
        }
        a.into_inner().close().await;
        pg_drop_fixture(&t).await;
    }
    // ## m3b-t18-safe06 (named exception): MariaDB's lines too, judged (C19).
    let my_lines = lines_of(DbmsKind::MySql)
        .into_iter()
        .chain(lines_of(DbmsKind::MariaDb));
    for t in my_lines {
        mysql_fixture(&t).await;
        let a = tokio::sync::Mutex::new(admin(&t).await);
        let rows = if t.engine == DbmsKind::MariaDb {
            MARIADB_SERVER_BLOCKED
        } else {
            MYSQL_SERVER_BLOCKED
        };
        for user in ["hv_ro", t.user.as_str()] {
            let ro = Ro::open(&t, user, &t.database).await;
            judge_rows(
                &t,
                user,
                &ro,
                rows,
                || async { mysql_snapshot(&mut *a.lock().await, &t).await },
                &mut failures,
                &mut record,
            )
            .await;
            ro.close().await;
        }
        a.into_inner().close().await;
        mysql_drop_fixture(&t).await;
    }
    // Every row's outcome, as run B records its own (`t23-bypass-<target>.tsv`).
    let path = hacivat_testkit::artifact("t22-run-bypass.tsv");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("the artifact directory");
    }
    std::fs::write(&path, &record).expect("the record is written");
    assert!(!record.is_empty(), "no line of the active set ran a row");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// ---------------------------------------------------------------------------------------------
// MySQL fixtures

async fn mysql_fixture(t: &DockerTarget) {
    let mut a = admin(t).await;
    for sql in [
        "DROP PROCEDURE IF EXISTS hv_t22_escape",
        "DROP TABLE IF EXISTS hv_t22_bypass, hv_t22_m8, hv_t22_m9",
        "CREATE TABLE hv_t22_bypass (a int)",
        "INSERT INTO hv_t22_bypass VALUES (1), (2), (3)",
        // ## fix round 1 (I1): M7's procedure (`hv_ro` has no `EXECUTE` on it).
        "CREATE PROCEDURE hv_t22_escape() BEGIN COMMIT; START TRANSACTION READ WRITE; \
         INSERT INTO hv_t22_bypass VALUES (100); COMMIT; END",
    ] {
        batch(&mut a, &t.id, sql).await;
    }
    a.close().await;
}

async fn mysql_drop_fixture(t: &DockerTarget) {
    let mut a = admin(t).await;
    for sql in [
        "DROP PROCEDURE IF EXISTS hv_t22_escape",
        "DROP TABLE IF EXISTS hv_t22_bypass, hv_t22_m8, hv_t22_m9",
    ] {
        batch(&mut a, &t.id, sql).await;
    }
    a.close().await;
}

/// What the admin side channel sees: the table's rows, the test database's tables and
/// routines, the neutralised rows' user locks, and the global variables the privilege-only
/// rows try to change (fix round 1, I1: run B's side channel).
async fn mysql_snapshot(admin: &mut LiveConnection, t: &DockerTarget) -> String {
    scalar(
        admin,
        &format!(
            "SELECT CONCAT_WS('|', \
             (SELECT CONCAT(COUNT(*), ':', COALESCE(SUM(a), 0)) FROM hv_t22_bypass), \
             (SELECT GROUP_CONCAT(table_name ORDER BY table_name) FROM information_schema.tables \
              WHERE table_schema = '{db}'), \
             (SELECT COUNT(*) FROM information_schema.routines WHERE routine_schema = '{db}'), \
             IS_FREE_LOCK('hv_t22_m1'), IS_FREE_LOCK('hv_t22_m5'), \
             @@global.max_connections, @@global.read_only)",
            db = t.database
        ),
    )
    .await
}

// ---------------------------------------------------------------------------------------------
// The 20 s bound

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn ro_cursor_is_released_before_the_server_idle_kill() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    let ro = Ro::open(&t, t.user.as_str(), &t.database).await;
    let app = ro.app().to_string();
    let s = ro.session();
    // The guard's own setup: the server ends a transaction idle for 30 s with the connection.
    let setting = Recorder::new();
    run_spec(
        &s,
        spec_with(
            &setting,
            &[("SHOW idle_in_transaction_session_timeout", false)],
            FetchPolicy::default(),
        ),
    )
    .await
    .expect("SHOW runs");
    assert_eq!(
        setting.snap(0).cell(0, 0).to_value(),
        hacivat_values::Value::Text("30s".into())
    );
    let o = Recorder::new();
    run_spec(
        &s,
        spec_with(
            &o,
            &[("SELECT g FROM generate_series(1, 100000) g", true)],
            FetchPolicy::default(),
        ),
    )
    .await
    .expect("the guarded run succeeds");
    let Completeness::Suspended(info) = *o.snap(0).completeness() else {
        panic!("{:?}", o.snap(0).completeness());
    };
    assert_eq!(info.idle_bound, secs(20));
    let snap = s.state().borrow().clone();
    assert_eq!(
        (snap.tx.status, snap.tx.owner),
        (TxStatus::InTx, TxOwner::Guard)
    );
    let mut a = admin(&t).await;
    let backend = |state: &str| {
        format!(
            "SELECT coalesce(string_agg(pid::text, ','), '') FROM pg_stat_activity \
             WHERE application_name = '{app}' AND state LIKE '{state}%'"
        )
    };
    let pid = scalar(&mut a, &backend("idle in transaction")).await;
    assert!(
        !pid.is_empty(),
        "the guard's transaction is open on the server"
    );

    // The real bound: the cursor is released (never lost), its rows kept.
    wait_until("the idle bound", secs(60), || {
        !matches!(o.snap(0).completeness(), Completeness::Suspended(_))
    })
    .await;
    assert_eq!(
        *o.snap(0).completeness(),
        Completeness::Closed {
            reason: CloseReason::IdleTimeout
        }
    );
    assert_eq!(o.rows(), 10_000);
    assert_eq!(o.releases(), vec![(CloseReason::IdleTimeout, false)]);
    // The same backend, no longer in a transaction: the server never had to end it.
    wait_until("the session to settle", secs(10), || {
        s.state().borrow().cursor.is_none()
    })
    .await;
    assert_eq!(scalar(&mut a, &backend("idle in transaction")).await, "");
    assert_eq!(scalar(&mut a, &backend("idle")).await, pid);
    // And it runs the next statement.
    let o2 = Recorder::new();
    run_spec(
        &s,
        spec_with(&o2, &[("SELECT 1", true)], FetchPolicy::default()),
    )
    .await
    .expect("the session runs on");
    assert_eq!(o2.rows(), 1);
    assert_eq!(scalar(&mut a, &backend("idle")).await, pid);
    a.close().await;
    ro.close().await;
}

// ---------------------------------------------------------------------------------------------
// ## fix round 1 (review I1): run B's neutralised and privilege-only groups through `Run`

/// Run B's PostgreSQL rows that the server accepts inside the read-only transaction and the
/// guard's end neutralises (`ROLLBACK`, `DISCARD ALL` without `DEALLOCATE ALL`, the replay).
/// On the typed path the end waits for the unit (or its cursor): nothing may linger after it.
const PG_NEUTRALISED: &[(&str, &str)] = &[
    ("P5", "RESET transaction_read_only"),
    (
        "P6",
        "SET SESSION CHARACTERISTICS AS TRANSACTION READ WRITE",
    ),
    ("P6", "SET default_transaction_read_only = off"),
    ("P6", "RESET ALL"),
    ("P10", "PREPARE hv_t22_s AS SELECT 1"),
    ("P15", "SELECT pg_advisory_lock(220023)"),
    ("P15", "LOCK TABLE hv_t22 IN ACCESS EXCLUSIVE MODE"),
    ("P17", "SET search_path = pg_catalog"),
    ("P23", "SET standard_conforming_strings = off"),
    ("P25", "NOTIFY hv_t22, 'x'"),
    ("P25", "SELECT pg_notify('hv_t22', 'x')"),
    ("P26", "LISTEN hv_t22"),
    ("P26", "DECLARE hv_t22_c CURSOR WITH HOLD FOR SELECT 1"),
];

/// Run B's PostgreSQL rows that only `hv_ro`'s missing privileges stop (L5), as `hv_ro` only.
/// Pinned differences from run B (`42501` there): the typed driver refuses `COPY` before
/// anything is sent, so the two `COPY` rows end `protocol`.
// ## m3a-f5-demo (DEMO-7): `DO` blocks are refused before anything is sent on a read-only
// console (`INPUT`); the run-time-built calls inside them keep their privilege-layer rows on
// the M2b path (`docker_m2b_bypass.rs`, `Request::Exec`).
const PG_PRIVILEGE_ONLY: &[Row] = &[
    ("P11", "SELECT pg_terminate_backend({victim})", &[PG_PRIV]),
    ("P13", "COPY (SELECT 1) TO PROGRAM 'true'", &[PROTOCOL]),
    ("P14", "SELECT pg_read_file('/etc/hostname')", &[PG_PRIV]),
    ("P14", "COPY hv_t22 TO '/tmp/hv_t22_copy'", &[PROTOCOL]),
    ("P16", "SET ROLE hv_test", &[PG_PRIV]),
    ("P16", "SET SESSION AUTHORIZATION hv_test", &[PG_PRIV]),
    (
        "P31",
        "SELECT pg_create_physical_replication_slot('hv_t22_slot')",
        &["function"],
    ),
    (
        "P31",
        "DO $$ BEGIN EXECUTE 'SELECT pg_create_physical_' || 'replication_slot(''hv_t22_slot'')'; END $$",
        &[INPUT],
    ),
    ("P32", "SELECT pg_switch_wal()", &["function"]),
    (
        "P32",
        "DO $$ BEGIN EXECUTE 'SELECT pg_switch' || '_wal()'; END $$",
        &[INPUT],
    ),
    (
        "P32",
        "SELECT pg_logical_emit_message(true, 'hv_t22', 'x')",
        &["function"],
    ),
    (
        "P32",
        "DO $$ BEGIN EXECUTE 'SELECT pg_logical_' || 'emit_message(true, ''hv_t22'', ''x'')'; END $$",
        &[INPUT],
    ),
    (
        "P32",
        "DO $$ BEGIN EXECUTE 'SELECT pg_logical_' || 'emit_message(false, ''hv_t22'', ''x'')'; END $$",
        &[INPUT],
    ),
    (
        "P33",
        "SELECT lo_export(220022, '/tmp/hv_t22_lo')",
        &[PG_PRIV],
    ),
    ("P34", "CHECKPOINT", &[PG_PRIV]),
    ("P34", "SELECT pg_stat_reset()", &[PG_PRIV]),
];

/// The server files the privilege-only rows would write.
const PG_SERVER_FILES: [&str; 2] = ["/tmp/hv_t22_copy", "/tmp/hv_t22_lo"];

/// Run B's MySQL neutralised rows (`{tro}`: the transaction read-only variable): M1 (a lock in
/// an executable comment) M3 M5 (`GET_LOCK`) M9 M14.
const MYSQL_NEUTRALISED: &[(&str, &str)] = &[
    (
        "M1",
        "SELECT 1 /*!50000 , (SELECT GET_LOCK('hv_t22_m1', 0)) */",
    ),
    ("M3", "SET sql_mode = 'NO_BACKSLASH_ESCAPES'"),
    ("M5", "SELECT GET_LOCK('hv_t22_m5', 0)"),
    ("M9", "SET SESSION TRANSACTION READ WRITE"),
    ("M9", "SET @@session.{tro} = 0"),
    ("M14", "SET @hv_t22 = 1"),
];

const MY_PRIV: &str = "1227";

/// Run B's MySQL privilege-only rows, as `hv_ro` only. M4 (`INTO OUTFILE`) and M7 (the
/// documented `CALL` residual: only least privilege stops a writing procedure) end exactly as
/// in run B.
const MYSQL_PRIVILEGE_ONLY: &[Row] = &[
    (
        "M4",
        "SELECT a FROM hv_t22_bypass INTO OUTFILE '/tmp/hv_t22_out'",
        &[MY_PRIV],
    ),
    ("M10", "SET GLOBAL max_connections = 999", &[MY_PRIV]),
    (
        "M10",
        "SET PERSIST max_connections = 999",
        &[MY_PRIV, "1064"],
    ),
    ("M10", "RESET PERSIST max_connections", &["3615"]),
    ("M10", "FLUSH TABLES", &[MY_PRIV]),
    ("M10", "KILL {victim}", &["1095"]),
    (
        "M11",
        "CREATE FUNCTION hv_t22_udf RETURNS STRING SONAME 'hv_t22.so'",
        &[INPUT],
    ),
    ("M7", "CALL hv_t22_escape()", &[INPUT]),
];

/// MySQL's transaction read-only variable: `transaction_read_only` on MySQL 8.
const MY_TRO: &str = "transaction_read_only";

/// After a PostgreSQL neutralised row nothing of it may be left in the session (run B's
/// checks; each answers one row when all is well).
fn pg_linger_checks(search_path: &str) -> Vec<String> {
    vec![
        "SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM pg_listening_channels())".into(),
        "SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM pg_cursors WHERE name <> '')".into(),
        "SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM pg_prepared_statements WHERE from_sql)".into(),
        "SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM pg_locks WHERE locktype = 'advisory' \
         AND pid = pg_backend_pid())"
            .into(),
        "SELECT 1 WHERE current_setting('default_transaction_read_only') = 'on'".into(),
        "SELECT 1 WHERE current_setting('standard_conforming_strings') = 'on'".into(),
        format!(
            "SELECT 1 WHERE current_setting('search_path') = '{}'",
            search_path.replace('\'', "''")
        ),
    ]
}

/// After a MySQL neutralised row: the read-only setting, no user variable, no user lock, the
/// pinned `sql_mode`.
fn mysql_linger_checks() -> Vec<String> {
    vec![
        format!("SELECT 1 FROM DUAL WHERE @@session.{MY_TRO} = 1"),
        "SELECT 1 FROM DUAL WHERE @hv_t22 IS NULL".into(),
        "SELECT 1 FROM DUAL WHERE IS_USED_LOCK('hv_t22_m1') IS NULL \
         AND IS_USED_LOCK('hv_t22_m5') IS NULL"
            .into(),
        "SELECT 1 FROM DUAL WHERE @@session.sql_mode NOT LIKE '%NO_BACKSLASH_ESCAPES%'".into(),
    ]
}

/// What a PostgreSQL row may leave outside the fixture database: a replication slot, a server
/// file. Each found is undone and returned (an escape).
async fn pg_escapes(t: &DockerTarget) -> Vec<String> {
    let mut a = admin(t).await;
    let mut found = Vec::new();
    let slot = "SELECT count(*) FROM pg_replication_slots WHERE slot_name = 'hv_t22_slot'";
    if count(&mut a, slot).await > 0 {
        batch(
            &mut a,
            &t.id,
            "SELECT pg_drop_replication_slot('hv_t22_slot')",
        )
        .await;
        found.push(format!("{}: the replication slot hv_t22_slot", t.id));
    }
    for file in PG_SERVER_FILES {
        let exists = scalar(
            &mut a,
            &format!("SELECT pg_stat_file('{file}', true) IS NOT NULL"),
        )
        .await;
        if exists == "t" {
            batch(
                &mut a,
                &t.id,
                &format!("COPY (SELECT 1) TO PROGRAM 'rm -f {file}'"),
            )
            .await;
            found.push(format!("{}: the server file {file}", t.id));
        }
    }
    a.close().await;
    found
}

/// Judges one neutralised row: it ran (or the server refused it, any code), the side channel
/// is unchanged and nothing lingers; a gap or a lost connection fails it.
#[allow(clippy::too_many_arguments)]
fn judge_neutralised(
    t: &DockerTarget,
    id: &str,
    sql: &str,
    got: &str,
    before: &str,
    after: &str,
    lingering: &[String],
    failures: &mut Vec<String>,
) {
    let stopped =
        !["ran", GAP, "lost"].contains(&got) && got.chars().all(|c| c.is_ascii_alphanumeric());
    if before != after {
        failures.push(format!(
            "{} hv_ro {id} `{sql}`: {got}, and the side channel changed: {before} → {after}",
            t.id
        ));
    } else if !lingering.is_empty() {
        failures.push(format!(
            "{} hv_ro {id} `{sql}`: {got}, lingering: {}",
            t.id,
            lingering.join("; ")
        ));
    } else if got != "ran" && !stopped {
        failures.push(format!("{} hv_ro {id} `{sql}`: {got}", t.id));
    }
}

/// [`pg_snapshot`] and the advisory locks held by session `app`.
async fn pg_side(a: &mut LiveConnection, app: &str) -> String {
    let locks = format!(
        "SELECT count(*) FROM pg_locks l JOIN pg_stat_activity s ON s.pid = l.pid \
         WHERE l.locktype = 'advisory' AND s.application_name = '{app}'"
    );
    let slots = "SELECT count(*) FROM pg_replication_slots";
    format!(
        "{}|{}|{}",
        pg_snapshot(a).await,
        scalar(a, &locks).await,
        scalar(a, slots).await
    )
}

/// The PostgreSQL neutralised and privilege-only rows on `t`, as `hv_ro`, through `Run`.
async fn pg_neutralised_rows(t: &DockerTarget, failures: &mut Vec<String>, record: &mut String) {
    pg_fixture(t).await;
    let mut a = admin_on(t, PG_DB).await;
    // As run B: the remediation's `REVOKE` of `pg_logical_emit_message` (F1), so the
    // run-time-built calls meet `hv_ro`'s missing privilege.
    batch(&mut a, &t.id, &revoke_emit_message(t)).await;
    let search_path = scalar(&mut a, "SHOW search_path").await;
    let checks = pg_linger_checks(&search_path);
    let ro = Ro::open(t, "hv_ro", PG_DB).await;
    let app = ro.app().to_string();
    let baseline = pg_side(&mut a, &app).await;
    for (id, sql) in PG_NEUTRALISED {
        let got = outcome(&ro.try_row(sql).await);
        let mut lingering = ro.lingering(&checks).await;
        // A write is still refused by the read-only transaction (one the first keyword
        // does not show: a plain INSERT never leaves the client, m3a-f5-demo).
        let write = outcome(
            &ro.try_row_with(
                "WITH w AS (INSERT INTO hv_t22 VALUES (7) RETURNING a) SELECT * FROM w",
                false,
            )
            .await
            .0,
        );
        if write != PG_RO {
            lingering.push(format!("a write after it → {write}"));
        }
        let after = pg_side(&mut a, &app).await;
        record.push_str(&format!("{}\thv_ro\t{id}\t{got}\t{sql}\n", t.id));
        judge_neutralised(t, id, sql, &got, &baseline, &after, &lingering, failures);
    }
    let victim = scalar(&mut a, "SELECT pg_backend_pid()").await;
    for (id, sql, allowed) in PG_PRIVILEGE_ONLY {
        let run_sql = sql.replace("{victim}", &victim);
        let got = outcome(&ro.try_row(&run_sql).await);
        let after = pg_side(&mut a, &app).await;
        record.push_str(&format!("{}\thv_ro\t{id}\t{got}\t{sql}\n", t.id));
        if after != baseline || !allowed.contains(&got.as_str()) {
            failures.push(format!(
                "{} hv_ro {id} `{sql}`: {got} (allowed {allowed:?}), side channel {baseline} → \
                 {after}",
                t.id
            ));
        }
    }
    // P11's victim is alive (the side channel itself).
    assert_eq!(scalar(&mut a, "SELECT 1").await, "1", "{} P11", t.id);
    ro.close().await;
    a.close().await;
    for escaped in pg_escapes(t).await {
        failures.push(format!("escaped: {escaped}"));
    }
    pg_drop_fixture(t).await;
}

/// The MySQL neutralised and privilege-only rows on `t`, as `hv_ro`, through `Run`.
async fn mysql_neutralised_rows(t: &DockerTarget, failures: &mut Vec<String>, record: &mut String) {
    mysql_fixture(t).await;
    let mut a = admin(t).await;
    let max_connections = scalar(&mut a, "SELECT @@global.max_connections").await;
    let checks = mysql_linger_checks();
    let ro = Ro::open(t, "hv_ro", &t.database).await;
    let baseline = mysql_snapshot(&mut a, t).await;
    for (id, sql) in MYSQL_NEUTRALISED {
        let run_sql = sql.replace("{tro}", MY_TRO);
        let got = outcome(&ro.try_row(&run_sql).await);
        let mut lingering = ro.lingering(&checks).await;
        // DDL is still refused (since m3a-f5-demo by the client, before anything is sent),
        // and a write the first keyword does not show by the read-only transaction.
        let ddl = outcome(
            &ro.try_row_with("CREATE TABLE hv_t22_m9 (a int)", false)
                .await
                .0,
        );
        if ddl != INPUT {
            lingering.push(format!("DDL after it → {ddl}"));
        }
        let write = outcome(
            &ro.try_row_with(
                "WITH x AS (SELECT 1 AS a) DELETE FROM hv_t22_bypass WHERE a = 1",
                false,
            )
            .await
            .0,
        );
        if ![MY_RO, MY_TABLE_PRIV].contains(&write.as_str()) {
            lingering.push(format!("a write after it → {write}"));
        }
        let after = mysql_snapshot(&mut a, t).await;
        record.push_str(&format!("{}\thv_ro\t{id}\t{got}\t{sql}\n", t.id));
        judge_neutralised(t, id, sql, &got, &baseline, &after, &lingering, failures);
    }
    let victim = scalar(&mut a, "SELECT CONNECTION_ID()").await;
    for (id, sql, allowed) in MYSQL_PRIVILEGE_ONLY {
        let run_sql = sql.replace("{victim}", &victim);
        let got = outcome(&ro.try_row(&run_sql).await);
        let after = mysql_snapshot(&mut a, t).await;
        record.push_str(&format!("{}\thv_ro\t{id}\t{got}\t{sql}\n", t.id));
        if after != baseline || !allowed.contains(&got.as_str()) {
            failures.push(format!(
                "{} hv_ro {id} `{sql}`: {got} (allowed {allowed:?}), side channel {baseline} → \
                 {after}",
                t.id
            ));
        }
    }
    // M10's victim is alive.
    assert_eq!(scalar(&mut a, "SELECT 1").await, "1", "{} M10 KILL", t.id);
    ro.close().await;
    let now = scalar(&mut a, "SELECT @@global.max_connections").await;
    if now != max_connections {
        batch(
            &mut a,
            &t.id,
            &format!("SET GLOBAL max_connections = {max_connections}"),
        )
        .await;
        failures.push(format!(
            "{}: max_connections escaped ({max_connections} → {now}; restored)",
            t.id
        ));
    }
    a.close().await;
    mysql_drop_fixture(t).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn neutralised_and_privilege_only_rows_through_run() {
    if !docker_enabled() {
        return;
    }
    let _fixtures = FIXTURES.lock().await;
    let mut failures = Vec::new();
    let mut record = String::new();
    for t in lines_of(DbmsKind::Postgres) {
        pg_neutralised_rows(&t, &mut failures, &mut record).await;
    }
    for t in lines_of(DbmsKind::MySql) {
        mysql_neutralised_rows(&t, &mut failures, &mut record).await;
    }
    let path = hacivat_testkit::artifact("t22-run-neutralised.tsv");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("the artifact directory");
    }
    std::fs::write(&path, &record).expect("the record is written");
    assert!(!record.is_empty(), "no line of the active set ran a row");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// ---------------------------------------------------------------------------------------------
// ## fix round 1 (review I1, M6): a lock taken by a cursor's rows is released with the cursor

/// The single value of a one-row, one-column result, as text.
fn single(o: &Recorder) -> String {
    match o.snap(0).cell(0, 0).to_value() {
        hacivat_values::Value::Int(v) => v.to_string(),
        hacivat_values::Value::UInt(v) => v.to_string(),
        other => format!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_a_lock_held_by_a_read_only_cursor_goes_with_its_release() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    let ro = Ro::open(&t, t.user.as_str(), &t.database).await;
    let app = ro.app().to_string();
    let s = ro.session();
    let o = Recorder::new();
    run_spec(
        &s,
        spec_with(
            &o,
            &[(
                "SELECT g, pg_advisory_lock(220024) FROM generate_series(1, 100000) g",
                true,
            )],
            FetchPolicy::default(),
        ),
    )
    .await
    .expect("the guarded run succeeds");
    assert!(matches!(
        o.snap(0).completeness(),
        Completeness::Suspended(_)
    ));
    let mut a = admin(&t).await;
    let held = format!(
        "SELECT count(*) FROM pg_locks l JOIN pg_stat_activity s ON s.pid = l.pid \
         WHERE l.locktype = 'advisory' AND s.application_name = '{app}'"
    );
    assert!(
        count(&mut a, &held).await > 0,
        "the cursor's rows took the lock"
    );
    // Another request: the cursor is released, the guard's end (ROLLBACK, the reset with its
    // `pg_advisory_unlock_all()`) runs, then the request.
    let o2 = Recorder::new();
    run_spec(
        &s,
        spec_with(&o2, &[("SELECT 1", false)], FetchPolicy::default()),
    )
    .await
    .expect("the next unit runs");
    assert_eq!(o.releases(), vec![(CloseReason::SessionNeeded, false)]);
    assert_eq!(
        *o.snap(0).completeness(),
        Completeness::Closed {
            reason: CloseReason::SessionNeeded
        }
    );
    assert_eq!(
        count(&mut a, &held).await,
        0,
        "the lock went with the cursor"
    );
    a.close().await;
    ro.close().await;
}

/// A MySQL-family cross join large enough to stay a cursor; each row takes the same user lock
/// again (re-entrant).
const MY_LOCKING_BIG: &str = "WITH d AS (SELECT 0 AS x UNION ALL SELECT 1 UNION ALL SELECT 2 \
    UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 \
    UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) \
    SELECT a.x + b.x * 10 + c.x * 100 + e.x * 1000 + f.x * 10000 AS n, \
    GET_LOCK('hv_t22_cur', 0) AS l FROM d a, d b, d c, d e, d f";

/// Review M6: a real read-only MySQL cursor's release (drain, `KILL QUERY` on the side
/// connection, the guard's `ROLLBACK`, `COM_RESET_CONNECTION`, the replay) on the same
/// connection. MariaDB is included here (its cursor release is the same path). ## m3b-t18-safe06:
/// its bypass rows are judged through `Run` since Task 18 (`MARIADB_SERVER_BLOCKED`, C19).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn mysql_a_read_only_cursor_release_keeps_the_connection_and_drops_its_lock() {
    if !docker_enabled() {
        return;
    }
    for id in ["mysql84", "mariadb114"] {
        let Some(t) = require_target(id) else {
            continue;
        };
        let ro = Ro::open(&t, t.user.as_str(), &t.database).await;
        let s = ro.session();
        let thread = |o: &Recorder| single(o);
        let (reply, before) = ro.try_row_with("SELECT CONNECTION_ID()", false).await;
        reply.unwrap_or_else(|e| panic!("{id}: CONNECTION_ID: {:?}", e.kind));
        let before = thread(&before);
        let o = Recorder::new();
        run_spec(
            &s,
            spec_with(&o, &[(MY_LOCKING_BIG, true)], FetchPolicy::default()),
        )
        .await
        .unwrap_or_else(|e| panic!("{id}: the guarded run: {:?}", e.kind));
        assert!(
            matches!(o.snap(0).completeness(), Completeness::Suspended(_)),
            "{id}: {:?}",
            o.snap(0).completeness()
        );
        assert_eq!(s.state().borrow().tx.owner, TxOwner::Guard, "{id}");
        let mut a = admin(&t).await;
        assert_eq!(
            scalar(&mut a, "SELECT IS_USED_LOCK('hv_t22_cur')").await,
            before,
            "{id}: the cursor's rows took the lock"
        );
        // Another request releases the cursor first, on the same connection.
        let (reply, after) = ro.try_row_with("SELECT CONNECTION_ID()", false).await;
        reply.unwrap_or_else(|e| panic!("{id}: after the release: {:?}", e.kind));
        assert_eq!(thread(&after), before, "{id}: the same connection");
        assert_eq!(
            o.releases(),
            vec![(CloseReason::SessionNeeded, false)],
            "{id}"
        );
        assert_eq!(o.rows(), 10_000, "{id}");
        // `COM_RESET_CONNECTION` released the lock; no transaction is left on the thread.
        assert_eq!(
            scalar(&mut a, "SELECT IS_USED_LOCK('hv_t22_cur')").await,
            "",
            "{id}: the lock went with the cursor"
        );
        assert_eq!(
            count(
                &mut a,
                &format!(
                    "SELECT COUNT(*) FROM information_schema.processlist \
                     WHERE id = {before} AND command <> 'Sleep'"
                )
            )
            .await,
            0,
            "{id}"
        );
        let snap = s.state().borrow().clone();
        assert_eq!(
            (snap.tx.status, snap.cursor),
            (TxStatus::Idle, None),
            "{id}"
        );
        a.close().await;
        ro.close().await;
    }
}
