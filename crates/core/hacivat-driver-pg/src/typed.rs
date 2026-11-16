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
//!
//! ## m3b-t13-drivers (HV-EXEC-03 driver half; R-M3-60, -66, -77, -79)
//!
//! **Binds.** `spec.binds` travel as text-format parameters of the Bind message ([`TextParam`]:
//! format code 0, the value's text, or a NULL); the server parses each with its parameter's
//! type input function (R-M3-79). Nothing is ever put into the SQL text. The prepared
//! statement's parameter count must equal `binds.len()`, else `Unsupported` before the Bind
//! (only the Parse/Describe of the prepare went out). No change to the vendored copy: patch
//! §1's entry points take any `ToSql`, whose `encode_format` chooses the format.
//!
//! **Run as one** (`Protocol::Simple`, R-M3-60): the text goes out as one simple `Query`; each
//! statement is one result (`ResultStart … ResultEnd`, a column-less statement an empty one),
//! every column `LogicalType::Text` with `NativeType::Unknown`, decoded from the server's text
//! only (T7 m-10: the binary decoders never see it; a cell that is not UTF-8 is an error cell).
//! The first `max_result_sets` statements are kept, the rest read to their end, undecoded, and
//! counted (`QueryDone::skipped_result_sets`, R-M3-66). It never suspends, takes no binds
//! (`Unsupported`), and cannot be closed on the wire: `close_query` sends the protocol cancel at
//! once and reads the rest, never answering `Completed` past the closed result (fix round 1,
//! I-1: `close_simple`, `RawConn::take_close_report`). It runs as one implicit transaction
//! unless the text has its own `BEGIN`: an error rolls back the statements before it, whose
//! results were already handed out (M-2). Limits of the simple protocol: a set's `command_tag` is `None` (only the
//! last tag is observed, and a later statement's may already have been read), and its
//! `rows_affected` is the `CommandComplete` count of a column-less statement when positive, else
//! `None` (a `CREATE TABLE` and a `DELETE` of nothing both report 0 there).
//!
//! **Notices** (R-M3-77): while a typed query is open the connection task keeps the first
//! [`crate::exec::NOTICE_CAP`] notices' text (`Notice::new`: at most 16 KiB each, cut at a
//! character boundary; never written to a log). `next_event` hands each out as
//! `QueryEvent::Notice` before its next event, so all of a statement's notices precede `Done`;
//! notices raised before a server error come out before that error (the error is held for the
//! next call).

use std::io;
use std::pin::Pin;
use std::sync::Arc;

use futures::{StreamExt, TryStreamExt};
use hacivat_driver::{
    CancelHandle, CloseHow, CloseReport, Closed, Protocol, QueryDone, QueryEvent, QuerySpec,
    RawError, RawErrorKind, ResultEnd, TxStatus, TypedCaps,
};
use hacivat_values::{
    BindValue, BufKind, Chunk, ChunkBuilder, ColumnMeta, ErrorVal, LogicalType, NativeType,
    ResultSchema, Value, VarKind,
};
use postgres_types::{FromSql, IsNull, ToSql, Type, to_sql_checked};
use tokio::time::Instant;
use tokio_postgres::{
    Client, Portal, Row, RowStream, SimpleColumn, SimpleQueryMessage, SimpleQueryRow,
    SimpleQueryStream, Statement,
};

use crate::cancel::PgCancel;
use crate::connect::pg_error;
use crate::decode::{self, Col, Plan, TEXT_FORMAT};
use crate::exec::{
    NoticeWake, Notices, collect_notice_texts, drain_notices, next_notice_text, stop_notice_texts,
};
use crate::wire::{SETTLE, Settled, WireState};

/// What the PostgreSQL typed path can do: suspend (named portals), bind (text-format
/// parameters, m3b-t13), closes without a server-side kill, no idle bound of its own (the
/// session's, R-M3-20).
pub(crate) const CAPS: TypedCaps = TypedCaps {
    suspend: true,
    binds: true,
    close_needs_kill: false,
    idle_bound: None,
};

