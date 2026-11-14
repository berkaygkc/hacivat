//! A profile's sessions (HV-SESS-01, -02, -08; D-035; Task 15): the default session, client
//! and job sessions, the idle pool, side connections and the session overlay (Q17).
//!
//! - **Budget.** Every session holds a permit of the profile's [`Budget`] (M2a's fair
//!   semaphore) by its kind: Console → `Console`, Editor → `Editor`, Job → `Job`,
//!   Introspection and Service → `Introspection`. The permit lives as long as the session: a
//!   watcher task holds it until the session is `Disconnected` or `Broken`, or its actor ends
//!   (every handle gone). A request that has to wait closes one idle pooled session, so idle
//!   sessions never starve a console. Side connections (`Department::Cancel`) take no permit.
//! - **Pools** are keyed by `(kind, read_only)`: a read-only request is never served by a
//!   read-write session, nor the other way round. Dropping a [`PooledSession`] resets it
//!   (`Request::Reset`, HV-SESS-08) and only then returns it to the idle pool, so no two
//!   borrowers ever hold one session. A failed reset closes it (Task 7 already broke it), and
//!   the permit goes. A reset session never parks while a request waits for a permit (review
//!   C1): a console, editor or job gets its permit (the session closes), else a `pooled`
//!   request of the same key gets the session itself, else any other waiter gets the permit.
//!   An idle session closes 60 s after its release. A `Broken` or `Disconnected` session is
//!   never handed out again.
//! - **Single-session mode** (`Profile::single_session`, and SQLite) gives `open` and `pooled`
//!   the default session. A borrow of it is a guard that never resets (Q19: no `ROLLBACK` of the
//!   user's transaction); its requests queue behind the console's in the actor's FIFO.
//!   An MCP session never shares a user's session (HV-SESS-02), and a read-only request never
//!   shares a read-write default (decided on the default's own snapshot): both get their own
//!   session, or are refused at once when the budget is 1 (SQLite), where they could only wait
//!   forever.
//! - **The overlay (Q17).** After each successful connect the manager applies the connect's
//!   `profile_changes` to its in-memory profile, records the plaintext consent and keeps the
//!   secrets that worked (`SecretString`, zeroized on drop). Every later connect of the
//!   profile (sessions, pools, side connections; Task 17's reconnects read [`overlay`]) uses
//!   them, so a pin, a CA or a `Never` password answered once holds for the live connection.
//!   `close_all` (disconnect, `forget`, quit) drops it; it is never persisted, and its `Debug`
//!   redacts the secrets. A connect is absorbed only while the saved profile still names the
//!   destination it connected to (review I1); a new overlay starts from the saved profile.
//! - No environment plan or probe runs at spawn (Q27): connect already set the session up.
//!
//! [`overlay`]: SessionManager::overlay

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::mem;
use std::ops::Deref;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_net::ssh::HopSecrets;
use hacivat_proto::{Department, ErrClass};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

use crate::budget::{Budget, BudgetPermit, Priority};
use crate::pipeline::{
    self, ConnectDeps, ConnectError, ConnectRequest, Interaction, LiveConnection, Phase,
    ProfileChange, Purpose, SessionSecrets,
};
use crate::profile::Profile;
use crate::session::actor::{Reconnector, SessionHooks, SideConnector, spawn_supervised};
use crate::session::liveness::LivenessPolicy;
use crate::session::{
    ClientId, ClosePolicy, Command, Escalation, Request, SessionEvent, SessionHandle, SessionId,
    SessionKind, SessionSnapshot, SessionSpec, SessionState,
};
// ## m3a-t01b-seams
use crate::session::effective_read_only;

/// How long a released pooled session stays idle before it closes (HV-SESS-01).
pub const IDLE_CLOSE: Duration = Duration::from_secs(60);

/// What a profile's sessions keep for reconnects and side connections, in memory only (Q17):
/// the profile with the changes made since connect, the plaintext consent and the secrets that
/// worked. Dropped at disconnect, forget and quit; never persisted. `Debug` only (redacted).
#[derive(Debug)]
pub struct SessionOverlay {
    pub profile: Arc<Profile>,
    pub plaintext_consent: bool,
    pub secrets: SessionSecrets,
}

/// A session borrowed from the pool. Dropping it resets it and returns it to the idle pool
/// (closed after 60 s idle); a borrow of the default session (Single mode, SQLite) is never
/// reset (Q19).
///
/// It derefs to the session's [`SessionHandle`], which is `Clone`: a clone must not outlive
/// the borrow, since the session goes to the next borrower once this guard drops.
pub struct PooledSession {
    handle: SessionHandle,
    /// `None` for a borrow of the default session: nothing happens on drop.
    release: Option<Release>,
}

/// Where a pooled session goes back to.
struct Release {
    manager: Weak<SessionManager>,
    kind: SessionKind,
}

impl fmt::Debug for PooledSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PooledSession")
            .field("id", &self.handle.id)
            .field("owned", &self.release.is_some())
            .finish()
    }
}

impl Deref for PooledSession {
    type Target = SessionHandle;

    fn deref(&self) -> &SessionHandle {
        &self.handle
    }
}

