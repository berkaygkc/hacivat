//! The library watcher (HV-SCRIPT-08; R-M4b-14, -16): OS events through `notify` and
//! `notify-debouncer-full`, or polling, reduced to dirty-path hints that are reconciled
//! against the disk (`lstat`, `DiskStamp`, settle). No `notify` type is public.
//!
//! ## m4b-t01a-seams
//! Task 1a: the [`EventSource`] seam (`testing::FakeEventSource` drives the deterministic
//! tests) and the one way the OS debouncer is built, fixed by R-T0-2 (alg/28 C5, ADR 0009 D3):
//! `new_debouncer_opt::<_, RecommendedWatcher, NoCache>` with
//! `Config::default().with_follow_symlinks(false)`. Never `new_debouncer`, whose default
//! `FileIdMap` cache walks every added path with `follow_links(true)`, and never a following
//! config (`WatcherFollowsLinks`, `cargo xtask check-deps`).
//!
//! ## m4b-t09-watcher
//! Task 9 filled the seam (ADR 0009 `m4b-t09`):
//! 1. **Sources.** [`OsEventSource`] is the platform's recommended watcher, built through
//!    `new_debouncer_opt::<_, RecommendedWatcher, NoCache>` with [`watcher_config`], so no
//!    directory link is followed and no cache walks anything. A debounced event becomes one
//!    [`RawHint::Path`] per path; an event flagged for a rescan (FSEvents `MustScanSubDirs`,
//!    inotify `IN_Q_OVERFLOW`) becomes [`RawHint::Rescan`]; a watch-limit error becomes
//!    [`RawHint::Lost`]. Kinds are never trusted (alg/28 C3). [`PollEventSource`] hints each
//!    root every [`POLL_INTERVAL`], and the reconciler's scan finds what changed (not
//!    `notify`'s poll watcher, which misses same-second edits: [`poll`]).
//! 2. **Backend choice** ([`limits::backend_for`], R-M4b-14): a network file system or, on
//!    Linux, a library whose folders would take more than half the free inotify watches is
//!    polled. A source that fails to start, or reports [`RawHint::Lost`], is replaced by the
//!    poll source and the watcher says [`WatchEvent::BackendLost`] (then rescans).
//! 3. **Reconciliation** ([`Reconciler`]): a hint is a path, nothing more. A known file is
//!    looked at with `lstat` through [`pathsafe::join_with`] (every folder on the way
//!    re-validated, so nothing outside the library is touched); a folder, a link, the root or
//!    a new script name is reconciled by one [`scan`](crate::scan::scan) of the library (which
//!    never enters a directory link). Dot names, `node_modules`, other extensions and paths
//!    outside the root are dropped; a hint on `.git/index`, `HEAD` or `refs` is
//!    [`WatchEvent::RepositoryChanged`].
//! 4. **Settle** (alg/28 C6): a candidate is reported only when two looks one window apart
//!    agree; after [`WatchOpts::settle_tries`] looks it is reported as
//!    [`WatchEvent::Unsettled`].
//! 5. **Echo** ([`echo`]): a settled file whose `(len, mtime)` matches a recent write of
//!    Hacivat's is hashed and dropped when its whole stamp matches.
//! 6. **Renames** (alg/28 C4): a removal and a creation settled together whose content hash
//!    matches the removed file's last known hash (a tab read or saved it:
//!    [`Watcher::remember`]) are one [`WatchEvent::Renamed`].
//! 7. **Bounds:** more than [`MAX_HINTS_PER_WINDOW`] hints in one window, or more than
//!    [`HINT_CAPACITY`] waiting, is one [`WatchEvent::Rescan`]; undelivered events beyond
//!    [`MAX_OUTBOX`] collapse into one `Rescan` too.
//!
//! The watcher's own loop runs on a named thread (`hv-library-watch`, R-T9-5); the source is
//! unwatched and dropped on that thread, and the `notify` debouncer thread is joined in the
//! source's `Drop`.
//!
//! ## m4b-t09-watcher fix round 1 (review I1-I8; rulings R-T9-2..R-T9-5)
//! - **Racily clean files** (R-T9-3): an equal `(len, mtime)` younger than [`RACY_WINDOW`] (or
//!   the time's granularity) proves nothing. A hinted file a tab has open is re-hashed; any
//!   other one is [`WatchEvent::PossiblyChanged`]. An echo needs an equal content hash.
//! - **Renames** pair on a unique hash only; identical files stay removals and creations.
//! - **Unchanged trouble is quiet:** a scan stopped at the same limit with the same files, or a
//!   root still gone, is no event; a change of that state is one `Rescan`.
//! - **Bursts** count distinct paths over an extending window and become one `Rescan` once
//!   quiet; nothing settles while hints keep arriving (at most 2 s).
//! - **Linux** watches folder by folder ([`OsEventSource::per_directory`], R-T9-4) over
//!   [`Reconciler::watch_dirs`], never recursively through `notify`.
//! - **Windows** gets a root hint every [`BACKSTOP`] (R-T9-2).
//! - **Drop** waits at most [`DROP_JOIN_BOUND`], then abandons a thread stuck in a file-system
//!   call.
//!
//! ## m4b-t09-watcher fix round 2 (re-review N1, N2, m-fr1-1..6)
//! - Only names that pass the name filter count toward a burst (never `node_modules`, dot
//!   folders or other extensions); counting restarts after each `Rescan`; an open file's
//!   change is never learned silently by a rescan (it settles and is reported).
//! - A sync that adds folders is followed by one root look; a hint the watcher pushes itself
//!   wakes it at once.
//! - `.git/logs/HEAD` counts as a repository change; a far-future time is not racy forever; a
//!   hint through a linked folder re-syncs the folder watches; a backstop look that finds an
//!   unannounced change re-arms the source ([`WatchEvent::BackendRearmed`]).

pub mod debounce;
pub mod echo;
pub mod limits;
pub mod poll;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use icu_normalizer::ComposingNormalizerBorrowed;
use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, NoCache, new_debouncer_opt};
use tokio::sync::mpsc;

pub use self::debounce::{
    Debouncer, Flushed, HINT_CAPACITY, HintQueue, MAX_HINTS_PER_WINDOW, SettleQueue, Taken,
};
pub use self::echo::{ECHO_TTL, EchoHandle, EchoTable};
pub use self::limits::{FsKind, backend_for, choose_backend, inotify_budget};
pub use self::poll::{POLL_INTERVAL, PollEventSource};
pub use crate::service::PollReason;

use crate::fileio::DiskStamp;
use crate::pathsafe::{self, JoinedKind, PathError, RelPath};
use crate::registry::LibraryRoot;
use crate::scan::{self, EntryKind, Fs, Limits, LinkState, MetaKind, RealFs, ScanResult};

/// The debounce window (alg/28 HV-SCRIPT-08 Decision).
pub const DEBOUNCE: Duration = Duration::from_millis(150);

/// One hint from an event source. Kinds are never trusted (alg/28 C3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawHint {
    /// Something may have changed at this absolute path.
    Path(PathBuf),
    /// Events were dropped or coalesced under this absolute path: rescan it.
    Rescan(PathBuf),
    /// The backend stopped working; the service falls back to polling.
    Lost,
}

/// Where an event source delivers its batches of hints (one batch per debounce window).
pub type HintSink = Box<dyn FnMut(Vec<RawHint>) + Send>;

/// Why a watch did not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchError {
    /// Task 1a's stub watches nothing.
    NotImplemented,
    /// The backend refused (the text is the backend's, never file contents).
    Backend(String),
}

/// A source of hints for library roots: the OS watcher, the poller, or a test fake.
pub trait EventSource: Send {
    /// Starts delivering hints for everything under `root` to `sink`.
    fn watch(&mut self, root: &Path, sink: HintSink) -> Result<(), WatchError>;
    /// Stops delivering hints for `root`; unknown roots are ignored.
    fn unwatch(&mut self, root: &Path);

    // ## m4b-t09-watcher fix round 1 (defaulted, so `testing::FakeEventSource` is unchanged).

    /// Whether the source watches folders one by one (Linux inotify, R-T9-4) and needs
    /// [`sync_dirs`](Self::sync_dirs) after every scan.
    fn per_dir(&self) -> bool {
        false
    }

    /// The folders under `root` to watch now: absolute, found by the watcher's own
    /// `lstat`-verified, link-free walk ([`Reconciler::watch_dirs`]). A source that watches
    /// per folder adds the new ones and drops the vanished ones, and answers how many it
    /// added (fix round 2, N2: the watcher then looks at the root once more, for files made
    /// before their folder's watch); an error means it cannot (the watch limit), and the
    /// watcher falls back to polling.
    fn sync_dirs(&mut self, root: &Path, dirs: &BTreeSet<PathBuf>) -> Result<usize, WatchError> {
        let _ = (root, dirs);
        Ok(0)
    }

    /// How the last look at `root` went: whether anything changed, and what its scans cost.
    /// The poll source paces itself on it (2 s, backing off to 30 s while idle).
    fn pace(&mut self, root: &Path, changed: bool, cost: Duration) {
        let _ = (root, changed, cost);
    }
}

/// The configuration every OS and poll watcher is built with: directory links are never
/// followed (R-T0-2).
pub(crate) fn watcher_config() -> notify::Config {
    notify::Config::default().with_follow_symlinks(false)
}

