//! `FakeDriver`: a scripted in-process driver for pipeline, manager and UI tests (F4).
//! Working since Task 1 (feature `fake`).
//!
//! Each `connect` counts the call, records the request (password included, as a plain string,
//! for asserts; its `Debug` redacts it), waits the configured delay, then returns the next
//! scripted result or the default one. A successful connect reports `TlsStarted`/`TlsDone`
//! (with TLS params) and `AuthStarted`/`AuthDone` to the request's event callback. A gated
//! driver (`gated`) holds every connect until `release`, so a test can observe a connect in
//! flight without racing a delay.
//!
//! M2b (Task 1, Q23): connections run statements from a script (`exec`). The first unconsumed
//! [`FakeExec`] whose prefix matches is used once; an unmatched statement succeeds with an
//! empty summary and leaves the transaction status unchanged. `executed` records `execute` and
//! `batch` text in order. A cancel (through `cancel_handle`) ends the running statement with
//! `RawErrorKind::Cancelled` unless the driver `ignores_cancel`. `kill_connections` makes every
//! live connection fail with `Io(ConnectionReset)`.
//!
//! M2b (Task 7, T1 review M10): a statement counts as running from the `execute`/`batch` call,
//! so a cancel sent before its future is first polled is latched and still ends it; a cancel
//! sent while nothing runs is a no-op (as on a real server) and never reaches a later statement.
//!
//! M2b (Task 17 fix round 1, review I4): a connection that holds its endpoint's stream
//! (`consumes_factory`) fails its ping with `Io(ConnectionReset)` once that stream has ended,
//! so a test sees an SSH channel die with its tunnel, as a real driver would.
//!
//! M2b (finding F3): `execute` with [`Protocol::Extended`] answers a scripted result without
//! its row values (the row count stays), as the PostgreSQL and MySQL drivers do. A statement
//! whose values the caller reads must run with `ExecOpts::values`, or its script is not seen.
//!
//! M3a (Task 1b): typed scripting ([`FakeDriver::queries`]). A typed `query` takes the first
//! unconsumed [`FakeQuery`] whose prefix matches (once); an unmatched query answers one result
//! with an empty schema and no rows. Its events follow the grammar of `crate::typed`: the first
//! chunk holds `first_rows` rows, later chunks `chunk_rows`; a `suspendable` query with one
//! result set suspends after its first chunk and after each `fetch_more(n)` batch while rows
//! remain. Connections enforce the driver contract like the real drivers (P5): on the
//! PostgreSQL wire family a suspendable query needs `tx_status() != Idle`, else `Unsupported`;
//! a second query, and on MySQL and SQLite any statement, while a query is open answers
//! `Unsupported`; PostgreSQL runs a statement while the query is suspended (the canary). A
//! cancel (latched like M10) or a scripted `fail_after` error inside a PostgreSQL transaction
//! leaves it `Failed`, as the server does. `typed_log` records every typed call; `call_log`
//! every statement, typed call and reset in order. `tracks_transactions` makes connections set
//! their status from the leading keywords of `execute`/`batch` text (`BEGIN`, `COMMIT`, ...)
//! when no script entry says otherwise.
//!
//! M3a (Task 12 fix round 1, the PostgreSQL driver's behaviour, pinned by its Docker tests):
//! - a PostgreSQL batch that returns exactly its limit suspends even when no row is left; the
//!   next `fetch_more` ends the result with no `Rows`; `fetch_more(0)` asks for `chunk_rows`;
//! - a suspended PostgreSQL query whose transaction ended (`COMMIT`, `ROLLBACK`, `ROLLBACK TO`
//!   the portal's own or an older savepoint) stays the open query: `next_event` answers
//!   `Unsupported`, `fetch_more` answers 34000 once (failing a transaction block) and ends it,
//!   `close_query` answers `Closed { rows: sent }` (a drain's Execute meets the 34000 and fails
//!   a block), and a new `query` answers `Unsupported` until then;
//! - closing a PostgreSQL query that cannot suspend (`Execute(0)`) reads it to the end:
//!   `Completed { rows: total }`, or the error its rest meets (a `fail_after` inside it, a
//!   latched cancel), which ends it and fails a transaction block;
//! - a scripted result reports `rows_affected` and `command_tag` as scripted (`None` by
//!   default), which is what the PostgreSQL driver reports for a read (`rows_affected: None`).
//!
//! M3a (Task 14 fix round 1, named exception: the MySQL driver's behaviour, pinned by its wire
//! and Docker tests). On the MySQL family (an engine that closes early by a kill: caps
//! `close_needs_kill`, not the PostgreSQL wire), `close_query` follows the real driver:
//! - `Now` answers `NeedsServerKill { thread_id }` and keeps the run open while anything is
//!   unread; with nothing unread it finishes: `Completed { rows: total }`;
//! - `Drain` reads a rest of at most 4,096 rows (the stand-in for "finishes within `d`") to its
//!   end: `Completed { rows: total }`, or the scripted `fail_after` error it meets there (a 1317
//!   is the kill's: `Closed { rows: sent }`); a longer rest, or `kill_needed`, answers
//!   `NeedsServerKill` and keeps the run open;
//! - `AfterKill` with anything unread is `Closed { rows: sent }` (the kill's 1317), with nothing
//!   unread `Completed { rows: total }` (the kill came too late); with `kill_ignored` it runs
//!   out: `Err(Io(TimedOut))` and the connection is lost;
//! - `thread_id` is the connection's own `backend_id()`: every connection of a driver gets its
//!   own (`backend(id)` gives the first `id`, the next `id + 1`, ...; MySQL-family connections
//!   have one by default, never 0).

use std::collections::VecDeque;
use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_net::{BoxedStream, PresentedChain, Target, TlsSettings, TlsSummary, TlsVerified};
use hacivat_proto::DbmsKind;
use secrecy::ExposeSecret;
use tokio::sync::Notify;

use crate::driver::{
    CancelHandle, CancelMechanism, Driver, DriverCaps, DriverConnect, DriverDescriptor,
    DriverEvent, Endpoint, ExecOpts, ExecSummary, InsecureAuth, ProbeWarning, Protocol, RawConn,
    TxStatus,
};
use crate::errors::{RawError, RawErrorKind};
use crate::server::ServerInfo;
// ## m3a-t01b-seams
use crate::typed::{CloseHow, Closed, QueryDone, QueryEvent, QuerySpec, ResultEnd, TypedCaps};
use hacivat_values::{Chunk, ColumnMeta, LogicalType, NativeType, ResultSchema, Value};

// ## m2b-t01-interfaces
/// One scripted statement (Q23): used once, by the first statement starting with `prefix`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FakeExec {
    pub prefix: &'static str,
    pub result: Result<ExecSummary, RawError>,
    /// How long the statement runs (a cancel ends it early).
    pub delay: Duration,
    /// The connection's transaction status after the statement.
    pub tx_after: Option<TxStatus>,
    /// The statement panics (an actor's panic handling).
    pub panics: bool,
}

/// The statement script and log, shared by the driver and its connections.
#[derive(Default)]
struct ExecState {
    script: Mutex<Vec<Option<FakeExec>>>,
    executed: Mutex<Vec<String>>,
    // ## m3a-t01b-seams
    queries: Mutex<Vec<Option<FakeQuery>>>,
    typed: Mutex<Vec<FakeTypedCall>>,
    calls: Mutex<Vec<FakeCall>>,
}

impl ExecState {
    /// Logs `sql` and takes the first unconsumed entry whose prefix matches it.
    fn take(&self, sql: &str) -> Option<FakeExec> {
        self.log_statement(sql);
        let text = sql.trim_start();
        lock(&self.script)
            .iter_mut()
            .find(|e| e.as_ref().is_some_and(|e| text.starts_with(e.prefix)))
            .and_then(Option::take)
    }

    // ## m3a-t01b-seams

    /// Logs an `execute`/`batch` text (in `executed` and the call log).
    fn log_statement(&self, sql: &str) {
        lock(&self.executed).push(sql.to_string());
        lock(&self.calls).push(FakeCall::Statement(sql.to_string()));
    }

    /// Logs a typed call (in `typed_log` and the call log).
    fn log_typed(&self, call: FakeTypedCall) {
        lock(&self.typed).push(call.clone());
        lock(&self.calls).push(FakeCall::Typed(call));
    }

    /// The first unconsumed typed script entry whose prefix matches `sql`.
    fn take_query(&self, sql: &str) -> Option<FakeQuery> {
        let text = sql.trim_start();
        lock(&self.queries)
            .iter_mut()
            .find(|e| e.as_ref().is_some_and(|e| text.starts_with(e.prefix)))
            .and_then(Option::take)
    }
}

// ## m3a-t01b-seams

/// The rows of one scripted result set.
#[derive(Debug, Clone)]
pub enum FakeRows {
    Literal(Vec<Vec<Value>>),
    /// `count` rows whose cell at (row, column) is `cell(row, column)`, made chunk by chunk
    /// (`u64::MAX` streams without end).
    Generated {
        count: u64,
        cell: fn(u64, usize) -> Value,
    },
}

impl Default for FakeRows {
    fn default() -> Self {
        FakeRows::Literal(Vec::new())
    }
}

impl FakeRows {
    fn len(&self) -> u64 {
        match self {
            FakeRows::Literal(rows) => rows.len() as u64,
            FakeRows::Generated { count, .. } => *count,
        }
    }

    /// Rows `from..to`, `width` cells each.
    fn rows(&self, from: u64, to: u64, width: usize) -> Vec<Vec<Value>> {
        match self {
            FakeRows::Literal(rows) => rows[from as usize..to as usize].to_vec(),
            FakeRows::Generated { cell, .. } => (from..to)
                .map(|r| (0..width).map(|c| cell(r, c)).collect())
                .collect(),
        }
    }
}

/// One scripted result set: its columns (name, logical type) and rows.
#[derive(Debug, Clone, Default)]
pub struct FakeResultSet {
    pub columns: Vec<(&'static str, LogicalType)>,
    pub rows: FakeRows,
}

impl FakeResultSet {
    fn schema(&self) -> Arc<ResultSchema> {
        Arc::new(ResultSchema {
            columns: self
                .columns
                .iter()
                .map(|(name, logical)| ColumnMeta {
                    name: (*name).into(),
                    logical: logical.clone(),
                    native: NativeType::Unknown,
                    origin: None,
                })
                .collect(),
        })
    }
}

/// One scripted typed query: used once, by the first `query` whose text starts with `prefix`.
#[derive(Debug, Clone, Default)]
pub struct FakeQuery {
    pub prefix: &'static str,
    /// The result sets; none is one result with an empty schema (DML, DDL).
    pub sets: Vec<FakeResultSet>,
    /// After this many rows (over every set) `next_event` fails with the error.
    pub fail_after: Option<(u64, RawError)>,
    /// Before the first event.
    pub first_delay: Duration,
    /// Before every chunk after the first.
    pub chunk_delay: Duration,
    /// Every event is ready at once: no delay and no yield to the scheduler.
    pub never_pending: bool,
    /// A cancel does not end this query.
    pub ignores_cancel: bool,
    /// The connection's transaction status after `Done`.
    pub tx_after: Option<TxStatus>,
    /// Every kept result's `ResultEnd::rows_affected`.
    pub rows_affected: Option<u64>,
    /// `close_query(Drain(..))` answers `NeedsServerKill` whatever is left (MySQL); `AfterKill`
    /// then closes. Without it a drain past 4,096 rows still asks for a kill when the caps say
    /// `close_needs_kill` (MySQL's default caps do).
    pub kill_needed: bool,
    // ## m3a-t14-mysql-typed (review round 1, named exception)
    /// MySQL family: the kill never ends the query, so `close_query(AfterKill(..))` runs out:
    /// `Err(Io(TimedOut))`, and the connection is lost (every later call answers
    /// `Io(NotConnected)`), as the real driver abandons it.
    pub kill_ignored: bool,
}

/// A typed call, as `typed_log` records it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FakeTypedCall {
    Query(String),
    FetchMore(u32),
    Close(CloseHow),
}

