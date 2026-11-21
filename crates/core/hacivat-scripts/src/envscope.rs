//! `@<environment>/` scoping, fail closed (HV-SCRIPT-04; R-M4b-6).
//!
//! ## m4b-t01a-seams
//! Task 1a: the types and fail-closed stubs: [`scope_of`] hides every path and [`visible`]
//! answers `false`, so no file is shown under any connection. Task 4 writes the rules.

use hacivat_proto::EnvTag;

use crate::pathsafe::RelPath;
use crate::scan::ScriptEntry;

/// Where a file may be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvScope {
    /// No `@` folder on its path: every connection.
    Any,
    /// Only connections whose environment tag is in the set.
    Only(EnvSet),
    /// Never shown.
    Hidden(ScopeProblem),
}

/// A set of environment tags.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct EnvSet(u8);

impl EnvSet {
    /// The empty set.
    pub const fn empty() -> Self {
        EnvSet(0)
    }

    /// The set holding `tag` only.
    pub fn of(tag: EnvTag) -> Self {
        EnvSet(bit(tag))
    }

    pub fn with(self, tag: EnvTag) -> Self {
        EnvSet(self.0 | bit(tag))
    }

    pub fn contains(self, tag: EnvTag) -> bool {
        self.0 & bit(tag) != 0
    }

    pub fn intersect(self, other: EnvSet) -> Self {
        EnvSet(self.0 & other.0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

fn bit(tag: EnvTag) -> u8 {
    1 << EnvTag::ALL.iter().position(|t| *t == tag).unwrap_or(0)
}

/// Why a path is hidden everywhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeProblem {
    /// Task 1a's stub hides every path.
    NotImplemented,
}

/// The scope of `rel` from its `@` folders.
///
/// Task 1a stub (fail closed): every path is hidden.
pub fn scope_of(rel: &RelPath) -> EnvScope {
    let _ = rel;
    EnvScope::Hidden(ScopeProblem::NotImplemented)
}

/// Whether `entry` is shown under a connection whose environment is `env`.
///
/// Task 1a stub (fail closed): `false` for every file.
pub fn visible(entry: &ScriptEntry, env: Option<EnvTag>) -> bool {
    let _ = (entry, env);
    false
}
// ## m4b-t01a-seams
