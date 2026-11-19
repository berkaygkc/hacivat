// ## m4-t01c-core
//! Metadata connections (HV-SESS-01's metadata connection; HV-INTRO-01 step 4; R-M4-7, R-M4-29,
//! R-M4-38): the read-only connections the introspectors read the catalog through.
//!
//! - **Trusted text only.** A [`MetaConn`] runs only a [`MetaQuery`], built by the pack renderer
//!   (`hacivat-introspect`'s `RenderedQuery::as_meta_query`; check-deps `MetaQueryOnlyFromPacks`
//!   allows `MetaQuery::new` only there and in this file). Its fields are private and
//!   [`MetaQuery::new`] is its only constructor: no `Default`, `From`, `Deserialize` or builder,
//!   so a struct literal cannot bypass the rule. It never passes through `hacivat-exec`, the
//!   classifier or the history store, and never appears in the user's output log.
//! - **Server-enforced read-only** (Task 5): PostgreSQL `BEGIN ISOLATION LEVEL REPEATABLE READ
//!   READ ONLY` plus `SET LOCAL search_path = pg_catalog, pg_temp`; MySQL and MariaDB a read-only
//!   session (restored after every reconnect, asserted before every snapshot) plus
//!   `START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY`; SQLite a separate `mode=ro`
//!   connection (R-M4-38) that does not count against the profile's single session (U-6(e)).
//! - **Joined connections** (R-M4-29) run `joinable` sections inside a transaction Hacivat owns
//!   on a `Job(DdlApply | DriftCheck)` session; they never begin, commit or roll back, refuse
//!   text whose first keyword is `BEGIN`, `START`, `COMMIT`, `ROLLBACK`, `SAVEPOINT`, `RELEASE`
//!   or `SET`, and are refused on `Console` and `Editor` sessions.
//!
//! ## m4-t05-meta (bodies, M4 Task 5; ADR 0004 `## m4-t05 — metadata sessions`)
//!
//! A [`MetaConn`] reads in one of three ways:
//!
//! - **Its own connection** (PostgreSQL, MySQL and MariaDB profiles in multiple-session mode;
//!   SQLite files). [`SessionManager::meta_conn`] leases it: a slot of the per-database cap
//!   ([`MetaPools`]: 1 by default, at most [`META_POOL_MAX`]), then a permit of the profile's
//!   budget at `Priority::Introspection` (SQLite's reader takes none: it is a separate
//!   read-only reader, U-6(e)), then a headless `Purpose::Probe` connect for
//!   `Department::Introspection` with `application_name = "Hacivat (metadata)"` (MySQL: the
//!   `program_name` connect attribute), read-only. The session set-up runs at every connect
//!   and every reconnect:
//!   - PostgreSQL: `SET statement_timeout`. The read-only connect already sets
//!     `default_transaction_read_only`; every snapshot is
//!     `BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY` then
//!     `SET LOCAL search_path = pg_catalog, pg_temp` (R-M4-46).
//!   - MySQL and MariaDB: `SET SESSION TRANSACTION READ ONLY`, `SET time_zone = '+00:00'` and
//!     the statement timeout (MySQL `max_execution_time`, SELECTs only; MariaDB
//!     `max_statement_time`). Before every snapshot the session's read-only variable must read
//!     1 (`@@session.transaction_read_only`; MariaDB before 11.1 `@@session.tx_read_only`), else
//!     [`MetaError::ReadOnlyNotEnforced`]; then
//!     `START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY`.
//!   - SQLite: the main file opened read-only (`SQLITE_OPEN_READ_ONLY`, the flag form of
//!     `mode=ro`) straight through the driver, without the user-facing authorizer (R-M4-38);
//!     every snapshot is a deferred `BEGIN`. Attached files go through
//!     [`MetaConn::attach_read_only`]: SQLite opens them with the main file's flags.
//!
//!   A lost connection answers [`MetaError::ConnectionLost`] and is dropped; nothing is retried
//!   inside a snapshot. The next [`MetaConn::begin_snapshot`] connects again and runs the
//!   set-up (and the MySQL assertion) again. The connection closes when the `MetaConn` drops,
//!   and when the profile disconnects (`close_all`), whichever comes first; its permit goes
//!   back only after the close.
//! - **The user's session** (single-session profiles, SQLite `:memory:`, and SQLite's `temp`
//!   schema, alg/02 HV-INTRO-08 step 4): the reads run on the default session, through its
//!   queue, only while it is idle and outside a transaction. [`SessionManager::meta_conn`]
//!   waits while the session runs a request and answers [`MetaError::Busy`] while it holds a
//!   transaction or a cursor (alg/02 HV-INTRO-01 edge 10); every read checks again before and
//!   after it runs.
//! - **A joined session** ([`MetaConn::join`], R-M4-29).
//!
//! On the last two the server enforces nothing, so every text must pass [`check_read_text`]
//! (the carries' allowlist, the primary guard there; check-deps is defence in depth, R-W0-2):
//! exactly one statement whose first keyword, after comments, is `SELECT`, `WITH`, `SHOW`,
//! `TABLE` or `VALUES`; no MySQL executable comment (`/*!`, `/*M!`) anywhere; no nested block
//! comment before the keyword; no `;` but one at the very end; and no `INSERT`, `UPDATE`,
//! `DELETE`, `MERGE` or `INTO` word anywhere (a data-modifying `WITH`, `SELECT … INTO`,
//! `FOR UPDATE`).
//!
//! Every value comes back as text: text, JSON and XML as the server sent them, bytes as UTF-8
//! when they are, any other value as `hacivat_values::valfmt::raw_text` writes it (packs that
//! need the server's own spelling cast to text). The cell cap is off.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_driver::{
    DriverConnect, Endpoint, InsecureAuth, Phase, Protocol, QueryEvent, QuerySpec, RawConn,
    RawError, RawErrorKind, ServerInfo, ServerMessage, TxStatus, classify,
};
use hacivat_proto::{DbmsKind, ProfileId};
use hacivat_results::{Completeness, FetchPolicy, MemoryBudget, ResultId, ResultStore};
use hacivat_values::valfmt::{BinaryDisplay, ColumnFacts, RawText, raw_text};
use hacivat_values::{BindValue, LogicalType, ResultSchema, ValueRef};
use tokio::runtime::Handle;
use tokio::sync::{OwnedMutexGuard, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::budget::BudgetPermit;
use crate::pipeline::LiveConnection;
use crate::profile::ProfileEndpoint;
use crate::session::recovery::connection_ending;
use crate::session::{
    ErrorPolicy, ExecError, ExecErrorKind, JobKind, ObserverRef, RunEvent, RunId, RunObserver,
    RunSpec, RunUnit, SessionEffects, SessionHandle, SessionKind, SessionManager, SessionState,
    UnitId, UnitVerdict,
};

/// What metadata connection to open.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MetaConnSpec {
    pub profile: ProfileId,
    /// The database to read (PostgreSQL needs one connection per database); `None` is the
    /// profile's.
    pub database: Option<String>,
    /// The server-side limit of each catalog statement.
    pub statement_timeout: Duration,
}

