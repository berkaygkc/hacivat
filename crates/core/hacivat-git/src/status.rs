//! The status pipeline and its timed worker (HV-SCRIPT-11 steps 3–6; `alg/28` C12, m3, n3, n4).
//!
//! Everything, discovery and the small reads of `trust` included, runs on one worker thread per
//! call. The caller waits at most `GitLimits::timeout`; at the deadline it answers
//! `Unavailable(Timeout)`, sets the interrupt flag and returns **without joining** the worker,
//! because `gix`'s blocking opens of `.gitignore`, `.gitattributes` (and a FIFO in their place)
//! cannot be interrupted (`alg/28` §0.8). While that worker still runs, a new call for the same
//! library (by canonical root, m1) answers `Unavailable(Paused)` at once (m2): at most one stuck
//! worker per library. The worker stays cheap: every repository file a read could follow into
//! an unbounded stream is checked regular and capped first (`trust`, I3, R-T7-2).
//!
//! ## `gix` entry points used, and why none can start a program (Task 7 Step 3)
//!
//! The repository is opened with `Options::isolated().with(Trust::Reduced)`: no system,
//! installation, user or environment configuration, no includes, `git_binary: false` (so
//! `gix-path`'s lookups of the git CLI's own configuration, the Windows `git config` spawn of
//! `alg/28` §0.4, are never reached), and repository config sections that are not trusted;
//! `trust::check_trust` then proves both trust invariants. Program-valued keys (filter and diff
//! drivers, textconv, external diff) are read only from trusted sections, so none exists here.
//! - `gix::open_opts`: reads the repository's files; spawns nothing under isolated options.
//! - `Repository::git_dir_trust`, `config_snapshot().plumbing()`, `git_dir`, `common_dir`,
//!   `workdir`: accessors.
//! - `Repository::index_or_empty`: reads `.git/index`; never writes it. `Outcome::write_changes`
//!   is never called, so the index stays byte-identical (`index_is_never_written`).
//! - `Repository::index_worktree_status` with `FastEq` and **our** [`NoopSubmodules`]
//!   (never `Repository::status()`, whose `BuiltinSubmoduleStatus` re-opens the repository with
//!   `Options::default()` for every gitlink, C12): `lstat`s and hashes worktree files through
//!   the filter pipeline with no driver (built-in EOL conversion only), walks untracked
//!   directories, reads `.gitignore`/`.gitattributes`. No hook, no fsmonitor (gix implements
//!   neither for status), no submodule open.
//! - `Repository::head_tree_id_or_empty`, `Repository::pathspec`,
//!   `Repository::tree_index_status` with `TrackRenames::Disabled` (n3: no rename similarity
//!   work, whose budget would come from repository config): reads objects.
//! - `Repository::dirwalk_options`, `Repository::attributes_only` and
//!   `AttributeStack::at_entry`/`selected_attribute_matches`: attribute files only.
//! - `gix::sec::identity::is_path_owned_by_current_user` (via `trust`), `gix::utils::str::
//!   precompose` (via `map`), `gix::config::File::from_path_no_includes` (via `trust`): pure.
//!
//! `cargo xtask check-deps` (`NoGitSubprocess`) refuses any process API in this crate, and
//! `tests/hostile_repo.rs` runs the real call path under a spawn detector on every OS.

// ## m4b-t07-git
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use gix::bstr::{BStr, BString, ByteSlice as _};

use crate::discover::{self, Discovery, DotGit, Found};
use crate::map::{self, Badges};
use crate::trust;
use crate::{FailKind, FileStatus, GitError, GitLimits, GitPath, GitState, GitStatus, UnknownWhy};

/// The libraries whose worker is running (keyed by the root as the caller gave it).
static RUNNING: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

fn running() -> std::sync::MutexGuard<'static, BTreeSet<PathBuf>> {
    RUNNING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Removes its library from [`RUNNING`] when the worker ends (also on a panic).
struct Running(PathBuf);

impl Drop for Running {
    fn drop(&mut self) {
        running().remove(&self.0);
    }
}

fn failed(kind: FailKind) -> GitState {
    GitState::Unavailable(GitError::Failed(kind))
}

