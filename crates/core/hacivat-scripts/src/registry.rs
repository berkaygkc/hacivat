//! The library registry `<config>/libraries.toml` (HV-SCRIPT-01 steps 1, 2, 5; R-M4b-2).
//!
//! ## m4b-t01a-seams
//! Task 1a: the types. Task 3 writes the registry (one writer, `fsio::atomic_write`, 0600).

use std::fmt;
use std::path::PathBuf;

use hacivat_proto::ProfileId;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use crate::names::DatabaseName;

/// A registered library's stable ID (a v4 UUID, never reused). Serialized as its hyphenated
/// text (`libraries.toml`, `workspace.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LibraryId(pub Uuid);

// ## m4b-t01a-fr1 (review M4): no `Default`. A `#[serde(default)]` or `..Default::default()`
// would silently mint a new library identity; a fresh ID is always an explicit `new()`.
#[expect(
    clippy::new_without_default,
    reason = "a library identity is never a default value (T1a review M4)"
)]
impl LibraryId {
    /// A fresh random ID.
    pub fn new() -> Self {
        LibraryId(Uuid::new_v4())
    }
}

impl fmt::Display for LibraryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0.hyphenated(), f)
    }
}

impl Serialize for LibraryId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for LibraryId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Uuid::parse_str(&s)
            .map(LibraryId)
            .map_err(serde::de::Error::custom)
    }
}

/// A registered folder: the path as the user chose it, and its canonical form, which every
/// open re-validates against (HV-SCRIPT-03 step 8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryRoot {
    pub id: LibraryId,
    pub path: PathBuf,
    pub canonical: PathBuf,
}

/// A local mapping `(profile, database) → library` that beats automatic binding for that node
/// only (R-M4b-7). Kept in the registry, never in the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalMapping {
    pub profile: ProfileId,
    pub database: DatabaseName,
    pub library: LibraryId,
}

/// Why a registry change was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// Task 1a's stub: the registry changes nothing until Task 3.
    NotImplemented,
}
// ## m4b-t01a-seams
