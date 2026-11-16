//! Eviction under global memory pressure (HV-RES-07 eviction, R-M3-68, P-M3b-29; Task 8): the
//! budget closes background results oldest-hidden first with `Closed { reason: Evicted }`,
//! each keeping its first chunk; a visible result, a result still being written and a result
//! with nothing past its first chunk are never touched; a held snapshot keeps what it showed,
//! and every byte comes back to the budget.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_results::{
    Admission, CloseReason, Completeness, CursorInfo, FetchControl, FetchPolicy, MemoryBudget,
    PauseReason, ResultId, ResultSource, ResultStore, ResultWriter, StoreSource, TruncReason,
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

/// `rows` rows numbered from `from`.
fn chunk_from(from: u64, rows: u64) -> Chunk {
    Chunk::from_values(
        &schema(),
        (from..from + rows)
            .map(|r| vec![Value::Int(r as i64)])
            .collect(),
    )
}

/// The rows of every chunk [`fill`] appends.
const CHUNK_ROWS: u64 = 1_024;

/// Appends chunks of [`CHUNK_ROWS`] rows until the result holds at least `bytes`.
fn fill(w: &mut ResultWriter, bytes: u64) {
    let mut held = 0;
    let mut rows = 0;
    while held < bytes {
        let c = chunk_from(rows, CHUNK_ROWS);
        held += c.bytes();
        rows += CHUNK_ROWS;
        w.append(c);
    }
}

/// Records nothing the tests read; eviction never talks to the session.
struct NoControl;

