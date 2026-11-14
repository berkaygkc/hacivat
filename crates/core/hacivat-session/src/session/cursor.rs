//! Console cursors (HV-RES-05; Task 18): fetching more, fetching all, releasing a cursor
//! (another request first, the idle bound, the end of its transaction, a lost connection), and
//! the per-engine idle bounds (R-M3-20).
//!
//! A cursor is the open typed query the actor keeps after a run whose last unit suspended: its
//! [`Stream`] (the result's writer and counters), what it holds on the server ([`Hold`]) and
//! its idle deadline. The rules (ADR 0004 `m3a-t18`):
//!
//! - **Nothing else runs under a cursor.** `Core::handle` releases it before any request that
//!   is not a `FetchMore`, `FetchAll` or `CloseCursor`; a session close and the idle bound
//!   release it too. A cursor request that names another result is a no-op.
//! - **A release closes the query first, then ends what Hacivat opened.** `close_query(Drain)`
//!   for the policy's drain time (PostgreSQL: one more `Execute`; MySQL: a drain, else `KILL
//!   QUERY` on a side connection and `close_query(AfterKill)`; SQLite: steps, then finalize);
//!   then `COMMIT` when the statement ran to its end, else `ROLLBACK` (inside a user
//!   transaction: `RELEASE SAVEPOINT hv_cursor`, else `ROLLBACK TO SAVEPOINT hv_cursor; RELEASE
//!   SAVEPOINT hv_cursor`). When that statement fails the connection is discarded: Hacivat's
//!   transaction never stays open.
//! - **No driver future is dropped.** Every call on the open query runs under [`Watch::guard`],
//!   which waits for it whatever happens. Stop fetching only sets a flag the fetch loop reads
//!   between chunks; the server cancel goes out when the call in flight is `overdue` late
//!   (R-M3-16), and is awaited before anything else is sent.
//! - **Every result ends in a final state**, and every `FetchMore`/`FetchAll` on the open
//!   cursor publishes (the result is `Streaming` while it is served, then `Suspended` again,
//!   `Complete`, `Cancelled` or `Closed`).

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_driver::{
    CancelHandle, CloseHow, Closed, Phase, QueryDone, QueryEvent, RawError, RawErrorKind,
    ResultEnd, TxStatus, TypedCaps, classify,
};
use hacivat_proto::{DbmsKind, EnvTag, ErrClass};
use hacivat_results::{
    Admission, CloseReason, Completeness, CursorInfo, FetchPolicy, ResultId, ResultWriter,
    TruncReason,
};
use hacivat_values::{Chunk, ResultSchema};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::Instant as Tick;

use crate::pipeline::{ConnectError, LiveConnection};
use crate::session::actor::{Core, SideConnector};
use crate::session::cancel::{CANCEL_WAIT, is_cancel_code, kill_query_sql};
use crate::session::readonly::GuardedCursor;
use crate::session::schema::{Family, family};
use crate::session::{
    CursorBadge, ExecError, ExecErrorKind, ObserverRef, Reply, RunEvent, TxOwner, UnitId,
};

/// How long a read-only PostgreSQL cursor may stay idle: the guard's transaction is killed by
/// the server after 30 s (`idle_in_transaction_session_timeout`, R-M3-17).
pub const READ_ONLY_PG_IDLE: Duration = Duration::from_secs(20);

/// The most a cursor on a production profile may stay idle (R-M3-20).
pub const PRODUCTION_IDLE: Duration = Duration::from_secs(120);

/// How long `close_query(AfterKill)` reads on for the kill's error.
pub(crate) const KILL_WAIT: Duration = CANCEL_WAIT;

/// How often a result reports its row count while it streams.
const PROGRESS_EVERY: Duration = Duration::from_millis(50);

/// The savepoint around a cursor inside a user transaction (R-M3-23). PostgreSQL keeps
/// savepoints of one name as a stack, so a user's own `hv_cursor` is shadowed while the cursor
/// is open and intact after it: the rollback and the release act on the newest one.
pub(crate) const SAVEPOINT: &str = "SAVEPOINT hv_cursor";
pub(crate) const SAVEPOINT_RELEASE: &str = "RELEASE SAVEPOINT hv_cursor";
pub(crate) const SAVEPOINT_ROLLBACK: &str =
    "ROLLBACK TO SAVEPOINT hv_cursor; RELEASE SAVEPOINT hv_cursor";

/// How long a suspended cursor of `kind` may stay idle (R-M3-20): PostgreSQL read-write
/// `policy.idle_timeout`, read-only 20 s; MySQL/MariaDB `caps.idle_bound`; SQLite
/// `caps.idle_bound` (30 s in rollback-journal mode), else `policy.idle_timeout`; production at
/// most 120 s; `policy.idle_cap` caps every bound (Task 18). A bound the driver reports is
/// never exceeded on any engine.
pub fn idle_bound(
    kind: DbmsKind,
    read_only: bool,
    production: bool,
    caps: &TypedCaps,
    policy: &FetchPolicy,
) -> Duration {
    let mut bound = match family(kind) {
        Family::Postgres if read_only => READ_ONLY_PG_IDLE,
        _ => policy.idle_timeout,
    };
    if let Some(engine) = caps.idle_bound {
        bound = bound.min(engine);
    }
    if production {
        bound = bound.min(PRODUCTION_IDLE);
    }
    if let Some(cap) = policy.idle_cap {
        bound = bound.min(cap);
    }
    bound
}

