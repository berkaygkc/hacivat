//! The `ConnectionManager` over a `SessionManager` (HV-CONN-04, -05; D-085; B4, B18, B19, B20;
//! Task 17): liveness, recovery with backoff and replay, session-lost events, idle disconnect,
//! the session overlay on reconnect (Q17) and SSH group invalidation.
//!
//! `start_paused` only with `FakeDriver` (Q25); the SSH tests run in real time.

mod common;

use std::cell::RefCell;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use futures::FutureExt as _;
use futures::future::BoxFuture;
use hacivat_driver::fake::{FakeDriver, FakeExec};
use hacivat_driver::{ExecOpts, TxStatus};
use hacivat_net::TlsMode;
use hacivat_proto::{DbmsKind, ErrClass, ProfileId};
use hacivat_secrets::{AuthProviderId, Retention, SecretSlot, SecretString};
use hacivat_session::pipeline::{Answer, Interaction, ProfileChange, Question, UiPort};
use hacivat_session::profile::Profile;
use hacivat_session::session::{
    ClientId, ExecErrorKind, ExecOrigin, IsolationLevel, Request, SessionHandle, SessionKind,
    SessionManager, SessionSpec, SessionState, TxCommand, TxMode,
};
use hacivat_session::{ConnectionEvent, ConnectionManager, LiveState, ManagerConfig, budget_for};

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

/// A statement whose first run fails as if the connection reset.
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
/// (a ping here), and the manager reconnects. Waits for the reconnect's connect.
async fn lose_connection(m: &ConnectionManager, f: &FakeDriver) {
    let before = f.connect_calls();
    f.kill_connections();
    let _ = default_session(m).submit(Request::Ping);
    eventually("the reconnect's connect", || f.connect_calls() > before).await;
}

async fn set_schema(m: &ConnectionManager, name: &str) {
    let reply = default_session(m)
        .submit(Request::SwitchSchema(name.to_string()))
        .reply
        .await
        .unwrap();
    assert!(reply.is_ok(), "{reply:?}");
}

async fn set_mode(m: &ConnectionManager, mode: TxMode) {
    let reply = default_session(m)
        .submit(Request::Tx(TxCommand::SetMode(mode)))
        .reply
        .await
        .unwrap();
    assert!(reply.is_ok(), "{reply:?}");
}

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

/// The next manager event, within [`WAIT`] of real time.
async fn next_event(ev: &mut tokio::sync::broadcast::Receiver<ConnectionEvent>) -> ConnectionEvent {
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        match ev.try_recv() {
            Ok(e) => return e,
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {}
            Err(e) => panic!("the event channel failed: {e:?}"),
        }
        assert!(std::time::Instant::now() < deadline, "no event");
        tokio::time::sleep(ms(10)).await;
    }
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

#[tokio::test]
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

#[tokio::test]
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

#[tokio::test]
#[ignore = "needs Task 14: Question::Certificate and LiveConnection.profile_changes"]
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

#[tokio::test]
#[ignore = "needs Task 14: LiveConnection.secrets_used"]
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
#[ignore = "needs Task 14: the SSH transport step (keyboard-interactive hop)"]
async fn reconnect_needing_interaction_stops_with_lost() {
    // Q17
    let (m, f, ssh) = manager_kbd_otp().await;
    connect_and_wait(&m).await;
    f.kill_connections();
    ssh.kill_sessions();
    let _ = default_session(&m).submit(Request::Ping);
    wait_state(&m, |s| matches!(s, LiveState::Lost { .. })).await;
    assert_eq!(f.connect_calls(), 1);
}

#[tokio::test]
async fn replay_restores_schema_and_read_only() {
    // The brief's `replay_restores_schema_tx_mode_and_read_only`, split: a read-only session is
    // always Auto (Q14), so the transaction mode is replayed in the next test.
    let (m, f) = manager_ro(ok_pg());
    connect_and_wait(&m).await;
    set_schema(&m, "app").await;
    let mark = f.executed().len();
    lose_connection(&m, &f).await;
    wait_connected(&m, p().id).await;
    let after = f.executed()[mark..].to_vec();
    assert!(
        after.contains(&"SET default_transaction_read_only = on".to_string())
            && after.contains(&"SET search_path TO \"app\"".to_string()),
        "{after:?}"
    );
    assert!(snap(&default_session(&m)).read_only);
}

