//! The session actor (HV-SESS-01; Task 7): one Tokio task per connection that owns the
//! `LiveConnection`, runs requests one at a time from a FIFO queue, and publishes a `watch` of
//! [`SessionSnapshot`] (its generation bumps on every change) and a `broadcast` of
//! [`SessionEvent`].
//!
//! - While a request runs the actor still reads commands: new requests queue, a cancel climbs
//!   the ladder (`cancel`), a close waits for the request to end.
//! - A panic in the driver call is caught: the session becomes `Broken(Internal)`, its
//!   connection is discarded, and the panic message (never the SQL) is logged. Other sessions
//!   have their own tasks and are unaffected.
//! - A broken session is never reused: every later request is answered `NotConnected`.
//! - Spawning runs neither the environment plan nor a schema probe (Q27): connect already set
//!   the identity and read-only. The one exception is MySQL's `SET autocommit = 0` for a
//!   read-write session that starts Manual, so the server never autocommits under a Manual
//!   badge.
//! - A read-only session is always Auto (Q14) and runs every statement through
//!   `readonly::run_guarded`, which replays the environment plan after its reset (Q15).
//!
//! M2b Task 17 (`// ## m2b-t17-liveness` blocks):
//! - **Liveness** (HV-CONN-04, B18). A session spawned with a [`LivenessPolicy`] ticks its
//!   [`Supervisor`] on a 1 s `interval` (`MissedTickBehavior::Skip`) while it is idle. A `Ping`
//!   is `RawConn::ping` within `LivenessPolicy::ping_timeout`, still serving commands (a close
//!   aborts it); a `Suspect` ticks again at once; `Disconnect` closes the session gracefully
//!   (`Disconnected`); `IdleTransaction` emits `SessionEvent::IdleTransaction`. A user request
//!   (a user statement, a transaction command, a schema switch) is activity; pings, internal
//!   statements and resets are not.
//! - **Recovery** (HV-CONN-05, B19). When the connection is lost with a recovery case
//!   (`is_recovery_case`) and the session has `SessionHooks::reconnect`:
//!   - a loss is an I/O, protocol, timeout or TLS failure, or a server error that ends the
//!     connection (`server.shutdown`, `server.starting`, MySQL's server-gone codes; fix round
//!     2, I6);
//!   - with a transaction open by the last status seen before the loss (never the status a
//!     driver reports after it; fix rounds 2 and 3, C2 and R4), or the request in flight
//!     possibly opening one (a `BEGIN`, `START`, `XA` or `SET autocommit` in it, a user
//!     statement of a Manual session), or the user's own MySQL `autocommit = 0` in force (I8),
//!     nothing is replayed (fix round 1, C1): the session emits `SessionEvent::Lost` and breaks
//!     (the profile is `Lost`, with Retry);
//!   - otherwise (an idle session) it is not broken: it publishes `Connecting` (so it keeps
//!     its budget permit, T15 carry-in), notes what the loss took (`lost_state`), answers the
//!     request that was running (a user statement `ConnectionLost { outcome_unknown: true }`,
//!     never re-run; an internal idempotent one is re-queued once at the head) and the queued
//!     ones that never ran (`ConnectionLost { outcome_unknown: false }`; internal idempotent
//!     ones stay queued), and calls the reconnector once (it runs the backoff);
//!   - while it recovers, every request is answered `ConnectionLost { outcome_unknown: false
//!     }` at once (only internal idempotent reads wait), so nothing sent for the lost
//!     connection runs on the new one (C1), and a close ends the session at once;
//!   - on success the connection is replaced in place and the environment plan replayed
//!     (identity, read-only setup, last schema; MySQL's `autocommit = 0` when Manual), within
//!     the profile's connect timeout and still serving commands; a replay failure continues the
//!     same backoff (I1). Mode and isolation are kept, and the session emits
//!     `SessionEvent::Lost`, publishes `Idle`, then emits `SessionEvent::Reconnected`. A failed
//!     reconnect breaks the session with its class.
//! - The supervisor's idle disconnect emits `SessionEvent::IdleDisconnected` before the session
//!   publishes `Disconnected`.

use std::any::Any;
use std::collections::VecDeque;
use std::fmt;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::task::{Context, Poll};
use std::time::{Instant, SystemTime};

use futures::FutureExt as _;
use futures::future::BoxFuture;
use hacivat_driver::{
    CancelHandle, ExecOpts, ExecSummary, Phase, RawConn, RawError, RawErrorKind, TxStatus,
    carries_secret, classify,
};
use hacivat_proto::{AccessLabel, DbmsKind, ErrClass};
use tokio::runtime::Handle;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::{JoinError, JoinHandle};

use crate::pipeline::{ConnectError, LiveConnection};
use crate::profile::Profile;
use crate::session::cancel::{
    CANCEL_WAIT, SIDE_PREWARM, SLOW_CANCEL, kill_query_sql, terminate_sql,
};
// ## m2b-t17-liveness
use crate::session::liveness::{LivenessAction, LivenessPolicy, Supervisor};
use crate::session::readonly::run_guarded;
// ## m2b-t17-liveness
use crate::session::recovery::{
    LostState, Tracked, autocommit_set, connection_ending, is_recovery_case, lost_state,
    touches_tx, track_statement,
};
use crate::session::schema::{Family, family, first_keyword};
use crate::session::{
    CancelOutcome, CancelState, ClosePolicy, CloseRefused, Command, EnvPlan, Escalation, ExecError,
    ExecErrorKind, ExecOrigin, IsolationLevel, Reply, Request, SchemaCtx, SessionEvent,
    SessionHandle, SessionId, SessionSnapshot, SessionSpec, SessionState, TxCommand, TxError,
    TxMode, TxState, begin_sql, build_env_plan, default_mode, needs_schema_reprobe,
    parse_schema_probe, schema_probe_sql, switch_schema_sql,
};

/// Opens the side connection a cancel or a kill needs (`Department::Cancel`). Task 15's
/// implementation connects with the session overlay (Q17).
pub trait SideConnector: Send + Sync {
    fn open_side(&self) -> BoxFuture<'static, Result<LiveConnection, ConnectError>>;
}

/// Reconnects a lost session (Task 17); `attempt` counts from 1.
///
/// The actor calls it once per loss with `attempt` 1, synchronously before it answers the
/// request that found the loss; the future runs the whole recovery (the backoff, B19) and
/// resolves with a new connection, or with the error that ended the recovery.
pub trait Reconnector: Send + Sync {
    fn reconnect(&self, attempt: u32) -> BoxFuture<'static, Result<LiveConnection, ConnectError>>;

    // ## m2b-t17-liveness (fix round 1, I1)
    /// The connection the last `reconnect` gave failed its environment replay with `class` (a
    /// recovery case) before the session could use it. The actor calls `reconnect` again right
    /// after: that call continues this loss's backoff and give-up clock instead of starting
    /// over. The default does nothing.
    fn replay_failed(&self, _class: ErrClass) {}
}

/// What an actor may call outside itself.
#[derive(Clone)]
pub struct SessionHooks {
    pub side: Arc<dyn SideConnector>,
    /// `None`: a lost session stays lost (Task 17 supplies it).
    pub reconnect: Option<Arc<dyn Reconnector>>,
}

impl fmt::Debug for SessionHooks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionHooks")
            .field("reconnect", &self.reconnect.is_some())
            .finish_non_exhaustive()
    }
}

/// The next session ID.
fn next_id() -> SessionId {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    SessionId(NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst))
}

/// How many events a slow subscriber may lag behind.
const EVENT_CAPACITY: usize = 64;

/// First keywords of transaction control: Manual mode begins nothing before them, and they are
/// not counted as statements of the open transaction.
const TX_CONTROL: [&str; 6] = ["BEGIN", "START", "COMMIT", "END", "ROLLBACK", "ABORT"];