// ---------------------------------------------------------------------------------------------
// Stop fetching (R-M3-16)

/// "Stop fetching", from the cancel ladder to the request that runs: a flag the fetch loop
/// reads between chunks, and a wake-up for a call that is waiting on the server.
#[derive(Debug, Default)]
pub(crate) struct StopSignal {
    asked: AtomicBool,
    notify: Notify,
}

impl StopSignal {
    /// Asks the running request to stop.
    pub(crate) fn request(&self) {
        self.asked.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub(crate) fn is_requested(&self) -> bool {
        self.asked.load(Ordering::SeqCst)
    }

    /// A new request starts: an earlier stop does not apply to it.
    pub(crate) fn reset(&self) {
        self.asked.store(false, Ordering::SeqCst);
    }

    /// Waits until a stop is asked for (at once when it already was).
    async fn requested(&self) {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_requested() {
                return;
            }
            notified.await;
        }
    }
}

/// Sends the engine's server-side cancel for a connection's statement: PostgreSQL's protocol
/// cancel and SQLite's interrupt through the `CancelHandle`, MySQL's `KILL QUERY` on a side
/// connection.
#[derive(Clone)]
pub(crate) struct Canceller {
    kind: DbmsKind,
    handle: Option<Arc<dyn CancelHandle>>,
    backend: Option<u64>,
    side: Arc<dyn SideConnector>,
}

impl Canceller {
    fn fire(&self) -> BoxFuture<'static, Result<(), ErrClass>> {
        let kind = self.kind;
        if family(kind) == Family::MySql {
            let Some(sql) = self.backend.and_then(|id| kill_query_sql(kind, id)) else {
                return Box::pin(async { Err(ErrClass::Internal) });
            };
            let opened = self.side.open_side();
            return Box::pin(kill_on(opened, kind, sql));
        }
        match &self.handle {
            Some(handle) => {
                let cancel = handle.cancel();
                Box::pin(async move {
                    cancel
                        .await
                        .map_err(|e| classify(kind, Phase::PostConnect, &e).class)
                })
            }
            None => Box::pin(async { Err(ErrClass::Internal) }),
        }
    }
}

/// Runs `sql` (a kill) on the side connection `opened` gives, then closes it.
async fn kill_on(
    opened: impl Future<Output = Result<LiveConnection, ConnectError>>,
    kind: DbmsKind,
    sql: String,
) -> Result<(), ErrClass> {
    let mut live = opened.await.map_err(|e| e.class)?;
    let ran = live.raw().batch(&sql).await;
    live.close().await;
    ran.map_err(|e| classify(kind, Phase::PostConnect, &e).class)
}

/// A side connection opened ahead of a kill that may be needed (a MySQL release drains for
/// 250 ms first; the connect runs meanwhile). One nobody used is closed.
struct SideSlot(Option<JoinHandle<Result<LiveConnection, ConnectError>>>);

impl SideSlot {
    fn open(side: &Arc<dyn SideConnector>) -> SideSlot {
        SideSlot(Some(tokio::spawn(side.open_side())))
    }

    /// Kills `thread`'s statement on this connection.
    async fn kill(mut self, kind: DbmsKind, thread: u64) -> Result<(), ErrClass> {
        let Some(task) = self.0.take() else {
            return Err(ErrClass::Internal);
        };
        let Some(sql) = kill_query_sql(kind, thread) else {
            return Err(ErrClass::Internal);
        };
        let mut live = match task.await {
            Ok(Ok(live)) => live,
            Ok(Err(e)) => return Err(e.class),
            Err(_) => return Err(ErrClass::Internal),
        };
        let ran = live.raw().batch(&sql).await;
        live.close().await;
        ran.map_err(|e| classify(kind, Phase::PostConnect, &e).class)
    }
}

impl Drop for SideSlot {
    fn drop(&mut self) {
        let Some(task) = self.0.take() else {
            return;
        };
        match tokio::runtime::Handle::try_current() {
            Ok(rt) => {
                rt.spawn(async move {
                    if let Ok(Ok(live)) = task.await {
                        live.close().await;
                    }
                });
            }
            Err(_) => task.abort(),
        }
    }
}

/// When [`Watch::guard`] sends the server cancel for the call it waits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fire {
    Never,
    /// Once a stop was asked for and the call is `overdue` late (R-M3-16).
    OnStop,
    /// This long after the call started, stop or not (a close that must not wait for the
    /// server: zero runs the cancel alongside it).
    After(Duration),
}

/// Watches the driver calls of one request: waits for each to its end (never drops one) and
/// sends the server cancel at most once.
pub(crate) struct Watch {
    pub(crate) stop: Arc<StopSignal>,
    canceller: Canceller,
    pub(crate) overdue: Duration,
    /// The server cancel went out.
    pub(crate) fired: bool,
    /// The cancel in flight.
    cancel: Option<BoxFuture<'static, Result<(), ErrClass>>>,
    /// A cancel did not end within [`CANCEL_WAIT`]: it may still land, so the connection must
    /// not be used again.
    unsettled: bool,
}

