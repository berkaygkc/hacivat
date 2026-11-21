//! Library-relative paths and the name rules every open re-validates (HV-SCRIPT-03 steps 4, 8).
//!
//! ## m4b-t01a-seams
//! Task 1a: [`RelPath`], whose constructor enforces the structural invariant (relative,
//! `/`-separated, no empty, `.` or `..` component, no `\`, no NUL or other C0 control), and the
//! name policy seams as fail-closed stubs: [`component`] and [`join`] refuse every name until
//! Task 4 writes them (Windows device names, trailing dot or space, 255 bytes, `O_NOFOLLOW`
//! opens, the canonical-root check). A `RelPath` never reaches the file system except through
//! [`join`].

use std::ffi::OsStr;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::registry::LibraryRoot;

/// A path inside a library, relative to its root, `/`-separated. Identity is the exact text
/// (the OS bytes the scan saw, NFD on macOS included); display goes through NFC and escaping
/// (Task 4). Serialized as the plain string (`workspace.json`, Task 1b); deserializing applies
/// the same structural checks as [`RelPath::parse`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RelPath(Arc<str>);

/// Why a path or a name was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    /// Task 1a's stubs refuse every name.
    NotImplemented,
    /// Empty, absolute, an empty, `.` or `..` component, a `\`, or a control character.
    Malformed,
}

impl RelPath {
    /// A library-relative path from `/`-separated text, checked structurally.
    pub fn parse(text: &str) -> Result<Self, PathError> {
        let well_formed = !text.is_empty()
            && text.split('/').all(|c| {
                !c.is_empty()
                    && c != "."
                    && c != ".."
                    && !c.contains('\\')
                    && !c.chars().any(char::is_control)
            });
        if well_formed {
            Ok(RelPath(text.into()))
        } else {
            Err(PathError::Malformed)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The components, root first.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for RelPath {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for RelPath {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        RelPath::parse(&s).map_err(|_| serde::de::Error::custom("not a library-relative path"))
    }
}

/// Validates one file or folder name read from a library (the fuzz target's entry,
/// `fuzz/scripts/fuzz_targets/pathsafe.rs`).
///
/// Task 1a stub (fail closed): refuses every name.
pub fn component(name: &OsStr) -> Result<&str, PathError> {
    let _ = name;
    Err(PathError::NotImplemented)
}

/// The absolute path of `rel` under `root`, after re-validating every component and the
/// canonical root (HV-SCRIPT-03 step 8).
///
/// Task 1a stub (fail closed): refuses every path, so nothing is opened.
pub fn join(root: &LibraryRoot, rel: &RelPath) -> Result<PathBuf, PathError> {
    let _ = (root, rel);
    Err(PathError::NotImplemented)
}
// ## m4b-t01a-seams