/// Every call on a fake connection that touches the server, in order (`call_log`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FakeCall {
    /// An `execute` or `batch` text.
    Statement(String),
    Typed(FakeTypedCall),
    Reset,
}

/// Whether `kind` speaks the PostgreSQL wire protocol (the P5 contract applies).
fn pg_wire(kind: DbmsKind) -> bool {
    matches!(
        kind,
        DbmsKind::Postgres
            | DbmsKind::Redshift
            | DbmsKind::Cockroach
            | DbmsKind::Yugabyte
            | DbmsKind::Timescale
    )
}

// ## m3a-t14-mysql-typed (review round 1)
/// Whether `kind` speaks the MySQL wire protocol (a kill closes a result early).
fn mysql_family(kind: DbmsKind) -> bool {
    matches!(kind, DbmsKind::MySql | DbmsKind::MariaDb | DbmsKind::TiDb)
}

/// The thread ID of a MySQL-family driver's first connection when no `backend(id)` is set.
const FIRST_THREAD_ID: u64 = 1_000;

/// What a fake connection's typed path reports unless `with_typed_caps` says otherwise: the
/// real drivers' shapes (MySQL: a kill to close early, `net_write_timeout` − 15 s at the
/// server default).
fn default_typed_caps(kind: DbmsKind) -> TypedCaps {
    match kind {
        DbmsKind::MySql | DbmsKind::MariaDb | DbmsKind::TiDb => TypedCaps {
            suspend: true,
            binds: false,
            close_needs_kill: true,
            idle_bound: Some(Duration::from_secs(45)),
        },
        _ => TypedCaps {
            suspend: true,
            binds: false,
            close_needs_kill: false,
            idle_bound: None,
        },
    }
}

/// The transaction status `sql` leaves behind when connections track transactions: each
/// `;`-separated statement's leading keywords, in order (Hacivat's control text has no quoted
/// `;`).
fn tracked_tx(mut tx: TxStatus, sql: &str) -> TxStatus {
    for statement in sql.split(';') {
        let mut words = statement
            .split(|c: char| !c.is_ascii_alphabetic())
            .filter(|w| !w.is_empty());
        let first = words.next().unwrap_or_default();
        let second = words.next().unwrap_or_default();
        let is = |w: &str, k: &str| w.eq_ignore_ascii_case(k);
        tx = if is(first, "BEGIN") || is(first, "START") {
            if tx == TxStatus::Idle {
                TxStatus::InTx
            } else {
                tx
            }
        } else if is(first, "COMMIT") || is(first, "END") {
            TxStatus::Idle
        } else if is(first, "ROLLBACK") || is(first, "ABORT") {
            if is(second, "TO") {
                // `ROLLBACK TO SAVEPOINT` keeps the transaction and recovers a failed one.
                if tx == TxStatus::Idle {
                    tx
                } else {
                    TxStatus::InTx
                }
            } else {
                TxStatus::Idle
            }
        } else {
            tx
        };
    }
    tx
}

/// An open typed query of a [`FakeConn`].
struct TypedRun {
    sets: VecDeque<FakeResultSet>,
    skipped: u32,
    current: Option<OpenSet>,
    first_rows: u64,
    chunk_rows: u64,
    /// `Some(n)`: rows left before the query suspends (suspendable queries only).
    budget: Option<u64>,
    suspended: bool,
    started: bool,
    /// Rows delivered over every set.
    sent: u64,
    chunks: u64,
    script: FakeQuery,
    /// Savepoints open when the query started (round 2): `ROLLBACK TO` one of them, or to one
    /// made later, destroys the portal (PostgreSQL closes portals created inside a rolled-back
    /// savepoint).
    savepoint_depth: usize,
    // ## m3a-t12-pg-typed (fix round 1, review I4b)
    /// PostgreSQL ended the transaction under this suspended portal: the run is kept (still the
    /// connection's one open query, as the real driver keeps it) but the server has no portal.
    gone: bool,
}

/// The result set a typed query is delivering.
struct OpenSet {
    set: FakeResultSet,
    width: usize,
    total: u64,
    pos: u64,
}

impl TypedRun {
    fn new(spec: &QuerySpec<'_>, script: FakeQuery, can_suspend: bool) -> Self {
        let mut sets: VecDeque<FakeResultSet> = script.sets.iter().cloned().collect();
        if sets.is_empty() {
            sets.push_back(FakeResultSet::default());
        }
        // A query with more than one result set never suspends (R-M3-25), however many are
        // kept.
        let single = sets.len() == 1;
        let keep = spec.max_result_sets.max(1) as usize;
        let skipped = sets.len().saturating_sub(keep) as u32;
        sets.truncate(keep);
        let chunk_rows = u64::from(spec.chunk_rows.max(1));
        let first_rows = match spec.first_rows {
            0 => chunk_rows,
            n => u64::from(n),
        };
        let suspends = spec.suspendable && can_suspend && single;
        TypedRun {
            savepoint_depth: 0,
            sets,
            skipped,
            current: None,
            first_rows,
            chunk_rows,
            budget: suspends.then_some(first_rows),
            suspended: false,
            started: false,
            sent: 0,
            chunks: 0,
            script,
            gone: false,
        }
    }

    /// Rows not yet delivered, in this set and the ones after it.
    fn remaining(&self) -> u64 {
        let here = self.current.as_ref().map_or(0, |s| s.total - s.pos);
        self.sets
            .iter()
            .fold(here, |sum, s| sum.saturating_add(s.rows.len()))
    }
}

/// A [`DriverConnect`] endpoint without its stream factory.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FakeEndpoint {
    Network {
        direct: Option<Target>,
        resolved: Vec<SocketAddr>,
        tls_host: String,
    },
    File {
        path: PathBuf,
        create_if_missing: bool,
        read_only: bool,
    },
}

/// The TLS part of a [`DriverConnect`] without the rustls configuration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FakeTls {
    pub settings: TlsSettings,
    pub server_name: String,
    pub plaintext_allowed: bool,
}

/// What the last `connect` received: a [`DriverConnect`] without the factory and the event
/// callback, with the password as a plain string for asserts. `Debug` prints
/// `password: <redacted>` (P33).
#[derive(Clone, Default)]
pub struct FakeRequest {
    pub endpoint: Option<FakeEndpoint>,
    pub tls: Option<FakeTls>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub database: Option<String>,
    pub application_name: String,
    pub read_only: bool,
    pub options: Vec<(String, String)>,
    pub connect_timeout: Duration,
    // ## m2b-t01-interfaces
    pub insecure_auth: InsecureAuth,
}

impl fmt::Debug for FakeRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FakeRequest")
            .field("endpoint", &self.endpoint)
            .field("tls", &self.tls)
            .field("user", &self.user)
            .field(
                "password",
                &format_args!(
                    "{}",
                    if self.password.is_some() {
                        "<redacted>"
                    } else {
                        "None"
                    }
                ),
            )
            .field("database", &self.database)
            .field("application_name", &self.application_name)
            .field("read_only", &self.read_only)
            .field("options", &self.options)
            .field("connect_timeout", &self.connect_timeout)
            .field("insecure_auth", &self.insecure_auth)
            .finish()
    }
}

impl FakeRequest {
    fn of(req: &DriverConnect) -> Self {
        let endpoint = match &req.endpoint {
            Endpoint::Network {
                direct,
                resolved,
                tls_host,
                ..
            } => FakeEndpoint::Network {
                direct: direct.clone(),
                resolved: resolved.clone(),
                tls_host: tls_host.clone(),
            },
            Endpoint::File {
                path,
                create_if_missing,
                read_only,
            } => FakeEndpoint::File {
                path: path.clone(),
                create_if_missing: *create_if_missing,
                read_only: *read_only,
            },
        };
        FakeRequest {
            endpoint: Some(endpoint),
            tls: req.tls.as_ref().map(|t| FakeTls {
                settings: t.settings.clone(),
                server_name: t.server_name.to_str().into_owned(),
                plaintext_allowed: t.plaintext_allowed,
            }),
            user: req.user.clone(),
            password: req.password.as_ref().map(|p| p.expose_secret().to_string()),
            database: req.database.clone(),
            application_name: req.application_name.clone(),
            read_only: req.read_only,
            options: req.options.clone(),
            connect_timeout: req.connect_timeout,
            insecure_auth: req.insecure_auth,
        }
    }
}