#[tokio::test]
async fn replay_restores_tx_mode_and_isolation() {
    let (m, f) = manager(ok_pg());
    connect_and_wait(&m).await;
    set_mode(&m, TxMode::Manual).await;
    let isolation = default_session(&m)
        .submit(Request::Tx(TxCommand::SetIsolation(
            IsolationLevel::Serializable,
        )))
        .reply
        .await
        .unwrap();
    assert!(isolation.is_ok());
    lose_connection(&m, &f).await;
    wait_connected(&m, p().id).await;
    let tx = snap(&default_session(&m)).tx;
    assert_eq!(
        (tx.mode, tx.isolation, tx.status),
        (TxMode::Manual, IsolationLevel::Serializable, TxStatus::Idle)
    );
}

#[tokio::test]
#[ignore = "needs Task 14: the SSH transport step (LiveConnection.transport)"]
async fn one_ssh_death_reconnects_each_dependent_once() {
    let (m, s, f) = manager_two_profiles_one_bastion().await;
    connect_both(&m).await;
    s.kill_sessions();
    wait_connected_both(&m).await;
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
    tokio::time::sleep(secs(600)).await;
    let calls = f.connect_calls();
    tokio::time::sleep(secs(600)).await;
    assert_eq!(f.connect_calls(), calls, "no retries once Lost");
}

#[tokio::test]
#[ignore = "needs Task 14: Question::Certificate and LiveConnection.profile_changes"]
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
    f.kill_connections();
    let _ = default_session(&m).submit(Request::Ping);
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
    settle().await;
    assert_eq!(f.connect_calls(), 3);
}

#[tokio::test]
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
    eventually("the other sessions to break", || {
        others
            .iter()
            .all(|s| matches!(snap(s).state, SessionState::Broken(_)))
    })
    .await;
    eventually("the default's reconnect", || f.connect_calls() == 12).await;
    wait_connected(&m, p().id).await;
    settle().await;
    assert_eq!(f.connect_calls(), 12);
    assert!(
        default_session(&m)
            .submit(user_exec("SELECT 1"))
            .reply
            .await
            .unwrap()
            .is_ok()
    );
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

#[tokio::test]
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

#[tokio::test]
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
    assert_eq!(
        update.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::ConnectionLost {
            outcome_unknown: false
        }
    );
    wait_connected(&m, p().id).await;
    assert!(!f.executed().iter().any(|s| s.starts_with("UPDATE")));
    // The reconnected session runs new work.
    assert!(
        default_session(&m)
            .submit(user_exec("SELECT 1"))
            .reply
            .await
            .unwrap()
            .is_ok()
    );
}

#[tokio::test]
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
async fn session_lost_says_what_was_lost() {
    let (m, f) = manager(ok_pg().exec(vec![
        tx("BEGIN", TxStatus::InTx),
        tx("CREATE TEMP TABLE", TxStatus::InTx),
    ]));
    let mut ev = m.events();
    connect_and_wait(&m).await;
    let s = default_session(&m);
    for sql in [
        "BEGIN",
        "CREATE TEMP TABLE x (a int)",
        "SET work_mem = '1MB'",
    ] {
        assert!(s.submit(user_exec(sql)).reply.await.unwrap().is_ok());
    }
    lose_connection(&m, &f).await;
    wait_connected(&m, p().id).await;
    let ConnectionEvent::SessionLost { profile, lost } = next_event(&mut ev).await else {
        panic!("a SessionLost event");
    };
    assert_eq!(profile, p().id);
    assert!(lost.open_tx && lost.temp_objects && lost.session_vars && !lost.locks);
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
    let s = default_session(&m);
    assert!(s.submit(user_exec("BEGIN")).reply.await.unwrap().is_ok());
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
    assert!(ManagerConfig::m2b().reconnect && ManagerConfig::m2b().keepalive_presets);
    assert!(!ManagerConfig::default().reconnect && !ManagerConfig::default().keepalive_presets);
}

