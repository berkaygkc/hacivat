//! M3b Task 12 (C7, R-M3-73): per-session liveness. Every console session of a profile runs
//! its own supervisor: its own keep-alive and its own idle clock, so the idle-transaction
//! warning fires per session; the profile's idle time is the latest user activity on any
//! session, and the idle disconnect (the default session's, which ends the profile) waits for
//! every user transaction and user request on any of them. Deactivating a console other than
//! the default breaks only that session.
//!
//! FakeDriver on a paused clock; every wait is on a condition, and a transient state is held
//! open by a statement that runs until it is cancelled.

mod common;

use std::sync::Arc;

use common::*;
use hacivat_driver::fake::FakeDriver;
use hacivat_driver::{ExecOpts, TxStatus};
use hacivat_net::TlsMode;
use hacivat_proto::{Department, ErrClass};
use hacivat_session::pipeline::Interaction;
use hacivat_session::profile::Profile;
use hacivat_session::session::{
    CancelOutcome, ClientId, Escalation, ExecOrigin, Request, SessionEvent, SessionHandle,
    SessionKind, SessionSpec, SessionState,
};
use hacivat_session::{ConnectionEvent, ConnectionManager, LiveState, ManagerConfig};
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::TryRecvError;

/// M2b recovery with M2a's keep-alive (every session pings every 30 s).
fn config() -> ManagerConfig {
    ManagerConfig {
        reconnect: true,
        ..ManagerConfig::default()
    }
}

/// The test profile with an idle disconnect of `s` seconds.
fn idle(s: u32) -> Arc<Profile> {
    Arc::new(Profile {
        idle_disconnect_s: Some(s),
        ..pg_profile(TlsMode::Disable)
    })
}

/// A server whose connections follow `BEGIN` / `COMMIT` / `ROLLBACK`.
fn server() -> FakeDriver {
    ok_pg().tracks_transactions()
}

async fn connect(m: &ConnectionManager, profile: &Arc<Profile>) {
    m.connect(profile.clone(), None, Interaction::Headless);
    wait_connected(m, profile.id).await;
}

fn default_of(m: &ConnectionManager, profile: &Profile) -> SessionHandle {
    m.sessions(profile.id)
        .and_then(|s| s.default_session())
        .expect("the default session")
}

/// Console `n`'s own session, next to the default one.
async fn console(m: &ConnectionManager, profile: &Profile, n: u64) -> SessionHandle {
    let spec = SessionSpec {
        kind: SessionKind::Console(ClientId(n)),
        read_only: false,
        department: Department::Console,
    };
    m.sessions(profile.id)
        .expect("the profile is connected")
        .open(spec, Interaction::Headless)
        .await
        .expect("console session")
}

fn user_exec(sql: &str) -> Request {
    Request::Exec {
        sql: sql.to_string(),
        opts: ExecOpts::default(),
        origin: ExecOrigin::User,
    }
}

async fn ok(s: &SessionHandle, sql: &str) {
    let reply = s.submit(user_exec(sql)).reply.await.unwrap();
    assert!(reply.is_ok(), "{sql}: {reply:?}");
}

fn connected(m: &ConnectionManager, profile: &Profile) -> bool {
    matches!(m.state(profile.id), LiveState::Connected(_))
}

/// Waits (letting the paused clock run, 10 ms steps) until `done`; panics after [`WAIT`] of
/// real time.
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

fn drain<T: Clone>(ev: &mut Receiver<T>) -> Vec<T> {
    let mut out = Vec::new();
    loop {
        match ev.try_recv() {
            Ok(e) => out.push(e),
            Err(TryRecvError::Empty) => return out,
            Err(e) => panic!("the event channel failed: {e:?}"),
        }
    }
}

fn idle_warnings(events: &[SessionEvent]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, SessionEvent::IdleTransaction { .. }))
        .count()
}

