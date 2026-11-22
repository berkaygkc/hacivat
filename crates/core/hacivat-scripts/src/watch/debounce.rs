//! Windows, bursts and bounded queues (HV-SCRIPT-08 steps 2, 3, 5 and 9; R-M4b-14).
//!
//! ## m4b-t09-watcher
//! Pure, clock-injected parts of the watcher:
//! - [`Debouncer`] counts the distinct paths of a burst (hints less than a window apart) and
//!   turns more than [`MAX_HINTS_PER_WINDOW`] into one rescan once the burst is quiet;
//! - [`SettleQueue`] holds the second look of every candidate one window later (the settle
//!   rule, alg/28 C6);
//! - [`HintQueue`] carries hint batches from the event source's thread to the watcher's, holds
//!   at most [`HINT_CAPACITY`] hints, and turns an overflow into one rescan.
//!
//! The OS and poll sources already deliver one batch per `notify-debouncer-full` window, so
//! the watcher looks at a hint as soon as it arrives; the window here is the one the burst
//! rule and the settle rule count in.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::fileio::DiskStamp;
use crate::pathsafe::RelPath;
use crate::watch::RawHint;

/// More hints than this in one window are one rescan (R-M4b-14).
pub const MAX_HINTS_PER_WINDOW: usize = 1_000;

/// Hints waiting for the watcher at most; an overflow is one rescan (HV-SCRIPT-08 step 9).
pub const HINT_CAPACITY: usize = 10_000;

/// What a [`Debouncer`] hands on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Flushed {
    /// The backend stopped working.
    Lost,
    /// Too many hints, or the backend said events were dropped: rescan the library.
    Rescan,
    /// Paths that may have changed, each once, in byte order.
    Paths(Vec<PathBuf>),
}

/// Counts the distinct paths of a burst and collapses a burst into one rescan.
///
/// ## m4b-t09-watcher fix round 1 (I7)
/// A burst is a run of hints each less than one window after the previous one (an extending
/// window, so a `git checkout` spread over several of `notify`'s batches is one burst), and
/// it counts **distinct paths** (a file rewritten in a loop is one path, never a burst). More
/// than [`MAX_HINTS_PER_WINDOW`] of them makes the burst overflow: its paths are dropped, and
/// one `Rescan` is handed on once the burst has been quiet for a window, so the rescan sees
/// the end state. A burst that never quiets still gets one every `max_wait`.
#[derive(Debug)]
pub struct Debouncer {
    window: Duration,
    max_hints: usize,
    max_wait: Duration,
    /// The first and the latest path hint of the current burst.
    began: Option<Instant>,
    last_at: Option<Instant>,
    /// The distinct paths of the current burst (until it overflows).
    burst: HashSet<PathBuf>,
    /// Since when the current burst has overflowed, its rescan not yet handed on.
    overflowed: Option<Instant>,
    paths: BTreeSet<PathBuf>,
    rescan: bool,
    lost: bool,
    pending_at: Option<Instant>,
    // ## m4b-cifix-2-repo-op (review I1, R-CI2-3)
    /// While a repository operation runs (`.git/index.lock` exists), the latest moment it may
    /// hold the burst ([`REPO_OP_MAX`] after it began).
    op_until: Option<Instant>,
    /// An operation outlived its bound and its lock has not been seen gone since.
    op_stale: bool,
    // ## m4b-cifix-2-repo-op
}

/// How long an overflowed burst that keeps going waits for its rescan at most.
const DEFAULT_MAX_WAIT: Duration = Duration::from_secs(2);

// ## m4b-cifix-2-repo-op (review I1, R-CI2-3)
/// The longest one repository operation holds the burst: a stale `index.lock` (a crashed git)
/// must not hold settling for ever.
pub const REPO_OP_MAX: Duration = Duration::from_secs(30);
// ## m4b-cifix-2-repo-op

impl Debouncer {
    /// A debouncer with the R-M4b-14 burst limit.
    pub fn new(window: Duration) -> Self {
        Self::with_max(window, MAX_HINTS_PER_WINDOW)
    }

    pub fn with_max(window: Duration, max_hints: usize) -> Self {
        Self::with_burst(window, max_hints, DEFAULT_MAX_WAIT)
    }

