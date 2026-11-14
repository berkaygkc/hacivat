//! Typed streaming on PostgreSQL (HV-RES-04 PG; Task 12): the typed `RawConn` methods' engine
//! side over the vendored `tokio-postgres` patch §1, chunked into `hacivat_values::Chunk`s
//! through [`crate::decode`]. The grammar and the session's rules are `hacivat_driver::typed`'s.
//!
//! **One query.** `query` prepares the statement (column types through `tokio-postgres`'s type
//! cache; nothing here ever runs `DEALLOCATE ALL`), then requests each column in the format its
//! decoder wants (binary for the R-M3-31 scalar set and enums, text for the rest, P8):
//! - **not suspendable**: `query_with_formats`, the unnamed portal with Bind, `Flush`,
//!   `Execute(0)` and `Sync` in one batch (P5). The `Flush` has the server send `BindComplete`
//!   at once (it holds it until a `Flush` or `Sync` otherwise), so `query` returns when the
//!   statement is bound, not after its first buffer of rows: a slow statement's cancel finds it
//!   running. The rows stream with TCP backpressure (the response channel holds one batch of
//!   messages, so the connection stops reading the socket while nobody polls);
//! - **suspendable** (a console cursor, R-M3-23): only inside a transaction block, else
//!   `Unsupported` before anything is sent (P5; the session or the guard opens it first). A
//!   named portal from `bind_with_formats`, then `Execute(first_rows)`, and `Execute(n)` on each
//!   `fetch_more(n)`. Every Bind, Execute and Close of the portal carries its own `Sync`, which
//!   does not end the transaction block, so the portal survives them and any statement run
//!   between them (the read-only guard's canary, R-M3-17). An Execute that returned exactly its
//!   row limit without a `CommandComplete` ended in `PortalSuspended`: the query is `Suspended`
//!   (PostgreSQL suspends at the limit even when no row is left; the next Execute then completes
//!   with none).
//!
//! **Chunks.** The first chunk holds at most `first_rows` rows, later ones `chunk_rows`, each at
//! most about `chunk_bytes` of decoded data (sealed at the row that reaches it). A statement
//! without rows (DML, DDL, `SET`) is one result with an empty schema. `ResultEnd.rows_affected`
//! is the count of the command tag (`INSERT 0 3` → 3; `CREATE TABLE` → none), except for a
//! read: a result with columns under a `SELECT n`/`FETCH n` tag affected nothing, so it is
//! `None` (its count is `ResultEnd.rows`; fix round 1). DML with `RETURNING` keeps its count, and
//! so does a column-less `SELECT n` (`CREATE TABLE AS`). A resumed portal's tag counts only its
//! last Execute (PostgreSQL's rule), so the driver rewrites it with the result's own count.
//!
//! **Closing early.** A named portal: `close_query(Now)` closes it (`Close` + `Sync`), after
//! reading the rest of an Execute already in flight; `Drain(d)` also runs one `Execute(4096)`
//! within `d`: a result that completes meanwhile is `Completed` (with its total rows), else
//! `Closed`; a server error while closing (a cancel; a portal its transaction already
//! destroyed) still closes, and the status shows it (Hacivat's own `BEGIN` or savepoint is
//! rolled back anyway, R-M3-23). The unnamed portal's `Execute(0)` cannot be closed on the wire:
//! its rest is always read to the end, undecoded, whatever `how` says, and the answer is the
//! server's outcome (fix round 1): `Completed { rows: total }` (in autocommit its `Sync`
//! committed it), or the server's error (a cancel's 57014, a runtime error: it did not apply).
//! The session's protocol cancel, run alongside, is what ends it sooner.
//!
//! **Errors.** A server error waits for its `ReadyForQuery`, as `execute` does (at most 1 s;
//! after that the connection is broken), so the status is exact when the call returns. Anything
//! else on a typed stream breaks the connection: never used again. A `DataRow` whose field count
//! is not the statement's, a field longer than its message, an unknown message in the stream
//! answer `Protocol`; corruption below the stream (a frame the wire observer refuses, a
//! malformed notice the connection task cannot parse) ends the connection first, so it answers
//! `Io(ConnectionReset)`: fail-closed either way. `COPY` is refused before sending; a `COPY …
//! FROM STDIN` that slips past the check is answered by the wire observer (`CopyFail`) and is
//! `Unsupported` too.
//!
//! **Statements meanwhile.** Only while the query is `Suspended` may `execute`, `batch`,
//! `ping`, `server_info`, `current_user` and `list_databases` run (`allows_statements`); while
//! it streams they answer `Unsupported`, because their answer would wait behind rows nobody
//! reads. `reset` needs no open query at all.

