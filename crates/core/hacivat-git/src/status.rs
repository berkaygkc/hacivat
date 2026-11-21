//! The status pipeline and its timed worker (HV-SCRIPT-11 steps 3–6; `alg/28` C12, m3, n3, n4).
//!
//! Everything, discovery and the small reads of `trust` included, runs on one worker thread per
//! call. The caller waits at most `GitLimits::timeout`; at the deadline it answers
//! `Unavailable(Timeout)`, sets the interrupt flag and returns **without joining** the worker,
//! because `gix`'s blocking opens of `.gitignore`, `.gitattributes` (and a FIFO in their place)
//! cannot be interrupted (`alg/28` §0.8). While that worker still runs, a new call for the same
//! library answers `Unavailable(Timeout)` at once: at most one stuck worker per library.
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

use crate::discover::{self, Discovery, Found};
use crate::map::{self, Badges};
use crate::trust;
use crate::{FileStatus, GitError, GitLimits, GitState, GitStatus, UnknownWhy};

/// The libraries whose worker is running (keyed by the root as the caller gave it).
static RUNNING: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

fn running() -> std::sync::MutexGuard<'static, BTreeSet<PathBuf>> {
    RUNNING.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Removes its library from [`RUNNING`] when the worker ends (also on a panic).
struct Running(PathBuf);

impl Drop for Running {
    fn drop(&mut self) {
        running().remove(&self.0);
    }
}

fn failed(msg: impl Into<String>) -> GitState {
    GitState::Unavailable(GitError::Failed(msg.into()))
}

/// The whole status of `library_root` on the timed worker; `dataless` adds library-relative
/// paths to treat as cloud placeholders (the `testing` hook; empty in the product).
pub(crate) fn status_with(
    library_root: &Path,
    limits: &GitLimits,
    dataless: Vec<String>,
) -> GitState {
    let root = library_root.to_path_buf();
    let limits = *limits;
    run_timed(library_root.to_path_buf(), limits.timeout, move |interrupt| {
        pipeline(&root, &limits, &dataless, interrupt)
    })
}

/// Runs `work` on a worker thread and waits at most `timeout` for its answer (m3).
pub(crate) fn run_timed<F>(key: PathBuf, timeout: Duration, work: F) -> GitState
where
    F: FnOnce(&AtomicBool) -> GitState + Send + 'static,
{
    if !running().insert(key.clone()) {
        return GitState::Unavailable(GitError::Timeout);
    }
    let interrupt = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&interrupt);
    let (tx, rx) = mpsc::sync_channel(1);
    let guard = Running(key);
    let spawned = std::thread::Builder::new()
        .name("hacivat-repo-status".into())
        .spawn(move || {
            let guard = guard;
            let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&flag)))
                .unwrap_or_else(|_| failed("the repository status stopped unexpectedly"));
            // Leave RUNNING before answering, so that a call right after this answer starts.
            drop(guard);
            let _ = tx.send(out);
        });
    if let Err(e) = spawned {
        return failed(format!("the repository status could not start: {e}"));
    }
    match rx.recv_timeout(timeout) {
        Ok(st) => st,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            interrupt.store(true, Ordering::SeqCst);
            GitState::Unavailable(GitError::Timeout)
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            failed("the repository status stopped unexpectedly")
        }
    }
}

