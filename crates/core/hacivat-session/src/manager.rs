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
use tokio_util::sync::CancellationToken;

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

/// A running test connection (HV-CONN-07). Dropping it cancels the test.
#[derive(Debug)]
pub struct TestHandle {
    pub progress: watch::Receiver<Progress>,
    pub result: oneshot::Receiver<TestReport>,
    cancel: CancellationToken,
}

impl TestHandle {
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

impl Drop for TestHandle {
    fn drop(&mut self) {
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

    /// Starts a test of `draft` (Task 16). The stub's result never arrives.
    pub fn test(&self, _draft: ProfileDraft, _ix: Interaction) -> TestHandle {
        let (_progress_tx, progress) = watch::channel(fresh_progress());
        let (_result_tx, result) = oneshot::channel();
        TestHandle {
            progress,
            result,
            cancel: CancellationToken::new(),
        }
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
