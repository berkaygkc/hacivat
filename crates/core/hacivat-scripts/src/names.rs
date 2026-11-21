//! Logical-database and server database names (HV-SCRIPT-01 step 3, HV-SCRIPT-02).
//!
//! ## m4b-t01a-seams
//! Task 1a: the types. [`LogicalDb::parse`] is a fail-closed stub that refuses every name until
//! Task 3 writes the character rules (1–128 bytes, no C0/C1 controls, no bidi controls, NFC).

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The logical database a library binds (`database` or an alias in `workspace.toml`, or the
/// folder name): NFC, original case kept for display. Binding compares two of these with
/// NFC plus ASCII case-insensitive equality (Task 3), never with `==`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LogicalDb(pub(crate) Arc<str>);

/// Why a name was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameError {
    /// Task 1a's stub refuses every name.
    NotImplemented,
}

impl LogicalDb {
    /// A validated logical name from untrusted text.
    ///
    /// Task 1a stub (fail closed): refuses every name.
    pub fn parse(text: &str) -> Result<Self, NameError> {
        let _ = text;
        Err(NameError::NotImplemented)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LogicalDb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A database's exact name on its server, as the explorer shows it (`"main"` on SQLite,
/// R-M4b-32; the C-M4-4 fallback). A script tab binds to `(ProfileId, DatabaseName)` for its
/// whole life (R-M4b-4). Serialized as the plain string (`workspace.json`, Task 1b).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DatabaseName(Arc<str>);

impl DatabaseName {
    /// The server's own name; never validated, never joined to a path.
    pub fn new(exact: &str) -> Self {
        DatabaseName(exact.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DatabaseName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for DatabaseName {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for DatabaseName {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(DatabaseName(s.into()))
    }
}
// ## m4b-t01a-seams