#[derive(Default)]
struct Counters {
    connect: AtomicUsize,
    close: AtomicUsize,
    /// Connection objects alive: made for a connect, not yet dropped (closed or abandoned,
    /// e.g. with a pending connect when its runtime stops). M2a Task 18.
    alive: AtomicUsize,
    // ## m2b-t01-interfaces
    cancel: AtomicUsize,
    reset: AtomicUsize,
    /// Bumped by `kill_connections`: a connection made before the bump is dead.
    killed: AtomicU64,
    // ## m3a-t01b-seams
    /// `Rows` events delivered by typed queries.
    chunks: AtomicU64,
    ping: AtomicUsize,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// ## m2b-t17-liveness
/// The statements that set a fake connection's `autocommit` flag: a prefix and the value.
type AutocommitEffects = Arc<Vec<(&'static str, bool)>>;

/// A scripted driver (F4). Build it, register an `Arc` clone, and read its counters.
pub struct FakeDriver {
    descriptor: DriverDescriptor,
    server: ServerInfo,
    result: Result<(), RawError>,
    script: Mutex<VecDeque<Result<(), RawError>>>,
    delay: Duration,
    /// `Some` for a gated driver: connects wait until it reads `true` (`release`).
    gate: Option<tokio::sync::watch::Sender<bool>>,
    pings: Vec<Result<Duration, RawError>>,
    tls: TlsSummary,
    databases: Vec<String>,
    counters: Arc<Counters>,
    last: Mutex<Option<FakeRequest>>,
    // ## m2b-t01-interfaces
    exec: Arc<ExecState>,
    requests: Mutex<Vec<FakeRequest>>,
    backend: Option<u64>,
    presented: Option<Vec<Vec<u8>>>,
    tls_outcome: Option<TlsVerified>,
    ignores_cancel: bool,
    reset_fails: bool,
    consumes_factory: bool,
    // ## m2b-t17-liveness
    /// `Some`: connections report `autocommit` (from `true`), and these statements set it.
    autocommit: Option<AutocommitEffects>,
    // ## m3a-t01b-seams
    tracks_tx: bool,
    typed_caps: Option<TypedCaps>,
}

impl FakeDriver {
    /// A driver for `kind` that connects at once to a server reporting `kind`'s product name,
    /// without TLS.
    pub fn new(kind: DbmsKind) -> Self {
        FakeDriver {
            descriptor: DriverDescriptor {
                kind,
                crate_name: "hacivat-driver-fake",
                caps: DriverCaps {
                    stream_injection: true,
                    in_protocol_tls: kind != DbmsKind::Sqlite,
                    cancel: CancelMechanism::None,
                    file_based: kind == DbmsKind::Sqlite,
                },
            },
            server: ServerInfo {
                kind,
                product: kind.display_name().to_string(),
                version: None,
                raw_version: String::new(),
                is_replica: Some(false),
            },
            result: Ok(()),
            script: Mutex::new(VecDeque::new()),
            delay: Duration::ZERO,
            gate: None,
            pings: Vec::new(),
            tls: TlsSummary {
                encrypted: false,
                version: None,
                cipher: None,
                verified: TlsVerified::No,
            },
            databases: Vec::new(),
            counters: Arc::default(),
            last: Mutex::new(None),
            exec: Arc::default(),
            requests: Mutex::new(Vec::new()),
            backend: None,
            presented: None,
            tls_outcome: None,
            ignores_cancel: false,
            reset_fails: false,
            consumes_factory: false,
            autocommit: None,
            tracks_tx: false,
            typed_caps: None,
        }
    }

    /// What a connection's `server_info` reports.
    pub fn server(mut self, info: ServerInfo) -> Self {
        self.server = info;
        self
    }

    /// The result of every connect without a scripted one.
    pub fn connect_result(mut self, result: Result<(), RawError>) -> Self {
        self.result = result;
        self
    }

    /// Results for the next connects, in order; then `connect_result` applies.
    pub fn script(self, results: Vec<Result<(), RawError>>) -> Self {
        *lock(&self.script) = results.into();
        self
    }

    /// How long each connect takes.
    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    /// Holds every connect (after it is counted and recorded, before its delay and result)
    /// until [`release`](Self::release): a test sees the connect in flight for as long as it
    /// needs, however slow the machine, instead of racing a delay.
    pub fn gated(mut self) -> Self {
        self.gate = Some(tokio::sync::watch::Sender::new(false));
        self
    }

    /// Lets every held connect (and every later one) go on. Nothing for an ungated driver.
    pub fn release(&self) {
        if let Some(gate) = &self.gate {
            gate.send_replace(true);
        }
    }

    /// Ping results per connection, in order; then 1 ms.
    pub fn pings(mut self, pings: Vec<Result<Duration, RawError>>) -> Self {
        self.pings = pings;
        self
    }

    pub fn tls(mut self, summary: TlsSummary) -> Self {
        self.tls = summary;
        self
    }

    pub fn databases(mut self, names: Vec<String>) -> Self {
        self.databases = names;
        self
    }

    pub fn connect_calls(&self) -> usize {
        self.counters.connect.load(Ordering::SeqCst)
    }

    pub fn close_calls(&self) -> usize {
        self.counters.close.load(Ordering::SeqCst)
    }

    /// Connections opened (or being opened) and not yet dropped: closed connections and
    /// abandoned connects (their future dropped) both count as gone (M2a Task 18).
    pub fn open_connections(&self) -> usize {
        self.counters.alive.load(Ordering::SeqCst)
    }

    pub fn last_request(&self) -> Option<FakeRequest> {
        lock(&self.last).clone()
    }

    // ## m2b-t01-interfaces

    /// The statement script (see the module docs).
    pub fn exec(self, script: Vec<FakeExec>) -> Self {
        *lock(&self.exec.script) = script.into_iter().map(Some).collect();
        self
    }

    /// What the connections' `backend_id` reports: `id` for the first connection, `id + 1` for
    /// the next, ... (m3a-t14 round 1: each connection its own, so a kill names one).
    pub fn backend(mut self, id: u64) -> Self {
        self.backend = Some(id);
        self
    }

    /// The chain (DER, leaf first) written into `TlsParams.presented` when a connect with TLS
    /// fails with a TLS error (M2b Task 14: or with `InsecureAuthRefused`, after its handshake).
    pub fn presented(mut self, der: Vec<Vec<u8>>) -> Self {
        self.presented = Some(der);
        self
    }

    /// The outcome written into `TlsParams.outcome` when a connect with TLS succeeds (M2b Task
    /// 14: or fails with `InsecureAuthRefused`).
    pub fn tls_outcome(mut self, v: TlsVerified) -> Self {
        self.tls_outcome = Some(v);
        self
    }

    /// The text of every `execute` and `batch`, in order.
    pub fn executed(&self) -> Vec<String> {
        lock(&self.exec.executed).clone()
    }

    /// A cancel no longer ends the running statement.
    pub fn ignores_cancel(mut self) -> Self {
        self.ignores_cancel = true;
        self
    }

    /// Every `reset` fails.
    pub fn reset_fails(mut self) -> Self {
        self.reset_fails = true;
        self
    }

    // ## m2b-t17-liveness
    /// Connections report `RawConn::autocommit` as MySQL's status flags would: on at connect,
    /// then set by every statement starting with one of `effects`' prefixes, whatever its
    /// result (a batch's `SET` applies even when a later statement fails).
    pub fn autocommit_flag(mut self, effects: Vec<(&'static str, bool)>) -> Self {
        self.autocommit = Some(Arc::new(effects));
        self
    }

    /// Each connect opens the network endpoint's stream (as the PostgreSQL driver does) and
    /// fails with its I/O error. The connection holds the stream: once it ends (an SSH channel
    /// whose tunnel died), every ping fails with `Io(ConnectionReset)`.
    pub fn consumes_factory(mut self) -> Self {
        self.consumes_factory = true;
        self
    }

    /// `first` for the next connects, then `then` for every other one.
    pub fn script_then(self, first: Vec<Result<(), RawError>>, then: Result<(), RawError>) -> Self {
        self.script(first).connect_result(then)
    }

    /// Every request, in order.
    pub fn requests(&self) -> Vec<FakeRequest> {
        lock(&self.requests).clone()
    }

    pub fn cancel_calls(&self) -> usize {
        self.counters.cancel.load(Ordering::SeqCst)
    }

    pub fn reset_calls(&self) -> usize {
        self.counters.reset.load(Ordering::SeqCst)
    }

    /// Every live connection fails from now on with `Io(ConnectionReset)`; later connections
    /// are fine.
    pub fn kill_connections(&self) {
        self.counters.killed.fetch_add(1, Ordering::SeqCst);
    }

    // ## m3a-t01b-seams

    /// The typed query script (see the module docs).
    pub fn queries(self, q: Vec<FakeQuery>) -> Self {
        *lock(&self.exec.queries) = q.into_iter().map(Some).collect();
        self
    }

    /// Every typed call (`query`, `fetch_more`, `close_query`), in order, refused ones too.
    pub fn typed_log(&self) -> Vec<FakeTypedCall> {
        lock(&self.exec.typed).clone()
    }

    /// Every statement, typed call and reset, in order.
    pub fn call_log(&self) -> Vec<FakeCall> {
        lock(&self.exec.calls).clone()
    }

    /// Connections set their transaction status from the leading keywords of `execute` and
    /// `batch` text (`BEGIN`/`START` → in a transaction; `COMMIT`/`END`/`ROLLBACK` → idle;
    /// `ROLLBACK TO` keeps it and recovers a failed one), unless a script entry's `tx_after`
    /// says otherwise; and on the PostgreSQL wire family a failed statement inside a
    /// transaction leaves it `Failed`.
    pub fn tracks_transactions(mut self) -> Self {
        self.tracks_tx = true;
        self
    }

    /// What every connection's `typed_caps` reports (else the engine's shape).
    pub fn with_typed_caps(mut self, caps: TypedCaps) -> Self {
        self.typed_caps = Some(caps);
        self
    }

    /// `Rows` events delivered by typed queries, over every connection.
    pub fn chunks_sent(&self) -> u64 {
        self.counters.chunks.load(Ordering::SeqCst)
    }

    /// Pings sent, over every connection.
    pub fn ping_calls(&self) -> usize {
        self.counters.ping.load(Ordering::SeqCst)
    }
}

impl Driver for FakeDriver {
    fn descriptor(&self) -> &DriverDescriptor {
        &self.descriptor
    }

    fn connect(
        &self,
        req: DriverConnect,
    ) -> BoxFuture<'static, Result<Box<dyn RawConn>, RawError>> {
        // Recorded before it is counted: a test that saw the count also sees the request.
        let recorded = FakeRequest::of(&req);
        lock(&self.requests).push(recorded.clone());
        *lock(&self.last) = Some(recorded);
        let index = self.counters.connect.fetch_add(1, Ordering::SeqCst) as u64;
        // ## m3a-t14-mysql-typed (review round 1): each connection its own backend ID; the
        // MySQL family always has one (the thread ID a kill names), never 0.
        let backend = self
            .backend
            .map(|first| first.saturating_add(index))
            .or_else(|| mysql_family(self.descriptor.kind).then(|| FIRST_THREAD_ID + index));
        let scripted = lock(&self.script).pop_front();
        let result = match scripted {
            Some(result) => result,
            None => self.result.clone(),
        };
        let mut conn = FakeConn {
            kind: self.descriptor.kind,
            server: self.server.clone(),
            pings: self.pings.clone().into(),
            tls: self.tls.clone(),
            databases: self.databases.clone(),
            user: req.user.clone(),
            counters: self.counters.clone(),
            exec: self.exec.clone(),
            tx: TxStatus::Idle,
            born: self.counters.killed.load(Ordering::SeqCst),
            cancel: Arc::default(),
            ignores_cancel: self.ignores_cancel,
            reset_fails: self.reset_fails,
            backend,
            stream: None,
            autocommit: self
                .autocommit
                .as_ref()
                .map(|effects| (true, effects.clone())),
            tracks_tx: self.tracks_tx,
            caps: self
                .typed_caps
                .unwrap_or_else(|| default_typed_caps(self.descriptor.kind)),
            typed: None,
            savepoints: Vec::new(),
            lost: false,
        };
        self.counters.alive.fetch_add(1, Ordering::SeqCst);
        let delay = self.delay;
        let gate = self
            .gate
            .as_ref()
            .map(tokio::sync::watch::Sender::subscribe);
        let with_tls = req.tls.is_some();
        let tls_slots = req
            .tls
            .as_ref()
            .map(|t| (t.presented.clone(), t.outcome.clone()));
        let (presented, outcome) = (self.presented.clone(), self.tls_outcome);
        let factory = match &req.endpoint {
            Endpoint::Network { factory, .. } if self.consumes_factory => Some(factory.clone()),
            _ => None,
        };
        let events = req.events.clone();
        drop(req);
        Box::pin(async move {
            if let Some(mut gate) = gate {
                // The driver (the sender) outlives its connects in tests; if it is gone, go on.
                let _ = gate.wait_for(|open| *open).await;
            }
            if let Some(factory) = factory {
                let stream = factory.open().await.map_err(|e| RawError {
                    kind: RawErrorKind::Io(e.kind()),
                    server: None,
                })?;
                conn.stream = Some(stream);
            }
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            // ## m2b-t14-pipeline: an insecure-auth refusal comes after a handshake that
            // completed (Q20), so its chain and outcome are recorded too.
            let refused_after_tls = matches!(
                &result,
                Err(RawError {
                    kind: RawErrorKind::InsecureAuthRefused,
                    ..
                })
            );
            if let (Err(e), Some((slot, _)), Some(der)) = (&result, &tls_slots, presented)
                && (matches!(e.kind, RawErrorKind::Tls(_)) || refused_after_tls)
            {
                *lock(slot) = Some(PresentedChain::from_der(der));
            }
            if let (true, Some((_, slot)), Some(v)) = (refused_after_tls, &tls_slots, outcome) {
                *lock(slot) = Some(v);
            }
            result?;
            if let (Some((_, slot)), Some(v)) = (&tls_slots, outcome) {
                *lock(slot) = Some(v);
            }
            if with_tls {
                events(DriverEvent::TlsStarted);
                events(DriverEvent::TlsDone);
            }
            events(DriverEvent::AuthStarted);
            events(DriverEvent::AuthDone);
            Ok(Box::new(conn) as Box<dyn RawConn>)
        })
    }
}

/// A connection of [`FakeDriver`].
struct FakeConn {
    kind: DbmsKind,
    server: ServerInfo,
    pings: VecDeque<Result<Duration, RawError>>,
    tls: TlsSummary,
    databases: Vec<String>,
    user: Option<String>,
    counters: Arc<Counters>,
    // ## m2b-t01-interfaces
    exec: Arc<ExecState>,
    tx: TxStatus,
    /// `Counters::killed` when the connection was made.
    born: u64,
    cancel: Arc<CancelLatch>,
    ignores_cancel: bool,
    reset_fails: bool,
    backend: Option<u64>,
    /// The endpoint's stream, held open (`consumes_factory`).
    stream: Option<BoxedStream>,
    // ## m2b-t17-liveness
    /// What `autocommit` reports, and the statements that change it (`autocommit_flag`).
    autocommit: Option<(bool, AutocommitEffects)>,
    // ## m3a-t01b-seams
    tracks_tx: bool,
    caps: TypedCaps,
    typed: Option<TypedRun>,
    /// The open savepoints, oldest first, when tracking transactions (round 2).
    savepoints: Vec<String>,
    // ## m3a-t14-mysql-typed (review round 1)
    /// An `AfterKill` ran out (`kill_ignored`): the connection is gone, as the real MySQL
    /// driver abandons it; every later call answers `Io(NotConnected)`.
    lost: bool,
}

impl FakeConn {
    // ## m2b-t17-liveness
    /// `Io(ConnectionReset)` once the held endpoint stream has ended (EOF or an error); a
    /// stream with nothing to read is alive. Polled once, without waiting.
    fn stream_alive(&mut self) -> Result<(), RawError> {
        use std::pin::Pin;
        use std::task::{Context, Poll, Waker};
        use tokio::io::ReadBuf;
        let Some(stream) = self.stream.as_mut() else {
            return Ok(());
        };
        let mut byte = [0u8; 1];
        let mut buf = ReadBuf::new(&mut byte);
        let mut cx = Context::from_waker(Waker::noop());
        match Pin::new(&mut **stream).poll_read(&mut cx, &mut buf) {
            Poll::Ready(Ok(())) if buf.filled().is_empty() => Err(RawError {
                kind: RawErrorKind::Io(std::io::ErrorKind::ConnectionReset),
                server: None,
            }),
            Poll::Ready(Err(e)) => Err(RawError {
                kind: RawErrorKind::Io(e.kind()),
                server: None,
            }),
            _ => Ok(()),
        }
    }

    /// `Io(ConnectionReset)` once `kill_connections` ran after this connection was made;
    /// `Io(NotConnected)` once it was lost (m3a-t14 round 1).
    fn alive(&self) -> Result<(), RawError> {
        if self.lost {
            return Err(RawError {
                kind: RawErrorKind::Io(std::io::ErrorKind::NotConnected),
                server: None,
            });
        }
        if self.counters.killed.load(Ordering::SeqCst) > self.born {
            return Err(RawError {
                kind: RawErrorKind::Io(std::io::ErrorKind::ConnectionReset),
                server: None,
            });
        }
        Ok(())
    }

    /// Runs one statement from the script; it counts as running from this call (M10).
    fn run<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<ExecSummary, RawError>> {
        // ## m3a-t01b-seams: one open query per connection, except a suspended PostgreSQL
        // query, beside which a statement runs in the same transaction (P5).
        if let Some(open) = &self.typed
            && !(pg_wire(self.kind) && open.suspended)
        {
            self.exec.log_statement(sql);
            return Box::pin(async { Err(unsupported()) });
        }
        // Fix round 1: PostgreSQL refuses every statement of a failed transaction but the ones
        // that end it (25P02); the statement reaches the server, the script entry stays.
        if pg_wire(self.kind) && self.tx == TxStatus::Failed && !ends_or_recovers_tx(sql) {
            self.exec.log_statement(sql);
            return Box::pin(async { Err(in_failed_transaction()) });
        }
        self.cancel.begin();
        // ## m2b-t17-liveness: the server applies the setting whatever the statement's (or
        // the batch's) outcome.
        if let Some((on, effects)) = self.autocommit.as_mut() {
            let text = sql.trim_start();
            if let Some((_, set)) = effects.iter().find(|(prefix, _)| text.starts_with(prefix)) {
                *on = *set;
            }
        }
        Box::pin(async move {
            let result = self.run_entry(sql).await;
            self.cancel.running.store(false, Ordering::SeqCst);
            // Round 2: savepoints, and the portal a `ROLLBACK TO` closes.
            let mut rolled_back_to = None;
            if self.tracks_tx && result.is_ok() {
                rolled_back_to = track_savepoints(&mut self.savepoints, sql);
            }
            if self.tx == TxStatus::Idle {
                self.savepoints.clear();
            }
            // Fix round 1: a PostgreSQL portal does not outlive its transaction; round 2: nor a
            // savepoint it was opened inside that is rolled back. Task 12 fix round 1 (review
            // I4b): the suspended run stays open, as the real driver keeps its query; only the
            // server's portal is gone.
            let tx = self.tx;
            if pg_wire(self.kind)
                && let Some(run) = self.typed.as_mut()
                && (tx == TxStatus::Idle
                    || rolled_back_to.is_some_and(|depth| run.savepoint_depth > depth))
            {
                run.gone = true;
            }
            result
        })
    }

    async fn run_entry(&mut self, sql: &str) -> Result<ExecSummary, RawError> {
        let entry = self.exec.take(sql);
        self.alive()?;
        let Some(entry) = entry else {
            // ## m3a-t01b-seams
            if self.tracks_tx {
                self.tx = tracked_tx(self.tx, sql);
            }
            return Ok(ExecSummary {
                tx: self.tx,
                ..ExecSummary::default()
            });
        };
        // ## m3a-t01b-seams: a tracked connection follows the statement when the entry sets
        // no status; a failed statement inside a PostgreSQL transaction fails it.
        let tracked = (self.tracks_tx && entry.tx_after.is_none()).then(|| {
            if entry.result.is_err() && pg_wire(self.kind) && self.tx == TxStatus::InTx {
                TxStatus::Failed
            } else {
                tracked_tx(self.tx, sql)
            }
        });
        let entry = FakeExec {
            tx_after: entry.tx_after.or(tracked),
            ..entry
        };
        assert!(
            !entry.panics,
            "FakeExec `{}` panics (scripted)",
            entry.prefix
        );
        if !entry.delay.is_zero() {
            let cancelled = self.cancel.notify.notified();
            tokio::pin!(cancelled);
            cancelled.as_mut().enable();
            let ignores = self.ignores_cancel;
            // A cancel that came before this first poll (M10).
            if !ignores && self.cancel.latched.swap(false, Ordering::SeqCst) {
                return Err(RawError {
                    kind: RawErrorKind::Cancelled,
                    server: None,
                });
            }
            tokio::select! {
                () = tokio::time::sleep(entry.delay) => {}
                () = &mut cancelled, if !ignores => {
                    return Err(RawError { kind: RawErrorKind::Cancelled, server: None });
                }
            }
            self.alive()?;
        }
        if let Some(tx) = entry.tx_after {
            self.tx = tx;
        }
        let tx = self.tx;
        entry.result.map(|summary| ExecSummary { tx, ..summary })
    }
}

// ## m2b-f3-internal-reads
/// A scripted answer as `opts` returns it: [`Protocol::Extended`] is the count-only path, so
/// its rows are counted and their values dropped, whatever the script says (a value read takes
/// `ExecOpts::values`).
fn count_only(summary: ExecSummary, opts: ExecOpts) -> ExecSummary {
    match opts.protocol {
        Protocol::Extended => ExecSummary {
            rows: Vec::new(),
            truncated: false,
            ..summary
        },
        Protocol::Simple => summary,
    }
}

/// A [`FakeConn`]'s cancel state (M10): whether a statement runs, and a cancel that reached it
/// before it was first polled.
#[derive(Default)]
struct CancelLatch {
    notify: Notify,
    running: AtomicBool,
    latched: AtomicBool,
}

impl CancelLatch {
    /// A statement starts: it runs, and no earlier cancel applies to it.
    fn begin(&self) {
        self.latched.store(false, Ordering::SeqCst);
        self.running.store(true, Ordering::SeqCst);
    }
}

/// Cancels the statement running on one [`FakeConn`].
struct FakeCancel {
    latch: Arc<CancelLatch>,
    counters: Arc<Counters>,
}

impl CancelHandle for FakeCancel {
    fn cancel(&self) -> BoxFuture<'static, Result<(), RawError>> {
        self.counters.cancel.fetch_add(1, Ordering::SeqCst);
        if self.latch.running.load(Ordering::SeqCst) {
            self.latch.latched.store(true, Ordering::SeqCst);
        }
        self.latch.notify.notify_waiters();
        Box::pin(async { Ok(()) })
    }
}

impl Drop for FakeConn {
    fn drop(&mut self) {
        self.counters.alive.fetch_sub(1, Ordering::SeqCst);
    }
}

impl RawConn for FakeConn {
    fn kind(&self) -> DbmsKind {
        self.kind
    }

