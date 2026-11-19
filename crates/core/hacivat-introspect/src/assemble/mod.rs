// ## m4-t01c-core
//! Composite object assembly (HV-INTRO-13; Task 13): keys, indexes, foreign keys and checks
//! assembled from per-dialect catalog rows (`assemble::{pg, mysql, sqlite}`), columns resolved
//! by ordinal then name, an unresolved reference kept with the `BROKEN` flag, MySQL key parts
//! grouped client-side.
//!
//! **Fail-closed stub (M4 Task 1c).** No assembler exists yet; Task 13 declares the row and
//! result types with its adapters. An assembly that cannot be done answers
//! [`AssembleError::NotImplemented`], never an object with missing parts.

/// Why an assembly failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssembleError {
    /// Not written yet (M4 Task 1c stub).
    NotImplemented,
}
