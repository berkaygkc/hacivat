//! The `ConnectionManager` over a `SessionManager` (HV-CONN-04, -05; D-085; B4, B18, B19, B20;
//! Task 17): liveness, recovery with backoff and replay, session-lost events, idle disconnect,
//! the session overlay on reconnect (Q17) and SSH group invalidation.
//!
//! Every test over `FakeDriver` alone runs on a paused clock and waits on conditions (Q25,
//! review I5); the SSH tests use real sockets, so real time (the in-process testkit server).

mod common;

use std::cell::RefCell;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use futures::FutureExt as _;
use futures::future::BoxFuture;
use hacivat_driver::fake::{FakeDriver, FakeExec};
use hacivat_driver::{ExecOpts, TxStatus};
use hacivat_net::{TlsMode, TlsSummary, TlsVerified};
use hacivat_proto::{DbmsKind, ErrClass, ProfileId};
use hacivat_secrets::{AuthProviderId, Retention, SecretSlot, SecretString};
use hacivat_session::pipeline::{Answer, Interaction, ProfileChange, Question, UiPort};
use hacivat_session::profile::Profile;
use hacivat_session::session::{
    ClientId, ClosePolicy, Escalation, ExecErrorKind, ExecOrigin, IsolationLevel, Request,
    SessionHandle, SessionKind, SessionManager, SessionSpec, SessionState, TxCommand, TxMode,
};
use hacivat_session::{ConnectionEvent, ConnectionManager, LiveState, ManagerConfig, budget_for};
use hacivat_testkit::ssh_server::TestSshServer;
use tokio::sync::broadcast::error::TryRecvError;

// ---------------------------------------------------------------------------------------------
// Fixtures

/// M2b recovery (B19) with M2a's keep-alive (every profile pings every 30 s), so a Direct test
/// profile is pinged as the brief's tests expect.
fn config() -> ManagerConfig {
    ManagerConfig {
        reconnect: true,
        ..ManagerConfig::default()
    }
}

thread_local! {
    /// What `connect_and_wait` connects in this test: the profile and how it asks.
    static SETUP: RefCell<Option<(Arc<Profile>, Interaction)>> = const { RefCell::new(None) };
}

fn set_up(profile: Profile, ix: Interaction) {
    SETUP.with(|s| *s.borrow_mut() = Some((Arc::new(profile), ix)));
}

fn current() -> (Arc<Profile>, Interaction) {
    SETUP
        .with(|s| s.borrow().clone())
        .unwrap_or_else(|| (p(), Interaction::Headless))
}

fn manager(fake: FakeDriver) -> (Arc<ConnectionManager>, Arc<FakeDriver>) {
    manager_with(fake, config())
}

/// [`manager`] for `profile` (same ID as `p()`).
fn manager_profile(
    fake: FakeDriver,
    profile: Profile,
) -> (Arc<ConnectionManager>, Arc<FakeDriver>) {
    set_up(profile, Interaction::Headless);
    manager(fake)
}

async fn connect_and_wait(m: &ConnectionManager) {
    let (profile, ix) = current();
    m.connect(profile.clone(), None, ix);
    wait_connected(m, profile.id).await;
}

/// The test profile with an idle disconnect of `s` seconds.
fn idle(s: u32) -> Profile {
    Profile {
        idle_disconnect_s: Some(s),
        ..pg_profile(TlsMode::Disable)
    }
}

/// Waits for `done`, letting a paused clock run (10 ms steps); panics after [`WAIT`] of real
/// time.
async fn until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + WAIT;
    while !done() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(ms(10)).await;
    }
}

/// Waits (letting a paused clock run) until `id`'s state matches; panics after [`WAIT`] of
/// real time.
async fn wait_state_of(
    m: &ConnectionManager,
    id: ProfileId,
    what: impl Fn(&LiveState) -> bool,
) -> LiveState {
    let deadline = std::time::Instant::now() + WAIT;
    let mut changes = m.changes();
    loop {
        let state = m.state(id);
        if what(&state) {
            return state;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting; the state is {state:?}"
        );
        tokio::select! {
            _ = changes.changed() => {}
            () = tokio::time::sleep(ms(50)) => {}
        }
    }
}

async fn wait_state(m: &ConnectionManager, what: impl Fn(&LiveState) -> bool) -> LiveState {
    wait_state_of(m, p().id, what).await
}

fn default_session(m: &ConnectionManager) -> SessionHandle {
    m.sessions(p().id)
        .expect("the profile is connected")
        .default_session()
        .expect("it has a default session")
}

fn user_exec(sql: &str) -> Request {
    Request::Exec {
        sql: sql.to_string(),
        opts: ExecOpts::default(),
        origin: ExecOrigin::User,
    }
}

fn internal_exec(sql: &str) -> Request {
    Request::Exec {
        sql: sql.to_string(),
        opts: ExecOpts::default(),
        origin: ExecOrigin::Internal { idempotent: true },
    }
}

/// A statement whose run fails as if the connection reset.
fn fails_once_with_reset(prefix: &'static str) -> FakeExec {
    FakeExec {
        prefix,
        result: Err(io_reset()),
        delay: Duration::ZERO,
        tx_after: None,
        panics: false,
    }
}

/// The server drops every connection; the default session finds out at its next round trip
/// (a ping here).
fn drop_connections(m: &ConnectionManager, f: &FakeDriver) {
    f.kill_connections();
    let _ = default_session(m).submit(Request::Ping);
}

/// [`drop_connections`], then waits for the reconnect's connect.
async fn lose_connection(m: &ConnectionManager, f: &FakeDriver) {
    let before = f.connect_calls();
    drop_connections(m, f);
    until("the reconnect's connect", || f.connect_calls() > before).await;
}

async fn ok(s: &SessionHandle, request: Request) {
    let reply = s.submit(request).reply.await.unwrap();
    assert!(reply.is_ok(), "{reply:?}");
}

async fn set_schema(m: &ConnectionManager, name: &str) {
    ok(&default_session(m), Request::SwitchSchema(name.to_string())).await;
}

async fn set_mode(m: &ConnectionManager, mode: TxMode) {
    ok(&default_session(m), Request::Tx(TxCommand::SetMode(mode))).await;
}

/// The error kind `request` gets from `s`.
async fn refusal(s: &SessionHandle, request: Request) -> ExecErrorKind {
    s.submit(request).reply.await.unwrap().unwrap_err().kind
}

const NEVER_RAN: ExecErrorKind = ExecErrorKind::ConnectionLost {
    outcome_unknown: false,
};

/// The schema probe answering `app` (database `shop`).
fn probe_answers_app() -> FakeExec {
    rows(
        "SELECT current_database()",
        &[&["shop", "app", "app", "deploy", "app"]],
    )
}

/// A read-only profile over `fake`, whose schema probe answers `app`.
fn manager_ro(fake: FakeDriver) -> (Arc<ConnectionManager>, Arc<FakeDriver>) {
    manager_profile(
        fake.exec(vec![probe_answers_app()]),
        ro(pg_profile(TlsMode::Disable)),
    )
}

/// The next manager event, waiting up to [`WAIT`] of real time.
async fn next_event(ev: &mut tokio::sync::broadcast::Receiver<ConnectionEvent>) -> ConnectionEvent {
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        match ev.try_recv() {
            Ok(e) => return e,
            Err(TryRecvError::Empty) => {}
            Err(e) => panic!("the event channel failed: {e:?}"),
        }
        assert!(std::time::Instant::now() < deadline, "no event");
        tokio::time::sleep(ms(10)).await;
    }
}

fn executed_since(f: &FakeDriver, mark: usize) -> Vec<String> {
    f.executed()[mark..].to_vec()
}

// ---------------------------------------------------------------------------------------------
// The brief's tests

#[tokio::test(start_paused = true)]
async fn liveness_failure_reconnects_and_emits_session_lost() {
    let (m, f) = manager(ok_pg().pings(vec![Err(io_reset())]));
    let mut ev = m.events();
    connect_and_wait(&m).await;
    tokio::time::advance(secs(31)).await;
    settle().await;
    assert!(matches!(
        m.state(p().id),
        LiveState::Reconnecting { attempt: 1, .. } | LiveState::Connected(_)
    ));
    wait_connected(&m, p().id).await;
    assert!(matches!(
        ev.recv().await.unwrap(),
        ConnectionEvent::SessionLost { .. }
    ));
    assert_eq!(f.connect_calls(), 2);
}

