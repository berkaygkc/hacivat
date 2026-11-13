//! M2b Task 22: the live-state UI. Reconnecting (with the countdown to the next attempt), the
//! session-lost toast, the idle disconnect (toast, capsule, status bar, explorer), the
//! enforcement-gap alert, the honest read-only labels with the explanation popover (findings,
//! remediation SQL, Copy SQL, keyboard), the SSH path, and the debug cancel demo (Q39).
//!
//! Live states are set on the model directly where the manager's own path is covered by
//! `hacivat-session` (T17, T18); one test drives a real reconnect end to end through
//! `FakeDriver`. Connection events go through the manager's broadcast and the app's relay
//! (`publish_event`, feature `test-support`). Every wait is a condition (`settle_until`,
//! `wait_until`), never a fixed sleep (Q4).

mod support;

use std::sync::Arc;
use std::time::Duration;

use gpui::{TestAppContext, VisualTestContext};
use hacivat_app::connections::{ConnectionsModel, events, session_toasts};
use hacivat_app::l10n_global::tr;
use hacivat_driver::fake::FakeDriver;
#[cfg(debug_assertions)]
use hacivat_driver::{ExecSummary, fake::FakeExec};
use hacivat_driver::{RawError, RawErrorKind, SemVerLoose, ServerInfo, Support};
use hacivat_l10n::{Count, NoArgs, ids};
use hacivat_net::ssh::{HopSpec, SshAuth, SshSpec};
use hacivat_net::{AddressClass, TlsSummary, TlsVerified};
use hacivat_proto::{AccessLabel, DbmsKind, EnvTag, ProfileId};
use hacivat_secrets::AuthProviderId;
use hacivat_session::pipeline::{Progress, TrailStep};
use hacivat_session::profile::{Profile, TransportSpec};
use hacivat_session::session::LostState;
use hacivat_session::{ConnectionEvent, GapReason, LiveInfo, LiveState, PathKind, RoleWarning};
use hacivat_ui::capsule::CapsuleModel;
use hacivat_ui_core::gpui_surface::Role;
#[cfg(feature = "scene-check")]
use hacivat_ui_core::named_rules::assert_one_filled_badge;
#[cfg(feature = "scene-check")]
use hacivat_ui_core::theme::Theme;
use support::*;

// ---- fixtures ---------------------------------------------------------------------------------

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// `s` without Fluent's isolation marks.
fn plain(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

/// The message `msg` in the shell's language, isolation marks stripped.
fn t(vcx: &VisualTestContext, msg: hacivat_l10n::MsgRef<NoArgs>) -> String {
    vcx.cx.read(|cx| plain(&tr(cx, msg, NoArgs)))
}

/// `f` rendered in the shell's language, isolation marks stripped.
fn tf(vcx: &VisualTestContext, f: impl FnOnce(&gpui::App) -> gpui::SharedString) -> String {
    vcx.cx.read(|cx| plain(&f(cx)))
}

/// A shell over a PostgreSQL `FakeDriver` (no socket, IP hosts) with the profile `p` selected.
fn shell_with(
    cx: &mut TestAppContext,
    lang: &str,
    fake: FakeDriver,
    p: Profile,
) -> (&'static mut VisualTestContext, ProfileId) {
    shell_in(cx, lang, None, fake, p)
}

/// [`shell_with`] whose regional format (`ui.regional_format`) is `regional`.
fn shell_in(
    cx: &mut TestAppContext,
    lang: &str,
    regional: Option<&'static str>,
    fake: FakeDriver,
    p: Profile,
) -> (&'static mut VisualTestContext, ProfileId) {
    let vcx = shell_with_fake_prepared(cx, lang, fake, move |paths| {
        if let Some(r) = regional {
            write(
                paths.settings_toml(),
                &format!("[ui]\nregional_format = \"{r}\"\n"),
            );
        }
    });
    let id = add_profile(vcx, p);
    vcx.cx.update(|cx| ConnectionsModel::select(cx, Some(id)));
    vcx.run_until_parked();
    (vcx, id)
}

fn pg(name: &str, env: EnvTag) -> Profile {
    test_profile(name, Some(env), "10.0.4.12")
}

fn ro(mut p: Profile) -> Profile {
    p.read_only = true;
    p
}

fn ssh(mut p: Profile, bastion: &str) -> Profile {
    p.transport = TransportSpec::SshTunnel(SshSpec {
        hops: vec![HopSpec {
            host: bastion.to_string(),
            port: 22,
            user: "deploy".to_string(),
            auth: SshAuth::Agent,
            identities_only: false,
            identity_files: Vec::new(),
        }],
        host_keys: Default::default(),
        keepalive_s: None,
    });
    p
}

/// A live PostgreSQL connection of `major` (e.g. "16.4") with `access` and `warnings`.
fn info(version: &str, access: AccessLabel, warnings: Vec<RoleWarning>) -> LiveInfo {
    LiveInfo {
        server: ServerInfo {
            kind: DbmsKind::Postgres,
            product: "PostgreSQL".to_string(),
            version: SemVerLoose::parse(version),
            raw_version: version.to_string(),
            is_replica: Some(false),
        },
        support: Support::Supported,
        tls: TlsSummary {
            encrypted: true,
            version: Some("TLSv1.3".to_string()),
            cipher: None,
            verified: TlsVerified::Full,
        },
        address: AddressClass::Private,
        latency: Duration::from_millis(12),
        access,
        path: PathKind::Direct,
        user: Some("qa".to_string()),
        auth: AuthProviderId::Password,
        via: None,
        hops: 0,
        access_warnings: warnings,
    }
}

/// Puts `id` in `state` as the capsule's current profile (the mirror's job, done by hand).
fn set_state(vcx: &VisualTestContext, id: ProfileId, state: LiveState) {
    vcx.cx.update(|cx| {
        let m = cx.global_mut::<ConnectionsModel>();
        m.set_live(id, state);
        m.note_attempt(id);
        ConnectionsModel::select(cx, Some(id));
    });
    vcx.run_until_parked();
}

fn with_state(cx: &mut TestAppContext, state: LiveState) -> &'static mut VisualTestContext {
    let (vcx, id) = shell_with(
        cx,
        "en",
        FakeDriver::new(DbmsKind::Postgres),
        pg("orders-staging", EnvTag::Staging),
    );
    set_state(vcx, id, state);
    vcx
}

fn connected_with(
    cx: &mut TestAppContext,
    access: AccessLabel,
    warnings: Vec<RoleWarning>,
) -> (&'static mut VisualTestContext, ProfileId) {
    let (vcx, id) = shell_with(
        cx,
        "en",
        FakeDriver::new(DbmsKind::Postgres),
        ro(pg("orders-qa", EnvTag::Test)),
    );
    set_state(
        vcx,
        id,
        LiveState::Connected(Arc::new(info("16.4", access, warnings))),
    );
    (vcx, id)
}

/// The text of the capsule's `segment` label (`capsule-text-<segment>`).
fn capsule_text(vcx: &VisualTestContext, segment: &str) -> String {
    text_of(vcx, &format!("capsule-text-{segment}"))
        .unwrap_or_else(|| panic!("the capsule draws its `{segment}` text"))
}

/// The status bar's label with ID `status-<segment>`.
fn status_text(vcx: &VisualTestContext, segment: &str) -> String {
    text_of(vcx, &format!("status-{segment}"))
        .unwrap_or_else(|| panic!("the status bar draws its `{segment}` text"))
}

fn manager(vcx: &VisualTestContext) -> Arc<hacivat_session::ConnectionManager> {
    vcx.cx
        .read(|cx| cx.global::<ConnectionsModel>().manager().clone())
}

