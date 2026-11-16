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
//! - [`SqlDocument`]: a text, its lexing and its split, relexed and resplit around each change
//!   (`incremental`, M3b Task 5); [`DocSnapshot`] shares one revision's analysis and computes
//!   the execution split once. [`StreamSplitter`] splits a script fed in chunks.
//! - [`StmtKey`]: XXH3-128 of a statement's dialect, modes, delimiter and text.
//! - [`head::cursor_eligible`]: whether a unit may stay suspended as a console cursor.
//!
//! - [`exec_range::resolve`]: what Run at a caret or selection runs, for the preview and for
//!   execution, with the D-011 policies and the chooser's candidates.

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

// ## m3b-t01a-seams: parameters, templates and HvNorm1 (Task 6); incremental resplit and the
// streaming splitter (Task 5). Fail-closed stubs until then.
pub mod fingerprint;
pub mod incremental;
pub mod params;
pub mod stream;
pub mod templates;

pub use fingerprint::{Fingerprint, FingerprintAlgo, hv_norm1};
pub use params::{
    BindPlan, ParamDiag, ParamScan, ParamSettings, Placeholder, PlaceholderKind, RewrittenSql,
    scan_params,
};
pub use stream::{STREAM_STATEMENT_MAX, StreamItem, StreamSplitter};
pub use templates::{TemplatePattern, TemplateVar};
