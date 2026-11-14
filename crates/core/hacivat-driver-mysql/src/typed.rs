//! Typed streaming on MySQL and MariaDB (HV-RES-04 MySQL; Task 14): the typed `RawConn` methods'
//! engine side over the vendored `mysql_async`, chunked into `hacivat_values::Chunk`s through
//! [`crate::decode`]. The grammar and the session's rules are `hacivat_driver::typed`'s.
//!
//! **One query.** `query` keeps M2b's routing (`exec::extended_route`): a `SET` runs over
//! `COM_QUERY` (text protocol); a text the read-only connection must not run either way is
//! `Unsupported` before anything is sent; everything else is prepared (`prep`, so a statement
//! with a `?` placeholder is refused, `Unsupported`, before it runs: binds are M3b) and executed
//! (`COM_STMT_EXECUTE`, binary protocol). A statement the server cannot prepare
//! (`ER_UNSUPPORTED_PS`, 1295, from the prepare only) is `Unsupported` on a read-only connection
//! (the guard layer's refusal is never bypassed) and on a read-write one runs once more over
//! `COM_QUERY`, decoded as text. `query` returns once the first result's header (or its OK
//! packet) is read.
//!
//! **Suspension without a self-referential struct.** `mysql_async` keeps the pending result
//! inside `Conn` and `QueryResult` has no `Drop` that drains, so every read builds a
//! `QueryResult::new(&mut conn)` with the protocol type parameter the query started with
//! ([`Wire`], kept in [`Open`]) and drops it again: between calls the rows wait in the socket
//! (TCP backpressure; the server's `net_write_timeout` bounds how long, hence the idle bound in
//! [`TypedState::caps`]). Only a binary-protocol unit that is not a `CALL` suspends, because only
//! it has exactly one result set: a multi-result unit never suspends. It stops after its first
//! chunk and after each `fetch_more(n)` batch, but only after reading one row ahead: a result
//! that ends exactly at the limit ends there instead of suspending.
//!
//! **Result sets (R-M3-25).** The first `max_result_sets` results are kept (the first one even
//! with an empty schema: DML, DDL, `SET`); a later one with columns is kept while that limit
//! allows, and every other result set is read to its end, undecoded, and counted
//! (`QueryDone::skipped_result_sets`). OK packets between them carry only counts. The last kept
//! result's `ResultEnd` goes out after that drain, so its `rows_affected` is the unit's final OK
//! packet's, as `execute` reports it (a `CALL`'s write count), except that a result with
//! columns and no affected row reports `None` (a read, as on PostgreSQL and SQLite). A later
//! statement's error is reported after that `ResultEnd`.
//!
//! **Closing early.** MySQL cannot close a result at once: the rest is read or the server's
//! thread is killed. `close_query(Drain(d))` reads the rest (every remaining result set) for at
//! most `d`; when it runs out the answer is `NeedsServerKill { thread_id }` (this connection's
//! own thread ID from the handshake: the session sends `KILL QUERY <id>` on its side connection).
//! `Now` asks for the kill without reading, unless nothing is left unread (then it finishes
//! here). `AfterKill(d)` reads on to the kill's error (1317) or the end, at most `d`. The
//! outcome follows Task 12's rule: a rest read to its end is `Completed { rows }` (whatever
//! `how` was: the statement finished, the kill came too late), the kill's 1317 is
//! `Closed { rows: delivered }`, any other server error is that error (after the status is read
//! again). **No `mysql_async` future is ever dropped in flight:** the drain is a boxed future
//! that owns the `Conn` and is kept in [`TypedState`] when its time runs out, so `AfterKill`
//! resumes the very same read; it is dropped only with the connection (an `AfterKill` that runs
//! out too, or `close`), and then its `Conn` is abandoned.
//!
//! **Errors.** A server error (an ERR packet) leaves the connection in step: the query ends and
//! one `COM_PING` reads the transaction status again (an ERR packet has none, M2b's rule).
//! Anything else breaks the connection, which is never used again: `Protocol` for a malformed or
//! out-of-shape message (a row with too few or too many values, an empty packet, a column count
//! of 0 or past 65,535 (patch §6), a binary result set with a column type `mysql_common` cannot
//! read, a `LOCAL INFILE` request), else the I/O error. **A broken or discarded connection is
//! abandoned** (patch §7: the socket closed, nothing read, no `COM_QUIT`), never dropped:
//! `mysql_async`'s `Drop` would read the pending result in the background, parsing hostile rows
//! (and panicking on a refused column type) or a suspended cursor's whole rest.
//!
//! **Statements meanwhile.** Any other command first drains the pending result inside
//! `mysql_async` (silently, possibly gigabytes), so while a query is open (streaming, suspended
//! or draining) `execute`, `batch`, `ping`, `server_info`, `current_user`, `list_databases`,
//! `reset` and a second `query` answer `Unsupported`; `tls_summary` and `probe` answer from
//! what the connection already knows, without a round trip.
//!
//! **A dropped call poisons the connection.** `query`, `next_event` and `close_query` set
//! [`TypedState::poisoned`] for as long as they run (`connect.rs`): a future dropped before it
//! completed (mid-prepare, say) leaves a reply in the socket that the next command would take
//! for its own, so the next call abandons the connection and answers `Io(NotConnected)`.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::time::Duration;

