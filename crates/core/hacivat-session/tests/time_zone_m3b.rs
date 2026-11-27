//! M3b Task 22 (named exception in Task 12's session files; T12 concern 2, carry r1-m2): the
//! settings' time zone is scoped per profile, and the app can change it at runtime.
//!
//! - A zone one server refuses stops reaching that profile's sessions only; another profile's
//!   server still gets it.
//! - `ConnectionManager::set_time_zone` reaches every session opened afterwards (a new connect),
//!   and `System` sends nothing again.
//!
//! FakeDriver and a paused clock: no socket, no real time; every wait is on a condition.

mod common;

use std::sync::Arc;

use common::*;
use hacivat_driver::TxStatus;
use hacivat_driver::fake::FakeDriver;
use hacivat_proto::{ErrClass, ProfileId};
use hacivat_session::pipeline::Interaction;
use hacivat_session::profile::Profile;
use hacivat_session::{ConnectionManager, LiveState, ManagerConfig};
use hacivat_settings::TimeZonePref;

const ISTANBUL: &str = "SET TIME ZONE 'Europe/Istanbul'";

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

fn count(f: &FakeDriver, sql: &str) -> usize {
    f.executed().iter().filter(|s| s.as_str() == sql).count()
}

fn manager_zone(fake: FakeDriver, zone: TimeZonePref) -> (Arc<ConnectionManager>, Arc<FakeDriver>) {
    manager_with(
        fake,
        ManagerConfig {
            time_zone: zone,
            ..ManagerConfig::default()
        },
    )
}

fn another_profile() -> Arc<Profile> {
    Arc::new(Profile {
        id: ProfileId::new(),
        name: "orders-stg".into(),
        ..(*p()).clone()
    })
}

#[tokio::test(start_paused = true)]
async fn a_refused_zone_stays_with_the_profile_whose_server_refused_it() {
    let refused = "SET TIME ZONE 'Mars/Olympus'";
    // The first `SET TIME ZONE` is refused (one entry, consumed once); later ones succeed.
    let fake = fake_pg().exec(vec![tx_err("SET TIME ZONE", "22023", TxStatus::Idle)]);
    let (m, f) = manager_zone(fake, TimeZonePref::Iana("Mars/Olympus".into()));
    let first = p();
    m.connect(first.clone(), None, Interaction::Headless);
    until("the refusal", || {
        matches!(m.state(first.id), LiveState::Lost { .. })
    })
    .await;
    assert_eq!(
        m.state(first.id),
        LiveState::Lost {
            class: ErrClass::ConfigInvalid
        }
    );
    assert_eq!(count(&f, refused), 1);
    // T22 fix round 1 (I4): the UI can say which zone was refused, for that profile only.
    assert_eq!(
        m.refused_time_zone(first.id).as_deref(),
        Some("Mars/Olympus")
    );
    // Another profile's server still gets the zone (before: shared manager-wide, it did not).
    let second = another_profile();
    m.connect(second.clone(), None, Interaction::Headless);
    wait_connected(&m, second.id).await;
    assert_eq!(count(&f, refused), 2);
    assert_eq!(m.refused_time_zone(second.id), None);
    // The profile that refused it connects again without it.
    m.connect(first.clone(), None, Interaction::Headless);
    wait_connected(&m, first.id).await;
    assert_eq!(count(&f, refused), 2);
}

#[tokio::test(start_paused = true)]
async fn the_runtime_setter_reaches_sessions_opened_afterwards() {
    let (m, f) = manager_zone(fake_pg(), TimeZonePref::System);
    let profile = p();
    m.connect(profile.clone(), None, Interaction::Headless);
    wait_connected(&m, profile.id).await;
    assert!(f.executed().iter().all(|s| !s.contains("TIME ZONE")));

    m.set_time_zone(TimeZonePref::Iana("Europe/Istanbul".into()));
    // A session already open keeps the zone it connected with.
    assert_eq!(count(&f, ISTANBUL), 0);
    m.disconnect(profile.id);
    until("the disconnect", || {
        m.state(profile.id) == LiveState::Disconnected
    })
    .await;
    m.connect(profile.clone(), None, Interaction::Headless);
    wait_connected(&m, profile.id).await;
    assert_eq!(count(&f, ISTANBUL), 1);
    // The same zone again changes nothing; `System` sends nothing from the next connect on.
    m.set_time_zone(TimeZonePref::Iana("Europe/Istanbul".into()));
    m.set_time_zone(TimeZonePref::System);
    m.disconnect(profile.id);
    until("the disconnect", || {
        m.state(profile.id) == LiveState::Disconnected
    })
    .await;
    m.connect(profile.clone(), None, Interaction::Headless);
    wait_connected(&m, profile.id).await;
    assert_eq!(count(&f, ISTANBUL), 1);
}

#[tokio::test(start_paused = true)]
async fn the_setter_reaches_the_sessions_of_a_connected_profile() {
    let (m, f) = manager_zone(fake_pg(), TimeZonePref::System);
    let profile = p();
    m.connect(profile.clone(), None, Interaction::Headless);
    wait_connected(&m, profile.id).await;
    m.set_time_zone(TimeZonePref::Iana("Europe/Istanbul".into()));
    // A console session of the connected profile, opened after the change, sends it.
    let sessions = m.sessions(profile.id).expect("its sessions");
    let console = sessions
        .open(
            hacivat_session::session::SessionSpec {
                kind: hacivat_session::session::SessionKind::Console(
                    hacivat_session::session::ClientId(2),
                ),
                read_only: false,
                department: hacivat_proto::Department::Console,
            },
            Interaction::Headless,
        )
        .await
        .expect("a console session");
    run(&console, "SELECT 1").await.unwrap();
    assert_eq!(count(&f, ISTANBUL), 1);
}
