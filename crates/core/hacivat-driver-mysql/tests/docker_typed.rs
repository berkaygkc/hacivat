//! Typed streaming against the Docker matrix (Task 14): the type golden table in both
//! protocols, suspend and resume over 100k rows, the drain-or-kill close and an external
//! `KILL QUERY`, `CALL` with three result sets, the 1295 routes. `mysql84` on the `pr` set;
//! `mysql80` and `mariadb106/1011/114` too on `full`. Every test is `#[ignore = "docker"]` and
//! takes its targets from `require_target(..)` or `targets(Some(engine))` (Q6). Run with
//! `<main>/target/docker/run-docker-tests.sh --set pr|full -p hacivat-driver-mysql`.
//!
//! Fixtures are session-scoped (temporary tables, prepared statements) or created idempotently
//! (`hv_t14_three`, Q7). Failures print target ids and error kinds only, never a password.

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_driver::{
    CloseHow, Closed, DriverConnect, Endpoint, ExecOpts, InsecureAuth, Protocol, QueryDone,
    QueryEvent, QuerySpec, RawConn, RawError, RawErrorKind, ResultEnd, TypedCaps,
};
use hacivat_driver_mysql::{MySqlConn, connect_conn};
use hacivat_net::{
    AddressClass, Target, TlsMode, TlsParams, TlsSettings, build_tls_params, direct,
    install_crypto_provider,
};
use hacivat_proto::DbmsKind;
use hacivat_testkit::{DockerTarget, TlsDefault, targets};
use hacivat_values::{
    Chunk, DateVal, DecimalVal, LogicalType, OpaqueVal, OriginHint, ResultSchema, SpecialVal,
    TimeVal, TsVal, Value,
};
use secrecy::SecretString;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// Every MySQL and MariaDB target of the active set.
fn mysql_family() -> Vec<DockerTarget> {
    let mut ts = targets(Some(DbmsKind::MySql));
    ts.extend(targets(Some(DbmsKind::MariaDb)));
    assert!(!ts.is_empty(), "no MySQL-family target in the selected set");
    ts
}

fn params(t: &DockerTarget) -> Option<TlsParams> {
    install_crypto_provider();
    let mode = match t.tls_default {
        TlsDefault::SelfSigned => TlsMode::Require,
        TlsDefault::None => TlsMode::Auto,
    };
    let settings = TlsSettings {
        mode,
        ..TlsSettings::default()
    };
    build_tls_params(&settings, &t.host, t.port, AddressClass::Loopback, None)
        .unwrap_or_else(|e| panic!("{}: {e:?}", t.id))
}

fn request(t: &DockerTarget, read_only: bool) -> DriverConnect {
    let addr: SocketAddr = format!("{}:{}", t.host, t.port).parse().unwrap();
    let target = Target::Tcp {
        host: t.host.clone(),
        port: t.port,
    };
    DriverConnect {
        endpoint: Endpoint::Network {
            factory: direct(target.clone(), vec![addr], secs(5)),
            direct: Some(target),
            resolved: vec![addr],
            tls_host: t.host.clone(),
        },
        tls: params(t),
        user: Some(t.user.clone()),
        password: Some(SecretString::from(t.password.clone())),
        database: Some(t.database.clone()),
        application_name: "Hacivat tests".into(),
        read_only,
        options: vec![],
        connect_timeout: secs(20),
        events: Arc::new(|_| {}),
        insecure_auth: InsecureAuth::Allow,
    }
}

async fn connect(t: &DockerTarget, read_only: bool) -> MySqlConn {
    match connect_conn(request(t, read_only)).await {
        Ok(c) => c,
        Err(e) => panic!("{}: {:?}", t.id, e.kind),
    }
}

fn vendor(e: &RawError) -> Option<i64> {
    e.server.as_ref().and_then(|s| s.vendor_code)
}

async fn batch(t: &DockerTarget, c: &mut MySqlConn, sql: &str) {
    if let Err(e) = c.batch(sql).await {
        panic!("{}: {sql}: {:?} {:?}", t.id, e.kind, vendor(&e));
    }
}

fn spec(sql: &str, suspendable: bool) -> QuerySpec<'_> {
    QuerySpec {
        sql,
        binds: &[],
        protocol: Protocol::Extended,
        suspendable,
        first_rows: 501,
        chunk_rows: 4_096,
        chunk_bytes: 4 << 20,
        cell_cap: 1 << 20,
        max_result_sets: 1,
    }
}

