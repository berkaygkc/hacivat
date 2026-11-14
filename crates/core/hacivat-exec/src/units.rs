//! Execution units (HV-EXEC-01; Task 23): `exec_range::resolve` in execution mode, each unit
//! with its strict text and cursor eligibility; PostgreSQL ambiguity through the oracle when
//! it covers the region exactly (R-M3-24).

use std::ops::Range;
use std::sync::Arc;

use sql_split::AmbiguousBoundary;
use sql_split::DocSnapshot;
use sql_split::exec_range::{Candidate, CandidateKind, CaretOrSelection, ExecPolicy, UnitKind};

/// Which Run the user asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunAction {
    /// D-011: the statement at the caret (ask when several fit).
    Execute1,
    /// The largest statement at the caret.
    Execute2,
    /// The whole script.
    Execute3,
}

/// The policy of `a`: Execute1 is D-011's default, Execute2 `largest()`, Execute3
/// `whole_script()`.
pub fn policy_for(a: RunAction) -> ExecPolicy {
    match a {
        RunAction::Execute1 => ExecPolicy::default(),
        RunAction::Execute2 => ExecPolicy::largest(),
        RunAction::Execute3 => ExecPolicy::whole_script(),
    }
}

/// One unit to run: the exact text sent, where it is in the document, its kind and whether
/// it may stay suspended as a console cursor (R-M3-23).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExecUnit {
    pub text: Arc<str>,
    pub doc_range: Range<usize>,
    pub kind: UnitKind,
    pub suspendable: bool,
}

/// What Run at a caret or selection does.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Resolution {
    Units(Vec<ExecUnit>),
    Ask {
        candidates: Vec<Candidate>,
        preselect: usize,
    },
    NothingToRun,
    /// A boundary is uncertain: nothing is sent (R-M3-24).
    Uncertain(Vec<AmbiguousBoundary>),
    UnsupportedClientCommand {
        at: Range<usize>,
    },
}

/// The units Run `action` at `at` runs (Task 23). The caller runs it off the UI thread.
///
/// Task 1b stub (fail closed): `NothingToRun`.
pub fn resolve_units(
    doc: &DocSnapshot,
    at: &[CaretOrSelection],
    action: RunAction,
    remembered: Option<CandidateKind>,
) -> Resolution {
    let _ = (doc, at, action, remembered);
    Resolution::NothingToRun
}

/// The units of the chooser's candidate `c` (Task 23). Task 1b stub (fail closed): none.
pub fn units_for_candidate(doc: &DocSnapshot, c: &Candidate) -> Vec<ExecUnit> {
    let _ = (doc, c);
    Vec::new()
}
