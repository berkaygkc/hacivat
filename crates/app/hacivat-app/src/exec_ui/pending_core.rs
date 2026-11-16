//! ## m3b-t01b-seams — PLACEHOLDER for Task 1c's core types (P-M3b-20: Tasks 1b and 1c run in
//! parallel from Task 1a's manifest commit, so these do not exist on Task 1b's base).
//!
//! Each item copies the name, the fields and the variants the plan gives Task 1c, nothing
//! more. **At the merge with Task 1c this file is deleted** and every `pending_core::X` import
//! becomes the real path named on the item (`hacivat_exec::…` or
//! `hacivat_session::session::…`); no other line changes. Nothing outside `exec_ui`'s stubs and
//! `tests/m3b_routing.rs` names it.

use std::sync::Arc;
use std::time::Instant;

use hacivat_session::session::{RunId, UnitId};
use hacivat_values::BindValue;
use tokio::sync::oneshot;

/// Real path: `hacivat_exec::GateReason` (Task 1c, `gate.rs`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GateReason {
    pub fluent_id: &'static str,
    pub args: Vec<(&'static str, Arc<str>)>,
}

/// Real path: `hacivat_exec::ConfirmQuestion` (Task 1c, `gate.rs`).
#[derive(Debug, Clone)]
pub struct ConfirmQuestion {
    pub run: RunId,
    pub unit: u32,
    pub index: u32,
    pub total: u32,
    pub sql_masked: Arc<str>,
    pub reasons: Vec<GateReason>,
    pub type_to_confirm: Option<Arc<str>>,
    pub irreversible: bool,
    pub suppressible: bool,
    pub warn_only: bool,
    pub production: bool,
    pub connection: Arc<str>,
}

/// Real path: `hacivat_exec::ConfirmAnswer` (Task 1c, `gate.rs`). Any close is `Cancel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConfirmAnswer {
    Execute,
    ExecuteAll,
    ExecuteAndSuppress,
    Skip,
    SkipAll,
    Cancel,
}

/// Real path: `hacivat_exec::CommitQuestion` (Task 1c, `gate.rs`).
#[derive(Debug, Clone)]
pub struct CommitQuestion {
    pub run: RunId,
    pub unit: UnitId,
    pub affected: Option<u64>,
    pub deadline: Instant,
    pub savepoint: bool,
    pub production: bool,
    pub connection: Arc<str>,
}

/// Real path: `hacivat_session::session::CommitAnswer` (Task 1c, `session/mod.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommitAnswer {
    Commit,
    RollBack,
}

/// Real path: `hacivat_session::session::RunChoice` (Task 1c, `session/mod.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunChoice {
    Retry,
    Ignore,
    IgnoreAll,
    Stop,
    RollbackAndContinue,
}

/// Real path: `hacivat_exec::ParamPrompt` (Task 1c, `params.rs`).
#[derive(Debug, Clone)]
pub struct ParamPrompt {
    pub name: Arc<str>,
    pub occurrences: u32,
    pub remembered: Option<BindValue>,
}

/// Real path: `hacivat_exec::ConfirmPort` (Task 1c, `gate.rs`).
pub trait ConfirmPort: Send + Sync + std::fmt::Debug + 'static {
    fn confirm(&self, q: ConfirmQuestion) -> oneshot::Receiver<ConfirmAnswer>;
    fn commit_decision(&self, q: CommitQuestion) -> oneshot::Receiver<CommitAnswer>;
}
