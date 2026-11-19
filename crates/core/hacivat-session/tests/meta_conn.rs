// ## m4-t05-meta
//! Metadata sessions (M4 Task 5; HV-SESS-01's metadata connection, HV-INTRO-01 step 4,
//! R-M4-7, R-M4-29, R-M4-38, U-6(e)): leases through the budget at `Priority::Introspection`,
//! the read-only set-up and snapshot statements per engine, the single-session share of the
//! user's idle session, connection loss, binds, joined connections, and SQLite's separate
//! read-only reader on real files.
//!
//! `MetaQuery::new` is called here directly: test files under `tests/` are exempt from
//! check-deps `MetaQueryOnlyFromPacks`, and the hostile texts below must reach the connection.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use hacivat_driver::TxStatus;
use hacivat_driver::fake::{
    FakeCall, FakeDriver, FakeQuery, FakeResultSet, FakeRows, FakeTypedCall,
};
use hacivat_net::TlsMode;
use hacivat_proto::{DbmsKind, Department};
use hacivat_session::meta::{
    META_APPLICATION_NAME, MetaConn, MetaConnSpec, MetaError, MetaQuery, Rows,
};
use hacivat_session::pipeline::{ConnectDeps, Interaction};
use hacivat_session::profile::{Profile, ProfileEndpoint};
use hacivat_session::session::{
    ClientId, ExecErrorKind, JobKind, SessionHandle, SessionKind, SessionManager, SessionSpec,
    TxCommand, TxMode,
};
use hacivat_session::{Priority, budget_for};
use hacivat_values::{BindValue, LogicalType, Value};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------------------------
// Fixtures

fn manager_over(deps: ConnectDeps, profile: Profile) -> Arc<SessionManager> {
    let budget = Arc::new(budget_for(&profile));
    SessionManager::new(
        tokio::runtime::Handle::current(),
        Arc::new(profile),
        Arc::new(deps),
        budget,
    )
}

fn manager(fake: FakeDriver, profile: Profile) -> (Arc<SessionManager>, Arc<FakeDriver>) {
    let (deps, fake) = deps_with(fake);
    (manager_over(deps, profile), fake)
}

fn pg_n(n: u8) -> Profile {
    Profile {
        max_connections: n,
        ..pg_profile(TlsMode::Disable)
    }
}

fn spec_of(p: &SessionManager, profile: &Profile, database: Option<&str>) -> MetaConnSpec {
    let _ = p;
    MetaConnSpec {
        profile: profile.id,
        database: database.map(str::to_string),
        statement_timeout: Duration::from_secs(60),
    }
}

fn q(sql: &str) -> MetaQuery<'_> {
    MetaQuery::new(sql, &[], false)
}

fn joinable(sql: &str) -> MetaQuery<'_> {
    MetaQuery::new(sql, &[], true)
}

fn text(s: &str) -> BindValue {
    BindValue::Typed {
        hint: LogicalType::Text {
            max_chars: None,
            fixed: false,
        },
        text: Arc::from(s),
    }
}

/// A typed query answering one text column `name` with `values`.
fn answers(prefix: &'static str, name: &'static str, values: &[&str]) -> FakeQuery {
    FakeQuery {
        prefix,
        sets: vec![FakeResultSet {
            columns: vec![(
                name,
                LogicalType::Text {
                    max_chars: None,
                    fixed: false,
                },
            )],
            rows: FakeRows::Literal(
                values
                    .iter()
                    .map(|v| vec![Value::Text(Arc::from(*v))])
                    .collect(),
            ),
        }],
        ..FakeQuery::default()
    }
}

/// The MySQL/MariaDB session probe's answer: the read-only flag `ro` and the three character
/// sets `charset`.
fn session_probe(prefix: &'static str, ro: i64, charset: &str) -> FakeQuery {
    let text = || LogicalType::Text {
        max_chars: None,
        fixed: false,
    };
    FakeQuery {
        prefix,
        sets: vec![FakeResultSet {
            columns: vec![
                (
                    "ro",
                    LogicalType::Int {
                        bits: 64,
                        signed: true,
                    },
                ),
                ("client", text()),
                ("connection", text()),
                ("results", text()),
            ],
            rows: FakeRows::Literal(vec![vec![
                Value::Int(ro),
                Value::Text(Arc::from(charset)),
                Value::Text(Arc::from(charset)),
                Value::Text(Arc::from(charset)),
            ]]),
        }],
        ..FakeQuery::default()
    }
}

/// The user-session MySQL character-set probe's answer.
fn charset_probe(charset: &str) -> FakeQuery {
    let text = || LogicalType::Text {
        max_chars: None,
        fixed: false,
    };
    FakeQuery {
        prefix: "SELECT @@session.character_set_client",
        sets: vec![FakeResultSet {
            columns: vec![("c", text()), ("n", text()), ("r", text())],
            rows: FakeRows::Literal(vec![vec![
                Value::Text(Arc::from(charset)),
                Value::Text(Arc::from(charset)),
                Value::Text(Arc::from(charset)),
            ]]),
        }],
        ..FakeQuery::default()
    }
}

/// What a single-session PostgreSQL read sends around its statement (ruling R-W1-9).
const PG_USER_READ: [&str; 3] = [
    "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
    "SET LOCAL search_path = pg_catalog, pg_temp",
    "SET LOCAL statement_timeout = 60000",
];

fn cells(rows: &Rows) -> Vec<Vec<Option<String>>> {
    rows.rows
        .iter()
        .map(|r| r.iter().map(|c| c.as_deref().map(str::to_string)).collect())
        .collect()
}

