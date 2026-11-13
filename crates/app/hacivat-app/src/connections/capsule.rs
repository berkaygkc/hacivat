//! The live capsule model: connecting, connected and failed states from `LiveState` (NOTES surface 7, Task 18).
//!
//! The capsule, the status bar and the connection sheet read one source: the mirrored
//! `ConnectionsModel::live` state of [`current_profile`] (R19), so the status bar never says
//! "No connection" next to a connected capsule. The helpers here (environment marks and words,
//! the engine label, the encryption fact) are shared by the explorer, the sheet and the status
//! bar.
//!
//! M2b (Task 22, NOTES M2a §7 and M2b surfaces 6–7):
//! - **Reconnecting** shows the attempt and, between attempts, a countdown to the next one
//!   (the GPUI executor clock, `live::reconnect_seconds`).
//! - **Access** says how read-only is held: `hv-conn-access-ro-enforced` when the server role
//!   enforces it, `hv-conn-access-ro-guarded` otherwise (B21). The segment opens the read-only
//!   explanation (`access_popover`), as does `connection.explain_access`.
//! - **SSH path**: `hv-conn-path-ssh` with the bastion (the last hop) and the latency.
//! - **Idle**: after an idle disconnect the capsule keeps the profile and says so; its body
//!   connects again (B18, NOTES M2b open question 5).

use std::rc::Rc;
use std::sync::Arc;

use hacivat_driver::Support;
use hacivat_l10n::{Count, ExactDecimal, MsgRef, NoArgs, Text, ids};
use hacivat_net::{AddressClass, TlsVerified};
use hacivat_proto::{AccessLabel, DbmsKind, EnvTag, ProfileId};
use hacivat_session::profile::{Profile, ProfileEndpoint, TransportSpec};
use hacivat_session::{LiveInfo, LiveState, PathKind};
use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui::capsule::{CapsuleModel, CapsuleSegment, EnvKind};
use hacivat_ui::explain::ExplainPopover;
use hacivat_ui::icon::IconName;
use hacivat_ui_core::gpui_surface::{
    App, Bounds, Pixels, Point, SharedString, Window, point, px, size,
};

use crate::connections::error_view::open_error_dialog;
use crate::connections::live::{clock_seconds, note_started, reconnect_seconds};
use crate::connections::model::ConnectionsModel;
use crate::connections::{access_popover, session_toasts};
use crate::l10n_global::tr;
use crate::shell::Workspace;

/// The layer ID of the unencrypted explanation (`Workspace::open_anchored`).
pub const POPOVER_LAYER: &str = "popover";

/// The profile the capsule and the status bar follow (R19): the explorer's selection while it
/// has a live state, else the most recent connect attempt.
pub fn current_profile(cx: &App) -> Option<Arc<Profile>> {
    let m = cx.try_global::<ConnectionsModel>()?;
    let selected = m
        .selected()
        .filter(|id| m.live(*id) != LiveState::Disconnected);
    selected.or_else(|| m.current()).and_then(|id| m.get(id))
}

/// The capsule's mark for an environment tag (`Other` when the profile has none).
pub fn env_kind(tag: Option<EnvTag>) -> EnvKind {
    match tag {
        Some(EnvTag::Production) => EnvKind::Production,
        Some(EnvTag::Staging) => EnvKind::Staging,
        Some(EnvTag::Test) => EnvKind::Test,
        Some(EnvTag::Dev) => EnvKind::Dev,
        Some(EnvTag::Local) => EnvKind::Local,
        None => EnvKind::Other,
    }
}

// ## m2a-t21-shots: one family of environment words (`hv-conn-env-*`), shared with the
// connection dialog's Environment control, the import dialog and the status bar.
/// The message of the tag's word (`production`, …, `no environment`).
pub fn env_word_id(tag: Option<EnvTag>) -> MsgRef<NoArgs> {
    match tag {
        Some(EnvTag::Production) => ids::hv::CONN_ENV_PRODUCTION,
        Some(EnvTag::Staging) => ids::hv::CONN_ENV_STAGING,
        Some(EnvTag::Test) => ids::hv::CONN_ENV_TEST,
        Some(EnvTag::Dev) => ids::hv::CONN_ENV_DEV,
        Some(EnvTag::Local) => ids::hv::CONN_ENV_LOCAL,
        None => ids::hv::CONN_ENV_UNSET,
    }
}