/// The OS event source: one debouncer over the platform's recommended watcher.
pub struct OsEventSource {
    // ## m4b-t09-watcher: `Option` so that `Drop` can stop it (joining its thread), and the
    // per-root sinks its handler routes to.
    debouncer: Option<notify_debouncer_full::Debouncer<RecommendedWatcher, NoCache>>,
    sinks: Sinks,
    // ## m4b-t09-watcher fix round 1 (I5, R-T9-4): with `per_dir`, every folder is its own
    // non-recursive watch, added only for folders the watcher's walk verified.
    per_dir: bool,
    dirs: HashMap<PathBuf, BTreeSet<PathBuf>>,
}

impl OsEventSource {
    /// Builds the debouncer the only allowed way (R-T0-2). It watches nothing yet.
    ///
    /// ## m4b-t09-watcher fix round 1 (I5, R-T9-4): on Linux it watches folder by folder
    /// ([`per_directory`](Self::per_directory)); elsewhere it watches each root recursively.
    pub fn new() -> Result<Self, WatchError> {
        Self::build(cfg!(target_os = "linux"))
    }

    /// A source that never asks the backend for a recursive watch: the root and every folder
    /// [`EventSource::sync_dirs`] names get one non-recursive watch each. Linux's default
    /// (notify's recursive inotify walk follows a folder swapped for a link before it is
    /// watched, `walkdir`'s `follow_root_links`); on other systems for tests.
    pub fn per_directory() -> Result<Self, WatchError> {
        Self::build(true)
    }

    fn build(per_dir: bool) -> Result<Self, WatchError> {
        // ## m4b-t09-watcher: the handler routes each debounced batch to its root's sink.
        let sinks = Sinks::default();
        let debouncer = new_debouncer_opt::<_, RecommendedWatcher, NoCache>(
            DEBOUNCE,
            None,
            handler(sinks.clone()),
            NoCache,
            watcher_config(),
        )
        .map_err(|e| WatchError::Backend(e.to_string()))?;
        Ok(OsEventSource {
            debouncer: Some(debouncer),
            sinks,
            per_dir,
            dirs: HashMap::new(),
        })
    }
}

impl fmt::Debug for OsEventSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OsEventSource").finish_non_exhaustive()
    }
}

impl EventSource for OsEventSource {
    // ## m4b-t09-watcher: Task 1a's fail-closed stub replaced (named exception in this file).
    fn watch(&mut self, root: &Path, sink: HintSink) -> Result<(), WatchError> {
        let Some(debouncer) = self.debouncer.as_mut() else {
            return Err(WatchError::NotImplemented);
        };
        lock(&self.sinks).push((root.to_path_buf(), sink));
        let mode = if self.per_dir {
            RecursiveMode::NonRecursive
        } else {
            RecursiveMode::Recursive
        };
        if let Err(e) = debouncer.watch(root, mode) {
            lock(&self.sinks).retain(|(r, _)| r != root);
            return Err(WatchError::Backend(e.to_string()));
        }
        if self.per_dir {
            self.dirs
                .insert(root.to_path_buf(), BTreeSet::from([root.to_path_buf()]));
        }
        Ok(())
    }

    fn unwatch(&mut self, root: &Path) {
        // Fix round 2 (m-fr1-3): the debouncer's root list is a `Vec` scanned per watch and
        // per unwatch, so unwatching thousands of folders one by one is quadratic; for the
        // last root the whole debouncer is stopped instead (the source is not reused).
        let sole = lock(&self.sinks).iter().all(|(r, _)| r == root);
        if self.per_dir && sole {
            self.dirs.clear();
            if let Some(debouncer) = self.debouncer.take() {
                debouncer.stop();
            }
            lock(&self.sinks).clear();
            return;
        }
        if let Some(debouncer) = self.debouncer.as_mut() {
            for d in self.dirs.remove(root).unwrap_or_default() {
                if d != root {
                    let _ = debouncer.unwatch(&d);
                }
            }
            let _ = debouncer.unwatch(root);
        }
        lock(&self.sinks).retain(|(r, _)| r != root);
    }

    fn per_dir(&self) -> bool {
        self.per_dir
    }

    fn sync_dirs(&mut self, root: &Path, dirs: &BTreeSet<PathBuf>) -> Result<usize, WatchError> {
        if !self.per_dir {
            return Ok(0);
        }
        let (Some(debouncer), Some(have)) = (self.debouncer.as_mut(), self.dirs.get_mut(root))
        else {
            return Ok(0);
        };
        let mut added = 0;
        let gone: Vec<PathBuf> = have
            .iter()
            .filter(|d| *d != root && !dirs.contains(*d))
            .cloned()
            .collect();
        for d in gone {
            // A folder that vanished is unwatched by the kernel already; errors are moot.
            let _ = debouncer.unwatch(&d);
            have.remove(&d);
        }
        for d in dirs {
            if have.contains(d) || !d.starts_with(root) {
                continue;
            }
            match debouncer.watch(d, RecursiveMode::NonRecursive) {
                Ok(()) => {
                    have.insert(d.clone());
                    added += 1;
                }
                Err(e) if matches!(e.kind, notify::ErrorKind::MaxFilesWatch) => {
                    return Err(WatchError::Backend(e.to_string()));
                }
                // Gone (or not a folder any more) since the walk: the next walk decides.
                Err(_) => {}
            }
        }
        Ok(added)
    }
}
// ## m4b-t01a-seams

// ## m4b-t09-watcher

type Sinks = Arc<Mutex<Vec<(PathBuf, HintSink)>>>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Drop for OsEventSource {
    /// Stops the debouncer and joins its thread (the handler, and the sinks it holds, go with
    /// it); the platform watcher stops in its own `Drop`.
    fn drop(&mut self) {
        if let Some(debouncer) = self.debouncer.take() {
            debouncer.stop();
        }
        lock(&self.sinks).clear();
    }
}

/// The debouncer's handler: every debounced batch becomes hints for the roots it touches.
fn handler(sinks: Sinks) -> impl FnMut(DebounceEventResult) + Send + 'static {
    move |result| route(&sinks, result)
}

/// Splits one debounced batch into hints per watched root and hands each root its hints.
///
/// ## m4b-t09-watcher fix round 1 (m10): a path goes to every root that holds it, so nested
/// libraries sharing one source each see their own subtree.
fn route(sinks: &Sinks, result: DebounceEventResult) {
    let mut sinks = lock(sinks);
    let roots: Vec<PathBuf> = sinks.iter().map(|(r, _)| r.clone()).collect();
    let owners = |p: &Path| -> Vec<usize> {
        roots
            .iter()
            .enumerate()
            .filter(|(_, r)| p.starts_with(r))
            .map(|(i, _)| i)
            .collect()
    };
    let mut per: Vec<Vec<RawHint>> = vec![Vec::new(); roots.len()];
    let to_all = |per: &mut Vec<Vec<RawHint>>, make: &dyn Fn(&Path) -> RawHint| {
        for (i, r) in roots.iter().enumerate() {
            per[i].push(make(r));
        }
    };
    match result {
        Ok(events) => {
            for ev in events {
                if ev.need_rescan() {
                    if ev.paths.is_empty() {
                        to_all(&mut per, &|r| RawHint::Rescan(r.to_path_buf()));
                    }
                    for p in &ev.paths {
                        for i in owners(p) {
                            per[i].push(RawHint::Rescan(roots[i].clone()));
                        }
                    }
                    continue;
                }
                for p in &ev.paths {
                    for i in owners(p) {
                        per[i].push(RawHint::Path(p.clone()));
                    }
                }
            }
        }
        Err(errors) => {
            for e in errors {
                let lost = matches!(e.kind, notify::ErrorKind::MaxFilesWatch);
                if e.paths.is_empty() {
                    to_all(&mut per, &|r| {
                        if lost {
                            RawHint::Lost
                        } else {
                            RawHint::Rescan(r.to_path_buf())
                        }
                    });
                }
                for p in &e.paths {
                    for i in owners(p) {
                        per[i].push(if lost {
                            RawHint::Lost
                        } else {
                            RawHint::Path(p.clone())
                        });
                    }
                }
            }
        }
    }
    for ((_, sink), hints) in sinks.iter_mut().zip(per) {
        if !hints.is_empty() {
            sink(hints);
        }
    }
}

/// How a library is watched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// FSEvents, inotify or `ReadDirectoryChangesW`.
    Os,
    /// Hacivat's poll source, every [`POLL_INTERVAL`] or slower while idle.
    Poll(PollReason),
    /// ## m4b-t09-watcher fix round 1 (m4): the OS backend was lost and no fallback could
    /// watch the library either; nothing watches it (a diagnostic; Refresh still scans).
    Unwatched,
}

/// Why the OS backend was given up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostReason {
    /// The source refused to watch the root.
    StartFailed,
    /// The source reported that it stopped working (the watch limit was reached).
    Failed,
    /// ## m4b-t09-watcher fix round 1 (I5): the library has more folders than per-folder
    /// watches allow ([`WatchOpts::max_dirs`]), or the kernel refused one.
    WatchBudget,
    /// ## m4b-t09-watcher fix round 1 (m4): the fallback could not be built or could not
    /// watch; [`Backend::Unwatched`].
    NoFallback,
}

