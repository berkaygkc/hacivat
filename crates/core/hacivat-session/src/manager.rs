//! The `ConnectionManager`: one live state per profile, connect and test on the core runtime,
//! liveness pings and a bounded shutdown (HV-CONN-01, CONN-04 subset, P15). Bodies in Task 16.

use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_driver::{ServerInfo, Support};
use hacivat_net::{AddressClass, TlsSummary};
use hacivat_proto::{AccessLabel, ErrClass, ProfileId};
use hacivat_secrets::AuthProviderId;
use secrecy::SecretString;
use tokio::runtime::Handle;
use tokio::sync::{oneshot, watch};
use tokio_util::sync::{CancellationToken, DropGuard};

use crate::pipeline::{ConnectDeps, ConnectError, Interaction, Progress, fresh_progress};
use crate::profile::{Profile, ProfileDraft};
use crate::test_conn::TestReport;

/// How the bytes travel, as the capsule shows it (HV-CONN-01). Only `Direct` in M2a.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathKind {
    Direct,
}

/// A connected profile, for the capsule and the status bar (HV-CONN-01).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LiveInfo {
    pub server: ServerInfo,
    pub support: Support,
    pub tls: TlsSummary,
    pub address: AddressClass,
    pub latency: Duration,
    pub access: AccessLabel,
    pub path: PathKind,
    pub user: Option<String>,
    pub auth: AuthProviderId,
}

/// A profile's connection state (HV-CONN-01).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum LiveState {
    #[default]
    Disconnected,
    Connecting(Progress),
    Connected(Arc<LiveInfo>),
    Failed(Arc<ConnectError>),
    /// The liveness ping failed (CONN-04 subset; no reconnect in M2a).
    Lost {
        class: ErrClass,
    },
}

/// Manager timings (CONN-04, P15).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ManagerConfig {
    /// How often a live connection is pinged.
    pub liveness: Duration,
    /// How long quitting waits for connections to close.
    pub shutdown_budget: Duration,
}

impl Default for ManagerConfig {
    /// 30 s liveness, 2 s shutdown budget.
    fn default() -> Self {
        ManagerConfig {
            liveness: Duration::from_secs(30),
            shutdown_budget: Duration::from_secs(2),
        }
    }
}

/// The stub's senders, kept alive with the handle so its result never arrives (Task 16 drops
/// this field).
type StubSenders = (watch::Sender<Progress>, oneshot::Sender<TestReport>);

/// A running test connection (HV-CONN-07). Dropping it cancels the test. It has no `Drop` impl
/// of its own (a guard field cancels), so `progress` and `result` can be moved out of it
/// (`m.test(..).result.await`); the test is cancelled when the rest of the handle drops.
#[derive(Debug)]
pub struct TestHandle {
    pub progress: watch::Receiver<Progress>,
    pub result: oneshot::Receiver<TestReport>,
    cancel: CancellationToken,
    _guard: DropGuard,
    #[allow(dead_code, reason = "Task 16")]
    stub_senders: Option<StubSenders>,
}

impl TestHandle {
    /// A handle whose result never arrives while it lives (the Task 1 stub).
    fn pending(cancel: CancellationToken) -> TestHandle {
        let (progress_tx, progress) = watch::channel(fresh_progress());
        let (result_tx, result) = oneshot::channel();
        TestHandle {
            progress,
            result,
            _guard: cancel.clone().drop_guard(),
            cancel,
            stub_senders: Some((progress_tx, result_tx)),
        }
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

/// Every profile's connection, on the core runtime (HV-CONN-01).
pub struct ConnectionManager {
    #[allow(dead_code, reason = "Task 16")]
    rt: Handle,
    #[allow(dead_code, reason = "Task 16")]
    deps: Arc<ConnectDeps>,
    #[allow(dead_code, reason = "Task 16")]
    config: ManagerConfig,
    changes: watch::Sender<u64>,
}

impl ConnectionManager {
    pub fn new(rt: Handle, deps: Arc<ConnectDeps>, config: ManagerConfig) -> Arc<Self> {
        Arc::new(ConnectionManager {
            rt,
            deps,
            config,
            changes: watch::Sender::new(0),
        })
    }

    /// Starts connecting `profile` (Task 16).
    pub fn connect(
        &self,
        _profile: Arc<Profile>,
        _draft_password: Option<SecretString>,
        _ix: Interaction,
    ) {
    }

    /// Cancels a connect in progress (Task 16).
    pub fn cancel(&self, _id: ProfileId) {}

    pub fn disconnect(&self, _id: ProfileId) {}

    /// Cancels, disconnects and forgets the profile's session secrets (Task 16).
    pub fn forget(&self, _id: ProfileId) {}

    pub fn state(&self, _id: ProfileId) -> LiveState {
        LiveState::Disconnected
    }

    /// A generation counter, bumped on every state change.
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// Starts a test of `draft` (Task 16). The stub's result never arrives while the handle
    /// lives (its senders stay in the handle).
    pub fn test(&self, _draft: ProfileDraft, _ix: Interaction) -> TestHandle {
        TestHandle::pending(CancellationToken::new())
    }

    /// Closes every connection (Task 16).
    pub fn shutdown(&self) -> BoxFuture<'static, ()> {
        Box::pin(async {})
    }

    /// Sends a close request to every connection and blocks on a `std::sync::mpsc` reply for
    /// at most `budget`; `true` if everything closed in time (P15). The quit handler uses it.
    pub fn shutdown_blocking(&self, _budget: Duration) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot::error::TryRecvError;

    #[test]
    fn result_moves_out_and_the_rest_of_the_handle_cancels_on_drop() {
        let token = CancellationToken::new();
        let mut result = {
            let handle = TestHandle::pending(token.clone());
            let mut result = handle.result; // a partial move: `TestHandle` has no `Drop`
            assert!(!token.is_cancelled());
            // The stub's sender lives in the handle: nothing arrives, nothing is closed.
            assert!(matches!(result.try_recv(), Err(TryRecvError::Empty)));
            result
        }; // the rest of the handle drops here
        assert!(token.is_cancelled());
        assert!(matches!(result.try_recv(), Err(TryRecvError::Closed)));
    }

    #[test]
    fn dropping_or_cancelling_a_handle_cancels_the_test() {
        let token = CancellationToken::new();
        drop(TestHandle::pending(token.clone()));
        assert!(token.is_cancelled());

        let token = CancellationToken::new();
        let handle = TestHandle::pending(token.clone());
        handle.cancel();
        assert!(token.is_cancelled());
        drop(handle);
    }
}