/// The tag as a word (`production`, `staging`, …, `no environment`).
pub fn env_word(cx: &App, tag: Option<EnvTag>) -> SharedString {
    tr(cx, env_word_id(tag), NoArgs)
}

/// The capsule's environment label: the pre-cased production block, else the tag's word.
fn capsule_env_label(cx: &App, tag: Option<EnvTag>) -> SharedString {
    match tag {
        Some(EnvTag::Production) => tr(cx, ids::hv::CAPSULE_ENV_PRODUCTION, NoArgs),
        other => env_word(cx, other),
    }
}

/// The engine of a live connection with its major version (`PostgreSQL 16`), else the
/// profile's engine name. Connection data, not chrome text.
pub fn engine_label(p: &Profile, info: Option<&LiveInfo>) -> SharedString {
    match info {
        Some(info) => match &info.server.version {
            Some(v) => format!("{} {}", info.server.product, v.major).into(),
            None => info.server.product.clone().into(),
        },
        None => p.engine.display_name().into(),
    }
}

/// Whether a live connection crosses a network without encryption (the "unencrypted" fact):
/// not a SQLite file, not a Unix socket (NOTES §5).
pub fn is_unencrypted(p: &Profile, info: &LiveInfo) -> bool {
    !info.tls.encrypted
        && p.engine != DbmsKind::Sqlite
        && info.address != AddressClass::UnixSocket
        && !matches!(p.endpoint, ProfileEndpoint::File { .. })
}

/// The encryption fact of a live connection (the sheet and the status bar).
pub fn encryption_label(cx: &App, p: &Profile, info: &LiveInfo) -> SharedString {
    if p.engine == DbmsKind::Sqlite {
        return tr(cx, ids::hv::CONN_ENC_NONE_FILE, NoArgs);
    }
    if !info.tls.encrypted {
        return tr(cx, ids::hv::CONN_ENC_UNENCRYPTED, NoArgs);
    }
    // ## m2a-t21-shots: the protocol as the test strip writes it (`TLS 1.3`, not the
    // library's `TLSv1.3`), so the capsule, the status bar and the sheet agree with it.
    let version = Text(match info.tls.version.as_deref() {
        Some(v) if !crate::connections::report::tls_version_number(v).is_empty() => {
            format!("TLS {}", crate::connections::report::tls_version_number(v))
        }
        _ => "TLS".to_string(),
    });
    match info.tls.verified {
        // ## m2b-t01-interfaces: a pinned certificate reads as verified until Task 19's label.
        TlsVerified::Full | TlsVerified::Pinned => tr(
            cx,
            ids::hv::CONN_ENC_VERIFIED,
            ids::hv::HvConnEncVerified { version },
        ),
        // ## m2a-finalfix (I3): the chain was checked, the host name was not.
        TlsVerified::CaOnly => tr(
            cx,
            ids::hv::CONN_ENC_CA_ONLY,
            ids::hv::HvConnEncCaOnly { version },
        ),
        TlsVerified::No => tr(
            cx,
            ids::hv::CONN_ENC_UNVERIFIED,
            ids::hv::HvConnEncUnverified { version },
        ),
    }
}

/// A latency in whole milliseconds (`12 ms`).
pub fn latency_label(cx: &App, info: &LiveInfo) -> SharedString {
    let ms = u64::try_from(info.latency.as_millis()).unwrap_or(u64::MAX);
    tr(
        cx,
        ids::hv::CAPSULE_LATENCY,
        ids::hv::HvCapsuleLatency { ms: Count(ms) },
    )
}

/// Whether the live connection is read-only (guarded or enforced, R10).
pub fn is_read_only(info: &LiveInfo) -> bool {
    info.access != AccessLabel::ReadWrite
}

// ## m2b-t22-live

/// How a live connection's access reads in the capsule, the status bar and the popover (B21):
/// read-write, read-only enforced by the server's role, or only guarded by Hacivat.
pub fn access_label(cx: &App, access: AccessLabel) -> SharedString {
    let id = match access {
        AccessLabel::ReadWrite => ids::hv::CAPSULE_ACCESS_READ_WRITE,
        AccessLabel::ReadOnlyEnforced => ids::hv::CONN_ACCESS_RO_ENFORCED,
        AccessLabel::ReadOnlyGuarded => ids::hv::CONN_ACCESS_RO_GUARDED,
    };
    tr(cx, id, NoArgs)
}

