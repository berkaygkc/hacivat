//! M3a Task 31: typed-execution conformance over the Docker matrix (the M3a exit criterion "the
//! Docker matrix for typed execution on all three engines"). Every case goes the app's way: a
//! console document, `resolve_units` in execution mode, `ExecService::run`, a real session, the
//! typed driver; and an admin side channel checks what reached the server. Run with
//! `<main>/target/docker/run-docker-tests.sh --set pr|full -p hacivat-exec --test
//! docker_m3a_matrix`.
//!
//! - Lines: PostgreSQL 14–18, MySQL 8.0 and 8.4, MariaDB 10.6, 10.11 and 11.4 under `full`
//!   (pg17 and mysql84 under `pr`), and SQLite (a file per test) in every set. Each test fails
//!   when the active set has no line of an engine family, or (under `full`) misses a named line,
//!   and asserts at its end that every line ran (`assert_every_line_ran`). `pg17-tr` is the
//!   Turkish test's own line.
//! - Fixtures carry `hv_t31` names unique to the run and are dropped at the end (Q7).
//! - Waits are on conditions bounded by real time; the idle bound is injected (2 s). The one
//!   wall-clock budget is the plan's `cancel_within_one_second_on_every_engine`, which the
//!   Docker rule keeps (with margin), measured on a quiet matrix: it holds `QUIET` for writing,
//!   every other test for reading.
//! - Each test writes its engine × version × case rows to the Docker artifact
//!   `t31-<test>.tsv`; ADR 0006 `m3a-t31` tabulates them.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Instant;

use common::matrix::*;
use hacivat_driver::{Protocol, TxStatus};
use hacivat_exec::{LogMsg, UnitStatus, UnitSummary, console_ux};
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_results::{
    CloseReason, Completeness, FetchPolicy, MemoryBudget, ResultId, ResultStore, ResultWriter,
};
use hacivat_session::session::{
    CancelOutcome, ClosePolicy, ErrorPolicy, ObserverRef, RowsOutcome, RunEvent, RunId,
    RunObserver, RunSpec, RunUnit, SessionState, TxOwner, UnitId,
};
use hacivat_values::{
    DateVal, DecimalVal, LogicalType, OpaqueVal, ResultSchema, SpecialVal, TimeVal, TsVal, Value,
};

/// The cancel test's budget is measured on a quiet matrix: it takes this for writing, every
/// other test for reading.
static QUIET: tokio::sync::RwLock<()> = tokio::sync::RwLock::const_new(());

/// The lines come from the targets' properties (review I2): no Docker needed, so this runs in
/// every `cargo test`.
#[test]
fn lines_are_classified_from_the_targets_never_from_a_list() {
    let catalog = hacivat_driver::EngineCatalog::embedded();
    let all = hacivat_testkit::all_targets_unfiltered();
    let ids = |v: &[hacivat_testkit::DockerTarget]| -> Vec<String> {
        let mut ids: Vec<String> = v.iter().map(|t| t.id.clone()).collect();
        ids.sort();
        ids
    };
    let c = classify(&all, catalog).expect("the committed targets classify");
    assert_eq!(
        ids(&c.lines),
        [
            "mariadb1011",
            "mariadb106",
            "mariadb114",
            "mysql80",
            "mysql84",
            "pg14",
            "pg15",
            "pg16",
            "pg17",
            "pg18"
        ]
    );
    assert_eq!(
        ids(&c.twins),
        ["mysql84-x509", "pg17-auth", "pg17-tls", "pg17-tr"]
    );
    assert_eq!(ids(&c.below_floor), ["pg13"]);
    assert!(missing_tested(&c.lines, catalog).is_empty());

    let like = |id: &str, engine: DbmsKind, major: &str| {
        let mut t = all[0].clone();
        t.id = id.into();
        t.engine = engine;
        t.major = major.into();
        t
    };
    // A line added later joins the matrix by itself.
    let mut more = all.clone();
    more.push(like("pg19", DbmsKind::Postgres, "19"));
    let c = classify(&more, catalog).unwrap();
    assert!(c.lines.iter().any(|t| t.id == "pg19"));
    // A second target of a major that is no twin of its line, or one of an engine the
    // matrix does not know, is an error, never a silent skip.
    let mut odd = all.clone();
    odd.push(like("pg17b", DbmsKind::Postgres, "17"));
    assert!(classify(&odd, catalog).unwrap_err().contains("pg17b"));
    let mut other = all.clone();
    other.push(like("mssql22", DbmsKind::MsSql, "16"));
    assert!(classify(&other, catalog).unwrap_err().contains("mssql22"));
    // Under `full`, a tested major without a line is named.
    let lines: Vec<_> = classify(&all, catalog)
        .unwrap()
        .lines
        .into_iter()
        .filter(|t| t.id != "mysql84")
        .collect();
    assert_eq!(missing_tested(&lines, catalog), ["MySql 8.4"]);
}

fn fetched(rows: u64, more: bool) -> Option<RowsOutcome> {
    Some(RowsOutcome::Fetched { rows, more })
}

fn rows_of(s: &UnitStatus) -> Option<RowsOutcome> {
    match s {
        UnitStatus::Succeeded(UnitSummary { rows, .. }) => *rows,
        _ => None,
    }
}

fn int(v: &Value) -> i64 {
    match v {
        Value::Int(i) => *i,
        Value::UInt(u) => i64::try_from(*u).expect("an int"),
        other => panic!("not an integer: {other:?}"),
    }
}

/// The line's server version must be the line's own (no target answers for another).
fn check_version(c: &Console) {
    let v = c
        .version
        .as_ref()
        .unwrap_or_else(|| panic!("{}: no version", c.line.id));
    match &c.line.target {
        Some(t) => assert!(
            version_matches(&t.major, v),
            "{}: the server is {v:?}, not {}",
            c.line.id,
            t.major
        ),
        None => assert_eq!(v.major, 3, "sqlite: {v:?}"),
    }
}

/// `n` rows of one integer column counting from 1, in order, as a read head (cursor
/// eligible, R-M3-23).
fn counting(line: &Line, n: u64) -> String {
    match line.engine {
        DbmsKind::Postgres => format!("SELECT g FROM generate_series(1, {n}) g"),
        DbmsKind::Sqlite => format!(
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c LIMIT {n}) \
             SELECT x FROM c"
        ),
        _ => format!(
            "WITH d(x) AS (SELECT 0 UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 \
             UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 \
             UNION ALL SELECT 8 UNION ALL SELECT 9) \
             SELECT n FROM (SELECT a.x + b.x * 10 + c.x * 100 + e.x * 1000 + f.x * 10000 + 1 \
             AS n FROM d a, d b, d c, d e, d f) s WHERE n <= {n} ORDER BY n"
        ),
    }
}

/// PostgreSQL: table `t` and `t_f(int)`, a `VOLATILE` function that inserts its argument into
/// `t` and returns it (review I1). A read head that calls it writes one row per row read, so
/// the side channel sees whether Hacivat's transaction was committed or rolled back.
async fn pg_writer(a: &mut Side, t: &str) {
    a.batch(&format!("CREATE TABLE {t} (a int)")).await;
    a.batch(&format!(
        "CREATE FUNCTION {t}_f(int) RETURNS int LANGUAGE sql VOLATILE \
         AS $$ INSERT INTO {t} VALUES ($1) RETURNING a $$"
    ))
    .await;
}

async fn pg_writer_drop(a: &mut Side, t: &str) {
    a.batch(&format!("DROP FUNCTION {t}_f(int)")).await;
    a.batch(&format!("DROP TABLE {t}")).await;
}

/// [`counting`]'s rows, through [`pg_writer`]'s function: the same values, each one written.
fn writing(t: &str, n: u64) -> String {
    format!("SELECT {t}_f(g::int) AS g FROM generate_series(1, {n}) g")
}

/// A read head of 100 million rows that streams as it is read (no sort): far more than any
/// test reads.
fn endless(line: &Line) -> String {
    match line.engine {
        DbmsKind::Postgres => "SELECT g FROM generate_series(1, 100000000) g".into(),
        DbmsKind::Sqlite => "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c \
                             LIMIT 100000000) SELECT x FROM c"
            .into(),
        _ => "WITH d(x) AS (SELECT 0 UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 \
              UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 \
              UNION ALL SELECT 8 UNION ALL SELECT 9) \
              SELECT a.x + b.x * 10 + c.x * 100 + e.x * 1000 + f.x * 10000 + g.x * 100000 \
              + h.x * 1000000 + i.x * 10000000 AS n \
              FROM d a, d b, d c, d e, d f, d g, d h, d i"
            .into(),
    }
}

/// What the side channel says about the console's connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ServerView {
    /// PostgreSQL: backends of the console's `application_name` idle in a transaction.
    idle_in_tx: i64,
    /// The console's statement still running on the server (PostgreSQL: active backends of its
    /// `application_name`; MySQL family: its thread not sleeping).
    running: i64,
}

async fn server_view(a: &mut Side, c: &Console, backend: Option<&str>) -> ServerView {
    match c.line.engine {
        DbmsKind::Postgres => {
            let app = &c.profile.application_name;
            let idle_in_tx = a
                .count(&format!(
                    "SELECT count(*) FROM pg_stat_activity WHERE application_name = '{app}' \
                     AND state LIKE 'idle in transaction%'"
                ))
                .await;
            let running = a
                .count(&format!(
                    "SELECT count(*) FROM pg_stat_activity WHERE application_name = '{app}' \
                     AND state = 'active'"
                ))
                .await;
            ServerView {
                idle_in_tx,
                running,
            }
        }
        DbmsKind::MySql | DbmsKind::MariaDb => {
            let id = backend.expect("the console's thread id");
            let running = a
                .count(&format!(
                    "SELECT COUNT(*) FROM information_schema.processlist \
                     WHERE id = {id} AND command <> 'Sleep'"
                ))
                .await;
            ServerView {
                idle_in_tx: 0,
                running,
            }
        }
        _ => ServerView {
            idle_in_tx: 0,
            running: 0,
        },
    }
}

