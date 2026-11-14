//! The result store's tests (Task 7): lookups across chunk boundaries, publication on every
//! append and state change, generation versus version, snapshot isolation, and the
//! `from_rows` fixture. The target builds only with `--features test-support`
//! (`ResultStore::from_rows`, P6).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use hacivat_results::{
    CloseReason, Completeness, CursorInfo, MemoryBudget, ResultId, ResultStore, ResultWriter,
    StoreSnapshot,
};
use hacivat_values::{
    CellFlags, Chunk, ColumnMeta, LogicalType, NativeType, ResultSchema, Value, ValueRef,
};

// ---------------------------------------------------------------------------------------------
// Fixtures

/// `n` integer columns named `c0`, `c1`, ….
fn int_schema(n: usize) -> Arc<ResultSchema> {
    let columns: Vec<ColumnMeta> = (0..n)
        .map(|i| ColumnMeta {
            name: format!("c{i}").into(),
            logical: LogicalType::Int {
                bits: 64,
                signed: true,
            },
            native: NativeType::Unknown,
            origin: None,
        })
        .collect();
    Arc::new(ResultSchema {
        columns: columns.into(),
    })
}

/// `n` rows of one column whose values are the global row numbers `start..start + n`.
fn chunk_from(schema: &ResultSchema, start: u64, n: u64) -> Chunk {
    let rows = (start..start + n)
        .map(|r| vec![Value::Int(r as i64)])
        .collect();
    Chunk::from_values(schema, rows)
}

fn new_store() -> (Arc<ResultStore>, ResultWriter, Arc<ResultSchema>) {
    let schema = int_schema(1);
    let (s, w) = ResultStore::new(
        ResultId(1),
        schema.clone(),
        MemoryBudget::new(1 << 30, 1 << 28),
        1,
    );
    (s, w, schema)
}

/// A store whose chunks have `sizes` rows, in order, numbered from 0.
fn store_with_chunks(sizes: &[u64]) -> (Arc<ResultStore>, ResultWriter) {
    let (s, mut w, schema) = new_store();
    let mut next = 0;
    for &n in sizes {
        w.append(chunk_from(&schema, next, n));
        next += n;
    }
    (s, w)
}

fn int_at(s: &StoreSnapshot, row: u64, col: u32) -> i64 {
    match s.cell(row, col) {
        ValueRef::Int(v) => v,
        other => panic!("row {row} col {col}: {other:?}"),
    }
}

fn suspended() -> Completeness {
    let now = Instant::now();
    Completeness::Suspended(CursorInfo {
        opened_at: now,
        last_fetch: now,
        holds_tx: true,
        idle_bound: Duration::from_secs(600),
    })
}

// ---------------------------------------------------------------------------------------------
// Lookups

#[test]
fn lookups_at_every_chunk_boundary() {
    let (s, _w) = store_with_chunks(&[500, 4_096, 1_308]);
    let snap = s.snapshot();
    assert_eq!(snap.row_count(), 5_904);
    for r in [0, 499, 500, 4_595, 4_596, 5_903] {
        assert_eq!(int_at(&snap, r, 0), r as i64, "row {r}");
    }
    // And every row in between: no off-by-one anywhere in the chunk starts.
    for r in 0..snap.row_count() {
        assert_eq!(int_at(&snap, r, 0), r as i64, "row {r}");
    }
}

#[test]
fn a_single_row_chunk_between_large_ones_is_found() {
    let (s, _w) = store_with_chunks(&[4_096, 1, 4_096, 1]);
    let snap = s.snapshot();
    assert_eq!(snap.row_count(), 8_194);
    for r in [4_095, 4_096, 4_097, 8_192, 8_193] {
        assert_eq!(int_at(&snap, r, 0), r as i64, "row {r}");
    }
}

#[test]
fn an_empty_chunk_adds_no_rows_and_lookups_stay_exact() {
    let (s, mut w, schema) = new_store();
    w.append(chunk_from(&schema, 0, 10));
    w.append(chunk_from(&schema, 10, 0));
    w.append(chunk_from(&schema, 10, 5));
    let snap = s.snapshot();
    assert_eq!(snap.row_count(), 15);
    for r in 0..15 {
        assert_eq!(int_at(&snap, r, 0), r as i64, "row {r}");
    }
}