async fn event(t: &DockerTarget, c: &mut MySqlConn) -> QueryEvent {
    match c.next_event().await {
        Ok(e) => e,
        Err(e) => panic!("{}: next_event: {:?} {:?}", t.id, e.kind, vendor(&e)),
    }
}

/// One whole typed query that does not suspend: its schema, chunks, end and done.
struct Ran {
    schema: Arc<ResultSchema>,
    chunks: Vec<Chunk>,
    end: ResultEnd,
    done: QueryDone,
}

impl Ran {
    /// Every cell, row by row, as the store holds it.
    fn rows(&self) -> Vec<Vec<Value>> {
        let width = self.schema.columns.len() as u32;
        self.chunks
            .iter()
            .flat_map(|c| {
                (0..c.rows())
                    .map(move |r| (0..width).map(|col| c.cell(r, col).to_value()).collect())
            })
            .collect()
    }
}

async fn run(t: &DockerTarget, c: &mut MySqlConn, s: QuerySpec<'_>) -> Ran {
    if let Err(e) = c.query(s).await {
        panic!("{}: query {:?}: {:?} {:?}", t.id, s.sql, e.kind, vendor(&e));
    }
    let schema = match event(t, c).await {
        QueryEvent::ResultStart(s) => s,
        other => panic!("{}: {other:?}", t.id),
    };
    let mut chunks = Vec::new();
    let end = loop {
        match event(t, c).await {
            QueryEvent::Rows(chunk) => chunks.push(chunk),
            QueryEvent::ResultEnd(end) => break end,
            other => panic!("{}: {other:?}", t.id),
        }
    };
    let done = match event(t, c).await {
        QueryEvent::Done(d) => d,
        other => panic!("{}: {other:?}", t.id),
    };
    Ran {
        schema,
        chunks,
        end,
        done,
    }
}

async fn select_one(t: &DockerTarget, c: &mut MySqlConn) {
    match c.select_i64("SELECT 1").await {
        Ok(Some(1)) => {}
        other => panic!("{}: {:?}", t.id, other.map_err(|e| e.kind)),
    }
}

// The golden table -------------------------------------------------------------------------

const GOLDEN_TABLE: &str = "CREATE TEMPORARY TABLE hv_t14_types (\
    id INT PRIMARY KEY, \
    ti TINYINT, tu TINYINT UNSIGNED, b1 TINYINT(1), si SMALLINT, mi MEDIUMINT, i INT, \
    iu INT UNSIGNED, bi BIGINT, bu BIGINT UNSIGNED, y YEAR, \
    d10 DECIMAL(10,2), d65 DECIMAL(65,30), du DECIMAL(10,2) UNSIGNED, \
    f FLOAT, db DOUBLE, \
    c CHAR(5), vc VARCHAR(20), tx TEXT, en ENUM('small','medium'), st SET('a','b','c'), \
    bn BINARY(3), vb VARBINARY(8), bl BLOB, js JSON, \
    dt DATE, dtm DATETIME(6), ts TIMESTAMP(3) NULL, tm TIME(2), \
    bt BIT(5), g GEOMETRY) DEFAULT CHARSET = utf8mb4";

const GOLDEN_ROWS: &str = "INSERT INTO hv_t14_types VALUES \
    (1, -128, 255, 1, -32768, 8388607, -2147483648, 4294967295, -9223372036854775808, \
     18446744073709551615, 2024, \
     12345678.90, -99999999999999999999999999999999999.111111111111111111111111111111, 0.00, \
     0.1, 1.7976931348623157e308, \
     'ab', 'İstanbul ğüşıöç', 'metin', 'medium', 'a,c', \
     X'00FF10', X'', X'DEADBEEF', '{\"a\": [1, 2]}', \
     '2024-02-29', '2024-02-29 13:14:15.123456', '2024-02-29 13:14:15.123', '13:14:15.25', \
     b'10110', ST_GeomFromText('POINT(1 1)')), \
    (2, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, \
     NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, \
     NULL), \
    (3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, -0.50, 0, 0, -1.5, -2.5e-10, '', '', '', 'small', '', \
     X'000000', X'01', X'', 'null', \
     '0000-00-00', '0000-00-00 00:00:00', '0000-00-00 00:00:00', '838:59:59', \
     b'00000', NULL), \
    (4, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1999, 1, 1, 1, 1, 1, 'x', 'x', 'x', 'small', 'b', \
     X'010203', X'02', X'03', '[]', \
     '2024-00-15', '2024-05-00 10:00:00', '1970-01-01 00:00:01', '-01:30:00.50', \
     b'11111', NULL), \
    (5, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1999, 1, 1, 1, 1, 1, 'x', 'x', 'x', 'small', 'b', \
     X'010203', X'02', X'03', '[]', \
     '2024-02-30', '2024-02-30 01:02:03', NULL, '24:00:00', b'1', NULL)";

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