impl Drop for PooledSession {
    fn drop(&mut self) {
        let Some(Release { manager, kind }) = self.release.take() else {
            // Q19: a borrow of the default session never resets it.
            return;
        };
        if let Some(manager) = manager.upgrade() {
            manager.give_back(self.handle.clone(), kind);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The overlay

/// The overlay as the manager keeps it.
struct Overlay {
    profile: Arc<Profile>,
    plaintext_consent: bool,
    secrets: SessionSecrets,
}

impl Overlay {
    fn new(profile: Arc<Profile>) -> Overlay {
        Overlay {
            profile,
            plaintext_consent: false,
            secrets: SessionSecrets::default(),
        }
    }

    /// Takes in what a successful connect learned.
    fn absorb(&mut self, changes: &[ProfileChange], secrets: SessionSecrets, consent: bool) {
        if !changes.is_empty() {
            let mut profile = Profile::clone(&self.profile);
            for change in changes {
                apply_change(&mut profile, change);
            }
            self.profile = Arc::new(profile);
        }
        self.plaintext_consent |= consent;
        merge_secrets(&mut self.secrets, secrets);
    }

    /// Replaces the profile (the app saved it). The consent and the secrets stay while the
    /// profile still names the same destination; an edit that sends the connection elsewhere
    /// drops them, so a secret never reaches a server it was not typed for. The plaintext
    /// consent also goes when the TLS mode changes or "allow public plaintext" is unticked
    /// (review M3): the user took back what it answered.
    fn replace_profile(&mut self, profile: Arc<Profile>) {
        if !same_destination(&self.profile, &profile) {
            self.secrets = SessionSecrets::default();
            self.plaintext_consent = false;
        }
        if takes_consent_back(&self.profile, &profile) {
            self.plaintext_consent = false;
        }
        self.profile = profile;
    }

    fn view(&self) -> SessionOverlay {
        SessionOverlay {
            profile: self.profile.clone(),
            plaintext_consent: self.plaintext_consent,
            secrets: copy_secrets(&self.secrets),
        }
    }
}

/// Applies one connect's change to the in-memory profile, as `ConnectionsModel::apply_changes`
/// does to the saved one.
fn apply_change(p: &mut Profile, change: &ProfileChange) {
    match change {
        ProfileChange::AddPin(pin) => {
            if !p.tls.pins.contains(pin) {
                p.tls.pins.push(pin.clone());
            }
        }
        ProfileChange::ReplacePin { old, new } => {
            p.tls
                .pins
                .retain(|pin| !(pin.endpoint == new.endpoint && pin.sha256 == *old) && pin != new);
            p.tls.pins.push(new.clone());
        }
        ProfileChange::SetCaFile(path) => p.tls.ca_file = Some(path.clone()),
        ProfileChange::AllowPublicPlaintext => p.tls.allow_public_plaintext = true,
        // ## m2b-t21-prompts (fix round 1): the Remember an SSH secret prompt chose.
        ProfileChange::SshRetention(retention) => p.ssh_retention = *retention,
    }
}

/// Whether two profiles send their secrets to the same place.
fn same_destination(a: &Profile, b: &Profile) -> bool {
    a.id == b.id
        && a.engine == b.engine
        && a.endpoint == b.endpoint
        && a.user == b.user
        && a.auth == b.auth
        && a.transport == b.transport
        && a.tls.client_cert == b.tls.client_cert
}

/// The newer secrets win, field by field (a connect returns only what it used).
fn merge_secrets(into: &mut SessionSecrets, from: SessionSecrets) {
    let SessionSecrets {
        db,
        hops,
        tls_key_passphrase,
    } = from;
    if db.is_some() {
        into.db = db;
    }
    if tls_key_passphrase.is_some() {
        into.tls_key_passphrase = tls_key_passphrase;
    }
    for (i, hop) in hops.into_iter().enumerate() {
        match into.hops.get_mut(i) {
            Some(slot) => {
                if hop.password.is_some() {
                    slot.password = hop.password;
                }
                if hop.passphrase.is_some() {
                    slot.passphrase = hop.passphrase;
                }
            }
            None => into.hops.push(hop),
        }
    }
}

/// A copy for one connect; each copy zeroizes itself on drop.
fn copy_secrets(s: &SessionSecrets) -> SessionSecrets {
    SessionSecrets {
        db: s.db.clone(),
        hops: s
            .hops
            .iter()
            .map(|h| HopSecrets {
                password: h.password.clone(),
                passphrase: h.passphrase.clone(),
            })
            .collect(),
        tls_key_passphrase: s.tls_key_passphrase.clone(),
    }
}

// ---------------------------------------------------------------------------------------------
// The manager

/// A session the manager knows, without keeping it alive.
struct Tracked {
    commands: mpsc::WeakUnboundedSender<Command>,
    state: watch::Receiver<SessionSnapshot>,
}

/// A reset session waiting in the pool.
struct Idle {
    handle: SessionHandle,
    /// Which release parked it (its idle timer closes only that stay).
    token: u64,
}

type PoolKey = (SessionKind, bool);

/// Where a released session goes once it is reset.
enum AfterReset {
    /// To a `pooled` request that waited for one.
    Handed,
    /// Closed: its permit goes to a waiting request, or the manager closed everything.
    Close,
    /// Into the idle pool, from this release's token.
    Parked(u64),
}

struct State {
    /// The profile as saved (`update_profile`).
    profile: Arc<Profile>,
    overlay: Option<Overlay>,
    /// Bumped by `close_all`: work started before it never registers a session or an overlay.
    epoch: u64,
    /// Cancels the connects in flight at `close_all`.
    cancel: CancellationToken,
    sessions: HashMap<SessionId, Tracked>,
    idle: HashMap<PoolKey, Vec<Idle>>,
    next_token: u64,
    default: Option<SessionHandle>,
    /// `pooled` requests waiting for a permit, by key: a released session goes to the first
    /// (review C1).
    waiting: HashMap<PoolKey, VecDeque<Waiter>>,
    next_waiter: u64,
    // ## m2b-t17-liveness
    /// Bumped when the saved profile takes the plaintext consent back (a TLS-mode change, an
    /// unticked "allow public plaintext"): a connect started before it never brings the consent
    /// back when it lands (T15 re-review R1).
    consent_gen: u64,
    // ## m2b-t22-live
    /// Where the enforcement gaps of the sessions this manager starts go (the
    /// `ConnectionManager` relays them as `ConnectionEvent::EnforcementGap`; its monitor
    /// already relays the default session's).
    gap_sink: Option<GapSink>,
    // ## m3a-t01b-seams
    /// The manager-wide session policy (`set_policy`).
    policy: SessionPolicy,
}

impl State {
    // ## m3a-t01b-seams
    /// Whether a session of the current profile is read-only (R-M3-29, P4).
    fn effective_read_only(&self) -> bool {
        effective_read_only(&self.current(), self.policy.production_read_only)
    }
}

// ## m3a-t01b-seams
/// What every session of a profile follows besides the profile itself (R-M3-29, P4).
/// `ConnectionManager` sets it right after `SessionManager::new`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SessionPolicy {
    /// Production-tagged profiles get read-only sessions (R-M3-6 REVISED; M3b removes it).
    pub production_read_only: bool,
}

// ## m2b-t22-live
/// Receives a session's enforcement gap (T17 carry: a pooled read-only session's gap reaches
/// the app as the default session's does).
pub(crate) type GapSink = Arc<dyn Fn(hacivat_guard::GapReason) + Send + Sync>;

/// A `pooled` request waiting for a released session or a permit, whichever comes first.
struct Waiter {
    id: u64,
    tx: oneshot::Sender<SessionHandle>,
}

/// Takes a waiter out of the list when its request ends (served, or dropped).
struct WaiterGuard {
    manager: Weak<SessionManager>,
    key: PoolKey,
    id: u64,
}

impl Drop for WaiterGuard {
    fn drop(&mut self) {
        if let Some(manager) = self.manager.upgrade() {
            let mut st = manager.lock();
            if let Some(queue) = st.waiting.get_mut(&self.key) {
                queue.retain(|w| w.id != self.id);
                if queue.is_empty() {
                    st.waiting.remove(&self.key);
                }
            }
        }
    }
}

impl State {
    /// The profile every connect uses: the overlay's, else the saved one.
    fn current(&self) -> Arc<Profile> {
        self.overlay
            .as_ref()
            .map_or_else(|| self.profile.clone(), |o| o.profile.clone())
    }

    /// Takes the overlay's part of `live` (its secrets leave the connection) and returns the
    /// profile the session runs with. `None` when the saved profile no longer names the
    /// destination `used` connected to (review I1): nothing of that connect is kept, and the
    /// caller closes it. A new overlay starts from the saved profile, never from the
    /// connect's snapshot.
    fn absorb(
        &mut self,
        used: &Profile,
        live: &mut LiveConnection,
        consent_gen: u64,
    ) -> Option<Arc<Profile>> {
        if !same_destination(used, &self.profile) {
            return None;
        }
        // ## m2b-t17-liveness (T15 R1)
        live.plaintext_consent = keep_consent(
            consent_gen,
            self.consent_gen,
            live.plaintext_consent,
            &mut live.profile_changes,
        );
        let secrets = mem::take(&mut live.secrets_used);
        let saved = self.profile.clone();
        let overlay = self.overlay.get_or_insert_with(|| Overlay::new(saved));
        overlay.absorb(&live.profile_changes, secrets, live.plaintext_consent);
        Some(overlay.profile.clone())
    }

    /// Hands a reset session to a request of `key` waiting for one; false when none waits.
    fn hand_to_waiter(&mut self, key: &PoolKey, handle: &SessionHandle) -> bool {
        let Some(queue) = self.waiting.get_mut(key) else {
            return false;
        };
        while let Some(waiter) = queue.pop_front() {
            if waiter.tx.send(handle.clone()).is_ok() {
                return true;
            }
        }
        false
    }

    /// Drops what the manager keeps of session `id`.
    fn forget(&mut self, id: SessionId) {
        self.sessions.remove(&id);
        for parked in self.idle.values_mut() {
            parked.retain(|i| i.handle.id != id);
        }
        if self.default.as_ref().is_some_and(|d| d.id == id) {
            self.default = None;
        }
    }
}

/// Whether a session can still run requests.
fn usable(state: &SessionState) -> bool {
    !matches!(state, SessionState::Disconnected | SessionState::Broken(_))
}

fn alive(h: &SessionHandle) -> bool {
    usable(&h.state.borrow().state)
}

/// The budget priority of `kind` (CONN-02).
fn priority(kind: &SessionKind) -> Priority {
    match kind {
        SessionKind::Console(_) => Priority::Console,
        SessionKind::Editor(_) => Priority::Editor,
        SessionKind::Job(..) => Priority::Job,
        SessionKind::Introspection { .. } | SessionKind::Service => Priority::Introspection,
    }
}

/// The pipeline purpose of `kind`'s connect.
fn purpose(kind: &SessionKind) -> Purpose {
    match kind {
        SessionKind::Introspection { .. } | SessionKind::Service => Purpose::Probe,
        _ => Purpose::Main,
    }
}

/// The error of a dedicated session that a budget of 1 can never serve next to the default
/// one (review M2): it would wait forever.
fn no_room() -> ConnectError {
    pipeline::error(ErrClass::ServerTooManyConnections, Phase::Validate)
}

/// The error of work that `close_all` overtook.
fn closed() -> ConnectError {
    pipeline::error(ErrClass::Cancelled, Phase::PostConnect)
}

/// One profile's sessions.
pub struct SessionManager {
    rt: Handle,
    deps: Arc<ConnectDeps>,
    budget: Arc<Budget>,
    state: Mutex<State>,
    /// Serialises the opening of the default session (one connection, however many ask).
    default_lock: tokio::sync::Mutex<()>,
    me: Weak<SessionManager>,
}

impl fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never the overlay itself: only whether there is one.
        let (profile, open, overlay) = {
            let st = self.lock();
            (st.profile.id, st.sessions.len(), st.overlay.is_some())
        };
        f.debug_struct("SessionManager")
            .field("profile", &profile)
            .field("open", &open)
            .field("overlay", &overlay)
            .finish_non_exhaustive()
    }
}

