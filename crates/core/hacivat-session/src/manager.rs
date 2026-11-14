//! The `ConnectionManager`: one live state per profile, connect and test on the core runtime,
//! liveness pings and a bounded shutdown (HV-CONN-01, CONN-04 subset, P15, R12, R13).
//!
//! M2b (Task 17; HV-CONN-04, -05, D-085, B4, B18–B20):
//! - A connected profile holds an `Arc<SessionManager>` and its default console session (B4):
//!   the manager runs the connect itself (its progress, draft password and rejection count,
//!   M2a) and the `SessionManager` adopts the connection as that session. `Connected` is
//!   published when the session is Idle, which it is from its spawn.
//! - The session's actor runs the liveness supervisor (`LivenessPolicy`: the B18 presets with
//!   `ManagerConfig::m2b`, else `liveness` as every profile's keep-alive, M2a).
//! - With `ManagerConfig::reconnect` (M2b) a lost default session recovers in place: the
//!   [`Recovery`] runs the B19 backoff (`Reconnecting { attempt, next_in }`) with headless
//!   connects over the session overlay (Q17); the session replays its environment, and the
//!   profile is `Connected` again, then `SessionLost { lost }`. A connect that would have to
//!   ask ends it with `Lost` at once; an `auth.*` failure with `Failed`; five minutes with
//!   `Lost`. A disconnect, `forget` or the shutdown ends it at once. Without it (M2a's
//!   `Default`) a loss is `Lost` (R12).
//! - Events: the idle disconnect (`Disconnected` + `IdleDisconnected`), the idle transaction,
//!   the guard's gaps, each connect's profile changes (success or failure).
//! - Fix round 1: an edit saved while the first connect runs is kept (the sessions start from
//!   the saved profile, and a moved destination is connected anew, I2); only an idle session
//!   reconnects, and a loss inside a transaction ends `Lost` then `SessionLost` (C1); a failed
//!   replay continues the backoff (I1).
//! - B20: a dead SSH chain makes every default session over its generation check its
//!   connection; their reconnects take the generation's lock one at a time, so the chain comes
//!   back once and the others share it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_driver::{ServerInfo, Support};
use hacivat_guard::{GapReason, RoleWarning};
use hacivat_net::ssh::{TransportGeneration, TunnelEvent};
use hacivat_net::{AddressClass, TlsSummary};
use hacivat_proto::{AccessLabel, Department, ErrClass, ErrFamily, ProfileId};
use hacivat_secrets::AuthProviderId;
use secrecy::SecretString;
use tokio::runtime::Handle;
use tokio::sync::{broadcast, oneshot, watch};
use tokio_util::sync::{CancellationToken, DropGuard};

use crate::budget::{Budget, BudgetPermit, Priority, budget_for};
use crate::pipeline::{
    ConnectDeps, ConnectError, ConnectRequest, Interaction, LiveConnection, Phase, ProfileChange,
    Progress, Purpose, TrailStep, connect, error, fresh_progress,
};
use crate::profile::{Profile, ProfileDraft, TransportSpec};
use crate::session::{
    LivenessPolicy, LostState, Reconnector, RecoveryPolicy, Request, SessionEvent, SessionHandle,
    SessionManager, SessionSnapshot, SessionState, is_recovery_case,
};
use crate::test_conn::TestReport;
// ## m3a-t01b-seams
use crate::session::{SessionPolicy, effective_read_only};
use crate::test_conn::test_connection_with_policy;

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
    /// How often a live connection is pinged: the keep-alive of every profile without one of
    /// its own, unless `presets` (M2a compatibility).
    pub liveness: Duration,
    /// How long quitting waits for connections to close.
    pub shutdown_budget: Duration,
    // ## m2b-t17-liveness
    /// B18's presets (`LivenessPolicy::for_profile`): a profile without its own keep-alive gets
    /// 60 s over SSH and none otherwise (instead of `liveness`), and one without its own idle
    /// disconnect gets its environment's (instead of none). Without them only the profile's
    /// own `idle_disconnect_s` applies (review M3).
    pub presets: bool,
    /// B19: a lost default session reconnects with backoff; without it (M2a, R12) the profile
    /// is `Lost`.
    pub reconnect: bool,
    // ## m3a-t01b-seams
    /// R-M3-6 REVISED, R-M3-29: production-tagged profiles get read-only sessions and Test
    /// connections (`effective_read_only`). Off in `default()` and `m2b()`; the app's boot
    /// turns it on. M3b removes it when confirmations land.
    pub production_read_only: bool,
}

