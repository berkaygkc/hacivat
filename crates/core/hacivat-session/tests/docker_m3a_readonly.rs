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
//! - One difference from run B, by design: `COPY` is refused by the typed PostgreSQL driver
//!   before anything is sent (Task 12), so P14 may also end as `protocol`.

#![allow(
    clippy::result_large_err,
    reason = "Reply is the session's fixed interface"
)]

mod common;

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
    ("P8", "CALL hv_t22_commit()", &["2D000"]),
    (
        "P9",
        "DO $$ BEGIN INSERT INTO hv_t22 VALUES (9); END $$",
        &[PG_RO],
    ),
    (
        "P14",
        "COPY hv_t22 FROM '/etc/hostname'",
        &[PG_RO, PG_PRIV, PROTOCOL],
    ),
    (
        "P26",
        "CREATE TEMP TABLE hv_t22_tmp (a int)",
        &[PG_RO, PG_PRIV],
    ),
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
const MY_DB_PRIV: &str = "1044";

/// Run B's MySQL server-blocked rows on this task's fixture table.
const MYSQL_SERVER_BLOCKED: &[Row] = &[
    (
        "M1",
        "/*!50000 DROP TABLE hv_t22_bypass */",
        &[MY_RO, MY_TABLE_PRIV],
    ),
    (
        "M1",
        "/*!50000 INSERT INTO hv_t22_bypass VALUES (50) */",
        &[MY_RO, MY_TABLE_PRIV],
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
    (
        "M8",
        "CREATE TABLE hv_t22_m8 (a int)",
        &[MY_RO, MY_TABLE_PRIV],
    ),
    (
        "M8",
        "TRUNCATE TABLE hv_t22_bypass",
        &[MY_RO, MY_TABLE_PRIV],
    ),
    ("M8", "DELETE FROM hv_t22_bypass", &[MY_RO, MY_TABLE_PRIV]),
    ("M12", "HANDLER hv_t22_bypass OPEN", &[PROTOCOL]),
    ("M13", "XA START 'hv_t22'", &[PROTOCOL]),
    (
        "M17",
        "CREATE TEMPORARY TABLE hv_t22_tmp (a int)",
        &[MY_RO, MY_DB_PRIV],
    ),
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
        let o = Recorder::new();
        let reply = run_spec(
            &self.session(),
            spec_with(&o, &[(sql, read_head(sql))], FetchPolicy::default()),
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
        reply
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
) where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = String>,
{
    let baseline = snapshot().await;
    for (id, sql, allowed) in rows {
        let reply = ro.try_row(sql).await;
        let got = outcome(&reply);
        let after = snapshot().await;
        eprintln!("{} {who} {id} {got}\t{sql}", t.id);
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
    let mut failures = Vec::new();
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
            )
            .await;
            ro.close().await;
        }
        a.into_inner().close().await;
        pg_drop_fixture(&t).await;
    }
    for t in lines_of(DbmsKind::MySql) {
        mysql_fixture(&t).await;
        let a = tokio::sync::Mutex::new(admin(&t).await);
        for user in ["hv_ro", t.user.as_str()] {
            let ro = Ro::open(&t, user, &t.database).await;
            judge_rows(
                &t,
                user,
                &ro,
                MYSQL_SERVER_BLOCKED,
                || async { mysql_snapshot(&mut *a.lock().await, &t).await },
                &mut failures,
            )
            .await;
            ro.close().await;
        }
        a.into_inner().close().await;
        mysql_drop_fixture(&t).await;
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// ---------------------------------------------------------------------------------------------
// MySQL fixtures

async fn mysql_fixture(t: &DockerTarget) {
    let mut a = admin(t).await;
    for sql in [
        "DROP TABLE IF EXISTS hv_t22_bypass, hv_t22_m8",
        "CREATE TABLE hv_t22_bypass (a int)",
        "INSERT INTO hv_t22_bypass VALUES (1), (2), (3)",
    ] {
        batch(&mut a, &t.id, sql).await;
    }
    a.close().await;
}

async fn mysql_drop_fixture(t: &DockerTarget) {
    let mut a = admin(t).await;
    batch(
        &mut a,
        &t.id,
        "DROP TABLE IF EXISTS hv_t22_bypass, hv_t22_m8",
    )
    .await;
    a.close().await;
}

/// What the admin side channel sees: the table's rows and the test database's tables.
async fn mysql_snapshot(admin: &mut LiveConnection, t: &DockerTarget) -> String {
    scalar(
        admin,
        &format!(
            "SELECT CONCAT_WS('|', \
             (SELECT CONCAT(COUNT(*), ':', COALESCE(SUM(a), 0)) FROM hv_t22_bypass), \
             (SELECT GROUP_CONCAT(table_name ORDER BY table_name) FROM information_schema.tables \
              WHERE table_schema = '{db}'))",
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
