//! Hostile database files on the typed path (Task 15, R12, P28): a file is untrusted input.
//! `rusqlite` `expect`s on schema text that is not UTF-8 (`column.rs:181-212`), so column
//! names, declared types and origins are read inside `catch_unwind` on the connection's
//! thread: a valid name is kept when only the declared type is invalid (the column is then
//! `Opaque`), only an invalid name becomes `?`, and the connection survives. Cell text that is
//! not UTF-8 is kept as bytes (`Lossy`), never replaced.

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

mod common;
mod typed_support;

use std::sync::Arc;

use common::simple;
use hacivat_driver_sqlite::decode;
use hacivat_values::{
    BufKind, ChunkBuilder, LenUnit, LogicalType, ResultSchema, TruncatedVal, Value, VarKind,
};
use typed_support::*;

/// One schema row to rewrite: `(old name, name, tbl_name, sql)`, the last three raw bytes.
type SchemaRow<'a> = (&'a str, &'a [u8], &'a [u8], &'a [u8]);

/// Rewrites the schema rows of `fixture(setup)`: each `(name, tbl_name, sql)` replaces the
/// row named `old` (raw bytes, possibly not UTF-8), as only a crafted file can hold them.
fn rewritten(setup: &str, rows: &[SchemaRow<'_>]) -> Fixture {
    let f = fixture(setup);
    let c = rusqlite::Connection::open(&f.path).unwrap();
    c.execute_batch("PRAGMA writable_schema = ON").unwrap();
    for (old, name, tbl, sql) in rows {
        let n = c
            .execute(
                "UPDATE sqlite_schema SET name = CAST(?1 AS TEXT), tbl_name = CAST(?2 AS TEXT),
                 sql = CAST(?3 AS TEXT) WHERE name = ?4",
                rusqlite::params![name, tbl, sql, old],
            )
            .unwrap();
        assert_eq!(n, 1, "{old}");
    }
    c.execute_batch("PRAGMA writable_schema = OFF").unwrap();
    drop(c);
    f
}

/// Table `t(a, b)` with rows `(1, 'x')` and `(2, 'y')`, declared by `create` (raw bytes).
fn hostile_table(create: &[u8]) -> Fixture {
    rewritten(
        "CREATE TABLE t(a INTEGER, b TEXT); INSERT INTO t VALUES (1, 'x'), (2, 'y');",
        &[("t", b"t".as_slice(), b"t".as_slice(), create)],
    )
}

fn hostile_file_with_decltype(decl: &[u8]) -> Fixture {
    hostile_table(&[b"CREATE TABLE t(a ".as_slice(), decl, b", b TEXT)"].concat())
}

fn hostile_file_with_column_name(name: &[u8]) -> Fixture {
    hostile_table(&[b"CREATE TABLE t(".as_slice(), name, b" INTEGER, b TEXT)"].concat())
}

async fn typed_run(f: &Fixture, sql: &str) -> Run {
    let mut c = open_rw(f).await;
    let r = run(&mut *c, spec(sql)).await.unwrap();
    // The connection's thread survived the metadata reads.
    assert!(c.ping().await.is_ok());
    r
}

async fn schema_of(f: &Fixture, sql: &str) -> Arc<ResultSchema> {
    typed_run(f, sql).await.schema
}

#[tokio::test]
async fn non_utf8_declared_type_does_not_panic() {
    let f = hostile_file_with_decltype(&[0xff, 0xfe]);
    let s = schema_of(&f, "SELECT * FROM t").await;
    // The valid name is kept (P28); the declared type cannot be read, so the column is Opaque.
    assert_eq!(&*s.columns[0].name, "a");
    assert!(matches!(s.columns[0].logical, LogicalType::Opaque { .. }));
    assert_eq!(s.columns[0].native, hacivat_values::NativeType::Unknown);
    // The other column is untouched.
    assert_eq!(&*s.columns[1].name, "b");
    assert_eq!(
        s.columns[1].logical,
        LogicalType::Text {
            max_chars: None,
            fixed: false
        }
    );
    // The values of the Opaque column come through whole, of their own class.
    let r = typed_run(&f, "SELECT * FROM t").await;
    assert_eq!(r.column(0), [Value::Int(1), Value::Int(2)]);
    assert_eq!(
        r.column(1),
        [Value::Text("x".into()), Value::Text("y".into())]
    );
}

#[tokio::test]
async fn non_utf8_column_name_becomes_a_question_mark() {
    let f = hostile_file_with_column_name(&[0xff]);
    let s = schema_of(&f, "SELECT * FROM t").await;
    assert_eq!(&*s.columns[0].name, "?");
    assert!(matches!(s.columns[0].logical, LogicalType::Opaque { .. }));
    assert_eq!(&*s.columns[1].name, "b");
    let r = typed_run(&f, "SELECT * FROM t").await;
    assert_eq!(r.column(0), [Value::Int(1), Value::Int(2)]);
}

#[tokio::test]
async fn m2b_column_names_are_guarded_too() {
    let f = hostile_file_with_column_name(&[0xe2, 0x82]);
    let mut c = open_rw(&f).await;
    let s = c.execute("SELECT * FROM t", simple()).await.unwrap();
    assert_eq!(s.columns, ["?", "b"]);
    assert_eq!(s.rows.len(), 2);
    assert!(c.ping().await.is_ok());
}

#[tokio::test]
async fn non_utf8_origin_names_leave_the_origin_out() {
    // A table whose name is not UTF-8, read through a view: names and declared types are
    // valid, the origin's table name is not.
    let f = rewritten(
        "CREATE TABLE x(a INTEGER, b TEXT); INSERT INTO x VALUES (1, 'x');
         CREATE VIEW v AS SELECT a, b FROM x;",
        &[
            (
                "x",
                [0xff].as_slice(),
                [0xff].as_slice(),
                b"CREATE TABLE \xff(a INTEGER, b TEXT)".as_slice(),
            ),
            (
                "v",
                b"v".as_slice(),
                b"v".as_slice(),
                b"CREATE VIEW v AS SELECT a, b FROM \xff".as_slice(),
            ),
        ],
    );
    let r = typed_run(&f, "SELECT a, b FROM v").await;
    assert_eq!(r.names(), ["a", "b"]);
    assert_eq!(
        r.schema.columns[0].logical,
        LogicalType::Int {
            bits: 64,
            signed: true
        }
    );
    assert_eq!(r.schema.columns[0].origin, None);
    assert_eq!(r.column(0), [Value::Int(1)]);
}

#[tokio::test]
async fn a_hostile_file_never_stops_the_connection() {
    let f = hostile_file_with_column_name(&[0xff, 0xff, 0xff]);
    let mut c = open_rw(&f).await;
    for _ in 0..3 {
        let r = run(&mut *c, spec("SELECT * FROM t")).await.unwrap();
        assert_eq!(r.names(), ["?", "b"]);
        let r = run(&mut *c, cursor("SELECT b, * FROM t")).await.unwrap();
        assert_eq!(r.names(), ["b", "?", "b"]);
    }
    assert_eq!(
        scalar(&mut *c, "SELECT count(*) FROM t").await,
        Value::Int(2)
    );
}

/// A deterministic byte generator (xorshift), so the cases are the same on every run.
struct Bytes(u64);

impl Bytes {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn take(&mut self, n: usize) -> Vec<u8> {
        // Mostly ASCII with some lead and continuation bytes, so both valid and invalid UTF-8
        // come up, with sequences cut at every position.
        (0..n)
            .map(|_| match self.next() % 8 {
                0 => 0xc3,
                1 => 0x9f,
                2 => 0xe2,
                3 => 0xff,
                _ => b'a' + (self.next() % 26) as u8,
            })
            .collect()
    }
}

#[tokio::test]
async fn hostile_cells_are_kept_exactly_and_never_panic() {
    let f = fixture("CREATE TABLE t(s TEXT, b BLOB, n);");
    let w = rusqlite::Connection::open(&f.path).unwrap();
    let mut rng = Bytes(0x5eed_1234_abcd_0001);
    let mut cases = Vec::new();
    for i in 0..300 {
        let len = (rng.next() % 40) as usize;
        let bytes = rng.take(len);
        w.execute(
            "INSERT INTO t VALUES (CAST(?1 AS TEXT), ?1, CASE ?2 % 3 WHEN 0 THEN CAST(?1 AS TEXT) WHEN 1 THEN ?1 ELSE ?2 END)",
            rusqlite::params![bytes, i],
        )
        .unwrap();
        cases.push((i, bytes));
    }
    drop(w);
    let mut c = open_rw(&f).await;
    let cap = 16;
    let r = run(&mut *c, small_chunks(cap, "SELECT s, b, n FROM t"))
        .await
        .unwrap();
    assert_eq!(r.rows(), 300);
    for (row, (i, bytes)) in cases.iter().enumerate() {
        let row = row as u64;
        let text = ChunkBuilder::capped_value(VarKind::Text, bytes, cap);
        let blob = ChunkBuilder::capped_value(VarKind::Bytes, bytes, cap);
        assert_eq!(r.cell(row, 0), text, "text {bytes:?}");
        assert_eq!(r.cell(row, 1), blob, "blob {bytes:?}");
        let mixed = match i % 3 {
            0 => text,
            1 => blob,
            _ => Value::Int(*i),
        };
        assert_eq!(r.cell(row, 2), mixed, "mixed {i}");
        // Never replaced: valid text is the bytes, invalid text keeps the bytes.
        match r.cell(row, 0) {
            Value::Text(s) => assert_eq!(s.as_bytes(), &bytes[..]),
            Value::Lossy(b) => assert_eq!(&*b, &bytes[..]),
            Value::Truncated(t) => assert_eq!(t.total_len, Some(bytes.len() as u64)),
            other => panic!("{other:?}"),
        }
    }
}

/// Small chunks (first 5 rows, then 7) under a cell cap of `cap` bytes.
fn small_chunks(cap: u32, sql: &str) -> hacivat_driver::QuerySpec<'_> {
    hacivat_driver::QuerySpec {
        cell_cap: cap,
        chunk_rows: 7,
        first_rows: 5,
        ..spec(sql)
    }
}