/// Starts the actor of `live` on `rt` (the module docs). The session is read-only when `spec`
/// or `profile` says so.
pub fn spawn_session(
    rt: &Handle,
    spec: SessionSpec,
    profile: Arc<Profile>,
    live: LiveConnection,
    hooks: SessionHooks,
) -> SessionHandle {
    spawn_supervised(rt, spec, profile, live, hooks, None)
}

// ## m2b-t17-liveness
/// [`spawn_session`] with `liveness` ticking a [`Supervisor`] (the module docs); `None` never
/// ticks.
pub(crate) fn spawn_supervised(
    rt: &Handle,
    spec: SessionSpec,
    profile: Arc<Profile>,
    mut live: LiveConnection,
    hooks: SessionHooks,
    liveness: Option<LivenessPolicy>,
) -> SessionHandle {
    let read_only = spec.read_only || profile.read_only;
    let kind = live.raw().kind();
    let (initial_mode, initial_isolation) = if read_only {
        (TxMode::Auto, IsolationLevel::Default)
    } else {
        // Smart needs the classifier (M3); until then it starts as the safer Manual.
        let mode = match profile
            .tx_mode
            .unwrap_or_else(|| default_mode(profile.environment))
        {
            TxMode::Smart => TxMode::Manual,
            mode => mode,
        };
        (mode, profile.isolation.unwrap_or_default())
    };
    let mut tx = TxState {
        mode: initial_mode,
        isolation: initial_isolation,
        ..TxState::default()
    };
    tx.observe(live.raw().tx_status(), false);
    let snapshot = SessionSnapshot {
        state: SessionState::Idle,
        tx: tx.clone(),
        schema: None,
        cancel: CancelState::Idle,
        read_only,
        // Review M4: the label follows the session; a read-only session is never labelled
        // read-write, whatever the connect was asked for.
        access: if read_only && live.access == AccessLabel::ReadWrite {
            AccessLabel::ReadOnlyGuarded
        } else {
            live.access
        },
        generation: 0,
    };
    let id = next_id();
    let (commands, receiver) = mpsc::unbounded_channel();
    let state = Arc::new(watch::Sender::new(snapshot));
    let events = broadcast::Sender::new(EVENT_CAPACITY);
    let handle = SessionHandle {
        id,
        kind: spec.kind,
        commands,
        state: state.subscribe(),
        events: events.clone(),
        next_seq: Arc::new(AtomicU64::new(1)),
    };
    let core = Core {
        kind,
        profile,
        read_only,
        live: Some(live),
        tx,
        schema: None,
        initial_mode,
        initial_isolation,
        broken: None,
        last_class: None,
        tracked: Tracked::default(),
        lost: None,
        autocommit_off: false,
    };
    let ctl = Ctl {
        id,
        state: state.clone(),
        events,
        commands: receiver,
        queue: VecDeque::new(),
        hooks,
        liveness: None,
    };
    rt.spawn(async move {
        let mut ctl = ctl;
        // On the runtime: an interval needs its timer.
        ctl.liveness = liveness.map(Liveness::new);
        // The driver calls have their own guard (`run_one`); this one keeps a defect in the
        // actor itself from leaving a session that claims to be usable.
        if AssertUnwindSafe(run(core, ctl))
            .catch_unwind()
            .await
            .is_err()
        {
            tracing::error!(session = id.0, "the session actor panicked");
            state.send_modify(|s| {
                s.state = SessionState::Broken(ErrClass::Internal);
                s.generation = s.generation.wrapping_add(1);
            });
        }
    });
    handle
}

// ---------------------------------------------------------------------------------------------
// The connection side: what runs one request.

/// The connection and what the session knows about it; borrowed by the running request.
pub(crate) struct Core {
    pub(crate) kind: DbmsKind,
    profile: Arc<Profile>,
    read_only: bool,
    live: Option<LiveConnection>,
    pub(crate) tx: TxState,
    schema: Option<SchemaCtx>,
    pub(crate) initial_mode: TxMode,
    pub(crate) initial_isolation: IsolationLevel,
    /// Set once the connection must not be used again.
    pub(crate) broken: Option<ErrClass>,
    /// The class of the last driver error.
    pub(crate) last_class: Option<ErrClass>,
    // ## m2b-t17-liveness
    /// What the user's statements did since the connection was made (B19).
    tracked: Tracked,
    /// What the last loss took, until the reconnect reports it.
    lost: Option<LostState>,
    /// MySQL: the user's own `SET autocommit = 0` is in force (fix round 3, I8). Every
    /// statement then runs in an implicit transaction, as in Manual mode.
    autocommit_off: bool,
}

fn not_connected() -> ExecError {
    ExecError::of(ExecErrorKind::NotConnected)
}

fn tx_error(e: TxError) -> ExecError {
    ExecError::of(ExecErrorKind::Tx(e))
}

impl Core {
    pub(crate) fn conn(&mut self) -> Result<&mut dyn RawConn, ExecError> {
        self.live
            .as_mut()
            .map(LiveConnection::raw)
            .ok_or_else(not_connected)
    }

    /// Reads the protocol's transaction status after a round trip. Nothing once the session is
    /// broken: a driver may report `Failed` for a connection it found closed, which says
    /// nothing about the transaction (Task 17 fix round 2, C2).
    pub(crate) fn observe(&mut self, statement: bool) {
        if self.broken.is_some() {
            return;
        }
        if let Some(live) = self.live.as_mut() {
            let status = live.raw().tx_status();
            self.tx.observe(status, statement);
        }
    }

    /// The reply of a request without rows.
    pub(crate) fn summary(&self) -> ExecSummary {
        ExecSummary {
            tx: self.tx.status,
            ..ExecSummary::default()
        }
    }

    /// The session's current environment plan (its last schema included).
    pub(crate) fn env_plan(&self) -> EnvPlan {
        build_env_plan(
            self.kind,
            &self.profile,
            self.read_only,
            self.schema.as_ref(),
        )
    }

    /// Classifies a driver error; a lost connection breaks the session. `sent`: the statement
    /// may have reached the server.
    ///
    /// Task 17 fix round 2 (I6): a server error that ends the connection (`server.shutdown`,
    /// `server.starting`, `network.*` after connect; MySQL's server-gone codes) is a lost
    /// connection too, so the recovery starts on the request it answered, with the server's
    /// message kept.
    pub(crate) fn raw_error(&mut self, e: &RawError, sent: bool) -> ExecError {
        let class = classify(self.kind, Phase::PostConnect, e).class;
        self.last_class = Some(class);
        let kind = match &e.kind {
            RawErrorKind::Cancelled => ExecErrorKind::Cancelled,
            RawErrorKind::Db => match connection_ending(self.kind, e, class) {
                Some(lost) => {
                    self.last_class = Some(lost);
                    self.broken.get_or_insert(lost);
                    ExecErrorKind::ConnectionLost {
                        outcome_unknown: sent,
                    }
                }
                None => ExecErrorKind::Db,
            },
            RawErrorKind::Io(_)
            | RawErrorKind::Protocol
            | RawErrorKind::Timeout
            | RawErrorKind::Tls(_) => {
                self.broken.get_or_insert(class);
                ExecErrorKind::ConnectionLost {
                    outcome_unknown: sent,
                }
            }
            _ => ExecErrorKind::Internal,
        };
        ExecError {
            kind,
            server: e.server.clone(),
        }
    }

    /// Runs one trusted control statement.
    pub(crate) async fn control(&mut self, sql: &str) -> Result<(), ExecError> {
        let ran = self.conn()?.batch(sql).await;
        // The error first: a lost connection's status is never read (C2).
        let ran = ran.map_err(|e| self.raw_error(&e, true));
        self.observe(false);
        ran
    }

