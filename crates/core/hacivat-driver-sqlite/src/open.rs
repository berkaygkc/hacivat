//! Opening a database file on its own blocking thread, with safe flags (HV-CONN-01), and the
//! connection that talks to that thread over a channel.
//!
//! `rusqlite::Connection` is `Send` but not `Sync`, so it never leaves the thread that opened
//! it (spec §5.4: one dedicated blocking thread per SQLite connection). [`SqliteConn`] sends it
//! jobs over an `mpsc` channel and awaits each reply on a `oneshot`, so the async executor never
//! blocks on SQLite.
//!
//! ## m3a-t15-sqlite-typed
//! A typed query is one job too (`typed.rs`), which stays on the thread while the query is open
//! (streaming or suspended). Every other job would queue behind it and wait for a command only
//! this connection's owner can send, so while a query is open `execute`, `batch`, `ping` and
//! `server_info` answer `Unsupported` and `probe` reports nothing: one open query per
//! connection (`hacivat_driver::typed`). After the open, SQLite waits at most
//! [`EXEC_BUSY_TIMEOUT`] for a file another process has locked, then answers `SQLITE_BUSY`
//! (R4; no retry; [`crate::error_hint`] names its hint).

use std::io;
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use hacivat_driver::{
    CancelHandle, CloseHow, Closed, ExecOpts, ExecSummary, ProbeWarning, Protocol, QueryEvent,
    QuerySpec, RawConn, RawError, RawErrorKind, SemVerLoose, ServerInfo, TxStatus, TypedCaps,
};
use hacivat_net::{TlsSummary, TlsVerified};
use hacivat_proto::DbmsKind;
use rusqlite::config::DbConfig;
use rusqlite::{Connection, OpenFlags};
use tokio::sync::{mpsc, oneshot};

use crate::exec::{PROGRESS_STEPS, SqliteCancel, interrupted, tx_status};
use crate::harden::{AuthTrail, harden_read_only};
use crate::typed::{self, unsupported};
use crate::{gone, invalid_path, raw_error};

// ## m3a-t15-sqlite-typed
/// How long a statement waits for a file another process has locked, after the open (R4): then
/// `SQLITE_BUSY`, with no retry. The open's header read waits the connect timeout instead. A
/// cancel during the wait takes effect only when it ends (SQLite's busy handler sleeps without
/// checking the interrupt).
pub(crate) const EXEC_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// The one name SQLite reads as an in-memory database rather than a file.
pub(crate) const MEMORY: &str = ":memory:";

/// The open flags: `READ_ONLY` when read-only; `READ_WRITE | CREATE` only when the file may
/// be created; otherwise `READ_WRITE`. `NO_MUTEX` because the connection is used from one
/// thread. `SQLITE_OPEN_URI` is never set (a `vfs` cannot sneak in).
pub(crate) fn open_flags(create_if_missing: bool, read_only: bool) -> OpenFlags {
    let access = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else if create_if_missing {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    };
    access | OpenFlags::SQLITE_OPEN_NO_MUTEX
}

/// Only an absolute path or `:memory:` reaches SQLite, and never a network path.
///
/// A path that names another machine (`hacivat_net::is_network_path`: `\\host\share`,
/// `//host/share`, `\??\`, device and UNC prefixes) is refused first, before any file system
/// call, on every OS.
///
/// The bundled engine is built with `SQLITE_USE_URI`, so a name that starts with `file:` is
/// parsed as a URI (`?vfs=`, `?mode=`, `?immutable=`) even without `SQLITE_OPEN_URI`. An
/// absolute path never starts with `file:`. A relative path would also depend on the app's
/// working directory, and the empty path opens a private temporary database.
///
/// Some Windows path prefixes are refused; see [`prefix_refused`].
pub(crate) fn check_path(path: &Path) -> Result<(), RawError> {
    if hacivat_net::is_network_path(path) {
        return Err(invalid_path());
    }
    if path.as_os_str() == MEMORY {
        return Ok(());
    }
    let refused = match path.components().next() {
        Some(Component::Prefix(p)) => prefix_refused(p.kind()),
        _ => false,
    };
    if path.is_absolute() && !refused {
        Ok(())
    } else {
        Err(invalid_path())
    }
}

