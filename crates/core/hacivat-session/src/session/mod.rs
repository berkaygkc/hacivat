//! Sessions (HV-SESS-01 to -08): one actor per database connection with a request queue,
//! transaction state, a cancel ladder, environment replay and schema tracking; the router and
//! pools of a profile's `SessionManager`; liveness and recovery; guarded read-only execution.
//!
//! Types are complete since Task 1 (M2b). Bodies: Task 7 (`actor`, `tx`, `cancel`, `env`,
//! `schema`, `reset`), Task 15 (`router`, `pool`), Task 17 (`liveness`, `recovery`), Task 18
//! (`readonly`). Until then every stub fails closed: a session executes nothing.

pub mod actor;
pub mod cancel;
pub mod env;
pub mod liveness;
pub mod pool;
pub mod readonly;
pub mod recovery;
pub mod reset;
pub mod router;
pub mod schema;
pub mod tx;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use futures::future::BoxFuture;
use hacivat_driver::{ExecOpts, ExecSummary, ServerMessage};
use hacivat_guard::GapReason;
use hacivat_proto::{AccessLabel, Department, ErrClass};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

pub use actor::{Reconnector, SessionHooks, SideConnector, spawn_session};
pub use cancel::{CancelOutcome, CancelState, Escalation};
pub use env::{EnvPlan, EnvStep, build_env_plan};
pub use liveness::{LivenessAction, LivenessPolicy, Supervisor};
pub use pool::{PooledSession, SessionManager, SessionOverlay};
pub use recovery::{
    LostState, RecoveryPolicy, Tracked, is_recovery_case, lost_state, track_statement,
};
pub use router::{RouteRequest, RouteTarget, RoutingMode, route};
pub use schema::{
    SchemaCtx, needs_schema_reprobe, parse_schema_probe, schema_probe_sql, switch_schema_sql,
};
pub use tx::{IsolationLevel, TxCommand, TxError, TxMode, TxState, begin_sql, default_mode};

/// A session's ID, unique in the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(pub u64);

/// A client of a profile's sessions (a console tab, an MCP client).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientId(pub u64);

impl ClientId {
    /// The app's own client.
    pub const DEFAULT: ClientId = ClientId(0);
}

/// What a job session does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobKind {
    Export,
    Import,
    DdlApply,
    DriftCheck,
}

/// What a session is for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SessionKind {
    Console(ClientId),
    /// A data editor, by its document key.
    Editor(String),
    Introspection {
        database: Option<String>,
    },
    Service,
    Job(u64, JobKind),
}

/// What to open.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionSpec {
    pub kind: SessionKind,
    pub read_only: bool,
    pub department: Department,
}

/// Where a session is (HV-SESS-01).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum SessionState {
    #[default]
    Disconnected,
    Connecting,
    Idle,
    Busy {
        req: u64,
        since: Instant,
    },
    Cancelling {
        req: u64,
        since: Instant,
    },
    Broken(ErrClass),
}

/// What a session shows (its `watch`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionSnapshot {
    pub state: SessionState,
    pub tx: TxState,
    pub schema: Option<SchemaCtx>,
    pub cancel: CancelState,
    pub read_only: bool,
    pub access: AccessLabel,
    /// Bumped by every change (so by every reconnect too).
    pub generation: u64,
}

/// Who sent a statement: the user, or Hacivat itself (an internal statement may be retried
/// after a reconnect only when idempotent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecOrigin {
    User,
    Internal { idempotent: bool },
}

/// One request to a session.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Request {
    Exec {
        sql: String,
        opts: ExecOpts,
        origin: ExecOrigin,
    },
    Tx(TxCommand),
    SwitchSchema(String),
    Ping,
    Reset,
}

/// Which read-only layer refused a statement (SQL-SAFE-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DenyLayer {
    Input,
    Protocol,
    Transaction,
    Engine,
}

/// Why a request failed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ExecErrorKind {
    /// The server refused the statement (`ExecError::server` has its message).
    Db,
    /// The connection dropped; `outcome_unknown` when the statement may have run.
    ConnectionLost {
        outcome_unknown: bool,
    },
    Cancelled,
    ReadOnlyDenied(DenyLayer),
    Tx(TxError),
    EnforcementGap(GapReason),
    NotConnected,
    Internal,
}

/// A failed request.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExecError {
    pub kind: ExecErrorKind,
    pub server: Option<ServerMessage>,
}

impl ExecError {
    /// An error of `kind` without a server message.
    pub fn of(kind: ExecErrorKind) -> ExecError {
        ExecError { kind, server: None }
    }
}