/// After a cursor ended: the session is idle with no cursor and no transaction of anyone's,
/// the server agrees, the connection is the same one, and the next statement runs.
async fn assert_freed(c: &Console, a: &mut Side, backend: Option<&str>, what: &str) {
    let id = &c.line.id;
    let mut state = c.s.state();
    tokio::time::timeout(
        secs(30),
        state.wait_for(|s| s.state == SessionState::Idle && s.cursor.is_none()),
    )
    .await
    .unwrap_or_else(|_| panic!("{id} {what}: the session stayed busy: {:?}", c.snap()))
    .expect("the session is alive");
    let snap = c.snap();
    assert_eq!(snap.tx.status, TxStatus::Idle, "{id} {what}: {:?}", snap.tx);
    assert_eq!(
        server_view(a, c, backend).await,
        ServerView {
            idle_in_tx: 0,
            running: 0
        },
        "{id} {what}: the server still holds something"
    );
    if let Some(b) = backend {
        assert_eq!(
            c.backend_id().await,
            b,
            "{id} {what}: the connection was replaced"
        );
    }
    let one = c.ok("SELECT 1").await;
    assert_eq!(cell(one.tab(0), 0, 0), Value::Int(1), "{id} {what}");
}

/// The console's own backend id on the MySQL family (its thread is checked by id).
async fn mysql_backend(c: &Console) -> Option<String> {
    if c.line.is_mysql_family() {
        Some(c.backend_id().await)
    } else {
        None
    }
}

/// [`mysql_backend`] on a read-only console: through Run, because the guard sends an internal
/// `Exec` with the extended protocol, which brings no values back on MySQL.
async fn ro_mysql_backend(c: &Console) -> Option<String> {
    if !c.line.is_mysql_family() {
        return None;
    }
    let ran = c.ok("SELECT CONNECTION_ID()").await;
    Some(int(&cell(ran.tab(0), 0, 0)).to_string())
}

// ---------------------------------------------------------------------------------------------
// Golden tables

fn dec(s: &str) -> Value {
    Value::Decimal(DecimalVal::parse(s).unwrap())
}

fn text(s: &str) -> Value {
    Value::Text(s.into())
}

fn bytes(b: &[u8]) -> Value {
    Value::Bytes(b.into())
}

fn opaque(type_name: &str, text: &str) -> Value {
    Value::Opaque(Arc::new(OpaqueVal {
        type_name: type_name.into(),
        text: text.into(),
    }))
}

fn civil(y: i32, m: u8, d: u8) -> DateVal {
    DateVal::from_civil(y, m, d).unwrap()
}

fn date(y: i32, m: u8, d: u8) -> Value {
    Value::Date(civil(y, m, d))
}

/// Seconds since 1970 of a civil date and time (UTC).
fn epoch(y: i32, mo: u8, d: u8, h: i64, mi: i64, s: i64) -> i64 {
    i64::from(civil(y, mo, d).0) * 86_400 + h * 3_600 + mi * 60 + s
}

fn ts(secs: i64, nanos: u32) -> Value {
    Value::Timestamp(TsVal { secs, nanos })
}

const H: i64 = 3_600_000_000;

/// The golden table's fixture statements on `line` (`t` names the table; PostgreSQL's enum
/// is `<t>_mood`).
fn golden_fixture(line: &Line, t: &str) -> Vec<String> {
    match line.engine {
        DbmsKind::Postgres => vec![
            format!("CREATE TYPE {t}_mood AS ENUM ('sad', 'ok', 'happy')"),
            format!(
                "CREATE TABLE {t} (id int4, b bool, i2 int2, i8 int8, f8 float8, \
                 n numeric(10,2), t text, vc varchar(10), by bytea, u uuid, jb jsonb, d date, \
                 ts timestamp, tstz timestamptz, iv interval, e {t}_mood, ai int4[], ine inet)"
            ),
            format!(
                "INSERT INTO {t} VALUES \
                 (1, true, -2, 9223372036854775807, 0.5, -1.50, 'ş İ ı ğ Ü ç', 'abc', \
                  '\\x00ff0a', '6f1e9a0c-0000-4000-8000-000000000001', '{{\"a\": 1}}', \
                  '2000-01-01', '1999-12-31 23:59:59.999999', '2026-11-11 07:00:00+00', \
                  '1 year 2 mons 3 days 04:05:06.000007', 'happy', '{{1,NULL,3}}', \
                  '192.168.0.1/24'), \
                 (2, false, 0, 0, 'NaN', 'NaN', '', '', '\\x', \
                  '00000000-0000-0000-0000-000000000000', '[]', 'infinity', '-infinity', \
                  'infinity', '0 seconds', 'sad', '{{}}', '::1'), \
                 (3, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, \
                  NULL, NULL, NULL, NULL, NULL)"
            ),
        ],
        DbmsKind::Sqlite => vec![
            format!(
                "CREATE TABLE {t} (id INTEGER PRIMARY KEY, r REAL, t TEXT, b BLOB, \
                 n NUMERIC(10,2), flag BOOLEAN, d DATE, plain)"
            ),
            format!(
                "INSERT INTO {t} VALUES \
                 (1, 1.5, 'ş İ ı ğ Ü ç', x'00ff', 1.25, 1, '2026-11-14', 'x'), \
                 (2, NULL, NULL, NULL, NULL, NULL, NULL, NULL), \
                 (3, -0.5, '', x'', 3, 0, 20261114, x'01')"
            ),
        ],
        _ => vec![
            format!(
                "CREATE TABLE {t} (id INT PRIMARY KEY, ti TINYINT, bu BIGINT UNSIGNED, \
                 d10 DECIMAL(10,2), db DOUBLE, vc VARCHAR(30), tx TEXT, \
                 en ENUM('small','medium'), vb VARBINARY(8), js JSON, dt DATE, \
                 dtm DATETIME(6), tm TIME(2), bt BIT(5)) DEFAULT CHARSET = utf8mb4"
            ),
            format!(
                "INSERT INTO {t} VALUES \
                 (1, -128, 18446744073709551615, 12345678.90, 1.5, 'İstanbul ğüşıöç', \
                  'metin', 'medium', X'DEADBEEF', '{{\"a\": [1, 2]}}', '2024-02-29', \
                  '2024-02-29 13:14:15.123456', '13:14:15.25', b'10110'), \
                 (2, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, \
                  NULL), \
                 (3, 0, 0, -0.50, -2.5e-10, '', '', 'small', X'', 'null', '1970-01-01', \
                  '1970-01-01 00:00:01', '-01:30:00.50', b'00000')"
            ),
        ],
    }
}

fn golden_drop(line: &Line, t: &str) -> Vec<String> {
    match line.engine {
        DbmsKind::Postgres => vec![
            format!("DROP TABLE IF EXISTS {t}"),
            format!("DROP TYPE IF EXISTS {t}_mood"),
        ],
        _ => vec![format!("DROP TABLE IF EXISTS {t}")],
    }
}