/// The SSH host a profile's database is reached from: the last hop (the bastion).
fn profile_bastion(p: &Profile) -> Option<String> {
    match &p.transport {
        TransportSpec::SshTunnel(ssh) => ssh.hops.last().map(|h| h.host.clone()),
        _ => None,
    }
}

/// The path of `p` (the capsule, the status bar and the sheet): `SSH via <bastion>` through a
/// tunnel (the live connection's bastion, else the profile's last hop), else Direct; and
/// whether it is direct.
pub fn path_label(cx: &App, p: &Profile, info: Option<&LiveInfo>) -> (SharedString, bool) {
    let bastion = match info {
        Some(info) if info.path == PathKind::Ssh => Some(
            info.via
                .clone()
                .or_else(|| profile_bastion(p))
                .unwrap_or_default(),
        ),
        Some(_) => None,
        None => profile_bastion(p),
    };
    match bastion {
        Some(host) => (
            tr(
                cx,
                ids::hv::CONN_PATH_SSH,
                ids::hv::HvConnPathSsh { host: Text(host) },
            ),
            false,
        ),
        None => (tr(cx, ids::hv::CAPSULE_PATH_DIRECT, NoArgs), true),
    }
}

/// The reconnect label (`Reconnecting · attempt n`).
pub fn reconnecting_label(cx: &App, attempt: u32) -> SharedString {
    tr(
        cx,
        ids::hv::CAPSULE_RECONNECTING,
        ids::hv::HvCapsuleReconnecting {
            n: Count(u64::from(attempt)),
        },
    )
}

fn step_label(cx: &App, step: hacivat_session::pipeline::TrailStep) -> SharedString {
    // ## m2a-t21-shots: the trail's own step names (`hv-conn-step-*`).
    crate::connections::report::step_label(&crate::connections::report::Fmt::ui(cx), step).into()
}

/// The running clock of a connect (`1.25 s`), from the GPUI executor clock (P22).
pub fn clock_label(cx: &App, id: ProfileId) -> SharedString {
    let seconds = clock_seconds(cx, id);
    tr(
        cx,
        ids::hv::CAPSULE_CLOCK,
        ids::hv::HvCapsuleClock {
            seconds: ExactDecimal(format!("{seconds:.2}")),
        },
    )
}

/// The `capsule` hook: the current profile's live state as a capsule model; `None` (the M1
/// "No connection" placeholder) with no model, no current profile, or while disconnected.
pub fn model(cx: &App) -> Option<CapsuleModel> {
    let m = cx.try_global::<ConnectionsModel>()?;
    let p = current_profile(cx)?;
    let env = env_kind(p.environment);
    let env_label = capsule_env_label(cx, p.environment);
    let name: SharedString = p.name.clone().into();
    match m.live(p.id) {
        // ## m2b-t22-live: an idle disconnect keeps the profile in the capsule.
        LiveState::Disconnected if session_toasts::is_idle(cx, p.id) => Some(CapsuleModel::Idle {
            env,
            env_label,
            name,
            engine: engine_label(&p, None),
            label: tr(cx, ids::hv::CONN_IDLE_CAP, NoArgs),
        }),
        LiveState::Disconnected => None,
        LiveState::Connecting(progress) => Some(CapsuleModel::Connecting {
            env,
            env_label,
            name,
            engine: engine_label(&p, None),
            label: tr(cx, ids::hv::CAPSULE_CONNECTING, NoArgs),
            step: progress
                .current
                .map(|s| step_label(cx, s))
                .unwrap_or_default(),
            clock: clock_label(cx, p.id),
        }),
        LiveState::Connected(info) => {
            let read_only = is_read_only(&info);
            // ## m2b-t22-live: the honest access label (B21) and the SSH path.
            let (path, direct) = path_label(cx, &p, Some(&info));
            Some(CapsuleModel::Connection {
                env,
                env_label,
                name,
                engine: engine_label(&p, Some(&info)),
                access: access_label(cx, session_toasts::effective_access(cx, p.id, info.access)),
                read_only,
                path,
                direct,
                latency: Some(latency_label(cx, &info)),
                unencrypted: is_unencrypted(&p, &info)
                    .then(|| tr(cx, ids::hv::CONN_ENC_UNENCRYPTED, NoArgs)),
                unsupported: matches!(info.support, Support::Unsupported { .. })
                    .then(|| tr(cx, ids::hv::CAPSULE_UNSUPPORTED, NoArgs)),
            })
        }
        LiveState::Failed(_) | LiveState::Lost { .. } => Some(CapsuleModel::Failed {
            env,
            env_label,
            label: tr(cx, ids::hv::CAPSULE_FAILED, NoArgs),
            name,
            retry: tr(cx, ids::hv::CAPSULE_RETRY, NoArgs),
        }),
        // ## m2b-t22-live: the attempt, and between attempts the countdown to the next.
        LiveState::Reconnecting { attempt, next_in } => Some(CapsuleModel::Reconnecting {
            env,
            env_label,
            name,
            engine: engine_label(&p, None),
            label: reconnecting_label(cx, attempt),
            next: next_in
                .map(|next| reconnect_seconds(cx, p.id, attempt, next))
                .filter(|seconds| *seconds > 0)
                .map(|seconds| {
                    tr(
                        cx,
                        ids::hv::CAPSULE_NEXT,
                        ids::hv::HvCapsuleNext {
                            seconds: Count(seconds),
                        },
                    )
                }),
        }),
    }
}

