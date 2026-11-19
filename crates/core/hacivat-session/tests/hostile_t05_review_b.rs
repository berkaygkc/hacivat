// ## m4-t05-meta (fix round 1)
//! Independent security review B of M4 Task 5: the reviewer's hostile tests
//! (`review_t05b_hostile.rs`). Every test asserts the requirement; each failed at `18b6a8c` and
//! passes after fix round 1 (rulings R-W1-9, I2-A, I4-B; ADR 0004 `## m4-t05`). H1 asserts the
//! ruling's form: the read runs inside Hacivat's own read-only transaction, never the user's.
//!
//! `MetaQuery::new` is called here directly: test files are exempt from check-deps
//! `MetaQueryOnlyFromPacks`, and the hostile texts must reach the guards.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use hacivat_driver::TxStatus;
use hacivat_driver::fake::{
    FakeCall, FakeDriver, FakeQuery, FakeResultSet, FakeRows, FakeTypedCall,
};
use hacivat_net::TlsMode;
use hacivat_proto::Department;
use hacivat_proto::EnvTag;
use hacivat_session::budget_for;
use hacivat_session::meta::{MetaConn, MetaConnSpec, MetaError, MetaQuery, check_read_text};
use hacivat_session::pipeline::Interaction;
use hacivat_session::profile::Profile;
use hacivat_session::session::{
    ClientId, JobKind, SessionHandle, SessionKind, SessionManager, SessionSpec, TxCommand, TxMode,
};
use hacivat_values::{LogicalType, Value};
use tokio_util::sync::CancellationToken;

fn manager(fake: FakeDriver, profile: Profile) -> (Arc<SessionManager>, Arc<FakeDriver>) {
    let (deps, fake) = deps_with(fake);
    let budget = Arc::new(budget_for(&profile));
    (
        SessionManager::new(
            tokio::runtime::Handle::current(),
            Arc::new(profile),
            Arc::new(deps),
            budget,
        ),
        fake,
    )
}

fn single_pg() -> Profile {
    Profile {
        single_session: true,
        max_connections: 3,
        ..pg_profile(TlsMode::Disable)
    }
}

fn spec(profile: &Profile) -> MetaConnSpec {
    MetaConnSpec {
        profile: profile.id,
        database: None,
        statement_timeout: Duration::from_secs(60),
    }
}

fn relnames() -> FakeQuery {
    FakeQuery {
        prefix: "SELECT relname",
        sets: vec![FakeResultSet {
            columns: vec![(
                "relname",
                LogicalType::Text {
                    max_chars: None,
                    fixed: false,
                },
            )],
            rows: FakeRows::Literal(vec![vec![Value::Text(Arc::from("orders"))]]),
        }],
        ..FakeQuery::default()
    }
}

async fn open(sm: &Arc<SessionManager>, kind: SessionKind) -> SessionHandle {
    let department = match kind {
        SessionKind::Job(..) => Department::Executor,
        _ => Department::Console,
    };
    sm.open(
        SessionSpec {
            kind,
            read_only: false,
            department,
        },
        Interaction::Headless,
    )
    .await
    .unwrap_or_else(|e| panic!("opens: {:?}", e.class))
}