/// Why a metadata read failed. Codes only: the edge maps them to Fluent.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MetaError {
    /// No metadata connection can be had (the stub's only answer; the profile is not
    /// connected; the engine has none).
    Unavailable,
    /// The profile's connection budget has no room now.
    Busy,
    ConnectionLost,
    Timeout,
    /// The session cannot be joined (not a Hacivat-owned `Job(DdlApply | DriftCheck)`
    /// transaction), or the query is not `joinable`.
    NotJoinable,
    /// A joined connection was handed transaction-control or `SET` text.
    RefusedText,
    /// The server did not confirm the read-only session or transaction.
    ReadOnlyNotEnforced,
    /// The server refused the statement; its own message, verbatim.
    Server {
        sqlstate: Option<Arc<str>>,
        message: ServerMessage,
    },
    Cancelled,
}

/// One catalog statement: trusted pack text with its binds (names and ids always travel as
/// binds, R-M4-3).
///
/// Its fields are private and [`MetaQuery::new`] is its only constructor, so a struct literal
/// outside this module does not compile. (On a stable toolchain rustdoc does not check the
/// `E0451` code of a `compile_fail` test, so the test would pass on any compile error; the T1c
/// review compiled the literal outside the crate and saw exactly E0451, "fields `sql`, `binds`
/// and `joinable` of struct `MetaQuery` are private". The positive test below proves the rest
/// of the snippet compiles.)
///
/// ```compile_fail,E0451
/// use hacivat_session::meta::MetaQuery;
/// let q: MetaQuery<'static> = MetaQuery { sql: "SELECT 1", binds: &[], joinable: false };
/// ```
///
/// while the constructor does (the same values, through `new`):
///
/// ```
/// use hacivat_session::meta::MetaQuery;
/// let q: MetaQuery<'static> = MetaQuery::new("SELECT 1", &[], false);
/// assert!(!q.joinable());
/// ```
#[derive(Debug, Clone, Copy)]
pub struct MetaQuery<'a> {
    sql: &'a str,
    binds: &'a [BindValue],
    joinable: bool,
}

impl<'a> MetaQuery<'a> {
    /// Called only by the pack renderer (`RenderedQuery::as_meta_query`); check-deps
    /// `MetaQueryOnlyFromPacks` refuses the call anywhere else.
    #[doc(hidden)]
    pub fn new(sql: &'a str, binds: &'a [BindValue], joinable: bool) -> Self {
        MetaQuery {
            sql,
            binds,
            joinable,
        }
    }

    pub fn sql(&self) -> &'a str {
        self.sql
    }

    pub fn binds(&self) -> &'a [BindValue] {
        self.binds
    }

    /// The pack section is marked joinable (R-M4-29): it may run on a joined connection.
    pub fn joinable(&self) -> bool {
        self.joinable
    }
}

/// What a catalog statement answered: column names and text cells (`None` for SQL NULL).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Rows {
    pub columns: Vec<Arc<str>>,
    pub rows: Vec<Vec<Option<Arc<str>>>>,
}

// ## m4-t05-meta

/// The most metadata connections one database of a profile may have at once.
pub const META_POOL_MAX: usize = 3;

/// The `application_name` (PostgreSQL) and `program_name` (MySQL, MariaDB) of a metadata
/// connection.
pub const META_APPLICATION_NAME: &str = "Hacivat (metadata)";

/// The per-database cap of a profile's metadata connections (1 by default, at most
/// [`META_POOL_MAX`]); a request over the cap waits for a connection of the same database to
/// drop.
pub struct MetaPools {
    size: AtomicUsize,
    slots: Mutex<HashMap<Option<String>, Arc<Semaphore>>>,
}

impl Default for MetaPools {
    fn default() -> Self {
        MetaPools {
            size: AtomicUsize::new(1),
            slots: Mutex::new(HashMap::new()),
        }
    }
}

impl fmt::Debug for MetaPools {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MetaPools")
            .field("size", &self.size.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl MetaPools {
    /// The semaphore of `database`, made with the current size on first use.
    fn slots_of(&self, database: &Option<String>) -> Arc<Semaphore> {
        let size = self.size.load(Ordering::SeqCst);
        self.slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(database.clone())
            .or_insert_with(|| Arc::new(Semaphore::new(size)))
            .clone()
    }
}

/// How a [`MetaConn`] reads.
enum Via {
    /// [`MetaConn::offline`]: nothing behind it, every read answers `Unavailable`.
    Offline,
    Own(Box<Own>),
    Session(Box<OnSession>),
}

/// A read-only metadata connection (or a joined one, R-M4-29). Built only by
/// [`SessionManager::meta_conn`] and [`MetaConn::join`] (and the inert [`MetaConn::offline`]).
pub struct MetaConn {
    dbms: DbmsKind,
    server: ServerInfo,
    joined: bool,
    via: Via,
}

impl fmt::Debug for MetaConn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let via = match &self.via {
            Via::Offline => "offline",
            Via::Own(_) => "own",
            Via::Session(s) if s.joined => "joined",
            Via::Session(_) => "user-session",
        };
        f.debug_struct("MetaConn")
            .field("dbms", &self.dbms)
            .field("joined", &self.joined)
            .field("via", &via)
            .finish_non_exhaustive()
    }
}

impl MetaConn {
    pub fn dbms(&self) -> DbmsKind {
        self.dbms
    }

    /// What the server reported at connect. A joined connection does not know its server:
    /// kind `Unknown`, no version (the caller knows the profile's engine).
    pub fn server(&self) -> &ServerInfo {
        &self.server
    }

    /// Opens one consistent read-only snapshot (see the module docs per engine).
    pub fn begin_snapshot(&mut self) -> BoxFuture<'_, Result<SnapshotTxn<'_>, MetaError>> {
        Box::pin(async move {
            match &mut self.via {
                Via::Offline => return Err(MetaError::Unavailable),
                Via::Own(own) => own.begin().await?,
                Via::Session(s) => s.check_open()?,
            }
            Ok(SnapshotTxn { conn: Some(self) })
        })
    }

    /// A connection that reads inside `session`'s open transaction, which Hacivat owns on a
    /// `Job(DdlApply | DriftCheck)` session (R-M4-29); refused on any other session.
    ///
    /// Accepted only for a usable `Job(_, DdlApply | DriftCheck)` session inside a transaction
    /// (`TxStatus::InTx`): a job session has no user, so its transaction is Hacivat's. Every
    /// other kind (`Console`, `Editor`, `Introspection`, `Service`, an export or import job)
    /// and a job outside a transaction answer [`MetaError::NotJoinable`]. The SQLite writer
    /// while an M7 apply holds it has no seam yet: it is refused too (M7 adds it).
    pub fn join(session: &SessionHandle) -> Result<MetaConn, MetaError> {
        let on = OnSession {
            handle: session.clone(),
            joined: true,
        };
        on.check_open()?;
        Ok(MetaConn {
            dbms: DbmsKind::Unknown,
            server: unknown_server(DbmsKind::Unknown),
            joined: true,
            via: Via::Session(Box::new(on)),
        })
    }

