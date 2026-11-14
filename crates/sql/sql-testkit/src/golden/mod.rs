//! The D-011 goldens (Task 13): `┃` carets and `«…»` selections in the fixtures, or
//! `caret = { after = "…", occurrence = n }` anchors; each case resolves through
//! `sql_split::exec_range::resolve` and compares units, candidates and labels.
//!
//! Task 1b stub: `load` finds no case and `run` fails every case, so a golden suite cannot pass
//! before Task 13 (its ID-coverage meta-test catches a silently empty corpus).

mod markers;
mod runner;

use std::path::Path;

/// One golden case (Task 13 adds its fields).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    pub id: String,
}

/// The cases of the corpus file at `path` (Task 13).
pub fn load(path: &Path) -> Vec<Case> {
    let _ = path;
    Vec::new()
}

/// Runs `case`: `Err` names what differs (Task 13).
pub fn run(case: &Case) -> Result<(), String> {
    Err(format!(
        "{}: the golden runner is not implemented yet",
        case.id
    ))
}
