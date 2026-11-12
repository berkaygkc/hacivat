//! The effective locale settings: settings over OS over defaults (I18N-06).
//!
//! Three independent axes come out of here: the UI language (negotiation sources), the regional
//! format (`ui.regional_format`, then the OS) and the time zone (`ui.time_zone`, then the OS, then
//! UTC). An explicit UI language never sets the regional format (I18N-03 step 7).

use hacivat_settings::{LanguagePref, Settings, TimeZonePref};

use crate::negotiate::{NegotiationCtx, PrefSource};

/// The environment variables `detect` reads, in no particular order.
const ENV_VARS: [&str; 7] = [
    "HACIVAT_LANG",
    "LANGUAGE",
    "LC_ALL",
    "LC_MESSAGES",
    "LC_NUMERIC",
    "LC_TIME",
    "LANG",
];

/// The UI-language variables, in priority order after the OS list (I18N-03 step 1).
const UI_ENV_AFTER_OS: [&str; 4] = ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"];

/// What the OS reports about the user's locale.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct OsLocale {
    /// Preferred UI languages, most preferred first (`sys-locale`).
    pub languages: Vec<String>,
    /// The regional format, when the OS reports it separately from the languages.
    pub format: Option<String>,
    /// An IANA zone such as `Europe/Istanbul`.
    pub time_zone: Option<String>,
    /// The locale environment variables that are set and not empty.
    pub env: Vec<(&'static str, String)>,
}