    // ## m4-t01c-core (Part B)
    /// A connection with no server behind it, for tests that must hand an introspector a
    /// `MetaConn` (the seams test, Task 15's engine tests with `FakeIntrospector`). It can never
    /// read: every snapshot answers [`MetaError::Unavailable`], and Task 5 must keep it inert
    /// (no driver connection, no text accepted). Not for product code.
    #[doc(hidden)]
    pub fn offline(dbms: DbmsKind) -> MetaConn {
        MetaConn {
            dbms,
            server: unknown_server(dbms),
            joined: false,
            via: Via::Offline,
        }
    }

    // ## m4-t05-meta
    /// SQLite: attaches `file` as schema `schema` on the separate read-only connection, outside
    /// a snapshot. SQLite opens an attached file with the main file's flags, so it is read-only
    /// too (R-M4-38); the name and the path travel as binds. Every other connection answers
    /// [`MetaError::Unavailable`].
    pub fn attach_read_only<'a>(
        &'a mut self,
        schema: &'a str,
        file: &'a Path,
    ) -> BoxFuture<'a, Result<(), MetaError>> {
        Box::pin(async move {
            let Via::Own(own) = &mut self.via else {
                return Err(MetaError::Unavailable);
            };
            if own.flavor != Flavor::Sqlite {
                return Err(MetaError::Unavailable);
            }
            let Some(path) = file.to_str() else {
                return Err(MetaError::Unavailable);
            };
            let binds = [text_bind(path), text_bind(schema)];
            own.read("ATTACH DATABASE ? AS ?", &binds).await.map(drop)
        })
    }

    /// Whether the connection's current transaction has an XID assigned
    /// (`pg_current_xact_id_if_assigned() IS NOT NULL`): the `#pg-canary` check of Task 5's
    /// tests. PostgreSQL's own connection only; anything else answers
    /// [`MetaError::Unavailable`].
    #[cfg(any(test, feature = "test-support"))]
    pub fn debug_xid_assigned(&mut self) -> BoxFuture<'_, Result<bool, MetaError>> {
        Box::pin(async move {
            let Via::Own(own) = &mut self.via else {
                return Err(MetaError::Unavailable);
            };
            if own.flavor != Flavor::Pg {
                return Err(MetaError::Unavailable);
            }
            let rows = own
                .read("SELECT pg_current_xact_id_if_assigned() IS NOT NULL", &[])
                .await?;
            Ok(first_cell(&rows) == Some("true"))
        })
    }
}

impl Drop for MetaConn {
    fn drop(&mut self) {
        if let Via::Own(own) = &self.via {
            // The watcher closes the connection, then gives the permit and the slot back.
            own.closed.cancel();
        }
    }
}

/// One snapshot of a [`MetaConn`]: every [`rows`](Self::rows) call reads the same catalog
/// state; [`commit`](Self::commit) ends it (a read-only transaction's commit writes nothing).
/// Dropped without a commit, the snapshot is rolled back before the connection reads again
/// (or closes).
pub struct SnapshotTxn<'c> {
    conn: Option<&'c mut MetaConn>,
}

impl fmt::Debug for SnapshotTxn<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SnapshotTxn")
            .field("conn", &self.conn)
            .finish()
    }
}

impl<'c> SnapshotTxn<'c> {
    /// Runs one catalog statement.
    pub fn rows<'q>(&'q mut self, q: MetaQuery<'q>) -> BoxFuture<'q, Result<Rows, MetaError>> {
        Box::pin(async move {
            let Some(conn) = self.conn.as_deref_mut() else {
                return Err(MetaError::Unavailable);
            };
            match &mut conn.via {
                Via::Offline => Err(MetaError::Unavailable),
                Via::Own(own) => own.read(q.sql(), q.binds()).await,
                Via::Session(s) => s.read(q).await,
            }
        })
    }

    /// Ends the snapshot (a joined connection or the user's session sends nothing).
    pub fn commit(mut self) -> BoxFuture<'c, Result<(), MetaError>> {
        let conn = self.conn.take();
        Box::pin(async move {
            let Some(conn) = conn else {
                return Err(MetaError::Unavailable);
            };
            match &mut conn.via {
                Via::Offline => Err(MetaError::Unavailable),
                Via::Own(own) => own.commit().await,
                Via::Session(s) => s.check_after(),
            }
        })
    }
}

impl Drop for SnapshotTxn<'_> {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take()
            && let Via::Own(own) = &mut conn.via
        {
            own.abandoned = true;
        }
    }
}

impl SessionManager {
    /// A metadata connection for `spec` (the module docs): its own connection (a
    /// `Priority::Introspection` lease through the profile's budget; SQLite's read-only reader
    /// takes none) or the user's idle session (single-session profiles, SQLite `:memory:` and
    /// `temp`). `cancel` ends the wait (`Cancelled`). `Unavailable` when `spec` names another
    /// profile, the engine is not one M4 introspects, or the profile disconnected.
    pub fn meta_conn<'a>(
        &'a self,
        spec: MetaConnSpec,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<MetaConn, MetaError>> {
        Box::pin(async move {
            let profile = self.meta_profile();
            if spec.profile != profile.id {
                return Err(MetaError::Unavailable);
            }
            let memory = matches!(
                &profile.endpoint,
                ProfileEndpoint::File { path, .. } if path.as_os_str() == ":memory:"
            );
            match profile.engine {
                DbmsKind::Sqlite if memory || spec.database.as_deref() == Some("temp") => {
                    self.meta_on_user_session(profile.engine, cancel).await
                }
                DbmsKind::Sqlite => self.meta_sqlite(spec, cancel).await,
                DbmsKind::Postgres | DbmsKind::MySql | DbmsKind::MariaDb => {
                    if self.meta_single() {
                        self.meta_on_user_session(profile.engine, cancel).await
                    } else {
                        self.meta_network(spec, cancel).await
                    }
                }
                _ => Err(MetaError::Unavailable),
            }
        })
    }

    /// Sets how many metadata connections one database may have at once, clamped to
    /// `1..=META_POOL_MAX`; databases already used keep their cap.
    pub fn set_meta_pool_size(&self, n: usize) {
        self.meta_pools()
            .size
            .store(n.clamp(1, META_POOL_MAX), Ordering::SeqCst);
    }

    /// The user's default session, once it is idle outside a transaction.
    async fn meta_on_user_session(
        &self,
        engine: DbmsKind,
        cancel: &CancellationToken,
    ) -> Result<MetaConn, MetaError> {
        let Some(handle) = self.default_session() else {
            return Err(MetaError::Unavailable);
        };
        let on = OnSession {
            handle,
            joined: false,
        };
        on.wait_idle(cancel).await?;
        let version = cancellable(cancel, on.handle.classifier_context())
            .await?
            .server_version;
        let mut server = unknown_server(engine);
        server.version = version;
        Ok(MetaConn {
            dbms: engine,
            server,
            joined: false,
            via: Via::Session(Box::new(on)),
        })
    }