impl SessionManager {
    pub fn new(
        rt: Handle,
        profile: Arc<Profile>,
        deps: Arc<ConnectDeps>,
        budget: Arc<Budget>,
    ) -> Arc<SessionManager> {
        Arc::new_cyclic(|me| SessionManager {
            rt,
            deps,
            budget,
            state: Mutex::new(State {
                profile,
                overlay: None,
                epoch: 0,
                cancel: CancellationToken::new(),
                sessions: HashMap::new(),
                idle: HashMap::new(),
                next_token: 0,
                default: None,
                waiting: HashMap::new(),
                next_waiter: 0,
                consent_gen: 0,
                gap_sink: None,
                policy: SessionPolicy::default(),
            }),
            default_lock: tokio::sync::Mutex::new(()),
            me: me.clone(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    // ## m3a-t01b-seams

    /// Sets the policy every session opened from now on follows (R-M3-29, P4).
    /// `ConnectionManager` calls it right after `new`, before any session exists.
    pub fn set_policy(&self, p: SessionPolicy) {
        self.lock().policy = p;
    }

    /// The connection budget: `(in use, max)` (R-M3-43: "Waiting for a free connection").
    pub fn budget_use(&self) -> (u8, u8) {
        (self.budget.in_use(), self.budget.max())
    }

    /// Whether a session of `profile` is read-only under this manager's policy.
    fn read_only_for(&self, profile: &Profile) -> bool {
        effective_read_only(profile, self.lock().policy.production_read_only)
    }

    fn profile(&self) -> Arc<Profile> {
        self.lock().profile.clone()
    }

    /// Single-session mode: the profile asks for it, or the engine is SQLite (budget 1).
    fn single(&self) -> bool {
        let p = self.profile();
        p.single_session || p.engine == hacivat_proto::DbmsKind::Sqlite
    }

    /// Opens a session for `spec` (the module docs). Dropping the future at any point gives
    /// back its place in the budget queue or its permit, and cancels its connect.
    pub fn open(
        self: &Arc<Self>,
        spec: SessionSpec,
        ix: Interaction,
    ) -> BoxFuture<'static, Result<SessionHandle, ConnectError>> {
        let this = self.clone();
        let gate = self.gate();
        if self.single() {
            return Box::pin(async move {
                if spec.department != Department::Mcp
                    && let Some(handle) = this.shared(spec.read_only, ix.clone(), &gate).await?
                {
                    return Ok(handle);
                }
                // MCP, or a read-only request the read-write default cannot serve: its own
                // session (HV-SESS-02), if the budget has room for one next to the default.
                if this.budget.max() <= 1 {
                    return Err(no_room());
                }
                let permit = gate.wait(this.permit(priority(&spec.kind))).await?;
                this.start(spec, ix, permit, false, &gate).await
            });
        }
        // Queued now, not at the first poll: the order of calls is the order of service.
        let permit = self.permit(priority(&spec.kind));
        // Only `open` makes the app's own console the default (review M1: never a borrow).
        let as_default = spec.kind == SessionKind::Console(ClientId::DEFAULT)
            && spec.department != Department::Mcp;
        Box::pin(async move {
            let permit = gate.wait(permit).await?;
            this.start(spec, ix, permit, as_default, &gate).await
        })
    }

    /// Borrows a pooled session of `kind`: an idle one of the same `(kind, read_only)`, else a
    /// new one (headless). A request that waits for a permit also takes the first session of
    /// its key released meanwhile (review C1).
    pub fn pooled(
        self: &Arc<Self>,
        kind: SessionKind,
    ) -> BoxFuture<'static, Result<PooledSession, ConnectError>> {
        // ## m3a-t01b-seams: the effective flag, so a production borrow finds its parked
        // read-only session again (P4).
        let read_only = self.lock().effective_read_only();
        if self.single() {
            let this = self.clone();
            let gate = self.gate();
            return Box::pin(async move {
                if let Some(handle) = this.shared(read_only, Interaction::Headless, &gate).await? {
                    return Ok(PooledSession {
                        handle,
                        release: None,
                    });
                }
                // The profile turned read-only under a read-write default (review I2).
                if this.budget.max() <= 1 {
                    return Err(no_room());
                }
                this.pooled_owned(kind, read_only).await
            });
        }
        self.pooled_owned(kind, read_only)
    }

    /// A pooled session the pool owns (reset on release).
    fn pooled_owned(
        self: &Arc<Self>,
        kind: SessionKind,
        read_only: bool,
    ) -> BoxFuture<'static, Result<PooledSession, ConnectError>> {
        let key = (kind.clone(), read_only);
        let release = Release {
            manager: self.me.clone(),
            kind: kind.clone(),
        };
        if let Some(handle) = self.take_idle(&key) {
            return Box::pin(async move {
                Ok(PooledSession {
                    handle,
                    release: Some(release),
                })
            });
        }
        let gate = self.gate();
        let permit = self.permit(priority(&kind));
        let (mut handed, waiter) = self.wait_for_release(key);
        let this = self.clone();
        Box::pin(async move {
            let handed_over = tokio::select! {
                biased;
                // ## m2b-t17-liveness (T15 R2): `close_all` ends the wait.
                () = gate.closed.cancelled() => return Err(closed()),
                Ok(handle) = &mut handed => Ok(handle),
                permit = permit => {
                    // Out of the list first; a session handed meanwhile still wins.
                    drop(waiter);
                    match handed.try_recv() {
                        Ok(handle) => Ok(handle),
                        Err(_) => Err(permit),
                    }
                }
            };
            let handle = match handed_over {
                // Any permit this request won goes straight back to the budget.
                Ok(handle) => handle,
                Err(permit) => {
                    let spec = SessionSpec {
                        kind,
                        read_only,
                        department: Department::Introspection,
                    };
                    this.start(spec, Interaction::Headless, permit, false, &gate)
                        .await?
                }
            };
            Ok(PooledSession {
                handle,
                release: Some(release),
            })
        })
    }

    /// Puts a `pooled` request on the list a released session of `key` is handed to.
    fn wait_for_release(&self, key: PoolKey) -> (oneshot::Receiver<SessionHandle>, WaiterGuard) {
        let (tx, rx) = oneshot::channel();
        let mut st = self.lock();
        let id = st.next_waiter;
        st.next_waiter += 1;
        st.waiting
            .entry(key.clone())
            .or_default()
            .push_back(Waiter { id, tx });
        let guard = WaiterGuard {
            manager: self.me.clone(),
            key,
            id,
        };
        (rx, guard)
    }

    /// The default session, while it can run requests.
    ///
    /// Only the `ConnectionManager` (and this manager's `close_all`) closes it: a client that
    /// closes it directly ends the profile's connection (`Disconnected`), since the profile
    /// lives on its default session (B4; fix round 1, review M2).
    pub fn default_session(&self) -> Option<SessionHandle> {
        self.lock().default.clone().filter(alive)
    }

    /// Sessions open now (idle pooled ones included).
    pub fn open_count(&self) -> usize {
        self.lock().sessions.len()
    }

    // ## m2b-t23-conformance (named exception, controller ruling I1)
    /// Released pooled sessions parked now, waiting to be borrowed again: what a test waits on
    /// instead of guessing when a release's reset has parked its session.
    pub fn parked_count(&self) -> usize {
        self.lock().idle.values().map(Vec::len).sum()
    }

    /// Closes every session and drops the overlay (disconnect, `forget`, quit). A running
    /// request is deactivated first, an open transaction is rolled back, and a connect still
    /// in flight is cancelled and never registers a session or brings the overlay back.
    pub fn close_all(&self) -> BoxFuture<'static, ()> {
        let sessions: Vec<_> = {
            let mut st = self.lock();
            st.epoch += 1;
            st.cancel.cancel();
            st.cancel = CancellationToken::new();
            // Its secrets zeroize as they drop.
            st.overlay = None;
            let sessions = st
                .sessions
                .values()
                .filter_map(|t| Some((t.commands.upgrade()?, t.state.clone())))
                .collect();
            st.default = None;
            st.idle.clear();
            sessions
        };
        Box::pin(async move {
            let closes = sessions
                .into_iter()
                .map(|(commands, state)| close_now(commands, state));
            futures::future::join_all(closes).await;
        })
    }

