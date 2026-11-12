//! Language negotiation (I18N-03): the UI fallback chain over the available catalogs, always
//! ending in `en`, and a format locale that need not have a catalog.

use std::cmp::Reverse;

use unic_langid::LanguageIdentifier;

use crate::catalog::{CatalogLocale, Status, is_pseudo};

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

/// One normalized preference.
struct Request {
    /// As requested, with extensions (`tr-TR-u-nu-arab`).
    locale: icu_locale::Locale,
    /// An explicit UI-language choice (`ui.language`, `HACIVAT_LANG`, `KARAGOZ_LANG`): may pick
    /// an experimental catalog.
    explicit: bool,
    /// A regional source that may supply the format locale (the OS list, `LC_ALL`,
    /// `LC_NUMERIC`, `LC_TIME`, `LANG`). Explicit UI choices and the message-only `LANGUAGE`
    /// and `LC_MESSAGES` never set the format locale.
    regional: bool,
}

/// Environment variables that name the UI language explicitly.
const EXPLICIT_ENV: [&str; 2] = ["HACIVAT_LANG", "KARAGOZ_LANG"];
/// Environment variables that only concern messages.
const MESSAGE_ENV: [&str; 2] = ["LANGUAGE", "LC_MESSAGES"];

/// Negotiates the UI chain and format locale (I18N-03 steps 1–7). Every request, in priority
/// order, appends its best catalog (and that catalog's same-script parents); the chain then ends
/// right after the first `en`, because English is complete and nothing after it is reached.
pub fn negotiate(ctx: &NegotiationCtx, avail: &[CatalogLocale]) -> Negotiated {
    let en = unic_langid::langid!("en");
    let mut diags = Vec::new();
    let requests = collect(ctx, &mut diags);
    let has_en = avail.iter().any(|c| c.tag == en);

    let mut chain: Vec<LanguageIdentifier> = Vec::new();
    for r in &requests {
        let id = r.locale.id.to_string();
        let allow_experimental = ctx.allow_experimental || r.explicit;
        let matched: Vec<LanguageIdentifier> = if is_pseudo(&id) {
            // Pseudo-locales are generated from `en`, so they need it and a permission.
            match id.parse() {
                Ok(tag) if ctx.allow_pseudo && has_en => vec![tag],
                _ => Vec::new(),
            }
        } else {
            best_match(&r.locale.id, avail, allow_experimental)
                .map(|found| with_parents(found, avail, allow_experimental))
                .unwrap_or_default()
        };
        if matched.is_empty() {
            diags.push(NegDiag::NoMatch { requested: id });
        }
        for tag in matched {
            if !chain.contains(&tag) {
                chain.push(tag);
            }
        }
    }
    match chain.iter().position(|t| *t == en) {
        Some(i) => chain.truncate(i + 1),
        None => chain.push(en),
    }

    let format_locale = format_locale(ctx, &requests, &mut diags);
    Negotiated {
        chain,
        format_locale,
        diags,
    }
}

/// `found` followed by its parents that exist in `avail`: first without the region, then
/// without the script (`pt-Latn-BR` → `pt-BR` → `pt`, `uz-Latn` → `uz`). A parent whose
/// likely script differs is skipped, because a chain never crosses scripts (`zh-Hant` ↛ `zh`,
/// which is Simplified).
fn with_parents(
    found: &CatalogLocale,
    avail: &[CatalogLocale],
    allow_experimental: bool,
) -> Vec<LanguageIdentifier> {
    let tag = &found.tag;
    let mut out = vec![tag.clone()];
    let mut candidates = Vec::new();
    if tag.region.is_some() {
        let mut p = tag.clone();
        p.region = None;
        candidates.push(p);
    }
    if tag.region.is_some() && tag.script.is_some() {
        let mut p = tag.clone();
        p.script = None;
        candidates.push(p);
    }
    if tag.region.is_some() || tag.script.is_some() {
        let mut p = tag.clone();
        p.region = None;
        p.script = None;
        candidates.push(p);
    }
    for parent in candidates {
        let usable = avail.iter().any(|c| {
            c.tag == parent
                && c.max.script == found.max.script
                && (allow_experimental || c.status != Status::Experimental)
        });
        if usable && !out.contains(&parent) {
            out.push(parent);
        }
    }
    out
}