/// The `capsule_segment` hook: the body of a failed capsule opens the error dialog, Retry
/// connects again, and the unencrypted segment opens its explanation at `anchor` (P4).
pub fn on_segment(
    segment: CapsuleSegment,
    anchor: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    match segment {
        CapsuleSegment::Body => open_failure(window, cx),
        CapsuleSegment::Retry => retry(cx),
        CapsuleSegment::Unencrypted => explain_unencrypted(anchor, window, cx),
        // ## m2b-t22-live: the read-only explanation under the segment.
        CapsuleSegment::Access => explain_access(segment_bounds(anchor, cx), window, cx),
    }
}

/// Opens the error dialog of the current profile's failed connect; a lost one reconnects, and
/// so does one disconnected while idle (B18: it reconnects when used).
fn open_failure(window: &mut Window, cx: &mut App) {
    let Some(p) = current_profile(cx) else {
        return;
    };
    match cx.global::<ConnectionsModel>().live(p.id) {
        LiveState::Failed(error) => open_error_dialog(p.id, error, window, cx),
        // A lost connection has no connect error to show: the body retries, like Retry.
        LiveState::Lost { .. } => retry(cx),
        // ## m2b-t22-live
        LiveState::Disconnected if session_toasts::is_idle(cx, p.id) => {
            note_started(cx, p.id);
            ConnectionsModel::connect(cx, p.id, None);
        }
        _ => {}
    }
}

/// Whether the current profile's last connect failed or its connection was lost.
pub fn can_retry(cx: &App) -> bool {
    current_profile(cx).is_some_and(|p| {
        matches!(
            cx.global::<ConnectionsModel>().live(p.id),
            LiveState::Failed(_) | LiveState::Lost { .. }
        )
    })
}

/// Connects the current profile again after a failure (capsule Retry, `connection.retry`).
pub fn retry(cx: &mut App) {
    if !can_retry(cx) {
        return;
    }
    if let Some(p) = current_profile(cx) {
        note_started(cx, p.id);
        ConnectionsModel::connect(cx, p.id, None);
    }
}

/// Whether the capsule shows the "unencrypted" segment.
pub fn has_unencrypted(cx: &App) -> bool {
    matches!(
        model(cx),
        Some(CapsuleModel::Connection {
            unencrypted: Some(_),
            ..
        })
    )
}

