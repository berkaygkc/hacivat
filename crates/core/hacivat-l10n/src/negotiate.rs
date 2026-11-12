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
    /// An explicit user choice (`ui.language`): may pick an experimental catalog.
    explicit: bool,
    /// May supply the format locale: not `ui.language` (an explicit UI language never sets the
    /// format locale), not the message-only `LANGUAGE`/`LC_MESSAGES`.
    regional: bool,
    /// Index of the `PrefSource` it came from.
    source: usize,
}

/// Negotiates the UI chain and format locale (I18N-03 steps 1–7). The highest-priority source
/// that matches any catalog decides the chain (setting `en` with OS `tr-TR` gives `[en]`);
/// every source still counts for the format locale.
pub fn negotiate(ctx: &NegotiationCtx, avail: &[CatalogLocale]) -> Negotiated {
    let mut diags = Vec::new();
    let requests = collect(ctx, &mut diags);
    let has_en = avail.iter().any(|c| c.tag == unic_langid::langid!("en"));

    let mut chain: Vec<LanguageIdentifier> = Vec::new();
    let mut deciding: Option<usize> = None;
    for r in &requests {
        if deciding.is_some_and(|s| s != r.source) {
            break;
        }
        let before = chain.len();
        let id = r.locale.id.to_string();
        if is_pseudo(&id) {
            match id.parse() {
                Ok(tag) if ctx.allow_pseudo && has_en => push_unique(&mut chain, tag),
                _ => diags.push(NegDiag::NoMatch { requested: id }),
            }
            continue;
        }
        let allow_experimental = ctx.allow_experimental || r.explicit;
        match best_match(&r.locale.id, avail, allow_experimental) {
            Some(found) => {
                push_unique(&mut chain, found.tag.clone());
                // A regional catalog is followed by its language parent (`pt-BR` → `pt`).
                if found.tag.region.is_some() {
                    let mut parent = found.tag.clone();
                    parent.region = None;
                    if avail.iter().any(|c| {
                        c.tag == parent && (allow_experimental || c.status != Status::Experimental)
                    }) {
                        push_unique(&mut chain, parent);
                    }
                }
            }
            None => diags.push(NegDiag::NoMatch { requested: id }),
        }
        if chain.len() > before {
            deciding = Some(r.source);
        }
    }
    push_unique(&mut chain, unic_langid::langid!("en"));

    let format_locale = format_locale(ctx, &requests, &chain, &mut diags);
    Negotiated {
        chain,
        format_locale,
        diags,
    }
}

fn push_unique(chain: &mut Vec<LanguageIdentifier>, tag: LanguageIdentifier) {
    if !chain.contains(&tag) {
        chain.push(tag);
    }
}

/// Step 1–2: raw preferences in priority order, normalized and deduplicated.
fn collect(ctx: &NegotiationCtx, diags: &mut Vec<NegDiag>) -> Vec<Request> {
    let mut out: Vec<Request> = Vec::new();
    for (index, source) in ctx.sources.iter().enumerate() {
        let mut add =
            |raw: &str, name: &str, explicit: bool, regional: bool, diags: &mut Vec<NegDiag>| {
                let Some(locale) = normalize(raw, name, diags) else {
                    return;
                };
                if !out.iter().any(|r| r.locale == locale) {
                    out.push(Request {
                        locale,
                        explicit,
                        regional,
                        source: index,
                    });
                }
            };
        match source {
            PrefSource::Setting(tag) => add(tag, "setting", true, false, diags),
            PrefSource::Env(var, value) => {
                let regional = !matches!(*var, "LANGUAGE" | "LC_MESSAGES");
                for part in value.split(':') {
                    let bare = strip_codeset(part);
                    if bare == "C" || bare == "POSIX" {
                        diags.push(NegDiag::CLocale {
                            var: (*var).to_string(),
                        });
                        add("en", var, false, regional, diags);
                    } else {
                        add(part, var, false, regional, diags);
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

/// Step 7: `ui.regional_format` → the OS regional format → the first regional request →
/// `chain[0]`, each given a region from likely subtags when it has none. Extensions are kept
/// (`tr-TR-u-nu-arab`).
fn format_locale(
    ctx: &NegotiationCtx,
    requests: &[Request],
    chain: &[LanguageIdentifier],
    diags: &mut Vec<NegDiag>,
) -> String {
    for (value, source) in [
        (&ctx.format_pref, "regional_format"),
        (&ctx.os_format, "os_format"),
    ] {
        if let Some(locale) = value.as_deref().and_then(|v| normalize(v, source, diags)) {
            return with_region(locale);
        }
    }
    if let Some(r) = requests
        .iter()
        .find(|r| r.regional && !is_pseudo(&r.locale.id.to_string()))
    {
        return with_region(r.locale.clone());
    }
    chain
        .iter()
        .find(|t| !is_pseudo(&t.to_string()))
        .and_then(|t| icu_locale::Locale::try_from_str(&t.to_string()).ok())
        .map_or_else(|| "en-US".to_string(), with_region)
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
