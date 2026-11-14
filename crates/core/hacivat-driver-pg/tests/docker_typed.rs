//! PostgreSQL typed streaming against the Docker matrix (Task 12; HV-RES-04 PG, HV-RES-01 PG
//! mapping, R-M3-18, R-M3-31, P5). Every test is `#[ignore = "docker"]`; targets come only from
//! `require_target(..)` and `targets(Some(Postgres))` (Q6): pg17 on the `pr` set, one server per
//! major from 14 to 18 on `full`. Fixtures are temporary (types in `pg_temp`, temp tables) or
//! created idempotently. Run with `<main>/target/docker/run-docker-tests.sh --set pr -p
//! hacivat-driver-pg`.

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_driver::{
    CloseHow, Closed, DriverConnect, Endpoint, ExecOpts, InsecureAuth, Protocol, QueryDone,
    QueryEvent, QuerySpec, RawConn, RawErrorKind, ResultEnd, TxStatus,
};
use hacivat_driver_pg::{PgConn, connect_conn};
use hacivat_net::{AddressClass, Target, TlsMode, TlsSettings, build_tls_params, direct};
use hacivat_proto::DbmsKind;
use hacivat_testkit::{DockerTarget, TlsDefault, require_target, targets};
use hacivat_values::{
    Chunk, DateVal, DecimalVal, LogicalType, OriginHint, ResultSchema, SpecialVal, TimeVal, TsVal,
    Value,
};
use secrecy::SecretString;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn request(t: &DockerTarget, database: &str) -> DriverConnect {
    let addr: SocketAddr = format!("{}:{}", t.host, t.port).parse().unwrap();
    let factory = direct(
        Target::Tcp {
            host: t.host.clone(),
            port: t.port,
        },
        vec![addr],
        secs(5),
    );
    let mode = match t.tls_default {
        TlsDefault::SelfSigned => TlsMode::Require,
        TlsDefault::None => TlsMode::Disable,
    };
    let settings = TlsSettings {
        mode,
        ..TlsSettings::default()
    };
    DriverConnect {
        endpoint: Endpoint::Network {
            factory,
            direct: None,
            resolved: vec![],
            tls_host: t.host.clone(),
        },
        tls: build_tls_params(&settings, &t.host, t.port, AddressClass::Loopback, None).unwrap(),
        user: Some(t.user.clone()),
        password: Some(SecretString::from(t.password.clone())),
        database: Some(database.to_string()),
        application_name: "Hacivat".into(),
        read_only: false,
        options: vec![],
        connect_timeout: secs(10),
        events: Arc::new(|_| {}),
        insecure_auth: InsecureAuth::Allow,
    }
}

async fn pg_on(t: &DockerTarget) -> PgConn {
    connect_conn(request(t, &t.database))
        .await
        .unwrap_or_else(|e| panic!("{}: {e:?}", t.id))
}

/// One server per PostgreSQL major from 14 to 18 in the active set (pg17 alone on `pr`); the
/// active set must hold one.
fn version_targets() -> Vec<DockerTarget> {
    let mut majors = BTreeSet::new();
    let list: Vec<DockerTarget> = targets(Some(DbmsKind::Postgres))
        .into_iter()
        .filter(|t| t.major.parse::<u32>().is_ok_and(|m| (14..=18).contains(&m)))
        .filter(|t| t.tls_default == TlsDefault::None)
        .filter(|t| majors.insert(t.major.clone()))
        .collect();
    assert!(
        !list.is_empty(),
        "no PostgreSQL 14-18 target in the active set"
    );
    list
}

fn spec(sql: &str, suspendable: bool, first_rows: u32) -> QuerySpec<'_> {
    QuerySpec {
        sql,
        binds: &[],
        protocol: Protocol::Extended,
        suspendable,
        first_rows,
        chunk_rows: 4_096,
        chunk_bytes: 4 << 20,
        cell_cap: 1 << 20,
        max_result_sets: 1,
    }
}

/// A whole non-suspended result.
struct Result1 {
    schema: Arc<ResultSchema>,
    chunks: Vec<Chunk>,
    end: ResultEnd,
    done: QueryDone,
}

