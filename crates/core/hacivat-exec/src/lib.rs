//! The run service (HV-EXEC-01 wrapping, -05 submit, -11 model; Task 23): execution units from
//! the execution-mode split with cursor eligibility and the PostgreSQL oracle for ambiguity,
//! one session request per run, run status, result tabs, the output-log model and the console
//! UX state.
//!
//! Types complete since M3a Task 1b. Task 1b stubs (fail closed): `resolve_units` answers
//! `NothingToRun`; `ExecService::run` sends nothing and answers its ticket with an aborted
//! summary. `SessionLog` works (a bounded ring). The other bodies panic naming Task 23.

pub mod log;
mod observer;
pub mod service;
pub mod sqlctx;
pub mod status;
mod titles;
pub mod units;
pub mod ux;

pub use log::{LogArg, LogLine, LogMsg, SessionLog, Severity, copy_invariant};
pub use service::{ExecService, ResultTab, RunRequest, RunTicket, TabKey};
pub use status::{ErrorView, RunStatus, UnitStatus, UnitStatusEntry, UnitSummary};
pub use units::{ExecUnit, Resolution, RunAction, policy_for, resolve_units, units_for_candidate};
pub use ux::{CancelUx, ConsoleUx, TxBadge, console_ux};
