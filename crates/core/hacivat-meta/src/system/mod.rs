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
// ## m4-t23-minicat (named exception in Task 26's module): the checked JSON payload.
#[cfg(feature = "system")]
pub use decode::decode_json;
#[cfg(feature = "system")]
pub use normalize::normalize;

use crate::ids::NsKey;
use crate::name::Name;
use crate::namespace::NamespaceSnapshot;
// ## m4-t23-minicat
use crate::namespace::Obj;
use sql_catalog::SettingInfo;

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
    // ## m4-t23-minicat (named exception in Task 26's module): the side tables of a snapshot.
    /// MySQL/MariaDB built-in functions (the generator's name probe): schema-less `Routine`
    /// objects, flagged `SYSTEM | PRE_INTROSPECTED`, with `args = None` (name only, never "no
    /// arguments"). Empty for PostgreSQL, whose built-ins live in `pg_catalog`.
    pub builtins: Vec<Obj>,
    /// PostgreSQL operators with their operand, result and function types.
    pub operators: Arc<[SystemOperator]>,
    /// PostgreSQL types with their SQL spelling, kind, category and array element.
    pub types: Arc<[SystemType]>,
    /// Setting names (PostgreSQL GUCs with type and context; MySQL/MariaDB variables, MariaDB
    /// with scope as `context` and type), never their values; sorted by name.
    pub settings: Arc<[SettingInfo]>,
}

// ## m4-t23-minicat (named exception in Task 26's module)
/// One built-in operator of a snapshot (PostgreSQL).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemOperator {
    pub schema: Name,
    pub name: Name,
    /// `infix`, `prefix` or `postfix`.
    pub kind: Name,
    pub left: Option<Name>,
    pub right: Option<Name>,
    pub result: Name,
    /// The implementing function, as the server prints it.
    pub function: Name,
    pub comment: Option<Name>,
}

/// One built-in type of a snapshot (PostgreSQL).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemType {
    pub schema: Name,
    pub name: Name,
    /// The SQL spelling (`double precision` for `float8`).
    pub sql_name: Name,
    /// `base`, `composite`, `domain`, `enum`, `pseudo`, `range` or `multirange`.
    pub kind: Name,
    /// `pg_type.typcategory`, one letter.
    pub category: Name,
    /// The element type of an array type.
    pub element: Option<Name>,
    pub comment: Option<Name>,
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
