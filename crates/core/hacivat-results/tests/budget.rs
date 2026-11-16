//! The memory budget (HV-RES-07 v1, R-M3-19, R-M3-33; Task 7): admission against the
//! per-result cap and the global budget, "Fetch more anyway", exact accounting across
//! concurrent results, and the bytes a held snapshot keeps. No eviction in M3a.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_results::{
    Admission, CloseReason, Completeness, CursorInfo, MemoryBudget, PauseReason, ResultId,
    ResultStore, ResultWriter,
};
use hacivat_values::{Chunk, ColumnMeta, LogicalType, NativeType, ResultSchema, Value};

// ---------------------------------------------------------------------------------------------
// Fixtures

fn schema() -> Arc<ResultSchema> {
    Arc::new(ResultSchema {
        columns: vec![ColumnMeta {
            name: "n".into(),
            logical: LogicalType::Int {
                bits: 64,
                signed: true,
            },
            native: NativeType::Unknown,
            origin: None,
        }]
        .into(),
    })
}

fn chunk(rows: u64) -> Chunk {
    Chunk::from_values(
        &schema(),
        (0..rows).map(|r| vec![Value::Int(r as i64)]).collect(),
    )
}

/// The bytes one row of [`chunk`] takes (the value layout is Task 6's, so it is measured).
fn row_bytes() -> u64 {
    chunk(1).bytes()
}

fn store_with_budget(budget: Arc<MemoryBudget>) -> (Arc<ResultStore>, ResultWriter) {
    ResultStore::new(ResultId(1), schema(), budget, 1)
}

/// Appends chunks of 1,024 rows until the result holds at least `bytes`; returns what it holds.
fn fill(w: &mut ResultWriter, bytes: u64) -> u64 {
    let per_chunk = chunk(1_024).bytes();
    let mut held = 0;
    while held < bytes {
        let rows = ((bytes - held) / row_bytes()).clamp(1, 1_024);
        let c = chunk(rows);
        held += c.bytes();
        w.append(c);
    }
    assert!(held >= bytes && held < bytes + per_chunk);
    held
}