impl FetchControl for NoControl {
    fn fetch_more(&self, _result: ResultId, _rows: u64) {}
    fn close(&self, _result: ResultId, _reason: CloseReason) {}
    fn fetch_all(&self, _result: ResultId) {}
}

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_id() -> ResultId {
    ResultId(NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}

/// A finished result of at least `bytes` on `budget`, shown in its console (`visible`) or in
/// the background, with its writer gone (the run ended).
fn store_filled(budget: &Arc<MemoryBudget>, bytes: u64, visible: bool) -> Arc<StoreSource> {
    let (store, mut w) = ResultStore::new(next_id(), schema(), budget.clone(), 1);
    fill(&mut w, bytes);
    w.set_state(Completeness::Complete);
    drop(w);
    let source = StoreSource::new(store, Arc::new(NoControl), FetchPolicy::default());
    source.set_visible(visible);
    source
}

fn closed(source: &Arc<StoreSource>) -> bool {
    matches!(
        source.snapshot().completeness(),
        Completeness::Closed { .. }
    )
}

fn closed_reason(source: &Arc<StoreSource>) -> Option<CloseReason> {
    match source.snapshot().completeness() {
        Completeness::Closed { reason } => Some(*reason),
        _ => None,
    }
}

fn int_at(source: &Arc<StoreSource>, row: u64) -> i64 {
    match source.snapshot().cell(row, 0) {
        hacivat_values::ValueRef::Int(n) => n,
        other => panic!("not an int: {other:?}"),
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
// The plan's tests (Task 8, Step 1)

#[test]
fn eviction_skips_visible_results() {
    let b = MemoryBudget::new(4 << 20, 4 << 20);
    let (a, b2) = (
        store_filled(&b, 2 << 20, true),
        store_filled(&b, 2 << 20, false),
    );
    assert!(b.evict_for(1 << 20) >= 1 << 20);
    assert!(!closed(&a));
    assert_eq!(closed_reason(&b2), Some(CloseReason::Evicted));
}

#[test]
fn eviction_keeps_chunk_zero() {
    let b = MemoryBudget::new(4 << 20, 4 << 20);
    let hidden = store_filled(&b, 1 << 20, false);
    let before = hidden.snapshot();
    let first = chunk_from(0, CHUNK_ROWS).bytes();
    let used = b.used();
    drop(before);
    let released = b.evict_for(1);
    let after = hidden.snapshot();
    assert_eq!(after.row_count(), CHUNK_ROWS, "the first chunk stays");
    assert_eq!(after.bytes(), first);
    assert_eq!(int_at(&hidden, 0), 0);
    assert_eq!(int_at(&hidden, CHUNK_ROWS - 1), (CHUNK_ROWS - 1) as i64);
    assert_eq!(
        released,
        used - first,
        "every chunk past the first is released"
    );
    assert_eq!(b.used(), first);
    assert_eq!(after.paused(), None);
}

#[test]
fn oldest_hidden_result_goes_first() {
    let b = MemoryBudget::new(64 << 20, 64 << 20);
    let x = store_filled(&b, 1 << 20, true);
    let y = store_filled(&b, 1 << 20, true);
    let z = store_filled(&b, 1 << 20, true);
    // Hidden in the order z, x, y: z has been out of sight the longest.
    z.set_visible(false);
    x.set_visible(false);
    y.set_visible(false);
    assert!(b.evict_for(1) > 0);
    assert_eq!(closed_reason(&z), Some(CloseReason::Evicted));
    assert!(!closed(&x) && !closed(&y), "one result was enough");
    // The next pressure takes the next oldest.
    assert!(b.evict_for(1) > 0);
    assert_eq!(closed_reason(&x), Some(CloseReason::Evicted));
    assert!(!closed(&y));
    // A result shown again and hidden later becomes the youngest.
    let w = store_filled(&b, 1 << 20, true);
    y.set_visible(true);
    w.set_visible(false);
    y.set_visible(false);
    assert!(b.evict_for(1) > 0);
    assert_eq!(closed_reason(&w), Some(CloseReason::Evicted));
    assert!(!closed(&y));
}

#[test]
fn evicted_snapshot_still_reads_chunk_zero() {
    let b = MemoryBudget::new(8 << 20, 8 << 20);
    let hidden = store_filled(&b, 1 << 20, false);
    b.evict_for(1);
    let snap = hidden.snapshot();
    assert_eq!(
        *snap.completeness(),
        Completeness::Closed {
            reason: CloseReason::Evicted
        }
    );
    for row in [0, 1, CHUNK_ROWS / 2, CHUNK_ROWS - 1] {
        assert_eq!(int_at(&hidden, row), row as i64);
    }
    // Nothing more is offered: no cursor, no fetch.
    assert!(!hidden.caps().has_cursor);
}

#[test]
fn eviction_with_a_held_snapshot_releases_on_drop() {
    let b = MemoryBudget::new(8 << 20, 8 << 20);
    let hidden = store_filled(&b, 1 << 20, false);
    let held = hidden.snapshot();
    let rows = held.row_count();
    let used = b.used();
    let first = chunk_from(0, CHUNK_ROWS).bytes();
    // The held snapshot keeps the chunks: nothing is released yet.
    assert_eq!(b.evict_for(1 << 20), 0);
    assert_eq!(b.used(), used);
    assert_eq!(closed_reason(&hidden), Some(CloseReason::Evicted));
    // What it showed is what it still shows (a copy from it never changes).
    assert_eq!(held.row_count(), rows);
    assert_eq!(*held.completeness(), Completeness::Complete);
    assert_eq!(
        held.cell(rows - 1, 0),
        hacivat_values::ValueRef::Int((rows - 1) as i64)
    );
    drop(held);
    assert_eq!(b.used(), first, "the held chunks are given back on drop");
}

#[test]
fn admit_evicts_before_pausing() {
    let b = MemoryBudget::new(4 << 20, 4 << 20);
    let hidden = store_filled(&b, 3 << 20, false);
    let (_running, mut w) = ResultStore::new(next_id(), schema(), b.clone(), 1);
    fill(&mut w, 512 << 10);
    // The next chunk would pass the global budget: the hidden result makes room.
    assert_eq!(w.admit(1 << 20), Admission::Ok);
    assert_eq!(closed_reason(&hidden), Some(CloseReason::Evicted));
    assert!(b.used() + (1 << 20) <= b.global());
    w.set_state(Completeness::Complete);
}

// ---------------------------------------------------------------------------------------------
// What eviction never does

/// Nothing to evict (everything visible): the run pauses for the global budget, and the
/// visible result keeps every row.
#[test]
fn admit_pauses_when_nothing_can_be_evicted() {
    let b = MemoryBudget::new(4 << 20, 4 << 20);
    let shown = store_filled(&b, 3 << 20, true);
    let rows = shown.snapshot().row_count();
    let (_running, w) = ResultStore::new(next_id(), schema(), b.clone(), 1);
    assert_eq!(
        w.admit(2 << 20),
        Admission::Pause(PauseReason::GlobalBudget)
    );
    assert!(!closed(&shown));
    assert_eq!(shown.snapshot().row_count(), rows);
}

/// A result counts as visible until the app says it is in the background: a console that
/// never reports visibility loses nothing.
#[test]
fn a_result_is_visible_until_told_otherwise() {
    let b = MemoryBudget::new(4 << 20, 4 << 20);
    let (store, mut w) = ResultStore::new(next_id(), schema(), b.clone(), 1);
    fill(&mut w, 2 << 20);
    w.set_state(Completeness::Complete);
    drop(w);
    assert_eq!(b.evict_for(1 << 20), 0);
    assert_eq!(*store.snapshot().completeness(), Completeness::Complete);
    store.set_visible(false);
    assert!(b.evict_for(1 << 20) >= 1 << 20);
}

/// A result whose writer still writes (streaming) or holds an open cursor (suspended) is
/// never evicted: its session would keep appending, and only the session closes a cursor.
#[test]
fn running_and_suspended_results_are_never_evicted() {
    let b = MemoryBudget::new(64 << 20, 64 << 20);
    let (streaming, mut ws) = ResultStore::new(next_id(), schema(), b.clone(), 1);
    fill(&mut ws, 1 << 20);
    let (parked, mut wp) = ResultStore::new(next_id(), schema(), b.clone(), 1);
    fill(&mut wp, 1 << 20);
    wp.set_state(suspended());
    streaming.set_visible(false);
    parked.set_visible(false);
    assert_eq!(b.evict_for(u64::MAX), 0);
    assert_eq!(
        *streaming.snapshot().completeness(),
        Completeness::Streaming
    );
    assert!(matches!(
        parked.snapshot().completeness(),
        Completeness::Suspended(_)
    ));
    ws.set_state(Completeness::Complete);
    wp.set_state(Completeness::Closed {
        reason: CloseReason::UserClosed,
    });
}

/// A result with only its first chunk has nothing to give: it stays as it is and never reads
/// as evicted.
#[test]
fn a_one_chunk_result_is_left_alone() {
    let b = MemoryBudget::new(64 << 20, 64 << 20);
    let (store, mut w) = ResultStore::new(next_id(), schema(), b.clone(), 1);
    w.append(chunk_from(0, 10));
    w.set_state(Completeness::Complete);
    store.set_visible(false);
    let before = store.snapshot();
    assert_eq!(b.evict_for(1), 0);
    assert!(
        Arc::ptr_eq(&before, &store.snapshot()),
        "nothing was published"
    );
}

/// An evicted result never reports `Complete` again (or any other state): a late state from
/// its writer is dropped, and so is a late chunk, which is never charged.
#[test]
fn an_evicted_result_stays_evicted() {
    let b = MemoryBudget::new(64 << 20, 64 << 20);
    let (store, mut w) = ResultStore::new(next_id(), schema(), b.clone(), 1);
    fill(&mut w, 1 << 20);
    w.set_state(Completeness::Truncated(TruncReason::FollowingResultSets));
    store.set_visible(false);
    assert!(b.evict_for(1) > 0);
    let evicted = store.snapshot();
    let used = b.used();
    w.set_state(Completeness::Complete);
    w.append(chunk_from(0, 100));
    drop(w);
    assert!(Arc::ptr_eq(&evicted, &store.snapshot()));
    assert_eq!(
        *store.snapshot().completeness(),
        Completeness::Closed {
            reason: CloseReason::Evicted
        }
    );
    assert_eq!(b.used(), used);
    // A second pressure finds nothing more to take from it.
    assert_eq!(b.evict_for(1), 0);
}

/// Eviction publishes like the writer does: a new version, the `watch` told, the same
/// generation.
#[test]
fn eviction_publishes_a_new_version() {
    let b = MemoryBudget::new(64 << 20, 64 << 20);
    let hidden = store_filled(&b, 1 << 20, false);
    let mut changes = hidden.changes();
    let before = hidden.snapshot();
    b.evict_for(1);
    let after = hidden.snapshot();
    assert_eq!(after.version(), before.version() + 1);
    assert_eq!(after.generation(), before.generation());
    assert!(changes.has_changed().unwrap());
    assert_eq!(
        *changes.borrow_and_update(),
        (after.generation(), after.version())
    );
}

/// Accounting returns every byte: evicted, held, and finally dropped.
#[test]
fn accounting_returns_every_byte() {
    let b = MemoryBudget::new(64 << 20, 64 << 20);
    let x = store_filled(&b, 1 << 20, false);
    let y = store_filled(&b, 2 << 20, false);
    let held = y.snapshot();
    let z = store_filled(&b, 1 << 20, true);
    let used = b.used();
    let released = b.evict_for(u64::MAX);
    assert_eq!(b.used(), used - released);
    drop(held);
    let firsts = 2 * chunk_from(0, CHUNK_ROWS).bytes();
    assert_eq!(b.used(), firsts + z.snapshot().bytes());
    drop((x, y, z));
    assert_eq!(b.used(), 0);
}

/// A closed tab's store is gone: eviction neither keeps it alive nor trips over it.
#[test]
fn a_dropped_result_is_skipped() {
    let b = MemoryBudget::new(64 << 20, 64 << 20);
    let gone = store_filled(&b, 1 << 20, false);
    drop(gone);
    assert_eq!(b.used(), 0);
    let kept = store_filled(&b, 1 << 20, false);
    assert!(b.evict_for(1) > 0);
    assert_eq!(closed_reason(&kept), Some(CloseReason::Evicted));
}

/// Writers appending and admitting on many threads while eviction runs: no deadlock, no
/// visible result touched, and the books balance at the end.
#[test]
fn eviction_races_writers_safely() {
    let b = MemoryBudget::new(6 << 20, 64 << 20);
    let shown = store_filled(&b, 1 << 20, true);
    let shown_rows = shown.snapshot().row_count();
    let backgrounds: Vec<_> = (0..4).map(|_| store_filled(&b, 1 << 20, false)).collect();
    let writers: Vec<_> = (0..4)
        .map(|i| {
            let b = b.clone();
            std::thread::spawn(move || {
                // Each writer wants 2 MiB: 8 MiB together on a 6 MiB budget that already
                // holds 5 MiB, so the background results must give way.
                let (s, mut w) = ResultStore::new(next_id(), schema(), b, 1);
                let (mut held, mut n) = (0, 0);
                while held < 2 << 20 {
                    let c = chunk_from(n * 256, 256);
                    if w.admit(c.bytes()) != Admission::Ok {
                        break;
                    }
                    held += c.bytes();
                    n += 1;
                    w.append(c);
                }
                w.set_state(Completeness::Complete);
                if i % 2 == 0 {
                    s.set_visible(false);
                }
                s
            })
        })
        .collect();
    let stores: Vec<Arc<ResultStore>> = writers
        .into_iter()
        .map(|t| t.join().expect("a writer panicked"))
        .collect();
    assert!(!closed(&shown));
    assert_eq!(shown.snapshot().row_count(), shown_rows);
    let held: u64 = stores.iter().map(|s| s.snapshot().bytes()).sum::<u64>()
        + backgrounds
            .iter()
            .map(|s| s.snapshot().bytes())
            .sum::<u64>()
        + shown.snapshot().bytes();
    assert_eq!(b.used(), held);
    let evicted = backgrounds
        .iter()
        .filter(|s| closed_reason(s) == Some(CloseReason::Evicted))
        .count();
    assert!(evicted > 0, "the writers needed room");
    drop((stores, backgrounds, shown));
    assert_eq!(b.used(), 0);
}