/// Opens the explanation of the "unencrypted" segment as an anchored popover (NOTES §7).
pub fn explain_unencrypted(anchor: Point<Pixels>, window: &mut Window, cx: &mut App) {
    if !has_unencrypted(cx) {
        return;
    }
    let Some(p) = current_profile(cx) else {
        return;
    };
    let host = match &p.endpoint {
        ProfileEndpoint::Server { host, .. } => host.clone(),
        ProfileEndpoint::Socket { path, .. } | ProfileEndpoint::File { path, .. } => {
            path.display().to_string()
        }
    };
    let body = vec![
        tr(
            cx,
            ids::hv::CONN_UNENC_BODY,
            ids::hv::HvConnUnencBody { host: Text(host) },
        ),
        tr(cx, ids::hv::CONN_UNENC_FIX, NoArgs),
    ];
    let view = ExplainPopover::view(
        tr(cx, ids::hv::CONN_UNENC_TITLE, NoArgs),
        IconName::LockOpen,
        body,
        cx,
    );
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    ws.update(cx, |ws, cx| {
        ws.open_anchored(POPOVER_LAYER, anchor, view, window, cx);
    });
}

/// The keyboard path to the explanation (`connection.explain_unencrypted`): anchored under
/// the title bar's middle, where the capsule sits. The segment's real bounds are known only
/// inside `Capsule`'s click handler (Task 9's widget exposes no bounds), so a command run
/// from the keyboard or the palette uses this fixed anchor.
pub fn explain_unencrypted_from_keyboard(window: &mut Window, cx: &mut App) {
    let size = window.viewport_size();
    let t = hacivat_ui_core::theme::Theme::get(cx).tokens;
    let anchor = point(size.width / 2.0 - px(150.0), px(t.spacing.titlebar_height));
    explain_unencrypted(anchor, window, cx);
}

// ## m2b-t22-live

/// The bounds of a capsule segment whose bottom-left is `anchor` (the segment hook reports
/// only that corner): the capsule's height above it.
fn segment_bounds(anchor: Point<Pixels>, cx: &App) -> Bounds<Pixels> {
    let t = hacivat_ui_core::theme::Theme::get(cx).tokens;
    let h = px(t.spacing.control_height);
    Bounds {
        origin: point(anchor.x, anchor.y - h),
        size: size(px(1.0), h),
    }
}

/// Whether the capsule's connection is live and read-only (its access can be explained).
pub fn has_access_explanation(cx: &App) -> bool {
    current_profile(cx).is_some_and(|p| access_popover::facts_for(cx, p.id).is_some())
}

/// Opens the read-only explanation of the capsule's connection under `trigger`.
pub fn explain_access(trigger: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    if let Some(p) = current_profile(cx) {
        access_popover::open(p.id, trigger, window, cx);
    }
}

/// The keyboard path to the explanation (`connection.explain_access`): under the title bar's
/// middle, where the capsule sits (as `explain_unencrypted_from_keyboard`).
pub fn explain_access_from_keyboard(window: &mut Window, cx: &mut App) {
    let viewport = window.viewport_size();
    let t = hacivat_ui_core::theme::Theme::get(cx).tokens;
    let trigger = Bounds {
        origin: point(viewport.width / 2.0 - px(150.0), px(0.0)),
        size: size(px(1.0), px(t.spacing.titlebar_height - 6.0)),
    };
    explain_access(trigger, window, cx);
}

/// Registers `connection.explain_access` (the palette's "Explain read-only access", NOTES M2b
/// surface 6), enabled while the capsule shows a live read-only connection.
pub fn register(cx: &mut App) {
    let _ = cx.default_global::<ActionRegistry>().register(ActionDef {
        id: "connection.explain_access",
        title_id: ids::hv::CMD_CONNECTION_EXPLAIN_ACCESS.id,
        context: "Workspace",
        run: Rc::new(explain_access_from_keyboard),
        enabled: has_access_explanation,
    });
}

// ## m2a-t18-explorer: shots fixtures (R28, P19)

#[cfg(feature = "shots")]
pub use fixtures::*;

#[cfg(feature = "shots")]
mod fixtures {
    use std::sync::Arc;
    use std::time::Duration;

