//! Helpers shared by the typed-streaming tests (Task 15, `typed` and `typed_hostile`): query
//! specs, database files built with plain `rusqlite`, and a collector that drives a typed query
//! to its end while checking the event grammar (`hacivat_driver::typed`).

#![allow(dead_code, reason = "each test target uses a subset")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hacivat_driver::{
    Driver, Protocol, QueryDone, QueryEvent, QuerySpec, RawConn, RawError, ResultEnd,
};
use hacivat_driver_sqlite::SqliteDriver;
use hacivat_values::{Chunk, ResultSchema, Value};
use tempfile::TempDir;

use crate::common::request;

/// A database file in its own temporary directory.
pub struct Fixture {
    pub dir: TempDir,
    pub path: PathBuf,
}

/// `db.sqlite` in a fresh directory, set up by `setup` (any number of statements) through a
/// plain connection, then closed.
pub fn fixture(setup: &str) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute_batch(setup)
        .unwrap_or_else(|e| panic!("{setup}: {e}"));
    drop(c);
    Fixture { dir, path }
}

/// `n` rows `(a INTEGER, b TEXT)` in table `t`: `a` from 1, `b` = `'row ' || a`.
pub fn numbered(n: u64) -> Fixture {
    fixture(&format!(
        "CREATE TABLE t(a INTEGER, b TEXT);
         WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < {n})
         INSERT INTO t SELECT x, 'row ' || x FROM c;"
    ))
}

async fn connect(f: &Fixture, read_only: bool) -> Box<dyn RawConn> {
    SqliteDriver::new()
        .connect(request(&f.path, read_only))
        .await
        .unwrap()
}

/// A read-write profile's connection.
pub async fn open_rw(f: &Fixture) -> Box<dyn RawConn> {
    connect(f, false).await
}

/// A read-only profile's connection (the hardened open).
pub async fn open_ro(f: &Fixture) -> Box<dyn RawConn> {
    connect(f, true).await
}

/// A non-suspendable typed query with the session's defaults.
pub fn spec(sql: &str) -> QuerySpec<'_> {
    QuerySpec {
        sql,
        binds: &[],
        protocol: Protocol::Extended,
        suspendable: false,
        first_rows: 501,
        chunk_rows: 4_096,
        chunk_bytes: 4 << 20,
        cell_cap: 1 << 20,
        max_result_sets: 1,
    }
}

/// A console cursor: `spec` that may suspend.
pub fn cursor(sql: &str) -> QuerySpec<'_> {
    QuerySpec {
        suspendable: true,
        ..spec(sql)
    }
}

/// One result driven to its end.
#[derive(Debug)]
pub struct Run {
    pub schema: Arc<ResultSchema>,
    pub chunks: Vec<Chunk>,
    pub end: ResultEnd,
    pub done: QueryDone,
    /// How often the query suspended (each one resumed with `fetch_more`).
    pub suspensions: u32,
}

impl Run {
    pub fn rows(&self) -> u64 {
        self.chunks.iter().map(|c| u64::from(c.rows())).sum()
    }

    pub fn chunk_rows(&self) -> Vec<u32> {
        self.chunks.iter().map(Chunk::rows).collect()
    }

    /// The cell at (`row`, `col`) across chunks, owned.
    pub fn cell(&self, row: u64, col: u32) -> Value {
        let mut row = row;
        for c in &self.chunks {
            let n = u64::from(c.rows());
            if row < n {
                return c.cell(row as u32, col).to_value();
            }
            row -= n;
        }
        panic!("row outside the result");
    }

    /// Every cell of column `col`, in order.
    pub fn column(&self, col: u32) -> Vec<Value> {
        (0..self.rows()).map(|r| self.cell(r, col)).collect()
    }

    pub fn names(&self) -> Vec<String> {
        names(&self.schema)
    }
}

pub fn names(schema: &ResultSchema) -> Vec<String> {
    schema.columns.iter().map(|c| c.name.to_string()).collect()
}