use std::io;
use std::pin::Pin;

use futures::TryStreamExt;
use hacivat_driver::{
    CloseHow, Closed, Protocol, QueryDone, QueryEvent, QuerySpec, RawError, RawErrorKind,
    ResultEnd, TxStatus, TypedCaps,
};
use hacivat_values::{Chunk, ChunkBuilder};
use postgres_types::{FromSql, Type};
use tokio::time::Instant;
use tokio_postgres::{Client, Portal, Row, RowStream, Statement};

use crate::connect::pg_error;
use crate::decode::{self, Plan};
use crate::exec::{Notices, drain_notices};
use crate::wire::{SETTLE, Settled, WireState};

/// What the PostgreSQL typed path can do: suspend (named portals), no binds yet (M3b), closes
/// without a server-side kill, no idle bound of its own (the session's, R-M3-20).
pub(crate) const CAPS: TypedCaps = TypedCaps {
    suspend: true,
    binds: false,
    close_needs_kill: false,
    idle_bound: None,
};

/// The rows of `close_query(Drain(..))`'s one Execute.
pub const DRAIN_ROWS: i32 = 4_096;

/// What the typed path uses of a connection.
pub(crate) struct Ctx<'a> {
    pub client: &'a Client,
    pub wire: &'a WireState,
    pub notices: &'a Notices,
}

/// A cell's bytes as the server sent them (`None`: NULL), whatever its type.
struct RawCell<'a>(Option<&'a [u8]>);

impl<'a> FromSql<'a> for RawCell<'a> {
    fn from_sql(_: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(RawCell(Some(raw)))
    }

    fn from_sql_null(_: &Type) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(RawCell(None))
    }

    fn accepts(_: &Type) -> bool {
        true
    }
}

/// The spec's limits, normalized (a zero row count means one chunk's worth).
#[derive(Debug, Clone, Copy)]
struct Limits {
    first_rows: u32,
    chunk_rows: u32,
    chunk_bytes: u64,
    cell_cap: u32,
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
        }
    }
}