impl Result1 {
    fn rows(&self) -> u64 {
        self.chunks.iter().map(|c| u64::from(c.rows())).sum()
    }

    /// The cell at (`row`, `col`) across chunks.
    fn cell(&self, mut row: u32, col: u32) -> Value {
        for c in &self.chunks {
            if row < c.rows() {
                return c.cell(row, col).to_value();
            }
            row -= c.rows();
        }
        panic!("row outside the result");
    }
}

async fn run(c: &mut PgConn, s: QuerySpec<'_>) -> Result1 {
    c.query(s)
        .await
        .unwrap_or_else(|e| panic!("{}: {e:?}", s.sql));
    let QueryEvent::ResultStart(schema) = c.next_event().await.unwrap() else {
        panic!("ResultStart first");
    };
    let mut chunks = vec![];
    let end = loop {
        match c.next_event().await.unwrap() {
            QueryEvent::Rows(chunk) => chunks.push(chunk),
            QueryEvent::ResultEnd(end) => break end,
            other => panic!("{other:?}"),
        }
    };
    let QueryEvent::Done(done) = c.next_event().await.unwrap() else {
        panic!("Done after ResultEnd");
    };
    Result1 {
        schema,
        chunks,
        end,
        done,
    }
}

/// The observer's count is exact once the connection is idle: after a round trip every
/// earlier `Sync` (the portal's and the statement's `Close` included) has had its
/// `ReadyForQuery`.
async fn assert_synced(c: &mut PgConn) {
    c.ping().await.unwrap();
    assert!(c.wire_state().synced(), "every Sync answered");
}

async fn scalar(c: &mut PgConn, sql: &str) -> Option<String> {
    let s = c.execute(sql, ExecOpts::values(1)).await.unwrap();
    s.rows
        .into_iter()
        .next()
        .and_then(|r| r.into_iter().next().flatten())
}

/// Named (protocol- or SQL-level) portals open on the connection's session.
async fn open_portals(c: &mut PgConn) -> String {
    scalar(c, "SELECT count(*) FROM pg_cursors WHERE name <> ''")
        .await
        .unwrap()
}

fn dec(s: &str) -> Value {
    Value::Decimal(DecimalVal::parse(s).unwrap())
}

fn civil(y: i32, m: u8, d: u8) -> DateVal {
    DateVal::from_civil(y, m, d).unwrap()
}

/// Seconds since 1970 of a civil date and time (UTC).
fn epoch(y: i32, mo: u8, d: u8, h: i64, mi: i64, s: i64) -> i64 {
    i64::from(civil(y, mo, d).0) * 86_400 + h * 3_600 + mi * 60 + s
}

const GOLDEN_DDL: &str = "\
    CREATE TYPE pg_temp.hv_t12_mood AS ENUM ('sad', 'ok', 'happy');
    CREATE TEMP TABLE hv_types (
        b bool, i2 int2, i4 int4, i8 int8, o oid, f4 float4, f8 float8,
        n numeric, n102 numeric(10,2), n305 numeric(30,5),
        t text, vc varchar(10), bc char(3), nm name, ch \"char\",
        by bytea, u uuid, j json, jb jsonb,
        d date, tm time, ttz timetz, ts timestamp, tstz timestamptz, iv interval,
        e pg_temp.hv_t12_mood,
        ai int4[], mo money, ine inet, tsv tsvector);
    INSERT INTO hv_types VALUES
      (true, -2, -123456, 9223372036854775807, 4294967295, 0.1, '-0',
       12345.678, -1.50, 1234567890.12345,
       'ş İ ı', 'abc', 'ab', 'pg_class', 'r',
       '\\x00ff0a', '6f1e9a0c-0000-4000-8000-000000000001', '{\"b\":1.50,\"a\":1e3}', '{\"a\": 1}',
       '2000-01-01', '24:00:00', '01:00:00+03', '1999-12-31 23:59:59.999999',
       '2026-11-11 07:00:00+00', '1 year 2 mons 3 days 04:05:06.000007',
       'happy', '{1,NULL,3}', 12.34, '192.168.0.1/24', 'a b'),
      (false, 0, 0, 0, 0, 'NaN', '-Infinity',
       'Infinity', 'NaN', 0,
       '', '', '', '', '',
       '\\x', '00000000-0000-0000-0000-000000000000', 'null', '[]',
       'infinity', '00:00:00', '23:59:59-05', '-infinity',
       'infinity', '0 seconds',
       'sad', '{}', 0, '::1', ''),
      (NULL, NULL, NULL, NULL, NULL, NULL, NULL,
       '-Infinity', NULL, NULL,
       NULL, NULL, NULL, NULL, NULL,
       NULL, NULL, NULL, NULL,
       '4713-01-01 BC', NULL, NULL, '294276-12-31 23:59:59.999999',
       '0001-01-01 00:00:00+00', NULL,
       NULL, NULL, NULL, NULL, NULL);";