    /// Replaces the overlay's profile, keeping the consent and the secrets (Q17) while the
    /// profile still names the same destination.
    pub fn update_profile(&self, profile: Arc<Profile>) {
        let mut st = self.lock();
        // ## m2b-t17-liveness (T15 R1): connects in flight lose the consent too.
        if takes_consent_back(&st.profile, &profile) {
            st.consent_gen += 1;
        }
        if let Some(overlay) = st.overlay.as_mut() {
            overlay.replace_profile(profile.clone());
        }
        st.profile = profile;
    }

    /// The overlay, while the profile is connected: a copy (its secrets zeroize on drop).
    pub fn overlay(&self) -> Option<SessionOverlay> {
        self.lock().overlay.as_ref().map(Overlay::view)
    }

    // -----------------------------------------------------------------------------------------

    /// A permit of priority `p`. When the budget is spent the request queues and one idle
    /// pooled session closes to make room.
    fn permit(&self, p: Priority) -> BoxFuture<'static, BudgetPermit> {
        if let Some(permit) = self.budget.try_acquire(p) {
            return Box::pin(async move { permit });
        }
        let waiting = self.budget.acquire(p);
        self.evict_idle();
        waiting
    }

    /// Closes the idle session parked longest.
    fn evict_idle(&self) {
        let victim = {
            let mut st = self.lock();
            let oldest = st
                .idle
                .iter()
                .flat_map(|(key, parked)| parked.iter().map(move |i| (i.token, key)))
                .min_by_key(|(token, _)| *token)
                .map(|(token, key)| (token, key.clone()));
            oldest.and_then(|(token, key)| {
                let parked = st.idle.get_mut(&key)?;
                let at = parked.iter().position(|i| i.token == token)?;
                Some(parked.remove(at).handle)
            })
        };
        if let Some(handle) = victim {
            self.rt.spawn(async move {
                let _ = handle.close(ClosePolicy::RollBack).await;
            });
        }
    }

    /// Single mode: the default session for a request, opened (once, whoever asks) when there
    /// is none that can run. `None` when a `read_only` request would land on a read-write
    /// default: the decision reads the session's own snapshot, not the profile flag (review
    /// I2). The default-session lock is never held while waiting for a permit (review M2).
    async fn shared(
        self: &Arc<Self>,
        read_only: bool,
        ix: Interaction,
        gate: &Gate,
    ) -> Result<Option<SessionHandle>, ConnectError> {
        let serves = |h: SessionHandle| (!read_only || h.state.borrow().read_only).then_some(h);
        let mut one_at_a_time = self.default_lock.lock().await;
        if let Some(handle) = self.default_session() {
            return Ok(serves(handle));
        }
        // A new default is read-only exactly when the profile is (## m3a-t01b-seams: under the
        // manager's policy, P4).
        if read_only && !self.lock().effective_read_only() {
            return Ok(None);
        }
        let permit = match self.budget.try_acquire(Priority::Console) {
            Some(permit) => permit,
            None => {
                drop(one_at_a_time);
                let permit = gate.wait(self.permit(Priority::Console)).await?;
                one_at_a_time = self.default_lock.lock().await;
                if let Some(handle) = self.default_session() {
                    // Someone else opened it meanwhile; this permit goes back.
                    return Ok(serves(handle));
                }
                permit
            }
        };
        let spec = SessionSpec {
            kind: SessionKind::Console(ClientId::DEFAULT),
            read_only: false,
            department: Department::Console,
        };
        let handle = self.start(spec, ix, permit, true, gate).await?;
        drop(one_at_a_time);
        Ok(Some(handle))
    }

    /// Connects and spawns a session for `spec` on `permit`; nothing when `close_all` ran since
    /// the request was made (`gate`, T15 R2).
    async fn start(
        self: &Arc<Self>,
        spec: SessionSpec,
        ix: Interaction,
        permit: BudgetPermit,
        as_default: bool,
        gate: &Gate,
    ) -> Result<SessionHandle, ConnectError> {
        if self.lock().epoch != gate.epoch {
            return Err(closed());
        }
        let (live, profile, epoch) = self
            .connect_live(spec.department, spec.read_only, ix, purpose(&spec.kind))
            .await?;
        let spec = SessionSpec {
            // ## m3a-t01b-seams (R-M3-29)
            read_only: spec.read_only || self.read_only_for(&profile),
            ..spec
        };
        match self.register(
            spec,
            profile,
            live,
            permit,
            epoch,
            as_default,
            Hooked::default(),
        ) {
            Ok(handle) => {
                // ## m2b-t22-live: no monitor watches this session; its gaps go to the sink.
                self.forward_gaps(&handle);
                Ok(handle)
            }
            Err(live) => {
                live.close().await;
                Err(closed())
            }
        }
    }

    /// Runs the pipeline on the core runtime with the overlay (Q17), and takes the connect's
    /// part of the overlay. Dropping the future cancels the connect; a connection that lands
    /// after that is closed.
    async fn connect_live(
        &self,
        department: Department,
        read_only: bool,
        ix: Interaction,
        purpose: Purpose,
    ) -> Result<(LiveConnection, Arc<Profile>, u64), ConnectError> {
        let (profile, secrets, consent, epoch, token, consent_gen) = {
            let st = self.lock();
            let (secrets, consent) = st.overlay.as_ref().map_or((None, false), |o| {
                (Some(copy_secrets(&o.secrets)), o.plaintext_consent)
            });
            (
                st.current(),
                secrets,
                consent,
                st.epoch,
                st.cancel.child_token(),
                st.consent_gen,
            )
        };
        let req = ConnectRequest {
            // ## m3a-t01b-seams (R-M3-29)
            read_only: read_only || self.read_only_for(&profile),
            profile: profile.clone(),
            draft_password: None,
            purpose,
            department,
            prior_rejections: 0,
            draft_ssh: Vec::new(),
            overlay_secrets: secrets,
            plaintext_consent: consent,
        };
        let (tx, rx) = oneshot::channel();
        let deps = self.deps.clone();
        let cancel = token.clone();
        self.rt.spawn(async move {
            let progress = watch::Sender::new(pipeline::fresh_progress());
            let result = pipeline::connect(&deps, req, ix, cancel, progress).await;
            if let Err(Ok(live)) = tx.send(result) {
                // Nobody waits for it any more.
                live.close().await;
            }
        });
        let cancel_on_drop = token.drop_guard();
        let result = rx.await;
        cancel_on_drop.disarm();
        let mut live = match result {
            Ok(Ok(live)) => live,
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err(pipeline::error(ErrClass::Internal, Phase::PostConnect)),
        };
        let used = {
            let mut st = self.lock();
            // After `close_all`, or once the saved profile points elsewhere (review I1),
            // nothing of this connect is kept.
            if st.epoch == epoch {
                st.absorb(&profile, &mut live, consent_gen)
            } else {
                None
            }
        };
        match used {
            Some(profile) => Ok((live, profile, epoch)),
            None => {
                live.close().await;
                Err(closed())
            }
        }
    }

    /// Spawns the session and keeps track of it; its permit goes to the watcher. `Err` (the
    /// connection, to close) when `close_all` ran since the connect started. `hooked`: the
    /// default session's reconnects and liveness (Task 17); none for the others.
    #[allow(clippy::too_many_arguments, reason = "private; two call sites")]
    fn register(
        &self,
        spec: SessionSpec,
        profile: Arc<Profile>,
        live: LiveConnection,
        permit: BudgetPermit,
        epoch: u64,
        as_default: bool,
        hooked: Hooked,
    ) -> Result<SessionHandle, LiveConnection> {
        let mut st = self.lock();
        if st.epoch != epoch {
            return Err(live);
        }
        let hooks = SessionHooks {
            side: Arc::new(Side {
                manager: self.me.clone(),
            }),
            reconnect: hooked.reconnect,
        };
        let handle = spawn_supervised(&self.rt, spec, profile, live, hooks, hooked.liveness);
        st.sessions.insert(
            handle.id,
            Tracked {
                commands: handle.commands.downgrade(),
                state: handle.state(),
            },
        );
        if as_default {
            st.default = Some(handle.clone());
        }
        self.watch(handle.id, handle.state(), permit);
        Ok(handle)
    }

    /// Holds `permit` until session `id` is `Disconnected` or `Broken` or its actor ends, then
    /// forgets the session and frees the permit.
    fn watch(
        &self,
        id: SessionId,
        mut state: watch::Receiver<SessionSnapshot>,
        permit: BudgetPermit,
    ) {
        let manager = self.me.clone();
        self.rt.spawn(async move {
            let _permit = permit;
            while usable(&state.borrow_and_update().state) {
                if state.changed().await.is_err() {
                    break;
                }
            }
            if let Some(manager) = manager.upgrade() {
                manager.lock().forget(id);
            }
        });
    }

    /// An idle session of `key` that can still run; dead ones are dropped on the way.
    fn take_idle(&self, key: &PoolKey) -> Option<SessionHandle> {
        let mut st = self.lock();
        let parked = st.idle.get_mut(key)?;
        while let Some(idle) = parked.pop() {
            if alive(&idle.handle) {
                return Some(idle.handle);
            }
        }
        None
    }

    /// A borrow ended: reset the session, then park it for [`IDLE_CLOSE`].
    fn give_back(&self, handle: SessionHandle, kind: SessionKind) {
        let released_at = tokio::time::Instant::now();
        let epoch = self.lock().epoch;
        let manager = self.me.clone();
        self.rt.spawn(async move {
            if !alive(&handle) {
                // Broken or closed: never reset, never reused; the watcher freed its permit.
                return;
            }
            let reset = handle.submit(Request::Reset).reply.await;
            if !matches!(reset, Ok(Ok(_))) {
                // The reset broke the session and closed its connection (Task 7).
                let _ = handle.close(ClosePolicy::RollBack).await;
                return;
            }
            let key = (kind, handle.state.borrow().read_only);
            let next = manager.upgrade().map_or(AfterReset::Close, |m| {
                m.after_reset(&handle, key.clone(), epoch)
            });
            let token = match next {
                AfterReset::Handed => return,
                AfterReset::Close => {
                    // Its permit goes to the request that waits for one.
                    let _ = handle.close(ClosePolicy::RollBack).await;
                    return;
                }
                AfterReset::Parked(token) => token,
            };
            let id = handle.id;
            // The pool holds it now.
            drop(handle);
            tokio::time::sleep_until(released_at + IDLE_CLOSE).await;
            let expired = manager.upgrade().and_then(|m| m.expire(&key, id, token));
            if let Some(handle) = expired {
                let _ = handle.close(ClosePolicy::RollBack).await;
            }
        });
    }

    /// Where a reset session goes: `Close` when `close_all` ran since its release or the
    /// manager already forgot it.
    ///
    /// Requests waiting for a permit come first (review C1): a higher-priority one (a console,
    /// an editor, a job) gets the permit (the session closes); otherwise a `pooled` request of
    /// the same key gets the session itself; otherwise any other waiter gets the permit. Only
    /// when nobody waits is the session parked.
    fn after_reset(&self, handle: &SessionHandle, key: PoolKey, epoch: u64) -> AfterReset {
        let mut st = self.lock();
        if st.epoch != epoch || !st.sessions.contains_key(&handle.id) {
            return AfterReset::Close;
        }
        let first = self.budget.first_waiter();
        if first.is_some_and(|p| p < priority(&key.0)) {
            return AfterReset::Close;
        }
        if st.hand_to_waiter(&key, handle) {
            return AfterReset::Handed;
        }
        if first.is_some() {
            return AfterReset::Close;
        }
        let token = st.next_token;
        st.next_token += 1;
        st.idle.entry(key).or_default().push(Idle {
            handle: handle.clone(),
            token,
        });
        AfterReset::Parked(token)
    }

    /// Takes session `id` out of the pool if it is still parked from release `token`.
    fn expire(&self, key: &PoolKey, id: SessionId, token: u64) -> Option<SessionHandle> {
        let mut st = self.lock();
        let parked = st.idle.get_mut(key)?;
        let at = parked
            .iter()
            .position(|i| i.handle.id == id && i.token == token)?;
        Some(parked.remove(at).handle)
    }
}

