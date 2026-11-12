//! Parsing pasted connection text: native URL forms and templates, with the password split off
//! as a secret (HV-DRV-04). Bodies in Task 7.

use hacivat_net::TlsMode;
use hacivat_proto::DbmsKind;
use secrecy::SecretString;

use super::{FieldId, FieldMap};

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

/// Parses `input`, preferring `preferred`'s forms when ambiguous (Task 7).
pub fn parse(_input: &str, _preferred: Option<DbmsKind>) -> ParseResult {
    ParseResult::Unrecognised(UnrecognisedReason::NoMatch)
}

/// A cheap check: does `text` look like connection text worth parsing (Task 7)?
pub fn looks_like_connection(_text: &str) -> bool {
    false
}