fn date(y: i32, m: u8, d: u8) -> Value {
    Value::Date(DateVal::from_civil(y, m, d).unwrap())
}

fn ts(secs: i64, nanos: u32) -> Value {
    Value::Timestamp(TsVal { secs, nanos })
}

fn partial(s: &str) -> Value {
    Value::Special(SpecialVal::PartialZeroDate(s.into()))
}

fn interval(micros: i64) -> Value {
    Value::Interval {
        months: 0,
        days: 0,
        micros,
    }
}

const H: i64 = 3_600_000_000;
/// 2024-02-29 00:00:00 UTC in seconds.
const LEAP_DAY: i64 = 1_709_164_800;

/// The golden rows: what the store holds for [`GOLDEN_ROWS`] on `engine`.
fn golden(engine: DbmsKind) -> Vec<Vec<Value>> {
    let json = |s: &str| match engine {
        // MariaDB's JSON is LONGTEXT (with a check constraint): text on the wire.
        DbmsKind::MariaDb => text(s),
        _ => Value::Json(s.into()),
    };
    let point = "000000000101000000000000000000F03F000000000000F03F";
    let ones = |id: i64| {
        let mut row = vec![Value::Int(id)];
        row.extend((0..8).map(|_| Value::Int(1)));
        row.push(Value::UInt(1));
        row.push(Value::Int(1999));
        row.extend([
            dec("1.00"),
            dec(&format!("1.{}", "0".repeat(30))),
            dec("1.00"),
        ]);
        row.extend([Value::F32(1.0), Value::F64(1.0)]);
        row.extend([text("x"), text("x"), text("x"), text("small"), text("b")]);
        row.extend([bytes(&[1, 2, 3]), bytes(&[2]), bytes(&[3]), json("[]")]);
        row
    };
    let mut row4 = ones(4);
    row4.extend([
        partial("2024-00-15"),
        partial("2024-05-00 10:00:00.000000"),
        ts(1, 0),
        interval(-(H + 30 * 60_000_000 + 500_000)),
        opaque("bit", "11111"),
        Value::Null,
    ]);
    let mut row5 = ones(5);
    row5.extend([
        opaque("date", "2024-02-30"),
        opaque("datetime", "2024-02-30 01:02:03.000000"),
        Value::Null,
        Value::Time(TimeVal::END_OF_DAY),
        opaque("bit", "00001"),
        Value::Null,
    ]);
    vec![
        vec![
            Value::Int(1),
            Value::Int(-128),
            Value::Int(255),
            Value::Int(1),
            Value::Int(-32768),
            Value::Int(8_388_607),
            Value::Int(-2_147_483_648),
            Value::Int(4_294_967_295),
            Value::Int(i64::MIN),
            Value::UInt(u64::MAX),
            Value::Int(2024),
            dec("12345678.90"),
            dec(&format!("-{}.{}", "9".repeat(35), "1".repeat(30))),
            dec("0.00"),
            Value::F32(0.1),
            Value::F64(f64::MAX),
            text("ab"),
            text("İstanbul ğüşıöç"),
            text("metin"),
            text("medium"),
            text("a,c"),
            bytes(&[0, 0xFF, 0x10]),
            bytes(&[]),
            bytes(&[0xDE, 0xAD, 0xBE, 0xEF]),
            json(r#"{"a": [1, 2]}"#),
            date(2024, 2, 29),
            ts(LEAP_DAY + 13 * 3_600 + 14 * 60 + 15, 123_456_000),
            ts(LEAP_DAY + 13 * 3_600 + 14 * 60 + 15, 123_000_000),
            Value::Time(TimeVal::from_micros(13 * H + 14 * 60_000_000 + 15_250_000)),
            opaque("bit", "10110"),
            opaque("geometry", point),
        ],
        std::iter::once(Value::Int(2))
            .chain((0..30).map(|_| Value::Null))
            .collect(),
        vec![
            Value::Int(3),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::UInt(0),
            Value::Int(0),
            dec("-0.50"),
            dec(&format!("0.{}", "0".repeat(30))),
            dec("0.00"),
            Value::F32(-1.5),
            Value::F64(-2.5e-10),
            text(""),
            text(""),
            text(""),
            text("small"),
            text(""),
            bytes(&[0, 0, 0]),
            bytes(&[1]),
            bytes(&[]),
            json("null"),
            Value::Special(SpecialVal::ZeroDate),
            Value::Special(SpecialVal::ZeroDate),
            Value::Special(SpecialVal::ZeroDate),
            interval(838 * H + 59 * 60_000_000 + 59_000_000),
            opaque("bit", "00000"),
            Value::Null,
        ],
        row4,
        row5,
    ]
}

/// Values compare by bits for floats, so `-0.0` and `0.0` differ as stored.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::F64(x), Value::F64(y)) => x.to_bits() == y.to_bits(),
        (Value::F32(x), Value::F32(y)) => x.to_bits() == y.to_bits(),
        _ => a == b,
    }
}

