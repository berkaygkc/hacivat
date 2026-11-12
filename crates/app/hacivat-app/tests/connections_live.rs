//! M2a Task 18: the live mirror, the capsule and the status bar from one source, Escape to
//! cancel, Retry, delete and edit while connecting, and the bounded quit (P15, P39). Every
//! connect runs through `FakeDriver` (dialing itself, so no socket; hosts are IP literals).

mod support;

use std::time::{Duration, Instant};

use gpui::{TestAppContext, VisualTestContext};
use hacivat_app::connections::ConnectionsModel;
use hacivat_driver::fake::FakeDriver;
use hacivat_driver::{RawError, RawErrorKind};
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_secrets::{AuthProviderId, Retention, SecretKey, SecretSlot, SecretString};
use hacivat_session::LiveState;
use hacivat_session::profile::{Profile, ProfileDraft};
use hacivat_ui::capsule::{CapsuleModel, EnvKind};
use hacivat_ui_core::named_rules::assert_one_filled_badge;
use hacivat_ui_core::theme::Theme;
use support::*;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn ok_pg() -> FakeDriver {
    FakeDriver::new(DbmsKind::Postgres)
}

fn refused() -> RawError {
    RawError {
        kind: RawErrorKind::Io(std::io::ErrorKind::ConnectionRefused),
        server: None,
    }
}

/// A shell over `fake` with one profile `a` (dev, loopback).
fn with_fake(cx: &mut TestAppContext, fake: FakeDriver) -> &'static mut VisualTestContext {
    let vcx = shell_with_fake(cx, "en", fake);
    add_profile(vcx, test_profile("a", Some(EnvTag::Dev), "127.0.0.1"));
    vcx
}

/// Waits (in real time, core work runs on Tokio) until `name` is no longer connecting.
fn connect_and_settle(vcx: &VisualTestContext, name: &str) {
    connect(vcx, name);
    let deadline = Instant::now() + Duration::from_secs(5);
    while matches!(live(vcx, name), LiveState::Connecting(_)) {
        assert!(Instant::now() < deadline, "the connect never finished");
        settle(vcx, ms(10));
    }
}

fn connect_and_wait(vcx: &VisualTestContext, name: &str) {
    connect_and_settle(vcx, name);
    assert!(
        matches!(live(vcx, name), LiveState::Connected(_)),
        "{:?}",
        live(vcx, name)
    );
}

fn quit(vcx: &VisualTestContext) {
    vcx.executor().allow_parking();
    vcx.cx.update(|cx| cx.shutdown());
}

fn session_secret(vcx: &VisualTestContext, name: &str) -> Option<SecretString> {
    let id = profile_id(vcx, name);
    vcx.cx.read(|cx| {
        cx.global::<ConnectionsModel>()
            .secrets()
            .get(
                &SecretKey {
                    profile: id,
                    slot: SecretSlot::DbPassword,
                },
                Retention::ForSession,
            )
            .expect("the memory store answers")
    })
}

fn profile(vcx: &VisualTestContext, name: &str) -> Profile {
    let id = profile_id(vcx, name);
    vcx.cx
        .read(|cx| (*cx.global::<ConnectionsModel>().get(id).expect("saved")).clone())
}

fn save_edit(vcx: &VisualTestContext, name: &str, change: impl FnOnce(&mut ProfileDraft)) {
    let mut draft = ProfileDraft::edit(profile(vcx, name));
    change(&mut draft);
    save_draft(vcx, draft);
}

fn popover_text(vcx: &VisualTestContext) -> String {
    texts_drawn_by(vcx, "hacivat-ui/src/explain.rs").join(" ")
}

#[gpui::test]
fn model_mirrors_manager_states(cx: &mut TestAppContext) {
    let vcx = with_fake(cx, ok_pg().delay(ms(200)));
    connect(vcx, "a");
    settle(vcx, ms(50));
    assert!(matches!(live(vcx, "a"), LiveState::Connecting(_)));
    assert!(matches!(
        capsule_model(vcx),
        CapsuleModel::Connecting { .. }
    ));
    settle(vcx, ms(300));
    assert!(matches!(live(vcx, "a"), LiveState::Connected(_)));
}

