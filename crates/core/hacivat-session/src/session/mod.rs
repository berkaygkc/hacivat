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
// ## m3b-t01c-core-seams: verdicts and the classifier hook (R-M3-83), confirm-in-transaction
// and Smart (Task 19), out-of-band answers (P-M3b-3).
pub(crate) mod confirm;
pub(crate) mod run_gate;
// ## m3b-t24-exec-ui (named exception: the T24 I1 ruling)
pub(crate) mod run_context;
// ## m3b-t26-prod-unlock: a production profile's startup script runs only harmless statements.
pub(crate) mod smart;
pub(crate) mod startup_check;
pub mod verdict;

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
use hacivat_results::{
    CloseReason, Completeness, FetchControl, FetchPolicy, ResultId, ResultWriter,
};
use hacivat_values::ResultSchema;
// ## m3b-t01c-core-seams
use hacivat_values::BindValue;
use std::time::Duration;

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
pub use tx::TxOwner;
// ## m3a-f1-followups
pub use schema::context_probe_sql;
// ## m3b-t01c-core-seams
pub use verdict::{RoCheck, SafetyDenial, SessionEffects, StatementClassifier, UnitVerdict};
// ## m3b-t18-safe06
pub use verdict::{ClassifierContext, RoAnswer};

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
    // ## m3a-t01b-seams (served since Task 18: `run.rs`, `cursor.rs`)
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
    // ## m3b-t24-exec-ui (named exception: the T24 I1 ruling)
    /// The session's classification context now (Run's gate, Count rows): answered in the
    /// queue's order, so after every earlier request; read again first when a request may have
    /// changed it. [`SessionHandle::classifier_context`] sends it and reads the answer.
    ReadContext,
    // ## m4-t05-meta (named exception, ruling R-W1-9)
    /// A metadata read (`crate::meta`), checked and run in one actor step when it is taken.
    /// Only `crate::meta` builds one ([`MetaRead`]'s fields are private).
    MetaRead(MetaRead),
}

// ## m4-t05-meta (named exception, ruling R-W1-9)
/// One metadata read on a session (`crate::meta`): trusted pack text with its binds. The actor
/// checks its condition when it takes the request, in the same step that runs it, and answers
/// a refusal without sending anything:
/// - **On the user's session**: the transaction must be idle, with no cursor, no MySQL
///   `LOCK TABLES` and MySQL `autocommit` on (`TxError::InProgress` otherwise). The read runs
///   inside Hacivat's own read-only transaction, rolled back after it.
/// - **Joined** (R-M4-29): a `Job(DdlApply | DriftCheck)` session inside its transaction
///   (`TxError::Unsupported` otherwise); nothing but the read is sent.
///
/// It never goes through the user-run path, the transaction modes, the verdicts, the history
/// or the activity, and never marks the run context stale.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MetaRead {
    pub(crate) sql: Arc<str>,
    pub(crate) binds: Arc<[BindValue]>,
    pub(crate) joined: bool,
    /// The server-side limit of the read (PostgreSQL `SET LOCAL statement_timeout`).
    pub(crate) timeout: Duration,
    // ## m4-t15b-facets (named exception in Task 5's seam; T15b review m-1)
    /// The engine the reader expects the session to be (`Unknown`: any). A joined read of
    /// another engine's text is refused before anything is sent.
    pub(crate) engine: hacivat_proto::DbmsKind,
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
    // ## m3b-t01c-core-seams
    /// The injected classifier refused it (`Core::step0b`, R-M3-83; Task 18).
    Classifier,
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
    // ## m3b-t01c-core-seams
    /// The classifier refused the statement on a read-only session (Task 18): nothing sent.
    SafetyDenied(SafetyDenial),
    /// The session's queue is full (Task 11): nothing queued.
    QueueFull,
    // ## m3b-t24-exec-ui (fix round 2, named exception)
    /// The run was classified under a context the session no longer has (another request
    /// changed it in between): nothing sent.
    ContextChanged,
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
    // ## m3b-t23-escalation (named exception)
    /// The cancel or the terminate of request `req` failed (its escalation's reply is
    /// `CancelOutcome::Failed`); `server` is the server's own error when the statement that
    /// carried it (`pg_terminate_backend`, `KILL`) answered one, shown verbatim by the UI.
    /// Sent before the failure's reply, so a caller that got the reply finds it already queued.
    CancelFailed {
        req: u64,
        server: Option<ServerMessage>,
    },
    // ## m4-t01c-core (named): how a transaction ended, for refresh after DDL (HV-INTRO-12,
    // R-M4-42: pending targets apply on COMMIT, are dropped on ROLLBACK, and a ROLLBACK TO drops
    // those after the savepoint). `TxChanged(TxState)` cannot say it: its `status` reads
    // `InTx` → `Idle` after both COMMIT and ROLLBACK, and stays `InTx` across `ROLLBACK TO
    // SAVEPOINT`, and `TxState` names no savepoint. No session emits it until Task 21.
    TxEnded {
        outcome: TxOutcome,
    },
}

