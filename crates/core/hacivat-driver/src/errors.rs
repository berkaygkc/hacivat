//! Error classification: raw driver errors become a class, a retry flag and quick fixes, by
//! code and never by localized server text (HV-CONN-03).
//!
//! The registry (every class, whether M2a reaches it, and its quick fixes) and the code tables
//! live in `catalog/errors.toml`, embedded at build time and parsed once.

use std::collections::BTreeMap;
use std::io;
use std::sync::OnceLock;

use hacivat_net::{TlsFailure, TlsSetupError};
use hacivat_proto::{DbmsKind, ErrClass, FixId};
use serde::Deserialize;

/// The connect-pipeline phase an error happened in (HV-CONN-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Phase {
    Validate,
    PreHooks,
    CredentialFiles,
    NameResolution,
    Transport,
    Tls,
    Auth,
    DriverConnect,
    PostConnect,
}

/// The server's own error, shown verbatim in its own field (I18N-14, HV-CONN-03).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerMessage {
    pub sqlstate: Option<String>,
    pub vendor_code: Option<i64>,
    pub severity: Option<String>,
    pub text: String,
    pub detail: Option<String>,
    pub hint: Option<String>,
    /// The server's message language, when known.
    pub lang_hint: Option<String>,
}

/// What kind of failure a driver saw (HV-CONN-03).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RawErrorKind {
    /// The server answered with an error ([`RawError::server`] has it).
    Db,
    Dns,
    Io(io::ErrorKind),
    Tls(TlsFailure),
    /// TLS was required and the server does not offer it (P9).
    TlsNotOffered,
    Timeout,
    Protocol,
    Cancelled,
    Unsupported,
    /// The server asked for a password and the profile supplied none (PostgreSQL's
    /// authentication request without a password). It classifies as `auth.invalid`: M2a has no
    /// `auth.password_required` class, and `auth.invalid` already offers "Re-enter password",
    /// the prompt. MySQL needs no kind: a missing password reaches the server, which answers
    /// 1045 "using password: NO" (`auth.invalid` by code).
    PasswordRequired,
    Other,
}

/// A driver error before classification (HV-CONN-03).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RawError {
    pub kind: RawErrorKind,
    pub server: Option<ServerMessage>,
}

/// A classified error (HV-CONN-03).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Classified {
    pub class: ErrClass,
    pub retryable: bool,
    pub fixes: Vec<FixId>,
}

/// Classifies `err` from `kind`'s driver in `phase` (HV-CONN-03 steps 1–5, 7 and 8).
///
/// A driver flattens its cause chain into one [`RawError`]: the server's message is the
/// innermost cause, so it is read first.
/// 1. A cancel is the user's own doing and wins over everything else.
/// 2. The server's code, through the engine's tables (`errors.toml`). Never its text.
/// 3. The transport: DNS, socket errors, TLS failures, timeouts. Any I/O failure while resolving
///    names is `network.dns`.
/// 4. Fragile English text rules, only for an error with no code at all, and only when the
///    server's language is unknown or English.
/// 5. Otherwise `internal`; the surface shows the server's text verbatim.
///
/// The retry flag and the fixes follow the class (steps 7 and 8).
pub fn classify(kind: DbmsKind, phase: Phase, err: &RawError) -> Classified {
    classified(class_of(kind, phase, err))
}

/// Classifies a TLS setup problem found before connecting: an unreadable CA, certificate or key
/// file, an encrypted key, or a host that is neither an IP literal nor a DNS name. Each one is
/// in the profile, so it is `config.invalid`.
pub fn classify_tls_setup(err: &TlsSetupError) -> Classified {
    let class = match err {
        TlsSetupError::CaFileUnreadable(_)
        | TlsSetupError::CertFileUnreadable(_)
        | TlsSetupError::KeyUnreadable(_)
        | TlsSetupError::KeyEncrypted(_)
        | TlsSetupError::InvalidServerName(_) => ErrClass::ConfigInvalid,
    };
    classified(class)
}