    use hacivat_driver::{SemVerLoose, ServerInfo, Support};
    use hacivat_net::ssh::{HopSpec, SshAuth, SshSpec};
    use hacivat_net::{AddressClass, TlsSummary, TlsVerified};
    use hacivat_proto::{AccessLabel, DbmsKind, EnvTag, ErrClass, ProfileId};
    use hacivat_secrets::AuthProviderId;
    use hacivat_session::pipeline::{Progress, StepOutcome, StepTrace, TrailStep};
    use hacivat_session::profile::{Profile, ProfileDraft, ProfileEndpoint, TransportSpec};
    use hacivat_session::session::LostState;
    use hacivat_session::{GapReason, LiveInfo, LiveState, PathKind, RoleWarning};
    use hacivat_ui_core::gpui_surface::{App, Window, point, px};

    use crate::connections::live::note_started;
    use crate::connections::model::ConnectionsModel;

    /// Saves a profile named `name` and makes it the capsule's current profile in `state`.
    fn show(cx: &mut App, name: &str, env: EnvTag, read_only: bool, state: LiveState) -> ProfileId {
        let profile = Profile {
            name: name.to_string(),
            environment: Some(env),
            read_only,
            endpoint: ProfileEndpoint::Server {
                host: "10.0.4.12".to_string(),
                port: None,
            },
            database: Some("orders".to_string()),
            user: Some("deploy".to_string()),
            ..Profile::default()
        };
        let id = profile.id;
        ConnectionsModel::save(
            cx,
            ProfileDraft {
                is_new: true,
                ..ProfileDraft::edit(profile)
            },
        )
        .detach();
        if matches!(state, LiveState::Connecting(_)) {
            note_started(cx, id);
        }
        let m = cx.global_mut::<ConnectionsModel>();
        m.set_live(id, state);
        m.note_attempt(id);
        ConnectionsModel::select(cx, Some(id));
        id
    }

    /// A live PostgreSQL 16.4 connection (also the explorer fixture's live rows).
    pub(crate) fn sample_live_info(encrypted: bool, read_only: bool, support: Support) -> LiveInfo {
        LiveInfo {
            server: ServerInfo {
                kind: DbmsKind::Postgres,
                product: "PostgreSQL".to_string(),
                version: SemVerLoose::parse("16.4"),
                raw_version: "16.4".to_string(),
                is_replica: Some(false),
            },
            support,
            tls: TlsSummary {
                encrypted,
                version: encrypted.then(|| "TLSv1.3".to_string()),
                cipher: None,
                verified: if encrypted {
                    TlsVerified::Full
                } else {
                    TlsVerified::No
                },
            },
            address: AddressClass::Private,
            latency: Duration::from_millis(12),
            access: if read_only {
                AccessLabel::ReadOnlyGuarded
            } else {
                AccessLabel::ReadWrite
            },
            path: PathKind::Direct,
            user: Some("deploy".to_string()),
            auth: AuthProviderId::Password,
            via: None,
            hops: 0,
            access_warnings: Vec::new(),
        }
    }

    /// The capsule while connecting (staging, signing in).
    pub fn connecting_fixture(_window: &mut Window, cx: &mut App) {
        let progress = Progress {
            trail: vec![
                StepTrace {
                    step: TrailStep::Resolve,
                    outcome: StepOutcome::Done,
                    elapsed: Some(Duration::from_millis(4)),
                },
                StepTrace {
                    step: TrailStep::SignIn,
                    outcome: StepOutcome::Running,
                    elapsed: None,
                },
            ],
            current: Some(TrailStep::SignIn),
            started: std::time::Instant::now(),
        };
        show(
            cx,
            "orders-staging",
            EnvTag::Staging,
            false,
            LiveState::Connecting(progress),
        );
    }

    /// A dev connection without TLS (the "unencrypted" segment).
    pub fn connected_unencrypted_fixture(_window: &mut Window, cx: &mut App) {
        show(
            cx,
            "orders-dev",
            EnvTag::Dev,
            false,
            LiveState::Connected(Arc::new(sample_live_info(false, false, Support::Supported))),
        );
    }

    /// A read-only production connection (the filled block and the red lock).
    pub fn production_ro_fixture(_window: &mut Window, cx: &mut App) {
        show(
            cx,
            "orders-prod",
            EnvTag::Production,
            true,
            LiveState::Connected(Arc::new(sample_live_info(true, true, Support::Supported))),
        );
    }