/// A request's result.
pub type Reply = Result<ExecSummary, ExecError>;

/// A submitted request: its sequence number (for cancels) and its reply.
#[derive(Debug)]
pub struct RequestTicket {
    pub seq: u64,
    pub reply: oneshot::Receiver<Reply>,
}

impl RequestTicket {
    /// Cancels this request on `h` (Q18).
    pub fn cancel(&self, h: &SessionHandle) -> oneshot::Receiver<CancelOutcome> {
        h.cancel(self.seq)
    }
}

/// What `close` does with an open transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClosePolicy {
    RefuseIfOpenTx,
    RollBack,
}

/// Why `close` refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CloseRefused {
    OpenTransaction,
}

/// What a session tells its owner.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SessionEvent {
    SlowCancel { req: u64 },
    IdleTransaction { minutes: u32 },
    EnforcementGap(GapReason),
    Lost(LostState),
    Reconnected,
    SchemaChanged(SchemaCtx),
    TxChanged(TxState),
    // ## m2b-t17-liveness (fix round 1, review M2/H4)
    /// The liveness supervisor closed the session after its idle threshold (B18); sent just
    /// before it publishes `Disconnected`, so a close by anyone else is never taken for one.
    IdleDisconnected,
}

/// What a handle sends its actor.
#[derive(Debug)]
pub(crate) enum Command {
    Submit {
        seq: u64,
        request: Request,
        reply: oneshot::Sender<Reply>,
    },
    Cancel {
        seq: u64,
        reply: oneshot::Sender<CancelOutcome>,
    },
    Escalate {
        seq: u64,
        escalation: Escalation,
        reply: oneshot::Sender<CancelOutcome>,
    },
    Close {
        policy: ClosePolicy,
        reply: oneshot::Sender<Result<(), CloseRefused>>,
    },
}

/// A session's handle (Clone): submit requests, cancel them, watch the state, close.
#[derive(Debug, Clone)]
pub struct SessionHandle {
    pub(crate) id: SessionId,
    pub(crate) kind: SessionKind,
    pub(crate) commands: mpsc::UnboundedSender<Command>,
    pub(crate) state: watch::Receiver<SessionSnapshot>,
    pub(crate) events: broadcast::Sender<SessionEvent>,
    pub(crate) next_seq: Arc<AtomicU64>,
}

impl SessionHandle {
    pub fn id(&self) -> SessionId {
        self.id
    }

    pub fn kind(&self) -> &SessionKind {
        &self.kind
    }

    /// Queues `req`. Without a running actor it is answered `NotConnected` at once.
    pub fn submit(&self, req: Request) -> RequestTicket {
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);
        let (reply, rx) = oneshot::channel();
        let command = Command::Submit {
            seq,
            request: req,
            reply,
        };
        if let Err(mpsc::error::SendError(Command::Submit { reply, .. })) =
            self.commands.send(command)
        {
            let _ = reply.send(Err(ExecError::of(ExecErrorKind::NotConnected)));
        }
        RequestTicket { seq, reply: rx }
    }

    /// Cancels request `seq`: `AlreadyFinished` unless `seq` is running (Q18). Without a
    /// running actor, `Disconnected`.
    pub fn cancel(&self, seq: u64) -> oneshot::Receiver<CancelOutcome> {
        let (reply, rx) = oneshot::channel();
        if let Err(mpsc::error::SendError(Command::Cancel { reply, .. })) =
            self.commands.send(Command::Cancel { seq, reply })
        {
            let _ = reply.send(CancelOutcome::Disconnected);
        }
        rx
    }

    /// The next rung of the cancel ladder for request `seq`.
    pub fn escalate(&self, seq: u64, e: Escalation) -> oneshot::Receiver<CancelOutcome> {
        let (reply, rx) = oneshot::channel();
        let command = Command::Escalate {
            seq,
            escalation: e,
            reply,
        };
        if let Err(mpsc::error::SendError(Command::Escalate { reply, .. })) =
            self.commands.send(command)
        {
            let _ = reply.send(CancelOutcome::Disconnected);
        }
        rx
    }

    pub fn state(&self) -> watch::Receiver<SessionSnapshot> {
        self.state.clone()
    }

    pub fn events(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    /// Closes the session by `p`. A session without a running actor is already closed.
    pub fn close(&self, p: ClosePolicy) -> BoxFuture<'static, Result<(), CloseRefused>> {
        let (reply, rx) = oneshot::channel();
        let sent = self.commands.send(Command::Close { policy: p, reply });
        Box::pin(async move {
            if sent.is_err() {
                return Ok(());
            }
            rx.await.unwrap_or(Ok(()))
        })
    }
}