#[test]
fn every_column_of_a_wide_chunk_is_read() {
    let schema = int_schema(3);
    let (s, mut w) = ResultStore::new(
        ResultId(2),
        schema.clone(),
        MemoryBudget::new(1 << 30, 1 << 28),
        1,
    );
    for start in [0_i64, 600] {
        let rows = (start..start + 600)
            .map(|r| vec![Value::Int(r), Value::Int(-r), Value::Int(r * 10)])
            .collect();
        w.append(Chunk::from_values(&schema, rows));
    }
    let snap = s.snapshot();
    for r in [0_u64, 599, 600, 1_199] {
        let r_i = r as i64;
        assert_eq!(
            (
                int_at(&snap, r, 0),
                int_at(&snap, r, 1),
                int_at(&snap, r, 2)
            ),
            (r_i, -r_i, r_i * 10)
        );
    }
}

#[test]
fn flags_follow_the_cell() {
    let (s, mut w, schema) = new_store();
    w.append(chunk_from(&schema, 0, 3));
    let rows = vec![
        vec![Value::Lossy(Arc::from(&[0x66_u8, 0xff][..]))],
        vec![Value::Null],
    ];
    w.append(Chunk::from_values(&schema, rows));
    let snap = s.snapshot();
    assert_eq!(snap.flags(1, 0), CellFlags::empty());
    assert_eq!(snap.flags(3, 0), CellFlags::LOSSY);
    assert!(matches!(snap.cell(3, 0), ValueRef::Lossy(b) if b == [0x66, 0xff]));
    assert_eq!(snap.cell(4, 0), ValueRef::Null);
}

#[test]
#[should_panic(expected = "row 10 outside a result of 10 rows")]
fn a_row_past_the_end_panics_with_a_message() {
    let (s, _w) = store_with_chunks(&[10]);
    let _ = s.snapshot().cell(10, 0);
}

#[test]
#[should_panic(expected = "row 0 outside a result of 0 rows")]
fn an_empty_result_has_no_cells() {
    let (s, _w, _) = new_store();
    let _ = s.snapshot().flags(0, 0);
}

// ---------------------------------------------------------------------------------------------
// Publication

#[test]
fn every_append_publishes() {
    let (s, mut w, schema) = new_store();
    let mut changes = s.changes();
    assert!(!changes.has_changed().unwrap());
    w.append(chunk_from(&schema, 0, 500));
    assert_eq!(s.snapshot().row_count(), 500);
    assert!(changes.has_changed().unwrap());
    assert_eq!(*changes.borrow_and_update(), (1, 1));
    w.append(chunk_from(&schema, 500, 10));
    assert_eq!(s.snapshot().row_count(), 510);
    assert_eq!(*changes.borrow_and_update(), (1, 2));
}

#[test]
fn every_state_change_publishes_and_keeps_the_rows() {
    let (s, mut w) = store_with_chunks(&[100]);
    let mut changes = s.changes();
    w.set_state(suspended());
    let snap = s.snapshot();
    assert!(matches!(snap.completeness(), Completeness::Suspended(_)));
    assert_eq!((snap.row_count(), snap.version()), (100, 2));
    assert_eq!(*changes.borrow_and_update(), (1, 2));
    w.set_state(Completeness::Closed {
        reason: CloseReason::UserClosed,
    });
    let snap = s.snapshot();
    assert_eq!(
        *snap.completeness(),
        Completeness::Closed {
            reason: CloseReason::UserClosed
        }
    );
    assert_eq!((snap.row_count(), int_at(&snap, 99, 0)), (100, 99));
    assert_eq!(*changes.borrow_and_update(), (1, 3));
}

#[test]
fn a_new_store_publishes_an_empty_streaming_snapshot() {
    let (s, w, schema) = new_store();
    let snap = s.snapshot();
    assert_eq!(w.id(), ResultId(1));
    assert_eq!(s.id(), ResultId(1));
    assert_eq!(*snap.completeness(), Completeness::Streaming);
    assert_eq!(
        (
            snap.generation(),
            snap.version(),
            snap.row_count(),
            snap.bytes()
        ),
        (1, 0, 0, 0)
    );
    assert!(Arc::ptr_eq(snap.schema(), &schema));
    assert_eq!(*s.changes().borrow(), (1, 0));
}