/// The `testing` hooks (empty in the product).
#[derive(Default)]
pub(crate) struct Hooks {
    /// Library-relative paths treated as cloud placeholders.
    pub(crate) dataless: Vec<String>,
    /// A stall run on the worker, at the given point.
    pub(crate) stall: Option<(crate::testing::Stall, Box<dyn FnOnce() + Send>)>,
}

impl Hooks {
    fn stall_at(&mut self, at: crate::testing::Stall) {
        if self.stall.as_ref().is_some_and(|(a, _)| *a == at)
            && let Some((_, f)) = self.stall.take()
        {
            f();
        }
    }
}

/// Resolves the library path (the stall hook stands in for a slow mount here).
fn resolve(library_root: &Path, hooks: &mut Hooks) -> Option<PathBuf> {
    hooks.stall_at(crate::testing::Stall::BeforeResolve);
    std::fs::canonicalize(library_root).ok()
}

/// The whole status of `library_root` on the timed worker.
///
/// F2: the caller does no file-system work at all (a stale network mount blocks path
/// resolution without limit). It claims the library by the spelling it was given; the worker
/// resolves the path and then claims the canonical library too, answering `Paused` when another
/// worker already holds it (m1: an alias reaches the cap once resolved). Both claims are
/// released when the worker ends.
pub(crate) fn status_with(library_root: &Path, limits: &GitLimits, hooks: Hooks) -> GitState {
    let mut hooks = hooks;
    let limits = *limits;
    let given = library_root.to_path_buf();
    let spelling = given.clone();
    // ## m4b-t20-adversarial fix round 2 (named line, I1-fr1 b): with the overrun backoff.
    run_timed_backoff(
        spelling,
        limits.timeout,
        limits.retry_after,
        move |interrupt| {
            let Some(root) = resolve(&given, &mut hooks) else {
                return failed(FailKind::LibraryUnreadable);
            };
            let _canonical = if root == given {
                None
            } else if running().insert(root.clone()) {
                Some(Running(root.clone()))
            } else {
                return GitState::Unavailable(GitError::Paused);
            };
            hooks.stall_at(crate::testing::Stall::AfterResolve);
            pipeline(&root, &limits, &hooks.dataless, interrupt)
        },
    )
}

/// Runs `work` on a worker thread and waits at most `timeout` for its answer (m3). Since
/// m4b-t20 fix round 2 the status uses [`run_timed_backoff`]; this form (no backoff) is the
/// unit tests'.
#[cfg(test)]
pub(crate) fn run_timed<F>(key: PathBuf, timeout: Duration, work: F) -> GitState
where
    F: FnOnce(&AtomicBool) -> GitState + Send + 'static,
{
    run_timed_backoff(key, timeout, Duration::ZERO, work)
}

// ## m4b-t20-adversarial fix round 2 (named exception, re-review I1-fr1 b)
/// Libraries whose last worker overran its call's budget: no new worker before `until`.
struct Overrun {
    until: std::time::Instant,
    backoff: Duration,
}

static OVERRAN: Mutex<std::collections::BTreeMap<PathBuf, Overrun>> =
    Mutex::new(std::collections::BTreeMap::new());

fn overran() -> std::sync::MutexGuard<'static, std::collections::BTreeMap<PathBuf, Overrun>> {
    OVERRAN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The longest wait after overruns in a row.
const RETRY_MAX: Duration = Duration::from_secs(3_600);