// ## m4-t01c-core
/// How a transaction (or part of one) ended ([`SessionEvent::TxEnded`]).
///
/// **Where the outcome comes from** (for the emitter, Task 21): the server's answer, never the
/// statement text. The command tag or the transaction status after the statement decides:
/// PostgreSQL answers `COMMIT` in a failed transaction with the tag `ROLLBACK`; `END`,
/// `ABORT`, `COMMIT AND CHAIN`, MySQL's implicit commits (DDL, `START TRANSACTION` inside a
/// transaction, `LOCK TABLES`) and SQLite's `RELEASE` of the outermost savepoint end a
/// transaction without saying so in the text. Savepoint names are compared through
/// `sql_ident::key::match_key` (PostgreSQL folds an unquoted name; MySQL compares them
/// case-insensitively), never byte for byte.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TxOutcome {
    Committed,
    RolledBack,
    /// `ROLLBACK TO SAVEPOINT <name>`: the transaction stays open; what ran after the
    /// savepoint is undone. The name exactly as the server knows it.
    RolledBackTo(Arc<str>),
    // ## m4-t01c-core (fix round 1, I4)
    /// The transaction ended, but how is not known: the connection was lost after `COMMIT` or
    /// `ROLLBACK` was sent and before the server answered. Treated as committed for refresh
    /// after DDL (refresh more, never less: [`TxOutcome::counts_as_committed`]).
    Unknown,
}

impl TxOutcome {
    /// Whether refresh after DDL applies the transaction's pending targets: on a commit, and
    /// when the outcome is unknown (the changes may have been committed).
    pub fn counts_as_committed(&self) -> bool {
        matches!(self, TxOutcome::Committed | TxOutcome::Unknown)
    }
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
    // ## m3b-t01c-core-seams (P-M3b-3): answers to a waiting run, through the `RunGate`
    // slot; never queued behind the request that waits. `question`: the id the run's event
    // carried (review I1). `reply`: the run waited on exactly that question.
    Choose {
        run_seq: u64,
        question: u64,
        choice: RunChoice,
        reply: oneshot::Sender<bool>,
    },
    CommitDecision {
        run_seq: u64,
        unit: UnitId,
        question: u64,
        answer: CommitAnswer,
        reply: oneshot::Sender<bool>,
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

    // ## m3b-t01c-core-seams (P-M3b-3, P-M3b-25)
    /// Answers question `question` (`RunEvent::NeedsChoice::question`, Task 11) of the run sent
    /// as request `run_seq`: `true` when that run waits on exactly that question. A second
    /// answer, an answer to an older question (also one a Retry asked again for the same unit)
    /// or any other id is refused (`false`) and changes nothing (review I1). It never queues
    /// behind the running request. Without a running actor, `false`.
    pub fn choose(&self, run_seq: u64, question: u64, c: RunChoice) -> oneshot::Receiver<bool> {
        let (reply, rx) = oneshot::channel();
        let command = Command::Choose {
            run_seq,
            question,
            choice: c,
            reply,
        };
        if let Err(mpsc::error::SendError(Command::Choose { reply, .. })) =
            self.commands.send(command)
        {
            let _ = reply.send(false);
        }
        rx
    }

    /// Answers the confirm-in-transaction question `question` about `unit`
    /// (`RunEvent::NeedsCommit::question`, Task 19) of the run sent as request `run_seq`:
    /// `true` when that run waits on exactly that question; anything else is refused, as for
    /// [`choose`](Self::choose). Without a running actor, `false`.
    pub fn commit_decision(
        &self,
        run_seq: u64,
        unit: UnitId,
        question: u64,
        a: CommitAnswer,
    ) -> oneshot::Receiver<bool> {
        let (reply, rx) = oneshot::channel();
        let command = Command::CommitDecision {
            run_seq,
            unit,
            question,
            answer: a,
            reply,
        };
        if let Err(mpsc::error::SendError(Command::CommitDecision { reply, .. })) =
            self.commands.send(command)
        {
            let _ = reply.send(false);
        }
        rx
    }

    // ## m3b-t24-exec-ui (named exception: the T24 I1 ruling)
    /// The context Run classifies this session's statements in (plan amendment C2(b)/I1): its
    /// lexing-mode setting, its search path (MySQL/MariaDB: the current database) and the
    /// server version, the way the read-only guard's classifier reads them (`ClassifierContext`).
    /// Queued like any request, so it answers after every request sent before it, and a request
    /// that may have changed the context makes the session read it again first: every console
    /// sharing the session sees the same one. A read the session cannot make (a failed
    /// transaction, a suspended cursor on the connection, a lost connection) answers an
    /// unknown context, never a stale one.
    pub fn classifier_context(
        &self,
    ) -> impl std::future::Future<Output = ClassifierContext> + Send + 'static + use<> {
        let answer = self.run_context();
        async move { answer.await.0 }
    }

