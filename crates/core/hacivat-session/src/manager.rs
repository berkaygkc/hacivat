//! The `ConnectionManager`: one live state per profile, connect and test on the core runtime,
//! liveness pings and a bounded shutdown (HV-CONN-01, CONN-04 subset, P15, R12, R13).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_driver::{ServerInfo, Support, classify};
use hacivat_guard::{GapReason, RoleWarning};
use hacivat_net::{AddressClass, TlsSummary};
use hacivat_proto::{AccessLabel, Department, ErrClass, ProfileId};
use hacivat_secrets::AuthProviderId;
use secrecy::SecretString;
use tokio::runtime::Handle;
use tokio::sync::{broadcast, oneshot, watch};
use tokio_util::sync::{CancellationToken, DropGuard};

use crate::budget::{Budget, BudgetPermit, Priority, budget_for};
use crate::pipeline::{
    ConnectDeps, ConnectError, ConnectRequest, Interaction, LiveConnection, Phase, ProfileChange,
    Progress, Purpose, TrailStep, connect, fresh_progress,
};
use crate::profile::{Profile, ProfileDraft};
use crate::session::{LostState, SessionManager};
use crate::test_conn::{TestReport, test_connection};

/// How the bytes travel, as the capsule shows it (HV-CONN-01). Only `Direct` in M2a.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathKind {
    Direct,
    // ## m2b-t01-interfaces
    Ssh,
}