    fn server_info(&mut self) -> BoxFuture<'_, Result<ServerInfo, RawError>> {
        let info = self.server.clone();
        Box::pin(async move { Ok(info) })
    }

    fn ping(&mut self) -> BoxFuture<'_, Result<Duration, RawError>> {
        // ## m3a-t01b-seams
        self.counters.ping.fetch_add(1, Ordering::SeqCst);
        let next = self
            .alive()
            .and_then(|()| self.stream_alive())
            .and_then(|()| {
                self.pings
                    .pop_front()
                    .unwrap_or(Ok(Duration::from_millis(1)))
            });
        Box::pin(async move { next })
    }

    fn tls_summary(&mut self) -> BoxFuture<'_, TlsSummary> {
        let tls = self.tls.clone();
        Box::pin(async move { tls })
    }

    fn probe(&mut self) -> BoxFuture<'_, Vec<ProbeWarning>> {
        Box::pin(async { Vec::new() })
    }

    fn current_user(&mut self) -> BoxFuture<'_, Result<Option<String>, RawError>> {
        let user = self.user.clone();
        Box::pin(async move { Ok(user) })
    }

    fn list_databases(&mut self, limit: usize) -> BoxFuture<'_, Result<Vec<String>, RawError>> {
        let names = self.databases.iter().take(limit).cloned().collect();
        Box::pin(async move { Ok(names) })
    }

    fn close(self: Box<Self>) -> BoxFuture<'static, ()> {
        self.counters.close.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {})
    }

    fn cancel_handle(&self) -> Option<Arc<dyn CancelHandle>> {
        Some(Arc::new(FakeCancel {
            latch: self.cancel.clone(),
            counters: self.counters.clone(),
        }))
    }

    // ## m2b-t01-interfaces

    fn execute<'a>(
        &'a mut self,
        sql: &'a str,
        opts: ExecOpts,
    ) -> BoxFuture<'a, Result<ExecSummary, RawError>> {
        let run = self.run(sql);
        // ## m2b-f3-internal-reads: the count-only path keeps no values, as the PostgreSQL
        // and MySQL drivers run it, so a script can never answer a value read sent there.
        Box::pin(async move { run.await.map(|summary| count_only(summary, opts)) })
    }

    fn batch<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), RawError>> {
        let run = self.run(sql);
        Box::pin(async move { run.await.map(|_| ()) })
    }

    fn tx_status(&self) -> TxStatus {
        self.tx
    }

    fn reset(&mut self) -> BoxFuture<'_, Result<(), RawError>> {
        self.counters.reset.fetch_add(1, Ordering::SeqCst);
        // ## m3a-t01b-seams
        lock(&self.exec.calls).push(FakeCall::Reset);
        let result = match self.alive() {
            Ok(()) if self.reset_fails => Err(RawError {
                kind: RawErrorKind::Other,
                server: None,
            }),
            alive => alive,
        };
        if result.is_ok() {
            self.tx = TxStatus::Idle;
        }
        Box::pin(async move { result })
    }

    fn backend_id(&self) -> Option<u64> {
        self.backend
    }

    // ## m2b-t17-liveness
    fn autocommit(&self) -> Option<bool> {
        self.autocommit.as_ref().map(|(on, _)| *on)
    }

    // ## m3a-t01b-seams

    fn typed_caps(&self) -> TypedCaps {
        self.caps
    }

    fn query<'a>(&'a mut self, spec: QuerySpec<'a>) -> BoxFuture<'a, Result<(), RawError>> {
        self.exec
            .log_typed(FakeTypedCall::Query(spec.sql.to_string()));
        let started = self.start_query(&spec);
        Box::pin(async move { started })
    }

    fn next_event(&mut self) -> BoxFuture<'_, Result<QueryEvent, RawError>> {
        Box::pin(self.next_typed_event())
    }

    fn fetch_more(&mut self, rows: u32) -> BoxFuture<'_, Result<(), RawError>> {
        self.exec.log_typed(FakeTypedCall::FetchMore(rows));
        let resumed = self.resume_typed(rows);
        Box::pin(async move { resumed })
    }

    fn close_query(&mut self, how: CloseHow) -> BoxFuture<'_, Result<Closed, RawError>> {
        self.exec.log_typed(FakeTypedCall::Close(how));
        let closed = self.close_typed(how);
        Box::pin(async move { closed })
    }
}