    async fn handle(&mut self, request: Request) -> Reply {
        if self.live.is_none() {
            return Err(not_connected());
        }
        match request {
            Request::Exec { sql, opts, origin } => self.exec(&sql, opts, origin).await,
            Request::Tx(cmd) => self.tx_command(cmd).await,
            Request::SwitchSchema(name) => self.switch_schema(&name).await,
            Request::Ping => {
                let pinged = self.conn()?.ping().await;
                match pinged {
                    Ok(_) => Ok(self.summary()),
                    Err(e) => Err(self.raw_error(&e, false)),
                }
            }
            Request::Reset => {
                let reset = self.reset().await;
                // ## m2b-t17-liveness: a reset session starts with nothing to lose.
                if reset.is_ok() {
                    self.tracked = Tracked::default();
                    // `COM_RESET_CONNECTION` restores the session's `autocommit`.
                    self.autocommit_off = false;
                }
                reset
            }
        }
    }

    async fn exec(&mut self, sql: &str, opts: ExecOpts, origin: ExecOrigin) -> Reply {
        let keyword = first_keyword(sql);
        // Advisory (first keyword only): the server refuses anyway.
        if self.tx.status == TxStatus::Failed && keyword != "ROLLBACK" {
            return Err(tx_error(TxError::FailedNeedsRollback));
        }
        let control = TX_CONTROL.contains(&keyword.as_str());
        let mut summary = if self.read_only {
            let replay = self.env_plan().replay();
            let kind = self.kind;
            let ran = run_guarded(self.conn()?, kind, sql, opts, &replay).await;
            if let Err(e) = &ran
                && matches!(e.kind, ExecErrorKind::ConnectionLost { .. })
            {
                self.broken.get_or_insert(ErrClass::NetworkReset);
            }
            self.observe(false);
            ran?
        } else {
            // Hacivat's own statements never open the user's transaction (review M9).
            if origin == ExecOrigin::User
                && self.tx.mode == TxMode::Manual
                && self.tx.status == TxStatus::Idle
                && !control
            {
                for statement in self.manual_begin(&keyword) {
                    self.control(&statement).await?;
                }
            }
            let ran = self.conn()?.execute(sql, opts).await;
            let ran = ran.map_err(|e| self.raw_error(&e, true));
            self.observe(!control);
            ran?
        };
        // ## m2b-t17-liveness: what a reconnect would lose (B19).
        if origin == ExecOrigin::User {
            track_statement(&mut self.tracked, sql);
            if family(self.kind) == Family::MySql
                && let Some(on) = autocommit_set(sql)
            {
                self.autocommit_off = !on;
            }
        }
        if origin == ExecOrigin::User && needs_schema_reprobe(sql) {
            self.reprobe().await;
        }
        summary.tx = self.tx.status;
        Ok(summary)
    }

    /// What Manual mode sends before a statement while no transaction is open. MySQL needs
    /// nothing (`autocommit = 0` opens one); SQLite takes the write lock at once (`BEGIN
    /// IMMEDIATE`) unless the statement reads, by its first keyword.
    fn manual_begin(&self, keyword: &str) -> Vec<String> {
        match family(self.kind) {
            Family::MySql => Vec::new(),
            Family::Sqlite if !matches!(keyword, "SELECT" | "WITH") => {
                vec!["BEGIN IMMEDIATE".to_string()]
            }
            _ => begin_sql(self.kind, self.tx.isolation, false),
        }
    }

    /// Probes the schema context again (Q27); a failed probe keeps the last one.
    async fn reprobe(&mut self) {
        let sql = schema_probe_sql(self.kind);
        if sql.is_empty() {
            return;
        }
        let Ok(conn) = self.conn() else {
            return;
        };
        let probed = conn.execute(sql, ExecOpts::default()).await;
        match probed {
            Ok(answer) => {
                self.observe(false);
                let ctx = parse_schema_probe(self.kind, &answer);
                self.schema = (ctx != SchemaCtx::default()).then_some(ctx);
            }
            Err(e) => {
                let e = self.raw_error(&e, false);
                self.observe(false);
                tracing::debug!(kind = ?e.kind, "the schema probe failed");
            }
        }
    }

    async fn tx_command(&mut self, cmd: TxCommand) -> Reply {
        let mysql = family(self.kind) == Family::MySql;
        match cmd {
            TxCommand::SetMode(mode) => {
                if self.read_only {
                    // Q14: a read-only session is always Auto.
                    return if mode == TxMode::Auto {
                        Ok(self.summary())
                    } else {
                        Err(tx_error(TxError::Unsupported))
                    };
                }
                match mode {
                    TxMode::Smart => return Err(tx_error(TxError::NeedsClassifier)),
                    TxMode::Manual => {
                        if self.tx.mode != TxMode::Manual && mysql {
                            self.control("SET autocommit = 0").await?;
                        }
                    }
                    TxMode::Auto => {
                        if self.tx.status != TxStatus::Idle {
                            return Err(tx_error(TxError::InProgress));
                        }
                        if self.tx.mode == TxMode::Manual && mysql {
                            self.control("SET autocommit = 1").await?;
                            // Whatever the user had set is overridden now.
                            self.autocommit_off = false;
                        }
                    }
                }
                self.tx.mode = mode;
            }
            TxCommand::SetIsolation(level) => {
                if self.tx.status != TxStatus::Idle {
                    return Err(tx_error(TxError::InProgress));
                }
                self.tx.isolation = level;
            }
            TxCommand::Begin => {
                if self.read_only {
                    return Err(tx_error(TxError::Unsupported));
                }
                if self.tx.status != TxStatus::Idle {
                    return Err(tx_error(TxError::InProgress));
                }
                for statement in begin_sql(self.kind, self.tx.isolation, false) {
                    self.control(&statement).await?;
                }
            }
            // Q14: no manual transaction control on a read-only session (review M8).
            TxCommand::Commit | TxCommand::Rollback if self.read_only => {
                return Err(tx_error(TxError::Unsupported));
            }
            TxCommand::Commit => {
                self.control("COMMIT").await?;
                self.reprobe().await;
            }
            TxCommand::Rollback => {
                self.control("ROLLBACK").await?;
                self.reprobe().await;
            }
        }
        Ok(self.summary())
    }

    async fn switch_schema(&mut self, name: &str) -> Reply {
        if matches!(family(self.kind), Family::Sqlite | Family::Other) {
            return Err(tx_error(TxError::Unsupported));
        }
        let sql = switch_schema_sql(self.kind, name);
        if sql.is_empty() {
            // An empty name or a NUL: a caller's defect, never sent.
            return Err(ExecError::of(ExecErrorKind::Internal));
        }
        self.control(&sql).await?;
        self.reprobe().await;
        Ok(self.summary())
    }

    /// Closes the connection (its cleanup guards run); nothing when already closed.
    async fn close(&mut self) {
        if let Some(live) = self.live.take()
            && AssertUnwindSafe(live.close()).catch_unwind().await.is_err()
        {
            tracing::error!("closing a session's connection panicked");
        }
    }

    // ## m2b-t17-liveness

    /// Notes what the loss took: the transaction is gone, its mode and isolation stay (B19).
    /// `before` is the last status observed before the loss (fix rounds 2 and 3, C2 and R4): a
    /// driver's status after a loss says nothing about the transaction.
    fn note_loss(&mut self, before: TxStatus) {
        let tracked = Tracked {
            tx: before,
            ..self.tracked.clone()
        };
        self.lost = Some(lost_state(&tracked));
        self.tracked = Tracked::default();
        self.tx = TxState {
            mode: self.tx.mode,
            isolation: self.tx.isolation,
            ..TxState::default()
        };
    }

    /// Puts a reconnect's connection in place of the lost one; its access label.
    fn install(&mut self, mut live: LiveConnection) -> AccessLabel {
        let access = if self.read_only && live.access == AccessLabel::ReadWrite {
            AccessLabel::ReadOnlyGuarded
        } else {
            live.access
        };
        let status = live.raw().tx_status();
        self.live = Some(live);
        self.broken = None;
        self.last_class = None;
        self.tx.observe(status, false);
        access
    }