/// An Execute's row limit on the wire (`i32`; never 0, which would mean "all rows").
fn max_rows(rows: u32) -> i32 {
    i32::try_from(rows.max(1)).unwrap_or(i32::MAX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// `ResultStart` is next.
    Start,
    /// Rows, until the current Execute's stream ends.
    Rows,
    /// The portal stopped at its row limit: `fetch_more` or `close_query`.
    Suspended,
    /// `ResultEnd` went out: `Done` is next.
    Ended,
}

/// The open typed query of a connection. Fields drop in order: the stream and the read-ahead
/// row, then the portal (`Close` + `Sync`), then the statement (`Close` + `Sync`, its last
/// clone).
pub(crate) struct Open {
    stream: Option<Pin<Box<RowStream>>>,
    /// The first row `fetch_more` read to see the server's answer, not yet in a chunk.
    peeked: Option<Row>,
    builder: Option<ChunkBuilder>,
    portal: Option<Portal>,
    /// Held for its drop: the statement's `Close` goes out when the query ends, after the
    /// portal's.
    _statement: Statement,
    plan: Plan,
    limits: Limits,
    stage: Stage,
    /// The current stream ended (`Some(rows_affected)` of its last message).
    ended: Option<Option<u64>>,
    /// Rows received in this result (in chunks, in the builder or peeked).
    rows: u64,
    /// Rows handed out in chunks.
    delivered: u64,
    chunks: u64,
    /// Rows received in the current Execute, and its limit (0: the unnamed portal, all rows).
    batch_rows: u64,
    batch_max: u64,
    /// The portal suspended at least once.
    resumed: bool,
}

/// What reading the current stream gave.
enum Fill {
    Chunk(Chunk),
    /// The stream ended (`rows_affected` of its last message) with no rows left to hand out.
    End(Option<u64>),
}

impl Open {
    /// The chunk's row limit: `first_rows` for the first chunk, `chunk_rows` after.
    fn row_limit(&self) -> u32 {
        if self.chunks == 0 {
            self.limits.first_rows
        } else {
            self.limits.chunk_rows
        }
    }

    fn take_chunk(&mut self) -> Option<Chunk> {
        let b = self.builder.take()?;
        let chunk = b.seal();
        self.delivered += u64::from(chunk.rows());
        self.chunks += 1;
        Some(chunk)
    }

    fn sealable(&self) -> bool {
        self.builder.as_ref().is_some_and(|b| {
            b.rows() > 0
                && (b.rows() >= self.row_limit()
                    || b.bytes() >= self.limits.chunk_bytes
                    || b.is_full())
        })
    }

    fn push(&mut self, row: &Row) -> Result<(), tokio_postgres::Error> {
        let limit = self.row_limit();
        let Open {
            builder,
            plan,
            limits,
            ..
        } = self;
        let b = builder.get_or_insert_with(|| plan.builder(limit));
        // `Row` checked the field count against the statement's columns; a half-pushed row
        // only happens with an error, which drops the query and its builder.
        for (i, decoder) in plan.cols.iter().enumerate() {
            let raw = row.try_get::<_, RawCell<'_>>(i)?.0;
            decode::push(b, i as u32, decoder, raw, limits.cell_cap);
        }
        b.end_row();
        self.rows += 1;
        self.batch_rows += 1;
        Ok(())
    }

    /// Reads rows into the builder until a chunk is due or the current stream ends.
    async fn fill(&mut self) -> Result<Fill, tokio_postgres::Error> {
        loop {
            if self.sealable()
                && let Some(chunk) = self.take_chunk()
            {
                return Ok(Fill::Chunk(chunk));
            }
            if let Some(affected) = self.ended {
                // A builder exists only once a row is in it.
                return Ok(match self.take_chunk() {
                    Some(chunk) => Fill::Chunk(chunk),
                    None => Fill::End(affected),
                });
            }
            let row = match self.peeked.take() {
                Some(row) => Some(row),
                None => match self.stream.as_mut() {
                    Some(s) => s.as_mut().try_next().await?,
                    None => None,
                },
            };
            match row {
                Some(row) => self.push(&row)?,
                None => {
                    let affected = self.stream.take().and_then(|s| s.rows_affected());
                    self.ended = Some(affected);
                }
            }
        }
    }

    /// Whether the current Execute stopped at its row limit (`PortalSuspended`).
    fn stopped_at_limit(&self, affected: Option<u64>) -> bool {
        self.portal.is_some()
            && affected.is_none()
            && self.batch_max > 0
            && self.batch_rows >= self.batch_max
    }
}

fn unsupported() -> RawError {
    RawError {
        kind: RawErrorKind::Unsupported,
        server: None,
    }
}

fn not_connected() -> RawError {
    RawError {
        kind: RawErrorKind::Io(io::ErrorKind::NotConnected),
        server: None,
    }
}

/// Whether the open query lets a statement run now: none is open, or it is suspended.
pub(crate) fn allows_statements(open: &Option<Open>) -> bool {
    open.as_ref().is_none_or(|q| q.stage == Stage::Suspended)
}

/// Whether `sql` is a `COPY` (its first keyword, after whitespace and comments; block comments
/// nest, as PostgreSQL's do). ASCII casing only.
pub(crate) fn is_copy(sql: &str) -> bool {
    let b = sql.as_bytes();
    let mut i = 0;
    loop {
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c') {
            i += 1;
        }
        if b[i..].starts_with(b"--") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b[i..].starts_with(b"/*") {
            let mut depth = 0usize;
            while i < b.len() {
                if b[i..].starts_with(b"/*") {
                    depth += 1;
                    i += 2;
                } else if b[i..].starts_with(b"*/") {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            continue;
        }
        break;
    }
    let word = &b[i..];
    word.len() >= 4
        && word[..4].eq_ignore_ascii_case(b"copy")
        && word
            .get(4)
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || *c == b'_' || *c == b'$' || *c >= 0x80))
}