    pub fn with_burst(window: Duration, max_hints: usize, max_wait: Duration) -> Self {
        Debouncer {
            window,
            max_hints,
            max_wait,
            began: None,
            last_at: None,
            burst: HashSet::new(),
            overflowed: None,
            paths: BTreeSet::new(),
            rescan: false,
            lost: false,
            pending_at: None,
            op_until: None,
            op_stale: false,
        }
    }

    // ## m4b-cifix-2-repo-op (review I1, R-CI2-3)
    /// A repository operation began at `at` (git took `.git/index.lock`): until it ends, or at
    /// most [`REPO_OP_MAX`], the burst does not end on a quiet window. Its distinct paths are
    /// counted across the whole operation, and nothing but open files settles (the watcher's
    /// R-T9-6 exception), so a checkout that pauses for a window mid-way is still one burst. A
    /// second call while one runs changes nothing, and so does one after an operation outlived
    /// its bound, until [`end_operation`](Self::end_operation) says the lock went (a stale lock
    /// never holds twice).
    pub fn begin_operation(&mut self, at: Instant) {
        if self.op_until.is_some() || self.op_stale {
            return;
        }
        self.op_until = Some(at + REPO_OP_MAX);
        let quiet = self.last_at.is_none_or(|l| at >= l + self.window);
        if quiet && self.overflowed.is_none() {
            self.burst.clear();
            self.began = Some(at);
        }
        self.last_at = Some(self.last_at.map_or(at, |l| l.max(at)));
    }

    /// The repository operation ended at `at` (the lock went): an operation whose distinct
    /// paths overflowed is one `Rescan`, handed on once the hints are quiet for a window;
    /// otherwise the held paths are released to the ordinary hold.
    pub fn end_operation(&mut self, at: Instant) {
        self.op_stale = false;
        self.finish_operation(at);
    }

    fn finish_operation(&mut self, at: Instant) {
        if self.op_until.take().is_none() {
            return;
        }
        // An overflowed operation falls back to the ordinary rule from its end: its rescan is
        // handed on once the hints have been quiet for a window (at most `max_wait` later), so
        // the events FSEvents still delivers after the lock went are swallowed too, and the
        // one rescan sees the end state.
        if self.overflowed.is_some() {
            self.overflowed = Some(at);
            self.paths.clear();
            self.mark(at);
        }
    }

    /// Whether a repository operation holds the burst.
    pub fn in_operation(&self) -> bool {
        self.op_until.is_some()
    }
    // ## m4b-cifix-2-repo-op

    /// Takes one hint that arrived at `at` (a path counts toward a burst).
    pub fn push(&mut self, hint: RawHint, at: Instant) {
        match hint {
            RawHint::Lost => {
                self.lost = true;
                self.mark(at);
            }
            RawHint::Rescan(_) => {
                self.rescan = true;
                self.paths.clear();
                self.mark(at);
            }
            RawHint::Path(path) => self.push_path(path, at, true),
        }
    }

    /// Takes one path hint that arrived at `at`.
    ///
    /// ## m4b-t09-watcher fix round 2 (N1): only a `counted` hint (one that passed the
    /// watcher's name filter as a possible script or folder) counts toward a burst and extends
    /// it; an uncounted one (another extension: build output) is only passed on, unless a
    /// burst has overflowed.
    pub fn push_path(&mut self, path: PathBuf, at: Instant, counted: bool) {
        if !counted {
            if self.overflowed.is_none() {
                self.paths.insert(path);
                self.mark(at);
            }
            return;
        }
        // ## m4b-cifix-2-repo-op: a repository operation is one burst, pauses and all.
        if self.op_until.is_none() && self.last_at.is_none_or(|l| at >= l + self.window) {
            // A new burst; an overflowed one that was not flushed yet is due now.
            if self.overflowed.take().is_some() {
                self.rescan = true;
                self.paths.clear();
                self.mark(at);
            }
            self.burst.clear();
            self.began = Some(at);
        }
        self.last_at = Some(at);
        if self.overflowed.is_some() {
            return;
        }
        self.burst.insert(path.clone());
        self.paths.insert(path);
        self.mark(at);
        if self.burst.len() > self.max_hints {
            self.overflowed = Some(at);
            self.burst.clear();
            self.paths.clear();
        }
    }

    fn mark(&mut self, at: Instant) {
        self.pending_at.get_or_insert(at);
    }