#[tokio::test]
async fn shutdown_during_the_backoff_returns_within_the_budget() {
    let (m, f) = manager(ok_pg().script_then(vec![Ok(())], Err(refused())));
    connect_and_wait(&m).await;
    f.kill_connections();
    let _ = default_session(&m).submit(Request::Ping);
    // The first reconnect is refused; the session now sleeps in its backoff.
    wait_state(&m, |s| {
        matches!(s, LiveState::Reconnecting { attempt: 2, .. })
    })
    .await;
    let t = std::time::Instant::now();
    m.shutdown().await;
    assert!(t.elapsed() < secs(1), "{:?}", t.elapsed());
    assert_eq!(m.state(p().id), LiveState::Disconnected);
    let calls = f.connect_calls();
    tokio::time::sleep(ms(1500)).await;
    assert_eq!(f.connect_calls(), calls, "no reconnect after the shutdown");
    eventually("every connection closed", || f.open_connections() == 0).await;
}

#[tokio::test]
async fn disconnect_while_reconnecting_ends_the_recovery() {
    let (m, f) = manager(ok_pg().script_then(vec![Ok(())], Err(refused())));
    connect_and_wait(&m).await;
    f.kill_connections();
    let _ = default_session(&m).submit(Request::Ping);
    wait_state(&m, |s| {
        matches!(s, LiveState::Reconnecting { attempt: 2, .. })
    })
    .await;
    m.disconnect(p().id);
    assert_eq!(m.state(p().id), LiveState::Disconnected);
    assert!(m.sessions(p().id).is_none());
    let calls = f.connect_calls();
    tokio::time::sleep(ms(1500)).await;
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

#[tokio::test]
async fn live_info_names_the_path() {
    let (m, _) = manager(ok_pg());
    connect_and_wait(&m).await;
    let LiveState::Connected(info) = m.state(p().id) else {
        panic!("connected");
    };
    assert_eq!((info.hops, info.via.as_deref()), (0, None));
    assert!(info.access_warnings.is_empty());
}

#[tokio::test]
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
    settle().await;
    assert_eq!(f.connect_calls(), 1);
}

// ---------------------------------------------------------------------------------------------
// Task 14 fixtures (ignored tests above): a certificate pinned at connect, a `Never` password
// typed once, a keyboard-interactive hop, two profiles behind one bastion.

/// A UI that pins the reviewed certificate and types `password` (and nothing else).
struct PinAndPassword(String);

impl UiPort for PinAndPassword {
    fn ask(&self, q: Question) -> BoxFuture<'static, Answer> {
        let answer = match q {
            Question::Certificate { .. } => Answer::Pin,
            Question::Password { .. } => Answer::Password(SecretString::from(self.0.clone())),
            _ => Answer::Cancelled,
        };
        Box::pin(async move { answer })
    }
}

fn manager_with_pin_answer() -> (Arc<ConnectionManager>, Arc<FakeDriver>) {
    set_up(
        pg_profile(TlsMode::VerifyFull),
        Interaction::Interactive(Arc::new(PinAndPassword(String::new()))),
    );
    manager(
        ok_pg()
            .presented(vec![b"fixture certificate".to_vec()])
            .script(vec![Err(tls_unknown()), Ok(())]),
    )
}

fn manager_never_retention_prompted(password: &str) -> (Arc<ConnectionManager>, Arc<FakeDriver>) {
    set_up(
        never_profile(),
        Interaction::Interactive(Arc::new(PinAndPassword(password.to_string()))),
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
            Question::HostKey { .. } => Answer::TrustHostKey,
            _ => Answer::Cancelled,
        };
        Box::pin(async move { answer })
    }
}

