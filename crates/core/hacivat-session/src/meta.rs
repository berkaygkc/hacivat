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
//!   - MySQL and MariaDB: `SET NAMES utf8mb4`, `SET SESSION TRANSACTION READ ONLY`,
//!     `SET time_zone = '+00:00'` and the statement timeout (MySQL `max_execution_time`,
//!     SELECTs only; MariaDB `max_statement_time`). Before every snapshot the session's
//!     read-only variable must read 1 (`@@session.transaction_read_only`; MariaDB before 11.1
//!     `@@session.tx_read_only`) and `character_set_client`, `character_set_connection` and
//!     `character_set_results` must read `utf8mb4`, else [`MetaError::ReadOnlyNotEnforced`];
//!     then `START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY`.
//!   - SQLite: the main file opened read-only (`SQLITE_OPEN_READ_ONLY`, the flag form of
//!     `mode=ro`) straight through the driver, without the user-facing authorizer (R-M4-38);
//!     every snapshot is a deferred `BEGIN`. Attached files go through
//!     [`MetaConn::attach_read_only`]: SQLite opens them with the main file's flags.
//!
//!   Every text passes the same token guard as on a session ([`check_own_text`]: the read
//!   allowlist, plus a reading `PRAGMA` on SQLite), so neither a write, a lock nor a known
//!   side-effect function reaches the connection; MySQL and MariaDB snapshots also send
//!   `SET TRANSACTION ISOLATION LEVEL REPEATABLE READ` first. A lost connection answers
//!   [`MetaError::ConnectionLost`]
//!   and is dropped; nothing is retried inside a snapshot. A snapshot whose transaction ended
//!   under it (a MySQL deadlock) answers `ConnectionLost` too. The next
//!   [`MetaConn::begin_snapshot`] connects again and runs the set-up (and the MySQL assertion)
//!   again. A snapshot dropped without its commit is rolled back at once, on the runtime. The
//!   connection closes when the `MetaConn` drops, and when the profile disconnects
//!   (`close_all`), whichever comes first; its permit goes back only after the close.
//! - **The user's session** (single-session profiles, SQLite `:memory:`, and SQLite's `temp`
//!   schema, alg/02 HV-INTRO-08 step 4). [`SessionManager::meta_conn`] waits while the
//!   default session runs a request and answers [`MetaError::Busy`] while it holds a
//!   transaction or a cursor (alg/02 HV-INTRO-01 edge 10). Every read is a
//!   `Request::MetaRead` (ruling R-W1-9): the actor checks the session when it takes the
//!   request, in the same step that runs it, and refuses (`Busy`, nothing sent) unless the
//!   transaction is idle with no cursor (MySQL: no `LOCK TABLES`, `autocommit` on, and the
//!   connection's character sets `utf8mb4`). Otherwise it runs the read inside Hacivat's own
//!   read-only transaction and rolls it back: PostgreSQL `BEGIN ISOLATION LEVEL REPEATABLE
//!   READ READ ONLY`, `SET LOCAL search_path = pg_catalog, pg_temp`, `SET LOCAL
//!   statement_timeout`; MySQL and MariaDB `START TRANSACTION WITH CONSISTENT SNAPSHOT, READ
//!   ONLY`; SQLite a deferred `BEGIN` (none on a read-only profile's connection, whose
//!   authorizer refuses `BEGIN` and whose engine is read-only already). It never goes through
//!   the user's run path, the transaction modes, the verdicts, the history or the activity,
//!   and never moves the console's run context.
//! - **A joined session** ([`MetaConn::join`], R-M4-29): a `Request::MetaRead` the actor runs
//!   only while the `Job(DdlApply | DriftCheck)` session is inside its transaction; nothing but
//!   the read is sent.
//!
//! On the last two the server enforces nothing for the joined path, and the user's session is
//! shared, so every text must also pass [`check_read_text_for`] (the carries' allowlist on
//! tokens of the project's lexer; check-deps is defence in depth, R-W0-2), in the actor.
//!
//! Every value comes back as text: text, JSON and XML as the server sent them, bytes as UTF-8
//! when they are, any other value as `hacivat_values::valfmt::raw_text` writes it (packs that
//! need the server's own spelling cast to text). The cell cap is off.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_driver::{
    DriverConnect, Endpoint, ExecSummary, InsecureAuth, Phase, Protocol, QueryEvent, QuerySpec,
    RawConn, RawError, RawErrorKind, ServerInfo, ServerMessage, TxStatus, classify,
};
use hacivat_proto::{DbmsKind, ErrClass, ProfileId};
use hacivat_values::valfmt::{BinaryDisplay, ColumnFacts, RawText, raw_text};
use hacivat_values::{BindValue, LogicalType, ResultSchema, ValueRef};
use sql_dialect::{DialectCtx, DialectId, LexModes, ServerVersion, lex_profile};
use sql_lexer::{LexState, Lexed, TokenKind, lex};
use tokio::runtime::Handle;
use tokio::sync::{OwnedMutexGuard, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::budget::BudgetPermit;
use crate::pipeline::LiveConnection;
use crate::profile::ProfileEndpoint;
use crate::session::actor::Core;
use crate::session::recovery::connection_ending;
use crate::session::schema::{Family, family};
use crate::session::{
    DenyLayer, ExecError, ExecErrorKind, JobKind, MetaRead, Reply, Request, SessionHandle,
    SessionKind, SessionManager, SessionState, TxError,
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

/// The statement timeout of a joined read (the job's transaction has its own deadline).
const JOINED_TIMEOUT: Duration = Duration::from_secs(60);

/// The per-database cap of a profile's metadata connections (1 by default, at most
/// [`META_POOL_MAX`]); a request over the cap waits for a connection of the same database to
/// drop. `None` is the profile's database (review A M3: one key for one database).
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

    // ## m4-t15b-facets (named exception in Task 5's file; T15b CI round)
    /// Whether the connection reads inside a caller's Hacivat-owned transaction
    /// ([`MetaConn::join`], [`MetaConn::join_as`]): a reader whose sections are not joinable
    /// refuses it at once instead of answering nothing.
    pub fn is_joined(&self) -> bool {
        self.joined
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
    /// (`TxStatus::InTx`): a job session has no user, so its transaction is Hacivat's (the
    /// session's kind decides, not `TxState::owner`: a job begins its transaction with its own
    /// statements). Every other kind (`Console`, `Editor`, `Introspection`, `Service`, an
    /// export or import job) and a job outside a transaction answer
    /// [`MetaError::NotJoinable`]; the actor checks again when it takes each read. The SQLite
    /// writer while an M7 apply holds it has no seam yet: it is refused too (M7 adds it).
    pub fn join(session: &SessionHandle) -> Result<MetaConn, MetaError> {
        MetaConn::join_as(session, DbmsKind::Unknown)
    }

    // ## m4-t15b-facets (named exception in Task 5's file; T15b review m-1)
    /// [`MetaConn::join`] for a reader that expects the session to be of engine `engine`: every
    /// read on another engine's session is refused (`NotJoinable`) by the session's actor
    /// before anything is sent (`Unknown`: any engine, as `join`). The connection still reports
    /// `DbmsKind::Unknown` (it knows no server of its own).
    pub fn join_as(session: &SessionHandle, engine: DbmsKind) -> Result<MetaConn, MetaError> {
        let on = OnSession {
            handle: session.clone(),
            joined: true,
            timeout: JOINED_TIMEOUT,
            dbms: engine,
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
            own.read_outside("ATTACH DATABASE ? AS ?", &binds)
                .await
                .map(drop)
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
                .read_outside("SELECT pg_current_xact_id_if_assigned() IS NOT NULL", &[])
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
/// Dropped without a commit, the snapshot is rolled back at once on the runtime (review B I4:
/// no read transaction, and no lock, outlives it).
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
                Via::Own(own) => own.read_in_snapshot(q.sql(), q.binds(), true).await,
                Via::Session(s) => s.read(q).await,
            }
        })
    }

    // ## m4-t05-meta (fix round 2, review B R-M3; round 3: gated by `test-support`)
    /// [`rows`](Self::rows) without the token guard, on an own connection only: the Docker
    /// tests send writes through it to prove that the **server** refuses them (`25006`,
    /// `1792`, `SQLITE_READONLY`). It exists only in a debug build with the crate's
    /// `test-support` feature, which only dev-dependencies enable (`hacivat-exec`'s, and
    /// `hacivat-app`'s self dev-dependency): no product build has it, debug or release. It
    /// still takes only a [`MetaQuery`], which only packs and tests can build, and
    /// `tests/meta_conn.rs` checks that only `tests/` files name it. A session path answers
    /// [`MetaError::Unavailable`].
    #[cfg(all(debug_assertions, feature = "test-support"))]
    #[doc(hidden)]
    pub fn rows_unguarded<'q>(
        &'q mut self,
        q: MetaQuery<'q>,
    ) -> BoxFuture<'q, Result<Rows, MetaError>> {
        Box::pin(async move {
            let Some(conn) = self.conn.as_deref_mut() else {
                return Err(MetaError::Unavailable);
            };
            match &mut conn.via {
                Via::Own(own) => own.read_in_snapshot(q.sql(), q.binds(), false).await,
                Via::Offline | Via::Session(_) => Err(MetaError::Unavailable),
            }
        })
    }

    /// Ends the snapshot (a joined connection or the user's session sends nothing: each of
    /// their reads was its own transaction step).
    pub fn commit(mut self) -> BoxFuture<'c, Result<(), MetaError>> {
        let conn = self.conn.take();
        Box::pin(async move {
            let Some(conn) = conn else {
                return Err(MetaError::Unavailable);
            };
            match &mut conn.via {
                Via::Offline => Err(MetaError::Unavailable),
                Via::Own(own) => own.commit().await,
                Via::Session(_) => Ok(()),
            }
        })
    }
}

