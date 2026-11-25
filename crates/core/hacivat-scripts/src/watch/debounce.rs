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
//! The OS and poll sources already deliver one batch per debounce window (gatefix-1: the OS
//! source's own debounce thread, `watch::FLUSH_GAP`), so
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
    // ## m4b-gatefix-1-tail
    /// When the last repository operation ended, while its burst is still taking the events
    /// the backend delivers late (see [`OP_QUIET`]).
    after_op: Option<Instant>,
    // ## m4b-gatefix-1-tail
    // ## m4b-gatefix-1 fix round 1 (review C1, I1; ruling R-GF-1)
    /// When the last rescan was handed on: the next one is never due before a window later,
    /// so [`due`](Self::due) can never answer the same past instant twice.
    last_rescan: Option<Instant>,
    /// Paths of files a tab has open: handed on at the next flush whatever a burst does
    /// (R-GF-1), never swallowed by an overflow nor held by a repository operation.
    open: BTreeSet<PathBuf>,
    // ## m4b-gatefix-1 fix round 1
}

// ## m4b-gatefix-1-tail (CI 45202154953 macOS `git_checkout_2000_is_one_rescan`)
/// After a repository operation ends, its burst goes on until the hints have been quiet this
/// long (instead of one window), at most [`REPO_OP_MAX`] after the end. FSEvents hands a
/// checkout's per-file events over in chunks for a second or more after git let go of the lock
/// (a trace: 700 of 2,000 files delivered by then, the rest in chunks up to 220 ms apart for
/// 1.9 s). With a one-window gap the operation's burst ended at the first pause, so neither
/// part reached [`MAX_HINTS_PER_WINDOW`] and each file was reported on its own.
pub const OP_QUIET: Duration = Duration::from_secs(1);
// ## m4b-gatefix-1-tail