/// Closes a session now: a running request is deactivated, then the session closes with
/// `RollBack`.
async fn close_now(
    commands: mpsc::UnboundedSender<Command>,
    state: watch::Receiver<SessionSnapshot>,
) {
    let running = match state.borrow().state {
        SessionState::Busy { req, .. } | SessionState::Cancelling { req, .. } => Some(req),
        _ => None,
    };
    if let Some(seq) = running {
        let (reply, _) = oneshot::channel();
        let _ = commands.send(Command::Escalate {
            seq,
            escalation: Escalation::Deactivate,
            reply,
        });
    }
    let (reply, closed) = oneshot::channel();
    let close = Command::Close {
        policy: ClosePolicy::RollBack,
        reply,
    };
    if commands.send(close).is_ok() {
        let _ = closed.await;
    }
}

/// The side connections of a manager's sessions (`Department::Cancel`): outside the budget,
/// headless, with the overlay (Q17).
struct Side {
    manager: Weak<SessionManager>,
}

impl SideConnector for Side {
    fn open_side(&self) -> BoxFuture<'static, Result<LiveConnection, ConnectError>> {
        let manager = self.manager.upgrade();
        Box::pin(async move {
            let manager = manager.ok_or_else(closed)?;
            let (live, _, _) = manager
                .connect_live(
                    Department::Cancel,
                    false,
                    Interaction::Headless,
                    Purpose::Main,
                )
                .await?;
            Ok(live)
        })
    }
}

