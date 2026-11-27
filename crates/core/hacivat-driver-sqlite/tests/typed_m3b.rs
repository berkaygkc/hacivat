//! M3b Task 13 (HV-EXEC-03 driver half; R-M3-60, R-M3-66, R-M3-79) on SQLite, in process:
//! binds by hint, a placeholder without a bind refused before any step (the M3a "would run as
//! NULL" gap closed), a hostile value bound and never spliced, and Run as one (`Batch`) with
//! its result-set cap.

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

mod common;
mod typed_support;

use std::sync::Arc;

use hacivat_driver::{Protocol, QueryEvent, QuerySpec, RawConn, RawError, RawErrorKind};
use hacivat_values::{BindValue, LogicalType, Value};
use typed_support::*;

// ## m3b-t13-drivers

/// A database with an empty table `t(a INTEGER, b TEXT)`, not yet opened.
struct Sqlite {
    f: Fixture,
}

fn sqlite() -> Sqlite {
    Sqlite {
        f: fixture("CREATE TABLE t(a INTEGER, b TEXT);"),
    }
}

fn typed(hint: LogicalType, text: &str) -> BindValue {
    BindValue::Typed {
        hint,
        text: Arc::from(text),
    }
}

fn text(s: &str) -> BindValue {
    typed(
        LogicalType::Text {
            max_chars: None,
            fixed: false,
        },
        s,
    )
}

fn int(s: &str) -> BindValue {
    typed(
        LogicalType::Int {
            bits: 64,
            signed: true,
        },
        s,
    )
}

fn bound<'a>(sql: &'a str, binds: &'a [BindValue]) -> QuerySpec<'a> {
    QuerySpec { binds, ..spec(sql) }
}

fn as_one(sql: &str, max_result_sets: u32) -> QuerySpec<'_> {
    QuerySpec {
        protocol: Protocol::Simple,
        max_result_sets,
        ..spec(sql)
    }
}

/// The error `query` answers for `sql` with `binds` on a fresh read-write connection; the
/// table is then checked untouched.
async fn query_err(db: Sqlite, sql: &str, binds: &[BindValue]) -> RawError {
    let mut c = open_rw(&db.f).await;
    let e = match c.query(bound(sql, binds)).await {
        Ok(()) => panic!("{sql}: ran"),
        Err(e) => e,
    };
    assert_eq!(
        scalar(&mut *c, "SELECT count(*) FROM t").await,
        Value::Int(0),
        "{sql}: nothing ran"
    );
    e
}

/// The events of one query to `Done`, `Rows` as their row counts.
async fn events(c: &mut dyn RawConn, spec: QuerySpec<'_>) -> Result<Vec<String>, RawError> {
    c.query(spec).await?;
    let mut out = Vec::new();
    loop {
        let e = c.next_event().await?;
        out.push(match &e {
            QueryEvent::ResultStart(s) => format!("start {}", s.columns.len()),
            QueryEvent::Rows(ch) => format!("rows {}", ch.rows()),
            QueryEvent::ResultEnd(end) => format!(
                "end {} {:?} {}",
                end.rows,
                end.rows_affected,
                end.command_tag.as_deref().unwrap_or("-")
            ),
            QueryEvent::Done(d) => format!("done {}", d.skipped_result_sets),
            other => panic!("unexpected {other:?}"),
        });
        if matches!(e, QueryEvent::Done(_)) {
            return Ok(out);
        }
    }
}

#[tokio::test]
async fn sqlite_unbound_placeholder_is_refused_not_null() {
    assert_eq!(
        query_err(sqlite(), "SELECT ?", &[]).await.kind,
        RawErrorKind::Unsupported
    );
    // A write with a missing bind never runs with NULL in its place, nor with an extra one.
    for (sql, binds) in [
        ("INSERT INTO t VALUES (?, ?)", vec![int("1")]),
        ("INSERT INTO t VALUES (:a, :b)", vec![]),
        ("INSERT INTO t VALUES (?1, 'x')", vec![int("1"), int("2")]),
        ("INSERT INTO t VALUES (1, 'x')", vec![int("1")]),
    ] {
        assert_eq!(
            query_err(sqlite(), sql, &binds).await.kind,
            RawErrorKind::Unsupported,
            "{sql}"
        );
    }
}