    /// Since when something waits to be flushed (so a hint pushed by the watcher itself, not
    /// by a source, still wakes it).
    pub fn pending_at(&self) -> Option<Instant> {
        self.pending_at
    }

    /// Until when reports wait because hints are still arriving: one window after the latest
    /// hint, but never longer than `max_wait` after the burst began (fix round 1, I7: the
    /// files of a `git checkout` must not settle one by one before the burst is known to
    /// overflow). `None` before the first hint.
    ///
    /// ## m4b-t09-watcher fix round 3 (I-fr2-1): while a burst has overflowed, the hold lasts
    /// until its rescan is handed on ([`due`](Self::due)), with no fixed end: a slow
    /// checkout's first thousand files never settle one by one before the rescan. (Open files
    /// are the watcher's exception, R-T9-6.)
    pub fn hold_until(&self) -> Option<Instant> {
        let last = self.last_at?;
        let began = self.began.unwrap_or(last);
        let hold = (last + self.window).min(began + self.max_wait);
        // ## m4b-cifix-2-repo-op: a repository operation holds until it ends, at most its
        // bound.
        let hold = self.op_until.map_or(hold, |until| hold.max(until));
        Some(match self.due() {
            Some(due) => hold.max(due),
            None => hold,
        })
    }

    /// Whether reports wait at `now` ([`hold_until`](Self::hold_until)).
    pub fn holding(&self, now: Instant) -> bool {
        self.hold_until().is_some_and(|t| now < t)
    }

    /// When an overflowed burst's rescan is due.
    pub fn due(&self) -> Option<Instant> {
        let since = self.overflowed?;
        // ## m4b-cifix-2-repo-op: an operation's rescan waits for its end, at most its bound.
        if let Some(until) = self.op_until {
            return Some(until);
        }
        let quiet = self.last_at.map_or(since, |l| l + self.window);
        Some(quiet.min(since + self.max_wait))
    }

    /// Everything pending at `now`: `Lost` first, then either `Rescan` or the paths.
    pub fn flush(&mut self, now: Instant) -> Vec<Flushed> {
        // ## m4b-cifix-2-repo-op: an operation past its bound (a stale lock) ends here.
        if self.op_until.is_some_and(|until| now >= until) {
            self.finish_operation(now);
            self.op_stale = true;
        }
        let mut out = Vec::new();
        if std::mem::take(&mut self.lost) {
            out.push(Flushed::Lost);
        }
        if let Some(due) = self.due()
            && now >= due
        {
            self.rescan = true;
            // Fix round 3 (I-fr2-1, reverting fr2's reset): a burst still going stays
            // overflowed (swallowing, one rescan per `max_wait`); the count starts afresh only
            // once the burst has been quiet for a window (a new burst, in `push_path`). Noise
            // never counts (the watcher's name filter), so this cannot starve a real edit.
            let going = self.last_at.is_some_and(|l| now < l + self.window);
            self.overflowed = going.then_some(now);
            self.burst.clear();
        }
        if std::mem::take(&mut self.rescan) {
            self.paths.clear();
            out.push(Flushed::Rescan);
        } else if !self.paths.is_empty() {
            out.push(Flushed::Paths(
                std::mem::take(&mut self.paths).into_iter().collect(),
            ));
        }
        self.pending_at = None;
        out
    }

    pub fn window(&self) -> Duration {
        self.window
    }
}

/// When each candidate is looked at again.
#[derive(Debug)]
pub struct SettleQueue<K: Ord + Clone> {
    due: BTreeMap<K, Instant>,
}

impl<K: Ord + Clone> Default for SettleQueue<K> {
    fn default() -> Self {
        SettleQueue {
            due: BTreeMap::new(),
        }
    }
}

impl<K: Ord + Clone> SettleQueue<K> {
    /// Looks at `key` again at `at` (replacing an earlier time).
    pub fn schedule(&mut self, key: K, at: Instant) {
        self.due.insert(key, at);
    }

    pub fn remove(&mut self, key: &K) {
        self.due.remove(key);
    }