// ## m2b-t17-liveness
// The default session's reconnects and liveness (Task 17), and two carry-ins of the T15
// re-review: R1 (the consent generation) and R2 (`close_all` ends requests still waiting).

/// What the default session gets beyond the others: its reconnects and its liveness.
#[derive(Default)]
struct Hooked {
    reconnect: Option<Arc<dyn Reconnector>>,
    liveness: Option<LivenessPolicy>,
}

/// What a request saw when it was made: a `close_all` since then ends it (T15 R2), whether it
/// waits for a permit, for a released session or for its connect.
struct Gate {
    epoch: u64,
    closed: CancellationToken,
}

impl Gate {
    /// Waits for `permit` unless `close_all` comes first.
    async fn wait(
        &self,
        permit: BoxFuture<'static, BudgetPermit>,
    ) -> Result<BudgetPermit, ConnectError> {
        tokio::select! {
            biased;
            () = self.closed.cancelled() => Err(closed()),
            permit = permit => Ok(permit),
        }
    }
}

/// Whether saving `new` over `old` takes the plaintext consent back: the TLS mode changed, or
/// "allow public plaintext" was unticked (review M3).
fn takes_consent_back(old: &Profile, new: &Profile) -> bool {
    let (old, new) = (&old.tls, &new.tls);
    old.mode != new.mode || (old.allow_public_plaintext && !new.allow_public_plaintext)
}