/// What the watcher reports, after reconciliation and echo filtering. Paths are files
/// (`.sql`/`.md`) relative to the library root; folders are implied by the files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// A known file has new contents (its stamp settled).
    Changed(RelPath),
    /// A new file appeared (its stamp settled).
    Created(RelPath),
    /// A known file is gone (it stayed gone for one window).
    Removed(RelPath),
    /// A removed file reappeared elsewhere with the same contents (alg/28 C4).
    Renamed { from: RelPath, to: RelPath },
    /// A file whose stamp was still changing after [`WatchOpts::settle_tries`] looks: treat
    /// it as changed or created, knowing a writer may still be at work (alg/28 step 5).
    Unsettled(RelPath),
    /// ## m4b-t09-watcher fix round 1 (I1, R-T9-3): a hint arrived for a known file whose
    /// `(len, mtime)` did not change, but whose time is too recent to prove it unchanged (the
    /// file system's time granularity, or 2 s), and whose contents were never hashed (no tab
    /// has it open). A tab that has it open re-reads or re-hashes it; nothing else needs to.
    PossiblyChanged(RelPath),
    /// Anything may have changed: rescan the library (a burst, a dropped-events signal, a
    /// root that moved or is gone, a scan that hit a limit).
    Rescan,
    /// The repository's index, `HEAD` or refs changed (refresh the status, HV-SCRIPT-11).
    RepositoryChanged,
    /// The OS backend stopped; the library is polled from now on (a diagnostic).
    BackendLost(LostReason),
    /// ## m4b-t09-watcher fix round 2 (m-fr1-5): a backstop look found a change no event had
    /// announced, so the root's watch was set up again (a diagnostic: the backend had stopped
    /// silently, as `ReadDirectoryChangesW` can after an overflow).
    BackendRearmed,
}

/// One delivery of events.
pub type WatchBatch = Vec<WatchEvent>;

/// Undelivered events kept at most while the receiver is full; past it they collapse into one
/// [`WatchEvent::Rescan`].
pub const MAX_OUTBOX: usize = 10_000;

/// How a watcher works. `Default` is the R-M4b-14 / HV-SCRIPT-08 configuration.
#[derive(Clone)]
pub struct WatchOpts {
    /// The settle and burst window.
    pub window: Duration,
    pub max_hints_per_window: usize,
    pub hint_capacity: usize,
    /// Looks before a still-changing file is reported anyway.
    pub settle_tries: u32,
    /// The scan's limits (the same as the service's).
    pub limits: Limits,
    /// What the caller chose the source for (reported by [`Watcher::backend`]).
    pub backend: Backend,
    /// The file system the reconciler looks through (tests count the paths it is asked for).
    pub fs: Arc<dyn Fs + Send + Sync>,
    /// ## m4b-t09-watcher fix round 1 (I5): the most folders a per-folder source may watch
    /// (half the free inotify watches, R-M4b-14); past it the library is polled.
    pub max_dirs: usize,
    /// ## m4b-t09-watcher fix round 1 (I8, R-T9-2): a root hint this often, whatever the
    /// backend says (Windows: 30 s, because `ReadDirectoryChangesW` can stop silently).
    pub backstop: Option<Duration>,
}

/// The Windows backstop interval (R-T9-2).
pub const BACKSTOP: Duration = Duration::from_secs(30);

impl Default for WatchOpts {
    fn default() -> Self {
        WatchOpts {
            window: DEBOUNCE,
            max_hints_per_window: MAX_HINTS_PER_WINDOW,
            hint_capacity: HINT_CAPACITY,
            settle_tries: 10,
            limits: Limits::default(),
            backend: Backend::Os,
            fs: Arc::new(RealFs),
            max_dirs: 100_000,
            backstop: cfg!(windows).then_some(BACKSTOP),
        }
    }
}

impl fmt::Debug for WatchOpts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WatchOpts")
            .field("window", &self.window)
            .field("max_hints_per_window", &self.max_hints_per_window)
            .field("hint_capacity", &self.hint_capacity)
            .field("settle_tries", &self.settle_tries)
            .field("limits", &self.limits)
            .field("backend", &self.backend)
            .field("max_dirs", &self.max_dirs)
            .field("backstop", &self.backstop)
            .finish_non_exhaustive()
    }
}

/// What a look at one file found (the scan's view of an entry).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    kind: EntryKind,
    link: LinkState,
    len: u64,
    mtime: SystemTime,
    dataless: bool,
    target: Option<RelPath>,
}

impl Seen {
    fn of(e: &scan::ScriptEntry) -> Seen {
        Seen {
            kind: e.kind,
            link: e.link,
            len: e.len,
            mtime: e.mtime,
            dataless: e.dataless,
            target: e.target.clone(),
        }
    }

    /// Equal for a reader: a file evicted to the cloud (or downloaded) has not changed.
    fn same_content(&self, o: &Seen) -> bool {
        self.kind == o.kind
            && self.link == o.link
            && self.len == o.len
            && self.mtime == o.mtime
            && self.target == o.target
    }

    fn hashable(&self) -> bool {
        self.link == LinkState::Plain && !self.dataless
    }
}

#[derive(Debug, Clone)]
struct Known {
    seen: Seen,
    /// The content hash when a tab or Hacivat's own write told us.
    hash: Option<u128>,
    /// ## m4b-t09-watcher fix round 1 (I1, R-T9-3): when it was last seen with this stamp
    /// (wall clock, taken before the look).
    seen_at: SystemTime,
}

#[derive(Debug, Clone)]
struct Candidate {
    last: Option<Seen>,
    tries: u32,
    /// Looked at by scans (a link, a new name, a folder change), not by `lstat`.
    via_scan: bool,
}

enum Look {
    Seen(Option<Seen>),
    /// Not a plain file any more (or a folder on the way changed): a scan decides.
    Escalate,
    RootGone,
}

/// What the name filter makes of a hint's path.
enum Named {
    Drop,
    Root,
    Repository,
    Rel(RelPath),
}

enum HintKind {
    Drop,
    Repository,
    File(RelPath),
    Scan,
}

/// What one scan says against the known files.
enum ScanOutcome {
    Diff(BTreeMap<RelPath, Option<Seen>>),
    /// The root or the limit state changed: the library was re-learned.
    Rescan,
    /// The root is still gone, or the scan stopped at the same limit with the same files.
    Quiet,
}

/// The names under `.git/` whose change means the status may have changed.
///
/// ## m4b-t09-watcher fix round 2 (m-fr1-1): `logs` too: `logs/HEAD` is appended on every move
/// of `HEAD`'s branch, so a ref-only change (`reset --soft`, `update-ref`) is seen.
const REPOSITORY_NAMES: [&str; 5] = ["index", "HEAD", "packed-refs", "refs", "logs"];

/// ## m4b-t09-watcher fix round 1 (I1, R-T9-3): below this age an equal `(len, mtime)` does not
/// prove a file unchanged; git calls such an entry "racily clean".
pub const RACY_WINDOW: Duration = Duration::from_secs(2);

/// The coarsest time step a modification time suggests: whole even seconds → 2 s (FAT), whole
/// seconds → 1 s (HFS+, ext3, many SMB servers), anything finer → none.
fn granularity(t: SystemTime) -> Duration {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) if d.subsec_nanos() == 0 && d.as_secs() % 2 == 0 => Duration::from_secs(2),
        Ok(d) if d.subsec_nanos() == 0 => Duration::from_secs(1),
        Ok(_) => Duration::ZERO,
        Err(_) => Duration::from_secs(2),
    }
}

/// Whether a file with time `mtime`, last seen with that stamp at `seen_at`, may have been
/// written again since without its `(len, mtime)` changing: its time is within the file
/// system's granularity, or within [`RACY_WINDOW`], of that look (or after it: clock skew).
///
/// ## m4b-t09-watcher fix round 2 (m-fr1-2): a time in the future is racy only while it is
/// within that window of the look; a file dated far ahead (clock skew, an extracted archive)
/// is not re-verified forever.
fn racy(mtime: SystemTime, seen_at: SystemTime) -> bool {
    let window = RACY_WINDOW.max(granularity(mtime));
    match seen_at.duration_since(mtime) {
        Ok(age) => age < window,
        Err(ahead) => ahead.duration() < window,
    }
}

/// At most this many open files are kept back from one rescan to be settled and reported on
/// their own (fix round 2, N1).
const MAX_OPEN_KEPT: usize = 256;

/// What the last [`Reconciler::tick`] did, for the watcher's bookkeeping.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TickInfo {
    /// Hints were looked at (the poll source paces itself on these ticks).
    pub saw_hints: bool,
    /// A scan ran (a per-folder source re-syncs its folders after one).
    pub scanned: bool,
    /// What the tick's scans cost.
    pub cost: Duration,
    /// ## m4b-t09-watcher fix round 2 (m-fr1-5): a backstop scan found a change no hint had
    /// announced: the source may have stopped silently (re-arm it).
    pub rearm: bool,
}

/// Hints in, events out: the deterministic heart of the watcher (clock-injected; tests drive
/// it with a real temp folder and chosen instants).
pub struct Reconciler {
    root: LibraryRoot,
    opts: WatchOpts,
    known: HashMap<RelPath, Known>,
    /// The NFC form of every known path that is not NFC itself (macOS keeps NFD names).
    nfc_alias: HashMap<String, RelPath>,
    /// The NFC form of every folder that holds a known file.
    folders: HashSet<String>,
    debouncer: Debouncer,
    settle: SettleQueue<RelPath>,
    candidates: BTreeMap<RelPath, Candidate>,
    scan_due: Option<Instant>,
    echo: EchoHandle,
    /// ## m4b-t09-watcher fix round 1 (I3): the limit the last scan stopped at, and the root
    /// problem it found, so an unchanged state is not one `Rescan` per poll pass.
    limited: Option<scan::LimitHit>,
    root_problem: Option<scan::RootProblem>,
    /// ## m4b-t09-watcher fix round 1 (I8, R-T9-2): the next backstop root hint.
    backstop_next: Option<Instant>,
    /// The wall clock at the start of the current tick (before any look).
    wall: SystemTime,
    /// The tick's own clock (fix round 2: candidates a rescan keeps back are due from it).
    now: Instant,
    /// A hint arrived since the last backstop (m-fr1-5).
    hinted_since_backstop: bool,
    info: TickInfo,
}

