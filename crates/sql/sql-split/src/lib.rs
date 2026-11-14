//! Statement splitting (SQL-SPLIT-01..03; Task 8), `SqlDocument` and its snapshots, cursor
//! eligibility (R-M3-23; Task 8), and caret/selection to execution units with the D-011
//! chooser (SQL-SPLIT-04; Task 13).
//!
//! Execution mode never uses a heuristic boundary (D-06): an uncertain boundary is an
//! `AmbiguousBoundary`, which execution never runs (R-M3-24).
//!
//! Types complete since M3a Task 1b. Task 1b stubs (fail closed): the splitter returns one
//! `Heuristic` item over the input, `head::cursor_eligible` answers `false`,
//! `exec_range::resolve` answers `NothingToRun`. `SqlDocument` works over them (a full rebuild
//! per change, as M3a's body will be).

pub mod client;
pub mod document;
pub mod exec_range;
pub mod head;
pub mod item;
pub mod key;
pub mod recover;
pub mod splitter;

pub use document::{DocDelta, DocSnapshot, SqlDocument};
pub use item::{
    AmbiguityReason, AmbiguousBoundary, ClientSegment, Confidence, Item, Split, SplitMode, Stmt,
    StmtHead,
};
pub use key::StmtKey;
pub use splitter::split;
