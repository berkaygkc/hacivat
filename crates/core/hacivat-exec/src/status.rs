//! Run status, per unit (Task 23): what the gutter marks and the output log show, and the
//! error view of a failed unit.

use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use hacivat_driver::ServerMessage;
use hacivat_proto::DbmsKind;
use hacivat_session::session::{
    ExecError, ExecErrorKind, RowsOutcome, RunId, RunSummary, TxError, UnitId,
};
use sql_text::LineIndex;

use crate::log::deny_id;

/// A succeeded unit's numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnitSummary {
    pub rows: Option<RowsOutcome>,
    pub exec_ms: u64,
    pub fetch_ms: u64,
}

/// A failed unit, for the log and the gutter: a localized title, the SQLSTATE or vendor code
/// chip, the server's text verbatim (never inside a translated sentence), Hacivat's own hint
/// when it has one, and where the error is in the document.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ErrorView {
    /// What kind of failure it was ([`error_title_id`]); shown where there is no server text.
    pub title_id: &'static str,
    pub code: Option<Arc<str>>,
    pub server_text: Option<Arc<str>>,
    /// Hacivat's hint, a Fluent ID (SQLite's busy hint, `error-exec-sqlite-busy-hint`).
    pub hint_id: Option<&'static str>,
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

impl UnitStatus {
    /// The status's name (`Queued`, `Running`, …), for tests and diagnostics.
    pub fn name(&self) -> &'static str {
        match self {
            UnitStatus::Queued { .. } => "Queued",
            UnitStatus::Running { .. } => "Running",
            UnitStatus::Fetching { .. } => "Fetching",
            UnitStatus::Suspended { .. } => "Suspended",
            UnitStatus::Cancelling => "Cancelling",
            UnitStatus::Succeeded(_) => "Succeeded",
            UnitStatus::Failed(_) => "Failed",
            UnitStatus::Cancelled => "Cancelled",
            UnitStatus::Skipped => "Skipped",
        }
    }

    /// Whether the unit is done with the run (it may still hold a suspended result).
    pub fn is_final(&self) -> bool {
        matches!(
            self,
            UnitStatus::Succeeded(_)
                | UnitStatus::Failed(_)
                | UnitStatus::Cancelled
                | UnitStatus::Skipped
                | UnitStatus::Suspended { .. }
        )
    }
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
    /// The run (`None` only for the default value).
    pub run: Option<RunId>,
    /// When its first unit started (`None` while queued, or when it never started).
    pub started: Option<Instant>,
    pub units: Arc<[UnitStatusEntry]>,
    /// Set once the run ended (or never started); later changes are the cursor's.
    pub summary: Option<RunSummary>,
}

/// The title of an error of `kind` (an `exec-*` Fluent ID), for the gutter tooltip and the log
/// line under "Statement N failed" when the server sent no text (NOTES ruling 17: no curated
/// per-code titles in M3a).
pub fn error_title_id(kind: &ExecErrorKind) -> &'static str {
    match kind {
        ExecErrorKind::Db => "exec-error-server",
        ExecErrorKind::ConnectionLost {
            outcome_unknown: false,
        } => "exec-error-connection-lost",
        ExecErrorKind::ConnectionLost {
            outcome_unknown: true,
        } => "exec-error-connection-lost-unknown",
        ExecErrorKind::Cancelled => "exec-error-cancelled",
        ExecErrorKind::ReadOnlyDenied(layer) => deny_id(*layer),
        ExecErrorKind::Tx(TxError::FailedNeedsRollback) => "exec-error-tx-failed",
        ExecErrorKind::Tx(TxError::InProgress) => "exec-error-tx-in-progress",
        ExecErrorKind::Tx(TxError::NeedsClassifier | TxError::Unsupported) => {
            "exec-error-tx-unsupported"
        }
        ExecErrorKind::EnforcementGap(_) => "exec-error-enforcement-gap",
        ExecErrorKind::NotConnected => "exec-error-not-connected",
        ExecErrorKind::Internal => "exec-error-internal",
    }
}

/// The SQLite primary result code `SQLITE_BUSY`; an extended code keeps it in its low byte.
const SQLITE_BUSY: i64 = 5;

/// Hacivat's hint for a server error, as a Fluent ID: SQLite's busy hint for `SQLITE_BUSY`
/// (any extended code), as `hacivat_driver_sqlite::error_hint` gives it (the same ID and rule;
/// this crate does not depend on the SQLite driver, ADR 0006 `m3a-t23`).
pub fn error_hint_id(engine: DbmsKind, e: &ExecError) -> Option<&'static str> {
    let code = e.server.as_ref()?.vendor_code?;
    (engine == DbmsKind::Sqlite && e.kind == ExecErrorKind::Db && code & 0xff == SQLITE_BUSY)
        .then_some("error-exec-sqlite-busy-hint")
}

/// The view of `e`, a failure of the unit whose text is `unit_text` at `unit_start` in the
/// document. `position` is the server's 1-based character cursor into the sent text, when it
/// reported one ([`doc_offset_of`]).
pub fn error_view(
    engine: DbmsKind,
    e: &ExecError,
    unit_text: &str,
    unit_start: usize,
    position: Option<u32>,
) -> ErrorView {
    let server: Option<&ServerMessage> = e.server.as_ref();
    let code = server.and_then(|m| {
        m.sqlstate
            .as_deref()
            .map(Arc::from)
            .or_else(|| m.vendor_code.map(|c| Arc::from(c.to_string())))
    });
    ErrorView {
        title_id: error_title_id(&e.kind),
        code,
        server_text: server.map(|m| Arc::from(m.text.as_str())),
        hint_id: error_hint_id(engine, e),
        // ## m3a-t33-exec-ui (named exception): without a caller's position, the server
        // message's own (the driver reports it since Task 33).
        doc_offset: position
            .or_else(|| server.and_then(|m| m.position))
            .and_then(|p| doc_offset_of(unit_text, unit_start, p)),
    }
}

/// The document offset of a server error cursor: `position` is 1-based in characters of the
/// sent text (PostgreSQL's `P` field), mapped through `LineIndex::char_to_byte` into the unit
/// and then by the unit's start into the document. `None` for 0 or a cursor past the text's
/// end.
pub fn doc_offset_of(unit_text: &str, unit_start: usize, position: u32) -> Option<usize> {
    let char_idx = usize::try_from(position.checked_sub(1)?).ok()?;
    let at = LineIndex::new(unit_text).char_to_byte(unit_text, char_idx)?;
    unit_start.checked_add(usize::from(at))
}