/// The quick fixes of `class`, in button order, from `errors.toml`.
pub fn fixes_for(class: ErrClass) -> Vec<FixId> {
    registry()
        .classes
        .iter()
        .find(|c| c.id == class)
        .map_or_else(default_fixes, |c| c.fixes.clone())
}

/// The classes M2a can produce, in taxonomy order, from `errors.toml`.
pub fn reachable_classes() -> Vec<ErrClass> {
    registry()
        .classes
        .iter()
        .filter(|c| c.reachable)
        .map(|c| c.id)
        .collect()
}

/// The class IDs of `errors.toml`, in file order (one per `ErrClass::ALL`).
pub fn registry_ids() -> Vec<&'static str> {
    registry().classes.iter().map(|c| c.id.id()).collect()
}

fn classified(class: ErrClass) -> Classified {
    Classified {
        class,
        retryable: class.retryable_on_reconnect(),
        fixes: fixes_for(class),
    }
}

fn class_of(kind: DbmsKind, phase: Phase, err: &RawError) -> ErrClass {
    if err.kind == RawErrorKind::Cancelled {
        return ErrClass::Cancelled;
    }
    if let Some(class) = err.server.as_ref().and_then(|s| by_code(kind, s)) {
        return class;
    }
    if let Some(class) = by_transport(phase, &err.kind) {
        return class;
    }
    if let Some(class) = err.server.as_ref().and_then(by_english_text) {
        return class;
    }
    ErrClass::Internal
}

/// Step 2: the engine's code tables.
fn by_code(kind: DbmsKind, server: &ServerMessage) -> Option<ErrClass> {
    let codes = &registry().codes;
    let lookup = |table: &str, code: &str| codes.get(table).and_then(|t| t.get(code)).copied();
    match kind {
        DbmsKind::Postgres => {
            let state = server.sqlstate.as_deref()?.to_ascii_uppercase();
            lookup("postgres", &state)
        }
        DbmsKind::MySql => lookup("mysql", &server.vendor_code?.to_string()),
        DbmsKind::MariaDb => {
            let code = server.vendor_code?.to_string();
            lookup("mariadb", &code).or_else(|| lookup("mysql", &code))
        }
        // An extended result code keeps its primary code in the low byte.
        DbmsKind::Sqlite => lookup("sqlite", &(server.vendor_code? & 0xff).to_string()),
        _ => None,
    }
}

/// Step 3: transport and TLS rules.
fn by_transport(phase: Phase, kind: &RawErrorKind) -> Option<ErrClass> {
    use io::ErrorKind as K;
    Some(match kind {
        RawErrorKind::Dns => ErrClass::NetworkDns,
        RawErrorKind::Timeout => ErrClass::NetworkTimeout,
        RawErrorKind::TlsNotOffered => ErrClass::TlsNotSupportedByServer,
        RawErrorKind::PasswordRequired => ErrClass::AuthInvalid,
        RawErrorKind::Tls(failure) => match failure {
            TlsFailure::UnknownIssuer | TlsFailure::Expired | TlsFailure::NotYetValid => {
                ErrClass::TlsUntrusted
            }
            TlsFailure::NameMismatch => ErrClass::TlsHostname,
            TlsFailure::Protocol => ErrClass::TlsProtocol,
            TlsFailure::ClientCertRequired => ErrClass::TlsClientCertRequired,
            TlsFailure::Other => ErrClass::TlsHandshake,
        },
        RawErrorKind::Io(io) => match io {
            K::ConnectionRefused => ErrClass::NetworkRefused,
            K::TimedOut => ErrClass::NetworkTimeout,
            K::ConnectionReset | K::ConnectionAborted | K::BrokenPipe | K::UnexpectedEof => {
                ErrClass::NetworkReset
            }
            K::HostUnreachable | K::NetworkUnreachable | K::NetworkDown | K::AddrNotAvailable => {
                ErrClass::NetworkUnreachable
            }
            _ if phase == Phase::NameResolution => ErrClass::NetworkDns,
            _ => return None,
        },
        RawErrorKind::Other if phase == Phase::NameResolution => ErrClass::NetworkDns,
        RawErrorKind::Db
        | RawErrorKind::Protocol
        | RawErrorKind::Cancelled
        | RawErrorKind::Unsupported
        | RawErrorKind::Other => return None,
    })
}

