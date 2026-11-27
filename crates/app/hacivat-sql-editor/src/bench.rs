// ## m3c-t44: the worker's timer for the real-app B3 bench (R-M3-9; feature `bench` only).
//! The worker's own timer (M3c Task 44, R-M3-9): while armed, every analysis the per-document
//! worker runs records how long it took to bring its kept document up to the job's text, in two
//! parts — the scan for the one edit between the two texts (`change_between`, which grows with
//! the document) and `SqlDocument::apply` (the incremental relex and resplit) — and the whole
//! analysis (with the execution split, highlighting, statements and folds).
//!
//! The figures are taken where the real app takes them, on the GPUI background executor, so
//! they include the rope and the scheduling the app's worker really has. The real-app B3 bench
//! (`hacivat_app::bench::b3`) arms the timer for its measured keystrokes and grades the sum of
//! the two parts against R-M3-9's 2 ms p95, printing both. Process-wide: the bench runs one
//! typing console at a time. The module exists only with the `bench` feature; no product build
//! records anything.

use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// At most this many samples are kept (a 2,000-keystroke run records at most 2,000).
pub const MAX_SAMPLES: usize = 100_000;

/// How the worker brought its document to a job's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Built {
    /// The kept document took the one edit: the incremental relex and resplit (R-M3-9's case).
    Incremental,
    /// The kept document already had the job's text (nothing to relex).
    Unchanged,
    /// A fresh document was built (the first job, a new SQL context, an edit it could not
    /// apply): a full lex, not a relex figure.
    Fresh,
}

/// One analysis the worker ran while armed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerSample {
    pub kind: Built,
    /// The scan for the one edit between the kept text and the job's rope (`change_between`;
    /// zero when no document was kept).
    pub diff: Duration,
    /// `SqlDocument::apply` (the relex and the resplit) — or the fresh build for
    /// [`Built::Fresh`].
    pub apply: Duration,
    /// The whole analysis: `diff` and `apply`, then the execution split, highlighting,
    /// statements, folds.
    pub analysis: Duration,
}

impl WorkerSample {
    /// What R-M3-9 grades: the scan and the apply together, the worker's whole relex step.
    pub fn relex(&self) -> Duration {
        self.diff + self.apply
    }
}

static ARMED: AtomicBool = AtomicBool::new(false);
static SAMPLES: Mutex<Vec<WorkerSample>> = Mutex::new(Vec::new());

thread_local! {
    /// The current analysis's scan time on this worker thread (one analysis at a time per
    /// thread: `analyse_from` runs synchronously).
    static DIFF: Cell<Duration> = const { Cell::new(Duration::ZERO) };
}

/// Starts (`true`) or stops recording. Arming clears what an earlier run left.
pub fn arm(on: bool) {
    if on {
        take();
    }
    ARMED.store(on, Ordering::SeqCst);
}

/// Whether the worker records now.
pub fn armed() -> bool {
    ARMED.load(Ordering::Relaxed)
}

/// Arms the timer until the guard drops (also on a panic, so an armed timer never outlives a
/// failed test or run).
#[must_use = "the timer is disarmed when the guard drops"]
pub fn armed_until_dropped() -> Disarm {
    arm(true);
    Disarm(())
}

/// Disarms the timer when dropped ([`armed_until_dropped`]).
pub struct Disarm(());

impl Drop for Disarm {
    fn drop(&mut self) {
        arm(false);
    }
}

/// The samples recorded since the last `take` (or `arm(true)`); the store is left empty.
pub fn take() -> Vec<WorkerSample> {
    std::mem::take(&mut *SAMPLES.lock().unwrap_or_else(|e| e.into_inner()))
}

/// The worker notes the current analysis's scan time (`Duration::ZERO` resets it).
pub(crate) fn note_diff(d: Duration) {
    DIFF.with(|c| c.set(d));
}

/// The scan time noted for the current analysis on this thread.
pub(crate) fn noted_diff() -> Duration {
    DIFF.with(Cell::get)
}

/// Called by the worker after each analysis (only while armed; capped at [`MAX_SAMPLES`]).
pub(crate) fn record(sample: WorkerSample) {
    if !armed() {
        return;
    }
    let mut samples = SAMPLES.lock().unwrap_or_else(|e| e.into_inner());
    if samples.len() < MAX_SAMPLES {
        samples.push(sample);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(us: u64) -> WorkerSample {
        WorkerSample {
            kind: Built::Incremental,
            diff: Duration::from_micros(us),
            apply: Duration::from_micros(2 * us),
            analysis: Duration::from_micros(4 * us),
        }
    }

    /// One test drives the process-wide store, so no other test races it.
    #[test]
    fn records_only_while_armed_and_take_empties() {
        arm(false);
        take();
        record(sample(1));
        assert!(take().is_empty(), "unarmed: nothing kept");
        {
            let _armed = armed_until_dropped();
            assert!(armed());
            assert!(take().is_empty(), "arming clears what was left");
            record(sample(2));
            record(sample(3));
            assert_eq!(take(), vec![sample(2), sample(3)]);
            assert!(take().is_empty());
        }
        assert!(!armed(), "the guard disarms when it drops");
        record(sample(4));
        assert!(take().is_empty());
        // A panic while armed still disarms.
        let r = std::panic::catch_unwind(|| {
            let _armed = armed_until_dropped();
            panic!("a failed run");
        });
        assert!(r.is_err());
        assert!(!armed());
    }

    #[test]
    fn the_relex_is_the_scan_and_the_apply() {
        assert_eq!(sample(5).relex(), Duration::from_micros(15));
        note_diff(Duration::from_micros(7));
        assert_eq!(noted_diff(), Duration::from_micros(7));
        note_diff(Duration::ZERO);
        assert_eq!(noted_diff(), Duration::ZERO);
    }
}
