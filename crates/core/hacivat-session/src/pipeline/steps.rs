//! Connect steps: the ordered list the pipeline runs, each may leave a cleanup guard
//! (HV-CONN-01, P4, P23). Steps and `StepState` in Task 16.

use std::sync::Arc;

use futures::future::BoxFuture;
use hacivat_net::CleanupGuard;
use tokio_util::sync::CancellationToken;

use super::{ConnectError, Phase};
use crate::profile::Profile;

/// What a step works on (HV-CONN-01).
pub struct StepCx {
    pub profile: Arc<Profile>,
    pub cancel: CancellationToken,
    #[allow(dead_code, reason = "Task 16")]
    state: StepState,
}

/// The pipeline's working state between steps (Task 16).
#[allow(dead_code, reason = "Task 16")]
struct StepState {}

/// One connect step, merged into the ordered list by `order` (HV-CONN-01 data structures).
pub trait ConnectStep: Send + Sync {
    fn phase(&self) -> Phase;
    fn order(&self) -> i32;
    fn applies(&self, p: &Profile) -> bool;
    fn apply<'a>(
        &'a self,
        cx: &'a mut StepCx,
    ) -> BoxFuture<'a, Result<Option<CleanupGuard>, ConnectError>>;
}
