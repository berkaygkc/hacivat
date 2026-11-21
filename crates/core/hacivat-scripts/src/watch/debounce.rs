//! Windows, bursts and bounded queues (HV-SCRIPT-08 steps 2, 3, 5 and 9; R-M4b-14).
//!
//! ## m4b-t09-watcher
//! Pure, clock-injected parts of the watcher:
//! - [`Debouncer`] counts the hints of one window (150 ms from the first hint after a quiet
//!   window) and turns more than [`MAX_HINTS_PER_WINDOW`] into one rescan;
//! - [`SettleQueue`] holds the second look of every candidate one window later (the settle
//!   rule, alg/28 C6);
//! - [`HintQueue`] carries hint batches from the event source's thread to the watcher's, holds
//!   at most [`HINT_CAPACITY`] hints, and turns an overflow into one rescan.
//!
//! The OS and poll sources already deliver one batch per `notify-debouncer-full` window, so
//! the watcher looks at a hint as soon as it arrives; the window here is the one the burst
//! rule and the settle rule count in.

use std::collections::{BTreeMap, BTreeSet};
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

/// Counts hints per window and collapses a burst into one rescan.
#[derive(Debug)]
pub struct Debouncer {
    window: Duration,
    max_hints: usize,
    /// The start of the window being counted, and its count.
    started: Option<Instant>,
    count: usize,
    paths: BTreeSet<PathBuf>,
    rescan: bool,
    lost: bool,
}

impl Debouncer {
    /// A debouncer with the R-M4b-14 burst limit.
    pub fn new(window: Duration) -> Self {
        Self::with_max(window, MAX_HINTS_PER_WINDOW)
    }

    pub fn with_max(window: Duration, max_hints: usize) -> Self {
        Debouncer {
            window,
            max_hints,
            started: None,
            count: 0,
            paths: BTreeSet::new(),
            rescan: false,
            lost: false,
        }
    }

    /// Takes one hint that arrived at `at`.
    pub fn push(&mut self, hint: RawHint, at: Instant) {
        match hint {
            RawHint::Lost => self.lost = true,
            RawHint::Rescan(_) => {
                self.rescan = true;
                self.paths.clear();
            }
            RawHint::Path(path) => {
                if self.started.is_none_or(|s| at >= s + self.window) {
                    self.started = Some(at);
                    self.count = 0;
                }
                self.count += 1;
                if self.count == self.max_hints + 1 {
                    // Once per window: the rescan covers everything seen so far.
                    self.rescan = true;
                    self.paths.clear();
                } else if !self.rescan {
                    // After the rescan was handed on, later hints of the same window are
                    // changes the rescan may not have seen: looked at one by one.
                    self.paths.insert(path);
                }
            }
        }
    }

    /// Everything pending at `now`: `Lost` first, then either `Rescan` or the paths.
    pub fn flush(&mut self, now: Instant) -> Vec<Flushed> {
        let mut out = Vec::new();
        if std::mem::take(&mut self.lost) {
            out.push(Flushed::Lost);
        }
        if std::mem::take(&mut self.rescan) {
            self.paths.clear();
            out.push(Flushed::Rescan);
        } else if !self.paths.is_empty() {
            out.push(Flushed::Paths(
                std::mem::take(&mut self.paths).into_iter().collect(),
            ));
        }
        if self.started.is_some_and(|s| now >= s + self.window) {
            self.started = None;
            self.count = 0;
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