#[tokio::test]
#[ignore = "docker"]
async fn type_golden_table_hv_types() {
    for t in version_targets() {
        let mut c = pg_on(&t).await;
        c.batch(GOLDEN_DDL)
            .await
            .unwrap_or_else(|e| panic!("{}: {e:?}", t.id));
        let table_oid: u32 = scalar(&mut c, "SELECT 'pg_temp.hv_types'::regclass::oid")
            .await
            .unwrap()
            .parse()
            .unwrap();
        let r = run(
            &mut c,
            spec("SELECT * FROM hv_types ORDER BY i4 NULLS LAST", false, 501),
        )
        .await;
        let id = &t.id;
        let cols = &r.schema.columns;
        assert_eq!(cols.len(), 30, "{id}");
        for (i, col) in cols.iter().enumerate() {
            assert_eq!(
                col.origin,
                Some(OriginHint::Pg {
                    table_oid,
                    attnum: i16::try_from(i + 1).unwrap()
                }),
                "{id} {}",
                col.name
            );
        }
        let names: Vec<&str> = cols.iter().map(|c| &*c.name).collect();
        let at = |name: &str| names.iter().position(|n| *n == name).unwrap() as u32;
        assert_eq!(
            cols[at("n102") as usize].logical,
            LogicalType::Decimal {
                precision: Some(10),
                scale: Some(2)
            },
            "{id}"
        );
        assert_eq!(
            cols[at("bc") as usize].logical,
            LogicalType::Text {
                max_chars: Some(3),
                fixed: true
            },
            "{id}"
        );
        assert_eq!(
            cols[at("tstz") as usize].logical,
            LogicalType::Timestamp {
                precision: 6,
                tz: true
            },
            "{id}"
        );
        match &cols[at("e") as usize].logical {
            LogicalType::Enum(labels) => assert_eq!(
                labels.iter().map(|l| &**l).collect::<Vec<_>>(),
                ["sad", "ok", "happy"],
                "{id}"
            ),
            other => panic!("{id}: {other:?}"),
        }
        assert_eq!(r.rows(), 3, "{id}");
        assert_eq!(r.end.rows_affected, Some(3), "{id}");
        assert_eq!(r.end.command_tag.as_deref(), Some("SELECT 3"), "{id}");
        assert_eq!(r.done.tx, TxStatus::Idle, "{id}");

        // Row 0 (i4 = -123456 sorts first).
        let uuid: [u8; 16] = [
            0x6f, 0x1e, 0x9a, 0x0c, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 0, 0, 1,
        ];
        let expect_row0: Vec<(&str, Value)> = vec![
            ("b", Value::Bool(true)),
            ("i2", Value::Int(-2)),
            ("i4", Value::Int(-123_456)),
            ("i8", Value::Int(i64::MAX)),
            ("o", Value::Int(i64::from(u32::MAX))),
            ("f4", Value::F32(0.1)),
            ("n", dec("12345.678")),
            ("n102", dec("-1.50")),
            ("n305", dec("1234567890.12345")),
            ("t", Value::Text("ş İ ı".into())),
            ("vc", Value::Text("abc".into())),
            ("bc", Value::Text("ab ".into())),
            ("nm", Value::Text("pg_class".into())),
            ("ch", Value::Text("r".into())),
            ("by", Value::Bytes(Arc::from(&[0u8, 0xff, 0x0a][..]))),
            ("u", Value::Uuid(uuid)),
            ("j", Value::Json("{\"b\":1.50,\"a\":1e3}".into())),
            ("jb", Value::Json("{\"a\": 1}".into())),
            ("d", Value::Date(civil(2000, 1, 1))),
            ("tm", Value::Time(TimeVal::END_OF_DAY)),
            (
                "ttz",
                Value::TimeTz(
                    TimeVal {
                        nanos_of_day: 3_600_000_000_000,
                    },
                    10_800,
                ),
            ),
            (
                "ts",
                Value::Timestamp(TsVal {
                    secs: epoch(1999, 12, 31, 23, 59, 59),
                    nanos: 999_999_000,
                }),
            ),
            (
                "tstz",
                Value::TimestampTz {
                    utc: TsVal {
                        secs: epoch(2026, 11, 11, 7, 0, 0),
                        nanos: 0,
                    },
                    offset: None,
                },
            ),
            (
                "iv",
                Value::Interval {
                    months: 14,
                    days: 3,
                    micros: ((4 * 60 + 5) * 60 + 6) * 1_000_000 + 7,
                },
            ),
            ("e", Value::Text("happy".into())),
        ];
        for (name, want) in expect_row0 {
            assert_eq!(r.cell(0, at(name)), want, "{id} row 0 {name}");
        }
        match r.cell(0, at("f8")) {
            Value::F64(v) => assert!(v == 0.0 && v.is_sign_negative(), "{id} -0"),
            other => panic!("{id}: {other:?}"),
        }
        for (name, type_name, text) in [
            ("ai", "_int4", Some("{1,NULL,3}")),
            ("mo", "money", None),
            ("ine", "inet", Some("192.168.0.1/24")),
            ("tsv", "tsvector", Some("'a' 'b'")),
        ] {
            match r.cell(0, at(name)) {
                Value::Opaque(o) => {
                    assert_eq!(&*o.type_name, type_name, "{id} {name}");
                    if let Some(text) = text {
                        assert_eq!(&*o.text, text, "{id} {name}");
                    } else {
                        assert!(o.text.contains("12"), "{id} {name}: {}", o.text);
                    }
                }
                other => panic!("{id} {name}: {other:?}"),
            }
        }

        // Row 1: the special values.
        assert!(
            matches!(r.cell(1, at("f4")), Value::F32(v) if v.is_nan()),
            "{id}"
        );
        assert_eq!(r.cell(1, at("f8")), Value::F64(f64::NEG_INFINITY), "{id}");
        assert_eq!(
            r.cell(1, at("n")),
            Value::Special(SpecialVal::PosInfinity),
            "{id}"
        );
        assert_eq!(
            r.cell(1, at("n102")),
            Value::Special(SpecialVal::NaN),
            "{id}"
        );
        assert_eq!(r.cell(1, at("n305")), dec("0.00000"), "{id}");
        assert_eq!(r.cell(1, at("t")), Value::Text("".into()), "{id}");
        assert_eq!(r.cell(1, at("bc")), Value::Text("   ".into()), "{id}");
        assert_eq!(
            r.cell(1, at("by")),
            Value::Bytes(Arc::from(&[][..])),
            "{id}"
        );
        assert_eq!(r.cell(1, at("j")), Value::Json("null".into()), "{id}");
        assert_eq!(
            r.cell(1, at("d")),
            Value::Special(SpecialVal::PosInfinity),
            "{id}"
        );
        assert_eq!(
            r.cell(1, at("ts")),
            Value::Special(SpecialVal::NegInfinity),
            "{id}"
        );
        assert_eq!(
            r.cell(1, at("tstz")),
            Value::Special(SpecialVal::PosInfinity),
            "{id}"
        );
        assert_eq!(
            r.cell(1, at("ttz")),
            Value::TimeTz(
                TimeVal {
                    nanos_of_day: ((23 * 60 + 59) * 60 + 59) * 1_000_000_000,
                },
                -18_000
            ),
            "{id}"
        );
        assert_eq!(
            r.cell(1, at("iv")),
            Value::Interval {
                months: 0,
                days: 0,
                micros: 0
            },
            "{id}"
        );

        // Row 2: NULLs, the BC date and the timestamp past i64 microseconds since 1970.
        for name in ["b", "i4", "t", "by", "u", "j", "jb", "tm", "e", "ai", "tsv"] {
            assert_eq!(r.cell(2, at(name)), Value::Null, "{id} {name}");
        }
        assert_eq!(
            r.cell(2, at("n")),
            Value::Special(SpecialVal::NegInfinity),
            "{id}"
        );
        // 4713 BC is astronomical year -4712.
        assert_eq!(r.cell(2, at("d")), Value::Date(civil(-4712, 1, 1)), "{id}");
        assert_eq!(
            r.cell(2, at("ts")),
            Value::Timestamp(TsVal {
                secs: epoch(294_276, 12, 31, 23, 59, 59),
                nanos: 999_999_000,
            }),
            "{id}"
        );
        assert_eq!(
            r.cell(2, at("tstz")),
            Value::TimestampTz {
                utc: TsVal {
                    secs: epoch(1, 1, 1, 0, 0, 0),
                    nanos: 0
                },
                offset: None
            },
            "{id}"
        );
        assert_synced(&mut c).await;
        Box::new(c).close().await;
    }
}