/// Sends `event` through the manager's broadcast and waits until the app handled it.
fn emit(vcx: &VisualTestContext, event: ConnectionEvent) {
    let m = manager(vcx);
    settle_until(vcx, WAIT, || m.event_subscribers() > 0);
    let before = vcx.cx.read(events::handled_events);
    m.publish_event(event);
    settle_until(vcx, WAIT, || vcx.cx.read(events::handled_events) > before);
}

fn shown(vcx: &VisualTestContext) -> Vec<session_toasts::ShownToast> {
    vcx.run_until_parked();
    vcx.cx.read(session_toasts::shown)
}

fn has_sticky_toast(vcx: &VisualTestContext) -> bool {
    shown(vcx).iter().any(|s| s.sticky)
}

/// The newest visible sticky session toast, isolation marks stripped.
fn sticky_toast(vcx: &VisualTestContext) -> session_toasts::ShownToast {
    let mut toast = shown(vcx)
        .into_iter()
        .rev()
        .find(|s| s.sticky)
        .expect("a sticky session toast");
    toast.title = toast.title.map(|s| plain(&s));
    toast.lines = toast.lines.iter().map(|s| plain(s)).collect();
    toast
}

/// How the toast whose headline is `headline` is announced, as `toast.rs` declares it to
/// assistive technology in the next frame (fix round 1, M3: never the seam's own rule).
fn announced_role(vcx: &VisualTestContext, headline: &str) -> Option<Role> {
    hacivat_ui::popover::a11y_record();
    vcx.cx.update(|cx| cx.refresh_windows());
    vcx.run_until_parked();
    hacivat_ui::popover::a11y_take()
        .into_iter()
        .find(|d| plain(&d.name) == headline && matches!(d.role, Some(Role::Alert | Role::Status)))
        .and_then(|d| d.role)
}

/// How many visible toasts read as message `id` (drawn without the localization).
fn toasts_with(vcx: &VisualTestContext, id: &str) -> usize {
    toast_ids(vcx).iter().filter(|t| t.as_str() == id).count()
}

fn popover_text(vcx: &VisualTestContext) -> String {
    texts_drawn_by(vcx, "connections/access_popover.rs").join(" ")
}

fn button_visible(vcx: &mut VisualTestContext, id: &str) -> bool {
    vcx.debug_bounds(Box::leak(format!("button-{id}").into_boxed_str()))
        .is_some()
}

fn layer_open(vcx: &VisualTestContext) -> bool {
    layer_ids(vcx).contains(&"popover")
}

fn lost(open_tx: bool, temp: bool, vars: bool) -> LostState {
    LostState {
        open_tx,
        temp_objects: temp,
        session_vars: vars,
        prepared: 0,
        locks: false,
    }
}

// ---- reconnecting -----------------------------------------------------------------------------

#[gpui::test]
fn reconnecting_capsule_shows_attempt_and_countdown(cx: &mut TestAppContext) {
    let vcx = with_state(
        cx,
        LiveState::Reconnecting {
            attempt: 3,
            next_in: Some(secs(4)),
        },
    );
    assert_eq!(
        capsule_text(vcx, "run"),
        tf(vcx, |cx| tr(
            cx,
            ids::hv::CAPSULE_RECONNECTING,
            ids::hv::HvCapsuleReconnecting { n: Count(3) }
        ))
    );
    assert_eq!(capsule_text(vcx, "run"), "Reconnecting · attempt 3");
    assert_eq!(
        capsule_text(vcx, "next"),
        tf(vcx, |cx| tr(
            cx,
            ids::hv::CAPSULE_NEXT,
            ids::hv::HvCapsuleNext { seconds: Count(4) }
        ))
    );
    assert_eq!(capsule_text(vcx, "next"), "in 4 s");
    assert!(vcx.debug_bounds("capsule-run").is_some(), "the run marker");
    // The status bar says the same attempt.
    assert!(
        plain(&status_bar_text(vcx)).contains("Reconnecting · attempt 3"),
        "{}",
        status_bar_text(vcx)
    );
    // While an attempt is under way there is no countdown.
    let id = profile_id(vcx, "orders-staging");
    set_state(
        vcx,
        id,
        LiveState::Reconnecting {
            attempt: 4,
            next_in: None,
        },
    );
    assert_eq!(capsule_text(vcx, "run"), "Reconnecting · attempt 4");
    assert_eq!(text_of(vcx, "capsule-text-next"), None);
}

/// Ruling (Task 22 round 1): a reconnect's countdown repaints once a second; the 50 ms clock
/// runs only while an attempt runs; nothing ticks otherwise.
#[gpui::test]
fn a_reconnect_countdown_repaints_once_a_second(cx: &mut TestAppContext) {
    let vcx = with_state(
        cx,
        LiveState::Reconnecting {
            attempt: 2,
            next_in: Some(secs(4)),
        },
    );
    let interval = || {
        vcx.cx
            .read(hacivat_app::connections::live::repaint_interval)
    };
    assert_eq!(interval(), Some(secs(1)));
    // A wait of 2.5 s repaints when the shown second changes (in 0.5 s), then once a second.
    let id = profile_id(vcx, "orders-staging");
    set_state(
        vcx,
        id,
        LiveState::Reconnecting {
            attempt: 3,
            next_in: Some(Duration::from_millis(2500)),
        },
    );
    assert_eq!(interval(), Some(Duration::from_millis(500)));
    set_state(
        vcx,
        id,
        LiveState::Reconnecting {
            attempt: 3,
            next_in: None,
        },
    );
    assert_eq!(interval(), None, "an attempt under way has no clock");
    let progress = Progress {
        trail: Vec::new(),
        current: None,
        started: std::time::Instant::now(),
    };
    set_state(vcx, id, LiveState::Connecting(progress));
    assert_eq!(interval(), Some(hacivat_app::connections::live::TICK));
    set_state(vcx, id, LiveState::Disconnected);
    assert_eq!(interval(), None);
}

#[gpui::test]
fn one_filled_badge_while_reconnecting_production(cx: &mut TestAppContext) {
    let (vcx, id) = shell_with(
        cx,
        "en",
        FakeDriver::new(DbmsKind::Postgres),
        pg("orders-prod", EnvTag::Production),
    );
    set_state(
        vcx,
        id,
        LiveState::Reconnecting {
            attempt: 1,
            next_in: None,
        },
    );
    assert!(matches!(
        capsule_model(vcx),
        CapsuleModel::Reconnecting { .. }
    ));
    // The production block stays first.
    let fill = vcx.debug_bounds("cp-fill").expect("the PRODUCTION block");
    let body = vcx.debug_bounds("capsule-body").expect("the name");
    assert!(fill.right() <= body.left(), "{fill:?} then {body:?}");
    #[cfg(feature = "scene-check")]
    {
        let cp_fill = vcx
            .cx
            .read(|cx| Theme::get(cx).tokens.colors.cp_fill.to_hsla());
        vcx.update(|window, _| assert_one_filled_badge(window, cp_fill, true));
    }
}

/// A refused connect error (a recovery case, `network.refused`).
fn refused() -> RawError {
    RawError {
        kind: RawErrorKind::Io(std::io::ErrorKind::ConnectionRefused),
        server: None,
    }
}