#[test]
fn the_decoders_take_any_value_without_panicking() {
    use rusqlite::types::ValueRef as Sql;
    let mut rng = Bytes(0x0dd_ba11);
    let kinds = [
        decode::ColKind::Int,
        decode::ColKind::Float,
        decode::ColKind::Text,
        decode::ColKind::Bytes,
        decode::ColKind::Bool,
        decode::ColKind::Opaque,
    ];
    for kind in kinds {
        let plan = [kind.buf()];
        let mut b = ChunkBuilder::new(&plan, 64);
        let mut want = Vec::new();
        for i in 0..200i64 {
            let len = (rng.next() % 12) as usize;
            let bytes = rng.take(len);
            let v = match i % 5 {
                0 => Sql::Null,
                1 => Sql::Integer(i % 3 - 1),
                2 => Sql::Real(i as f64 / 3.0),
                3 => Sql::Text(&bytes),
                _ => Sql::Blob(&bytes),
            };
            decode::push(&mut b, 0, kind, v, 8);
            b.end_row();
            want.push(match (kind, v) {
                (decode::ColKind::Bool, Sql::Integer(n @ (0 | 1))) => Value::Bool(n == 1),
                _ => decode::value_of(v, 8),
            });
        }
        let chunk = b.seal();
        for (row, w) in want.iter().enumerate() {
            assert_eq!(
                &chunk.cell(row as u32, 0).to_value(),
                w,
                "{kind:?} row {row}"
            );
        }
    }
    // A value past the cap keeps its total length.
    assert_eq!(
        decode::value_of(Sql::Blob(&[7; 10]), 4),
        Value::Truncated(Box::new(TruncatedVal {
            prefix: Value::Bytes([7; 4].into()),
            total_len: Some(10),
            unit: LenUnit::Bytes,
        }))
    );
    assert_eq!(decode::ColKind::Opaque.buf(), BufKind::Values);
}