    /// A slot of `database`'s cap, unless `cancel` comes first.
    async fn meta_slot(
        &self,
        database: &Option<String>,
        cancel: &CancellationToken,
    ) -> Result<OwnedSemaphorePermit, MetaError> {
        let slots = self.meta_pools().slots_of(database);
        cancellable(cancel, slots.acquire_owned())
            .await?
            .map_err(|_| MetaError::Unavailable)
    }

    /// PostgreSQL, MySQL, MariaDB: an own read-only connection.
    async fn meta_network(
        &self,
        spec: MetaConnSpec,
        cancel: &CancellationToken,
    ) -> Result<MetaConn, MetaError> {
        // Taken first: a `close_all` from now on ends this request and its connection.
        let closed = self.meta_closed_token();
        let slot = self.meta_slot(&spec.database, cancel).await?;
        let permit = cancellable(cancel, self.meta_permit())
            .await?
            .ok_or(MetaError::Unavailable)?;
        let target = Target {
            database: spec.database,
            timeout: spec.statement_timeout,
        };
        let manager = self.meta_manager();
        // The connect itself is bounded by the profile's timeout and never dropped half-way (a
        // dropped one could leave its cleanup guards behind); a `close_all` meanwhile fails the
        // pipeline's connect, or the watcher closes the connection at once.
        let (conn, flavor, server) = connect_network(self, &target).await?;
        if closed.is_cancelled() {
            conn.close().await;
            return Err(MetaError::Unavailable);
        }
        Ok(self.meta_own(
            conn,
            flavor,
            server,
            target,
            closed,
            Some(permit),
            slot,
            manager,
        ))
    }

    /// SQLite: the separate read-only reader of the main file, outside the budget.
    async fn meta_sqlite(
        &self,
        spec: MetaConnSpec,
        cancel: &CancellationToken,
    ) -> Result<MetaConn, MetaError> {
        let closed = self.meta_closed_token();
        let slot = self.meta_slot(&spec.database, cancel).await?;
        let target = Target {
            database: spec.database,
            timeout: spec.statement_timeout,
        };
        let manager = self.meta_manager();
        let (conn, server) = connect_sqlite(self).await?;
        if closed.is_cancelled() {
            conn.close().await;
            return Err(MetaError::Unavailable);
        }
        Ok(self.meta_own(
            conn,
            Flavor::Sqlite,
            server,
            target,
            closed,
            None,
            slot,
            manager,
        ))
    }

    /// Wraps a fresh connection and starts its watcher: it closes the connection when the
    /// `MetaConn` drops or the profile disconnects, then frees the permit and the slot.
    #[allow(clippy::too_many_arguments, reason = "private; two call sites")]
    fn meta_own(
        &self,
        conn: Conn,
        flavor: Flavor,
        server: ServerInfo,
        target: Target,
        closed: CancellationToken,
        permit: Option<BudgetPermit>,
        slot: OwnedSemaphorePermit,
        manager: Weak<SessionManager>,
    ) -> MetaConn {
        let rt = self.meta_runtime();
        let cell = Arc::new(tokio::sync::Mutex::new(Some(conn)));
        let watched = cell.clone();
        let until = closed.clone();
        rt.spawn(async move {
            until.cancelled().await;
            let taken = watched.lock().await.take();
            if let Some(conn) = taken {
                conn.close().await;
            }
            drop(permit);
            drop(slot);
        });
        let dbms = server.kind;
        MetaConn {
            dbms,
            server,
            joined: false,
            via: Via::Own(Box::new(Own {
                cell,
                closed,
                manager,
                target,
                flavor,
                rt,
                abandoned: false,
                in_flight: false,
            })),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// An own connection

/// The engine family of an own connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flavor {
    Pg,
    MySql,
    MariaDb { transaction_read_only: bool },
    Sqlite,
}

/// What an own connection reconnects to.
#[derive(Debug, Clone)]
struct Target {
    database: Option<String>,
    timeout: Duration,
}

/// An open connection: through the pipeline (network engines) or the driver (SQLite).
enum Conn {
    Live(Box<LiveConnection>),
    Raw(Box<dyn RawConn>),
}

impl Conn {
    fn raw(&mut self) -> &mut dyn RawConn {
        match self {
            Conn::Live(live) => live.raw(),
            Conn::Raw(raw) => &mut **raw,
        }
    }

    fn close(self) -> BoxFuture<'static, ()> {
        match self {
            Conn::Live(live) => (*live).close(),
            Conn::Raw(raw) => raw.close(),
        }
    }
}

struct Own {
    /// The connection; `None` once lost (the next snapshot connects again) or closed.
    cell: Arc<tokio::sync::Mutex<Option<Conn>>>,
    /// Cancelled by `close_all` (its parent) or by the `MetaConn`'s drop.
    closed: CancellationToken,
    manager: Weak<SessionManager>,
    target: Target,
    flavor: Flavor,
    rt: Handle,
    /// A snapshot was dropped without its commit: roll back before the next read.
    abandoned: bool,
    /// A read's future was dropped before it ended: the connection's state is unknown.
    in_flight: bool,
}

impl Own {
    /// The connection, locked; a lost one is connected again when `reconnect`.
    async fn conn(&mut self, reconnect: bool) -> Result<OwnedMutexGuard<Option<Conn>>, MetaError> {
        if self.closed.is_cancelled() {
            return Err(MetaError::Unavailable);
        }
        let mut guard = self.cell.clone().lock_owned().await;
        if std::mem::take(&mut self.in_flight)
            && let Some(conn) = guard.take()
        {
            // A dropped query future leaves the connection mid-statement: never reused.
            self.rt.spawn(conn.close());
        }
        if guard.is_none() {
            if !reconnect {
                return Err(MetaError::ConnectionLost);
            }
            let manager = self.manager.upgrade().ok_or(MetaError::Unavailable)?;
            // Never dropped half-way (as the first connect); the profile's disconnect fails the
            // pipeline's connect, or is seen right after it.
            let fresh = match self.flavor {
                Flavor::Sqlite => connect_sqlite(&manager).await.map(|(c, _)| c),
                _ => connect_network(&manager, &self.target)
                    .await
                    .map(|(c, ..)| c),
            }?;
            if self.closed.is_cancelled() {
                fresh.close().await;
                return Err(MetaError::Unavailable);
            }
            self.abandoned = false;
            *guard = Some(fresh);
        }
        Ok(guard)
    }

    /// After an operation: a lost connection is dropped (closed in the background).
    fn settle<T>(
        &self,
        guard: &mut OwnedMutexGuard<Option<Conn>>,
        done: Result<T, MetaError>,
    ) -> Result<T, MetaError> {
        if matches!(done, Err(MetaError::ConnectionLost))
            && let Some(conn) = guard.take()
        {
            self.rt.spawn(conn.close());
        }
        done
    }

