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

use std::any::Any;
use std::collections::VecDeque;
use std::fmt;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::task::{Context, Poll};
use std::time::Instant;

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
use crate::session::readonly::run_guarded;
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
pub trait Reconnector: Send + Sync {
    fn reconnect(&self, attempt: u32) -> BoxFuture<'static, Result<LiveConnection, ConnectError>>;
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
    mut live: LiveConnection,
    hooks: SessionHooks,
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
    };
    let ctl = Ctl {
        id,
        state: state.clone(),
        events,
        commands: receiver,
        queue: VecDeque::new(),
        hooks,
    };
    rt.spawn(async move {
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

    /// Reads the protocol's transaction status after a round trip.
    pub(crate) fn observe(&mut self, statement: bool) {
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
    pub(crate) fn raw_error(&mut self, e: &RawError, sent: bool) -> ExecError {
        let class = classify(self.kind, Phase::PostConnect, e).class;
        self.last_class = Some(class);
        let kind = match &e.kind {
            RawErrorKind::Cancelled => ExecErrorKind::Cancelled,
            RawErrorKind::Db => ExecErrorKind::Db,
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
        self.observe(false);
        ran.map_err(|e| self.raw_error(&e, true))
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
            Request::Reset => self.reset().await,
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
            self.observe(false);
            if let Err(e) = &ran
                && matches!(e.kind, ExecErrorKind::ConnectionLost { .. })
            {
                self.broken.get_or_insert(ErrClass::NetworkReset);
            }
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
            self.observe(!control);
            ran.map_err(|e| self.raw_error(&e, true))?
        };
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
        self.observe(false);
        match probed {
            Ok(answer) => {
                let ctx = parse_schema_probe(self.kind, &answer);
                self.schema = (ctx != SchemaCtx::default()).then_some(ctx);
            }
            Err(e) => {
                let e = self.raw_error(&e, false);
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
}

// ---------------------------------------------------------------------------------------------
// The control side: the queue, the snapshot, the events.

struct Queued {
    seq: u64,
    request: Request,
    reply: oneshot::Sender<Reply>,
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
                if !run_one(&mut core, &mut ctl, job).await {
                    return;
                }
                continue;
            }
            None => {}
        }
        match ctl.commands.recv().await {
            Some(cmd) => ctl.idle_command(cmd),
            // Every handle is gone.
            None => {
                core.close().await;
                return;
            }
        }
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

/// Runs one request while serving commands; false when every handle is gone.
async fn run_one(core: &mut Core, ctl: &mut Ctl, job: Queued) -> bool {
    let Queued {
        seq,
        request,
        reply,
    } = job;
    let Some(live) = core.live.as_mut() else {
        let _ = reply.send(Err(not_connected()));
        return true;
    };
    // Driver calls, so guarded like the request itself (review M5).
    let probed = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let raw = live.raw();
        (raw.backend_id(), raw.cancel_handle())
    }));
    let Ok((backend, handle)) = probed else {
        tracing::error!(session = ctl.id.0, req = seq, "a driver call panicked");
        break_now(core, ctl, ErrClass::Internal).await;
        let _ = reply.send(Err(ExecError::of(ExecErrorKind::Internal)));
        return true;
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
            return false;
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

    if let Some(class) = core.broken {
        core.close().await;
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
    let _ = reply.send(result);
    ladder.finish(outcome);
    if let Some(reply) = deactivated {
        let _ = reply.send(CancelOutcome::Cancelled);
    }
    true
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
