// ## m3b-t01c-core-seams
//! The exec gate (HV-EXEC-04, R-M3-55, R-M3-84; Task 15): what Run does with each prepared
//! unit given its classification and the connection: run it, warn, ask for a confirmation
//! (type-to-confirm for the destructive kinds), run it in a transaction and then ask
//! (confirm-in-transaction; the session picks the form, P-M3b-1), wrap an `EXPLAIN ANALYZE`
//! (D-062), or deny it. Pure and synchronous: it never talks to a session.
//!
//! STUB until Task 15 (fail closed, F31): every unit is denied, a unit whose report does not
//! match its final text with `exec-gate-stale-classification` (P-M3b-2), every other with
//! `exec-gate-not-implemented`. The app calls the gate only from Task 24 on (R-M3-52).

use std::sync::Arc;
use std::time::Instant;

use hacivat_driver::TxStatus;
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_session::session::{
    CommitAnswer, RunId, SessionEffects, TxMode, TxOwner, UnitId, UnitVerdict,
};
use sql_safety::{GateClass, SafetyReport};
use tokio::sync::oneshot;

use crate::params::PreparedUnit;
use crate::suppress::SuppressSet;

/// What the gate knows of the connection a run goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnSafety {
    pub engine: DbmsKind,
    pub env: Option<EnvTag>,
    pub read_only: bool,
    pub tx_status: TxStatus,
    pub tx_owner: TxOwner,
    pub tx_mode: TxMode,
    /// `exec.confirm_unsafe_where` (off production).
    pub confirm_unsafe_where: bool,
}

/// Why the gate decided as it did: a Fluent ID and its arguments (server text never).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GateReason {
    pub fluent_id: &'static str,
    pub args: Vec<(&'static str, Arc<str>)>,
}

impl GateReason {
    /// A reason without arguments.
    pub fn of(fluent_id: &'static str) -> GateReason {
        GateReason {
            fluent_id,
            args: Vec::new(),
        }
    }
}

/// What Run does with one unit.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GateDecision {
    Run,
    /// A dialog with Cancel focused (`Lock` on production).
    Warn(Vec<GateReason>),
    Confirm {
        reasons: Vec<GateReason>,
        /// The exact bytes the user types (no case folding), when asked.
        type_to_confirm: Option<Arc<str>>,
        /// MySQL/MariaDB DDL: "cannot be rolled back".
        irreversible: bool,
        /// "Execute and don't ask again" is offered (never with type-to-confirm).
        suppressible: bool,
    },
    /// Run in a transaction, then ask Commit/Roll back (the session picks the form; P-M3b-1).
    RunInTxThenConfirm {
        reasons: Vec<GateReason>,
    },
    /// D-062.
    ExplainAnalyzeRollback,
    Deny(GateReason),
}

/// The gate's answer for one unit.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GateItem {
    /// The unit's index in the run.
    pub unit: u32,
    pub decision: GateDecision,
    /// What the session is told (`RunUnit::verdict`).
    pub verdict: UnitVerdict,
    pub effects: SessionEffects,
    pub class: GateClass,
    /// The classified text's fingerprint (`SuppressSet` keys on it).
    pub fingerprint: u64,
}

/// The gate's answer for a run.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct GatePlan {
    pub items: Vec<GateItem>,
}

impl GatePlan {
    /// Whether any unit needs a dialog before the run is sent (`Confirm` or `Warn`; R-M3-84).
    pub fn needs_prompt(&self) -> bool {
        self.items.iter().any(|i| {
            matches!(
                i.decision,
                GateDecision::Confirm { .. } | GateDecision::Warn(_)
            )
        })
    }

    /// The first denied unit, if any (a run with one sends nothing).
    pub fn first_denied(&self) -> Option<&GateItem> {
        self.items
            .iter()
            .find(|i| matches!(i.decision, GateDecision::Deny(_)))
    }
}

/// The gate (the module docs): one item per unit, `reports[i]` classifying
/// `units[i].final_text`; `suppressed` holds the console's "don't ask again" pairs.
///
/// STUB until Task 15: every unit is denied.
pub fn gate(
    units: &[PreparedUnit],
    reports: &[SafetyReport],
    conn: &ConnSafety,
    suppressed: &SuppressSet,
    console: u64,
) -> GatePlan {
    let _ = (conn, suppressed, console);
    let items = units
        .iter()
        .enumerate()
        .map(|(i, unit)| {
            let report = reports.get(i);
            let reason = match report {
                Some(r) if r.matches_text(&unit.final_text) => "exec-gate-not-implemented",
                _ => "exec-gate-stale-classification",
            };
            GateItem {
                unit: u32::try_from(i).unwrap_or(u32::MAX),
                decision: GateDecision::Deny(GateReason::of(reason)),
                verdict: UnitVerdict::Unknown,
                effects: SessionEffects::empty(),
                class: GateClass::Unknown,
                fingerprint: report.map_or(0, |r| r.fingerprint.hash),
            }
        })
        .collect();
    GatePlan { items }
}

/// Where the app asks the user (Task 21's dialogs), from the run service (R-M3-84).
pub trait ConfirmPort: Send + Sync + std::fmt::Debug + 'static {
    /// A `Confirm` or `Warn` unit, before the run is sent. Any close of the dialog is
    /// `Cancel`.
    fn confirm(&self, q: ConfirmQuestion) -> oneshot::Receiver<ConfirmAnswer>;
    /// A confirm-in-transaction question (`RunEvent::NeedsCommit`). Any close is `RollBack`.
    fn commit_decision(&self, q: CommitQuestion) -> oneshot::Receiver<CommitAnswer>;
}

/// One confirmation ("statement `index` of `total`").
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConfirmQuestion {
    pub run: RunId,
    pub unit: u32,
    pub index: u32,
    pub total: u32,
    /// The statement with its bind values masked.
    pub sql_masked: Arc<str>,
    pub reasons: Vec<GateReason>,
    pub type_to_confirm: Option<Arc<str>>,
    pub irreversible: bool,
    pub suppressible: bool,
    pub warn_only: bool,
    pub production: bool,
    /// The connection's name.
    pub connection: Arc<str>,
}

/// The answer to a [`ConfirmQuestion`]; any close is `Cancel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConfirmAnswer {
    Execute,
    ExecuteAll,
    ExecuteAndSuppress,
    Skip,
    SkipAll,
    Cancel,
}

/// A confirm-in-transaction question, from `RunEvent::NeedsCommit`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CommitQuestion {
    pub run: RunId,
    pub unit: UnitId,
    pub affected: Option<u64>,
    pub deadline: Instant,
    /// The form the session chose (`NeedsCommit::savepoint`): Keep/Undo when true.
    pub savepoint: bool,
    pub production: bool,
    pub connection: Arc<str>,
}
