//! The run service (HV-EXEC-01 wrapping, -05 submit, -11 model; Task 23): execution units from
//! the execution-mode split with cursor eligibility and the PostgreSQL oracle for ambiguity,
//! one session request per run, run status, result tabs, the output-log model and the console
//! UX state.
//!
//! Types since M3a Task 1b; bodies since Task 23 (ADR 0006 `m3a-t23`).
//!
//! M3b (Task 1c seams; ADR 0006 `m3b-t01c`): the classifier pool, parameter preparation, the
//! exec gate and its confirmation port, the D-062 wrap, "don't ask again", the auditor chain,
//! "Count rows" and the history store. Every one is a fail-closed stub until its task, and
//! none is wired into Run before Task 24 (R-M3-52).

pub mod log;
mod observer;
pub mod service;
pub mod sqlctx;
pub mod status;
mod titles;
pub mod units;
pub mod ux;
// ## m3b-t01c-core-seams
pub mod audit;
pub mod classify_pool;
pub mod count;
pub mod explain_wrap;
pub mod gate;
pub mod history;
pub mod params;
pub mod suppress;

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
// ## m3a-t33-exec-ui (named exception)
pub use ux::session_live;
// ## m3b-t01c-core-seams
pub use audit::{AuditorChain, SINK_CAPACITY, SyncAuditor};
pub use classify_pool::ClassifyPool;
pub use count::count_sql;
// ## m3b-t16-run-service (fix round 1, named exception: re-export)
pub use count::count_prepared;
pub use explain_wrap::explain_analyze_inner;
pub use gate::{
    CommitQuestion, ConfirmAnswer, ConfirmPort, ConfirmQuestion, ConnSafety, GateDecision,
    GateItem, GatePlan, GateReason, gate,
};
pub use history::{
    HistoryConfig, HistoryError, HistoryHit, HistoryMeta, HistoryOrigin, HistoryQuery,
    HistoryRecord, HistoryStore, Outcome,
};
pub use params::{OffsetMap, ParamPrompt, ParamValueStore, Prepared, PreparedUnit, prepare_units};
pub use suppress::SuppressSet;
// Ruling I2 (m3b-t01c): the app takes no `sql-safety` dependency; the classifier's types it
// builds or passes on (Task 24's `ClassifyCtx`/`Expect` for the pool, the reports it hands to
// `gate`, the class a `GateItem` carries, the reasons a view names) come through here.
pub use sql_safety::{
    ClassifyCtx, Expect, FnCatalog, GateClass, Limits, Reason, ReasonCode, SafetyReport,
};
// The session's answers the run service's callers name with these (Tasks 21, 23, 24).
pub use hacivat_session::session::{CommitAnswer, ErrorPolicy, RunChoice};
// ## m3b-t16-run-service (named exception: re-exports only)
pub use log::span_message;
pub use service::{
    COUNT_TIMEOUT, CountError, CountRequest, CountTicket, SKIPPED_BY_USER, apply_answer,
};
pub use titles::{TITLE_GRAPHEMES, TITLE_MAX_GRAPHEMES, comment_title, short_title};