impl Default for ManagerConfig {
    /// M2a's: 30 s liveness for every profile, 2 s shutdown budget, no reconnect, no B18
    /// presets.
    fn default() -> Self {
        ManagerConfig {
            liveness: Duration::from_secs(30),
            shutdown_budget: Duration::from_secs(2),
            presets: false,
            reconnect: false,
            production_read_only: false,
        }
    }
}

// ## m2b-t01-interfaces
impl ManagerConfig {
    /// M2b's configuration: liveness per profile from `LivenessPolicy::for_profile` (B18) and
    /// reconnects (B19). The app boot switches to it (Task 22).
    pub fn m2b() -> ManagerConfig {
        ManagerConfig {
            presets: true,
            reconnect: true,
            // ## m3a-t01b-seams: unchanged for M2b callers; the app boot sets it.
            production_read_only: false,
            ..ManagerConfig::default()
        }
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

// ## m2b-t17-liveness

/// A connected profile (B4): its `SessionManager`, the default console session, and the switch
/// that stops its monitor and its recovery. The session's watcher holds the budget permit the
/// connect took.
struct Live {
    /// Which landing this is (the connect's attempt ID), so late work of an older one is ignored.
    seq: u64,
    sessions: Arc<SessionManager>,
    default: SessionHandle,
    stop: CancellationToken,
    info: Arc<LiveInfo>,
    /// The SSH chain generation the default session runs over (B20).
    transport: Option<TransportGeneration>,
    /// A reconnect's info, until the session runs on it.
    pending: Option<Arc<LiveInfo>>,
}

impl Live {
    /// Stops the monitor and any recovery now, then closes every session (an open transaction
    /// is rolled back) and drops the overlay; each permit returns as its session closes.
    fn close(self) -> BoxFuture<'static, ()> {
        self.stop.cancel();
        let sessions = self.sessions;
        Box::pin(async move { sessions.close_all().await })
    }
}

/// One profile's slot.
struct Entry {
    state: LiveState,
    /// The connect in flight: its id and cancel token (single-flight).
    attempt: Option<(u64, CancellationToken)>,
    live: Option<Live>,
    // ## m2b-t17-liveness (fix round 1, I2)
    /// The profile as last saved (`connect`, `update_profile`): a connect in flight lands on it.
    saved: Option<Arc<Profile>>,
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
    // ## m2b-t17-liveness
    /// Ends the watch on the tunnels' events (dropping the manager).
    tunnel_watch: CancellationToken,
    /// One lock per SSH chain generation: the dependents of a dead chain reconnect one at a
    /// time, so the first brings the chain back and the others share it (B20).
    transport_locks: Mutex<HashMap<TransportGeneration, Arc<tokio::sync::Mutex<()>>>>,
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
            // A failed connect may still ask the profile for changes (Task 14).
            self.publish_changes(id, &e.profile_changes);
            self.set(&mut inner, id, LiveState::Failed(Arc::new(e)));
        }
    }

    /// Sends `event`; nobody listening is fine.
    fn emit(&self, event: ConnectionEvent) {
        let _ = self.events.send(event);
    }

    /// The profile changes a connect asks for, if any (the app saves them).
    fn publish_changes(&self, id: ProfileId, changes: &[ProfileChange]) {
        if !changes.is_empty() {
            self.emit(ConnectionEvent::ProfileChanges {
                profile: id,
                changes: changes.to_vec(),
            });
        }
    }

    /// The liveness of `p`'s default session (B18). Without the presets (`ManagerConfig`'s
    /// M2a default) a profile without a keep-alive of its own pings every `config.liveness`,
    /// and only its own idle disconnect applies (review M3).
    fn policy_for(&self, p: &Profile) -> LivenessPolicy {
        let preset = LivenessPolicy::for_profile(p);
        if self.config.presets {
            return preset;
        }
        LivenessPolicy {
            keepalive: if p.keepalive_s.is_some() {
                preset.keepalive
            } else {
                Some(self.config.liveness)
            },
            idle_disconnect: p.idle_disconnect_s.and(preset.idle_disconnect),
            ..preset
        }
    }

    /// A connection arrived: it becomes `id`'s default console session (B4), or is closed at
    /// once when the attempt was cancelled, forgotten or overtaken by shutdown.
    ///
    /// Fix round 1, I2: the sessions start from the profile as saved now (`used` is the one the
    /// connect started with), so an edit saved during the connect is kept. When that edit sends
    /// the connection elsewhere (T15's destination rule), this connection is closed and the
    /// saved destination connected instead, without the draft password (it was typed for the
    /// old one).
    #[allow(clippy::too_many_arguments, reason = "one private call site")]
    fn landed(
        self: &Arc<Self>,
        id: ProfileId,
        attempt: u64,
        used: Arc<Profile>,
        live: LiveConnection,
        permit: BudgetPermit,
        ix: Interaction,
    ) {
        let mut inner = self.lock();
        let budget = inner.budgets.get(&id).cloned();
        let (true, Some(budget)) = (Self::current(&inner, id, attempt), budget) else {
            drop(inner);
            self.rt.spawn(async move {
                live.close().await;
                drop(permit);
            });
            return;
        };
        let profile = inner
            .entries
            .get(&id)
            .and_then(|e| e.saved.clone())
            .unwrap_or_else(|| used.clone());
        let info = Arc::new(info_of(&live, &profile));
        let (changes, transport) = (live.profile_changes.clone(), live.transport);
        let sessions = SessionManager::new(
            self.rt.clone(),
            profile.clone(),
            self.deps.clone(),
            budget.clone(),
        );
        // ## m3a-t01b-seams: before any session exists (R-M3-29, P4).
        sessions.set_policy(SessionPolicy {
            production_read_only: self.config.production_read_only,
        });
        // ## m2b-t22-live (T17 carry): the gaps of the profile's other sessions (pooled
        // read-only ones) are relayed as the default session's are (Q35).
        let weak = Arc::downgrade(self);
        sessions.relay_gaps(Arc::new(move |reason| {
            if let Some(shared) = weak.upgrade() {
                shared.emit(ConnectionEvent::EnforcementGap {
                    profile: id,
                    reason,
                });
            }
        }));
        let stop = CancellationToken::new();
        let reconnect = self.config.reconnect.then(|| {
            Arc::new(Recovery {
                shared: Arc::downgrade(self),
                sessions: Arc::downgrade(&sessions),
                id,
                seq: attempt,
                stop: stop.clone(),
                loss: Arc::new(Mutex::new(None)),
            }) as Arc<dyn Reconnector>
        });
        let policy = self.policy_for(&profile);
        let default = match sessions.adopt_default(&used, live, permit, reconnect, Some(policy)) {
            Ok(handle) => handle,
            Err((live, permit)) => {
                tracing::debug!(profile = %id, "the profile moved during the connect; reconnecting");
                let token = inner
                    .entries
                    .get(&id)
                    .and_then(|e| e.attempt.as_ref())
                    .map(|(_, token)| token.clone())
                    .unwrap_or_default();
                self.set(&mut inner, id, LiveState::Connecting(fresh_progress()));
                drop(inner);
                // The permit goes only once the connection is closed (SQLite's budget of 1).
                self.rt.spawn(async move {
                    live.close().await;
                    drop(permit);
                });
                let again = self
                    .clone()
                    .run_connect(profile, None, 0, ix, attempt, token, budget);
                self.rt.spawn(again);
                return;
            }
        };
        // Subscribed before anything can happen to the session.
        let (state, events) = (default.state(), default.events());
        if let Some(entry) = inner.entries.get_mut(&id) {
            entry.attempt = None;
            entry.live = Some(Live {
                seq: attempt,
                sessions,
                default,
                stop: stop.clone(),
                info: info.clone(),
                transport,
                pending: None,
            });
        }
        // The default session is Idle from its spawn: the profile is connected.
        self.set(&mut inner, id, LiveState::Connected(info));
        self.publish_changes(id, &changes);
        drop(inner);
        let monitor = monitor(Arc::downgrade(self), id, attempt, state, events, stop);
        self.rt.spawn(monitor);
    }

    /// Whether `seq` is still `id`'s live connection.
    fn mine(inner: &Inner, id: ProfileId, seq: u64) -> bool {
        !inner.shutting_down
            && inner
                .entries
                .get(&id)
                .and_then(|e| e.live.as_ref())
                .is_some_and(|l| l.seq == seq)
    }

    /// The recovery waits `next_in` before attempt `attempt` (`None`: it is connecting).
    fn reconnecting(&self, id: ProfileId, seq: u64, attempt: u32, next_in: Option<Duration>) {
        let mut inner = self.lock();
        if Self::mine(&inner, id, seq) {
            self.set(&mut inner, id, LiveState::Reconnecting { attempt, next_in });
        }
    }

    /// A reconnect landed; the session replays its environment before the profile shows
    /// connected again.
    fn reconnected(&self, id: ProfileId, seq: u64, live: &LiveConnection, profile: &Profile) {
        let mut inner = self.lock();
        if !Self::mine(&inner, id, seq) {
            return;
        }
        if let Some(l) = inner.entries.get_mut(&id).and_then(|e| e.live.as_mut()) {
            l.pending = Some(Arc::new(info_of(live, profile)));
            l.transport = live.transport;
        }
        self.publish_changes(id, &live.profile_changes);
    }

    /// The default session runs on its new connection: `Connected`, then `SessionLost`
    /// (always, B19).
    fn session_back(&self, id: ProfileId, seq: u64, lost: LostState) {
        let mut inner = self.lock();
        if !Self::mine(&inner, id, seq) {
            return;
        }
        let Some(l) = inner.entries.get_mut(&id).and_then(|e| e.live.as_mut()) else {
            return;
        };
        if let Some(info) = l.pending.take() {
            l.info = info;
        }
        let info = l.info.clone();
        self.set(&mut inner, id, LiveState::Connected(info));
        self.emit(ConnectionEvent::SessionLost { profile: id, lost });
    }

    /// Ends live connection `seq` of `id` with `state`: its sessions close and its overlay
    /// goes. False when it already ended (or another one replaced it).
    fn end_live(&self, id: ProfileId, seq: u64, state: LiveState) -> bool {
        let live = {
            let mut inner = self.lock();
            if !Self::mine(&inner, id, seq) {
                return false;
            }
            let live = inner.entries.get_mut(&id).and_then(|e| e.live.take());
            self.set(&mut inner, id, state);
            live
        };
        if let Some(live) = live {
            self.rt.spawn(live.close());
        }
        true
    }

    /// The lock of the SSH chain generation `id`'s default session runs over, if any (B20).
    fn transport_lock(&self, id: ProfileId) -> Option<Arc<tokio::sync::Mutex<()>>> {
        let generation = self
            .lock()
            .entries
            .get(&id)
            .and_then(|e| e.live.as_ref())
            .and_then(|l| l.transport)?;
        let mut locks = self
            .transport_locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Some(lock_for(&mut locks, generation))
    }

    /// An SSH chain died (B20): every default session over that generation checks its
    /// connection now. The check fails over the dead chain and the session recovers, once (its
    /// actor runs one recovery at a time), under the generation's lock.
    fn tunnel_dead(&self, generation: TransportGeneration) {
        let dependents: Vec<(SessionHandle, Arc<SessionManager>)> = self
            .lock()
            .entries
            .values()
            .filter_map(|e| e.live.as_ref())
            .filter(|l| l.transport == Some(generation))
            .map(|l| (l.default.clone(), l.sessions.clone()))
            .collect();
        tracing::debug!(dependents = dependents.len(), "an SSH chain died");
        for (session, sessions) in dependents {
            // The answer is not needed: a failed ping starts the recovery.
            let _ = session.submit(Request::Ping);
            // Parked pooled sessions rode the same chain: never hand one out (review M6).
            sessions.close_idle();
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
            // ## m3a-t01b-seams (R-M3-29)
            read_only: effective_read_only(&profile, self.config.production_read_only),
            profile: profile.clone(),
            draft_password: draft,
            purpose: Purpose::Main,
            department: Department::Console,
            prior_rejections,
            draft_ssh: Vec::new(),
            overlay_secrets: None,
            plaintext_consent: false,
        };
        let deps = self.deps.clone();
        let pipeline = connect(&deps, req, ix.clone(), token, progress_tx);
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
            Ok(live) => self.landed(id, attempt, profile, live, permit, ix),
            Err(e) => self.failed(id, attempt, e),
        }
    }

    /// Takes every connection and cancels every connect; the returned future closes them
    /// within `budget` and says whether all closed in time. A recovery in progress ends at
    /// once (its session closes).
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
        let closes: Vec<_> = lives.into_iter().map(Live::close).collect();
        Box::pin(async move {
            let closes = futures::future::join_all(closes);
            tokio::time::timeout(budget, closes).await.is_ok()
        })
    }
}