/// `SELECT` of 50 columns of five kinds over `generate_series(1, n)`.
fn wide_select(n: u64) -> String {
    let cols: Vec<String> = (0..50)
        .map(|k| match k % 5 {
            0 => format!("g + {k} AS c{k}"),
            1 => format!("(g + {k})::text AS c{k}"),
            2 => format!("(g + {k})::numeric(12,2) AS c{k}"),
            3 => format!(
                "timestamptz '2026-01-01 00:00:00+00' + (g + {k}) * interval '1 second' AS c{k}"
            ),
            _ => format!("(g + {k}) % 2 = 0 AS c{k}"),
        })
        .collect();
    format!("SELECT {} FROM generate_series(1, {n}) g", cols.join(", "))
}

fn first_int(chunk: &Chunk) -> (i64, i64) {
    let int = |r| match chunk.cell(r, 0).to_value() {
        Value::Int(v) => v,
        other => panic!("{other:?}"),
    };
    (int(0), int(chunk.rows() - 1))
}

#[tokio::test]
#[ignore = "docker"]
async fn million_rows_suspend_resume_close() {
    let Some(t) = require_target("pg17") else {
        return;
    };
    let mut c = pg_on(&t).await;
    // P5: a portal lives inside a transaction block.
    c.batch("BEGIN").await.unwrap();
    let sql = wide_select(1_000_000);
    let started = Instant::now();
    c.query(spec(&sql, true, 501)).await.unwrap();
    let QueryEvent::ResultStart(schema) = c.next_event().await.unwrap() else {
        panic!("ResultStart");
    };
    assert_eq!(schema.columns.len(), 50);
    let QueryEvent::Rows(first) = c.next_event().await.unwrap() else {
        panic!("Rows");
    };
    assert_eq!(first.rows(), 501, "the first chunk");
    assert_eq!(first_int(&first), (1, 501));
    assert_eq!(
        first.cell(0, 2).to_value(),
        dec("3.00"),
        "numeric(12,2) keeps its scale"
    );
    assert_eq!(c.next_event().await.unwrap(), QueryEvent::Suspended);
    assert_eq!(c.tx_status(), TxStatus::InTx);
    assert_eq!(open_portals(&mut c).await, "1", "the portal is open");
    c.fetch_more(10_000).await.unwrap();
    let mut sizes = vec![];
    let mut last = 501;
    loop {
        match c.next_event().await.unwrap() {
            QueryEvent::Rows(chunk) => {
                let (lo, hi) = first_int(&chunk);
                assert_eq!(lo, last + 1, "rows continue where they stopped");
                last = hi;
                sizes.push(chunk.rows());
            }
            QueryEvent::Suspended => break,
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(sizes, [4_096, 4_096, 1_808]);
    assert_eq!(
        c.close_query(CloseHow::Now).await.unwrap(),
        Closed::Closed { rows: 10_501 }
    );
    assert_synced(&mut c).await;
    assert_eq!(c.tx_status(), TxStatus::InTx);
    assert_eq!(open_portals(&mut c).await, "0", "Close reached the server");
    c.batch("ROLLBACK").await.unwrap();
    assert_eq!(c.tx_status(), TxStatus::Idle);
    // A million rows were never materialized: the portal ran 10,501 of them.
    assert!(started.elapsed() < secs(60), "{:?}", started.elapsed());
    assert_synced(&mut c).await;
    Box::new(c).close().await;
}

#[tokio::test]
#[ignore = "docker"]
async fn statement_between_portal_executes_keeps_the_portal() {
    for t in version_targets() {
        let mut c = pg_on(&t).await;
        c.batch("BEGIN").await.unwrap();
        c.query(spec(
            "SELECT g FROM generate_series(1, 100000) g",
            true,
            501,
        ))
        .await
        .unwrap();
        assert!(matches!(
            c.next_event().await.unwrap(),
            QueryEvent::ResultStart(_)
        ));
        assert!(matches!(c.next_event().await.unwrap(), QueryEvent::Rows(_)));
        assert_eq!(c.next_event().await.unwrap(), QueryEvent::Suspended);
        // The read-only guard's canary runs while the portal waits (P5's premise).
        assert_eq!(
            scalar(&mut c, "SELECT txid_current_if_assigned()").await,
            None,
            "{}",
            t.id
        );
        let s = c.execute("SELECT 1", ExecOpts::default()).await.unwrap();
        assert_eq!(s.row_count, 1, "{}", t.id);
        c.fetch_more(100).await.unwrap();
        let QueryEvent::Rows(chunk) = c.next_event().await.unwrap() else {
            panic!("{}: Rows", t.id);
        };
        assert_eq!(first_int(&chunk), (502, 601), "{}", t.id);
        assert_eq!(c.next_event().await.unwrap(), QueryEvent::Suspended);
        assert_eq!(
            c.close_query(CloseHow::Now).await.unwrap(),
            Closed::Closed { rows: 601 }
        );
        c.batch("ROLLBACK").await.unwrap();
        assert_synced(&mut c).await;
        Box::new(c).close().await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn portal_inside_a_user_transaction_keeps_in_tx() {
    let Some(t) = require_target("pg17") else {
        return;
    };
    let mut c = pg_on(&t).await;
    c.batch("BEGIN; CREATE TEMP TABLE hv_t12_tx(a int) ON COMMIT DROP; INSERT INTO hv_t12_tx SELECT generate_series(1, 1200)")
        .await
        .unwrap();
    assert_eq!(c.tx_status(), TxStatus::InTx);
    c.query(spec("SELECT a FROM hv_t12_tx ORDER BY a", true, 501))
        .await
        .unwrap();
    let mut rows = 0u64;
    let end = loop {
        match c.next_event().await.unwrap() {
            QueryEvent::ResultStart(_) => {}
            QueryEvent::Rows(chunk) => rows += u64::from(chunk.rows()),
            QueryEvent::Suspended => {
                assert_eq!(c.tx_status(), TxStatus::InTx);
                c.fetch_more(500).await.unwrap();
            }
            QueryEvent::ResultEnd(end) => break end,
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(rows, 1_200);
    assert_eq!((end.rows, end.rows_affected), (1_200, Some(1_200)));
    assert_eq!(end.command_tag.as_deref(), Some("SELECT 1200"));
    let QueryEvent::Done(done) = c.next_event().await.unwrap() else {
        panic!("Done");
    };
    assert_eq!(done.tx, TxStatus::InTx, "the user's transaction stays open");
    // The rows the transaction inserted are still there, and it commits.
    assert_eq!(
        scalar(&mut c, "SELECT count(*) FROM hv_t12_tx")
            .await
            .as_deref(),
        Some("1200")
    );
    c.batch("COMMIT").await.unwrap();
    assert_eq!(c.tx_status(), TxStatus::Idle);
    assert_eq!(open_portals(&mut c).await, "0");
    assert_synced(&mut c).await;
    Box::new(c).close().await;
}

#[tokio::test]
#[ignore = "docker"]
async fn drain_completes_a_short_rest_and_closes_a_long_one() {
    let Some(t) = require_target("pg17") else {
        return;
    };
    let mut c = pg_on(&t).await;
    c.batch("BEGIN").await.unwrap();
    for (n, want) in [
        (600u64, Closed::Completed { rows: 600 }),
        (100_000, Closed::Closed { rows: 501 }),
    ] {
        let sql = format!("SELECT g FROM generate_series(1, {n}) g");
        c.query(spec(&sql, true, 501)).await.unwrap();
        while c.next_event().await.unwrap() != QueryEvent::Suspended {}
        assert_eq!(
            c.close_query(CloseHow::Drain(secs(10))).await.unwrap(),
            want,
            "{n}"
        );
        assert_eq!(open_portals(&mut c).await, "0", "{n}");
    }
    c.batch("ROLLBACK").await.unwrap();
    assert_synced(&mut c).await;
    Box::new(c).close().await;
}

#[tokio::test]
#[ignore = "docker"]
async fn copy_is_refused_before_sending() {
    let Some(t) = require_target("pg17") else {
        return;
    };
    let mut c = pg_on(&t).await;
    assert_synced(&mut c).await;
    let before = c.wire_state().ready_count();
    for sql in [
        "COPY (SELECT 1) TO STDOUT",
        "/* export */ copy pg_class from stdin",
    ] {
        let e = c.query(spec(sql, false, 501)).await.unwrap_err();
        assert_eq!(e.kind, RawErrorKind::Unsupported, "{sql}");
    }
    assert_eq!(c.wire_state().ready_count(), before, "no message went out");
    assert_eq!(scalar(&mut c, "SELECT 1").await.as_deref(), Some("1"));
    Box::new(c).close().await;
}

/// Waits until `pid` runs `pg_sleep` (seen from another connection), so the cancel finds a
/// statement to cancel.
async fn wait_until_sleeping(side: &mut PgConn, pid: u64) {
    let sql =
        format!("SELECT wait_event FROM pg_stat_activity WHERE pid = {pid} AND state = 'active'");
    let deadline = Instant::now() + secs(20);
    loop {
        let seen = scalar(side, &sql).await;
        if seen.as_deref() == Some("PgSleep") {
            return;
        }
        if Instant::now() >= deadline {
            let all = side
                .execute(
                    "SELECT pid, state, wait_event, query FROM pg_stat_activity",
                    ExecOpts::values(50),
                )
                .await
                .unwrap();
            panic!(
                "pid {pid} never ran pg_sleep (last {seen:?}): {:?}",
                all.rows
            );
        }
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn protocol_cancel_during_execute() {
    for t in version_targets() {
        let mut c = pg_on(&t).await;
        let mut side = pg_on(&t).await;
        let pid = c.backend_id().unwrap();
        let cancel = c.cancel_handle().unwrap();
        c.query(spec("SELECT pg_sleep(60)", false, 501))
            .await
            .unwrap();
        assert!(matches!(
            c.next_event().await.unwrap(),
            QueryEvent::ResultStart(_)
        ));
        let started = Instant::now();
        let (r, cancelled) = tokio::join!(c.next_event(), async {
            wait_until_sleeping(&mut side, pid).await;
            cancel.cancel().await
        });
        assert!(cancelled.is_ok(), "{}: {cancelled:?}", t.id);
        let e = r.unwrap_err();
        assert_eq!(e.kind, RawErrorKind::Db, "{}", t.id);
        assert_eq!(
            e.server.unwrap().sqlstate.as_deref(),
            Some("57014"),
            "{}",
            t.id
        );
        assert!(started.elapsed() < secs(10), "{}", t.id);
        assert_eq!(c.tx_status(), TxStatus::Idle, "{}", t.id);
        assert_eq!(scalar(&mut c, "SELECT 1").await.as_deref(), Some("1"));
        assert_synced(&mut c).await;
        Box::new(c).close().await;
        Box::new(side).close().await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn dml_and_ddl_report_one_empty_result() {
    let Some(t) = require_target("pg17") else {
        return;
    };
    let mut c = pg_on(&t).await;
    let r = run(
        &mut c,
        spec("CREATE TEMP TABLE hv_t12_dml(a int)", false, 501),
    )
    .await;
    assert!(r.schema.columns.is_empty());
    assert_eq!(r.end.rows_affected, None);
    assert_eq!(r.end.command_tag.as_deref(), Some("CREATE TABLE"));
    let r = run(
        &mut c,
        spec(
            "INSERT INTO hv_t12_dml SELECT generate_series(1, 3)",
            false,
            501,
        ),
    )
    .await;
    assert!(r.schema.columns.is_empty());
    assert_eq!((r.end.rows, r.end.rows_affected), (0, Some(3)));
    assert_eq!(r.end.command_tag.as_deref(), Some("INSERT 0 3"));
    let r = run(
        &mut c,
        spec("UPDATE hv_t12_dml SET a = a + 1 RETURNING a", false, 501),
    )
    .await;
    assert_eq!((r.rows(), r.end.rows_affected), (3, Some(3)));
    assert_eq!(r.end.command_tag.as_deref(), Some("UPDATE 3"));
    assert_synced(&mut c).await;
    Box::new(c).close().await;
}

#[tokio::test]
#[ignore = "docker"]
async fn large_values_are_truncated_at_the_cell_cap() {
    let Some(t) = require_target("pg17") else {
        return;
    };
    let mut c = pg_on(&t).await;
    let r = run(
        &mut c,
        spec(
            "SELECT repeat('ş', 3000000) AS t, decode(repeat('ab', 5242880), 'hex') AS b",
            false,
            501,
        ),
    )
    .await;
    match r.cell(0, 0) {
        Value::Truncated(t) => {
            assert_eq!(t.total_len, Some(6_000_000));
            assert!(matches!(&t.prefix, Value::Text(p) if p.len() == 1 << 20));
        }
        other => panic!("{:?}", std::mem::discriminant(&other)),
    }
    match r.cell(0, 1) {
        Value::Truncated(t) => {
            assert_eq!(t.total_len, Some(5 << 20));
            assert!(matches!(&t.prefix, Value::Bytes(p) if p.len() == 1 << 20));
        }
        other => panic!("{:?}", std::mem::discriminant(&other)),
    }
    assert_synced(&mut c).await;
    Box::new(c).close().await;
}

/// A `SQL_ASCII` database, created once (another run may race to create it).
const ASCII_DB: &str = "hv_t12_sql_ascii";

#[tokio::test]
#[ignore = "docker"]
async fn sql_ascii_text_that_is_not_utf8_is_lossy() {
    let Some(t) = require_target("pg17") else {
        return;
    };
    let mut admin = pg_on(&t).await;
    let exists = scalar(
        &mut admin,
        &format!("SELECT count(*) FROM pg_database WHERE datname = '{ASCII_DB}'"),
    )
    .await;
    if exists.as_deref() == Some("0") {
        let r = admin
            .batch(&format!(
                "CREATE DATABASE {ASCII_DB} ENCODING 'SQL_ASCII' LC_COLLATE 'C' LC_CTYPE 'C' \
                 TEMPLATE template0"
            ))
            .await;
        if let Err(e) = r {
            let code = e.server.as_ref().and_then(|s| s.sqlstate.clone());
            assert!(matches!(code.as_deref(), Some("42P04" | "23505")), "{e:?}");
        }
    }
    Box::new(admin).close().await;
    let mut c = connect_conn(request(&t, ASCII_DB)).await.unwrap();
    // Under the driver's `client_encoding = UTF8` the server refuses such bytes itself
    // (22021); a session that switches to SQL_ASCII receives them as they are stored.
    let refused = match c.query(spec("SELECT E'a\\303'::text", false, 501)).await {
        Ok(()) => {
            assert!(matches!(
                c.next_event().await.unwrap(),
                QueryEvent::ResultStart(_)
            ));
            c.next_event().await.unwrap_err()
        }
        Err(e) => e,
    };
    assert_eq!(
        refused.server.and_then(|s| s.sqlstate).as_deref(),
        Some("22021")
    );
    c.batch("SET client_encoding = 'SQL_ASCII'").await.unwrap();
    let r = run(
        &mut c,
        spec("SELECT E'a\\303'::text, 'ok'::text", false, 501),
    )
    .await;
    assert_eq!(r.cell(0, 0), Value::Lossy(Arc::from(&[0x61u8, 0xc3][..])));
    assert_eq!(r.cell(0, 1), Value::Text("ok".into()));
    assert_synced(&mut c).await;
    Box::new(c).close().await;
}