#[tokio::test(start_paused = true)]
async fn auth_failure_during_reconnect_stops_with_failed() {
    let (m, f) = manager(
        FakeDriver::new(DbmsKind::Postgres)
            .script(vec![Ok(()), Err(db("28P01"))])
            .pings(vec![Err(io_reset())]),
    );
    connect_and_wait(&m).await;
    tokio::time::advance(secs(60)).await;
    settle().await;
    assert!(matches!(m.state(p().id), LiveState::Failed(_)));
    assert_eq!(f.connect_calls(), 2);
}

#[tokio::test(start_paused = true)]
async fn user_statement_is_never_replayed_after_connection_loss() {
    let (m, f) = manager(ok_pg().exec(vec![slow("INSERT INTO t VALUES (1)", ms(200))]));
    connect_and_wait(&m).await;
    let s = m.sessions(p().id).unwrap().default_session().unwrap();
    let t = s.submit(user_exec("INSERT INTO t VALUES (1)"));
    settle().await;
    f.kill_connections();
    assert_eq!(
        t.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::ConnectionLost {
            outcome_unknown: true
        }
    );
    wait_connected(&m, p().id).await;
    assert_eq!(
        f.executed()
            .iter()
            .filter(|s| s.starts_with("INSERT"))
            .count(),
        1
    );
}

#[tokio::test(start_paused = true)]
async fn internal_idempotent_read_is_retried_once() {
    // Not the schema probe's text (Q27).
    let (m, f) = manager(ok_pg().exec(vec![fails_once_with_reset("SELECT 42")]));
    connect_and_wait(&m).await;
    let s = default_session(&m);
    assert!(
        s.submit(internal_exec("SELECT 42"))
            .reply
            .await
            .unwrap()
            .is_ok()
    );
    assert_eq!(
        f.executed()
            .iter()
            .filter(|x| x.starts_with("SELECT 42"))
            .count(),
        2
    );
}

#[tokio::test(start_paused = true)]
async fn reconnect_after_pin_answer_uses_the_pin() {
    // Q17
    let (m, f) = manager_with_pin_answer();
    connect_and_wait(&m).await;
    lose_connection(&m, &f).await;
    wait_connected(&m, p().id).await;
    assert_eq!(
        f.last_request().unwrap().tls.unwrap().settings.pins.len(),
        1
    );
}

#[tokio::test(start_paused = true)]
async fn reconnect_with_never_retention_reuses_session_secret() {
    // Q17
    let (m, f) = manager_never_retention_prompted("pw");
    connect_and_wait(&m).await;
    lose_connection(&m, &f).await;
    wait_connected(&m, p().id).await;
    assert_eq!(f.connect_calls(), 2);
    assert!(nothing_stored(&m));
}

#[tokio::test]
async fn reconnect_needing_interaction_stops_with_lost() {
    // Q17: the one-time code was answered once; the tunnel dies, and the headless reconnect
    // would have to ask for a new one.
    let (m, f, ssh) = manager_kbd_otp().await;
    connect_and_wait(&m).await;
    ssh.kill_sessions();
    let state = wait_state(&m, |s| matches!(s, LiveState::Lost { .. })).await;
    assert_eq!(
        state,
        LiveState::Lost {
            class: ErrClass::SshAuth
        }
    );
    assert_eq!(f.connect_calls(), 1);
}

#[tokio::test(start_paused = true)]
async fn replay_restores_schema_and_read_only() {
    // The brief's `replay_restores_schema_tx_mode_and_read_only`, split: a read-only session is
    // always Auto (Q14), so the transaction mode is replayed in the next test.
    let (m, f) = manager_ro(ok_pg());
    connect_and_wait(&m).await;
    set_schema(&m, "app").await;
    let mark = f.executed().len();
    lose_connection(&m, &f).await;
    wait_connected(&m, p().id).await;
    let after = executed_since(&f, mark);
    assert!(
        after.contains(&"SET default_transaction_read_only = on".to_string())
            && after.contains(&"SET search_path TO \"app\"".to_string()),
        "{after:?}"
    );
    assert!(snap(&default_session(&m)).read_only);
}

#[tokio::test(start_paused = true)]
async fn replay_restores_tx_mode_and_isolation() {
    let (m, f) = manager(ok_pg());
    connect_and_wait(&m).await;
    set_mode(&m, TxMode::Manual).await;
    ok(
        &default_session(&m),
        Request::Tx(TxCommand::SetIsolation(IsolationLevel::Serializable)),
    )
    .await;
    lose_connection(&m, &f).await;
    wait_connected(&m, p().id).await;
    let tx = snap(&default_session(&m)).tx;
    assert_eq!(
        (tx.mode, tx.isolation, tx.status),
        (TxMode::Manual, IsolationLevel::Serializable, TxStatus::Idle)
    );
}

#[tokio::test]
async fn one_ssh_death_reconnects_each_dependent_once() {
    let (m, s, f) = manager_two_profiles_one_bastion().await;
    connect_both(&m).await;
    assert_eq!((f.connect_calls(), s.sessions_accepted()), (2, 1));
    s.kill_sessions();
    wait_connected_both(&m, &f).await;
    assert_eq!(f.connect_calls(), 4);
    assert_eq!(s.sessions_accepted(), 2);
}

#[tokio::test(start_paused = true)]
async fn idle_disconnect_emits_event_and_state_disconnected() {
    let (m, _) = manager_profile(ok_pg(), idle(300));
    let mut ev = m.events();
    connect_and_wait(&m).await;
    tokio::time::advance(secs(301)).await;
    settle().await;
    assert_eq!(m.state(p().id), LiveState::Disconnected);
    assert!(matches!(
        ev.recv().await.unwrap(),
        ConnectionEvent::IdleDisconnected { .. }
    ));
}

#[tokio::test(start_paused = true)]
async fn gives_up_after_five_minutes_with_lost() {
    let (m, f) = manager(
        FakeDriver::new(DbmsKind::Postgres)
            .script_then(vec![Ok(())], Err(refused()))
            .pings(vec![Err(io_reset())]),
    );
    connect_and_wait(&m).await;
    tokio::time::advance(secs(400)).await;
    // The backoff's later sleeps run on the paused clock: wait for the state, not 50 yields.
    wait_state(&m, |s| matches!(s, LiveState::Reconnecting { .. })).await;
    let lost_at = tokio::time::Instant::now();
    wait_state(&m, |s| matches!(s, LiveState::Lost { .. })).await;
    assert_eq!(
        m.state(p().id),
        LiveState::Lost {
            class: ErrClass::NetworkRefused
        }
    );
    // 0, 1, 2, 4, 8 s, then every 30 s (±20 %) until five minutes: a dozen tries, not one.
    let gave_up_after = lost_at.elapsed();
    assert!(
        gave_up_after >= secs(240) && gave_up_after <= secs(340),
        "{gave_up_after:?}"
    );
    assert!(f.connect_calls() >= 10, "{}", f.connect_calls());
    let calls = f.connect_calls();
    tokio::time::sleep(secs(600)).await;
    assert_eq!(f.connect_calls(), calls, "no retries once Lost");
}

#[tokio::test(start_paused = true)]
async fn profile_changes_are_published() {
    let (m, _) = manager_with_pin_answer();
    let mut ev = m.events();
    connect_and_wait(&m).await;
    assert!(matches!(
        ev.recv().await.unwrap(),
        ConnectionEvent::ProfileChanges { changes, .. } if matches!(changes[0], ProfileChange::AddPin(_))
    ));
}

// ---------------------------------------------------------------------------------------------
// Fix round 1: the review's hostile scenarios, as regressions.

#[tokio::test(start_paused = true)]
async fn a_loss_inside_a_transaction_is_lost_with_session_lost_and_never_replayed() {
    // Review H1 / C1, with the transaction open: nothing is replayed and nothing the user sends
    // afterwards runs anywhere; the profile is `Lost` and the toast says what was lost.
    let (m, f) = manager(ok_pg().exec(vec![
        tx("BEGIN", TxStatus::InTx),
        tx("INSERT", TxStatus::InTx),
        tx("CREATE TEMP TABLE", TxStatus::InTx),
    ]));
    let mut ev = m.events();
    connect_and_wait(&m).await;
    let s = default_session(&m);
    for sql in [
        "BEGIN",
        "INSERT INTO t VALUES (1)",
        "CREATE TEMP TABLE x (a int)",
        "SET work_mem = '1MB'",
    ] {
        ok(&s, user_exec(sql)).await;
    }
    let mark = f.executed().len();
    drop_connections(&m, &f);
    let state = wait_state(&m, |s| matches!(s, LiveState::Lost { .. })).await;
    assert_eq!(
        state,
        LiveState::Lost {
            class: ErrClass::NetworkReset
        }
    );
    let ConnectionEvent::SessionLost { profile, lost } = next_event(&mut ev).await else {
        panic!("a SessionLost event");
    };
    assert_eq!(profile, p().id);
    assert!(lost.open_tx && lost.temp_objects && lost.session_vars && !lost.locks);
    // The user goes on with the transaction they think is open: nothing runs.
    for request in [
        user_exec("UPDATE t SET a = 2"),
        Request::Tx(TxCommand::Commit),
    ] {
        assert_eq!(refusal(&s, request).await, ExecErrorKind::NotConnected);
    }
    tokio::time::sleep(secs(400)).await;
    assert_eq!(f.connect_calls(), 1, "no reconnect");
    assert!(
        executed_since(&f, mark).is_empty(),
        "{:?}",
        executed_since(&f, mark)
    );
}