/// A Windows path prefix that never names a local database file:
/// - device namespaces (`\\.\PhysicalDrive0`, `\\?\GLOBALROOT\Device\…`);
/// - UNC shares (`\\server\share\…`, `\\?\UNC\server\share\…`). Opening one performs SMB
///   authentication, which leaks the user's NTLM hash to that host. A dead share can block the
///   thread, and SQLite's locking over SMB is unreliable.
fn prefix_refused(prefix: Prefix<'_>) -> bool {
    match prefix {
        Prefix::DeviceNS(_) | Prefix::UNC(..) | Prefix::VerbatimUNC(..) => true,
        Prefix::Verbatim(name) => name.eq_ignore_ascii_case("GLOBALROOT"),
        Prefix::Disk(_) | Prefix::VerbatimDisk(_) => false,
    }
}

/// When something exists at `path`, it must be a regular file (a symlink to one is fine). A
/// FIFO, socket, device or directory is refused before SQLite opens it, so a read on it can
/// never block the connection's thread. A missing path is left to SQLite (`CANTOPEN`, or a new
/// file with `create_if_missing`).
fn check_existing(path: &Path) -> Result<(), RawError> {
    if path.as_os_str() == MEMORY {
        return Ok(());
    }
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => Ok(()),
        Ok(_) => Err(invalid_path()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(RawError {
            kind: RawErrorKind::Io(e.kind()),
            server: None,
        }),
    }
}

/// Opens and hardens the connection, on the calling (blocking) thread. `guard`: the read-only
/// profile's authorizer trail; with it the file is opened read-only and hardened
/// ([`harden_read_only`]) after the header is read.
fn open_blocking(
    path: &Path,
    create_if_missing: bool,
    read_only: bool,
    guard: Option<Arc<AuthTrail>>,
    connect_timeout: Duration,
) -> Result<Connection, RawError> {
    check_path(path)?;
    check_existing(path)?;
    let read_only = read_only || guard.is_some();
    let mut conn = Connection::open_with_flags(path, open_flags(create_if_missing, read_only))
        .map_err(raw_error)?;
    // Extension loading stays off: rusqlite is built without `load_extension`, and SQLite
    // keeps `load_extension()` disabled until a C call enables it, which nothing here makes.
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)
        .map_err(raw_error)?;
    // The file may come from anywhere: SQL stored in its schema (views, triggers) may not call
    // functions that are not marked innocuous.
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)
        .map_err(raw_error)?;
    // A file locked by another process makes the header read wait, at most this long.
    conn.busy_timeout(connect_timeout).map_err(raw_error)?;
    // SQLite reads the file lazily; read the header now so a file that is not a database
    // (`SQLITE_NOTADB`) or is locked fails at connect, not at the first query.
    conn.query_row("PRAGMA schema_version", [], |r| r.get::<_, i64>(0))
        .map_err(raw_error)?;
    if let Some(trail) = guard {
        harden_read_only(&mut conn, trail)?;
    }
    // ## m3a-t15-sqlite-typed: the execution busy timeout (R4).
    conn.busy_timeout(EXEC_BUSY_TIMEOUT).map_err(raw_error)?;
    Ok(conn)
}

/// A unit of work run on the connection's thread.
type Job = Box<dyn FnOnce(&Connection) + Send>;

/// An open SQLite connection, living on its own blocking thread (HV-CONN-01).
pub(crate) struct SqliteConn {
    /// `None` after `close`; dropping the sender ends the thread's loop.
    jobs: Option<mpsc::Sender<Job>>,
    /// Resolves once the thread has dropped the connection.
    closed: Option<oneshot::Receiver<()>>,
    /// Read-only was asked for, so a read-only file is no surprise.
    read_only: bool,
    /// The read-only profile's authorizer trail; `None` on a read-write profile.
    guard: Option<Arc<AuthTrail>>,
    /// The transaction status after the last statement.
    tx: TxStatus,
    cancel: Arc<SqliteCancel>,
    // ## m3a-t15-sqlite-typed
    /// The open typed query (streaming or suspended), if any.
    typed: Option<typed::Open>,
    /// A cursor's idle bound, cached: read at the open and on `probe`, re-read after a statement
    /// that may change it (`typed::may_change_journal`).
    idle_bound: Option<Duration>,
}

