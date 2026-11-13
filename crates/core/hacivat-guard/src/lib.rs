//! The read-only guard, sans I/O (SQL-SAFE-05, HV-SESS-04, MCP-09 step 1; B21): per-engine
//! guard plans, post-statement checks, input limits, SQLite hardening and its authorizer, the
//! role probe and the honest access label. No I/O, no async, no runtime: the session actor
//! (`hacivat-session::session::readonly`) runs what this crate decides.
//!
//! Task 1 ships the interface: [`plan_for`] works (Q8, Task 5's exact values); every other
//! function is a fail-closed stub ("No silent weakening") until Task 5 fills it.

pub mod input;
pub mod label;
pub mod plan;
pub mod post;
pub mod probe;
pub mod sqlite;

pub use input::{InputViolation, MAX_STATEMENT_BYTES, check_input};
pub use label::{access_label, remediation};
pub use plan::{GuardPlan, ResetKind, plan_for};
pub use post::{GapReason, PostCheck, PostFacts, TxStatusLite, check_post};
pub use probe::{RoleFindings, RoleWarning, interpret_probe, role_probe_sql};
pub use sqlite::{
    Authorization, SqliteAction, SqliteHardening, sqlite_authorize, sqlite_hardening,
};