/// The error of a failed typed call, the connection left exact (see the module docs).
async fn failed(wire: &WireState, e: &tokio_postgres::Error) -> RawError {
    let mut raw = pg_error(e);
    if e.as_db_error().is_none() {
        if wire.take_copy_refused() {
            // A COPY FROM STDIN past the check: the observer answered it with CopyFail.
            return match wire.settle(SETTLE).await {
                Settled::Synced => unsupported(),
                _ => {
                    wire.mark_broken();
                    not_connected()
                }
            };
        }
        // Not the server's own error: the stream cannot be trusted any more.
        wire.mark_broken();
        // `tokio_postgres::Error` has no public kind; its `Display` names `Kind::Parse` and
        // `Kind::UnexpectedMessage` with these fixed strings (`src/error/mod.rs` of the vendored
        // 0.7.18, without a cause suffix). A re-vendor must keep them, or this falls back to
        // `pg_error`'s kind, still broken (fail-closed).
        let what = e.to_string();
        if what == "error parsing response from server" || what == "unexpected message from server"
        {
            raw.kind = RawErrorKind::Protocol;
        }
        return raw;
    }
    match wire.settle(SETTLE).await {
        Settled::Synced => {
            if wire.take_copy_refused() {
                raw = unsupported();
            }
        }
        Settled::Closed => {
            wire.mark_broken();
            if !matches!(raw.kind, RawErrorKind::Db | RawErrorKind::Io(_)) {
                raw.kind = RawErrorKind::Io(io::ErrorKind::ConnectionAborted);
            }
        }
        Settled::Expired => {
            wire.mark_broken();
            if !matches!(raw.kind, RawErrorKind::Io(_)) {
                raw.kind = RawErrorKind::Io(io::ErrorKind::TimedOut);
            }
        }
    }
    raw
}

fn no_params() -> std::iter::Empty<&'static (dyn postgres_types::ToSql + Sync)> {
    std::iter::empty()
}

/// `RawConn::query`.
pub(crate) async fn query(
    cx: Ctx<'_>,
    open: &mut Option<Open>,
    spec: QuerySpec<'_>,
) -> Result<(), RawError> {
    if cx.wire.is_broken() {
        return Err(not_connected());
    }
    if open.is_some()
        || spec.protocol == Protocol::Simple
        || !spec.binds.is_empty()
        || is_copy(spec.sql)
    {
        return Err(unsupported());
    }
    // P5: a portal outside a transaction block would end at its first Sync.
    if spec.suspendable && cx.wire.tx() == TxStatus::Idle {
        return Err(unsupported());
    }
    drain_notices(cx.notices);
    cx.wire.clear_tag();
    let _ = cx.wire.take_copy_refused();
    let limits = Limits::of(&spec);
    let statement = match cx.client.prepare(spec.sql).await {
        Ok(s) => s,
        Err(e) => return Err(failed(cx.wire, &e).await),
    };
    // A parameter would need a bind (M3b).
    if !statement.params().is_empty() {
        return Err(unsupported());
    }
    let plan = Plan::for_columns(statement.columns());
    // A statement without columns has no rows to suspend in: the unnamed portal.
    let portal_path = spec.suspendable && !plan.cols.is_empty();
    let started = async {
        if portal_path {
            let portal = cx
                .client
                .bind_with_formats(&statement, no_params(), &plan.formats)
                .await?;
            let first = max_rows(limits.first_rows);
            let stream = cx.client.query_portal_raw(&portal, first).await?;
            Ok((Some(portal), stream, first as u64))
        } else {
            let stream = cx
                .client
                .query_with_formats(&statement, no_params(), &plan.formats)
                .await?;
            Ok((None, stream, 0))
        }
    };
    match started.await {
        Ok((portal, stream, batch_max)) => {
            *open = Some(Open {
                stream: Some(Box::pin(stream)),
                peeked: None,
                builder: None,
                portal,
                _statement: statement,
                plan,
                limits,
                stage: Stage::Start,
                ended: None,
                rows: 0,
                delivered: 0,
                chunks: 0,
                batch_rows: 0,
                batch_max,
                resumed: false,
            });
            Ok(())
        }
        Err(e) => {
            drop(statement);
            Err(failed(cx.wire, &e).await)
        }
    }
}