/// Discover, check, open, check again, collect.
fn pipeline(
    root: &Path,
    limits: &GitLimits,
    extra_dataless: &[String],
    interrupt: &AtomicBool,
) -> GitState {
    let root = match std::fs::canonicalize(root) {
        Ok(p) => p,
        Err(e) => return failed(format!("the library folder could not be read: {e}")),
    };
    let found = match discover::find(&root) {
        Discovery::Found(f) => f,
        Discovery::NotARepo => return GitState::NotARepo,
        Discovery::NonUtf8Prefix => {
            return failed("the library's path inside its repository is not UTF-8");
        }
    };
    let layout = match trust::check_layout(&found, &trust::current_user_owns) {
        Ok(l) => l,
        Err(st) => return st,
    };
    if trust::unsupported_format(&layout.common_dir) {
        return GitState::Unavailable(GitError::Unsupported);
    }
    let options = gix::open::Options::isolated().with(gix::sec::Trust::Reduced);
    let repo = match gix::open_opts(&found.worktree, options) {
        Ok(r) => r,
        Err(e) => return failed(format!("the repository could not be opened: {e}")),
    };
    if let Err(e) = trust::check_trust(&repo)
        .and_then(|()| trust::check_resolved(&repo, &layout, &found.worktree))
    {
        return GitState::Unavailable(e);
    }
    match collect(&repo, &found, &root, limits, extra_dataless, interrupt) {
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
                Some(_) => self.push(e.rela_path.as_bstr(), FileStatus::New),
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
fn gix_failed(what: &str) -> impl Fn(gix::Error) -> GitState + '_ {
    move |e| failed(format!("the repository {what} failed: {e}"))
}

/// The status proper, on an opened and checked repository.
fn collect(
    repo: &gix::Repository,
    found: &Found,
    library: &Path,
    limits: &GitLimits,
    extra_dataless: &[String],
    interrupt: &AtomicBool,
) -> Result<GitState, GitState> {
    let timed_out = || GitState::Unavailable(GitError::Timeout);
    let too_many = || {
        failed(format!(
            "the repository lists more than {} changed paths",
            limits.max_entries
        ))
    };
    let prefix = found.prefix.as_str();
    let index = repo.index_or_empty().map_err(gix_failed("index read"))?;
    let mut badges = Badges::default();

    // Gitlinks are Unknown(Submodule); dataless tracked files are Unknown(Dataless) and leave
    // the pathspec, so gix never opens them (I2: [Observed] it never reads an excluded path;
    // [Inferred] that this keeps the OS from downloading one).
    let extra: BTreeSet<BString> = extra_dataless
        .iter()
        .map(|l| {
            if prefix.is_empty() {
                BString::from(l.as_str())
            } else {
                BString::from(format!("{prefix}/{l}"))
            }
        })
        .collect();
    let mut excludes: Vec<BString> = Vec::new();
    let mut tracked: Vec<(BString, gix::index::entry::Mode)> = Vec::new();
    for entry in index.entries() {
        let path = entry.path(&index);
        if !map::under_prefix(path, prefix) {
            continue;
        }
        if entry.mode.is_submodule() {
            badges.add(path, FileStatus::Unknown(UnknownWhy::Submodule));
            continue;
        }
        let dataless = extra.contains(path)
            || gix::path::from_bstr(path)
                .ok()
                .and_then(|p| std::fs::symlink_metadata(found.worktree.join(p)).ok())
                .is_some_and(|m| map::is_dataless(&m));
        if dataless {
            if badges.get(path).is_none() {
                excludes.push(format!(":(exclude,top,literal){path}").into());
            }
            badges.add(path, FileStatus::Unknown(UnknownWhy::Dataless));
            continue;
        }
        tracked.push((path.to_owned(), entry.mode));
    }
    if interrupt.load(Ordering::SeqCst) {
        return Err(timed_out());
    }

    let mut patterns: Vec<BString> = Vec::new();
    if !prefix.is_empty() {
        patterns.push(format!(":(top,literal){prefix}").into());
    } else if !excludes.is_empty() {
        // Exclusions alone would match nothing: start from the whole worktree.
        patterns.push(":(top)".into());
    }
    patterns.extend(excludes);

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
        .map_err(gix_failed("walk setup"))?
        .emit_untracked(gix::dir::walk::EmissionMode::Matching);
    let options = gix::status::index_worktree::Options {
        sorting: None,
        dirwalk_options: Some(dirwalk),
        rewrites: None,
        thread_limit: None,
    };
    let walked = repo.index_worktree_status(
        &index,
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
    walked.map_err(gix_failed("worktree comparison"))?;
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
        .map_err(gix_failed("HEAD lookup"))?
        .detach();
    let mut pathspec = repo
        .pathspec(
            false,
            &patterns,
            true,
            &index,
            gix::worktree::stack::state::attributes::Source::WorktreeThenIdMapping,
        )
        .map_err(gix_failed("pathspec"))?;
    let mut staged: Vec<(BString, FileStatus)> = Vec::new();
    let mut overflow = false;
    repo.tree_index_status(
        &head_tree,
        &index,
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
    .map_err(gix_failed("HEAD comparison"))?;
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
            &index,
            gix::worktree::stack::state::attributes::Source::WorktreeThenIdMapping,
        )
        .map_err(gix_failed("attribute setup"))?;
    let changed = badges.paths();
    let mut probe: Vec<(BString, Option<gix::index::entry::Mode>)> = tracked
        .into_iter()
        .map(|(p, m)| (p, Some(m)))
        .collect();
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

    let by_path = badges.into_library(prefix, library, &map::is_dataless);
    if by_path.len() > limits.max_entries {
        return Err(too_many());
    }
    Ok(GitState::Ready(Arc::new(GitStatus { by_path })))
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
        assert!(matches!(st, GitState::Unavailable(GitError::Timeout)), "{st:?}");
        let s2 = Arc::clone(&started);
        let second = run_timed(k.clone(), Duration::from_secs(5), move |_| {
            s2.fetch_add(1, Ordering::SeqCst);
            GitState::NotARepo
        });
        assert!(matches!(second, GitState::Unavailable(GitError::Timeout)));
        release_tx.send(()).unwrap();
        wait_released(&k);
        assert_eq!(started.load(Ordering::SeqCst), 1, "a second worker started");
        assert!(saw_interrupt.load(Ordering::SeqCst), "the interrupt flag was not set");
        // Once released, the library runs again.
        let third = run_timed(k, Duration::from_secs(5), |_| GitState::NotARepo);
        assert!(matches!(third, GitState::NotARepo));
    }

    #[test]
    fn a_panicking_worker_is_unavailable_and_released() {
        let k = key("panics");
        let st = run_timed(k.clone(), Duration::from_secs(5), |_| panic!("boom"));
        assert!(matches!(st, GitState::Unavailable(GitError::Failed(_))), "{st:?}");
        wait_released(&k);
    }
}