fn statements_since(f: &FakeDriver, from: usize) -> Vec<String> {
    f.call_log()[from..]
        .iter()
        .filter_map(|c| match c {
            FakeCall::Statement(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

const READ: &str = "SELECT relname FROM pg_catalog.pg_class";

/// H1: a single-session profile whose console is in Manual (the production default) or Smart
/// mode. The metadata read must not begin a transaction on the user's session.
async fn user_mode_case(mode: Option<TxMode>, env: Option<EnvTag>) {
    let profile = Profile {
        environment: env,
        ..single_pg()
    };
    let (sm, fake) = manager(
        fake_pg().tracks_transactions().queries(vec![relnames()]),
        profile.clone(),
    );
    let user = open(&sm, SessionKind::Console(ClientId::DEFAULT)).await;
    if let Some(mode) = mode {
        tx_cmd(&user, TxCommand::SetMode(mode)).await.unwrap();
    }
    let cancel = CancellationToken::new();
    let mut c = sm
        .meta_conn(spec(&profile), &cancel)
        .await
        .expect("meta conn");
    let from = fake.call_log().len();
    let mut snap = c.begin_snapshot().await.expect("snapshot");
    let got = snap.rows(MetaQuery::new(READ, &[], false)).await;
    let sent = statements_since(&fake, from);
    let after = snap_of(&user);
    // Ruling R-W1-9: no user transaction control; only Hacivat's own read-only transaction,
    // rolled back in the same actor step.
    assert_eq!(
        sent,
        [
            "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
            "SET LOCAL search_path = pg_catalog, pg_temp",
            "SET LOCAL statement_timeout = 60000",
            "ROLLBACK",
        ],
        "mode={mode:?} env={env:?}"
    );
    assert_eq!(
        after.0,
        TxStatus::Idle,
        "the user's session was left inside a transaction"
    );
    assert_eq!(after.1, "None", "no transaction owner left behind");
    assert!(got.is_ok(), "{got:?}");
}

fn snap_of(h: &SessionHandle) -> (TxStatus, String) {
    let s = snap(h);
    (s.tx.status, format!("{:?}", s.tx.owner))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h1a_manual_mode_user_session_meta_read_opens_no_transaction() {
    user_mode_case(Some(TxMode::Manual), None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h1b_smart_mode_user_session_meta_read_opens_no_transaction() {
    user_mode_case(Some(TxMode::Smart), None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h1c_production_profile_default_mode_meta_read_opens_no_transaction() {
    user_mode_case(None, Some(EnvTag::Production)).await;
}

/// H2: the user's BEGIN is queued (not yet run) when the metadata read checks the session; the
/// read is queued behind it and runs INSIDE the user's transaction; a ROLLBACK queued right after
/// the read can end the transaction before `check_after` looks.
#[tokio::test(flavor = "current_thread")]
async fn h2_meta_read_never_runs_inside_a_queued_user_transaction() {
    let profile = single_pg();
    let (sm, fake) = manager(
        fake_pg().tracks_transactions().queries(vec![relnames()]),
        profile.clone(),
    );
    let user = open(&sm, SessionKind::Console(ClientId::DEFAULT)).await;
    let cancel = CancellationToken::new();
    let mut c = sm
        .meta_conn(spec(&profile), &cancel)
        .await
        .expect("meta conn");
    let mut snap = c.begin_snapshot().await.expect("snapshot");
    let from = fake.call_log().len();
    // The console's BEGIN is submitted but the actor has not run it yet (current_thread).
    let begin = user.submit(exec("BEGIN"));
    let mut read = Box::pin(snap.rows(MetaQuery::new(READ, &[], false)));
    // One poll: check_open sees `Idle`, the read is queued behind BEGIN.
    assert!(futures::poll!(&mut read).is_pending());
    let rollback = user.submit(exec("ROLLBACK"));
    let got = read.await;
    begin.reply.await.unwrap().unwrap();
    rollback.reply.await.unwrap().unwrap();
    let log: Vec<String> = fake.call_log()[from..]
        .iter()
        .filter_map(|c| match c {
            FakeCall::Statement(s) => Some(format!("stmt:{s}")),
            FakeCall::Typed(FakeTypedCall::Query(s)) => Some(format!("query:{s}")),
            _ => None,
        })
        .collect();
    let begin_at = log.iter().position(|l| l == "stmt:BEGIN");
    let read_at = log
        .iter()
        .position(|l| l.starts_with("query:SELECT relname"));
    let rollback_at = log.iter().position(|l| l == "stmt:ROLLBACK");
    let inside =
        matches!((begin_at, read_at, rollback_at), (Some(b), Some(r), Some(e)) if b < r && r < e);
    assert!(
        !inside,
        "the metadata read ran inside the user's transaction (answered {got:?}): {log:?}"
    );
    // Fix round 1: the actor took the read inside the user's transaction and refused it,
    // sending nothing of it.
    assert_eq!(got.map(drop), Err(MetaError::Busy));
    assert_eq!(read_at, None, "{log:?}");
}

/// H3: texts the joined/user-session allowlist must refuse (not a single plain read).
#[test]
fn h3_allowlist_refuses_writes_locks_and_comment_tricks() {
    let mut passed = Vec::new();
    for t in [
        // PostgreSQL ends a `--` comment at `\r` as well as `\n`; the allowlist only at `\n`.
        "-- x\rCREATE TABLE evil AS\nSELECT 1",
        "-- x\rCOPY (\nSELECT 1) TO PROGRAM 'touch /tmp/pwned'",
        "-- x\rCREATE VIEW evil AS\nSELECT 1",
        // `$` counts as a word character, so a dollar-quoted literal glued to INTO hides it.
        "SELECT $$x$$INTO evil",
        // PostgreSQL 14 (the floor) lexes `1INTO` as `1 INTO`.
        "SELECT 1INTO evil",
        // Row locks (XID assignment on PostgreSQL).
        "SELECT * FROM t FOR SHARE",
        "SELECT * FROM t FOR KEY SHARE",
        "SELECT * FROM t LOCK IN SHARE MODE",
    ] {
        if check_read_text(t) == Ok(()) {
            passed.push(t);
        }
    }
    assert!(passed.is_empty(), "accepted: {passed:#?}");
}

/// H4: the `\r` comment text reaches a joined DdlApply transaction's server unchanged.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h4_joined_conn_never_sends_hidden_ddl() {
    let (sm, fake) = manager(
        fake_pg().tracks_transactions(),
        Profile {
            max_connections: 4,
            ..pg_profile(TlsMode::Disable)
        },
    );
    let job = open(&sm, SessionKind::Job(1, JobKind::DdlApply)).await;
    run(&job, "BEGIN").await.unwrap();
    let mut c = MetaConn::join(&job).unwrap();
    let from = fake.call_log().len();
    let mut snap = c.begin_snapshot().await.unwrap();
    let hidden = "-- x\rCREATE TABLE evil AS\nSELECT 1";
    let got = snap.rows(MetaQuery::new(hidden, &[], true)).await;
    let sent: Vec<FakeCall> = fake.call_log()[from..].to_vec();
    assert!(
        sent.is_empty(),
        "the joined connection sent DDL hidden behind a \\r comment: {sent:?}"
    );
    assert_eq!(got.map(drop), Err(MetaError::RefusedText));
}

/// H5: a snapshot dropped without its commit (an introspector's early `?`) must not keep the
/// SQLite reader's read transaction, and its SHARED lock, open while the `MetaConn` lives: the
/// user's write commits.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h5_abandoned_sqlite_snapshot_holds_no_lock() {
    use hacivat_driver::{Driver, DriverConnect, Endpoint, InsecureAuth};
    use hacivat_proto::DbmsKind;
    use hacivat_session::profile::ProfileEndpoint;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.db");
    let mut conn = hacivat_driver_sqlite::SqliteDriver::new()
        .connect(DriverConnect {
            endpoint: Endpoint::File {
                path: path.clone(),
                create_if_missing: true,
                read_only: false,
            },
            tls: None,
            user: None,
            password: None,
            database: None,
            application_name: "fixture".into(),
            read_only: false,
            options: Vec::new(),
            connect_timeout: secs(10),
            events: Arc::new(|_| {}),
            insecure_auth: InsecureAuth::default(),
        })
        .await
        .unwrap();
    conn.batch("CREATE TABLE t(a)").await.unwrap();
    conn.batch("INSERT INTO t VALUES (1)").await.unwrap();
    conn.close().await;
    let profile = Profile {
        engine: DbmsKind::Sqlite,
        endpoint: ProfileEndpoint::File {
            path: path.clone(),
            create_if_missing: false,
        },
        user: None,
        ..sqlite_profile()
    };
    let budget = Arc::new(budget_for(&profile));
    let sm = SessionManager::new(
        tokio::runtime::Handle::current(),
        Arc::new(profile.clone()),
        Arc::new(common::docker_support::deps()),
        budget,
    );
    let user = open(&sm, SessionKind::Console(ClientId::DEFAULT)).await;
    let cancel = CancellationToken::new();
    let mut c = sm.meta_conn(spec(&profile), &cancel).await.unwrap();
    {
        let mut snap = c.begin_snapshot().await.unwrap();
        snap.rows(MetaQuery::new("SELECT a FROM t", &[], false))
            .await
            .unwrap();
        // Dropped without commit; the MetaConn stays alive (cached by the caller).
    }
    let started = std::time::Instant::now();
    let wrote = run(&user, "INSERT INTO t VALUES (2)").await;
    let took = started.elapsed();
    assert!(
        wrote.is_ok(),
        "the user's write was blocked by the metadata reader's lock: {:?} after {took:?}",
        wrote.map(drop).map_err(|e| e.kind)
    );
    // Fix round 1 (I4-B): the dropped snapshot rolled back at once, well inside the user's
    // 5 s busy timeout, while the `MetaConn` still lives.
    assert!(took < Duration::from_secs(4), "{took:?}");
    drop(c);
}

/// H5 control: the same steps with the snapshot committed; the user's write goes through.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h5_control_committed_snapshot() {
    use hacivat_driver::{Driver, DriverConnect, Endpoint, InsecureAuth};
    use hacivat_proto::DbmsKind;
    use hacivat_session::profile::ProfileEndpoint;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.db");
    let mut conn = hacivat_driver_sqlite::SqliteDriver::new()
        .connect(DriverConnect {
            endpoint: Endpoint::File {
                path: path.clone(),
                create_if_missing: true,
                read_only: false,
            },
            tls: None,
            user: None,
            password: None,
            database: None,
            application_name: "fixture".into(),
            read_only: false,
            options: Vec::new(),
            connect_timeout: secs(10),
            events: Arc::new(|_| {}),
            insecure_auth: InsecureAuth::default(),
        })
        .await
        .unwrap();
    conn.batch("CREATE TABLE t(a)").await.unwrap();
    conn.batch("INSERT INTO t VALUES (1)").await.unwrap();
    conn.close().await;
    let profile = Profile {
        engine: DbmsKind::Sqlite,
        endpoint: ProfileEndpoint::File {
            path: path.clone(),
            create_if_missing: false,
        },
        user: None,
        ..sqlite_profile()
    };
    let budget = Arc::new(budget_for(&profile));
    let sm = SessionManager::new(
        tokio::runtime::Handle::current(),
        Arc::new(profile.clone()),
        Arc::new(common::docker_support::deps()),
        budget,
    );
    let user = open(&sm, SessionKind::Console(ClientId::DEFAULT)).await;
    let cancel = CancellationToken::new();
    let mut c = sm.meta_conn(spec(&profile), &cancel).await.unwrap();
    {
        let mut snap = c.begin_snapshot().await.unwrap();
        snap.rows(MetaQuery::new("SELECT a FROM t", &[], false))
            .await
            .unwrap();
        snap.commit().await.unwrap();
    }
    let started = std::time::Instant::now();
    let wrote = run(&user, "INSERT INTO t VALUES (2)").await;
    assert!(started.elapsed() < Duration::from_secs(4));
    drop(c);
    assert!(
        wrote.is_ok(),
        "the user's write was blocked by the metadata reader's lock"
    );
}
