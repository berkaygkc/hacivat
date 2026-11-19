//! The system-catalog layer (HV-INTRO-16 consumer side; L7, D-017, R-M4-6, R-M4-33): the
//! built-in objects of each supported server version, decoded from the shipped snapshots in
//! `assets/system-catalogs/<dbms>-<ver>.hvsnap.zst`. Selection is exact (PostgreSQL by major,
//! MySQL and MariaDB by major.minor); SQLite never uses a snapshot.
//!
//! M4 Task 1a seam: [`SystemCatalogs::for_server`] answers `None` (system lookups `Unknown`)
//! until Task 26; decoding refuses until Task 23.

#[cfg(feature = "system")]
pub mod decode;
#[cfg(feature = "embedded-system")]
pub mod embedded;
#[cfg(feature = "system")]
pub mod normalize;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use hacivat_proto::DbmsKind;

#[cfg(feature = "system")]
pub use decode::decode;
#[cfg(feature = "system")]
pub use normalize::normalize;

use crate::ids::NsKey;
use crate::name::Name;
use crate::namespace::NamespaceSnapshot;

/// The built-in objects of one server line, as namespaces flagged `PRE_INTROSPECTED`.
#[derive(Debug, Clone)]
pub struct SystemSnapshot {
    pub dbms: DbmsKind,
    /// The snapshot's line (`17`, `8.4`, `11.4`).
    pub line: Name,
    /// The exact server version it was generated from.
    pub server_version: Name,
    /// The canonical JSON format number (1).
    pub format: u32,
    pub namespaces: BTreeMap<NsKey, Arc<NamespaceSnapshot>>,
}

/// Why a snapshot set cannot be read.
#[derive(Debug)]
pub enum SystemError {
    Io(std::io::Error),
    /// Not a snapshot, a bad frame, a checksum or digest mismatch, a truncated asset.
    Corrupt(&'static str),
    /// A format number this build does not read.
    UnknownFormat(u32),
    /// Not written yet (M4 Task 1a stub).
    NotImplemented,
}

impl fmt::Display for SystemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SystemError::Io(e) => write!(f, "system catalog i/o: {e}"),
            SystemError::Corrupt(why) => write!(f, "corrupt system catalog: {why}"),
            SystemError::UnknownFormat(n) => write!(f, "system catalog format {n}"),
            SystemError::NotImplemented => write!(f, "not implemented"),
        }
    }
}

impl std::error::Error for SystemError {}

/// The snapshot set an app ships or a test opens.
#[derive(Debug, Clone, Default)]
pub struct SystemCatalogs {
    by_line: BTreeMap<(DbmsKind, Name), Arc<SystemSnapshot>>,
}

impl SystemCatalogs {
    /// A set with no snapshot: every server is live-introspected or `Unknown` offline.
    pub fn empty() -> Self {
        SystemCatalogs::default()
    }

    /// The snapshots in `dir` (a `manifest.toml` and its `.hvsnap.zst` files).
    ///
    /// Task 1a stub: `Err(SystemError::NotImplemented)`.
    #[cfg(feature = "system")]
    pub fn open(dir: &std::path::Path) -> Result<Self, SystemError> {
        let _ = dir;
        Err(SystemError::NotImplemented)
    }

    /// The snapshots embedded in the binary (feature `embedded-system`, the app only).
    ///
    /// Task 1a stub: the empty set (the embedded table is empty until Task 23 ships assets).
    #[cfg(feature = "embedded-system")]
    pub fn from_embedded() -> Self {
        let _ = embedded::SNAPSHOTS;
        SystemCatalogs::empty()
    }

    /// The snapshot of `dbms` at `version` (exact line match, R-M4-6), if shipped.
    ///
    /// Task 1a stub: `None`.
    pub fn for_server(&self, dbms: DbmsKind, version: &str) -> Option<Arc<SystemSnapshot>> {
        let _ = (&self.by_line, dbms, version);
        None
    }
}