/// End to end over the M2b manager config (boot's `ManagerConfig::m2b()`): a 1 s keep-alive
/// finds the dead connection, the first reconnect is refused, the capsule counts down to the
/// second, which connects; the session-lost toast says it reconnected and claims no rollback.
#[gpui::test]
fn a_real_reconnect_counts_down_and_ends_with_the_reconnected_toast(cx: &mut TestAppContext) {
    let mut p = pg("orders-dev", EnvTag::Dev);
    p.keepalive_s = Some(1);
    let driver = FakeDriver::new(DbmsKind::Postgres).script(vec![Ok(()), Err(refused()), Ok(())]);
    let (vcx, _) = shell_with(cx, "en", driver, p);
    connect(vcx, "orders-dev");
    wait_until(vcx, "the first connect", |vcx| {
        matches!(live(vcx, "orders-dev"), LiveState::Connected(_))
    });
    fake(vcx).kill_connections();
    wait_until(vcx, "the countdown to attempt 2", |vcx| {
        matches!(
            live(vcx, "orders-dev"),
            LiveState::Reconnecting {
                attempt: 2,
                next_in: Some(_)
            }
        )
    });
    assert_eq!(capsule_text(vcx, "run"), "Reconnecting · attempt 2");
    let next = capsule_text(vcx, "next");
    assert!(next == "in 1 s" || next == "in 2 s", "{next}");
    wait_until(vcx, "the reconnect", |vcx| {
        matches!(live(vcx, "orders-dev"), LiveState::Connected(_))
    });
    settle_until(vcx, WAIT, || has_sticky_toast(vcx));
    let toast = sticky_toast(vcx);
    assert_eq!(toast.title.as_deref(), Some("Reconnected to orders-dev"));
    assert_eq!(
        toast.lines,
        [tf(vcx, |cx| tr(
            cx,
            ids::hv::CONN_LOST_BODY,
            ids::hv::HvConnLostBody {
                state: ids::hv::HvConnLostBodyState::Reconnected
            }
        ))]
    );
    assert_eq!(fake(vcx).connect_calls(), 3);
}

// ---- session lost -----------------------------------------------------------------------------

#[gpui::test]
fn session_lost_toast_lists_lost_items(cx: &mut TestAppContext) {
    let (vcx, id) = shell_with(
        cx,
        "en",
        FakeDriver::new(DbmsKind::Postgres),
        pg("orders-prod", EnvTag::Production),
    );
    emit(
        vcx,
        ConnectionEvent::SessionLost {
            profile: id,
            lost: lost(true, true, true),
        },
    );
    let toast = sticky_toast(vcx);
    assert!(
        toast
            .lines
            .iter()
            .any(|l| l == &t(vcx, ids::hv::CONN_LOST_TX)),
        "{toast:?}"
    );
    assert!(
        toast
            .lines
            .contains(&"Also gone: temporary tables and session variables".to_string()),
        "{toast:?}"
    );
    assert_eq!(toast.actions.len(), 2);
    assert_eq!(toast.actions, ["Show details", "Dismiss"]);
    assert_eq!(
        announced_role(vcx, toast.title.as_deref().expect("a title")),
        Some(Role::Alert)
    );
    // A transaction was open, so nothing reconnected on its own: the title says so.
    assert_eq!(
        toast.title.as_deref(),
        Some("The session on orders-prod was lost")
    );
    // What the model says is what is drawn.
    let drawn: Vec<String> = toast_texts(vcx).iter().map(|s| plain(s)).collect();
    for line in toast.lines.iter().chain(toast.title.iter()) {
        assert!(drawn.contains(line), "{line} in {drawn:?}");
    }
}

#[gpui::test]
fn session_lost_toast_without_losses_shows_body_only(cx: &mut TestAppContext) {
    let (vcx, id) = shell_with(
        cx,
        "en",
        FakeDriver::new(DbmsKind::Postgres),
        pg("orders-prod", EnvTag::Production),
    );
    emit(
        vcx,
        ConnectionEvent::SessionLost {
            profile: id,
            lost: LostState::default(),
        },
    );
    let toast = sticky_toast(vcx);
    assert_eq!(toast.lines.len(), 1, "{toast:?}");
    assert_eq!(toast.title.as_deref(), Some("Reconnected to orders-prod"));
}

/// The dispatch's rule: a reconnected session never claims a rollback.
#[gpui::test]
fn a_reconnected_session_claims_no_rollback(cx: &mut TestAppContext) {
    let (vcx, id) = shell_with(
        cx,
        "en",
        FakeDriver::new(DbmsKind::Postgres),
        pg("orders-prod", EnvTag::Production),
    );
    emit(
        vcx,
        ConnectionEvent::SessionLost {
            profile: id,
            lost: LostState {
                prepared: 2,
                locks: true,
                ..lost(false, true, false)
            },
        },
    );
    let toast = sticky_toast(vcx);
    let rollback = t(vcx, ids::hv::CONN_LOST_TX);
    assert!(!toast.lines.contains(&rollback), "{toast:?}");
    assert!(
        toast.lines.iter().all(|l| !l.contains("rolled back")),
        "{toast:?}"
    );
    assert_eq!(
        toast.lines[1],
        "Also gone: temporary tables, 2 prepared statements, and session locks"
    );
}

#[gpui::test]
fn two_quick_losses_show_one_toast(cx: &mut TestAppContext) {
    let (vcx, id) = shell_with(
        cx,
        "en",
        FakeDriver::new(DbmsKind::Postgres),
        pg("orders-prod", EnvTag::Production),
    );
    for temp in [true, false] {
        emit(
            vcx,
            ConnectionEvent::SessionLost {
                profile: id,
                lost: lost(false, temp, false),
            },
        );
    }
    let lost_toasts: Vec<_> = shown(vcx)
        .into_iter()
        .filter(|s| s.kind == "lost")
        .collect();
    assert_eq!(lost_toasts.len(), 1, "{lost_toasts:?}");
    // The newer one stays: it lost nothing more.
    assert_eq!(lost_toasts[0].lines.len(), 1);
}

#[gpui::test]
fn show_details_selects_the_connection(cx: &mut TestAppContext) {
    let (vcx, id) = shell_with(
        cx,
        "en",
        FakeDriver::new(DbmsKind::Postgres),
        pg("orders-prod", EnvTag::Production),
    );
    let other = add_profile(vcx, pg("other", EnvTag::Dev));
    vcx.cx
        .update(|cx| ConnectionsModel::select(cx, Some(other)));
    emit(
        vcx,
        ConnectionEvent::SessionLost {
            profile: id,
            lost: lost(true, false, false),
        },
    );
    let toast_id = vcx.cx.read(|cx| {
        *hacivat_ui::toast::ToastQueue::visible_ids(cx)
            .last()
            .expect("a toast")
    });
    let selector: &'static str =
        Box::leak(format!("button-toast-action-{toast_id}-0").into_boxed_str());
    click(vcx, selector);
    assert_eq!(
        vcx.cx.read(|cx| cx.global::<ConnectionsModel>().selected()),
        Some(id)
    );
    assert!(shown(vcx).is_empty(), "the action dismisses the toast");
}

#[gpui::test]
fn turkish_lost_list_uses_ve(cx: &mut TestAppContext) {
    // Ruling (Task 22 round 1): the conjunction follows the UI language, even over an English
    // regional format.
    let (vcx, id) = shell_in(
        cx,
        "tr",
        Some("en-US"),
        FakeDriver::new(DbmsKind::Postgres),
        pg("orders-prod", EnvTag::Production),
    );
    emit(
        vcx,
        ConnectionEvent::SessionLost {
            profile: id,
            lost: lost(false, true, true),
        },
    );
    let toast = sticky_toast(vcx);
    assert!(toast.lines.iter().any(|l| l.contains(" ve ")), "{toast:?}");
    assert!(
        toast
            .lines
            .contains(&"Ayrıca kayboldu: geçici tablolar ve oturum değişkenleri".to_string()),
        "{toast:?}"
    );
    // Turkish is longer: nothing on the toast is clipped.
    for r in registry_frame(vcx)
        .iter()
        .filter(|r| r.path.contains("hacivat-ui/src/toast.rs:"))
    {
        assert!(!r.clipped, "{} is clipped", r.text);
    }
}