impl fmt::Debug for Reconciler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reconciler")
            .field("known", &self.known.len())
            .field("candidates", &self.candidates.len())
            .finish_non_exhaustive()
    }
}

impl Reconciler {
    /// Scans the library once (names and metadata only) to know what is there.
    pub fn new(root: LibraryRoot, opts: WatchOpts) -> Self {
        let mut r = Reconciler {
            debouncer: Debouncer::with_max(opts.window, opts.max_hints_per_window),
            root,
            opts,
            known: HashMap::new(),
            nfc_alias: HashMap::new(),
            folders: HashSet::new(),
            settle: SettleQueue::default(),
            candidates: BTreeMap::new(),
            scan_due: None,
            echo: EchoHandle::default(),
            limited: None,
            root_problem: None,
            backstop_next: None,
            wall: SystemTime::now(),
            now: Instant::now(),
            hinted_since_backstop: false,
            info: TickInfo::default(),
        };
        let started = SystemTime::now();
        let s = r.scan();
        r.learn_state(&s, started);
        r
    }

    /// The echo table the save path records into.
    pub fn echo(&self) -> EchoHandle {
        self.echo.clone()
    }

    /// Takes a hint that arrived at `at` (looked at by the next [`tick`](Self::tick)).
    ///
    /// ## m4b-t09-watcher fix round 2 (N1)
    /// A path hint first goes through the name filter `classify` starts with (string work
    /// only): a path outside the root, under a skipped folder (`node_modules`, `.git/objects`,
    /// any dot folder) or too deep is dropped here and never counts toward a burst. A name
    /// with another extension (build output, `*.js`) is still looked at, since it may be a
    /// folder, but is not counted either.
    pub fn push(&mut self, hint: RawHint, at: Instant) {
        let RawHint::Path(path) = hint else {
            self.debouncer.push(hint, at);
            return;
        };
        let counted = match self.name_filter(&path) {
            Named::Drop => return,
            Named::Root | Named::Repository => true,
            Named::Rel(rel) => {
                let name = rel.name();
                scan::script_kind(name).is_some()
                    || !name.contains('.')
                    || self.lookup(&rel).is_some()
            }
        };
        self.debouncer.push_path(path, at, counted);
    }

    /// The content stamp of `rel` as a tab read or saved it: kept while the file still has
    /// that length and time, so a later move of it can be paired by hash, and an edit that
    /// keeps its `(len, mtime)` is still seen (R-T9-3).
    pub fn remember(&mut self, rel: &RelPath, stamp: &DiskStamp) {
        if let Some(k) = self.known.get_mut(rel)
            && k.seen.len == stamp.len
            && k.seen.mtime == stamp.mtime
        {
            k.hash = Some(stamp.hash);
        }
    }

    /// ## m4b-t09-watcher fix round 1 (I8, R-T9-2): from `from` on, a root hint every
    /// [`WatchOpts::backstop`] (nothing when it is `None`).
    pub fn start_backstop(&mut self, from: Instant) {
        self.backstop_next = self.opts.backstop.map(|every| from + every);
    }

