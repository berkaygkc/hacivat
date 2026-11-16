//! The store's read budget (HV-RES-02: `cell()` ≤ 30 ns; Task 7). Perf tests are
//! `#[ignore = "perf"]` and run only with `HACIVAT_PERF=1`, in `--release`, through the gate's
//! perf line (P9). `hacivat-results` has no `hacivat-testkit` dependency, so the check on
//! `HACIVAT_PERF` is inlined (the same rule as `hacivat_testkit::perf_enabled`).

use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use hacivat_results::{FetchPolicy, MemoryBudget, ResultId, ResultStore};
use hacivat_values::{Chunk, ColumnMeta, LogicalType, NativeType, ResultSchema, Value, ValueRef};

fn perf_enabled() -> bool {
    std::env::var("HACIVAT_PERF").as_deref() == Ok("1")
}

/// A grid frame reads a viewport: this many rows of every column, from a random first row.
const FRAME_ROWS: u64 = 50;

/// 10,000,000 rows × 2 columns, streamed as the session does (500 rows first, then 4,096 per
/// chunk: 2,442 chunks), and the p50 over 2,000 random viewport frames of the time per cell.
/// A frame's cells come from one snapshot, as the grid's do.
#[test]
#[ignore = "perf"]
fn ten_million_rows_cell_p50_under_30_ns() {
    if !perf_enabled() {
        return;
    }
    const ROWS: u64 = 10_000_000;
    let int = || ColumnMeta {
        name: "n".into(),
        logical: LogicalType::Int {
            bits: 64,
            signed: true,
        },
        native: NativeType::Unknown,
        origin: None,
    };
    let schema = Arc::new(ResultSchema {
        columns: vec![int(), int()].into(),
    });
    let (store, mut w) = ResultStore::new(
        ResultId(1),
        schema.clone(),
        MemoryBudget::new(u64::MAX, u64::MAX),
        1,
    );
    let policy = FetchPolicy::default();
    let mut next = 0_u64;
    let mut size = u64::from(policy.first_rows);
    while next < ROWS {
        let n = size.min(ROWS - next);
        let rows = (next..next + n)
            .map(|r| vec![Value::Int(r as i64), Value::Int(-(r as i64))])
            .collect();
        w.append(Chunk::from_values(&schema, rows));
        next += n;
        size = u64::from(policy.chunk_rows);
    }
    let snap = store.snapshot();
    assert_eq!(snap.row_count(), ROWS);

    // xorshift64: frame starts without a random-number crate.
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut starts = Vec::with_capacity(2_000);
    for _ in 0..2_000 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        starts.push(state % (ROWS - FRAME_ROWS));
    }
    let cols = snap.schema().columns.len() as u32;
    let frame = |start: u64| {
        let mut sum = 0_i64;
        for row in start..start + FRAME_ROWS {
            for col in 0..cols {
                if let ValueRef::Int(v) = snap.cell(black_box(row), black_box(col)) {
                    sum = sum.wrapping_add(v);
                }
            }
        }
        black_box(sum)
    };
    // One warm pass, then the measured one.
    for &s in &starts {
        frame(s);
    }
    let cells = (FRAME_ROWS * u64::from(cols)) as f64;
    let mut per_cell: Vec<f64> = starts
        .iter()
        .map(|&s| {
            let t = Instant::now();
            frame(s);
            t.elapsed().as_nanos() as f64 / cells
        })
        .collect();
    per_cell.sort_by(f64::total_cmp);
    let p50 = per_cell[per_cell.len() / 2];
    let p99 = per_cell[per_cell.len() * 99 / 100];
    eprintln!("cell(): p50 {p50:.1} ns, p99 {p99:.1} ns per cell over 10M rows");
    assert!(p50 < 30.0, "cell() p50 {p50:.1} ns (p99 {p99:.1} ns)");
}

/// Publishing stays O(chunks) and cheap (alg/21: publish ≤ 50 µs): each of the 2,442 appends
/// of a 10M-row stream is a publish, and their p99 stays under 50 µs (the p99, so one
/// preemption of a loaded machine does not decide it).
#[test]
#[ignore = "perf"]
fn publish_p99_of_a_ten_million_row_stream_is_under_50_us() {
    if !perf_enabled() {
        return;
    }
    let schema = Arc::new(ResultSchema {
        columns: Vec::new().into(),
    });
    let (store, mut w) = ResultStore::new(
        ResultId(2),
        schema.clone(),
        MemoryBudget::new(u64::MAX, u64::MAX),
        1,
    );
    // Zero-column chunks: the publish cost, not the decode cost, is measured.
    let chunk = || Chunk::from_values(&schema, vec![Vec::new(); 4_096]);
    let mut took: Vec<u128> = (0..2_442)
        .map(|_| {
            let c = chunk();
            let t = Instant::now();
            w.append(c);
            t.elapsed().as_nanos()
        })
        .collect();
    assert_eq!(store.snapshot().row_count(), 2_442 * 4_096);
    took.sort_unstable();
    let (p50, p99, last) = (
        took[took.len() / 2],
        took[took.len() * 99 / 100],
        took[took.len() - 1],
    );
    eprintln!("append + publish over 2,442 chunks: p50 {p50} ns, p99 {p99} ns, max {last} ns");
    assert!(p99 < 50_000, "publish p99 {p99} ns (p50 {p50} ns)");
}