impl Watch {
    /// Sends the server cancel now (once per request). The next [`Watch::guard`] carries it
    /// and awaits it.
    pub(crate) fn fire_now(&mut self) {
        if !self.fired {
            self.fired = true;
            self.cancel = Some(self.canceller.fire());
        }
    }

    /// Waits for `call`. A cancel sent before or during it is awaited before this returns, so
    /// it can never reach a later statement.
    pub(crate) async fn guard<T>(&mut self, call: impl Future<Output = T>, fire: Fire) -> T {
        let started = Tick::now();
        tokio::pin!(call);
        let mut at = match fire {
            Fire::After(d) => Some(started + d),
            _ => None,
        };
        let on_stop = fire == Fire::OnStop;
        let out = loop {
            let armed = at.is_some();
            tokio::select! {
                biased;
                out = &mut call => break out,
                () = self.stop.requested(), if on_stop && !armed && !self.fired => {
                    at = Some(started + self.overdue);
                }
                () = sleep_until_opt(at), if armed && !self.fired => self.fire_now(),
                sent = poll_opt(&mut self.cancel), if self.cancel.is_some() => {
                    self.cancel = None;
                    if let Err(class) = sent {
                        tracing::debug!(class = class.id(), "the server cancel failed");
                    }
                }
            }
        };
        if let Some(cancel) = self.cancel.take()
            && tokio::time::timeout(CANCEL_WAIT, cancel).await.is_err()
        {
            self.unsettled = true;
        }
        out
    }
}

