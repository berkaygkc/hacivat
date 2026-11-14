//! M3a Task 18: production consoles are read-only (R-M3-6 REVISED, R-M3-29, P4). The
//! production policy (`SessionPolicy::production_read_only`, set by the app's boot) reaches
//! every session of a production-tagged profile: the default console, other consoles, pooled
//! and job sessions, single-session borrows and reconnects.

mod common;

use std::sync::Arc;

use common::runs::*;
use common::*;
use hacivat_driver::fake::FakeDriver;
use hacivat_net::TlsMode;
use hacivat_proto::{AccessLabel, DbmsKind, Department, EnvTag, ProfileId};
use hacivat_session::budget_for;
use hacivat_session::pipeline::Interaction;
use hacivat_session::profile::Profile;
use hacivat_session::session::{
    ClientId, DenyLayer, ExecErrorKind, JobKind, SessionHandle, SessionKind, SessionManager,
    SessionPolicy, SessionSpec, TxMode,
};
use hacivat_session::{ConnectionManager, ManagerConfig, effective_read_only};

/// The app's configuration: M2b's, with production profiles read-only.
fn app_config() -> ManagerConfig {
    ManagerConfig {
        production_read_only: true,
        ..ManagerConfig::m2b()
    }
}

/// A PostgreSQL fake whose reset verify probe answers true (a pooled session is reset when
/// its borrow ends; the verify fails closed on no row).
fn fake() -> FakeDriver {
    fake_pg().exec(
        (0..8)
            .map(|_| rows("SELECT current_user = session_user", &[&["t"]]))
            .collect(),
    )
}

fn tagged(env: EnvTag) -> Profile {
    Profile {
        id: ProfileId::new(),
        environment: Some(env),
        max_connections: 8,
        ..pg_profile(TlsMode::Disable)
    }
}

/// A profile's session manager under the app's policy.
fn sessions_of(fake: FakeDriver, profile: Profile) -> (Arc<SessionManager>, Arc<FakeDriver>) {
    let (deps, fake) = deps_with(fake);
    let budget = Arc::new(budget_for(&profile));
    let sm = SessionManager::new(
        tokio::runtime::Handle::current(),
        Arc::new(profile),
        Arc::new(deps),
        budget,
    );
    sm.set_policy(SessionPolicy {
        production_read_only: true,
    });
    (sm, fake)
}

fn spec(kind: SessionKind) -> SessionSpec {
    SessionSpec {
        kind,
        read_only: false,
        department: Department::Console,
    }
}

fn default_of(m: &ConnectionManager, id: ProfileId) -> SessionHandle {
    m.sessions(id).unwrap().default_session().unwrap()
}

/// A session is read-only by every sign it gives: its snapshot, its label, and what it does
/// with a statement: every one goes through the read-only guard, which refuses transaction
/// control before anything is sent (the fake has no server to refuse a write, so the guard's
/// own refusal is the proof that the session runs guarded).
async fn assert_read_only(h: &SessionHandle, f: &FakeDriver, what: &str) {
    let s = snap(h);
    assert!(s.read_only, "{what}: the snapshot");
    assert_eq!(s.access, AccessLabel::ReadOnlyGuarded, "{what}: the label");
    assert_eq!(
        s.tx.mode,
        TxMode::Auto,
        "{what}: a read-only session is Auto"
    );
    let e = run(h, "COMMIT").await.unwrap_err();
    assert_eq!(
        e.kind,
        ExecErrorKind::ReadOnlyDenied(DenyLayer::Transaction),
        "{what}"
    );
    assert!(
        !f.executed().iter().any(|s| s == "COMMIT"),
        "{what}: the statement was sent"
    );
}

#[tokio::test(start_paused = true)]
async fn production_session_is_read_only_after_reconnect() {
    let profile = Arc::new(Profile {
        keepalive_s: Some(5),
        ..tagged(EnvTag::Production)
    });
    let (m, f) = manager_with(fake().pings(vec![Err(io_reset())]), app_config());
    m.connect(profile.clone(), None, Interaction::Headless);
    wait_connected(&m, profile.id).await;
    let h = default_of(&m, profile.id);
    assert_read_only(&h, &f, "before the loss").await;
    // The keep-alive finds the connection gone; the session reconnects in place.
    let mut events = h.events();
    tokio::time::advance(secs(6)).await;
    loop {
        let event = next(&mut events).await;
        if event == hacivat_session::session::SessionEvent::Reconnected {
            break;
        }
    }
    assert_eq!(f.connect_calls(), 2);
    // Both connects were read-only, and so is the session on its new connection.
    assert!(
        f.requests().iter().all(|r| r.read_only),
        "{:?}",
        f.requests()
    );
    assert_read_only(&h, &f, "after the reconnect").await;
    // A typed run on it sends nothing either (the typed guard fails closed until Task 22).
    let sent = f.call_log().len();
    let o = Recorder::new();
    run_spec(&h, spec_of(&o, &["DELETE FROM t"]))
        .await
        .unwrap_err();
    assert_eq!(f.call_log().len(), sent);
}