    /// When [`tick`](Self::tick) has work next.
    pub fn next_due(&self) -> Option<Instant> {
        // Settle looks wait while a burst is still arriving (never a busy wake-up).
        let hold = self.debouncer.hold_until();
        let held = |t: Option<Instant>| t.map(|t| hold.map_or(t, |h| t.max(h)));
        // Fix round 3 (R-T9-6): an open file's look is never held.
        let settle = self
            .settle
            .iter()
            .map(|(rel, t)| {
                if self.is_open(rel) {
                    *t
                } else {
                    hold.map_or(*t, |h| (*t).max(h))
                }
            })
            .min();
        [
            settle,
            held(self.scan_due),
            self.debouncer.due(),
            self.debouncer.pending_at(),
            self.backstop_next,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Whether a tab has `rel` open (its content hash is known: R-T9-6).
    fn is_open(&self, rel: &RelPath) -> bool {
        self.known.get(rel).is_some_and(|k| k.hash.is_some())
    }

    /// The files known now (for tests and diagnostics).
    pub fn known_files(&self) -> usize {
        self.known.len()
    }

    /// What the last tick did.
    pub fn last_tick(&self) -> TickInfo {
        self.info
    }

    /// Looks at what arrived and at what is due at `now`; answers the settled events.
    pub fn tick(&mut self, now: Instant) -> Vec<WatchEvent> {
        self.wall = SystemTime::now();
        self.now = now;
        self.info = TickInfo::default();
        let mut repository = false;
        let mut need_scan = false;
        let mut backstop_tick = false;
        if let Some(due) = self.backstop_next
            && due <= now
        {
            need_scan = true;
            backstop_tick = !self.hinted_since_backstop;
            self.hinted_since_backstop = false;
            self.backstop_next = self.opts.backstop.map(|every| now + every);
        }
        for flushed in self.debouncer.flush(now) {
            match flushed {
                // The watcher switches the source; nothing to reconcile.
                Flushed::Lost => {}
                Flushed::Rescan => return self.rescan(),
                Flushed::Paths(paths) => {
                    self.info.saw_hints = true;
                    self.hinted_since_backstop = true;
                    for p in paths {
                        match self.classify(&p) {
                            HintKind::Drop => {}
                            HintKind::Repository => repository = true,
                            HintKind::Scan => need_scan = true,
                            HintKind::File(rel) => match self.look(&rel) {
                                Look::RootGone => return self.rescan(),
                                Look::Escalate => need_scan = true,
                                Look::Seen(obs) => self.first_look(rel, obs, now),
                            },
                        }
                    }
                }
            }
        }

        // Fix round 1 (I7): while hints are still arriving, first looks happen but nothing
        // settles: a burst that overflows becomes one `Rescan`, never its first thousand files
        // one by one.
        let holding = self.debouncer.holding(now);
        let mut settled: Vec<(RelPath, Option<Seen>, bool)> = Vec::new();
        let mut due = self.settle.take_due(now);
        if holding {
            // Fix round 3 (R-T9-6): an open file settles even while a burst holds the rest;
            // the others wait for the hold's end (or the rescan, which drops them).
            let hold = self.debouncer.hold_until().unwrap_or(now);
            let (open, wait): (Vec<RelPath>, Vec<RelPath>) =
                due.into_iter().partition(|rel| self.is_open(rel));
            for rel in wait {
                self.settle.schedule(rel, hold);
            }
            due = open;
        }
        for rel in due {
            let Some(c) = self.candidates.get(&rel).cloned() else {
                continue;
            };
            if c.via_scan {
                continue;
            }
            match self.look(&rel) {
                Look::RootGone => return self.rescan(),
                Look::Escalate => {
                    self.candidates.remove(&rel);
                    need_scan = true;
                }
                Look::Seen(obs) => self.again(rel, c, obs, now, &mut settled),
            }
        }

        let scan_check = !holding && self.scan_due.is_some_and(|t| t <= now);
        if need_scan || scan_check {
            let diff = match self.scan_diff() {
                ScanOutcome::Diff(diff) => diff,
                // The scan re-learned the library (and kept open files' changes back).
                ScanOutcome::Rescan => {
                    let mut out = vec![WatchEvent::Rescan];
                    if repository {
                        out.push(WatchEvent::RepositoryChanged);
                    }
                    return out;
                }
                ScanOutcome::Quiet => {
                    self.scan_due = None;
                    self.candidates.retain(|_, c| !c.via_scan);
                    BTreeMap::new()
                }
            };
            if backstop_tick && !diff.is_empty() {
                self.info.rearm = true;
            }
            if scan_check {
                let scanned: Vec<(RelPath, Candidate)> = self
                    .candidates
                    .iter()
                    .filter(|(_, c)| c.via_scan)
                    .map(|(r, c)| (r.clone(), c.clone()))
                    .collect();
                for (rel, c) in scanned {
                    match diff.get(&rel) {
                        Some(obs) => self.again(rel, c, obs.clone(), now, &mut settled),
                        // Equal to what is known again.
                        None => {
                            self.candidates.remove(&rel);
                        }
                    }
                }
            }
            for (rel, obs) in diff {
                let fresh = self.candidates.get(&rel).is_none_or(|c| !c.via_scan);
                if fresh && !settled.iter().any(|(r, _, _)| *r == rel) {
                    self.settle.remove(&rel);
                    self.candidates.insert(
                        rel,
                        Candidate {
                            last: obs,
                            tries: 0,
                            via_scan: true,
                        },
                    );
                }
            }
            let pending = self.candidates.values().any(|c| c.via_scan);
            if scan_check || self.scan_due.is_none() {
                self.scan_due = pending.then(|| now + self.opts.window);
            }
        }

        let mut out = self.report(settled, now);
        if repository {
            out.push(WatchEvent::RepositoryChanged);
        }
        out
    }

    /// Forgets every pending look, re-learns the library from one scan and says `Rescan`.
    fn rescan(&mut self) -> Vec<WatchEvent> {
        let started = SystemTime::now();
        let s = self.scan();
        self.learn_state(&s, started);
        vec![WatchEvent::Rescan]
    }

    /// Takes a scan as the truth: the root and limit state, and (when the root is there) the
    /// known files, keeping the hashes of files that did not change.
    ///
    /// Every pending look is dropped. ## m4b-t09-watcher fix round 2 (N1): an open file (its
    /// hash is known) that changed or vanished is not learned silently: its old stamp is kept
    /// and it becomes a candidate, so it settles and is reported on its own (`Changed`,
    /// `Removed`), however many rescans a burst causes; at most [`MAX_OPEN_KEPT`] per rescan.
    fn learn_state(&mut self, s: &ScanResult, started: SystemTime) {
        self.candidates.clear();
        self.settle.clear();
        self.scan_due = None;
        self.root_problem = s.root.clone();
        self.limited = s.stopped;
        if s.root.is_some() {
            return;
        }
        let mut old = std::mem::take(&mut self.known);
        let scanned: HashMap<RelPath, Seen> = files_of(s).collect();
        let mut kept: Vec<(RelPath, Option<Seen>)> = Vec::new();
        for (rel, k) in &old {
            if k.hash.is_none() || kept.len() >= MAX_OPEN_KEPT {
                continue;
            }
            match scanned.get(rel) {
                Some(seen) if seen.same_content(&k.seen) => {}
                // Stopped at a limit: absence proves nothing.
                None if s.stopped.is_some() => {}
                obs => kept.push((rel.clone(), obs.cloned())),
            }
        }
        self.known = scanned
            .into_iter()
            .map(|(rel, seen)| {
                let hash = old
                    .get(&rel)
                    .filter(|k| k.seen.same_content(&seen))
                    .and_then(|k| k.hash);
                (
                    rel,
                    Known {
                        seen,
                        hash,
                        seen_at: started,
                    },
                )
            })
            .collect();
        let due = self.now + self.opts.window;
        for (rel, obs) in kept {
            if let Some(k) = old.remove(&rel) {
                self.known.insert(rel.clone(), k);
            }
            self.candidates.insert(
                rel.clone(),
                Candidate {
                    last: obs,
                    tries: 0,
                    via_scan: false,
                },
            );
            self.settle.schedule(rel, due);
        }
        self.reindex();
    }

    fn scan(&self) -> ScanResult {
        scan::scan(&self.root, &self.opts.limits, &*self.opts.fs)
    }

    /// One scan against the known files.
    ///
    /// ## m4b-t09-watcher fix round 1 (I3): a root that is still gone, or a scan that stops
    /// at the same limit with the same files as before, is `Quiet`; only a change of that
    /// state is a `Rescan` (before, every poll pass of a library at its limit was one).
    fn scan_diff(&mut self) -> ScanOutcome {
        let started = SystemTime::now();
        let clock = Instant::now();
        let s = self.scan();
        self.info.scanned = true;
        self.info.cost += clock.elapsed();
        if let Some(problem) = &s.root {
            if self.root_problem.as_ref() == Some(problem) {
                return ScanOutcome::Quiet;
            }
            self.root_problem = Some(problem.clone());
            self.candidates.clear();
            self.settle.clear();
            self.scan_due = None;
            return ScanOutcome::Rescan;
        }
        if self.root_problem.is_some() || s.stopped != self.limited {
            self.learn_state(&s, started);
            return ScanOutcome::Rescan;
        }
        let now: HashMap<RelPath, Seen> = files_of(&s).collect();
        if s.stopped.is_some() {
            let same = now.len() == self.known.len()
                && now
                    .iter()
                    .all(|(rel, seen)| self.known.get(rel).is_some_and(|k| k.seen == *seen));
            if same {
                return ScanOutcome::Quiet;
            }
            self.learn_state(&s, started);
            return ScanOutcome::Rescan;
        }
        let mut diff = BTreeMap::new();
        for (rel, k) in self.known.iter_mut() {
            match now.get(rel) {
                None => {
                    diff.insert(rel.clone(), None);
                }
                Some(seen) if *seen != k.seen => {
                    diff.insert(rel.clone(), Some(seen.clone()));
                }
                // R-T9-3: an open file whose equal stamp is too recent to trust is looked at
                // again (and re-hashed when it settles).
                Some(seen) if k.hash.is_some() && racy(seen.mtime, k.seen_at) => {
                    diff.insert(rel.clone(), Some(seen.clone()));
                }
                Some(_) => k.seen_at = started,
            }
        }
        for (rel, seen) in now {
            if !self.known.contains_key(&rel) {
                diff.insert(rel, Some(seen));
            }
        }
        ScanOutcome::Diff(diff)
    }

    /// The first look at a known file after a hint.
    fn first_look(&mut self, rel: RelPath, obs: Option<Seen>, now: Instant) {
        let known = self.known.get(&rel);
        let unchanged = match (&obs, known) {
            (Some(s), Some(k)) => *s == k.seen,
            (None, None) => true,
            _ => false,
        };
        // R-T9-3: a hint on a file whose equal stamp is too recent to prove anything.
        let racy_now = match (&obs, known) {
            (Some(s), Some(k)) => *s == k.seen && racy(s.mtime, k.seen_at),
            _ => false,
        };
        let pending = self.candidates.get(&rel);
        if unchanged && !racy_now && pending.is_none() {
            if let Some(k) = self.known.get_mut(&rel) {
                k.seen_at = self.wall;
            }
            return;
        }
        if pending.is_some_and(|c| c.via_scan) {
            // A scan is already settling it.
            return;
        }
        let tries = pending.map_or(0, |c| c.tries);
        self.candidates.insert(
            rel.clone(),
            Candidate {
                last: obs,
                tries,
                via_scan: false,
            },
        );
        self.settle.schedule(rel, now + self.opts.window);
    }

    /// A later look at a candidate: settled when it agrees with the last one.
    fn again(
        &mut self,
        rel: RelPath,
        c: Candidate,
        obs: Option<Seen>,
        now: Instant,
        settled: &mut Vec<(RelPath, Option<Seen>, bool)>,
    ) {
        if obs == c.last {
            self.candidates.remove(&rel);
            settled.push((rel, obs, false));
        } else if c.tries + 1 >= self.opts.settle_tries {
            self.candidates.remove(&rel);
            settled.push((rel, obs, true));
        } else {
            if !c.via_scan {
                self.settle.schedule(rel.clone(), now + self.opts.window);
            }
            self.candidates.insert(
                rel,
                Candidate {
                    last: obs,
                    tries: c.tries + 1,
                    via_scan: c.via_scan,
                },
            );
        }
    }

    /// Settled looks → events: renames paired by a unique hash, echoes dropped, racily clean
    /// files verified, the known files kept in step.
    fn report(
        &mut self,
        settled: Vec<(RelPath, Option<Seen>, bool)>,
        now: Instant,
    ) -> Vec<WatchEvent> {
        let wall = self.wall;
        let mut removed: Vec<RelPath> = Vec::new();
        let mut appeared: Vec<(RelPath, Seen, bool, bool)> = Vec::new(); // (rel, seen, new, forced)
        let mut verify: Vec<(RelPath, Seen, Known)> = Vec::new();
        for (rel, obs, forced) in settled {
            match (self.known.get(&rel), obs) {
                (None, None) => {}
                (Some(_), None) => removed.push(rel),
                (None, Some(s)) => appeared.push((rel, s, true, forced)),
                (Some(k), Some(s)) => {
                    if !k.seen.same_content(&s) {
                        appeared.push((rel, s, false, forced));
                    } else if racy(s.mtime, k.seen_at) {
                        verify.push((rel, s, k.clone()));
                    } else {
                        let hash = k.hash;
                        self.known.insert(
                            rel,
                            Known {
                                seen: s,
                                hash,
                                seen_at: wall,
                            },
                        );
                    }
                }
            }
        }
        if removed.is_empty() && appeared.is_empty() && verify.is_empty() {
            return Vec::new();
        }
        removed.sort();
        appeared.sort_by(|a, b| a.0.cmp(&b.0));
        verify.sort_by(|a, b| a.0.cmp(&b.0));

        let mut hashes: HashMap<RelPath, Option<DiskStamp>> = HashMap::new();
        let mut events = Vec::new();

        // Renames (fix round 1, I2): a removed file with a known hash and a new file of the
        // same length and hash, only when that hash is unique on both sides; an ambiguous
        // match (identical files) stays a removal and a creation.
        let mut removed_by_hash: HashMap<u128, Vec<RelPath>> = HashMap::new();
        let mut lens: HashSet<u64> = HashSet::new();
        for a in &removed {
            if let Some(Known {
                seen,
                hash: Some(h),
                ..
            }) = self.known.get(a)
            {
                removed_by_hash.entry(*h).or_default().push(a.clone());
                lens.insert(seen.len);
            }
        }
        let mut appeared_by_hash: HashMap<u128, Vec<RelPath>> = HashMap::new();
        if !removed_by_hash.is_empty() {
            for (b, s, new, _) in &appeared {
                if *new
                    && s.hashable()
                    && lens.contains(&s.len)
                    && let Some(st) = self.stamp_of(b, s, &mut hashes)
                {
                    appeared_by_hash.entry(st.hash).or_default().push(b.clone());
                }
            }
        }
        let mut pairs: Vec<(RelPath, RelPath)> = removed_by_hash
            .iter()
            .filter_map(
                |(h, froms)| match (froms.as_slice(), appeared_by_hash.get(h)) {
                    ([from], Some(tos)) if tos.len() == 1 => Some((from.clone(), tos[0].clone())),
                    _ => None,
                },
            )
            .collect();
        pairs.sort();
        let renamed_from: HashSet<RelPath> = pairs.iter().map(|(a, _)| a.clone()).collect();
        let paired: HashSet<RelPath> = pairs.iter().map(|(_, b)| b.clone()).collect();
        for (from, to) in pairs {
            events.push(WatchEvent::Renamed { from, to });
        }
        for a in &removed {
            self.known.remove(a);
            if !renamed_from.contains(a) {
                events.push(WatchEvent::Removed(a.clone()));
            }
        }

        for (rel, seen, new, forced) in appeared {
            let mut hash = None;
            let mut echo = false;
            if seen.hashable()
                && self
                    .echo
                    .lock()
                    .may_be_echo_at(&rel, seen.len, seen.mtime, now)
                && let Some(stamp) = self.stamp_of(&rel, &seen, &mut hashes)
            {
                hash = Some(stamp.hash);
                echo = self.echo.lock().is_echo_at(&rel, &stamp, now);
            }
            if paired.contains(&rel) {
                hash = hashes.get(&rel).copied().flatten().map(|s| s.hash);
            } else if !echo {
                events.push(if forced {
                    WatchEvent::Unsettled(rel.clone())
                } else if new {
                    WatchEvent::Created(rel.clone())
                } else {
                    WatchEvent::Changed(rel.clone())
                });
            }
            self.known.insert(
                rel,
                Known {
                    seen,
                    hash,
                    seen_at: wall,
                },
            );
        }

        // Racily clean (fix round 1, I1, R-T9-3): the stamp proves nothing. With a known hash
        // the file is hashed again: another hash is a change, unless it is Hacivat's own
        // write (the echo needs an equal content hash). Without one, nobody has it open, and
        // the consumer is told it may have changed.
        for (rel, seen, k) in verify {
            let mut hash = k.hash;
            match k.hash {
                Some(h) if seen.hashable() => match self.stamp_of(&rel, &seen, &mut hashes) {
                    Some(st) if st.hash == h => {}
                    Some(st) => {
                        hash = Some(st.hash);
                        if !self.echo.lock().is_echo_at(&rel, &st, now) {
                            events.push(WatchEvent::Changed(rel.clone()));
                        }
                    }
                    None => {
                        hash = None;
                        events.push(WatchEvent::PossiblyChanged(rel.clone()));
                    }
                },
                _ => events.push(WatchEvent::PossiblyChanged(rel.clone())),
            }
            self.known.insert(
                rel,
                Known {
                    seen,
                    hash,
                    seen_at: wall,
                },
            );
        }
        self.reindex();
        events
    }

    /// The stamp of the plain file `rel` (hashed at most once per report), when it still has
    /// the length and time `seen` says.
    fn stamp_of(
        &self,
        rel: &RelPath,
        seen: &Seen,
        cache: &mut HashMap<RelPath, Option<DiskStamp>>,
    ) -> Option<DiskStamp> {
        if let Some(hit) = cache.get(rel) {
            return *hit;
        }
        let stamp = match pathsafe::join_with(&*self.opts.fs, &self.root, rel) {
            Ok(j) if j.kind == JoinedKind::File && j.target.is_none() => {
                echo::stamp_if_len(&j.path, seen.len)
                    .ok()
                    .flatten()
                    .filter(|s| s.mtime == seen.mtime)
            }
            _ => None,
        };
        cache.insert(rel.clone(), stamp);
        stamp
    }

    /// ## m4b-t09-watcher fix round 1 (I5, R-T9-4): the folders a per-folder source watches:
    /// the root, its `.git` folder (for `HEAD`, `index` and `packed-refs`; nothing below it),
    /// and every folder the scan would enter, found by our own walk through the [`Fs`] seam:
    /// `lstat` only, a link is never entered, skipped names (`.`, `node_modules`) and
    /// dataless folders are not watched, depth and names visited bounded like the scan.
    /// `Err(count)` once there are more than [`WatchOpts::max_dirs`].
    pub fn watch_dirs(&self) -> Result<BTreeSet<PathBuf>, usize> {
        let fs = &*self.opts.fs;
        let root = &self.root.canonical;
        let mut out = BTreeSet::from([root.clone()]);
        let git = root.join(".git");
        if fs
            .symlink_metadata(&git)
            .is_ok_and(|m| m.kind == MetaKind::Dir && !m.dataless)
        {
            // Fix round 2 (m-fr1-1): and `.git/logs`, for `logs/HEAD` (ref-only moves).
            let logs = git.join("logs");
            if fs
                .symlink_metadata(&logs)
                .is_ok_and(|m| m.kind == MetaKind::Dir && !m.dataless)
            {
                out.insert(logs);
            }
            out.insert(git);
        }
        let limits = &self.opts.limits;
        let mut visited = 0usize;
        let mut stack = vec![(root.clone(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            if depth + 1 >= limits.max_depth {
                continue;
            }
            let Ok(names) = fs.read_dir(&dir, limits.max_visited) else {
                continue;
            };
            for name in names {
                visited += 1;
                if visited > limits.max_visited {
                    return Ok(out);
                }
                let Some(n) = name.to_str() else {
                    continue;
                };
                if pathsafe::is_skipped(n) || pathsafe::component(&name).is_err() {
                    continue;
                }
                let p = dir.join(n);
                if fs
                    .symlink_metadata(&p)
                    .is_ok_and(|m| m.kind == MetaKind::Dir && !m.dataless)
                {
                    out.insert(p.clone());
                    if out.len() > self.opts.max_dirs {
                        return Err(out.len());
                    }
                    stack.push((p, depth + 1));
                }
            }
        }
        Ok(out)
    }

    /// What a hint's path is to this library.
    /// The string-only part of [`classify`](Self::classify): no file-system call.
    fn name_filter(&self, path: &Path) -> Named {
        let rest = match path.strip_prefix(&self.root.canonical) {
            Ok(r) => r,
            Err(_) => match path.strip_prefix(&self.root.path) {
                Ok(r) => r,
                Err(_) => return Named::Drop,
            },
        };
        let mut parts: Vec<&str> = Vec::new();
        for c in rest.components() {
            let Component::Normal(name) = c else {
                return Named::Drop;
            };
            let Some(name) = name.to_str() else {
                return Named::Drop;
            };
            parts.push(name);
        }
        let Some(first) = parts.first() else {
            return Named::Root;
        };
        if first.eq_ignore_ascii_case(".git") {
            let status = parts.get(1).is_some_and(|n| REPOSITORY_NAMES.contains(n));
            return if status {
                Named::Repository
            } else {
                Named::Drop
            };
        }
        if parts.iter().any(|p| pathsafe::is_skipped(p)) || parts.len() > self.opts.limits.max_depth
        {
            return Named::Drop;
        }
        match RelPath::parse(&parts.join("/")) {
            Ok(rel) => Named::Rel(rel),
            Err(_) => Named::Drop,
        }
    }

    fn classify(&self, path: &Path) -> HintKind {
        let rel = match self.name_filter(path) {
            Named::Drop => return HintKind::Drop,
            Named::Root => return HintKind::Scan,
            Named::Repository => return HintKind::Repository,
            Named::Rel(rel) => rel,
        };
        if let Some(known) = self.lookup(&rel) {
            return HintKind::File(known);
        }
        if self.folders.contains(nfc(rel.as_str()).as_str()) {
            return HintKind::Scan;
        }
        if scan::script_kind(rel.name()).is_some() {
            return HintKind::Scan;
        }
        match pathsafe::join_with(&*self.opts.fs, &self.root, &rel) {
            Ok(j) if j.kind == JoinedKind::Folder => HintKind::Scan,
            // Fix round 2 (m-fr1-4): a hint through a folder that became a link means a watch
            // may sit on its target; the scan re-syncs the folder watches, which drops it.
            Err(PathError::LinkOnPath) => HintKind::Scan,
            _ => HintKind::Drop,
        }
    }

    /// The known path a hint names, byte for byte or by its NFC form.
    fn lookup(&self, rel: &RelPath) -> Option<RelPath> {
        if self.known.contains_key(rel) {
            return Some(rel.clone());
        }
        let n = nfc(rel.as_str());
        if let Ok(as_nfc) = RelPath::parse(&n)
            && self.known.contains_key(&as_nfc)
        {
            return Some(as_nfc);
        }
        self.nfc_alias.get(n.as_str()).cloned()
    }

    /// `lstat` of a known file, through folders re-validated one by one.
    fn look(&self, rel: &RelPath) -> Look {
        let fs = &*self.opts.fs;
        match pathsafe::join_with(fs, &self.root, rel) {
            Ok(j) => match (j.kind, j.target.is_some()) {
                (JoinedKind::Absent, _) => Look::Seen(None),
                (JoinedKind::File, false) => {
                    let Some(kind) = scan::script_kind(rel.name()) else {
                        return Look::Escalate;
                    };
                    match fs.symlink_metadata(&j.path) {
                        Ok(m) if m.kind == MetaKind::File => Look::Seen(Some(Seen {
                            kind,
                            link: LinkState::Plain,
                            len: m.len,
                            mtime: m.mtime,
                            dataless: m.dataless,
                            target: None,
                        })),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Look::Seen(None),
                        _ => Look::Escalate,
                    }
                }
                _ => Look::Escalate,
            },
            Err(PathError::RootChanged | PathError::RootInEnvFolder) => Look::RootGone,
            Err(_) => Look::Escalate,
        }
    }

    fn reindex(&mut self) {
        self.nfc_alias.clear();
        self.folders.clear();
        for rel in self.known.keys() {
            let n = nfc(rel.as_str());
            for (i, _) in n.match_indices('/') {
                self.folders.insert(n[..i].to_string());
            }
            if n != rel.as_str() {
                self.nfc_alias.insert(n, rel.clone());
            }
        }
    }
}

fn files_of(s: &ScanResult) -> impl Iterator<Item = (RelPath, Seen)> + '_ {
    s.entries
        .iter()
        .filter(|e| e.kind != EntryKind::Folder)
        .map(|e| (e.rel.clone(), Seen::of(e)))
}

fn nfc(s: &str) -> String {
    ComposingNormalizerBorrowed::new_nfc()
        .normalize(s)
        .into_owned()
}

/// Builds the source to fall back to when the OS backend fails.
pub type Fallback = Box<dyn FnMut() -> Result<Box<dyn EventSource>, WatchError> + Send>;

/// The default fallback: the poll source.
fn poll_fallback() -> Fallback {
    Box::new(|| Ok(Box::new(PollEventSource::new()?) as Box<dyn EventSource>))
}

/// How long dropping a [`Watcher`] waits for its thread before abandoning it (I6).
pub const DROP_JOIN_BOUND: Duration = Duration::from_millis(250);

/// Watches one library: a source, the reconciler on its own thread, and the events delivered
/// to `sink`.
///
/// ## m4b-t09-watcher fix round 1 (I6)
/// Dropping it signals the thread to stop and waits at most [`DROP_JOIN_BOUND`] for it to end
/// (it ends at its next wake-up and then unwatches and drops its source); a thread stuck in a
/// file-system call (a hung network share) is abandoned, never waited for, and ends on its
/// own when the call returns. So a drop never blocks for long, on any thread.
pub struct Watcher {
    queue: Arc<HintQueue>,
    thread: Option<JoinHandle<()>>,
    done: std::sync::mpsc::Receiver<()>,
    echo: EchoHandle,
    backend: Arc<Mutex<Backend>>,
    token: Weak<()>,
}

impl fmt::Debug for Watcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Watcher")
            .field("backend", &self.backend())
            .finish_non_exhaustive()
    }
}

/// Whether a dropped watcher left anything running (for tests and diagnostics): its thread,
/// or a source still holding its hint queue.
#[derive(Debug, Clone)]
pub struct WatcherProbe {
    token: Weak<()>,
    queue: Weak<HintQueue>,
}

impl WatcherProbe {
    pub fn released(&self) -> bool {
        self.token.upgrade().is_none() && self.queue.upgrade().is_none()
    }
}

struct Running {
    root: LibraryRoot,
    window: Duration,
    queue: Arc<HintQueue>,
    source: Option<Box<dyn EventSource>>,
    fallback: Fallback,
    rec: Reconciler,
    sink: mpsc::Sender<WatchBatch>,
    outbox: Vec<WatchEvent>,
    backend: Arc<Mutex<Backend>>,
    // Dropped last (fields drop in order): the probe and the drop's wait see the end only
    // after the source is gone.
    _done: std::sync::mpsc::Sender<()>,
    _token: Arc<()>,
}

fn hint_sink(queue: &Arc<HintQueue>) -> HintSink {
    let queue = queue.clone();
    Box::new(move |batch| queue.push(batch))
}

/// The folders a per-folder source must watch now, synced; `Err` when it cannot (too many
/// folders, or the kernel refused one).
fn sync_dirs(
    source: &mut dyn EventSource,
    rec: &Reconciler,
    root: &Path,
) -> Result<usize, LostReason> {
    if !source.per_dir() {
        return Ok(0);
    }
    let dirs = rec.watch_dirs().map_err(|_| LostReason::WatchBudget)?;
    source
        .sync_dirs(root, &dirs)
        .map_err(|_| LostReason::WatchBudget)
}

impl Watcher {
    /// Picks the backend for `root` (R-M4b-14: [`backend_for`]) and starts watching it.
    ///
    /// ## m4b-t09-watcher fix round 1 (m6): `opts.backend` set to `Backend::Poll(_)` by the
    /// caller (R-T9-1's per-library "poll" choice) is kept; otherwise the backend is measured.
    /// On Linux the per-folder budget ([`WatchOpts::max_dirs`]) is half the free inotify
    /// watches.
    pub fn open(
        root: LibraryRoot,
        mut opts: WatchOpts,
        sink: mpsc::Sender<WatchBatch>,
    ) -> Result<Watcher, WatchError> {
        if !matches!(opts.backend, Backend::Poll(_)) {
            opts.backend = backend_for(&root.canonical);
        }
        if let Some(free) = inotify_budget() {
            opts.max_dirs = usize::try_from(free / 2).unwrap_or(usize::MAX);
        }
        let source: Result<Box<dyn EventSource>, WatchError> = match opts.backend {
            Backend::Os => OsEventSource::new().map(|s| Box::new(s) as Box<dyn EventSource>),
            Backend::Poll(_) | Backend::Unwatched => {
                PollEventSource::new().map(|s| Box::new(s) as Box<dyn EventSource>)
            }
        };
        let source = source.unwrap_or_else(|e| Box::new(Refusing(e)));
        Watcher::start(root, opts, sink, source)
    }