/// [`run_timed`], with a backoff after an overrun: a worker abandoned at the deadline that
/// ends later records it, and until its backoff (`retry`, doubling on each overrun in a row,
/// at most [`RETRY_MAX`]) has passed the library answers `Unavailable(Timeout)` at once and
/// starts no worker. A worker that answers in time clears it. Together with the one-worker
/// claim this keeps at most one worker per library, never back to back on a status that cannot
/// finish, and the library is retried, never `Paused` for good.
pub(crate) fn run_timed_backoff<F>(
    key: PathBuf,
    timeout: Duration,
    retry: Duration,
    work: F,
) -> GitState
where
    F: FnOnce(&AtomicBool) -> GitState + Send + 'static,
{
    if overran()
        .get(&key)
        .is_some_and(|o| std::time::Instant::now() < o.until)
    {
        return GitState::Unavailable(GitError::Timeout);
    }
    if !running().insert(key.clone()) {
        // m2: an earlier worker for this library is still blocked in a read.
        return GitState::Unavailable(GitError::Paused);
    }
    let interrupt = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&interrupt);
    // The caller gave up (the interrupt flag is also set by a full walk, so not that one).
    let abandoned = Arc::new(AtomicBool::new(false));
    let gave_up = Arc::clone(&abandoned);
    let (tx, rx) = mpsc::sync_channel(1);
    let record_key = key.clone();
    let guard = Running(key);
    let spawned = std::thread::Builder::new()
        .name("hacivat-repo-status".into())
        .spawn(move || {
            let guard = guard;
            let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&flag)))
                .unwrap_or_else(|_| failed(FailKind::Worker));
            {
                let mut o = overran();
                if gave_up.load(Ordering::SeqCst) {
                    let backoff = o
                        .get(&record_key)
                        .map_or(retry, |prev| (prev.backoff * 2).max(retry))
                        .min(RETRY_MAX);
                    o.insert(
                        record_key,
                        Overrun {
                            until: std::time::Instant::now() + backoff,
                            backoff,
                        },
                    );
                } else {
                    o.remove(&record_key);
                }
            }
            // Leave RUNNING before answering, so that a call right after this answer starts.
            drop(guard);
            let _ = tx.send(out);
        });
    if let Err(e) = spawned {
        let _ = e;
        return failed(FailKind::Worker);
    }
    match rx.recv_timeout(timeout) {
        Ok(st) => st,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            abandoned.store(true, Ordering::SeqCst);
            interrupt.store(true, Ordering::SeqCst);
            GitState::Unavailable(GitError::Timeout)
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => failed(FailKind::Worker),
    }
}
// ## m4b-t20-adversarial

/// Discover, check, open, check again, collect. `root` is canonical.
fn pipeline(
    root: &Path,
    limits: &GitLimits,
    extra_dataless: &[String],
    interrupt: &AtomicBool,
) -> GitState {
    let root = root.to_path_buf();
    let found = match discover::find(&root) {
        Discovery::Found(f) => f,
        Discovery::NotARepo => return GitState::NotARepo,
        Discovery::NonUtf8Prefix => {
            return failed(FailKind::PathNotUtf8);
        }
    };
    // F3: one placeholder test for every path checked before the open: the real flag (from
    // `lstat`), or the `testing` hook's library-relative set.
    let hooked: BTreeSet<PathBuf> = extra_dataless.iter().map(|e| root.join(e)).collect();
    let placeholder = |p: &Path| {
        hooked.contains(p) || std::fs::symlink_metadata(p).is_ok_and(|m| map::is_dataless(&m))
    };
    // p1: `check_layout` checks every file it reads (gitfile, `commondir`, alternates) for
    // placeholders before reading it.
    // ## m4b-t20-adversarial fix round 1 (named lines, I1): interruptible.
    let layout = match trust::check_layout_bounded(
        &found,
        &trust::current_user_owns,
        &placeholder,
        interrupt,
    ) {
        Ok(l) => l,
        Err(st) => return st,
    };
    if let Err(st) = trust::check_internals_dataless(&layout, &placeholder, interrupt) {
        return st;
    }
    if trust::unsupported_format(&layout.git_dir, &layout.common_dir) {
        return GitState::Unavailable(GitError::Unsupported);
    }
    // ## m4b-t20-adversarial fix round 4 (named lines, I-fr3-1): no config value gix may open
    // as a path leaves the local drives.
    if trust::config_names_a_remote_path(&layout.git_dir, &layout.common_dir) {
        return GitState::Unavailable(GitError::Redirected);
    }
    // I3 (R-T7-2), after the format check: a reftable repository has a file at `refs/heads`.
    if let Err(st) = trust::check_repository_files(&layout, &placeholder) {
        return st;
    }
    let extra: BTreeSet<String> = extra_dataless.iter().cloned().collect();
    let facts = match trust::check_library_tree(&found.worktree, &root, &placeholder) {
        Ok(f) => f,
        Err(st) => return st,
    };
    if interrupt.load(Ordering::SeqCst) {
        return GitState::Unavailable(GitError::Timeout);
    }
    // G1 (R-T7-5): never scan `refs/replace` (gix reads every replace ref whole, and walks a
    // symlinked `refs/replace` out of the repository). An API override is not a repository
    // section, so the trust invariants still hold. Cost: no CLI parity for `git replace`.
    let options = gix::open::Options::isolated()
        .with(gix::sec::Trust::Reduced)
        .config_overrides(["core.useReplaceRefs=false"]);
    let repo = match gix::open_opts(&found.worktree, options) {
        Ok(r) => r,
        Err(_) => return failed(FailKind::Open),
    };
    if let Err(e) = trust::check_trust(&repo)
        .and_then(|()| trust::check_resolved(&repo, &layout, &found.worktree))
    {
        return GitState::Unavailable(e);
    }
    if let Err(st) = trust::check_owners_again(&repo, &found.worktree, &trust::current_user_owns) {
        return st;
    }
    match collect(&repo, &found, &root, limits, &extra, &facts, interrupt) {
        // ## m4b-t20-adversarial (named lines, N3): the lock of a git dir above the library.
        Ok(GitState::Ready(s)) if !found.prefix.is_empty() || found.kind != DotGit::Dir => {
            let mut s = Arc::unwrap_or_clone(s);
            s.index_lock = Some(layout.git_dir.join("index.lock"));
            GitState::Ready(Arc::new(s))
        }
        Ok(st) | Err(st) => st,
    }
}