    /// Replays the environment after a reconnect (B19): the plan (identity, read-only setup,
    /// the last schema), then MySQL's `autocommit = 0` for a Manual session.
    async fn replay(&mut self) -> Result<(), ExecError> {
        for statement in self.env_plan().statements() {
            self.control(&statement).await?;
        }
        if !self.read_only && self.tx.mode == TxMode::Manual && family(self.kind) == Family::MySql {
            self.control("SET autocommit = 0").await?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// The control side: the queue, the snapshot, the events.

struct Queued {
    seq: u64,
    request: Request,
    reply: oneshot::Sender<Reply>,
    // ## m2b-t17-liveness
    /// Already re-queued once after a lost connection (B19: internal idempotent reads only).
    retried: bool,
}

type CloseReply = oneshot::Sender<Result<(), CloseRefused>>;

/// What waits its turn: requests and closes, in the order they came (review M2: a close never
/// jumps ahead of a request queued before it).
enum Job {
    Request(Queued),
    Close(ClosePolicy, CloseReply),
}

struct Ctl {
    id: SessionId,
    state: Arc<watch::Sender<SessionSnapshot>>,
    events: broadcast::Sender<SessionEvent>,
    commands: mpsc::UnboundedReceiver<Command>,
    queue: VecDeque<Job>,
    hooks: SessionHooks,
    // ## m2b-t17-liveness
    liveness: Option<Liveness>,
}

// ## m2b-t17-liveness

/// A session's liveness: its supervisor and the interval that ticks it.
struct Liveness {
    sup: Supervisor,
    ticker: tokio::time::Interval,
}

impl Liveness {
    fn new(policy: LivenessPolicy) -> Liveness {
        let mut ticker = tokio::time::interval(policy.tick);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let (now, wall) = clocks();
        Liveness {
            sup: Supervisor::new(policy, now, wall),
            ticker,
        }
    }
}

/// Both clocks now. The monotonic one is Tokio's, so a paused test clock drives it.
fn clocks() -> (Instant, SystemTime) {
    (tokio::time::Instant::now().into_std(), SystemTime::now())
}

/// Whether `request` is the user's activity (B18): pings, internal statements and resets are
/// not.
fn is_activity(request: &Request) -> bool {
    matches!(
        request,
        Request::Exec {
            origin: ExecOrigin::User,
            ..
        } | Request::Tx(_)
            | Request::SwitchSchema(_)
    )
}

/// Whether `request` may run again after a reconnect (B19): an internal idempotent statement.
fn is_retryable(request: &Request) -> bool {
    matches!(
        request,
        Request::Exec {
            origin: ExecOrigin::Internal { idempotent: true },
            ..
        }
    )
}

/// Waits for the next tick; never, without liveness.
async fn next_tick(liveness: &mut Option<Liveness>) {
    match liveness {
        Some(l) => {
            l.ticker.tick().await;
        }
        None => std::future::pending().await,
    }
}

/// How a request (or a ping) left the session.
enum Flow {
    Go,
    /// Every handle is gone, or the session closed.
    Stop,
    /// The connection was lost; the reconnect runs.
    Recover(BoxFuture<'static, Result<LiveConnection, ConnectError>>),
}

impl Ctl {
    /// Changes the snapshot and bumps its generation.
    fn publish(&self, f: impl FnOnce(&mut SessionSnapshot)) {
        self.state.send_modify(|s| {
            f(s);
            s.generation = s.generation.wrapping_add(1);
        });
    }

    fn emit(&self, event: SessionEvent) {
        // No subscriber is fine.
        let _ = self.events.send(event);
    }

    /// Answers every queued request with `kind`; queued closes keep their turn.
    fn drain(&mut self, kind: &ExecErrorKind) {
        for job in std::mem::take(&mut self.queue) {
            match job {
                Job::Request(q) => {
                    let _ = q.reply.send(Err(ExecError::of(kind.clone())));
                }
                close @ Job::Close(..) => self.queue.push_back(close),
            }
        }
    }

    // ## m2b-t17-liveness
    /// The connection is lost while these wait: a request that never ran is answered
    /// `ConnectionLost { outcome_unknown: false }` (it never runs on the new connection, where
    /// the transaction it belonged to is gone), except an internal idempotent one, which waits
    /// for the reconnect. Queued closes keep their turn.
    fn drain_lost(&mut self) {
        for job in std::mem::take(&mut self.queue) {
            match job {
                Job::Request(q) if is_retryable(&q.request) => {
                    self.queue.push_back(Job::Request(q));
                }
                Job::Request(q) => {
                    let lost = ExecErrorKind::ConnectionLost {
                        outcome_unknown: false,
                    };
                    let _ = q.reply.send(Err(ExecError::of(lost)));
                }
                close @ Job::Close(..) => self.queue.push_back(close),
            }
        }
    }

    /// Work outside the idle loop ended: a user request was activity; anything else only moves
    /// the supervisor's clocks past the time it took.
    fn after_work(&mut self, activity: bool, tx: TxStatus) {
        if let Some(l) = self.liveness.as_mut() {
            let (now, wall) = clocks();
            if activity {
                let _ = l.sup.tick(now, wall, true, tx);
            } else {
                l.sup.resync(now, wall);
            }
        }
    }

    /// A command while no request runs.
    fn idle_command(&mut self, cmd: Command) {
        match cmd {
            Command::Submit {
                seq,
                request,
                reply,
            } => self.queue.push_back(Job::Request(Queued {
                seq,
                request,
                reply,
                retried: false,
            })),
            // Q18: only a running request can be cancelled.
            Command::Cancel { reply, .. } | Command::Escalate { reply, .. } => {
                let _ = reply.send(CancelOutcome::AlreadyFinished);
            }
            Command::Close { policy, reply } => self.queue.push_back(Job::Close(policy, reply)),
        }
    }

    /// Stops taking commands and answers everything still waiting: the session is closed.
    fn shut(&mut self) {
        self.commands.close();
        while let Ok(cmd) = self.commands.try_recv() {
            match cmd {
                Command::Submit { reply, .. } => {
                    let _ = reply.send(Err(not_connected()));
                }
                Command::Cancel { reply, .. } | Command::Escalate { reply, .. } => {
                    let _ = reply.send(CancelOutcome::Disconnected);
                }
                Command::Close { reply, .. } => {
                    let _ = reply.send(Ok(()));
                }
            }
        }
        for job in self.queue.drain(..) {
            match job {
                Job::Request(q) => {
                    let _ = q.reply.send(Err(not_connected()));
                }
                Job::Close(_, reply) => {
                    let _ = reply.send(Ok(()));
                }
            }
        }
    }
}

/// Breaks the session now: closes the connection, answers the queued requests
/// `NotConnected` and publishes `Broken(class)`.
async fn break_now(core: &mut Core, ctl: &mut Ctl, class: ErrClass) {
    core.broken = Some(class);
    core.close().await;
    ctl.drain(&ExecErrorKind::NotConnected);
    ctl.publish(|s| {
        s.state = SessionState::Broken(class);
        s.cancel = CancelState::Idle;
    });
}

async fn run(mut core: Core, mut ctl: Ctl) {
    // Mode enforcement, not the environment plan (Q27 clarification): a Manual MySQL session
    // must not autocommit. A driver panic here breaks the session like any other (review M5).
    if !core.read_only && core.tx.mode == TxMode::Manual && family(core.kind) == Family::MySql {
        let set = AssertUnwindSafe(core.control("SET autocommit = 0"))
            .catch_unwind()
            .await;
        let class = match set {
            Ok(Ok(())) => None,
            Ok(Err(_)) => Some(
                core.broken
                    .or(core.last_class)
                    .unwrap_or(ErrClass::Internal),
            ),
            Err(_) => Some(ErrClass::Internal),
        };
        if let Some(class) = class {
            break_now(&mut core, &mut ctl, class).await;
        }
    }
    loop {
        match ctl.queue.pop_front() {
            Some(Job::Close(policy, reply)) => {
                if close(&mut core, &mut ctl, policy, reply).await {
                    return;
                }
                continue;
            }
            Some(Job::Request(job)) => {
                // ## m2b-t17-liveness
                let activity = is_activity(&job.request);
                let flow = run_one(&mut core, &mut ctl, job).await;
                if !follow(&mut core, &mut ctl, flow).await {
                    return;
                }
                ctl.after_work(activity, core.tx.status);
                continue;
            }
            None => {}
        }
        // ## m2b-t17-liveness: commands first, so a request that comes with a tick runs.
        tokio::select! {
            biased;
            cmd = ctl.commands.recv() => match cmd {
                Some(cmd) => ctl.idle_command(cmd),
                // Every handle is gone.
                None => {
                    core.close().await;
                    return;
                }
            },
            () = next_tick(&mut ctl.liveness), if ctl.liveness.is_some() => {
                let flow = on_tick(&mut core, &mut ctl).await;
                if !follow(&mut core, &mut ctl, flow).await {
                    return;
                }
                ctl.after_work(false, core.tx.status);
            }
        }
    }
}

// ## m2b-t17-liveness

/// Acts on how a request or a tick ended; false when the actor is done.
async fn follow(core: &mut Core, ctl: &mut Ctl, flow: Flow) -> bool {
    match flow {
        Flow::Go => true,
        Flow::Stop => false,
        Flow::Recover(reconnect) => recover(core, ctl, reconnect).await,
    }
}

/// One supervisor tick while the session is idle (the module docs).
async fn on_tick(core: &mut Core, ctl: &mut Ctl) -> Flow {
    if core.live.is_none() {
        // Broken: nothing to watch.
        return Flow::Go;
    }
    let Some(l) = ctl.liveness.as_mut() else {
        return Flow::Go;
    };
    let (now, wall) = clocks();
    let mut action = l.sup.tick(now, wall, false, core.tx.status);
    if action == Some(LivenessAction::Suspect) {
        // Q31: one check at once, never one per missed tick.
        tracing::debug!(
            session = ctl.id.0,
            "the clocks jumped; checking the connection"
        );
        action = l.sup.tick(now, wall, false, core.tx.status);
    }
    let minutes = l.sup.policy().idle_minutes();
    let timeout = l.sup.policy().ping_timeout();
    match action {
        None | Some(LivenessAction::Suspect) => Flow::Go,
        Some(LivenessAction::IdleTransaction) => {
            ctl.emit(SessionEvent::IdleTransaction { minutes });
            Flow::Go
        }
        Some(LivenessAction::Disconnect) => {
            tracing::debug!(session = ctl.id.0, "idle disconnect");
            // Before `Disconnected`: the owner tells it from any other close (review M2).
            ctl.emit(SessionEvent::IdleDisconnected);
            // Nothing is open (the supervisor never disconnects an open transaction).
            let (reply, _) = oneshot::channel();
            close(core, ctl, ClosePolicy::RollBack, reply).await;
            Flow::Stop
        }
        Some(LivenessAction::Ping) => keepalive(core, ctl, timeout).await,
    }
}

/// How a keep-alive ping ended.
enum Pinged {
    Answered(Result<Result<std::time::Duration, RawError>, tokio::time::error::Elapsed>),
    /// A close came: it runs next, the ping is dropped.
    Aborted,
    Abandoned,
}

/// A keep-alive ping within `timeout`, serving commands meanwhile. A failure is a loss.
async fn keepalive(core: &mut Core, ctl: &mut Ctl, timeout: std::time::Duration) -> Flow {
    let pinged = {
        let Ok(conn) = core.conn() else {
            return Flow::Go;
        };
        let ping = tokio::time::timeout(timeout, conn.ping());
        tokio::pin!(ping);
        loop {
            tokio::select! {
                biased;
                answered = &mut ping => break Pinged::Answered(answered),
                cmd = ctl.commands.recv() => match cmd {
                    None => break Pinged::Abandoned,
                    Some(Command::Close { policy, reply }) => {
                        ctl.queue.push_back(Job::Close(policy, reply));
                        break Pinged::Aborted;
                    }
                    Some(other) => ctl.idle_command(other),
                },
            }
        }
    };
    match pinged {
        Pinged::Abandoned => {
            core.close().await;
            Flow::Stop
        }
        Pinged::Aborted => Flow::Go,
        Pinged::Answered(Ok(Ok(_))) => {
            if let Some(l) = ctl.liveness.as_mut() {
                l.sup.note_ping(clocks().0);
            }
            Flow::Go
        }
        Pinged::Answered(answered) => {
            let class = match answered {
                Ok(Err(e)) => {
                    let _ = core.raw_error(&e, false);
                    core.broken
                        .or(core.last_class)
                        .unwrap_or(ErrClass::Internal)
                }
                _ => ErrClass::NetworkTimeout,
            };
            tracing::debug!(
                session = ctl.id.0,
                class = class.id(),
                "the keep-alive failed"
            );
            core.broken = Some(class);
            core.close().await;
            match begin_recovery(core, ctl, class, false) {
                Some(reconnect) => Flow::Recover(reconnect),
                None => {
                    break_now(core, ctl, class).await;
                    Flow::Go
                }
            }
        }
    }
}

/// Starts recovering from a loss of `class` (the module docs), when it is a recovery case, the
/// session has a reconnector and no close waits; `None` otherwise (the caller breaks the
/// session). The dead connection is already closed.
///
/// Fix round 1, C1: only an idle session reconnects. When the lost connection had a
/// transaction open (the last status observed before the loss, never one read after it; fix
/// rounds 2 and 3, C2 and R4), or the request in flight may have opened one (`tx_unknown`), or
/// the user's own MySQL `autocommit = 0` is in force (I8), nothing is replayed and nothing
/// runs on a new connection: the session tells what was lost (`SessionEvent::Lost`) and
/// returns `None`, so it breaks (`Lost` with Retry).
fn begin_recovery(
    core: &mut Core,
    ctl: &mut Ctl,
    class: ErrClass,
    tx_unknown: bool,
) -> Option<BoxFuture<'static, Result<LiveConnection, ConnectError>>> {
    let reconnector = ctl.hooks.reconnect.clone()?;
    let closing = ctl.queue.iter().any(|j| matches!(j, Job::Close(..)));
    if !is_recovery_case(class) || closing {
        return None;
    }
    // The last status observed before the loss (`observe` reads none after it; fix round 3,
    // R4): a transaction that just committed is not reported rolled back.
    let before = core.tx.status;
    core.note_loss(before);
    // I8: under the user's own `autocommit = 0` a transaction may be open whatever the status
    // says, and a reconnect would silently turn autocommit back on.
    let unknown = tx_unknown || core.autocommit_off;
    if unknown || core.lost.as_ref().is_some_and(|l| l.open_tx) {
        tracing::debug!(
            session = ctl.id.0,
            class = class.id(),
            "connection lost inside a transaction; not reconnecting"
        );
        let lost = core.lost.take().unwrap_or_default();
        ctl.emit(SessionEvent::Lost(LostState {
            open_tx: true,
            ..lost
        }));
        return None;
    }
    tracing::debug!(
        session = ctl.id.0,
        class = class.id(),
        "connection lost; reconnecting"
    );
    ctl.drain_lost();
    let tx = core.tx.clone();
    ctl.publish(|s| {
        s.state = SessionState::Connecting;
        s.cancel = CancelState::Idle;
        s.tx = tx;
    });
    // Synchronously, before the caller answers the request that found the loss: whoever sees
    // that answer already sees the reconnect under way.
    Some(reconnector.reconnect(1))
}

/// Whether `request`, lost in flight, may have opened a transaction (C1): a `BEGIN`, a
/// `START TRANSACTION` or a `SET autocommit` anywhere in it (fix round 2, R1), or any user
/// statement of a Manual session (it begins one first).
fn may_open_tx(request: &Request, mode: TxMode) -> bool {
    match request {
        Request::Tx(TxCommand::Begin) => true,
        Request::Exec {
            sql,
            origin: ExecOrigin::User,
            ..
        } => mode == TxMode::Manual || touches_tx(sql),
        _ => false,
    }
}

/// What the actor does with a command while it recovers.
enum Recovering {
    Go,
    Close(CloseReply),
    Abandoned,
}

/// A command while the session recovers (fix round 1, C1): nothing the user sends for the lost
/// connection ever runs on the new one, so every request is answered `ConnectionLost {
/// outcome_unknown: false }` (it never ran), except an internal idempotent read, which waits.
fn recovering_command(ctl: &mut Ctl, cmd: Option<Command>) -> Recovering {
    match cmd {
        None => Recovering::Abandoned,
        Some(Command::Close { reply, .. }) => Recovering::Close(reply),
        Some(Command::Submit {
            seq,
            request,
            reply,
        }) if is_retryable(&request) => {
            ctl.queue.push_back(Job::Request(Queued {
                seq,
                request,
                reply,
                retried: false,
            }));
            Recovering::Go
        }
        Some(Command::Submit { reply, .. }) => {
            let lost = ExecErrorKind::ConnectionLost {
                outcome_unknown: false,
            };
            let _ = reply.send(Err(ExecError::of(lost)));
            Recovering::Go
        }
        Some(other) => {
            ctl.idle_command(other);
            Recovering::Go
        }
    }
}

/// Ends a recovery on a close: the session is `Disconnected`.
async fn close_recovering(core: &mut Core, ctl: &mut Ctl, reply: CloseReply) {
    core.close().await;
    ctl.shut();
    ctl.publish(|s| {
        s.state = SessionState::Disconnected;
        s.cancel = CancelState::Idle;
    });
    let _ = reply.send(Ok(()));
}

/// Waits for `reconnect`, then replays the environment, serving commands all along (the module
/// docs). The replay is part of the attempt (fix round 1, I1): it is bounded by the profile's
/// connect timeout, a close ends it at once, and a failure continues the same backoff and
/// give-up clock (`Reconnector::replay_failed`). False when the actor is done (closed, or
/// every handle gone).
async fn recover(
    core: &mut Core,
    ctl: &mut Ctl,
    mut reconnect: BoxFuture<'static, Result<LiveConnection, ConnectError>>,
) -> bool {
    let limit = std::time::Duration::from_secs(u64::from(core.profile.connect_timeout_s.max(1)));
    loop {
        let landed = loop {
            tokio::select! {
                biased;
                cmd = ctl.commands.recv() => match recovering_command(ctl, cmd) {
                    Recovering::Go => {}
                    Recovering::Close(reply) => {
                        close_recovering(core, ctl, reply).await;
                        return false;
                    }
                    Recovering::Abandoned => return false,
                },
                landed = &mut reconnect => break landed,
            }
        };
        let live = match landed {
            Ok(live) => live,
            Err(e) => {
                tracing::debug!(
                    session = ctl.id.0,
                    class = e.class.id(),
                    "the reconnect ended"
                );
                break_now(core, ctl, e.class).await;
                return true;
            }
        };
        let access = core.install(live);
        let replayed = {
            let replay = tokio::time::timeout(limit, core.replay());
            tokio::pin!(replay);
            loop {
                tokio::select! {
                    biased;
                    cmd = ctl.commands.recv() => match recovering_command(ctl, cmd) {
                        Recovering::Go => {}
                        Recovering::Close(reply) => break Err(Some(reply)),
                        Recovering::Abandoned => break Err(None),
                    },
                    replayed = &mut replay => break Ok(replayed),
                }
            }
        };
        let failed = match replayed {
            Err(Some(reply)) => {
                close_recovering(core, ctl, reply).await;
                return false;
            }
            Err(None) => {
                core.close().await;
                return false;
            }
            Ok(Ok(Ok(()))) => None,
            Ok(Ok(Err(e))) => {
                tracing::debug!(session = ctl.id.0, kind = ?e.kind, "the replay failed");
                Some(
                    core.broken
                        .or(core.last_class)
                        .unwrap_or(ErrClass::Internal),
                )
            }
            Ok(Err(_)) => {
                tracing::debug!(session = ctl.id.0, "the replay timed out");
                Some(ErrClass::NetworkTimeout)
            }
        };
        if let Some(class) = failed {
            core.close().await;
            if is_recovery_case(class)
                && let Some(reconnector) = ctl.hooks.reconnect.clone()
            {
                reconnector.replay_failed(class);
                reconnect = reconnector.reconnect(1);
                continue;
            }
            break_now(core, ctl, class).await;
            return true;
        }
        let lost = core.lost.take().unwrap_or_default();
        ctl.emit(SessionEvent::Lost(lost));
        let (tx, schema) = (core.tx.clone(), core.schema.clone());
        ctl.publish(|s| {
            s.state = SessionState::Idle;
            s.cancel = CancelState::Idle;
            s.tx = tx;
            s.schema = schema;
            s.access = access;
        });
        ctl.emit(SessionEvent::Reconnected);
        if let Some(l) = ctl.liveness.as_mut() {
            // A fresh connection: the next keep-alive is a full period away.
            l.sup.note_ping(clocks().0);
        }
        return true;
    }
}

/// Closes the session by `policy`; false when it refused (an open transaction).
async fn close(core: &mut Core, ctl: &mut Ctl, policy: ClosePolicy, reply: CloseReply) -> bool {
    if core.live.is_some() && core.tx.status != TxStatus::Idle {
        match policy {
            ClosePolicy::RefuseIfOpenTx => {
                let _ = reply.send(Err(CloseRefused::OpenTransaction));
                return false;
            }
            ClosePolicy::RollBack => {
                if let Err(e) = core.control("ROLLBACK").await {
                    tracing::debug!(kind = ?e.kind, "the closing rollback failed");
                }
            }
        }
    }
    ctl.shut();
    core.close().await;
    ctl.publish(|s| {
        s.state = SessionState::Disconnected;
        s.cancel = CancelState::Idle;
    });
    let _ = reply.send(Ok(()));
    true
}

/// How a running request ended.
enum Ended {
    Done(Result<Reply, Box<dyn Any + Send>>),
    /// `escalate(Deactivate)`: the request was dropped; the escalation's reply.
    Deactivated(oneshot::Sender<CancelOutcome>),
    /// Every handle is gone.
    Abandoned,
}

/// Runs one request while serving commands: `Stop` when every handle is gone, `Recover` when
/// the connection was lost and the session reconnects (Task 17).
async fn run_one(core: &mut Core, ctl: &mut Ctl, job: Queued) -> Flow {
    let Queued {
        seq,
        request,
        reply,
        retried,
    } = job;
    let Some(live) = core.live.as_mut() else {
        let _ = reply.send(Err(not_connected()));
        return Flow::Go;
    };
    // ## m2b-t17-liveness: an internal idempotent read may run once more after a reconnect; a
    // request that may open a transaction leaves its state unknown if it is lost (C1).
    let retry = (!retried && is_retryable(&request)).then(|| request.clone());
    let tx_unknown = !core.read_only && may_open_tx(&request, core.tx.mode);
    // Driver calls, so guarded like the request itself (review M5).
    let probed = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let raw = live.raw();
        (raw.backend_id(), raw.cancel_handle())
    }));
    let Ok((backend, handle)) = probed else {
        tracing::error!(session = ctl.id.0, req = seq, "a driver call panicked");
        break_now(core, ctl, ErrClass::Internal).await;
        let _ = reply.send(Err(ExecError::of(ExecErrorKind::Internal)));
        return Flow::Go;
    };
    let mut ladder = Ladder::new(seq, core.kind, backend, handle, ctl.hooks.side.clone());
    let since = Instant::now();
    ctl.publish(|s| s.state = SessionState::Busy { req: seq, since });
    let (tx_before, schema_before) = (core.tx.clone(), core.schema.clone());
    let mut prewarm_at =
        (family(core.kind) == Family::MySql).then(|| tokio::time::Instant::now() + SIDE_PREWARM);

