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
//! The watcher's own loop runs on a named thread (`hv-library-watch`) that dropping the
//! [`Watcher`] stops and joins; the source is unwatched and dropped on that thread, and the
//! `notify` debouncer thread is joined in the source's `Drop`.

pub mod debounce;
pub mod echo;
pub mod limits;
pub mod poll;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

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
}

impl OsEventSource {
    /// Builds the debouncer the only allowed way (R-T0-2). It watches nothing yet.
    pub fn new() -> Result<Self, WatchError> {
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
        if let Err(e) = debouncer.watch(root, RecursiveMode::Recursive) {
            lock(&self.sinks).retain(|(r, _)| r != root);
            return Err(WatchError::Backend(e.to_string()));
        }
        Ok(())
    }

    fn unwatch(&mut self, root: &Path) {
        if let Some(debouncer) = self.debouncer.as_mut() {
            let _ = debouncer.unwatch(root);
        }
        lock(&self.sinks).retain(|(r, _)| r != root);
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

/// Splits one debounced batch into hints per watched root (the deepest root that holds the
/// path) and hands each root its hints.
fn route(sinks: &Sinks, result: DebounceEventResult) {
    let mut sinks = lock(sinks);
    let roots: Vec<PathBuf> = sinks.iter().map(|(r, _)| r.clone()).collect();
    let owner = |p: &Path| {
        roots
            .iter()
            .enumerate()
            .filter(|(_, r)| p.starts_with(r))
            .max_by_key(|(_, r)| r.as_os_str().len())
            .map(|(i, _)| i)
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
                        if let Some(i) = owner(p) {
                            per[i].push(RawHint::Rescan(roots[i].clone()));
                        }
                    }
                    continue;
                }
                for p in &ev.paths {
                    if let Some(i) = owner(p) {
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
                    if let Some(i) = owner(p) {
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
    /// `notify`'s poll watcher every [`POLL_INTERVAL`].
    Poll(PollReason),
}

/// Why the OS backend was given up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostReason {
    /// The source refused to watch the root.
    StartFailed,
    /// The source reported that it stopped working (the watch limit was reached).
    Failed,
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
    /// Anything may have changed: rescan the library (a burst, a dropped-events signal, a
    /// root that moved or is gone, a scan that hit a limit).
    Rescan,
    /// The repository's index, `HEAD` or refs changed (refresh the status, HV-SCRIPT-11).
    RepositoryChanged,
    /// The OS backend stopped; the library is polled from now on (a diagnostic).
    BackendLost(LostReason),
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
}

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

enum HintKind {
    Drop,
    Repository,
    File(RelPath),
    Scan,
}

/// The names under `.git/` whose change means the status may have changed.
const REPOSITORY_NAMES: [&str; 4] = ["index", "HEAD", "packed-refs", "refs"];

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
        };
        let s = r.scan();
        if s.root.is_none() {
            r.known = files_of(&s)
                .map(|(rel, seen)| (rel, Known { seen, hash: None }))
                .collect();
        }
        r.reindex();
        r
    }

    /// The echo table the save path records into.
    pub fn echo(&self) -> EchoHandle {
        self.echo.clone()
    }

    /// Takes a hint that arrived at `at` (looked at by the next [`tick`](Self::tick)).
    pub fn push(&mut self, hint: RawHint, at: Instant) {
        self.debouncer.push(hint, at);
    }

    /// The content stamp of `rel` as a tab read or saved it: kept while the file still has
    /// that length and time, so a later move of it can be paired by hash.
    pub fn remember(&mut self, rel: &RelPath, stamp: &DiskStamp) {
        if let Some(k) = self.known.get_mut(rel)
            && k.seen.len == stamp.len
            && k.seen.mtime == stamp.mtime
        {
            k.hash = Some(stamp.hash);
        }
    }