fn check_rows(t: &DockerTarget, protocol: &str, schema: &ResultSchema, got: &[Vec<Value>]) {
    let want = golden(t.engine);
    assert_eq!(got.len(), want.len(), "{} {protocol}: row count", t.id);
    for (r, (g, w)) in got.iter().zip(&want).enumerate() {
        assert_eq!(g.len(), w.len(), "{} {protocol}: row {r} width", t.id);
        for (c, (gv, wv)) in g.iter().zip(w).enumerate() {
            assert!(
                same(gv, wv),
                "{} {protocol}: row {r} column {}: got {gv:?}, want {wv:?}",
                t.id,
                schema.columns[c].name
            );
        }
    }
}

fn logical(schema: &ResultSchema, name: &str) -> LogicalType {
    schema
        .columns
        .iter()
        .find(|c| &*c.name == name)
        .unwrap_or_else(|| panic!("no column {name}"))
        .logical
        .clone()
}

fn check_schema(t: &DockerTarget, schema: &ResultSchema) {
    let dec = |p, s| LogicalType::Decimal {
        precision: Some(p),
        scale: Some(s),
    };
    let int = |bits, signed| LogicalType::Int { bits, signed };
    for (name, want) in [
        ("b1", int(8, true)),
        ("tu", int(8, false)),
        ("mi", int(24, true)),
        ("bu", int(64, false)),
        ("y", int(16, false)),
        ("d10", dec(10, 2)),
        ("d65", dec(65, 30)),
        ("du", dec(10, 2)),
        ("f", LogicalType::Float { bits: 32 }),
        (
            "c",
            LogicalType::Text {
                max_chars: None,
                fixed: true,
            },
        ),
        (
            "en",
            LogicalType::Text {
                max_chars: None,
                fixed: false,
            },
        ),
        ("bn", LogicalType::Bytes { max: Some(3) }),
        ("vb", LogicalType::Bytes { max: Some(8) }),
        ("dt", LogicalType::Date),
        (
            "dtm",
            LogicalType::Timestamp {
                precision: 6,
                tz: false,
            },
        ),
        (
            "ts",
            LogicalType::Timestamp {
                precision: 3,
                tz: false,
            },
        ),
        (
            "tm",
            LogicalType::Time {
                precision: 2,
                tz: false,
            },
        ),
        (
            "bt",
            LogicalType::Opaque {
                type_name: "bit".into(),
            },
        ),
        (
            "g",
            LogicalType::Opaque {
                type_name: "geometry".into(),
            },
        ),
    ] {
        assert_eq!(logical(schema, name), want, "{}: column {name}", t.id);
    }
    let json = logical(schema, "js");
    match t.engine {
        DbmsKind::MariaDb => assert!(matches!(json, LogicalType::Text { .. }), "{}", t.id),
        _ => assert_eq!(json, LogicalType::Json { binary: false }, "{}", t.id),
    }
    let i = schema.columns.iter().find(|c| &*c.name == "i").unwrap();
    match &i.origin {
        Some(OriginHint::MySql {
            schema,
            table,
            column,
        }) => {
            assert_eq!(&**schema, "hv_test", "{}", t.id);
            assert_eq!(&**table, "hv_t14_types", "{}", t.id);
            assert_eq!(&**column, "i", "{}", t.id);
        }
        other => panic!("{}: origin {other:?}", t.id),
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn type_golden_table() {
    for t in mysql_family() {
        let mut c = connect(&t, false).await;
        // Zero, partial-zero and out-of-calendar dates are this session's to store.
        batch(&t, &mut c, "SET SESSION sql_mode = 'ALLOW_INVALID_DATES'").await;
        batch(&t, &mut c, "SET SESSION time_zone = '+00:00'").await;
        batch(&t, &mut c, GOLDEN_TABLE).await;
        batch(&t, &mut c, GOLDEN_ROWS).await;
        let sql = "SELECT * FROM hv_t14_types ORDER BY id";
        let binary = run(&t, &mut c, spec(sql, false)).await;
        check_schema(&t, &binary.schema);
        check_rows(&t, "binary", &binary.schema, &binary.rows());
        assert_eq!(binary.end.rows, 5, "{}", t.id);
        assert_eq!(binary.end.command_tag.as_deref(), Some("SELECT"));
        // The same rows over COM_QUERY, decoded from text.
        c.force_text_protocol(true);
        let text = run(&t, &mut c, spec(sql, false)).await;
        c.force_text_protocol(false);
        let shape = |s: &ResultSchema| -> Vec<(String, LogicalType)> {
            s.columns
                .iter()
                .map(|c| (c.name.to_string(), c.logical.clone()))
                .collect()
        };
        assert_eq!(shape(&text.schema), shape(&binary.schema), "{}", t.id);
        check_rows(&t, "text", &text.schema, &text.rows());
        assert_eq!(text.done.skipped_result_sets, 0);
        Box::new(c).close().await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn the_idle_bound_is_15_s_below_net_write_timeout() {
    for t in mysql_family() {
        let mut c = connect(&t, false).await;
        let nwt = c
            .select_i64("SELECT @@net_write_timeout")
            .await
            .unwrap()
            .unwrap();
        let bound = Duration::from_secs(u64::try_from(nwt).unwrap()) - secs(15);
        assert_eq!(
            c.typed_caps(),
            TypedCaps {
                suspend: true,
                binds: false,
                close_needs_kill: true,
                idle_bound: Some(bound),
            },
            "{}",
            t.id
        );
        // The servers run with their default, so 45 s.
        assert_eq!(bound, secs(45), "{}", t.id);
    }
}

// Suspend, resume, close -------------------------------------------------------------------

/// `n` numbered rows 1..=n (n a power of ten up to 10^7) without recursion, in order, with a
/// padding column of `pad` bytes.
fn numbers(digits: u32, pad: usize, ordered: bool) -> String {
    let names = ["a", "b", "c", "e", "f", "g", "h"];
    let used = &names[..digits as usize];
    let sum: Vec<String> = used
        .iter()
        .enumerate()
        .map(|(i, n)| format!("{n}.x * {}", 10_u64.pow(i as u32)))
        .collect();
    let from: Vec<String> = used.iter().map(|n| format!("d {n}")).collect();
    format!(
        "WITH d(x) AS (SELECT 0 UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 \
         UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 \
         UNION ALL SELECT 8 UNION ALL SELECT 9) \
         SELECT {} + 1 AS n, REPEAT('x', {pad}) AS pad FROM {}{}",
        sum.join(" + "),
        from.join(", "),
        if ordered { " ORDER BY n" } else { "" }
    )
}

fn ints(c: &Chunk) -> Vec<i64> {
    (0..c.rows())
        .map(|r| match c.cell(r, 0).to_value() {
            Value::Int(v) => v,
            other => panic!("{other:?}"),
        })
        .collect()
}

#[tokio::test]
#[ignore = "docker"]
async fn hundred_thousand_rows_suspend_and_resume() {
    for t in mysql_family() {
        let mut c = connect(&t, false).await;
        let sql = numbers(5, 8, true);
        c.query(spec(&sql, true))
            .await
            .unwrap_or_else(|e| panic!("{}: {:?}", t.id, e.kind));
        assert!(matches!(
            event(&t, &mut c).await,
            QueryEvent::ResultStart(_)
        ));
        let mut seen: Vec<i64> = Vec::new();
        let first = match event(&t, &mut c).await {
            QueryEvent::Rows(chunk) => chunk,
            other => panic!("{}: {other:?}", t.id),
        };
        assert_eq!(first.rows(), 501, "{}", t.id);
        seen.extend(ints(&first));
        assert_eq!(event(&t, &mut c).await, QueryEvent::Suspended, "{}", t.id);
        // Suspended: nothing else runs on the connection.
        assert_eq!(
            c.execute("SELECT 1", ExecOpts::default())
                .await
                .unwrap_err()
                .kind,
            RawErrorKind::Unsupported
        );
        assert_eq!(
            c.next_event().await.unwrap_err().kind,
            RawErrorKind::Unsupported
        );
        c.fetch_more(10_000).await.unwrap();
        let mut suspensions = 1;
        let end = loop {
            match event(&t, &mut c).await {
                QueryEvent::Rows(chunk) => {
                    assert!(chunk.rows() <= 4_096, "{}", t.id);
                    seen.extend(ints(&chunk));
                }
                QueryEvent::Suspended => {
                    suspensions += 1;
                    // Each batch of 10,000 ends at a read-ahead point.
                    assert_eq!((seen.len() - 501) % 10_000, 0, "{}", t.id);
                    c.fetch_more(10_000).await.unwrap();
                }
                QueryEvent::ResultEnd(end) => break end,
                other => panic!("{}: {other:?}", t.id),
            }
        };
        assert_eq!(end.rows, 100_000, "{}", t.id);
        assert_eq!(end.kept, 100_000, "{}", t.id);
        assert!(suspensions >= 10, "{}: {suspensions}", t.id);
        assert!(
            seen.iter().copied().eq(1..=100_000),
            "{}: rows out of order or missing",
            t.id
        );
        assert!(matches!(event(&t, &mut c).await, QueryEvent::Done(_)));
        select_one(&t, &mut c).await;
    }
}

/// 10^7 rows of 1 KB, unordered: far more than every buffer between the server and the client,
/// so the server is still sending (blocked on its write) when a kill arrives. The width is a
/// balance: with 100-byte rows the client parses many buffered rows before the kill's error
/// (MySQL 8.4 once took 1.03 s in a debug build beside the other tests); with 4 KB rows MariaDB
/// sends megabytes more before it acts on the kill (MariaDB 10.6: 1.5-2.3 s). At 1 KB every
/// target measured 10-263 ms.
fn large_stream() -> String {
    numbers(7, 1_000, false)
}

/// A query far larger than the socket buffers, suspended after its first chunk (the server's
/// thread is then blocked writing).
async fn suspended_large_query(t: &DockerTarget, c: &mut MySqlConn, sql: &str) {
    c.query(spec(sql, true))
        .await
        .unwrap_or_else(|e| panic!("{}: {:?}", t.id, e.kind));
    assert!(matches!(event(t, c).await, QueryEvent::ResultStart(_)));
    match event(t, c).await {
        QueryEvent::Rows(chunk) => assert_eq!(chunk.rows(), 501),
        other => panic!("{}: {other:?}", t.id),
    }
    assert_eq!(event(t, c).await, QueryEvent::Suspended, "{}", t.id);
}

/// The kill-to-usable times of every target: printed, then each must be under 1 s.
fn assert_within_1s(what: &str, times: &[(String, Duration)]) {
    for (id, took) in times {
        eprintln!("{id}: {what}: usable after {took:?}");
    }
    let slow: Vec<_> = times.iter().filter(|(_, t)| *t >= secs(1)).collect();
    assert!(slow.is_empty(), "{what}: {slow:?}");
}

#[tokio::test]
#[ignore = "docker"]
async fn external_kill_then_drain_leaves_the_connection_usable_within_1s() {
    let mut times = Vec::new();
    for t in mysql_family() {
        let mut c = connect(&t, false).await;
        let mut side = connect(&t, false).await;
        suspended_large_query(&t, &mut c, &large_stream()).await;
        let id = c.backend_id().unwrap();
        batch(&t, &mut side, &format!("KILL QUERY {id}")).await;
        let killed = Instant::now();
        // The drain reads the rows already on their way, then the kill's 1317.
        let closed = c
            .close_query(CloseHow::Drain(secs(10)))
            .await
            .unwrap_or_else(|e| panic!("{}: {:?}", t.id, e.kind));
        assert_eq!(closed, Closed::Closed { rows: 501 }, "{}", t.id);
        select_one(&t, &mut c).await;
        times.push((t.id.clone(), killed.elapsed()));
        Box::new(side).close().await;
    }
    assert_within_1s("external kill, then drain", &times);
}

#[tokio::test]
#[ignore = "docker"]
async fn drain_runs_out_then_the_kill_and_after_kill_leave_the_connection_usable() {
    let mut times = Vec::new();
    for t in mysql_family() {
        let mut c = connect(&t, false).await;
        let mut side = connect(&t, false).await;
        suspended_large_query(&t, &mut c, &large_stream()).await;
        // The session's protocol: drain for 250 ms, else kill this thread and read to 1317.
        let thread_id = match c
            .close_query(CloseHow::Drain(Duration::from_millis(250)))
            .await
        {
            Ok(Closed::NeedsServerKill { thread_id }) => thread_id,
            other => panic!("{}: {:?}", t.id, other.map_err(|e| e.kind)),
        };
        assert_eq!(Some(thread_id), c.backend_id(), "{}: its own thread", t.id);
        batch(&t, &mut side, &format!("KILL QUERY {thread_id}")).await;
        let killed = Instant::now();
        let closed = c
            .close_query(CloseHow::AfterKill(secs(10)))
            .await
            .unwrap_or_else(|e| panic!("{}: {:?}", t.id, e.kind));
        assert_eq!(closed, Closed::Closed { rows: 501 }, "{}", t.id);
        select_one(&t, &mut c).await;
        times.push((t.id.clone(), killed.elapsed()));
        // The side connection's own session was never touched.
        select_one(&t, &mut side).await;
        Box::new(side).close().await;
    }
    assert_within_1s("drain, kill, AfterKill", &times);
}

#[tokio::test]
#[ignore = "docker"]
async fn an_unread_stream_past_net_write_timeout_ends_as_a_lost_connection() {
    // R19: why the idle bound sits below `net_write_timeout`. Lowered here for the test only
    // (the driver never sets it, and its caps keep the value read at connect).
    for t in mysql_family() {
        let mut c = connect(&t, false).await;
        let mut side = connect(&t, false).await;
        batch(&t, &mut c, "SET SESSION net_write_timeout = 2").await;
        suspended_large_query(&t, &mut c, &large_stream()).await;
        let id = c.backend_id().unwrap();
        // Condition: the server aborted the blocked write and ended the connection.
        let sql = format!("SELECT COUNT(*) FROM information_schema.processlist WHERE id = {id}");
        let deadline = Instant::now() + secs(60);
        while side.select_i64(&sql).await.unwrap() != Some(0) {
            assert!(
                Instant::now() < deadline,
                "{}: the server kept the thread",
                t.id
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        // The rows already on their way, then the lost connection: an error, never a hang.
        c.fetch_more(u32::MAX).await.unwrap();
        let e = loop {
            match c.next_event().await {
                Ok(QueryEvent::Rows(_)) => {}
                Ok(other) => panic!("{}: {other:?}", t.id),
                Err(e) => break e,
            }
        };
        assert!(
            matches!(e.kind, RawErrorKind::Io(_) | RawErrorKind::Protocol),
            "{}: {:?}",
            t.id,
            e.kind
        );
        assert_eq!(
            c.ping().await.unwrap_err().kind,
            RawErrorKind::Io(std::io::ErrorKind::NotConnected),
            "{}",
            t.id
        );
        Box::new(side).close().await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn a_short_rest_drains_to_completed() {
    for t in mysql_family() {
        let mut c = connect(&t, false).await;
        let sql = numbers(3, 8, true);
        suspended_large_query(&t, &mut c, &sql).await;
        assert_eq!(
            c.close_query(CloseHow::Drain(secs(10))).await.unwrap(),
            Closed::Completed { rows: 1_000 },
            "{}",
            t.id
        );
        select_one(&t, &mut c).await;
    }
}

// Result sets and routes -------------------------------------------------------------------

#[tokio::test]
#[ignore = "docker"]
async fn call_with_three_sets_keeps_the_first_and_counts_two() {
    for t in mysql_family() {
        let mut c = connect(&t, false).await;
        batch(
            &t,
            &mut c,
            "CREATE PROCEDURE IF NOT EXISTS hv_test.hv_t14_three() \
             BEGIN SELECT 1 AS n; SELECT 2 AS n; SELECT 3 AS n; END",
        )
        .await;
        // Suspendable, but a multi-result unit never suspends.
        let ran = run(&t, &mut c, spec("CALL hv_test.hv_t14_three()", true)).await;
        assert_eq!(ran.rows(), vec![vec![Value::Int(1)]], "{}", t.id);
        assert_eq!(ran.end.rows, 1);
        assert_eq!(ran.end.command_tag.as_deref(), Some("CALL"));
        assert_eq!(ran.done.skipped_result_sets, 2, "{}", t.id);
        select_one(&t, &mut c).await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn non_preparable_statement_retries_over_com_query_on_read_write() {
    for t in mysql_family() {
        let mut c = connect(&t, false).await;
        // PREPARE and EXECUTE are not preparable (1295): both run once more as text.
        let prepared = run(
            &t,
            &mut c,
            spec("PREPARE hv_t14_s FROM 'SELECT 1 AS n, ''tr'' AS s'", false),
        )
        .await;
        assert!(prepared.schema.columns.is_empty(), "{}", t.id);
        assert_eq!(prepared.end.command_tag.as_deref(), Some("PREPARE"));
        let executed = run(&t, &mut c, spec("EXECUTE hv_t14_s", true)).await;
        assert_eq!(
            executed.rows(),
            vec![vec![Value::Int(1), Value::Text("tr".into())]],
            "{}",
            t.id
        );
        batch(&t, &mut c, "DEALLOCATE PREPARE hv_t14_s").await;
        select_one(&t, &mut c).await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn read_only_1295_stays_unsupported() {
    for t in mysql_family() {
        let mut c = connect(&t, true).await;
        let e = c
            .query(spec("PREPARE hv_t14_s FROM 'SELECT 1'", false))
            .await
            .unwrap_err();
        assert_eq!(e.kind, RawErrorKind::Unsupported, "{}", t.id);
        // Never run as text: no statement of that name exists.
        let e = c.query(spec("EXECUTE hv_t14_s", false)).await.unwrap_err();
        assert_eq!(e.kind, RawErrorKind::Unsupported, "{}", t.id);
        select_one(&t, &mut c).await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn a_value_past_max_allowed_packet_is_null_with_a_warning() {
    for t in mysql_family() {
        let mut c = connect(&t, false).await;
        // The server refuses to build it (warning 1301) instead of sending a packet the client
        // would refuse; the warning count reaches `Done`.
        for protocol in [false, true] {
            c.force_text_protocol(protocol);
            let ran = run(
                &t,
                &mut c,
                spec("SELECT REPEAT('x', @@max_allowed_packet + 1) AS big", false),
            )
            .await;
            assert_eq!(ran.rows(), vec![vec![Value::Null]], "{}", t.id);
            assert!(ran.done.warnings >= 1, "{}: {:?}", t.id, ran.done);
        }
        c.force_text_protocol(false);
        select_one(&t, &mut c).await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn dml_and_ddl_report_one_empty_result() {
    for t in mysql_family() {
        let mut c = connect(&t, false).await;
        let created = run(
            &t,
            &mut c,
            spec("CREATE TEMPORARY TABLE hv_t14_dml (a INT)", true),
        )
        .await;
        assert!(created.schema.columns.is_empty() && created.chunks.is_empty());
        assert_eq!(created.end.command_tag.as_deref(), Some("CREATE"));
        let inserted = run(
            &t,
            &mut c,
            spec("INSERT INTO hv_t14_dml VALUES (1), (2), (3)", true),
        )
        .await;
        assert_eq!(inserted.end.rows_affected, Some(3), "{}", t.id);
        assert_eq!((inserted.end.rows, inserted.end.kept), (0, 0));
        // A server error: the query ends, the connection stays in step.
        let e = c
            .query(spec("SELECT * FROM hv_t14_missing", false))
            .await
            .unwrap_err();
        assert_eq!((e.kind.clone(), vendor(&e)), (RawErrorKind::Db, Some(1146)));
        select_one(&t, &mut c).await;
    }
}
