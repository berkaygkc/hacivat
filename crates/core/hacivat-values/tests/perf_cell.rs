//! M3a Task 6 perf: `Chunk::cell` on a 4,096 × 50 chunk of the B1 mix, p50 under 30 ns
//! (alg/21's budget). Runs only with `HACIVAT_PERF=1`, in release, through the gate's perf line
//! (`HACIVAT_PERF=1 cargo test --release -p hacivat-values -- --ignored`).

use std::hint::black_box;
use std::time::Instant;

use hacivat_values::{
    BufKind, Chunk, ChunkBuilder, ColumnMeta, LogicalType, NativeType, ResultSchema, TempUnit,
    VarKind,
};

const ROWS: u32 = 4_096;
const COLS: u32 = 50;

/// `hacivat_testkit::perf_enabled`, inlined (the crate has no testkit dev-dependency).
fn perf_enabled() -> bool {
    std::env::var_os("HACIVAT_PERF").is_some_and(|v| v == "1")
}

/// The B1 mix: int8, text of about 12 characters, numeric(14,2), timestamp; every 7th cell
/// NULL and every 997th an exception.
fn b1_chunk() -> Chunk {
    let kinds = [
        (
            BufKind::I64,
            LogicalType::Int {
                bits: 64,
                signed: true,
            },
        ),
        (
            BufKind::Var(VarKind::Text),
            LogicalType::Text {
                max_chars: None,
                fixed: false,
            },
        ),
        (
            BufKind::Dec64 { scale: 2 },
            LogicalType::Decimal {
                precision: Some(14),
                scale: Some(2),
            },
        ),
        (
            BufKind::Temporal {
                unit: TempUnit::Micros,
            },
            LogicalType::Timestamp {
                precision: 6,
                tz: false,
            },
        ),
    ];
    let plan: Vec<BufKind> = (0..COLS).map(|c| kinds[c as usize % 4].0).collect();
    let schema = ResultSchema {
        columns: (0..COLS)
            .map(|c| ColumnMeta {
                name: format!("c{c}").into(),
                logical: kinds[c as usize % 4].1.clone(),
                native: NativeType::Unknown,
                origin: None,
            })
            .collect(),
    };
    let mut b = ChunkBuilder::with_schema(&plan, &schema, ROWS);
    for r in 0..ROWS {
        for c in 0..COLS {
            let n = i64::from(r) * i64::from(COLS) + i64::from(c);
            if n % 7 == 0 {
                b.push_null(c);
            } else if n % 997 == 0 {
                b.push_value(
                    c,
                    hacivat_values::Value::Special(hacivat_values::SpecialVal::NaN),
                );
            } else {
                match c % 4 {
                    0 => b.push_i64(c, n),
                    1 => b.push_bytes(c, format!("müşteri-{n:04}").as_bytes(), 1 << 20),
                    2 => b.push_dec64(c, n * 100 + 25),
                    _ => b.push_temporal(c, 1_700_000_000_000_000 + n),
                }
            }
        }
        b.end_row();
    }
    b.seal()
}

#[test]
#[ignore = "perf"]
fn cell_lookup_p50_under_30_ns() {
    if !perf_enabled() {
        return;
    }
    let chunk = b1_chunk();
    // A fixed pseudo-random walk over the chunk (xorshift), so caches see a grid-like spread.
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let probes: Vec<(u32, u32)> = (0..4_096)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (
                (state % u64::from(ROWS)) as u32,
                ((state >> 32) % u64::from(COLS)) as u32,
            )
        })
        .collect();
    let mut samples: Vec<f64> = (0..400)
        .map(|_| {
            let start = Instant::now();
            for &(r, c) in &probes {
                black_box(chunk.cell(black_box(r), black_box(c)));
            }
            start.elapsed().as_nanos() as f64 / probes.len() as f64
        })
        .collect();
    samples.sort_by(f64::total_cmp);
    let p50 = samples[samples.len() / 2];
    println!("Chunk::cell p50 {p50:.2} ns over 4,096 × 50 (B1 mix)");
    assert!(p50 < 30.0, "Chunk::cell p50 {p50:.2} ns, budget 30 ns");
}