/// Review focus 4: the default idles far past its threshold while console 2 holds `BEGIN`; the
/// profile stays connected and the transaction open. After console 2's `COMMIT` and one idle
/// period, the profile disconnects.
#[tokio::test(start_paused = true)]
async fn idle_disconnect_waits_for_any_session_tx() {
    let (m, f) = manager_with(server(), config());
    let profile = idle(300);
    let mut ev = m.events();
    connect(&m, &profile).await;
    let console2 = console(&m, &profile, 2).await;
    ok(&console2, "BEGIN").await;
    ok(&console2, "UPDATE t SET a = 1").await;

    tokio::time::sleep(secs(1800)).await;
    assert!(connected(&m, &profile), "{:?}", m.state(profile.id));
    assert_eq!(console2.state().borrow().tx.status, TxStatus::InTx);
    assert!(
        !f.executed().iter().any(|s| s.starts_with("ROLLBACK")),
        "a user transaction is never ended by idle logic: {:?}",
        f.executed()
    );
    // Console 2's own supervisor never disconnects either: it is still there.
    assert_eq!(console2.state().borrow().state, SessionState::Idle);

    ok(&console2, "COMMIT").await;
    let committed = tokio::time::Instant::now();
    until("the idle disconnect", || {
        m.state(profile.id) == LiveState::Disconnected
    })
    .await;
    // One idle period after the commit, the profile's last activity.
    assert!(
        committed.elapsed() >= secs(300),
        "{:?}",
        committed.elapsed()
    );
    let events = drain(&mut ev);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ConnectionEvent::IdleDisconnected { .. })),
        "{events:?}"
    );
}

/// The profile's idle time is the most recent activity on any session: activity on console 2
/// alone keeps it, and the threshold counts from that activity.
#[tokio::test(start_paused = true)]
async fn the_profile_idles_from_its_last_activity_on_any_session() {
    let (m, _f) = manager_with(server(), config());
    let profile = idle(300);
    connect(&m, &profile).await;
    let console2 = console(&m, &profile, 2).await;
    tokio::time::sleep(secs(250)).await;
    ok(&console2, "SELECT 1").await;
    let last = tokio::time::Instant::now();
    tokio::time::sleep(secs(200)).await;
    assert!(
        connected(&m, &profile),
        "within 300 s of console 2's activity"
    );
    until("the idle disconnect", || {
        m.state(profile.id) == LiveState::Disconnected
    })
    .await;
    assert!(last.elapsed() >= secs(300), "{:?}", last.elapsed());
}

/// Each session warns of its own idle transaction, on its own clock, once per idle period.
#[tokio::test(start_paused = true)]
async fn idle_transaction_fires_per_session() {
    let (m, _f) = manager_with(server(), config());
    let profile = idle(300);
    let mut manager_events = m.events();
    connect(&m, &profile).await;
    let default = default_of(&m, &profile);
    let console2 = console(&m, &profile, 2).await;
    let console3 = console(&m, &profile, 3).await;
    let (mut ev1, mut ev2, mut ev3) = (default.events(), console2.events(), console3.events());

    ok(&default, "BEGIN").await;
    tokio::time::sleep(secs(100)).await;
    ok(&console2, "BEGIN").await;
    let console2_began = tokio::time::Instant::now();
    // Console 3 has no transaction: it never warns.

    tokio::time::sleep(secs(250)).await;
    // The default's transaction idled 350 s, console 2's 250 s.
    assert_eq!(idle_warnings(&drain(&mut ev1)), 1, "the default warned");
    assert_eq!(idle_warnings(&drain(&mut ev2)), 0, "console 2 not yet");
    until("console 2's warning", || {
        idle_warnings(&drain(&mut ev2)) == 1
    })
    .await;
    assert!(
        console2_began.elapsed() >= secs(300),
        "on console 2's own clock: {:?}",
        console2_began.elapsed()
    );
    // Once per idle period: nothing more while they stay idle.
    tokio::time::sleep(secs(900)).await;
    assert_eq!(idle_warnings(&drain(&mut ev1)), 0);
    assert_eq!(idle_warnings(&drain(&mut ev2)), 0);
    assert_eq!(
        idle_warnings(&drain(&mut ev3)),
        0,
        "console 3 holds nothing"
    );
    // The default's warning still reaches the app as the profile's.
    assert!(
        drain(&mut manager_events)
            .iter()
            .any(|e| matches!(e, ConnectionEvent::IdleTransaction { .. })),
    );
    // Nothing was ended, and the profile is still connected: a console's own supervisor
    // never disconnects, not even console 3's, idle 1,250 s without a transaction.
    assert!(connected(&m, &profile));
    assert_eq!(console3.state().borrow().state, SessionState::Idle);
    assert_eq!(default.state().borrow().tx.status, TxStatus::InTx);
    assert_eq!(console2.state().borrow().tx.status, TxStatus::InTx);

    // Activity starts a new idle period: console 2 warns again 300 s after it.
    ok(&console2, "UPDATE t SET a = 1").await;
    tokio::time::sleep(secs(290)).await;
    assert_eq!(idle_warnings(&drain(&mut ev2)), 0);
    until("console 2's second warning", || {
        idle_warnings(&drain(&mut ev2)) == 1
    })
    .await;
}

