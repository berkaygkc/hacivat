//! M3b Task 23 (named exceptions in the session crate): what the cancel-escalation UI needs
//! from the session.
//!
//! - A terminate the server refuses is told with the server's own error
//!   (`SessionEvent::CancelFailed`), queued before the escalation's `Failed` reply, so the
//!   dialog shows the text verbatim.
//! - A time zone a server refused stops being "why the profile is down" once the profile
//!   connects again, or once its startup script fails instead (carry T22 I4): the capsule then
//!   offers Edit connection, not the zone line.
//!
//! FakeDriver and a paused clock: no socket, no real time; every wait is on a condition.

mod common;

use std::sync::Arc;

use common::*;
use hacivat_driver::fake::{FakeDriver, FakeExec};
use hacivat_driver::{RawError, RawErrorKind, ServerMessage, TxStatus};
use hacivat_proto::{DbmsKind, ErrClass};
use hacivat_session::pipeline::Interaction;
use hacivat_session::profile::Profile;
use hacivat_session::session::{CancelOutcome, Escalation, SessionEvent};
use hacivat_session::{ConnectionManager, LiveState, ManagerConfig};
use hacivat_settings::TimeZonePref;

/// The server's refusal of a terminate (PostgreSQL's words for a superuser's backend).
fn refusal() -> ServerMessage {
    ServerMessage {
        sqlstate: Some("42501".into()),
        vendor_code: None,
        severity: Some("ERROR".into()),
        text: "must be a superuser to terminate superuser process".into(),
        detail: None,
        hint: None,
        lang_hint: None,
        position: None,
    }
}

#[tokio::test(start_paused = true)]
async fn a_refused_terminate_tells_the_server_text_before_its_reply() {
    let side = Arc::new(FakeDriver::new(DbmsKind::Postgres).exec(vec![FakeExec {
        prefix: "SELECT pg_terminate_backend",
        result: Err(RawError {
            kind: RawErrorKind::Db,
            server: Some(refusal()),
        }),
        delay: std::time::Duration::ZERO,
        tx_after: None,
        panics: false,
    }]));
    let (h, f) = session_with_side(
        fake_pg()
            .backend(7)
            .ignores_cancel()
            .exec(vec![slow("select x", secs(60))]),
        &side,
    )
    .await;
    let a = h.submit(exec("select x"));
    let mut events = h.events();
    settle().await;
    let e = h.escalate(a.seq, Escalation::Terminate);
    let outcome = e.await.expect("the escalation replies");
    assert!(matches!(outcome, CancelOutcome::Failed(_)), "{outcome:?}");
    assert_eq!(side.executed(), ["SELECT pg_terminate_backend(7)"]);
    // Already queued when the reply came: no wait.
    let mut told = None;
    while let Ok(ev) = events.try_recv() {
        if let SessionEvent::CancelFailed { req, server } = ev {
            told = Some((req, server));
        }
    }
    assert_eq!(told, Some((a.seq, Some(refusal()))));
    // The statement still runs; nothing more was sent on the side.
    f.kill_connections();
    let _ = a.reply.await;
}

/// A manager whose sessions send `zone`, over `fake`.
fn manager_zone(fake: FakeDriver, zone: &str) -> (Arc<ConnectionManager>, Arc<FakeDriver>) {
    manager_with(
        fake,
        ManagerConfig {
            time_zone: TimeZonePref::Iana(zone.into()),
            ..ManagerConfig::default()
        },
    )
}

/// Waits (letting the paused clock run) until `done`; panics after [`WAIT`] of real time.
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

/// Connects `profile` and waits for its config loss (the first `SET TIME ZONE` is refused).
async fn refused_once(m: &ConnectionManager, profile: &Arc<Profile>) {
    m.connect(profile.clone(), None, Interaction::Headless);
    until("the refusal", || {
        matches!(m.state(profile.id), LiveState::Lost { .. })
    })
    .await;
    assert_eq!(
        m.refused_time_zone(profile.id).as_deref(),
        Some("Mars/Olympus")
    );
}

#[tokio::test(start_paused = true)]
async fn a_refused_zone_is_forgotten_once_the_profile_connects() {
    let fake = fake_pg().exec(vec![tx_err("SET TIME ZONE", "22023", TxStatus::Idle)]);
    let (m, _f) = manager_zone(fake, "Mars/Olympus");
    let profile = p();
    refused_once(&m, &profile).await;
    m.connect(profile.clone(), None, Interaction::Headless);
    wait_connected(&m, profile.id).await;
    assert_eq!(m.refused_time_zone(profile.id), None);
}

#[tokio::test(start_paused = true)]
async fn a_failed_startup_script_replaces_the_refused_zone_as_the_cause() {
    // The zone is refused first; on the next connect (no zone sent) the script fails.
    let fake = fake_pg().exec(vec![
        tx_err("SET TIME ZONE", "22023", TxStatus::Idle),
        tx_err("SET search_path TO app", "3F000", TxStatus::Idle),
    ]);
    let (m, f) = manager_zone(fake, "Mars/Olympus");
    let profile = Arc::new(Profile {
        startup_script: Some("SET search_path TO app".to_string()),
        ..(*p()).clone()
    });
    refused_once(&m, &profile).await;
    m.connect(profile.clone(), None, Interaction::Headless);
    until("the script's failure", || {
        f.executed().iter().any(|s| s == "SET search_path TO app")
            && matches!(m.state(profile.id), LiveState::Lost { .. })
    })
    .await;
    assert_eq!(
        m.state(profile.id),
        LiveState::Lost {
            class: ErrClass::ConfigInvalid
        }
    );
    assert_eq!(m.refused_time_zone(profile.id), None);
}