    /// When [`tick`](Self::tick) has work next.
    pub fn next_due(&self) -> Option<Instant> {
        match (self.settle.next(), self.scan_due) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// The files known now (for tests and diagnostics).
    pub fn known_files(&self) -> usize {
        self.known.len()
    }

    /// Looks at what arrived and at what is due at `now`; answers the settled events.
    pub fn tick(&mut self, now: Instant) -> Vec<WatchEvent> {
        let mut repository = false;
        let mut need_scan = false;
        for flushed in self.debouncer.flush(now) {
            match flushed {
                // The watcher switches the source; nothing to reconcile.
                Flushed::Lost => {}
                Flushed::Rescan => return self.rescan(),
                Flushed::Paths(paths) => {
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

        let mut settled: Vec<(RelPath, Option<Seen>, bool)> = Vec::new();
        for rel in self.settle.take_due(now) {
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

        let scan_check = self.scan_due.is_some_and(|t| t <= now);
        if need_scan || scan_check {
            let Some(diff) = self.scan_diff() else {
                return self.rescan();
            };
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
        self.candidates.clear();
        self.settle.clear();
        self.scan_due = None;
        let s = self.scan();
        if s.root.is_none() {
            let old = std::mem::take(&mut self.known);
            self.known = files_of(&s)
                .map(|(rel, seen)| {
                    let hash = old
                        .get(&rel)
                        .filter(|k| k.seen.same_content(&seen))
                        .and_then(|k| k.hash);
                    (rel, Known { seen, hash })
                })
                .collect();
            self.reindex();
        }
        vec![WatchEvent::Rescan]
    }

    fn scan(&self) -> ScanResult {
        scan::scan(&self.root, &self.opts.limits, &*self.opts.fs)
    }

    /// What differs between the known files and one scan now; `None` when the scan cannot be
    /// compared (the root is gone or moved, or a limit stopped it).
    fn scan_diff(&self) -> Option<BTreeMap<RelPath, Option<Seen>>> {
        let s = self.scan();
        if s.root.is_some() || s.stopped.is_some() {
            return None;
        }
        let now: HashMap<RelPath, Seen> = files_of(&s).collect();
        let mut diff = BTreeMap::new();
        for (rel, k) in &self.known {
            match now.get(rel) {
                None => {
                    diff.insert(rel.clone(), None);
                }
                Some(seen) if *seen != k.seen => {
                    diff.insert(rel.clone(), Some(seen.clone()));
                }
                Some(_) => {}
            }
        }
        for (rel, seen) in now {
            if !self.known.contains_key(&rel) {
                diff.insert(rel, Some(seen));
            }
        }
        Some(diff)
    }

    /// The first look at a known file after a hint.
    fn first_look(&mut self, rel: RelPath, obs: Option<Seen>, now: Instant) {
        let unchanged = match (&obs, self.known.get(&rel)) {
            (Some(s), Some(k)) => *s == k.seen,
            (None, None) => true,
            _ => false,
        };
        let pending = self.candidates.get(&rel);
        if unchanged && pending.is_none() {
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

    /// Settled looks → events: renames paired by hash, echoes dropped, the known files kept in
    /// step.
    fn report(
        &mut self,
        settled: Vec<(RelPath, Option<Seen>, bool)>,
        now: Instant,
    ) -> Vec<WatchEvent> {
        let mut removed: Vec<RelPath> = Vec::new();
        let mut appeared: Vec<(RelPath, Seen, bool, bool)> = Vec::new(); // (rel, seen, new, forced)
        for (rel, obs, forced) in settled {
            match (self.known.get(&rel), obs) {
                (None, None) => {}
                (Some(_), None) => removed.push(rel),
                (None, Some(s)) => appeared.push((rel, s, true, forced)),
                (Some(k), Some(s)) => {
                    if k.seen.same_content(&s) {
                        let hash = k.hash;
                        self.known.insert(rel, Known { seen: s, hash });
                    } else {
                        appeared.push((rel, s, false, forced));
                    }
                }
            }
        }
        if removed.is_empty() && appeared.is_empty() {
            return Vec::new();
        }
        removed.sort();
        appeared.sort_by(|a, b| a.0.cmp(&b.0));

        let mut hashes: HashMap<RelPath, Option<DiskStamp>> = HashMap::new();
        let mut events = Vec::new();

        // Renames: a removed file whose hash is known, and a new one of the same length and
        // hash.
        let mut paired: HashSet<RelPath> = HashSet::new();
        let mut renamed_from: HashSet<RelPath> = HashSet::new();
        for a in &removed {
            let Some(Known {
                seen: old,
                hash: Some(h),
            }) = self.known.get(a).cloned()
            else {
                continue;
            };
            for (b, s, new, _) in &appeared {
                if !*new || paired.contains(b) || !s.hashable() || s.len != old.len {
                    continue;
                }
                let stamp = self.stamp_of(b, s, &mut hashes);
                if stamp.is_some_and(|st| st.hash == h) {
                    paired.insert(b.clone());
                    renamed_from.insert(a.clone());
                    events.push(WatchEvent::Renamed {
                        from: a.clone(),
                        to: b.clone(),
                    });
                    break;
                }
            }
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
            self.known.insert(rel, Known { seen, hash });
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

    /// What a hint's path is to this library.
    fn classify(&self, path: &Path) -> HintKind {
        let rest = match path.strip_prefix(&self.root.canonical) {
            Ok(r) => r,
            Err(_) => match path.strip_prefix(&self.root.path) {
                Ok(r) => r,
                Err(_) => return HintKind::Drop,
            },
        };
        let mut parts: Vec<&str> = Vec::new();
        for c in rest.components() {
            let Component::Normal(name) = c else {
                return HintKind::Drop;
            };
            let Some(name) = name.to_str() else {
                return HintKind::Drop;
            };
            parts.push(name);
        }
        let Some(first) = parts.first() else {
            return HintKind::Scan;
        };
        if first.eq_ignore_ascii_case(".git") {
            let status = parts.get(1).is_some_and(|n| REPOSITORY_NAMES.contains(n));
            return if status {
                HintKind::Repository
            } else {
                HintKind::Drop
            };
        }
        if parts.iter().any(|p| pathsafe::is_skipped(p)) || parts.len() > self.opts.limits.max_depth
        {
            return HintKind::Drop;
        }
        let Ok(rel) = RelPath::parse(&parts.join("/")) else {
            return HintKind::Drop;
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

/// Watches one library: a source, the reconciler on its own thread, and the events delivered
/// to `sink`. Dropping it stops everything it started.
pub struct Watcher {
    queue: Arc<HintQueue>,
    thread: Option<JoinHandle<()>>,
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
    _token: Arc<()>,
}

fn hint_sink(queue: &Arc<HintQueue>) -> HintSink {
    let queue = queue.clone();
    Box::new(move |batch| queue.push(batch))
}

impl Watcher {
    /// Picks the backend for `root` (R-M4b-14: [`backend_for`]) and starts watching it.
    pub fn open(
        root: LibraryRoot,
        mut opts: WatchOpts,
        sink: mpsc::Sender<WatchBatch>,
    ) -> Result<Watcher, WatchError> {
        let backend = backend_for(&root.canonical);
        opts.backend = backend;
        let source: Result<Box<dyn EventSource>, WatchError> = match backend {
            Backend::Os => OsEventSource::new().map(|s| Box::new(s) as Box<dyn EventSource>),
            Backend::Poll(_) => PollEventSource::new().map(|s| Box::new(s) as Box<dyn EventSource>),
        };
        let source = source.unwrap_or_else(|e| Box::new(Refusing(e)));
        Watcher::start(root, opts, sink, source)
    }

    /// Starts watching `root` with `source`, falling back to polling if it fails.
    ///
    /// Blocks while the source registers the tree (inotify adds one watch per folder; the poll
    /// source walks it once) and while the library is scanned once: call it off the UI thread.
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
        let rec = Reconciler::new(root.clone(), opts);
        let echo = rec.echo();
        let backend = Arc::new(Mutex::new(backend));
        let token = Arc::new(());
        let probe = Arc::downgrade(&token);
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
            _token: token,
        };
        let thread = std::thread::Builder::new()
            .name("hv-library-watch".into())
            .spawn(move || running.run())
            .map_err(|e| WatchError::Backend(e.to_string()))?;
        Ok(Watcher {
            queue,
            thread: Some(thread),
            echo,
            backend,
            token: probe,
        })
    }

    /// The table `fileio::save_with_echo`'s callback records into.
    pub fn echo(&self) -> EchoHandle {
        self.echo.clone()
    }

    /// The content stamp a tab read or saved, for rename pairing.
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
        if let Some(t) = self.thread.take() {
            let _ = t.join();
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
                self.fall_back(now);
            }
            let events = self.rec.tick(now);
            self.outbox.extend(events);
            if !self.deliver() {
                break;
            }
        }
        if let Some(mut source) = self.source.take() {
            source.unwatch(&self.root.canonical);
        }
    }

    /// The source stopped working: poll instead, say so, and rescan what may have been missed.
    fn fall_back(&mut self, now: Instant) {
        if matches!(*lock(&self.backend), Backend::Poll(PollReason::BackendLost)) {
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
        *lock(&self.backend) = Backend::Poll(PollReason::BackendLost);
        self.outbox
            .push(WatchEvent::BackendLost(LostReason::Failed));
        self.rec
            .push(RawHint::Rescan(self.root.canonical.clone()), now);
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
    fn paths_go_to_the_deepest_root_and_outside_paths_nowhere() {
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
            vec![vec![RawHint::Path("/lib/a.sql".into())]]
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