#[tokio::test(start_paused = true)]
async fn requests_sent_while_reconnecting_never_run_on_the_new_connection() {
    // Review H1 / C1, with the session idle: it reconnects, but what the user sends during the
    // backoff never ran and never will; an internal idempotent read waits for the new one.
    let (m, f) = manager(ok_pg().script(vec![Ok(()), Err(refused())]));
    connect_and_wait(&m).await;
    let s = default_session(&m);
    let mark = f.executed().len();
    drop_connections(&m, &f);
    wait_state(&m, |s| {
        matches!(s, LiveState::Reconnecting { attempt: 2, .. })
    })
    .await;
    assert_eq!(
        refusal(&s, user_exec("UPDATE t SET a = 2")).await,
        NEVER_RAN
    );
    assert_eq!(refusal(&s, Request::Tx(TxCommand::Commit)).await, NEVER_RAN);
    let read = s.submit(internal_exec("SELECT 7"));
    wait_connected(&m, p().id).await;
    assert!(read.reply.await.unwrap().is_ok());
    let after = executed_since(&f, mark);
    assert!(
        !after
            .iter()
            .any(|x| x.starts_with("UPDATE") || x.starts_with("COMMIT")),
        "{after:?}"
    );
    assert!(after.iter().any(|x| x.starts_with("SELECT 7")));
}

/// A statement whose run finds the connection lost, after which the driver reports the
/// transaction `Failed` (the PostgreSQL driver's status for a connection it found closed).
fn lost_and_failed(prefix: &'static str) -> FakeExec {
    FakeExec {
        tx_after: Some(TxStatus::Failed),
        ..fails_once_with_reset(prefix)
    }
}

/// A MySQL server error with `code`.
fn mysql_error(code: i64) -> hacivat_driver::RawError {
    hacivat_driver::RawError {
        kind: hacivat_driver::RawErrorKind::Db,
        server: Some(hacivat_driver::ServerMessage {
            sqlstate: Some("HY000".into()),
            vendor_code: Some(code),
            severity: None,
            text: format!("server error {code}"),
            detail: None,
            hint: None,
            lang_hint: None,
        }),
    }
}

#[tokio::test(start_paused = true)]
async fn an_autocommit_statement_that_finds_the_loss_reconnects() {
    // Review H12 / C2: the driver says `Failed` after the loss; the session was idle before the
    // statement, so it reconnects, and the toast claims no rollback.
    let (m, f) = manager(ok_pg().exec(vec![lost_and_failed("SELECT 1")]));
    let mut ev = m.events();
    connect_and_wait(&m).await;
    let s = default_session(&m);
    assert_eq!(
        refusal(&s, user_exec("SELECT 1")).await,
        ExecErrorKind::ConnectionLost {
            outcome_unknown: true
        }
    );
    let ConnectionEvent::SessionLost { lost, .. } = next_event(&mut ev).await else {
        panic!("a SessionLost event");
    };
    assert!(!lost.open_tx, "{lost:?}");
    assert!(matches!(m.state(p().id), LiveState::Connected(_)));
    assert_eq!(f.connect_calls(), 2);
    let tx = snap(&default_session(&m)).tx;
    assert_eq!(tx.status, TxStatus::Idle);
}

#[tokio::test(start_paused = true)]
async fn a_read_only_session_whose_schema_switch_finds_the_loss_reconnects() {
    // C2 on a control statement: a read-only session is always Auto.
    let (m, f) = manager_profile(
        ok_pg().exec(vec![lost_and_failed("SET search_path")]),
        ro(pg_profile(TlsMode::Disable)),
    );
    connect_and_wait(&m).await;
    let s = default_session(&m);
    assert_eq!(
        refusal(&s, Request::SwitchSchema("app".into())).await,
        ExecErrorKind::ConnectionLost {
            outcome_unknown: true
        }
    );
    until("the reconnect", || f.connect_calls() == 2).await;
    wait_connected(&m, p().id).await;
    assert!(snap(&default_session(&m)).read_only);
}

#[tokio::test(start_paused = true)]
async fn an_internal_read_that_finds_the_loss_reconnects() {
    // C2 on the internal path (schema tree, completion).
    let (m, f) = manager(ok_pg().exec(vec![lost_and_failed("SELECT 7")]));
    connect_and_wait(&m).await;
    let s = default_session(&m);
    let reply = s
        .submit(Request::Exec {
            sql: "SELECT 7".into(),
            opts: ExecOpts::default(),
            origin: ExecOrigin::Internal { idempotent: false },
        })
        .reply
        .await
        .unwrap();
    assert!(matches!(
        reply.unwrap_err().kind,
        ExecErrorKind::ConnectionLost { .. }
    ));
    until("the reconnect", || f.connect_calls() == 2).await;
    wait_connected(&m, p().id).await;
}