    let ended = {
        let work = AssertUnwindSafe(core.handle(request)).catch_unwind();
        tokio::pin!(work);
        loop {
            let deadline = ladder.next_deadline();
            let prewarm = prewarm_at;
            tokio::select! {
                // The request first: it is polled (and its statement started) before any
                // command is read, so a cancel always finds it running.
                biased;
                done = &mut work => break Ended::Done(done),
                cmd = ctl.commands.recv() => match cmd {
                    None => break Ended::Abandoned,
                    Some(cmd) => {
                        if let Some(reply) = ladder.command(ctl, cmd) {
                            break Ended::Deactivated(reply);
                        }
                    }
                },
                joined = join_slot(&mut ladder.mechanism), if ladder.mechanism.is_some() => {
                    ladder.mechanism_done(joined);
                }
                () = sleep_opt(prewarm), if prewarm.is_some() => {
                    prewarm_at = None;
                    ladder.prewarm();
                }
                () = sleep_opt(deadline), if deadline.is_some() => ladder.tick(ctl),
            }
        }
    };

    let mut deactivated = None;
    let mut result = match ended {
        Ended::Abandoned => {
            ladder.release();
            ladder.finish(CancelOutcome::Disconnected);
            core.close().await;
            return Flow::Stop;
        }
        Ended::Done(Ok(result)) => result,
        Ended::Done(Err(panic)) => {
            tracing::error!(
                session = ctl.id.0,
                req = seq,
                panic = %loggable(panic_text(panic.as_ref())),
                "a driver call panicked; the session is broken"
            );
            core.broken = Some(ErrClass::Internal);
            Err(ExecError::of(ExecErrorKind::Internal))
        }
        Ended::Deactivated(reply) => {
            deactivated = Some(reply);
            core.broken = Some(ErrClass::Cancelled);
            ctl.drain(&ExecErrorKind::Cancelled);
            Err(ExecError::of(ExecErrorKind::Cancelled))
        }
    };
    // The server's own cancel codes, once a cancel was asked for.
    if ladder.requested()
        && let Err(e) = &mut result
        && crate::session::cancel::is_cancel_code(core.kind, e)
    {
        e.kind = ExecErrorKind::Cancelled;
    }
    if core.broken.is_none() {
        // Q18, review C1: a cancel still in flight must land before the next request starts.
        // One that has not ended after the wait is aborted, but its kill may still reach the
        // server, so the connection is never used again.
        if !ladder.settle().await {
            tracing::warn!(
                session = ctl.id.0,
                req = seq,
                "a cancel did not end in time; the session is broken"
            );
            core.broken = Some(ErrClass::Cancelled);
        }
    } else {
        ladder.abort();
    }
    ladder.release();

