//! The `ConnectionManager` and the core runtime (HV-CONN-01, CONN-04 subset, P15, R12, R13).

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use hacivat_proto::ErrClass;
use hacivat_secrets::{Retention, SecretString};
use hacivat_session::pipeline::Interaction;
use hacivat_session::profile::Profile;
use hacivat_session::{CoreRuntime, LiveState, ManagerConfig};

#[tokio::test]
async fn concurrent_connects_single_flight() {
    let (m, fake) = manager(ok_pg().delay(ms(100)));
    m.connect(p(), None, Interaction::Headless);
    m.connect(p(), None, Interaction::Headless);
    wait_connected(&m, p().id).await;
    assert_eq!(fake.connect_calls(), 1);
}

#[tokio::test]
async fn every_state_change_bumps_the_generation() {
    let (m, _) = manager(ok_pg());
    let changes = m.changes();
    let before = *changes.borrow();
    connect_and_wait(&m).await;
    let LiveState::Connected(info) = m.state(p().id) else {
        panic!("connected");
    };
    assert_eq!(info.user.as_deref(), Some("deploy"));
    assert!(
        *changes.borrow() >= before + 2,
        "Connecting, then Connected"
    );
    m.disconnect(p().id);
    assert_eq!(m.state(p().id), LiveState::Disconnected);
}

#[tokio::test]
async fn a_failed_connect_keeps_its_error() {
    let (m, _) = manager(ok_pg().connect_result(Err(refused())));
    m.connect(p(), None, Interaction::Headless);
    let mut changes = m.changes();
    let e = loop {
        if let LiveState::Failed(e) = m.state(p().id) {
            break e;
        }
        changes.changed().await.unwrap();
    };
    assert_eq!(e.class, ErrClass::NetworkRefused);
}

#[tokio::test(start_paused = true)]
async fn liveness_failure_marks_lost() {
    let (m, _) = manager_with(
        ok_pg().pings(vec![Err(io_reset())]),
        ManagerConfig {
            liveness: Duration::from_secs(30),
            ..Default::default()
        },
    );
    connect_and_wait(&m).await;
    tokio::time::advance(Duration::from_secs(31)).await;
    settle().await;
    assert_eq!(
        m.state(p().id),
        LiveState::Lost {
            class: ErrClass::NetworkReset
        }
    );
}

#[tokio::test]
async fn disconnect_forgets_session_secrets() {
    let (m, _fake, store) = manager_with_store(ok_pg());
    let k = db_key(p().id);
    store
        .put(&k, SecretString::from("x"), Retention::ForSession)
        .unwrap();
    m.connect(Arc::new(session_profile()), None, Interaction::Headless);
    wait_connected(&m, p().id).await;
    m.disconnect(p().id);
    settle().await;
    assert!(store.get(&k, Retention::ForSession).unwrap().is_none());
}

#[tokio::test]
async fn forget_cancels_inflight_and_clears_state() {
    let (m, fake) = manager(ok_pg().delay(ms(300)));
    m.connect(p(), None, Interaction::Headless);
    tokio::time::sleep(ms(20)).await;
    m.forget(p().id);
    assert_eq!(m.state(p().id), LiveState::Disconnected);
    tokio::time::sleep(ms(400)).await;
    assert_eq!(fake.close_calls(), 1);
    assert_eq!(m.state(p().id), LiveState::Disconnected);
}

#[test]
fn shutdown_closes_all_within_budget() {
    // A plain test with its own CoreRuntime, as at quit: shutdown_blocking blocks this
    // thread (P15).
    let rt = CoreRuntime::new().unwrap();
    let (m, fake) = manager_on(rt.handle(), ok_pg());
    for p in three_profiles() {
        m.connect(p.clone(), None, Interaction::Headless);
        rt.handle().block_on(wait_connected(&m, p.id));
    }
    let t = Instant::now();
    assert!(m.shutdown_blocking(Duration::from_secs(2)));
    assert!(t.elapsed() <= Duration::from_secs(2));
    assert_eq!(fake.close_calls(), 3);
    rt.shutdown(Duration::from_secs(1));
}

#[test]
fn a_connection_landing_after_shutdown_is_closed_at_once() {
    let rt = CoreRuntime::new().unwrap();
    let (m, fake) = manager_on(rt.handle(), ok_pg().delay(ms(200)));
    m.connect(p(), None, Interaction::Headless);
    // Shut down while the driver is connecting.
    while fake.connect_calls() == 0 {
        std::thread::sleep(ms(5));
    }
    assert!(m.shutdown_blocking(Duration::from_secs(2)));
    // A connect after shutdown started does nothing.
    m.connect(p(), None, Interaction::Headless);
    std::thread::sleep(ms(400));
    assert_eq!((fake.connect_calls(), fake.close_calls()), (1, 1));
    assert_eq!(m.state(p().id), LiveState::Disconnected);
    rt.shutdown(Duration::from_secs(1));
}

#[tokio::test]
async fn test_takes_no_budget_permit() {
    let (m, _) = manager(ok_pg());
    let p1 = Arc::new(Profile {
        max_connections: 1,
        ..p().as_ref().clone()
    });
    m.connect(p1.clone(), None, Interaction::Headless);
    wait_connected(&m, p1.id).await;
    let r = m
        .test(
            draft(p1.as_ref().clone(), Some("pw")),
            Interaction::Headless,
        )
        .result
        .await
        .unwrap();
    assert!(r.ok, "{:?}", r.error.map(|e| e.class));
}

