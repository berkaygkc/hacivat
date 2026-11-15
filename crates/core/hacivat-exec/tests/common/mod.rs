//! Shared fixtures of the run-service tests (append-only: each task adds one block headed
//! `// ## m3a-tNN-<slug>`; Task 23's helpers, then Task 31's matrix helpers).
#![allow(dead_code, reason = "each test file uses a subset of the fixtures")]
#![cfg_attr(not(unix), allow(unused_imports, unused_variables))]

// ## m3a-t01b-seams

use hacivat_proto::DbmsKind;
use hacivat_secrets::{AuthProviderId, Retention};
use hacivat_session::profile::{Profile, ProfileEndpoint};
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, LexModes};
use sql_split::exec_range::CaretOrSelection;
use sql_split::{DocSnapshot, SqlDocument};
use sql_text::TextSize;

/// A profile of `engine` that a `FakeDriver` session connects without a password or a socket.
pub fn profile_of(engine: DbmsKind) -> Profile {
    let endpoint = match engine {
        DbmsKind::Sqlite => ProfileEndpoint::File {
            path: std::env::temp_dir().join("hv-exec-test.db"),
            create_if_missing: true,
        },
        _ => ProfileEndpoint::Server {
            host: "127.0.0.1".into(),
            port: None,
        },
    };
    Profile {
        name: "orders".into(),
        engine,
        endpoint,
        user: Some("deploy".into()),
        auth: AuthProviderId::None,
        retention: Retention::Never,
        ..Profile::default()
    }
}

/// A PostgreSQL 18 context with the default modes.
pub fn pg18() -> DialectCtx {
    DialectCtx {
        id: DialectId::Postgres,
        version: Some(sql_dialect::ServerVersion {
            major: 18,
            minor: 0,
            patch: 0,
        }),
        modes: LexModes::default_for(DialectId::Postgres),
    }
}

/// A PostgreSQL document's snapshot.
pub fn doc(text: &str) -> DocSnapshot {
    SqlDocument::new(text.to_string(), pg18(), ClientFlavor::None).snapshot()
}

/// A caret at byte `at`.
pub fn caret(at: u32) -> CaretOrSelection {
    CaretOrSelection::Caret(TextSize::from(at))
}

// ## m3a-t23-exec

use std::sync::Arc;
use std::time::Duration;

use hacivat_driver::fake::{FakeDriver, FakeQuery, FakeResultSet, FakeRows};
use hacivat_driver::{Driver, RawError, RawErrorKind, ServerMessage};
use hacivat_exec::{
    ExecService, ExecUnit, LogMsg, Resolution, RunAction, RunRequest, RunStatus, RunTicket,
    SessionLog, resolve_units,
};
use hacivat_results::{FetchPolicy, MemoryBudget};
use hacivat_session::session::SessionHandle;
use hacivat_session::testing::fake_session;
use hacivat_values::{LogicalType, Value};
use sql_text::TextRange;

/// A document of `text` in `ctx` (psql conventions on the PostgreSQL family, mysql's on the
/// MySQL family).
pub fn doc_in(text: &str, ctx: DialectCtx) -> DocSnapshot {
    let flavor = match ctx.id {
        DialectId::Postgres | DialectId::PgWire(_) => ClientFlavor::Psql,
        DialectId::MySql | DialectId::MariaDb => ClientFlavor::MySqlCli,
        _ => ClientFlavor::None,
    };
    SqlDocument::new(text.to_string(), ctx, flavor).snapshot()
}

/// A PostgreSQL 18 document with psql conventions.
pub fn pg_doc(text: &str) -> DocSnapshot {
    doc_in(text, pg18())
}

/// A MySQL 8.4 document with the default `sql_mode`.
pub fn mysql_doc(text: &str) -> DocSnapshot {
    let version = sql_dialect::ServerVersion::new(8, 4, 0);
    doc_in(text, DialectCtx::new(DialectId::MySql, Some(version)))
}

/// A selection of bytes `r`.
pub fn sel(r: std::ops::Range<u32>) -> CaretOrSelection {
    CaretOrSelection::Selection(TextRange::new(
        TextSize::from(r.start),
        TextSize::from(r.end),
    ))
}

/// What Run `action` at `at` runs in the PostgreSQL document `text`.
pub fn resolve_doc(text: &str, at: CaretOrSelection, action: RunAction) -> Resolution {
    resolve_units(&pg_doc(text), &[at], action, None)
}

/// The units of `r`; panics naming `r` otherwise.
pub fn units(r: &Resolution) -> &[ExecUnit] {
    match r {
        Resolution::Units(u) => u,
        other => panic!("expected units, got {other:?}"),
    }
}