/// What the store holds for the golden rows on `line` (from the fixture's literals and each
/// engine's documented type mapping, R-M3-31).
fn golden_rows(line: &Line) -> Vec<Vec<Value>> {
    match line.engine {
        DbmsKind::Postgres => {
            let uuid: [u8; 16] = [
                0x6f, 0x1e, 0x9a, 0x0c, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 0, 0, 1,
            ];
            vec![
                vec![
                    Value::Int(1),
                    Value::Bool(true),
                    Value::Int(-2),
                    Value::Int(i64::MAX),
                    Value::F64(0.5),
                    dec("-1.50"),
                    text("ş İ ı ğ Ü ç"),
                    text("abc"),
                    bytes(&[0, 0xff, 0x0a]),
                    Value::Uuid(uuid),
                    Value::Json("{\"a\": 1}".into()),
                    date(2000, 1, 1),
                    ts(epoch(1999, 12, 31, 23, 59, 59), 999_999_000),
                    Value::TimestampTz {
                        utc: TsVal {
                            secs: epoch(2026, 11, 11, 7, 0, 0),
                            nanos: 0,
                        },
                        offset: None,
                    },
                    Value::Interval {
                        months: 14,
                        days: 3,
                        micros: ((4 * 60 + 5) * 60 + 6) * 1_000_000 + 7,
                    },
                    text("happy"),
                    opaque("_int4", "{1,NULL,3}"),
                    opaque("inet", "192.168.0.1/24"),
                ],
                vec![
                    Value::Int(2),
                    Value::Bool(false),
                    Value::Int(0),
                    Value::Int(0),
                    Value::F64(f64::NAN),
                    Value::Special(SpecialVal::NaN),
                    text(""),
                    text(""),
                    bytes(&[]),
                    Value::Uuid([0; 16]),
                    Value::Json("[]".into()),
                    Value::Special(SpecialVal::PosInfinity),
                    Value::Special(SpecialVal::NegInfinity),
                    Value::Special(SpecialVal::PosInfinity),
                    Value::Interval {
                        months: 0,
                        days: 0,
                        micros: 0,
                    },
                    text("sad"),
                    opaque("_int4", "{}"),
                    opaque("inet", "::1"),
                ],
                std::iter::once(Value::Int(3))
                    .chain((0..17).map(|_| Value::Null))
                    .collect(),
            ]
        }
        DbmsKind::Sqlite => vec![
            vec![
                Value::Int(1),
                Value::F64(1.5),
                text("ş İ ı ğ Ü ç"),
                bytes(&[0, 0xff]),
                Value::F64(1.25),
                Value::Bool(true),
                text("2026-11-14"),
                text("x"),
            ],
            std::iter::once(Value::Int(2))
                .chain((0..7).map(|_| Value::Null))
                .collect(),
            vec![
                Value::Int(3),
                Value::F64(-0.5),
                text(""),
                bytes(&[]),
                // An integer stays exact in a REAL buffer (Task 15).
                Value::Int(3),
                Value::Bool(false),
                Value::Int(20_261_114),
                bytes(&[1]),
            ],
        ],
        engine => {
            let json = |s: &str| match engine {
                // MariaDB's JSON is LONGTEXT (with a check constraint): text on the wire.
                DbmsKind::MariaDb => text(s),
                _ => Value::Json(s.into()),
            };
            let leap_day = epoch(2024, 2, 29, 0, 0, 0);
            vec![
                vec![
                    Value::Int(1),
                    Value::Int(-128),
                    Value::UInt(u64::MAX),
                    dec("12345678.90"),
                    Value::F64(1.5),
                    text("İstanbul ğüşıöç"),
                    text("metin"),
                    text("medium"),
                    bytes(&[0xDE, 0xAD, 0xBE, 0xEF]),
                    json(r#"{"a": [1, 2]}"#),
                    date(2024, 2, 29),
                    ts(leap_day + 13 * 3_600 + 14 * 60 + 15, 123_456_000),
                    Value::Time(TimeVal::from_micros(13 * H + 14 * 60_000_000 + 15_250_000)),
                    opaque("bit", "10110"),
                ],
                std::iter::once(Value::Int(2))
                    .chain((0..13).map(|_| Value::Null))
                    .collect(),
                vec![
                    Value::Int(3),
                    Value::Int(0),
                    Value::UInt(0),
                    dec("-0.50"),
                    Value::F64(-2.5e-10),
                    text(""),
                    text(""),
                    text("small"),
                    bytes(&[]),
                    json("null"),
                    date(1970, 1, 1),
                    ts(1, 0),
                    // A negative TIME is an interval (Task 14).
                    Value::Interval {
                        months: 0,
                        days: 0,
                        micros: -(H + 30 * 60_000_000 + 500_000),
                    },
                    opaque("bit", "00000"),
                ],
            ]
        }
    }
}

/// The column types the golden check pins per engine: (column, logical type).
fn golden_types(line: &Line) -> Vec<(&'static str, LogicalType)> {
    let dec = |p, s| LogicalType::Decimal {
        precision: Some(p),
        scale: Some(s),
    };
    match line.engine {
        DbmsKind::Postgres => vec![
            ("n", dec(10, 2)),
            (
                "tstz",
                LogicalType::Timestamp {
                    precision: 6,
                    tz: true,
                },
            ),
            (
                "e",
                LogicalType::Enum(vec!["sad".into(), "ok".into(), "happy".into()].into()),
            ),
        ],
        DbmsKind::Sqlite => vec![
            ("n", LogicalType::Float { bits: 64 }),
            ("flag", LogicalType::Bool),
            (
                "d",
                LogicalType::Text {
                    max_chars: None,
                    fixed: false,
                },
            ),
        ],
        _ => vec![
            ("d10", dec(10, 2)),
            (
                "bu",
                LogicalType::Int {
                    bits: 64,
                    signed: false,
                },
            ),
            (
                "dtm",
                LogicalType::Timestamp {
                    precision: 6,
                    tz: false,
                },
            ),
            (
                "bt",
                LogicalType::Opaque {
                    type_name: "bit".into(),
                },
            ),
        ],
    }
}

fn check_golden(line: &Line, schema: &ResultSchema, got: &[Vec<Value>]) {
    let id = &line.id;
    let want = golden_rows(line);
    assert_eq!(got.len(), want.len(), "{id}: rows");
    for (r, (g, w)) in got.iter().zip(&want).enumerate() {
        assert_eq!(g.len(), w.len(), "{id}: row {r} width");
        for (c, (gv, wv)) in g.iter().zip(w).enumerate() {
            assert!(
                same(gv, wv),
                "{id}: row {r} column {}: got {gv:?}, want {wv:?}",
                schema.columns[c].name
            );
        }
    }
    for (name, want) in golden_types(line) {
        let col = schema
            .columns
            .iter()
            .find(|c| &*c.name == name)
            .unwrap_or_else(|| panic!("{id}: no column {name}"));
        assert_eq!(col.logical, want, "{id}: column {name}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn typed_golden_tables_per_engine_and_version() {
    if !docker_enabled() {
        return;
    }
    let _quiet = QUIET.read().await;
    let record = Record::new();
    let lines = lines();
    for line in &lines {
        let table = unique("hv_t31_types");
        let mut a = admin(line).await;
        for sql in golden_fixture(line, &table) {
            a.batch(&sql).await;
        }
        let c = Console::dev(line).await;
        check_version(&c);
        let ran = c.ok(&format!("SELECT * FROM {table} ORDER BY id")).await;
        assert_eq!(ran.names(), ["Succeeded"], "{}", line.id);
        assert_eq!(rows_of(ran.unit(0)), fetched(3, false), "{}", line.id);
        assert_eq!(c.log_ids(), ["exec-rows-retrieved"], "{}", line.id);
        let snap = ran.tab(0).source.snapshot();
        assert_eq!(*snap.completeness(), Completeness::Complete, "{}", line.id);
        let width = u32::try_from(snap.schema().columns.len()).unwrap();
        let got: Vec<Vec<Value>> = (0..snap.row_count())
            .map(|r| (0..width).map(|c| snap.cell(r, c).to_value()).collect())
            .collect();
        check_golden(line, snap.schema(), &got);
        // Nothing of Hacivat's is left open after a cursor that completed, as the server sees
        // it (review M5).
        let backend = mysql_backend(&c).await;
        assert_freed(&c, &mut a, backend.as_deref(), "golden").await;
        record.add(
            line,
            c.version.as_ref(),
            "golden",
            &format!("{} rows x {width} columns", got.len()),
        );
        c.shutdown().await;
        for sql in golden_drop(line, &table) {
            a.batch(&sql).await;
        }
        a.close().await;
    }
    assert_every_line_ran(&record, &lines);
    record.write("golden");
}

// ---------------------------------------------------------------------------------------------
// Cursors

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn select_streams_suspends_fetches_more_and_closes() {
    if !docker_enabled() {
        return;
    }
    let _quiet = QUIET.read().await;
    let record = Record::new();
    let lines = lines();
    for line in &lines {
        let id = &line.id;
        let mut a = admin(line).await;
        let c = Console::dev(line).await;
        let backend = mysql_backend(&c).await;
        // On PostgreSQL the read head writes each row it reads (review I1), so the server
        // shows whether Hacivat's transaction rolled back (Close) or committed (Fetch all).
        let t = unique("hv_t31_cw");
        if line.is_pg() {
            pg_writer(&mut a, &t).await;
        }
        let read_head = |n| {
            if line.is_pg() {
                writing(&t, n)
            } else {
                counting(line, n)
            }
        };
        let written = format!("SELECT count(*) FROM {t}");

        // A read head suspends at its read-ahead, with the first rows in order.
        let ran = c.ok(&read_head(100_000)).await;
        assert_eq!(ran.names(), ["Suspended"], "{id}");
        assert!(
            matches!(ran.unit(0), UnitStatus::Suspended { rows: 10_000 }),
            "{id}: {:?}",
            ran.unit(0)
        );
        let src = ran.tab(0).source.clone();
        assert!(
            matches!(src.snapshot().completeness(), Completeness::Suspended(_)),
            "{id}"
        );
        assert_eq!(src.snapshot().row_count(), 10_000, "{id}");
        assert_eq!(
            (
                int(&cell(ran.tab(0), 0, 0)),
                int(&cell(ran.tab(0), 9_999, 0))
            ),
            (1, 10_000),
            "{id}"
        );
        assert_eq!(c.log_ids(), ["exec-rows-retrieved-more"], "{id}");
        let snap = c.settled().await;
        assert_eq!(snap.cursor.map(|b| b.rows), Some(10_000), "{id}");
        if line.is_pg() {
            // Hacivat's own transaction holds the portal; it is never the user's.
            assert_eq!(
                (snap.tx.status, snap.tx.owner),
                (TxStatus::InTx, TxOwner::Cursor),
                "{id}"
            );
            assert_eq!(server_view(&mut a, &c, None).await.idle_in_tx, 1, "{id}");
        }

        // The grid reads near the loaded end: one fetch-more, the rows continue in order.
        src.ensure_rows(9_999);
        wait_until(&format!("{id}: more rows"), secs(30), || {
            let s = src.snapshot();
            s.row_count() > 10_000 && matches!(s.completeness(), Completeness::Suspended(_))
        })
        .await;
        let loaded = src.snapshot().row_count();
        assert_eq!(
            int(&cell(ran.tab(0), loaded - 1, 0)),
            i64::try_from(loaded).unwrap(),
            "{id}"
        );

        // Close cursor: the rows stay, the unit says more were available, the log says why.
        src.close_cursor();
        wait_until(&format!("{id}: the close"), secs(30), || {
            matches!(
                src.snapshot().completeness(),
                Completeness::Closed {
                    reason: CloseReason::UserClosed
                }
            )
        })
        .await;
        assert_eq!(src.snapshot().row_count(), loaded, "{id}: the rows stay");
        assert_freed(&c, &mut a, backend.as_deref(), "after Close cursor").await;
        assert_eq!(
            rows_of(&ran.unit_now(0)),
            fetched(loaded, true),
            "{id}: the unit follows its result"
        );
        let closed = if line.is_pg() {
            "cursor-closed-user-closed-rolled-back"
        } else {
            "cursor-closed-user-closed"
        };
        assert!(c.log_ids().contains(&closed), "{id}: {:?}", c.log_ids());
        if line.is_pg() {
            assert_eq!(
                a.count(&written).await,
                0,
                "{id}: rolled back on the server"
            );
        }
        record.add(
            line,
            c.version.as_ref(),
            "suspend 10000, fetch more, close",
            &format!("{loaded} rows kept; {closed}"),
        );

        // Fetch all: a result a little past the read-ahead completes, and says so.
        c.log.clear();
        let ran = c.ok(&read_head(30_000)).await;
        let src = ran.tab(0).source.clone();
        src.fetch_all();
        wait_until(&format!("{id}: fetch all"), secs(30), || {
            *src.snapshot().completeness() == Completeness::Complete
        })
        .await;
        assert_eq!(src.snapshot().row_count(), 30_000, "{id}");
        assert_eq!(int(&cell(ran.tab(0), 29_999, 0)), 30_000, "{id}");
        wait_until(&format!("{id}: the unit completes"), secs(10), || {
            rows_of(&ran.unit_now(0)) == fetched(30_000, false)
        })
        .await;
        assert_eq!(
            c.log_ids(),
            ["exec-rows-retrieved-more", "exec-rows-retrieved"],
            "{id}"
        );
        assert_freed(&c, &mut a, backend.as_deref(), "after Fetch all").await;
        if line.is_pg() {
            assert_eq!(
                a.count(&written).await,
                30_000,
                "{id}: committed on the server"
            );
        }
        record.add(
            line,
            c.version.as_ref(),
            "fetch all",
            "30000 rows, complete",
        );
        c.shutdown().await;
        if line.is_pg() {
            pg_writer_drop(&mut a, &t).await;
        }
        a.close().await;
    }
    assert_every_line_ran(&record, &lines);
    record.write("cursor");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn stop_fetching_keeps_rows_and_frees_the_session() {
    if !docker_enabled() {
        return;
    }
    let _quiet = QUIET.read().await;
    let record = Record::new();
    let lines = lines();
    for line in &lines {
        let id = &line.id;
        let mut a = admin(line).await;
        let c = Console::dev(line).await;
        let backend = mysql_backend(&c).await;
        let ran = c.ok(&endless(line)).await;
        let src = ran.tab(0).source.clone();
        // Fetch all of an endless stream, then Stop while the rows flow.
        src.fetch_all();
        wait_until(&format!("{id}: rows flow"), secs(60), || {
            src.snapshot().row_count() >= 50_000
        })
        .await;
        let mut req = None;
        wait_until(&format!("{id}: the fetch is the request"), secs(10), || {
            req = c.busy_request();
            req.is_some()
        })
        .await;
        let outcome = c.s.cancel(req.unwrap()).await.expect("the cancel answers");
        assert_eq!(outcome, CancelOutcome::Cancelled, "{id}");
        wait_until(&format!("{id}: the result stops"), secs(30), || {
            *src.snapshot().completeness() == Completeness::Cancelled
        })
        .await;
        // The rows stay, in order.
        let kept = src.snapshot().row_count();
        assert!(kept >= 50_000, "{id}: {kept}");
        if !line.is_mysql_family() {
            assert_eq!(
                int(&cell(ran.tab(0), kept - 1, 0)),
                i64::try_from(kept).unwrap(),
                "{id}"
            );
        }
        assert_freed(&c, &mut a, backend.as_deref(), "after Stop").await;
        assert_eq!(
            src.snapshot().row_count(),
            kept,
            "{id}: nothing changed the kept rows"
        );
        let unit = ran.unit_now(0);
        record.add(
            line,
            c.version.as_ref(),
            "stop fetching",
            &format!("{kept} rows kept; unit {}; {:?}", unit.name(), c.log_ids()),
        );
        c.shutdown().await;
        a.close().await;
    }
    assert_every_line_ran(&record, &lines);
    record.write("stop");
}

// ---------------------------------------------------------------------------------------------
// Cancel

/// How a cancelled case ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ends {
    /// The server stops the statement with an error: the unit is `Cancelled`.
    Cancelled,
    /// The server's cancel ends the statement normally (MySQL's `KILL QUERY` interrupts
    /// `SLEEP()`, which then returns 1 without an error, as the MySQL manual says; MariaDB's
    /// `DO` swallows the interruption): the unit succeeds and the cancel answers
    /// `AlreadyFinished`. The stop still lands within the budget and frees the server.
    Completes,
}

/// 10⁹ rows counted from a ten-row CTE: a statement that computes for many seconds before
/// its first row, and that `KILL QUERY` ends with an error (1317).
const MYSQL_BILLION: &str = "WITH d(x) AS (SELECT 0 UNION ALL SELECT 1 UNION ALL SELECT 2 \
    UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL \
    SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) \
    SELECT COUNT(*) FROM d a, d b, d c, d e, d f, d g, d h, d i, d j";

/// The cancel cases of `line`: (case, statement, how it ends). Each computes for a minute or
/// more before its first row.
fn cancel_cases(line: &Line, t: &str) -> Vec<(&'static str, String, Ends)> {
    match line.engine {
        DbmsKind::Postgres => vec![
            ("read head", "SELECT pg_sleep(60)".into(), Ends::Cancelled),
            (
                "not a read head",
                "DO $$ BEGIN PERFORM pg_sleep(60); END $$".into(),
                Ends::Cancelled,
            ),
        ],
        DbmsKind::Sqlite => {
            let forever = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) \
                           SELECT count(*) AS n FROM c";
            vec![
                ("read head", forever.into(), Ends::Cancelled),
                (
                    "not a read head",
                    format!("CREATE TABLE {t} AS {forever}"),
                    Ends::Cancelled,
                ),
            ]
        }
        engine => {
            let mut cases = vec![
                ("read head", MYSQL_BILLION.into(), Ends::Cancelled),
                // DML (the table is the test's fixture): the kill ends it with 1317 and the
                // statement's rollback leaves no row.
                (
                    "not a read head",
                    // MySQL takes `INSERT … WITH … SELECT` and honours the kill at once in the
                    // CTE's plan (the derived-table plan took up to 1.2 s on mysql80); MariaDB
                    // runs the derived-table form.
                    if engine == DbmsKind::MariaDb {
                        format!("INSERT INTO {t} (n) {}", mysql_billion_derived())
                    } else {
                        format!("INSERT INTO {t} (n) {MYSQL_BILLION}")
                    },
                    Ends::Cancelled,
                ),
                // The server's semantics, pinned: a killed `SLEEP()` returns 1 without an
                // error on MySQL (its manual), and MariaDB reports the interruption.
                (
                    "SLEEP, read head",
                    "SELECT SLEEP(60) AS s".into(),
                    if engine == DbmsKind::MariaDb {
                        Ends::Cancelled
                    } else {
                        Ends::Completes
                    },
                ),
                (
                    "SLEEP, not a read head",
                    "DO SLEEP(60)".into(),
                    Ends::Completes,
                ),
            ];
            // MariaDB's `DO` swallows the interruption (pinned). MySQL 8.0's `DO` or `SET` of a
            // killed subquery ends normally in some runs and with 1317 in others, so it is not
            // a case.
            if engine == DbmsKind::MariaDb {
                cases.push((
                    "DO, not a read head",
                    format!("DO ({MYSQL_BILLION})"),
                    Ends::Completes,
                ));
            }
            cases
        }
    }
}

/// [`MYSQL_BILLION`]'s count over derived tables, for MariaDB's `INSERT … SELECT` (without a
/// CTE).
fn mysql_billion_derived() -> String {
    let digits = "(SELECT 0 AS x UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 \
                  UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 \
                  UNION ALL SELECT 8 UNION ALL SELECT 9)";
    let from: Vec<String> = ["a", "b", "c", "e", "f", "g", "h", "i", "j"]
        .iter()
        .map(|alias| format!("{digits} {alias}"))
        .collect();
    format!("SELECT COUNT(*) FROM {}", from.join(", "))
}

/// A read head whose first rows come at once and whose next row takes minutes: the cursor
/// suspends among the fast rows, and Fetch all then sits in one long step (a chunk that is
/// overdue on the server: PostgreSQL's cancel, MySQL's `KILL QUERY` on its side connection,
/// SQLite's interrupt; the ledger's Task 18 note). MariaDB materialises a `UNION ALL` before
/// its first row, so its shape is a sequence-engine scan whose last row sleeps (review M6).
/// Returns the statement and its fast rows.
fn slow_tail(line: &Line) -> Option<(String, u64)> {
    match line.engine {
        DbmsKind::Postgres => Some((
            "SELECT g FROM generate_series(1, 20000) g UNION ALL SELECT 0 FROM pg_sleep(60)".into(),
            20_000,
        )),
        DbmsKind::Sqlite => Some((
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) \
             SELECT x FROM c WHERE x <= 20000 OR x % 1000000000 = 0"
                .into(),
            20_000,
        )),
        DbmsKind::MySql => Some((
            "WITH d(x) AS (SELECT 0 UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 \
             UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 \
             UNION ALL SELECT 8 UNION ALL SELECT 9) \
             SELECT a.x + b.x * 10 + c.x * 100 + e.x * 1000 + f.x * 10000 AS n \
             FROM d a, d b, d c, d e, d f \
             UNION ALL SELECT COUNT(*) FROM d a, d b, d c, d e, d f, d g, d h, d i, d j"
                .into(),
            100_000,
        )),
        DbmsKind::MariaDb => Some((
            "SELECT IF(seq <= 20000, seq, SLEEP(60)) AS n FROM seq_1_to_20001".into(),
            20_000,
        )),
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn cancel_within_one_second_on_every_engine() {
    if !docker_enabled() {
        return;
    }
    // The budget is measured on a quiet matrix.
    let _quiet = QUIET.write().await;
    let record = Record::new();
    let mut failures = Vec::new();
    let lines = lines();
    for line in &lines {
        let id = &line.id;
        let mut a = admin(line).await;
        let c = Console::dev(line).await;
        let backend = mysql_backend(&c).await;
        let table = unique("hv_t31_cancel");
        if line.is_mysql_family() {
            a.batch(&format!("CREATE TABLE {table} (n BIGINT)")).await;
        }
        for (case, sql, ends) in cancel_cases(line, &table) {
            c.log.clear();
            let t = c.start(&sql);
            let seq = t.seq;
            let status = t.status.clone();
            // The statement has run past the overdue mark (R-M3-16: 1 s), so Stop sends the
            // server cancel at once: the server, or the run status on SQLite, says so.
            match line.engine {
                DbmsKind::Postgres => {
                    let app = &c.profile.application_name;
                    let sql = format!(
                        "SELECT count(*) FROM pg_stat_activity WHERE application_name = \
                         '{app}' AND state = 'active' \
                         AND clock_timestamp() - query_start > interval '1100 milliseconds'"
                    );
                    wait_count(&mut a, &format!("{id} {case}: running"), secs(30), &sql, 1).await;
                }
                DbmsKind::Sqlite => {
                    wait_until(&format!("{id} {case}: running"), secs(30), || {
                        matches!(
                            status.borrow().units[0].status,
                            UnitStatus::Running { since }
                                if since.elapsed() > std::time::Duration::from_millis(1_100)
                        )
                    })
                    .await;
                }
                _ => {
                    let thread = backend.as_deref().unwrap();
                    let sql = format!(
                        "SELECT COUNT(*) FROM information_schema.processlist \
                         WHERE id = {thread} AND command <> 'Sleep' AND time >= 2"
                    );
                    wait_count(&mut a, &format!("{id} {case}: running"), secs(30), &sql, 1).await;
                }
            }
            let started = Instant::now();
            let outcome = c.s.cancel(seq).await.expect("the cancel answers");
            let ran = ended(t).await;
            let took = started.elapsed();
            let seen = format!(
                "{outcome:?}; {:?} {}; {:?}; {} ms",
                ran.names(),
                ran.outcome(),
                c.log_ids(),
                took.as_millis()
            );
            let as_expected = match ends {
                Ends::Cancelled => {
                    outcome == CancelOutcome::Cancelled
                        && ran.names() == ["Cancelled"]
                        && ran.summary.aborted
                        && c.log_ids().contains(&"exec-cancelled")
                }
                Ends::Completes => {
                    outcome == CancelOutcome::AlreadyFinished
                        && ran.names() == ["Succeeded"]
                        && !c.log_ids().contains(&"exec-cancelled")
                        // `SLEEP()` interrupted: 1.
                        && ran
                            .tabs
                            .first()
                            .is_none_or(|tab| cell(tab, 0, 0) == Value::Int(1))
                }
            };
            // The plan's budget (kept by the Docker rule, with margin).
            if !as_expected || took >= secs(1) {
                failures.push(format!("{id} {case} (expected {ends:?}): {seen}"));
            }
            // The server stopped it, and the console goes on on the same connection.
            assert_freed(&c, &mut a, backend.as_deref(), case).await;
            if line.is_sqlite() && case == "not a read head" {
                assert_eq!(
                    a.count(&format!(
                        "SELECT count(*) FROM sqlite_master WHERE name = '{table}'"
                    ))
                    .await,
                    0,
                    "{id}: the interrupted CREATE TABLE AS left nothing"
                );
            }
            if line.is_mysql_family() && case == "not a read head" {
                assert_eq!(
                    a.count(&format!("SELECT COUNT(*) FROM {table}")).await,
                    0,
                    "{id}: the interrupted INSERT left no row"
                );
            }
            record.add(
                line,
                c.version.as_ref(),
                &format!("cancel ({case})"),
                &format!(
                    "{outcome:?}, unit {} in {} ms",
                    ran.names().join(","),
                    took.as_millis()
                ),
            );
        }

        // Stop while a fetch sits in a long step: the chunk is overdue, so the server cancel
        // goes out at once (R-M3-16), the rows already fetched stay.
        if let Some((sql, fast)) = slow_tail(line) {
            let case = "fetch in a long step";
            c.log.clear();
            let ran = c.ok(&sql).await;
            assert_eq!(ran.names(), ["Suspended"], "{id} {case}");
            let src = ran.tab(0).source.clone();
            src.fetch_all();
            // The fetch got past the read-ahead, and no chunk has come for more than the
            // overdue mark (the last chunk of fast rows waits in the driver until it is
            // sealed, so the store holds whole chunks only).
            let mut last = (0, Instant::now());
            wait_until(&format!("{id} {case}: the long step"), secs(60), || {
                let n = src.snapshot().row_count();
                if n != last.0 {
                    last = (n, Instant::now());
                }
                n > 10_000
                    && c.busy_request().is_some()
                    && last.1.elapsed() > std::time::Duration::from_millis(1_100)
            })
            .await;
            let stuck_at = last.0;
            let req = c.busy_request().expect("the fetch is the request");
            let started = Instant::now();
            let outcome = c.s.cancel(req).await.expect("the cancel answers");
            let took = started.elapsed();
            wait_until(&format!("{id} {case}: the result stops"), secs(10), || {
                *src.snapshot().completeness() == Completeness::Cancelled
            })
            .await;
            let seen = format!(
                "{outcome:?}; {} rows; {:?}; {} ms",
                src.snapshot().row_count(),
                c.log_ids(),
                took.as_millis()
            );
            eprintln!("{id} {case}: {seen}");
            let kept = src.snapshot().row_count();
            if outcome != CancelOutcome::Cancelled
                || !(stuck_at..=fast).contains(&kept)
                || took >= secs(1)
            {
                failures.push(format!("{id} {case}: {seen}"));
            }
            assert_freed(&c, &mut a, backend.as_deref(), case).await;
            record.add(
                line,
                c.version.as_ref(),
                &format!("stop ({case})"),
                &format!("{outcome:?} in {} ms", took.as_millis()),
            );
        }
        c.shutdown().await;
        if line.is_mysql_family() {
            a.batch(&format!("DROP TABLE {table}")).await;
        }
        a.close().await;
    }
    assert_every_line_ran(&record, &lines);
    record.write("cancel");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// ---------------------------------------------------------------------------------------------
// Read-only and production

/// One bypass row: (catalogue id, statement with `{t}` for the fixture table, the outcomes
/// that count as stopped).
type Row = (&'static str, &'static str, &'static [&'static str]);

const PG_RO: &str = "25006";

// ## m3a-f5-demo (DEMO-7): a statement whose first keyword changes data or schema is
// refused by the console before anything is sent (`INPUT`); the rows a first keyword does not
// show still meet the guard and the server.

/// The client-side refusal: "Not sent: this console is read-only …" (`DenyLayer::Input`).
const INPUT: &str = "input";

/// The PostgreSQL rows (run B's server-blocked catalogue on this task's fixture, plus plain
/// writes): every one goes through Run with no classifier. Ordered so the one that ends in
/// a gap (which discards the connection) comes last.
const PG_ROWS: &[Row] = &[
    ("W1", "INSERT INTO {t} VALUES (7)", &[INPUT]),
    ("W2", "UPDATE {t} SET a = a + 1", &[INPUT]),
    ("W3", "DELETE FROM {t}", &[INPUT]),
    ("W4", "TRUNCATE {t}", &[INPUT]),
    ("W5", "DROP TABLE {t}", &[INPUT]),
    // F5 review m-1: `ANALYZE` runs inside a read-only transaction and its statistics outlive
    // the rollback.
    ("W7", "ANALYZE {t}", &[INPUT]),
    (
        "P1",
        "WITH d AS (DELETE FROM {t} RETURNING *) SELECT * FROM d",
        &[PG_RO],
    ),
    ("P2", "SELECT * INTO {t}_p2 FROM {t}", &[PG_RO]),
    ("P3", "EXPLAIN (ANALYZE) DELETE FROM {t}", &[PG_RO]),
    // Split by the console into `SELECT 1` and the DROP, which is never sent.
    ("P4", "SELECT 1; DROP TABLE {t}", &[INPUT]),
    ("P5", "SET TRANSACTION READ WRITE", &["25001"]),
    (
        "P5",
        "SELECT set_config('transaction_read_only', 'off', true)",
        &["25001"],
    ),
    ("P7", "COMMIT", &["1a"]),
    ("P7", "/* x */ END", &["1a"]),
    ("P7", "ROLLBACK", &["1a"]),
    ("P8", "CALL {t}_commit()", &[INPUT]),
    (
        "P9",
        "DO $$ BEGIN INSERT INTO {t} VALUES (9); END $$",
        &[INPUT],
    ),
    ("P14", "COPY {t} FROM '/etc/hostname'", &["protocol"]),
    ("P26", "CREATE TEMP TABLE {t}_tmp (a int)", &[INPUT]),
    ("P37", "SELECT nextval('{t}_seq')", &[PG_RO]),
    ("P38", "SELECT {t}_sd_write()", &[PG_RO]),
    (
        "P32",
        "SELECT pg_logical_emit_message(false, 'hv_t31', 'x')",
        &["function"],
    ),
    ("P35", "SELECT txid_current()", &["gap"]),
];

/// MySQL 1792 (`ER_CANT_EXECUTE_IN_READ_ONLY_TRANSACTION`): the error view's code chip shows
/// its SQLSTATE, which is the vendor code's own (Task 23: the SQLSTATE when there is one).
const MY_RO: &str = "25006";

/// The MySQL and MariaDB rows (run B's on this task's fixture, plus plain writes). The rows
/// that end in a gap on MariaDB (`LOCK TABLES`) come last.
const MYSQL_ROWS: &[Row] = &[
    ("W1", "INSERT INTO {t} VALUES (7)", &[INPUT]),
    ("W2", "UPDATE {t} SET a = a + 1", &[INPUT]),
    ("W3", "DELETE FROM {t}", &[INPUT]),
    ("M1", "/*!50000 INSERT INTO {t} VALUES (50) */", &[INPUT]),
    ("M1", "/*!50000 DROP TABLE {t} */", &[INPUT]),
    // Split by the console into `SELECT 1` and the DROP, which is never sent.
    ("M2", "SELECT 1; DROP TABLE {t}", &[INPUT]),
    ("M5", "SELECT * FROM {t} FOR UPDATE", &[MY_RO]),
    (
        "M6",
        "PREPARE hv_t31_s FROM 'DELETE FROM {t}'",
        &["protocol"],
    ),
    ("M8", "CREATE TABLE {t}_m8 (a int)", &[INPUT]),
    ("M8", "TRUNCATE TABLE {t}", &[INPUT]),
    // F5 review m-1: statistics writes.
    ("W7", "ANALYZE TABLE {t}", &[INPUT]),
    ("M13", "XA START 'hv_t31'", &["protocol", "XAE09"]),
    ("M17", "CREATE TEMPORARY TABLE {t}_tmp (a int)", &[INPUT]),
    ("M7", "/*!50000 COMMIT */", &["1a"]),
    ("M5", "LOCK TABLES {t} WRITE", &["protocol", "gap"]),
];

/// The SQLite rows: writes, schema changes, `ATTACH` and pragmas that write.
const SQLITE_ROWS: &[Row] = &[
    ("W1", "INSERT INTO {t} VALUES (7)", &[INPUT]),
    ("W2", "UPDATE {t} SET a = a + 1", &[INPUT]),
    ("W3", "DELETE FROM {t}", &[INPUT]),
    ("W5", "DROP TABLE {t}", &[INPUT]),
    ("S1", "CREATE TABLE {t}_new (a int)", &[INPUT]),
    ("S2", "CREATE TEMP TABLE {t}_tmp (a int)", &[INPUT]),
    ("S3", "ATTACH DATABASE '{t}_x.db' AS x", &[INPUT]),
    ("S4", "PRAGMA user_version = 7", &["23", "8"]),
    ("S5", "VACUUM", &[INPUT]),
    // F5 review m-1: statistics writes.
    ("S7", "ANALYZE", &[INPUT]),
    ("S6", "SELECT 1; DELETE FROM {t}", &[INPUT]),
    // A write the first keyword does not show meets the authorizer.
    (
        "W6",
        "WITH x AS (SELECT 1 AS a) DELETE FROM {t} WHERE a = 1",
        &["23", "8"],
    ),
];

/// The read-only fixture of `line` (table `t` holding 1, 2, 3; PostgreSQL also a sequence, a
/// writing `SECURITY DEFINER` function and a committing procedure).
fn ro_fixture(line: &Line, t: &str) -> Vec<String> {
    let mut v = vec![
        format!("CREATE TABLE {t} (a int)"),
        format!("INSERT INTO {t} VALUES (1), (2), (3)"),
    ];
    if line.is_pg() {
        v.extend([
            format!("CREATE SEQUENCE {t}_seq"),
            format!(
                "CREATE FUNCTION {t}_sd_write() RETURNS int LANGUAGE sql SECURITY DEFINER \
                 AS $$ INSERT INTO {t} VALUES (99) RETURNING a $$"
            ),
            format!(
                "CREATE PROCEDURE {t}_commit() LANGUAGE plpgsql \
                 AS $$ BEGIN COMMIT; INSERT INTO {t} VALUES (8); COMMIT; END $$"
            ),
        ]);
    }
    v
}

fn ro_drop(line: &Line, t: &str) -> Vec<String> {
    let mut v = Vec::new();
    if line.is_pg() {
        v.extend([
            format!("DROP PROCEDURE IF EXISTS {t}_commit()"),
            format!("DROP FUNCTION IF EXISTS {t}_sd_write()"),
            format!("DROP SEQUENCE IF EXISTS {t}_seq"),
            format!("DROP TABLE IF EXISTS {t}_p2"),
        ]);
    }
    if line.is_mysql_family() {
        v.push(format!("DROP TABLE IF EXISTS {t}_m8"));
    }
    if line.is_sqlite() {
        v.push(format!("DROP TABLE IF EXISTS {t}_new"));
    }
    v.push(format!("DROP TABLE IF EXISTS {t}"));
    v
}

/// Whether table `t` can be locked for writing at once from the side channel: nothing the
/// console ran holds a lock or a transaction on it. PostgreSQL `LOCK … ACCESS EXCLUSIVE
/// NOWAIT`, the MySQL family `SELECT … FOR UPDATE NOWAIT`, SQLite `BEGIN EXCLUSIVE`.
async fn lock_probe(a: &mut Side, line: &Line, t: &str) -> Result<(), String> {
    match line.engine {
        DbmsKind::Postgres => {
            let probed = a
                .try_batch(&format!(
                    "BEGIN; LOCK TABLE {t} IN ACCESS EXCLUSIVE MODE NOWAIT; ROLLBACK"
                ))
                .await;
            if probed.is_err() {
                // The failed probe left the side channel's transaction aborted.
                a.batch("ROLLBACK").await;
            }
            probed
        }
        DbmsKind::Sqlite => {
            let probed = a.try_batch("BEGIN EXCLUSIVE; ROLLBACK").await;
            if probed.is_err() {
                let _ = a.try_batch("ROLLBACK").await;
            }
            probed
        }
        _ => {
            a.try_batch(&format!("SELECT a FROM {t} FOR UPDATE NOWAIT"))
                .await
        }
    }
}

/// What the side channel sees of the fixture: the table's rows, the schema's relations with
/// the fixture's prefix, and (per engine) the sequence, prepared transactions, temporary
/// relations, the file's bytes.
async fn ro_snapshot(a: &mut Side, line: &Line, t: &str) -> String {
    match line.engine {
        DbmsKind::Postgres => {
            a.scalar(&format!(
                "SELECT concat_ws('|', \
                 (SELECT count(*) || ':' || coalesce(sum(a), 0) FROM {t}), \
                 (SELECT string_agg(relname, ',' ORDER BY relname) FROM pg_class \
                  WHERE relname LIKE '{t}%'), \
                 (SELECT last_value || ':' || is_called FROM {t}_seq), \
                 (SELECT count(*) FROM pg_prepared_xacts), \
                 (SELECT count(*) FROM pg_class WHERE relpersistence = 't'))"
            ))
            .await
        }
        DbmsKind::Sqlite => {
            let rows = a
                .scalar(&format!(
                    "SELECT count(*) || ':' || coalesce(sum(a), 0) FROM {t}"
                ))
                .await;
            let schema = a
                .scalar("SELECT group_concat(name, ',') FROM sqlite_master")
                .await;
            let version = a.scalar("PRAGMA user_version").await;
            let file = std::fs::read(line.sqlite_path()).expect("the database file");
            let attached = line
                .sqlite_path()
                .with_file_name(format!("{t}_x.db"))
                .exists();
            format!(
                "{rows}|{schema}|{version}|{}|{attached}",
                file.iter().map(|b| u64::from(*b)).sum::<u64>()
            )
        }
        _ => {
            let db = &line.target().database;
            a.scalar(&format!(
                "SELECT CONCAT_WS('|', \
                 (SELECT CONCAT(COUNT(*), ':', COALESCE(SUM(a), 0)) FROM {t}), \
                 (SELECT GROUP_CONCAT(table_name ORDER BY table_name) \
                  FROM information_schema.tables \
                  WHERE table_schema = '{db}' AND table_name LIKE '{t}%'))"
            ))
            .await
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn read_only_bypass_rows_blocked_through_run_without_classifier() {
    if !docker_enabled() {
        return;
    }
    let _quiet = QUIET.read().await;
    let record = Record::new();
    let mut failures = Vec::new();
    let lines = lines();
    for line in &lines {
        let id = &line.id;
        let t = unique("hv_t31_ro");
        let mut a = admin(line).await;
        for sql in ro_fixture(line, &t) {
            a.batch(&sql).await;
        }
        let rows = match line.engine {
            DbmsKind::Postgres => PG_ROWS,
            DbmsKind::Sqlite => SQLITE_ROWS,
            _ => MYSQL_ROWS,
        };
        // The test account: it may write, so only the read-only enforcement stops a row.
        let mut c = Console::open(
            line,
            Opts {
                read_only: true,
                ..Opts::dev()
            },
        )
        .await;
        assert!(c.snap().read_only, "{id}: the console is read-only");
        let baseline = ro_snapshot(&mut a, line, &t).await;
        let control = format!("SELECT count(*) FROM {t}");
        for (row, template, allowed) in rows {
            let sql = template.replace("{t}", &t);
            let got = match c.try_run(&sql).await {
                Ok(ran) => ran.outcome(),
                Err(resolution) => format!("not sent: {resolution}"),
            };
            if got == "gap" {
                c.recover().await;
            }
            let after = ro_snapshot(&mut a, line, &t).await;
            record.add(line, c.version.as_ref(), &format!("{row} {sql}"), &got);
            if after != baseline {
                failures.push(format!(
                    "{id} {row} `{sql}`: {got}, and the side channel changed: {baseline} → \
                     {after}"
                ));
            } else if !allowed.contains(&got.as_str()) {
                failures.push(format!("{id} {row} `{sql}`: {got}, not one of {allowed:?}"));
            }
            // Nothing is held after the row (review M3): the console's backend is idle out of
            // any transaction, and the fixture can be locked at once from the side channel.
            let backend = ro_mysql_backend(&c).await;
            let view = server_view(&mut a, &c, backend.as_deref()).await;
            if view.idle_in_tx != 0 || view.running != 0 {
                failures.push(format!(
                    "{id} {row} `{sql}`: the server still holds {view:?}"
                ));
            }
            if let Err(code) = lock_probe(&mut a, line, &t).await {
                failures.push(format!("{id} {row} `{sql}`: the lock probe failed: {code}"));
            }
            // The console still runs statements: a refusal is the server's or the guard's,
            // never a console that runs nothing.
            let ran = c.ok(&control).await;
            assert_eq!(int(&cell(ran.tab(0), 0, 0)), 3, "{id} after {row}");
        }
        c.shutdown().await;
        for sql in ro_drop(line, &t) {
            a.batch(&sql).await;
        }
        a.close().await;
    }
    assert_every_line_ran(&record, &lines);
    record.write("bypass");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn production_console_rejects_insert_on_every_engine() {
    if !docker_enabled() {
        return;
    }
    let _quiet = QUIET.read().await;
    let record = Record::new();
    let lines = lines();
    for line in &lines {
        let id = &line.id;
        let t = unique("hv_t31_prod");
        let mut a = admin(line).await;
        a.batch(&format!("CREATE TABLE {t} (a int)")).await;
        let count = format!("SELECT count(*) FROM {t}");
        let insert = format!("INSERT INTO {t} VALUES (1)");

        // The control: the same INSERT writes on a development console.
        let dev = Console::dev(line).await;
        let ux = console_ux(Some(EnvTag::Dev), true, &dev.snap(), None, None);
        assert!(!ux.read_only && !ux.forced_read_only, "{id}: {ux:?}");
        let ran = dev.ok(&insert).await;
        assert_eq!(rows_of(ran.unit(0)), Some(RowsOutcome::Affected(1)), "{id}");
        assert_eq!(a.count(&count).await, 1, "{id}: the control wrote");
        dev.shutdown().await;

        // A production console under the app's policy: read-only, and every write refused.
        let prod = Console::open(
            line,
            Opts {
                env: Some(EnvTag::Production),
                production_policy: true,
                ..Opts::default()
            },
        )
        .await;
        let ux = console_ux(Some(EnvTag::Production), true, &prod.snap(), None, None);
        assert!(
            ux.production && ux.read_only && ux.forced_read_only,
            "{id}: {ux:?}"
        );
        let mut outcomes = Vec::new();
        for sql in [
            insert.clone(),
            format!("DELETE FROM {t}"),
            format!("UPDATE {t} SET a = 2"),
        ] {
            let ran = prod.run(&sql).await;
            assert_eq!(ran.names(), ["Failed"], "{id} `{sql}`");
            assert_eq!(a.count(&count).await, 1, "{id} `{sql}`: nothing changed");
            assert_eq!(
                a.count(&format!("SELECT count(*) FROM {t} WHERE a = 1"))
                    .await,
                1,
                "{id} `{sql}`"
            );
            outcomes.push(ran.outcome());
        }
        // ## m3a-f5-demo (DEMO-7): refused by the console, nothing sent (`input`), on
        // every engine; not merely rejected by the server.
        assert!(outcomes.iter().all(|o| o == INPUT), "{id}: {outcomes:?}");
        // Reads still run on production.
        let ran = prod.ok(&count).await;
        assert_eq!(int(&cell(ran.tab(0), 0, 0)), 1, "{id}");
        record.add(
            line,
            prod.version.as_ref(),
            "production INSERT/DELETE/UPDATE",
            &outcomes.join(","),
        );
        prod.shutdown().await;
        a.batch(&format!("DROP TABLE {t}")).await;
        a.close().await;
    }
    assert_every_line_ran(&record, &lines);
    record.write("production");
}

// ---------------------------------------------------------------------------------------------
// Turkish

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn turkish_collation_text_round_trips_on_pg17_tr() {
    if !docker_enabled() {
        return;
    }
    let _quiet = QUIET.read().await;
    let Some(target) = hacivat_testkit::require_target("pg17-tr") else {
        assert!(!must_exist(), "pg17-tr is missing from the full set");
        return;
    };
    let line = line_of(target);
    let record = Record::new();
    let t = unique("hv_t31_tr");
    let coll = format!("{t}_coll");
    let mut a = admin(&line).await;
    a.batch(&format!(
        "CREATE COLLATION {coll} (provider = libc, locale = 'tr_TR.UTF-8')"
    ))
    .await;
    a.batch(&format!("CREATE TABLE {t} (ad text COLLATE {coll})"))
        .await;
    let c = Console::dev(&line).await;
    check_version(&c);

    // The console sends Turkish text byte for byte: the side channel reads the same bytes.
    let words = [
        "ılık",
        "iğne",
        "İstanbul",
        "Irmak",
        "çay",
        "cam",
        "şeker",
        "sabun",
        "öğle",
        "ok",
        "üzüm",
        "uzun",
        "ğ",
        "g",
    ];
    let values: Vec<String> = words.iter().map(|w| format!("('{w}')")).collect();
    let ran = c
        .ok(&format!("INSERT INTO {t} VALUES {}", values.join(", ")))
        .await;
    assert_eq!(rows_of(ran.unit(0)), Some(RowsOutcome::Affected(14)));
    // The Turkish alphabet: c ç, g ğ, ı i (I with ı, İ with i), o ö, s ş, u ü.
    let order = [
        "cam",
        "çay",
        "g",
        "ğ",
        "ılık",
        "Irmak",
        "iğne",
        "İstanbul",
        "ok",
        "öğle",
        "sabun",
        "şeker",
        "uzun",
        "üzüm",
    ];
    assert_eq!(
        a.scalar(&format!("SELECT string_agg(ad, ',' ORDER BY ad) FROM {t}"))
            .await,
        order.join(","),
        "the server's order under tr_TR"
    );

    // And reads it back through the typed path, in the collation's order, with the server's
    // Turkish case mapping.
    let ran = c
        .ok(&format!(
            "SELECT ad, upper(ad) AS buyuk, lower(ad) AS kucuk FROM {t} ORDER BY ad"
        ))
        .await;
    let tab = ran.tab(0);
    let got: Vec<Value> = (0..14).map(|r| cell(tab, r, 0)).collect();
    let want: Vec<Value> = order.iter().map(|w| text(w)).collect();
    assert_eq!(got, want);
    let row = |w: &str| u64::try_from(order.iter().position(|o| *o == w).unwrap()).unwrap();
    for (word, upper, lower) in [
        ("ılık", "ILIK", "ılık"),
        ("iğne", "İĞNE", "iğne"),
        ("Irmak", "IRMAK", "ırmak"),
        ("İstanbul", "İSTANBUL", "istanbul"),
        ("şeker", "ŞEKER", "şeker"),
    ] {
        assert_eq!(cell(tab, row(word), 1), text(upper), "upper({word})");
        assert_eq!(cell(tab, row(word), 2), text(lower), "lower({word})");
    }

    // The server's own Turkish message (lc_messages) reaches the error view verbatim.
    let missing = format!("{t}_yok");
    let ran = c.run(&format!("SELECT * FROM {missing}")).await;
    let UnitStatus::Failed(view) = ran.unit(0) else {
        panic!("{:?}", ran.names());
    };
    assert_eq!(view.code.as_deref(), Some("42P01"));
    let server_text = view.server_text.as_deref().unwrap_or_default();
    // Verbatim (review M4): exactly what the server says to the side channel for the same
    // statement, and Turkish.
    let direct = a.error_of(&format!("SELECT * FROM {missing}")).await;
    assert_eq!(direct.sqlstate.as_deref(), Some("42P01"));
    assert_eq!(
        server_text, direct.text,
        "the error view is the server's text"
    );
    assert!(server_text.contains(&missing), "{server_text}");
    assert!(
        !server_text.contains("does not exist") && !server_text.is_ascii(),
        "the server's message is Turkish: {server_text}"
    );
    record.add(
        &line,
        c.version.as_ref(),
        "Turkish text, collation and message",
        server_text,
    );
    c.shutdown().await;
    a.batch(&format!("DROP TABLE {t}")).await;
    a.batch(&format!("DROP COLLATION {coll}")).await;
    a.close().await;
    record.write("turkish");
}

// ---------------------------------------------------------------------------------------------
// PostgreSQL: no transaction outlives a cursor

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn incomplete_cursor_leaves_no_transaction_in_pg_stat_activity() {
    if !docker_enabled() {
        return;
    }
    let _quiet = QUIET.read().await;
    let record = Record::new();
    let lines = pg_lines();
    for line in &lines {
        let id = &line.id;
        let mut a = admin(line).await;
        // A read head whose function writes a row per row read (review I1): a cursor that
        // does not complete must leave none of them (R-M3-23), one that completes all of them.
        let t = unique("hv_t31_w");
        pg_writer(&mut a, &t).await;
        let big = writing(&t, 1_000_000);
        let written = format!("SELECT count(*) FROM {t}");
        let idle_cap = FetchPolicy {
            idle_cap: Some(secs(2)),
            ..FetchPolicy::default()
        };
        let c = Console::dev(line).await;

        // Each way a cursor ends without completing; before it, the server shows Hacivat's
        // transaction idle with the portal (so the check after it is not vacuous).
        for path in ["close cursor", "another statement", "idle bound", "stop"] {
            c.log.clear();
            let fetch = if path == "idle bound" {
                idle_cap
            } else {
                FetchPolicy::default()
            };
            let ran = c.run_with(&big, fetch).await;
            assert_eq!(ran.names(), ["Suspended"], "{id} {path}");
            assert_eq!(
                server_view(&mut a, &c, None).await.idle_in_tx,
                1,
                "{id} {path}: the cursor's transaction"
            );
            let src = ran.tab(0).source.clone();
            match path {
                "close cursor" => src.close_cursor(),
                "another statement" => {
                    c.ok("SELECT 1").await;
                }
                "idle bound" => {}
                _ => {
                    src.fetch_all();
                    wait_until(&format!("{id}: rows flow"), secs(30), || {
                        src.snapshot().row_count() >= 50_000
                    })
                    .await;
                    let mut req = None;
                    wait_until(&format!("{id}: the fetch"), secs(10), || {
                        req = c.busy_request();
                        req.is_some()
                    })
                    .await;
                    c.s.cancel(req.unwrap()).await.expect("the cancel answers");
                }
            }
            // `assert_freed` waits for the session to be idle without a cursor, then asks
            // the server: no transaction, nothing running.
            assert_freed(&c, &mut a, None, path).await;
            // The server's own account: what the read head wrote was rolled back.
            assert_eq!(
                a.count(&written).await,
                0,
                "{id} {path}: Hacivat's transaction was rolled back on the server"
            );
            let ids = c.log_ids();
            let rolled_back = ids.iter().any(|i| i.ends_with("-rolled-back"));
            assert!(
                rolled_back || path == "stop",
                "{id} {path}: the log says it was rolled back: {ids:?}"
            );
            record.add(
                line,
                c.version.as_ref(),
                path,
                &format!("0 rows written; {ids:?}"),
            );
        }

        // The same read head run to its end commits, like an autocommit statement.
        let ran = c.run(&writing(&t, 12_000)).await;
        let src = ran.tab(0).source.clone();
        src.fetch_all();
        wait_until(&format!("{id}: the run completes"), secs(30), || {
            *src.snapshot().completeness() == Completeness::Complete
        })
        .await;
        assert_freed(&c, &mut a, None, "completed").await;
        assert_eq!(a.count(&written).await, 12_000, "{id}: committed");
        record.add(line, c.version.as_ref(), "completed", "12000 rows written");
        a.batch(&format!("TRUNCATE {t}")).await;

        // Closing the console with a cursor open: the connection goes, and its transaction
        // with it.
        let ran = c.run(&big).await;
        assert_eq!(ran.names(), ["Suspended"], "{id} close");
        assert_eq!(server_view(&mut a, &c, None).await.idle_in_tx, 1, "{id}");
        let app = c.profile.application_name.clone();
        c.s.close(ClosePolicy::RollBack)
            .await
            .expect("the session closes");
        c.shutdown().await;
        let sql = format!("SELECT count(*) FROM pg_stat_activity WHERE application_name = '{app}'");
        wait_count(
            &mut a,
            &format!("{id}: the backend goes"),
            secs(30),
            &sql,
            0,
        )
        .await;
        assert_eq!(
            a.count(&written).await,
            0,
            "{id} console closed: rolled back"
        );
        record.add(
            line,
            None,
            "console closed",
            "no backend left; 0 rows written",
        );

        // A read-only console: the guard's transaction ends with its cursor, and the server's
        // own bound is the guard's startup option (Task 22).
        let ro = Console::open(
            line,
            Opts {
                read_only: true,
                ..Opts::dev()
            },
        )
        .await;
        let shown = ro.ok("SHOW idle_in_transaction_session_timeout").await;
        assert_eq!(cell(shown.tab(0), 0, 0), text("30s"), "{id}");
        let ran = ro.run(&counting(line, 1_000_000)).await;
        assert_eq!(ran.names(), ["Suspended"], "{id} read-only");
        assert_eq!(server_view(&mut a, &ro, None).await.idle_in_tx, 1, "{id}");
        ran.tab(0).source.close_cursor();
        assert_freed(&ro, &mut a, None, "read-only close").await;
        record.add(
            line,
            ro.version.as_ref(),
            "read-only close cursor",
            "idle_in_transaction_session_timeout 30s; no transaction",
        );
        ro.shutdown().await;
        pg_writer_drop(&mut a, &t).await;
        a.close().await;
    }
    assert_every_line_ran(&record, &lines);
    record.write("pg-no-tx");
}

// ---------------------------------------------------------------------------------------------
// E1: DML and DDL

/// Keeps every `UpdateCount` of a run (the command tag the run service does not log).
struct Tags {
    budget: Arc<MemoryBudget>,
    counts: Mutex<Vec<(Option<u64>, Option<String>)>>,
    stores: Mutex<Vec<Arc<ResultStore>>>,
}

impl RunObserver for Tags {
    fn open_result(&self, _: UnitId, _: u32, schema: Arc<ResultSchema>) -> ResultWriter {
        let (store, writer) = ResultStore::new(ResultId(u64::MAX), schema, self.budget.clone(), 0);
        self.stores.lock().unwrap().push(store);
        writer
    }

    fn event(&self, ev: &RunEvent) {
        if let RunEvent::UpdateCount {
            rows, command_tag, ..
        } = ev
        {
            self.counts
                .lock()
                .unwrap()
                .push((*rows, command_tag.clone()));
        }
    }
}

fn dml_script(t: &str) -> String {
    format!(
        "CREATE TABLE {t} (a int);\n\
         INSERT INTO {t} VALUES (1), (2), (3);\n\
         UPDATE {t} SET a = a + 10 WHERE a >= 2;\n\
         DELETE FROM {t} WHERE a = 1;\n"
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn dml_and_ddl_report_update_counts_on_a_dev_console() {
    if !docker_enabled() {
        return;
    }
    let _quiet = QUIET.read().await;
    let record = Record::new();
    let lines = lines();
    for line in &lines {
        let id = &line.id;
        let mut a = admin(line).await;
        let c = Console::dev(line).await;
        let t = unique("hv_t31_dml");

        // The whole script through the run service: four units, each succeeded with its count.
        let ran = c.run(&dml_script(&t)).await;
        assert_eq!(
            ran.names(),
            ["Succeeded", "Succeeded", "Succeeded", "Succeeded"],
            "{id}: {}",
            ran.outcome()
        );
        // DDL: PostgreSQL's command tag has no count ("Completed"); MySQL's OK packet and
        // SQLite's changes counter say 0 rows, which the log shows as the mysql and sqlite3
        // clients do ("0 rows affected").
        let ddl = if line.is_pg() {
            None
        } else {
            Some(RowsOutcome::Affected(0))
        };
        let counts: Vec<Option<RowsOutcome>> = (0..4).map(|i| rows_of(ran.unit(i))).collect();
        assert_eq!(
            counts,
            [
                ddl,
                Some(RowsOutcome::Affected(3)),
                Some(RowsOutcome::Affected(2)),
                Some(RowsOutcome::Affected(1)),
            ],
            "{id}"
        );
        assert!(ran.tabs.is_empty(), "{id}: no columns, no result tab");
        let lines_logged: Vec<String> = c
            .log
            .lines()
            .iter()
            .map(|l| match &l.msg {
                LogMsg::RowsAffected { rows, .. } => format!("affected {rows}"),
                LogMsg::Completed { .. } => "completed".into(),
                other => other.fluent_id().into(),
            })
            .collect();
        let created = if line.is_pg() {
            "completed"
        } else {
            "affected 0"
        };
        assert_eq!(
            lines_logged,
            [
                created,
                "affected 3",
                "affected 2",
                "affected 1",
                "exec-summary"
            ],
            "{id}"
        );
        // The server agrees.
        let check = if line.is_mysql_family() {
            format!("SELECT CONCAT(COUNT(*), ':', SUM(a)) FROM {t}")
        } else {
            format!("SELECT count(*) || ':' || sum(a) FROM {t}")
        };
        assert_eq!(a.scalar(&check).await, "2:25", "{id}");
        let dropped = c.run(&format!("DROP TABLE {t}")).await;
        assert_eq!(dropped.names(), ["Succeeded"], "{id}");
        assert_eq!(rows_of(dropped.unit(0)), ddl, "{id}: DROP");
        assert_eq!(
            a.count(&match line.engine {
                DbmsKind::Postgres =>
                    format!("SELECT count(*) FROM pg_class WHERE relname = '{t}'"),
                DbmsKind::Sqlite =>
                    format!("SELECT count(*) FROM sqlite_master WHERE name = '{t}'"),
                _ => format!(
                    "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = '{t}'"
                ),
            })
            .await,
            0,
            "{id}: the table is gone"
        );

        // The command tags, from the same units on the same session.
        let t2 = unique("hv_t31_tag");
        let mut text = dml_script(&t2);
        text.push_str(&format!("DROP TABLE {t2}\n"));
        let units = c.units(&text, hacivat_exec::RunAction::Execute3);
        let tags = Arc::new(Tags {
            budget: MemoryBudget::new(1 << 30, 256 << 20),
            counts: Mutex::new(Vec::new()),
            stores: Mutex::new(Vec::new()),
        });
        let run = RunId(u64::MAX);
        let spec = RunSpec {
            run,
            units: units
                .iter()
                .enumerate()
                .map(|(i, u)| RunUnit {
                    id: UnitId {
                        run,
                        ordinal: u32::try_from(i).unwrap(),
                    },
                    sql: u.text.clone(),
                    protocol: Protocol::Extended,
                    suspendable: u.suspendable,
                })
                .collect(),
            policy: ErrorPolicy::StopOnError,
            fetch: FetchPolicy::default(),
            observer: ObserverRef(tags.clone()),
        };
        c.s.run(spec)
            .reply
            .await
            .expect("the actor replies")
            .unwrap_or_else(|e| panic!("{id}: {:?}", e.kind));
        let got = tags.counts.lock().unwrap().clone();
        let want: Vec<(Option<u64>, Option<String>)> = match line.engine {
            DbmsKind::Postgres => vec![
                (None, Some("CREATE TABLE".into())),
                (Some(3), Some("INSERT 0 3".into())),
                (Some(2), Some("UPDATE 2".into())),
                (Some(1), Some("DELETE 1".into())),
                (None, Some("DROP TABLE".into())),
            ],
            _ => vec![
                (Some(0), Some("CREATE".into())),
                (Some(3), Some("INSERT".into())),
                (Some(2), Some("UPDATE".into())),
                (Some(1), Some("DELETE".into())),
                (Some(0), Some("DROP".into())),
            ],
        };
        assert_eq!(got, want, "{id}");
        record.add(
            line,
            c.version.as_ref(),
            "CREATE/INSERT/UPDATE/DELETE/DROP",
            &format!("{got:?}"),
        );
        c.shutdown().await;
        a.close().await;
    }
    assert_every_line_ran(&record, &lines);
    record.write("dml");
}

// ## m3a-f5-demo (DEMO-9): the PO's demo step 9 through the run service on every MySQL-family
// line: `CALL` of a procedure answering three result sets keeps the first (`n = 1`) in one
// result tab and says two more were read and not shown. The demo script as the PO ran it (the
// fixture's `CREATE PROCEDURE` and the `CALL`, as a whole script, with the procedure already
// there) stops at the failed `CREATE` (R-M3-26), so its `CALL` is skipped and shows nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn call_with_three_result_sets_shows_the_first_through_run() {
    if !docker_enabled() {
        return;
    }
    let _quiet = QUIET.read().await;
    let record = Record::new();
    let lines: Vec<Line> = server_lines()
        .into_iter()
        .filter(Line::is_mysql_family)
        .collect();
    let create = "CREATE PROCEDURE hv_test.hv_t14_three() \
                  BEGIN SELECT 1 AS n; SELECT 2 AS n; SELECT 3 AS n; END";
    for line in &lines {
        let id = &line.id;
        let mut a = admin(line).await;
        a.batch(&create.replace("CREATE PROCEDURE", "CREATE PROCEDURE IF NOT EXISTS"))
            .await;
        let c = Console::dev(line).await;
        let ran = c.ok("CALL hv_test.hv_t14_three();").await;
        assert_eq!(ran.tabs.len(), 1, "{id}: one result tab");
        assert_eq!(int(&cell(ran.tab(0), 0, 0)), 1, "{id}: the first set");
        assert!(
            c.log_ids().contains(&"exec-more-result-sets"),
            "{id}: {:?}",
            c.log_ids()
        );
        let script = c
            .run(&format!("{create};\nCALL hv_test.hv_t14_three();"))
            .await;
        assert_eq!(script.names(), ["Failed", "Skipped"], "{id}");
        record.add(
            line,
            c.version.as_ref(),
            "CALL with three result sets",
            &format!("{} tab, script {:?}", ran.tabs.len(), script.names()),
        );
        c.shutdown().await;
        a.close().await;
    }
    assert!(!lines.is_empty(), "no MySQL-family line");
    record.write("call-three-sets");
}