// ## m4b-gatefix-1 fix round 1 (review C1)
/// The longest the post-operation phase ([`OP_QUIET`]) lasts after the lock went (the traced
/// late chunks lasted 1.9 s). After it the ordinary one-window rule is back, and an
/// overflowed burst's rescan is due at most `max_wait` after the overflow in either phase.
pub const OP_LATE_MAX: Duration = Duration::from_secs(4);
// ## m4b-gatefix-1 fix round 1

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
            // ## m4b-gatefix-1-tail
            after_op: None,
            // ## m4b-gatefix-1 fix round 1
            last_rescan: None,
            open: BTreeSet::new(),
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
        // ## m4b-gatefix-1-tail: the operation's burst takes what is still delivered.
        self.after_op = Some(at);
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

    // ## m4b-gatefix-1-tail
    /// The pause that ends the current burst at `at`: [`OP_QUIET`] in the post-operation
    /// phase (at most [`OP_LATE_MAX`] after the lock went), one window otherwise.
    fn gap(&self, at: Instant) -> Duration {
        if self.in_late_phase(at) {
            OP_QUIET
        } else {
            self.window
        }
    }

    // ## m4b-gatefix-1 fix round 1 (review C1)
    /// Whether `at` is in the post-operation phase.
    fn in_late_phase(&self, at: Instant) -> bool {
        self.after_op.is_some_and(|end| at < end + OP_LATE_MAX)
    }

    /// Takes the hint of a file a tab has open (R-GF-1): it counts toward the burst like any
    /// other, and is handed on at the next flush even while the burst is overflowed or a
    /// repository operation holds it, so an open file's edit is reported within the normal
    /// debounce (E2).
    pub fn push_open(&mut self, path: PathBuf, at: Instant) {
        self.push_path(path.clone(), at, true);
        self.open.insert(path);
        self.mark(at);
    }
    // ## m4b-gatefix-1 fix round 1

    /// Whether the burst of the operation that ended is still taking late events.
    pub fn after_operation(&self) -> bool {
        self.after_op.is_some()
    }
    // ## m4b-gatefix-1-tail
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
        // ## m4b-gatefix-1-tail: right after an operation the pause that ends it is longer.
        let gap = self.gap(at);
        if self.op_until.is_none() && self.last_at.is_none_or(|l| at >= l + gap) {
            self.after_op = None;
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
        // ## m4b-gatefix-1-tail: after an operation, until its late events have stopped.
        // ## m4b-gatefix-1 fix round 1 (C1): at most OP_LATE_MAX after the lock went.
        let hold = self.after_op.map_or(hold, |end| {
            (last + OP_QUIET).min(end + OP_LATE_MAX).max(hold)
        });
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
        // ## m4b-gatefix-1-tail: an overflowed operation's rescan waits for its late events,
        // ## m4b-gatefix-1 fix round 1 (C1): but never past `max_wait` after the overflow,
        // whatever the traffic, and never before a window after the last rescan.
        let gap = match self.after_op {
            Some(end) if since < end + OP_LATE_MAX => OP_QUIET,
            _ => self.window,
        };
        let quiet = self.last_at.map_or(since, |l| l + gap);
        let due = quiet.min(since + self.max_wait);
        Some(self.last_rescan.map_or(due, |r| due.max(r + self.window)))
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
            // ## m4b-gatefix-1 fix round 1 (C1): the rescan of an operation's burst ends the
            // post-operation phase and its overflow; what still comes is a new count (and
            // late copies of what the rescan saw fold into it, the watcher's `RescanTail`).
            let going =
                self.after_op.is_none() && self.last_at.is_some_and(|l| now < l + self.window);
            self.overflowed = going.then_some(now);
            self.after_op = None;
            self.burst.clear();
            debug_assert!(
                self.last_rescan.is_none_or(|r| now > r),
                "a rescan due twice at one instant"
            );
            self.last_rescan = Some(now);
        }
        // ## m4b-gatefix-1 fix round 1 (R-GF-1): open files first, whatever the burst does.
        let open = std::mem::take(&mut self.open);
        if std::mem::take(&mut self.rescan) {
            self.paths.clear();
            if !open.is_empty() {
                out.push(Flushed::Paths(open.into_iter().collect()));
            }
            out.push(Flushed::Rescan);
        } else {
            let mut paths = std::mem::take(&mut self.paths);
            paths.extend(open);
            if !paths.is_empty() {
                out.push(Flushed::Paths(paths.into_iter().collect()));
            }
        }
        self.pending_at = None;
        // ## m4b-gatefix-1-tail: the operation's burst is over once it has been quiet (or at
        // its bound, fix round 1: OP_LATE_MAX).
        if let Some(end) = self.after_op
            && self.overflowed.is_none()
            && (self.last_at.is_none_or(|l| now >= l + OP_QUIET) || now >= end + OP_LATE_MAX)
        {
            self.after_op = None;
        }
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
    // ## m4b-t20-adversarial (named exception, N3)
    /// The latest `index.lock` of the repository above the library the service handed on,
    /// when it handed one since the last take (`Some(None)`: there is none any more).
    pub outer_lock: Option<Option<std::path::PathBuf>>,
}

#[derive(Debug, Default)]
struct QState {
    hints: Vec<RawHint>,
    overflowed: bool,
    lost: bool,
    stopped: bool,
    remembered: Vec<(RelPath, DiskStamp)>,
    // ## m4b-t20-adversarial (named line, N3)
    outer_lock: Option<Option<std::path::PathBuf>>,
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

    // ## m4b-t20-adversarial (named exception, N3)
    /// Hands the watcher the `index.lock` of the repository above the library (the latest
    /// one wins).
    pub fn set_outer_lock(&self, lock: Option<std::path::PathBuf>) {
        let mut st = self.lock();
        st.outer_lock = Some(lock);
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
            if !st.hints.is_empty()
                || st.overflowed
                || st.lost
                || !st.remembered.is_empty()
                || st.outer_lock.is_some()
            {
                let mut hints = std::mem::take(&mut st.hints);
                if std::mem::take(&mut st.lost) {
                    hints.push(RawHint::Lost);
                }
                return Some(Taken {
                    hints,
                    overflowed: std::mem::take(&mut st.overflowed),
                    remembered: std::mem::take(&mut st.remembered),
                    // ## m4b-t20-adversarial (named line, N3)
                    outer_lock: st.outer_lock.take(),
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