async fn sleep_until_opt(at: Option<Tick>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

async fn poll_opt<T>(f: &mut Option<BoxFuture<'static, T>>) -> T {
    match f {
        Some(f) => f.await,
        None => std::future::pending().await,
    }
}

// ---------------------------------------------------------------------------------------------
// One result's stream

/// What an open query holds on the server besides itself.
#[derive(Debug)]
pub(crate) enum Hold {
    /// Nothing of Hacivat's (MySQL, SQLite, a unit that never suspends).
    None,
    /// Hacivat's own `BEGIN` (PostgreSQL in autocommit, R-M3-23).
    OwnTx,
    /// `SAVEPOINT hv_cursor` inside the user's transaction.
    Savepoint,
    /// The read-only guard's transaction (Task 22).
    Guard(GuardedCursor),
}

impl Hold {
    fn holds_tx(&self) -> bool {
        !matches!(self, Hold::None)
    }
}

/// How one pass over a query's events ended.
pub(crate) enum Pumped {
    Done,
    Suspended,
    /// A stop was asked for; the query is still open.
    Stopped,
    Failed(RawError),
}

/// What the fetch loop decided.
pub(crate) enum Step {
    Complete,
    /// The target is reached, or a memory budget pauses the result; the query is suspended.
    Suspend,
    Stopped,
    Failed(RawError),
}

/// The first result set of one typed query on its way into the store.
pub(crate) struct Stream {
    pub(crate) unit: UnitId,
    pub(crate) observer: ObserverRef,
    pub(crate) fetch: FetchPolicy,
    /// The query was sent `suspendable` (PostgreSQL: a named portal).
    pub(crate) suspendable: bool,
    /// `None` before `ResultStart`, and for a result without columns (DML, DDL).
    writer: Option<ResultWriter>,
    pub(crate) started: bool,
    pub(crate) columns: usize,
    /// Rows in the store.
    pub(crate) rows: u64,
    /// Rows the driver handed over (`rows` plus those counted past the cap).
    pub(crate) seen: u64,
    /// The per-result cap or the global budget was reached: rows are counted, not kept.
    pub(crate) capped: bool,
    bytes: u64,
    last_chunk_bytes: u64,
    /// When the statement was sent.
    sent_at: Tick,
    pub(crate) exec_ms: u64,
    fetching: Duration,
    mark: Tick,
    last_progress: Option<Tick>,
    pub(crate) end: Option<ResultEnd>,
    pub(crate) done: Option<QueryDone>,
    opened_at: std::time::Instant,
    /// `finish` ran: the result has its final state.
    ended: bool,
}

/// A stream that goes away without its final state (its request was dropped by a deactivate,
/// or unwound by a panic) ends its result as the writer's own drop guard would: `Closed {
/// Cancelled }`, rows kept, and the observer hears of it.
impl Drop for Stream {
    fn drop(&mut self) {
        if !self.ended && self.writer.is_some() {
            self.finish(Completeness::Closed {
                reason: CloseReason::Cancelled,
            });
        }
    }
}

impl Stream {
    pub(crate) fn new(
        unit: UnitId,
        observer: ObserverRef,
        fetch: FetchPolicy,
        suspendable: bool,
    ) -> Stream {
        let now = Tick::now();
        Stream {
            unit,
            observer,
            fetch,
            suspendable,
            writer: None,
            started: false,
            columns: 0,
            rows: 0,
            seen: 0,
            capped: false,
            bytes: 0,
            last_chunk_bytes: 0,
            sent_at: now,
            exec_ms: 0,
            fetching: Duration::ZERO,
            mark: now,
            last_progress: None,
            end: None,
            done: None,
            opened_at: now.into_std(),
            ended: false,
        }
    }

    pub(crate) fn result(&self) -> Option<ResultId> {
        self.writer.as_ref().map(ResultWriter::id)
    }

    pub(crate) fn fetch_ms(&self) -> u64 {
        millis(self.fetching)
    }

    /// The first response came: the statement's own time ends here.
    pub(crate) fn on_first_response(&mut self) {
        if !self.started {
            self.started = true;
            self.exec_ms = millis(self.sent_at.elapsed());
            self.mark = Tick::now();
        }
    }

    fn on_start(&mut self, schema: Arc<ResultSchema>) {
        if self.started {
            // A second result set (the driver keeps one, R-M3-25): its rows are not this
            // result's.
            self.capped = true;
            return;
        }
        self.on_first_response();
        self.columns = schema.columns.len();
        if self.columns == 0 {
            return;
        }
        let writer = self.observer.0.open_result(self.unit, 0, schema);
        let result = writer.id();
        self.writer = Some(writer);
        self.observer.0.event(&RunEvent::ResultOpened {
            unit: self.unit,
            ordinal: 0,
            result,
        });
    }

    /// Takes one chunk. A suspendable query's batch was admitted before it was asked for; a
    /// stream that cannot stop is admitted chunk by chunk, and past a budget its rows are
    /// counted, not kept.
    fn on_rows(&mut self, chunk: Chunk) {
        let now = Tick::now();
        self.fetching += now.saturating_duration_since(self.mark);
        self.mark = now;
        let rows = u64::from(chunk.rows());
        self.seen = self.seen.saturating_add(rows);
        let Some(writer) = self.writer.as_mut() else {
            return;
        };
        if !self.suspendable && !self.capped && writer.admit(chunk.bytes()) != Admission::Ok {
            self.capped = true;
        }
        if self.capped {
            return;
        }
        self.last_chunk_bytes = chunk.bytes();
        self.bytes = self.bytes.saturating_add(chunk.bytes());
        self.rows = self.rows.saturating_add(rows);
        writer.append(chunk);
        if self
            .last_progress
            .is_none_or(|at| now.saturating_duration_since(at) >= PROGRESS_EVERY)
        {
            self.last_progress = Some(now);
            self.observer.0.event(&RunEvent::RowsFetched {
                result: writer.id(),
                total: self.rows,
            });
        }
    }

    /// A fetch resumes: the idle time before it is not fetch time.
    fn resume(&mut self) {
        self.mark = Tick::now();
        if let Some(writer) = self.writer.as_mut() {
            writer.set_state(Completeness::Streaming);
        }
    }

    /// Whether the next batch fits the memory budgets (the answer reaches the next publish).
    fn admits_next(&self) -> bool {
        self.writer
            .as_ref()
            .is_none_or(|w| w.admit(self.last_chunk_bytes) == Admission::Ok)
    }

    /// Sets the result's state (a final one, or `Suspended`) and publishes.
    pub(crate) fn set_state(&mut self, state: Completeness) {
        if let Some(writer) = self.writer.as_mut() {
            writer.set_state(state);
        }
    }

    /// The result ended in `state`: publishes it and tells the observer.
    pub(crate) fn finish(&mut self, state: Completeness) {
        self.ended = true;
        self.set_state(state);
        if let Some(result) = self.result() {
            self.observer.0.event(&RunEvent::ResultEnded {
                result,
                rows: self.rows,
                completeness: state,
                exec_ms: self.exec_ms,
                fetch_ms: self.fetch_ms(),
            });
        }
    }

    /// The open query was closed before its result ended (a release, a stop, a failure).
    /// `rolled_back`: Hacivat's own transaction or savepoint ended in a rollback (R-M3-23).
    pub(crate) fn released(&self, reason: CloseReason, rolled_back: bool) {
        if let Some(result) = self.result() {
            self.observer.0.event(&RunEvent::CursorReleased {
                result,
                rows: self.rows,
                reason,
                rolled_back,
            });
        }
    }

    /// The state of a result whose statement ran to its end, once its hold ended as `ended`:
    /// a gap the guard found there closes it (its rows stay), a lost connection too.
    pub(crate) fn state_after(&self, ended: &Result<(), ExecError>) -> Completeness {
        match ended {
            Err(e) if matches!(e.kind, ExecErrorKind::EnforcementGap(_)) => Completeness::Closed {
                reason: CloseReason::EnforcementGap,
            },
            _ => self.completed_state(),
        }
    }

    /// The state of a result whose statement ran to its end.
    pub(crate) fn completed_state(&self) -> Completeness {
        if self.capped && self.seen > self.rows {
            Completeness::Truncated(TruncReason::ResultCap)
        } else if self.done.is_some_and(|d| d.skipped_result_sets > 0) {
            Completeness::Truncated(TruncReason::FollowingResultSets)
        } else {
            Completeness::Complete
        }
    }
}

pub(crate) fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// The suspended query a session holds.
pub(crate) struct OpenCursor {
    s: Stream,
    hold: Hold,
    info: CursorInfo,
    /// When the idle bound releases it.
    idle_at: Tick,
}

impl OpenCursor {
    pub(crate) fn result(&self) -> Option<ResultId> {
        self.s.result()
    }

    pub(crate) fn idle_at(&self) -> Tick {
        self.idle_at
    }

    /// Whether the cursor sits inside the user's own transaction.
    pub(crate) fn in_user_tx(&self, tx: TxStatus, owner: TxOwner) -> bool {
        tx != TxStatus::Idle
            && matches!(self.hold, Hold::Savepoint | Hold::None)
            && owner == TxOwner::User
    }

    fn badge(&self) -> Option<CursorBadge> {
        Some(CursorBadge {
            result: self.s.result()?,
            rows: self.s.rows,
            since: self.info.opened_at,
            holds_tx: self.info.holds_tx,
        })
    }
}

fn raw(kind: RawErrorKind) -> RawError {
    RawError { kind, server: None }
}

impl Core {
    /// The session's open cursor, as its snapshot shows it.
    pub(crate) fn cursor_badge(&self) -> Option<CursorBadge> {
        self.cursor.as_ref().and_then(OpenCursor::badge)
    }

    /// When the idle bound releases the open cursor.
    pub(crate) fn cursor_deadline(&self) -> Option<Tick> {
        self.cursor.as_ref().map(OpenCursor::idle_at)
    }

    /// The watch of one request's driver calls. `stop`: the request's stop signal, or none for
    /// work no cancel can name (the idle bound, a closing session).
    pub(crate) fn watch(&mut self, stop: Option<Arc<StopSignal>>) -> Watch {
        let (handle, backend) = match self.live.as_mut() {
            Some(live) => {
                let conn = live.raw();
                (conn.cancel_handle(), conn.backend_id())
            }
            None => (None, None),
        };
        Watch {
            stop: stop.unwrap_or_default(),
            canceller: Canceller {
                kind: self.kind,
                handle,
                backend,
                side: self.side.clone(),
            },
            overdue: FetchPolicy::default().overdue,
            fired: false,
            cancel: None,
            unsettled: false,
        }
    }

    /// A cancel that did not settle may still reach the connection: it is never used again.
    pub(crate) fn note_settle(&mut self, w: &mut Watch) {
        if std::mem::take(&mut w.unsettled) {
            tracing::warn!("a server cancel did not end in time; the session is broken");
            self.broken.get_or_insert(ErrClass::Cancelled);
        }
    }

    fn typed_caps(&mut self) -> TypedCaps {
        self.live
            .as_mut()
            .map_or(TypedCaps::NONE, |live| live.raw().typed_caps())
    }

    /// Reads the open query's events into `s` until it ends, suspends or fails, or until a
    /// stop is seen. After every chunk the actor gets a turn to read its commands (R-M3-16), so
    /// a stop is seen within one chunk even when the driver is always ready.
    pub(crate) async fn pump(&mut self, s: &mut Stream, w: &mut Watch) -> Pumped {
        loop {
            let event = match self.live.as_mut() {
                Some(live) => w.guard(live.raw().next_event(), Fire::OnStop).await,
                None => Err(raw(RawErrorKind::Io(std::io::ErrorKind::NotConnected))),
            };
            self.note_settle(w);
            match event {
                Err(e) => return Pumped::Failed(e),
                Ok(QueryEvent::ResultStart(schema)) => s.on_start(schema),
                Ok(QueryEvent::Rows(chunk)) => {
                    s.on_first_response();
                    s.on_rows(chunk);
                    tokio::task::yield_now().await;
                    if w.stop.is_requested() {
                        return Pumped::Stopped;
                    }
                }
                Ok(QueryEvent::ResultEnd(end)) => {
                    s.on_first_response();
                    if s.end.is_none() {
                        s.end = Some(end);
                    }
                }
                Ok(QueryEvent::Suspended) => return Pumped::Suspended,
                Ok(QueryEvent::Done(done)) => {
                    s.on_first_response();
                    s.done = Some(done);
                    return Pumped::Done;
                }
            }
        }
    }

    /// Fetches until `s` holds `target` rows (or `byte_target` bytes), the result ends, a
    /// budget pauses it, a stop is seen or the driver fails. Each batch is at most one chunk's
    /// rows, admitted before it is asked for, so nothing in flight exceeds a chunk.
    pub(crate) async fn fetch_until(
        &mut self,
        s: &mut Stream,
        first: Pumped,
        target: u64,
        byte_target: Option<u64>,
        w: &mut Watch,
    ) -> Step {
        let mut pumped = first;
        loop {
            match pumped {
                Pumped::Done => return Step::Complete,
                Pumped::Stopped => return Step::Stopped,
                Pumped::Failed(e) => return Step::Failed(e),
                Pumped::Suspended => {}
            }
            if w.stop.is_requested() {
                return Step::Stopped;
            }
            if s.rows >= target || byte_target.is_some_and(|b| s.bytes >= b) || !s.admits_next() {
                return Step::Suspend;
            }
            let batch = (target - s.rows).min(u64::from(s.fetch.chunk_rows.max(1)));
            let batch = u32::try_from(batch).unwrap_or(u32::MAX);
            let resumed = match self.live.as_mut() {
                Some(live) => w.guard(live.raw().fetch_more(batch), Fire::OnStop).await,
                None => Err(raw(RawErrorKind::Io(std::io::ErrorKind::NotConnected))),
            };
            self.note_settle(w);
            if let Err(e) = resumed {
                return Step::Failed(e);
            }
            pumped = self.pump(s, w).await;
        }
    }

    /// Closes the open query `how`. MySQL's `NeedsServerKill` is served here: `KILL QUERY` for
    /// the connection's own thread on a side connection (opened ahead when `prewarm`), then
    /// `close_query(AfterKill)`. A kill that cannot be sent leaves an unread result on the
    /// wire, so the connection is discarded.
    pub(crate) async fn close_open(
        &mut self,
        how: CloseHow,
        fire: Fire,
        prewarm: bool,
        w: &mut Watch,
    ) -> Result<Closed, RawError> {
        let lost = || raw(RawErrorKind::Io(std::io::ErrorKind::NotConnected));
        let kills = self.typed_caps().close_needs_kill;
        let mut slot = (kills && prewarm).then(|| SideSlot::open(&self.side));
        // "Alongside" means the cancel is on its way when the close starts to read.
        if fire == Fire::After(Duration::ZERO) {
            w.fire_now();
        }
        let closed = match self.live.as_mut() {
            Some(live) => w.guard(live.raw().close_query(how), fire).await,
            None => Err(lost()),
        };
        self.note_settle(w);
        let Ok(Closed::NeedsServerKill { thread_id }) = closed else {
            return closed;
        };
        let slot = slot.take().unwrap_or_else(|| SideSlot::open(&self.side));
        if let Err(class) = slot.kill(self.kind, thread_id).await {
            tracing::debug!(class = class.id(), "the kill before a close failed");
            self.broken.get_or_insert(class);
            return Err(lost());
        }
        let conn = self.live.as_mut().ok_or_else(lost)?.raw();
        let after = conn.close_query(CloseHow::AfterKill(KILL_WAIT)).await;
        match after {
            // Still asking: the driver did not take the kill. Never loop; discard.
            Ok(Closed::NeedsServerKill { .. }) => {
                self.broken.get_or_insert(ErrClass::Internal);
                Err(lost())
            }
            Ok(Closed::Completed { rows }) => {
                // The statement ended before the kill landed; a server that keeps the kill
                // for the thread's next statement spends it on this one.
                let _ = conn.batch("DO 0").await;
                Ok(Closed::Completed { rows })
            }
            other => other,
        }
    }

    /// When a close's server cancel goes out. A stop: at once for a stream that cannot be
    /// closed early on the wire (PostgreSQL's `Execute(0)` is read to its end, a SQLite step
    /// is not interrupted by a close), else once it is `overdue` late. A release: only past
    /// its own drain time. Never on MySQL, whose close asks for its kill itself.
    fn close_fire(&self, s: &Stream, stopping: bool, overdue: Duration) -> Fire {
        match family(self.kind) {
            Family::MySql => Fire::Never,
            Family::Sqlite if stopping => Fire::After(Duration::ZERO),
            Family::Postgres if stopping && !s.suspendable => Fire::After(Duration::ZERO),
            _ if stopping => Fire::After(overdue),
            _ => Fire::After(s.fetch.release_drain.saturating_add(overdue)),
        }
    }

    /// Ends what Hacivat opened around a query that is closed: `COMMIT` or the savepoint's
    /// release when the statement ran to its end, else the rollback; the guard's steps on a
    /// read-only session. Returns whether a rollback undid the statement, and the failure. A
    /// failure discards the connection: Hacivat's transaction is never left open.
    pub(crate) async fn end_hold(
        &mut self,
        hold: Hold,
        completed: bool,
    ) -> (bool, Result<(), ExecError>) {
        if self.broken.is_some() || self.live.is_none() {
            self.forget_hold(&hold);
            return (false, Ok(()));
        }
        let (sql, rolled_back) = match hold {
            Hold::None => return (false, Ok(())),
            Hold::OwnTx if completed => ("COMMIT", false),
            Hold::OwnTx => ("ROLLBACK", true),
            Hold::Savepoint if completed => (SAVEPOINT_RELEASE, false),
            Hold::Savepoint => (SAVEPOINT_ROLLBACK, true),
            Hold::Guard(guard) => {
                let replay = self.env_plan().replay();
                let finished = match self.live.as_mut() {
                    Some(live) => guard.finish(live.raw(), &replay).await,
                    None => Ok(()),
                };
                self.holder = TxOwner::User;
                let ended = finished.map_err(|failure| {
                    if let Some(class) = failure.discard {
                        self.broken.get_or_insert(class);
                    }
                    failure.error
                });
                self.observe(false);
                return (false, ended);
            }
        };
        let own = sql == "COMMIT" || sql == "ROLLBACK";
        let ended = self.control(sql).await;
        if own {
            self.holder = TxOwner::User;
            if self.broken.is_none() && self.tx.status != TxStatus::Idle {
                // The transaction is still there: nothing may run in it.
                self.broken.get_or_insert(ErrClass::Internal);
            }
            self.observe(false);
        } else if ended.is_err() {
            self.broken.get_or_insert(ErrClass::Internal);
        }
        if self.broken.is_some() {
            self.forget_own_tx(own);
        }
        // A commit that failed rolled back.
        (rolled_back || (own && ended.is_err()), ended)
    }

    /// The connection is gone, and Hacivat's own transaction with it: the session has no
    /// transaction of the user's to report lost.
    fn forget_hold(&mut self, hold: &Hold) {
        self.forget_own_tx(matches!(hold, Hold::OwnTx | Hold::Guard(_)));
    }

    fn forget_own_tx(&mut self, own: bool) {
        if own {
            self.holder = TxOwner::User;
            self.tx.status = TxStatus::Idle;
            self.tx.since = None;
            self.tx.statements = 0;
            self.tx.owner = TxOwner::None;
        }
    }

    /// Keeps `s` as the session's cursor: the result is `Suspended` (with the pause reason of
    /// the last admission, if a budget stopped it) and the idle bound starts over.
    pub(crate) fn suspend(&mut self, mut s: Stream, hold: Hold) {
        // After `query`: SQLite's bound depends on the databases the statement opened.
        let caps = self.typed_caps();
        let production = self.profile.environment == Some(EnvTag::Production);
        let bound = idle_bound(self.kind, self.read_only, production, &caps, &s.fetch);
        let now = Tick::now();
        let info = CursorInfo {
            opened_at: s.opened_at,
            last_fetch: now.into_std(),
            holds_tx: hold.holds_tx(),
            idle_bound: bound,
        };
        s.set_state(Completeness::Suspended(info));
        self.cursor = Some(OpenCursor {
            s,
            hold,
            info,
            idle_at: now + bound,
        });
    }

    /// Ends a query that is closed (or gone) and whose result does not continue: ends the
    /// hold, sets the final state and reports the release. `closed`: what the close answered.
    /// `Err` only when the connection can no longer be used, or the guard found a gap.
    async fn settle_closed(
        &mut self,
        mut s: Stream,
        hold: Hold,
        reason: CloseReason,
        closed: Result<Closed, RawError>,
    ) -> Result<(), ExecError> {
        let (completed, total) = match closed {
            Ok(Closed::Completed { rows }) => (true, rows),
            Ok(_) => (false, 0),
            Err(e) => {
                let _ = self.raw_error(&e, false);
                (false, 0)
            }
        };
        let (rolled_back, ended) = self.end_hold(hold, completed).await;
        let gap = matches!(
            &ended,
            Err(e) if matches!(e.kind, ExecErrorKind::EnforcementGap(_))
        );
        let lost = self.broken.is_some();
        // Rows a drain read past the store are not shown: only a result that lost none is
        // complete.
        let whole = completed && total == s.seen && !s.capped;
        let reason = if gap {
            CloseReason::EnforcementGap
        } else if lost {
            CloseReason::ConnectionLost
        } else {
            reason
        };
        if whole && !gap && !lost {
            s.finish(Completeness::Complete);
        } else {
            s.released(reason, rolled_back);
            s.finish(match reason {
                CloseReason::Cancelled => Completeness::Cancelled,
                reason => Completeness::Closed { reason },
            });
        }
        if gap {
            return ended;
        }
        if lost {
            return Err(ExecError::of(ExecErrorKind::ConnectionLost {
                outcome_unknown: false,
            }));
        }
        Ok(())
    }

    /// Releases the open cursor for `reason` (the module docs); nothing without one.
    pub(crate) async fn release_cursor(
        &mut self,
        reason: CloseReason,
        w: &mut Watch,
    ) -> Result<(), ExecError> {
        let Some(OpenCursor { s, hold, .. }) = self.cursor.take() else {
            return Ok(());
        };
        self.release_stream(s, hold, reason, w).await
    }

    /// Releases a suspended query that is not (or no longer) the session's cursor.
    pub(crate) async fn release_stream(
        &mut self,
        s: Stream,
        hold: Hold,
        reason: CloseReason,
        w: &mut Watch,
    ) -> Result<(), ExecError> {
        let fire = self.close_fire(&s, false, w.overdue);
        let how = CloseHow::Drain(s.fetch.release_drain);
        let closed = self.close_open(how, fire, true, w).await;
        self.settle_closed(s, hold, reason, closed).await
    }

    /// Releases the open cursor outside a request (the idle bound).
    pub(crate) async fn release_unasked(&mut self, reason: CloseReason) -> Result<(), ExecError> {
        let mut w = self.watch(None);
        self.release_cursor(reason, &mut w).await
    }

    /// Releases the open cursor of a session that is closing. An engine that closes a result
    /// early by a kill is not asked to: closing its connection abandons the result.
    pub(crate) async fn release_closing(&mut self) -> Result<(), ExecError> {
        if self.typed_caps().close_needs_kill {
            self.cursor_lost(CloseReason::TabClosed);
            return Ok(());
        }
        let Some(OpenCursor { s, hold, .. }) = self.cursor.take() else {
            return Ok(());
        };
        let mut w = self.watch(None);
        let fire = Fire::After(w.overdue);
        let closed = self.close_open(CloseHow::Now, fire, false, &mut w).await;
        self.settle_closed(s, hold, CloseReason::TabClosed, closed)
            .await
    }

    /// The connection under the open cursor is gone (or about to be dropped): its rows stay,
    /// the result is `Closed` for `reason`.
    pub(crate) fn cursor_lost(&mut self, reason: CloseReason) {
        let Some(OpenCursor { mut s, hold, .. }) = self.cursor.take() else {
            return;
        };
        self.forget_hold(&hold);
        s.released(reason, false);
        s.finish(Completeness::Closed { reason });
    }

    /// A stop was seen while `s`'s query is open: closes it the engine's way and ends the
    /// hold. Returns whether the statement ran to its end all the same (`Execute(0)` read to
    /// its end before the cancel landed; a kill that came too late), and the failure that is
    /// not the cancel's own.
    pub(crate) async fn stop_stream(
        &mut self,
        s: &mut Stream,
        hold: Hold,
        w: &mut Watch,
    ) -> (bool, Option<ExecError>) {
        let fire = self.close_fire(s, true, w.overdue);
        let closed = self.close_open(CloseHow::Now, fire, true, w).await;
        let (completed, total, failure) = match closed {
            Ok(Closed::Completed { rows }) => (true, rows, None),
            Ok(_) => (false, 0, None),
            Err(e) => {
                let e = self.raw_error(&e, true);
                let cancelled = e.kind == ExecErrorKind::Cancelled || is_cancel_code(self.kind, &e);
                (false, 0, (!cancelled).then_some(e))
            }
        };
        let (rolled_back, ended) = self.end_hold(hold, completed).await;
        let whole = completed && total == s.seen && !s.capped;
        let failure = failure.or(ended.err());
        if whole && self.broken.is_none() {
            let state = s.completed_state();
            s.finish(state);
        } else {
            let (reason, state) = if self.broken.is_some() {
                let reason = CloseReason::ConnectionLost;
                (reason, Completeness::Closed { reason })
            } else {
                (CloseReason::Cancelled, Completeness::Cancelled)
            };
            if s.suspendable {
                s.released(reason, rolled_back);
            }
            s.finish(state);
        }
        (completed, failure)
    }

    /// The driver failed `s`'s query with `e`: makes sure nothing of it stays open, ends the
    /// hold with a rollback and sets the result's final state. Returns the error as the
    /// session reports it.
    pub(crate) async fn fail_stream(
        &mut self,
        s: &mut Stream,
        hold: Hold,
        e: &RawError,
        w: &mut Watch,
    ) -> ExecError {
        let error = self.raw_error(e, true);
        s.on_first_response();
        if self.broken.is_none() {
            // An error ends the query on every driver; closing what is already closed sends
            // nothing.
            let _ = self.close_open(CloseHow::Now, Fire::Never, false, w).await;
        }
        let (rolled_back, _) = self.end_hold(hold, false).await;
        let cancelled = w.stop.is_requested()
            && (error.kind == ExecErrorKind::Cancelled || is_cancel_code(self.kind, &error));
        let (reason, state) = if self.broken.is_some() {
            let reason = CloseReason::ConnectionLost;
            (reason, Completeness::Closed { reason })
        } else if cancelled {
            (CloseReason::Cancelled, Completeness::Cancelled)
        } else {
            let reason = CloseReason::ServerTerminated;
            (reason, Completeness::Closed { reason })
        };
        if s.suspendable {
            s.released(reason, rolled_back);
        }
        s.finish(state);
        error
    }

    /// Whether `result` is the open cursor's.
    fn is_open(&self, result: ResultId) -> bool {
        self.cursor
            .as_ref()
            .is_some_and(|c| c.result() == Some(result))
    }

    /// Serves a `FetchMore` (`rows` more) or a `FetchAll` (`None`) on the open cursor.
    async fn fetch(&mut self, result: ResultId, rows: Option<u64>, w: &mut Watch) -> Reply {
        // A late request (the result completed, was released, or is another one's): a no-op.
        if !self.is_open(result) {
            return Ok(self.summary());
        }
        let Some(OpenCursor { mut s, hold, .. }) = self.cursor.take() else {
            return Ok(self.summary());
        };
        w.overdue = s.fetch.overdue;
        // `Streaming` while it is served, so the source asks for nothing more meanwhile.
        s.resume();
        let target = rows.map_or(u64::MAX, |n| s.rows.saturating_add(n));
        let step = self
            .fetch_until(&mut s, Pumped::Suspended, target, None, w)
            .await;
        match step {
            Step::Suspend => {
                self.suspend(s, hold);
                Ok(self.summary())
            }
            Step::Complete => {
                let (_, ended) = self.end_hold(hold, true).await;
                let state = s.state_after(&ended);
                s.finish(state);
                ended.map(|()| self.summary())
            }
            Step::Stopped => {
                let (_, failure) = self.stop_stream(&mut s, hold, w).await;
                Err(failure.unwrap_or_else(|| ExecError::of(ExecErrorKind::Cancelled)))
            }
            Step::Failed(e) => Err(self.fail_stream(&mut s, hold, &e, w).await),
        }
    }

    /// `Request::FetchMore` (Task 18).
    pub(crate) async fn fetch_more(&mut self, result: ResultId, rows: u64, w: &mut Watch) -> Reply {
        self.fetch(result, Some(rows), w).await
    }

    /// `Request::FetchAll` (Task 18).
    pub(crate) async fn fetch_all(&mut self, result: ResultId, w: &mut Watch) -> Reply {
        self.fetch(result, None, w).await
    }

    /// `Request::CloseCursor` (Task 18).
    pub(crate) async fn close_cursor(
        &mut self,
        result: ResultId,
        reason: CloseReason,
        w: &mut Watch,
    ) -> Reply {
        if !self.is_open(result) {
            return Ok(self.summary());
        }
        self.release_cursor(reason, w).await?;
        Ok(self.summary())
    }
}