    // ## m2b-t17-liveness: a lost connection the session recovers from (B19).
    let mut recovery = None;
    let mut reply = Some(reply);
    if let Some(class) = core.broken {
        core.close().await;
        recovery = begin_recovery(core, ctl, class, tx_unknown);
        if recovery.is_some()
            && let Some(request) = retry
            && matches!(
                &result,
                Err(e) if matches!(e.kind, ExecErrorKind::ConnectionLost { .. })
            )
            && let Some(reply) = reply.take()
        {
            // Once, at the head, after the reconnect (B19).
            ctl.queue.push_front(Job::Request(Queued {
                seq,
                request,
                reply,
                retried: true,
            }));
        }
    }
    if recovery.is_some() {
        // `begin_recovery` published `Connecting`.
    } else if let Some(class) = core.broken {
        ctl.drain(&ExecErrorKind::NotConnected);
        ctl.publish(|s| {
            s.state = SessionState::Broken(class);
            s.cancel = CancelState::Idle;
            s.tx = core.tx.clone();
            s.schema = core.schema.clone();
        });
    } else {
        ctl.publish(|s| {
            s.state = SessionState::Idle;
            s.cancel = CancelState::Idle;
            s.tx = core.tx.clone();
            s.schema = core.schema.clone();
        });
    }
    if core.tx != tx_before {
        ctl.emit(SessionEvent::TxChanged(core.tx.clone()));
    }
    if core.schema != schema_before {
        ctl.emit(SessionEvent::SchemaChanged(
            core.schema.clone().unwrap_or_default(),
        ));
    }
    let outcome = match &result {
        Err(e) if e.kind == ExecErrorKind::Cancelled => CancelOutcome::Cancelled,
        // A rung was climbed and the request then failed: a terminate ends it with a lost
        // connection (review M1).
        Err(_) if ladder.requested() => CancelOutcome::Cancelled,
        _ => CancelOutcome::AlreadyFinished,
    };
    if let Some(reply) = reply {
        let _ = reply.send(result);
    }
    ladder.finish(outcome);
    if let Some(reply) = deactivated {
        let _ = reply.send(CancelOutcome::Cancelled);
    }
    recovery.map_or(Flow::Go, Flow::Recover)
}

