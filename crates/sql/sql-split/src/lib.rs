//! Statement splitting (SQL-SPLIT-01..03; Task 8), `SqlDocument` and its snapshots, cursor
//! eligibility (R-M3-23; Task 8), and caret/selection to execution units with the D-011
//! chooser (SQL-SPLIT-04; Task 13).
//!
//! Execution mode never uses a heuristic boundary (D-06): an uncertain boundary is an
//! `AmbiguousBoundary`, which execution never runs (R-M3-24).
//!
//! - [`split`] / [`split_from`]: one forward loop over the lexer's tokens that recognises
//!   client commands (psql, mysql, sqlite3), tracks blocks per dialect
//!   (`sql_dialect::SplitProfile`), and returns the editor or the execution view ([`Split`]).
//!   [`Split::is_certain`] / [`Split::runnable`] say what execution may send: every uncertain
//!   boundary lies inside the one statement it refuses, which is never `Exact`.
//! - [`SqlDocument`]: a text, its lexing and its split, rebuilt on every change in M3a;
//!   [`DocSnapshot`] shares one revision's analysis and computes the execution split once.
//! - [`StmtKey`]: XXH3-128 of a statement's dialect, modes, delimiter and text.
//! - [`head::cursor_eligible`]: whether a unit may stay suspended as a console cursor.
//!
//! Task 1b stub (fail closed) still here: `exec_range::resolve` answers `NothingToRun`
//! (Task 13).

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
    AmbiguityReason, AmbiguousBoundary, ClientKind, ClientSegment, Confidence, CopyData, DiagCode,
    Diagnostic, Item, Split, SplitMode, Stmt, StmtHead,
};
pub use key::StmtKey;
pub use splitter::{SplitStart, split, split_from};