impl Drop for SnapshotTxn<'_> {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take()
            && let Via::Own(own) = &mut conn.via
        {
            own.abandon();
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
            let timeout = spec.statement_timeout;
            match profile.engine {
                DbmsKind::Sqlite if memory || spec.database.as_deref() == Some("temp") => {
                    self.meta_on_user_session(profile.engine, timeout, cancel)
                        .await
                }
                DbmsKind::Sqlite => self.meta_sqlite(spec, cancel).await,
                DbmsKind::Postgres | DbmsKind::MySql | DbmsKind::MariaDb => {
                    if self.meta_single() {
                        self.meta_on_user_session(profile.engine, timeout, cancel)
                            .await
                    } else {
                        let spec = MetaConnSpec {
                            database: spec.database.or_else(|| profile.database.clone()),
                            ..spec
                        };
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
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<MetaConn, MetaError> {
        let Some(handle) = self.default_session() else {
            return Err(MetaError::Unavailable);
        };
        let on = OnSession {
            handle,
            joined: false,
            timeout,
            dbms: engine,
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
        // pipeline's connect, or is seen right after it.
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
        let cell = Arc::new(tokio::sync::Mutex::new(Slot {
            conn: Some(conn),
            snapshot: None,
            in_flight: false,
        }));
        let watched = cell.clone();
        let until = closed.clone();
        rt.spawn(async move {
            until.cancelled().await;
            let taken = watched.lock().await.conn.take();
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
                next_snapshot: 0,
                open: None,
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

/// The connection and its state, shared by the `MetaConn`, its watcher and a dropped
/// snapshot's rollback.
struct Slot {
    /// `None` once lost (the next snapshot connects again) or closed.
    conn: Option<Conn>,
    /// The snapshot whose transaction is open on the connection.
    snapshot: Option<u64>,
    /// A statement's future was dropped before it ended: the connection's state is unknown.
    in_flight: bool,
}

impl Slot {
    /// Drops the connection (closed in the background) and what it held.
    fn discard(&mut self, rt: &Handle) {
        if let Some(conn) = self.conn.take() {
            rt.spawn(conn.close());
        }
        self.snapshot = None;
        self.in_flight = false;
    }
}

struct Own {
    cell: Arc<tokio::sync::Mutex<Slot>>,
    /// Cancelled by `close_all` (its parent) or by the `MetaConn`'s drop.
    closed: CancellationToken,
    manager: Weak<SessionManager>,
    target: Target,
    flavor: Flavor,
    rt: Handle,
    next_snapshot: u64,
    /// The snapshot a live `SnapshotTxn` holds.
    open: Option<u64>,
}

impl Own {
    /// The connection, locked; a lost one is connected again when `reconnect`.
    async fn conn(&mut self, reconnect: bool) -> Result<OwnedMutexGuard<Slot>, MetaError> {
        if self.closed.is_cancelled() {
            return Err(MetaError::Unavailable);
        }
        let mut slot = self.cell.clone().lock_owned().await;
        if slot.in_flight {
            // A dropped statement future leaves the connection mid-statement: never reused.
            slot.discard(&self.rt);
        }
        if slot.conn.is_none() {
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
            slot.conn = Some(fresh);
            slot.snapshot = None;
        }
        Ok(slot)
    }

    fn kind(&self) -> DbmsKind {
        flavor_kind(self.flavor)
    }

    /// After an operation: a lost connection is dropped (closed in the background).
    fn settle<T>(&self, slot: &mut Slot, done: Result<T, MetaError>) -> Result<T, MetaError> {
        if matches!(done, Err(MetaError::ConnectionLost)) {
            slot.discard(&self.rt);
        }
        done
    }

    /// Trusted control text, unless the profile disconnects first.
    async fn control(&self, slot: &mut Slot, sql: &str) -> Result<(), MetaError> {
        let kind = self.kind();
        let Some(conn) = slot.conn.as_mut() else {
            return Err(MetaError::ConnectionLost);
        };
        slot.in_flight = true;
        let done = guarded(&self.closed, conn.raw().batch(sql))
            .await
            .and_then(|r| r.map_err(|e| meta_error(kind, &e)));
        slot.in_flight = false;
        self.settle(slot, done)
    }

    /// One typed read, unless the profile disconnects first.
    async fn query(
        &self,
        slot: &mut Slot,
        sql: &str,
        binds: &[BindValue],
    ) -> Result<Rows, MetaError> {
        let kind = self.kind();
        let Some(conn) = slot.conn.as_mut() else {
            return Err(MetaError::ConnectionLost);
        };
        slot.in_flight = true;
        let done = guarded(&self.closed, read_rows(conn.raw(), kind, sql, binds))
            .await
            .and_then(|r| r.map_err(|e| e.meta(kind)));
        slot.in_flight = false;
        self.settle(slot, done)
    }

    /// The transaction status the driver reports after the last round trip.
    fn tx(slot: &mut Slot) -> TxStatus {
        slot.conn
            .as_mut()
            .map_or(TxStatus::Idle, |c| c.raw().tx_status())
    }

    async fn begin(&mut self) -> Result<(), MetaError> {
        let mut slot = self.conn(true).await?;
        // A snapshot left open (its rollback not run yet), or a begin cut short: never read on
        // in it.
        if slot.snapshot.is_some() || Self::tx(&mut slot) != TxStatus::Idle {
            self.control(&mut slot, "ROLLBACK").await?;
            slot.snapshot = None;
        }
        match self.flavor {
            Flavor::Pg => {
                self.control(&mut slot, "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
                    .await?;
                self.control(&mut slot, "SET LOCAL search_path = pg_catalog, pg_temp")
                    .await?;
            }
            Flavor::MySql | Flavor::MariaDb { .. } => {
                let rows = self
                    .query(&mut slot, mysql_session_probe(self.flavor), &[])
                    .await?;
                if !mysql_session_confirmed(&rows) {
                    return Err(MetaError::ReadOnlyNotEnforced);
                }
                // Fix round 2 (R-W1-13): the consistent snapshot needs REPEATABLE READ, whatever
                // the session's isolation; `SET TRANSACTION` covers this one transaction only.
                self.control(&mut slot, "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
                    .await?;
                self.control(
                    &mut slot,
                    "START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY",
                )
                .await?;
            }
            Flavor::Sqlite => self.control(&mut slot, "BEGIN").await?,
        }
        if Self::tx(&mut slot) != TxStatus::InTx {
            // Whatever opened, it is not the read-only transaction this snapshot needs.
            slot.discard(&self.rt);
            return Err(MetaError::ReadOnlyNotEnforced);
        }
        self.next_snapshot += 1;
        slot.snapshot = Some(self.next_snapshot);
        self.open = Some(self.next_snapshot);
        Ok(())
    }

    /// A read of the open snapshot: refused when its text fails the token guard
    /// ([`check_own_text`]; `guard` false only for the debug-build `rows_unguarded`), and
    /// `ConnectionLost` (the connection dropped) when the transaction ended under it.
    async fn read_in_snapshot(
        &mut self,
        sql: &str,
        binds: &[BindValue],
        guard: bool,
    ) -> Result<Rows, MetaError> {
        if guard {
            check_own_text(sql, self.kind())?;
        }
        let mut slot = self.conn(false).await?;
        if slot.snapshot.is_none() || slot.snapshot != self.open {
            return Err(MetaError::ConnectionLost);
        }
        let rows = self.query(&mut slot, sql, binds).await?;
        if Self::tx(&mut slot) != TxStatus::InTx {
            // A MySQL deadlock or `innodb_rollback_on_timeout` ended it (review A M5).
            slot.discard(&self.rt);
            return Err(MetaError::ConnectionLost);
        }
        Ok(rows)
    }

    /// A read outside a snapshot (Hacivat's own text: `ATTACH`, the XID probe).
    async fn read_outside(&mut self, sql: &str, binds: &[BindValue]) -> Result<Rows, MetaError> {
        let mut slot = self.conn(false).await?;
        self.query(&mut slot, sql, binds).await
    }

    async fn commit(&mut self) -> Result<(), MetaError> {
        let open = self.open.take();
        let mut slot = self.conn(false).await?;
        if slot.snapshot.is_none() || slot.snapshot != open {
            return Err(MetaError::ConnectionLost);
        }
        slot.snapshot = None;
        self.control(&mut slot, "COMMIT").await?;
        if Self::tx(&mut slot) != TxStatus::Idle {
            slot.discard(&self.rt);
            return Err(MetaError::ReadOnlyNotEnforced);
        }
        Ok(())
    }

    /// A snapshot dropped without its commit: rolled back at once on the runtime (or the
    /// connection closed, when a statement of it was cut short or the rollback fails). A
    /// later snapshot that took the connection first already rolled it back.
    fn abandon(&mut self) {
        let Some(id) = self.open.take() else {
            return;
        };
        let cell = self.cell.clone();
        let rt = self.rt.clone();
        let closed = self.closed.clone();
        self.rt.spawn(async move {
            let mut slot = cell.lock().await;
            if slot.snapshot != Some(id) {
                return;
            }
            slot.snapshot = None;
            if slot.in_flight || closed.is_cancelled() {
                slot.discard(&rt);
                return;
            }
            let Some(conn) = slot.conn.as_mut() else {
                return;
            };
            let rolled = conn.raw().batch("ROLLBACK").await;
            let idle = conn.raw().tx_status() == TxStatus::Idle;
            if rolled.is_err() || !idle {
                slot.discard(&rt);
            }
        });
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

/// The MySQL/MariaDB statement that reads the session's read-only flag (1) and its three
/// character sets (`utf8mb4`, the T4 carry).
fn mysql_session_probe(flavor: Flavor) -> &'static str {
    match flavor {
        Flavor::MariaDb {
            transaction_read_only: false,
        } => {
            "SELECT @@session.tx_read_only, @@session.character_set_client, \
             @@session.character_set_connection, @@session.character_set_results"
        }
        _ => {
            "SELECT @@session.transaction_read_only, @@session.character_set_client, \
             @@session.character_set_connection, @@session.character_set_results"
        }
    }
}

/// The probe answered read-only and `utf8mb4` three times.
fn mysql_session_confirmed(rows: &Rows) -> bool {
    let Some(row) = rows.rows.first() else {
        return false;
    };
    let cell = |i: usize| row.get(i).and_then(|c| c.as_deref());
    cell(0) == Some("1") && (1..4).all(|i| cell(i) == Some("utf8mb4"))
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
            "SET NAMES utf8mb4".into(),
            "SET SESSION TRANSACTION READ ONLY".into(),
            "SET time_zone = '+00:00'".into(),
            format!("SET SESSION max_execution_time = {millis}"),
        ],
        Flavor::MariaDb { .. } => vec![
            "SET NAMES utf8mb4".into(),
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

/// Why a typed read failed.
enum ReadFail {
    Raw(RawError),
    /// The answer had no shape a catalog read has (a suspension, a cut cell).
    Shape,
}

impl ReadFail {
    fn meta(self, kind: DbmsKind) -> MetaError {
        match self {
            ReadFail::Raw(e) => meta_error(kind, &e),
            ReadFail::Shape => MetaError::Unavailable,
        }
    }
}

/// One typed query with binds, its rows as text (the cell cap off).
async fn read_rows(
    raw: &mut dyn RawConn,
    kind: DbmsKind,
    sql: &str,
    binds: &[BindValue],
) -> Result<Rows, ReadFail> {
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
    raw.query(spec).await.map_err(ReadFail::Raw)?;
    let mut out = Rows::default();
    let mut facts: Vec<Facts> = Vec::new();
    loop {
        match raw.next_event().await.map_err(ReadFail::Raw)? {
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
            QueryEvent::Suspended => return Err(ReadFail::Shape),
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
fn cell_text(v: ValueRef<'_>, f: &Facts, engine: DbmsKind) -> Result<Option<Arc<str>>, ReadFail> {
    let text = match v {
        ValueRef::Null => return Ok(None),
        // The cap is off: a cut value can only be one past 4 GiB, never a catalog fact.
        ValueRef::Truncated(_) => return Err(ReadFail::Shape),
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
            server_error(kind, message)
        }
        RawErrorKind::Io(_)
        | RawErrorKind::Protocol
        | RawErrorKind::Timeout
        | RawErrorKind::Tls(_) => MetaError::ConnectionLost,
        _ => MetaError::Unavailable,
    }
}

/// The server's refusal, or its statement timeout.
fn server_error(kind: DbmsKind, message: ServerMessage) -> MetaError {
    if is_statement_timeout(kind, &message) {
        return MetaError::Timeout;
    }
    MetaError::Server {
        sqlstate: message.sqlstate.as_deref().map(Arc::from),
        message,
    }
}

/// The server's statement-timeout error: PostgreSQL `57014` (no metadata read is ever
/// cancelled otherwise), MySQL 3024 (`ER_QUERY_TIMEOUT`), MariaDB 1969
/// (`ER_STATEMENT_TIMEOUT`). An unknown engine (a joined read) takes any of them.
fn is_statement_timeout(kind: DbmsKind, m: &ServerMessage) -> bool {
    let pg = m.sqlstate.as_deref() == Some("57014");
    let my = matches!(m.vendor_code, Some(3024 | 1969));
    match kind {
        DbmsKind::Postgres => pg,
        DbmsKind::MySql | DbmsKind::MariaDb => my,
        DbmsKind::Unknown => pg || my,
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------------
// Reads on a session (the user's idle one, or a joined job): `Request::MetaRead`

struct OnSession {
    handle: SessionHandle,
    /// A joined `Job(DdlApply | DriftCheck)` transaction (R-M4-29); else the user's session.
    joined: bool,
    /// The user's path: the server-side limit of each read (PostgreSQL).
    timeout: Duration,
    /// The engine, when known (the user's path).
    dbms: DbmsKind,
}

impl OnSession {
    /// Whether the session may be read on now (a fast answer; the actor decides when it takes
    /// the read): a joined job inside its transaction, or the user's session idle outside one.
    fn check_open(&self) -> Result<(), MetaError> {
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
        if s.state != SessionState::Idle || s.tx.status != TxStatus::Idle || s.cursor.is_some() {
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

    /// One read as a `Request::MetaRead`: the actor checks the session and the text when it
    /// takes it, and runs it in one step.
    async fn read(&mut self, q: MetaQuery<'_>) -> Result<Rows, MetaError> {
        if self.joined && !q.joinable() {
            return Err(MetaError::NotJoinable);
        }
        self.check_open()?;
        let read = MetaRead {
            sql: Arc::from(q.sql()),
            binds: Arc::from(q.binds().to_vec()),
            joined: self.joined,
            timeout: self.timeout,
            // ## m4-t15b-facets (named exception; T15b review m-1)
            engine: if self.joined {
                self.dbms
            } else {
                DbmsKind::Unknown
            },
        };
        let reply = self
            .handle
            .submit(Request::MetaRead(read))
            .reply
            .await
            .map_err(|_| MetaError::ConnectionLost)?;
        match reply {
            Ok(summary) => Ok(rows_of(summary)),
            Err(e) => Err(meta_read_error(e, self.joined, self.dbms)),
        }
    }
}

fn usable(state: &SessionState) -> bool {
    !matches!(state, SessionState::Disconnected | SessionState::Broken(_))
}

/// What a refused or failed `Request::MetaRead` means (the actor's refusals use existing
/// kinds: `Tx(InProgress)` busy, `Tx(Unsupported)` not joinable, `ReadOnlyDenied(Input)` the
/// text, any other `ReadOnlyDenied` the read-only transaction or the character sets).
fn meta_read_error(e: ExecError, joined: bool, dbms: DbmsKind) -> MetaError {
    match e.kind {
        ExecErrorKind::Tx(TxError::InProgress) | ExecErrorKind::QueueFull => {
            if joined {
                MetaError::NotJoinable
            } else {
                MetaError::Busy
            }
        }
        ExecErrorKind::Tx(_) => MetaError::NotJoinable,
        ExecErrorKind::ReadOnlyDenied(DenyLayer::Input) => MetaError::RefusedText,
        ExecErrorKind::ReadOnlyDenied(_) => MetaError::ReadOnlyNotEnforced,
        ExecErrorKind::ConnectionLost { .. } | ExecErrorKind::NotConnected => {
            MetaError::ConnectionLost
        }
        ExecErrorKind::Cancelled => MetaError::Cancelled,
        ExecErrorKind::Db => match e.server {
            Some(message) => server_error(dbms, message),
            None => MetaError::Unavailable,
        },
        _ => MetaError::Unavailable,
    }
}

/// The text rows of a `MetaRead`'s reply.
fn rows_of(s: ExecSummary) -> Rows {
    Rows {
        columns: s.columns.into_iter().map(Arc::from).collect(),
        rows: s
            .rows
            .into_iter()
            .map(|r| r.into_iter().map(|c| c.map(Arc::from)).collect())
            .collect(),
    }
}

/// The reply of a `MetaRead`: its rows as text.
fn summary_of(rows: Rows) -> ExecSummary {
    let n = rows.rows.len() as u64;
    ExecSummary {
        columns: rows.columns.iter().map(|c| c.to_string()).collect(),
        rows: rows
            .rows
            .into_iter()
            .map(|r| r.into_iter().map(|c| c.map(|c| c.to_string())).collect())
            .collect(),
        row_count: n,
        ..ExecSummary::default()
    }
}

// ## m4-t05-meta (named exception, ruling R-W1-9): the actor's half of `Request::MetaRead`,
// called by `Core::handle` when it takes the request.
impl Core {
    pub(crate) async fn meta_read(&mut self, r: &MetaRead) -> Reply {
        // ## m4-t15b-facets (named exception; T15b review m-1): a reader that names another
        // engine than the session's gets nothing sent.
        if r.engine != DbmsKind::Unknown && r.engine != self.kind {
            return Err(ExecError::of(ExecErrorKind::Tx(TxError::Unsupported)));
        }
        // The text first, with the session's own engine: nothing is sent for refused text.
        if check_read_text_for(&r.sql, self.kind).is_err() {
            return Err(ExecError::of(ExecErrorKind::ReadOnlyDenied(
                DenyLayer::Input,
            )));
        }
        let raw_tx = self.conn()?.tx_status();
        if r.joined {
            let open = self.meta_joinable
                && self.tx.status == TxStatus::InTx
                && raw_tx == TxStatus::InTx
                && self.cursor.is_none();
            if !open {
                return Err(ExecError::of(ExecErrorKind::Tx(TxError::Unsupported)));
            }
            let read = self.meta_select(r).await;
            self.observe(false);
            return read;
        }
        let fam = family(self.kind);
        let autocommit = fam != Family::MySql
            || (self.conn()?.autocommit() == Some(true) && !self.autocommit_off);
        let idle = self.tx.status == TxStatus::Idle
            && raw_tx == TxStatus::Idle
            && self.cursor.is_none()
            && !self.table_locks
            && autocommit;
        if !idle {
            return Err(ExecError::of(ExecErrorKind::Tx(TxError::InProgress)));
        }
        if fam == Family::MySql {
            // The character sets the `#{ident:…}` quoting relies on (the T4 carry).
            let probe = "SELECT @@session.character_set_client, \
                         @@session.character_set_connection, @@session.character_set_results";
            let kind = self.kind;
            let got = {
                let raw = self.conn()?;
                read_rows(raw, kind, probe, &[]).await
            };
            let rows = match got {
                Ok(rows) => rows,
                Err(ReadFail::Raw(e)) => return Err(self.raw_error(&e, true)),
                Err(ReadFail::Shape) => return Err(ExecError::of(ExecErrorKind::Internal)),
            };
            let utf8mb4 = rows.rows.first().is_some_and(|row| {
                row.len() == 3 && row.iter().all(|c| c.as_deref() == Some("utf8mb4"))
            });
            if !utf8mb4 {
                return Err(ExecError::of(ExecErrorKind::ReadOnlyDenied(
                    DenyLayer::Protocol,
                )));
            }
        }
        let millis = r.timeout.as_millis().clamp(1, u128::from(u32::MAX));
        let timeout = format!("SET LOCAL statement_timeout = {millis}");
        let begin: Vec<&str> = match fam {
            Family::Postgres => vec![
                "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
                "SET LOCAL search_path = pg_catalog, pg_temp",
                &timeout,
            ],
            // Fix round 2 (R-W1-13): REPEATABLE READ for this one transaction, whatever the
            // session's isolation. A one-time `SET TRANSACTION` the user left pending is consumed
            // either way (the documented limit, ADR 0004 `## m4-t05`).
            Family::MySql => vec![
                "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ",
                "START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY",
            ],
            // A read-only profile's connection refuses `BEGIN` (its authorizer) and its engine
            // is read-only already (`SQLITE_OPEN_READ_ONLY`, `query_only`).
            Family::Sqlite if self.read_only => Vec::new(),
            Family::Sqlite => vec!["BEGIN"],
            Family::Other => return Err(ExecError::of(ExecErrorKind::Internal)),
        };
        let began = !begin.is_empty();
        for sql in begin {
            let sent = self.conn()?.batch(sql).await;
            if let Err(e) = sent {
                let failed = self.raw_error(&e, true);
                let _ = self.meta_end(began).await;
                return Err(failed);
            }
        }
        if began && self.conn()?.tx_status() != TxStatus::InTx {
            let _ = self.meta_end(began).await;
            return Err(ExecError::of(ExecErrorKind::ReadOnlyDenied(
                DenyLayer::Transaction,
            )));
        }
        let read = self.meta_select(r).await;
        let ended = self.meta_end(began).await;
        match (read, ended) {
            (Ok(rows), Ok(())) => Ok(rows),
            (Err(e), _) | (Ok(_), Err(e)) => Err(e),
        }
    }

    /// Runs the read's statement, its rows as a text summary.
    async fn meta_select(&mut self, r: &MetaRead) -> Reply {
        let kind = self.kind;
        let got = {
            let raw = self.conn()?;
            read_rows(raw, kind, &r.sql, &r.binds).await
        };
        match got {
            Ok(rows) => Ok(summary_of(rows)),
            Err(ReadFail::Raw(e)) => Err(self.raw_error(&e, true)),
            Err(ReadFail::Shape) => Err(ExecError::of(ExecErrorKind::Internal)),
        }
    }

    /// Rolls Hacivat's read transaction back. The session must be idle afterwards: otherwise
    /// it is broken (closed and recovered by the actor), so no user statement ever runs inside
    /// a transaction of Hacivat's.
    async fn meta_end(&mut self, began: bool) -> Result<(), ExecError> {
        let rolled = if began && self.live.is_some() {
            match self.conn()?.batch("ROLLBACK").await {
                Ok(()) => Ok(()),
                Err(e) => Err(self.raw_error(&e, true)),
            }
        } else {
            Ok(())
        };
        let idle = self
            .live
            .as_mut()
            .is_some_and(|live| live.raw().tx_status() == TxStatus::Idle);
        if began && !idle && self.broken.is_none() {
            self.broken = Some(ErrClass::Internal);
        }
        self.observe(false);
        rolled
    }
}

// ---------------------------------------------------------------------------------------------
// The text guards (tokens of the project's lexer, I2-A / I3-B)

/// The first keywords a metadata read may have.
const READ_KEYWORDS: [&str; 5] = ["SELECT", "WITH", "SHOW", "TABLE", "VALUES"];

/// Words a read never holds anywhere: writes (a data-modifying `WITH`, `MERGE`), `SELECT …
/// INTO` (a table, an outfile, variables), statements that run other statements, and
/// PostgreSQL's `UESCAPE` (fix round 2, I5: a `U&"…"` name is refused outright, so its escape
/// clause has no use either).
const DENY_WORDS: [&str; 11] = [
    "INSERT", "UPDATE", "DELETE", "MERGE", "INTO", "COPY", "CALL", "PREPARE", "DECLARE", "EXECUTE",
    "UESCAPE",
];

/// Functions with side effects a read-only transaction does not stop (or does not see on a
/// joined or shared session), by exact name, wherever the name appears.
const DENY_FUNCTIONS: [&str; 15] = [
    "SET_CONFIG",
    "NEXTVAL",
    "SETVAL",
    "LASTVAL",
    "PG_NOTIFY",
    "GET_LOCK",
    "RELEASE_ALL_LOCKS",
    "SLEEP",
    "BENCHMARK",
    "PG_TERMINATE_BACKEND",
    "PG_CANCEL_BACKEND",
    "PG_SLEEP",
    "PG_PROMOTE",
    "PG_RELOAD_CONF",
    "LOAD_EXTENSION",
];

/// The same, by name prefix, wherever the name appears.
const DENY_PREFIXES: [&str; 7] = [
    "PG_ADVISORY",
    "PG_TRY_ADVISORY",
    "LO_",
    "DBLINK",
    "TXID_CURRENT",
    "PG_CURRENT_XACT_ID",
    "RELEASE_LOCK",
];

/// PostgreSQL's administration families (fix round 2, review A minor and review B R-M1), by
/// name prefix, refused where they are **called** (the name followed by `(`): the statistics
/// reset, WAL and log switches, configuration reload, replication slots and origins, logical
/// messages and decoding, backups, recovery control, server files and imports. A call is the
/// rule, so the catalog views of the same families (`pg_replication_slots`,
/// `pg_replication_origin_status`, `pg_file_settings`) stay readable.
const DENY_CALL_PREFIXES: [&str; 15] = [
    "PG_STAT_RESET",
    "PG_SWITCH_",
    "PG_RELOAD_",
    "PG_ROTATE_",
    "PG_CREATE_",
    "PG_DROP_",
    "PG_COPY_",
    "PG_REPLICATION_",
    "PG_LOGICAL_",
    "PG_LOG_",
    "PG_BACKUP_",
    "PG_WAL_REPLAY_",
    "PG_FILE_",
    "PG_IMPORT_",
    "PG_PROMOTE",
];

/// The lexer contexts `kind`'s text is checked in. Every one must accept it: a text two
/// readings of the same engine (a version's numbers, a session's quoting modes) could read
/// differently is refused. `Unknown` (a joined read the caller cannot place) takes them all.
fn guard_contexts(kind: DbmsKind) -> Vec<DialectCtx> {
    let pg = || {
        let mut out = Vec::new();
        for major in [14, 15, 16] {
            for scs in [true, false] {
                out.push(DialectCtx {
                    id: DialectId::Postgres,
                    version: Some(ServerVersion::new(major, 0, 0)),
                    modes: LexModes {
                        standard_conforming_strings: scs,
                        ..LexModes::default()
                    },
                });
            }
        }
        out
    };
    let my = |id: DialectId| {
        [
            LexModes::default(),
            LexModes {
                no_backslash_escapes: true,
                ..LexModes::default()
            },
            LexModes {
                ansi_quotes: true,
                ..LexModes::default()
            },
        ]
        .into_iter()
        .map(|modes| DialectCtx {
            id,
            version: None,
            modes,
        })
        .collect::<Vec<_>>()
    };
    let sqlite = || vec![DialectCtx::new(DialectId::Sqlite, None)];
    match kind {
        DbmsKind::Postgres
        | DbmsKind::Redshift
        | DbmsKind::Cockroach
        | DbmsKind::Yugabyte
        | DbmsKind::Timescale => pg(),
        DbmsKind::MySql | DbmsKind::TiDb => my(DialectId::MySql),
        DbmsKind::MariaDb => my(DialectId::MariaDb),
        DbmsKind::Sqlite => sqlite(),
        _ => {
            let mut all = pg();
            all.extend(my(DialectId::MySql));
            all.extend(my(DialectId::MariaDb));
            all.extend(sqlite());
            all
        }
    }
}

/// [`check_read_text_for`] with every engine's lexer (the engine is not known).
pub fn check_read_text(sql: &str) -> Result<(), MetaError> {
    check_read_text_for(sql, DbmsKind::Unknown)
}

/// The token guard of every metadata read (own connection, user's session, joined session):
/// `Err(RefusedText)` unless, in every lexer context of `kind` ([`guard_contexts`]):
/// - the text has no control character but `\n` and `\t`, no MySQL executable comment
///   (`/*!`, `/*M!`) and no `;` but one at the very end;
/// - it lexes without an error, and has no unknown, client-command, executable-comment or
///   PostgreSQL `U&"…"` identifier token (I5: its escapes can spell any name);
/// - its first token is a word, `SELECT`, `WITH`, `SHOW`, `TABLE` or `VALUES`;
/// - no word or quoted name is [`DENY_WORDS`] or [`DENY_FUNCTIONS`] or starts with
///   [`DENY_PREFIXES`]; no name starting with [`DENY_CALL_PREFIXES`] is called;
/// - no `FOR UPDATE`, `FOR SHARE`, `FOR KEY SHARE`, `FOR NO KEY UPDATE`, `LOCK IN SHARE
///   MODE`, MariaDB `NEXT VALUE FOR` or `PREVIOUS VALUE FOR`, MySQL `LAST_INSERT_ID(…)` with
///   an argument, or `:=`.
pub fn check_read_text_for(sql: &str, kind: DbmsKind) -> Result<(), MetaError> {
    check_text(sql, kind, false)
}

/// The guard of an own connection's snapshot (fix round 2, review B R-M3): [`check_read_text_for`],
/// and on SQLite also a reading `PRAGMA` (no `=` in it): that connection is opened read-only,
/// and T18 reads `schema_version` and the built-in lists through pragmas.
pub fn check_own_text(sql: &str, kind: DbmsKind) -> Result<(), MetaError> {
    check_text(sql, kind, kind == DbmsKind::Sqlite)
}

fn check_text(sql: &str, kind: DbmsKind, sqlite_pragma: bool) -> Result<(), MetaError> {
    let refused = Err(MetaError::RefusedText);
    if !plain_text(sql) {
        return refused;
    }
    for ctx in guard_contexts(kind) {
        let Ok(profile) = lex_profile(&ctx) else {
            return refused;
        };
        let lexed = lex(sql, profile, ctx.modes, &LexState::Initial);
        // ## m4-t17-mysql: the SHOW CREATE exemption holds only in a MySQL-family lexer context.
        let mysql_lexer = matches!(ctx.id, DialectId::MySql | DialectId::MariaDb);
        if !read_tokens_ok(sql, &lexed, sqlite_pragma, mysql_lexer) {
            return refused;
        }
    }
    Ok(())
}

/// No control character but `\n` and `\t`; no MySQL executable comment; no `;` but a final
/// one.
fn plain_text(sql: &str) -> bool {
    if sql
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return false;
    }
    // Fix round 2: no space but ASCII's, and no invisible format character (zero-width
    // spaces and joiners, the word joiner, the BOM, the soft hyphen). Engines read them as
    // name characters, and packs never hold them.
    let invisible = |c: char| {
        (c.is_whitespace() && !c.is_ascii())
            || matches!(c, '\u{00AD}' | '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}')
    };
    if sql.chars().any(invisible) {
        return false;
    }
    if sql.contains("/*!") || sql.contains("/*M!") || sql.contains("/*m!") {
        return false;
    }
    let trimmed = sql.trim_end();
    let body = trimmed.strip_suffix(';').unwrap_or(trimmed);
    !body.contains(';')
}

/// The indices of `lexed`'s tokens that are not trivia.
fn significant(lexed: &Lexed) -> impl Iterator<Item = usize> + '_ {
    (0..lexed.len()).filter(|&i| !lexed.is_trivia(i))
}

fn token_text<'s>(sql: &'s str, lexed: &Lexed, i: usize) -> &'s str {
    let r = lexed.range(i);
    &sql[usize::from(r.start())..usize::from(r.end())]
}

/// A name's text without its quotes (`"nextval"`, `` `sleep` ``).
fn unquoted(text: &str) -> &str {
    text.trim_matches(|c| c == '"' || c == '`' || c == '[' || c == ']')
}

fn denied_name(upper: &str) -> bool {
    DENY_WORDS.contains(&upper)
        || DENY_FUNCTIONS.contains(&upper)
        || DENY_PREFIXES.iter().any(|p| upper.starts_with(p))
}

fn denied_call(upper: &str) -> bool {
    DENY_CALL_PREFIXES.iter().any(|p| upper.starts_with(p))
}

/// One lexer context's reading of a read's text (see [`check_read_text_for`]).
fn read_tokens_ok(sql: &str, lexed: &Lexed, sqlite_pragma: bool, mysql_lexer: bool) -> bool {
    if !lexed.errors().is_empty() {
        return false;
    }
    let sig: Vec<usize> = significant(lexed).collect();
    let Some(&first) = sig.first() else {
        return false;
    };
    if lexed.kind(first) != TokenKind::Word {
        return false;
    }
    let keyword = token_text(sql, lexed, first).to_ascii_uppercase();
    let pragma = sqlite_pragma && keyword == "PRAGMA";
    if !pragma && !READ_KEYWORDS.contains(&keyword.as_str()) {
        return false;
    }
    let upper = |i: usize| token_text(sql, lexed, i).to_ascii_uppercase();
    let is = |i: Option<usize>, kind: TokenKind| i.is_some_and(|i| lexed.kind(i) == kind);
    // ## m4-t17-mysql (named exception in Task 5's file; R-M4-37): a `SHOW CREATE …` statement
    // holds no expression, so a quoted identifier there is the object's name, never a call or
    // a keyword. Such a name may be non-ASCII (`İ`) or spell a denied word (`update`): only
    // backtick-quoted identifiers not followed by `(`, read by a MySQL/MariaDB lexer, are
    // exempt (T5 reviewer A, minor 1); every word is still checked. What keeps the quoting
    // sound is the connection's character set: `SET NAMES utf8mb4` at every connect and the
    // `utf8mb4` assertion before every snapshot (a GBK lead byte could swallow a doubled
    // backtick; ADR 0008 `m4-t17`).
    let show_create = mysql_lexer
        && keyword == "SHOW"
        && sig
            .get(1)
            .is_some_and(|&i| lexed.kind(i) == TokenKind::Word && upper(i) == "CREATE");
    for (n, &i) in sig.iter().enumerate() {
        let next = sig.get(n + 1).copied();
        let after_next = sig.get(n + 2).copied();
        let name = match lexed.kind(i) {
            TokenKind::Unknown
            | TokenKind::ExecutableComment
            | TokenKind::ClientCommand
            | TokenKind::UnicodeQuotedIdent => return false,
            TokenKind::Semicolon if next.is_some() => return false,
            TokenKind::Word => upper(i),
            TokenKind::QuotedIdent
                if show_create
                    && token_text(sql, lexed, i).starts_with('`')
                    && !is(next, TokenKind::LParen) =>
            {
                continue;
            }
            TokenKind::QuotedIdent => unquoted(token_text(sql, lexed, i)).to_ascii_uppercase(),
            TokenKind::Op => {
                let op = token_text(sql, lexed, i);
                let glued = i + 1 < lexed.len() && token_text(sql, lexed, i + 1).starts_with('=');
                // A setting pragma (`PRAGMA x = v`) is never a read.
                if op.contains(":=") || (op.ends_with(':') && glued) || (pragma && op.contains('='))
                {
                    return false;
                }
                continue;
            }
            _ => continue,
        };
        // Pack text names catalog objects in ASCII (user names travel as binds): a non-ASCII
        // name is a lookalike (`nеxtval` with a Cyrillic `е`) or glued to an invisible
        // character, never a pack's.
        if !name.is_ascii() || denied_name(&name) {
            return false;
        }
        let called = is(next, TokenKind::LParen);
        if called && denied_call(&name) {
            return false;
        }
        // MySQL: `LAST_INSERT_ID(n)` sets the session's value; `LAST_INSERT_ID()` reads it.
        if name == "LAST_INSERT_ID" && called && !is(after_next, TokenKind::RParen) {
            return false;
        }
        let next_word = next
            .filter(|&j| lexed.kind(j) == TokenKind::Word)
            .map(upper);
        let after = next_word.as_deref();
        if name == "FOR" && matches!(after, Some("UPDATE" | "SHARE" | "KEY" | "NO")) {
            return false;
        }
        if name == "LOCK" && after == Some("IN") {
            return false;
        }
        // MariaDB's sequence syntax: `NEXT VALUE FOR s`, `PREVIOUS VALUE FOR s`.
        if matches!(name.as_str(), "NEXT" | "PREVIOUS") && after == Some("VALUE") {
            return false;
        }
    }
    true
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
#[cfg(any(test, feature = "test-support"))]
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
        for (ok, kind) in [
            ("SELECT 1", DbmsKind::Unknown),
            (
                "  select relname FROM pg_catalog.pg_class",
                DbmsKind::Unknown,
            ),
            ("-- hv:pg/Identity\nSELECT 1", DbmsKind::Unknown),
            ("/* hv:pg/Identity */ SELECT 1", DbmsKind::Unknown),
            (
                "/* a */ -- b\n WITH x AS (SELECT 1) SELECT * FROM x",
                DbmsKind::Unknown,
            ),
            ("TABLE pg_catalog.pg_namespace", DbmsKind::Unknown),
            ("VALUES (1)", DbmsKind::Unknown),
            ("SELECT 1;", DbmsKind::Unknown),
            ("SELECT 1 ;  \n", DbmsKind::Unknown),
            (
                "SELECT update_rule, delete_rule FROM information_schema.referential_constraints",
                DbmsKind::Unknown,
            ),
            ("SELECT(1)", DbmsKind::Unknown),
            (
                "SELECT c.relname FROM pg_catalog.pg_class c WHERE c.relnamespace = $1",
                DbmsKind::Postgres,
            ),
            ("SHOW CREATE TABLE `orders`", DbmsKind::MySql),
            (
                "SELECT table_name FROM information_schema.tables WHERE table_schema = ?",
                DbmsKind::MariaDb,
            ),
            (
                "SELECT name FROM pragma_table_xinfo(?, 'temp')",
                DbmsKind::Sqlite,
            ),
            ("# hv:mysql/Tables\nSELECT 1", DbmsKind::MySql),
            ("SELECT 'a\tb'", DbmsKind::Unknown),
        ] {
            assert_eq!(check_read_text_for(ok, kind), Ok(()), "{ok} ({kind:?})");
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
            // Review A I2 / review B I3: `\r` ends a PostgreSQL `--` comment.
            "-- x\rCREATE TABLE evil AS\nSELECT 1",
            "-- x\rCOPY (\nSELECT 1) TO '/tmp/hv_evil'",
            "-- x\rPREPARE p AS\nSELECT 1",
            "--\rDECLARE c CURSOR WITH HOLD FOR\nSELECT 1",
            "-- x\rCREATE VIEW evil AS\nSELECT 1",
            "SELECT 1\r",
            "SELECT\u{0b}1",
            // Glued tokens: the lexer, not word boundaries, finds them.
            "SELECT $$x$$INTO evil",
            "SELECT 1INTO evil",
            // Row locks.
            "SELECT * FROM t FOR SHARE",
            "SELECT * FROM t FOR KEY SHARE",
            "SELECT * FROM t FOR NO KEY UPDATE",
            "SELECT * FROM t LOCK IN SHARE MODE",
            // Assignments.
            "SELECT @x := 1",
            "SELECT @x:=1",
            // Side-effect functions.
            "SELECT pg_advisory_lock(1)",
            "SELECT pg_try_advisory_xact_lock(1)",
            "SELECT set_config('search_path', 'evil', false)",
            "SELECT nextval('s')",
            "SELECT \"nextval\"('s')",
            "SELECT setval('s', 1)",
            "VALUES (nextval('s'))",
            "WITH x AS (SELECT setval('s', 1)) SELECT * FROM x",
            "SELECT lo_create(0)",
            "SELECT dblink_exec('dbname=x', 'DROP TABLE t')",
            "SELECT txid_current()",
            "SELECT pg_current_xact_id()",
            "SELECT pg_notify('c', 'x')",
            "SELECT GET_LOCK('x', 0)",
            "SELECT RELEASE_LOCK('x')",
            "SELECT RELEASE_ALL_LOCKS()",
            "SELECT SLEEP(100)",
            "SELECT BENCHMARK(1, 1)",
            "SELECT pg_terminate_backend(1)",
            "SELECT 1 FROM t WHERE EXECUTE",
            "\u{00a0}SELECT 1",
            "\u{FF33}ELECT 1",
        ] {
            assert_eq!(
                check_read_text(refused),
                Err(MetaError::RefusedText),
                "{refused:?}"
            );
        }
    }

    #[test]
    fn an_own_snapshot_never_runs_transaction_or_session_control() {
        for (sql, kind) in [
            ("COMMIT", DbmsKind::Postgres),
            ("rollback", DbmsKind::MySql),
            ("BEGIN", DbmsKind::Sqlite),
            ("SET search_path = x", DbmsKind::Postgres),
            ("SET SESSION TRANSACTION READ WRITE", DbmsKind::MariaDb),
            ("START TRANSACTION", DbmsKind::MySql),
            ("XA START 'x'", DbmsKind::MySql),
            ("DISCARD ALL", DbmsKind::Postgres),
            ("SELECT 1; COMMIT", DbmsKind::Postgres),
            ("/*!COMMIT*/", DbmsKind::MySql),
            ("-- x\rCOMMIT\n", DbmsKind::Postgres),
            ("ATTACH 'x' AS y", DbmsKind::Sqlite),
            ("VACUUM", DbmsKind::Sqlite),
            // Fix round 2 (review B R-M3): the same token guard as the session paths.
            ("CREATE TABLE t (a int)", DbmsKind::Postgres),
            ("INSERT INTO t VALUES (1)", DbmsKind::MySql),
            ("SELECT pg_advisory_lock(1)", DbmsKind::Postgres),
            (
                "SELECT set_config('search_path', 'x', false)",
                DbmsKind::Postgres,
            ),
            ("SELECT GET_LOCK('x', 0)", DbmsKind::MySql),
            ("SELECT NEXT VALUE FOR s", DbmsKind::MariaDb),
            ("SELECT U&\"nextval\"('s')", DbmsKind::Postgres),
            // A setting pragma is not a read; only SQLite takes pragmas at all.
            ("PRAGMA writable_schema = ON", DbmsKind::Sqlite),
            ("PRAGMA query_only=0", DbmsKind::Sqlite),
            ("PRAGMA table_xinfo('t')", DbmsKind::Postgres),
            ("SELECT load_extension('x')", DbmsKind::Sqlite),
        ] {
            assert_eq!(
                check_own_text(sql, kind),
                Err(MetaError::RefusedText),
                "{sql:?}"
            );
        }
        for (sql, kind) in [
            ("SELECT 1", DbmsKind::Postgres),
            ("PRAGMA table_xinfo('t')", DbmsKind::Sqlite),
            ("PRAGMA main.schema_version", DbmsKind::Sqlite),
            ("SELECT * FROM pragma_table_list", DbmsKind::Sqlite),
            ("SHOW CREATE TABLE `t`", DbmsKind::MySql),
            (
                "SELECT slot_name FROM pg_catalog.pg_replication_slots",
                DbmsKind::Postgres,
            ),
            ("SELECT LAST_INSERT_ID()", DbmsKind::MySql),
        ] {
            assert_eq!(check_own_text(sql, kind), Ok(()), "{sql:?}");
        }
    }

    #[test]
    fn round_two_names_are_refused() {
        // I5 (review A): a `U&"…"` name can spell any function; refused outright, with `UESCAPE`.
        // Review A minor / review B R-M1: the sequence syntax, `LAST_INSERT_ID(n)` and the PG
        // administration families.
        for (sql, kind) in [
            ("SELECT U&\"nextval\"('s')", DbmsKind::Postgres),
            ("SELECT U&\"pg_advisory_lock\"(1)", DbmsKind::Postgres),
            (
                "SELECT u&\"set_config\"('search_path','evil',false)",
                DbmsKind::Postgres,
            ),
            ("SELECT U&\"\\006Eextval\"('s')", DbmsKind::Postgres),
            (
                "SELECT U&\"!006Eextval\" UESCAPE '!' ('s')",
                DbmsKind::Postgres,
            ),
            ("SELECT pg_catalog.U&\"txid_current\"()", DbmsKind::Postgres),
            ("SELECT NEXT VALUE FOR s", DbmsKind::MariaDb),
            ("SELECT PREVIOUS VALUE FOR s", DbmsKind::MariaDb),
            ("SELECT LASTVAL(s)", DbmsKind::MariaDb),
            ("SELECT SETVAL(s, 1)", DbmsKind::MariaDb),
            ("SELECT LAST_INSERT_ID(5)", DbmsKind::MySql),
            ("SELECT pg_reload_conf()", DbmsKind::Postgres),
            ("SELECT pg_stat_reset()", DbmsKind::Postgres),
            (
                "SELECT pg_stat_reset_shared('bgwriter')",
                DbmsKind::Postgres,
            ),
            ("SELECT pg_switch_wal()", DbmsKind::Postgres),
            (
                "SELECT pg_create_logical_replication_slot('s', 'test_decoding')",
                DbmsKind::Postgres,
            ),
            (
                "SELECT pg_create_physical_replication_slot('s')",
                DbmsKind::Postgres,
            ),
            ("SELECT pg_create_restore_point('x')", DbmsKind::Postgres),
            ("SELECT pg_drop_replication_slot('s')", DbmsKind::Postgres),
            (
                "SELECT pg_replication_origin_create('o')",
                DbmsKind::Postgres,
            ),
            (
                "SELECT pg_replication_slot_advance('s', '0/0')",
                DbmsKind::Postgres,
            ),
            (
                "SELECT * FROM pg_logical_slot_get_changes('s', NULL, NULL)",
                DbmsKind::Postgres,
            ),
            (
                "SELECT pg_logical_emit_message(true, 'p', 'x')",
                DbmsKind::Postgres,
            ),
            ("SELECT pg_rotate_logfile()", DbmsKind::Postgres),
            ("SELECT pg_promote()", DbmsKind::Postgres),
            ("SELECT pg_terminate_backend(1)", DbmsKind::Postgres),
            ("SELECT pg_cancel_backend(1)", DbmsKind::Postgres),
            (
                "SELECT pg_catalog . \"pg_reload_conf\" ()",
                DbmsKind::Postgres,
            ),
            ("SELECT load_extension('x')", DbmsKind::Sqlite),
        ] {
            assert_eq!(
                check_read_text_for(sql, kind),
                Err(MetaError::RefusedText),
                "{sql:?}"
            );
            assert_eq!(check_read_text(sql), Err(MetaError::RefusedText), "{sql:?}");
        }
        // The views of the same families stay readable; `LAST_INSERT_ID()` only reads.
        for (sql, kind) in [
            (
                "SELECT * FROM pg_catalog.pg_replication_origin_status",
                DbmsKind::Postgres,
            ),
            (
                "SELECT name FROM pg_catalog.pg_file_settings",
                DbmsKind::Postgres,
            ),
            ("SELECT LAST_INSERT_ID()", DbmsKind::MySql),
            ("SELECT next_value FROM t", DbmsKind::MariaDb),
        ] {
            assert_eq!(check_read_text_for(sql, kind), Ok(()), "{sql:?}");
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
        assert!(
            mysql_session_probe(Flavor::MariaDb {
                transaction_read_only: false
            })
            .starts_with("SELECT @@session.tx_read_only,")
        );
        assert!(
            mysql_session_probe(Flavor::MariaDb {
                transaction_read_only: true
            })
            .starts_with("SELECT @@session.transaction_read_only,")
        );
        assert!(
            mysql_session_probe(Flavor::MySql)
                .starts_with("SELECT @@session.transaction_read_only,")
        );
        for probe in [
            mysql_session_probe(Flavor::MySql),
            mysql_session_probe(Flavor::MariaDb {
                transaction_read_only: false,
            }),
        ] {
            for var in [
                "character_set_client",
                "character_set_connection",
                "character_set_results",
            ] {
                assert!(probe.contains(var), "{probe}");
            }
        }
    }

    #[test]
    fn the_mysql_session_needs_read_only_and_utf8mb4() {
        let rows = |cells: &[&str]| Rows {
            columns: Vec::new(),
            rows: vec![cells.iter().map(|c| Some(Arc::from(*c))).collect()],
        };
        assert!(mysql_session_confirmed(&rows(&[
            "1", "utf8mb4", "utf8mb4", "utf8mb4"
        ])));
        assert!(!mysql_session_confirmed(&rows(&[
            "0", "utf8mb4", "utf8mb4", "utf8mb4"
        ])));
        assert!(!mysql_session_confirmed(&rows(&[
            "1", "gbk", "utf8mb4", "utf8mb4"
        ])));
        assert!(!mysql_session_confirmed(&rows(&["1"])));
        assert!(!mysql_session_confirmed(&Rows::default()));
    }
}
