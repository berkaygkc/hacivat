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
//!
//! ## m4b-t07-git
//! Task 7: the body. [`status`] runs the whole pipeline on one timed worker thread
//! (`status::run_timed`): discovery up to the home directory and the filesystem boundary
//! (`discover`), the layout and ownership checks before anything is opened and the two trust
//! invariants after (`trust`), the status itself through `index_worktree_status` with a no-op
//! submodule handler plus `tree_index_status` (`status`), and the mapping to library-relative
//! badges with the `Unknown` overrides (`map`). ADR 0009 `## m4b-t07 — git status`.

// ## m4b-t07-git
mod discover;
mod map;
mod status;
mod trust;
// ## m4b-t07-git

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
    /// An unsupported format (`sha256`, `reftable`), or a repository config that cannot be read
    /// (not a regular file, over the size cap, or not parseable: fail closed, m4).
    Unsupported,
    // ## m4b-t07-git (fix round 1)
    /// A file the status would read to decide what is ignored or filtered (`.gitignore`,
    /// `.gitattributes`, `.gitmodules`, `info/exclude`, `info/attributes`) is a cloud
    /// placeholder. It is not opened (opening downloads it), so no status is given (R-T7-3).
    Dataless,
    /// The diagnostic "status is paused: a file in the repository does not respond". An
    /// earlier status of this library is still blocked inside a file read that cannot be
    /// interrupted (a FIFO named `.gitignore`, say). That worker is never joined, and no
    /// second one starts while it runs, so the library answers this until the read returns
    /// or the app restarts (m2). The call that hit the budget first answers `Timeout`.
    Paused,
    /// The repository is too large for the status; the reason names the diagnostic. A library
    /// inside such a repository has no badges.
    TooLarge(TooLargeWhy),
    // ## m4b-t07-git
    /// The repository could not be read. The kind is fixed text of ours; no message from
    /// `gix` (which can quote config bytes) is kept (m3).
    Failed(FailKind),
}

// ## m4b-t07-git (fix round 3, G2)
/// Why a repository is too large for the status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TooLargeWhy {
    /// "The repository is too large for the status": its index holds more entries than gix
    /// decodes under reduced trust (16 MiB of decoded entries, 209,715 entries; measured in
    /// `index_entry_ceiling_is_measured_at_the_boundary`; fix round 2, n1).
    Index,
    /// "Too many loose objects; run git gc": more than 20,000 entries in an `objects/??/`
    /// folder set, too many to check for cloud placeholders before the status (R-T7-6).
    LooseObjects,
}
// ## m4b-t07-git

// ## m4b-t07-git (fix round 1, m3)
/// What failed, as a fixed kind (never a library's text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailKind {
    /// The library folder could not be resolved (missing, unreadable).
    LibraryUnreadable,
    /// The library's path inside its repository is not UTF-8.
    PathNotUtf8,
    /// A path's owner could not be read.
    OwnerUnreadable,
    /// `gix` could not open the repository.
    Open,
    /// The index could not be read.
    Index,
    /// The worktree comparison or the walk failed (an unreadable tracked file fails it all).
    Worktree,
    /// `HEAD` or the HEAD-vs-index comparison failed.
    Head,
    /// The pathspec or the attribute setup failed.
    Setup,
    /// More changed paths than `GitLimits::max_entries`: no partial map.
    TooManyEntries,
    /// A repository file over its size cap, or a library too large to pre-check (R-T7-2).
    TooLarge,
    /// The worker thread could not start, or stopped without an answer.
    Worker,
}
// ## m4b-t07-git

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
    // ## m4b-t07-git (fix round 1)
    /// The index spells the path differently from the disk only by case (a case-only rename
    /// on a case-insensitive file system). The key is the disk's spelling (R-T7-1).
    CaseMismatch,
    /// Not a regular file, link or folder (an untracked FIFO, socket or device; m5).
    NotAFile,
    // ## m4b-t07-git
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

// ## m4b-t07-git
/// The status of the repository that holds `library_root`, if any (HV-SCRIPT-11).
///
/// Blocks the calling thread for at most `limits.timeout` (call it from the blocking pool):
/// the pipeline runs on its own worker, and at the deadline this answers
/// `Unavailable(Timeout)` without waiting for it. The caller itself touches no file (F2): it
/// claims the library by the spelling it was given, and the worker claims the canonical path
/// too once it has resolved it. While a worker for the same library is still running, a new
/// call answers `Unavailable(Paused)` and starts nothing (an alias is caught once its own worker
/// has resolved it). Pass the registry's canonical root.
/// Never runs a program, never writes the index, never reads configuration outside the
/// repository, never opens a cloud placeholder.
pub fn status(library_root: &Path, limits: &GitLimits) -> GitState {
    status::status_with(library_root, limits, status::Hooks::default())
}

/// Test hooks for the integration tests (`tests/`); not part of the product API.
#[doc(hidden)]
pub mod testing {
    use super::{GitLimits, GitState};
    use std::path::Path;

    /// [`status`](super::status), with `dataless` (library-relative, `/`-separated) treated as
    /// cloud placeholders in addition to the ones the file flags name. Entries may name files
    /// inside the repository's `.git` when the library is the worktree (`.git/HEAD`). The hook
    /// can only add `Unknown(Dataless)` answers, index-copy removals and the
    /// `Unavailable(Dataless)` refusal; it cannot remove one, so it cannot weaken the status.
    pub fn status_treating_as_dataless(
        library_root: &Path,
        limits: &GitLimits,
        dataless: &[&str],
    ) -> GitState {
        let hooks = crate::status::Hooks {
            dataless: dataless.iter().map(|s| (*s).to_owned()).collect(),
            stall: None,
        };
        crate::status::status_with(library_root, limits, hooks)
    }

    /// Where [`status_with_stall`] blocks the worker.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Stall {
        /// Before the worker resolves the library path (a stale network mount).
        BeforeResolve,
        /// After it has resolved the path and claimed the canonical library.
        AfterResolve,
    }

    /// [`status`](super::status), with `stall` run on the worker at `at` (it blocks as long as
    /// the test wants). It can only delay the status.
    pub fn status_with_stall(
        library_root: &Path,
        limits: &GitLimits,
        at: Stall,
        stall: impl FnOnce() + Send + 'static,
    ) -> GitState {
        let hooks = crate::status::Hooks {
            dataless: Vec::new(),
            stall: Some((at, Box::new(stall))),
        };
        crate::status::status_with(library_root, limits, hooks)
    }
}
// ## m4b-t07-git
// ## m4b-t01a-seams

#[cfg(test)]
mod tests {
    use super::*;

    // ## m4b-t07-git: the stub's test is gone with the stub. A folder in no repository is
    // `NotARepo`, and the status creates nothing in it.
    #[test]
    fn plain_folder_is_not_a_repo_and_nothing_is_created() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(matches!(
            status(tmp.path(), &GitLimits::default()),
            GitState::NotARepo
        ));
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