    /// Starts watching `root` with `source`, falling back to polling if it fails.
    ///
    /// Blocks while the source registers the tree and while the library is scanned once: call
    /// it off the UI thread and off an async executor (the core runtime's blocking pool).
    pub fn start(
        root: LibraryRoot,
        opts: WatchOpts,
        sink: mpsc::Sender<WatchBatch>,
        source: Box<dyn EventSource>,
    ) -> Result<Watcher, WatchError> {
        Watcher::start_with_fallback(root, opts, sink, source, poll_fallback())
    }

    /// [`start`](Self::start) with the source to fall back to.
    pub fn start_with_fallback(
        root: LibraryRoot,
        opts: WatchOpts,
        sink: mpsc::Sender<WatchBatch>,
        mut source: Box<dyn EventSource>,
        mut fallback: Fallback,
    ) -> Result<Watcher, WatchError> {
        let queue = Arc::new(HintQueue::new(opts.hint_capacity));
        let mut outbox = Vec::new();
        let mut backend = opts.backend;
        if let Err(e) = source.watch(&root.canonical, hint_sink(&queue)) {
            let mut polled = fallback().map_err(|_| e.clone())?;
            polled.watch(&root.canonical, hint_sink(&queue))?;
            source = polled;
            backend = Backend::Poll(PollReason::BackendLost);
            outbox.push(WatchEvent::BackendLost(LostReason::StartFailed));
        }
        // After the watch: a change between the scan and the first event is seen either by the
        // scan or by a hint.
        let window = opts.window;
        let mut rec = Reconciler::new(root.clone(), opts);
        let start = Instant::now();
        if let Err(reason) = sync_dirs(source.as_mut(), &rec, &root.canonical) {
            // Too many folders for one watch each: poll from the start.
            source.unwatch(&root.canonical);
            let mut polled = fallback()?;
            polled.watch(&root.canonical, hint_sink(&queue))?;
            source = polled;
            backend = Backend::Poll(PollReason::WatchBudget);
            outbox.push(WatchEvent::BackendLost(reason));
        } else if source.per_dir() {
            // Folders made between the scan and their watch: one look once they are watched.
            rec.push(RawHint::Path(root.canonical.clone()), start);
        }
        if backend == Backend::Os {
            rec.start_backstop(start);
        }
        let echo = rec.echo();
        let backend = Arc::new(Mutex::new(backend));
        let token = Arc::new(());
        let probe = Arc::downgrade(&token);
        let (done_tx, done) = std::sync::mpsc::channel();
        let running = Running {
            root,
            window,
            queue: queue.clone(),
            source: Some(source),
            fallback,
            rec,
            sink,
            outbox,
            backend: backend.clone(),
            _done: done_tx,
            _token: token,
        };
        let thread = std::thread::Builder::new()
            .name("hv-library-watch".into())
            .spawn(move || running.run())
            .map_err(|e| WatchError::Backend(e.to_string()))?;
        Ok(Watcher {
            queue,
            thread: Some(thread),
            done,
            echo,
            backend,
            token: probe,
        })
    }

