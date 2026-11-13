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
#[cfg(feature = "scene-check")]
use hacivat_ui_core::named_rules::assert_one_filled_badge;
#[cfg(feature = "scene-check")]
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
    #[cfg(feature = "scene-check")]
    {
        let cp_fill = vcx
            .cx
            .read(|cx| Theme::get(cx).tokens.colors.cp_fill.to_hsla());
        vcx.update(|window, _| assert_one_filled_badge(window, cp_fill, true));
    }
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
    assert_eq!(fake(vcx).open_connections(), 2);
    quit(vcx);
    assert_eq!(fake(vcx).close_calls(), 2);
    assert_eq!(fake(vcx).open_connections(), 0);
}

/// RF5 (P39) at quit: the connect in flight is cancelled and the quit stays within its budget.
/// The core runtime stops at the end of the quit (P15) and drops the attempt task before the
/// delayed handshake lands, so the late connection is abandoned (never opened, nothing to
/// close) rather than closed: the fake's open-connection count goes back to 0 either way.
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
    assert_eq!(fake(vcx).open_connections(), 0);
}

/// A memory backend whose `put` takes 300 ms (a slow keychain): longer than the core
/// runtime's own 200 ms shutdown budget, so only `drain_writes` waits for it.
struct SlowPut(hacivat_secrets::MemoryBackend);

impl hacivat_secrets::SecretBackend for SlowPut {
    fn get(&self, key: &SecretKey) -> Result<Option<SecretString>, hacivat_secrets::SecretError> {
        self.0.get(key)
    }
    fn put(
        &self,
        key: &SecretKey,
        value: &SecretString,
    ) -> Result<(), hacivat_secrets::SecretError> {
        std::thread::sleep(ms(300));
        self.0.put(key, value)
    }
    fn delete(&self, key: &SecretKey) -> Result<(), hacivat_secrets::SecretError> {
        self.0.delete(key)
    }
    fn name(&self) -> &'static str {
        "slow-put"
    }
}

/// P15: a Forever password queued just before quit reaches the keyring, because the quit
/// drains the writer's queue (bounded) before the runtime stops. Without `drain_writes` the
/// runtime's 200 ms budget ends first and the entry is missing.
#[gpui::test]
fn quit_drains_queued_profile_writes(cx: &mut TestAppContext) {
    let backend = std::sync::Arc::new(SlowPut(hacivat_secrets::MemoryBackend::new()));
    let vcx = shell_with_fake_backend(cx, "en", ok_pg(), backend.clone(), |_| {});
    let mut p = test_profile("drained", Some(EnvTag::Dev), "127.0.0.1");
    p.auth = AuthProviderId::Password;
    p.retention = Retention::Forever;
    let id = save_draft(
        vcx,
        ProfileDraft {
            password: Some(SecretString::from("hv_test_pw".to_string())),
            is_new: true,
            ..ProfileDraft::edit(p)
        },
    );
    let key = SecretKey {
        profile: id,
        slot: SecretSlot::DbPassword,
    };
    use hacivat_secrets::SecretBackend as _;
    assert!(
        backend.get(&key).unwrap().is_none(),
        "the put is still queued"
    );
    quit(vcx);
    assert!(backend.get(&key).unwrap().is_some());
    let text = std::fs::read_to_string(paths(vcx).connections_toml()).expect("written");
    assert!(text.contains("drained"), "{text}");
}

#[gpui::test]
fn a_failed_production_capsule_keeps_production_first(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", ok_pg().connect_result(Err(refused())));
    add_profile(
        vcx,
        test_profile("orders-prod", Some(EnvTag::Production), "10.0.4.12"),
    );
    connect_and_settle(vcx, "orders-prod");
    assert!(matches!(
        capsule_model(vcx),
        CapsuleModel::Failed {
            env: EnvKind::Production,
            ..
        }
    ));
    let fill = vcx
        .debug_bounds("cp-fill")
        .expect("the PRODUCTION block is drawn");
    let body = vcx.debug_bounds("capsule-body").expect("the name is drawn");
    assert!(fill.right() <= body.left(), "{fill:?} then {body:?}");
    let texts = texts_drawn_by(vcx, "hacivat-ui/src/capsule.rs");
    assert_eq!(
        texts.first().map(String::as_str),
        Some("PRODUCTION"),
        "{texts:?}"
    );
}

#[gpui::test]
fn the_body_of_a_lost_connection_reconnects(cx: &mut TestAppContext) {
    let vcx = with_fake(cx, ok_pg());
    let id = profile_id(vcx, "a");
    vcx.cx.update(|cx| {
        let m = cx.global_mut::<ConnectionsModel>();
        m.set_live(
            id,
            LiveState::Lost {
                class: hacivat_proto::ErrClass::NetworkReset,
            },
        );
        m.note_attempt(id);
    });
    vcx.run_until_parked();
    assert!(matches!(capsule_model(vcx), CapsuleModel::Failed { .. }));
    click(vcx, "capsule-body");
    connect_and_settle_after_retry(vcx);
    assert!(matches!(live(vcx, "a"), LiveState::Connected(_)));
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

// ## m2a-finalfix
/// I3: a chain checked without the host name (`VerifyCa`) is "CA verified" on every surface
/// that names the encryption (the status bar and the sheet share `encryption_label`), never
/// plain "verified".
#[gpui::test]
fn ca_only_verification_is_not_called_verified(cx: &mut TestAppContext) {
    use hacivat_net::{TlsSummary, TlsVerified};
    let vcx = with_fake(
        cx,
        ok_pg().tls(TlsSummary {
            encrypted: true,
            version: Some("TLSv1.3".into()),
            cipher: None,
            verified: TlsVerified::CaOnly,
        }),
    );
    connect_and_wait(vcx, "a");
    let status = strip_marks(&status_bar_text(vcx));
    assert!(status.contains("TLS 1.3 · CA verified"), "{status}");
    let LiveState::Connected(info) = live(vcx, "a") else {
        panic!("connected");
    };
    let p = profile(vcx, "a");
    let sheet = vcx
        .cx
        .read(|cx| hacivat_app::connections::capsule::encryption_label(cx, &p, &info).to_string());
    assert_eq!(strip_marks(&sheet), "TLS 1.3 · CA verified");
}
