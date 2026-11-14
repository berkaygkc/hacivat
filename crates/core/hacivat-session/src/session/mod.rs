//! Sessions (HV-SESS-01 to -08): one actor per database connection with a request queue,
//! transaction state, a cancel ladder, environment replay and schema tracking; the router and
//! pools of a profile's `SessionManager`; liveness and recovery; guarded read-only execution.
//!
//! Types are complete since Task 1 (M2b). Bodies: Task 7 (`actor`, `tx`, `cancel`, `env`,
//! `schema`, `reset`), Task 15 (`router`, `pool`), Task 17 (`liveness`, `recovery`), Task 18
//! (`readonly`). Until then every stub fails closed: a session executes nothing.

pub mod actor;
pub mod cancel;
// ## m3a-t01b-seams: console cursors (Task 18).
pub mod cursor;
pub mod env;
pub mod liveness;
pub mod pool;
pub mod readonly;
pub mod recovery;
pub mod reset;
pub mod router;
// ## m3a-t01b-seams: `Request::Run` (Task 18).
pub mod run;
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
// ## m3a-t01b-seams
use std::fmt;
use std::hash::{Hash, Hasher};

use hacivat_driver::{Protocol, TxStatus};
use hacivat_proto::EnvTag;
use hacivat_results::{
    CloseReason, Completeness, FetchControl, FetchPolicy, ResultId, ResultWriter,
};
use hacivat_values::ResultSchema;

use crate::profile::Profile;

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
// ## m3a-t01b-seams
pub use pool::SessionPolicy;
pub use tx::TxOwner;

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
    // ## m3a-t01b-seams
    /// The open console cursor, when a result is suspended on this session (Task 18).
    pub cursor: Option<CursorBadge>,
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
    // ## m3a-t01b-seams (the actor answers these `Internal` until Task 18)
    /// Runs typed units in order; the reply comes after `RunEvent::RunEnded`.
    Run(RunSpec),
    /// Resumes a suspended result for `rows` more rows.
    FetchMore {
        result: ResultId,
        rows: u64,
    },
    /// Fetches a suspended result until it completes or a budget pauses it (R-M3-33).
    FetchAll {
        result: ResultId,
    },
    /// Releases a result's cursor (its rows stay).
    CloseCursor {
        result: ResultId,
        reason: CloseReason,
    },
}

/// Which read-only layer refused a statement (SQL-SAFE-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DenyLayer {
    Input,
    Protocol,
    Transaction,
    Engine,
    // ## m2b-t23-f1
    /// PostgreSQL: the statement calls a function whose effect a rollback does not undo
    /// (`hacivat_guard::check_functions`; Task 23 finding F1).
    Function,
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
    SlowCancel {
        req: u64,
    },
    IdleTransaction {
        minutes: u32,
    },
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

// ## m3a-t01b-seams
// Runs (HV-EXEC-05, HV-RES-05; Task 18): typed units streamed into result stores, observed
// through a `RunObserver`, with console cursors. Types complete since M3a Task 1b; the actor
// answers `Run`, `FetchMore`, `FetchAll` and `CloseCursor` with `Internal` until Task 18.

/// A run's ID, unique in the process (the run service counts them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RunId(pub u64);

/// One unit of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UnitId {
    pub run: RunId,
    pub ordinal: u32,
}

/// What a run does after a failed unit (R-M3-26: M3a stops; Ask and continue are M3b).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ErrorPolicy {
    /// The first failure stops the run; later units end `Skipped`.
    #[default]
    StopOnError,
}

/// One statement of a run: the exact text sent (no terminator, no leading comments).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RunUnit {
    pub id: UnitId,
    pub sql: Arc<str>,
    pub protocol: Protocol,
    /// The unit may stay suspended as a console cursor (R-M3-23: a read head, and only the
    /// last unit of its run stays suspended).
    pub suspendable: bool,
}

/// One `Request::Run`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RunSpec {
    pub run: RunId,
    pub units: Arc<[RunUnit]>,
    pub policy: ErrorPolicy,
    pub fetch: FetchPolicy,
    pub observer: ObserverRef,
}

/// Sees a run on the actor task. Every call is O(1) and never blocks.
pub trait RunObserver: Send + Sync + 'static {
    /// A unit's `ordinal`-th result set starts: the store the actor writes its rows into.
    fn open_result(&self, unit: UnitId, ordinal: u32, schema: Arc<ResultSchema>) -> ResultWriter;
    fn event(&self, ev: &RunEvent);
}