    /// The keys due by `now`, in key order, removed from the queue.
    pub fn take_due(&mut self, now: Instant) -> Vec<K> {
        let due: Vec<K> = self
            .due
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(k, _)| k.clone())
            .collect();
        for k in &due {
            self.due.remove(k);
        }
        due
    }

    /// The earliest time something is due.
    pub fn next(&self) -> Option<Instant> {
        self.due.values().min().copied()
    }

    /// Every key and when it is due.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &Instant)> {
        self.due.iter()
    }

    pub fn len(&self) -> usize {
        self.due.len()
    }

    pub fn is_empty(&self) -> bool {
        self.due.is_empty()
    }

    pub fn clear(&mut self) {
        self.due.clear();
    }
}

/// What the watcher's thread takes from a [`HintQueue`] at once.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Taken {
    /// In arrival order; a `Lost` is never dropped by an overflow.
    pub hints: Vec<RawHint>,
    /// Hints were dropped because the queue was full: rescan.
    pub overflowed: bool,
    /// Content stamps the service learned (a tab read or saved the file), for rename pairing.
    pub remembered: Vec<(RelPath, DiskStamp)>,
}

#[derive(Debug, Default)]
struct QState {
    hints: Vec<RawHint>,
    overflowed: bool,
    lost: bool,
    stopped: bool,
    remembered: Vec<(RelPath, DiskStamp)>,
}

/// The bounded hand-over between an event source's thread and the watcher's thread.
#[derive(Debug)]
pub struct HintQueue {
    state: Mutex<QState>,
    ready: Condvar,
    capacity: usize,
}

impl Default for HintQueue {
    fn default() -> Self {
        Self::new(HINT_CAPACITY)
    }
}

/// Remembered stamps kept at most between two looks (only tabs produce them).
const MAX_REMEMBERED: usize = 1_024;

impl HintQueue {
    pub fn new(capacity: usize) -> Self {
        HintQueue {
            state: Mutex::new(QState::default()),
            ready: Condvar::new(),
            capacity,
        }
    }

    /// Adds a batch; never blocks the source for longer than the lock.
    pub fn push(&self, batch: Vec<RawHint>) {
        if batch.is_empty() {
            return;
        }
        let mut st = self.lock();
        if st.stopped {
            return;
        }
        let lost = batch.contains(&RawHint::Lost);
        st.lost |= lost;
        if !st.overflowed {
            let paths = batch.into_iter().filter(|h| *h != RawHint::Lost);
            let room = self.capacity.saturating_sub(st.hints.len());
            let incoming: Vec<RawHint> = paths.collect();
            if incoming.len() > room {
                st.overflowed = true;
                st.hints.clear();
            } else {
                st.hints.extend(incoming);
            }
        }
        drop(st);
        self.ready.notify_one();
    }

    /// Hands a content stamp to the watcher (bounded; the oldest goes first).
    pub fn remember(&self, rel: RelPath, stamp: DiskStamp) {
        let mut st = self.lock();
        if st.remembered.len() >= MAX_REMEMBERED {
            st.remembered.remove(0);
        }
        st.remembered.push((rel, stamp));
        drop(st);
        self.ready.notify_one();
    }

    /// Waits until something is queued, `until` passes (then answers an empty [`Taken`]), or
    /// the queue is stopped (`None`).
    pub fn take(&self, until: Option<Instant>) -> Option<Taken> {
        let mut st = self.lock();
        loop {
            if st.stopped {
                return None;
            }
            if !st.hints.is_empty() || st.overflowed || st.lost || !st.remembered.is_empty() {
                let mut hints = std::mem::take(&mut st.hints);
                if std::mem::take(&mut st.lost) {
                    hints.push(RawHint::Lost);
                }
                return Some(Taken {
                    hints,
                    overflowed: std::mem::take(&mut st.overflowed),
                    remembered: std::mem::take(&mut st.remembered),
                });
            }
            match until {
                None => {
                    st = self.ready.wait(st).unwrap_or_else(PoisonError::into_inner);
                }
                Some(t) => {
                    let now = Instant::now();
                    if now >= t {
                        return Some(Taken::default());
                    }
                    st = self
                        .ready
                        .wait_timeout(st, t - now)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                }
            }
        }
    }

    /// Wakes the watcher's thread for good; later pushes are dropped.
    pub fn stop(&self) {
        self.lock().stopped = true;
        self.ready.notify_all();
    }

    /// Hints queued now (for tests).
    pub fn queued(&self) -> usize {
        self.lock().hints.len()
    }

    fn lock(&self) -> MutexGuard<'_, QState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
// ## m4b-t09-watcher