// ## m3a-t01b-seams
/// What the fake answers a call its contract refuses.
fn unsupported() -> RawError {
    RawError {
        kind: RawErrorKind::Unsupported,
        server: None,
    }
}

/// The most rows a drain reads before it gives up and closes (one PostgreSQL `Execute`).
const DRAIN_ROWS: u64 = 4_096;

/// Follows `sql`'s savepoint statements on `stack` (oldest first; names compared without case):
/// `SAVEPOINT s` pushes, `RELEASE [SAVEPOINT] s` pops `s` and every later one, `ROLLBACK [WORK |
/// TRANSACTION] TO [SAVEPOINT] s` pops every later one and keeps `s`, a transaction's start or
/// end clears it. Returns the index of the savepoint the last `ROLLBACK TO` rolled back to:
/// portals opened with more savepoints open than that index are gone.
fn track_savepoints(stack: &mut Vec<String>, sql: &str) -> Option<usize> {
    let mut rolled_back_to = None;
    for statement in sql.split(';') {
        let words: Vec<String> = statement
            .split(|c: char| c.is_whitespace())
            .filter(|w| !w.is_empty())
            .map(|w| w.trim_matches('"').to_ascii_lowercase())
            .collect();
        let word = |i: usize| words.get(i).map(String::as_str).unwrap_or_default();
        let find = |stack: &[String], name: &str| stack.iter().rposition(|n| n == name);
        match word(0) {
            "savepoint" if !word(1).is_empty() => stack.push(word(1).to_string()),
            "release" => {
                let name = if word(1) == "savepoint" {
                    word(2)
                } else {
                    word(1)
                };
                if let Some(i) = find(stack, name) {
                    stack.truncate(i);
                }
            }
            "rollback" | "abort" => {
                let mut i = 1;
                if matches!(word(i), "work" | "transaction") {
                    i += 1;
                }
                if word(i) == "to" {
                    i += 1;
                    if word(i) == "savepoint" {
                        i += 1;
                    }
                    if let Some(at) = find(stack, word(i)) {
                        stack.truncate(at + 1);
                        rolled_back_to = Some(at);
                    }
                } else {
                    stack.clear();
                }
            }
            "begin" | "start" | "commit" | "end" => stack.clear(),
            _ => {}
        }
    }
    rolled_back_to
}

/// Whether `sql` ends the transaction or rolls back to a savepoint: what PostgreSQL still
/// accepts in a failed transaction (`COMMIT` there rolls back).
fn ends_or_recovers_tx(sql: &str) -> bool {
    let first = sql
        .split(|c: char| !c.is_ascii_alphabetic())
        .find(|w| !w.is_empty())
        .unwrap_or_default();
    ["ROLLBACK", "ABORT", "COMMIT", "END"]
        .iter()
        .any(|k| first.eq_ignore_ascii_case(k))
}

/// A PostgreSQL server error with `sqlstate`.
fn pg_error(sqlstate: &str, text: &str) -> RawError {
    RawError {
        kind: RawErrorKind::Db,
        server: Some(crate::errors::ServerMessage {
            sqlstate: Some(sqlstate.to_string()),
            vendor_code: None,
            severity: Some("ERROR".to_string()),
            text: text.to_string(),
            detail: None,
            hint: None,
            lang_hint: None,
        }),
    }
}

/// PostgreSQL's answer to a command in a failed transaction.
fn in_failed_transaction() -> RawError {
    pg_error(
        "25P02",
        "current transaction is aborted, commands ignored until end of transaction block",
    )
}

/// PostgreSQL's answer to an Execute on a portal its transaction's end destroyed.
fn portal_does_not_exist() -> RawError {
    pg_error("34000", "portal \"\" does not exist")
}

fn cancelled_error() -> RawError {
    RawError {
        kind: RawErrorKind::Cancelled,
        server: None,
    }
}

/// Waits `delay` unless a cancel that applies (latched before, or sent during the wait) ends
/// the query first (`Err(Cancelled)`, M10).
async fn wait_unless_cancelled(
    latch: &CancelLatch,
    delay: Duration,
    ignores: bool,
) -> Result<(), RawError> {
    let cancelled = latch.notify.notified();
    tokio::pin!(cancelled);
    cancelled.as_mut().enable();
    if !ignores && latch.latched.swap(false, Ordering::SeqCst) {
        return Err(cancelled_error());
    }
    if delay.is_zero() {
        return Ok(());
    }
    tokio::select! {
        () = tokio::time::sleep(delay) => Ok(()),
        () = &mut cancelled, if !ignores => Err(cancelled_error()),
    }
}

impl FakeConn {
    /// `query`'s checks, then the open run (the module docs).
    fn start_query(&mut self, spec: &QuerySpec<'_>) -> Result<(), RawError> {
        self.alive()?;
        if self.typed.is_some() {
            return Err(unsupported());
        }
        // Fix round 1: M3a's drivers speak the extended protocol only (`typed.rs`).
        if spec.protocol == Protocol::Simple {
            return Err(unsupported());
        }
        if pg_wire(self.kind) && self.tx == TxStatus::Failed {
            return Err(in_failed_transaction());
        }
        // P5: a portal outside a transaction block would end at its Sync.
        if spec.suspendable && pg_wire(self.kind) && self.tx == TxStatus::Idle {
            return Err(unsupported());
        }
        let script = self.exec.take_query(spec.sql).unwrap_or_default();
        let mut run = TypedRun::new(spec, script, self.caps.suspend);
        run.savepoint_depth = self.savepoints.len();
        self.typed = Some(run);
        self.cancel.begin();
        Ok(())
    }

    /// Ends the open run: the query no longer runs on the server.
    fn end_typed(&mut self) {
        self.typed = None;
        self.cancel.running.store(false, Ordering::SeqCst);
    }

    /// Ends the open run with `err`; inside a PostgreSQL transaction the server fails it.
    fn fail_typed(&mut self, err: RawError) -> RawError {
        self.end_typed();
        if pg_wire(self.kind) && self.tx == TxStatus::InTx {
            self.tx = TxStatus::Failed;
        }
        err
    }

    /// `fetch_more`'s effect: a suspended run resumes for `rows` more rows.
    fn resume_typed(&mut self, rows: u32) -> Result<(), RawError> {
        self.alive()?;
        // ## m3a-t12-pg-typed (fix round 1, review I4b): the Execute on a portal its transaction
        // destroyed is the server's 34000, once; inside a transaction block (a `ROLLBACK TO`
        // case) that error fails the transaction; nothing is open afterwards.
        if self.typed.as_ref().is_some_and(|run| run.gone) {
            return Err(self.fail_typed(portal_does_not_exist()));
        }
        match self.typed.as_mut() {
            Some(run) if run.suspended => {
                run.suspended = false;
                // Review I4e: 0 asks for a chunk's worth, as the PostgreSQL driver does.
                run.budget = Some(match rows {
                    0 => run.chunk_rows,
                    n => u64::from(n),
                });
            }
            _ => return Err(unsupported()),
        }
        self.cancel.begin();
        Ok(())
    }

    async fn next_typed_event(&mut self) -> Result<QueryEvent, RawError> {
        let (ignores, never_pending, delay) = match self.typed.as_ref() {
            None => return Err(unsupported()),
            // Also a run whose portal is gone: no I/O until `fetch_more` or `close_query`.
            Some(run) if run.suspended => return Err(unsupported()),
            Some(run) => {
                let delay = if !run.started {
                    run.script.first_delay
                } else if run.current.is_some() && run.chunks > 0 {
                    run.script.chunk_delay
                } else {
                    Duration::ZERO
                };
                (
                    self.ignores_cancel || run.script.ignores_cancel,
                    run.script.never_pending,
                    delay,
                )
            }
        };
        if let Err(e) = self.alive() {
            return Err(self.fail_typed(e));
        }
        let cancel = self.cancel.clone();
        if !never_pending {
            tokio::task::yield_now().await;
            if let Err(e) = wait_unless_cancelled(&cancel, delay, ignores).await {
                return Err(self.fail_typed(e));
            }
            if let Err(e) = self.alive() {
                return Err(self.fail_typed(e));
            }
        } else if !ignores && cancel.latched.swap(false, Ordering::SeqCst) {
            return Err(self.fail_typed(cancelled_error()));
        }
        self.step_typed()
    }