/// Our submodule handler (C12): records the gitlink and answers "no change" without opening
/// anything. Gitlinks are `Unknown(Submodule)` from the index walk anyway.
#[derive(Clone, Default)]
struct NoopSubmodules {
    seen: Arc<Mutex<Vec<BString>>>,
}

impl gix::status::plumbing::index_as_worktree::traits::SubmoduleStatus for NoopSubmodules {
    type Output = ();

    fn status(
        &mut self,
        _entry: &gix::index::Entry,
        rela_path: &BStr,
    ) -> gix::Result<Option<Self::Output>> {
        self.seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(rela_path.to_owned());
        Ok(None)
    }
}

/// Collects the index-vs-worktree entries; stops the walk (through the interrupt flag) once
/// more than `max` paths are recorded.
struct Recorder<'a> {
    out: Vec<(BString, FileStatus)>,
    max: usize,
    overflow: bool,
    interrupt: &'a AtomicBool,
}

impl<'a> Recorder<'a> {
    fn push(&mut self, path: &BStr, s: FileStatus) {
        self.out.push((path.to_owned(), s));
        if self.out.len() > self.max {
            self.overflow = true;
            self.interrupt.store(true, Ordering::SeqCst);
        }
    }
}

// (Named lifetimes in these headers: `check-deps`' `BackendInPublicApi` reads the `_` of
// `'_` as the `ByteSlice as _` import and would count the private impl as public API.)
impl<'index, 'a> gix::status::plumbing::index_as_worktree_with_renames::VisitEntry<'index>
    for Recorder<'a>
{
    type ContentChange = ();
    type SubmoduleStatus = ();

    fn visit_entry(
        &mut self,
        entry: gix::status::plumbing::index_as_worktree_with_renames::Entry<'index, (), ()>,
    ) {
        use gix::dir::entry::{Kind, Status};
        use gix::status::plumbing::index_as_worktree_with_renames::{Entry, Summary};
        if let Entry::DirectoryContents { entry: e, .. } = &entry {
            if e.status != Status::Untracked {
                return;
            }
            match e.disk_kind {
                // A nested repository inside the library (U-04): never opened.
                Some(Kind::Repository) => {
                    let path = e.rela_path.as_bstr().trim_end_with(|c| c == '/');
                    self.push(path.as_bstr(), FileStatus::Unknown(UnknownWhy::Submodule));
                }
                Some(Kind::Directory) | None => {}
                // m5: a FIFO, socket or device is not a file (the pre-walk records the ones
                // gix does not emit).
                Some(Kind::Untrackable) => self.push(
                    e.rela_path.as_bstr(),
                    FileStatus::Unknown(UnknownWhy::NotAFile),
                ),
                Some(Kind::File | Kind::Symlink) => {
                    self.push(e.rela_path.as_bstr(), FileStatus::New)
                }
            }
            return;
        }
        let badge = match entry.summary() {
            Some(Summary::Conflict) => FileStatus::Conflicted,
            Some(Summary::Added | Summary::IntentToAdd | Summary::Renamed | Summary::Copied) => {
                FileStatus::New
            }
            Some(Summary::Modified | Summary::TypeChange) => FileStatus::Modified,
            // Deleted paths have no badge; `NeedsUpdate` (racy stat, content equal) is no change.
            Some(Summary::Removed) | None => return,
        };
        self.push(entry.destination_rela_path(), badge);
    }
}

