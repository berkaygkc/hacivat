//! The run service (HV-EXEC-01 wrapping, -05 submit, -11 model; Task 23): execution units from
//! the execution-mode split with cursor eligibility and the PostgreSQL oracle for ambiguity,
//! one session request per run, run status, result tabs, the output-log model and the console
//! UX state.
//!
//! Types since M3a Task 1b; bodies since Task 23 (ADR 0006 `m3a-t23`).

pub mod log;
mod observer;
pub mod service;
pub mod sqlctx;
pub mod status;
mod titles;
pub mod units;
pub mod ux;

pub use log::{
    LogArg, LogLine, LogMsg, SessionLog, Severity, UnitMsg, copy_invariant, duration_message,
    size_message,
};
pub use service::{ExecService, ResultTab, RunRequest, RunTicket, TabKey};
pub use status::{
    ErrorView, RunStatus, UnitStatus, UnitStatusEntry, UnitSummary, doc_offset_of, error_hint_id,
    error_title_id, error_view,
};
pub use units::{
    ExecUnit, LaneOracle, OracleLane, RefuseReason, Resolution, RunAction, SplitOracle, policy_for,
    resolve_units, resolve_units_with, uncertain_message_id, units_for_candidate,
};
pub use ux::{CancelUx, ConsoleUx, TxBadge, console_ux};
