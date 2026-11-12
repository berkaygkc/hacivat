//! Parsing pasted connection text: native URL forms and templates, with the password split off
//! as a secret (HV-DRV-04).
//!
//! Recognizers, in order:
//! 1. **Native forms** (`native`): `postgres://`/`postgresql://` URIs (multi-host included),
//!    the libpq DSN, `mysql://`/`mariadb://`, `jdbc:` (stripped and re-routed), and SQLite
//!    paths, `file:` URIs and `:memory:`. The scheme names the engine, so a URL for another
//!    engine than the preferred one is reported with its own engine (the caller proposes the
//!    switch, never silently).
//! 2. **Catalog templates** (`matcher`), the preferred engine's first, then the others'; within
//!    a group the match with the fewest extras wins, then catalog order.
//!
//! The input is trimmed; over 64 KiB it is `TooLong`, unread. Every value is decoded exactly
//! once. The password, from the user info or a password-like parameter, ends up only in
//! `secrets`. Parsing is locale-invariant (ASCII case folding only). The password field never
//! reaches this parser: masked inputs never call the paste filter (R21).

use std::sync::OnceLock;

use hacivat_net::TlsMode;
use hacivat_proto::DbmsKind;
use secrecy::SecretString;

use super::matcher::{Matched, match_template_with_password};
use super::native::{self, strip_ci};
use super::{FieldId, FieldMap, Template, compile};
use crate::EngineCatalog;

/// The longest text `parse` reads.
const MAX_INPUT: usize = 64 * 1024;

/// What a recognised URL parameter maps to in the profile (HV-DRV-04).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MappedTo {
    TlsMode(TlsMode),
    ApplicationName,
    /// Seconds.
    ConnectTimeout(u32),
    ReadOnly(bool),
    PgOptions,
}

/// One URL parameter that became a profile setting (HV-DRV-04).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MappedParam {
    pub key: String,
    pub value: String,
    pub to: MappedTo,
}

/// Something the parser noticed (HV-DRV-04).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ParseWarning {
    DuplicateKey(String),
    /// The URL carried a password; it moved to the password field.
    PasswordMoved,
    MultipleHosts {
        used: String,
        dropped: usize,
    },
}

/// A recognised connection text (HV-DRV-04). Holds secrets, so `Debug` only.
#[derive(Debug)]
pub struct ParseOutcome {
    pub kind: DbmsKind,
    pub conn_type: String,
    pub fields: FieldMap,
    pub extra: Vec<(String, String)>,
    pub secrets: Vec<(FieldId, SecretString)>,
    pub mapped: Vec<MappedParam>,
    pub warnings: Vec<ParseWarning>,
}

/// Why a text is not a connection (HV-DRV-04).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnrecognisedReason {
    Empty,
    TooLong,
    NoMatch,
    /// A DNS SRV URL with a port.
    SrvPort,
}

/// The result of [`parse`] (HV-DRV-04). Holds secrets, so `Debug` only (P6).
#[derive(Debug)]
pub enum ParseResult {
    Recognised(ParseOutcome),
    Unrecognised(UnrecognisedReason),
}

impl ParseResult {
    /// Why the text was not recognised, or `None` when it was (for asserts, P6).
    pub fn unrecognised(&self) -> Option<UnrecognisedReason> {
        match self {
            ParseResult::Recognised(_) => None,
            ParseResult::Unrecognised(r) => Some(*r),
        }
    }
}

/// Parses `input`, preferring `preferred`'s templates when the text has no native form.
pub fn parse(input: &str, preferred: Option<DbmsKind>) -> ParseResult {
    if input.len() > MAX_INPUT {
        return ParseResult::Unrecognised(UnrecognisedReason::TooLong);
    }
    let text = input.trim();
    if text.is_empty() {
        return ParseResult::Unrecognised(UnrecognisedReason::Empty);
    }
    match native::parse(text).or_else(|| by_template(text, preferred)) {
        Some(o) => ParseResult::Recognised(o),
        None => ParseResult::Unrecognised(UnrecognisedReason::NoMatch),
    }
}

/// A cheap check: does `text` look like connection text worth parsing (R21, the mock's
/// `looksLikeConn`)? True for `^\s*(jdbc:)?(postgres|postgresql|mysql|mariadb)://` and
/// `^\s*file:` (ASCII case ignored), or `\b(host|dbname)\s*=\s*\S` (case as written).
pub fn looks_like_connection(text: &str) -> bool {
    let t = text.trim_start();
    let after_jdbc = strip_ci(t, "jdbc:").unwrap_or(t);
    ["postgres://", "postgresql://", "mysql://", "mariadb://"]
        .iter()
        .any(|s| strip_ci(after_jdbc, s).is_some())
        || strip_ci(t, "file:").is_some()
        || names_a_dsn_key(text)
}

/// `\b(host|dbname)\s*=\s*\S`, with `\b` an ASCII word boundary as in JavaScript.
fn names_a_dsn_key(text: &str) -> bool {
    ["host", "dbname"].iter().any(|word| {
        text.match_indices(word).any(|(i, _)| {
            let boundary = text[..i]
                .chars()
                .next_back()
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
            boundary
                && text[i + word.len()..]
                    .trim_start()
                    .strip_prefix('=')
                    .is_some_and(|v| !v.trim_start().is_empty())
        })
    })
}

/// A catalog template, compiled once.
struct Candidate {
    kind: DbmsKind,
    conn_type: String,
    template: Template,
}

/// Every non-empty catalog template, in catalog order. One that fails to compile is skipped
/// (`every_catalog_template_compiles_and_round_trips` keeps that from happening).
fn candidates() -> &'static [Candidate] {
    static CANDIDATES: OnceLock<Vec<Candidate>> = OnceLock::new();
    CANDIDATES.get_or_init(|| {
        EngineCatalog::embedded()
            .engines()
            .flat_map(|e| {
                e.conn_types
                    .iter()
                    .filter(|ct| !ct.template.is_empty())
                    .filter_map(move |ct| {
                        Some(Candidate {
                            kind: e.kind,
                            conn_type: ct.id.clone(),
                            template: compile(&ct.template, &ct.id).ok()?,
                        })
                    })
            })
            .collect()
    })
}

/// The template fallback: the preferred engine's templates, then the others'.
fn by_template(text: &str, preferred: Option<DbmsKind>) -> Option<ParseOutcome> {
    let all = candidates();
    preferred
        .and_then(|p| best(all.iter().filter(|c| c.kind == p), text))
        .or_else(|| best(all.iter().filter(|c| Some(c.kind) != preferred), text))
}

/// The match with the fewest extras among `cands`, the first on a tie.
fn best<'a>(cands: impl Iterator<Item = &'a Candidate>, text: &str) -> Option<ParseOutcome> {
    let mut best: Option<(&Candidate, Matched)> = None;
    for c in cands {
        if let Some(m) = match_template_with_password(&c.template, text)
            && best.as_ref().is_none_or(|(_, b)| m.1.len() < b.1.len())
        {
            best = Some((c, m));
        }
    }
    let (c, (map, extra, password)) = best?;
    native::from_template(c.kind, &c.conn_type, map, extra, password)
}
