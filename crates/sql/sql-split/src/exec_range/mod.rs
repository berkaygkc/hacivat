//! Caret or selection to execution units, with the D-011 policies and chooser candidates
//! (SQL-SPLIT-04 aligned to D-011, R-M3-8; Task 13).
//!
//! One function, [`resolve`], serves the Run preview (`SplitMode::Editor`) and execution
//! (`SplitMode::Execution`): they agree on every exact statement, and execution never returns
//! a unit that crosses a heuristic boundary (an uncertain one is `Ambiguous`).

mod candidates;
mod policy;

pub use candidates::{
    Candidate, CandidateKind, CaretOrSelection, RangeResolution, SubqueryOracle, UnitKind,
    UnitSpan, resolve,
};
pub use policy::{ExecPolicy, InsidePolicy, OutsidePolicy, SelectionPolicy};