/// The count of a command tag (`INSERT 0 3` → 3, `SELECT 10` → 10, `CREATE TABLE` → none).
fn tag_count(tag: &str) -> Option<u64> {
    tag.rsplit(' ').next()?.parse().ok()
}

/// Whether a tag is a read's (`SELECT n`, `FETCH n`): its count is rows returned, not affected.
fn is_read_tag(tag: &str) -> bool {
    matches!(tag.split(' ').next(), Some("SELECT" | "FETCH"))
}

/// A resumed portal's tag with the result's own count in place of the last Execute's.
fn retag(tag: &str, rows: u64) -> String {
    match tag.rsplit_once(' ') {
        Some((head, count)) if count.parse::<u64>().is_ok() => format!("{head} {rows}"),
        _ => tag.to_string(),
    }
}

/// `RawConn::next_event`.
pub(crate) async fn next_event(
    cx: Ctx<'_>,
    open: &mut Option<Open>,
) -> Result<QueryEvent, RawError> {
    if cx.wire.is_broken() {
        *open = None;
        return Err(not_connected());
    }
    let Some(q) = open.as_mut() else {
        return Err(unsupported());
    };
    match q.stage {
        Stage::Start => {
            q.stage = Stage::Rows;
            return Ok(QueryEvent::ResultStart(q.plan.schema.clone()));
        }
        Stage::Suspended => return Err(unsupported()),
        Stage::Ended => {
            let warnings = drain_notices(cx.notices)
                .iter()
                .filter(|code| code.starts_with("01"))
                .count();
            *open = None;
            return Ok(QueryEvent::Done(QueryDone {
                tx: cx.wire.tx(),
                warnings: u16::try_from(warnings).unwrap_or(u16::MAX),
                skipped_result_sets: 0,
            }));
        }
        Stage::Rows => {}
    }
    match q.fill().await {
        Ok(Fill::Chunk(chunk)) => Ok(QueryEvent::Rows(chunk)),
        Ok(Fill::End(affected)) => {
            q.ended = None;
            if q.stopped_at_limit(affected) {
                q.stage = Stage::Suspended;
                return Ok(QueryEvent::Suspended);
            }
            q.stage = Stage::Ended;
            let tag = cx.wire.last_tag();
            // Fix round 1 (review I3): a result with columns under a `SELECT n`/`FETCH n` tag
            // affected nothing (its count is `rows`); DML with `RETURNING` and a column-less
            // `SELECT n` (`CREATE TABLE AS`, `SELECT INTO`) keep the tag's count.
            let read = !q.plan.cols.is_empty() && tag.as_deref().is_some_and(is_read_tag);
            let rows_affected = if read {
                None
            } else if q.resumed {
                Some(q.rows)
            } else {
                tag.as_deref().map_or(affected, tag_count)
            };
            let command_tag = if q.resumed {
                tag.map(|t| retag(&t, q.rows))
            } else {
                tag
            };
            Ok(QueryEvent::ResultEnd(ResultEnd {
                rows: q.rows,
                kept: q.delivered,
                rows_affected,
                command_tag,
            }))
        }
        Err(e) => {
            *open = None;
            Err(failed(cx.wire, &e).await)
        }
    }
}

