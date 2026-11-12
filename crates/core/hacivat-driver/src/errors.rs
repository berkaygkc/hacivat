//! Error classification: raw driver errors become a class, a retry flag and quick fixes, by
//! code and never by localized server text (HV-CONN-03). Bodies in Task 10.

use std::io;

use hacivat_net::TlsFailure;
use hacivat_proto::{DbmsKind, ErrClass, FixId};

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

/// Classifies `err` from `kind`'s driver in `phase` (Task 10). The stub answers `internal`.
pub fn classify(_kind: DbmsKind, _phase: Phase, _err: &RawError) -> Classified {
    Classified {
        class: ErrClass::Internal,
        retryable: false,
        fixes: Vec::new(),
    }
}

/// The quick fixes of `class`, from `errors.toml` (Task 10).
pub fn fixes_for(_class: ErrClass) -> Vec<FixId> {
    Vec::new()
}

/// The classes M2a can produce, from `errors.toml` (Task 10).
pub fn reachable_classes() -> Vec<ErrClass> {
    Vec::new()
}