/// How much of a panic message is logged.
const PANIC_LOG_CHARS: usize = 200;

/// A panic message as logged: redacted when it looks like it carries a secret, and cut short,
/// since a driver's message may quote connection details (review M13).
fn loggable(text: &str) -> String {
    if carries_secret(text) {
        return "<redacted>".to_string();
    }
    text.chars().take(PANIC_LOG_CHARS).collect()
}

fn panic_text(p: &(dyn Any + Send)) -> &str {
    p.downcast_ref::<&'static str>()
        .copied()
        .or_else(|| p.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("a panic without a message")
}

/// A task that is aborted when its handle is dropped: a mechanism that is aborted takes the
/// tasks it waits for with it, so none of them is left detached (review C1).
struct AbortOnDrop<T>(JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl<T> Future for AbortOnDrop<T> {
    type Output = Result<T, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.get_mut().0).poll(cx)
    }
}

/// Waits for the task in `slot`; never, when there is none.
async fn join_slot<T>(slot: &mut Option<JoinHandle<T>>) -> Result<T, JoinError> {
    match slot {
        Some(handle) => handle.await,
        None => std::future::pending().await,
    }
}

/// Sleeps until `at`; never, without one.
async fn sleep_opt(at: Option<tokio::time::Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

// ---------------------------------------------------------------------------------------------
// The cancel ladder of one running request (`cancel` module docs).

struct Ladder {
    seq: u64,
    kind: DbmsKind,
    backend: Option<u64>,
    handle: Option<Arc<dyn CancelHandle>>,
    side: Arc<dyn SideConnector>,
    rung: CancelState,
    /// When the first rung was asked for.
    since: Option<Instant>,
    cancel_sent: bool,
    slow_at: Option<tokio::time::Instant>,
    /// Callers waiting for the outcome, each until its deadline.
    waiters: Vec<(tokio::time::Instant, oneshot::Sender<CancelOutcome>)>,
    failed: Option<ErrClass>,
    /// The cancel or kill in flight.
    mechanism: Option<JoinHandle<Result<(), ErrClass>>>,
    /// MySQL's pre-opened side connection (Q37).
    prewarmed: Option<JoinHandle<Result<LiveConnection, ConnectError>>>,
}

impl Ladder {
    fn new(
        seq: u64,
        kind: DbmsKind,
        backend: Option<u64>,
        handle: Option<Arc<dyn CancelHandle>>,
        side: Arc<dyn SideConnector>,
    ) -> Self {
        Ladder {
            seq,
            kind,
            backend,
            handle,
            side,
            rung: CancelState::Idle,
            since: None,
            cancel_sent: false,
            slow_at: None,
            waiters: Vec::new(),
            failed: None,
            mechanism: None,
            prewarmed: None,
        }
    }

    fn requested(&self) -> bool {
        self.since.is_some()
    }

    /// A command while the request runs; `Some` (the reply) to deactivate.
    fn command(&mut self, ctl: &mut Ctl, cmd: Command) -> Option<oneshot::Sender<CancelOutcome>> {
        match cmd {
            Command::Cancel { seq, reply } if seq == self.seq => {
                self.climb(ctl, None);
                self.send_cancel();
                self.wait(reply);
            }
            Command::Escalate {
                seq,
                escalation: Escalation::Terminate,
                reply,
            } if seq == self.seq => {
                self.climb(ctl, Some(CancelState::Terminating));
                self.send_terminate();
                self.wait(reply);
            }
            Command::Escalate {
                seq,
                escalation: Escalation::Deactivate,
                reply,
            } if seq == self.seq => {
                self.climb(ctl, Some(CancelState::Deactivating));
                return Some(reply);
            }
            other => ctl.idle_command(other),
        }
        None
    }

    /// Moves to a rung. The first one drops the queue (B23) and starts the slow-cancel clock;
    /// `rung` replaces the state (`None`: a plain cancel never steps back from a higher rung).
    fn climb(&mut self, ctl: &mut Ctl, rung: Option<CancelState>) {
        let since = match self.since {
            Some(since) => since,
            None => {
                let now = Instant::now();
                self.since = Some(now);
                self.slow_at = Some(tokio::time::Instant::now() + SLOW_CANCEL);
                self.rung = CancelState::Cancelling { since: now };
                ctl.drain(&ExecErrorKind::Cancelled);
                now
            }
        };
        if let Some(rung) = rung {
            self.rung = rung;
        }
        let (req, cancel) = (self.seq, self.rung.clone());
        ctl.publish(|s| {
            s.state = SessionState::Cancelling { req, since };
            s.cancel = cancel;
        });
    }

    fn wait(&mut self, reply: oneshot::Sender<CancelOutcome>) {
        if let Some(class) = self.failed {
            let _ = reply.send(CancelOutcome::Failed(class));
            return;
        }
        self.waiters
            .push((tokio::time::Instant::now() + CANCEL_WAIT, reply));
    }

    /// Sends the cancel once: MySQL `KILL QUERY` on a side connection, else the connection's
    /// `CancelHandle`.
    fn send_cancel(&mut self) {
        if self.cancel_sent {
            return;
        }
        self.cancel_sent = true;
        if family(self.kind) == Family::MySql {
            let sql = self.backend.and_then(|id| kill_query_sql(self.kind, id));
            self.on_side(sql);
        } else if let Some(handle) = &self.handle {
            let kind = self.kind;
            let cancel = handle.cancel();
            self.spawn(async move {
                cancel
                    .await
                    .map_err(|e| classify(kind, Phase::PostConnect, &e).class)
            });
        } else {
            self.fail(ErrClass::Internal);
        }
    }

    fn send_terminate(&mut self) {
        let sql = self.backend.and_then(|id| terminate_sql(self.kind, id));
        self.on_side(sql);
    }

    /// Runs `sql` on a side connection (the pre-opened one, else a new one), then closes it.
    fn on_side(&mut self, sql: Option<String>) {
        let Some(sql) = sql else {
            // No server connection ID, or an engine without the statement.
            self.fail(ErrClass::Internal);
            return;
        };
        let opened = self.prewarmed.take().map(AbortOnDrop);
        let side = self.side.clone();
        let kind = self.kind;
        self.spawn(async move {
            let connected = match opened {
                Some(task) => match task.await {
                    Ok(connected) => connected,
                    Err(_) => return Err(ErrClass::Internal),
                },
                None => side.open_side().await,
            };
            let mut live = connected.map_err(|e| e.class)?;
            let ran = live.raw().batch(&sql).await;
            live.close().await;
            ran.map_err(|e| classify(kind, Phase::PostConnect, &e).class)
        });
    }

    /// Starts `f` after the mechanism already in flight, if any (one slot, in order).
    fn spawn(&mut self, f: impl Future<Output = Result<(), ErrClass>> + Send + 'static) {
        let before = self.mechanism.take().map(AbortOnDrop);
        self.mechanism = Some(tokio::spawn(async move {
            let first = match before {
                Some(task) => task.await.unwrap_or(Err(ErrClass::Internal)),
                None => Ok(()),
            };
            let second = f.await;
            first.and(second)
        }));
    }

    fn mechanism_done(&mut self, joined: Result<Result<(), ErrClass>, JoinError>) {
        self.mechanism = None;
        match joined {
            Ok(Ok(())) => {}
            Ok(Err(class)) => self.fail(class),
            Err(_) => self.fail(ErrClass::Internal),
        }
    }

    fn fail(&mut self, class: ErrClass) {
        tracing::debug!(class = class.id(), "the cancel failed");
        self.failed = Some(class);
        for (_, reply) in self.waiters.drain(..) {
            let _ = reply.send(CancelOutcome::Failed(class));
        }
    }

    fn next_deadline(&self) -> Option<tokio::time::Instant> {
        self.waiters
            .iter()
            .map(|(at, _)| *at)
            .chain(self.slow_at)
            .min()
    }

    /// Answers the callers whose wait ran out (`StillRunning`) and reports a slow cancel.
    fn tick(&mut self, ctl: &Ctl) {
        let now = tokio::time::Instant::now();
        let (due, waiting): (Vec<_>, Vec<_>) =
            self.waiters.drain(..).partition(|(at, _)| *at <= now);
        self.waiters = waiting;
        for (_, reply) in due {
            let _ = reply.send(CancelOutcome::StillRunning);
        }
        if self.slow_at.is_some_and(|at| at <= now) {
            self.slow_at = None;
            if let CancelState::Cancelling { since } = self.rung {
                self.rung = CancelState::SlowCancel { since };
                let rung = self.rung.clone();
                ctl.publish(|s| s.cancel = rung);
                ctl.emit(SessionEvent::SlowCancel { req: self.seq });
            }
        }
    }

    /// Pre-opens MySQL's side connection (Q37), unless a cancel already went out.
    fn prewarm(&mut self) {
        if self.cancel_sent || self.prewarmed.is_some() {
            return;
        }
        self.prewarmed = Some(tokio::spawn(self.side.open_side()));
    }

    /// Waits up to [`CANCEL_WAIT`] for the cancel in flight. False when it did not end: it is
    /// then aborted (and awaited, so nothing more of it runs), but what it already sent may
    /// still land, so the caller must not reuse the connection (review C1).
    async fn settle(&mut self) -> bool {
        let Some(mut task) = self.mechanism.take() else {
            return true;
        };
        if tokio::time::timeout(CANCEL_WAIT, &mut task).await.is_ok() {
            return true;
        }
        task.abort();
        let _ = task.await;
        false
    }

    /// Aborts the cancel in flight (the connection is closing anyway).
    fn abort(&mut self) {
        if let Some(task) = self.mechanism.take() {
            task.abort();
        }
    }

    /// Closes a pre-opened side connection nobody used.
    fn release(&mut self) {
        if let Some(task) = self.prewarmed.take() {
            tokio::spawn(async move {
                if let Ok(Ok(live)) = task.await {
                    live.close().await;
                }
            });
        }
    }

    /// Answers every waiting caller with `outcome`.
    fn finish(&mut self, outcome: CancelOutcome) {
        for (_, reply) in self.waiters.drain(..) {
            let _ = reply.send(outcome);
        }
    }
}