#[tokio::test]
async fn sqlite_binds_by_hint() {
    let db = sqlite();
    let mut c = open_rw(&db.f).await;
    assert!(c.typed_caps().binds);
    let binds = [
        int("42"),
        typed(LogicalType::Float { bits: 64 }, "2.5"),
        text("x"),
        typed(LogicalType::Bytes { max: None }, "\\x00ff"),
        BindValue::Null,
        // Not an integer: kept as the text the user typed, never guessed.
        int("4x"),
        typed(LogicalType::Bool, "TRUE"),
        typed(LogicalType::Float { bits: 64 }, "NaN"),
    ];
    let sql = "SELECT typeof(?1), typeof(?2), typeof(?3), typeof(?4), typeof(?5), typeof(?6), \
               typeof(?7), typeof(?8), ?1, ?2, ?3, ?4, ?6, ?7";
    let r = run(&mut *c, bound(sql, &binds)).await.unwrap();
    let types: Vec<Value> = (0..8).map(|i| r.cell(0, i)).collect();
    let want: Vec<Value> = [
        "integer", "real", "text", "blob", "null", "text", "integer", "text",
    ]
    .iter()
    .map(|t| Value::Text(Arc::from(*t)))
    .collect();
    assert_eq!(types, want);
    assert_eq!(r.cell(0, 8), Value::Int(42));
    assert_eq!(r.cell(0, 9), Value::F64(2.5));
    assert_eq!(r.cell(0, 10), Value::Text(Arc::from("x")));
    assert_eq!(r.cell(0, 11), Value::Bytes(Arc::from(&[0x00_u8, 0xff][..])));
    assert_eq!(r.cell(0, 12), Value::Text(Arc::from("4x")));
    assert_eq!(r.cell(0, 13), Value::Int(1));
    // Into a table, by position.
    run(
        &mut *c,
        bound("INSERT INTO t VALUES (?, ?)", &[int("7"), text("seven")]),
    )
    .await
    .unwrap();
    let r = run(&mut *c, spec("SELECT a, b FROM t")).await.unwrap();
    assert_eq!(r.cell(0, 0), Value::Int(7));
    assert_eq!(r.cell(0, 1), Value::Text(Arc::from("seven")));
}

#[tokio::test]
async fn value_with_quote_is_bound_not_spliced() {
    let hostile = "x'); DROP TABLE t; --";
    let db = sqlite();
    let mut c = open_rw(&db.f).await;
    let end = run(
        &mut *c,
        bound("INSERT INTO t(b) VALUES (?)", &[text(hostile)]),
    )
    .await
    .unwrap()
    .end;
    assert_eq!(end.rows_affected, Some(1));
    let r = run(&mut *c, spec("SELECT b FROM t")).await.unwrap();
    assert_eq!(r.rows(), 1);
    assert_eq!(r.cell(0, 0), Value::Text(Arc::from(hostile)));
    // The table still exists, and so does every other one.
    assert_eq!(
        scalar(
            &mut *c,
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 't'"
        )
        .await,
        Value::Int(1)
    );
    // In a WHERE clause the value matches only itself.
    let r = run(
        &mut *c,
        bound(
            "SELECT count(*) FROM t WHERE b = ?",
            &[text("x' OR '1'='1")],
        ),
    )
    .await
    .unwrap();
    assert_eq!(r.cell(0, 0), Value::Int(0));
}