/// Step 1–2: raw preferences in priority order, normalized (duplicates are harmless: the chain
/// deduplicates, and the format locale takes the first regional one).
fn collect(ctx: &NegotiationCtx, diags: &mut Vec<NegDiag>) -> Vec<Request> {
    let mut out: Vec<Request> = Vec::new();
    let mut add =
        |raw: &str, name: &str, explicit: bool, regional: bool, diags: &mut Vec<NegDiag>| {
            if let Some(locale) = normalize(raw, name, diags) {
                out.push(Request {
                    locale,
                    explicit,
                    regional,
                });
            }
        };
    for source in &ctx.sources {
        match source {
            PrefSource::Setting(tag) => add(tag, "setting", true, false, diags),
            PrefSource::Env(var, value) => {
                let explicit = EXPLICIT_ENV.contains(var);
                let regional = !explicit && !MESSAGE_ENV.contains(var);
                for part in value.split(':') {
                    let bare = strip_codeset(part);
                    if bare == "C" || bare == "POSIX" {
                        diags.push(NegDiag::CLocale {
                            var: (*var).to_string(),
                        });
                        add("en", var, explicit, regional, diags);
                    } else {
                        add(part, var, explicit, regional, diags);
                    }
                }
            }
            PrefSource::OsList(tags) => {
                for tag in tags {
                    add(tag, "os", false, true, diags);
                }
            }
        }
    }
    out
}

/// `tr_TR.ISO-8859-9` and `tr_TR@euro` → `tr_TR`.
fn strip_codeset(raw: &str) -> &str {
    let raw = raw.trim();
    raw.split(['.', '@']).next().unwrap_or(raw)
}

/// Step 2: trim, cut the codeset/modifier, `_` → `-`, parse and canonicalize (`iw` → `he`).
fn normalize(raw: &str, source: &str, diags: &mut Vec<NegDiag>) -> Option<icu_locale::Locale> {
    let bare = strip_codeset(raw);
    if bare.is_empty() {
        return None;
    }
    match icu_locale::Locale::try_from_str(&bare.replace('_', "-")) {
        Ok(mut locale) => {
            icu_locale::LocaleCanonicalizer::new_extended().canonicalize(&mut locale);
            Some(locale)
        }
        Err(_) => {
            diags.push(NegDiag::Unparsable {
                source: source.to_string(),
                value: raw.to_string(),
            });
            None
        }
    }
}

/// Step 3: the best catalog for one request. Likely-subtag scoring: 4 exact tag; 3 same
/// language, script and region; 2 same language and script, generic catalog; 1 regional
/// sibling. Never across languages or scripts. Ties: stable > beta > experimental, then the
/// more generic tag, then lexical order.
fn best_match<'a>(
    req: &icu_locale::LanguageIdentifier,
    avail: &'a [CatalogLocale],
    allow_experimental: bool,
) -> Option<&'a CatalogLocale> {
    let req_tag = req.to_string();
    let mut req_max = req.clone();
    icu_locale::LocaleExpander::new_extended().maximize(&mut req_max);
    let lang = req_max.language.as_str();
    let script = req_max.script.as_ref().map(|s| s.as_str());
    let region = req_max.region.as_ref().map(|r| r.as_str());

    avail
        .iter()
        .filter(|c| allow_experimental || c.status != Status::Experimental)
        .filter_map(|c| {
            if c.max.language.as_str() != lang
                || c.max.script.as_ref().map(|s| s.as_str()) != script
            {
                return None;
            }
            let score = if c.tag.to_string() == req_tag {
                4
            } else if c.max.region.as_ref().map(|r| r.as_str()) == region {
                3
            } else if c.tag.region.is_none() {
                2
            } else {
                1
            };
            Some((c, score))
        })
        .min_by_key(|(c, score)| {
            let status = match c.status {
                Status::Stable => 0,
                Status::Beta => 1,
                Status::Experimental => 2,
            };
            let subtags = c.tag.to_string().split('-').count();
            (Reverse(*score), status, subtags, c.tag.to_string())
        })
        .map(|(c, _)| c)
}

/// Step 7, regional sources only: `ui.regional_format` → the OS regional format → the first
/// regional request (OS list, `LC_ALL`, `LC_NUMERIC`, `LC_TIME`, `LANG`) → `en-US`. An explicit
/// UI language (`ui.language`, `HACIVAT_LANG`) never sets it. A tag without a region gets one
/// from likely subtags; extensions are kept (`tr-TR-u-nu-arab`).
fn format_locale(ctx: &NegotiationCtx, requests: &[Request], diags: &mut Vec<NegDiag>) -> String {
    for (value, source) in [
        (&ctx.format_pref, "regional_format"),
        (&ctx.os_format, "os_format"),
    ] {
        if let Some(locale) = value.as_deref().and_then(|v| normalize(v, source, diags)) {
            return with_region(locale);
        }
    }
    requests
        .iter()
        .find(|r| r.regional && !is_pseudo(&r.locale.id.to_string()))
        .map_or_else(|| "en-US".to_string(), |r| with_region(r.locale.clone()))
}

/// `tr` → `tr-TR`, `zh-Hant` → `zh-Hant-TW`; a tag with a region is returned as is.
fn with_region(mut locale: icu_locale::Locale) -> String {
    if locale.id.region.is_none() {
        let mut max = locale.id.clone();
        icu_locale::LocaleExpander::new_extended().maximize(&mut max);
        locale.id.region = max.region;
    }
    locale.to_string()
}