/// Ruling (Task 22 round 1): an English UI over a Turkish regional format says "and".
#[gpui::test]
fn english_lost_list_over_a_turkish_region_uses_and(cx: &mut TestAppContext) {
    let (vcx, id) = shell_in(
        cx,
        "en",
        Some("tr-TR"),
        FakeDriver::new(DbmsKind::Postgres),
        pg("orders-prod", EnvTag::Production),
    );
    emit(
        vcx,
        ConnectionEvent::SessionLost {
            profile: id,
            lost: lost(false, true, true),
        },
    );
    let toast = sticky_toast(vcx);
    assert!(
        toast
            .lines
            .contains(&"Also gone: temporary tables and session variables".to_string()),
        "{toast:?}"
    );
}

// ---- idle -------------------------------------------------------------------------------------

#[gpui::test]
fn idle_disconnected_toast_once(cx: &mut TestAppContext) {
    let (vcx, id) = shell_with(
        cx,
        "en",
        FakeDriver::new(DbmsKind::Postgres),
        pg("orders-dev", EnvTag::Dev),
    );
    emit(vcx, ConnectionEvent::IdleDisconnected { profile: id });
    emit(vcx, ConnectionEvent::IdleDisconnected { profile: id });
    assert_eq!(toasts_with(vcx, "hv-conn-idle-disconnected"), 1);
    // B18: dev disconnects after 4 h; the toast says so with its unit.
    assert!(
        toast_texts(vcx).iter().any(|s| plain(s)
            == "Disconnected orders-dev after 4 h idle. It reconnects when you use it."),
        "{:?}",
        toast_texts(vcx)
    );
}

#[gpui::test]
fn an_idle_threshold_in_minutes_says_minutes(cx: &mut TestAppContext) {
    let mut p = pg("orders-dev", EnvTag::Dev);
    p.idle_disconnect_s = Some(90 * 60);
    let (vcx, id) = shell_with(cx, "en", FakeDriver::new(DbmsKind::Postgres), p);
    emit(vcx, ConnectionEvent::IdleDisconnected { profile: id });
    assert_eq!(toasts_with(vcx, "hv-conn-idle-disconnected-minutes"), 1);
    assert!(
        toast_texts(vcx)
            .iter()
            .any(|s| plain(s).contains("after 90 min idle")),
        "{:?}",
        toast_texts(vcx)
    );
}

#[gpui::test]
fn an_idle_connection_says_so_everywhere_and_reconnects_from_the_capsule(cx: &mut TestAppContext) {
    let (vcx, id) = shell_with(
        cx,
        "en",
        FakeDriver::new(DbmsKind::Postgres),
        pg("orders-dev", EnvTag::Dev),
    );
    vcx.cx
        .update(|cx| cx.global_mut::<ConnectionsModel>().note_attempt(id));
    emit(vcx, ConnectionEvent::IdleDisconnected { profile: id });
    assert!(matches!(capsule_model(vcx), CapsuleModel::Idle { .. }));
    assert_eq!(capsule_text(vcx, "idle"), "Disconnected (idle)");
    assert!(
        plain(&status_bar_text(vcx)).contains("Disconnected (idle)"),
        "{}",
        status_bar_text(vcx)
    );
    assert!(vcx.debug_bounds("explorer-row-idle-0").is_some());
    // B18: it reconnects when used; the capsule's body does it.
    click(vcx, "capsule-body");
    wait_until(vcx, "the reconnect", |vcx| {
        matches!(live(vcx, "orders-dev"), LiveState::Connected(_))
    });
    assert!(!vcx.cx.read(|cx| session_toasts::is_idle(cx, id)));
    assert!(vcx.debug_bounds("explorer-row-idle-0").is_none());
}

/// An idle production capsule keeps its PRODUCTION block first, and it stays the one filled
/// badge.
#[gpui::test]
fn an_idle_production_capsule_keeps_one_filled_badge_first(cx: &mut TestAppContext) {
    let (vcx, id) = shell_with(
        cx,
        "en",
        FakeDriver::new(DbmsKind::Postgres),
        pg("orders-prod", EnvTag::Production),
    );
    vcx.cx
        .update(|cx| cx.global_mut::<ConnectionsModel>().note_attempt(id));
    emit(vcx, ConnectionEvent::IdleDisconnected { profile: id });
    assert!(matches!(capsule_model(vcx), CapsuleModel::Idle { .. }));
    let fill = vcx.debug_bounds("cp-fill").expect("the PRODUCTION block");
    let idle = vcx.debug_bounds("capsule-idle").expect("the idle segment");
    assert!(fill.right() <= idle.left(), "{fill:?} then {idle:?}");
    assert!(
        toast_texts(vcx)
            .iter()
            .any(|s| plain(s).contains("after 1 h idle")),
        "production disconnects after 1 h (B18): {:?}",
        toast_texts(vcx)
    );
    #[cfg(feature = "scene-check")]
    {
        let cp_fill = vcx
            .cx
            .read(|cx| Theme::get(cx).tokens.colors.cp_fill.to_hsla());
        vcx.update(|window, _| assert_one_filled_badge(window, cp_fill, true));
    }
}

#[gpui::test]
fn a_late_idle_event_after_a_reconnect_is_ignored(cx: &mut TestAppContext) {
    let (vcx, id) = shell_with(
        cx,
        "en",
        FakeDriver::new(DbmsKind::Postgres),
        pg("orders-dev", EnvTag::Dev),
    );
    connect(vcx, "orders-dev");
    wait_until(vcx, "the connect", |vcx| {
        matches!(live(vcx, "orders-dev"), LiveState::Connected(_))
    });
    emit(vcx, ConnectionEvent::IdleDisconnected { profile: id });
    assert!(!vcx.cx.read(|cx| session_toasts::is_idle(cx, id)));
    assert_eq!(toasts_with(vcx, "hv-conn-idle-disconnected"), 0);
}

#[gpui::test]
fn an_idle_transaction_is_a_notice(cx: &mut TestAppContext) {
    let (vcx, id) = shell_with(
        cx,
        "en",
        FakeDriver::new(DbmsKind::Postgres),
        pg("orders-prod", EnvTag::Production),
    );
    emit(
        vcx,
        ConnectionEvent::IdleTransaction {
            profile: id,
            minutes: 60,
        },
    );
    let notices: Vec<_> = shown(vcx)
        .into_iter()
        .filter(|s| s.kind == "idle-tx")
        .collect();
    assert_eq!(notices.len(), 1);
    assert!(!notices[0].sticky);
    assert_eq!(
        announced_role(vcx, &plain(&notices[0].lines[0])),
        Some(Role::Status)
    );
    assert!(plain(&notices[0].lines[0]).contains("open and idle for 60 min"));
}

// ---- enforcement gap --------------------------------------------------------------------------

#[gpui::test]
fn enforcement_gap_sticky_alert(cx: &mut TestAppContext) {
    let (vcx, id) = shell_with(
        cx,
        "en",
        FakeDriver::new(DbmsKind::Postgres),
        ro(pg("orders-prod", EnvTag::Production)),
    );
    emit(
        vcx,
        ConnectionEvent::EnforcementGap {
            profile: id,
            reason: GapReason::TransactionEndedByStatement,
        },
    );
    settle_until(vcx, secs(2), || has_sticky_toast(vcx));
    let title = sticky_toast(vcx).title.expect("a title");
    assert_eq!(announced_role(vcx, &title), Some(Role::Alert));
    assert_eq!(
        sticky_toast(vcx).lines[0],
        t(vcx, ids::hv::CONN_GAP_TX_ENDED)
    );
    assert_eq!(
        sticky_toast(vcx).title.as_deref(),
        Some("Read-only guard on orders-prod")
    );
}