/// A shared [`RunObserver`]. Equality and hashing are by pointer, so `Request` keeps its
/// derives.
#[derive(Clone)]
pub struct ObserverRef(pub Arc<dyn RunObserver>);

impl ObserverRef {
    fn addr(&self) -> *const () {
        Arc::as_ptr(&self.0).cast::<()>()
    }
}

impl fmt::Debug for ObserverRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ObserverRef").field(&self.addr()).finish()
    }
}

impl PartialEq for ObserverRef {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.addr(), other.addr())
    }
}

impl Eq for ObserverRef {}

impl Hash for ObserverRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.addr().hash(state);
    }
}

/// How a unit ended.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UnitEnd {
    Succeeded {
        rows: Option<RowsOutcome>,
    },
    Failed(ExecError),
    Cancelled,
    /// Not run: an earlier unit failed (`StopOnError`).
    Skipped,
}

/// What a succeeded unit produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowsOutcome {
    Fetched { rows: u64, more: bool },
    Affected(u64),
}

/// A finished run, for the output log ("2 of 3 statements executed, 1 failed").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RunSummary {
    pub executed: u32,
    pub total: u32,
    pub failed: u32,
    pub skipped: u32,
    /// The run stopped before its last unit (a failure under `StopOnError`, a cancel, a loss).
    pub aborted: bool,
    pub duration_ms: u64,
}

/// What a run reports to its observer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RunEvent {
    UnitStarted {
        unit: UnitId,
        at: Instant,
    },
    ResultOpened {
        unit: UnitId,
        ordinal: u32,
        result: ResultId,
    },
    /// At most one per 50 ms per result.
    RowsFetched {
        result: ResultId,
        total: u64,
    },
    ResultEnded {
        result: ResultId,
        rows: u64,
        completeness: Completeness,
        exec_ms: u64,
        fetch_ms: u64,
    },
    UpdateCount {
        unit: UnitId,
        rows: Option<u64>,
        command_tag: Option<String>,
    },
    /// Result sets after the first were drained and counted (R-M3-25).
    MoreResultSets {
        unit: UnitId,
        skipped: u32,
    },
    /// A cursor was released. `rolled_back`: Hacivat's own transaction (or savepoint) ended in
    /// a rollback, which undid what the read head's functions wrote (R-M3-23, P26).
    CursorReleased {
        result: ResultId,
        rows: u64,
        reason: CloseReason,
        rolled_back: bool,
    },
    TxChanged {
        from: TxStatus,
        to: TxStatus,
    },
    UnitEnded {
        unit: UnitId,
        outcome: UnitEnd,
        exec_ms: u64,
        fetch_ms: u64,
    },
    RunEnded(RunSummary),
}

/// The open cursor a session holds (its snapshot's `cursor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CursorBadge {
    pub result: ResultId,
    pub rows: u64,
    pub since: Instant,
    /// The cursor holds a transaction (Hacivat's own `BEGIN`, a savepoint, or the guard's).
    pub holds_tx: bool,
}

/// Whether a session of `profile` is read-only (R-M3-29, P4): the profile says so, or the
/// production policy is on and the profile is tagged production. Every site that decides a
/// session's (or a Test connection's) read-only flag reads this.
pub fn effective_read_only(profile: &Profile, production_read_only: bool) -> bool {
    profile.read_only || (production_read_only && profile.environment == Some(EnvTag::Production))
}

impl SessionHandle {
    /// Submits `Request::Run(spec)`; the reply comes after `RunEvent::RunEnded`.
    pub fn run(&self, spec: RunSpec) -> RequestTicket {
        self.submit(Request::Run(spec))
    }
}

/// A result source's commands, as requests to its session. The tickets are dropped: the
/// outcome reaches the result through the run's observer.
impl FetchControl for SessionHandle {
    fn fetch_more(&self, result: ResultId, rows: u64) {
        let _ = self.submit(Request::FetchMore { result, rows });
    }

    fn close(&self, result: ResultId, reason: CloseReason) {
        let _ = self.submit(Request::CloseCursor { result, reason });
    }

    fn fetch_all(&self, result: ResultId) {
        let _ = self.submit(Request::FetchAll { result });
    }
}
