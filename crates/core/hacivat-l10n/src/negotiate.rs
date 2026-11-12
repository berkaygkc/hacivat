//! Language negotiation (I18N-03). Body in Task 6.

use unic_langid::LanguageIdentifier;

use crate::catalog::CatalogLocale;

/// One source of language preferences, in priority order.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PrefSource {
    /// `ui.language` from settings.
    Setting(String),
    /// An environment variable and its value (`LANG`, `LC_ALL`, ...).
    Env(&'static str, String),
    /// The OS's preferred languages.
    OsList(Vec<String>),
}

/// Inputs to [`negotiate`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct NegotiationCtx {
    pub sources: Vec<PrefSource>,
    /// `ui.regional_format` when set.
    pub format_pref: Option<String>,
    /// The OS's regional format.
    pub os_format: Option<String>,
    pub allow_experimental: bool,
    pub allow_pseudo: bool,
}

/// A negotiation problem worth reporting.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NegDiag {
    /// `LANG=C` (or `POSIX`) in `var`.
    CLocale {
        var: String,
    },
    Unparsable {
        source: String,
        value: String,
    },
    NoMatch {
        requested: String,
    },
}

/// The negotiated UI fallback chain (always ending in `en`) and format locale.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Negotiated {
    pub chain: Vec<LanguageIdentifier>,
    pub format_locale: String,
    pub diags: Vec<NegDiag>,
}

/// Negotiates the UI chain and format locale. Stub: English (`["en"]`, `en-US`).
pub fn negotiate(ctx: &NegotiationCtx, avail: &[CatalogLocale]) -> Negotiated {
    let _ = (ctx, avail);
    Negotiated {
        chain: vec![unic_langid::langid!("en")],
        format_locale: "en-US".to_string(),
        diags: Vec::new(),
    }
}