/// The lock of chain generation `generation` (B20): one per generation, shared by every
/// dependent that reconnects from it. Locks nobody holds or waits for are forgotten.
fn lock_for(
    locks: &mut HashMap<TransportGeneration, Arc<tokio::sync::Mutex<()>>>,
    generation: TransportGeneration,
) -> Arc<tokio::sync::Mutex<()>> {
    locks.retain(|_, lock| Arc::strong_count(lock) > 1);
    locks.entry(generation).or_default().clone()
}

/// What a connected profile looks like (HV-CONN-01).
fn info_of(live: &LiveConnection, profile: &Profile) -> LiveInfo {
    LiveInfo {
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
        hops: match &profile.transport {
            TransportSpec::SshTunnel(ssh) => u8::try_from(ssh.hops.len()).unwrap_or(u8::MAX),
            _ => 0,
        },
        access_warnings: live.access_warnings.clone(),
    }
}

/// Relays what the default session of live connection `seq` tells (B19, Q35): the reconnect
/// it finished (`Connected`, then `SessionLost`), the idle transaction, the guard's gap, and
/// its end:
/// - `Disconnected` after `SessionEvent::IdleDisconnected`: the idle disconnect (state and
///   event); any other `Disconnected` (someone closed the session) only ends the profile's
///   connection (review M2);
/// - `Broken(cancelled)` (a deactivated request, an unsettled cancel): `Disconnected` (M7);
/// - `Broken(class)`: `Lost { class }` (Retry), unless the recovery already said how it ended;
///   after a loss inside a transaction (C1) it is followed by `SessionLost { lost }`.
///
/// A finished reconnect is read from `SessionEvent::Reconnected` (a broadcast, never
/// coalesced) or, should that event be lagged away, from the session leaving `Connecting`
/// (review M4); either way once.
async fn monitor(
    shared: Weak<Shared>,
    id: ProfileId,
    seq: u64,
    mut state: watch::Receiver<SessionSnapshot>,
    mut events: broadcast::Receiver<SessionEvent>,
    stop: CancellationToken,
) {
    let mut seen = Seen::default();
    let mut events_open = true;
    loop {
        tokio::select! {
            biased;
            () = stop.cancelled() => return,
            event = events.recv(), if events_open => {
                let Some(shared) = shared.upgrade() else { return };
                match event {
                    Ok(event) => seen.relay(&shared, id, seq, event),
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => events_open = false,
                }
            }
            changed = state.changed() => {
                let now = state.borrow_and_update().state.clone();
                let Some(shared) = shared.upgrade() else { return };
                let now = if changed.is_err() && usable(&now) {
                    // The actor is gone without saying why.
                    SessionState::Broken(ErrClass::Internal)
                } else {
                    now
                };
                if !usable(&now) {
                    // Fix round 4 (I10): whatever the session sent before it ended is relayed
                    // before the monitor stops (an enforcement gap, the idle disconnect).
                    loop {
                        match events.try_recv() {
                            Ok(event) => seen.relay(&shared, id, seq, event),
                            Err(broadcast::error::TryRecvError::Lagged(_)) => {}
                            Err(_) => break,
                        }
                    }
                }
                match now {
                    SessionState::Connecting => seen.recovering = true,
                    SessionState::Disconnected => {
                        if shared.end_live(id, seq, LiveState::Disconnected) && seen.idle {
                            shared.emit(ConnectionEvent::IdleDisconnected { profile: id });
                        }
                        return;
                    }
                    SessionState::Broken(ErrClass::Cancelled) => {
                        shared.end_live(id, seq, LiveState::Disconnected);
                        return;
                    }
                    SessionState::Broken(class) => {
                        if shared.end_live(id, seq, LiveState::Lost { class }) {
                            tracing::debug!(profile = %id, class = class.id(), "the session is lost");
                            if let Some(lost) = seen.lost.take() {
                                // C1: lost inside a transaction; nothing was replayed.
                                shared.emit(ConnectionEvent::SessionLost { profile: id, lost });
                            }
                        }
                        return;
                    }
                    _ if seen.recovering => {
                        seen.recovering = false;
                        shared.session_back(id, seq, seen.lost.take().unwrap_or_default());
                    }
                    _ => {}
                }
            }
        }
    }
}