/// The mapper for a failed `gix` step.
fn gix_failed(kind: FailKind) -> impl Fn(gix::Error) -> GitState {
    move |_| failed(kind)
}

/// The status proper, on an opened and checked repository.
fn collect(
    repo: &gix::Repository,
    found: &Found,
    library: &Path,
    limits: &GitLimits,
    extra_dataless: &BTreeSet<String>,
    facts: &trust::TreeFacts,
    interrupt: &AtomicBool,
) -> Result<GitState, GitState> {
    let timed_out = || GitState::Unavailable(GitError::Timeout);
    let too_many = || failed(FailKind::TooManyEntries);
    let prefix = found.prefix.as_str();
    // I2: whether the file system folds case (`core.ignoreCase`, as gix reads it).
    let icase = repo.filesystem_options().is_ok_and(|c| c.ignore_case);
    let on_disk = repo.index_or_empty().map_err(gix_failed(FailKind::Index))?;
    let mut badges = Badges::default();
    // n2: index spellings of the library folder that only the file system's own case folding
    // equates with the disk's (`Çal` in the index, `çal` on disk): the same directory by
    // `(dev, ino)` (Unix). One `lstat` per distinct leading folder of the index.
    let mut aliases: BTreeSet<String> = BTreeSet::new();
    if icase && !prefix.is_empty() {
        let n = prefix.split('/').count();
        let none = BTreeSet::new();
        let mut heads: BTreeSet<String> = BTreeSet::new();
        for entry in on_disk.entries() {
            let Ok(p) = entry.path(&on_disk).to_str() else {
                continue;
            };
            let comps: Vec<&str> = p.split('/').collect();
            if comps.len() <= n {
                continue;
            }
            let head = comps[..n].join("/");
            // ## m4b-cifix-2-index-paths (review C2, R-CI2-2): `head` is untrusted index
            // content; only a plain one, joined lexically under the worktree, is ever looked at
            // (a UNC `\\host\share` head would otherwise make Windows contact the host).
            if heads.insert(head.clone())
                && map::lib_relative(p, prefix, icase, &none).is_none()
                && map::alias_candidate(&found.worktree, &head)
                    .is_some_and(|candidate| map::same_file(&candidate, library))
            {
                aliases.insert(head);
            }
        }
    }
    // I1: the library-relative part of an index path, compared in NFC (and by case where the
    // file system folds it), whatever the disk's spelling of the library folder.
    let rel_of = |path: &BStr| {
        path.to_str()
            .ok()
            .and_then(|p| map::lib_relative(p, prefix, icase, &aliases))
            .map(|(rel, _)| rel)
    };
    // n5: dataless folders are listed by nobody: their tracked entries leave the index copy and
    // gix's walk excludes them (no badge: the scanner does not list them either).
    let dataless_dirs: Vec<String> = facts.dataless_dirs.iter().map(|d| map::nfc(d)).collect();
    let in_dataless_dir = |rel: &str| {
        let rel = map::nfc(rel);
        dataless_dirs
            .iter()
            .any(|d| rel == *d || rel.starts_with(&format!("{d}/")))
    };

    // Gitlinks are Unknown(Submodule). Dataless tracked files are Unknown(Dataless) and are
    // removed from an in-memory copy of the index (never written), so gix neither compares
    // nor hashes them: the walk sees an untracked name (an `lstat`, no open), which the
    // precedence turns back into Unknown(Dataless). [Observed] such a path is never read
    // (`excluded_path_is_never_opened_by_status`); [Inferred] that this keeps the OS from
    // downloading it (I2, n6). `alg/28` step 6 named `:(exclude)` pathspecs; 20,000 of them
    // made the walk quadratic (U-05: past the 5 s budget), so Task 7 drops entries instead.
    let extra: BTreeSet<String> = extra_dataless.iter().map(|l| map::nfc(l)).collect();
    let mut dataless_paths: BTreeSet<BString> = BTreeSet::new();
    let mut walk = map::Lister::default();
    let mut tracked: Vec<(BString, gix::index::entry::Mode)> = Vec::new();
    for entry in on_disk.entries() {
        let path = entry.path(&on_disk);
        // ## m4b-cifix-2-index-paths (review C2, R-CI2-2): an entry whose path is not made of
        // plain names leaves the index copy, so gix never stats it either, and is
        // `Unknown(UnsafePath)` when it falls in the library; nothing joins it to a directory.
        if !map::is_plain_index_path(path) {
            if rel_of(path).is_some() {
                badges.add(path, FileStatus::Unknown(UnknownWhy::UnsafePath));
            }
            dataless_paths.insert(path.to_owned());
            continue;
        }
        let Some(rel) = rel_of(path) else {
            continue;
        };
        if in_dataless_dir(&rel) {
            dataless_paths.insert(path.to_owned());
            continue;
        }
        if entry.mode.is_submodule() {
            badges.add(path, FileStatus::Unknown(UnknownWhy::Submodule));
            continue;
        }
        // ## m4b-cifix-2 fix round 2 (re-review N1): through the link-refusing walk, `lstat`
        // only at the last component (a plain path is UTF-8); a path under a linked folder is
        // never looked at here, and gix's own walk (`SymlinkCheck`) does not enter it either.
        let dataless = extra.contains(&map::nfc(&rel))
            || path
                .to_str()
                .ok()
                .and_then(|p| walk.lstat_under(&found.worktree, p))
                .is_some_and(|m| map::is_dataless(&m));
        if dataless {
            dataless_paths.insert(path.to_owned());
            badges.add(path, FileStatus::Unknown(UnknownWhy::Dataless));
            continue;
        }
        tracked.push((path.to_owned(), entry.mode));
    }
    if interrupt.load(Ordering::SeqCst) {
        return Err(timed_out());
    }
    let without_dataless;
    let index: &gix::index::State = if dataless_paths.is_empty() {
        &on_disk
    } else {
        let mut copy = gix::index::State::clone(&on_disk);
        copy.remove_entries(|_, path, _| dataless_paths.contains(path));
        // The cache-tree extension may name the removed paths; nothing here needs it.
        copy.remove_tree();
        without_dataless = copy;
        &without_dataless
    };

    let mut patterns: Vec<BString> = Vec::new();
    if !prefix.is_empty() {
        // I1: the disk's spelling, and its NFC and NFD forms, so the index's spelling matches
        // whichever it is (several positive patterns are their union). gix adds ICASE itself
        // where the file system folds case.
        let forms = [
            prefix.to_owned(),
            map::nfc(prefix),
            gix::utils::str::decompose(prefix.into()).into_owned(),
        ];
        for (i, form) in forms.iter().enumerate() {
            if !forms[..i].contains(form) {
                patterns.push(format!(":(top,literal){form}").into());
            }
        }
        for alias in &aliases {
            patterns.push(format!(":(top,literal){alias}").into());
        }
    } else if !facts.dataless_dirs.is_empty() {
        // Exclusions alone match nothing: start from the whole worktree.
        patterns.push(":(top)".into());
    }
    for d in &facts.dataless_dirs {
        let repo_rel = if prefix.is_empty() {
            d.clone()
        } else {
            format!("{prefix}/{d}")
        };
        patterns.push(format!(":(exclude,top,literal){repo_rel}").into());
    }

    // Index vs worktree.
    let submodules = NoopSubmodules::default();
    let mut rec = Recorder {
        out: Vec::new(),
        max: limits.max_entries,
        overflow: false,
        interrupt,
    };
    let dirwalk = repo
        .dirwalk_options()
        .map_err(gix_failed(FailKind::Setup))?
        .emit_untracked(gix::dir::walk::EmissionMode::Matching);
    let options = gix::status::index_worktree::Options {
        sorting: None,
        dirwalk_options: Some(dirwalk),
        rewrites: None,
        thread_limit: None,
    };
    let walked = repo.index_worktree_status(
        index,
        &patterns,
        &mut rec,
        gix::status::plumbing::index_as_worktree::traits::FastEq,
        submodules.clone(),
        &mut gix::progress::Discard,
        interrupt,
        options,
    );
    if rec.overflow {
        return Err(too_many());
    }
    if interrupt.load(Ordering::SeqCst) {
        return Err(timed_out());
    }
    walked.map_err(gix_failed(FailKind::Worktree))?;
    for (path, s) in rec.out {
        badges.add(path.as_bstr(), s);
    }
    for path in submodules
        .seen
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
    {
        badges.add(path.as_bstr(), FileStatus::Unknown(UnknownWhy::Submodule));
    }

    // HEAD vs index.
    let head_tree = repo
        .head_tree_id_or_empty()
        .map_err(gix_failed(FailKind::Head))?
        .detach();
    let mut pathspec = repo
        .pathspec(
            false,
            &patterns,
            true,
            index,
            gix::worktree::stack::state::attributes::Source::WorktreeThenIdMapping,
        )
        .map_err(gix_failed(FailKind::Setup))?;
    let mut staged: Vec<(BString, FileStatus)> = Vec::new();
    let mut overflow = false;
    repo.tree_index_status(
        &head_tree,
        index,
        Some(&mut pathspec),
        gix::status::tree_index::TrackRenames::Disabled,
        |change, _, _| {
            use gix::diff::index::ChangeRef;
            let (path, badge) = match change {
                ChangeRef::Addition { location, .. } | ChangeRef::Rewrite { location, .. } => {
                    (location, FileStatus::New)
                }
                ChangeRef::Modification { location, .. } => (location, FileStatus::Modified),
                ChangeRef::Deletion { .. } => return Ok(std::ops::ControlFlow::Continue(())),
            };
            staged.push((path.into_owned(), badge));
            if staged.len() > limits.max_entries {
                overflow = true;
                return Ok(std::ops::ControlFlow::Break(()));
            }
            if interrupt.load(Ordering::SeqCst) {
                return Ok(std::ops::ControlFlow::Break(()));
            }
            Ok(std::ops::ControlFlow::Continue(()))
        },
    )
    .map_err(gix_failed(FailKind::Head))?;
    if overflow {
        return Err(too_many());
    }
    if interrupt.load(Ordering::SeqCst) {
        return Err(timed_out());
    }
    for (path, s) in staged {
        badges.add(path.as_bstr(), s);
    }

    // Our own `filter` check (C2), for every tracked path and every changed one: a filtered
    // path's bytes are not what the index compares, so its badge is Unknown, never Modified.
    let mut attrs = repo
        .attributes_only(
            index,
            gix::worktree::stack::state::attributes::Source::WorktreeThenIdMapping,
        )
        .map_err(gix_failed(FailKind::Setup))?;
    let changed = badges.paths();
    let mut probe: Vec<(BString, Option<gix::index::entry::Mode>)> =
        tracked.into_iter().map(|(p, m)| (p, Some(m))).collect();
    probe.extend(changed.into_iter().map(|p| (p, None)));
    for (path, mode) in probe {
        if interrupt.load(Ordering::SeqCst) {
            return Err(timed_out());
        }
        if matches!(badges.get(path.as_bstr()), Some(FileStatus::Unknown(_))) {
            continue;
        }
        if filtered(&mut attrs, path.as_bstr(), mode) {
            badges.add(path.as_bstr(), FileStatus::Unknown(UnknownWhy::Filtered));
        }
    }

    let mut by_path = badges.into_library(prefix, icase, &aliases, library, &map::is_dataless);
    // m5: FIFOs, sockets and devices the pre-walk found (gix lists none of them).
    for rel in &facts.not_files {
        let s = FileStatus::Unknown(UnknownWhy::NotAFile);
        let e = by_path.entry(GitPath::new(rel)).or_insert(s);
        *e = map::stronger(*e, s);
    }
    if by_path.len() > limits.max_entries {
        return Err(too_many());
    }
    Ok(GitState::Ready(Arc::new(GitStatus {
        by_path,
        index_lock: None,
    })))
}