#[gpui::test]
fn the_connecting_clock_reads_the_executor_clock(cx: &mut TestAppContext) {
    let vcx = with_fake(cx, ok_pg().delay(Duration::from_secs(5)));
    connect(vcx, "a");
    settle(vcx, ms(20));
    vcx.executor().advance_clock(ms(1250));
    vcx.run_until_parked();
    let CapsuleModel::Connecting { clock, label, .. } = capsule_model(vcx) else {
        panic!("connecting");
    };
    assert_eq!(label.as_ref(), "Connecting");
    assert_eq!(strip_marks(&clock), "1.25 s");
}

fn strip_marks(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

#[gpui::test]
fn capsule_and_status_follow_the_current_connection(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", ok_pg()); // the fake server offers no TLS
    add_profile(
        vcx,
        test_profile("orders-prod", Some(EnvTag::Production), "10.0.4.12"),
    );
    connect_and_wait(vcx, "orders-prod");
    let m = capsule_model(vcx);
    assert!(
        matches!(m, CapsuleModel::Connection { env: EnvKind::Production, ref unencrypted, .. } if unencrypted.as_deref() == Some("unencrypted")),
        "{m:?}"
    );
    let cp_fill = vcx
        .cx
        .read(|cx| Theme::get(cx).tokens.colors.cp_fill.to_hsla());
    vcx.update(|window, _| assert_one_filled_badge(window, cp_fill, true));
    let status = status_bar_text(vcx);
    assert!(status.contains("production · read-write"), "{status}");
    assert!(status.contains("unencrypted"), "{status}");
    assert!(!status.contains("No connection"), "{status}");
}

#[gpui::test]
fn escape_cancels_connect_and_toasts(cx: &mut TestAppContext) {
    let vcx = with_fake(cx, ok_pg().delay(Duration::from_secs(5)));
    connect(vcx, "a");
    vcx.simulate_keystrokes("escape");
    settle(vcx, ms(50));
    assert_eq!(live(vcx, "a"), LiveState::Disconnected);
    assert_eq!(toast_texts(vcx), ["Connecting to a was cancelled."]);
    // Nothing is connecting any more: Escape is not consumed again.
    vcx.simulate_keystrokes("escape");
    assert_eq!(toast_texts(vcx).len(), 1);
}

#[gpui::test]
fn failed_capsule_retry_reconnects(cx: &mut TestAppContext) {
    let vcx = with_fake(cx, ok_pg().script(vec![Err(refused()), Ok(())]));
    connect_and_settle(vcx, "a");
    assert!(matches!(capsule_model(vcx), CapsuleModel::Failed { .. }));
    assert!(status_bar_text(vcx).contains("Couldn't connect"));
    click(vcx, "capsule-retry");
    settle(vcx, ms(50));
    let deadline = Instant::now() + Duration::from_secs(5);
    while matches!(live(vcx, "a"), LiveState::Connecting(_)) && Instant::now() < deadline {
        settle(vcx, ms(10));
    }
    assert!(matches!(live(vcx, "a"), LiveState::Connected(_)));
}

#[gpui::test]
fn retry_has_a_keyboard_path(cx: &mut TestAppContext) {
    let vcx = with_fake(cx, ok_pg().script(vec![Err(refused()), Ok(())]));
    assert!(!run(vcx, "connection.retry"), "nothing failed yet");
    connect_and_settle(vcx, "a");
    assert!(run(vcx, "connection.retry"));
    connect_and_settle_after_retry(vcx);
    assert!(matches!(live(vcx, "a"), LiveState::Connected(_)));
}

fn connect_and_settle_after_retry(vcx: &VisualTestContext) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        settle(vcx, ms(10));
        if !matches!(live(vcx, "a"), LiveState::Connecting(_)) || Instant::now() > deadline {
            break;
        }
    }
}

