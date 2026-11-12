//! The live capsule model: connecting, connected and failed states from `LiveState` (NOTES surface 7, Task 18).
//!
//! The capsule, the status bar and the connection sheet read one source: the mirrored
//! `ConnectionsModel::live` state of [`current_profile`] (R19), so the status bar never says
//! "No connection" next to a connected capsule. The helpers here (environment marks and words,
//! the engine label, the encryption fact) are shared by the explorer, the sheet and the status
//! bar.

use std::sync::Arc;

use hacivat_driver::Support;
use hacivat_l10n::{Count, ExactDecimal, NoArgs, Text, ids};
use hacivat_net::{AddressClass, TlsVerified};
use hacivat_proto::{AccessLabel, DbmsKind, EnvTag, ProfileId};
use hacivat_session::profile::{Profile, ProfileEndpoint};
use hacivat_session::{LiveInfo, LiveState};
use hacivat_ui::capsule::{CapsuleModel, CapsuleSegment, EnvKind};
use hacivat_ui::explain::ExplainPopover;
use hacivat_ui::icon::IconName;
use hacivat_ui_core::gpui_surface::{App, Pixels, Point, SharedString, Window, point, px};

use crate::connections::error_view::open_error_dialog;
use crate::connections::live::{clock_seconds, note_started};
use crate::connections::model::ConnectionsModel;
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

/// The tag as an argument of `hv-explorer-env`.
fn env_arg(tag: Option<EnvTag>) -> ids::hv::HvExplorerEnvEnv {
    use ids::hv::HvExplorerEnvEnv as E;
    match tag {
        Some(EnvTag::Production) => E::Production,
        Some(EnvTag::Staging) => E::Staging,
        Some(EnvTag::Test) => E::Test,
        Some(EnvTag::Dev) => E::Dev,
        Some(EnvTag::Local) => E::Local,
        None => E::Unset,
    }
}

/// The tag as a word (`production`, `staging`, …, `no environment`).
pub fn env_word(cx: &App, tag: Option<EnvTag>) -> SharedString {
    tr(
        cx,
        ids::hv::EXPLORER_ENV,
        ids::hv::HvExplorerEnv { env: env_arg(tag) },
    )
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
    let version = Text(
        info.tls
            .version
            .clone()
            .unwrap_or_else(|| "TLS".to_string()),
    );
    match info.tls.verified {
        TlsVerified::Full | TlsVerified::CaOnly => tr(
            cx,
            ids::hv::CONN_ENC_VERIFIED,
            ids::hv::HvConnEncVerified { version },
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

fn step_label(cx: &App, step: hacivat_session::pipeline::TrailStep) -> SharedString {
    use hacivat_session::pipeline::TrailStep as S;
    use ids::hv::HvCapsuleStepStep as A;
    let step = match step {
        S::PgPass => A::Pgpass,
        S::Resolve => A::Resolve,
        S::Connect => A::Connect,
        S::Tls => A::Tls,
        S::SignIn => A::SignIn,
        S::CheckServer => A::CheckServer,
        S::OpenFile => A::OpenFile,
    };
    tr(cx, ids::hv::CAPSULE_STEP, ids::hv::HvCapsuleStep { step })
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
            Some(CapsuleModel::Connection {
                env,
                env_label,
                name,
                engine: engine_label(&p, Some(&info)),
                access: if read_only {
                    tr(cx, ids::hv::CAPSULE_ACCESS_READ_ONLY, NoArgs)
                } else {
                    tr(cx, ids::hv::CAPSULE_ACCESS_READ_WRITE, NoArgs)
                },
                read_only,
                path: tr(cx, ids::hv::CAPSULE_PATH_DIRECT, NoArgs),
                direct: true,
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
    }
}

/// Opens the error dialog of the current profile's failed connect.
fn open_failure(window: &mut Window, cx: &mut App) {
    let Some(p) = current_profile(cx) else {
        return;
    };
    let state = cx.global::<ConnectionsModel>().live(p.id);
    if let LiveState::Failed(error) = state {
        open_error_dialog(p.id, error, window, cx);
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
/// the title bar's middle, where the capsule sits.
pub fn explain_unencrypted_from_keyboard(window: &mut Window, cx: &mut App) {
    let size = window.viewport_size();
    let t = hacivat_ui_core::theme::Theme::get(cx).tokens;
    let anchor = point(size.width / 2.0 - px(150.0), px(t.spacing.titlebar_height));
    explain_unencrypted(anchor, window, cx);
}

// ## m2a-t18-explorer: shots fixtures (R28, P19)

#[cfg(feature = "shots")]
pub use fixtures::*;

#[cfg(feature = "shots")]
mod fixtures {
    use std::sync::Arc;
    use std::time::Duration;

    use hacivat_driver::{SemVerLoose, ServerInfo, Support};
    use hacivat_net::{AddressClass, TlsSummary, TlsVerified};
    use hacivat_proto::{AccessLabel, DbmsKind, EnvTag, ErrClass, ProfileId};
    use hacivat_secrets::AuthProviderId;
    use hacivat_session::pipeline::{Progress, StepOutcome, StepTrace, TrailStep};
    use hacivat_session::profile::{Profile, ProfileDraft, ProfileEndpoint};
    use hacivat_session::{LiveInfo, LiveState, PathKind};
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

    fn info(encrypted: bool, read_only: bool, support: Support) -> LiveInfo {
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
            LiveState::Connected(Arc::new(info(false, false, Support::Supported))),
        );
    }

    /// A read-only production connection (the filled block and the red lock).
    pub fn production_ro_fixture(_window: &mut Window, cx: &mut App) {
        show(
            cx,
            "orders-prod",
            EnvTag::Production,
            true,
            LiveState::Connected(Arc::new(info(true, true, Support::Supported))),
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
            LiveState::Connected(Arc::new(info(
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
}