    /// The open run's next event, without waiting.
    fn step_typed(&mut self) -> Result<QueryEvent, RawError> {
        let pg = pg_wire(self.kind);
        let Some(run) = self.typed.as_mut() else {
            return Err(unsupported());
        };
        run.started = true;
        let Some(open) = run.current.as_mut() else {
            if let Some(set) = run.sets.pop_front() {
                let schema = set.schema();
                let width = set.columns.len();
                let total = set.rows.len();
                run.current = Some(OpenSet {
                    set,
                    width,
                    total,
                    pos: 0,
                });
                return Ok(QueryEvent::ResultStart(schema));
            }
            let skipped = run.skipped;
            if let Some(tx) = run.script.tx_after {
                self.tx = tx;
            }
            self.end_typed();
            return Ok(QueryEvent::Done(QueryDone {
                tx: self.tx,
                warnings: 0,
                skipped_result_sets: skipped,
            }));
        };
        // ## m3a-t12-pg-typed (fix round 1, review I4a): a PostgreSQL Execute that returned
        // exactly its limit ends in `PortalSuspended`, even when no row is left; the next
        // `fetch_more` then ends the result with no `Rows`.
        if pg && run.budget == Some(0) {
            run.suspended = true;
            self.cancel.running.store(false, Ordering::SeqCst);
            return Ok(QueryEvent::Suspended);
        }
        if open.pos == open.total {
            let end = ResultEnd {
                rows: open.total,
                kept: open.total,
                rows_affected: run.script.rows_affected,
                command_tag: None,
            };
            run.current = None;
            return Ok(QueryEvent::ResultEnd(end));
        }
        if let Some((n, err)) = &run.script.fail_after
            && run.sent >= *n
        {
            let err = err.clone();
            return Err(self.fail_typed(err));
        }
        if run.budget == Some(0) {
            run.suspended = true;
            self.cancel.running.store(false, Ordering::SeqCst);
            return Ok(QueryEvent::Suspended);
        }
        let mut size = if run.chunks == 0 {
            run.first_rows
        } else {
            run.chunk_rows
        };
        size = size.min(open.total - open.pos);
        if let Some(budget) = run.budget {
            size = size.min(budget);
        }
        if let Some((n, _)) = &run.script.fail_after {
            size = size.min(n - run.sent);
        }
        let schema = open.set.schema();
        let rows = open.set.rows.rows(open.pos, open.pos + size, open.width);
        open.pos += size;
        run.sent += size;
        run.chunks += 1;
        if let Some(budget) = run.budget.as_mut() {
            *budget -= size;
        }
        self.counters.chunks.fetch_add(1, Ordering::SeqCst);
        Ok(QueryEvent::Rows(Chunk::from_values(&schema, rows)))
    }

    /// `close_query`'s effect (the module docs).
    fn close_typed(&mut self, how: CloseHow) -> Result<Closed, RawError> {
        self.alive()?;
        let Some(run) = self.typed.as_ref() else {
            return Ok(Closed::Closed { rows: 0 });
        };
        let sent = run.sent;
        // ## m3a-t12-pg-typed (fix round 1, review I4b): closing a portal its transaction
        // destroyed is no error (PostgreSQL's Close of a missing portal); a drain's Execute meets
        // the server's 34000, which fails a transaction block.
        if run.gone {
            if matches!(how, CloseHow::Drain(_)) {
                let _ = self.fail_typed(portal_does_not_exist());
            } else {
                self.end_typed();
            }
            return Ok(Closed::Closed { rows: sent });
        }
        // Review I4d: a PostgreSQL `Execute(0)` (a run that cannot suspend) cannot be cut on the
        // wire: its rest is read to the end, and the answer is the server's outcome, the
        // completed result or the error it meets (a scripted failure, a latched cancel), which
        // fails a transaction block.
        if pg_wire(self.kind) && run.budget.is_none() {
            let total = sent.saturating_add(run.remaining());
            let ignores = self.ignores_cancel || run.script.ignores_cancel;
            let failure = match &run.script.fail_after {
                Some((n, err)) if *n < total => Some(err.clone()),
                _ if !ignores && self.cancel.latched.swap(false, Ordering::SeqCst) => {
                    Some(cancelled_error())
                }
                _ => None,
            };
            if let Some(err) = failure {
                return Err(self.fail_typed(err));
            }
            if let Some(tx) = run.script.tx_after {
                self.tx = tx;
            }
            self.end_typed();
            return Ok(Closed::Completed { rows: total });
        }
        // ## m3a-t14-mysql-typed (review round 1, named exception): the MySQL family closes as
        // the real driver does (the module docs).
        if self.caps.close_needs_kill && !pg_wire(self.kind) {
            return self.close_by_kill(how);
        }
        let kill = Closed::NeedsServerKill {
            thread_id: self.backend.unwrap_or(FIRST_THREAD_ID),
        };
        let closed = match how {
            CloseHow::Drain(_) if run.script.kill_needed => return Ok(kill),
            CloseHow::Drain(_) => match run.remaining() {
                left if left <= DRAIN_ROWS => Closed::Completed { rows: sent + left },
                // Fix round 1: an engine that closes early by a kill (MySQL) asks for one.
                _ if self.caps.close_needs_kill => return Ok(kill),
                _ => Closed::Closed { rows: sent },
            },
            CloseHow::Now | CloseHow::AfterKill(_) => Closed::Closed { rows: sent },
        };
        self.end_typed();
        Ok(closed)
    }

    // ## m3a-t14-mysql-typed (review round 1, named exception)
    /// `close_query` on the MySQL family, as `hacivat-driver-mysql` answers it (the module
    /// docs): a kill for anything unread, Task 12's outcome rule for a rest read to its end.
    fn close_by_kill(&mut self, how: CloseHow) -> Result<Closed, RawError> {
        let Some(run) = self.typed.as_ref() else {
            return Ok(Closed::Closed { rows: 0 });
        };
        let sent = run.sent;
        let left = run.remaining();
        let total = sent.saturating_add(left);
        let (kill_needed, kill_ignored, tx_after) = (
            run.script.kill_needed,
            run.script.kill_ignored,
            run.script.tx_after,
        );
        // The scripted failure the rest meets, if it is read.
        let failure = run
            .script
            .fail_after
            .as_ref()
            .filter(|(n, _)| *n < total)
            .map(|(_, e)| e.clone());
        let kill = Closed::NeedsServerKill {
            thread_id: self.backend.unwrap_or(FIRST_THREAD_ID),
        };
        match how {
            // Only a kill ends a MySQL result early; the query stays open for `AfterKill`.
            CloseHow::Now if left > 0 => Ok(kill),
            // The stand-in for a drain that runs out of time.
            CloseHow::Drain(_) if kill_needed || left > DRAIN_ROWS => Ok(kill),
            CloseHow::AfterKill(_) if kill_ignored => {
                self.end_typed();
                self.lost = true;
                Err(RawError {
                    kind: RawErrorKind::Io(std::io::ErrorKind::TimedOut),
                    server: None,
                })
            }
            // The kill's 1317 ended what was left.
            CloseHow::AfterKill(_) if left > 0 => {
                self.end_typed();
                Ok(Closed::Closed { rows: sent })
            }
            // Read to the end: nothing unread (`Now`, `AfterKill`: the kill came too late), or
            // a short rest drained.
            _ => {
                self.end_typed();
                match failure {
                    Some(err) if is_kill(&err) => Ok(Closed::Closed { rows: sent }),
                    Some(err) => Err(err),
                    None => {
                        if let Some(tx) = tx_after {
                            self.tx = tx;
                        }
                        Ok(Closed::Completed { rows: total })
                    }
                }
            }
        }
    }
}