#[gpui::test]
fn a_gap_that_changed_data_says_so(cx: &mut TestAppContext) {
    for reason in [GapReason::CanaryAssigned, GapReason::WriteSucceeded] {
        let (vcx, id) = shell_with(
            cx,
            "en",
            FakeDriver::new(DbmsKind::Postgres),
            ro(pg("orders-prod", EnvTag::Production)),
        );
        emit(
            vcx,
            ConnectionEvent::EnforcementGap {
                profile: id,
                reason,
            },
        );
        assert_eq!(
            sticky_toast(vcx).lines,
            [t(vcx, ids::hv::CONN_GAP_CHANGED_DATA)]
        );
        close_all_windows(vcx);
    }
}

// ---- read-only labels and the explanation -----------------------------------------------------

#[gpui::test]
fn read_only_labels_and_remediation_popover(cx: &mut TestAppContext) {
    let (vcx, id) = connected_with(
        cx,
        AccessLabel::ReadOnlyGuarded,
        vec![RoleWarning::Superuser],
    );
    assert_eq!(
        capsule_text(vcx, "access"),
        t(vcx, ids::hv::CONN_ACCESS_RO_GUARDED)
    );
    click(vcx, "capsule-access");
    assert!(layer_open(vcx));
    let text = popover_text(vcx);
    assert!(text.contains("<role>"), "{text}");
    assert!(text.contains("It is a superuser."), "{text}");
    assert!(text.contains("Found on this role:"), "{text}");
    assert!(
        text.contains("Read-only is guarded, not enforced"),
        "{text}"
    );
    assert!(text.contains("The role qa isn't limited"), "{text}");
    assert!(button_visible(vcx, "ro-copy-snippet"));

    // Enforced: the label says so, and its popover has no SQL to copy.
    vcx.simulate_keystrokes("escape");
    set_state(
        vcx,
        id,
        LiveState::Connected(Arc::new(info(
            "16.4",
            AccessLabel::ReadOnlyEnforced,
            vec![],
        ))),
    );
    assert_eq!(
        capsule_text(vcx, "access"),
        t(vcx, ids::hv::CONN_ACCESS_RO_ENFORCED)
    );
    click(vcx, "capsule-access");
    let text = popover_text(vcx);
    assert!(text.contains("Read-only, enforced by the server"), "{text}");
    assert!(!button_visible(vcx, "ro-copy-snippet"));
    // The status bar carries the capsule's label.
    assert!(
        plain(&status_bar_text(vcx)).contains("test · read-only (enforced by server role)"),
        "{}",
        status_bar_text(vcx)
    );
}

#[gpui::test]
fn copy_sql_copies_the_remediation_exactly(cx: &mut TestAppContext) {
    let (vcx, _) = connected_with(cx, AccessLabel::ReadOnlyGuarded, vec![]);
    click(vcx, "capsule-access");
    click(vcx, "button-ro-copy-snippet");
    let copied = vcx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .expect("the SQL is on the clipboard");
    assert_eq!(
        Some(copied.as_str()),
        hacivat_session::remediation(DbmsKind::Postgres)
    );
    assert!(
        toast_texts(vcx)
            .iter()
            .any(|s| s.starts_with("Copied the SQL.")),
        "{:?}",
        toast_texts(vcx)
    );
}

/// The T5 carry: PostgreSQL 13 gets the schema-grant form, with its `<schema>` placeholder.
#[gpui::test]
fn postgres_13_shows_the_schema_grant_form(cx: &mut TestAppContext) {
    let (vcx, id) = connected_with(cx, AccessLabel::ReadOnlyGuarded, vec![]);
    set_state(
        vcx,
        id,
        LiveState::Connected(Arc::new(info(
            "13.12",
            AccessLabel::ReadOnlyGuarded,
            vec![],
        ))),
    );
    click(vcx, "capsule-access");
    let text = popover_text(vcx);
    assert!(text.contains("<schema>"), "{text}");
    assert!(text.contains("<role>"), "{text}");
    click(vcx, "button-ro-copy-snippet");
    let copied = vcx.read_from_clipboard().and_then(|i| i.text());
    assert!(copied.is_some_and(|s| s.contains("GRANT USAGE ON SCHEMA <schema> TO <role>;")));
}

#[gpui::test]
fn enforced_with_temp_notes_temp_and_mariadb_has_no_sql(cx: &mut TestAppContext) {
    let (vcx, id) = connected_with(
        cx,
        AccessLabel::ReadOnlyEnforced,
        vec![RoleWarning::TempPrivilege],
    );
    click(vcx, "capsule-access");
    let text = popover_text(vcx);
    assert!(text.contains("It can create temporary tables"), "{text}");
    assert!(
        text.contains("TEMP alone doesn't weaken enforcement"),
        "{text}"
    );
    vcx.simulate_keystrokes("escape");

    let mut maria = info(
        "11.4",
        AccessLabel::ReadOnlyGuarded,
        vec![RoleWarning::EgressExtension("dblink".into())],
    );
    maria.server.kind = DbmsKind::MariaDb;
    maria.server.product = "MariaDB".into();
    set_state(vcx, id, LiveState::Connected(Arc::new(maria)));
    click(vcx, "capsule-access");
    let text = popover_text(vcx);
    assert!(
        text.contains("can't yet confirm that a MariaDB role"),
        "{text}"
    );
    assert!(
        text.contains("The dblink extension can reach other servers."),
        "{text}"
    );
    assert!(text.contains("there is no SQL to copy"), "{text}");
    assert!(!button_visible(vcx, "ro-copy-snippet"));
}

/// NOTES M2b I5: the segment is a Tab stop and Enter opens the popover; Tab reaches Copy SQL;
/// Escape closes it and gives the focus back to the segment.
#[gpui::test]
fn the_explanation_is_keyboard_reachable(cx: &mut TestAppContext) {
    let (vcx, _) = connected_with(cx, AccessLabel::ReadOnlyGuarded, vec![]);
    click(vcx, "capsule-access");
    // The popover took the focus on its first frame.
    let popover = focused(vcx).expect("the popover has the focus");
    vcx.simulate_keystrokes("tab");
    let copy = focused(vcx).expect("Copy SQL has the focus");
    assert_ne!(copy, popover);
    vcx.simulate_keystrokes("shift-tab");
    assert_eq!(focused(vcx), Some(popover.clone()));
    // Escape closes it and gives the focus back to the trigger, which the click focused.
    vcx.simulate_keystrokes("escape");
    assert!(!layer_open(vcx));
    let segment = focused(vcx).expect("the segment has the focus back");
    assert_ne!(segment, popover);
    assert_ne!(segment, copy);
    // Enter, or Space, on the focused segment opens it again (a keyboard click).
    vcx.simulate_keystrokes("enter");
    assert!(layer_open(vcx));
    vcx.simulate_keystrokes("escape");
    assert_eq!(focused(vcx), Some(segment.clone()));
    vcx.simulate_keystrokes("space");
    assert!(layer_open(vcx));
    vcx.simulate_keystrokes("escape");
    // The palette command too.
    assert!(run(vcx, "connection.explain_access"));
    assert!(layer_open(vcx));
}

/// The segment is in the Tab order of the window.
#[gpui::test]
fn tab_reaches_the_access_segment(cx: &mut TestAppContext) {
    let (vcx, _) = connected_with(cx, AccessLabel::ReadOnlyGuarded, vec![]);
    let mut reached = false;
    for _ in 0..40 {
        vcx.simulate_keystrokes("tab");
        if vcx.debug_bounds("capsule-access-ring").is_some() {
            reached = true;
            break;
        }
    }
    assert!(reached, "Tab never reached the access segment");
    vcx.simulate_keystrokes("enter");
    assert!(layer_open(vcx));
}

