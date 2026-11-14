//! The output-log model (HV-EXEC-11; Task 23): one ring of lines per console. A line's text is
//! a Fluent message (`fluent_id` and `args`) rendered by the app; server text stays in its own
//! field (`ErrorView::server_text`).
//!
//! Working since Task 1b: `SessionLog` keeps the newest [`SessionLog::CAPACITY`] lines and
//! bumps its `watch` on every change. `LogMsg::fluent_id` and `args`, the `Dropped` line for a
//! burst that outruns the ring, and `copy_invariant` arrive in Task 23.

use std::collections::VecDeque;
use std::ops::Range;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use hacivat_results::CloseReason;
use hacivat_session::session::{DenyLayer, RunSummary, UnitId};
use tokio::sync::watch;

use crate::status::ErrorView;

/// A line's severity (ink glyphs; never a new hue).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    Info,
    Notice,
    Warning,
    Error,
}

/// What a line says.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LogMsg {
    RowsRetrieved {
        rows: u64,
        more: bool,
        exec_ms: u64,
        fetch_ms: u64,
    },
    RowsAffected {
        rows: u64,
        ms: u64,
    },
    Completed {
        ms: u64,
    },
    TxCommitted,
    TxRolledBack,
    /// `rolled_back`: Hacivat's own transaction ended in a rollback (R-M3-23, P26).
    CursorClosed {
        rows: u64,
        reason: CloseReason,
        rolled_back: bool,
    },
    MoreResultSets {
        skipped: u32,
    },
    Summary(RunSummary),
    Error(ErrorView),
    Cancelled,
    ReadOnlyDenied(DenyLayer),
    /// `n` lines a burst pushed out of the ring.
    Dropped {
        n: u64,
    },
}

/// A message argument.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LogArg {
    U64(u64),
    Str(Arc<str>),
}

impl LogMsg {
    /// The message's Fluent ID in `hacivat-core.ftl`: a literal in every arm, which the
    /// `l10n check` code scan counts as used (P24; Task 23).
    pub fn fluent_id(&self) -> &'static str {
        unimplemented!("LogMsg::fluent_id ({self:?}): Task 23")
    }

    /// The message's arguments (Task 23).
    pub fn args(&self) -> Vec<(&'static str, LogArg)> {
        unimplemented!("LogMsg::args ({self:?}): Task 23")
    }
}

/// One line of a console's output log.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LogLine {
    pub seq: u64,
    pub at: SystemTime,
    pub severity: Severity,
    pub msg: LogMsg,
    /// The statement's range in the document (a click selects it).
    pub source: Option<Range<usize>>,
    pub unit: Option<UnitId>,
}

/// A console's output log: a ring of the newest lines.
#[derive(Debug)]
pub struct SessionLog {
    lines: Mutex<VecDeque<LogLine>>,
    changes: watch::Sender<u64>,
}

impl SessionLog {
    /// The most lines kept; a full ring drops the oldest.
    pub const CAPACITY: usize = 10_000;

    pub fn new() -> Arc<Self> {
        Arc::new(SessionLog {
            lines: Mutex::new(VecDeque::new()),
            changes: watch::Sender::new(0),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<LogLine>> {
        self.lines.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Appends `line` (dropping the oldest when the ring is full) and bumps `changes`.
    pub fn push(&self, line: LogLine) {
        {
            let mut lines = self.lock();
            if lines.len() == Self::CAPACITY {
                lines.pop_front();
            }
            lines.push_back(line);
        }
        self.changes.send_modify(|n| *n += 1);
    }

    /// The lines, oldest first.
    pub fn lines(&self) -> Arc<[LogLine]> {
        self.lock().iter().cloned().collect()
    }

    /// Bumped on every push and clear.
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// Empties the log (the toolbar's Clear) and bumps `changes`.
    pub fn clear(&self) {
        self.lock().clear();
        self.changes.send_modify(|n| *n += 1);
    }
}

/// The log as locale-free text for the clipboard: ISO timestamps, ASCII digits, `ms`
/// (Task 23).
pub fn copy_invariant(lines: &[LogLine]) -> String {
    let _ = lines;
    unimplemented!("copy_invariant: Task 23")
}