/// The consent a landing connect keeps (R1): when the consent was taken back since the connect
/// started (`then` is not `now`), none, and no `AllowPublicPlaintext` reaches the overlay.
fn keep_consent(then: u64, now: u64, consent: bool, changes: &mut Vec<ProfileChange>) -> bool {
    if then == now {
        return consent;
    }
    changes.retain(|c| *c != ProfileChange::AllowPublicPlaintext);
    false
}

impl SessionManager {
    fn gate(&self) -> Gate {
        let st = self.lock();
        Gate {
            epoch: st.epoch,
            closed: st.cancel.clone(),
        }
    }

    /// Adopts `live`, the `ConnectionManager`'s own connect of `used` (its progress, draft
    /// password and rejection count stay the manager's, M2a), as the default console session
    /// (B4), with its `reconnect` and `liveness` (Task 17). The connect's part of the overlay is
    /// taken as for any other connect. `Err` (the connection to close, then its permit) when the
    /// saved profile no longer names `used`'s destination (fix round 1, I2: the manager builds
    /// this `SessionManager` from the profile as saved when the connect lands); a consent the
    /// saved profile took back since `used` is dropped, as R1 drops it.
    pub(crate) fn adopt_default(
        &self,
        used: &Profile,
        mut live: LiveConnection,
        permit: BudgetPermit,
        reconnect: Option<Arc<dyn Reconnector>>,
        liveness: Option<LivenessPolicy>,
    ) -> Result<SessionHandle, (LiveConnection, Option<BudgetPermit>)> {
        let (profile, epoch) = {
            let mut st = self.lock();
            if takes_consent_back(used, &st.profile) {
                live.plaintext_consent = false;
                live.profile_changes
                    .retain(|c| *c != ProfileChange::AllowPublicPlaintext);
            }
            let consent_gen = st.consent_gen;
            match st.absorb(used, &mut live, consent_gen) {
                Some(profile) => (profile, st.epoch),
                None => return Err((live, Some(permit))),
            }
        };
        let spec = SessionSpec {
            kind: SessionKind::Console(ClientId::DEFAULT),
            // ## m3a-t01b-seams (R-M3-29)
            read_only: self.read_only_for(&profile),
            department: Department::Console,
        };
        let hooked = Hooked {
            reconnect,
            liveness,
        };
        // A fresh manager: `close_all` cannot have run, so the epoch matches; if it did, the
        // permit is already back.
        self.register(spec, profile, live, permit, epoch, true, hooked)
            .map_err(|live| (live, None))
    }

    // ## m2b-t22-live
    /// Sends the enforcement gaps of every session this manager starts from now on (pooled,
    /// dedicated, a single-mode default it reopens) to `sink`. The `ConnectionManager` sets it
    /// as it builds the manager; the default session it adopts is relayed by its own monitor.
    pub(crate) fn relay_gaps(&self, sink: GapSink) {
        self.lock().gap_sink = Some(sink);
    }

    /// Forwards `handle`'s enforcement gaps to the sink until the session ends. A gap is
    /// emitted before the session publishes `Broken` (T17 I10), so what is still queued when
    /// it ends is drained first.
    fn forward_gaps(&self, handle: &SessionHandle) {
        use tokio::sync::broadcast::error::{RecvError, TryRecvError};

        let Some(sink) = self.lock().gap_sink.clone() else {
            return;
        };
        let mut events = handle.events();
        let mut state = handle.state();
        self.rt.spawn(async move {
            let relay = |event: SessionEvent| {
                if let SessionEvent::EnforcementGap(reason) = event {
                    sink(reason);
                }
            };
            loop {
                tokio::select! {
                    biased;
                    event = events.recv() => match event {
                        Ok(event) => relay(event),
                        Err(RecvError::Lagged(_)) => {}
                        Err(RecvError::Closed) => return,
                    },
                    changed = state.changed() => {
                        if changed.is_ok() && usable(&state.borrow_and_update().state) {
                            continue;
                        }
                        loop {
                            match events.try_recv() {
                                Ok(event) => relay(event),
                                Err(TryRecvError::Lagged(_)) => {}
                                Err(_) => return,
                            }
                        }
                    }
                }
            }
        });
    }

    /// Closes every parked pooled session (fix round 1, review M6): their SSH chain died, so
    /// none may be handed out again.
    pub(crate) fn close_idle(&self) {
        let parked: Vec<SessionHandle> = {
            let mut st = self.lock();
            st.idle
                .drain()
                .flat_map(|(_, parked)| parked.into_iter().map(|i| i.handle))
                .collect()
        };
        for handle in parked {
            self.rt.spawn(async move {
                let _ = handle.close(ClosePolicy::RollBack).await;
            });
        }
    }

    /// One headless connect for a reconnect of the default session (B19), with the overlay
    /// (Q17), and the profile it connected with. It never prompts: a connect that would have
    /// to ask fails with `needs_interaction`.
    pub(crate) async fn reconnect_live(
        &self,
    ) -> Result<(LiveConnection, Arc<Profile>), ConnectError> {
        let (live, profile, _) = self
            .connect_live(
                Department::Console,
                false,
                Interaction::Headless,
                Purpose::Main,
            )
            .await?;
        Ok((live, profile))
    }
}

#[cfg(test)]
mod t17_tests {
    use super::*;

    #[test]
    fn a_connect_older_than_a_consent_take_back_keeps_no_consent() {
        let mut changes = vec![
            ProfileChange::AllowPublicPlaintext,
            ProfileChange::SetCaFile("/ca.pem".into()),
        ];
        assert!(keep_consent(3, 3, true, &mut changes));
        assert_eq!(changes.len(), 2);
        assert!(!keep_consent(2, 3, true, &mut changes));
        assert_eq!(changes, [ProfileChange::SetCaFile("/ca.pem".into())]);
    }

