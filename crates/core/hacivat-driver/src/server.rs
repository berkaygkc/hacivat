//! Server detection: loose versions, what a server says it is, and whether Hacivat supports it
//! (HV-DRV-01, C10). Bodies in Task 2.

use hacivat_proto::DbmsKind;

use crate::catalog::EngineCatalog;

/// A version parsed loosely, keeping the raw string (`16.15 (Debian ...)`) (HV-DRV-01).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SemVerLoose {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub raw: String,
}

impl SemVerLoose {
    /// Parses the leading `major[.minor[.patch]]` (Task 2).
    pub fn parse(_s: &str) -> Option<Self> {
        None
    }
}

/// What the server reported after connecting (HV-DRV-01).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerInfo {
    pub kind: DbmsKind,
    pub product: String,
    pub version: Option<SemVerLoose>,
    pub raw_version: String,
    pub is_replica: Option<bool>,
}

/// Whether a server version is supported (C10).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Support {
    Supported,
    /// Above the floor but not in the tested list: connect with a warning.
    NotTested,
    /// Below the floor: `server.unsupported_version`.
    Unsupported {
        floor: String,
    },
}

/// The support status of `info` in `catalog`; an engine without a floor is `Supported` (P4,
/// Task 2). The stub answers `Supported`.
pub fn support_status(_catalog: &EngineCatalog, _info: &ServerInfo) -> Support {
    Support::Supported
}

/// MySQL, MariaDB or TiDB from `@@version` and `@@version_comment` (Task 2). The stub answers
/// `MySql`.
pub fn detect_mysql_family(_version: &str, _version_comment: &str) -> DbmsKind {
    DbmsKind::MySql
}