use hacivat_driver::{
    CloseHow, Closed, Protocol, QueryDone, QueryEvent, QuerySpec, RawError, RawErrorKind,
    ResultEnd, TxStatus, TypedCaps,
};
use hacivat_values::{Chunk, ChunkBuilder};
use mysql_async::prelude::{Protocol as WireProtocol, Queryable};
use mysql_async::{BinaryProtocol, Column, Conn, QueryResult, Row, TextProtocol};

use crate::connect::raw_error;
use crate::decode::{Plan, Wire, binary_decodable};
use crate::exec::{ExtendedRoute, command_tag, extended_route, tx_from_status};

/// `ER_UNSUPPORTED_PS`.
const UNSUPPORTED_PS: u16 = 1295;
/// `ER_QUERY_INTERRUPTED`: a `KILL QUERY` ended the statement.
const QUERY_INTERRUPTED: u16 = 1317;
/// How much below the server's `net_write_timeout` a suspended cursor may idle (ER6).
const IDLE_MARGIN: Duration = Duration::from_secs(15);

/// What a connection's typed path knows and holds.
pub(crate) struct TypedState {
    pub caps: TypedCaps,
    /// This connection's thread ID (the handshake's), kept apart from the `Conn` because a drain
    /// may hold the `Conn`.
    pub thread_id: u64,
    pub open: Option<Open>,
    /// A drain that ran out of time, holding the `Conn` until `AfterKill` resumes it.
    draining: Option<Drain>,
    /// A call that talks to the server is running, or was dropped before it completed (then
    /// the connection is out of step and is never used again).
    pub poisoned: bool,
    /// Test seam: every query over `COM_QUERY` (text protocol).
    pub force_text: bool,
}

impl TypedState {
    /// The state of a connection with thread ID `thread_id` whose server reported
    /// `net_write_timeout` (`None`: unknown, so nothing suspends).
    pub fn new(thread_id: u64, net_write_timeout: Option<Duration>) -> Self {
        TypedState {
            caps: caps_for(net_write_timeout),
            thread_id,
            open: None,
            draining: None,
            poisoned: false,
            force_text: false,
        }
    }

    /// Whether a query is open (streaming, suspended or draining): every other command
    /// answers `Unsupported`.
    pub fn busy(&self) -> bool {
        self.open.is_some() || self.draining.is_some()
    }

    /// Drops the open query and any drain, whose `Conn` is abandoned (the connection is being
    /// discarded).
    pub fn discard(&mut self) {
        self.open = None;
        self.draining = None;
    }
}

/// Gives a connection up without reading what is pending on it (patch §7): the socket closes,
/// nothing is read in the background, no `COM_QUIT` is sent.
pub(crate) fn abandon(conn: &mut Option<Conn>) {
    if let Some(c) = conn.take() {
        c.abandon();
    }
}

/// The typed caps for a server's `net_write_timeout`: suspend only when there is an idle bound
/// below it (`net_write_timeout` − 15 s, never raised: the driver never sets the variable).
pub(crate) fn caps_for(net_write_timeout: Option<Duration>) -> TypedCaps {
    let bound = net_write_timeout
        .and_then(|t| t.checked_sub(IDLE_MARGIN))
        .filter(|b| !b.is_zero());
    TypedCaps {
        suspend: bound.is_some(),
        binds: false,
        close_needs_kill: true,
        idle_bound: bound,
    }
}

/// `SELECT @@net_write_timeout` on a new connection (ER6, R19). A server that refuses it (an
/// unknown variable) or a value that is not a positive number gives `None` (nothing suspends);
/// an I/O or protocol failure fails the connect: a connection that cannot answer one query is
/// not handed out as usable.
pub(crate) async fn read_net_write_timeout(conn: &mut Conn) -> Result<Option<Duration>, RawError> {
    let row = match conn
        .query_first::<Row, _>("SELECT @@net_write_timeout")
        .await
    {
        Ok(row) => row,
        Err(mysql_async::Error::Server(_)) => return Ok(None),
        Err(e) => return Err(raw_error(&e)),
    };
    Ok(row
        .and_then(|r| crate::probe::int(r.as_ref(0)))
        .and_then(|secs| u64::try_from(secs).ok())
        .filter(|s| *s > 0)
        .map(Duration::from_secs))
}

