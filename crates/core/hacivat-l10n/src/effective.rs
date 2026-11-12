//! The effective locale settings: settings over OS over defaults (I18N-06). Bodies in Task 9.

use hacivat_settings::Settings;

use crate::negotiate::NegotiationCtx;

/// What the OS reports about the user's locale.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct OsLocale {
    pub languages: Vec<String>,
    pub format: Option<String>,
    pub time_zone: Option<String>,
    pub env: Vec<(&'static str, String)>,
}

impl OsLocale {
    /// Reads `sys-locale`, the environment and `iana-time-zone`. Stub: empty.
    pub fn detect() -> OsLocale {
        OsLocale::default()
    }
}

/// The negotiation inputs and time zone that follow from settings and the OS. The caller builds
/// `FormatInputs::new(&negotiate(&e.ctx, ..), &e.time_zone)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EffectiveLocale {
    pub ctx: NegotiationCtx,
    pub time_zone: String,
}

/// Combines settings and the OS locale. Stub: empty sources, `UTC`.
pub fn effective(s: &Settings, os: &OsLocale, allow_pseudo: bool) -> EffectiveLocale {
    let _ = (s, os);
    EffectiveLocale {
        ctx: NegotiationCtx {
            allow_pseudo,
            ..NegotiationCtx::default()
        },
        time_zone: "UTC".to_string(),
    }
}