    /// A lost production connection: the capsule shows Failed with Retry.
    pub fn failed_fixture(_window: &mut Window, cx: &mut App) {
        show(
            cx,
            "orders-prod",
            EnvTag::Production,
            false,
            LiveState::Lost {
                class: ErrClass::NetworkRefused,
            },
        );
    }

    /// A server below the supported versions, connected anyway (PO Q3).
    pub fn unsupported_fixture(_window: &mut Window, cx: &mut App) {
        show(
            cx,
            "legacy",
            EnvTag::Test,
            false,
            LiveState::Connected(Arc::new(sample_live_info(
                true,
                false,
                Support::Unsupported {
                    floor: "14".to_string(),
                },
            ))),
        );
    }

    /// The explanation of the "unencrypted" segment, open.
    pub fn unencrypted_popover_fixture(window: &mut Window, cx: &mut App) {
        connected_unencrypted_fixture(window, cx);
        super::explain_unencrypted(point(px(520.0), px(34.0)), window, cx);
    }

    // ## m2b-t22-live (fix round 1, M1): the T22 states for the shots harness (Task 24
    // consumes them too).

    /// Saves `profile` and makes it the capsule's current profile in `state`.
    fn show_profile(cx: &mut App, profile: Profile, state: LiveState) -> ProfileId {
        let id = profile.id;
        ConnectionsModel::save(
            cx,
            ProfileDraft {
                is_new: true,
                ..ProfileDraft::edit(profile)
            },
        )
        .detach();
        let m = cx.global_mut::<ConnectionsModel>();
        m.set_live(id, state);
        m.note_attempt(id);
        ConnectionsModel::select(cx, Some(id));
        id
    }

    /// A profile `name` at 10.0.4.12, through `bastion` when given.
    fn profile(name: &str, env: EnvTag, read_only: bool, bastion: Option<&str>) -> Profile {
        let mut p = Profile {
            name: name.to_string(),
            environment: Some(env),
            read_only,
            endpoint: ProfileEndpoint::Server {
                host: "10.0.4.12".to_string(),
                port: None,
            },
            database: Some("orders".to_string()),
            user: Some("deploy".to_string()),
            ..Profile::default()
        };
        if let Some(host) = bastion {
            p.transport = TransportSpec::SshTunnel(SshSpec {
                hops: vec![HopSpec {
                    host: host.to_string(),
                    port: 22,
                    user: "deploy".to_string(),
                    auth: SshAuth::Agent,
                    identities_only: false,
                    identity_files: Vec::new(),
                }],
                host_keys: Default::default(),
                keepalive_s: None,
            });
        }
        p
    }

    /// A live PostgreSQL connection of `version` with `access` and the probe's `warnings`,
    /// through `bastion` when given, encrypted or not.
    fn live(
        version: &str,
        access: AccessLabel,
        warnings: Vec<RoleWarning>,
        bastion: Option<&str>,
        encrypted: bool,
    ) -> LiveState {
        let mut info = sample_live_info(
            encrypted,
            access != AccessLabel::ReadWrite,
            Support::Supported,
        );
        info.server.version = SemVerLoose::parse(version);
        info.server.raw_version = version.to_string();
        info.access = access;
        info.access_warnings = warnings;
        info.user = Some("app_ro".to_string());
        if let Some(host) = bastion {
            info.path = PathKind::Ssh;
            info.via = Some(host.to_string());
            info.hops = 1;
        }
        LiveState::Connected(Arc::new(info))
    }

    /// Production, reconnecting: attempt 3, the next in 4 s.
    pub fn reconnecting_fixture(_window: &mut Window, cx: &mut App) {
        show_profile(
            cx,
            profile("orders-prod", EnvTag::Production, false, None),
            LiveState::Reconnecting {
                attempt: 3,
                next_in: Some(Duration::from_secs(4)),
            },
        );
    }

    /// Disconnected after being idle, with its toast.
    pub fn idle_fixture(_window: &mut Window, cx: &mut App) {
        let id = show_profile(
            cx,
            profile("orders-dev", EnvTag::Dev, false, None),
            LiveState::Disconnected,
        );
        crate::connections::session_toasts::idle(id, cx);
    }