// ## m3a-t14-mysql-typed (review round 1)
/// Whether `e` is MySQL's `ER_QUERY_INTERRUPTED` (1317): a `KILL QUERY` ended the statement.
fn is_kill(e: &RawError) -> bool {
    e.server.as_ref().and_then(|s| s.vendor_code) == Some(1317)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::RawErrorKind;
    use secrecy::SecretString;

    fn request(password: Option<&str>, events: crate::driver::DriverEvents) -> DriverConnect {
        DriverConnect {
            endpoint: Endpoint::File {
                path: PathBuf::from("shop.db"),
                create_if_missing: true,
                read_only: false,
            },
            tls: None,
            user: Some("deploy".into()),
            password: password.map(SecretString::from),
            database: Some("shop".into()),
            application_name: "Hacivat".into(),
            read_only: false,
            options: vec![("k".into(), "v".into())],
            connect_timeout: Duration::from_secs(15),
            events,
            insecure_auth: InsecureAuth::default(),
        }
    }

    #[tokio::test]
    async fn scripted_results_then_the_default_and_counters() {
        let refused = RawError {
            kind: RawErrorKind::Io(std::io::ErrorKind::ConnectionRefused),
            server: None,
        };
        let fake = Arc::new(
            FakeDriver::new(DbmsKind::Postgres)
                .script(vec![Err(refused.clone())])
                .databases(vec!["a".into(), "b".into(), "c".into()])
                .pings(vec![Ok(Duration::from_millis(7))]),
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let events: crate::driver::DriverEvents = Arc::new(move |e| lock(&sink).push(e));

        let first = fake
            .connect(request(Some("hv_test_pw"), events.clone()))
            .await;
        assert_eq!(first.err(), Some(refused));
        assert!(lock(&seen).is_empty());

        let mut conn = fake
            .connect(request(None, events))
            .await
            .expect("the default result is Ok");
        assert_eq!(
            *lock(&seen),
            [DriverEvent::AuthStarted, DriverEvent::AuthDone]
        );
        assert_eq!(conn.kind(), DbmsKind::Postgres);
        assert_eq!(conn.server_info().await.unwrap().product, "PostgreSQL");
        assert_eq!(conn.ping().await.unwrap(), Duration::from_millis(7));
        assert_eq!(conn.ping().await.unwrap(), Duration::from_millis(1));
        assert_eq!(conn.list_databases(2).await.unwrap(), ["a", "b"]);
        assert_eq!(
            conn.current_user().await.unwrap().as_deref(),
            Some("deploy")
        );
        assert!(!conn.tls_summary().await.encrypted);
        conn.close().await;
        assert_eq!((fake.connect_calls(), fake.close_calls()), (2, 1));
    }

    #[tokio::test]
    async fn a_gated_connect_waits_for_the_release() {
        use futures::FutureExt as _;
        let fake = FakeDriver::new(DbmsKind::Postgres).gated();
        let mut held = fake.connect(request(None, Arc::new(|_| {})));
        assert_eq!(fake.connect_calls(), 1, "counted before the gate");
        for _ in 0..10 {
            assert!((&mut held).now_or_never().is_none(), "held at the gate");
            tokio::task::yield_now().await;
        }
        fake.release();
        assert!(held.await.is_ok());
        // Released for good: a later connect goes straight through.
        assert!(fake.connect(request(None, Arc::new(|_| {}))).await.is_ok());
        assert_eq!(fake.open_connections(), 0, "both connections dropped");
    }

    #[tokio::test]
    async fn the_last_request_keeps_the_password_for_asserts_but_never_prints_it() {
        let fake = FakeDriver::new(DbmsKind::Sqlite);
        let _ = fake
            .connect(request(Some("s3cret"), Arc::new(|_| {})))
            .await;
        let last = fake.last_request().expect("a request was recorded");
        assert_eq!(last.password.as_deref(), Some("s3cret"));
        assert_eq!(last.database.as_deref(), Some("shop"));
        assert_eq!(
            last.endpoint,
            Some(FakeEndpoint::File {
                path: PathBuf::from("shop.db"),
                create_if_missing: true,
                read_only: false
            })
        );
        let printed = format!("{last:?}");
        assert!(!printed.contains("s3cret"), "{printed}");
        assert!(printed.contains("password: <redacted>"), "{printed}");
    }

    // ## m2b-t01-interfaces

    fn slow(prefix: &'static str, delay: Duration) -> FakeExec {
        FakeExec {
            prefix,
            result: Ok(ExecSummary::default()),
            delay,
            tx_after: None,
            panics: false,
        }
    }

    #[tokio::test]
    async fn a_cancel_ends_the_running_statement_unless_ignored() {
        // An hour: only the cancel can end it within the test.
        let fake = FakeDriver::new(DbmsKind::Postgres)
            .exec(vec![slow("SELECT pg_sleep", Duration::from_secs(3600))])
            .backend(4242);
        let mut conn = fake.connect(request(None, Arc::new(|_| {}))).await.unwrap();
        assert_eq!(conn.backend_id(), Some(4242));
        let cancel = conn.cancel_handle().expect("a cancel handle");
        let (result, ()) = tokio::join!(
            conn.execute("SELECT pg_sleep(60)", ExecOpts::default()),
            async {
                tokio::task::yield_now().await;
                cancel.cancel().await.unwrap();
            }
        );
        assert_eq!(result.unwrap_err().kind, RawErrorKind::Cancelled);
        assert_eq!(fake.cancel_calls(), 1);

        let stubborn = FakeDriver::new(DbmsKind::Postgres)
            .exec(vec![slow("SELECT", Duration::from_millis(20))])
            .ignores_cancel();
        let mut conn = stubborn
            .connect(request(None, Arc::new(|_| {})))
            .await
            .unwrap();
        let cancel = conn.cancel_handle().unwrap();
        let (result, ()) = tokio::join!(conn.execute("SELECT 1", ExecOpts::default()), async {
            tokio::task::yield_now().await;
            cancel.cancel().await.unwrap();
        });
        assert!(result.is_ok(), "the statement ran to its end");
    }

    #[tokio::test]
    async fn killed_connections_fail_and_reset_is_counted() {
        let fake = FakeDriver::new(DbmsKind::MySql).exec(vec![FakeExec {
            prefix: "START TRANSACTION",
            result: Ok(ExecSummary::default()),
            delay: Duration::ZERO,
            tx_after: Some(TxStatus::InTx),
            panics: false,
        }]);
        let mut conn = fake.connect(request(None, Arc::new(|_| {}))).await.unwrap();
        conn.batch("START TRANSACTION READ ONLY").await.unwrap();
        assert_eq!(conn.tx_status(), TxStatus::InTx);
        conn.reset().await.unwrap();
        assert_eq!((conn.tx_status(), fake.reset_calls()), (TxStatus::Idle, 1));

        fake.kill_connections();
        let reset = RawErrorKind::Io(std::io::ErrorKind::ConnectionReset);
        assert_eq!(
            conn.execute("SELECT 1", ExecOpts::default())
                .await
                .unwrap_err()
                .kind,
            reset
        );
        assert_eq!(conn.ping().await.unwrap_err().kind, reset);
        let mut fresh = fake.connect(request(None, Arc::new(|_| {}))).await.unwrap();
        assert!(fresh.execute("SELECT 1", ExecOpts::default()).await.is_ok());
        assert_eq!(
            fake.executed(),
            ["START TRANSACTION READ ONLY", "SELECT 1", "SELECT 1"]
        );
        assert_eq!(fake.requests().len(), 2);
        assert_eq!(
            fake.requests()[0].insecure_auth,
            InsecureAuth::RefuseUnlessVerified
        );

        let failing = FakeDriver::new(DbmsKind::MySql).reset_fails();
        let mut conn = failing
            .connect(request(None, Arc::new(|_| {})))
            .await
            .unwrap();
        assert!(conn.reset().await.is_err());
    }

    // ## m2b-t17-liveness
    #[tokio::test]
    async fn a_held_stream_that_ends_fails_the_ping() {
        let far = Arc::new(Mutex::new(None));
        let keep = far.clone();
        let factory = hacivat_net::StreamFactory::new(move || {
            let keep = keep.clone();
            Box::pin(async move {
                let (near, far) = tokio::io::duplex(64);
                *lock(&keep) = Some(far);
                Ok(Box::new(near) as BoxedStream)
            })
        });
        let fake = FakeDriver::new(DbmsKind::Postgres).consumes_factory();
        let mut req = request(None, Arc::new(|_| {}));
        req.endpoint = Endpoint::Network {
            factory,
            direct: None,
            resolved: Vec::new(),
            tls_host: "db".into(),
        };
        let mut conn = fake.connect(req).await.unwrap();
        assert!(conn.ping().await.is_ok(), "an open, quiet stream is alive");
        drop(lock(&far).take());
        assert_eq!(
            conn.ping().await.unwrap_err().kind,
            RawErrorKind::Io(std::io::ErrorKind::ConnectionReset)
        );
    }

    #[tokio::test]
    async fn a_factory_consumer_fails_with_the_streams_error() {
        let refused = hacivat_net::StreamFactory::new(|| {
            Box::pin(async { Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused)) })
        });
        let fake = FakeDriver::new(DbmsKind::Postgres).consumes_factory();
        let mut req = request(None, Arc::new(|_| {}));
        req.endpoint = Endpoint::Network {
            factory: refused,
            direct: None,
            resolved: Vec::new(),
            tls_host: "db".into(),
        };
        let err = fake
            .connect(req)
            .await
            .err()
            .expect("the stream was refused");
        assert_eq!(
            err.kind,
            RawErrorKind::Io(std::io::ErrorKind::ConnectionRefused)
        );
    }

    // ## m2b-t07-session-actor

    /// T1 review M10: a cancel sent after `execute` but before its first poll still ends the
    /// statement; a cancel sent while nothing runs never reaches the next statement.
    #[tokio::test]
    async fn an_early_cancel_is_latched_and_an_idle_cancel_is_a_no_op() {
        let fake = FakeDriver::new(DbmsKind::Postgres).exec(vec![
            slow("SELECT pg_sleep", Duration::from_secs(3600)),
            // Short: it runs to its end unless the idle cancel wrongly reaches it.
            slow("SELECT later", Duration::from_millis(20)),
        ]);
        let mut conn = fake.connect(request(None, Arc::new(|_| {}))).await.unwrap();
        let cancel = conn.cancel_handle().unwrap();
        let early = conn.execute("SELECT pg_sleep(60)", ExecOpts::default());
        cancel.cancel().await.unwrap();
        assert_eq!(early.await.unwrap_err().kind, RawErrorKind::Cancelled);

        // Nothing runs now: this cancel is lost, as on a real server.
        cancel.cancel().await.unwrap();
        assert!(
            conn.execute("SELECT later", ExecOpts::default())
                .await
                .is_ok(),
            "the idle cancel did not reach the next statement"
        );
        assert_eq!(fake.cancel_calls(), 2);
    }

    // ## m2b-f3-internal-reads

    /// Finding F3: a scripted answer reaches a value read only through the values path; the
    /// count-only path counts its rows and drops their values, as the real drivers do, so a
    /// probe sent there cannot pass over the fake and fail on a server.
    #[tokio::test]
    async fn the_count_only_path_keeps_no_values() {
        let answer = || FakeExec {
            prefix: "SELECT current_user = session_user",
            result: Ok(ExecSummary {
                rows: vec![vec![Some("t".into())]],
                row_count: 1,
                ..ExecSummary::default()
            }),
            delay: Duration::ZERO,
            tx_after: None,
            panics: false,
        };
        for kind in [DbmsKind::Postgres, DbmsKind::MySql, DbmsKind::Sqlite] {
            let fake = FakeDriver::new(kind).exec(vec![answer(), answer()]);
            let mut conn = fake.connect(request(None, Arc::new(|_| {}))).await.unwrap();
            let counted = conn
                .execute("SELECT current_user = session_user", ExecOpts::default())
                .await
                .unwrap();
            assert_eq!((counted.row_count, counted.rows.len()), (1, 0), "{kind:?}");
            let read = conn
                .execute("SELECT current_user = session_user", ExecOpts::values(1))
                .await
                .unwrap();
            assert_eq!(read.rows, [vec![Some("t".to_string())]], "{kind:?}");
        }
    }
}

// ## m3a-t12-pg-typed (fix round 1): the fake follows the PostgreSQL driver (review I4a-e).
#[cfg(test)]
mod pg_typed_tests {
    use super::*;

    fn request() -> DriverConnect {
        DriverConnect {
            endpoint: Endpoint::File {
                path: PathBuf::from("t12.db"),
                create_if_missing: false,
                read_only: false,
            },
            tls: None,
            user: Some("hv_t12".into()),
            password: None,
            database: None,
            application_name: "Hacivat".into(),
            read_only: false,
            options: vec![],
            connect_timeout: Duration::from_secs(15),
            events: Arc::new(|_| {}),
            insecure_auth: InsecureAuth::default(),
        }
    }

    fn id_cell(row: u64, _: usize) -> Value {
        Value::Int(row as i64)
    }

    fn select(count: u64) -> FakeQuery {
        FakeQuery {
            prefix: "SELECT",
            sets: vec![FakeResultSet {
                columns: vec![(
                    "id",
                    LogicalType::Int {
                        bits: 64,
                        signed: true,
                    },
                )],
                rows: FakeRows::Generated {
                    count,
                    cell: id_cell,
                },
            }],
            ..FakeQuery::default()
        }
    }

