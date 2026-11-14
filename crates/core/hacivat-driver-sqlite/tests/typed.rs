//! Typed streaming on SQLite (Task 15, HV-RES-04 SQLite): one job on the connection's thread
//! steps the statement into columnar chunks, sends them on a bounded channel, and at the
//! read-ahead point waits for `fetch_more` or `close_query`. Affinity mapping, storage-class
//! exceptions, the read-only hardening on the typed path, the interrupt, the 5 s busy timeout
//! and the idle bound of a rollback-journal cursor.

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

mod common;
mod typed_support;

use std::time::{Duration, Instant};

use common::simple;
use hacivat_driver::{
    CloseHow, Closed, Driver as _, Protocol, QueryEvent, QuerySpec, RawErrorKind, TxStatus,
};
use hacivat_driver_sqlite::decode::{buf_kind_of, rule_of};
use hacivat_values::{BindValue, BufKind, LogicalType, SqliteAffinity, Value, ValueRef, VarKind};
use rusqlite::types::Type;
use typed_support::*;

const ROLLBACK_IDLE_BOUND: Option<Duration> = Some(Duration::from_secs(30));

fn rows_of(e: QueryEvent) -> hacivat_values::Chunk {
    match e {
        QueryEvent::Rows(c) => c,
        other => panic!("not rows: {other:?}"),
    }
}

