//! The metadata model (HV-MODEL-01..03, HV-INTRO-11, -14, -16 consumer side; alg/02 §1).
//!
//! M4 Task 1a seams: every type is declared here and every body is a fail-closed stub until its
//! owning task lands (Global Constraints, "No silent weakening").

pub mod codec;
pub mod delta;
#[cfg(feature = "test-support")]
pub mod fixture;
pub mod fingerprint;
pub mod ids;
pub mod kind;
pub mod merge;
pub mod name;
pub mod namespace;
pub mod overlay;
#[cfg(feature = "persist")]
pub mod persist;
pub mod props;
pub mod resolve;
pub mod search;
pub mod search_path;
pub mod snapshot;
pub mod sql_view;
pub mod system;
pub mod userdata;
pub mod view;