/// Opens `path` on a dedicated thread of the blocking pool. Resolves once the file is open and
/// its header has been read; the connection then stays on that thread. `read_only` opens the
/// file read-only (the endpoint's or a `mode=ro` option); `guarded` is a read-only profile
/// (`DriverConnect.read_only`), which also opens it read-only and hardens it (B21). The two
/// are independent controls. `connect_timeout` is SQLite's busy timeout: how long a file
/// locked by another process is waited for.
pub(crate) async fn open(
    path: PathBuf,
    create_if_missing: bool,
    read_only: bool,
    guarded: bool,
    connect_timeout: Duration,
) -> Result<SqliteConn, RawError> {
    type Opened = (Arc<SqliteCancel>, Option<Duration>);
    let (opened_tx, opened_rx) = oneshot::channel::<Result<Opened, RawError>>();
    let (jobs_tx, mut jobs_rx) = mpsc::channel::<Job>(1);
    let (closed_tx, closed_rx) = oneshot::channel::<()>();
    let guard = guarded.then(|| Arc::new(AuthTrail::default()));
    let trail = guard.clone();
    tokio::task::spawn_blocking(move || {
        let opened = open_blocking(&path, create_if_missing, read_only, trail, connect_timeout)
            .and_then(|mut conn| {
                let cancel = Arc::new(SqliteCancel::new(conn.get_interrupt_handle()));
                let latch = Arc::clone(&cancel);
                conn.progress_handler(PROGRESS_STEPS, Some(move || latch.pending()))
                    .map_err(raw_error)?;
                Ok((conn, cancel))
            });
        let (conn, cancel) = match opened {
            Ok(opened) => opened,
            Err(e) => {
                let _ = opened_tx.send(Err(e));
                return;
            }
        };
        let idle_bound = typed::idle_bound(&conn);
        // The caller gave up on the open: close the file straight away.
        if opened_tx.send(Ok((cancel, idle_bound))).is_err() {
            return;
        }
        while let Some(job) = jobs_rx.blocking_recv() {
            job(&conn);
        }
        drop(conn);
        let _ = closed_tx.send(());
    });
    let (cancel, idle_bound) = opened_rx.await.map_err(|_| gone())??;
    Ok(SqliteConn {
        jobs: Some(jobs_tx),
        closed: Some(closed_rx),
        read_only: read_only || guarded,
        guard,
        tx: TxStatus::Idle,
        cancel,
        typed: None,
        idle_bound,
    })
}

// ## m3a-t15-sqlite-typed
impl Drop for SqliteConn {
    /// Dropped with a typed query open (discarded without `close`): its job may be inside one
    /// long step, where no closed channel reaches it (the job sees its channels between rows),
    /// so the latch's close stops it, as `close` does. Nothing can send a job after this.
    fn drop(&mut self) {
        if self.typed.is_some() {
            self.cancel.close();
        }
    }
}

impl SqliteConn {
    /// `Unsupported` while a typed query is open: a job sent now would wait behind it.
    fn idle(&self) -> Result<(), RawError> {
        match self.typed {
            Some(_) => Err(unsupported()),
            None => Ok(()),
        }
    }

    /// Runs `f` on the connection's thread and returns its result.
    pub(crate) async fn call<R, F>(&self, f: F) -> Result<R, RawError>
    where
        R: Send + 'static,
        F: FnOnce(&Connection) -> R + Send + 'static,
    {
        let jobs = self.jobs.as_ref().ok_or_else(gone)?;
        let (tx, rx) = oneshot::channel();
        let job: Job = Box::new(move |conn| {
            let _ = tx.send(f(conn));
        });
        jobs.send(job).await.map_err(|_| gone())?;
        rx.await.map_err(|_| gone())
    }