    // ## m3b-t24-exec-ui (fix round 2, named exception)
    /// [`SessionHandle::classifier_context`] with the context's generation, which a run
    /// classified in it carries (`RunSpec::context_generation`): the session refuses the run
    /// when the generation has moved since. `None` when the session could not answer.
    pub fn run_context(
        &self,
    ) -> impl std::future::Future<Output = (ClassifierContext, Option<u64>)> + Send + 'static + use<>
    {
        let ticket = self.submit(Request::ReadContext);
        async move {
            match ticket.reply.await {
                Ok(Ok(summary)) => (
                    run_context::decode(&summary),
                    run_context::decode_generation(&summary),
                ),
                _ => (ClassifierContext::default(), None),
            }
        }
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
// through a `RunObserver`, with console cursors. Types complete since M3a Task 1b; bodies
// since Task 18 (`run.rs`, `cursor.rs`). A read-only session's typed units run inside the
// read-only guard (`readonly::guarded_query`, Task 22).

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
    // ## m3b-t01c-core-seams
    /// A failure asks (`RunEvent::NeedsChoice`, answered by `SessionHandle::choose`;
    /// R-M3-61, Task 11) when later units wait; a failed last unit ends the run.
    Ask,
}

// ## m3b-t01c-core-seams
/// The answer to `RunEvent::NeedsChoice` (R-M3-61, P-M3b-19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunChoice {
    Retry,
    Ignore,
    IgnoreAll,
    Stop,
    /// Inside a failed PostgreSQL transaction: `ROLLBACK`, keep the session's mode, go on.
    RollbackAndContinue,
}

/// The answer to `RunEvent::NeedsCommit` (Keep/Undo in the savepoint form).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommitAnswer {
    Commit,
    RollBack,
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
    // ## m3b-t01c-core-seams
    /// The bind values, in placeholder order (Task 11 sends them with the query when the
    /// driver's typed path binds, never with `Protocol::Simple`; otherwise the unit is refused
    /// before anything is sent).
    pub binds: Arc<[BindValue]>,
    /// The exec gate's verdict (`Unknown` when no gate ran); scheduling only, never trusted
    /// for safety (R-M3-83).
    pub verdict: UnitVerdict,
    pub effects: SessionEffects,
    /// The text with the bind values masked, for the log and the history.
    pub masked: Arc<str>,
    /// Result sets kept (R-M3-66: 100), each its own result; clamped to 1..=100.
    pub max_result_sets: u32,
    /// The cell cap of this unit (R-M3-82: a re-run with a 64 MiB limit); `None` takes
    /// `RunSpec::fetch`'s.
    pub cell_cap: Option<u32>,
}

