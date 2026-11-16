//! The SQL classifier (`alg/13` SQL-SAFE-01..07; M3b Task 1a seams).
//!
//! [`classify`] reads one request's text and answers a [`SafetyReport`]: per statement, the
//! effects it may have, its risk flags, the objects it touches, the destructive patterns it
//! holds and the reasons for any doubt. It is pure, synchronous and never panics; the caller
//! runs it on `hacivat-exec::classify_pool` (16 MiB stacks, R-M3-53). [`decide`] maps a report
//! to a verdict under a [`Profile`]; [`gate_class`] is the exec gate's view of one statement.
//!
//! Fail closed: until Task 14 merges, `classify` answers `Unknown` with the pre-decision
//! `[NotImplemented]`, which `decide` denies in every profile (F31). `decide`, `gate_class`
//! and `opens_write` are Task 4's: an empty or unnamed effect set is `Unknown`, never a read.
//!
//! Backends: libpg_query through `sql-oracle-pg` for PostgreSQL (Task 9) and the interim
//! `sqlparser` path with a lexer pre-scan for MySQL, MariaDB and SQLite (Task 10). No
//! `sqlparser` or `pg_query` type appears in this crate's public API.

// Crate-private: no backend type is public (R-M3-54).
mod backend;
pub mod classify;
pub mod destructive;
pub mod effects;
pub mod fingerprint;
pub mod functions;
pub mod gate_class;
pub mod limits;
pub mod policy;
pub mod reason;
pub mod report;
mod visit;

pub use classify::{ClassifyCtx, Expect, classify};
pub use destructive::{DestructiveKind, DestructiveReason};
pub use effects::{CoarseClass, EffectSet, RiskFlags};
pub use fingerprint::classified_hash;
pub use functions::{FnCatalog, FnLookup, denied_function_names, set_allowed};
pub use gate_class::{GateClass, gate_class, opens_write};
pub use limits::Limits;
pub use policy::{Actor, Decision, Profile, Verdict, decide};
pub use reason::{Reason, ReasonCode};
pub use report::{
    BackendVersion, Confidence, ObjectKind, ObjectRef, SafetyReport, SetTarget, StmtKind,
    StmtReport, TouchedObjects,
};

// ## m3b-t01a-seams (fix round 1, I1): the SQLite read-pragma list (Task 10) that Task 18's
// cross-crate equality test compares with the guard's.
pub mod pragmas;

pub use pragmas::sqlite_read_pragmas;

// ## m3b-t10-interim (named exception in T1a's file): the interim backend's harness entry, for
// Task 10's tests and read corpus until Task 14's `classify` runs the backend. It takes and
// gives this crate's types only (R-M3-54).
#[doc(hidden)]
pub use backend::interim::classify_interim;
// Fix round 1 (I2): the SAFE-07 constant-predicate rule every backend shares.
pub use destructive::{CmpOp, Predicate, always_true};