/// What the monitor has heard from the default session so far.
#[derive(Default)]
struct Seen {
    /// What the last loss took, until it is reported.
    lost: Option<LostState>,
    /// A reconnect is under way (`Lost` or `Connecting` seen) and not yet reported.
    recovering: bool,
    /// The supervisor announced its idle disconnect.
    idle: bool,
}

impl Seen {
    /// Takes in one session event, relaying what the app shows (Q35).
    fn relay(&mut self, shared: &Shared, id: ProfileId, seq: u64, event: SessionEvent) {
        match event {
            SessionEvent::Lost(l) => {
                self.lost = Some(l);
                self.recovering = true;
            }
            SessionEvent::Reconnected => {
                if std::mem::take(&mut self.recovering) {
                    shared.session_back(id, seq, self.lost.take().unwrap_or_default());
                }
            }
            SessionEvent::IdleDisconnected => self.idle = true,
            SessionEvent::IdleTransaction { minutes } => {
                shared.emit(ConnectionEvent::IdleTransaction {
                    profile: id,
                    minutes,
                });
            }
            SessionEvent::EnforcementGap(reason) => {
                shared.emit(ConnectionEvent::EnforcementGap {
                    profile: id,
                    reason,
                });
            }
            _ => {}
        }
    }
}

/// Whether a session in `state` still runs.
fn usable(state: &SessionState) -> bool {
    !matches!(state, SessionState::Disconnected | SessionState::Broken(_))
}