impl RunUnit {
    /// A unit as M3a sends it: no binds, no verdict, one result set, the run's cell cap.
    pub fn plain(id: UnitId, sql: Arc<str>, protocol: Protocol, suspendable: bool) -> RunUnit {
        RunUnit {
            id,
            masked: sql.clone(),
            sql,
            protocol,
            suspendable,
            binds: Arc::from(Vec::new()),
            verdict: UnitVerdict::Unknown,
            effects: SessionEffects::empty(),
            max_result_sets: 1,
            cell_cap: None,
        }
    }
}

/// One `Request::Run`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RunSpec {
    pub run: RunId,
    pub units: Arc<[RunUnit]>,
    pub policy: ErrorPolicy,
    pub fetch: FetchPolicy,
    pub observer: ObserverRef,
    // ## m3b-t01c-core-seams
    /// How long a confirm-in-transaction question waits before the statement is rolled back
    /// (D-014: [`RunSpec::CONFIRM_DEADLINE`]; tests inject a shorter one; Task 19).
    pub confirm_deadline: Duration,
    // ## m3b-t24-exec-ui (fix round 2, named exception)
    /// The session's context generation the units were classified under
    /// ([`SessionHandle::run_context`]): when it has moved by the time the run is taken, the
    /// session refuses it (`ExecErrorKind::ContextChanged`), sending nothing. `None`: not
    /// checked (a count, M3a's runs, tests).
    pub context_generation: Option<u64>,
}

impl RunSpec {
    // ## m3b-t01c-core-seams
    /// D-014's deadline: 5 minutes.
    pub const CONFIRM_DEADLINE: Duration = Duration::from_secs(300);
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
    // ## m3b-t01c-core-seams
    /// A unit failed under the `Ask` policy; the run waits for `SessionHandle::choose`
    /// (Task 11). `rollback_and_continue`: inside a failed PostgreSQL transaction (no Retry).
    NeedsChoice {
        unit: UnitId,
        /// The question's id: `SessionHandle::choose` names it (review I1).
        question: u64,
        error: ExecError,
        rollback_and_continue: bool,
    },
    /// A confirmed statement ran inside a transaction; the run waits for
    /// `SessionHandle::commit_decision` until `deadline` (Task 19). `savepoint`: the form the
    /// session chose (P-M3b-1): Keep/Undo inside an open transaction, else Commit/Roll back.
    NeedsCommit {
        unit: UnitId,
        /// The question's id: `SessionHandle::commit_decision` names it (review I1).
        question: u64,
        affected: Option<u64>,
        deadline: Instant,
        savepoint: bool,
    },
    CommitDecided {
        unit: UnitId,
        answer: CommitAnswer,
        by_deadline: bool,
    },
    // ## m3b-t26-prod-unlock (fix round 1, ruling on A-I2/B-I1)
    /// Sent right before `CommitDecided` when the rollback of `unit` did not undo it: the server
    /// warned that a table does not support transactions. The log never says "undone" then.
    ConfirmNotUndone {
        unit: UnitId,
    },
    /// A server notice of the unit (R-M3-77; Tasks 11 and 13).
    Notice {
        unit: UnitId,
        notice: hacivat_driver::Notice,
    },
    // ## m3b-t16-run-service (named exception: the run service logs these)
    /// `n` notices of `unit` past the per-unit cap (and the driver's own) were counted, never
    /// reported; their text is never kept (R-M3-77).
    NoticesDropped {
        unit: UnitId,
        n: u32,
    },
    /// A unit run as one (`Protocol::Simple`) was closed before its end (a stop), and the
    /// driver reported what ran after the closed result (`RawConn::take_close_report`, Task 13).
    ClosedEarly {
        unit: UnitId,
        report: hacivat_driver::CloseReport,
    },
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

/// Whether a session of `profile` is read-only (R-M3-29, P4): the profile says so. Every site
/// that decides a session's (or a Test connection's) read-only flag reads this.
///
/// ## m3b-t26-prod-unlock (R-M3-6 REVISED ends): a production tag no longer makes a session
/// read-only; production consoles are read-write in Manual mode (D-036) and every unit that is
/// not a read passes the exec gate and its confirmations (R-M3-55) before it is sent.
pub fn effective_read_only(profile: &Profile) -> bool {
    profile.read_only
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
