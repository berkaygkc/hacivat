// ## m3a-t01b-seams
//! Typed streaming (HV-RES-04, R-M3-16, R-M3-18): a query's result sets arrive as
//! [`QueryEvent`]s carrying columnar chunks, and a suspendable query can stop at a read-ahead
//! point and resume (`fetch_more`) or close (`close_query`).
//!
//! The [`crate::RawConn`] methods `typed_caps`, `query`, `next_event`, `fetch_more` and
//! `close_query` default to failing closed with `RawErrorKind::Unsupported`; Tasks 12, 14 and 15
//! implement them per engine, `FakeDriver` (feature `fake`) scripts them.
//!
//! **Event grammar.** `(ResultStart Rows* (Suspended {fetch_more} Rows*)* ResultEnd)* Done`. A
//! statement that returns no rows (DML, DDL, `SET`) still reports one result, with an empty
//! schema: `ResultStart` of zero columns, then `ResultEnd` with `rows_affected` and the command
//! tag. The first chunk holds at most `first_rows` rows, later chunks `chunk_rows` rows or
//! `chunk_bytes` bytes. After `max_result_sets` results the rest are drained and counted
//! (`QueryDone::skipped_result_sets`, R-M3-25); a query with more than one result set never
//! suspends.
//!
//! **Suspension.** Only a query sent with `suspendable: true` suspends: after its first chunk,
//! and after each `fetch_more(n)` batch of `n` rows while rows remain. `next_event` on a
//! suspended query answers `Unsupported` (call `fetch_more` or `close_query`).
//!
//! **The session's rules.**
//! - It never drops a `next_event`/`fetch_more`/`close_query` future before it completes,
//!   except when it discards the connection.
//! - One open query per connection, **except on PostgreSQL**: while the query is `Suspended`,
//!   `execute`/`batch` may run in the same transaction and the named portal survives (the
//!   R-M3-17 canary). MySQL and SQLite answer `Unsupported` to a statement while a query is
//!   open, and every engine answers `Unsupported` to a second `query` (P5).
//! - **PostgreSQL contract (P5):** `suspendable: true` requires `tx_status() != Idle`, else
//!   `query` answers `Unsupported` before sending anything (fail closed). The session opens
//!   `BEGIN` or `SAVEPOINT` (Task 18), or the read-only guard its read-only `BEGIN` (Task 22),
//!   before `query`. A Sync outside a transaction block would end the portal.

use std::sync::Arc;
use std::time::Duration;

use hacivat_values::{BindValue, Chunk, ResultSchema};

use crate::driver::{Protocol, TxStatus};

/// One typed query.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuerySpec<'a> {
    pub sql: &'a str,
    /// Empty in M3a (binds are M3b).
    pub binds: &'a [BindValue],
    /// `Extended` only in M3a; `Simple` answers `Unsupported`.
    pub protocol: Protocol,
    /// The query may stop at a read-ahead point and resume (a console cursor, R-M3-23).
    pub suspendable: bool,
    /// Rows of the first chunk (501: the session shows 500 and knows whether more exist).
    pub first_rows: u32,
    pub chunk_rows: u32,
    pub chunk_bytes: u64,
    /// The most bytes kept of one cell; longer values are truncated with their total length.
    pub cell_cap: u32,
    /// Result sets kept (1 in M3a, R-M3-25); the rest are drained and counted.
    pub max_result_sets: u32,
}

/// A notice's severity, as the server labels it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoticeSeverity {
    Debug,
    Log,
    Info,
    Notice,
    Warning,
}

/// The end of one result set.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResultEnd {
    /// Rows the server sent.
    pub rows: u64,
    /// Rows delivered in chunks (`rows` minus those counted past a cap).
    pub kept: u64,
    pub rows_affected: Option<u64>,
    pub command_tag: Option<String>,
}

/// The end of a query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QueryDone {
    pub tx: TxStatus,
    pub warnings: u16,
    pub skipped_result_sets: u32,
}

/// One step of a typed query (the module docs give the grammar).
#[derive(Debug, Clone, PartialEq)]
pub enum QueryEvent {
    ResultStart(Arc<ResultSchema>),
    Rows(Chunk),
    ResultEnd(ResultEnd),
    Suspended,
    Done(QueryDone),
}

/// How `close_query` ends an open query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CloseHow {
    /// Read on for at most this long; a result that completes meanwhile is `Completed`.
    Drain(Duration),
    /// Close the engine's way at once (PostgreSQL: close the portal).
    Now,
    /// After a server-side kill (MySQL `KILL QUERY`): read up to the kill's error, at most this
    /// long, and leave the connection usable.
    AfterKill(Duration),
}

/// What `close_query` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Closed {
    /// The result completed while draining.
    Completed { rows: u64 },
    /// The query was closed early; `rows` were delivered.
    Closed { rows: u64 },
    /// MySQL: the drain ran out; the session kills `thread_id` on a side connection, then
    /// calls `close_query(AfterKill(..))`.
    NeedsServerKill { thread_id: u64 },
}

/// What a connection's typed path can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypedCaps {
    pub suspend: bool,
    pub binds: bool,
    /// Closing early may need a server-side kill (MySQL).
    pub close_needs_kill: bool,
    /// How long a suspended cursor may idle: MySQL `net_write_timeout` − 15 s; SQLite 30 s in
    /// rollback-journal mode; `None` when the engine sets no bound.
    pub idle_bound: Option<Duration>,
}

impl TypedCaps {
    /// No typed path: what a driver without one reports.
    pub const NONE: TypedCaps = TypedCaps {
        suspend: false,
        binds: false,
        close_needs_kill: false,
        idle_bound: None,
    };
}