    /// Trusted control text, unless the profile disconnects first.
    async fn control(
        &mut self,
        guard: &mut OwnedMutexGuard<Option<Conn>>,
        sql: &str,
    ) -> Result<(), MetaError> {
        let kind = self.kind();
        let Some(conn) = guard.as_mut() else {
            return Err(MetaError::ConnectionLost);
        };
        self.in_flight = true;
        let done = guarded(&self.closed, conn.raw().batch(sql))
            .await
            .and_then(|r| r.map_err(|e| meta_error(kind, &e)));
        self.in_flight = false;
        self.settle(guard, done)
    }

    /// One typed read, unless the profile disconnects first.
    async fn query(
        &mut self,
        guard: &mut OwnedMutexGuard<Option<Conn>>,
        sql: &str,
        binds: &[BindValue],
    ) -> Result<Rows, MetaError> {
        let kind = self.kind();
        let Some(conn) = guard.as_mut() else {
            return Err(MetaError::ConnectionLost);
        };
        self.in_flight = true;
        let done = guarded(&self.closed, read_rows(conn.raw(), kind, sql, binds))
            .await
            .and_then(|r| r);
        self.in_flight = false;
        self.settle(guard, done)
    }

    fn kind(&self) -> DbmsKind {
        flavor_kind(self.flavor)
    }

    /// The transaction status the driver reports after the last round trip.
    fn tx(guard: &mut OwnedMutexGuard<Option<Conn>>) -> TxStatus {
        guard
            .as_mut()
            .map_or(TxStatus::Idle, |c| c.raw().tx_status())
    }

    async fn begin(&mut self) -> Result<(), MetaError> {
        let mut guard = self.conn(true).await?;
        // A snapshot dropped without its commit, or a begin cut short: never read on in it.
        if std::mem::take(&mut self.abandoned) || Self::tx(&mut guard) != TxStatus::Idle {
            self.control(&mut guard, "ROLLBACK").await?;
        }
        match self.flavor {
            Flavor::Pg => {
                self.control(
                    &mut guard,
                    "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
                )
                .await?;
                self.control(&mut guard, "SET LOCAL search_path = pg_catalog, pg_temp")
                    .await?;
            }
            Flavor::MySql | Flavor::MariaDb { .. } => {
                let rows = self
                    .query(&mut guard, read_only_probe(self.flavor), &[])
                    .await?;
                if first_cell(&rows) != Some("1") {
                    return Err(MetaError::ReadOnlyNotEnforced);
                }
                self.control(
                    &mut guard,
                    "START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY",
                )
                .await?;
            }
            Flavor::Sqlite => self.control(&mut guard, "BEGIN").await?,
        }
        if Self::tx(&mut guard) != TxStatus::InTx {
            // Whatever opened, it is not the read-only transaction this snapshot needs.
            if let Some(conn) = guard.take() {
                self.rt.spawn(conn.close());
            }
            return Err(MetaError::ReadOnlyNotEnforced);
        }
        Ok(())
    }

    async fn read(&mut self, sql: &str, binds: &[BindValue]) -> Result<Rows, MetaError> {
        let mut guard = self.conn(false).await?;
        self.query(&mut guard, sql, binds).await
    }

    async fn commit(&mut self) -> Result<(), MetaError> {
        let mut guard = self.conn(false).await?;
        self.control(&mut guard, "COMMIT").await?;
        if Self::tx(&mut guard) != TxStatus::Idle {
            return Err(MetaError::ReadOnlyNotEnforced);
        }
        Ok(())
    }
}

/// The engine of `flavor`.
fn flavor_kind(flavor: Flavor) -> DbmsKind {
    match flavor {
        Flavor::Pg => DbmsKind::Postgres,
        Flavor::MySql => DbmsKind::MySql,
        Flavor::MariaDb { .. } => DbmsKind::MariaDb,
        Flavor::Sqlite => DbmsKind::Sqlite,
    }
}

/// The MySQL/MariaDB statement that reads whether the session is read-only (1).
fn read_only_probe(flavor: Flavor) -> &'static str {
    match flavor {
        Flavor::MariaDb {
            transaction_read_only: false,
        } => "SELECT @@session.tx_read_only",
        _ => "SELECT @@session.transaction_read_only",
    }
}

/// Connects a network profile for metadata and runs the session set-up.
async fn connect_network(
    manager: &SessionManager,
    target: &Target,
) -> Result<(Conn, Flavor, ServerInfo), MetaError> {
    let database = target.database.clone();
    let adjust = move |p: &mut crate::profile::Profile| {
        p.application_name = META_APPLICATION_NAME.to_string();
        if database.is_some() {
            p.database.clone_from(&database);
        }
    };
    let live = manager
        .meta_connect(&adjust)
        .await
        .map_err(|_| MetaError::Unavailable)?;
    let server = live.server.clone();
    let flavor = match server.kind {
        DbmsKind::Postgres => Flavor::Pg,
        DbmsKind::MySql => Flavor::MySql,
        DbmsKind::MariaDb => Flavor::MariaDb {
            transaction_read_only: server
                .version
                .as_ref()
                .is_some_and(|v| (v.major, v.minor) >= (11, 1)),
        },
        _ => {
            live.close().await;
            return Err(MetaError::Unavailable);
        }
    };
    let mut conn = Conn::Live(Box::new(live));
    if let Err(e) = set_up(conn.raw(), flavor, target.timeout).await {
        conn.close().await;
        return Err(e);
    }
    Ok((conn, flavor, server))
}

/// The session set-up of a fresh own connection (module docs).
async fn set_up(raw: &mut dyn RawConn, flavor: Flavor, timeout: Duration) -> Result<(), MetaError> {
    let millis = timeout.as_millis().clamp(1, u128::from(u32::MAX));
    let statements: Vec<String> = match flavor {
        Flavor::Pg => vec![format!("SET statement_timeout = {millis}")],
        Flavor::MySql => vec![
            "SET SESSION TRANSACTION READ ONLY".into(),
            "SET time_zone = '+00:00'".into(),
            format!("SET SESSION max_execution_time = {millis}"),
        ],
        Flavor::MariaDb { .. } => vec![
            "SET SESSION TRANSACTION READ ONLY".into(),
            "SET time_zone = '+00:00'".into(),
            format!(
                "SET SESSION max_statement_time = {}.{:03}",
                millis / 1000,
                millis % 1000
            ),
        ],
        Flavor::Sqlite => Vec::new(),
    };
    let kind = flavor_kind(flavor);
    for sql in statements {
        raw.batch(&sql).await.map_err(|e| meta_error(kind, &e))?;
    }
    Ok(())
}

