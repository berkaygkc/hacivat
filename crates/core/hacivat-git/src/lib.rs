//! Read-only repository status of a script library (HV-SCRIPT-11;
//! `docs/research/algorithms/28-script-workspace.md`, ADR 0009 D2).
//!
//! The status comes from `gix` in-process, opened isolated and with reduced trust, so no
//! program from the repository's or the user's configuration ever runs: no hooks, no
//! fsmonitor, no filter or diff driver, never a subprocess for the git program
//! (`NoGitSubprocess`, `cargo xtask check-deps`). No `gix` type is public.
//!
//! ## m4b-t01a-seams
//! Task 1a: the types and a fail-closed [`status`] stub that answers
//! [`GitState::Unavailable`] with [`GitError::NotImplemented`] and opens nothing. Task 7 writes
//! the body (`discover`, `status`, `trust`, `map`).

use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// What the status of a library's repository is.
#[derive(Debug, Clone)]
pub enum GitState {
    /// The library is in no repository (not an error; no badges).
    NotARepo,
    /// A repository Hacivat refuses to read (shown as such; no badges).
    Untrusted(UntrustReason),
    /// No status this time (no badges, a notice).
    Unavailable(GitError),
    Ready(Arc<GitStatus>),
}

/// Why a repository is not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UntrustReason {
    /// The worktree, `.git`, or a gitfile's resolved git dir or common dir belongs to another
    /// user (Unix).
    ForeignOwner,
}

/// Why no status is available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    /// Task 1a's stub: nothing is read until Task 7.
    NotImplemented,
    /// The 5 s budget ran out; the caller answers without waiting for the worker.
    Timeout,
    /// After the open, the repository or a repository config section had full trust (C1).
    TrustPromoted,
    /// A gitfile, `commondir` or alternates point out of the repository.
    Redirected,
    /// An unsupported format (`sha256`, `reftable`).
    Unsupported,
    /// The repository could not be read (the message never holds file contents).
    Failed(String),
}

/// The status of every listed path that is not clean, keyed relative to the **library** root.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitStatus {
    pub by_path: BTreeMap<GitPath, FileStatus>,
}

/// A library-relative path with `/` separators, as the status keys it. `hacivat-scripts`
/// looks entries up by its own `RelPath` text (`by_path.get(rel.as_str())`), since this crate
/// cannot name that type (`hacivat-scripts` depends on this one).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GitPath(Arc<str>);

impl GitPath {
    pub fn new(rel: &str) -> Self {
        GitPath(rel.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for GitPath {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for GitPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One path's badge (R-M4b-11). Clean and ignored paths have no entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    /// The index or the worktree differs from HEAD.
    Modified,
    /// Untracked, or added in the index.
    New,
    /// Unmerged.
    Conflicted,
    Unknown(UnknownWhy),
}

/// Why a path's status is not known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownWhy {
    /// Its attributes name a `filter`; no driver runs, so the bytes are not compared (C2).
    Filtered,
    /// A cloud placeholder (dataless); never hashed (R-M4b-15).
    Dataless,
    /// A submodule root; the no-op `SubmoduleStatus` never opens it (R-T0-1).
    Submodule,
}

/// Bounds of one status call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GitLimits {
    /// The budget of one call (HV-SCRIPT-11: 5 s).
    pub timeout: Duration,
    /// The most entries one status lists before it stops (R-M4b-14: a library's 20,000 files).
    pub max_entries: usize,
}

impl Default for GitLimits {
    fn default() -> Self {
        GitLimits {
            timeout: Duration::from_secs(5),
            max_entries: 20_000,
        }
    }
}

/// The status of the repository that holds `library_root`, if any.
///
/// Task 1a stub (fail closed, F31): answers `Unavailable(NotImplemented)` and reads nothing.
pub fn status(library_root: &Path, limits: &GitLimits) -> GitState {
    let _ = (library_root, limits);
    GitState::Unavailable(GitError::NotImplemented)
}
// ## m4b-t01a-seams

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_stub_is_unavailable() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(matches!(
            status(tmp.path(), &GitLimits::default()),
            GitState::Unavailable(GitError::NotImplemented)
        ));
        // A stub reads nothing, so it creates nothing either.
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn status_is_looked_up_by_library_relative_text() {
        let mut s = GitStatus::default();
        s.by_path.insert(GitPath::new("a/b.sql"), FileStatus::New);
        assert_eq!(s.by_path.get("a/b.sql"), Some(&FileStatus::New));
        assert_eq!(GitLimits::default().timeout, Duration::from_secs(5));
    }
}