    /// Runs `sql` (one or more statements) on the connection.
    #[cfg(test)]
    pub(crate) async fn exec(&mut self, sql: &str) -> Result<(), RawError> {
        let sql = sql.to_string();
        self.call(move |c| c.execute_batch(&sql).map_err(raw_error))
            .await?
    }

    /// The SQLite extended result code `sql` fails with; panics when it succeeds.
    #[cfg(test)]
    pub(crate) async fn exec_err_code(&mut self, sql: &str) -> i64 {
        let err = self.exec(sql).await.expect_err("the statement succeeded");
        err.server
            .and_then(|s| s.vendor_code)
            .expect("an SQLite error carries its result code")
    }

    /// The first column of the first row of `sql`, as an integer.
    #[cfg(test)]
    pub(crate) async fn query_i64(&mut self, sql: &str) -> Result<i64, RawError> {
        let sql = sql.to_string();
        self.call(move |c| c.query_row(&sql, [], |r| r.get(0)).map_err(raw_error))
            .await?
    }
}

impl RawConn for SqliteConn {
    fn kind(&self) -> DbmsKind {
        DbmsKind::Sqlite
    }

    fn server_info(&mut self) -> BoxFuture<'_, Result<ServerInfo, RawError>> {
        Box::pin(async move {
            self.idle()?;
            let raw = self
                .call(|c| {
                    c.query_row("SELECT sqlite_version()", [], |r| r.get::<_, String>(0))
                        .map_err(raw_error)
                })
                .await??;
            Ok(ServerInfo {
                kind: DbmsKind::Sqlite,
                product: "SQLite".to_string(),
                version: SemVerLoose::parse(&raw),
                raw_version: raw,
                is_replica: None,
            })
        })
    }

    fn ping(&mut self) -> BoxFuture<'_, Result<Duration, RawError>> {
        Box::pin(async move {
            self.idle()?;
            let start = Instant::now();
            self.call(|c| {
                c.query_row("SELECT 1", [], |r| r.get::<_, i64>(0))
                    .map_err(raw_error)
            })
            .await??;
            Ok(start.elapsed())
        })
    }

    fn tls_summary(&mut self) -> BoxFuture<'_, TlsSummary> {
        Box::pin(async {
            TlsSummary {
                encrypted: false,
                version: None,
                cipher: None,
                verified: TlsVerified::No,
            }
        })
    }

    /// Nothing while a typed query is open. Refreshes the cursor's idle bound.
    fn probe(&mut self) -> BoxFuture<'_, Vec<ProbeWarning>> {
        let read_only = self.read_only;
        Box::pin(async move {
            if self.idle().is_err() {
                return Vec::new();
            }
            let probed = self
                .call(move |c| (crate::probe::probe(c, read_only), typed::idle_bound(c)))
                .await;
            match probed {
                Ok((warnings, idle_bound)) => {
                    self.idle_bound = idle_bound;
                    warnings
                }
                Err(_) => Vec::new(),
            }
        })
    }

    fn current_user(&mut self) -> BoxFuture<'_, Result<Option<String>, RawError>> {
        Box::pin(async { Ok(None) })
    }

    fn list_databases(&mut self, _limit: usize) -> BoxFuture<'_, Result<Vec<String>, RawError>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn close(mut self: Box<Self>) -> BoxFuture<'static, ()> {
        // Stop the running statement and every queued job first (review R1-I1): a request whose
        // future was dropped may still run one that never ends. Then dropping the sender ends
        // the thread's loop; it drops the connection, then signals.
        self.cancel.close();
        // An open typed query's channels close with it, which ends its job.
        self.typed = None;
        self.jobs = None;
        let closed = self.closed.take();
        Box::pin(async move {
            if let Some(closed) = closed {
                let _ = closed.await;
            }
        })
    }

    /// The connection's latched interrupt ([`SqliteCancel`]), made on its thread at open.
    fn cancel_handle(&self) -> Option<Arc<dyn CancelHandle>> {
        Some(self.cancel.clone())
    }

    // ## m2b-t01-interfaces: filled by Task 11 (`exec`).

    /// One statement (see `exec`). The transaction status is read after it, failed or not.
    fn execute<'a>(
        &'a mut self,
        sql: &'a str,
        opts: ExecOpts,
    ) -> BoxFuture<'a, Result<ExecSummary, RawError>> {
        let sql = sql.to_string();
        let guard = self.guard.clone();
        let cancel = Arc::clone(&self.cancel);
        Box::pin(async move {
            self.idle()?;
            let ticket = cancel.begin();
            let (result, tx, idle_bound) = self
                .call(move |c| {
                    cancel.start(ticket);
                    let r = crate::exec::execute(c, &sql, opts.max_rows, guard.as_deref(), || {
                        cancel.cancelled(ticket)
                    });
                    // ## T15 review round 1: the cached idle bound, re-read when it may change.
                    let idle_bound = typed::may_change_journal(&sql).then(|| typed::idle_bound(c));
                    cancel.end(ticket);
                    (r, tx_status(c), idle_bound)
                })
                .await?;
            self.tx = tx;
            if let Some(bound) = idle_bound {
                self.idle_bound = bound;
            }
            result
        })
    }

    /// Trusted control text, any number of statements (the authorizer still applies on a
    /// read-only profile).
    fn batch<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), RawError>> {
        let sql = sql.to_string();
        let cancel = Arc::clone(&self.cancel);
        Box::pin(async move {
            self.idle()?;
            let ticket = cancel.begin();
            let (result, tx, idle_bound) = self
                .call(move |c| {
                    cancel.start(ticket);
                    let r = if cancel.cancelled(ticket) {
                        Err(interrupted())
                    } else {
                        c.execute_batch(&sql).map_err(raw_error)
                    };
                    // ## T15 review round 1: the cached idle bound, re-read when it may change.
                    let idle_bound = typed::may_change_journal(&sql).then(|| typed::idle_bound(c));
                    cancel.end(ticket);
                    (r, tx_status(c), idle_bound)
                })
                .await?;
            self.tx = tx;
            if let Some(bound) = idle_bound {
                self.idle_bound = bound;
            }
            result
        })
    }

    /// From `sqlite3_get_autocommit` after the last `execute` or `batch`.
    fn tx_status(&self) -> TxStatus {
        self.tx
    }

    /// SQLite keeps no session state to reset (a no-op by design, not a stub).
    fn reset(&mut self) -> BoxFuture<'_, Result<(), RawError>> {
        Box::pin(async { Ok(()) })
    }

    fn backend_id(&self) -> Option<u64> {
        None
    }

    // ## m3a-t15-sqlite-typed (`typed.rs`)

    /// Suspends; no binds (M3b); closes by finalizing; the cached idle bound of the journal
    /// modes (`typed.rs`).
    fn typed_caps(&self) -> TypedCaps {
        typed::caps(self.idle_bound)
    }

    /// Returns once the statement is prepared and checked on the connection's thread, so
    /// SQLite's refusals (syntax, the read-only authorizer, `readonly-required`, two statements,
    /// a parameter) fail here; errors while stepping come from `next_event`.
    fn query<'a>(&'a mut self, spec: QuerySpec<'a>) -> BoxFuture<'a, Result<(), RawError>> {
        Box::pin(async move {
            if self.typed.is_some() || spec.protocol == Protocol::Simple || !spec.binds.is_empty() {
                return Err(unsupported());
            }
            let jobs = self.jobs.clone().ok_or_else(gone)?;
            let ticket = self.cancel.begin();
            let (job, open) =
                typed::start(&spec, self.guard.clone(), Arc::clone(&self.cancel), ticket);
            // ## T15 review round 1 (finding 1): the query is the connection's open one before
            // its job is sent, so a `query` future dropped at any later point still leaves it to
            // `close_query` or to the connection's drop (which stops the job).
            self.typed = Some(open);
            let job: Job = Box::new(move |conn| job.run(conn));
            if jobs.send(job).await.is_err() {
                self.typed = None;
                return Err(gone());
            }
            let started = match self.typed.as_mut() {
                Some(open) => open.started(&mut self.tx, &mut self.idle_bound).await,
                None => Err(gone()),
            };
            if started.is_err() {
                self.typed = None;
            }
            started
        })
    }

    fn next_event(&mut self) -> BoxFuture<'_, Result<QueryEvent, RawError>> {
        Box::pin(typed::next_event(
            &mut self.typed,
            &mut self.tx,
            &mut self.idle_bound,
        ))
    }

    fn fetch_more(&mut self, rows: u32) -> BoxFuture<'_, Result<(), RawError>> {
        let resumed = typed::fetch_more(&mut self.typed, rows);
        Box::pin(async move { resumed })
    }

    fn close_query(&mut self, how: CloseHow) -> BoxFuture<'_, Result<Closed, RawError>> {
        Box::pin(typed::close_query(
            &mut self.typed,
            &mut self.tx,
            &mut self.idle_bound,
            how,
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Prefix;
    use std::sync::atomic::Ordering;

    use super::prefix_refused;

    // ## m3a-t15-sqlite-typed
    /// A consumer that stops reading stops the stepping: the job parks on its full event
    /// channel after at most `EVENT_DEPTH` chunks, and resumes when they are read.
    #[tokio::test]
    async fn memory_is_bounded_by_the_channel_depth() {
        use hacivat_driver::{Protocol, QueryEvent, QuerySpec, RawConn};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t15-pace.db");
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE t(a INTEGER);
                 WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 100000)
                 INSERT INTO t SELECT x FROM c;",
            )
            .unwrap();
        let mut c = super::open(path, false, false, false, super::Duration::from_secs(15))
            .await
            .unwrap();
        let chunk_rows = 100;
        c.query(QuerySpec {
            sql: "SELECT a FROM t",
            binds: &[],
            protocol: Protocol::Extended,
            suspendable: false,
            first_rows: 501,
            chunk_rows,
            chunk_bytes: 4 << 20,
            cell_cap: 1 << 20,
            max_result_sets: 1,
        })
        .await
        .unwrap();
        let parked = async {
            let pace = c.typed.as_ref().unwrap().pace();
            while !pace.parked.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
            pace.rows.load(Ordering::SeqCst)
        };
        let stepped = tokio::time::timeout(super::Duration::from_secs(120), parked)
            .await
            .expect("the job parks on its full channel");
        // The plan's depth of 4: the first chunk and at most four more in flight.
        let bound = 501 + 4 * u64::from(chunk_rows);
        assert!(stepped <= bound, "{stepped} rows stepped, bound {bound}");
        // Reading resumes it, to the end.
        let mut rows = 0u64;
        loop {
            match c.next_event().await.unwrap() {
                QueryEvent::Rows(chunk) => rows += u64::from(chunk.rows()),
                QueryEvent::Done(_) => break,
                _ => {}
            }
        }
        assert_eq!(rows, 100_000);
    }

    #[test]
    fn device_and_unc_prefixes_are_refused_disks_are_not() {
        let s = OsStr::new;
        assert!(prefix_refused(Prefix::DeviceNS(s("PhysicalDrive0"))));
        assert!(prefix_refused(Prefix::Verbatim(s("globalroot"))));
        assert!(prefix_refused(Prefix::UNC(s("server"), s("share"))));
        assert!(prefix_refused(Prefix::VerbatimUNC(s("server"), s("share"))));
        assert!(!prefix_refused(Prefix::Verbatim(s("other"))));
        assert!(!prefix_refused(Prefix::Disk(b'C')));
        assert!(!prefix_refused(Prefix::VerbatimDisk(b'C')));
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use std::path::Path;

    use super::check_path;

    #[test]
    fn device_namespaces_and_unc_shares_are_refused() {
        for p in [
            r"\\.\PhysicalDrive0",
            r"\\.\C:",
            r"\\?\GLOBALROOT\Device\HarddiskVolume1\a.db",
            r"\\server\share\a.db",
            r"\\?\UNC\server\share\a.db",
        ] {
            assert!(check_path(Path::new(p)).is_err(), "{p}");
        }
        assert!(check_path(Path::new(r"C:\data\a.db")).is_ok());
        assert!(check_path(Path::new(r"\\?\C:\data\a.db")).is_ok());
    }
}