    fn spec(sql: &str, suspendable: bool) -> QuerySpec<'_> {
        QuerySpec {
            sql,
            binds: &[],
            protocol: Protocol::Extended,
            suspendable,
            first_rows: 501,
            chunk_rows: 100,
            chunk_bytes: 4 << 20,
            cell_cap: 1 << 20,
            max_result_sets: 1,
        }
    }

    async fn pg(queries: Vec<FakeQuery>) -> Box<dyn RawConn> {
        FakeDriver::new(DbmsKind::Postgres)
            .tracks_transactions()
            .queries(queries)
            .connect(request())
            .await
            .unwrap()
    }

    fn rows_of(e: &QueryEvent) -> u32 {
        match e {
            QueryEvent::Rows(c) => c.rows(),
            other => panic!("{other:?}"),
        }
    }

    fn sqlstate(e: &RawError) -> Option<&str> {
        e.server.as_ref().and_then(|s| s.sqlstate.as_deref())
    }

    /// `ResultStart`, the first 501 rows, then `Suspended`.
    async fn suspend(c: &mut Box<dyn RawConn>) {
        c.query(spec("SELECT * FROM t", true)).await.unwrap();
        assert!(matches!(
            c.next_event().await.unwrap(),
            QueryEvent::ResultStart(_)
        ));
        assert_eq!(rows_of(&c.next_event().await.unwrap()), 501);
        assert_eq!(c.next_event().await.unwrap(), QueryEvent::Suspended);
    }

    #[tokio::test]
    async fn a_batch_that_returns_exactly_its_limit_suspends() {
        // Review I4a: 501 rows with `first_rows` 501, as PostgreSQL's Execute(501) answers.
        let mut c = pg(vec![select(501)]).await;
        c.batch("BEGIN").await.unwrap();
        suspend(&mut c).await;
        c.fetch_more(100).await.unwrap();
        match c.next_event().await.unwrap() {
            QueryEvent::ResultEnd(end) => assert_eq!((end.rows, end.kept), (501, 501)),
            other => panic!("no Rows after an exact limit: {other:?}"),
        }
        assert!(matches!(c.next_event().await.unwrap(), QueryEvent::Done(_)));
    }

    #[tokio::test]
    async fn fetch_more_of_zero_asks_for_a_chunk() {
        // Review I4e: the PostgreSQL driver fetches `chunk_rows`; 0 never suspends at once.
        let mut c = pg(vec![select(10_000)]).await;
        c.batch("BEGIN").await.unwrap();
        suspend(&mut c).await;
        c.fetch_more(0).await.unwrap();
        assert_eq!(rows_of(&c.next_event().await.unwrap()), 100);
        assert_eq!(c.next_event().await.unwrap(), QueryEvent::Suspended);
    }

    #[tokio::test]
    async fn a_portal_its_commit_destroyed_is_still_the_open_query() {
        // Review I4b, as the PostgreSQL driver's Docker test pins it.
        let mut c = pg(vec![select(10_000), select(10)]).await;
        c.batch("BEGIN").await.unwrap();
        suspend(&mut c).await;
        c.batch("COMMIT").await.unwrap();
        assert_eq!(c.tx_status(), TxStatus::Idle);
        assert_eq!(
            c.next_event().await.unwrap_err().kind,
            RawErrorKind::Unsupported,
            "no I/O on a suspended query"
        );
        assert_eq!(
            c.query(spec("SELECT 1", false)).await.unwrap_err().kind,
            RawErrorKind::Unsupported,
            "still one open query"
        );
        let e = c.fetch_more(10).await.unwrap_err();
        assert_eq!(sqlstate(&e), Some("34000"));
        assert_eq!(c.tx_status(), TxStatus::Idle);
        // Then nothing is open.
        for e in [
            c.next_event().await.unwrap_err(),
            c.fetch_more(10).await.unwrap_err(),
        ] {
            assert_eq!(e.kind, RawErrorKind::Unsupported);
        }
        assert_eq!(
            c.close_query(CloseHow::Now).await.unwrap(),
            Closed::Closed { rows: 0 }
        );
        c.query(spec("SELECT 1", false)).await.unwrap();
    }

    #[tokio::test]
    async fn a_dead_portal_closes_without_an_error_with_its_rows() {
        let mut c = pg(vec![select(10_000)]).await;
        c.batch("BEGIN").await.unwrap();
        suspend(&mut c).await;
        c.batch("COMMIT").await.unwrap();
        assert_eq!(
            c.close_query(CloseHow::Now).await.unwrap(),
            Closed::Closed { rows: 501 }
        );
        assert_eq!(c.tx_status(), TxStatus::Idle);
    }

    #[tokio::test]
    async fn an_execute_on_a_portal_rolled_back_to_fails_the_block() {
        // Review I4b: inside a transaction block the 34000 aborts the transaction (as on the
        // server); a ROLLBACK TO recovers it.
        let mut c = pg(vec![select(10_000), select(10_000)]).await;
        c.batch("BEGIN").await.unwrap();
        c.batch("SAVEPOINT hv_cursor").await.unwrap();
        suspend(&mut c).await;
        c.batch("ROLLBACK TO SAVEPOINT hv_cursor").await.unwrap();
        assert_eq!(c.tx_status(), TxStatus::InTx);
        let e = c.fetch_more(10).await.unwrap_err();
        assert_eq!(sqlstate(&e), Some("34000"));
        assert_eq!(c.tx_status(), TxStatus::Failed);
        c.batch("ROLLBACK TO SAVEPOINT hv_cursor").await.unwrap();
        assert_eq!(c.tx_status(), TxStatus::InTx);
        // A drain's Execute meets the same 34000.
        suspend(&mut c).await;
        c.batch("ROLLBACK TO SAVEPOINT hv_cursor").await.unwrap();
        assert_eq!(
            c.close_query(CloseHow::Drain(Duration::from_millis(250)))
                .await
                .unwrap(),
            Closed::Closed { rows: 501 }
        );
        assert_eq!(c.tx_status(), TxStatus::Failed);
    }

    #[tokio::test]
    async fn closing_an_execute0_reads_it_to_the_end() {
        // Review I4d: a non-suspendable PostgreSQL run is `Completed` with its total rows,
        // however much is left and whatever `how` says.
        for how in [
            CloseHow::Now,
            CloseHow::Drain(Duration::from_millis(250)),
            CloseHow::AfterKill(Duration::from_secs(1)),
        ] {
            let mut c = pg(vec![select(50_000)]).await;
            c.query(spec("SELECT * FROM t", false)).await.unwrap();
            c.next_event().await.unwrap();
            assert_eq!(rows_of(&c.next_event().await.unwrap()), 501);
            assert_eq!(
                c.close_query(how).await.unwrap(),
                Closed::Completed { rows: 50_000 },
                "{how:?}"
            );
        }
    }

    #[tokio::test]
    async fn closing_an_execute0_meets_the_error_of_its_rest() {
        // Review I4d: an error inside the rest (a runtime error, a cancel) is the close's
        // answer, and it fails a transaction block.
        let failure = pg_error("22012", "division by zero");
        let mut c = pg(vec![FakeQuery {
            fail_after: Some((10_000, failure.clone())),
            ..select(50_000)
        }])
        .await;
        c.batch("BEGIN").await.unwrap();
        c.query(spec("SELECT * FROM t", false)).await.unwrap();
        c.next_event().await.unwrap();
        c.next_event().await.unwrap();
        assert_eq!(c.close_query(CloseHow::Now).await.unwrap_err(), failure);
        assert_eq!(c.tx_status(), TxStatus::Failed);
        // A cancel sent while it streamed.
        let mut c = pg(vec![select(50_000)]).await;
        let cancel = c.cancel_handle().unwrap();
        c.query(spec("SELECT * FROM t", false)).await.unwrap();
        c.next_event().await.unwrap();
        c.next_event().await.unwrap();
        cancel.cancel().await.unwrap();
        assert_eq!(
            c.close_query(CloseHow::Now).await.unwrap_err().kind,
            RawErrorKind::Cancelled
        );
        assert_eq!(c.tx_status(), TxStatus::Idle);
        // The rest ends before the scripted failure: completed.
        let mut c = pg(vec![FakeQuery {
            fail_after: Some((50_000, failure)),
            ..select(50_000)
        }])
        .await;
        c.query(spec("SELECT * FROM t", false)).await.unwrap();
        c.next_event().await.unwrap();
        assert_eq!(
            c.close_query(CloseHow::Now).await.unwrap(),
            Closed::Completed { rows: 50_000 }
        );
    }

    // ## m3a-t14-mysql-typed (review round 1, named exception): the MySQL family closes as
    // `hacivat-driver-mysql` does (its `tests/typed_wire.rs` pins the real side).

    async fn mysql(queries: Vec<FakeQuery>) -> Box<dyn RawConn> {
        FakeDriver::new(DbmsKind::MySql)
            .queries(queries)
            .connect(request())
            .await
            .unwrap()
    }

    fn mysql_error(code: i64) -> RawError {
        RawError {
            kind: RawErrorKind::Db,
            server: Some(crate::errors::ServerMessage {
                sqlstate: Some("HY000".to_string()),
                vendor_code: Some(code),
                severity: None,
                text: "an error".to_string(),
                detail: None,
                hint: None,
                lang_hint: None,
            }),
        }
    }

    #[tokio::test]
    async fn a_mysql_now_asks_for_the_kill_and_keeps_the_query_open() {
        let mut c = mysql(vec![select(50_000)]).await;
        suspend(&mut c).await;
        let thread_id = c.backend_id().unwrap();
        assert_eq!(
            c.close_query(CloseHow::Now).await.unwrap(),
            Closed::NeedsServerKill { thread_id }
        );
        // Still open: no statement runs until the kill and `AfterKill`.
        assert_eq!(
            c.batch("SELECT 1").await.unwrap_err().kind,
            RawErrorKind::Unsupported
        );
        assert_eq!(
            c.close_query(CloseHow::AfterKill(Duration::from_secs(1)))
                .await
                .unwrap(),
            Closed::Closed { rows: 501 }
        );
        c.batch("SELECT 1").await.unwrap();
    }

    #[tokio::test]
    async fn a_mysql_close_with_nothing_unread_is_completed() {
        // A result read to its end: `Now` and `AfterKill` (the kill came too late) finish it.
        for how in [CloseHow::Now, CloseHow::AfterKill(Duration::from_secs(1))] {
            let mut c = mysql(vec![select(300)]).await;
            c.query(spec("SELECT * FROM t", false)).await.unwrap();
            c.next_event().await.unwrap();
            assert_eq!(rows_of(&c.next_event().await.unwrap()), 300);
            assert!(matches!(
                c.next_event().await.unwrap(),
                QueryEvent::ResultEnd(_)
            ));
            assert_eq!(
                c.close_query(how).await.unwrap(),
                Closed::Completed { rows: 300 },
                "{how:?}"
            );
        }
        // A statement without rows, closed at once.
        let mut c = mysql(vec![]).await;
        c.query(spec("DELETE FROM t", false)).await.unwrap();
        assert_eq!(
            c.close_query(CloseHow::Now).await.unwrap(),
            Closed::Completed { rows: 0 }
        );
    }

    #[tokio::test]
    async fn a_mysql_drain_meets_the_failure_in_its_rest() {
        // A lock wait timeout in the drained rest is the close's answer.
        let mut c = mysql(vec![FakeQuery {
            fail_after: Some((800, mysql_error(1205))),
            ..select(1_000)
        }])
        .await;
        suspend(&mut c).await;
        assert_eq!(
            c.close_query(CloseHow::Drain(Duration::from_millis(250)))
                .await
                .unwrap_err(),
            mysql_error(1205)
        );
        c.query(spec("SELECT 1", false)).await.unwrap();
        // The kill's 1317 is a close, not an error.
        let mut c = mysql(vec![FakeQuery {
            fail_after: Some((800, mysql_error(1317))),
            ..select(1_000)
        }])
        .await;
        suspend(&mut c).await;
        assert_eq!(
            c.close_query(CloseHow::Drain(Duration::from_millis(250)))
                .await
                .unwrap(),
            Closed::Closed { rows: 501 }
        );
        // A short rest with no failure drains to its end.
        let mut c = mysql(vec![select(1_000)]).await;
        suspend(&mut c).await;
        assert_eq!(
            c.close_query(CloseHow::Drain(Duration::from_millis(250)))
                .await
                .unwrap(),
            Closed::Completed { rows: 1_000 }
        );
    }

    #[tokio::test]
    async fn a_mysql_kill_that_is_ignored_loses_the_connection() {
        let mut c = mysql(vec![FakeQuery {
            kill_ignored: true,
            ..select(50_000)
        }])
        .await;
        suspend(&mut c).await;
        assert!(matches!(
            c.close_query(CloseHow::Drain(Duration::from_millis(250)))
                .await
                .unwrap(),
            Closed::NeedsServerKill { .. }
        ));
        assert_eq!(
            c.close_query(CloseHow::AfterKill(Duration::from_secs(1)))
                .await
                .unwrap_err()
                .kind,
            RawErrorKind::Io(std::io::ErrorKind::TimedOut)
        );
        let lost = RawErrorKind::Io(std::io::ErrorKind::NotConnected);
        assert_eq!(c.ping().await.unwrap_err().kind, lost);
        assert_eq!(c.batch("SELECT 1").await.unwrap_err().kind, lost);
        assert_eq!(
            c.query(spec("SELECT 1", false)).await.unwrap_err().kind,
            lost
        );
    }

    #[tokio::test]
    async fn mysql_connections_have_their_own_thread_ids() {
        async fn ids(driver: FakeDriver) -> (Option<u64>, Option<u64>) {
            let a = driver.connect(request()).await.unwrap();
            let b = driver.connect(request()).await.unwrap();
            (a.backend_id(), b.backend_id())
        }
        assert_eq!(
            ids(FakeDriver::new(DbmsKind::MySql).backend(42)).await,
            (Some(42), Some(43))
        );
        let (a, b) = ids(FakeDriver::new(DbmsKind::MariaDb)).await;
        assert!(a.is_some_and(|a| a != 0) && b.is_some_and(|b| b != 0) && a != b);
        // Other engines report none unless told.
        assert_eq!(ids(FakeDriver::new(DbmsKind::Postgres)).await, (None, None));
        // The kill names the connection's own ID.
        let driver = FakeDriver::new(DbmsKind::MySql)
            .backend(7)
            .queries(vec![select(50_000), select(50_000)]);
        let _first = driver.connect(request()).await.unwrap();
        let mut second = driver.connect(request()).await.unwrap();
        suspend(&mut second).await;
        assert_eq!(
            second.close_query(CloseHow::Now).await.unwrap(),
            Closed::NeedsServerKill { thread_id: 8 }
        );
    }
}