#[test]
fn generation_and_version_are_separate() {
    let (first, mut w1, schema) = new_store();
    w1.append(chunk_from(&schema, 0, 10));
    w1.append(chunk_from(&schema, 10, 10));
    w1.set_state(Completeness::Complete);
    let s1 = first.snapshot();
    // Appends and state changes bump the version only.
    assert_eq!((s1.generation(), s1.version()), (1, 3));

    // A re-run into the same tab is a new store one generation on; its version restarts.
    let (second, mut w2) = ResultStore::new(
        ResultId(3),
        schema.clone(),
        MemoryBudget::new(1 << 30, 1 << 28),
        s1.generation() + 1,
    );
    let s2 = second.snapshot();
    assert_eq!((s2.generation(), s2.version(), s2.row_count()), (2, 0, 0));
    w2.append(chunk_from(&schema, 0, 5));
    let s2 = second.snapshot();
    assert_eq!((s2.generation(), s2.version(), s2.row_count()), (2, 1, 5));
    assert_eq!(*second.changes().borrow(), (2, 1));
    // The old generation is untouched.
    assert_eq!((s1.row_count(), first.snapshot().version()), (20, 3));
}

#[test]
fn an_old_snapshot_never_sees_new_rows() {
    let (s, mut w, schema) = new_store();
    w.append(chunk_from(&schema, 0, 10));
    let old = s.snapshot();
    w.append(chunk_from(&schema, 10, 10));
    w.set_state(Completeness::Complete);
    assert_eq!(old.row_count(), 10);
    assert_eq!(old.version(), 1);
    assert_eq!(*old.completeness(), Completeness::Streaming);
    assert_eq!(int_at(&old, 9, 0), 9);
    assert_eq!(s.snapshot().row_count(), 20);
}

#[test]
fn a_held_snapshot_keeps_its_chunks_after_the_store_is_gone() {
    let (s, mut w, schema) = new_store();
    w.append(chunk_from(&schema, 0, 4_096));
    w.append(chunk_from(&schema, 4_096, 4));
    let held = s.snapshot();
    drop(w);
    drop(s);
    assert_eq!(held.row_count(), 4_100);
    assert_eq!(int_at(&held, 4_099, 0), 4_099);
    assert_eq!(int_at(&held, 0, 0), 0);
}

#[test]
fn bytes_are_the_sum_of_the_chunks() {
    let (s, mut w, schema) = new_store();
    let a = chunk_from(&schema, 0, 500);
    let b = chunk_from(&schema, 500, 77);
    let expected = a.bytes() + b.bytes();
    w.append(a);
    w.append(b);
    assert_eq!(s.snapshot().bytes(), expected);
}

/// A reader on another thread that wakes on `changes()` always finds a snapshot at least as
/// new as the change it woke for, and every snapshot it loads is whole: its last row is
/// readable and holds the value its position implies (chunk starts and chunks agree).
#[test]
fn a_concurrent_reader_sees_only_whole_snapshots() {
    const CHUNKS: u64 = 300;
    const ROWS: u64 = 37;
    let (s, mut w, schema) = new_store();
    let reader_store = s.clone();
    let mut changes = s.changes();
    let reader = std::thread::spawn(move || {
        let mut last_version = 0;
        loop {
            futures::executor::block_on(changes.changed()).expect("the store is alive");
            let (_, notified) = *changes.borrow_and_update();
            let snap = reader_store.snapshot();
            assert!(
                snap.version() >= notified,
                "{} < {notified}",
                snap.version()
            );
            assert!(snap.version() >= last_version);
            last_version = snap.version();
            let n = snap.row_count();
            assert_eq!(n % ROWS, 0, "a snapshot with a partial chunk: {n} rows");
            if n > 0 {
                assert_eq!(int_at(&snap, n - 1, 0), (n - 1) as i64);
                assert_eq!(int_at(&snap, n - ROWS, 0), (n - ROWS) as i64);
            }
            if *snap.completeness() == Completeness::Complete {
                return n;
            }
        }
    });
    for i in 0..CHUNKS {
        w.append(chunk_from(&schema, i * ROWS, ROWS));
    }
    w.set_state(Completeness::Complete);
    assert_eq!(reader.join().expect("the reader panicked"), CHUNKS * ROWS);
}