/// One loss's place in the backoff, kept across the reconnects of that loss (fix round 1,
/// I1): a replay that fails continues the schedule and the give-up clock.
#[derive(Debug)]
struct Loss {
    lost_at: tokio::time::Instant,
    /// The next attempt's number (from 1).
    next: u32,
    last_class: ErrClass,
    /// `Reconnector::replay_failed` was called: the next `reconnect` continues this loss.
    continuing: bool,
}

/// The default session's recovery (B19, Q17): the backoff, `Reconnecting`, a headless connect
/// with the session overlay, and how it ends. It never prompts: a connect that would have to
/// ask (`needs_interaction`) ends it with `Lost` at once; an `auth.*` failure (or any other
/// failure that is not a recovery case) with `Failed`; five minutes without success with `Lost`.
struct Recovery {
    shared: Weak<Shared>,
    sessions: Weak<SessionManager>,
    id: ProfileId,
    seq: u64,
    /// Cancelled when the profile disconnects, is forgotten, or the manager shuts down.
    stop: CancellationToken,
    loss: Arc<Mutex<Option<Loss>>>,
}

impl Recovery {
    fn loss(&self) -> MutexGuard<'_, Option<Loss>> {
        self.loss.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Reconnector for Recovery {
    /// Runs the recovery of a new loss, or continues the current one after a failed replay
    /// (`replay_failed`); `attempt` is not used (the loss keeps the count).
    fn reconnect(&self, _attempt: u32) -> BoxFuture<'static, Result<LiveConnection, ConnectError>> {
        let policy = RecoveryPolicy::default();
        let mut rng = RecoveryPolicy::jitter_source();
        let cancelled = || error(ErrClass::Cancelled, Phase::PostConnect);
        let (lost_at, mut attempt, mut last_class) = {
            let mut loss = self.loss();
            let resume = loss
                .as_mut()
                .is_some_and(|l| std::mem::take(&mut l.continuing));
            if !resume {
                *loss = Some(Loss {
                    lost_at: tokio::time::Instant::now(),
                    next: 1,
                    last_class: ErrClass::NetworkReset,
                    continuing: false,
                });
            }
            let l = loss.as_ref().expect("set above");
            (l.lost_at, l.next, l.last_class)
        };
        let Some(mut delay) = policy.delay(attempt - 1, lost_at.elapsed(), &mut rng) else {
            // A failed replay used up the five minutes.
            if let Some(shared) = self.shared.upgrade() {
                shared.end_live(self.id, self.seq, LiveState::Lost { class: last_class });
            }
            return Box::pin(std::future::ready(Err(error(
                last_class,
                Phase::PostConnect,
            ))));
        };
        // Synchronously: the capsule shows the reconnect before the failed request answers.
        if let Some(shared) = self.shared.upgrade() {
            shared.reconnecting(self.id, self.seq, attempt, Some(delay));
        }
        let (shared, sessions, id, seq, stop, loss) = (
            self.shared.clone(),
            self.sessions.clone(),
            self.id,
            self.seq,
            self.stop.clone(),
            self.loss.clone(),
        );
        let note = move |next: u32, class: ErrClass| {
            let mut loss = loss.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(l) = loss.as_mut() {
                l.next = next;
                l.last_class = class;
            }
        };
        Box::pin(async move {
            loop {
                tokio::select! {
                    biased;
                    () = stop.cancelled() => return Err(cancelled()),
                    () = tokio::time::sleep(delay) => {}
                }
                let (Some(manager), Some(sessions)) = (shared.upgrade(), sessions.upgrade()) else {
                    return Err(cancelled());
                };
                manager.reconnecting(id, seq, attempt, None);
                // B20: one dependent of a dead chain at a time.
                let _chain = match manager.transport_lock(id) {
                    Some(lock) => tokio::select! {
                        biased;
                        () = stop.cancelled() => return Err(cancelled()),
                        guard = lock.lock_owned() => Some(guard),
                    },
                    None => None,
                };
                drop(manager);
                let landed = tokio::select! {
                    biased;
                    () = stop.cancelled() => return Err(cancelled()),
                    landed = sessions.reconnect_live() => landed,
                };
                drop(sessions);
                let Some(manager) = shared.upgrade() else {
                    return Err(cancelled());
                };
                let e = match landed {
                    Ok((live, profile)) => {
                        // A failed replay continues with the attempt after this one.
                        note(attempt + 1, last_class);
                        manager.reconnected(id, seq, &live, &profile);
                        return Ok(live);
                    }
                    Err(e) => e,
                };
                manager.publish_changes(id, &e.profile_changes);
                if stop.is_cancelled() {
                    // A disconnect, a forget or the shutdown says how it ends.
                    return Err(e);
                }
                if e.class.family() != ErrFamily::Auth && e.needs_interaction {
                    // Q17: recovery never prompts (a one-time code, an unknown host key, a
                    // locked vault); the capsule's Retry connects interactively.
                    manager.end_live(id, seq, LiveState::Lost { class: e.class });
                    return Err(e);
                }
                // `cancelled` without a stop: the saved profile moved while this connect ran
                // (T15 I1 refused it); the next attempt goes to the new destination (M7).
                if !is_recovery_case(e.class) && e.class != ErrClass::Cancelled {
                    // `auth.*` (a rejected password, or one the server wants and the overlay
                    // no longer has: the error dialog offers the re-entry) and every other
                    // failure a retry cannot fix (B19).
                    manager.end_live(id, seq, LiveState::Failed(Arc::new(e.clone())));
                    return Err(e);
                }
                if e.class != ErrClass::Cancelled {
                    last_class = e.class;
                }
                attempt += 1;
                note(attempt, last_class);
                match policy.delay(attempt - 1, lost_at.elapsed(), &mut rng) {
                    Some(next) => {
                        delay = next;
                        manager.reconnecting(id, seq, attempt, Some(next));
                    }
                    None => {
                        manager.end_live(id, seq, LiveState::Lost { class: last_class });
                        return Err(e);
                    }
                }
            }
        })
    }