/// A server error with SQLSTATE `code` and the server's text `text`.
pub fn pg_error(code: &str, text: &str) -> RawError {
    RawError {
        kind: RawErrorKind::Db,
        server: Some(ServerMessage {
            sqlstate: Some(code.into()),
            vendor_code: None,
            severity: Some("ERROR".into()),
            text: text.into(),
            detail: None,
            hint: None,
            lang_hint: None,
            position: None,
        }),
    }
}

fn id_cell(row: u64, _: usize) -> Value {
    Value::Int(row as i64)
}

/// `count` generated rows of one `id int8` column.
pub fn generated(count: u64) -> FakeResultSet {
    FakeResultSet {
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
    }
}

/// A scripted query: `count` rows for the first text starting with `prefix`, answered after
/// 10 ms (on a paused clock: the run is seen running before it ends).
pub fn rows_of(prefix: &'static str, count: u64) -> FakeQuery {
    FakeQuery {
        prefix,
        sets: vec![generated(count)],
        first_delay: Duration::from_millis(10),
        ..Default::default()
    }
}

/// `SELECT …`'s script: `count` rows.
pub fn rows_q(count: u64) -> FakeQuery {
    rows_of("SELECT", count)
}

/// A PostgreSQL fake that follows `BEGIN`/`COMMIT`/`ROLLBACK` (a suspendable PostgreSQL query
/// needs a transaction block, P5).
pub fn fake_pg() -> FakeDriver {
    FakeDriver::new(DbmsKind::Postgres).tracks_transactions()
}

/// A run service over a console session of a fake, with the console's output log.
pub struct Bench {
    pub svc: ExecService,
    pub log: Arc<SessionLog>,
    pub engine: DbmsKind,
}

impl Bench {
    /// A request running `sqls` as units, in order, with `fetch`. A unit is suspendable when
    /// its text starts with `SELECT` (the tests' stand-in for `cursor_eligible`).
    pub fn req_with(&self, sqls: &[&str], fetch: FetchPolicy) -> RunRequest {
        let mut at = 0;
        let units = sqls
            .iter()
            .map(|s| {
                let u = ExecUnit {
                    text: (*s).into(),
                    doc_range: at..at + s.len(),
                    kind: sql_split::exec_range::UnitKind::Statement,
                    suspendable: s.starts_with("SELECT"),
                };
                at += s.len() + 2;
                u
            })
            .collect();
        RunRequest {
            console: 7,
            engine: self.engine,
            units,
            fetch,
            log: self.log.clone(),
        }
    }

    pub fn req(&self, sqls: &[&str]) -> RunRequest {
        self.req_with(sqls, FetchPolicy::default())
    }

    /// The Fluent IDs of the console's log lines, oldest first.
    pub fn log_ids(&self) -> Vec<&'static str> {
        self.log.lines().iter().map(|l| l.msg.fluent_id()).collect()
    }

    pub fn log_msgs(&self) -> Vec<LogMsg> {
        self.log.lines().iter().map(|l| l.msg.clone()).collect()
    }
}

/// The run service over a console session of `fake` (`testing::fake_session`, P6): the bench,
/// the session and the fake.
pub async fn service_over(fake: FakeDriver) -> (Bench, SessionHandle, Arc<FakeDriver>) {
    let engine = fake.descriptor().kind;
    let (h, f) = fake_session(tokio::runtime::Handle::current(), fake, profile_of(engine)).await;
    let bench = Bench {
        svc: ExecService::new(MemoryBudget::new(1 << 30, 256 << 20)),
        log: SessionLog::new(),
        engine,
    };
    (bench, h, f)
}

/// The name of unit `i`'s status.
pub fn unit_status(s: &RunStatus, i: usize) -> &'static str {
    s.units[i].status.name()
}

/// Unit 0's status names as the ticket publishes them, until the run ends (each publish the
/// test task sees; a repeat of the last name once).
pub async fn status_trail(t: &mut RunTicket) -> Vec<&'static str> {
    let first = unit_status(&t.status.borrow_and_update(), 0);
    let mut trail = vec![first];
    while t.status.borrow().summary.is_none() && t.status.changed().await.is_ok() {
        let name = unit_status(&t.status.borrow_and_update(), 0);
        if trail.last() != Some(&name) {
            trail.push(name);
        }
    }
    trail
}

/// Waits until `done` holds, sleeping 5 ms of the (paused) clock between looks.
pub async fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..10_000 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("never happened: {what}");
}