/// SQLite: the main file, read-only, straight through the driver: no authorizer (R-M4-38).
async fn connect_sqlite(manager: &SessionManager) -> Result<(Conn, ServerInfo), MetaError> {
    let profile = manager.meta_profile();
    let ProfileEndpoint::File { path, .. } = &profile.endpoint else {
        return Err(MetaError::Unavailable);
    };
    let driver = manager
        .meta_driver(DbmsKind::Sqlite)
        .ok_or(MetaError::Unavailable)?;
    let req = DriverConnect {
        endpoint: Endpoint::File {
            path: path.clone(),
            create_if_missing: false,
            read_only: true,
        },
        tls: None,
        user: None,
        password: None,
        database: None,
        application_name: META_APPLICATION_NAME.to_string(),
        // Not the profile's read-only hardening: that is the user-facing authorizer.
        read_only: false,
        options: Vec::new(),
        connect_timeout: Duration::from_secs(u64::from(profile.connect_timeout_s.max(1))),
        events: Arc::new(|_| {}),
        insecure_auth: InsecureAuth::default(),
    };
    let mut raw = driver
        .connect(req)
        .await
        .map_err(|e| meta_error(DbmsKind::Sqlite, &e))?;
    let server = match raw.server_info().await {
        Ok(info) => info,
        Err(e) => {
            raw.close().await;
            return Err(meta_error(DbmsKind::Sqlite, &e));
        }
    };
    Ok((Conn::Raw(raw), server))
}

/// One typed query with binds, its rows as text (the cell cap off).
async fn read_rows(
    raw: &mut dyn RawConn,
    kind: DbmsKind,
    sql: &str,
    binds: &[BindValue],
) -> Result<Rows, MetaError> {
    let spec = QuerySpec {
        sql,
        binds,
        protocol: Protocol::Extended,
        suspendable: false,
        first_rows: 8_192,
        chunk_rows: 8_192,
        chunk_bytes: 16 << 20,
        cell_cap: u32::MAX,
        max_result_sets: 1,
    };
    raw.query(spec).await.map_err(|e| meta_error(kind, &e))?;
    let mut out = Rows::default();
    let mut facts: Vec<Facts> = Vec::new();
    loop {
        match raw.next_event().await.map_err(|e| meta_error(kind, &e))? {
            QueryEvent::ResultStart(schema) => {
                out.columns = schema.columns.iter().map(|c| c.name.clone()).collect();
                facts = Facts::of(&schema);
            }
            QueryEvent::Rows(chunk) => {
                for r in 0..chunk.rows() {
                    let mut row = Vec::with_capacity(facts.len());
                    for (c, f) in facts.iter().enumerate() {
                        row.push(cell_text(chunk.cell(r, c as u32), f, kind)?);
                    }
                    out.rows.push(row);
                }
            }
            QueryEvent::Done(_) => return Ok(out),
            QueryEvent::ResultEnd(_) | QueryEvent::Notice(_) => {}
            // A non-suspendable query never suspends; one that does is not a catalog read.
            QueryEvent::Suspended => return Err(MetaError::Unavailable),
        }
    }
}

/// What text conversion needs of a column.
struct Facts {
    logical: LogicalType,
}

impl Facts {
    fn of(schema: &ResultSchema) -> Vec<Facts> {
        schema
            .columns
            .iter()
            .map(|c| Facts {
                logical: c.logical.clone(),
            })
            .collect()
    }
}

/// A cell as text (module docs); `None` for SQL NULL.
fn cell_text(v: ValueRef<'_>, f: &Facts, engine: DbmsKind) -> Result<Option<Arc<str>>, MetaError> {
    let text = match v {
        ValueRef::Null => return Ok(None),
        // The cap is off: a cut value can only be one past 4 GiB, never a catalog fact.
        ValueRef::Truncated(_) => return Err(MetaError::Unavailable),
        ValueRef::Bytes(b) => match std::str::from_utf8(b) {
            Ok(s) => return Ok(Some(Arc::from(s))),
            Err(_) => v,
        },
        other => other,
    };
    let col = ColumnFacts {
        logical: &f.logical,
        native_name: "",
        id_like: false,
        binary: BinaryDisplay::Hex,
        engine,
    };
    Ok(match raw_text(text, &col) {
        RawText::Null => None,
        RawText::Text(t) => Some(Arc::from(t.as_ref())),
    })
}

/// A driver error as a [`MetaError`]: a lost connection, a statement timeout, the server's
/// refusal, a cancel; anything else (an unsupported call) is `Unavailable`.
fn meta_error(kind: DbmsKind, e: &RawError) -> MetaError {
    let class = classify(kind, Phase::PostConnect, e).class;
    match &e.kind {
        RawErrorKind::Cancelled => MetaError::Cancelled,
        RawErrorKind::Db => {
            if connection_ending(kind, e, class).is_some() {
                return MetaError::ConnectionLost;
            }
            let Some(message) = e.server.clone() else {
                return MetaError::Unavailable;
            };
            if is_statement_timeout(kind, &message) {
                return MetaError::Timeout;
            }
            MetaError::Server {
                sqlstate: message.sqlstate.as_deref().map(Arc::from),
                message,
            }
        }
        RawErrorKind::Io(_)
        | RawErrorKind::Protocol
        | RawErrorKind::Timeout
        | RawErrorKind::Tls(_) => MetaError::ConnectionLost,
        _ => MetaError::Unavailable,
    }
}

