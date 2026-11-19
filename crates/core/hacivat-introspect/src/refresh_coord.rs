// ## m4-t01c-core
//! The refresh-after-DDL coordinator (HV-INTRO-12; Task 21): per profile, applies a
//! [`RefreshPlan`] with the per-engine timing, keeps pending targets per session until
//! `COMMIT`/`ROLLBACK` (R-M4-42; exposed for M5's overlays, R-M4-11) and honours
//! `metadata.refresh_after_ddl`.
//!
//! **Fail-closed stub (M4 Task 1c).** It keeps nothing and refreshes nothing: every request
//! answers `IntroError::NotImplemented` (the explorer stays as loaded; a user refresh still
//! reloads), and no session has pending targets.

use hacivat_session::session::{SessionId, TxOutcome};

use crate::ddl_effects::{RefreshPlan, Target};
use crate::ticket::RefreshTicket;

/// One profile's coordinator.
#[derive(Debug, Default)]
pub struct RefreshCoordinator {
    _private: (),
}

impl RefreshCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes the plan of a run on `session`. Stub: `NotImplemented`.
    pub fn submit(&self, session: SessionId, plan: RefreshPlan) -> RefreshTicket {
        let _ = (session, plan);
        RefreshTicket::not_implemented()
    }

    /// `session`'s transaction ended: its pending targets are refreshed
    /// (`TxOutcome::counts_as_committed`) or dropped. Stub: nothing is pending.
    pub fn tx_ended(&self, session: SessionId, outcome: TxOutcome) {
        let _ = (session, outcome);
    }

    /// The targets waiting for `session`'s `COMMIT`. Stub: none.
    pub fn pending(&self, session: SessionId) -> Vec<Target> {
        let _ = session;
        Vec::new()
    }

    /// Refreshes `targets` now, whatever the preference (M7's Modify Object). Stub:
    /// `NotImplemented`.
    pub fn refresh_now(&self, targets: Vec<Target>) -> RefreshTicket {
        let _ = targets;
        RefreshTicket::not_implemented()
    }
}
