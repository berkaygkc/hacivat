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
//!    a group the match with the fewest extras wins, then catalog order. Only a text that no
//!    native form claims gets here: a native refusal is final (a looser template once swallowed
//!    `&password=…` into a socket path). With today's catalog every template starts with a
//!    native scheme, so this recognizer serves future engines' templates.
//!
//! The input is trimmed; over 64 KiB it is `TooLong`, unread. Every value is decoded exactly
//! once. The password, from the user info or a password-like parameter, ends up only in
//! `secrets`, and an `sslpassword` only in `tls_key_passphrase`. Parsing is locale-invariant
//! (ASCII case folding only). The password field never reaches this parser: masked inputs never
//! call the paste filter (R21).

use std::sync::OnceLock;

use hacivat_net::TlsMode;
use hacivat_proto::DbmsKind;
use secrecy::SecretString;

use super::matcher::{Matched, match_template_with_password};
use super::native::{self, Claim, strip_ci};
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
    /// The TLS client key passphrase from an `sslpassword` parameter (any ASCII case, after
    /// decoding), for `SecretSlot::TlsKeyPassphrase`. Never kept in `extra` (T7 fix round 1).
    pub tls_key_passphrase: Option<SecretString>,
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

/// Parses `input`, preferring `preferred`'s templates when no native form claims the text.
pub fn parse(input: &str, preferred: Option<DbmsKind>) -> ParseResult {
    if input.len() > MAX_INPUT {
        return ParseResult::Unrecognised(UnrecognisedReason::TooLong);
    }
    let text = input.trim();
    if text.is_empty() {
        return ParseResult::Unrecognised(UnrecognisedReason::Empty);
    }
    let found = match native::parse(text) {
        Claim::Claimed(verdict) => verdict,
        Claim::Unclaimed => by_template(text, preferred),
    };
    match found {
        // The outcome invariant (T7 fix round 2): a value that still carries a password-like
        // `key=` means some recognizer swallowed a secret into it; the text is refused rather
        // than stored with the secret in the clear. The public reason stays `NoMatch`.
        Some(o) if native::leaks_a_secret(&o) => {
            ParseResult::Unrecognised(UnrecognisedReason::NoMatch)
        }
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

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;
    use crate::url::native::{carries_secret, is_network_path};

    // The template fallback is unreachable through `parse` for texts with a native scheme (the
    // claim rule), so these call it directly: its own guard must hold on its own.

    #[test]
    fn the_template_path_refuses_a_swallowed_password() {
        // Failure messages name the case by index: the texts hold a fixture secret.
        for (i, text) in [
            "postgresql:///db?host=/sock&password=SECRETPW&flag",
            "postgresql://u@/db?host=/sock&password=SECRETPW&",
            "mysql://u@localhost/db?socket=/tmp/s&pwd=SECRETPW&x",
            "mariadb://u@localhost/db?socket=/tmp/s&PASS%2577ord=SECRETPW&x",
            "postgresql:///db?host=/sock;sslpassword=SECRETPW&flag",
        ]
        .into_iter()
        .enumerate()
        {
            assert!(by_template(text, None).is_none(), "case {i}");
            assert!(
                by_template(text, Some(DbmsKind::Postgres)).is_none(),
                "case {i}"
            );
        }
    }

    #[test]
    fn the_template_path_refuses_network_files() {
        for text in [
            "file://server/share/a.db",
            "file:////server/share/a.db",
            r"file:\\server\share\a.db",
            r"file:\\?\UNC\server\share\a.db",
            r"file:/\server\share\a.db",
        ] {
            assert!(
                by_template(text, Some(DbmsKind::Sqlite)).is_none(),
                "{text}"
            );
        }
        let local = by_template("file:/tmp/a.db", Some(DbmsKind::Sqlite)).unwrap();
        assert_eq!(local.fields.values[&FieldId::File], "/tmp/a.db");
    }

    #[test]
    fn the_template_path_moves_both_secrets() {
        let o = by_template(
            "postgresql://u:s3cret@h/db?a=b#c&pass%77ord=x2&SSLPASSWORD=kp",
            Some(DbmsKind::Postgres),
        )
        .unwrap();
        assert_eq!(
            (o.kind, o.conn_type.as_str()),
            (DbmsKind::Postgres, "default")
        );
        assert_eq!(o.extra, vec![("a".to_string(), "b#c".to_string())]);
        assert!(o.secrets.len() == 1 && o.secrets[0].1.expose_secret() == "x2");
        assert!(o.tls_key_passphrase.as_ref().unwrap().expose_secret() == "kp");
        let shown = format!("{o:?}");
        for s in ["s3cret", "x2", "kp"] {
            assert!(!shown.contains(s));
        }
    }

    #[test]
    fn swallowed_secret_shapes() {
        for v in [
            "/sock&password=x",
            "/sock&pwd=",
            "?Pass=x",
            ";sslpassword=x",
            "password=x",
            "host=h password=x a.db",
            "/s&PASS%77ord=x",
            "/s&PASS%2577ord=x",
            // Round 2: whitespace around `=`, `,`, an encoded `=` or separator.
            "host=h password = x x.db",
            "Server=h;Pwd =x;a.db",
            "Server=h; Pwd = x; a.db",
            "a,passwd=x",
            "x=password=1",
            "PassWord%3Dx",
            "/sock%26PassWord%3Dx",
            "a\tpassword\t=\tx",
            // Round 3: Unicode whitespace, and any non-key character as a boundary.
            "host=h\u{a0}password=x",
            "host=h password\u{a0}=x",
            "Server=h;\u{a0}Pwd=x",
            "x\u{3000}password\u{3000}=x",
            "x%C2%A0password=x",
            "x%E3%80%80password=x",
            "'password=x'",
            "\"password\"=x",
            "'password' = x",
            "{password=x}",
            "a|password=x",
            "a:password=x",
            "/tmp/password=x",
            "x+password=x",
            "/tmp/password%3D",
            "/tmp/password=1.db",
            "şifre password=x",
        ] {
            assert!(carries_secret(v), "{v}");
        }
        for v in [
            "/tmp/a=b",
            "/var/run/postgresql",
            "/run/passwordless",
            "password_db",
            "/s&passwords=x",
            "/s&mypassword=x",
            "/s&password",
            "x=password",
            "-c search_path=public",
            "-c password_encryption=scram-sha-256",
            "password-reset-service",
            "app=password-reset-service",
            "şifre_db",
            "/tmp/şifre.db",
            "şifre=x",
            "x.password_hash=1",
            "",
            "=",
            " = = ",
            "\u{a0}=\u{3000}=",
        ] {
            assert!(!carries_secret(v), "{v}");
        }
    }

    #[test]
    fn network_path_shapes() {
        for p in [
            "//h/s",
            r"\\h\s",
            r"\\?\UNC\h\s",
            r"\\.\pipe",
            r"\??\UNC\h\s\a.db",
            r"/??/UNC/h/s/a.db",
            r"\??/C:\a.db",
            r"/\h",
        ] {
            assert!(is_network_path(p), "{p}");
        }
        for p in [
            "/tmp/a.db",
            r"C:\a.db",
            "?",
            r"\?",
            r"\??",
            r"\??x",
            "a.db",
            "",
        ] {
            assert!(!is_network_path(p), "{p}");
        }
    }

    #[test]
    fn the_scan_is_linear() {
        let start = std::time::Instant::now();
        let n = 64 * 1024;
        for v in [
            "=".repeat(n),
            " =".repeat(n / 2),
            "a=".repeat(n / 2),
            "%3D".repeat(n / 3),
            "\u{a0}=".repeat(n / 3),
            format!("{}=", "a".repeat(n)),
            format!("{}=", "\u{3000}".repeat(n / 3)),
            "x\u{3000}".repeat(n / 4),
        ] {
            assert!(!carries_secret(&v));
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }
}