/// Step 4: fragile English text rules, only for a server error that carries no code at all (a
/// coded error the tables do not know stays `internal`), and only when the server speaks English
/// (or did not say).
///
/// The `[fragile]` needles are tried in alphabetical order (a `BTreeMap`) and the first match
/// wins, so they must stay disjoint: no needle may contain another (a unit test checks this).
fn by_english_text(server: &ServerMessage) -> Option<ErrClass> {
    if server.sqlstate.is_some() || server.vendor_code.is_some() {
        return None;
    }
    let english = server.lang_hint.as_deref().is_none_or(|tag| {
        tag.split(['-', '_'])
            .next()
            .is_some_and(|lang| lang.eq_ignore_ascii_case("en"))
    });
    if !english {
        return None;
    }
    let text = server.text.to_ascii_lowercase();
    registry()
        .fragile
        .iter()
        .find(|(needle, _)| text.contains(needle.as_str()))
        .map(|(_, class)| *class)
}

/// `catalog/errors.toml`, parsed.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    #[serde(rename = "class")]
    classes: Vec<ClassEntry>,
    /// Table name → code → class.
    codes: BTreeMap<String, BTreeMap<String, ErrClass>>,
    /// English text fragment → class.
    fragile: BTreeMap<String, ErrClass>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClassEntry {
    id: ErrClass,
    reachable: bool,
    #[serde(default = "default_fixes")]
    fixes: Vec<FixId>,
}

fn default_fixes() -> Vec<FixId> {
    vec![FixId::EditConnection]
}

const ERRORS_TOML: &str = include_str!("../catalog/errors.toml");

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut r: Registry = toml::from_str(ERRORS_TOML)
            .unwrap_or_else(|e| panic!("catalog/errors.toml is invalid: {e}"));
        r.fragile = r
            .fragile
            .into_iter()
            .map(|(needle, class)| (needle.to_ascii_lowercase(), class))
            .collect();
        r
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_registry_parses() {
        let r = registry();
        assert_eq!(r.classes.len(), ErrClass::ALL.len());
        assert_eq!(
            r.codes.keys().map(String::as_str).collect::<Vec<_>>(),
            ["mariadb", "mysql", "postgres", "sqlite"]
        );
    }

    #[test]
    fn every_table_entry_is_reachable() {
        let reachable = reachable_classes();
        let r = registry();
        for (table, codes) in &r.codes {
            for (code, class) in codes {
                assert!(reachable.contains(class), "{table} {code} -> {class:?}");
            }
        }
        for (needle, class) in &r.fragile {
            assert!(reachable.contains(class), "{needle} -> {class:?}");
            assert!(needle.is_ascii(), "{needle}");
        }
    }

    #[test]
    fn fragile_needles_are_disjoint() {
        let needles: Vec<&String> = registry().fragile.keys().collect();
        for a in &needles {
            for b in &needles {
                assert!(a == b || !a.contains(b.as_str()), "`{a}` contains `{b}`");
            }
        }
    }

    #[test]
    fn unknown_keys_and_ids_are_rejected() {
        assert!(toml::from_str::<Registry>("class = []\ncodes = {}\nfragile = {}\nx = 1").is_err());
        assert!(
            toml::from_str::<Registry>(
                "codes = {}\nfragile = {}\n[[class]]\nid = \"nope\"\nreachable = true"
            )
            .is_err()
        );
        let one: Registry = toml::from_str(
            "codes = {}\nfragile = {}\n[[class]]\nid = \"internal\"\nreachable = true",
        )
        .unwrap();
        assert_eq!(one.classes[0].fixes, [FixId::EditConnection]);
    }
}