#[gpui::test]
fn a_press_outside_closes_the_explanation(cx: &mut TestAppContext) {
    let (vcx, _) = connected_with(cx, AccessLabel::ReadOnlyGuarded, vec![]);
    click(vcx, "capsule-access");
    assert!(layer_open(vcx));
    let explorer = vcx.debug_bounds("explorer-list").expect("the explorer");
    vcx.simulate_click(explorer.center(), gpui::Modifiers::none());
    vcx.run_until_parked();
    assert!(!layer_open(vcx));
}

#[gpui::test]
fn the_popover_stays_between_the_bars(cx: &mut TestAppContext) {
    let (vcx, _) = connected_with(
        cx,
        AccessLabel::ReadOnlyGuarded,
        vec![
            RoleWarning::Superuser,
            RoleWarning::BypassRls,
            RoleWarning::Replication,
            RoleWarning::CreateRole,
            RoleWarning::ServerFileRoles,
            RoleWarning::ExecuteServerProgram,
            RoleWarning::SignalBackend,
            RoleWarning::WritePrivileges,
            RoleWarning::TempPrivilege,
            RoleWarning::EgressExtension("postgres_fdw".into()),
            RoleWarning::RoleGrantNotExpanded,
            RoleWarning::Unparsed,
        ],
    );
    vcx.simulate_resize(gpui::size(gpui::px(960.0), gpui::px(600.0)));
    vcx.run_until_parked();
    click(vcx, "capsule-access");
    let card = vcx.debug_bounds("access-popover").expect("the popover");
    assert!(card.top() >= gpui::px(46.0), "{card:?}");
    assert!(card.bottom() <= gpui::px(600.0 - 24.0 - 6.0), "{card:?}");
    assert_eq!(card.size.width, gpui::px(300.0));
    // All twelve findings at the minimum window: the body scrolls, Copy SQL stays in view.
    assert!(vcx.debug_bounds("access-popover-thumb").is_some());
    let copy = vcx
        .debug_bounds("button-ro-copy-snippet")
        .expect("Copy SQL");
    assert!(copy.bottom() <= card.bottom(), "{copy:?} in {card:?}");
}

#[gpui::test]
fn the_sheet_access_value_opens_the_explanation(cx: &mut TestAppContext) {
    let (vcx, id) = shell_with(
        cx,
        "en",
        FakeDriver::new(DbmsKind::Postgres),
        ro(pg("orders-qa", EnvTag::Test)),
    );
    // Before the role probe ran: guarded, and nothing to explain yet.
    assert_eq!(
        text_of(vcx, "sheet-access-label").as_deref(),
        Some("read-only · guarded")
    );
    assert!(vcx.debug_bounds("sheet-access").is_none());
    set_state(
        vcx,
        id,
        LiveState::Connected(Arc::new(info(
            "16.4",
            AccessLabel::ReadOnlyEnforced,
            vec![],
        ))),
    );
    assert_eq!(
        text_of(vcx, "sheet-access-label").as_deref(),
        Some("read-only · enforced")
    );
    click(vcx, "sheet-access");
    assert!(layer_open(vcx));
    assert!(popover_text(vcx).contains("Read-only, enforced by the server"));
    // Escape gives the focus back to the value, a Tab stop that Enter opens again.
    vcx.simulate_keystrokes("escape");
    assert!(!layer_open(vcx));
    vcx.simulate_keystrokes("enter");
    assert!(layer_open(vcx));
    assert_eq!(layer_ids(vcx), ["popover"], "one popover, not two");
}

#[gpui::test]
fn turkish_read_only_texts_fit(cx: &mut TestAppContext) {
    let (vcx, id) = shell_with(
        cx,
        "tr",
        FakeDriver::new(DbmsKind::Postgres),
        ro(pg("orders-qa", EnvTag::Test)),
    );
    set_state(
        vcx,
        id,
        LiveState::Connected(Arc::new(info(
            "16.4",
            AccessLabel::ReadOnlyGuarded,
            vec![
                RoleWarning::Superuser,
                RoleWarning::WritePrivileges,
                RoleWarning::TempPrivilege,
            ],
        ))),
    );
    assert_eq!(capsule_text(vcx, "access"), "salt okunur (korumalı)");
    assert_eq!(
        text_of(vcx, "sheet-access-label").as_deref(),
        Some("salt okunur · korumalı")
    );
    click(vcx, "capsule-access");
    // Turkish is longer: the popover's texts wrap inside its 300 px and none is cut. (The
    // single-line labels of the capsule and the sheet are checked with the platform's real
    // text system by the shots harness's `live-*` surfaces, in en/tr/en-XA/tr-XT at 1280x800
    // and 960x600: this test platform's glyph advances are not a font's.)
    let popover: Vec<_> = registry_frame(vcx)
        .into_iter()
        .filter(|r| r.path.contains("connections/access_popover.rs:"))
        .collect();
    assert!(
        popover
            .iter()
            .any(|r| r.text.as_ref() == "Süper kullanıcı.")
    );
    for r in &popover {
        assert!(!r.clipped && !r.truncated, "{} is cut: {r:?}", r.text);
    }
}

// ---- SSH path ---------------------------------------------------------------------------------

#[gpui::test]
fn ssh_path_segment_and_explorer_flag(cx: &mut TestAppContext) {
    let (vcx, id) = shell_with(
        cx,
        "en",
        FakeDriver::new(DbmsKind::Postgres),
        ssh(pg("orders-prod", EnvTag::Staging), "bastion.acme.io"),
    );
    let mut live = info("16.4", AccessLabel::ReadWrite, vec![]);
    live.path = PathKind::Ssh;
    live.via = Some("bastion.acme.io".into());
    live.hops = 1;
    set_state(vcx, id, LiveState::Connected(Arc::new(live)));
    assert!(capsule_text(vcx, "path").contains("bastion.acme.io"));
    assert_eq!(capsule_text(vcx, "path"), "SSH via bastion.acme.io");
    assert!(vcx.debug_bounds("explorer-row-route-0").is_some());
    assert_eq!(status_text(vcx, "path"), capsule_text(vcx, "path"));
    // The status bar mirrors the rest too: the latency and the encryption.
    let status = plain(&status_bar_text(vcx));
    assert!(
        status.contains("12 ms") && status.contains("TLS 1.3 · verified"),
        "{status}"
    );
    // The sheet's Path row says the same.
    assert!(
        texts_drawn_by(vcx, "connections/sheet.rs")
            .iter()
            .any(|s| plain(s) == "SSH via bastion.acme.io")
    );
}

/// The T20 carry: the trail's SSH step has its own label.
#[gpui::test]
fn the_ssh_step_has_its_own_label(cx: &mut TestAppContext) {
    let progress = Progress {
        trail: Vec::new(),
        current: Some(TrailStep::Ssh),
        started: std::time::Instant::now(),
    };
    let vcx = with_state(cx, LiveState::Connecting(progress));
    let CapsuleModel::Connecting { step, .. } = capsule_model(vcx) else {
        panic!("connecting");
    };
    assert_eq!(step.as_ref(), "SSH");
}

// ---- fix round 1 ------------------------------------------------------------------------------

/// The widest capsule: production, enforced, through SSH, unencrypted.
fn widest(cx: &mut TestAppContext, lang: &str) -> (&'static mut VisualTestContext, ProfileId) {
    let (vcx, id) = shell_with(
        cx,
        lang,
        FakeDriver::new(DbmsKind::Postgres),
        ro(ssh(
            pg("orders-prod", EnvTag::Production),
            "bastion.acme.io",
        )),
    );
    let mut live = info("16.4", AccessLabel::ReadOnlyEnforced, vec![]);
    live.path = PathKind::Ssh;
    live.via = Some("bastion.acme.io".into());
    live.hops = 1;
    live.tls.encrypted = false;
    live.tls.version = None;
    live.tls.verified = TlsVerified::No;
    set_state(vcx, id, LiveState::Connected(Arc::new(live)));
    (vcx, id)
}