/// Every statement (`execute`/`batch`) a fake's connections ran since `from` calls.
fn statements_since(f: &FakeDriver, from: usize) -> Vec<String> {
    f.call_log()[from..]
        .iter()
        .filter_map(|c| match c {
            FakeCall::Statement(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

/// Every typed query text since `from` calls.
fn queries_since(f: &FakeDriver, from: usize) -> Vec<String> {
    f.call_log()[from..]
        .iter()
        .filter_map(|c| match c {
            FakeCall::Typed(FakeTypedCall::Query(s)) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

async fn open(sm: &Arc<SessionManager>, kind: SessionKind) -> SessionHandle {
    let department = match kind {
        SessionKind::Job(..) => Department::Executor,
        SessionKind::Editor(_) => Department::DataEditor,
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
    .unwrap_or_else(|e| panic!("the session opens: {:?}", e.class))
}

fn default_console() -> SessionKind {
    SessionKind::Console(ClientId::DEFAULT)
}

/// A session of `kind` over a tracking PostgreSQL fake, inside a transaction.
async fn in_tx(sm: &Arc<SessionManager>, kind: SessionKind) -> SessionHandle {
    let h = open(sm, kind).await;
    run(&h, "BEGIN").await.expect("BEGIN runs");
    assert_eq!(snap(&h).tx.status, TxStatus::InTx);
    h
}

// ---------------------------------------------------------------------------------------------
// Leases

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meta_conn_uses_introspection_priority_and_budget() {
    let profile = pg_n(2);
    let (deps, fake) = deps_with(fake_pg().tracks_transactions());
    let budget = Arc::new(budget_for(&profile));
    let sm = SessionManager::new(
        tokio::runtime::Handle::current(),
        Arc::new(profile.clone()),
        Arc::new(deps),
        budget.clone(),
    );
    let cancel = CancellationToken::new();
    let _console = open(&sm, default_console()).await;
    let first = sm
        .meta_conn(spec_of(&sm, &profile, None), &cancel)
        .await
        .expect("a metadata connection");
    assert_eq!(sm.budget_use(), (2, 2), "it holds a slot of the budget");
    let req = fake.last_request().expect("the metadata connect");
    assert_eq!(req.application_name, META_APPLICATION_NAME);
    assert!(req.read_only, "the metadata connect is read-only");
    assert_eq!(first.dbms(), DbmsKind::Postgres);

    // The budget is spent: a second metadata request (another database, so not the cap)
    // queues at `Introspection`, then a console request at `Console`; the console, the higher
    // priority, gets the freed slot.
    let sm2 = sm.clone();
    let spec2 = spec_of(&sm, &profile, Some("other"));
    let (held_tx, held_rx) = tokio::sync::oneshot::channel::<()>();
    let meta2 = tokio::spawn(async move {
        let cancel = CancellationToken::new();
        let c = sm2.meta_conn(spec2, &cancel).await?;
        let _ = held_rx.await;
        drop(c);
        Ok::<(), MetaError>(())
    });
    settle_until("the metadata request queues", || {
        budget.first_waiter() == Some(Priority::Introspection)
    })
    .await;
    let sm3 = sm.clone();
    let console2 = tokio::spawn(async move { open(&sm3, SessionKind::Console(ClientId(9))).await });
    settle_until("the console request queues first", || {
        budget.first_waiter() == Some(Priority::Console)
    })
    .await;
    drop(first);
    let console2 = tokio::time::timeout(secs(10), console2)
        .await
        .expect("the console gets the freed slot")
        .unwrap();
    assert_eq!(budget.first_waiter(), Some(Priority::Introspection));
    assert!(!meta2.is_finished(), "the metadata request still waits");
    console2
        .close(hacivat_session::session::ClosePolicy::RollBack)
        .await
        .unwrap();
    settle_until("then the metadata request", || {
        budget.first_waiter().is_none()
    })
    .await;
    held_tx.send(()).unwrap();
    let got = tokio::time::timeout(secs(10), meta2)
        .await
        .expect("the metadata request ends");
    assert_eq!(got.unwrap(), Ok(()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_metadata_connection_per_database_by_default() {
    let profile = pg_n(5);
    let (sm, fake) = manager(fake_pg(), profile.clone());
    let cancel = CancellationToken::new();
    let first = sm
        .meta_conn(spec_of(&sm, &profile, Some("orders")), &cancel)
        .await
        .unwrap();
    let sm2 = sm.clone();
    let spec = spec_of(&sm, &profile, Some("orders"));
    let second = tokio::spawn(async move {
        let cancel = CancellationToken::new();
        sm2.meta_conn(spec, &cancel).await.map(drop)
    });
    settle().await;
    assert!(!second.is_finished(), "the cap of the database is 1");
    assert_eq!(fake.connect_calls(), 1);
    assert_eq!(
        fake.last_request().unwrap().database.as_deref(),
        Some("orders")
    );
    drop(first);
    let got = tokio::time::timeout(secs(10), second).await.unwrap();
    assert_eq!(got.unwrap(), Ok(()));
    // A cancelled wait answers `Cancelled`.
    let held = sm
        .meta_conn(spec_of(&sm, &profile, Some("orders")), &cancel)
        .await
        .unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    let waited = sm
        .meta_conn(spec_of(&sm, &profile, Some("orders")), &stop)
        .await;
    assert_eq!(waited.map(drop), Err(MetaError::Cancelled));
    drop(held);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn another_profiles_spec_is_unavailable() {
    let profile = pg_n(3);
    let (sm, _fake) = manager(fake_pg(), profile.clone());
    let cancel = CancellationToken::new();
    let mut spec = spec_of(&sm, &profile, None);
    spec.profile = "6f1c2a4e-1d16-4a16-8a16-000000000099".parse().unwrap();
    assert_eq!(
        sm.meta_conn(spec, &cancel).await.map(drop),
        Err(MetaError::Unavailable)
    );
}

// ---------------------------------------------------------------------------------------------
// The read-only statements per engine

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pg_snapshot_is_a_read_only_repeatable_read_with_the_path_pinned() {
    let profile = pg_n(3);
    let (sm, fake) = manager(
        fake_pg().tracks_transactions().queries(vec![answers(
            "SELECT nspname",
            "nspname",
            &["public"],
        )]),
        profile.clone(),
    );
    let cancel = CancellationToken::new();
    let mut c = sm
        .meta_conn(spec_of(&sm, &profile, None), &cancel)
        .await
        .unwrap();
    let from = fake.call_log().len();
    let mut snap = c.begin_snapshot().await.expect("a snapshot");
    let rows = snap
        .rows(q("SELECT nspname FROM pg_catalog.pg_namespace"))
        .await
        .unwrap();
    assert_eq!(cells(&rows), vec![vec![Some("public".to_string())]]);
    assert_eq!(&*rows.columns[0], "nspname");
    snap.commit().await.unwrap();
    assert_eq!(
        statements_since(&fake, from),
        [
            "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
            "SET LOCAL search_path = pg_catalog, pg_temp",
            "COMMIT"
        ]
    );
    // The set-up ran at connect: the statement timeout of the spec.
    assert!(
        fake.executed()
            .iter()
            .any(|s| s == "SET statement_timeout = 60000"),
        "{:?}",
        fake.executed()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_snapshot_is_rolled_back_at_once() {
    // Review B I4: a snapshot dropped without its commit holds no transaction while the
    // `MetaConn` lives on.
    let profile = pg_n(3);
    let (sm, fake) = manager(fake_pg().tracks_transactions(), profile.clone());
    let cancel = CancellationToken::new();
    let mut c = sm
        .meta_conn(spec_of(&sm, &profile, None), &cancel)
        .await
        .unwrap();
    let from = fake.call_log().len();
    let snap = c.begin_snapshot().await.unwrap();
    drop(snap);
    settle_until("the dropped snapshot rolls back", || {
        statements_since(&fake, from).last().map(String::as_str) == Some("ROLLBACK")
    })
    .await;
    // The next snapshot starts clean: no second rollback.
    let from = fake.call_log().len();
    let snap = c.begin_snapshot().await.unwrap();
    snap.commit().await.unwrap();
    assert_eq!(
        statements_since(&fake, from),
        [
            "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
            "SET LOCAL search_path = pg_catalog, pg_temp",
            "COMMIT"
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_own_snapshot_refuses_transaction_control_text() {
    // Review B M1: a pack text that would end the snapshot never reaches the connection.
    let profile = pg_n(3);
    let (sm, fake) = manager(fake_pg().tracks_transactions(), profile.clone());
    let cancel = CancellationToken::new();
    let mut c = sm
        .meta_conn(spec_of(&sm, &profile, None), &cancel)
        .await
        .unwrap();
    let mut snap = c.begin_snapshot().await.unwrap();
    let from = fake.call_log().len();
    for t in [
        "COMMIT",
        "SET search_path = public",
        "-- x\rCOMMIT\n",
        "DISCARD ALL",
    ] {
        assert_eq!(
            snap.rows(q(t)).await.map(drop),
            Err(MetaError::RefusedText),
            "{t:?}"
        );
    }
    assert_eq!(fake.call_log().len(), from, "nothing was sent");
    snap.commit().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_snapshot_whose_transaction_ended_under_it_is_lost() {
    // Review A M5: a MySQL deadlock (here, the fake's status) ends the transaction silently.
    let profile = pg_n(3);
    let (sm, fake) = manager(
        fake_pg().tracks_transactions().queries(vec![FakeQuery {
            tx_after: Some(TxStatus::Idle),
            ..answers("SELECT relname", "relname", &["orders"])
        }]),
        profile.clone(),
    );
    let cancel = CancellationToken::new();
    let mut c = sm
        .meta_conn(spec_of(&sm, &profile, None), &cancel)
        .await
        .unwrap();
    let connects = fake.connect_calls();
    let mut snap = c.begin_snapshot().await.unwrap();
    assert_eq!(
        snap.rows(q("SELECT relname FROM pg_catalog.pg_class"))
            .await
            .map(drop),
        Err(MetaError::ConnectionLost)
    );
    drop(snap);
    // The next snapshot runs on a new connection.
    c.begin_snapshot().await.unwrap().commit().await.unwrap();
    assert_eq!(fake.connect_calls(), connects + 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mysql_session_is_read_only_and_asserted_before_every_snapshot() {
    let profile = Profile {
        max_connections: 3,
        ..mysql_profile(TlsMode::Disable)
    };
    let (sm, fake) = manager(
        fake_mysql().tracks_transactions().queries(vec![
            session_probe("SELECT @@session.transaction_read_only", 1, "utf8mb4"),
            // The second snapshot finds the session read-write: refused.
            session_probe("SELECT @@session.transaction_read_only", 0, "utf8mb4"),
            // The third finds `SET NAMES gbk` (an `init_connect`, say): refused (T4 carry).
            session_probe("SELECT @@session.transaction_read_only", 1, "gbk"),
        ]),
        profile.clone(),
    );
    let cancel = CancellationToken::new();
    let mut c = sm
        .meta_conn(spec_of(&sm, &profile, None), &cancel)
        .await
        .unwrap();
    let req = fake.last_request().unwrap();
    assert_eq!(req.application_name, META_APPLICATION_NAME);
    assert!(req.read_only);
    let setup: Vec<String> = fake.executed();
    for s in [
        "SET NAMES utf8mb4",
        "SET SESSION TRANSACTION READ ONLY",
        "SET time_zone = '+00:00'",
        "SET SESSION max_execution_time = 60000",
    ] {
        assert!(setup.iter().any(|e| e == s), "{s}: {setup:?}");
    }
    assert!(
        !setup.iter().any(|e| e.contains("group_concat_max_len")),
        "{setup:?}"
    );
    let from = fake.call_log().len();
    let snap = c
        .begin_snapshot()
        .await
        .expect("the read-only session is confirmed");
    snap.commit().await.unwrap();
    assert_eq!(
        queries_since(&fake, from),
        [
            "SELECT @@session.transaction_read_only, @@session.character_set_client, \
             @@session.character_set_connection, @@session.character_set_results"
        ]
    );
    assert_eq!(
        statements_since(&fake, from),
        [
            "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ",
            "START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY",
            "COMMIT"
        ]
    );
    for why in ["not read-only", "not utf8mb4"] {
        let from = fake.call_log().len();
        assert_eq!(
            c.begin_snapshot().await.map(drop),
            Err(MetaError::ReadOnlyNotEnforced),
            "{why}"
        );
        assert!(
            statements_since(&fake, from).is_empty(),
            "{why}: no transaction starts"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mariadb_sets_max_statement_time() {
    let profile = Profile {
        max_connections: 3,
        ..mariadb_profile(TlsMode::Disable)
    };
    let fake = FakeDriver::new(DbmsKind::MariaDb).tracks_transactions();
    let (sm, fake) = manager(fake, profile.clone());
    let cancel = CancellationToken::new();
    let mut spec = spec_of(&sm, &profile, None);
    spec.statement_timeout = Duration::from_millis(1500);
    let _c = sm.meta_conn(spec, &cancel).await.unwrap();
    assert!(
        fake.executed()
            .iter()
            .any(|s| s == "SET SESSION max_statement_time = 1.500"),
        "{:?}",
        fake.executed()
    );
}

// ---------------------------------------------------------------------------------------------
// Single-session profiles: the user's idle session

fn single_pg() -> Profile {
    Profile {
        single_session: true,
        ..pg_n(3)
    }
}

#[tokio::test(start_paused = true)]
async fn single_connection_profile_waits_for_user_idle() {
    let profile = single_pg();
    let (sm, fake) = manager(
        fake_pg()
            .tracks_transactions()
            .exec(vec![slow("SELECT pg_sleep", secs(5))]),
        profile.clone(),
    );
    let user = open(&sm, default_console()).await;
    let ticket = user.submit(exec("SELECT pg_sleep(5)"));
    settle_until("the user's request runs", || {
        matches!(
            snap(&user).state,
            hacivat_session::session::SessionState::Busy { .. }
        )
    })
    .await;
    let cancel = CancellationToken::new();
    let mut pending = Box::pin(sm.meta_conn(spec_of(&sm, &profile, None), &cancel));
    assert!(
        tokio::time::timeout(secs(1), &mut pending).await.is_err(),
        "the metadata connection waits while the user's request runs"
    );
    ticket.reply.await.unwrap().unwrap();
    let c = tokio::time::timeout(secs(10), pending)
        .await
        .expect("granted once the user's session is idle")
        .expect("a metadata connection");
    assert_eq!(c.dbms(), DbmsKind::Postgres);
    assert_eq!(fake.connect_calls(), 1, "no connection of its own");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn never_runs_inside_a_user_transaction() {
    let profile = single_pg();
    let (sm, fake) = manager(
        fake_pg().tracks_transactions().queries(vec![answers(
            "SELECT relname",
            "relname",
            &["orders"],
        )]),
        profile.clone(),
    );
    let user = open(&sm, default_console()).await;
    run(&user, "BEGIN").await.unwrap();
    let cancel = CancellationToken::new();
    assert_eq!(
        sm.meta_conn(spec_of(&sm, &profile, None), &cancel)
            .await
            .map(drop),
        Err(MetaError::Busy)
    );
    run(&user, "COMMIT").await.unwrap();
    let mut c = sm
        .meta_conn(spec_of(&sm, &profile, None), &cancel)
        .await
        .expect("granted after COMMIT");
    let from = fake.call_log().len();
    let mut snap = c.begin_snapshot().await.unwrap();
    let rows = snap
        .rows(q("SELECT relname FROM pg_catalog.pg_class"))
        .await
        .unwrap();
    assert_eq!(cells(&rows), vec![vec![Some("orders".to_string())]]);
    snap.commit().await.unwrap();
    // Hacivat's own read-only transaction around the read, rolled back (ruling R-W1-9).
    let mut own = PG_USER_READ.to_vec();
    own.push("ROLLBACK");
    assert_eq!(statements_since(&fake, from), own);
    assert_eq!(snap_tx(&user), TxStatus::Idle);
    // The user begins again: the next read is refused before anything is sent.
    run(&user, "BEGIN").await.unwrap();
    let from = fake.call_log().len();
    assert_eq!(c.begin_snapshot().await.map(drop), Err(MetaError::Busy));
    assert_eq!(fake.call_log().len(), from);
    // Only reads reach the user's session.
    run(&user, "COMMIT").await.unwrap();
    let mut snap = c.begin_snapshot().await.unwrap();
    let from = fake.call_log().len();
    assert_eq!(
        snap.rows(q("DELETE FROM orders")).await.map(drop),
        Err(MetaError::RefusedText)
    );
    assert_eq!(fake.call_log().len(), from, "refused text sends nothing");
}

/// A single-session PostgreSQL profile whose console runs in `mode`, with one read answered.
async fn user_session_in(
    mode: TxMode,
) -> (Arc<SessionManager>, Arc<FakeDriver>, SessionHandle, Profile) {
    let profile = single_pg();
    let (sm, fake) = manager(
        fake_pg().tracks_transactions().queries(vec![answers(
            "SELECT relname",
            "relname",
            &["orders"],
        )]),
        profile.clone(),
    );
    let user = open(&sm, default_console()).await;
    tx_cmd(&user, TxCommand::SetMode(mode)).await.unwrap();
    assert_eq!(snap(&user).tx.mode, mode);
    (sm, fake, user, profile)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_read_leaves_a_manual_or_smart_session_idle() {
    // Review A C1, review B C1: the read never takes the user's run path, so Manual and Smart
    // never begin a user transaction for it.
    for mode in [TxMode::Manual, TxMode::Smart, TxMode::Auto] {
        let (sm, fake, user, profile) = user_session_in(mode).await;
        let cancel = CancellationToken::new();
        let mut c = sm
            .meta_conn(spec_of(&sm, &profile, None), &cancel)
            .await
            .unwrap();
        let from = fake.call_log().len();
        let mut s = c.begin_snapshot().await.unwrap();
        let rows = s
            .rows(q("SELECT relname FROM pg_catalog.pg_class"))
            .await
            .unwrap_or_else(|e| panic!("{mode:?}: {e:?}"));
        assert_eq!(cells(&rows), vec![vec![Some("orders".to_string())]]);
        s.commit().await.unwrap();
        let mut own = PG_USER_READ.to_vec();
        own.push("ROLLBACK");
        assert_eq!(statements_since(&fake, from), own, "{mode:?}");
        let after = snap(&user).tx;
        assert_eq!(after.status, TxStatus::Idle, "{mode:?}");
        assert_eq!(after.statements, 0, "{mode:?}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_read_queued_behind_a_user_begin_answers_busy_and_sends_nothing() {
    // Review A I1, review B I1: the actor checks the session when it takes the read.
    for mode in [TxMode::Manual, TxMode::Smart] {
        let (sm, fake, user, profile) = user_session_in(mode).await;
        let cancel = CancellationToken::new();
        let mut c = sm
            .meta_conn(spec_of(&sm, &profile, None), &cancel)
            .await
            .unwrap();
        let mut s = c.begin_snapshot().await.unwrap();
        let from = fake.call_log().len();
        // Queued, not yet taken (a current-thread runtime runs nothing until the next await).
        let begin = user.submit(exec("BEGIN"));
        let got = s.rows(q("SELECT relname FROM pg_catalog.pg_class")).await;
        begin.reply.await.unwrap().unwrap();
        assert_eq!(got.map(drop), Err(MetaError::Busy), "{mode:?}");
        // Only the user's BEGIN (and the session's own schema probe after it) ran: no query
        // and no statement of the read.
        let sent: Vec<FakeCall> = fake.call_log()[from..].to_vec();
        assert_eq!(sent.first(), Some(&FakeCall::Statement("BEGIN".into())));
        assert!(
            sent.iter().all(|c| match c {
                FakeCall::Typed(_) => false,
                FakeCall::Statement(s) => !PG_USER_READ.contains(&s.as_str()) && s != "ROLLBACK",
                FakeCall::Reset => true,
            }),
            "{mode:?}: {sent:?}"
        );
        assert_eq!(snap(&user).tx.status, TxStatus::InTx);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_read_moves_no_run_context() {
    // Review A I4: a run the console classified before a metadata read is not refused.
    let (sm, _fake, user, profile) = user_session_in(TxMode::Auto).await;
    let (_, generation) = user.run_context().await;
    let generation = generation.expect("a generation");
    let cancel = CancellationToken::new();
    let mut c = sm
        .meta_conn(spec_of(&sm, &profile, None), &cancel)
        .await
        .unwrap();
    let mut s = c.begin_snapshot().await.unwrap();
    s.rows(q("SELECT relname FROM pg_catalog.pg_class"))
        .await
        .unwrap();
    s.commit().await.unwrap();
    let (_, now) = user.run_context().await;
    assert_eq!(now, Some(generation), "the context generation did not move");
    let o = common::runs::Recorder::new();
    let mut run = common::runs::spec_of(&o, &["SELECT 1"]);
    run.context_generation = Some(generation);
    let reply = common::runs::run_spec(&user, run).await;
    assert!(
        !matches!(&reply, Err(e) if matches!(e.kind, ExecErrorKind::ContextChanged)),
        "{:?}",
        reply.map(drop).map_err(|e| e.kind)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mysql_user_session_read_needs_autocommit_and_utf8mb4() {
    let profile = Profile {
        single_session: true,
        max_connections: 3,
        ..mysql_profile(TlsMode::Disable)
    };
    let (sm, fake) = manager(
        fake_mysql()
            .tracks_transactions()
            .autocommit_flag(vec![
                ("SET autocommit = 0", false),
                ("SET autocommit = 1", true),
            ])
            .queries(vec![
                charset_probe("utf8mb4"),
                answers("SELECT table_name", "table_name", &["orders"]),
                charset_probe("gbk"),
            ]),
        profile.clone(),
    );
    let user = open(&sm, default_console()).await;
    let cancel = CancellationToken::new();
    let mut c = sm
        .meta_conn(spec_of(&sm, &profile, None), &cancel)
        .await
        .unwrap();
    let read = "SELECT table_name FROM information_schema.tables WHERE table_schema = ?";
    let binds = [text("shop")];
    // Idle, autocommit on, utf8mb4: Hacivat's own read-only transaction, rolled back.
    let from = fake.call_log().len();
    let mut s = c.begin_snapshot().await.unwrap();
    let rows = s.rows(MetaQuery::new(read, &binds, false)).await.unwrap();
    assert_eq!(cells(&rows), vec![vec![Some("orders".to_string())]]);
    s.commit().await.unwrap();
    assert_eq!(
        statements_since(&fake, from),
        [
            "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ",
            "START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY",
            "ROLLBACK"
        ]
    );
    // The user turned autocommit off: every SELECT would open a transaction. Refused, nothing
    // sent.
    run(&user, "SET autocommit = 0").await.unwrap();
    let mut s = c.begin_snapshot().await.unwrap();
    let from = fake.call_log().len();
    assert_eq!(
        s.rows(MetaQuery::new(read, &binds, false)).await.map(drop),
        Err(MetaError::Busy)
    );
    assert_eq!(fake.call_log().len(), from);
    drop(s);
    // Autocommit back on, but the connection's character sets are not utf8mb4: refused before
    // any transaction.
    run(&user, "SET autocommit = 1").await.unwrap();
    let mut s = c.begin_snapshot().await.unwrap();
    let from = fake.call_log().len();
    assert_eq!(
        s.rows(MetaQuery::new(read, &binds, false)).await.map(drop),
        Err(MetaError::ReadOnlyNotEnforced)
    );
    assert!(statements_since(&fake, from).is_empty());
    assert_eq!(snap_tx(&user), TxStatus::Idle);
}

// ---------------------------------------------------------------------------------------------
// Connection loss, binds, disconnect

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connection_loss_reports_and_does_not_retry_silently() {
    let profile = pg_n(3);
    let (sm, fake) = manager(fake_pg().tracks_transactions(), profile.clone());
    let cancel = CancellationToken::new();
    let mut c = sm
        .meta_conn(spec_of(&sm, &profile, None), &cancel)
        .await
        .unwrap();
    let mut snap = c.begin_snapshot().await.unwrap();
    let connects = fake.connect_calls();
    fake.kill_connections();
    assert_eq!(
        snap.rows(q("SELECT 1")).await.map(drop),
        Err(MetaError::ConnectionLost)
    );
    assert_eq!(
        snap.rows(q("SELECT 2")).await.map(drop),
        Err(MetaError::ConnectionLost),
        "the snapshot is gone: nothing is read again"
    );
    drop(snap);
    assert_eq!(fake.connect_calls(), connects, "no silent reconnect");
    // The next snapshot connects again and runs the set-up again.
    let timeouts_before = fake
        .executed()
        .iter()
        .filter(|s| s.starts_with("SET statement_timeout"))
        .count();
    let snap = c.begin_snapshot().await.expect("a new connection");
    snap.commit().await.unwrap();
    assert_eq!(fake.connect_calls(), connects + 1);
    let timeouts_after = fake
        .executed()
        .iter()
        .filter(|s| s.starts_with("SET statement_timeout"))
        .count();
    assert_eq!(timeouts_after, timeouts_before + 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn binds_are_never_spliced() {
    const SECRET_NAME: &str = "hv_bound_orders";
    let sql = "SELECT relname FROM pg_catalog.pg_class WHERE relname = $1";
    let binds = [text(SECRET_NAME)];
    // Its own connection.
    let profile = pg_n(3);
    let (sm, fake) = manager(fake_pg().tracks_transactions(), profile.clone());
    let cancel = CancellationToken::new();
    let mut c = sm
        .meta_conn(spec_of(&sm, &profile, None), &cancel)
        .await
        .unwrap();
    let mut snap = c.begin_snapshot().await.unwrap();
    snap.rows(MetaQuery::new(sql, &binds, false)).await.unwrap();
    snap.commit().await.unwrap();
    // A joined job session.
    let job = in_tx(&sm, SessionKind::Job(1, JobKind::DriftCheck)).await;
    let mut joined = MetaConn::join(&job).unwrap();
    let mut snap = joined.begin_snapshot().await.unwrap();
    snap.rows(MetaQuery::new(sql, &binds, true)).await.unwrap();
    snap.commit().await.unwrap();
    let texts: Vec<String> = fake
        .call_log()
        .iter()
        .filter_map(|c| match c {
            FakeCall::Statement(s) | FakeCall::Typed(FakeTypedCall::Query(s)) => Some(s.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(texts.iter().filter(|t| *t == sql).count(), 2, "{texts:?}");
    assert!(
        texts.iter().all(|t| !t.contains(SECRET_NAME)),
        "a bind value reached the text: {texts:?}"
    );
    let bound = fake.binds_seen();
    assert_eq!(
        bound.iter().filter(|b| b.as_slice() == binds).count(),
        2,
        "the value travels as a bind"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_metadata_connection_does_not_outlive_the_profiles_disconnect() {
    let profile = pg_n(3);
    let (sm, fake) = manager(fake_pg().tracks_transactions(), profile.clone());
    let cancel = CancellationToken::new();
    let mut c = sm
        .meta_conn(spec_of(&sm, &profile, None), &cancel)
        .await
        .unwrap();
    assert_eq!(fake.open_connections(), 1);
    sm.close_all().await;
    settle_until("the metadata connection closes", || {
        fake.open_connections() == 0
    })
    .await;
    settle_until("its permit goes back", || sm.budget_use().0 == 0).await;
    assert_eq!(
        c.begin_snapshot().await.map(drop),
        Err(MetaError::Unavailable)
    );
    assert_eq!(
        fake.connect_calls(),
        1,
        "never reconnected after the disconnect"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_waiting_at_the_disconnect_never_connects() {
    let profile = pg_n(1);
    let (deps, fake) = deps_with(fake_pg());
    let budget = Arc::new(budget_for(&profile));
    let sm = SessionManager::new(
        tokio::runtime::Handle::current(),
        Arc::new(profile.clone()),
        Arc::new(deps),
        budget.clone(),
    );
    let _console = open(&sm, default_console()).await;
    let sm2 = sm.clone();
    let spec = spec_of(&sm, &profile, None);
    let waiting = tokio::spawn(async move {
        let cancel = CancellationToken::new();
        sm2.meta_conn(spec, &cancel).await.map(drop)
    });
    settle_until("the request waits for the console's slot", || {
        budget.first_waiter() == Some(Priority::Introspection)
    })
    .await;
    let connects = fake.connect_calls();
    sm.close_all().await;
    let got = tokio::time::timeout(secs(10), waiting).await.unwrap();
    assert_eq!(got.unwrap(), Err(MetaError::Unavailable));
    assert_eq!(fake.connect_calls(), connects);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_a_metadata_connection_closes_it_and_frees_its_slot() {
    let profile = pg_n(3);
    let (sm, fake) = manager(fake_pg(), profile.clone());
    let cancel = CancellationToken::new();
    let c = sm
        .meta_conn(spec_of(&sm, &profile, None), &cancel)
        .await
        .unwrap();
    assert_eq!(sm.budget_use().0, 1);
    drop(c);
    settle_until("closed", || fake.open_connections() == 0).await;
    settle_until("the slot is free", || sm.budget_use().0 == 0).await;
}

#[tokio::test]
async fn an_offline_meta_conn_stays_inert() {
    for dbms in [
        DbmsKind::Postgres,
        DbmsKind::MySql,
        DbmsKind::MariaDb,
        DbmsKind::Sqlite,
    ] {
        let mut c = MetaConn::offline(dbms);
        assert_eq!(c.dbms(), dbms);
        assert_eq!(
            c.begin_snapshot().await.map(drop),
            Err(MetaError::Unavailable)
        );
        assert_eq!(
            c.attach_read_only("aux", Path::new("/tmp/x.db")).await,
            Err(MetaError::Unavailable)
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Joined connections (R-M4-29)

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn joined_conn_is_refused_on_console_and_editor_sessions() {
    let (sm, _fake) = manager(fake_pg().tracks_transactions(), pg_n(8));
    for kind in [
        default_console(),
        SessionKind::Console(ClientId(4)),
        SessionKind::Editor("orders".into()),
        SessionKind::Job(2, JobKind::Export),
        SessionKind::Job(3, JobKind::Import),
    ] {
        let h = in_tx(&sm, kind.clone()).await;
        assert_eq!(
            MetaConn::join(&h).map(drop),
            Err(MetaError::NotJoinable),
            "{kind:?}"
        );
    }
    // A job of the right kind, but outside a transaction.
    let idle = open(&sm, SessionKind::Job(5, JobKind::DdlApply)).await;
    assert_eq!(MetaConn::join(&idle).map(drop), Err(MetaError::NotJoinable));
    // An introspection session is never joined either.
    let pooled = sm
        .pooled(SessionKind::Introspection { database: None })
        .await
        .unwrap();
    assert_eq!(
        MetaConn::join(&pooled).map(drop),
        Err(MetaError::NotJoinable)
    );
    // The two job kinds R-M4-29 names are.
    for job in [JobKind::DdlApply, JobKind::DriftCheck] {
        let h = in_tx(&sm, SessionKind::Job(6, job)).await;
        let c = MetaConn::join(&h).expect("joinable");
        assert!(format!("{c:?}").contains("joined"), "{c:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn joined_conn_never_begins_or_ends_the_txn() {
    let (sm, fake) = manager(
        fake_pg().tracks_transactions().queries(vec![
            answers("SELECT a", "a", &["1"]),
            answers("SELECT b", "b", &["2"]),
            answers("SELECT c", "c", &["3"]),
        ]),
        pg_n(4),
    );
    let job = in_tx(&sm, SessionKind::Job(1, JobKind::DdlApply)).await;
    let mut c = MetaConn::join(&job).unwrap();
    let from = fake.call_log().len();
    let texts = [
        "SELECT a FROM pg_catalog.pg_class",
        "SELECT b FROM pg_catalog.pg_attribute",
        "SELECT c FROM pg_catalog.pg_constraint",
    ];
    let mut snap = c.begin_snapshot().await.unwrap();
    let mut got = Vec::new();
    for t in texts {
        got.push(cells(&snap.rows(joinable(t)).await.unwrap()));
    }
    snap.commit().await.unwrap();
    assert_eq!(
        got,
        [
            vec![vec![Some("1".to_string())]],
            vec![vec![Some("2".to_string())]],
            vec![vec![Some("3".to_string())]]
        ]
    );
    let log = fake.call_log()[from..].to_vec();
    assert_eq!(
        log,
        texts
            .iter()
            .map(|t| FakeCall::Typed(FakeTypedCall::Query((*t).to_string())))
            .collect::<Vec<_>>(),
        "exactly the three pack texts"
    );
    assert_eq!(
        snap_tx(&job),
        TxStatus::InTx,
        "the job's transaction is untouched"
    );
}

fn snap_tx(h: &SessionHandle) -> TxStatus {
    snap(h).tx.status
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn joined_conn_refuses_non_joinable_queries() {
    let (sm, fake) = manager(fake_pg().tracks_transactions(), pg_n(4));
    let job = in_tx(&sm, SessionKind::Job(1, JobKind::DriftCheck)).await;
    let mut c = MetaConn::join(&job).unwrap();
    let from = fake.call_log().len();
    let mut snap = c.begin_snapshot().await.unwrap();
    assert_eq!(
        snap.rows(q("SELECT relname FROM pg_catalog.pg_class"))
            .await
            .map(drop),
        Err(MetaError::NotJoinable)
    );
    snap.commit().await.unwrap();
    assert_eq!(fake.call_log().len(), from, "nothing was sent");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn joined_conn_refuses_transaction_control_text() {
    let (sm, fake) = manager(fake_pg().tracks_transactions(), pg_n(4));
    let job = in_tx(&sm, SessionKind::Job(1, JobKind::DdlApply)).await;
    let mut c = MetaConn::join(&job).unwrap();
    let from = fake.call_log().len();
    let mut snap = c.begin_snapshot().await.unwrap();
    for t in [
        "SET search_path = x",
        "begin",
        " COMMIT",
        "SAVEPOINT a",
        "ROLLBACK",
        "ROLLBACK TO SAVEPOINT a",
        "RELEASE SAVEPOINT a",
        "START TRANSACTION",
        "END",
        "ABORT",
        "PREPARE TRANSACTION 'x'",
        "XA END 'x'",
        "SELECT 1; COMMIT",
        "/*!50000 COMMIT */ SELECT 1",
        "/* /* */ SELECT 1 */ COMMIT",
        "WITH x AS (DELETE FROM t RETURNING 1) SELECT * FROM x",
        "SELECT * INTO t2 FROM t",
    ] {
        assert_eq!(
            snap.rows(joinable(t)).await.map(drop),
            Err(MetaError::RefusedText),
            "{t:?}"
        );
    }
    snap.commit().await.unwrap();
    assert_eq!(fake.call_log().len(), from, "nothing was sent");
    assert_eq!(snap_tx(&job), TxStatus::InTx);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_joined_conn_stops_when_the_transaction_ends() {
    let (sm, _fake) = manager(fake_pg().tracks_transactions(), pg_n(4));
    let job = in_tx(&sm, SessionKind::Job(1, JobKind::DdlApply)).await;
    let mut c = MetaConn::join(&job).unwrap();
    run(&job, "COMMIT").await.unwrap();
    assert_eq!(
        c.begin_snapshot().await.map(drop),
        Err(MetaError::NotJoinable)
    );
}

// ---------------------------------------------------------------------------------------------
// SQLite (real files, no Docker)

/// A SQLite file with one table `t(a)` holding `1`, made through the real driver (this crate
/// has no `rusqlite` dependency of its own).
async fn sqlite_file(dir: &Path, name: &str) -> std::path::PathBuf {
    use hacivat_driver::{Driver, DriverConnect, Endpoint, InsecureAuth};
    let path = dir.join(name);
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
        .expect("the fixture file opens");
    conn.batch("CREATE TABLE t(a)").await.unwrap();
    conn.batch("INSERT INTO t VALUES (1)").await.unwrap();
    conn.close().await;
    path
}

fn sqlite_on(path: &Path) -> Profile {
    Profile {
        engine: DbmsKind::Sqlite,
        endpoint: ProfileEndpoint::File {
            path: path.to_path_buf(),
            create_if_missing: false,
        },
        user: None,
        ..sqlite_profile()
    }
}

/// A manager of `profile` over the real drivers.
fn real_manager(profile: Profile) -> Arc<SessionManager> {
    manager_over(common::docker_support::deps(), profile)
}

fn vendor_code(e: &MetaError) -> Option<i64> {
    match e {
        MetaError::Server { message, .. } => message.vendor_code,
        _ => None,
    }
}

/// SQLITE_READONLY (8) or one of its extended codes (`(8 | n << 8)`).
fn is_readonly(e: &MetaError) -> bool {
    vendor_code(e).is_some_and(|c| c & 0xff == 8)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_meta_uses_a_read_only_connection_never_the_users() {
    let dir = tempfile::tempdir().unwrap();
    let path = sqlite_file(dir.path(), "main.db").await;
    let profile = sqlite_on(&path);
    let sm = real_manager(profile.clone());
    let user = open(&sm, default_console()).await;
    run(&user, "CREATE TEMP TABLE only_on_the_users(a)")
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    let mut c = sm
        .meta_conn(spec_of(&sm, &profile, None), &cancel)
        .await
        .unwrap();
    assert_eq!(c.dbms(), DbmsKind::Sqlite);
    let mut snap = c.begin_snapshot().await.unwrap();
    let main = snap
        .rows(q("SELECT name FROM sqlite_master WHERE type = 'table'"))
        .await
        .unwrap();
    assert_eq!(cells(&main), vec![vec![Some("t".to_string())]]);
    let temp = snap
        .rows(q("SELECT count(*) FROM temp.sqlite_master"))
        .await
        .unwrap();
    assert_eq!(
        cells(&temp),
        vec![vec![Some("0".to_string())]],
        "another connection: the user's temp table is not there"
    );
    // The token guard refuses the write (fix round 2, review B R-M3); past it, the engine
    // does (`unguarded`, with the `test-support` feature).
    assert_eq!(
        snap.rows(q("INSERT INTO t VALUES (2)")).await.map(drop),
        Err(MetaError::RefusedText)
    );
    if let Some(write) = unguarded(&mut snap, q("INSERT INTO t VALUES (2)")).await {
        assert!(
            write.as_ref().is_err_and(is_readonly),
            "read-only: {write:?}"
        );
    }
    snap.commit().await.unwrap();
}

// ## m4-t05-meta (fix round 3)
/// The engine's own answer past the token guard: `SnapshotTxn::rows_unguarded`, which exists
/// only in a debug build with this crate's `test-support` feature (on in `cargo test
/// --workspace`, where `hacivat-exec`'s and `hacivat-app`'s dev-dependencies enable it).
/// `None` without it: the guard's refusal, asserted before each call, still runs.
async fn unguarded<'q>(
    snap: &'q mut hacivat_session::meta::SnapshotTxn<'_>,
    q: MetaQuery<'q>,
) -> Option<Result<Rows, MetaError>> {
    #[cfg(all(debug_assertions, feature = "test-support"))]
    {
        Some(snap.rows_unguarded(q).await)
    }
    #[cfg(not(all(debug_assertions, feature = "test-support")))]
    {
        let _ = (snap, q);
        None
    }
}

#[test]
fn rows_unguarded_is_named_only_in_tests() {
    // Fix round 3: the guard-free read is a test seam. Outside its definition in `meta.rs`, only
    // files under a `tests/` directory may name it.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let mut offenders = Vec::new();
    let mut stack = vec![root.join("crates"), root.join("xtask"), root.join("fuzz")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if path.is_dir() {
                if name != "target" && name != "tests" {
                    stack.push(path);
                }
                continue;
            }
            if path.extension().is_some_and(|e| e == "rs")
                && std::fs::read_to_string(&path).is_ok_and(|s| s.contains("rows_unguarded"))
                && !path.ends_with("crates/core/hacivat-session/src/meta.rs")
            {
                offenders.push(path);
            }
        }
    }
    assert!(offenders.is_empty(), "{offenders:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_meta_conn_has_no_authorizer_and_refuses_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = sqlite_file(dir.path(), "ro.db").await;
    // A read-only profile: the user's connection has the authorizer, the reader does not.
    let profile = ro(sqlite_on(&path));
    let sm = real_manager(profile.clone());
    let _user = open(&sm, default_console()).await;
    let cancel = CancellationToken::new();
    let mut c = sm
        .meta_conn(spec_of(&sm, &profile, None), &cancel)
        .await
        .unwrap();
    // `BEGIN` (the snapshot) and `pragma_table_list` work: no authorizer.
    let mut snap = c.begin_snapshot().await.expect("BEGIN runs");
    let tables = snap
        .rows(q(
            "SELECT name FROM pragma_table_list WHERE schema = 'main' AND name = 't'",
        ))
        .await
        .expect("pragma_table_list runs");
    assert_eq!(cells(&tables), vec![vec![Some("t".to_string())]]);
    assert_eq!(
        snap.rows(q("CREATE TABLE u(a)")).await.map(drop),
        Err(MetaError::RefusedText)
    );
    if let Some(ddl) = unguarded(&mut snap, q("CREATE TABLE u(a)")).await {
        assert!(ddl.as_ref().is_err_and(is_readonly), "{ddl:?}");
    }
    drop(snap);
    let mut snap = c.begin_snapshot().await.unwrap();
    assert_eq!(
        snap.rows(q("CREATE TABLE u(a)")).await.map(drop),
        Err(MetaError::RefusedText)
    );
    if let Some(ddl) = unguarded(&mut snap, q("CREATE TABLE u(a)")).await {
        assert!(ddl.as_ref().is_err_and(is_readonly), "{ddl:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_attached_files_are_read_only_too() {
    let dir = tempfile::tempdir().unwrap();
    let path = sqlite_file(dir.path(), "main.db").await;
    let aux = sqlite_file(dir.path(), "arsiv.db").await;
    let profile = sqlite_on(&path);
    let sm = real_manager(profile.clone());
    let cancel = CancellationToken::new();
    let mut c = sm
        .meta_conn(spec_of(&sm, &profile, None), &cancel)
        .await
        .unwrap();
    c.attach_read_only("aux", &aux).await.expect("attached");
    let mut snap = c.begin_snapshot().await.unwrap();
    let rows = snap.rows(q("SELECT a FROM aux.t")).await.unwrap();
    assert_eq!(cells(&rows), vec![vec![Some("1".to_string())]]);
    for write in ["CREATE TABLE aux.u(a)", "INSERT INTO aux.t VALUES (2)"] {
        assert_eq!(
            snap.rows(q(write)).await.map(drop),
            Err(MetaError::RefusedText),
            "{write}"
        );
        if let Some(e) = unguarded(&mut snap, q(write)).await {
            assert!(e.as_ref().is_err_and(is_readonly), "{write}: {e:?}");
        }
    }
    snap.commit().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_temp_is_read_on_the_users_connection_only_when_idle() {
    let dir = tempfile::tempdir().unwrap();
    let path = sqlite_file(dir.path(), "main.db").await;
    let profile = sqlite_on(&path);
    let sm = real_manager(profile.clone());
    let user = open(&sm, default_console()).await;
    run(&user, "CREATE TEMP TABLE tt(a)").await.unwrap();
    let cancel = CancellationToken::new();
    let mut c = sm
        .meta_conn(spec_of(&sm, &profile, Some("temp")), &cancel)
        .await
        .expect("the user's session is idle");
    let mut snap = c.begin_snapshot().await.unwrap();
    let rows = snap
        .rows(q(
            "SELECT name FROM temp.sqlite_master WHERE type = 'table'",
        ))
        .await
        .unwrap();
    assert_eq!(cells(&rows), vec![vec![Some("tt".to_string())]]);
    snap.commit().await.unwrap();
    // Inside the user's transaction: refused, before and after acquisition.
    run(&user, "BEGIN").await.unwrap();
    assert_eq!(c.begin_snapshot().await.map(drop), Err(MetaError::Busy));
    assert_eq!(
        sm.meta_conn(spec_of(&sm, &profile, Some("temp")), &cancel)
            .await
            .map(drop),
        Err(MetaError::Busy)
    );
    run(&user, "ROLLBACK").await.unwrap();
    assert!(c.begin_snapshot().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_meta_read_runs_while_the_console_holds_the_single_session() {
    // U-6(e): the reader does not count against SQLite's budget of 1.
    let dir = tempfile::tempdir().unwrap();
    let path = sqlite_file(dir.path(), "main.db").await;
    let profile = sqlite_on(&path);
    let sm = real_manager(profile.clone());
    let user = open(&sm, default_console()).await;
    run(&user, "BEGIN").await.unwrap();
    run(&user, "INSERT INTO t VALUES (2)").await.unwrap();
    assert_eq!(
        sm.budget_use(),
        (1, 1),
        "the console holds the single session"
    );
    let cancel = CancellationToken::new();
    let mut c = tokio::time::timeout(
        secs(10),
        sm.meta_conn(spec_of(&sm, &profile, None), &cancel),
    )
    .await
    .expect("the reader never waits for the budget")
    .expect("a reader");
    assert_eq!(sm.budget_use(), (1, 1));
    let mut snap = c.begin_snapshot().await.unwrap();
    let rows = snap.rows(q("SELECT count(*) FROM t")).await.unwrap();
    assert_eq!(
        cells(&rows),
        vec![vec![Some("1".to_string())]],
        "the committed state, not the console's open transaction"
    );
    snap.commit().await.unwrap();
    run(&user, "ROLLBACK").await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_memory_reads_on_the_users_session() {
    let profile = sqlite_profile();
    let sm = real_manager(profile.clone());
    let user = open(&sm, default_console()).await;
    run(&user, "CREATE TABLE m(a)").await.unwrap();
    let cancel = CancellationToken::new();
    let mut c = sm
        .meta_conn(spec_of(&sm, &profile, None), &cancel)
        .await
        .unwrap();
    let mut snap = c.begin_snapshot().await.unwrap();
    let rows = snap
        .rows(q("SELECT name FROM sqlite_master WHERE type = 'table'"))
        .await
        .unwrap();
    assert_eq!(cells(&rows), vec![vec![Some("m".to_string())]]);
    snap.commit().await.unwrap();
}