    fn replay_failed(&self, class: ErrClass) {
        if let Some(l) = self.loss().as_mut() {
            l.continuing = true;
            l.last_class = class;
        }
    }
}

/// Watches the SSH chains for deaths (B20) until `stop`.
fn watch_tunnels(shared: &Arc<Shared>) {
    let mut events = shared.deps.tunnels.events();
    let weak = Arc::downgrade(shared);
    let stop = shared.tunnel_watch.clone();
    shared.rt.spawn(async move {
        loop {
            let event = tokio::select! {
                biased;
                () = stop.cancelled() => return,
                event = events.recv() => event,
            };
            match event {
                Ok(TunnelEvent::Dead { key, generation }) => {
                    let Some(shared) = weak.upgrade() else { return };
                    shared.tunnel_dead(TransportGeneration { key, generation });
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    });
}

/// Every profile's connection, on the core runtime (HV-CONN-01).
pub struct ConnectionManager {
    shared: Arc<Shared>,
}

impl Drop for ConnectionManager {
    /// Stops every session's liveness and recovery, closes every open connection (in the
    /// background) and takes every connect in flight; their tasks hold the shared state, so
    /// without this they would keep it (and the connections) alive. Marked as shutting down, so
    /// a connect that lands afterwards is closed at once and never starts a session.
    fn drop(&mut self) {
        self.shared.tunnel_watch.cancel();
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
            self.shared.rt.spawn(live.close());
        }
    }
}

impl ConnectionManager {
    pub fn new(rt: Handle, deps: Arc<ConnectDeps>, config: ManagerConfig) -> Arc<Self> {
        let shared = Arc::new(Shared {
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
            tunnel_watch: CancellationToken::new(),
            transport_locks: Mutex::new(HashMap::new()),
        });
        // ## m2b-t17-liveness (B20)
        watch_tunnels(&shared);
        Arc::new(ConnectionManager { shared })
    }

    // ## m2b-t01-interfaces

    /// The manager's events (Task 17 sends them; the app relays them, Q42).
    pub fn events(&self) -> broadcast::Receiver<ConnectionEvent> {
        self.shared.events.subscribe()
    }

    /// Sends `event` to every `events()` receiver: a test seam (fix round 1, M5), only with
    /// the `test-support` feature, so no production caller can inject profile changes.
    #[cfg(feature = "test-support")]
    pub fn publish_event(&self, event: ConnectionEvent) {
        // No receiver is not an error: nobody listens yet.
        let _ = self.shared.events.send(event);
    }

    /// How many `events()` receivers are open (test seam, feature `test-support`).
    #[cfg(feature = "test-support")]
    pub fn event_subscribers(&self) -> usize {
        self.shared.events.receiver_count()
    }

    /// The sessions of a connected (or reconnecting) profile (B4): its default console session
    /// is `SessionManager::default_session`.
    pub fn sessions(&self, id: ProfileId) -> Option<Arc<SessionManager>> {
        self.shared
            .lock()
            .entries
            .get(&id)
            .and_then(|e| e.live.as_ref())
            .map(|l| l.sessions.clone())
    }

    /// Gives a connected profile's sessions the saved `profile` (Q17): the app calls it after
    /// every save and `apply_changes`. Reconnects and new sessions connect with it.
    pub fn update_profile(&self, id: ProfileId, profile: Arc<Profile>) {
        let sessions = {
            let mut inner = self.shared.lock();
            let Some(entry) = inner.entries.get_mut(&id) else {
                return;
            };
            // A connect in flight lands on it (fix round 1, I2).
            entry.saved = Some(profile.clone());
            entry.live.as_ref().map(|l| l.sessions.clone())
        };
        if let Some(sessions) = sessions {
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

    /// The connection dialog's Save and connect after rejected tests (P8): connects with the
    /// draft password, if any, continuing the dialog's count of server rejections.
    pub fn connect_continuing(
        &self,
        profile: Arc<Profile>,
        draft_password: Option<SecretString>,
        prior_rejections: u8,
        ix: Interaction,
    ) {
        self.start(profile, draft_password, prior_rejections, ix);
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
            saved: None,
        });
        if entry.attempt.is_some() || entry.live.is_some() {
            return;
        }
        entry.saved = Some(profile.clone());
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

    /// Closes the profile's connection (cancelling a connect in progress, ending a recovery) and
    /// forgets its `ForSession` secrets.
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
        self.test_continuing(draft, 0, ix)
    }

    /// [`Self::test`] continuing the connection dialog's count of server rejections (P8).
    pub fn test_continuing(
        &self,
        draft: ProfileDraft,
        prior_rejections: u8,
        ix: Interaction,
    ) -> TestHandle {
        let token = CancellationToken::new();
        let (progress_tx, progress) = watch::channel(fresh_progress());
        let (result_tx, result) = oneshot::channel();
        let deps = self.shared.deps.clone();
        let cancel = token.clone();
        // ## m3a-t01b-seams: a Test of a production profile runs read-only under the policy
        // (R-M3-29), so it exercises the path its sessions take.
        let production_read_only = self.shared.config.production_read_only;
        self.shared.rt.spawn(async move {
            let report = test_connection_with_policy(
                &deps,
                draft,
                prior_rejections,
                production_read_only,
                ix,
                cancel,
                progress_tx,
            )
            .await;
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

    // ## m2b-t17-liveness

    #[tokio::test]
    async fn the_dependents_of_one_chain_generation_reconnect_one_at_a_time() {
        use hacivat_net::ssh::ChainKey;
        let generation = |key: u8, generation| TransportGeneration {
            key: ChainKey([key; 32]),
            generation,
        };
        let mut locks = HashMap::new();
        let first = lock_for(&mut locks, generation(1, 4));
        let held = first.clone().try_lock_owned().expect("free");
        // A second dependent of the same generation waits for the first.
        let second = lock_for(&mut locks, generation(1, 4));
        assert!(Arc::ptr_eq(&first, &second));
        assert!(second.try_lock().is_err());
        // Another chain, or the next generation of this one, has its own lock.
        assert!(lock_for(&mut locks, generation(2, 4)).try_lock().is_ok());
        assert!(lock_for(&mut locks, generation(1, 5)).try_lock().is_ok());
        drop(held);
        drop((first, second));
        // Nobody holds or waits for any: they are forgotten at the next lookup.
        let _again = lock_for(&mut locks, generation(3, 1));
        assert_eq!(locks.len(), 1);
    }
}