/// Every console session keeps its own connection alive.
#[tokio::test(start_paused = true)]
async fn every_console_session_pings_its_own_connection() {
    let (m, f) = manager_with(server(), config());
    let profile = idle(0);
    connect(&m, &profile).await;
    let _console2 = console(&m, &profile, 2).await;
    let before = f.ping_calls();
    tokio::time::sleep(secs(95)).await;
    // Two sessions, a ping each every 30 s.
    assert_eq!(f.ping_calls() - before, 6);
}

/// R-M3-73: the Disconnect of a console other than the default (`Escalation::Deactivate`)
/// breaks only its own session; the profile stays connected and the default keeps running.
#[tokio::test(start_paused = true)]
async fn deactivating_a_non_default_console_breaks_only_it() {
    let (m, f) = manager_with(
        server().exec(vec![common::slow("SELECT pg_sleep(600)", secs(600))]),
        config(),
    );
    let profile = idle(0);
    let mut ev = m.events();
    connect(&m, &profile).await;
    let default = default_of(&m, &profile);
    let console2 = console(&m, &profile, 2).await;
    ok(&console2, "BEGIN").await;
    let sessions = m.sessions(profile.id).expect("connected");
    // The open transaction is named before anything closes it.
    let open = sessions.open_transactions();
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0].1, SessionKind::Console(ClientId(2)));
    assert_eq!(open[0].2.status, TxStatus::InTx);

    // A statement held open on console 2 (600 s on the paused clock).
    let job = console2.submit(user_exec("SELECT pg_sleep(600)"));
    until("the statement runs", || {
        matches!(console2.state().borrow().state, SessionState::Busy { .. })
    })
    .await;
    let outcome = console2
        .escalate(job.seq, Escalation::Deactivate)
        .await
        .unwrap();
    assert_eq!(outcome, CancelOutcome::Cancelled);
    assert_eq!(
        console2.state().borrow().state,
        SessionState::Broken(ErrClass::Cancelled)
    );
    assert_eq!(console2.state().borrow().tx.status, TxStatus::Idle);
    // Only console 2 ended.
    assert!(connected(&m, &profile), "{:?}", m.state(profile.id));
    assert_eq!(default.state().borrow().state, SessionState::Idle);
    ok(&default, "SELECT 1").await;
    assert!(sessions.open_transactions().is_empty());
    let events = drain(&mut ev);
    assert!(
        !events.iter().any(|e| matches!(
            e,
            ConnectionEvent::SessionLost { .. } | ConnectionEvent::IdleDisconnected { .. }
        )),
        "{events:?}"
    );
    assert!(f.executed().iter().any(|s| s == "SELECT 1"));
}

/// R-M3-73: the default console's Disconnect closes the whole profile; the transactions the
/// other consoles hold are named first (`open_transactions`) and published gone afterwards.
#[tokio::test(start_paused = true)]
async fn deactivating_the_default_closes_the_profile_and_publishes_every_transaction() {
    let (m, _f) = manager_with(
        server().exec(vec![common::slow("SELECT pg_sleep(600)", secs(600))]),
        config(),
    );
    let profile = idle(0);
    connect(&m, &profile).await;
    let default = default_of(&m, &profile);
    let console2 = console(&m, &profile, 2).await;
    ok(&console2, "BEGIN").await;
    ok(&console2, "UPDATE t SET a = 1").await;
    let sessions = m.sessions(profile.id).expect("connected");
    let open = sessions.open_transactions();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].2.statements, 1);
    let mut ev2 = console2.events();

    let job = default.submit(user_exec("SELECT pg_sleep(600)"));
    until("the statement runs", || {
        matches!(default.state().borrow().state, SessionState::Busy { .. })
    })
    .await;
    assert_eq!(
        default
            .escalate(job.seq, Escalation::Deactivate)
            .await
            .unwrap(),
        CancelOutcome::Cancelled
    );
    until("the profile closes", || {
        m.state(profile.id) == LiveState::Disconnected
    })
    .await;
    until("console 2 closes", || {
        console2.state().borrow().state == SessionState::Disconnected
    })
    .await;
    assert_eq!(console2.state().borrow().tx.status, TxStatus::Idle);
    assert!(
        drain(&mut ev2)
            .iter()
            .any(|e| matches!(e, SessionEvent::TxChanged(tx) if tx.status == TxStatus::Idle)),
        "console 2's rolled-back transaction is published"
    );
}