/// `RawConn::fetch_more`.
pub(crate) async fn fetch_more(
    cx: Ctx<'_>,
    open: &mut Option<Open>,
    rows: u32,
) -> Result<(), RawError> {
    if cx.wire.is_broken() {
        *open = None;
        return Err(not_connected());
    }
    let Some(q) = open.as_mut() else {
        return Err(unsupported());
    };
    if q.stage != Stage::Suspended {
        return Err(unsupported());
    }
    let Some(portal) = q.portal.as_ref() else {
        return Err(unsupported());
    };
    let max = max_rows(if rows == 0 { q.limits.chunk_rows } else { rows });
    cx.wire.clear_tag();
    let stream = match cx.client.query_portal_raw(portal, max).await {
        Ok(s) => s,
        Err(e) => {
            *open = None;
            return Err(failed(cx.wire, &e).await);
        }
    };
    q.stream = Some(Box::pin(stream));
    q.stage = Stage::Rows;
    q.batch_rows = 0;
    q.batch_max = max as u64;
    q.resumed = true;
    // The server's first answer, so a portal it no longer has (its transaction ended: 34000)
    // or a failed transaction (25P02) fails here.
    let first = match q.stream.as_mut() {
        Some(s) => s.as_mut().try_next().await,
        None => Ok(None),
    };
    match first {
        Ok(Some(row)) => q.peeked = Some(row),
        Ok(None) => {
            let affected = q.stream.take().and_then(|s| s.rows_affected());
            q.ended = Some(affected);
        }
        Err(e) => {
            *open = None;
            return Err(failed(cx.wire, &e).await);
        }
    }
    Ok(())
}

/// How reading an Execute's rest ended.
enum Rest {
    /// `CommandComplete` (or an empty query).
    Completed,
    /// The portal stopped at its limit again.
    Suspended,
    /// The drain's time ran out.
    Interrupted,
}

/// Reads the rest of `q`'s current stream without decoding, until `deadline` if any.
async fn read_rest(q: &mut Open, deadline: Option<Instant>) -> Result<Rest, tokio_postgres::Error> {
    if q.peeked.take().is_some() {
        q.rows += 1;
        q.batch_rows += 1;
    }
    let affected = match q.ended.take() {
        Some(affected) => affected,
        None => {
            let Some(mut stream) = q.stream.take() else {
                return Ok(Rest::Completed);
            };
            let rows = &mut q.rows;
            let batch = &mut q.batch_rows;
            let read = async {
                while stream.as_mut().try_next().await?.is_some() {
                    *rows += 1;
                    *batch += 1;
                }
                Ok::<_, tokio_postgres::Error>(stream.rows_affected())
            };
            match deadline {
                Some(at) => match tokio::time::timeout_at(at, read).await {
                    Ok(r) => r?,
                    // The rest of this Execute is left to the connection task, which reads and
                    // drops it; the portal's Close queues behind it.
                    Err(_) => return Ok(Rest::Interrupted),
                },
                None => read.await?,
            }
        }
    };
    Ok(if q.stopped_at_limit(affected) {
        Rest::Suspended
    } else {
        Rest::Completed
    })
}

/// `RawConn::close_query`.
pub(crate) async fn close_query(
    cx: Ctx<'_>,
    open: &mut Option<Open>,
    how: CloseHow,
) -> Result<Closed, RawError> {
    if cx.wire.is_broken() {
        *open = None;
        return Err(not_connected());
    }
    let Some(mut q) = open.take() else {
        return Ok(Closed::Closed { rows: 0 });
    };
    // Rows read but not handed out are dropped with the query.
    q.builder = None;
    let delivered = q.delivered;
    let unnamed = q.portal.is_none();
    let outcome = close_rest(cx.client, &mut q, how).await;
    let total = q.rows;
    // The portal's Close + Sync, then the statement's.
    drop(q);
    match outcome {
        Ok(true) => Ok(Closed::Completed { rows: total }),
        Ok(false) => Ok(Closed::Closed { rows: delivered }),
        Err(e) => {
            let raw = failed(cx.wire, &e).await;
            if cx.wire.is_broken() || unnamed {
                // Fix round 1 (review I1): an `Execute(0)` that failed while its rest was read
                // (a cancel's 57014, a runtime error) did not apply: its implicit transaction
                // rolled back, or the user's failed. The caller must know, so it is the error
                // (the connection stays usable; the status is exact after `failed`).
                Err(raw)
            } else {
                // A named portal's server error while closing (a cancel; a portal its
                // transaction already destroyed): closed all the same; Hacivat's own `BEGIN` or
                // savepoint around it is rolled back anyway (R-M3-23), and the status says so.
                Ok(Closed::Closed { rows: delivered })
            }
        }
    }
}