#[tokio::test]
async fn production_tag_applies_to_the_next_session() {
    let dev = Arc::new(tagged(EnvTag::Dev));
    let (m, f) = manager_with(fake(), app_config());
    m.connect(dev.clone(), None, Interaction::Headless);
    wait_connected(&m, dev.id).await;
    let sm = m.sessions(dev.id).unwrap();
    let before = default_of(&m, dev.id);
    assert!(!snap(&before).read_only);
    // The profile is tagged production while its console is open.
    let prod = Arc::new(Profile {
        environment: Some(EnvTag::Production),
        ..(*dev).clone()
    });
    assert!(effective_read_only(&prod, m.policy().production_read_only));
    m.update_profile(dev.id, prod);
    // Every session opened from now on is read-only, whatever kind it is.
    let console = sm
        .open(spec(SessionKind::Console(ClientId(2))), ix_none())
        .await
        .unwrap();
    assert_read_only(&console, &f, "a console opened after the tag").await;
    let pooled = sm.pooled(SessionKind::Service).await.unwrap();
    assert_read_only(&pooled, &f, "a pooled session after the tag").await;
    // The session that was open keeps the mode it was opened with: a session never changes
    // sides under a statement. The app reconnects the profile to move it (Task 32).
    assert!(!snap(&before).read_only);
}

#[tokio::test]
async fn single_session_production_is_read_only() {
    let profile = Profile {
        single_session: true,
        ..tagged(EnvTag::Production)
    };
    let (sm, f) = sessions_of(fake(), profile);
    let console = sm
        .open(spec(SessionKind::Console(ClientId::DEFAULT)), ix_none())
        .await
        .unwrap();
    assert_read_only(&console, &f, "the single session").await;
    // Every other client shares it.
    let other = sm
        .open(spec(SessionKind::Console(ClientId(2))), ix_none())
        .await
        .unwrap();
    assert_eq!(other.id(), console.id());
    assert_eq!(f.connect_calls(), 1);
    assert!(f.requests().iter().all(|r| r.read_only));
}

#[tokio::test]
async fn every_session_kind_on_a_production_profile_is_read_only() {
    let kinds = [
        SessionKind::Console(ClientId::DEFAULT),
        SessionKind::Console(ClientId(2)),
        SessionKind::Editor("public.t".into()),
        SessionKind::Introspection { database: None },
        SessionKind::Service,
        SessionKind::Job(1, JobKind::Export),
    ];
    let (sm, f) = sessions_of(fake(), tagged(EnvTag::Production));
    let mut open = Vec::new();
    for kind in kinds.clone() {
        let h = sm.open(spec(kind.clone()), ix_none()).await.unwrap();
        assert_read_only(&h, &f, &format!("opened {kind:?}")).await;
        open.push(h);
    }
    // And borrowed from the pool.
    let borrowed = sm.pooled(SessionKind::Service).await.unwrap();
    assert_read_only(&borrowed, &f, "a pooled borrow").await;
    // Every connection was asked for read-only (the driver's own layer is on too).
    assert_eq!(f.connect_calls(), kinds.len() + 1);
    assert!(f.requests().iter().all(|r| r.read_only));

    // The same profile without the tag is read-write: the policy is what turned it.
    let (sm, f) = sessions_of(fake(), tagged(EnvTag::Staging));
    let h = sm
        .open(spec(SessionKind::Console(ClientId::DEFAULT)), ix_none())
        .await
        .unwrap();
    assert!(!snap(&h).read_only);
    run(&h, "COMMIT").await.unwrap();
    assert!(f.executed().iter().any(|s| s == "COMMIT"));
}

#[tokio::test]
async fn pooled_borrow_on_production_reuses_its_session() {
    // P4: the pool keys its idle sessions by the effective read-only flag, so a production
    // borrow finds the read-only session it parked instead of opening one per borrow.
    let (sm, f) = sessions_of(fake(), tagged(EnvTag::Production));
    let first = sm.pooled(SessionKind::Service).await.unwrap();
    let id = first.id();
    assert!(snap(&first).read_only);
    drop(first);
    settle_until("the release resets the session", || f.reset_calls() >= 1).await;
    settle().await;
    let second = sm.pooled(SessionKind::Service).await.unwrap();
    assert_eq!(second.id(), id);
    assert!(snap(&second).read_only);
    assert_eq!(f.connect_calls(), 1);
    assert_eq!(f.open_connections(), 1);
    // Many borrows in a row never grow the pool.
    drop(second);
    for n in 2..6 {
        settle_until("the release resets the session", || f.reset_calls() >= n).await;
        settle().await;
        drop(sm.pooled(SessionKind::Service).await.unwrap());
    }
    assert_eq!(f.connect_calls(), 1);
}

#[tokio::test]
async fn single_session_production_borrow_succeeds() {
    // P4: in single-session mode a borrow shares the default session, which is read-only by
    // the policy (the profile itself is not); the borrow must find it, not be refused.
    for kind in [DbmsKind::Postgres, DbmsKind::Sqlite] {
        let profile = match kind {
            DbmsKind::Sqlite => Profile {
                environment: Some(EnvTag::Production),
                ..sqlite_profile()
            },
            _ => Profile {
                single_session: true,
                ..tagged(EnvTag::Production)
            },
        };
        assert!(!profile.read_only);
        let (sm, f) = sessions_of(FakeDriver::new(kind), profile);
        let borrowed = sm.pooled(SessionKind::Service).await.unwrap();
        assert!(snap(&borrowed).read_only, "{kind:?}");
        let again = sm
            .pooled(SessionKind::Introspection { database: None })
            .await
            .unwrap();
        assert_eq!(again.id(), borrowed.id(), "{kind:?}");
        let console = sm
            .open(spec(SessionKind::Console(ClientId::DEFAULT)), ix_none())
            .await
            .unwrap();
        assert_eq!(console.id(), borrowed.id(), "{kind:?}");
        assert_eq!(f.connect_calls(), 1, "{kind:?}");
    }
}
