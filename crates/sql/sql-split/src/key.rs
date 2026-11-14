//! Content-addressed statement keys (R-M3-4, R-M3-42; Task 8).

/// The XXH3-128 (`twox-hash`) of a statement's dialect, modes, delimiter and text; no
/// interner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StmtKey(pub u128);