/// Runs `spec` to its end; each suspension resumes with `fetch_more(resume)`. Checks the event
/// grammar: `ResultStart`, then `Rows`/`Suspended`, one `ResultEnd`, `Done`.
pub async fn run_with(
    c: &mut dyn RawConn,
    spec: QuerySpec<'_>,
    resume: u32,
) -> Result<Run, RawError> {
    c.query(spec).await?;
    let QueryEvent::ResultStart(schema) = c.next_event().await? else {
        panic!("{}: the first event is not ResultStart", spec.sql);
    };
    let mut chunks = Vec::new();
    let mut suspensions = 0;
    let end = loop {
        match c.next_event().await? {
            QueryEvent::Rows(chunk) => {
                assert!(chunk.rows() > 0, "{}: an empty chunk", spec.sql);
                chunks.push(chunk);
            }
            QueryEvent::Suspended => {
                assert!(
                    spec.suspendable,
                    "{}: suspended without being asked",
                    spec.sql
                );
                suspensions += 1;
                c.fetch_more(resume).await?;
            }
            QueryEvent::ResultEnd(end) => break end,
            other => panic!("{}: unexpected {other:?}", spec.sql),
        }
    };
    let QueryEvent::Done(done) = c.next_event().await? else {
        panic!("{}: ResultEnd is not followed by Done", spec.sql);
    };
    assert_eq!(
        end.kept,
        chunks.iter().map(|c| u64::from(c.rows())).sum::<u64>()
    );
    Ok(Run {
        schema,
        chunks,
        end,
        done,
        suspensions,
    })
}

/// [`run_with`], resuming by one default chunk.
pub async fn run(c: &mut dyn RawConn, spec: QuerySpec<'_>) -> Result<Run, RawError> {
    run_with(c, spec, 4_096).await
}

/// The first cell of the first row of `sql` (a typed query).
pub async fn scalar(c: &mut dyn RawConn, sql: &str) -> Value {
    let r = run(c, spec(sql))
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e:?}"));
    r.cell(0, 0)
}

/// The cell at (`row`, `col`) of `SELECT * FROM t` on a database set up by `setup`.
pub async fn cell_of(setup: &str, row: u64, col: u32) -> Value {
    let f = fixture(setup);
    let mut c = open_rw(&f).await;
    let r = run(&mut *c, spec("SELECT * FROM t")).await.unwrap();
    r.cell(row, col)
}

/// The SQLite extended result code of an engine error.
pub fn code(e: &RawError) -> i64 {
    e.server
        .as_ref()
        .and_then(|s| s.vendor_code)
        .unwrap_or_else(|| panic!("no result code: {e:?}"))
}

/// The engine's text of an error.
pub fn text_of(e: &RawError) -> String {
    e.server
        .as_ref()
        .map(|s| s.text.clone())
        .unwrap_or_default()
}

/// Whether another connection is kept from committing right now: `BEGIN EXCLUSIVE` with no
/// busy wait fails with `SQLITE_BUSY` while a live statement holds a SHARED lock (rollback
/// journal).
pub fn writer_blocked(f: &Fixture) -> bool {
    let w = rusqlite::Connection::open(&f.path).unwrap();
    w.busy_timeout(Duration::ZERO).unwrap();
    match w.execute_batch("BEGIN EXCLUSIVE; COMMIT") {
        Ok(()) => false,
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::DatabaseBusy =>
        {
            true
        }
        Err(e) => panic!("BEGIN EXCLUSIVE: {e}"),
    }
}

/// Waits until another connection can commit: `BEGIN EXCLUSIVE` with a long busy wait, which
/// returns as soon as the lock is free (a condition, not a fixed time).
pub fn wait_for_writer(f: &Fixture) {
    let w = rusqlite::Connection::open(&f.path).unwrap();
    w.busy_timeout(Duration::from_secs(60)).unwrap();
    w.execute_batch("BEGIN EXCLUSIVE; COMMIT").unwrap();
}

/// Awaits `fut`, failing instead of hanging when a regression blocks it forever. Generous (two
/// minutes): a guard against hangs, never a time budget.
pub async fn unhung<T>(fut: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(120), fut)
        .await
        .expect("blocked: no answer in two minutes")
}

/// An endless count: its one step never yields a row; only an interrupt ends it.
pub const ENDLESS_STEP: &str =
    "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) FROM c";

/// One step that reads table `t` forever (it holds the table's lock): no row ever comes.
pub const ENDLESS_STEP_OVER_T: &str = "SELECT count(*) FROM t, (WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT x FROM c)";

/// Endless rows.
pub const ENDLESS_ROWS: &str =
    "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT x FROM c";

/// Rows 1 to 11 come at once, then the next step scans forever.
pub const ELEVEN_THEN_ENDLESS: &str =
    "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT x FROM c WHERE x <= 11";
