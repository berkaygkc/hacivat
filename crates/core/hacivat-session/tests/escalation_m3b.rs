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

// ## m3b-t23-escalation (fix round 1)

/// A side statement scripted on `prefix` that fails with `text` after `delay`.
fn side_refusal(prefix: &'static str, text: &str, delay: std::time::Duration) -> FakeExec {
    FakeExec {
        prefix,
        result: Err(RawError {
            kind: RawErrorKind::Db,
            server: Some(ServerMessage {
                text: text.into(),
                ..refusal()
            }),
        }),
        delay,
        tx_after: None,
        panics: false,
    }
}

/// Review I1: a cancel that failed never answers a later terminate. The terminate's reply and
/// its `CancelFailed` are its own, even when its side connection is slower than the 2 s
/// answer.
#[tokio::test(start_paused = true)]
async fn a_failed_cancel_never_answers_the_terminate() {
    let side = Arc::new(FakeDriver::new(DbmsKind::MySql).exec(vec![
        side_refusal(
            "KILL QUERY",
            "kill query refused",
            std::time::Duration::ZERO,
        ),
        side_refusal("KILL CONNECTION", "kill connection refused", secs(3)),
    ]));
    let (h, f) = session_with_side(
        fake_mysql()
            .backend(42)
            .exec(vec![slow("select sleep(60)", secs(60))]),
        &side,
    )
    .await;
    let a = h.submit(exec("select sleep(60)"));
    settle().await;
    let cancelled = h.cancel(a.seq).await.expect("the cancel replies");
    assert!(
        matches!(cancelled, CancelOutcome::Failed(_)),
        "{cancelled:?}"
    );
    let mut events = h.events();
    let e = h.escalate(a.seq, Escalation::Terminate);
    // The terminate is still on its way at the 2 s answer: not the cancel's old failure.
    assert_eq!(
        e.await.expect("the escalation replies"),
        CancelOutcome::StillRunning
    );
    // Its own refusal comes later, with its own text.
    let told = loop {
        if let SessionEvent::CancelFailed { req, server } = next(&mut events).await {
            break (req, server.map(|s| s.text));
        }
    };
    assert_eq!(told, (a.seq, Some("kill connection refused".to_string())));
    assert_eq!(side.executed(), ["KILL QUERY 42", "KILL CONNECTION 42"]);
    f.kill_connections();
    let _ = a.reply.await;
}

/// Review I1: after a failed cancel, a terminate that ends the request reports `Cancelled`.
#[tokio::test(start_paused = true)]
async fn a_terminate_after_a_failed_cancel_reports_its_own_success() {
    let side = Arc::new(FakeDriver::new(DbmsKind::MySql).exec(vec![side_refusal(
        "KILL QUERY",
        "kill query refused",
        std::time::Duration::ZERO,
    )]));
    let (h, f) = session_with_side(
        fake_mysql()
            .backend(42)
            .exec(vec![slow("select sleep(60)", secs(60))]),
        &side,
    )
    .await;
    let a = h.submit(exec("select sleep(60)"));
    settle().await;
    let cancelled = h.cancel(a.seq).await.expect("the cancel replies");
    assert!(
        matches!(cancelled, CancelOutcome::Failed(_)),
        "{cancelled:?}"
    );
    let e = h.escalate(a.seq, Escalation::Terminate);
    settle_until("the terminate on the side connection", || {
        side.executed() == ["KILL QUERY 42", "KILL CONNECTION 42"]
    })
    .await;
    // What the terminate does to the server connection.
    f.kill_connections();
    // Its own answer: done, or still on its way at the 2 s mark, never the cancel's failure.
    let outcome = e.await.expect("the escalation replies");
    assert!(
        matches!(
            outcome,
            CancelOutcome::Cancelled | CancelOutcome::StillRunning
        ),
        "{outcome:?}"
    );
    assert!(a.reply.await.expect("the request ends").is_err());
}

/// Review I4: a startup script refused before anything is sent is the cause now, not a zone
/// refused on an earlier connect.
#[tokio::test(start_paused = true)]
async fn a_refused_startup_script_replaces_the_refused_zone_as_the_cause() {
    let fake = fake_pg().exec(vec![tx_err("SET TIME ZONE", "22023", TxStatus::Idle)]);
    let (m, _f) = manager_zone(fake, "Mars/Olympus");
    let profile = p();
    refused_once(&m, &profile).await;
    // A psql client command: refused before any of it is sent.
    let scripted = Arc::new(Profile {
        startup_script: Some("\\set x 1".to_string()),
        ..(*profile).clone()
    });
    m.connect(scripted.clone(), None, Interaction::Headless);
    until("the refused script", || {
        matches!(m.state(profile.id), LiveState::Lost { .. })
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

/// Review I4: a startup script past the connect timeout is the cause now.
#[tokio::test(start_paused = true)]
async fn a_startup_script_past_the_timeout_replaces_the_refused_zone_as_the_cause() {
    let fake = fake_pg().exec(vec![
        tx_err("SET TIME ZONE", "22023", TxStatus::Idle),
        slow("SELECT pg_advisory_lock", secs(600)),
    ]);
    let (m, f) = manager_zone(fake, "Mars/Olympus");
    let profile = p();
    refused_once(&m, &profile).await;
    let scripted = Arc::new(Profile {
        startup_script: Some("SELECT pg_advisory_lock(1)".to_string()),
        connect_timeout_s: 5,
        ..(*profile).clone()
    });
    m.connect(scripted.clone(), None, Interaction::Headless);
    until("the script to start", || {
        f.executed()
            .iter()
            .any(|s| s.starts_with("SELECT pg_advisory_lock"))
    })
    .await;
    until("the timeout", || {
        matches!(m.state(profile.id), LiveState::Lost { .. })
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