    /// The table `fileio::save_with_echo`'s callback records into.
    pub fn echo(&self) -> EchoHandle {
        self.echo.clone()
    }

    /// The content stamp a tab read or saved: for rename pairing, and so an edit that keeps
    /// the file's `(len, mtime)` is still seen (R-T9-3).
    pub fn remember(&self, rel: RelPath, stamp: DiskStamp) {
        self.queue.remember(rel, stamp);
    }

    /// How the library is watched now.
    pub fn backend(&self) -> Backend {
        *lock(&self.backend)
    }

    /// A probe that tells whether everything this watcher started has stopped.
    pub fn probe(&self) -> WatcherProbe {
        WatcherProbe {
            token: self.token.clone(),
            queue: Arc::downgrade(&self.queue),
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.queue.stop();
        let Some(thread) = self.thread.take() else {
            return;
        };
        match self.done.recv_timeout(DROP_JOIN_BOUND) {
            // The thread has dropped everything it held: the join returns at once.
            Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                let _ = thread.join();
            }
            // Stuck (a hung file-system call): abandoned; it ends when the call returns.
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => drop(thread),
        }
    }
}

impl Running {
    fn run(mut self) {
        loop {
            let retry = (!self.outbox.is_empty()).then(|| Instant::now() + self.window);
            let wake = match (self.rec.next_due(), retry) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            let Some(taken) = self.queue.take(wake) else {
                break;
            };
            let now = Instant::now();
            for (rel, stamp) in &taken.remembered {
                self.rec.remember(rel, stamp);
            }
            if taken.overflowed {
                self.rec
                    .push(RawHint::Rescan(self.root.canonical.clone()), now);
            }
            let mut lost = false;
            for hint in taken.hints {
                if hint == RawHint::Lost {
                    lost = true;
                } else {
                    self.rec.push(hint, now);
                }
            }
            if lost {
                self.fall_back(now, LostReason::Failed);
            }
            let events = self.rec.tick(now);
            let info = self.rec.last_tick();
            if let Some(source) = self.source.as_mut() {
                let synced = if info.scanned {
                    sync_dirs(source.as_mut(), &self.rec, &self.root.canonical)
                } else {
                    Ok(0)
                };
                match synced {
                    Err(reason) => self.fall_back(now, reason),
                    Ok(added) => {
                        if added > 0 {
                            // Fix round 2 (N2): files made in those folders before their
                            // watches were added; one more look, as `start` does.
                            self.rec
                                .push(RawHint::Path(self.root.canonical.clone()), now);
                        }
                        if info.saw_hints || !events.is_empty() {
                            source.pace(&self.root.canonical, !events.is_empty(), info.cost);
                        }
                    }
                }
            }
            if info.rearm && *lock(&self.backend) == Backend::Os {
                self.rearm(now);
            }
            self.outbox.extend(events);
            if !self.deliver() {
                break;
            }
        }
        if let Some(mut source) = self.source.take() {
            source.unwatch(&self.root.canonical);
        }
    }