#[gpui::test]
fn delete_while_connecting_cancels_and_forgets_secrets(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", ok_pg().delay(Duration::from_secs(5)));
    let mut p = test_profile("a", Some(EnvTag::Dev), "127.0.0.1");
    p.auth = AuthProviderId::Password;
    p.retention = Retention::ForSession;
    save_draft(
        vcx,
        ProfileDraft {
            password: Some(SecretString::from("hv_test_pw".to_string())),
            is_new: true,
            ..ProfileDraft::edit(p)
        },
    );
    settle(vcx, ms(50));
    assert!(session_secret(vcx, "a").is_some());
    let id = profile_id(vcx, "a");
    connect(vcx, "a");
    settle(vcx, ms(20));
    vcx.cx
        .update(|cx| ConnectionsModel::delete(cx, id).detach());
    settle(vcx, ms(50));
    let (state, secret) = vcx.cx.read(|cx| {
        let m = cx.global::<ConnectionsModel>();
        let secret = m
            .secrets()
            .get(
                &SecretKey {
                    profile: id,
                    slot: SecretSlot::DbPassword,
                },
                Retention::ForSession,
            )
            .expect("the memory store answers");
        (m.manager().state(id), secret)
    });
    assert_eq!(state, LiveState::Disconnected);
    assert!(secret.is_none());
}

#[gpui::test]
fn unencrypted_segment_opens_explanation(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", ok_pg());
    add_profile(vcx, test_profile("a", Some(EnvTag::Dev), "10.0.4.12"));
    connect_and_wait(vcx, "a");
    click(vcx, "capsule-unencrypted");
    assert_eq!(layer_ids(vcx), ["popover"]);
    // The widget records its wrapped paragraphs before its title label, so test for both.
    assert!(
        popover_text(vcx).contains("Connected without encryption"),
        "{}",
        popover_text(vcx)
    );
    assert!(popover_text(vcx).contains("10.0.4.12 doesn't offer TLS."));
}

#[gpui::test]
fn the_explanation_has_a_keyboard_path(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", ok_pg());
    add_profile(vcx, test_profile("a", Some(EnvTag::Dev), "10.0.4.12"));
    connect_and_wait(vcx, "a");
    assert!(run(vcx, "connection.explain_unencrypted"));
    assert_eq!(layer_ids(vcx), ["popover"]);
}

#[gpui::test]
fn quit_closes_every_connection(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", ok_pg());
    add_profile(vcx, test_profile("a", Some(EnvTag::Dev), "127.0.0.1"));
    add_profile(vcx, test_profile("b", Some(EnvTag::Dev), "127.0.0.1"));
    connect_and_wait(vcx, "a");
    connect_and_wait(vcx, "b");
    assert_eq!(fake(vcx).close_calls(), 0);
    quit(vcx);
    assert_eq!(fake(vcx).close_calls(), 2);
}

/// RF5 (P39) at quit: the connect in flight is cancelled and the quit stays within its budget.
/// The core runtime stops at the end of the quit (P15), which drops the attempt before its
/// handshake completes, so no connection is ever opened, and none is left open.
#[gpui::test]
fn quit_during_connect_cancels_within_budget(cx: &mut TestAppContext) {
    let vcx = with_fake(cx, ok_pg().delay(ms(300)));
    connect(vcx, "a");
    settle(vcx, ms(20));
    assert_eq!(fake(vcx).connect_calls(), 1);
    let started = Instant::now();
    quit(vcx);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    let id = profile_id(vcx, "a");
    let state = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().manager().state(id));
    assert_eq!(state, LiveState::Disconnected);
    std::thread::sleep(ms(500));
    assert_eq!(fake(vcx).close_calls(), 0);
}

#[gpui::test]
fn quit_drains_queued_profile_writes(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", ok_pg());
    add_profile(vcx, test_profile("drained", Some(EnvTag::Dev), "127.0.0.1"));
    quit(vcx);
    let text = std::fs::read_to_string(paths(vcx).connections_toml()).expect("written");
    assert!(text.contains("drained"), "{text}");
}

#[gpui::test]
fn editing_a_connecting_profile_cancels_then_saves(cx: &mut TestAppContext) {
    let vcx = with_fake(cx, ok_pg().delay(Duration::from_secs(5)));
    connect(vcx, "a");
    settle(vcx, ms(20));
    save_edit(vcx, "a", |d| d.profile.database = Some("orders".into()));
    settle(vcx, ms(50));
    assert_eq!(live(vcx, "a"), LiveState::Disconnected);
    assert_eq!(profile(vcx, "a").database.as_deref(), Some("orders"));
}