impl OsLocale {
    /// Reads `sys-locale`, the environment and `iana-time-zone`.
    ///
    /// The regional format comes from `LC_ALL`, then `LC_NUMERIC`/`LC_TIME`, then `LANG` on
    /// Linux and the BSDs. macOS and Windows keep it in OS settings that `sys-locale` does not
    /// expose, so `format` stays `None` there and the OS language list supplies the region.
    pub fn detect() -> OsLocale {
        let env: Vec<(&'static str, String)> = ENV_VARS
            .iter()
            .filter_map(|&name| {
                let value = std::env::var(name).ok()?;
                (!value.trim().is_empty()).then_some((name, value))
            })
            .collect();
        let format = if cfg!(all(unix, not(target_os = "macos"))) {
            format_from_env(&env)
        } else {
            None
        };
        OsLocale {
            languages: sys_locale::get_locales().collect(),
            format,
            time_zone: iana_time_zone::get_timezone()
                .ok()
                .filter(|z| !z.trim().is_empty()),
            env,
        }
    }
}

/// The POSIX regional format: `LC_ALL`, then `LC_NUMERIC`, then `LC_TIME`, then `LANG`.
/// `C` and `POSIX` say nothing about a region and are skipped.
fn format_from_env(env: &[(&'static str, String)]) -> Option<String> {
    ["LC_ALL", "LC_NUMERIC", "LC_TIME", "LANG"]
        .iter()
        .filter_map(|name| env.iter().find(|(n, _)| n == name).map(|(_, v)| v))
        .find(|v| {
            let bare = v.trim().split(['.', '@']).next().unwrap_or_default();
            !bare.is_empty() && bare != "C" && bare != "POSIX"
        })
        .cloned()
}

/// The negotiation inputs and time zone that follow from settings and the OS. The caller builds
/// `FormatInputs::new(&negotiate(&e.ctx, ..), &e.time_zone)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EffectiveLocale {
    pub ctx: NegotiationCtx,
    pub time_zone: String,
}

/// Combines settings and the OS locale.
///
/// UI sources, in priority order (I18N-03 step 1, desktop): `ui.language`, `HACIVAT_LANG`, the
/// OS list, `LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, `LANG`. `negotiate` classifies them (an explicit
/// choice may pick an experimental catalog; only the OS list and the regional variables may set
/// the format locale). The time zone is `ui.time_zone` when ICU4X knows it, else the OS zone,
/// else `UTC`; an unknown zone in the settings is treated as `system` (I18N-06 edge 7).
pub fn effective(s: &Settings, os: &OsLocale, allow_pseudo: bool) -> EffectiveLocale {
    let env = |name: &str| {
        os.env
            .iter()
            .find(|(n, v)| *n == name && !v.trim().is_empty())
            .map(|(n, v)| PrefSource::Env(n, v.clone()))
    };
    let mut sources = Vec::new();
    if let LanguagePref::Tag(tag) = &s.ui.language {
        sources.push(PrefSource::Setting(tag.clone()));
    }
    sources.extend(env("HACIVAT_LANG"));
    if !os.languages.is_empty() {
        sources.push(PrefSource::OsList(os.languages.clone()));
    }
    sources.extend(UI_ENV_AFTER_OS.iter().filter_map(|name| env(name)));

    let format_pref = match &s.ui.regional_format {
        LanguagePref::Tag(tag) => Some(tag.clone()),
        LanguagePref::System => None,
    };
    let setting_zone = match &s.ui.time_zone {
        TimeZonePref::Iana(zone) if is_known_zone(zone) => Some(zone.clone()),
        TimeZonePref::Iana(zone) => {
            tracing::warn!(zone, "l10n: unknown ui.time_zone, using the system zone");
            None
        }
        TimeZonePref::System => None,
    };
    let time_zone = setting_zone
        .or_else(|| os.time_zone.clone().filter(|z| !z.trim().is_empty()))
        .unwrap_or_else(|| "UTC".to_string());
    EffectiveLocale {
        ctx: NegotiationCtx {
            sources,
            format_pref,
            os_format: os.format.clone(),
            allow_experimental: false,
            allow_pseudo,
        },
        time_zone,
    }
}

/// Whether the tz database knows `zone` (an IANA ID or alias); the formatters use the same data.
fn is_known_zone(zone: &str) -> bool {
    jiff::tz::TimeZone::get(zone).is_ok()
}

/// The regional format policy (controller ruling, Task 9 fix round 1): numbers and dates follow
/// the REGION, as the OS does. macOS reports "English, region Turkey" as `en-TR`, a locale CLDR
/// has no data for; formatting it as `en` would print `1,234.5` for a user whose system shows
/// `1.234,5`. So a `language-REGION` pair is kept when
/// - the region is the language's own likely region (`de-DE`, `ca-ES`), or
/// - the region's likely language is the tag's language (`de-AT`, `en-GB`), or
/// - CLDR has number or date data for exactly that pair (`fr-CA`, `fr-CH`, `en-HK`, `es-US`);
///
/// otherwise it becomes the region's likely language with that region (`en-TR` → `tr-TR`).
/// Extensions (`-u-nu-…`) are kept. User-customised separators need a native per-OS shim, which
/// is an open item.
pub(crate) fn regional_format_locale(mut locale: icu_locale::Locale) -> icu_locale::Locale {
    use icu_locale::LocaleExpander;
    use icu_locale::subtags::Language;
    let Some(region) = locale.id.region else {
        return locale;
    };
    let language = locale.id.language;
    if language == Language::UNKNOWN {
        return locale;
    }
    let expander = LocaleExpander::new_extended();
    let mut own = icu_locale::LanguageIdentifier::from(language);
    expander.maximize(&mut own);
    if own.region == Some(region) {
        return locale;
    }
    let mut by_region = icu_locale::LanguageIdentifier::UNKNOWN;
    by_region.region = Some(region);
    expander.maximize(&mut by_region);
    if by_region.language == language
        || by_region.language == Language::UNKNOWN
        || has_own_data(&locale)
    {
        return locale;
    }
    locale.id.language = by_region.language;
    locale.id.script = None;
    locale.id.variants.clear();
    locale
}

/// Whether ICU4X's compiled CLDR data has decimal symbols or a Gregorian year-month-day pattern
/// for exactly this locale: a direct hit, not a fallback to a parent. (ICU4X drops data equal to
/// its parent's, so a pair without a hit formats exactly like its parent.)
fn has_own_data(locale: &icu_locale::Locale) -> bool {
    use icu_datetime::provider::semantic_skeletons::DatetimePatternsDateGregorianV1;
    use icu_decimal::provider::DecimalSymbolsV1;
    use icu_provider::prelude::*;
    /// The attribute key of the `YMD` field set in `DatetimePatternsDateGregorianV1`.
    const YMD: &DataMarkerAttributes = DataMarkerAttributes::from_str_or_panic("ym0d");
    let mut bare = locale.clone();
    bare.extensions = Default::default();
    let data_locale = DataLocale::from(&bare);
    let request = |attributes: &'static DataMarkerAttributes| {
        let mut metadata = DataRequestMetadata::default();
        metadata.silent = true;
        DataRequest {
            id: DataIdentifierBorrowed::for_marker_attributes_and_locale(attributes, &data_locale),
            metadata,
        }
    };
    let decimal = DataProvider::<DecimalSymbolsV1>::load(
        &icu_decimal::provider::Baked,
        request(Default::default()),
    );
    let dates = DataProvider::<DatetimePatternsDateGregorianV1>::load(
        &icu_datetime::provider::Baked,
        request(YMD),
    );
    decimal.is_ok_and(|r| r.metadata.locale.is_none())
        || dates.is_ok_and(|r| r.metadata.locale.is_none())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
        pairs.iter().map(|(n, v)| (*n, v.to_string())).collect()
    }