    #[test]
    fn the_consent_is_taken_back_by_a_mode_change_or_an_untick_only() {
        let base = Profile::default();
        let mut ticked = base.clone();
        ticked.tls.allow_public_plaintext = true;
        let mut require = base.clone();
        require.tls.mode = hacivat_net::TlsMode::Require;
        assert!(takes_consent_back(&base, &require));
        assert!(takes_consent_back(&ticked, &base));
        assert!(!takes_consent_back(&base, &ticked));
        let renamed = Profile {
            name: "renamed".into(),
            ..base.clone()
        };
        assert!(!takes_consent_back(&base, &renamed));
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use hacivat_net::CertPin;
    use secrecy::{ExposeSecret, SecretString};

    use super::*;

    fn secret(s: &str) -> SecretString {
        SecretString::from(s.to_string())
    }

    fn pin(endpoint: &str, byte: u8) -> CertPin {
        CertPin {
            endpoint: endpoint.into(),
            sha256: [byte; 32],
        }
    }

    fn profile() -> Arc<Profile> {
        Arc::new(Profile {
            name: "orders".into(),
            ..Profile::default()
        })
    }

    fn secrets(db: Option<&str>, hops: &[(Option<&str>, Option<&str>)]) -> SessionSecrets {
        SessionSecrets {
            db: db.map(secret),
            hops: hops
                .iter()
                .map(|(pw, pp)| HopSecrets {
                    password: pw.map(secret),
                    passphrase: pp.map(secret),
                })
                .collect(),
            tls_key_passphrase: None,
        }
    }

    #[test]
    fn a_connect_applies_its_changes_and_keeps_its_secrets() {
        let mut o = Overlay::new(profile());
        o.absorb(
            &[
                ProfileChange::AddPin(pin("db:5432", 1)),
                ProfileChange::AddPin(pin("db:5432", 1)),
                ProfileChange::SetCaFile(PathBuf::from("/ca.pem")),
                ProfileChange::AllowPublicPlaintext,
            ],
            secrets(Some("s3cret"), &[]),
            true,
        );
        let v = o.view();
        assert_eq!(v.profile.tls.pins, [pin("db:5432", 1)]);
        assert_eq!(v.profile.tls.ca_file, Some(PathBuf::from("/ca.pem")));
        assert!(v.profile.tls.allow_public_plaintext);
        assert!(v.plaintext_consent);
        assert_eq!(v.secrets.db.as_ref().unwrap().expose_secret(), "s3cret");
    }

    #[test]
    fn a_replaced_pin_takes_the_old_ones_place() {
        let mut o = Overlay::new(profile());
        o.absorb(
            &[ProfileChange::AddPin(pin("db:5432", 1))],
            SessionSecrets::default(),
            false,
        );
        o.absorb(
            &[ProfileChange::ReplacePin {
                old: [1; 32],
                new: pin("db:5432", 2),
            }],
            SessionSecrets::default(),
            false,
        );
        assert_eq!(o.view().profile.tls.pins, [pin("db:5432", 2)]);
    }

    #[test]
    fn later_connects_add_secrets_without_losing_earlier_ones() {
        let mut o = Overlay::new(profile());
        o.absorb(&[], secrets(Some("pw1"), &[(Some("hop-pw"), None)]), false);
        // A side connect that used only the hop passphrase.
        o.absorb(&[], secrets(None, &[(None, Some("hop-pp"))]), false);
        let v = o.view();
        assert_eq!(v.secrets.db.as_ref().unwrap().expose_secret(), "pw1");
        let hop = &v.secrets.hops[0];
        assert_eq!(hop.password.as_ref().unwrap().expose_secret(), "hop-pw");
        assert_eq!(hop.passphrase.as_ref().unwrap().expose_secret(), "hop-pp");
        // A newer password replaces the older one.
        o.absorb(&[], secrets(Some("pw2"), &[]), false);
        assert_eq!(o.view().secrets.db.unwrap().expose_secret(), "pw2");
        // Consent stays once given.
        o.absorb(&[], SessionSecrets::default(), true);
        o.absorb(&[], SessionSecrets::default(), false);
        assert!(o.view().plaintext_consent);
    }

    #[test]
    fn update_profile_keeps_the_secrets_for_the_same_destination_only() {
        let base = profile();
        let mut o = Overlay::new(base.clone());
        o.absorb(
            &[ProfileChange::AddPin(pin("db:5432", 1))],
            secrets(Some("s3cret"), &[]),
            true,
        );
        let renamed = Profile {
            name: "renamed".into(),
            ..Profile::clone(&o.view().profile)
        };
        o.replace_profile(Arc::new(renamed));
        let v = o.view();
        assert_eq!(v.profile.name, "renamed");
        assert_eq!(v.profile.tls.pins.len(), 1);
        assert!(v.secrets.db.is_some() && v.plaintext_consent);
        // Another host: the password and the consent were for the old one.
        let moved = Profile {
            endpoint: crate::profile::ProfileEndpoint::Server {
                host: "203.0.113.99".into(),
                port: None,
            },
            ..Profile::clone(&v.profile)
        };
        o.replace_profile(Arc::new(moved));
        let v = o.view();
        assert!(v.secrets.db.is_none());
        assert!(!v.plaintext_consent);
        // Another user too.
        let mut o = Overlay::new(base.clone());
        o.absorb(&[], secrets(Some("s3cret"), &[]), false);
        o.replace_profile(Arc::new(Profile {
            user: Some("admin".into()),
            ..Profile::clone(&base)
        }));
        assert!(o.view().secrets.db.is_none());
    }

    #[test]
    fn the_plaintext_consent_goes_with_a_tls_change_and_the_secrets_stay() {
        let base = profile();
        // The TLS mode changes.
        let mut o = Overlay::new(base.clone());
        o.absorb(&[], secrets(Some("s3cret"), &[]), true);
        let mut p = Profile::clone(&base);
        p.tls.mode = hacivat_net::TlsMode::Require;
        o.replace_profile(Arc::new(p));
        assert!(!o.view().plaintext_consent);
        assert!(o.view().secrets.db.is_some());
        // "Allow public plaintext" is unticked.
        let mut ticked = Profile::clone(&base);
        ticked.tls.allow_public_plaintext = true;
        let mut o = Overlay::new(Arc::new(ticked.clone()));
        o.absorb(&[], SessionSecrets::default(), true);
        ticked.tls.allow_public_plaintext = false;
        o.replace_profile(Arc::new(ticked));
        assert!(!o.view().plaintext_consent);
        // A rename keeps it.
        let mut o = Overlay::new(base.clone());
        o.absorb(&[], SessionSecrets::default(), true);
        o.replace_profile(Arc::new(Profile {
            name: "renamed".into(),
            ..Profile::clone(&base)
        }));
        assert!(o.view().plaintext_consent);
    }

    #[test]
    fn the_overlay_debug_never_shows_a_secret() {
        let mut o = Overlay::new(profile());
        o.absorb(
            &[],
            SessionSecrets {
                db: Some(secret("s3cret")),
                hops: vec![HopSecrets {
                    password: Some(secret("hv_test_pw")),
                    passphrase: Some(secret("hv_test_pp")),
                }],
                tls_key_passphrase: Some(secret("hv_test_tls")),
            },
            false,
        );
        let shown = format!("{:?}", o.view());
        for s in ["s3cret", "hv_test_pw", "hv_test_pp", "hv_test_tls"] {
            assert!(!shown.contains(s), "{s} leaked");
        }
    }

    #[test]
    fn route_kinds_map_to_budget_priorities() {
        assert_eq!(
            priority(&SessionKind::Console(ClientId(1))),
            Priority::Console
        );
        assert_eq!(priority(&SessionKind::Editor("t".into())), Priority::Editor);
        assert_eq!(
            priority(&SessionKind::Job(1, crate::session::JobKind::Export)),
            Priority::Job
        );
        assert_eq!(
            priority(&SessionKind::Introspection { database: None }),
            Priority::Introspection
        );
        assert_eq!(priority(&SessionKind::Service), Priority::Introspection);
    }
}
