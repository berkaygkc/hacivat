//! Run status, per unit (Task 23): what the gutter marks and the output log show.

use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use hacivat_session::session::{RowsOutcome, RunSummary, UnitId};

/// A succeeded unit's numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnitSummary {
    pub rows: Option<RowsOutcome>,
    pub exec_ms: u64,
    pub fetch_ms: u64,
}

/// A failed unit, for the log and the gutter: a localized title, the SQLSTATE or vendor code
/// chip, the server's text verbatim (never inside a translated sentence), and where the error
/// is in the document.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ErrorView {
    pub title_id: &'static str,
    pub code: Option<Arc<str>>,
    pub server_text: Option<Arc<str>>,
    pub doc_offset: Option<usize>,
}

/// Where a unit is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UnitStatus {
    Queued { position: u32 },
    Running { since: Instant },
    Fetching { rows: u64 },
    Suspended { rows: u64 },
    Cancelling,
    Succeeded(UnitSummary),
    Failed(ErrorView),
    Cancelled,
    Skipped,
}

/// One unit's status, with its range in the document.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UnitStatusEntry {
    pub unit: UnitId,
    pub doc_range: Range<usize>,
    pub status: UnitStatus,
}

/// A run's status.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct RunStatus {
    pub units: Arc<[UnitStatusEntry]>,
    pub summary: Option<RunSummary>,
}