    #[test]
    fn posix_format_order_is_lc_all_numeric_time_lang() {
        let all = env(&[
            ("LANG", "en_US.UTF-8"),
            ("LC_TIME", "de_DE.UTF-8"),
            ("LC_NUMERIC", "tr_TR.UTF-8"),
            ("LC_ALL", "fr_FR.UTF-8"),
        ]);
        assert_eq!(format_from_env(&all).as_deref(), Some("fr_FR.UTF-8"));
        assert_eq!(format_from_env(&all[..3]).as_deref(), Some("tr_TR.UTF-8"));
        assert_eq!(format_from_env(&all[..2]).as_deref(), Some("de_DE.UTF-8"));
        assert_eq!(format_from_env(&all[..1]).as_deref(), Some("en_US.UTF-8"));
        assert_eq!(format_from_env(&[]), None);
    }

    #[test]
    fn c_locale_says_nothing_about_the_region() {
        let e = env(&[("LC_ALL", "C.UTF-8"), ("LANG", "tr_TR.UTF-8")]);
        assert_eq!(format_from_env(&e).as_deref(), Some("tr_TR.UTF-8"));
        assert_eq!(format_from_env(&env(&[("LANG", "POSIX")])), None);
    }

    #[test]
    fn sources_follow_the_desktop_priority() {
        let mut s = Settings::default();
        s.ui.language = LanguagePref::Tag("tr".into());
        let os = OsLocale {
            languages: vec!["de-DE".into()],
            env: env(&[
                ("LANG", "en_US.UTF-8"),
                ("LC_NUMERIC", "tr_TR.UTF-8"),
                ("HACIVAT_LANG", "pl"),
                ("LANGUAGE", "fr:en"),
            ]),
            ..OsLocale::default()
        };
        let e = effective(&s, &os, false);
        assert_eq!(
            e.ctx.sources,
            vec![
                PrefSource::Setting("tr".into()),
                PrefSource::Env("HACIVAT_LANG", "pl".into()),
                PrefSource::OsList(vec!["de-DE".into()]),
                PrefSource::Env("LANGUAGE", "fr:en".into()),
                PrefSource::Env("LANG", "en_US.UTF-8".into()),
            ]
        );
        assert!(!e.ctx.allow_pseudo && !e.ctx.allow_experimental);
    }
}