/// Reads what `how` needs of `q` before it closes; `true` when the result completed: always
/// for the unnamed portal once its rest is read (fix round 1, review I1: in autocommit its
/// `Sync` committed it), while draining for a named one.
async fn close_rest(
    client: &Client,
    q: &mut Open,
    how: CloseHow,
) -> Result<bool, tokio_postgres::Error> {
    let drain = match how {
        CloseHow::Drain(d) => Some(Instant::now() + d),
        CloseHow::Now | CloseHow::AfterKill(_) => None,
    };
    if q.portal.is_none() {
        // `Execute(0)`: read to the end, whatever `how` says; it ran to its end on the server.
        if q.stage != Stage::Ended {
            read_rest(q, None).await?;
        }
        return Ok(true);
    }
    let rest = if q.stage == Stage::Ended {
        Rest::Completed
    } else if q.stage == Stage::Suspended {
        Rest::Suspended
    } else {
        // An Execute in flight: its rest (at most its row limit).
        read_rest(q, drain).await?
    };
    let rest = match (rest, drain) {
        (Rest::Suspended, Some(at)) if Instant::now() < at => {
            let Some(portal) = q.portal.as_ref() else {
                return Ok(false);
            };
            let stream = client.query_portal_raw(portal, DRAIN_ROWS).await?;
            q.stream = Some(Box::pin(stream));
            q.batch_rows = 0;
            q.batch_max = DRAIN_ROWS as u64;
            read_rest(q, Some(at)).await?
        }
        (rest, _) => rest,
    };
    Ok(drain.is_some() && matches!(rest, Rest::Completed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_is_the_first_keyword_only() {
        for sql in [
            "COPY t TO STDOUT",
            "copy",
            "\t\r\n CoPy (SELECT 1) TO STDOUT",
            "-- a\n-- b\nCOPY t FROM STDIN",
            "/* a /* nested */ still a comment */COPY t TO STDOUT",
            "/**/copy(SELECT 1) TO STDOUT",
        ] {
            assert!(is_copy(sql), "{sql:?}");
        }
        for sql in [
            "",
            "   ",
            "copyright",
            "COPY_t",
            "copy$1",
            "\"copy\" t",
            "SELECT 1; COPY t TO STDOUT",
            "/* COPY */ SELECT 1",
            "-- COPY\nSELECT 1",
            "/* unterminated COPY",
            "copyş",
        ] {
            assert!(!is_copy(sql), "{sql:?}");
        }
    }

    #[test]
    fn counts_come_from_the_tag() {
        assert_eq!(tag_count("INSERT 0 3"), Some(3));
        assert_eq!(tag_count("SELECT 10"), Some(10));
        assert_eq!(tag_count("CREATE TABLE"), None);
        assert_eq!(tag_count("BEGIN"), None);
        assert_eq!(retag("SELECT 0", 1_200), "SELECT 1200");
        assert_eq!(retag("INSERT 0 7", 9), "INSERT 0 9");
        assert_eq!(retag("FETCH", 9), "FETCH");
    }

    #[test]
    fn limits_never_ask_for_all_rows() {
        assert_eq!(max_rows(0), 1);
        assert_eq!(max_rows(501), 501);
        assert_eq!(max_rows(u32::MAX), i32::MAX);
        let spec = QuerySpec {
            sql: "",
            binds: &[],
            protocol: Protocol::Extended,
            suspendable: true,
            first_rows: 0,
            chunk_rows: 0,
            chunk_bytes: 0,
            cell_cap: 0,
            max_result_sets: 1,
        };
        let l = Limits::of(&spec);
        assert_eq!((l.first_rows, l.chunk_rows), (1, 1));
    }
}
