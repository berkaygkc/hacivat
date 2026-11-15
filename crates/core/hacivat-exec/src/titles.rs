//! Result tab titles: `Result N` in M3a (Task 23; comment titles are M3b).
//!
//! The title is the run service's own label for a tab (logs, tests, a future tab list); the
//! app's tab strip shows its localized `hv-results-result-n`, numbered per console (Task 26).

use std::sync::Arc;

/// The title of a run's `n`-th result tab, from 1.
pub(crate) fn result_title(n: usize) -> Arc<str> {
    format!("Result {n}").into()
}
