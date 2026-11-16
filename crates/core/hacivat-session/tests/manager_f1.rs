//! Follow-up F1-b (M3a; ADR 0004 `m3a-f1`): a profile's default session ends the whole profile
//! when it ends (`end_live` closes every session with a rollback), so
//! - its idle disconnect waits while another session of the profile holds a user transaction
//!   or has had user activity within the idle threshold;
//! - when it is lost, a user transaction another session held is never rolled back silently:
//!   the manager emits one `SessionLost` with `open_tx` (the app's "transaction rolled back"
//!   toast).
//!
//! Every test runs over `FakeDriver` on a paused clock and waits on conditions (Q25).

mod common;

use std::sync::Arc;

use common::*;
use hacivat_driver::fake::{FakeDriver, FakeExec};
use hacivat_driver::{ExecOpts, ExecSummary, TxStatus};
use hacivat_net::TlsMode;
use hacivat_proto::{Department, ErrClass};
use hacivat_session::pipeline::Interaction;
use hacivat_session::profile::Profile;
use hacivat_session::session::{
    ClientId, ExecOrigin, Request, SessionHandle, SessionKind, SessionSpec, SessionState,
    TxCommand, TxMode,
};
use hacivat_session::{ConnectionEvent, ConnectionManager, LiveState, ManagerConfig};
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::TryRecvError;

/// M2b recovery with M2a's keep-alive (every profile pings every 30 s).
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

/// Console `n`'s own session (Task 32's console sessions), next to the default one.
async fn own_session(m: &ConnectionManager, profile: &Profile, n: u64) -> SessionHandle {
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

fn tx_of(s: &SessionHandle) -> TxStatus {
    s.state().borrow().tx.status
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

/// Every event emitted so far.
fn drain(ev: &mut Receiver<ConnectionEvent>) -> Vec<ConnectionEvent> {
    let mut out = Vec::new();
    loop {
        match ev.try_recv() {
            Ok(e) => out.push(e),
            Err(TryRecvError::Empty) => return out,
            Err(e) => panic!("the event channel failed: {e:?}"),
        }
    }
}

fn rollbacks(f: &FakeDriver) -> usize {
    f.executed()
        .iter()
        .filter(|s| s.starts_with("ROLLBACK"))
        .count()
}

/// The ruling's test: the default goes idle far past its threshold while Console 2's own
/// session holds a user transaction; the profile stays connected and the transaction open.
/// Once it commits, the threshold counts from the commit.
#[tokio::test(start_paused = true)]
async fn the_default_never_idle_disconnects_while_another_session_holds_a_user_transaction() {
    let (m, f) = manager_with(server(), config());
    let profile = idle(300);
    let mut ev = m.events();
    connect(&m, &profile).await;
    let console2 = own_session(&m, &profile, 2).await;
    ok(&console2, "BEGIN").await;
    ok(&console2, "UPDATE t SET a = 1").await;
    assert_eq!(tx_of(&console2), TxStatus::InTx);

    tokio::time::sleep(secs(1800)).await;
    assert!(connected(&m, &profile), "{:?}", m.state(profile.id));
    assert_eq!(tx_of(&console2), TxStatus::InTx, "the transaction is open");
    assert!(
        matches!(console2.state().borrow().state, SessionState::Idle),
        "{:?}",
        console2.state().borrow().state
    );
    assert_eq!(rollbacks(&f), 0, "{:?}", f.executed());
    assert!(
        !drain(&mut ev)
            .iter()
            .any(|e| matches!(e, ConnectionEvent::IdleDisconnected { .. })),
        "no idle disconnect"
    );

    // The commit is the profile's last activity: the threshold counts from it.
    ok(&console2, "COMMIT").await;
    assert_eq!(tx_of(&console2), TxStatus::Idle);
    tokio::time::sleep(secs(290)).await;
    assert!(
        connected(&m, &profile),
        "within the threshold of the commit"
    );
    until("the idle disconnect", || {
        m.state(profile.id) == LiveState::Disconnected
    })
    .await;
    let events = drain(&mut ev);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ConnectionEvent::IdleDisconnected { .. })),
        "{events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ConnectionEvent::SessionLost { .. })),
        "nothing was held when it disconnected: {events:?}"
    );
}

/// User activity on another session keeps the profile connected; the idle threshold counts
/// from the profile's last activity on any session.
#[tokio::test(start_paused = true)]
async fn activity_on_another_session_keeps_the_profile_connected() {
    let (m, _) = manager_with(server(), config());
    let profile = idle(300);
    connect(&m, &profile).await;
    let console2 = own_session(&m, &profile, 2).await;
    for _ in 0..10 {
        tokio::time::sleep(secs(100)).await;
        assert!(connected(&m, &profile), "{:?}", m.state(profile.id));
        ok(&console2, "SELECT 1").await;
    }
    tokio::time::sleep(secs(290)).await;
    assert!(connected(&m, &profile));
    until("the idle disconnect", || {
        m.state(profile.id) == LiveState::Disconnected
    })
    .await;
}