// ## m3b-t13-drivers
/// One bind value as a text-format parameter (R-M3-79): the value's text, or NULL; the server
/// parses it with the parameter type's input function. Its `Debug` never shows the value.
pub(crate) struct TextParam<'a>(pub &'a BindValue);

impl std::fmt::Debug for TextParam<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.0 {
            BindValue::Null => "TextParam(NULL)",
            BindValue::Typed { .. } => "TextParam(..)",
        })
    }
}

impl ToSql for TextParam<'_> {
    fn to_sql(
        &self,
        _: &Type,
        out: &mut bytes::BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match self.0 {
            BindValue::Null => Ok(IsNull::Yes),
            BindValue::Typed { text, .. } => {
                out.extend_from_slice(text.as_bytes());
                Ok(IsNull::No)
            }
        }
    }

    fn accepts(_: &Type) -> bool {
        true
    }

    fn encode_format(&self, _: &Type) -> postgres_types::Format {
        postgres_types::Format::Text
    }

    to_sql_checked!();
}

/// The rows of `close_query(Drain(..))`'s one Execute.
pub const DRAIN_ROWS: i32 = 4_096;

/// What the typed path uses of a connection.
pub(crate) struct Ctx<'a> {
    pub client: &'a Client,
    pub wire: &'a WireState,
    pub notices: &'a Notices,
    // ## m3b-t13-drivers (fix round 1, I-1)
    /// The connection's protocol cancel: a Run as one closed early is cancelled at once.
    pub cancel: &'a Arc<PgCancel>,
    /// What the last close of a Run as one did beyond its result (`take_close_report`).
    pub report: &'a mut Option<CloseReport>,
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
    // ## m3b-t13-drivers
    /// A server error was read after notices still to hand out: the error is next.
    Failed,
}

// ## m3b-t13-drivers
/// The open typed query of a connection: an extended-protocol statement (a portal), or a
/// simple-protocol text run as one (R-M3-60).
pub(crate) enum Open {
    Ext(Ext),
    Simple(Simple),
}

/// An extended-protocol query. Fields drop in order: the stream and the read-ahead row, then
/// the portal (`Close` + `Sync`), then the statement (`Close` + `Sync`, its last clone).
pub(crate) struct Ext {
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
    // ## m3b-t13-drivers
    /// The server error `Stage::Failed` hands out.
    error: Option<RawError>,
    /// Woken when a notice is kept (fix round 1: notices come out while a statement runs).
    woken: NoticeWake,
}

/// What reading the current stream gave.
enum Fill {
    Chunk(Chunk),
    /// The stream ended (`rows_affected` of its last message) with no rows left to hand out.
    End(Option<u64>),
    // ## m3b-t13-drivers (fix round 1)
    /// A notice was kept while waiting for the server: hand it out, then read on.
    Notice,
}

// ## m3b-t13-drivers (fix round 1)
/// `next`'s output, unless the query is woken by a notice first (`None`; `next` is dropped
/// unfinished, which a stream's `try_next` allows: nothing is lost). A wake signal whose
/// sender is gone never wakes again.
async fn or_notice<T>(next: impl Future<Output = T>, woken: &mut NoticeWake) -> Option<T> {
    let next = std::pin::pin!(next);
    match futures::future::select(next, woken.next()).await {
        futures::future::Either::Left((v, _)) => Some(v),
        futures::future::Either::Right((Some(()), _)) => None,
        futures::future::Either::Right((None, rest)) => Some(rest.await),
    }
}

