//! The SQLite driver through its public surface: `SqliteDriver::connect` and the `RawConn` it
//! returns (Task 15). Every test works on files inside its own temporary directory.

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_driver::{
    Driver, DriverConnect, Endpoint, ProbeWarning, RawConn, RawError, RawErrorKind,
};
use hacivat_driver_sqlite::SqliteDriver;
use hacivat_proto::DbmsKind;
use tempfile::TempDir;

/// A path inside a temporary directory that lives as long as the value.
struct TmpPath {
    _dir: TempDir,
    path: PathBuf,
}

impl Deref for TmpPath {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

/// A path named `name` in a fresh temporary directory; nothing is created at it.
fn tmp_path(name: &str) -> TmpPath {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    TmpPath { _dir: dir, path }
}

/// An existing database file, written by a separate plain `rusqlite` connection.
fn tmp_db() -> TmpPath {
    let p = tmp_path("existing.db");
    let c = rusqlite::Connection::open(&*p).unwrap();
    c.execute_batch("CREATE TABLE seed(x); INSERT INTO seed VALUES (1);")
        .unwrap();
    drop(c);
    p
}

fn request(path: &Path, create_if_missing: bool, read_only: bool) -> DriverConnect {
    DriverConnect {
        endpoint: Endpoint::File {
            path: path.to_path_buf(),
            create_if_missing,
            read_only,
        },
        tls: None,
        user: None,
        password: None,
        database: None,
        application_name: "Hacivat".into(),
        read_only: false,
        options: vec![],
        connect_timeout: Duration::from_secs(15),
        events: Arc::new(|_| {}),
    }
}

async fn open(
    path: &Path,
    create_if_missing: bool,
    read_only: bool,
) -> Result<Box<dyn RawConn>, RawError> {
    SqliteDriver::new()
        .connect(request(path, create_if_missing, read_only))
        .await
}

/// The error of an open that must fail (`Box<dyn RawConn>` has no `Debug` for `unwrap_err`).
fn open_err(r: Result<Box<dyn RawConn>, RawError>) -> RawError {
    match r {
        Ok(_) => panic!("the open succeeded"),
        Err(e) => e,
    }
}

#[test]
fn the_descriptor_is_file_based_sqlite() {
    let d = SqliteDriver::new();
    assert_eq!(d.descriptor().kind, DbmsKind::Sqlite);
    assert!(d.descriptor().caps.file_based);
    assert!(!d.descriptor().caps.stream_injection);
}

#[tokio::test]
async fn opens_existing_and_reports_version() {
    let f = tmp_db();
    let mut c = open(&f, false, false).await.unwrap();
    assert_eq!(c.kind(), DbmsKind::Sqlite);
    let i = c.server_info().await.unwrap();
    assert_eq!(i.kind, DbmsKind::Sqlite);
    assert_eq!(i.product, "SQLite");
    assert!(i.raw_version.starts_with("3."), "{}", i.raw_version);
    assert_eq!(i.is_replica, None);
    assert!(i.version.unwrap().major >= 3);
}

#[tokio::test]
async fn missing_file_without_create_is_cantopen() {
    let p = tmp_path("nope.db");
    let err = open_err(open(&p, false, false).await);
    assert_eq!(err.kind, RawErrorKind::Db);
    assert_eq!(err.server.unwrap().vendor_code, Some(14));
    assert!(!p.exists(), "a failed open must not create the file");
}

#[tokio::test]
async fn missing_file_read_only_is_cantopen_even_with_create() {
    let p = tmp_path("nope.db");
    let err = open_err(open(&p, true, true).await);
    assert_eq!(err.server.unwrap().vendor_code, Some(14));
    assert!(!p.exists(), "read-only never creates");
}

#[tokio::test]
async fn create_if_missing_creates() {
    let p = tmp_path("new.db");
    open(&p, true, false).await.unwrap();
    assert!(p.exists());
}

#[tokio::test]
async fn a_directory_is_refused_before_sqlite_opens_it() {
    let dir = tempfile::tempdir().unwrap();
    for create in [false, true] {
        let err = open_err(open(dir.path(), create, false).await);
        assert_eq!(err.kind, RawErrorKind::Io(std::io::ErrorKind::InvalidInput));
        assert_eq!(err.server, None);
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_fifo_is_refused_before_sqlite_opens_it() {
    // Reading a FIFO with no writer blocks forever; it must never reach SQLite.
    let p = tmp_path("pipe.db");
    let made = std::process::Command::new("mkfifo")
        .arg(&*p)
        .status()
        .unwrap();
    assert!(made.success());
    for (create, read_only) in [(false, false), (true, false), (false, true)] {
        let err = open_err(open(&p, create, read_only).await);
        assert_eq!(err.kind, RawErrorKind::Io(std::io::ErrorKind::InvalidInput));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_to_a_database_file_opens() {
    let f = tmp_db();
    let link = f.with_file_name("link.db");
    std::os::unix::fs::symlink(&*f, &link).unwrap();
    let mut c = open(&link, false, false).await.unwrap();
    assert!(c.ping().await.is_ok());
}

#[tokio::test]
async fn a_locked_file_fails_within_the_connect_timeout() {
    let f = tmp_db();
    // Another connection holds an exclusive lock: no reader can get in until it commits.
    let holder = rusqlite::Connection::open(&*f).unwrap();
    holder.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let mut req = request(&f, false, false);
    req.connect_timeout = Duration::from_millis(200);
    let start = Instant::now();
    let err = open_err(SqliteDriver::new().connect(req).await);
    let waited = start.elapsed();
    assert_eq!(err.server.unwrap().vendor_code.map(|c| c & 0xff), Some(5));
    assert!(waited >= Duration::from_millis(150), "{waited:?}");
    // rusqlite's own default would wait 5 s.
    assert!(waited < Duration::from_secs(3), "{waited:?}");
    holder.execute_batch("COMMIT").unwrap();
}

#[tokio::test]
async fn a_file_that_is_not_a_database_fails_at_connect() {
    let p = tmp_path("notes.txt");
    std::fs::write(
        &*p,
        b"this is not a database file, only some plain text in it",
    )
    .unwrap();
    let err = open_err(open(&p, false, false).await);
    assert_eq!(err.kind, RawErrorKind::Db);
    assert_eq!(err.server.unwrap().vendor_code, Some(26), "SQLITE_NOTADB");
}

#[tokio::test]
async fn a_relative_path_is_refused_before_sqlite_sees_it() {
    // With the bundled engine's SQLITE_USE_URI, a name starting with `file:` would be read as a
    // URI (`?vfs=`, `?mode=memory`). Only absolute paths and `:memory:` reach SQLite, so such a
    // name never does. This URI would open an in-memory database if SQLite parsed it.
    for p in [
        "file::memory:",
        "file:hv-t15.db?vfs=unix-none",
        "relative.db",
        "",
    ] {
        let err = open_err(open(Path::new(p), false, false).await);
        assert_eq!(
            err.kind,
            RawErrorKind::Io(std::io::ErrorKind::InvalidInput),
            "{p:?}"
        );
        assert_eq!(err.server, None, "{p:?}");
        assert!(p.is_empty() || !Path::new(p).exists(), "{p:?}");
    }
}

#[tokio::test]
async fn memory_is_the_one_name_that_is_not_a_path() {
    let mut c = open(Path::new(":memory:"), false, false).await.unwrap();
    assert!(
        c.probe()
            .await
            .iter()
            .any(|w| matches!(w, ProbeWarning::SqliteJournal { mode } if mode == "memory"))
    );
}

#[tokio::test]
async fn probe_reports_journal_mode() {
    let warnings = open(&tmp_db(), false, false).await.unwrap().probe().await;
    assert!(
        warnings
            .iter()
            .any(|w| matches!(w, ProbeWarning::SqliteJournal { mode } if mode == "delete")),
        "{warnings:?}"
    );
    assert!(!warnings.contains(&ProbeWarning::SqliteFileNotWritable));
}

#[tokio::test]
async fn probe_reports_wal_journal_mode() {
    let f = tmp_db();
    let c = rusqlite::Connection::open(&*f).unwrap();
    let mode: String = c
        .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    drop(c);
    let warnings = open(&f, false, false).await.unwrap().probe().await;
    assert!(
        warnings
            .iter()
            .any(|w| matches!(w, ProbeWarning::SqliteJournal { mode } if mode == "wal")),
        "{warnings:?}"
    );
}

#[tokio::test]
async fn probe_reports_a_file_it_cannot_write() {
    let f = tmp_db();
    let mut perms = std::fs::metadata(&*f).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&*f, perms).unwrap();
    // Read-write was asked for; SQLite falls back to read-only on a write-protected file.
    let warnings = open(&f, false, false).await.unwrap().probe().await;
    assert!(
        warnings.contains(&ProbeWarning::SqliteFileNotWritable),
        "{warnings:?}"
    );
    // Asked for read-only: nothing to warn about.
    let warnings = open(&f, false, true).await.unwrap().probe().await;
    assert!(
        !warnings.contains(&ProbeWarning::SqliteFileNotWritable),
        "{warnings:?}"
    );
}

#[tokio::test]
async fn ping_user_databases_and_cancel() {
    let mut c = open(&tmp_db(), false, false).await.unwrap();
    let rtt = c.ping().await.unwrap();
    assert!(rtt < Duration::from_secs(5), "{rtt:?}");
    assert_eq!(c.current_user().await.unwrap(), None);
    assert_eq!(c.list_databases(100).await.unwrap(), Vec::<String>::new());
    assert!(c.cancel_handle().is_none(), "the interrupt handle is M2b");
}

#[tokio::test]
async fn close_returns_after_the_file_is_closed() {
    // In WAL mode the last connection to close checkpoints and deletes the `-wal` file, so
    // the file's absence right after `close` shows the connection is gone, not just orphaned.
    let f = tmp_db();
    let c = rusqlite::Connection::open(&*f).unwrap();
    let _: String = c
        .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))
        .unwrap();
    drop(c);
    let wal = f.with_file_name("existing.db-wal");
    let c = open(&f, false, false).await.unwrap();
    assert!(wal.exists(), "the open connection holds the WAL");
    c.close().await;
    assert!(
        !wal.exists(),
        "close resolved before the connection was closed"
    );
}

#[test]
fn does_not_block_the_async_executor() {
    // P33: if open blocked the only executor thread, the tick could only run after it.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let tick = tokio::spawn(async { Instant::now() });
        let _c = open(&tmp_db(), false, false).await.unwrap();
        let opened = Instant::now();
        assert!(
            tick.await.unwrap() < opened,
            "the spawned task ran only after open returned"
        );
    });
}