/// The default session is lost (its keep-alive fails, no recovery) while Console 2 holds a
/// user transaction: closing the profile rolls it back, and the manager says so.
#[tokio::test(start_paused = true)]
async fn losing_the_default_never_rolls_back_another_sessions_transaction_silently() {
    let (m, f) = manager_with(
        server().pings(vec![Err(io_reset())]),
        ManagerConfig::default(),
    );
    let profile = Arc::new(pg_profile(TlsMode::Disable));
    let mut ev = m.events();
    connect(&m, &profile).await;
    let console2 = own_session(&m, &profile, 2).await;
    // ## m3b-t12-session-carries (named exception): console 2 now keeps its own keep-alive
    // (R-M3-73) and this fake fails every connection's first ping; its `BEGIN` comes 5 s
    // after the connect, so its ping is due after the default's and the default is lost
    // first, as this test means.
    tokio::time::sleep(secs(5)).await;
    ok(&console2, "BEGIN").await;
    until("the loss", || {
        matches!(m.state(profile.id), LiveState::Lost { .. })
    })
    .await;
    assert_eq!(
        m.state(profile.id),
        LiveState::Lost {
            class: ErrClass::NetworkReset
        }
    );
    until("the SessionLost event", || {
        drain(&mut ev).into_iter().any(|e| {
            matches!(
                e,
                ConnectionEvent::SessionLost { profile: p, lost } if p == profile.id && lost.open_tx
            )
        })
    })
    .await;
    // The healthy session was closed with its transaction rolled back, and the app was told.
    until("console 2's close", || {
        !matches!(
            console2.state().borrow().state,
            SessionState::Idle | SessionState::Busy { .. }
        )
    })
    .await;
    assert_eq!(rollbacks(&f), 1, "{:?}", f.executed());
}

/// Without a transaction elsewhere, a loss of an idle default says nothing more than before.
#[tokio::test(start_paused = true)]
async fn losing_an_idle_default_without_a_transaction_elsewhere_emits_no_session_lost() {
    let (m, _) = manager_with(
        server().pings(vec![Err(io_reset())]),
        ManagerConfig::default(),
    );
    let profile = Arc::new(pg_profile(TlsMode::Disable));
    let mut ev = m.events();
    connect(&m, &profile).await;
    let console2 = own_session(&m, &profile, 2).await;
    ok(&console2, "SELECT 1").await;
    until("the loss", || {
        matches!(m.state(profile.id), LiveState::Lost { .. })
    })
    .await;
    // Fix round 1 (M5): a condition, not a settle sleep, before the negative assertion.
    until("console 2's close", || {
        !matches!(
            console2.state().borrow().state,
            SessionState::Idle | SessionState::Busy { .. }
        )
    })
    .await;
    let events = drain(&mut ev);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ConnectionEvent::SessionLost { .. })),
        "{events:?}"
    );
}

/// Fix round 1, C1: the snapshot's transaction moves only when a request ends. Console 2 runs
/// a Manual-mode `UPDATE` (the session sends `BEGIN` first, so a transaction is open on the
/// server) that is still running when the default session is lost: closing it rolls that
/// transaction back, and the manager says so.
#[tokio::test(start_paused = true)]
async fn losing_the_default_while_another_session_may_be_opening_a_transaction_is_told() {
    let slow_update = FakeExec {
        prefix: "UPDATE",
        result: Ok(ExecSummary::default()),
        delay: secs(600),
        tx_after: None,
        panics: false,
    };
    let (m, f) = manager_with(
        server()
            .exec(vec![slow_update])
            .pings(vec![Err(io_reset())]),
        ManagerConfig::default(),
    );
    let profile = Arc::new(pg_profile(TlsMode::Disable));
    let mut ev = m.events();
    connect(&m, &profile).await;
    let console2 = own_session(&m, &profile, 2).await;
    let manual = console2
        .submit(Request::Tx(TxCommand::SetMode(TxMode::Manual)))
        .reply
        .await
        .unwrap();
    assert!(manual.is_ok(), "{manual:?}");
    let update = console2.submit(user_exec("UPDATE t SET a = 1"));
    until("the UPDATE to start inside its BEGIN", || {
        f.executed().iter().any(|s| s.starts_with("UPDATE"))
    })
    .await;
    assert!(
        matches!(console2.state().borrow().state, SessionState::Busy { .. }),
        "{:?}",
        console2.state().borrow().state
    );
    assert_eq!(
        tx_of(&console2),
        TxStatus::Idle,
        "the snapshot does not know the transaction yet"
    );
    until("the loss", || {
        matches!(m.state(profile.id), LiveState::Lost { .. })
    })
    .await;
    until("the SessionLost event", || {
        drain(&mut ev).into_iter().any(|e| {
            matches!(
                e,
                ConnectionEvent::SessionLost { profile: p, lost } if p == profile.id && lost.open_tx
            )
        })
    })
    .await;
    let reply = update.reply.await.unwrap();
    assert!(reply.is_err(), "the UPDATE never completed: {reply:?}");
}