// ## m2b-t01-interfaces
/// What the manager reports besides state changes (Task 17; the app's `events.rs` relays them).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConnectionEvent {
    SessionLost {
        profile: ProfileId,
        lost: LostState,
    },
    IdleDisconnected {
        profile: ProfileId,
    },
    IdleTransaction {
        profile: ProfileId,
        minutes: u32,
    },
    /// The guard saw a statement escape it (the copy depends on `reason`, Q35).
    EnforcementGap {
        profile: ProfileId,
        reason: GapReason,
    },
    /// A connect changed the profile (a pin, a CA file, the plaintext consent): the app saves
    /// them through `ConnectionsModel::apply_changes`.
    ProfileChanges {
        profile: ProfileId,
        changes: Vec<ProfileChange>,
    },
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
    // ## m2b-t01-interfaces
    /// The bastion of an SSH path.
    pub via: Option<String>,
    /// SSH hops (0 for a direct path).
    pub hops: u8,
    pub access_warnings: Vec<RoleWarning>,
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
    // ## m2b-t01-interfaces
    /// Reconnecting after a loss (Task 17): `attempt` from 1, `next_in` until the next try.
    Reconnecting {
        attempt: u32,
        next_in: Option<Duration>,
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

// ## m2b-t01-interfaces
impl ManagerConfig {
    /// M2b's configuration: liveness per profile from `LivenessPolicy::for_profile` (B18).
    /// Task 1 stub: M2a's (Task 17).
    pub fn m2b() -> ManagerConfig {
        ManagerConfig::default()
    }
}

/// A running test connection (HV-CONN-07). Dropping it cancels the test. It has no `Drop` impl
/// of its own (a guard field cancels), so `progress` and `result` can be moved out of it
/// (`m.test(..).result.await`); the test is cancelled when the rest of the handle drops.
#[derive(Debug)]
pub struct TestHandle {
    pub progress: watch::Receiver<Progress>,
    pub result: oneshot::Receiver<TestReport>,
    cancel: CancellationToken,
    _guard: DropGuard,
}

impl TestHandle {
    fn new(
        progress: watch::Receiver<Progress>,
        result: oneshot::Receiver<TestReport>,
        cancel: CancellationToken,
    ) -> TestHandle {
        TestHandle {
            progress,
            result,
            _guard: cancel.clone().drop_guard(),
            cancel,
        }
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

/// The longest a liveness ping may take before it counts as a timeout.
const PING_BUDGET: Duration = Duration::from_secs(10);

/// A connection the manager holds: the pipeline's connection (shared with its liveness
/// task), the budget permit it took, and the switch that stops its pings.
struct Live {
    conn: Arc<tokio::sync::Mutex<Option<LiveConnection>>>,
    permit: BudgetPermit,
    stop: CancellationToken,
}

impl Live {
    /// Stops the pings, closes the connection and then returns the permit.
    async fn close(self) {
        self.stop.cancel();
        let conn = self.conn.lock().await.take();
        if let Some(conn) = conn {
            conn.close().await;
        }
        drop(self.permit);
    }
}

/// One profile's slot.
struct Entry {
    state: LiveState,
    /// The connect in flight: its id and cancel token (single-flight).
    attempt: Option<(u64, CancellationToken)>,
    live: Option<Live>,
}

struct Inner {
    entries: HashMap<ProfileId, Entry>,
    /// One budget per profile, kept across `forget` and a changed `max_connections`, so a
    /// connection still closing holds its permit and the next connect waits for it (a second
    /// SQLite connection never opens before the first has closed).
    budgets: HashMap<ProfileId, Arc<Budget>>,
    shutting_down: bool,
    generation: u64,
    next_attempt: u64,
}

struct Shared {
    rt: Handle,
    deps: Arc<ConnectDeps>,
    config: ManagerConfig,
    changes: watch::Sender<u64>,
    inner: Mutex<Inner>,
    // ## m2b-t01-interfaces
    events: broadcast::Sender<ConnectionEvent>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Sets `id`'s state and bumps the generation.
    fn set(&self, inner: &mut Inner, id: ProfileId, state: LiveState) {
        if let Some(entry) = inner.entries.get_mut(&id) {
            entry.state = state;
        }
        inner.generation += 1;
        self.changes.send_replace(inner.generation);
    }

    /// Whether `attempt` is still `id`'s connect in flight.
    fn current(inner: &Inner, id: ProfileId, attempt: u64) -> bool {
        !inner.shutting_down
            && inner
                .entries
                .get(&id)
                .and_then(|e| e.attempt.as_ref())
                .is_some_and(|(a, _)| *a == attempt)
    }

    fn progress(&self, id: ProfileId, attempt: u64, progress: Progress) {
        let mut inner = self.lock();
        if Self::current(&inner, id, attempt) {
            self.set(&mut inner, id, LiveState::Connecting(progress));
        }
    }

    fn failed(&self, id: ProfileId, attempt: u64, e: ConnectError) {
        let mut inner = self.lock();
        if Self::current(&inner, id, attempt) {
            if let Some(entry) = inner.entries.get_mut(&id) {
                entry.attempt = None;
            }
            self.set(&mut inner, id, LiveState::Failed(Arc::new(e)));
        }
    }

    /// A connection arrived: it becomes `id`'s live connection, or is closed at once when the
    /// attempt was cancelled, forgotten or overtaken by shutdown.
    fn landed(
        self: &Arc<Self>,
        id: ProfileId,
        attempt: u64,
        live: LiveConnection,
        permit: BudgetPermit,
    ) {
        let mut inner = self.lock();
        if !Self::current(&inner, id, attempt) {
            drop(inner);
            self.rt.spawn(async move {
                live.close().await;
                drop(permit);
            });
            return;
        }
        let info = LiveInfo {
            server: live.server.clone(),
            support: live.support.clone(),
            tls: live.tls.clone(),
            address: live.address,
            // The post-connect round trips (server and user checks).
            latency: live
                .trail
                .iter()
                .find(|t| t.step == TrailStep::CheckServer)
                .and_then(|t| t.elapsed)
                .unwrap_or_default(),
            access: live.access,
            path: live.path,
            user: live.user.clone(),
            auth: live.auth,
            via: live.via.clone(),
            hops: 0,
            access_warnings: live.access_warnings.clone(),
        };
        let kind = live.server.kind;
        let stop = CancellationToken::new();
        let conn = Arc::new(tokio::sync::Mutex::new(Some(live)));
        if let Some(entry) = inner.entries.get_mut(&id) {
            entry.attempt = None;
            entry.live = Some(Live {
                conn: conn.clone(),
                permit,
                stop: stop.clone(),
            });
        }
        self.set(&mut inner, id, LiveState::Connected(Arc::new(info)));
        drop(inner);
        let shared = self.clone();
        self.rt
            .spawn(async move { shared.liveness(id, kind, conn, stop).await });
    }

    /// Pings `conn` every `config.liveness`; the first failure marks the profile `Lost` and
    /// closes the connection (CONN-04 subset, R12: no reconnect in M2a).
    async fn liveness(
        self: Arc<Self>,
        id: ProfileId,
        kind: hacivat_proto::DbmsKind,
        conn: Arc<tokio::sync::Mutex<Option<LiveConnection>>>,
        stop: CancellationToken,
    ) {
        loop {
            // `biased`: a stop wins over an interval that is due at the same time (a busy
            // runtime polls both ready at once), so no ping starts after a close or a drop.
            tokio::select! {
                biased;
                () = stop.cancelled() => return,
                () = tokio::time::sleep(self.config.liveness) => {}
            }
            let pinged = {
                let mut guard = conn.lock().await;
                let Some(live) = guard.as_mut() else { return };
                // The stop may have come while this task waited for the lock.
                if stop.is_cancelled() {
                    return;
                }
                // A close or shutdown never waits for a ping: it stops the ping, which frees
                // the connection.
                tokio::select! {
                    biased;
                    () = stop.cancelled() => return,
                    pinged = tokio::time::timeout(PING_BUDGET, live.raw().ping()) => pinged,
                }
            };
            let class = match pinged {
                Ok(Ok(_)) => continue,
                Ok(Err(raw)) => classify(kind, Phase::PostConnect, &raw).class,
                Err(_) => ErrClass::NetworkTimeout,
            };
            if stop.is_cancelled() {
                return;
            }
            let lost = {
                let mut inner = self.lock();
                let mine = inner
                    .entries
                    .get(&id)
                    .and_then(|e| e.live.as_ref())
                    .is_some_and(|l| Arc::ptr_eq(&l.conn, &conn));
                if !mine {
                    return;
                }
                let lost = inner.entries.get_mut(&id).and_then(|e| e.live.take());
                self.set(&mut inner, id, LiveState::Lost { class });
                lost
            };
            tracing::debug!(profile = %id, class = class.id(), "liveness ping failed");
            if let Some(live) = lost {
                live.close().await;
            }
            return;
        }
    }

    /// Runs one connect: a Console permit, then the pipeline, mirroring its progress.
    #[allow(clippy::too_many_arguments, reason = "one private call site")]
    async fn run_connect(
        self: Arc<Self>,
        profile: Arc<Profile>,
        draft: Option<SecretString>,
        prior_rejections: u8,
        ix: Interaction,
        attempt: u64,
        token: CancellationToken,
        budget: Arc<Budget>,
    ) {
        let id = profile.id;
        let permit = tokio::select! {
            () = token.cancelled() => return,
            permit = budget.acquire(Priority::Console) => permit,
        };
        let (progress_tx, mut progress_rx) = watch::channel(fresh_progress());
        let req = ConnectRequest {
            read_only: profile.read_only,
            profile,
            draft_password: draft,
            purpose: Purpose::Main,
            department: Department::Console,
            prior_rejections,
            draft_ssh: Vec::new(),
            overlay_secrets: None,
            plaintext_consent: false,
        };
        let deps = self.deps.clone();
        let pipeline = connect(&deps, req, ix, token, progress_tx);
        tokio::pin!(pipeline);
        let result = loop {
            tokio::select! {
                result = &mut pipeline => break result,
                Ok(()) = progress_rx.changed() => {
                    let p = progress_rx.borrow_and_update().clone();
                    self.progress(id, attempt, p);
                }
            }
        };
        match result {
            Ok(live) => self.landed(id, attempt, live, permit),
            Err(e) => self.failed(id, attempt, e),
        }
    }

    /// Takes every connection and cancels every connect; the returned future closes them
    /// within `budget` and says whether all closed in time.
    fn shutdown_within(&self, budget: Duration) -> BoxFuture<'static, bool> {
        let mut inner = self.lock();
        inner.shutting_down = true;
        let mut lives = Vec::new();
        let ids: Vec<ProfileId> = inner.entries.keys().copied().collect();
        for id in ids {
            if let Some(entry) = inner.entries.get_mut(&id) {
                if let Some((_, token)) = entry.attempt.take() {
                    token.cancel();
                }
                lives.extend(entry.live.take());
            }
            self.set(&mut inner, id, LiveState::Disconnected);
        }
        drop(inner);
        Box::pin(async move {
            let closes = futures::future::join_all(lives.into_iter().map(Live::close));
            tokio::time::timeout(budget, closes).await.is_ok()
        })
    }
}

/// Every profile's connection, on the core runtime (HV-CONN-01).
pub struct ConnectionManager {
    shared: Arc<Shared>,
}

impl Drop for ConnectionManager {
    /// Stops every liveness task, closes every open connection (in the background) and takes
    /// every connect in flight; their tasks hold the shared state, so without this they would
    /// keep it (and the connections) alive. Marked as shutting down, so a connect that lands
    /// afterwards is closed at once and never starts a liveness task.
    fn drop(&mut self) {
        let mut inner = self.shared.lock();
        inner.shutting_down = true;
        let mut lives = Vec::new();
        for entry in inner.entries.values_mut() {
            if let Some((_, token)) = entry.attempt.take() {
                token.cancel();
            }
            lives.extend(entry.live.take());
        }
        drop(inner);
        // `Drop` cannot wait: the open connections are closed on the core runtime.
        for live in lives {
            live.stop.cancel();
            self.shared.rt.spawn(live.close());
        }
    }
}

impl ConnectionManager {
    pub fn new(rt: Handle, deps: Arc<ConnectDeps>, config: ManagerConfig) -> Arc<Self> {
        Arc::new(ConnectionManager {
            shared: Arc::new(Shared {
                rt,
                deps,
                config,
                changes: watch::Sender::new(0),
                inner: Mutex::new(Inner {
                    entries: HashMap::new(),
                    budgets: HashMap::new(),
                    shutting_down: false,
                    generation: 0,
                    next_attempt: 0,
                }),
                events: broadcast::Sender::new(64),
            }),
        })
    }

    // ## m2b-t01-interfaces

    /// The manager's events (Task 17 sends them; the app relays them, Q42).
    pub fn events(&self) -> broadcast::Receiver<ConnectionEvent> {
        self.shared.events.subscribe()
    }

    /// Sends `event` to every `events()` receiver (the senders inside the manager, and tests).
    #[doc(hidden)]
    pub fn publish_event(&self, event: ConnectionEvent) {
        // No receiver is not an error: nobody listens yet.
        let _ = self.shared.events.send(event);
    }

    /// How many `events()` receivers are open.
    #[doc(hidden)]
    pub fn event_subscribers(&self) -> usize {
        self.shared.events.receiver_count()
    }

    /// The sessions of a connected profile (Task 15, Task 17). Task 1 stub: none.
    pub fn sessions(&self, _id: ProfileId) -> Option<Arc<SessionManager>> {
        None
    }

    /// Gives a connected profile's sessions the saved `profile` (Q17): the app calls it after
    /// every save and `apply_changes`. Task 1 stub: nothing holds a profile yet (Task 17).
    pub fn update_profile(&self, id: ProfileId, profile: Arc<Profile>) {
        if let Some(sessions) = self.sessions(id) {
            sessions.update_profile(profile);
        }
    }

    /// Starts connecting `profile` on the core runtime. Single-flight: while a connect for the
    /// profile is in flight, or it is connected, this does nothing. The connect waits for one
    /// `Console` permit of the profile's budget (R13).
    pub fn connect(
        &self,
        profile: Arc<Profile>,
        draft_password: Option<SecretString>,
        ix: Interaction,
    ) {
        self.start(profile, draft_password, 0, ix);
    }

    /// The error dialog's password re-entry (R8, P8): connects with `password` as the draft,
    /// continuing the failed connect's count of server rejections (`ConnectError::attempts`),
    /// so one gesture never gets more than 3. The password is stored only if it works.
    pub fn connect_reentry(
        &self,
        profile: Arc<Profile>,
        password: SecretString,
        prior_rejections: u8,
        ix: Interaction,
    ) {
        self.start(profile, Some(password), prior_rejections, ix);
    }

    fn start(
        &self,
        profile: Arc<Profile>,
        draft_password: Option<SecretString>,
        prior_rejections: u8,
        ix: Interaction,
    ) {
        let shared = &self.shared;
        let mut inner = shared.lock();
        if inner.shutting_down {
            return;
        }
        let attempt = inner.next_attempt;
        inner.next_attempt += 1;
        let entry = inner.entries.entry(profile.id).or_insert_with(|| Entry {
            state: LiveState::Disconnected,
            attempt: None,
            live: None,
        });
        if entry.attempt.is_some() || entry.live.is_some() {
            return;
        }
        let token = CancellationToken::new();
        entry.attempt = Some((attempt, token.clone()));
        let wanted = budget_for(&profile);
        let budget = inner
            .budgets
            .entry(profile.id)
            .or_insert_with(|| Arc::new(budget_for(&profile)));
        // A new size applies once the old budget is idle (nothing open or closing).
        if budget.max() != wanted.max() && budget.in_use() == 0 {
            *budget = Arc::new(wanted);
        }
        let budget = budget.clone();
        shared.set(
            &mut inner,
            profile.id,
            LiveState::Connecting(fresh_progress()),
        );
        drop(inner);
        let task = shared.clone().run_connect(
            profile,
            draft_password,
            prior_rejections,
            ix,
            attempt,
            token,
            budget,
        );
        shared.rt.spawn(task);
    }

    /// Cancels a connect in progress; the profile is `Disconnected` at once, and a connection
    /// that still arrives is closed.
    pub fn cancel(&self, id: ProfileId) {
        let shared = &self.shared;
        let mut inner = shared.lock();
        let Some(entry) = inner.entries.get_mut(&id) else {
            return;
        };
        let Some((_, token)) = entry.attempt.take() else {
            return;
        };
        token.cancel();
        shared.set(&mut inner, id, LiveState::Disconnected);
    }

    /// Closes the profile's connection (cancelling a connect in progress) and forgets its
    /// `ForSession` secrets.
    pub fn disconnect(&self, id: ProfileId) {
        self.close(id, false);
    }

    /// Cancels, disconnects and forgets the profile's session secrets, then drops its slot.
    pub fn forget(&self, id: ProfileId) {
        self.close(id, true);
    }

    fn close(&self, id: ProfileId, remove: bool) {
        let shared = &self.shared;
        let mut inner = shared.lock();
        let live = match inner.entries.get_mut(&id) {
            Some(entry) => {
                if let Some((_, token)) = entry.attempt.take() {
                    token.cancel();
                }
                entry.live.take()
            }
            None => None,
        };
        shared.set(&mut inner, id, LiveState::Disconnected);
        if remove {
            inner.entries.remove(&id);
        }
        drop(inner);
        // In-process maps only: no keyring call.
        shared.deps.secrets.forget_session(id);
        if let Some(live) = live {
            shared.rt.spawn(live.close());
        }
    }

    pub fn state(&self, id: ProfileId) -> LiveState {
        self.shared
            .lock()
            .entries
            .get(&id)
            .map(|e| e.state.clone())
            .unwrap_or_default()
    }

    /// A generation counter, bumped on every state change.
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.shared.changes.subscribe()
    }

    /// Starts a test of `draft` on the core runtime (HV-CONN-07): fresh resources, no budget
    /// permit, nothing stored. Dropping the handle (or `cancel`) stops it.
    pub fn test(&self, draft: ProfileDraft, ix: Interaction) -> TestHandle {
        let token = CancellationToken::new();
        let (progress_tx, progress) = watch::channel(fresh_progress());
        let (result_tx, result) = oneshot::channel();
        let deps = self.shared.deps.clone();
        let cancel = token.clone();
        self.shared.rt.spawn(async move {
            let report = test_connection(&deps, draft, ix, cancel, progress_tx).await;
            let _ = result_tx.send(report);
        });
        TestHandle::new(progress, result, token)
    }

    /// How long quitting may wait for connections to close (`ManagerConfig::shutdown_budget`;
    /// M2a Task 18's quit handler).
    pub fn shutdown_budget(&self) -> Duration {
        self.shared.config.shutdown_budget
    }

    /// Closes every connection within `config.shutdown_budget`. A connect still in flight is
    /// cancelled, and a connection that lands later is closed at once.
    pub fn shutdown(&self) -> BoxFuture<'static, ()> {
        let closing = self
            .shared
            .shutdown_within(self.shared.config.shutdown_budget);
        Box::pin(async move {
            closing.await;
        })
    }

    /// Sends a close request to every connection and blocks on a `std::sync::mpsc` reply for
    /// at most `budget`; `true` if everything closed in time (P15). The quit handler uses it,
    /// from a thread outside the core runtime.
    pub fn shutdown_blocking(&self, budget: Duration) -> bool {
        let (tx, rx) = std::sync::mpsc::channel();
        let closing = self.shared.shutdown_within(budget);
        self.shared.rt.spawn(async move {
            let _ = tx.send(closing.await);
        });
        rx.recv_timeout(budget).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot::error::TryRecvError;

    fn handle(token: &CancellationToken) -> (TestHandle, oneshot::Sender<TestReport>) {
        let (_progress_tx, progress) = watch::channel(fresh_progress());
        let (result_tx, result) = oneshot::channel();
        (TestHandle::new(progress, result, token.clone()), result_tx)
    }

    #[test]
    fn result_moves_out_and_the_rest_of_the_handle_cancels_on_drop() {
        let token = CancellationToken::new();
        let (mut result, sender) = {
            let (h, sender) = handle(&token);
            let mut result = h.result; // a partial move: `TestHandle` has no `Drop`
            assert!(!token.is_cancelled());
            assert!(matches!(result.try_recv(), Err(TryRecvError::Empty)));
            (result, sender)
        }; // the rest of the handle drops here
        assert!(token.is_cancelled());
        drop(sender);
        assert!(matches!(result.try_recv(), Err(TryRecvError::Closed)));
    }

    #[test]
    fn dropping_or_cancelling_a_handle_cancels_the_test() {
        let token = CancellationToken::new();
        drop(handle(&token));
        assert!(token.is_cancelled());

        let token = CancellationToken::new();
        let (h, _sender) = handle(&token);
        h.cancel();
        assert!(token.is_cancelled());
    }
}