    /// The source stopped working (or cannot watch every folder): poll instead, say so, and
    /// rescan what may have been missed.
    fn fall_back(&mut self, now: Instant, reason: LostReason) {
        if matches!(*lock(&self.backend), Backend::Poll(_) | Backend::Unwatched) {
            // Already the fallback: keep it (there is nothing further to fall back to) and
            // rescan what it may have missed.
            self.rec
                .push(RawHint::Rescan(self.root.canonical.clone()), now);
            return;
        }
        if let Some(mut old) = self.source.take() {
            old.unwatch(&self.root.canonical);
        }
        if let Ok(mut polled) = (self.fallback)()
            && polled
                .watch(&self.root.canonical, hint_sink(&self.queue))
                .is_ok()
        {
            self.source = Some(polled);
        }
        // Fix round 1 (m4): say so when nothing watches the library any more.
        let (backend, reason) = match (&self.source, reason) {
            (None, _) => (Backend::Unwatched, LostReason::NoFallback),
            (Some(_), LostReason::WatchBudget) => (
                Backend::Poll(PollReason::WatchBudget),
                LostReason::WatchBudget,
            ),
            (Some(_), r) => (Backend::Poll(PollReason::BackendLost), r),
        };
        *lock(&self.backend) = backend;
        self.outbox.push(WatchEvent::BackendLost(reason));
        self.rec
            .push(RawHint::Rescan(self.root.canonical.clone()), now);
    }

    /// Fix round 2 (m-fr1-5): sets the root's watch up again after a backstop look found an
    /// unannounced change; falls back to polling if that fails.
    fn rearm(&mut self, now: Instant) {
        let Some(source) = self.source.as_mut() else {
            return;
        };
        source.unwatch(&self.root.canonical);
        if source
            .watch(&self.root.canonical, hint_sink(&self.queue))
            .is_ok()
        {
            self.outbox.push(WatchEvent::BackendRearmed);
        } else {
            self.fall_back(now, LostReason::Failed);
        }
    }

    /// Hands the outbox to the receiver; `false` once nobody listens.
    fn deliver(&mut self) -> bool {
        if self.outbox.is_empty() {
            return true;
        }
        if self.outbox.len() > MAX_OUTBOX {
            let mut kept: Vec<WatchEvent> = self
                .outbox
                .drain(..)
                .filter(|e| matches!(e, WatchEvent::BackendLost(_)))
                .collect();
            kept.push(WatchEvent::Rescan);
            self.outbox = kept;
        }
        match self.sink.try_send(std::mem::take(&mut self.outbox)) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(batch)) => {
                self.outbox = batch;
                true
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }
}

/// A source that could not be built: every watch fails, so the watcher falls back.
struct Refusing(WatchError);

impl EventSource for Refusing {
    fn watch(&mut self, _root: &Path, _sink: HintSink) -> Result<(), WatchError> {
        Err(self.0.clone())
    }

    fn unwatch(&mut self, _root: &Path) {}
}
// ## m4b-t09-watcher

#[cfg(test)]
mod tests {
    use super::*;

    use notify::event::{Event, EventKind, Flag};
    use notify_debouncer_full::DebouncedEvent;

    #[test]
    fn follow_symlinks_is_off() {
        assert!(!watcher_config().follow_symlinks());
        assert_eq!(DEBOUNCE, Duration::from_millis(150));
    }

    /// m1 hardening (carries-w1 T9): the cache named in the source is the crate's own
    /// `NoCache`, never a local stand-in of the same name.
    #[test]
    fn debouncer_cache_is_nocache_or_never_leaves_root() {
        let name = std::any::type_name::<NoCache>();
        assert!(
            name.starts_with("notify_debouncer_full::") && name.ends_with("::NoCache"),
            "{name}"
        );
        let watcher = std::any::type_name::<RecommendedWatcher>();
        assert!(watcher.starts_with("notify::"), "{watcher}");
    }

    fn sinks_for(root: &Path) -> (Sinks, Arc<Mutex<Vec<Vec<RawHint>>>>) {
        let got: Arc<Mutex<Vec<Vec<RawHint>>>> = Arc::default();
        let seen = got.clone();
        let sinks = Sinks::default();
        lock(&sinks).push((root.to_path_buf(), Box::new(move |b| lock(&seen).push(b))));
        (sinks, got)
    }

    #[test]
    fn rescan_flag_becomes_a_rescan_hint() {
        // FSEvents `MustScanSubDirs` and inotify `IN_Q_OVERFLOW` arrive as `Flag::Rescan` with
        // no path (notify `fsevent.rs:116`, `inotify.rs:213`).
        let root = Path::new("/lib");
        let (sinks, got) = sinks_for(root);
        let ev = Event::new(EventKind::Other).set_flag(Flag::Rescan);
        route(&sinks, Ok(vec![DebouncedEvent::new(ev, Instant::now())]));
        assert_eq!(*lock(&got), vec![vec![RawHint::Rescan(root.into())]]);
    }

    #[test]
    fn watch_limit_error_is_lost_and_other_errors_rescan() {
        let root = Path::new("/lib");
        let (sinks, got) = sinks_for(root);
        let limit = notify::Error::new(notify::ErrorKind::MaxFilesWatch).add_path("/lib/d".into());
        let other = notify::Error::generic("boom");
        let pathed = notify::Error::generic("denied").add_path("/lib/x".into());
        route(&sinks, Err(vec![limit, other, pathed]));
        assert_eq!(
            *lock(&got),
            vec![vec![
                RawHint::Lost,
                RawHint::Rescan(root.into()),
                RawHint::Path("/lib/x".into())
            ]]
        );
    }

    #[test]
    /// Fix round 1 (m10): nested libraries sharing a source both see the inner subtree.
    fn paths_go_to_every_root_that_holds_them_and_outside_paths_nowhere() {
        let outer = Path::new("/lib");
        let (sinks, outer_got) = sinks_for(outer);
        let inner_got: Arc<Mutex<Vec<Vec<RawHint>>>> = Arc::default();
        let seen = inner_got.clone();
        lock(&sinks).push(("/lib/inner".into(), Box::new(move |b| lock(&seen).push(b))));
        let mut ev = Event::new(EventKind::Any);
        ev.paths = vec![
            "/lib/a.sql".into(),
            "/lib/inner/b.sql".into(),
            "/elsewhere/c.sql".into(),
        ];
        route(&sinks, Ok(vec![DebouncedEvent::new(ev, Instant::now())]));
        assert_eq!(
            *lock(&outer_got),
            vec![vec![
                RawHint::Path("/lib/a.sql".into()),
                RawHint::Path("/lib/inner/b.sql".into())
            ]]
        );
        assert_eq!(
            *lock(&inner_got),
            vec![vec![RawHint::Path("/lib/inner/b.sql".into())]]
        );
    }

    #[test]
    fn os_source_delivers_and_stops_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<Vec<RawHint>>();
        let tx = Mutex::new(tx);
        let mut src = OsEventSource::new().unwrap();
        src.watch(
            &root,
            Box::new(move |b| {
                let _ = lock(&tx).send(b);
            }),
        )
        .unwrap();
        // Condition wait (10 s ceiling): write until a batch for the root arrives.
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut got = None;
        let mut n = 0u32;
        while got.is_none() && Instant::now() < deadline {
            n += 1;
            std::fs::write(root.join("a.sql"), format!("SELECT {n}")).unwrap();
            got = rx.recv_timeout(Duration::from_secs(1)).ok();
        }
        let batch = got.expect("no batch within 10 s");
        assert!(
            batch
                .iter()
                .all(|h| matches!(h, RawHint::Path(p) if p.starts_with(&root)))
        );
        let sinks = Arc::downgrade(&src.sinks);
        drop(src);
        // The debouncer thread was joined, so its handler (and the sinks it held) is gone.
        assert!(sinks.upgrade().is_none());
    }
}
