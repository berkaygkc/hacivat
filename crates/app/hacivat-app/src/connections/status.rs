//! The status bar's connection segments (NOTES surface 7, Task 18).
//!
//! The status bar mirrors the capsule from the same source (`capsule::current_profile` and its
//! mirrored live state): the environment segment (`hv-status-env`), then the state or the path
//! with its latency, the encryption fact, the "unsupported" mark and the engine version. With
//! no current connection it shows M1's "No connection".
//!
//! M2b (Task 22): the environment segment carries the capsule's access label ("production ·
//! read-only (enforced by server role)", NOTES M2b surface 6); a reconnect shows its attempt,
//! an idle disconnect "Disconnected (idle)", and an SSH path `SSH via <bastion>` (label ID
//! `status-path`, the capsule's path text).

use hacivat_driver::Support;
use hacivat_l10n::{NoArgs, Text, ids};
use hacivat_session::LiveState;
use hacivat_session::profile::Profile;
use hacivat_ui::statusbar::{env_segment, status_label};
use hacivat_ui::text::TextSource;
use hacivat_ui_core::gpui_surface::{AnyElement, App, IntoElement, SharedString};
use hacivat_ui_core::theme::Theme;

use crate::connections::capsule::{
    access_label, current_profile, encryption_key, encryption_label, env_kind, env_word,
    is_read_only, latency_label, path_label, reconnecting_label,
};
use crate::connections::model::ConnectionsModel;
use crate::connections::session_toasts;
use crate::l10n_global::tr;

/// The access word of a profile without a live connection (`hv-capsule-access-*`).
fn profile_access(cx: &App, read_only: bool) -> SharedString {
    // ## m2a-t21-shots: no second copy of the environment and access words.
    let access = if read_only {
        ids::hv::CAPSULE_ACCESS_READ_ONLY
    } else {
        ids::hv::CAPSULE_ACCESS_READ_WRITE
    };
    tr(cx, access, NoArgs)
}

/// `production · read-only` and the like: the environment word (`hv-conn-env-*`) and the
/// access text in `hv-status-env`.
fn env_text(cx: &App, p: &Profile, access: SharedString) -> SharedString {
    tr(
        cx,
        ids::hv::STATUS_ENV,
        ids::hv::HvStatusEnv {
            env: Text(env_word(cx, p.environment).to_string()),
            access: Text(access.to_string()),
        },
    )
}

/// The `status_left` hook (see the module docs).
pub fn segments(theme: &Theme, cx: &App) -> Vec<AnyElement> {
    let no_connection = || {
        vec![
            status_label(
                theme,
                tr(cx, ids::hv::STATUS_NO_CONNECTION, NoArgs),
                TextSource::Fluent,
            )
            .into_any_element(),
        ]
    };
    let (Some(m), Some(p)) = (cx.try_global::<ConnectionsModel>(), current_profile(cx)) else {
        return no_connection();
    };
    let kind = env_kind(p.environment);
    // ## m2b-t22-live (fix round 1): every segment after the environment may ellipsize when
    // the bar is full (long locales at the minimum window; the tooltip keeps the whole text).
    let fluent = |text: SharedString| {
        status_label(theme, text, TextSource::Fluent)
            .truncate(true)
            .into_any_element()
    };
    let profile_env = || {
        env_segment(
            theme,
            kind,
            env_text(cx, &p, profile_access(cx, p.read_only)),
            p.read_only,
        )
    };
    match m.live(p.id) {
        // ## m2b-t22-live: an idle disconnect keeps the profile (NOTES M2b surface 7).
        LiveState::Disconnected if session_toasts::is_idle(cx, p.id) => vec![
            profile_env(),
            fluent(tr(cx, ids::hv::CONN_IDLE_CAP, NoArgs)),
        ],
        LiveState::Disconnected => no_connection(),
        LiveState::Connecting(_) => vec![
            profile_env(),
            fluent(tr(cx, ids::hv::CAPSULE_CONNECTING, NoArgs)),
        ],
        // ## m2b-t22-live: the attempt, as the capsule says it.
        LiveState::Reconnecting { attempt, .. } => {
            vec![profile_env(), fluent(reconnecting_label(cx, attempt))]
        }
        LiveState::Failed(_) | LiveState::Lost { .. } => vec![
            profile_env(),
            fluent(tr(cx, ids::hv::CAPSULE_FAILED, NoArgs)),
        ],
        LiveState::Connected(info) => {
            let read_only = is_read_only(&info);
            // ## m2b-t22-live: the capsule's access label and path (B21, SSH).
            let (path, _) = path_label(cx, &p, Some(&info));
            let mut out = vec![
                env_segment(
                    theme,
                    kind,
                    env_text(
                        cx,
                        &p,
                        access_label(cx, session_toasts::effective_access(cx, p.id, info.access)),
                    ),
                    read_only,
                ),
                status_label(theme, path, TextSource::Fluent)
                    .truncate(true)
                    .id("status-path")
                    .into_any_element(),
                fluent(latency_label(cx, &info)),
                // ## m2b-t24-shots (follow-up): which label it is, for the shots' markers.
                status_label(theme, encryption_label(cx, &p, &info), TextSource::Fluent)
                    .truncate(true)
                    .id(SharedString::from(format!(
                        "status-enc-{}",
                        encryption_key(&p, &info)
                    )))
                    .into_any_element(),
            ];
            if matches!(info.support, Support::Unsupported { .. }) {
                out.push(fluent(tr(cx, ids::hv::CAPSULE_UNSUPPORTED, NoArgs)));
            }
            let version = if info.server.raw_version.is_empty() {
                info.server.product.clone()
            } else {
                format!("{} {}", info.server.product, info.server.raw_version)
            };
            // ## m2b-t22-live (fix round 1): the last segment, it ellipsizes when the bar is full.
            out.push(
                status_label(theme, version, TextSource::Data)
                    .truncate(true)
                    .into_any_element(),
            );
            out
        }
    }
}