/// Readers on several threads load snapshots while the writer appends; none of them blocks
/// the writer (the read path takes no lock), and each sees row counts that only grow.
#[test]
fn readers_on_many_threads_never_see_rows_go_backwards() {
    let (s, mut w, schema) = new_store();
    let done = Arc::new(AtomicBool::new(false));
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let s = s.clone();
            let done = done.clone();
            std::thread::spawn(move || {
                let mut last = 0;
                let mut loads = 0_u64;
                while !done.load(Ordering::Acquire) || loads == 0 {
                    let snap = s.snapshot();
                    assert!(snap.row_count() >= last);
                    last = snap.row_count();
                    if last > 0 {
                        assert_eq!(int_at(&snap, last - 1, 0), (last - 1) as i64);
                    }
                    loads += 1;
                }
                last
            })
        })
        .collect();
    for i in 0..200 {
        w.append(chunk_from(&schema, i * 50, 50));
    }
    done.store(true, Ordering::Release);
    for r in readers {
        assert!(r.join().expect("a reader panicked") <= 10_000);
    }
    assert_eq!(s.snapshot().row_count(), 10_000);
}

// ---------------------------------------------------------------------------------------------
// The `from_rows` fixture (R-M3-15)

fn schema2() -> Arc<ResultSchema> {
    let text = ColumnMeta {
        name: "name".into(),
        logical: LogicalType::Text {
            max_chars: None,
            fixed: false,
        },
        native: NativeType::Unknown,
        origin: None,
    };
    let mut columns = int_schema(1).columns.to_vec();
    columns.push(text);
    Arc::new(ResultSchema {
        columns: columns.into(),
    })
}

fn rows2(n: i64) -> Vec<Vec<Value>> {
    (0..n)
        .map(|i| vec![Value::Int(i), Value::Text(format!("row {i}").into())])
        .collect()
}

#[test]
fn from_rows_fixture_is_complete() {
    let s = ResultStore::from_rows(schema2(), rows2(3), Completeness::Complete);
    let snap = s.snapshot();
    assert_eq!(snap.row_count(), 3);
    assert_eq!(*snap.completeness(), Completeness::Complete);
    assert_eq!(snap.cell(2, 0), ValueRef::Int(2));
    assert_eq!(snap.cell(2, 1), ValueRef::Text("row 2"));
    assert_eq!(snap.schema().columns.len(), 2);
}

#[test]
fn from_rows_keeps_every_row_across_chunks_and_the_given_state() {
    let s = ResultStore::from_rows(schema2(), rows2(10_000), suspended());
    let snap = s.snapshot();
    assert_eq!(snap.row_count(), 10_000);
    assert!(matches!(snap.completeness(), Completeness::Suspended(_)));
    for r in [0_u64, 499, 500, 4_595, 4_596, 9_999] {
        assert_eq!(snap.cell(r, 0), ValueRef::Int(r as i64), "row {r}");
        let text = format!("row {r}");
        assert_eq!(snap.cell(r, 1), ValueRef::Text(&text), "row {r}");
    }
}

#[test]
fn from_rows_with_no_rows_is_a_header_only_result() {
    let s = ResultStore::from_rows(schema2(), Vec::new(), Completeness::Complete);
    let snap = s.snapshot();
    assert_eq!((snap.row_count(), snap.bytes()), (0, 0));
    assert_eq!(*snap.completeness(), Completeness::Complete);
    assert_eq!(snap.schema().columns.len(), 2);
}

#[test]
fn from_rows_fixtures_get_distinct_ids() {
    let a = ResultStore::from_rows(schema2(), rows2(1), Completeness::Complete);
    let b = ResultStore::from_rows(schema2(), rows2(1), Completeness::Complete);
    assert_ne!(a.id(), b.id());
}