/// The server's statement-timeout error: PostgreSQL `57014` (no metadata connection is ever
/// cancelled otherwise), MySQL 3024 (`ER_QUERY_TIMEOUT`), MariaDB 1969
/// (`ER_STATEMENT_TIMEOUT`).
fn is_statement_timeout(kind: DbmsKind, m: &ServerMessage) -> bool {
    match kind {
        DbmsKind::Postgres => m.sqlstate.as_deref() == Some("57014"),
        DbmsKind::MySql | DbmsKind::MariaDb => matches!(m.vendor_code, Some(3024 | 1969)),
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------------
// Reads on a session (the user's idle one, or a joined job)

struct OnSession {
    handle: SessionHandle,
    /// A joined `Job(DdlApply | DriftCheck)` transaction (R-M4-29); else the user's session.
    joined: bool,
}

impl OnSession {
    /// Whether the session may be read on now: a joined job inside its transaction, or the
    /// user's session idle outside one.
    fn check_open(&self) -> Result<(), MetaError> {
        self.check(true)
    }

    /// After a read: a joined job still inside its transaction, the user's session still
    /// outside one (it may have taken its next request meanwhile).
    fn check_after(&self) -> Result<(), MetaError> {
        self.check(false)
    }

    fn check(&self, idle: bool) -> Result<(), MetaError> {
        let s = self.handle.state.borrow();
        if !usable(&s.state) {
            return Err(if self.joined {
                MetaError::NotJoinable
            } else {
                MetaError::ConnectionLost
            });
        }
        if self.joined {
            let job = matches!(
                self.handle.kind(),
                SessionKind::Job(_, JobKind::DdlApply | JobKind::DriftCheck)
            );
            if !job || s.tx.status != TxStatus::InTx {
                return Err(MetaError::NotJoinable);
            }
            return Ok(());
        }
        if (idle && s.state != SessionState::Idle)
            || s.tx.status != TxStatus::Idle
            || s.cursor.is_some()
        {
            return Err(MetaError::Busy);
        }
        Ok(())
    }

    /// The user's session: waits while it runs a request; `Busy` while it holds a
    /// transaction or a cursor.
    async fn wait_idle(&self, cancel: &CancellationToken) -> Result<(), MetaError> {
        let mut state = self.handle.state();
        loop {
            {
                let s = state.borrow_and_update();
                if !usable(&s.state) {
                    return Err(MetaError::ConnectionLost);
                }
                if s.tx.status != TxStatus::Idle || s.cursor.is_some() {
                    return Err(MetaError::Busy);
                }
                if s.state == SessionState::Idle {
                    return Ok(());
                }
            }
            cancellable(cancel, state.changed())
                .await?
                .map_err(|_| MetaError::ConnectionLost)?;
        }
    }

    async fn read(&mut self, q: MetaQuery<'_>) -> Result<Rows, MetaError> {
        if self.joined && !q.joinable() {
            return Err(MetaError::NotJoinable);
        }
        check_read_text(q.sql())?;
        self.check_open()?;
        let rows = run_on(&self.handle, q).await?;
        // The user's session must still be outside a transaction (a request queued before this
        // read may have opened one): else the rows are dropped, `Busy`. A joined one must still
        // be inside its own.
        self.check_after()?;
        Ok(rows)
    }
}

fn usable(state: &SessionState) -> bool {
    !matches!(state, SessionState::Disconnected | SessionState::Broken(_))
}

/// Run IDs of metadata reads: the top half of the range, never the run service's.
static NEXT_RUN: AtomicU64 = AtomicU64::new(1 << 63);

/// One statement as a one-unit run on `handle` (the only typed path a session's queue takes),
/// its rows collected.
async fn run_on(handle: &SessionHandle, q: MetaQuery<'_>) -> Result<Rows, MetaError> {
    let run = RunId(NEXT_RUN.fetch_add(1, Ordering::Relaxed));
    let sql: Arc<str> = Arc::from(q.sql());
    let unit = RunUnit {
        id: UnitId { run, ordinal: 0 },
        sql: sql.clone(),
        protocol: Protocol::Extended,
        suspendable: false,
        binds: Arc::from(q.binds().to_vec()),
        verdict: UnitVerdict::Unknown,
        effects: SessionEffects::empty(),
        // Pack text: the binds travel apart and never appear in it.
        masked: sql,
        max_result_sets: 1,
        cell_cap: Some(u32::MAX),
    };
    let collector = Arc::new(Collector::default());
    let spec = RunSpec {
        run,
        units: Arc::from(vec![unit]),
        policy: ErrorPolicy::StopOnError,
        fetch: FetchPolicy {
            cell_cap: u32::MAX,
            ..FetchPolicy::default()
        },
        observer: ObserverRef(collector.clone()),
        confirm_deadline: RunSpec::CONFIRM_DEADLINE,
        context_generation: None,
    };
    match handle.run(spec).reply.await {
        Err(_) => return Err(MetaError::ConnectionLost),
        Ok(Err(e)) => return Err(exec_error(e)),
        Ok(Ok(_)) => {}
    }
    collector.rows()
}

/// A failed run as a [`MetaError`].
fn exec_error(e: ExecError) -> MetaError {
    match e.kind {
        ExecErrorKind::ConnectionLost { .. } | ExecErrorKind::NotConnected => {
            MetaError::ConnectionLost
        }
        ExecErrorKind::Cancelled => MetaError::Cancelled,
        ExecErrorKind::Db => match e.server {
            Some(message) => MetaError::Server {
                sqlstate: message.sqlstate.as_deref().map(Arc::from),
                message,
            },
            None => MetaError::Unavailable,
        },
        ExecErrorKind::ReadOnlyDenied(_) | ExecErrorKind::SafetyDenied(_) => MetaError::RefusedText,
        _ => MetaError::Unavailable,
    }
}

/// Collects a metadata read's one result.
#[derive(Default)]
struct Collector {
    store: Mutex<Option<Arc<ResultStore>>>,
}

impl RunObserver for Collector {
    fn open_result(
        &self,
        _: UnitId,
        ordinal: u32,
        schema: Arc<ResultSchema>,
    ) -> hacivat_results::ResultWriter {
        // Its own budget, as large as a result may be (R-M3-19's per-result cap is the grid's).
        let budget = MemoryBudget::new(4 << 30, 4 << 30);
        let (store, writer) = ResultStore::new(ResultId(u64::from(ordinal)), schema, budget, 0);
        *self.store.lock().unwrap_or_else(PoisonError::into_inner) = Some(store);
        writer
    }

    fn event(&self, _ev: &RunEvent) {}
}

impl Collector {
    fn rows(&self) -> Result<Rows, MetaError> {
        let store = self
            .store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        // A statement without columns opened no result.
        let Some(store) = store else {
            return Ok(Rows::default());
        };
        let snap = store.snapshot();
        if *snap.completeness() != Completeness::Complete {
            return Err(MetaError::Unavailable);
        }
        let schema = snap.schema().clone();
        let facts = Facts::of(&schema);
        let mut out = Rows {
            columns: schema.columns.iter().map(|c| c.name.clone()).collect(),
            rows: Vec::with_capacity(snap.row_count() as usize),
        };
        for r in 0..snap.row_count() {
            let mut row = Vec::with_capacity(facts.len());
            for (c, f) in facts.iter().enumerate() {
                row.push(cell_text(snap.cell(r, c as u32), f, DbmsKind::Unknown)?);
            }
            out.rows.push(row);
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------------------------
// The read-text allowlist

/// The first keywords a read on a session (joined or the user's) may have.
const READ_KEYWORDS: [&str; 5] = ["SELECT", "WITH", "SHOW", "TABLE", "VALUES"];

/// Words a read never holds anywhere (a data-modifying `WITH`, `SELECT … INTO`, `FOR UPDATE`).
const WRITE_WORDS: [&str; 5] = ["INSERT", "UPDATE", "DELETE", "MERGE", "INTO"];

/// The allowlist of text read on a session where the server enforces nothing (module docs):
/// `Err(RefusedText)` unless `sql` is one statement whose first keyword, after comments, is a
/// [`READ_KEYWORDS`] word; it never holds a MySQL executable comment, a nested block comment
/// before that keyword, a `;` but at the very end, or a [`WRITE_WORDS`] word.
pub fn check_read_text(sql: &str) -> Result<(), MetaError> {
    let refused = Err(MetaError::RefusedText);
    let upper = sql.to_ascii_uppercase();
    if upper.contains("/*!") || upper.contains("/*M!") {
        return refused;
    }
    // One statement: a `;` only as the last character that is not whitespace.
    let trimmed = sql.trim_end();
    let body = trimmed.strip_suffix(';').unwrap_or(trimmed);
    if body.contains(';') {
        return refused;
    }
    if words(&upper).any(|w| WRITE_WORDS.contains(&w)) {
        return refused;
    }
    let Some(rest) = skip_leading_comments(body) else {
        return refused;
    };
    let end = rest
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    let (keyword, after) = rest.split_at(end);
    if after.starts_with(|c: char| c.is_ascii_digit() || c == '_' || c == '$') {
        return refused;
    }
    if READ_KEYWORDS
        .iter()
        .any(|k| keyword.eq_ignore_ascii_case(k))
    {
        Ok(())
    } else {
        refused
    }
}

/// `sql` after its leading whitespace and comments; `None` for an unterminated or nested block
/// comment (PostgreSQL nests them, MySQL does not: either reading could hide a keyword).
fn skip_leading_comments(sql: &str) -> Option<&str> {
    let mut rest = sql.trim_start();
    loop {
        if let Some(line) = rest.strip_prefix("--") {
            rest = line
                .split_once('\n')
                .map_or("", |(_, after)| after)
                .trim_start();
        } else if let Some(block) = rest.strip_prefix("/*") {
            let (inside, after) = block.split_once("*/")?;
            if inside.contains("/*") {
                return None;
            }
            rest = after.trim_start();
        } else {
            return Some(rest);
        }
    }
}

/// The words of `text`: runs of ASCII letters, digits, `_` and `$`.
fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
        .filter(|w| !w.is_empty())
}

// ---------------------------------------------------------------------------------------------
// Helpers

/// A server we know nothing of but its kind.
fn unknown_server(kind: DbmsKind) -> ServerInfo {
    ServerInfo {
        kind,
        product: String::new(),
        version: None,
        raw_version: String::new(),
        is_replica: None,
    }
}

/// A text bind.
fn text_bind(s: &str) -> BindValue {
    BindValue::Typed {
        hint: LogicalType::Text {
            max_chars: None,
            fixed: false,
        },
        text: Arc::from(s),
    }
}

/// The first cell of the first row.
fn first_cell(rows: &Rows) -> Option<&str> {
    rows.rows.first()?.first()?.as_deref()
}

/// `fut`, unless `cancel` comes first (`Cancelled`).
async fn cancellable<T>(
    cancel: &CancellationToken,
    fut: impl Future<Output = T>,
) -> Result<T, MetaError> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(MetaError::Cancelled),
        out = fut => Ok(out),
    }
}

/// `fut`, unless the profile disconnects (or the `MetaConn` drops) first (`Unavailable`).
async fn guarded<T>(
    closed: &CancellationToken,
    fut: impl Future<Output = T>,
) -> Result<T, MetaError> {
    tokio::select! {
        biased;
        () = closed.cancelled() => Err(MetaError::Unavailable),
        out = fut => Ok(out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_query_keeps_what_it_was_given() {
        let binds = [BindValue::Null];
        let q = MetaQuery::new("SELECT 1 WHERE $1 = $1", &binds, true);
        assert_eq!(q.sql(), "SELECT 1 WHERE $1 = $1");
        assert_eq!(q.binds().len(), 1);
        assert!(q.joinable());
    }

    // ## m4-t05-meta
    #[test]
    fn the_allowlist_takes_reads_only() {
        for ok in [
            "SELECT 1",
            "  select relname FROM pg_catalog.pg_class",
            "-- hv:pg/Identity\nSELECT 1",
            "/* hv:pg/Identity */ SELECT 1",
            "/* a */ -- b\n WITH x AS (SELECT 1) SELECT * FROM x",
            "SHOW CREATE TABLE `t`",
            "TABLE pg_catalog.pg_namespace",
            "VALUES (1)",
            "SELECT 1;",
            "SELECT 1 ;  \n",
            "SELECT update_rule, delete_rule FROM information_schema.referential_constraints",
            "SELECT(1)",
        ] {
            assert_eq!(check_read_text(ok), Ok(()), "{ok}");
        }
    }

    #[test]
    fn the_allowlist_refuses_everything_else() {
        for refused in [
            "",
            "   ",
            "-- only a comment",
            "/* unterminated SELECT 1",
            "BEGIN",
            "begin",
            " COMMIT",
            "SET search_path = x",
            "SAVEPOINT a",
            "RELEASE a",
            "ROLLBACK",
            "START TRANSACTION",
            "END",
            "ABORT",
            "XA START 'x'",
            "PREPARE TRANSACTION 'x'",
            "DO $$ BEGIN END $$",
            "CALL p()",
            "PRAGMA table_list",
            "SELECT 1; SELECT 2",
            "SELECT 1; DELETE FROM t",
            "SELECT 1 /*! ; DROP TABLE t */",
            "SELECT /*!50000 1 */",
            "SELECT /*M!100000 1 */",
            "/* /* */ SELECT 1 */ DELETE FROM t",
            "WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d",
            "SELECT * INTO new_t FROM t",
            "SELECT * FROM t FOR UPDATE",
            "SELECT 1 INTO OUTFILE '/tmp/x'",
            "SELECTX 1",
            "SELECT_1",
            "(SELECT 1)",
            "# SELECT\nDELETE FROM t",
            "UPDATE t SET a = 1",
            "MERGE INTO t USING s ON true WHEN MATCHED THEN DELETE",
        ] {
            assert_eq!(
                check_read_text(refused),
                Err(MetaError::RefusedText),
                "{refused:?}"
            );
        }
    }

    #[test]
    fn statement_timeouts_are_told_apart_per_engine() {
        let msg = |sqlstate: Option<&str>, code: Option<i64>| ServerMessage {
            sqlstate: sqlstate.map(str::to_string),
            vendor_code: code,
            severity: Some("ERROR".into()),
            text: String::new(),
            detail: None,
            hint: None,
            lang_hint: None,
            position: None,
        };
        let db = |m: ServerMessage| RawError {
            kind: RawErrorKind::Db,
            server: Some(m),
        };
        assert_eq!(
            meta_error(DbmsKind::Postgres, &db(msg(Some("57014"), None))),
            MetaError::Timeout
        );
        assert_eq!(
            meta_error(DbmsKind::MySql, &db(msg(Some("HY000"), Some(3024)))),
            MetaError::Timeout
        );
        assert_eq!(
            meta_error(DbmsKind::MariaDb, &db(msg(Some("70100"), Some(1969)))),
            MetaError::Timeout
        );
        assert!(matches!(
            meta_error(DbmsKind::Postgres, &db(msg(Some("25006"), None))),
            MetaError::Server { sqlstate: Some(s), .. } if &*s == "25006"
        ));
        assert_eq!(
            meta_error(
                DbmsKind::MySql,
                &RawError {
                    kind: RawErrorKind::Io(std::io::ErrorKind::ConnectionReset),
                    server: None
                }
            ),
            MetaError::ConnectionLost
        );
    }

    #[test]
    fn mariadb_reads_tx_read_only_before_11_1() {
        assert_eq!(
            read_only_probe(Flavor::MariaDb {
                transaction_read_only: false
            }),
            "SELECT @@session.tx_read_only"
        );
        assert_eq!(
            read_only_probe(Flavor::MariaDb {
                transaction_read_only: true
            }),
            "SELECT @@session.transaction_read_only"
        );
        assert_eq!(
            read_only_probe(Flavor::MySql),
            "SELECT @@session.transaction_read_only"
        );
    }
}