/// What the typed path uses of a connection.
pub(crate) struct Ctx<'a> {
    pub conn: &'a mut Option<Conn>,
    pub tx: &'a mut TxStatus,
    pub read_only: bool,
    pub state: &'a mut TypedState,
}

/// The spec's limits, normalized (a zero row count means one chunk's worth).
#[derive(Debug, Clone, Copy)]
struct Limits {
    first_rows: u32,
    chunk_rows: u32,
    chunk_bytes: u64,
    cell_cap: u32,
    max_sets: u32,
}

impl Limits {
    fn of(spec: &QuerySpec<'_>) -> Limits {
        let chunk_rows = spec.chunk_rows.max(1);
        Limits {
            first_rows: match spec.first_rows {
                0 => chunk_rows,
                n => n,
            },
            chunk_rows,
            chunk_bytes: spec.chunk_bytes,
            cell_cap: spec.cell_cap,
            max_sets: spec.max_result_sets.max(1),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// `ResultStart` is next.
    Start,
    /// Rows of the current result.
    Rows,
    /// At a read-ahead point: `fetch_more` or `close_query`.
    Suspended,
    /// The stashed error is next (a later statement's, after the kept results ended).
    Failed,
    /// `Done` is next.
    Done,
}

/// The open typed query of a connection: what it has read and what it will do next. The rows
/// themselves wait in `Conn`'s pending result.
pub(crate) struct Open {
    wire: Wire,
    plan: Plan,
    limits: Limits,
    stage: Stage,
    builder: Option<ChunkBuilder>,
    /// The row read ahead at a suspension point, not yet in a chunk.
    peeked: Option<Row>,
    /// The current result set's terminator was read.
    ended: bool,
    /// Rows of the current result pushed into chunks or the builder.
    rows: u64,
    /// Rows of the current result handed out in chunks.
    delivered: u64,
    chunks: u64,
    /// Suspend when `rows` reaches this (`None`: never).
    target: Option<u64>,
    /// Results started (`ResultStart` sent or due).
    kept: u32,
    skipped: u32,
    command_tag: Option<String>,
    /// A later statement's error, after the current result's end.
    error: Option<RawError>,
}

impl Open {
    /// `suspendable`: the unit may suspend (binary protocol, not a `CALL`, a suspendable spec,
    /// caps that allow it): it stops when `rows` reaches `target`.
    fn new(wire: Wire, plan: Plan, limits: Limits, suspendable: bool, tag: Option<String>) -> Self {
        Open {
            wire,
            plan,
            limits,
            stage: Stage::Start,
            builder: None,
            peeked: None,
            ended: false,
            rows: 0,
            delivered: 0,
            chunks: 0,
            target: suspendable.then_some(u64::from(limits.first_rows)),
            kept: 1,
            skipped: 0,
            command_tag: tag,
            error: None,
        }
    }

    /// The next kept result (never suspended: only a single-result unit suspends).
    fn start_next(&mut self, plan: Plan) {
        self.plan = plan;
        self.stage = Stage::Start;
        self.builder = None;
        self.ended = false;
        self.rows = 0;
        self.delivered = 0;
        self.chunks = 0;
        self.target = None;
        self.kept += 1;
    }

    /// The chunk's row limit: `first_rows` for the first chunk, `chunk_rows` after.
    fn row_limit(&self) -> u32 {
        if self.chunks == 0 {
            self.limits.first_rows
        } else {
            self.limits.chunk_rows
        }
    }

    fn at_target(&self) -> bool {
        self.target.is_some_and(|t| self.rows >= t)
    }

    fn sealable(&self) -> bool {
        self.builder.as_ref().is_some_and(|b| {
            b.rows() > 0
                && (b.rows() >= self.row_limit()
                    || b.bytes() >= self.limits.chunk_bytes
                    || b.is_full()
                    || self.at_target())
        })
    }

    fn take_chunk(&mut self) -> Option<Chunk> {
        let chunk = self.builder.take()?.seal();
        self.delivered += u64::from(chunk.rows());
        self.chunks += 1;
        Some(chunk)
    }

    /// Decodes one row into the builder. A row whose value count is not the result's column
    /// count is a protocol error, found before anything of it is pushed.
    fn push(&mut self, row: Row) -> Result<(), Fail> {
        let values = row.unwrap_raw();
        if values.len() != self.plan.width() {
            return Err(Fail::Protocol);
        }
        let limit = self.row_limit();
        let Open {
            builder,
            plan,
            limits,
            wire,
            ..
        } = self;
        let b = builder.get_or_insert_with(|| plan.builder(limit));
        plan.push_row(b, &values, *wire, limits.cell_cap);
        self.rows += 1;
        Ok(())
    }
}

/// Why a typed read failed.
enum Fail {
    /// `mysql_async`'s error (a server error keeps the connection in step).
    My(mysql_async::Error),
    /// The server sent something out of shape the driver itself found.
    Protocol,
}

impl From<mysql_async::Error> for Fail {
    fn from(e: mysql_async::Error) -> Self {
        Fail::My(e)
    }
}

pub(crate) fn unsupported() -> RawError {
    RawError {
        kind: RawErrorKind::Unsupported,
        server: None,
    }
}

pub(crate) fn not_connected() -> RawError {
    RawError {
        kind: RawErrorKind::Io(io::ErrorKind::NotConnected),
        server: None,
    }
}

fn protocol() -> RawError {
    RawError {
        kind: RawErrorKind::Protocol,
        server: None,
    }
}

/// The raw error of a failure that breaks the connection: `Protocol` for anything malformed
/// (a driver error, an `InvalidData`/`InvalidInput` parse error, an unexpected end inside a
/// packet the stream delivered whole, the packet codec's own errors: a packet past
/// `max_allowed_packet`, packets out of sequence), else `raw_error`'s I/O kind.
fn breaking_error(conn: Option<&Conn>, e: &Fail) -> RawError {
    let Fail::My(e) = e else {
        return protocol();
    };
    let malformed = match e {
        mysql_async::Error::Driver(_) => true,
        mysql_async::Error::Io(mysql_async::IoError::Io(io)) => match io.kind() {
            io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput => true,
            // A lost socket marks the connection disconnected; a short parse does not.
            io::ErrorKind::UnexpectedEof => conn.is_some_and(|c| !c.is_disconnected()),
            // `mysql_common`'s codec errors come wrapped as `Other`; a TLS failure inside an
            // I/O error stays one (`raw_error` reads it).
            io::ErrorKind::Other => io
                .get_ref()
                .is_some_and(|inner| inner.downcast_ref::<rustls::Error>().is_none()),
            _ => false,
        },
        _ => false,
    };
    if malformed { protocol() } else { raw_error(e) }
}

/// The transaction status in the connection's last OK packet, if any.
fn last_status(conn: &Conn) -> Option<TxStatus> {
    conn.last_ok_packet()
        .map(|ok| tx_from_status(ok.status_flags()))
}

/// After a server error: one `COM_PING` reads the status flags again (an ERR packet has none);
/// `false` when the ping failed too (the connection is then broken).
async fn resync(conn: &mut Conn, tx: &mut TxStatus) -> bool {
    match conn.ping().await {
        Ok(()) => {
            if let Some(status) = last_status(conn) {
                *tx = status;
            }
            true
        }
        Err(mysql_async::Error::Server(_)) => true,
        Err(_) => false,
    }
}

/// After a server error the connection is still in step: the status is read again; a ping
/// that fails too abandons it (`false`).
async fn resync_or_abandon(conn: &mut Option<Conn>, tx: &mut TxStatus) -> bool {
    let in_step = match conn.as_mut() {
        Some(c) => resync(c, tx).await,
        None => false,
    };
    if !in_step {
        abandon(conn);
    }
    in_step
}

/// Ends the open query with `e`: a server error keeps the connection (status read again), any
/// other failure breaks it (the `Conn` is abandoned and never used again).
async fn fail(cx: Ctx<'_>, e: Fail) -> RawError {
    cx.state.open = None;
    if let Fail::My(mysql_async::Error::Server(s)) = &e {
        let raw = raw_error(&mysql_async::Error::Server(s.clone()));
        resync_or_abandon(cx.conn, cx.tx).await;
        return raw;
    }
    let raw = breaking_error(cx.conn.as_ref(), &e);
    abandon(cx.conn);
    raw
}

/// The next row of the pending result (`None` at its end, after `mysql_async` moved on to the
/// next result set's header). A new `QueryResult` of the query's own protocol each time.
async fn next_row<P: WireProtocol>(conn: &mut Conn) -> Result<Option<Row>, mysql_async::Error> {
    QueryResult::<'_, 'static, P>::new(&mut *conn).next().await
}

/// The pending result's columns (`None`: nothing pending, or a pending error).
fn pending_columns<P: WireProtocol>(conn: &mut Conn) -> Option<std::sync::Arc<[Column]>> {
    QueryResult::<'_, 'static, P>::new(&mut *conn).columns()
}

/// Whether nothing of the unit is left on the wire: no rows pending and no further result
/// (what is left, if anything, is an empty result or a pending error, both read without I/O).
fn nothing_unread(conn: &mut Conn, wire: Wire) -> bool {
    match wire {
        Wire::Binary => QueryResult::<'_, 'static, BinaryProtocol>::new(&mut *conn).is_empty(),
        Wire::Text => QueryResult::<'_, 'static, TextProtocol>::new(&mut *conn).is_empty(),
    }
}

/// Whether a result set of `columns` can be read in `wire` without `mysql_common` panicking.
fn readable(wire: Wire, columns: &[Column]) -> bool {
    wire == Wire::Text || columns.iter().all(|c| binary_decodable(c.column_type()))
}

/// Reads every remaining result set to its end, undecoded, counting those with columns. A
/// pending error (a later statement's) comes back as the error.
async fn drain_rest<P: WireProtocol>(
    conn: &mut Conn,
    wire: Wire,
    skipped: &mut u32,
) -> Result<(), Fail> {
    loop {
        let columns = pending_columns::<P>(conn);
        if columns.as_deref().is_some_and(|c| !readable(wire, c)) {
            return Err(Fail::Protocol);
        }
        // Also surfaces a pending error (whose `columns` are `None`).
        let first = next_row::<P>(conn).await?;
        let Some(columns) = columns else {
            return Ok(());
        };
        if first.is_some() {
            while next_row::<P>(conn).await?.is_some() {}
        }
        if !columns.is_empty() {
            *skipped = skipped.saturating_add(1);
        }
    }
}

/// What reading the current result gave.
enum Fill {
    Chunk(Chunk),
    /// At a read-ahead point, with the next row peeked.
    Suspend,
    /// The current result ended, with no rows left to hand out.
    End,
}

/// Reads rows into the builder until a chunk is due, a read-ahead point or the result's end.
async fn fill<P: WireProtocol>(conn: &mut Conn, q: &mut Open) -> Result<Fill, Fail> {
    loop {
        if q.sealable()
            && let Some(chunk) = q.take_chunk()
        {
            return Ok(Fill::Chunk(chunk));
        }
        if q.ended {
            return Ok(match q.take_chunk() {
                Some(chunk) => Fill::Chunk(chunk),
                None => Fill::End,
            });
        }
        if q.at_target() {
            // The batch is out (a builder with rows was sealed above): look one row ahead.
            return Ok(match next_row::<P>(conn).await? {
                Some(row) => {
                    q.peeked = Some(row);
                    Fill::Suspend
                }
                None => {
                    q.ended = true;
                    Fill::End
                }
            });
        }
        let row = match q.peeked.take() {
            Some(row) => Some(row),
            None => next_row::<P>(conn).await?,
        };
        match row {
            Some(row) => q.push(row)?,
            None => q.ended = true,
        }
    }
}

/// The affected rows to report for a result: the last OK packet's count, except `None` for a
/// result with columns and no affected row (a read; a positive count, a `CALL`'s writes, stays
/// for the read-only post-check).
fn affected(conn: &Conn, plan: &Plan) -> Option<u64> {
    conn.last_ok_packet()
        .map(|ok| ok.affected_rows())
        .filter(|n| *n > 0 || plan.width() == 0)
}

/// After the current result ended: the next kept result, or the drain of every other one; the
/// current result's end either way (sent before a later statement's error).
async fn finish<P: WireProtocol>(conn: &mut Conn, q: &mut Open) -> Result<ResultEnd, Fail> {
    let end = |q: &Open, conn: &Conn| ResultEnd {
        rows: q.rows,
        kept: q.delivered,
        rows_affected: affected(conn, &q.plan),
        command_tag: q.command_tag.clone(),
    };
    if q.kept < q.limits.max_sets
        && let Some(columns) = pending_columns::<P>(conn).filter(|c| !c.is_empty())
    {
        if !readable(q.wire, &columns) {
            return Err(Fail::Protocol);
        }
        let done = end(q, conn);
        q.start_next(Plan::for_columns(&columns));
        return Ok(done);
    }
    let drained = drain_rest::<P>(conn, q.wire, &mut q.skipped).await;
    let done = end(q, conn);
    match drained {
        Ok(()) => q.stage = Stage::Done,
        Err(Fail::My(mysql_async::Error::Server(s))) => {
            q.error = Some(raw_error(&mysql_async::Error::Server(s)));
            q.stage = Stage::Failed;
        }
        Err(e) => return Err(e),
    }
    Ok(done)
}

/// The query's first result: its protocol and columns, after the statement was sent.
async fn start(
    conn: &mut Conn,
    sql: &str,
    route: ExtendedRoute,
) -> Result<(Wire, std::sync::Arc<[Column]>), StartFail> {
    match route {
        ExtendedRoute::Refused => Err(StartFail::Refused),
        ExtendedRoute::Text => {
            let result = conn.query_iter(sql).await.map_err(StartFail::My)?;
            Ok((Wire::Text, result.columns().unwrap_or_else(|| [].into())))
        }
        ExtendedRoute::Prepared => {
            let statement = conn.prep(sql).await.map_err(StartFail::Prepare)?;
            if statement.num_params() > 0 {
                return Err(StartFail::Binds);
            }
            let result = conn
                .exec_iter(&statement, ())
                .await
                .map_err(StartFail::My)?;
            Ok((Wire::Binary, result.columns().unwrap_or_else(|| [].into())))
        }
    }
}

enum StartFail {
    /// Nothing was sent (`extended_route` refused the text).
    Refused,
    /// The statement has placeholders (binds are M3b); prepared, not executed.
    Binds,
    /// The prepare failed: nothing ran (the only failure a 1295 retry follows).
    Prepare(mysql_async::Error),
    /// The statement was sent and failed.
    My(mysql_async::Error),
}

fn is_unsupported_ps(e: &mysql_async::Error) -> bool {
    matches!(e, mysql_async::Error::Server(s) if s.code == UNSUPPORTED_PS)
}

/// `RawConn::query`.
pub(crate) async fn query(cx: Ctx<'_>, spec: QuerySpec<'_>) -> Result<(), RawError> {
    if cx.state.busy() || spec.protocol == Protocol::Simple || !spec.binds.is_empty() {
        return Err(unsupported());
    }
    let Some(conn) = cx.conn.as_mut() else {
        return Err(not_connected());
    };
    let limits = Limits::of(&spec);
    let tag = command_tag(spec.sql);
    let route = if cx.state.force_text {
        ExtendedRoute::Text
    } else {
        extended_route(spec.sql, cx.read_only)
    };
    let started = match start(conn, spec.sql, route).await {
        // Only a refused prepare is retried: nothing ran.
        Err(StartFail::Prepare(e)) if is_unsupported_ps(&e) => {
            if cx.read_only {
                // The refused prepare changed nothing; it is never re-sent as text.
                return Err(unsupported());
            }
            start(conn, spec.sql, ExtendedRoute::Text).await
        }
        other => other,
    };
    let (wire, columns) = match started {
        Ok(started) => started,
        Err(StartFail::Refused | StartFail::Binds) => return Err(unsupported()),
        Err(StartFail::My(e) | StartFail::Prepare(e)) => {
            return Err(fail(cx, Fail::My(e)).await);
        }
    };
    if !readable(wire, &columns) {
        // Abandoned before any row is read: nothing parses the refused column type.
        return Err(fail(cx, Fail::Protocol).await);
    }
    let suspendable = spec.suspendable
        && cx.state.caps.suspend
        && wire == Wire::Binary
        && tag.as_deref() != Some("CALL")
        && !columns.is_empty();
    cx.state.open = Some(Open::new(
        wire,
        Plan::for_columns(&columns),
        limits,
        suspendable,
        tag,
    ));
    Ok(())
}

/// `RawConn::next_event`.
pub(crate) async fn next_event(cx: Ctx<'_>) -> Result<QueryEvent, RawError> {
    if cx.state.draining.is_some() {
        return Err(unsupported());
    }
    let Some(q) = cx.state.open.as_mut() else {
        return Err(unsupported());
    };
    let Some(conn) = cx.conn.as_mut() else {
        cx.state.open = None;
        return Err(not_connected());
    };
    match q.stage {
        Stage::Start => {
            q.stage = Stage::Rows;
            return Ok(QueryEvent::ResultStart(q.plan.schema.clone()));
        }
        Stage::Suspended => return Err(unsupported()),
        Stage::Failed => {
            let e = q.error.take().unwrap_or_else(protocol);
            cx.state.open = None;
            resync_or_abandon(cx.conn, cx.tx).await;
            return Err(e);
        }
        Stage::Done => {
            if let Some(status) = last_status(conn) {
                *cx.tx = status;
            }
            let done = QueryDone {
                tx: *cx.tx,
                warnings: conn.get_warnings(),
                skipped_result_sets: q.skipped,
            };
            cx.state.open = None;
            return Ok(QueryEvent::Done(done));
        }
        Stage::Rows => {}
    }
    let filled = match q.wire {
        Wire::Binary => fill::<BinaryProtocol>(conn, q).await,
        Wire::Text => fill::<TextProtocol>(conn, q).await,
    };
    let filled = match filled {
        Ok(f) => f,
        Err(e) => return Err(fail(cx, e).await),
    };
    match filled {
        Fill::Chunk(chunk) => Ok(QueryEvent::Rows(chunk)),
        Fill::Suspend => {
            q.stage = Stage::Suspended;
            Ok(QueryEvent::Suspended)
        }
        Fill::End => {
            let finished = match q.wire {
                Wire::Binary => finish::<BinaryProtocol>(conn, q).await,
                Wire::Text => finish::<TextProtocol>(conn, q).await,
            };
            match finished {
                Ok(end) => Ok(QueryEvent::ResultEnd(end)),
                Err(e) => Err(fail(cx, e).await),
            }
        }
    }
}

/// `RawConn::fetch_more`.
pub(crate) async fn fetch_more(cx: Ctx<'_>, rows: u32) -> Result<(), RawError> {
    if cx.state.draining.is_some() {
        return Err(unsupported());
    }
    let Some(q) = cx.state.open.as_mut() else {
        return Err(unsupported());
    };
    if q.stage != Stage::Suspended {
        return Err(unsupported());
    }
    let n = if rows == 0 { q.limits.chunk_rows } else { rows };
    q.target = Some(q.rows.saturating_add(u64::from(n)));
    q.stage = Stage::Rows;
    Ok(())
}

/// What a drain read: the current result's remaining rows and the result sets after it.
struct Drained {
    rows: u64,
}

type DrainFuture = Pin<Box<dyn Future<Output = (Conn, Result<Drained, Fail>)> + Send>>;

/// A drain in progress: the future owns the `Conn` until it completes.
struct Drain {
    fut: DrainFuture,
    /// Rows of the current result handed out before the drain began.
    delivered: u64,
    /// Rows of the current result read before the drain began (chunks, builder, peeked).
    read: u64,
}

/// The `Conn` a drain future owns. If the future is dropped before it completes (an `AfterKill`
/// that ran out, `close`, a dropped connection), the `Conn` is abandoned (patch §7), so nothing
/// reads the rest of the result in the background.
struct Held(Option<Conn>);

impl Drop for Held {
    fn drop(&mut self) {
        if let Some(conn) = self.0.take() {
            conn.abandon();
        }
    }
}

/// Reads the current result's rest (unless it already ended) and every result set after it.
async fn drain_all<P: WireProtocol>(
    conn: &mut Conn,
    wire: Wire,
    current_ended: bool,
) -> Result<Drained, Fail> {
    let mut rows = 0_u64;
    if !current_ended {
        while next_row::<P>(conn).await?.is_some() {
            rows += 1;
        }
    }
    let mut skipped = 0;
    drain_rest::<P>(conn, wire, &mut skipped).await?;
    Ok(Drained { rows })
}

/// The drain of `q`'s rest, owning `conn`.
fn drain_future(conn: Conn, q: &Open) -> DrainFuture {
    let wire = q.wire;
    let ended = q.ended;
    Box::pin(async move {
        let mut held = Held(Some(conn));
        let drained = match held.0.as_mut() {
            Some(conn) => match wire {
                Wire::Binary => drain_all::<BinaryProtocol>(conn, wire, ended).await,
                Wire::Text => drain_all::<TextProtocol>(conn, wire, ended).await,
            },
            None => Err(Fail::Protocol),
        };
        let conn = held
            .0
            .take()
            .expect("the drain holds its connection to the end");
        (conn, drained)
    })
}

/// How a closed query's rest ended, as `close_query` answers it (Task 12's rule): read to its
/// end is `Completed { rows }`; the kill's 1317 is `Closed { rows: delivered }`; any other server
/// error is that error, after the status is read again; anything else breaks the connection.
async fn closed_by(
    cx: Ctx<'_>,
    ended: Result<u64, Fail>,
    delivered: u64,
) -> Result<Closed, RawError> {
    match ended {
        Ok(rows) => {
            if let Some(status) = cx.conn.as_ref().and_then(last_status) {
                *cx.tx = status;
            }
            Ok(Closed::Completed { rows })
        }
        Err(Fail::My(mysql_async::Error::Server(s))) => {
            let killed = s.code == QUERY_INTERRUPTED;
            let raw = raw_error(&mysql_async::Error::Server(s));
            if !resync_or_abandon(cx.conn, cx.tx).await {
                return Err(not_connected());
            }
            if killed {
                Ok(Closed::Closed { rows: delivered })
            } else {
                Err(raw)
            }
        }
        Err(e) => {
            let raw = breaking_error(cx.conn.as_ref(), &e);
            abandon(cx.conn);
            Err(raw)
        }
    }
}

/// `RawConn::close_query`.
pub(crate) async fn close_query(cx: Ctx<'_>, how: CloseHow) -> Result<Closed, RawError> {
    let kill = Closed::NeedsServerKill {
        thread_id: cx.state.thread_id,
    };
    if cx.state.draining.is_some() && how == CloseHow::Now {
        // A drain that ran out is still reading: only the kill ends it now.
        return Ok(kill);
    }
    if cx.state.draining.is_none() {
        let Some(q) = cx.state.open.as_mut() else {
            return Ok(Closed::Closed { rows: 0 });
        };
        match q.stage {
            // Nothing is left unread: the rest was drained when the last kept result ended.
            Stage::Done => {
                let rows = q.rows;
                cx.state.open = None;
                return closed_by(cx, Ok(rows), 0).await;
            }
            // A later statement's error was the last thing read.
            Stage::Failed => {
                let delivered = q.delivered;
                let e = q.error.take().unwrap_or_else(protocol);
                cx.state.open = None;
                if !resync_or_abandon(cx.conn, cx.tx).await {
                    return Err(not_connected());
                }
                let killed = e
                    .server
                    .as_ref()
                    .and_then(|s| s.vendor_code)
                    .is_some_and(|c| c == i64::from(QUERY_INTERRUPTED));
                return if killed {
                    Ok(Closed::Closed { rows: delivered })
                } else {
                    Err(e)
                };
            }
            Stage::Start | Stage::Rows | Stage::Suspended => {}
        }
        let Some(conn) = cx.conn.as_mut() else {
            cx.state.open = None;
            return Err(not_connected());
        };
        // `Now`: MySQL ends a result early only by a kill, unless nothing of it is left on the
        // wire; then it finishes here (no kill that could reach the next statement).
        if how == CloseHow::Now && (q.peeked.is_some() || !nothing_unread(conn, q.wire)) {
            return Ok(kill);
        }
        let Some(conn) = cx.conn.take() else {
            return Err(not_connected());
        };
        let Some(q) = cx.state.open.take() else {
            *cx.conn = Some(conn);
            return Ok(Closed::Closed { rows: 0 });
        };
        cx.state.draining = Some(Drain {
            fut: drain_future(conn, &q),
            delivered: q.delivered,
            read: q.rows + u64::from(q.peeked.is_some()),
        });
    }
    let Some(drain) = cx.state.draining.as_mut() else {
        return Ok(Closed::Closed { rows: 0 });
    };
    let limit = match how {
        CloseHow::Drain(d) | CloseHow::AfterKill(d) => Some(d),
        // Only a drain just started for a query with nothing left on the wire: it reads no
        // packet (an empty result or a pending error), so it needs no bound.
        CloseHow::Now => None,
    };
    let finished = match limit {
        // The timeout borrows the boxed future: when it runs out, the future stays as it is,
        // in the middle of its read, for the next call.
        Some(d) => tokio::time::timeout(d, &mut drain.fut).await.ok(),
        None => Some((&mut drain.fut).await),
    };
    let Some((conn, drained)) = finished else {
        return match how {
            CloseHow::AfterKill(_) => {
                // The kill did not end the result in time: the drain (and its `Conn`) is
                // dropped, the `Conn` abandoned.
                cx.state.draining = None;
                Err(RawError {
                    kind: RawErrorKind::Io(io::ErrorKind::TimedOut),
                    server: None,
                })
            }
            // The future stays in the middle of its read, for `AfterKill`.
            _ => Ok(kill),
        };
    };
    let Drain {
        delivered, read, ..
    } = cx.state.draining.take().expect("the drain just completed");
    *cx.conn = Some(conn);
    closed_by(cx, drained.map(|d| read + d.rows), delivered).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_idle_bound_is_15_s_below_net_write_timeout_and_never_raised() {
        let caps = caps_for(Some(Duration::from_secs(60)));
        assert_eq!(
            caps,
            TypedCaps {
                suspend: true,
                binds: false,
                close_needs_kill: true,
                idle_bound: Some(Duration::from_secs(45)),
            }
        );
        assert_eq!(
            caps_for(Some(Duration::from_secs(20))).idle_bound,
            Some(Duration::from_secs(5))
        );
        // No bound below the server's: nothing suspends.
        for nwt in [
            None,
            Some(Duration::from_secs(15)),
            Some(Duration::from_secs(3)),
        ] {
            let caps = caps_for(nwt);
            assert!(!caps.suspend && caps.idle_bound.is_none(), "{nwt:?}");
            assert!(caps.close_needs_kill);
        }
    }

    #[test]
    fn limits_never_ask_for_zero_rows() {
        let spec = QuerySpec {
            sql: "",
            binds: &[],
            protocol: Protocol::Extended,
            suspendable: true,
            first_rows: 0,
            chunk_rows: 0,
            chunk_bytes: 0,
            cell_cap: 0,
            max_result_sets: 0,
        };
        let l = Limits::of(&spec);
        assert_eq!((l.first_rows, l.chunk_rows, l.max_sets), (1, 1, 1));
    }
}