/// A profile behind `ssh` with one hop (`user`, `auth`) and host keys accepted on first use.
fn ssh_profile_via(
    ssh: &hacivat_testkit::ssh_server::TestSshServer,
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
            keepalive_s: Some(1),
        }),
        ssh_retention: Retention::Forever,
        ..pg_profile(TlsMode::Disable)
    }
}

thread_local! {
    /// The test's temporary SSH store directories, removed when its thread ends.
    static DIRS: RefCell<Vec<tempfile::TempDir>> = const { RefCell::new(Vec::new()) };
}

/// Deps over `fake` whose SSH stores live in a temporary directory.
fn deps_with_ssh(fake: FakeDriver) -> (hacivat_session::pipeline::ConnectDeps, Arc<FakeDriver>) {
    let dir = tempfile::tempdir().unwrap();
    let known = hacivat_net::ssh::KnownHosts::load(Vec::new(), dir.path().join("known_hosts"));
    let tunnels = hacivat_net::ssh::TunnelManager::with_agent(
        known.clone(),
        hacivat_net::ssh::TunnelConfig {
            idle_grace: ms(500),
            keepalive: secs(1),
            keepalive_max: 2,
        },
        None,
    );
    let (mut deps, fake) = deps_with(fake);
    deps.tunnels = tunnels;
    deps.known_hosts = known;
    DIRS.with(|d| d.borrow_mut().push(dir));
    (deps, fake)
}

async fn manager_kbd_otp() -> (
    Arc<ConnectionManager>,
    Arc<FakeDriver>,
    hacivat_testkit::ssh_server::TestSshServer,
) {
    let ssh = hacivat_testkit::ssh_server::TestSshServer::start(
        hacivat_testkit::ssh_server::TestSshConfig::default()
            .user_kbd("ops", &[("Verification code: ", false, "424242")]),
    );
    let profile = ssh_profile_via(
        &ssh,
        "ops",
        hacivat_net::ssh::SshAuth::KeyboardInteractive,
        pid(),
    );
    set_up(profile, Interaction::Interactive(Arc::new(OneTimeCode)));
    let (deps, fake) = deps_with_ssh(ok_pg());
    let m = ConnectionManager::new(tokio::runtime::Handle::current(), Arc::new(deps), config());
    (m, fake, ssh)
}

thread_local! {
    static BOTH: RefCell<Vec<Arc<Profile>>> = const { RefCell::new(Vec::new()) };
}

async fn manager_two_profiles_one_bastion() -> (
    Arc<ConnectionManager>,
    hacivat_testkit::ssh_server::TestSshServer,
    Arc<FakeDriver>,
) {
    let ssh = hacivat_testkit::ssh_server::TestSshServer::start(
        hacivat_testkit::ssh_server::TestSshConfig::default().user_pw("ops", "pw"),
    );
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
    let (deps, fake) = deps_with_ssh(ok_pg());
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

async fn wait_connected_both(m: &ConnectionManager) {
    for p in both() {
        wait_state_of(m, p.id, |s| matches!(s, LiveState::Reconnecting { .. })).await;
    }
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
        // `ForSession`: the draft password is kept for this app session, so the headless
        // reconnect finds it in the store (the overlay keeps it too once Task 14 lands).
        let profile = Arc::new(Profile {
            retention: Retention::ForSession,
            ..docker_support::profile_for(&t, mode)
        });
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
        // The next round trips find the loss (a server may first answer with its FATAL).
        let mut lost = false;
        for _ in 0..3 {
            let reply = session.submit(user_exec("SELECT 1")).reply.await.unwrap();
            if let Err(e) = reply
                && matches!(
                    e.kind,
                    ExecErrorKind::ConnectionLost { .. } | ExecErrorKind::NotConnected
                )
            {
                lost = true;
                break;
            }
        }
        assert!(lost, "{id}: the loss was never seen");
        wait_state_of(&m, profile.id, |s| matches!(s, LiveState::Connected(_))).await;
        assert!(
            matches!(
                next_event(&mut ev).await,
                ConnectionEvent::SessionLost { .. }
            ),
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