/// Ruling I1: at the 960x600 minimum window the capsule degrades in its fixed order; the palette
/// field keeps its minimum width and the trailing controls stay on the window.
#[gpui::test]
fn the_capsule_degrades_to_fit_the_minimum_window(cx: &mut TestAppContext) {
    for lang in ["en", "tr"] {
        let (vcx, _) = widest(cx, lang);
        // Wide: everything shows. (This test platform's glyphs are wider than a font's, so
        // "wide" is wider than a real screen; the shots harness checks the real sizes.)
        vcx.simulate_resize(gpui::size(gpui::px(4000.0), gpui::px(900.0)));
        settle_until(vcx, WAIT, || text_of(vcx, "capsule-text-latency").is_some());
        assert!(text_of(vcx, "capsule-text-path").is_some());
        assert!(text_of(vcx, "capsule-text-access").is_some());
        // The minimum window: the latency goes first, then the path text, then the access text.
        vcx.simulate_resize(gpui::size(gpui::px(960.0), gpui::px(600.0)));
        settle_until(vcx, WAIT, || text_of(vcx, "capsule-text-latency").is_none());
        wait_until_mut(vcx, "the palette field at its minimum or wider", |vcx| {
            let palette = vcx.debug_bounds("palette-field");
            palette.is_some_and(|b| b.size.width >= gpui::px(170.0) - gpui::px(0.5))
        });
        let palette = vcx
            .debug_bounds("palette-field")
            .expect("the palette field");
        assert!(palette.right() <= gpui::px(960.0), "{palette:?}");
        let capsule = vcx.debug_bounds("capsule").expect("the capsule");
        assert!(
            capsule.right() <= palette.left(),
            "{capsule:?} then {palette:?}"
        );
        // The trailing controls stay on the window, unclipped.
        for r in registry_frame(vcx)
            .iter()
            .filter(|r| r.path.contains("hacivat-ui/src/titlebar.rs:"))
        {
            assert!(!r.clipped, "{} is clipped ({lang})", r.text);
        }
        // The collapsed access segment stays a button that explains, named by its full label.
        assert!(vcx.debug_bounds("capsule-access").is_some());
        hacivat_ui::popover::a11y_record();
        vcx.cx.update(|cx| cx.refresh_windows());
        vcx.run_until_parked();
        let declared = hacivat_ui::popover::a11y_take();
        let label = t(vcx, ids::hv::CONN_ACCESS_RO_ENFORCED);
        let access = declared
            .iter()
            .find(|d| d.role == Some(Role::Button) && plain(&d.name) == label)
            .expect("the access segment is a button named by its label");
        assert_eq!(
            access.description.as_deref().map(plain),
            Some(t(vcx, ids::hv::CMD_CONNECTION_EXPLAIN_ACCESS))
        );
        click(vcx, "capsule-access");
        assert!(layer_open(vcx));
        vcx.simulate_keystrokes("escape");
        // The status bar keeps the full text.
        assert_eq!(
            status_text(vcx, "path"),
            tf(vcx, |cx| tr(
                cx,
                ids::hv::CONN_PATH_SSH,
                ids::hv::HvConnPathSsh {
                    host: hacivat_l10n::Text("bastion.acme.io".into())
                }
            ))
        );
        // Wide again: everything comes back.
        vcx.simulate_resize(gpui::size(gpui::px(4000.0), gpui::px(900.0)));
        settle_until(vcx, WAIT, || text_of(vcx, "capsule-text-latency").is_some());
        assert!(text_of(vcx, "capsule-text-access").is_some());
        close_all_windows(vcx);
    }
}

/// Ruling I2: the SSH route flag sits in the name column; the live meta keeps its widths.
#[gpui::test]
fn the_ssh_flag_never_squeezes_the_explorer_meta(cx: &mut TestAppContext) {
    // Two live production read-only rows whose names are as long, one through SSH: the meta
    // (latency, engine) is laid out exactly as wide in both; the SSH name gives the room.
    let (vcx, a) = shell_with(
        cx,
        "tr",
        FakeDriver::new(DbmsKind::Postgres),
        ro(ssh(pg("orders-aaa", EnvTag::Production), "bastion.acme.io")),
    );
    let b = add_profile(vcx, ro(pg("orders-bbb", EnvTag::Production)));
    let mut through_ssh = info("16.4", AccessLabel::ReadOnlyEnforced, vec![]);
    through_ssh.path = PathKind::Ssh;
    through_ssh.via = Some("bastion.acme.io".into());
    set_state(
        vcx,
        b,
        LiveState::Connected(Arc::new(info(
            "16.4",
            AccessLabel::ReadOnlyEnforced,
            vec![],
        ))),
    );
    set_state(vcx, a, LiveState::Connected(Arc::new(through_ssh)));
    let route = vcx
        .debug_bounds("explorer-row-route-0")
        .expect("the route flag");
    let row = vcx.debug_bounds("explorer-row-0").expect("the row");
    assert!(route.left() > row.left());
    let latency = |i: usize| {
        registry_frame(vcx)
            .into_iter()
            .find(|r| r.path.ends_with(&format!("#explorer-row-latency-{i}")))
            .unwrap_or_else(|| panic!("row {i} draws its latency"))
    };
    let engine = |i: usize| {
        registry_frame(vcx)
            .into_iter()
            .find(|r| r.path.ends_with(&format!("#explorer-row-engine-{i}")))
            .unwrap_or_else(|| panic!("row {i} draws its engine"))
    };
    assert_eq!(latency(0).painted_px, latency(1).painted_px);
    assert_eq!(latency(0).truncated, latency(1).truncated);
    assert_eq!(engine(0).painted_px, engine(1).painted_px);
}

/// M4: a connect that starts while a countdown sleeps gets its 50 ms clock at once.
#[gpui::test]
fn a_connect_clock_never_waits_for_a_countdown(cx: &mut TestAppContext) {
    let (vcx, a) = shell_with(
        cx,
        "en",
        FakeDriver::new(DbmsKind::Postgres),
        pg("orders-staging", EnvTag::Staging),
    );
    let b = add_profile(vcx, pg("orders-dev", EnvTag::Dev));
    vcx.cx.update(|cx| {
        hacivat_app::connections::live::mirror_state(
            cx,
            a,
            LiveState::Reconnecting {
                attempt: 3,
                next_in: Some(secs(4)),
            },
        )
    });
    vcx.run_until_parked();
    let wait = || vcx.cx.read(hacivat_app::connections::live::tick_wait);
    assert_eq!(wait(), Some(secs(1)));
    // The countdown follows the executor clock.
    vcx.executor().advance_clock(Duration::from_millis(1500));
    vcx.run_until_parked();
    assert_eq!(capsule_text(vcx, "next"), "in 3 s");
    let progress = Progress {
        trail: Vec::new(),
        current: None,
        started: std::time::Instant::now(),
    };
    vcx.cx.update(|cx| {
        hacivat_app::connections::live::mirror_state(cx, b, LiveState::Connecting(progress))
    });
    vcx.run_until_parked();
    assert_eq!(wait(), Some(hacivat_app::connections::live::TICK));
}