#[tokio::test(start_paused = true)]
async fn a_statement_inside_a_transaction_that_finds_the_loss_is_lost() {
    // C1 still holds when the statement (not a ping) finds the loss.
    let (m, f) =
        manager(ok_pg().exec(vec![tx("BEGIN", TxStatus::InTx), lost_and_failed("UPDATE")]));
    let mut ev = m.events();
    connect_and_wait(&m).await;
    let s = default_session(&m);
    ok(&s, user_exec("BEGIN")).await;
    assert_eq!(
        refusal(&s, user_exec("UPDATE t SET a = 2")).await,
        ExecErrorKind::ConnectionLost {
            outcome_unknown: true
        }
    );
    wait_state(&m, |s| matches!(s, LiveState::Lost { .. })).await;
    let ConnectionEvent::SessionLost { lost, .. } = next_event(&mut ev).await else {
        panic!("a SessionLost event");
    };
    assert!(lost.open_tx);
    tokio::time::sleep(secs(60)).await;
    assert_eq!(f.connect_calls(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_shutdown_fatal_answering_a_statement_starts_the_recovery() {
    // Review H11 / I6: PostgreSQL answers with `FATAL 57P01` (`server.shutdown`). The
    // recovery starts on this request, which keeps the server's message.
    let (m, f) = manager(ok_pg().exec(vec![FakeExec {
        result: Err(db("57P01")),
        ..fails_once_with_reset("SELECT 1")
    }]));
    let mut ev = m.events();
    connect_and_wait(&m).await;
    let s = default_session(&m);
    let e = s
        .submit(user_exec("SELECT 1"))
        .reply
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(
        e.kind,
        ExecErrorKind::ConnectionLost {
            outcome_unknown: true
        }
    );
    assert_eq!(e.server.and_then(|m| m.sqlstate).as_deref(), Some("57P01"));
    let ConnectionEvent::SessionLost { lost, .. } = next_event(&mut ev).await else {
        panic!("a SessionLost event");
    };
    assert!(!lost.open_tx);
    assert_eq!(f.connect_calls(), 2);
    ok(&default_session(&m), user_exec("SELECT 1")).await;
}

#[tokio::test(start_paused = true)]
async fn a_starting_server_answering_a_statement_starts_the_recovery() {
    // I6: `57P03` (`server.starting`) after connect ends the connection too.
    let (m, f) = manager(ok_pg().exec(vec![FakeExec {
        result: Err(db("57P03")),
        ..fails_once_with_reset("SELECT 1")
    }]));
    connect_and_wait(&m).await;
    let s = default_session(&m);
    assert!(matches!(
        refusal(&s, user_exec("SELECT 1")).await,
        ExecErrorKind::ConnectionLost { .. }
    ));
    until("the reconnect", || f.connect_calls() == 2).await;
    wait_connected(&m, p().id).await;
}

#[tokio::test(start_paused = true)]
async fn a_mysql_connection_ending_error_starts_the_recovery() {
    // I6: MySQL's ER_CLIENT_INTERACTION_TIMEOUT (4031) comes as a server error before the
    // server closes the connection.
    let (m, f) = manager_profile(
        FakeDriver::new(DbmsKind::MySql).exec(vec![FakeExec {
            result: Err(mysql_error(4031)),
            ..fails_once_with_reset("SELECT 1")
        }]),
        mysql_profile(TlsMode::Disable),
    );
    connect_and_wait(&m).await;
    let s = default_session(&m);
    assert!(matches!(
        refusal(&s, user_exec("SELECT 1")).await,
        ExecErrorKind::ConnectionLost { .. }
    ));
    until("the reconnect", || f.connect_calls() == 2).await;
    wait_connected(&m, p().id).await;
    // An ordinary server error is no loss.
    let (m, f) = manager_profile(
        FakeDriver::new(DbmsKind::MySql).exec(vec![FakeExec {
            result: Err(mysql_error(1062)),
            ..fails_once_with_reset("INSERT")
        }]),
        mysql_profile(TlsMode::Disable),
    );
    connect_and_wait(&m).await;
    let s = default_session(&m);
    assert_eq!(
        refusal(&s, user_exec("INSERT INTO t VALUES (1)")).await,
        ExecErrorKind::Db
    );
    tokio::time::sleep(secs(40)).await;
    assert_eq!(f.connect_calls(), 1);
    assert!(matches!(m.state(p().id), LiveState::Connected(_)));
}

#[tokio::test(start_paused = true)]
async fn a_batch_that_begins_a_transaction_leaves_it_unknown_and_is_lost() {
    // Review R1: `BEGIN` after the first statement of a batch, or MySQL's `SET autocommit`.
    for sql in [
        "INSERT INTO t VALUES (1); BEGIN; UPDATE t SET a = 2",
        "SET autocommit = 0",
        // Fix round 3, R3.
        "XA START 'x'",
    ] {
        let first = sql.split(' ').next().unwrap();
        let prefix: &'static str = match first {
            "SET" => "SET",
            "XA" => "XA",
            _ => "INSERT",
        };
        let (m, f) = manager(ok_pg().exec(vec![lost_and_failed(prefix)]));
        connect_and_wait(&m).await;
        let s = default_session(&m);
        let _ = s.submit(user_exec(sql)).reply.await;
        wait_state(&m, |s| matches!(s, LiveState::Lost { .. })).await;
        assert_eq!(f.connect_calls(), 1, "{sql}");
    }
}

/// A PostgreSQL error with `sqlstate` at `severity`.
fn pg_error(sqlstate: &str, severity: &str) -> hacivat_driver::RawError {
    let mut e = db(sqlstate);
    if let Some(server) = e.server.as_mut() {
        server.severity = Some(severity.to_string());
    }
    e
}

#[tokio::test(start_paused = true)]
async fn a_relayed_remote_error_keeps_the_session_and_its_transaction() {
    // Review H14 / I7: `postgres_fdw` and `dblink` relay a remote server's failure as a local
    // ERROR with its SQLSTATE; the local connection and the user's transaction are fine.
    for sqlstate in ["08006", "08001", "57P01"] {
        let (m, f) = manager(ok_pg().exec(vec![
            tx("BEGIN", TxStatus::InTx),
            FakeExec {
                result: Err(pg_error(sqlstate, "ERROR")),
                tx_after: Some(TxStatus::Failed),
                ..fails_once_with_reset("SELECT * FROM remote_t")
            },
            tx("ROLLBACK", TxStatus::Idle),
        ]));
        connect_and_wait(&m).await;
        let s = default_session(&m);
        ok(&s, user_exec("BEGIN")).await;
        assert_eq!(
            refusal(&s, user_exec("SELECT * FROM remote_t")).await,
            ExecErrorKind::Db,
            "{sqlstate}"
        );
        let now = snap(&s);
        assert!(
            matches!(now.state, SessionState::Idle) && now.tx.status == TxStatus::Failed,
            "{sqlstate}: {now:?}"
        );
        // The transaction is still the user's to end.
        ok(&s, user_exec("ROLLBACK")).await;
        tokio::time::sleep(secs(40)).await;
        assert_eq!(f.connect_calls(), 1, "{sqlstate}");
        assert!(matches!(m.state(p().id), LiveState::Connected(_)));
    }
}

#[tokio::test(start_paused = true)]
async fn a_users_mysql_autocommit_off_ends_a_loss_lost_and_is_never_undone() {
    // Review H13 / I8: after the user's own `SET autocommit = 0` the server keeps an implicit
    // transaction; a lost `UPDATE` must not reconnect (a later COMMIT would commit nothing,
    // and the reconnect would turn autocommit back on).
    let (m, f) = manager_profile(
        FakeDriver::new(DbmsKind::MySql).exec(vec![fails_once_with_reset("UPDATE")]),
        mysql_profile(TlsMode::Disable),
    );
    let mut ev = m.events();
    connect_and_wait(&m).await;
    let s = default_session(&m);
    ok(&s, user_exec("SET autocommit = 0")).await;
    assert_eq!(
        refusal(&s, user_exec("UPDATE t SET a = 1")).await,
        ExecErrorKind::ConnectionLost {
            outcome_unknown: true
        }
    );
    wait_state(&m, |s| matches!(s, LiveState::Lost { .. })).await;
    let ConnectionEvent::SessionLost { lost, .. } = next_event(&mut ev).await else {
        panic!("a SessionLost event");
    };
    assert!(lost.open_tx);
    assert_eq!(
        refusal(&s, Request::Tx(TxCommand::Commit)).await,
        ExecErrorKind::NotConnected
    );
    tokio::time::sleep(secs(60)).await;
    assert_eq!(f.connect_calls(), 1);
}

#[tokio::test(start_paused = true)]
async fn an_idle_loss_under_the_users_autocommit_off_is_lost_and_turning_it_on_reconnects() {
    // I8: even an idle loss (a ping) is `Lost` while the user's `autocommit = 0` holds; once
    // they turn it back on, an idle loss reconnects again.
    let mysql = || {
        manager_profile(
            FakeDriver::new(DbmsKind::MySql),
            mysql_profile(TlsMode::Disable),
        )
    };
    let (m, f) = mysql();
    connect_and_wait(&m).await;
    ok(
        &default_session(&m),
        user_exec("SET @@session.autocommit = OFF"),
    )
    .await;
    drop_connections(&m, &f);
    wait_state(&m, |s| matches!(s, LiveState::Lost { .. })).await;
    assert_eq!(f.connect_calls(), 1);

    let (m, f) = mysql();
    connect_and_wait(&m).await;
    let s = default_session(&m);
    ok(&s, user_exec("SET autocommit = 0")).await;
    ok(&s, user_exec("SET autocommit = 1")).await;
    lose_connection(&m, &f).await;
    wait_connected(&m, p().id).await;
    // A global setting is not this session's.
    let (m, f) = mysql();
    connect_and_wait(&m).await;
    ok(&default_session(&m), user_exec("SET GLOBAL autocommit = 0")).await;
    lose_connection(&m, &f).await;
    wait_connected(&m, p().id).await;
}

#[tokio::test(start_paused = true)]
async fn a_commit_whose_follow_up_probe_finds_the_loss_reports_no_rollback() {
    // Review H15 / R4: the COMMIT succeeded (Idle observed); the schema re-probe after it
    // finds the loss. The session was idle by then, so it reconnects and claims no rollback.
    let (m, f) = manager(ok_pg().exec(vec![
        tx("BEGIN", TxStatus::InTx),
        tx("COMMIT", TxStatus::Idle),
        fails_once_with_reset("SELECT current_database()"),
    ]));
    let mut ev = m.events();
    connect_and_wait(&m).await;
    let s = default_session(&m);
    ok(&s, Request::Tx(TxCommand::Begin)).await;
    ok(&s, Request::Tx(TxCommand::Commit)).await;
    let ConnectionEvent::SessionLost { lost, .. } = next_event(&mut ev).await else {
        panic!("a SessionLost event");
    };
    assert!(!lost.open_tx, "{lost:?}");
    assert!(matches!(m.state(p().id), LiveState::Connected(_)));
    assert_eq!(f.connect_calls(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_manual_session_sends_nothing_after_a_loss() {
    // C1, Manual mode: no statement sent during the reconnect opens a transaction on the new
    // connection (`manual_begin` never runs for it).
    let (m, f) = manager(ok_pg().script(vec![Ok(()), Err(refused())]));
    connect_and_wait(&m).await;
    set_mode(&m, TxMode::Manual).await;
    let s = default_session(&m);
    let mark = f.executed().len();
    drop_connections(&m, &f);
    wait_state(&m, |s| {
        matches!(s, LiveState::Reconnecting { attempt: 2, .. })
    })
    .await;
    assert_eq!(
        refusal(&s, user_exec("INSERT INTO t VALUES (2)")).await,
        NEVER_RAN
    );
    wait_connected(&m, p().id).await;
    settle().await;
    let after = executed_since(&f, mark);
    assert!(
        !after
            .iter()
            .any(|x| x.starts_with("INSERT") || x.starts_with("BEGIN")),
        "{after:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_begin_lost_in_flight_leaves_the_transaction_unknown_and_is_lost() {
    // C1: a BEGIN whose outcome is unknown may have opened a transaction; no reconnect.
    let (m, f) = manager(ok_pg().exec(vec![slow("BEGIN", ms(200))]));
    let mut ev = m.events();
    connect_and_wait(&m).await;
    let s = default_session(&m);
    let begin = s.submit(user_exec("BEGIN"));
    settle().await;
    f.kill_connections();
    assert_eq!(
        begin.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::ConnectionLost {
            outcome_unknown: true
        }
    );
    wait_state(&m, |s| matches!(s, LiveState::Lost { .. })).await;
    let ConnectionEvent::SessionLost { lost, .. } = next_event(&mut ev).await else {
        panic!("a SessionLost event");
    };
    assert!(lost.open_tx);
    tokio::time::sleep(secs(60)).await;
    assert_eq!(f.connect_calls(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_replay_that_keeps_failing_follows_the_backoff_and_gives_up() {
    // Review H2 / I1: every reconnect lands, then its replay's first statement fails with a
    // reset. The failures count against one schedule and one five-minute clock.
    let fails: Vec<FakeExec> = (0..40)
        .map(|_| fails_once_with_reset("SET application_name"))
        .collect();
    let (m, f) = manager(ok_pg().exec(fails));
    connect_and_wait(&m).await;
    let lost_at = tokio::time::Instant::now();
    drop_connections(&m, &f);
    let state = wait_state(&m, |s| matches!(s, LiveState::Lost { .. })).await;
    assert_eq!(
        state,
        LiveState::Lost {
            class: ErrClass::NetworkReset
        }
    );
    let gave_up_after = lost_at.elapsed();
    assert!(
        gave_up_after >= secs(240) && gave_up_after <= secs(340),
        "{gave_up_after:?}"
    );
    // 0, 1, 2, 4, 8 s, then every 30 s: about a dozen connects, never one per failure.
    let connects = f.connect_calls() - 1;
    assert!((8..=16).contains(&connects), "{connects}");
}

#[tokio::test(start_paused = true)]
async fn a_close_during_the_replay_ends_it_at_once() {
    // Review H8 / I1: the replay's statement hangs; the user disconnects a second in.
    let (m, f) = manager(ok_pg().exec(vec![slow("SET application_name", secs(600))]));
    connect_and_wait(&m).await;
    drop_connections(&m, &f);
    until("the replay to start", || {
        f.executed()
            .iter()
            .any(|x| x.starts_with("SET application_name"))
    })
    .await;
    tokio::time::advance(secs(1)).await;
    let t = tokio::time::Instant::now();
    m.disconnect(p().id);
    assert_eq!(m.state(p().id), LiveState::Disconnected);
    until("every connection closed", || f.open_connections() == 0).await;
    assert!(t.elapsed() < secs(2), "{:?}", t.elapsed());
    let calls = f.connect_calls();
    tokio::time::sleep(secs(40)).await;
    assert_eq!(f.connect_calls(), calls);
    assert_eq!(m.state(p().id), LiveState::Disconnected);
}

#[tokio::test(start_paused = true)]
async fn a_replay_that_hangs_times_out_and_the_next_attempt_follows() {
    // I1: the replay is bounded by the connect timeout (15 s by default), then counts as a
    // failed attempt.
    let (m, f) = manager(ok_pg().exec(vec![slow("SET application_name", secs(600))]));
    connect_and_wait(&m).await;
    let lost_at = tokio::time::Instant::now();
    lose_connection(&m, &f).await;
    wait_connected(&m, p().id).await;
    let took = lost_at.elapsed();
    assert!(took >= secs(15) && took < secs(30), "{took:?}");
    assert_eq!(f.connect_calls(), 3, "connect, timed-out replay, reconnect");
}

#[tokio::test(start_paused = true)]
async fn a_direct_close_of_the_default_session_is_not_an_idle_disconnect() {
    // Review H4 / M2.
    let (m, _) = manager_profile(ok_pg(), idle(300));
    let mut ev = m.events();
    connect_and_wait(&m).await;
    let closed = default_session(&m).close(ClosePolicy::RollBack).await;
    assert!(closed.is_ok());
    wait_state(&m, |s| *s == LiveState::Disconnected).await;
    settle().await;
    assert!(
        matches!(ev.try_recv(), Err(TryRecvError::Empty)),
        "no IdleDisconnected"
    );
}

#[tokio::test(start_paused = true)]
async fn an_edit_saved_during_the_first_connect_reaches_the_reconnect() {
    // Review H10 / I2.
    let (m, f) = manager(ok_pg().gated());
    m.connect(p(), None, Interaction::Headless);
    until("the connect to reach the driver", || f.connect_calls() == 1).await;
    m.update_profile(
        p().id,
        Arc::new(Profile {
            database: Some("orders_v2".into()),
            ..pg_profile(TlsMode::Disable)
        }),
    );
    f.release();
    wait_connected(&m, p().id).await;
    lose_connection(&m, &f).await;
    wait_connected(&m, p().id).await;
    assert_eq!(
        f.last_request().unwrap().database.as_deref(),
        Some("orders_v2")
    );
}

#[tokio::test(start_paused = true)]
async fn a_destination_moved_during_the_first_connect_is_connected_anew_without_the_draft() {
    // I2 with T15's destination rule: the landed connection goes to the old user, so it is
    // closed and the saved one is connected instead; the password typed for the old one is
    // never sent to the new one (here it would have to be asked: `Failed`, never prompted).
    let pw = Profile {
        auth: AuthProviderId::Password,
        retention: Retention::Never,
        ..pg_profile(TlsMode::Disable)
    };
    let (m, f) = manager(ok_pg().gated());
    m.connect(
        Arc::new(pw.clone()),
        Some(SecretString::from("pw")),
        Interaction::Headless,
    );
    until("the connect to reach the driver", || f.connect_calls() == 1).await;
    m.update_profile(
        p().id,
        Arc::new(Profile {
            user: Some("admin".into()),
            ..pw
        }),
    );
    f.release();
    let LiveState::Failed(e) = wait_state(&m, |s| matches!(s, LiveState::Failed(_))).await else {
        unreachable!()
    };
    assert!(e.class == ErrClass::AuthInvalid && e.needs_interaction);
    assert_eq!(f.connect_calls(), 1, "the new user's connect asked first");
    until("the old connection closed", || f.open_connections() == 0).await;
    assert!(m.sessions(p().id).is_none());
}

#[tokio::test(start_paused = true)]
async fn a_destination_moved_during_the_first_connect_lands_on_the_new_one() {
    // I2: without a password, the saved destination connects.
    let (m, f) = manager(ok_pg().gated());
    m.connect(p(), None, Interaction::Headless);
    until("the connect to reach the driver", || f.connect_calls() == 1).await;
    m.update_profile(
        p().id,
        Arc::new(Profile {
            user: Some("admin".into()),
            ..pg_profile(TlsMode::Disable)
        }),
    );
    f.release();
    wait_connected(&m, p().id).await;
    assert_eq!(f.connect_calls(), 2);
    assert_eq!(f.last_request().unwrap().user.as_deref(), Some("admin"));
    until("the old connection closed", || f.open_connections() == 1).await;
}

// ---------------------------------------------------------------------------------------------
// Beyond the brief: the review probes and the controller's carry-ins.

#[tokio::test(start_paused = true)]
async fn a_profile_that_never_connected_never_shows_reconnecting() {
    let (m, f) = manager(ok_pg().connect_result(Err(refused())));
    let (profile, ix) = current();
    m.connect(profile, None, ix);
    wait_state(&m, |s| matches!(s, LiveState::Failed(_))).await;
    // Time passes: no ping, no recovery, no retry of a connect that never landed.
    tokio::time::sleep(secs(400)).await;
    assert!(matches!(m.state(p().id), LiveState::Failed(_)));
    assert_eq!(f.connect_calls(), 1);
    assert!(m.sessions(p().id).is_none());
}

#[tokio::test(start_paused = true)]
async fn the_permit_is_held_through_an_in_place_reconnect() {
    // T15 carry-in: a Broken session frees its permit, so an in-place reconnect must never
    // run without one. Budget 1; the first reconnect is refused, so the session waits ~1 s
    // in its backoff, while a borrower waits for the only permit.
    let one = Profile {
        max_connections: 1,
        ..pg_profile(TlsMode::Disable)
    };
    let (m, f) = manager_profile(ok_pg().script(vec![Ok(()), Err(refused())]), one);
    connect_and_wait(&m).await;
    let sessions = m.sessions(p().id).unwrap();
    drop_connections(&m, &f);
    wait_state(&m, |s| {
        matches!(s, LiveState::Reconnecting { attempt: 2, .. })
    })
    .await;
    let mut borrow = sessions.pooled(SessionKind::Service);
    assert!((&mut borrow).now_or_never().is_none());
    wait_connected(&m, p().id).await;
    settle().await;
    // The reconnected default holds the only permit: the borrower still waits, and no
    // connection beyond the budget was ever opened.
    assert!((&mut borrow).now_or_never().is_none());
    assert_eq!(
        f.connect_calls(),
        3,
        "connect, refused reconnect, reconnect"
    );
    assert_eq!(f.open_connections(), 1);
    // Disconnect: the waiting borrower ends instead of connecting afterwards (T15 R2).
    m.disconnect(p().id);
    let ended = tokio::time::timeout(secs(5), borrow).await;
    assert!(matches!(ended, Ok(Err(_))), "the waiter ends with an error");
    tokio::time::sleep(secs(40)).await;
    assert_eq!(f.connect_calls(), 3);
}

#[tokio::test(start_paused = true)]
async fn many_sessions_on_a_lost_server_reconnect_the_default_once() {
    // Review probe: no reconnect storm. The profile's other sessions break (M3 reconnects
    // consoles lazily, HV-CONN-05; pools open new ones); only the default recovers, once.
    let wide = Profile {
        max_connections: 12,
        ..pg_profile(TlsMode::Disable)
    };
    let (m, f) = manager_profile(ok_pg(), wide);
    connect_and_wait(&m).await;
    let sessions = m.sessions(p().id).unwrap();
    let mut others = Vec::new();
    for i in 1..=10 {
        let spec = SessionSpec {
            kind: SessionKind::Console(ClientId(i)),
            read_only: false,
            department: hacivat_proto::Department::Console,
        };
        others.push(sessions.open(spec, Interaction::Headless).await.unwrap());
    }
    assert_eq!(f.connect_calls(), 11);
    f.kill_connections();
    for s in others.iter().chain([&default_session(&m)]) {
        let _ = s.submit(Request::Ping);
    }
    until("the other sessions to break", || {
        others
            .iter()
            .all(|s| matches!(snap(s).state, SessionState::Broken(_)))
    })
    .await;
    until("the default's reconnect", || f.connect_calls() == 12).await;
    wait_connected(&m, p().id).await;
    tokio::time::sleep(secs(40)).await;
    assert_eq!(f.connect_calls(), 12);
    ok(&default_session(&m), user_exec("SELECT 1")).await;
}

#[tokio::test(start_paused = true)]
async fn a_reconnect_needing_a_password_stops_at_once_without_asking() {
    // Q17: recovery never prompts. The saved profile now names another user, so the overlay
    // keeps no password for it (T15 I1), and the headless reconnect would have to ask. It is
    // an `auth.*` failure, so `Failed` (B19), whose error dialog offers the re-entry.
    let pw = Profile {
        auth: AuthProviderId::Password,
        retention: Retention::Never,
        ..pg_profile(TlsMode::Disable)
    };
    let (m, f) = manager_profile(ok_pg().pings(vec![Err(io_reset())]), pw.clone());
    m.connect(
        Arc::new(pw.clone()),
        Some(SecretString::from("pw")),
        Interaction::Headless,
    );
    wait_connected(&m, p().id).await;
    m.update_profile(
        p().id,
        Arc::new(Profile {
            user: Some("admin".into()),
            ..pw
        }),
    );
    tokio::time::advance(secs(31)).await;
    let LiveState::Failed(e) = wait_state(&m, |s| matches!(s, LiveState::Failed(_))).await else {
        unreachable!()
    };
    assert!(e.class == ErrClass::AuthInvalid && e.needs_interaction);
    // No backoff loop and no prompt: nothing more is tried, however long it waits.
    tokio::time::sleep(secs(400)).await;
    assert_eq!(f.connect_calls(), 1);
    assert!(matches!(m.state(p().id), LiveState::Failed(_)));
    assert!(m.sessions(p().id).is_none());
}

#[tokio::test(start_paused = true)]
async fn reconnect_uses_the_saved_profile_edits() {
    // Q17: `update_profile` reaches the overlay the reconnect connects with.
    let (m, f) = manager(ok_pg());
    connect_and_wait(&m).await;
    m.update_profile(
        p().id,
        Arc::new(Profile {
            database: Some("orders_v2".into()),
            ..pg_profile(TlsMode::Disable)
        }),
    );
    lose_connection(&m, &f).await;
    wait_connected(&m, p().id).await;
    assert_eq!(
        f.last_request().unwrap().database.as_deref(),
        Some("orders_v2")
    );
}

#[tokio::test(start_paused = true)]
async fn statements_queued_behind_a_lost_one_never_run() {
    let (m, f) = manager(ok_pg().exec(vec![slow("INSERT INTO t VALUES (1)", ms(200))]));
    connect_and_wait(&m).await;
    let s = default_session(&m);
    let insert = s.submit(user_exec("INSERT INTO t VALUES (1)"));
    let update = s.submit(user_exec("UPDATE t SET a = 2"));
    settle().await;
    f.kill_connections();
    assert_eq!(
        insert.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::ConnectionLost {
            outcome_unknown: true
        }
    );
    assert_eq!(update.reply.await.unwrap().unwrap_err().kind, NEVER_RAN);
    wait_connected(&m, p().id).await;
    assert!(!f.executed().iter().any(|s| s.starts_with("UPDATE")));
    // The reconnected session runs new work.
    ok(&default_session(&m), user_exec("SELECT 1")).await;
}

#[tokio::test(start_paused = true)]
async fn an_internal_read_is_retried_only_once() {
    let (m, f) = manager(ok_pg().exec(vec![
        fails_once_with_reset("SELECT 42"),
        fails_once_with_reset("SELECT 42"),
    ]));
    connect_and_wait(&m).await;
    let reply = default_session(&m)
        .submit(internal_exec("SELECT 42"))
        .reply
        .await
        .unwrap();
    assert!(matches!(
        reply.unwrap_err().kind,
        ExecErrorKind::ConnectionLost { .. }
    ));
    wait_connected(&m, p().id).await;
    assert_eq!(
        f.executed()
            .iter()
            .filter(|x| x.starts_with("SELECT 42"))
            .count(),
        2
    );
}

#[tokio::test(start_paused = true)]
async fn an_idle_loss_reconnects_and_reports_what_was_lost() {
    let (m, f) = manager(ok_pg());
    let mut ev = m.events();
    connect_and_wait(&m).await;
    let s = default_session(&m);
    for sql in ["CREATE TEMP TABLE x (a int)", "SET work_mem = '1MB'"] {
        ok(&s, user_exec(sql)).await;
    }
    lose_connection(&m, &f).await;
    wait_connected(&m, p().id).await;
    let ConnectionEvent::SessionLost { profile, lost } = next_event(&mut ev).await else {
        panic!("a SessionLost event");
    };
    assert_eq!(profile, p().id);
    assert!(!lost.open_tx && lost.temp_objects && lost.session_vars && !lost.locks);
    // The next loss reports only what happened since.
    lose_connection(&m, &f).await;
    wait_connected(&m, p().id).await;
    let ConnectionEvent::SessionLost { lost, .. } = next_event(&mut ev).await else {
        panic!("a SessionLost event");
    };
    assert!(lost.is_empty(), "{lost:?}");
}

#[tokio::test(start_paused = true)]
async fn an_idle_transaction_is_reported_once_and_never_disconnected() {
    let (m, _) = manager_profile(ok_pg().exec(vec![tx("BEGIN", TxStatus::InTx)]), idle(300));
    let mut ev = m.events();
    connect_and_wait(&m).await;
    ok(&default_session(&m), user_exec("BEGIN")).await;
    tokio::time::sleep(secs(900)).await;
    assert!(matches!(m.state(p().id), LiveState::Connected(_)));
    assert_eq!(
        next_event(&mut ev).await,
        ConnectionEvent::IdleTransaction {
            profile: p().id,
            minutes: 5
        }
    );
    assert!(ev.try_recv().is_err(), "once per idle period");
}

#[tokio::test(start_paused = true)]
async fn the_m2b_config_keeps_a_direct_profile_quiet() {
    // B18 presets apply with `ManagerConfig::m2b()`: no keep-alive for a Direct profile.
    let (m, f) = manager_with(ok_pg().pings(vec![Err(io_reset())]), ManagerConfig::m2b());
    connect_and_wait(&m).await;
    tokio::time::sleep(secs(600)).await;
    assert!(matches!(m.state(p().id), LiveState::Connected(_)));
    assert_eq!(f.connect_calls(), 1);
    assert!(ManagerConfig::m2b().reconnect && ManagerConfig::m2b().presets);
    assert!(!ManagerConfig::default().reconnect && !ManagerConfig::default().presets);
}

#[tokio::test(start_paused = true)]
async fn the_default_config_has_no_idle_presets() {
    // Review M3: until the app switches to `m2b()` (Task 22), a production profile is never
    // idle-disconnected by its environment's preset (M2a had no idle disconnect).
    let production = Profile {
        environment: Some(hacivat_proto::EnvTag::Production),
        ..pg_profile(TlsMode::Disable)
    };
    let (m, _) = manager_with(ok_pg(), ManagerConfig::default());
    m.connect(Arc::new(production), None, Interaction::Headless);
    wait_connected(&m, p().id).await;
    tokio::time::sleep(secs(2 * 3600)).await;
    assert!(matches!(m.state(p().id), LiveState::Connected(_)));
}

#[tokio::test(start_paused = true)]
async fn shutdown_during_the_backoff_returns_within_the_budget() {
    let (m, f) = manager(ok_pg().script_then(vec![Ok(())], Err(refused())));
    connect_and_wait(&m).await;
    drop_connections(&m, &f);
    // The first reconnect is refused; the session now sleeps in its backoff.
    wait_state(&m, |s| {
        matches!(s, LiveState::Reconnecting { attempt: 2, .. })
    })
    .await;
    let t = tokio::time::Instant::now();
    m.shutdown().await;
    assert!(t.elapsed() < m.shutdown_budget(), "{:?}", t.elapsed());
    assert_eq!(m.state(p().id), LiveState::Disconnected);
    let calls = f.connect_calls();
    tokio::time::sleep(secs(40)).await;
    assert_eq!(f.connect_calls(), calls, "no reconnect after the shutdown");
    until("every connection closed", || f.open_connections() == 0).await;
}

#[tokio::test(start_paused = true)]
async fn disconnect_while_reconnecting_ends_the_recovery() {
    let (m, f) = manager(ok_pg().script_then(vec![Ok(())], Err(refused())));
    connect_and_wait(&m).await;
    drop_connections(&m, &f);
    wait_state(&m, |s| {
        matches!(s, LiveState::Reconnecting { attempt: 2, .. })
    })
    .await;
    m.disconnect(p().id);
    assert_eq!(m.state(p().id), LiveState::Disconnected);
    assert!(m.sessions(p().id).is_none());
    let calls = f.connect_calls();
    tokio::time::sleep(secs(40)).await;
    assert_eq!(f.connect_calls(), calls);
    assert_eq!(m.state(p().id), LiveState::Disconnected);
}

#[tokio::test(start_paused = true)]
async fn the_m2a_config_marks_a_lost_session_lost() {
    // `ManagerConfig::default()` keeps M2a's behaviour (R12): no reconnect.
    let (m, f) = manager_with(
        ok_pg().pings(vec![Err(io_reset())]),
        ManagerConfig::default(),
    );
    connect_and_wait(&m).await;
    tokio::time::advance(secs(31)).await;
    let state = wait_state(&m, |s| matches!(s, LiveState::Lost { .. })).await;
    assert_eq!(
        state,
        LiveState::Lost {
            class: ErrClass::NetworkReset
        }
    );
    assert_eq!(f.connect_calls(), 1);
    assert!(m.sessions(p().id).is_none());
}

#[tokio::test(start_paused = true)]
async fn a_deactivated_default_session_ends_disconnected_not_lost_cancelled() {
    // Review M7: a deactivate breaks the session with `cancelled`, which is no loss.
    let (m, _) = manager(ok_pg().exec(vec![slow("SELECT pg_sleep", secs(600))]));
    connect_and_wait(&m).await;
    let s = default_session(&m);
    let running = s.submit(user_exec("SELECT pg_sleep(600)"));
    until("the statement to run", || {
        matches!(snap(&s).state, SessionState::Busy { .. })
    })
    .await;
    let _ = s.escalate(running.seq, Escalation::Deactivate).await;
    assert_eq!(
        wait_state(&m, |s| !matches!(s, LiveState::Connected(_))).await,
        LiveState::Disconnected
    );
}

#[tokio::test(start_paused = true)]
async fn live_info_names_the_path() {
    let (m, _) = manager(ok_pg());
    connect_and_wait(&m).await;
    let LiveState::Connected(info) = m.state(p().id) else {
        panic!("connected");
    };
    assert_eq!((info.hops, info.via.as_deref()), (0, None));
    assert!(info.access_warnings.is_empty());
}

#[tokio::test(start_paused = true)]
async fn close_all_ends_requests_waiting_for_a_permit() {
    // T15 R2: a request queued for a permit when the sessions close never connects later.
    let (deps, f) = deps_with(ok_pg());
    let one = Arc::new(Profile {
        max_connections: 1,
        ..pg_profile(TlsMode::Disable)
    });
    let sm = SessionManager::new(
        tokio::runtime::Handle::current(),
        one.clone(),
        Arc::new(deps),
        Arc::new(budget_for(&one)),
    );
    let console = SessionSpec {
        kind: SessionKind::Console(ClientId(1)),
        read_only: false,
        department: hacivat_proto::Department::Console,
    };
    let _held = sm
        .open(console.clone(), Interaction::Headless)
        .await
        .unwrap();
    let waiting_open = sm.open(
        SessionSpec {
            kind: SessionKind::Console(ClientId(2)),
            ..console
        },
        Interaction::Headless,
    );
    let waiting_borrow = sm.pooled(SessionKind::Service);
    sm.close_all().await;
    let (open, borrow) = tokio::join!(
        tokio::time::timeout(WAIT, waiting_open),
        tokio::time::timeout(WAIT, waiting_borrow)
    );
    assert!(matches!(open, Ok(Err(_))), "the waiting open ends");
    assert!(matches!(borrow, Ok(Err(_))), "the waiting borrow ends");
    tokio::time::sleep(secs(40)).await;
    assert_eq!(f.connect_calls(), 1);
}

// ---------------------------------------------------------------------------------------------
// Task 14 fixtures: a certificate pinned at connect, a `Never` password typed once, a
// keyboard-interactive hop, two profiles behind one bastion.

fn manager_with_pin_answer() -> (Arc<ConnectionManager>, Arc<FakeDriver>) {
    set_up(
        pg_profile(TlsMode::VerifyFull),
        ScriptedUi::answers_m2b([Answer::Pin]).ix(),
    );
    // The pinned retry reports an encrypted, pinned connection (Task 14 refuses one the
    // request did not allow).
    manager(
        ok_pg()
            .presented(vec![include_bytes!("fixtures/test-ca.der").to_vec()])
            .tls(TlsSummary {
                encrypted: true,
                version: Some("TLSv1.3".into()),
                cipher: None,
                verified: TlsVerified::Pinned,
            })
            .script(vec![Err(tls_unknown()), Ok(())]),
    )
}

fn manager_never_retention_prompted(password: &str) -> (Arc<ConnectionManager>, Arc<FakeDriver>) {
    set_up(
        never_profile(),
        ScriptedUi::answers_m2b([Answer::Password(sec(password))]).ix(),
    );
    manager(ok_pg())
}

/// Whether nothing was stored for the test profile, in any retention.
fn nothing_stored(_m: &ConnectionManager) -> bool {
    let store = thread_store();
    [Retention::Forever, Retention::ForSession, Retention::Never]
        .into_iter()
        .all(|r| store.get(&db_key(pid()), r).unwrap().is_none())
}

/// A TCP server that accepts and holds every connection: the database behind the bastion.
async fn held_database() -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept().await {
            held.push(stream);
        }
    });
    addr
}

/// A testkit SSH server whose channels reach a held local socket.
async fn bastion(config: hacivat_testkit::ssh_server::TestSshConfig) -> TestSshServer {
    let config = config.forward_to(hacivat_testkit::ssh_server::ForwardTarget::Redirect(
        held_database().await,
    ));
    TestSshServer::start(config)
}

/// A profile behind `ssh` with one hop (`user`, `auth`) and host keys accepted on first use.
fn ssh_profile_via(
    ssh: &TestSshServer,
    user: &str,
    auth: hacivat_net::ssh::SshAuth,
    id: ProfileId,
) -> Profile {
    use hacivat_net::ssh::{HopSpec, HostKeyPolicy, SshSpec};
    Profile {
        id,
        transport: hacivat_session::profile::TransportSpec::SshTunnel(SshSpec {
            hops: vec![HopSpec {
                host: ssh.host(),
                port: ssh.addr.port(),
                user: user.into(),
                auth,
                identities_only: false,
                identity_files: Vec::new(),
            }],
            host_keys: HostKeyPolicy::AcceptNew,
            keepalive_s: None,
        }),
        ssh_retention: Retention::Forever,
        ..pg_profile(TlsMode::Disable)
    }
}

thread_local! {
    /// The test's temporary SSH store directories, removed when its thread ends.
    static DIRS: RefCell<Vec<tempfile::TempDir>> = const { RefCell::new(Vec::new()) };
}

/// Deps over `fake` whose SSH stores live in a temporary directory, with no transport stub:
/// the driver's stream is the SSH channel.
fn deps_with_ssh(fake: FakeDriver) -> (hacivat_session::pipeline::ConnectDeps, Arc<FakeDriver>) {
    let dir = tempfile::tempdir().unwrap();
    let known = hacivat_net::ssh::KnownHosts::load(Vec::new(), dir.path().join("known_hosts"));
    let tunnels = hacivat_net::ssh::TunnelManager::with_agent(
        known.clone(),
        hacivat_net::ssh::TunnelConfig::default(),
        None,
    );
    let (mut deps, fake) = deps_with(fake);
    deps.tunnels = tunnels;
    deps.known_hosts = known;
    deps.transport_wrap = None;
    DIRS.with(|d| d.borrow_mut().push(dir));
    (deps, fake)
}

async fn manager_kbd_otp() -> (Arc<ConnectionManager>, Arc<FakeDriver>, TestSshServer) {
    let ssh = bastion(
        hacivat_testkit::ssh_server::TestSshConfig::default()
            .user_kbd("ops", &[("Verification code: ", false, "424242")]),
    )
    .await;
    let profile = ssh_profile_via(
        &ssh,
        "ops",
        hacivat_net::ssh::SshAuth::KeyboardInteractive,
        pid(),
    );
    set_up(profile, Interaction::Interactive(Arc::new(OneTimeCode)));
    // The driver's connection rides the SSH channel: it dies with the tunnel (review I4).
    let (deps, fake) = deps_with_ssh(ok_pg().consumes_factory());
    let m = ConnectionManager::new(tokio::runtime::Handle::current(), Arc::new(deps), config());
    (m, fake, ssh)
}

/// A UI that answers every keyboard-interactive round with a one-time code.
struct OneTimeCode;

impl UiPort for OneTimeCode {
    fn ask(&self, q: Question) -> BoxFuture<'static, Answer> {
        let answer = match q {
            Question::Keyboard { request, .. } => Answer::Keyboard(
                request
                    .prompts
                    .iter()
                    .map(|_| SecretString::from("424242"))
                    .collect(),
            ),
            _ => Answer::Cancelled,
        };
        Box::pin(async move { answer })
    }
}

thread_local! {
    static BOTH: RefCell<Vec<Arc<Profile>>> = const { RefCell::new(Vec::new()) };
}

async fn manager_two_profiles_one_bastion()
-> (Arc<ConnectionManager>, TestSshServer, Arc<FakeDriver>) {
    let ssh =
        bastion(hacivat_testkit::ssh_server::TestSshConfig::default().user_pw("ops", "pw")).await;
    let profiles: Vec<Arc<Profile>> = [pid(), ProfileId::new()]
        .into_iter()
        .map(|id| {
            Arc::new(ssh_profile_via(
                &ssh,
                "ops",
                hacivat_net::ssh::SshAuth::Password,
                id,
            ))
        })
        .collect();
    let store = thread_store();
    for p in &profiles {
        store
            .put(
                &hacivat_secrets::SecretKey {
                    profile: p.id,
                    slot: SecretSlot::SshPassword { hop: 0 },
                },
                SecretString::from("pw"),
                Retention::Forever,
            )
            .unwrap();
    }
    BOTH.with(|b| *b.borrow_mut() = profiles);
    // The driver's connections ride the SSH channels: they die with the tunnel (review I4).
    let (deps, fake) = deps_with_ssh(ok_pg().consumes_factory());
    let m = ConnectionManager::new(tokio::runtime::Handle::current(), Arc::new(deps), config());
    (m, ssh, fake)
}

fn both() -> Vec<Arc<Profile>> {
    BOTH.with(|b| b.borrow().clone())
}

async fn connect_both(m: &ConnectionManager) {
    for p in both() {
        m.connect(p.clone(), None, Interaction::Headless);
        wait_connected(m, p.id).await;
    }
}

/// Waits on end states only (review I4): both reconnects made, both profiles connected.
async fn wait_connected_both(m: &ConnectionManager, f: &FakeDriver) {
    until("both reconnects", || f.connect_calls() >= 4).await;
    for p in both() {
        wait_connected(m, p.id).await;
    }
}

// ---------------------------------------------------------------------------------------------
// Docker: a real server ends the default session's backend; the manager reconnects it.

mod docker_support;

/// The statement that answers the connection's server-side ID, and the one that ends `id`.
fn kill_statements(engine: DbmsKind, id: &str) -> (&'static str, String) {
    match engine {
        DbmsKind::Postgres => (
            "SELECT pg_backend_pid()",
            format!("SELECT pg_terminate_backend({id})"),
        ),
        _ => ("SELECT CONNECTION_ID()", format!("KILL {id}")),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn a_real_server_ending_the_session_is_reconnected_with_session_lost() {
    if !docker_support::docker_enabled() {
        return;
    }
    for id in ["pg17", "mysql84"] {
        let Some(t) = docker_support::target_required(id) else {
            continue;
        };
        let mode = docker_support::auto_or_require(&t);
        let m = ConnectionManager::new(
            tokio::runtime::Handle::current(),
            Arc::new(docker_support::deps()),
            ManagerConfig::m2b(),
        );
        // The overlay keeps the draft password that worked (Q17), so the headless reconnect
        // signs in without the store.
        let profile = Arc::new(docker_support::profile_for(&t, mode));
        let mut ev = m.events();
        m.connect(
            profile.clone(),
            Some(docker_support::docker_password()),
            Interaction::Headless,
        );
        wait_state_of(&m, profile.id, |s| {
            assert!(!matches!(s, LiveState::Failed(_)), "{id}: {s:?}");
            matches!(s, LiveState::Connected(_))
        })
        .await;
        let session = default_session_of(&m, profile.id);
        let (ask, _) = kill_statements(t.engine, "");
        let answer = session
            .submit(Request::Exec {
                sql: ask.to_string(),
                opts: ExecOpts {
                    protocol: hacivat_driver::Protocol::Simple,
                    ..ExecOpts::default()
                },
                origin: ExecOrigin::Internal { idempotent: true },
            })
            .reply
            .await
            .unwrap()
            .unwrap_or_else(|e| panic!("{id}: {e:?}"));
        let backend = answer.rows[0][0].clone().expect("an ID");
        let (_, kill) = kill_statements(t.engine, &backend);
        let mut killer = docker_support::connect_target(&t, mode).await;
        killer
            .raw()
            .batch(&kill)
            .await
            .unwrap_or_else(|e| panic!("{id}: {e:?}"));
        killer.close().await;
        // An Auto-mode user statement finds the loss (fix round 2, C2 and I6): the server may
        // first answer with its FATAL, or the driver finds the connection closed. A bounded
        // number of round trips; the session is idle, so it reconnects.
        let mut found = false;
        for _ in 0..3 {
            let reply = session.submit(user_exec("SELECT 1")).reply.await.unwrap();
            if let Err(e) = reply
                && matches!(e.kind, ExecErrorKind::ConnectionLost { .. })
            {
                found = true;
                break;
            }
        }
        assert!(found, "{id}: the loss was never seen");
        // `SessionLost` comes right after the profile is `Connected` again; it claims no
        // rollback (the session was idle before the statement).
        let ConnectionEvent::SessionLost { lost, .. } = next_event(&mut ev).await else {
            panic!("{id}: a SessionLost event");
        };
        assert!(!lost.open_tx, "{id}: {lost:?}");
        assert!(
            matches!(m.state(profile.id), LiveState::Connected(_)),
            "{id}"
        );
        let again = default_session_of(&m, profile.id)
            .submit(user_exec("SELECT 1"))
            .reply
            .await
            .unwrap();
        assert!(again.is_ok(), "{id}: {again:?}");
        m.shutdown().await;
    }
}

fn default_session_of(m: &ConnectionManager, id: ProfileId) -> SessionHandle {
    m.sessions(id).unwrap().default_session().unwrap()
}
