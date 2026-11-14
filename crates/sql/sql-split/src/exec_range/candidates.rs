//! Execution units, chooser candidates and `resolve` (Task 13).

use sql_text::{TextRange, TextSize};

use crate::document::DocSnapshot;
use crate::exec_range::policy::ExecPolicy;
use crate::item::{AmbiguousBoundary, SplitMode, Stmt};
use crate::key::StmtKey;

/// Where Run was asked: a caret, or a selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaretOrSelection {
    Caret(TextSize),
    Selection(TextRange),
}

/// What a unit is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnitKind {
    Statement,
    Subquery,
    Batch,
    /// Sent as written (a selection run as one, M3b).
    Verbatim,
    /// psql `COPY … FROM STDIN` with its data range.
    CopyIn {
        data: TextRange,
    },
}

/// One unit to run: its strict range (no terminator, no leading comments; the sent text), its
/// kind, its statement key and its title comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnitSpan {
    pub range: TextRange,
    pub kind: UnitKind,
    pub key: Option<StmtKey>,
    pub title_comment: Option<TextRange>,
}

/// A chooser candidate's kind, in resolve order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CandidateKind {
    Subquery,
    Statement,
    LargestOrBatch,
    FromCaret,
    WholeScript,
}

/// One chooser entry: its units and its label (whitespace collapsed, controls as spaces; the
/// UI truncates it).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Candidate {
    pub kind: CandidateKind,
    pub units: Vec<UnitSpan>,
    pub label: String,
}

/// What Run at a caret or selection runs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RangeResolution {
    Units(Vec<UnitSpan>),
    Ask {
        candidates: Vec<Candidate>,
        preselect: usize,
    },
    NothingToRun,
    /// An uncertain boundary is involved: nothing runs (R-M3-24).
    Ambiguous(Vec<AmbiguousBoundary>),
    UnsupportedClientCommand {
        at: TextRange,
    },
}

/// The innermost runnable subquery at a caret (needs the parser: `None` in M3a).
pub trait SubqueryOracle: Send + Sync {
    fn innermost_runnable_subquery(
        &self,
        doc: &DocSnapshot,
        stmt: &Stmt,
        caret: TextSize,
    ) -> Option<TextRange>;
}

/// What `at` runs under `policy` in `mode` (Task 13); `remembered` is the console's last
/// chosen kind, preselected in the chooser.
///
/// Task 1b stub (fail closed): `NothingToRun`.
pub fn resolve(
    doc: &DocSnapshot,
    at: &[CaretOrSelection],
    policy: &ExecPolicy,
    mode: SplitMode,
    subq: Option<&dyn SubqueryOracle>,
    remembered: Option<CandidateKind>,
) -> RangeResolution {
    let _ = (doc, at, policy, mode, subq.is_some(), remembered);
    RangeResolution::NothingToRun
}