#[tokio::test]
async fn million_row_temp_file_first_chunk_then_suspend_resume_finalize() {
    let f = numbered(1_000_000);
    let mut c = open_rw(&f).await;
    c.query(cursor("SELECT a, b FROM t")).await.unwrap();
    // The cursor holds a SHARED lock in rollback-journal mode: its idle bound is 30 s (ER6).
    assert_eq!(c.typed_caps().idle_bound, ROLLBACK_IDLE_BOUND);
    let QueryEvent::ResultStart(schema) = c.next_event().await.unwrap() else {
        panic!("no ResultStart");
    };
    assert_eq!(names(&schema), ["a", "b"]);
    let first = rows_of(c.next_event().await.unwrap());
    assert_eq!(first.rows(), 501);
    assert!(matches!(first.cell(0, 0), ValueRef::Int(1)));
    assert!(matches!(first.cell(500, 1), ValueRef::Text("row 501")));
    assert_eq!(c.next_event().await.unwrap(), QueryEvent::Suspended);
    // A suspended query answers only fetch_more and close_query.
    assert_eq!(
        c.next_event().await.unwrap_err().kind,
        RawErrorKind::Unsupported
    );
    assert!(writer_blocked(&f), "the live statement holds its lock");
    c.fetch_more(10_000).await.unwrap();
    let mut sizes = Vec::new();
    loop {
        match c.next_event().await.unwrap() {
            QueryEvent::Rows(ch) => sizes.push(ch.rows()),
            QueryEvent::Suspended => break,
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(sizes, [4_096, 4_096, 1_808]);
    assert_eq!(
        c.close_query(CloseHow::Now).await.unwrap(),
        Closed::Closed { rows: 10_501 }
    );
    // Finalized before close_query returned: the lock is gone.
    assert!(!writer_blocked(&f));
    assert_eq!(
        scalar(&mut *c, "SELECT count(*) FROM t").await,
        Value::Int(1_000_000)
    );
}

#[tokio::test]
async fn a_whole_result_streams_without_suspending() {
    let f = numbered(10_000);
    let mut c = open_rw(&f).await;
    let r = run(&mut *c, spec("SELECT a, b FROM t")).await.unwrap();
    assert_eq!(r.suspensions, 0);
    assert_eq!(r.chunk_rows(), [501, 4_096, 4_096, 1_307]);
    assert_eq!(r.end.rows, 10_000);
    assert_eq!(r.end.kept, 10_000);
    assert_eq!(r.end.rows_affected, None);
    assert_eq!(r.end.command_tag.as_deref(), Some("SELECT"));
    assert_eq!(r.done.tx, TxStatus::Idle);
    assert_eq!(r.cell(9_999, 1), Value::Text("row 10000".into()));
    // A cursor resumed batch by batch sees the same rows.
    let r = run_with(&mut *c, cursor("SELECT a FROM t"), 3_000)
        .await
        .unwrap();
    assert_eq!(r.suspensions, 4);
    assert_eq!(
        r.column(0),
        (1..=10_000).map(Value::Int).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn chunks_seal_at_first_rows_chunk_rows_and_chunk_bytes() {
    let f = numbered(2_000);
    let mut c = open_rw(&f).await;
    let r = run(
        &mut *c,
        QuerySpec {
            first_rows: 10,
            chunk_rows: 700,
            ..spec("SELECT a, b FROM t")
        },
    )
    .await
    .unwrap();
    assert_eq!(r.chunk_rows(), [10, 700, 700, 590]);
    // By bytes: every chunk but the last reaches the byte limit, none passes its row limit.
    let limit = 4_000;
    let r = run(
        &mut *c,
        QuerySpec {
            chunk_bytes: limit,
            ..spec("SELECT a, b FROM t")
        },
    )
    .await
    .unwrap();
    assert_eq!(r.rows(), 2_000);
    let (last, sealed) = r.chunks.split_last().unwrap();
    assert!(!sealed.is_empty());
    for ch in sealed {
        assert!(ch.bytes() >= limit, "{} bytes", ch.bytes());
        assert!(ch.rows() <= 4_096);
    }
    assert!(last.rows() > 0);
}

#[tokio::test]
async fn a_cursor_that_ends_at_the_read_ahead_point_completes() {
    // Exactly the first batch: the read-ahead step finds no row, so the result ends without a
    // suspension and the statement is finalized (no lock held).
    let f = numbered(501);
    let mut c = open_rw(&f).await;
    let r = run(&mut *c, cursor("SELECT a FROM t")).await.unwrap();
    assert_eq!((r.suspensions, r.rows()), (0, 501));
    assert!(!writer_blocked(&f));
    // One row more: it suspends, and the held row is the next batch's first.
    let f = numbered(502);
    let mut c = open_rw(&f).await;
    let r = run_with(&mut *c, cursor("SELECT a FROM t"), 1)
        .await
        .unwrap();
    assert_eq!(r.suspensions, 1);
    assert_eq!(r.chunk_rows(), [501, 1]);
    assert_eq!(r.cell(501, 0), Value::Int(502));
}

#[tokio::test]
async fn dml_and_ddl_report_one_empty_result() {
    let f = fixture("");
    let mut c = open_rw(&f).await;
    let r = run(&mut *c, spec("CREATE TABLE u(a INTEGER)"))
        .await
        .unwrap();
    assert!(r.schema.columns.is_empty());
    assert!(r.chunks.is_empty());
    assert_eq!(
        (
            r.end.rows,
            r.end.rows_affected,
            r.end.command_tag.as_deref()
        ),
        (0, Some(0), Some("CREATE"))
    );
    let r = run(&mut *c, spec("INSERT INTO u VALUES (1), (2), (3)"))
        .await
        .unwrap();
    assert!(r.schema.columns.is_empty());
    assert_eq!(
        (r.end.rows_affected, r.end.command_tag.as_deref()),
        (Some(3), Some("INSERT"))
    );
    assert_eq!(r.done.tx, TxStatus::Idle);
    let r = run(&mut *c, spec("BEGIN")).await.unwrap();
    assert_eq!(r.done.tx, TxStatus::InTx);
    assert_eq!(c.tx_status(), TxStatus::InTx);
    let r = run(&mut *c, spec("UPDATE u SET a = a * 10 RETURNING a"))
        .await
        .unwrap();
    assert_eq!(
        r.column(0),
        [Value::Int(10), Value::Int(20), Value::Int(30)]
    );
    assert_eq!(r.end.rows_affected, Some(3));
    run(&mut *c, spec("ROLLBACK")).await.unwrap();
    assert_eq!(c.tx_status(), TxStatus::Idle);
    assert_eq!(scalar(&mut *c, "SELECT sum(a) FROM u").await, Value::Int(6));
    // Text without a statement runs nothing and still reports one result.
    let r = run(&mut *c, spec("-- nothing to run")).await.unwrap();
    assert!(r.schema.columns.is_empty());
    assert_eq!((r.end.rows_affected, r.end.command_tag), (None, None));
}

#[tokio::test]
async fn storage_class_mixing_becomes_exceptions() {
    assert!(matches!(
        cell_of(
            "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1), ('abc')",
            1,
            0
        )
        .await
        .view(),
        ValueRef::Text("abc")
    ));
    let f = fixture(
        "CREATE TABLE t(i INTEGER, r REAL, s TEXT, b BLOB);
         INSERT INTO t VALUES (1, 1.5, 'x', x'00');
         INSERT INTO t VALUES (2.5, 'y', 7, 'text in a blob column');  -- TEXT stores 7 as '7'
         INSERT INTO t VALUES (x'ff', x'01', x'02', 3);",
    );
    let mut c = open_rw(&f).await;
    let r = run(&mut *c, spec("SELECT * FROM t")).await.unwrap();
    let kinds: Vec<_> = r.schema.columns.iter().map(|c| c.logical.clone()).collect();
    assert_eq!(
        kinds,
        [
            LogicalType::Int {
                bits: 64,
                signed: true
            },
            LogicalType::Float { bits: 64 },
            LogicalType::Text {
                max_chars: None,
                fixed: false
            },
            LogicalType::Bytes { max: None },
        ]
    );
    // Rows 1 and 2: a cell of another storage class is a whole value of its own class, kept
    // exactly (TEXT affinity turned the 7 into text, so that one is inline).
    assert_eq!(
        [r.cell(1, 0), r.cell(1, 1), r.cell(1, 2), r.cell(1, 3)],
        [
            Value::F64(2.5),
            Value::Text("y".into()),
            Value::Text("7".into()),
            Value::Text("text in a blob column".into())
        ]
    );
    assert_eq!(
        [r.cell(2, 0), r.cell(2, 1), r.cell(2, 2), r.cell(2, 3)],
        [
            Value::Bytes([0xff].into()),
            Value::Bytes([1].into()),
            Value::Bytes([2].into()),
            Value::Int(3)
        ]
    );
}

#[test]
fn affinity_table() {
    use SqliteAffinity as A;
    // (declared type, affinity) — SQLite's five rules, in order, ASCII case-insensitive.
    for (decl, want) in [
        ("INT", A::Integer),
        ("integer", A::Integer),
        ("TINYINT(1)", A::Integer),
        ("BIGINT UNSIGNED", A::Integer),
        ("POINT", A::Integer), // "POINT" holds "INT": SQLite's rule 1
        ("CHAR(10)", A::Text),
        ("VARCHAR(255)", A::Text),
        ("clob", A::Text),
        ("TEXT", A::Text),
        ("CHARINT", A::Integer), // rule 1 wins over rule 2
        ("BLOB", A::Blob),
        ("", A::Blob),
        ("REAL", A::Real),
        ("FLOAT", A::Real),
        ("double precision", A::Real),
        ("NUMERIC(10,2)", A::Numeric),
        ("DECIMAL", A::Numeric),
        ("BOOLEAN", A::Numeric),
        ("DATETIME", A::Numeric),
        ("STRING", A::Numeric), // no rule matches "STRING": NUMERIC, as in SQLite
    ] {
        assert_eq!(rule_of(Some(decl)).0, want, "{decl}");
    }
    assert_eq!(rule_of(None).0, A::Blob);
    // (declared type, first non-NULL storage class of the first chunk, buffer).
    let text = BufKind::Var(VarKind::Text);
    let bytes = BufKind::Var(VarKind::Bytes);
    let f64 = BufKind::F64 { f32: false };
    for (decl, first, want) in [
        (Some("INTEGER"), Some(Type::Text), BufKind::I64),
        (Some("VARCHAR(5)"), Some(Type::Integer), text),
        (Some("REAL"), Some(Type::Integer), f64),
        (Some("DOUBLE"), None, f64),
        (Some("BLOB"), Some(Type::Integer), BufKind::I64),
        (Some("BLOB"), Some(Type::Text), text),
        (Some("BLOB"), None, bytes),
        (None, Some(Type::Real), f64),
        (None, Some(Type::Blob), bytes),
        (None, None, bytes),
        (Some("BOOL"), Some(Type::Integer), BufKind::Bool),
        (Some("boolean"), Some(Type::Text), BufKind::Bool),
        (Some("DATE"), Some(Type::Integer), text),
        (Some("DATETIME(6)"), Some(Type::Real), text),
        (Some("timestamp"), None, text),
        (Some("NUMERIC(10,2)"), Some(Type::Real), f64),
        (Some("DECIMAL"), Some(Type::Integer), BufKind::I64),
        (Some("DECIMAL"), Some(Type::Text), text),
        (Some("DECIMAL"), None, BufKind::I64),
    ] {
        assert_eq!(buf_kind_of(decl, first), want, "{decl:?} {first:?}");
    }
}

#[tokio::test]
async fn declared_types_set_the_logical_type_and_the_native_type() {
    let f = fixture(
        "CREATE TABLE t(n NUMERIC(10,2), flag BOOLEAN, d DATE, plain);
         INSERT INTO t VALUES (1.25, 1, '2026-11-14', 'x');
         INSERT INTO t VALUES (3, 0, 20261114, x'01');
         INSERT INTO t VALUES (NULL, 2, NULL, 5);",
    );
    let mut c = open_rw(&f).await;
    let r = run(&mut *c, spec("SELECT n, flag, d, plain, n + 1 AS e FROM t"))
        .await
        .unwrap();
    let col = |i: usize| &r.schema.columns[i];
    assert_eq!(col(0).logical, LogicalType::Float { bits: 64 });
    assert_eq!(
        col(0).native,
        hacivat_values::NativeType::Sqlite {
            decl: Some("NUMERIC(10,2)".into()),
            affinity: SqliteAffinity::Numeric
        }
    );
    assert_eq!(col(1).logical, LogicalType::Bool);
    assert_eq!(
        col(2).logical,
        LogicalType::Text {
            max_chars: None,
            fixed: false
        }
    );
    // A column without a declared type, and an expression: the first value's class.
    assert_eq!(
        col(3).logical,
        LogicalType::Text {
            max_chars: None,
            fixed: false
        }
    );
    assert_eq!(
        col(3).native,
        hacivat_values::NativeType::Sqlite {
            decl: None,
            affinity: SqliteAffinity::Blob
        }
    );
    assert_eq!(col(4).logical, LogicalType::Float { bits: 64 });
    assert_eq!(
        r.column(0),
        [Value::F64(1.25), Value::Int(3), Value::Null],
        "integers stay exact in a REAL buffer"
    );
    assert_eq!(
        r.column(1),
        [Value::Bool(true), Value::Bool(false), Value::Int(2)]
    );
    assert_eq!(
        r.column(2),
        [
            Value::Text("2026-11-14".into()),
            Value::Int(20_261_114),
            Value::Null
        ]
    );
    assert_eq!(
        r.column(3),
        [
            Value::Text("x".into()),
            Value::Bytes([1].into()),
            Value::Int(5)
        ]
    );
    // Origin hints from the column metadata (captured now, used in M6).
    assert_eq!(
        col(0).origin,
        Some(hacivat_values::OriginHint::Sqlite {
            database: "main".into(),
            table: "t".into(),
            column: "n".into()
        })
    );
    assert_eq!(col(4).origin, None);
}

#[tokio::test]
async fn large_and_invalid_text_keeps_its_bytes() {
    let f = fixture(
        "CREATE TABLE t(s TEXT, b BLOB);
         INSERT INTO t VALUES (CAST(x'61c3' AS TEXT), zeroblob(5000));
         INSERT INTO t VALUES (replace(hex(zeroblob(3000)), '00', 'ş'), x'00');",
    );
    let mut c = open_rw(&f).await;
    let r = run(
        &mut *c,
        QuerySpec {
            cell_cap: 999,
            ..spec("SELECT s, b FROM t")
        },
    )
    .await
    .unwrap();
    assert_eq!(r.cell(0, 0), Value::Lossy([0x61, 0xc3].into()));
    let Value::Truncated(t) = r.cell(0, 1) else {
        panic!("not truncated: {:?}", r.cell(0, 1));
    };
    assert_eq!(t.total_len, Some(5_000));
    assert_eq!(t.prefix, Value::Bytes(vec![0; 999].into()));
    // 3,000 two-byte characters under a 999-byte cap: cut at the character boundary below it.
    let Value::Truncated(t) = r.cell(1, 0) else {
        panic!("not truncated: {:?}", r.cell(1, 0));
    };
    assert_eq!(t.total_len, Some(6_000));
    assert_eq!(t.prefix, Value::Text("ş".repeat(499).into()));
}

#[tokio::test]
async fn read_only_authorizer_refuses_attach_on_the_typed_path() {
    let f = fixture("CREATE TABLE t(a); INSERT INTO t VALUES (1);");
    let mut c = open_ro(&f).await;
    let other = f.dir.path().join("other.db");
    let attach_file = format!("ATTACH '{}' AS o", other.display());
    for sql in [attach_file.as_str(), "ATTACH ':memory:' AS m"] {
        let e = c.query(spec(sql)).await.unwrap_err();
        assert_eq!((e.kind.clone(), code(&e)), (RawErrorKind::Db, 23), "{sql}");
    }
    assert!(!other.exists(), "ATTACH never created a file");
    for sql in [
        "INSERT INTO t VALUES (2)",
        "CREATE TABLE u(a)",
        "PRAGMA query_only = 0",
    ] {
        let e = c.query(spec(sql)).await.unwrap_err();
        assert_eq!(code(&e), 23, "{sql}");
    }
    // What never asks the authorizer is refused by sqlite3_stmt_readonly.
    let e = c.query(spec("VACUUM")).await.unwrap_err();
    assert_eq!((code(&e), text_of(&e).as_str()), (23, "readonly-required"));
    // Reads run, the journal-mode query too (Q16), and the hardening is unchanged.
    assert_eq!(scalar(&mut *c, "SELECT a FROM t").await, Value::Int(1));
    assert_eq!(
        scalar(&mut *c, "PRAGMA journal_mode").await,
        Value::Text("delete".into())
    );
    assert_eq!(scalar(&mut *c, "PRAGMA query_only").await, Value::Int(1));
    assert_eq!(
        scalar(&mut *c, "SELECT count(*) FROM t").await,
        Value::Int(1)
    );
}

#[tokio::test]
async fn interrupt_during_a_long_step_and_between_steps() {
    let f = numbered(3);
    let mut c = open_rw(&f).await;
    let cancel = c.cancel_handle().unwrap();
    // During one long step: only the interrupt ends it.
    c.query(spec(ENDLESS_STEP)).await.unwrap();
    let (event, cancelled) = unhung(async {
        tokio::join!(c.next_event(), async {
            tokio::task::yield_now().await;
            cancel.cancel().await
        })
    })
    .await;
    cancelled.unwrap();
    assert_eq!(event.unwrap_err().kind, RawErrorKind::Cancelled);
    // Between steps: a suspended cursor fails at its next batch.
    c.query(QuerySpec {
        first_rows: 1,
        ..cursor("SELECT a FROM t")
    })
    .await
    .unwrap();
    assert!(matches!(
        c.next_event().await.unwrap(),
        QueryEvent::ResultStart(_)
    ));
    assert_eq!(rows_of(c.next_event().await.unwrap()).rows(), 1);
    assert_eq!(c.next_event().await.unwrap(), QueryEvent::Suspended);
    cancel.cancel().await.unwrap();
    c.fetch_more(10).await.unwrap();
    assert_eq!(
        unhung(c.next_event()).await.unwrap_err().kind,
        RawErrorKind::Cancelled
    );
    // While the consumer stalls: the job is parked on the full channel between steps.
    let f = numbered(100_000);
    let mut c = open_rw(&f).await;
    let cancel = c.cancel_handle().unwrap();
    c.query(QuerySpec {
        chunk_rows: 100,
        ..spec("SELECT a FROM t")
    })
    .await
    .unwrap();
    assert!(matches!(
        c.next_event().await.unwrap(),
        QueryEvent::ResultStart(_)
    ));
    cancel.cancel().await.unwrap();
    let mut chunks = 0;
    let err = loop {
        match unhung(c.next_event()).await {
            Ok(QueryEvent::Rows(_)) => chunks += 1,
            Ok(other) => panic!("unexpected {other:?}"),
            Err(e) => break e,
        }
    };
    assert_eq!(err.kind, RawErrorKind::Cancelled);
    assert!(chunks <= 6, "{chunks} chunks after the cancel");
    // No cancel reaches the next query.
    assert_eq!(
        scalar(&mut *c, "SELECT count(*) FROM t").await,
        Value::Int(100_000)
    );
}

#[tokio::test]
async fn busy_after_five_seconds_has_the_hint() {
    let f = fixture("CREATE TABLE t(a);");
    let mut c = open_rw(&f).await;
    // The execution busy timeout, set after the open (the header read used the connect
    // timeout).
    assert_eq!(
        scalar(&mut *c, "PRAGMA busy_timeout").await,
        Value::Int(5_000)
    );
    let holder = rusqlite::Connection::open(&f.path).unwrap();
    holder
        .execute_batch("BEGIN IMMEDIATE; INSERT INTO t VALUES (1);")
        .unwrap();
    let err = run(&mut *c, spec("INSERT INTO t VALUES (2)"))
        .await
        .unwrap_err();
    assert_eq!(err.kind, RawErrorKind::Db);
    assert_eq!(code(&err) & 0xff, 5, "SQLITE_BUSY");
    assert_eq!(text_of(&err), "database is locked");
    assert_eq!(
        hacivat_driver_sqlite::error_hint(&err),
        Some("error-exec-sqlite-busy-hint")
    );
    // No retry: the same statement runs once the other connection commits.
    holder.execute_batch("COMMIT").unwrap();
    let r = run(&mut *c, spec("INSERT INTO t VALUES (2)"))
        .await
        .unwrap();
    assert_eq!(r.end.rows_affected, Some(1));
    // Other errors carry no hint.
    let e = c.query(spec("SELEC 1")).await.unwrap_err();
    assert_eq!(hacivat_driver_sqlite::error_hint(&e), None);
}

#[tokio::test]
async fn closed_command_channel_ends_the_job() {
    let f = numbered(2_000);
    let mut c = open_rw(&f).await;
    c.query(cursor("SELECT a FROM t")).await.unwrap();
    assert!(matches!(
        c.next_event().await.unwrap(),
        QueryEvent::ResultStart(_)
    ));
    rows_of(c.next_event().await.unwrap());
    assert_eq!(c.next_event().await.unwrap(), QueryEvent::Suspended);
    assert!(writer_blocked(&f));
    // The session discards the connection: the job's command channel closes, the job ends
    // (the statement is finalized) and the thread drops the connection.
    drop(c);
    wait_for_writer(&f);
    // `close` with a suspended cursor returns too.
    let mut c = open_rw(&f).await;
    c.query(cursor("SELECT a FROM t")).await.unwrap();
    assert!(matches!(
        c.next_event().await.unwrap(),
        QueryEvent::ResultStart(_)
    ));
    rows_of(c.next_event().await.unwrap());
    assert_eq!(c.next_event().await.unwrap(), QueryEvent::Suspended);
    c.close().await;
    assert!(!writer_blocked(&f));
}

#[tokio::test]
async fn dropping_the_connection_stops_a_long_step() {
    // One step that reads the table forever: no row ever comes, so no closed channel is seen.
    let f = numbered(10);
    let mut c = open_rw(&f).await;
    c.query(spec(
        "SELECT count(*) FROM t, (WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT x FROM c)",
    ))
    .await
    .unwrap();
    // `query` returns before the first step: wait until the step holds the table's lock.
    unhung(async {
        while !writer_blocked(&f) {
            tokio::task::yield_now().await;
        }
    })
    .await;
    // The session discards the connection without `close`: the step stops and the lock goes.
    drop(c);
    wait_for_writer(&f);
}

#[tokio::test]
async fn dropping_the_connection_mid_drain_stops_the_step() {
    // Rows 1 to 10 of a join with `t`, the eleventh held, then one step that never ends.
    let f = numbered(1);
    let mut c = open_rw(&f).await;
    c.query(QuerySpec {
        first_rows: 10,
        ..cursor(
            "SELECT x FROM (WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT x FROM c), t WHERE x <= 11",
        )
    })
    .await
    .unwrap();
    assert!(matches!(
        c.next_event().await.unwrap(),
        QueryEvent::ResultStart(_)
    ));
    assert_eq!(rows_of(c.next_event().await.unwrap()).rows(), 10);
    assert_eq!(c.next_event().await.unwrap(), QueryEvent::Suspended);
    // A drain with an hour to go is under way, and the session discards the connection.
    use futures::FutureExt as _;
    assert!(
        c.close_query(CloseHow::Drain(Duration::from_secs(3_600)))
            .now_or_never()
            .is_none()
    );
    drop(c);
    wait_for_writer(&f);
}

#[tokio::test]
async fn one_open_query_per_connection() {
    let f = numbered(2_000);
    let mut c = open_rw(&f).await;
    c.query(cursor("SELECT a FROM t")).await.unwrap();
    assert!(matches!(
        c.next_event().await.unwrap(),
        QueryEvent::ResultStart(_)
    ));
    rows_of(c.next_event().await.unwrap());
    assert_eq!(c.next_event().await.unwrap(), QueryEvent::Suspended);
    let unsupported = Some(RawErrorKind::Unsupported);
    assert_eq!(
        c.execute("SELECT 1", simple()).await.err().map(|e| e.kind),
        unsupported
    );
    assert_eq!(c.batch("SELECT 1").await.err().map(|e| e.kind), unsupported);
    assert_eq!(c.ping().await.err().map(|e| e.kind), unsupported);
    assert_eq!(c.server_info().await.err().map(|e| e.kind), unsupported);
    assert_eq!(
        c.query(spec("SELECT 1")).await.err().map(|e| e.kind),
        unsupported
    );
    assert!(c.probe().await.is_empty(), "the probe waits for no job");
    // Closing frees the connection; fetch_more and close_query without a query.
    assert_eq!(
        c.close_query(CloseHow::Now).await.unwrap(),
        Closed::Closed { rows: 501 }
    );
    assert_eq!(c.fetch_more(1).await.err().map(|e| e.kind), unsupported);
    assert_eq!(
        c.close_query(CloseHow::Now).await.unwrap(),
        Closed::Closed { rows: 0 }
    );
    assert_eq!(c.execute("SELECT 1", simple()).await.unwrap().rows.len(), 1);
    assert!(c.ping().await.is_ok());
}

#[tokio::test]
async fn drain_completes_a_short_rest_and_closes_a_long_one() {
    let f = numbered(1_000);
    let mut c = open_rw(&f).await;
    let suspend = async |c: &mut Box<dyn hacivat_driver::RawConn>, sql: &'static str| {
        c.query(QuerySpec {
            first_rows: 10,
            ..cursor(sql)
        })
        .await
        .unwrap();
        assert!(matches!(
            c.next_event().await.unwrap(),
            QueryEvent::ResultStart(_)
        ));
        assert_eq!(rows_of(c.next_event().await.unwrap()).rows(), 10);
        assert_eq!(c.next_event().await.unwrap(), QueryEvent::Suspended);
    };
    suspend(&mut c, "SELECT a FROM t").await;
    assert_eq!(
        unhung(c.close_query(CloseHow::Drain(Duration::from_secs(60))))
            .await
            .unwrap(),
        Closed::Completed { rows: 1_000 }
    );
    // Endless rows: the drain's time runs out between steps.
    suspend(&mut c, ENDLESS_ROWS).await;
    assert_eq!(
        unhung(c.close_query(CloseHow::Drain(Duration::from_millis(50))))
            .await
            .unwrap(),
        Closed::Closed { rows: 10 }
    );
    // One endless step: the progress handler ends it at the drain's deadline.
    suspend(&mut c, ELEVEN_THEN_ENDLESS).await;
    assert_eq!(
        unhung(c.close_query(CloseHow::Drain(Duration::from_millis(50))))
            .await
            .unwrap(),
        Closed::Closed { rows: 10 }
    );
    // The deadline does not outlive the drain.
    assert_eq!(
        scalar(&mut *c, "SELECT count(*) FROM t").await,
        Value::Int(1_000)
    );
    // Closing a streaming query (not suspended): the rest is dropped.
    c.query(QuerySpec {
        chunk_rows: 10,
        ..spec(ENDLESS_ROWS)
    })
    .await
    .unwrap();
    assert!(matches!(
        c.next_event().await.unwrap(),
        QueryEvent::ResultStart(_)
    ));
    let first = rows_of(c.next_event().await.unwrap()).rows();
    assert_eq!(
        c.close_query(CloseHow::Now).await.unwrap(),
        Closed::Closed {
            rows: u64::from(first)
        }
    );
    assert!(c.ping().await.is_ok());
}

#[tokio::test]
async fn idle_bound_follows_the_journal_mode() {
    let f = numbered(10);
    let mut c = open_rw(&f).await;
    assert_eq!(c.typed_caps().idle_bound, ROLLBACK_IDLE_BOUND);
    let caps = c.typed_caps();
    assert!(caps.suspend && !caps.binds && !caps.close_needs_kill);
    run(&mut *c, spec("PRAGMA journal_mode = WAL"))
        .await
        .unwrap();
    // Read at the start of each typed query.
    run(&mut *c, cursor("SELECT a FROM t")).await.unwrap();
    assert_eq!(c.typed_caps().idle_bound, None);
    drop(c);
    // A WAL file from the open on.
    let mut c = open_rw(&f).await;
    assert_eq!(c.typed_caps().idle_bound, None);
    // A rollback-journal database attached to a WAL one: bounded again.
    let other = f.dir.path().join("other.db");
    rusqlite::Connection::open(&other)
        .unwrap()
        .execute_batch("CREATE TABLE o(x)")
        .unwrap();
    let attach = format!("ATTACH '{}' AS o", other.display());
    run(&mut *c, spec(&attach)).await.unwrap();
    run(&mut *c, cursor("SELECT a FROM t")).await.unwrap();
    assert_eq!(c.typed_caps().idle_bound, ROLLBACK_IDLE_BOUND);
    // An in-memory database holds no file lock.
    let mut req = common::request(std::path::Path::new(":memory:"), false);
    if let hacivat_driver::Endpoint::File {
        create_if_missing, ..
    } = &mut req.endpoint
    {
        *create_if_missing = true;
    }
    let mut c = hacivat_driver_sqlite::SqliteDriver::new()
        .connect(req)
        .await
        .unwrap();
    run(&mut *c, cursor("SELECT 1")).await.unwrap();
    assert_eq!(c.typed_caps().idle_bound, None);
}

#[tokio::test]
async fn simple_protocol_parameters_and_binds_are_unsupported() {
    let f = numbered(1);
    let mut c = open_rw(&f).await;
    let simple_q = QuerySpec {
        protocol: Protocol::Simple,
        ..spec("SELECT 1")
    };
    assert_eq!(
        c.query(simple_q).await.unwrap_err().kind,
        RawErrorKind::Unsupported
    );
    let binds = [BindValue::Null];
    let bound = QuerySpec {
        binds: &binds,
        ..spec("SELECT ?")
    };
    assert_eq!(
        c.query(bound).await.unwrap_err().kind,
        RawErrorKind::Unsupported
    );
    // A parameter without a bind would run as NULL: refused until binds (M3b).
    for sql in ["SELECT ?", "DELETE FROM t WHERE a = :id", "SELECT $x"] {
        assert_eq!(
            c.query(spec(sql)).await.unwrap_err().kind,
            RawErrorKind::Unsupported,
            "{sql}"
        );
    }
    assert_eq!(
        scalar(&mut *c, "SELECT count(*) FROM t").await,
        Value::Int(1)
    );
    // Two statements are one error, as on the M2b path.
    let e = c.query(spec("SELECT 1; SELECT 2")).await.unwrap_err();
    assert_eq!(text_of(&e), "multiple-statements");
}

/// Perf (release, `HACIVAT_PERF=1`): the first chunk of a million-row file within 20 ms of
/// `query`, median of 10.
#[tokio::test]
#[ignore = "perf"]
async fn first_chunk_of_a_million_rows_within_20_ms_release() {
    if std::env::var_os("HACIVAT_PERF").is_none_or(|v| v != "1") {
        return;
    }
    let f = numbered(1_000_000);
    let mut c = open_rw(&f).await;
    let mut times = Vec::new();
    for _ in 0..10 {
        let start = Instant::now();
        c.query(cursor("SELECT a, b FROM t")).await.unwrap();
        assert!(matches!(
            c.next_event().await.unwrap(),
            QueryEvent::ResultStart(_)
        ));
        rows_of(c.next_event().await.unwrap());
        times.push(start.elapsed());
        c.close_query(CloseHow::Now).await.unwrap();
    }
    times.sort();
    let median = times[times.len() / 2];
    println!("first chunk of a million rows: median {median:?} of 10");
    assert!(median <= Duration::from_millis(20), "median {median:?}");
}