/// M7: a gap alert never waits behind two sticky session-lost toasts.
#[gpui::test]
fn a_gap_alert_takes_a_sticky_slot_at_once(cx: &mut TestAppContext) {
    let (vcx, a) = shell_with(
        cx,
        "en",
        FakeDriver::new(DbmsKind::Postgres),
        pg("orders-a", EnvTag::Dev),
    );
    let b = add_profile(vcx, pg("orders-b", EnvTag::Dev));
    let c = add_profile(vcx, ro(pg("orders-c", EnvTag::Production)));
    for p in [a, b] {
        emit(
            vcx,
            ConnectionEvent::SessionLost {
                profile: p,
                lost: lost(true, false, false),
            },
        );
    }
    assert_eq!(shown(vcx).iter().filter(|s| s.sticky).count(), 2);
    emit(
        vcx,
        ConnectionEvent::EnforcementGap {
            profile: c,
            reason: GapReason::WriteSucceeded,
        },
    );
    let visible = shown(vcx);
    assert!(visible.iter().any(|s| s.kind == "gap"), "{visible:?}");
    assert_eq!(visible.iter().filter(|s| s.sticky).count(), 2);
    // The demoted lost toast waits first in line and shows when a slot frees.
    assert_eq!(vcx.cx.read(hacivat_ui::toast::ToastQueue::waiting), 1);
}

/// M8: a gap that changed data downgrades "enforced" to guarded until the connection changes.
#[gpui::test]
fn a_gap_that_changed_data_downgrades_the_enforced_label(cx: &mut TestAppContext) {
    let (vcx, id) = connected_with(cx, AccessLabel::ReadOnlyEnforced, vec![]);
    assert_eq!(
        capsule_text(vcx, "access"),
        t(vcx, ids::hv::CONN_ACCESS_RO_ENFORCED)
    );
    emit(
        vcx,
        ConnectionEvent::EnforcementGap {
            profile: id,
            reason: GapReason::CanaryAssigned,
        },
    );
    assert_eq!(
        capsule_text(vcx, "access"),
        t(vcx, ids::hv::CONN_ACCESS_RO_GUARDED)
    );
    click(vcx, "capsule-access");
    assert!(popover_text(vcx).contains("Read-only is guarded, not enforced"));
    vcx.simulate_keystrokes("escape");
    // A new connection is probed anew.
    vcx.cx.update(|cx| {
        hacivat_app::connections::live::mirror_state(
            cx,
            id,
            LiveState::Connected(Arc::new(info(
                "16.4",
                AccessLabel::ReadOnlyEnforced,
                vec![],
            ))),
        )
    });
    vcx.run_until_parked();
    assert_eq!(
        capsule_text(vcx, "access"),
        t(vcx, ids::hv::CONN_ACCESS_RO_ENFORCED)
    );
}

/// M9 and M10: one explanation at a time, and it closes when its connection ends.
#[gpui::test]
fn one_explanation_at_a_time_and_it_follows_the_connection(cx: &mut TestAppContext) {
    let (vcx, id) = connected_with(cx, AccessLabel::ReadOnlyGuarded, vec![]);
    click(vcx, "capsule-access");
    assert!(run(vcx, "connection.explain_access"));
    assert_eq!(layer_ids(vcx), ["popover"]);
    set_state(
        vcx,
        id,
        LiveState::Lost {
            class: hacivat_proto::ErrClass::NetworkReset,
        },
    );
    vcx.cx.update(|cx| cx.refresh_windows());
    settle_until(vcx, WAIT, || !layer_open(vcx));
}

// ---- the damaged vault (T21 carry) ------------------------------------------------------------

/// A connect whose saved password sits in a vault whose file was damaged under it shows the
/// Damaged notice once; Close goes on to the password prompt, and the typed password connects.
#[gpui::test]
fn a_damaged_vault_shows_the_notice_once_then_asks_the_password(cx: &mut TestAppContext) {
    use hacivat_secrets::{
        FallbackBackend, FileVault, MemoryBackend, Retention, SecretBackend as _, SecretKey,
        SecretSlot, SecretString, VaultBackend as _, VaultParams,
    };

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("vault.bin");
    let vault = Arc::new(FileVault::with_params(
        path.clone(),
        VaultParams {
            m_kib: 64,
            t: 1,
            p: 1,
        },
    ));
    vault
        .create(SecretString::from("correct horse"))
        .expect("create");
    let mut p = pg("orders-dev", EnvTag::Dev);
    p.auth = AuthProviderId::Password;
    p.retention = Retention::Forever;
    vault
        .put(
            &SecretKey {
                profile: p.id,
                slot: SecretSlot::DbPassword,
            },
            &SecretString::from("hv_test_pw"),
        )
        .expect("put");
    // The file is damaged under the unlocked vault: its nonce changed.
    let mut bytes = std::fs::read(&path).expect("read");
    bytes[40] ^= 0xFF;
    std::fs::write(&path, &bytes).expect("write");
    let backend = FallbackBackend::new(Arc::new(MemoryBackend::new()), vault, Box::new(|| false));
    assert!(backend.probe_available(), "the vault is chosen");
    let vcx = shell_with_fake_backend(
        cx,
        "en",
        FakeDriver::new(DbmsKind::Postgres),
        Arc::new(backend),
        |_| {},
    );
    add_profile(vcx, p);
    connect(vcx, "orders-dev");
    wait_until(vcx, "the Damaged notice", |vcx| {
        layer_ids(vcx) == ["vault-damaged"]
    });
    assert_eq!(
        text_of(vcx, "vault-damaged-title").as_deref(),
        Some("The vault can't be read")
    );
    // Close (it has the focus) goes on: the password, as if nothing were stored.
    vcx.simulate_keystrokes("enter");
    wait_until(vcx, "the password prompt", |vcx| {
        layer_ids(vcx) == ["password-prompt"]
    });
    vcx.simulate_input("hv_test_pw");
    vcx.simulate_keystrokes("enter");
    wait_until(vcx, "the connect", |vcx| {
        matches!(live(vcx, "orders-dev"), LiveState::Connected(_))
    });
    assert!(layer_ids(vcx).is_empty(), "the notice showed once");
    drop(dir);
}

// ---- the debug cancel demo (Q39) --------------------------------------------------------------

#[cfg(debug_assertions)]
#[gpui::test]
fn debug_cancel_command_cancels_the_demo_sleep(cx: &mut TestAppContext) {
    let driver = FakeDriver::new(DbmsKind::Postgres).exec(vec![FakeExec {
        prefix: "SELECT pg_sleep(30)",
        result: Ok(ExecSummary::default()),
        delay: secs(30),
        tx_after: None,
        panics: false,
    }]);
    let (vcx, _) = shell_with(cx, "en", driver, pg("orders-dev", EnvTag::Dev));
    connect(vcx, "orders-dev");
    wait_until(vcx, "the connect", |vcx| {
        matches!(live(vcx, "orders-dev"), LiveState::Connected(_))
    });
    assert!(!run(vcx, "debug.cancel"), "nothing runs yet");
    assert!(run(vcx, "debug.run_pg_sleep"));
    wait_until(vcx, "the demo statement to run", |vcx| {
        fake(vcx)
            .executed()
            .iter()
            .any(|s| s == "SELECT pg_sleep(30)")
    });
    assert!(run(vcx, "debug.cancel"));
    settle_until(vcx, secs(5), || fake(vcx).cancel_calls() == 1);
    wait_until(vcx, "the result toast", |vcx| {
        toast_texts(vcx)
            .iter()
            .any(|s| s == "The demo statement was cancelled.")
    });
    assert_eq!(fake(vcx).cancel_calls(), 1);
}

/// Q39: the demo commands exist only in debug builds.
#[cfg(not(debug_assertions))]
#[gpui::test]
fn release_builds_have_no_cancel_demo(cx: &mut TestAppContext) {
    let (vcx, _) = shell_with(
        cx,
        "en",
        FakeDriver::new(DbmsKind::Postgres),
        pg("orders-dev", EnvTag::Dev),
    );
    vcx.cx.read(|cx| {
        let reg = cx.global::<hacivat_ui::actions::ActionRegistry>();
        assert!(reg.get("debug.run_pg_sleep").is_none());
        assert!(reg.get("debug.cancel").is_none());
    });
}