#[tokio::test]
async fn run_as_one_gives_one_result_per_statement() {
    let db = sqlite();
    let mut c = open_rw(&db.f).await;
    let got = events(
        &mut *c,
        as_one(
            "INSERT INTO t VALUES (1, 'a'), (2, 'b'); SELECT a FROM t ORDER BY a; \
             SELECT a, b FROM t WHERE a = 2;",
            100,
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        got,
        [
            "start 0",
            "end 0 Some(2) INSERT",
            "start 1",
            "rows 2",
            "end 2 None SELECT",
            "start 2",
            "rows 1",
            "end 1 None SELECT",
            "done 0",
        ]
    );
}

#[tokio::test]
async fn run_as_one_counts_the_sets_past_the_cap_and_still_runs_them() {
    let db = sqlite();
    let mut c = open_rw(&db.f).await;
    let got = events(
        &mut *c,
        as_one(
            "SELECT 1; SELECT 2; INSERT INTO t VALUES (3, 'c'); SELECT 4;",
            2,
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        got,
        [
            "start 1",
            "rows 1",
            "end 1 None SELECT",
            "start 1",
            "rows 1",
            "end 1 None SELECT",
            "done 2",
        ]
    );
    // The statement past the cap ran: counted, never dropped.
    assert_eq!(
        scalar(&mut *c, "SELECT count(*) FROM t").await,
        Value::Int(1)
    );
}

#[tokio::test]
async fn run_as_one_takes_no_binds_and_stops_at_a_later_error() {
    let db = sqlite();
    let mut c = open_rw(&db.f).await;
    let binds = [int("1")];
    let spec_with_binds = QuerySpec {
        binds: &binds,
        ..as_one("SELECT ?", 100)
    };
    assert_eq!(
        c.query(spec_with_binds).await.unwrap_err().kind,
        RawErrorKind::Unsupported
    );
    // A placeholder inside Run as one never runs as NULL.
    assert_eq!(
        c.query(as_one("SELECT 1; INSERT INTO t VALUES (?, 'p')", 100))
            .await
            .map(|()| "started"),
        Ok("started")
    );
    let mut seen = Vec::new();
    let e = loop {
        match c.next_event().await {
            Ok(QueryEvent::Done(_)) => panic!("the placeholder statement ran"),
            Ok(e) => seen.push(e),
            Err(e) => break e,
        }
    };
    assert_eq!(e.kind, RawErrorKind::Unsupported);
    assert!(matches!(seen.last(), Some(QueryEvent::ResultEnd(_))));
    assert_eq!(
        scalar(&mut *c, "SELECT count(*) FROM t").await,
        Value::Int(0)
    );
}

#[tokio::test]
async fn run_as_one_on_read_only_refuses_a_write_before_it_runs() {
    let f = fixture("CREATE TABLE t(a INTEGER, b TEXT); INSERT INTO t VALUES (1, 'a');");
    let mut c = open_ro(&f).await;
    c.query(as_one("SELECT a FROM t; DELETE FROM t;", 100))
        .await
        .unwrap();
    let e = loop {
        match c.next_event().await {
            Ok(QueryEvent::Done(_)) => panic!("the DELETE ran on a read-only connection"),
            Ok(_) => {}
            Err(e) => break e,
        }
    };
    // The authorizer's refusal at the DELETE's prepare (`SQLITE_AUTH`).
    assert_eq!(e.kind, RawErrorKind::Db);
    assert_eq!(code(&e) & 0xff, 23, "{}", text_of(&e));
    drop(c);
    let check = rusqlite::Connection::open(&f.path).unwrap();
    let n: i64 = check
        .query_row("SELECT count(*) FROM t", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}

// ## m3b-t13-drivers (fix round 1)

#[test]
fn bytes_hints_read_backslash_x_hex_and_refuse_it_malformed() {
    use hacivat_driver_sqlite::typed::{HexText, hex_bytes};
    assert_eq!(hex_bytes("\\x00ff"), HexText::Bytes(vec![0x00, 0xff]));
    assert_eq!(hex_bytes("\\xABcd"), HexText::Bytes(vec![0xab, 0xcd]));
    assert_eq!(hex_bytes("\\x"), HexText::Bytes(vec![]));
    for bad in ["\\x0", "\\x0g", "\\xzz", "\\X00", "\\x 0", "\\x00f"] {
        assert_eq!(hex_bytes(bad), HexText::Malformed, "{bad:?}");
    }
    for plain in ["", "abc", "x00", "\\\\x00", "0x00"] {
        assert_eq!(hex_bytes(plain), HexText::NotHex, "{plain:?}");
    }
}

#[test]
fn only_a_canonical_integer_text_binds_as_an_integer() {
    use hacivat_driver_sqlite::typed::sqlite_value;
    use rusqlite::types::Value as V;
    for (text, want) in [
        ("0", V::Integer(0)),
        ("-5", V::Integer(-5)),
        ("9223372036854775807", V::Integer(i64::MAX)),
        ("-9223372036854775808", V::Integer(i64::MIN)),
        ("+5", V::Text("+5".into())),
        ("007", V::Text("007".into())),
        ("-0", V::Text("-0".into())),
        (" 5", V::Text(" 5".into())),
        ("１", V::Text("１".into())),
        ("9223372036854775808", V::Text("9223372036854775808".into())),
    ] {
        assert_eq!(sqlite_value(&int(text)), Some(want), "{text:?}");
    }
}

#[tokio::test]
async fn bytes_binds_store_the_same_bytes_as_the_other_engines_or_are_refused() {
    let db = sqlite();
    let mut c = open_rw(&db.f).await;
    let bytes = |s: &str| typed(LogicalType::Bytes { max: None }, s);
    let r = run(
        &mut *c,
        bound(
            "SELECT ?1, ?2, typeof(?2)",
            &[bytes("\\x00ff"), bytes("abc")],
        ),
    )
    .await
    .unwrap();
    assert_eq!(r.cell(0, 0), Value::Bytes(Arc::from(&[0x00_u8, 0xff][..])));
    assert_eq!(r.cell(0, 1), Value::Bytes(Arc::from(&b"abc"[..])));
    assert_eq!(r.cell(0, 2), Value::Text(Arc::from("blob")));
    // Malformed `\x`: refused before any step, nothing stored.
    for bad in ["\\x0", "\\xzz", "\\X00"] {
        assert_eq!(
            query_err(sqlite(), "INSERT INTO t(b) VALUES (?)", &[bytes(bad)])
                .await
                .kind,
            RawErrorKind::Unsupported,
            "{bad:?}"
        );
    }
}

#[tokio::test]
async fn run_as_one_on_read_only_refuses_what_writes_past_the_authorizer() {
    // `VACUUM` asks the authorizer nothing; `sqlite3_stmt_readonly` refuses it per statement.
    let f = fixture("CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1);");
    let mut c = open_ro(&f).await;
    c.query(as_one("SELECT a FROM t; VACUUM;", 100))
        .await
        .unwrap();
    let mut ends = 0;
    let e = loop {
        match c.next_event().await {
            Ok(QueryEvent::Done(_)) => panic!("VACUUM ran on a read-only connection"),
            Ok(QueryEvent::ResultEnd(_)) => ends += 1,
            Ok(_) => {}
            Err(e) => break e,
        }
    };
    assert_eq!(ends, 1, "the SELECT's result came first");
    assert_eq!((code(&e), text_of(&e).as_str()), (23, "readonly-required"));
}
