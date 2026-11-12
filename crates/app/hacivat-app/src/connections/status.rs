//! The status bar's connection segments (NOTES surface 7, Task 18).
//!
//! The status bar mirrors the capsule from the same source (`capsule::current_profile` and its
//! mirrored live state): the environment segment (`hv-status-env`), then the state or the path
//! with its latency, the encryption fact, the "unsupported" mark and the engine version. With
//! no current connection it shows M1's "No connection".

use hacivat_driver::Support;
use hacivat_l10n::{NoArgs, ids};
use hacivat_proto::EnvTag;
use hacivat_session::LiveState;
use hacivat_session::profile::Profile;
use hacivat_ui::statusbar::{env_segment, status_label};
use hacivat_ui::text::TextSource;
use hacivat_ui_core::gpui_surface::{AnyElement, App, IntoElement, SharedString};
use hacivat_ui_core::theme::Theme;

use crate::connections::capsule::{
    current_profile, encryption_label, env_kind, is_read_only, latency_label,
};
use crate::connections::model::ConnectionsModel;
use crate::l10n_global::tr;

fn status_env_arg(tag: Option<EnvTag>) -> ids::hv::HvStatusEnvEnv {
    use ids::hv::HvStatusEnvEnv as E;
    match tag {
        Some(EnvTag::Production) => E::Production,
        Some(EnvTag::Staging) => E::Staging,
        Some(EnvTag::Test) => E::Test,
        Some(EnvTag::Dev) => E::Dev,
        Some(EnvTag::Local) => E::Local,
        None => E::Unset,
    }
}

/// `production · read-only` and the like.
fn env_text(cx: &App, p: &Profile, read_only: bool) -> SharedString {
    use ids::hv::HvStatusEnvAccess as A;
    tr(
        cx,
        ids::hv::STATUS_ENV,
        ids::hv::HvStatusEnv {
            env: status_env_arg(p.environment),
            access: if read_only { A::ReadOnly } else { A::ReadWrite },
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
    let fluent =
        |text: SharedString| status_label(theme, text, TextSource::Fluent).into_any_element();
    match m.live(p.id) {
        LiveState::Disconnected => no_connection(),
        LiveState::Connecting(_) => vec![
            env_segment(theme, kind, env_text(cx, &p, p.read_only), p.read_only),
            fluent(tr(cx, ids::hv::CAPSULE_CONNECTING, NoArgs)),
        ],
        LiveState::Failed(_) | LiveState::Lost { .. } => vec![
            env_segment(theme, kind, env_text(cx, &p, p.read_only), p.read_only),
            fluent(tr(cx, ids::hv::CAPSULE_FAILED, NoArgs)),
        ],
        LiveState::Connected(info) => {
            let read_only = is_read_only(&info);
            let mut out = vec![
                env_segment(theme, kind, env_text(cx, &p, read_only), read_only),
                fluent(tr(cx, ids::hv::CAPSULE_PATH_DIRECT, NoArgs)),
                fluent(latency_label(cx, &info)),
                fluent(encryption_label(cx, &p, &info)),
            ];
            if matches!(info.support, Support::Unsupported { .. }) {
                out.push(fluent(tr(cx, ids::hv::CAPSULE_UNSUPPORTED, NoArgs)));
            }
            let version = if info.server.raw_version.is_empty() {
                info.server.product.clone()
            } else {
                format!("{} {}", info.server.product, info.server.raw_version)
            };
            out.push(status_label(theme, version, TextSource::Data).into_any_element());
            out
        }
    }
}