/// Whether `path`'s attributes set `filter` (set, or to a value). An attribute lookup that
/// fails counts as filtered: fail closed.
fn filtered(
    attrs: &mut gix::AttributeStack<'_>,
    path: &BStr,
    mode: Option<gix::index::entry::Mode>,
) -> bool {
    let mut outcome = attrs.selected_attribute_matches(["filter"]);
    let mode = mode.or(Some(gix::index::entry::Mode::FILE));
    let Ok(platform) = attrs.at_entry(path, mode) else {
        return true;
    };
    platform.matching_attributes(&mut outcome);
    outcome.iter_selected().any(|m| {
        matches!(
            m.assignment.state,
            gix::attrs::StateRef::Set | gix::attrs::StateRef::Value(_)
        )
    })
}
// ## m4b-t07-git

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn key(name: &str) -> PathBuf {
        PathBuf::from(format!("/hv-t07-unit/{name}"))
    }

    /// Waits until `key` left [`RUNNING`] (a condition, not a sleep budget).
    fn wait_released(k: &Path) {
        while running().contains(k) {
            std::thread::yield_now();
        }
    }

    #[test]
    fn answer_arrives_and_the_library_is_released() {
        let k = key("quick");
        let st = run_timed(k.clone(), Duration::from_secs(5), |_| GitState::NotARepo);
        assert!(matches!(st, GitState::NotARepo));
        assert!(!running().contains(&k), "released before the answer");
    }

    /// A worker that never answers in time: `Timeout` without joining it, the interrupt flag
    /// set, and no second worker for that library while it runs.
    #[test]
    fn a_stuck_worker_answers_timeout_and_blocks_a_second_one() {
        let k = key("stuck");
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let saw_interrupt = Arc::new(AtomicBool::new(false));
        let started = Arc::new(AtomicUsize::new(0));
        let (s, i) = (Arc::clone(&started), Arc::clone(&saw_interrupt));
        let st = run_timed(k.clone(), Duration::from_millis(50), move |flag| {
            s.fetch_add(1, Ordering::SeqCst);
            let _ = release_rx.recv();
            i.store(flag.load(Ordering::SeqCst), Ordering::SeqCst);
            GitState::NotARepo
        });
        assert!(
            matches!(st, GitState::Unavailable(GitError::Timeout)),
            "{st:?}"
        );
        let s2 = Arc::clone(&started);
        let second = run_timed(k.clone(), Duration::from_secs(5), move |_| {
            s2.fetch_add(1, Ordering::SeqCst);
            GitState::NotARepo
        });
        assert!(matches!(second, GitState::Unavailable(GitError::Paused)));
        release_tx.send(()).unwrap();
        wait_released(&k);
        assert_eq!(started.load(Ordering::SeqCst), 1, "a second worker started");
        assert!(
            saw_interrupt.load(Ordering::SeqCst),
            "the interrupt flag was not set"
        );
        // Once released, the library runs again.
        let third = run_timed(k, Duration::from_secs(5), |_| GitState::NotARepo);
        assert!(matches!(third, GitState::NotARepo));
    }

    #[test]
    fn a_panicking_worker_is_unavailable_and_released() {
        let k = key("panics");
        let st = run_timed(k.clone(), Duration::from_secs(5), |_| panic!("boom"));
        assert!(
            matches!(st, GitState::Unavailable(GitError::Failed(_))),
            "{st:?}"
        );
        wait_released(&k);
    }
}