#[test]
fn core_runtime_threads_are_named() {
    let rt = CoreRuntime::new().unwrap();
    let n = rt.handle().block_on(async {
        tokio::task::spawn_blocking(|| std::thread::current().name().map(String::from))
            .await
            .unwrap()
    });
    assert!(n.unwrap().starts_with("hv-core"));
}

// Fix round 1.

#[tokio::test]
async fn a_sqlite_reconnect_after_forget_waits_for_the_close() {
    // The budget outlives `forget`: the second open waits for the first close (200 ms).
    let (m, log) = logged_manager(
        hacivat_proto::DbmsKind::Sqlite,
        ms(200),
        false,
        ManagerConfig::default(),
    );
    let s = Arc::new(sqlite_profile());
    m.connect(s.clone(), None, Interaction::Headless);
    wait_connected(&m, s.id).await;
    m.forget(s.id);
    m.connect(s.clone(), None, Interaction::Headless);
    wait_connected(&m, s.id).await;
    assert_eq!(*log.lock().unwrap(), ["open", "close", "open"]);
}

#[tokio::test]
async fn a_changed_budget_waits_for_the_old_connection_to_close() {
    let (m, log) = logged_manager(
        hacivat_proto::DbmsKind::Postgres,
        ms(200),
        false,
        ManagerConfig::default(),
    );
    let one = Arc::new(Profile {
        max_connections: 1,
        ..p().as_ref().clone()
    });
    m.connect(one.clone(), None, Interaction::Headless);
    wait_connected(&m, one.id).await;
    m.disconnect(one.id);
    let two = Arc::new(Profile {
        max_connections: 2,
        ..one.as_ref().clone()
    });
    m.connect(two.clone(), None, Interaction::Headless);
    wait_connected(&m, two.id).await;
    assert_eq!(*log.lock().unwrap(), ["open", "close", "open"]);
}

#[tokio::test]
async fn shutdown_never_waits_for_a_hanging_ping() {
    let (m, log) = logged_manager(
        hacivat_proto::DbmsKind::Postgres,
        Duration::ZERO,
        true,
        ManagerConfig {
            liveness: ms(20),
            ..Default::default()
        },
    );
    connect_and_wait(&m).await;
    // Let the first ping start (and hang, holding the connection).
    while !log.lock().unwrap().contains(&"ping") {
        tokio::time::sleep(ms(5)).await;
    }
    let t = Instant::now();
    m.shutdown().await;
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    assert_eq!(*log.lock().unwrap(), ["open", "ping", "close"]);
}

#[tokio::test]
async fn dropping_the_manager_stops_its_liveness_pings() {
    let (m, log) = logged_manager(
        hacivat_proto::DbmsKind::Postgres,
        Duration::ZERO,
        false,
        ManagerConfig {
            liveness: ms(20),
            ..Default::default()
        },
    );
    connect_and_wait(&m).await;
    tokio::time::sleep(ms(60)).await;
    drop(m);
    let pings = || log.lock().unwrap().iter().filter(|e| **e == "ping").count();
    let after_drop = pings();
    assert!(after_drop >= 1);
    tokio::time::sleep(ms(100)).await;
    assert_eq!(pings(), after_drop);
}

#[tokio::test]
async fn a_connect_landing_after_the_manager_dropped_is_closed_without_pings() {
    // The exact window (pipeline done, `landed` not yet run) cannot be hit deterministically;
    // a connect in flight at the drop must end closed, with no liveness task left behind.
    let (m, log) = logged_manager(
        hacivat_proto::DbmsKind::Postgres,
        Duration::ZERO,
        false,
        ManagerConfig {
            liveness: ms(10),
            ..Default::default()
        },
    );
    m.connect(p(), None, Interaction::Headless);
    tokio::time::sleep(ms(5)).await;
    drop(m);
    tokio::time::sleep(ms(100)).await;
    let log = log.lock().unwrap().clone();
    assert!(!log.contains(&"ping"), "{log:?}");
    assert_eq!(
        log.iter().filter(|e| **e == "open").count(),
        log.iter().filter(|e| **e == "close").count(),
        "{log:?}"
    );
}

#[tokio::test]
async fn the_error_dialogs_reentry_continues_the_count() {
    let (m, fake) = manager(ok_pg().connect_result(Err(db("28P01"))));
    let pw = Arc::new(Profile {
        auth: hacivat_secrets::AuthProviderId::Password,
        retention: Retention::Never,
        ..p().as_ref().clone()
    });
    m.connect_reentry(
        pw.clone(),
        SecretString::from("again"),
        2,
        Interaction::Headless,
    );
    let mut changes = m.changes();
    let e = loop {
        if let LiveState::Failed(e) = m.state(pw.id) {
            break e;
        }
        changes.changed().await.unwrap();
    };
    assert_eq!((e.class, e.attempts), (ErrClass::AuthInvalid, 3));
    assert_eq!(fake.connect_calls(), 1);
}
