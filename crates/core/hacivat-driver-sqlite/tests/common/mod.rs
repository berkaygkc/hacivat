//! Helpers shared by the M2b tests (`harden`, `exec`): a database with one table, connections
//! opened through `SqliteDriver::connect` (read-only through `DriverConnect.read_only`, the
//! profile's guard), and short ways to run a statement.

#![allow(dead_code, reason = "each test target uses a subset")]

use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use hacivat_driver::{Driver, DriverConnect, Endpoint, ExecOpts, Protocol, RawConn, RawError};
use hacivat_driver_sqlite::SqliteDriver;
use tempfile::TempDir;

/// A database file; `_dir` keeps a temporary directory alive when the file owns one.
pub struct Db {
    pub path: PathBuf,
    _dir: Option<TempDir>,
}

/// An open connection, with the database it keeps alive.
pub struct Conn {
    conn: Box<dyn RawConn>,
    _db: Db,
}

impl Conn {
    /// Closes the connection; returns once the file is closed.
    pub async fn close(self) {
        self.conn.close().await;
    }
}

impl Deref for Conn {
    type Target = dyn RawConn;
    fn deref(&self) -> &Self::Target {
        &*self.conn
    }
}

impl DerefMut for Conn {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.conn
    }
}

pub fn tempdir() -> TempDir {
    tempfile::tempdir().unwrap()
}

/// `db.sqlite` in `dir`, holding table `t` with one row.
pub fn db_in(dir: &TempDir) -> Db {
    let path = dir.path().join("db.sqlite");
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute_batch("CREATE TABLE t(a); INSERT INTO t VALUES (1);")
        .unwrap();
    drop(c);
    Db { path, _dir: None }
}

/// A database in its own temporary directory, holding table `t` with one row.
pub fn db_with_table() -> Db {
    let dir = tempdir();
    let db = db_in(&dir);
    Db {
        path: db.path,
        _dir: Some(dir),
    }
}

/// A connect request for the file at `path`; `read_only` is the profile's guard.
pub fn request(path: &Path, read_only: bool) -> DriverConnect {
    DriverConnect {
        endpoint: Endpoint::File {
            path: path.to_path_buf(),
            create_if_missing: false,
            read_only: false,
        },
        tls: None,
        user: None,
        password: None,
        database: None,
        application_name: "Hacivat".into(),
        read_only,
        options: vec![],
        connect_timeout: Duration::from_secs(15),
        events: Arc::new(|_| {}),
        insecure_auth: Default::default(),
    }
}

async fn connect(db: Db, read_only: bool) -> Conn {
    let conn = SqliteDriver::new()
        .connect(request(&db.path, read_only))
        .await
        .unwrap();
    Conn { conn, _db: db }
}

/// A read-only profile's connection (`DriverConnect.read_only`): the hardened open.
pub async fn ro(db: Db) -> Conn {
    connect(db, true).await
}

/// A read-write profile's connection.
pub async fn rw(db: Db) -> Conn {
    connect(db, false).await
}

/// The options a short statement runs with.
pub fn simple() -> ExecOpts {
    ExecOpts {
        protocol: Protocol::Simple,
        ..ExecOpts::default()
    }
}

/// The first column of the first row of `sql`.
pub async fn text(c: &mut Conn, sql: &str) -> String {
    let s = c
        .execute(sql, simple())
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e:?}"));
    s.rows[0][0].clone().unwrap_or_default()
}

/// The error `sql` fails with; panics when it succeeds.
pub async fn err(c: &mut Conn, sql: &str) -> RawError {
    match c.execute(sql, simple()).await {
        Ok(s) => panic!("{sql} succeeded: {s:?}"),
        Err(e) => e,
    }
}

/// The SQLite extended result code `sql` fails with (`vendor_code`).
pub async fn err_code(c: &mut Conn, sql: &str) -> i64 {
    err(c, sql)
        .await
        .server
        .and_then(|s| s.vendor_code)
        .unwrap_or_else(|| panic!("{sql}: no result code"))
}

pub fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

pub fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}