impl Ext {
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
        let Ext {
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
                    Some(s) => match or_notice(s.as_mut().try_next(), &mut self.woken).await {
                        Some(row) => row?,
                        None => return Ok(Fill::Notice),
                    },
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
    // ## m3b-t13-drivers: a simple-protocol query never suspends.
    open.as_ref()
        .is_none_or(|q| matches!(q, Open::Ext(e) if e.stage == Stage::Suspended))
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

/// A bytes-hinted bind whose text starts with `\x` or `\X` but is not `\x` and pairs of hex
/// digits (fix round 1, I-2; the rule of the MySQL and SQLite drivers' `hex_bytes`). Valid `\x`
/// hex goes as is: `bytea`'s input decodes it to the same bytes the other engines store.
fn malformed_hex(b: &BindValue) -> bool {
    let BindValue::Typed {
        hint: LogicalType::Bytes { .. },
        text,
    } = b
    else {
        return false;
    };
    if text.starts_with("\\X") {
        return true;
    }
    text.strip_prefix("\\x")
        .is_some_and(|hex| hex.len() % 2 != 0 || !hex.bytes().all(|c| c.is_ascii_hexdigit()))
}

/// The binds of a query as text-format parameters (m3b-t13).
fn params<'a>(binds: &'a [BindValue]) -> impl ExactSizeIterator<Item = TextParam<'a>> + use<'a> {
    binds.iter().map(TextParam)
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
    if open.is_some() || is_copy(spec.sql) {
        return Err(unsupported());
    }
    *cx.report = None;
    // Fix round 1 (I-2): a bytes value in malformed `\x` form is refused here as on MySQL and
    // SQLite (the server would answer 22P02 for a `bytea` parameter, but store it verbatim in a
    // `text` one).
    if spec.binds.iter().any(malformed_hex) {
        return Err(unsupported());
    }
    // ## m3b-t13-drivers: Run as one; the simple protocol carries no parameters.
    if spec.protocol == Protocol::Simple {
        if !spec.binds.is_empty() {
            return Err(unsupported());
        }
        return query_simple(cx, open, spec).await;
    }
    // P5: a portal outside a transaction block would end at its first Sync.
    if spec.suspendable && cx.wire.tx() == TxStatus::Idle {
        return Err(unsupported());
    }
    drain_notices(cx.notices);
    cx.wire.clear_tag();
    let _ = cx.wire.take_copy_refused();
    let woken = collect_notice_texts(cx.notices);
    let limits = Limits::of(&spec);
    let statement = match cx.client.prepare(spec.sql).await {
        Ok(s) => s,
        Err(e) => {
            stop_notice_texts(cx.notices);
            return Err(failed(cx.wire, &e).await);
        }
    };
    // ## m3b-t13-drivers: one bind per parameter, else nothing is bound or run (a missing
    // bind never becomes NULL; an extra one is never ignored).
    if statement.params().len() != spec.binds.len() {
        drop(statement);
        stop_notice_texts(cx.notices);
        return Err(unsupported());
    }
    let plan = Plan::for_columns(statement.columns());
    // A statement without columns has no rows to suspend in: the unnamed portal.
    let portal_path = spec.suspendable && !plan.cols.is_empty();
    let started = async {
        if portal_path {
            let portal = cx
                .client
                .bind_with_formats(&statement, params(spec.binds), &plan.formats)
                .await?;
            let first = max_rows(limits.first_rows);
            let stream = cx.client.query_portal_raw(&portal, first).await?;
            Ok((Some(portal), stream, first as u64))
        } else {
            let stream = cx
                .client
                .query_with_formats(&statement, params(spec.binds), &plan.formats)
                .await?;
            Ok((None, stream, 0))
        }
    };
    match started.await {
        Ok((portal, stream, batch_max)) => {
            *open = Some(Open::Ext(Ext {
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
                error: None,
                woken,
            }));
            Ok(())
        }
        Err(e) => {
            drop(statement);
            stop_notice_texts(cx.notices);
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

/// The warning count of a query's end: the notice codes of class `01` since it began.
fn warning_count(notices: &Notices) -> u16 {
    let warnings = drain_notices(notices)
        .iter()
        .filter(|code| code.starts_with("01"))
        .count();
    u16::try_from(warnings).unwrap_or(u16::MAX)
}

/// `RawConn::next_event`.
pub(crate) async fn next_event(
    cx: Ctx<'_>,
    open: &mut Option<Open>,
) -> Result<QueryEvent, RawError> {
    if cx.wire.is_broken() {
        *open = None;
        stop_notice_texts(cx.notices);
        return Err(not_connected());
    }
    // ## m3b-t13-drivers: two kinds of open query.
    match open {
        None => Err(unsupported()),
        Some(Open::Ext(_)) => next_ext(cx, open).await,
        Some(Open::Simple(_)) => next_simple(cx, open).await,
    }
}

/// `next_event` of an extended-protocol query.
async fn next_ext(cx: Ctx<'_>, open: &mut Option<Open>) -> Result<QueryEvent, RawError> {
    let Some(Open::Ext(q)) = open.as_mut() else {
        return Err(unsupported());
    };
    if q.stage == Stage::Suspended {
        return Err(unsupported());
    }
    // ## m3b-t13-drivers: the notices raised so far come before the next event (R-M3-77).
    if let Some(notice) = next_notice_text(cx.notices) {
        return Ok(QueryEvent::Notice(notice));
    }
    match q.stage {
        Stage::Start => {
            q.stage = Stage::Rows;
            return Ok(QueryEvent::ResultStart(q.plan.schema.clone()));
        }
        Stage::Suspended => return Err(unsupported()),
        Stage::Ended => {
            let warnings = warning_count(cx.notices);
            *open = None;
            stop_notice_texts(cx.notices);
            return Ok(QueryEvent::Done(QueryDone {
                tx: cx.wire.tx(),
                warnings,
                skipped_result_sets: 0,
            }));
        }
        // ## m3b-t13-drivers
        Stage::Failed => {
            let e = q.error.take().unwrap_or_else(not_connected);
            *open = None;
            stop_notice_texts(cx.notices);
            return Err(e);
        }
        Stage::Rows => {}
    }
    // Fix round 1: a notice kept while the statement runs goes out at once; a wake whose
    // notice already went out reads on.
    let filled = loop {
        match q.fill().await {
            Ok(Fill::Notice) => {
                if let Some(notice) = next_notice_text(cx.notices) {
                    return Ok(QueryEvent::Notice(notice));
                }
            }
            other => break other,
        }
    };
    match filled {
        Ok(Fill::Notice) => unreachable!("handled above"),
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
            let raw = failed(cx.wire, &e).await;
            // ## m3b-t13-drivers: notices raised before the error come out first; the error
            // is held for the next call (a broken connection hands out nothing more).
            if !cx.wire.is_broken()
                && let Some(notice) = next_notice_text(cx.notices)
            {
                q.stage = Stage::Failed;
                q.error = Some(raw);
                q.stream = None;
                q.builder = None;
                return Ok(QueryEvent::Notice(notice));
            }
            *open = None;
            stop_notice_texts(cx.notices);
            Err(raw)
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
        stop_notice_texts(cx.notices);
        return Err(not_connected());
    }
    // ## m3b-t13-drivers: only a suspended portal resumes.
    let Some(Open::Ext(q)) = open.as_mut() else {
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
            stop_notice_texts(cx.notices);
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
            stop_notice_texts(cx.notices);
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
async fn read_rest(q: &mut Ext, deadline: Option<Instant>) -> Result<Rest, tokio_postgres::Error> {
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
        stop_notice_texts(cx.notices);
        return Err(not_connected());
    }
    // ## m3b-t13-drivers: a closed query's notices not yet handed out are dropped with it.
    stop_notice_texts(cx.notices);
    let mut q = match open.take() {
        None => return Ok(Closed::Closed { rows: 0 }),
        Some(Open::Simple(s)) => return close_simple(cx, s).await,
        Some(Open::Ext(q)) => q,
    };
    // ## m3b-t13-drivers: the server error held behind notices is the outcome (the rest was
    // read when it arrived).
    // Fix round 1 (M-4): for a named portal too: the server's error already ended the query, so
    // it is never swallowed by the close.
    if q.stage == Stage::Failed {
        return Err(q.error.take().unwrap_or_else(not_connected));
    }
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
    q: &mut Ext,
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

// ## m3b-t13-drivers: Run as one over the simple protocol (R-M3-60, R-M3-66; T7 m-10).

/// Where a simple-protocol query is between the server's messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SimpleStage {
    /// Between statements: a `RowDescription`, a `CommandComplete` (a statement without rows)
    /// or the end is next.
    Between,
    /// Rows of a kept result.
    Rows,
    /// Rows of a result past `max_result_sets`, read and dropped.
    Skip,
    /// The held server error is next.
    Failed,
    /// The stream ended: `Done` is next.
    Done,
}

/// A simple-protocol query: one result per statement, every column the server's text.
pub(crate) struct Simple {
    stream: Option<Pin<Box<SimpleQueryStream>>>,
    limits: Limits,
    max_sets: u32,
    stage: SimpleStage,
    /// The current kept result's plan (every column `Col::Text`).
    plan: Option<Plan>,
    builder: Option<ChunkBuilder>,
    /// Rows of the current result read, and handed out in chunks.
    rows: u64,
    delivered: u64,
    chunks: u64,
    /// Results started, and results past the limit read and counted.
    kept: u32,
    skipped: u32,
    /// A `ResultEnd` due right after the event just handed out.
    end: Option<ResultEnd>,
    error: Option<RawError>,
    /// Woken when a notice is kept (fix round 1: notices come out while a statement runs).
    woken: NoticeWake,
}

/// The plan of a simple-protocol result: every column text, of unknown native type (the
/// simple protocol names no type), decoded from the server's text only.
fn text_plan(columns: &[SimpleColumn]) -> Plan {
    let n = columns.len();
    let meta: Vec<ColumnMeta> = columns
        .iter()
        .map(|c| ColumnMeta {
            name: c.name().into(),
            logical: LogicalType::Text {
                max_chars: None,
                fixed: false,
            },
            native: NativeType::Unknown,
            origin: None,
        })
        .collect();
    Plan {
        schema: Arc::new(ResultSchema {
            columns: meta.into(),
        }),
        cols: vec![Col::Text; n],
        kinds: vec![BufKind::Var(VarKind::Text); n],
        formats: vec![TEXT_FORMAT; n],
    }
}

/// A cell whose text is not UTF-8 (a `SQL_ASCII` database): the simple protocol hands out text
/// only, so the bytes cannot be kept; the row is.
fn not_utf8() -> Value {
    Value::Error(Arc::new(ErrorVal {
        code: "pg-text".into(),
        message: "the server's text of this value is not UTF-8".into(),
    }))
}

fn protocol_error() -> RawError {
    RawError {
        kind: RawErrorKind::Protocol,
        server: None,
    }
}

impl Simple {
    fn row_limit(&self) -> u32 {
        if self.chunks == 0 {
            self.limits.first_rows
        } else {
            self.limits.chunk_rows
        }
    }

    fn sealable(&self) -> bool {
        self.builder.as_ref().is_some_and(|b| {
            b.rows() > 0
                && (b.rows() >= self.row_limit()
                    || b.bytes() >= self.limits.chunk_bytes
                    || b.is_full())
        })
    }

    fn take_chunk(&mut self) -> Option<Chunk> {
        let chunk = self.builder.take()?.seal();
        self.delivered += u64::from(chunk.rows());
        self.chunks += 1;
        Some(chunk)
    }

    /// The next kept result, of `plan`.
    fn start(&mut self, plan: Plan) -> Arc<ResultSchema> {
        let schema = plan.schema.clone();
        self.plan = Some(plan);
        self.builder = None;
        self.rows = 0;
        self.delivered = 0;
        self.chunks = 0;
        self.kept += 1;
        self.stage = SimpleStage::Rows;
        schema
    }

    /// One row of the current result, each cell the server's text (the row's field count was
    /// checked against the description by `tokio-postgres`).
    fn push(&mut self, row: &SimpleQueryRow) {
        let limit = self.row_limit();
        let Simple {
            plan,
            builder,
            limits,
            ..
        } = self;
        let Some(plan) = plan.as_ref() else {
            return;
        };
        let b = builder.get_or_insert_with(|| plan.builder(limit));
        for (i, col) in plan.cols.iter().enumerate() {
            match row.try_get(i) {
                Ok(text) => {
                    decode::push(b, i as u32, col, text.map(str::as_bytes), limits.cell_cap)
                }
                Err(_) => b.push_value(i as u32, not_utf8()),
            }
        }
        b.end_row();
        self.rows += 1;
    }
}

/// `query` with `Protocol::Simple`: the text goes out as one `Query`; its results follow from
/// `next_event`.
async fn query_simple(
    cx: Ctx<'_>,
    open: &mut Option<Open>,
    spec: QuerySpec<'_>,
) -> Result<(), RawError> {
    drain_notices(cx.notices);
    cx.wire.clear_tag();
    let _ = cx.wire.take_copy_refused();
    let woken = collect_notice_texts(cx.notices);
    match cx.client.simple_query_raw(spec.sql).await {
        Ok(stream) => {
            *open = Some(Open::Simple(Simple {
                stream: Some(Box::pin(stream)),
                limits: Limits::of(&spec),
                max_sets: spec.max_result_sets.max(1),
                stage: SimpleStage::Between,
                plan: None,
                builder: None,
                rows: 0,
                delivered: 0,
                chunks: 0,
                kept: 0,
                skipped: 0,
                end: None,
                error: None,
                woken,
            }));
            Ok(())
        }
        Err(e) => {
            stop_notice_texts(cx.notices);
            Err(failed(cx.wire, &e).await)
        }
    }
}

/// `next_event` of a simple-protocol query.
async fn next_simple(cx: Ctx<'_>, open: &mut Option<Open>) -> Result<QueryEvent, RawError> {
    let Some(Open::Simple(q)) = open.as_mut() else {
        return Err(unsupported());
    };
    loop {
        if let Some(notice) = next_notice_text(cx.notices) {
            return Ok(QueryEvent::Notice(notice));
        }
        if let Some(end) = q.end.take() {
            return Ok(QueryEvent::ResultEnd(end));
        }
        match q.stage {
            SimpleStage::Failed => {
                let e = q.error.take().unwrap_or_else(not_connected);
                *open = None;
                stop_notice_texts(cx.notices);
                return Err(e);
            }
            SimpleStage::Done => {
                let done = QueryDone {
                    tx: cx.wire.tx(),
                    warnings: warning_count(cx.notices),
                    skipped_result_sets: q.skipped,
                };
                *open = None;
                stop_notice_texts(cx.notices);
                return Ok(QueryEvent::Done(done));
            }
            SimpleStage::Between | SimpleStage::Rows | SimpleStage::Skip => {}
        }
        let message = match q.stream.as_mut() {
            // Fix round 1: a notice kept meanwhile goes out at once (the loop's top).
            Some(s) => match or_notice(s.as_mut().try_next(), &mut q.woken).await {
                Some(m) => m,
                None => continue,
            },
            None => Ok(None),
        };
        let message = match message {
            Ok(m) => m,
            Err(e) => {
                let raw = failed(cx.wire, &e).await;
                if cx.wire.is_broken() {
                    *open = None;
                    stop_notice_texts(cx.notices);
                    return Err(raw);
                }
                // The notices raised before it come out first (the loop's top).
                q.stage = SimpleStage::Failed;
                q.error = Some(raw);
                q.stream = None;
                q.builder = None;
                continue;
            }
        };
        match (q.stage, message) {
            (SimpleStage::Between, Some(SimpleQueryMessage::RowDescription(columns))) => {
                if q.kept < q.max_sets {
                    let schema = q.start(text_plan(&columns));
                    return Ok(QueryEvent::ResultStart(schema));
                }
                q.stage = SimpleStage::Skip;
            }
            // A statement without rows (DML, DDL, `SET`, an empty query): one empty result.
            (SimpleStage::Between, Some(SimpleQueryMessage::CommandComplete(n))) => {
                if q.kept < q.max_sets {
                    q.kept += 1;
                    q.end = Some(ResultEnd {
                        rows: 0,
                        kept: 0,
                        rows_affected: (n > 0).then_some(n),
                        command_tag: None,
                    });
                    return Ok(QueryEvent::ResultStart(Arc::new(ResultSchema {
                        columns: Arc::from([]),
                    })));
                }
                q.skipped = q.skipped.saturating_add(1);
            }
            (SimpleStage::Between, None) => q.stage = SimpleStage::Done,
            (SimpleStage::Rows, Some(SimpleQueryMessage::Row(row))) => {
                q.push(&row);
                if q.sealable()
                    && let Some(chunk) = q.take_chunk()
                {
                    return Ok(QueryEvent::Rows(chunk));
                }
            }
            (SimpleStage::Rows, Some(SimpleQueryMessage::CommandComplete(n))) => {
                let columnless = q.plan.as_ref().is_none_or(|p| p.cols.is_empty());
                let chunk = q.take_chunk();
                let end = ResultEnd {
                    rows: q.rows,
                    kept: q.delivered,
                    // A read affected nothing (its count is `rows`).
                    rows_affected: (columnless && n > 0).then_some(n),
                    command_tag: None,
                };
                q.stage = SimpleStage::Between;
                q.plan = None;
                match chunk {
                    Some(chunk) => {
                        q.end = Some(end);
                        return Ok(QueryEvent::Rows(chunk));
                    }
                    None => return Ok(QueryEvent::ResultEnd(end)),
                }
            }
            (SimpleStage::Skip, Some(SimpleQueryMessage::Row(_))) => {}
            (SimpleStage::Skip, Some(SimpleQueryMessage::CommandComplete(_))) => {
                q.skipped = q.skipped.saturating_add(1);
                q.stage = SimpleStage::Between;
            }
            // Out of shape (a row outside a result, a description inside one, the end inside
            // one): the stream cannot be trusted.
            _ => {
                cx.wire.mark_broken();
                *open = None;
                stop_notice_texts(cx.notices);
                return Err(protocol_error());
            }
        }
    }
}

/// `close_query` of a simple-protocol query closed before its end (fix round 1, I-1, controller
/// ruling (a)). It cannot be closed on the wire, so the driver sends the connection's protocol
/// cancel **at once**, whatever `how` and the transaction status (unless the server already
/// answered the whole text: a cancel then could reach the next statement), and reads the rest
/// undecoded to keep the connection in step. The answer:
/// - the rest ended in a server error (the cancel's 57014, or a statement's own): that error,
///   and a [`CloseReport`] with `rest_not_run`;
/// - it ended cleanly with nothing after the closed result: `Completed { rows }`;
/// - it ended cleanly but later statements ran (the cancel came too late): `Closed { rows:
///   delivered }`, never `Completed`, and a [`CloseReport`] with how many ran unseen.
async fn close_simple(cx: Ctx<'_>, mut q: Simple) -> Result<Closed, RawError> {
    match q.stage {
        SimpleStage::Failed => return Err(q.error.take().unwrap_or_else(not_connected)),
        SimpleStage::Done => return Ok(Closed::Completed { rows: q.rows }),
        SimpleStage::Between | SimpleStage::Rows | SimpleStage::Skip => {}
    }
    let mut rows = q.rows;
    // The closed result's rows are still coming; every other statement end read is one that ran.
    let mut current = q.stage == SimpleStage::Rows;
    let mut ran = 0_u32;
    let Some(mut stream) = q.stream.take() else {
        return Ok(Closed::Completed { rows });
    };
    if !cx.wire.synced() {
        // Fire and forget: the outcome is read from the stream either way.
        tokio::spawn(cx.cancel.cancel());
    }
    loop {
        match stream.as_mut().try_next().await {
            Ok(Some(SimpleQueryMessage::Row(_))) if current => rows += 1,
            Ok(Some(SimpleQueryMessage::CommandComplete(_))) if current => current = false,
            Ok(Some(SimpleQueryMessage::CommandComplete(_))) => ran = ran.saturating_add(1),
            Ok(Some(_)) => {}
            Ok(None) if ran == 0 => return Ok(Closed::Completed { rows }),
            Ok(None) => {
                *cx.report = Some(CloseReport {
                    statements_ran: ran,
                    rest_not_run: false,
                });
                return Ok(Closed::Closed { rows: q.delivered });
            }
            Err(e) => {
                *cx.report = Some(CloseReport {
                    statements_ran: ran,
                    rest_not_run: true,
                });
                return Err(failed(cx.wire, &e).await);
            }
        }
    }
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