fn global_of(b: Arc<MemoryBudget>) -> u64 {
    b.global()
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
// Admission

#[test]
fn per_result_cap_pauses_then_raise_lifts_it() {
    let (s, mut w) = store_with_budget(MemoryBudget::new(1 << 30, 1 << 20));
    fill(&mut w, 1 << 20);
    assert_eq!(w.admit(4_096), Admission::Pause(PauseReason::ResultCap));
    assert!(!s.cap_raised());
    s.raise_cap_to_global();
    assert!(s.cap_raised());
    assert_eq!(w.admit(4_096), Admission::Ok);
}

#[test]
fn admission_is_exact_at_the_cap() {
    let (_s, mut w) = store_with_budget(MemoryBudget::new(1 << 30, 1 << 20));
    let held = fill(&mut w, 1 << 19);
    let room = (1 << 20) - held;
    // A chunk that lands exactly on the cap still fits; one byte more does not.
    assert_eq!(w.admit(room), Admission::Ok);
    assert_eq!(w.admit(room + 1), Admission::Pause(PauseReason::ResultCap));
    assert_eq!(w.admit(u64::MAX), Admission::Pause(PauseReason::ResultCap));
}

#[test]
fn a_fresh_result_is_admitted() {
    let (_s, w) = store_with_budget(MemoryBudget::from_ram(Some(16 << 30)));
    assert_eq!(w.admit(4 << 20), Admission::Ok);
}

#[test]
fn global_budget_pauses_across_results() {
    let budget = MemoryBudget::new(1 << 20, 1 << 20);
    let (_a, mut wa) = ResultStore::new(ResultId(1), schema(), budget.clone(), 1);
    let (_b, mut wb) = ResultStore::new(ResultId(2), schema(), budget.clone(), 1);
    let held_a = fill(&mut wa, 3 << 18);
    // B holds little of its own, but A and B together reach the global budget.
    assert_eq!(wb.admit(4_096), Admission::Ok);
    let held_b = fill(&mut wb, (1 << 20) - held_a);
    assert_eq!(budget.used(), held_a + held_b);
    assert!(held_b < 1 << 19);
    assert_eq!(wb.admit(4_096), Admission::Pause(PauseReason::GlobalBudget));
    // A is under its own cap too, so it pauses for the global budget, not its cap.
    assert_eq!(wa.admit(4_096), Admission::Pause(PauseReason::GlobalBudget));
}

#[test]
fn a_raised_cap_still_stops_at_the_global_budget() {
    let budget = MemoryBudget::new(1 << 20, 1 << 18);
    let (s, mut w) = store_with_budget(budget.clone());
    let first = fill(&mut w, 1 << 18);
    assert_eq!(w.admit(4_096), Admission::Pause(PauseReason::ResultCap));
    s.raise_cap_to_global();
    // Under the raised cap the result grows to the whole global budget.
    assert_eq!(w.admit(4_096), Admission::Ok);
    let rest = fill(&mut w, (1 << 20) - first);
    assert_eq!(budget.used(), first + rest);
    assert_eq!(s.snapshot().bytes(), first + rest);
    // Past the global budget the reason is the global budget, never the raised cap.
    assert_eq!(w.admit(4_096), Admission::Pause(PauseReason::GlobalBudget));
}

#[test]
fn raising_one_results_cap_leaves_the_others() {
    let budget = MemoryBudget::new(1 << 30, 1 << 18);
    let (a, mut wa) = ResultStore::new(ResultId(1), schema(), budget.clone(), 1);
    let (_b, mut wb) = ResultStore::new(ResultId(2), schema(), budget.clone(), 1);
    fill(&mut wa, 1 << 18);
    fill(&mut wb, 1 << 18);
    a.raise_cap_to_global();
    assert_eq!(wa.admit(4_096), Admission::Ok);
    assert_eq!(wb.admit(4_096), Admission::Pause(PauseReason::ResultCap));
}

#[test]
fn from_ram_takes_a_quarter_capped_at_4_gib() {
    assert_eq!(global_of(MemoryBudget::from_ram(Some(8 << 30))), 2 << 30);
    assert_eq!(global_of(MemoryBudget::from_ram(Some(64 << 30))), 4 << 30);
    assert_eq!(global_of(MemoryBudget::from_ram(None)), 1 << 30);
    assert_eq!(
        MemoryBudget::from_ram(Some(8 << 30)).per_result(),
        256 << 20
    );
}

// ---------------------------------------------------------------------------------------------
// The memory banner's signal (R-M3-33)

#[test]
fn a_paused_suspended_result_says_why() {
    let (s, mut w) = store_with_budget(MemoryBudget::new(1 << 30, 1 << 18));
    fill(&mut w, 1 << 18);
    assert_eq!(s.snapshot().paused(), None);
    assert_eq!(w.admit(4_096), Admission::Pause(PauseReason::ResultCap));
    // The actor suspends the cursor; that publish carries the reason for the banner.
    w.set_state(suspended());
    assert_eq!(s.snapshot().paused(), Some(PauseReason::ResultCap));
    // "Fetch more anyway": the cap rises, the next admission passes, the stream resumes.
    s.raise_cap_to_global();
    assert_eq!(w.admit(4_096), Admission::Ok);
    w.set_state(Completeness::Streaming);
    assert_eq!(s.snapshot().paused(), None);
}

#[test]
fn a_closed_result_shows_no_banner() {
    let (s, mut w) = store_with_budget(MemoryBudget::new(1 << 30, 1 << 18));
    fill(&mut w, 1 << 18);
    assert_eq!(w.admit(4_096), Admission::Pause(PauseReason::ResultCap));
    w.set_state(suspended());
    assert_eq!(s.snapshot().paused(), Some(PauseReason::ResultCap));
    // "Close cursor" from the banner.
    w.set_state(Completeness::Closed {
        reason: CloseReason::MemoryLimit,
    });
    assert_eq!(s.snapshot().paused(), None);
}

#[test]
fn a_global_pause_is_reported_as_global() {
    let budget = MemoryBudget::new(1 << 19, 1 << 20);
    let (s, mut w) = store_with_budget(budget);
    fill(&mut w, 1 << 19);
    assert_eq!(w.admit(4_096), Admission::Pause(PauseReason::GlobalBudget));
    w.set_state(suspended());
    assert_eq!(s.snapshot().paused(), Some(PauseReason::GlobalBudget));
}

// ---------------------------------------------------------------------------------------------
// Accounting

#[test]
fn appends_charge_the_budget_and_dropping_the_result_returns_it() {
    let budget = MemoryBudget::new(1 << 30, 1 << 28);
    let (s, mut w) = store_with_budget(budget.clone());
    let a = chunk(500);
    let b = chunk(4_096);
    let a_bytes = a.bytes();
    let expected = a_bytes + b.bytes();
    assert!(a_bytes > 0);
    w.append(a);
    assert_eq!(budget.used(), a_bytes);
    w.append(b);
    assert_eq!(budget.used(), expected);
    assert_eq!(s.snapshot().bytes(), expected);
    drop(w);
    assert_eq!(budget.used(), expected, "the store still holds the rows");
    drop(s);
    assert_eq!(budget.used(), 0);
}

#[test]
fn a_held_snapshot_keeps_its_bytes_charged() {
    let budget = MemoryBudget::new(1 << 30, 1 << 28);
    let (s, mut w) = store_with_budget(budget.clone());
    w.append(chunk(1_000));
    let held = s.snapshot();
    drop(w);
    drop(s);
    assert_eq!(budget.used(), held.bytes());
    drop(held);
    assert_eq!(budget.used(), 0);
}

#[test]
fn concurrent_results_account_exactly() {
    const THREADS: u64 = 8;
    const CHUNKS: u64 = 40;
    let budget = MemoryBudget::new(u64::MAX, u64::MAX);
    // Every writer starts before any is joined, so their charges interleave.
    let writers: Vec<_> = (0..THREADS)
        .map(|i| {
            let budget = budget.clone();
            std::thread::spawn(move || {
                let (s, mut w) = ResultStore::new(ResultId(i), schema(), budget, 1);
                for n in 0..CHUNKS {
                    assert_eq!(w.admit(1 << 16), Admission::Ok);
                    w.append(chunk(1 + (n + i) % 97));
                }
                s
            })
        })
        .collect();
    let stores: Vec<Arc<ResultStore>> = writers
        .into_iter()
        .map(|t| t.join().expect("a writer panicked"))
        .collect();
    let sum: u64 = stores.iter().map(|s| s.snapshot().bytes()).sum();
    let rows: u64 = stores.iter().map(|s| s.snapshot().row_count()).sum();
    let sizes = || (0..THREADS).flat_map(|i| (0..CHUNKS).map(move |n| 1 + (n + i) % 97));
    assert_eq!(rows, sizes().sum::<u64>());
    assert_eq!(budget.used(), sum);
    assert_eq!(sum, sizes().map(|n| chunk(n).bytes()).sum::<u64>());
    drop(stores);
    assert_eq!(budget.used(), 0);
}
