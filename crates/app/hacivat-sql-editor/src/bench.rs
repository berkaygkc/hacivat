// ## m3c-t44: the worker's timer for the real-app B3 bench (R-M3-9; feature `bench` only).
//! The worker's own timer (M3c Task 44, R-M3-9): while armed, every analysis the per-document
//! worker runs records how long it took to bring its kept document up to the job's text (the
//! one-edit diff of the two texts, then `SqlDocument::apply`: the incremental relex and resplit)
//! and the whole analysis (with the execution split, highlighting, statements and folds).
//!
//! The figures are taken where the real app takes them, on the GPUI background executor, so
//! they include the rope and the scheduling the app's worker really has. The real-app B3 bench
//! (`hacivat_app::bench::b3`) arms the timer for its measured keystrokes and grades the relex
//! figure against R-M3-9's 2 ms p95. Process-wide: the bench runs one typing console at a time.
//! The module exists only with the `bench` feature; no product build records anything.

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
    /// The one-edit diff of the kept text and the job's rope, then `SqlDocument::apply` (the
    /// relex and the resplit) — or the fresh build for [`Built::Fresh`].
    pub relex: Duration,
    /// The whole analysis: `relex`, then the execution split, highlighting, statements, folds.
    pub analysis: Duration,
}

static ARMED: AtomicBool = AtomicBool::new(false);
static SAMPLES: Mutex<Vec<WorkerSample>> = Mutex::new(Vec::new());

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

/// The samples recorded since the last `take` (or `arm(true)`); the store is left empty.
pub fn take() -> Vec<WorkerSample> {
    std::mem::take(&mut *SAMPLES.lock().unwrap_or_else(|e| e.into_inner()))
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
            relex: Duration::from_micros(us),
            analysis: Duration::from_micros(2 * us),
        }
    }

    /// One test drives the process-wide store, so no other test races it.
    #[test]
    fn records_only_while_armed_and_take_empties() {
        arm(false);
        take();
        record(sample(1));
        assert!(take().is_empty(), "unarmed: nothing kept");
        record(sample(9));
        arm(true);
        assert!(take().is_empty(), "arming clears what was left");
        record(sample(2));
        record(sample(3));
        assert_eq!(take(), vec![sample(2), sample(3)]);
        assert!(take().is_empty());
        arm(false);
        record(sample(4));
        assert!(take().is_empty());
    }
}