    /// A production session lost inside a transaction (temp tables and variables too).
    pub fn lost_tx_fixture(_window: &mut Window, cx: &mut App) {
        let id = show_profile(
            cx,
            profile("orders-prod", EnvTag::Production, false, None),
            LiveState::Lost {
                class: ErrClass::NetworkReset,
            },
        );
        crate::connections::session_toasts::lost(
            id,
            LostState {
                open_tx: true,
                temp_objects: true,
                session_vars: true,
                prepared: 0,
                locks: false,
            },
            cx,
        );
    }

    /// A production session reconnected: temp tables, two prepared statements and locks lost.
    pub fn lost_reconnected_fixture(_window: &mut Window, cx: &mut App) {
        let id = show_profile(
            cx,
            profile("orders-prod", EnvTag::Production, false, None),
            live("16.4", AccessLabel::ReadWrite, Vec::new(), None, true),
        );
        crate::connections::session_toasts::lost(
            id,
            LostState {
                open_tx: false,
                temp_objects: true,
                session_vars: false,
                prepared: 2,
                locks: true,
            },
            cx,
        );
    }

    /// A gap that changed data on a production read-only connection (the label downgrades).
    pub fn gap_fixture(_window: &mut Window, cx: &mut App) {
        let id = show_profile(
            cx,
            profile("orders-prod", EnvTag::Production, true, None),
            live(
                "16.4",
                AccessLabel::ReadOnlyEnforced,
                Vec::new(),
                None,
                true,
            ),
        );
        crate::connections::session_toasts::gap(id, GapReason::CanaryAssigned, cx);
    }

    /// Production, enforced (TEMP found) through SSH, with its explanation open.
    pub fn ro_enforced_fixture(window: &mut Window, cx: &mut App) {
        show_profile(
            cx,
            profile(
                "orders-prod",
                EnvTag::Production,
                true,
                Some("bastion.acme.io"),
            ),
            live(
                "16.4",
                AccessLabel::ReadOnlyEnforced,
                vec![RoleWarning::TempPrivilege],
                Some("bastion.acme.io"),
                true,
            ),
        );
        super::explain_access_from_keyboard(window, cx);
    }

    /// Test, guarded with every finding, with its explanation open (it scrolls at 960x600).
    pub fn ro_guarded_fixture(window: &mut Window, cx: &mut App) {
        show_profile(
            cx,
            profile("orders-qa", EnvTag::Test, true, None),
            live(
                "16.4",
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
                    RoleWarning::EgressExtension("postgres_fdw".to_string()),
                    RoleWarning::RoleGrantNotExpanded,
                    RoleWarning::Unparsed,
                ],
                None,
                true,
            ),
        );
        super::explain_access_from_keyboard(window, cx);
    }

    /// Staging on PostgreSQL 13, guarded: the schema-grant remediation.
    pub fn ro_pg13_fixture(window: &mut Window, cx: &mut App) {
        show_profile(
            cx,
            profile("orders-staging", EnvTag::Staging, true, None),
            live(
                "13.12",
                AccessLabel::ReadOnlyGuarded,
                vec![RoleWarning::Superuser, RoleWarning::WritePrivileges],
                None,
                true,
            ),
        );
        super::explain_access_from_keyboard(window, cx);
    }

    /// The widest production capsule: enforced, through SSH, unencrypted; and a direct row.
    pub fn ssh_ro_fixture(_window: &mut Window, cx: &mut App) {
        show_profile(
            cx,
            profile("orders-replica", EnvTag::Production, true, None),
            live("16.4", AccessLabel::ReadOnlyGuarded, Vec::new(), None, true),
        );
        show_profile(
            cx,
            profile(
                "orders-prod",
                EnvTag::Production,
                true,
                Some("bastion.acme.io"),
            ),
            live(
                "16.4",
                AccessLabel::ReadOnlyEnforced,
                Vec::new(),
                Some("bastion.acme.io"),
                false,
            ),
        );
    }

    /// A read-write staging connection through SSH, unencrypted.
    pub fn ssh_rw_fixture(_window: &mut Window, cx: &mut App) {
        show_profile(
            cx,
            profile(
                "orders-staging",
                EnvTag::Staging,
                false,
                Some("bastion.acme.io"),
            ),
            live(
                "16.4",
                AccessLabel::ReadWrite,
                Vec::new(),
                Some("bastion.acme.io"),
                false,
            ),
        );
    }
}
