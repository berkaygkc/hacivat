//! The result source's M3b duties (Task 8): `cap_raised` follows the store (ledger 253 M7),
//! visibility reaches eviction, asks for more rows coalesce per store (ledger 109 M4: every
//! source of one store shares one ask ledger), and the console's cell cap comes from the
//! `results.max_value_bytes` setting (R-M3-82).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hacivat_results::window::{MAX_CELL_CAP, MIN_CELL_CAP};
use hacivat_results::{
    CloseReason, Completeness, CursorInfo, FetchControl, FetchPolicy, MemoryBudget, ResultId,
    ResultSource, ResultStore, ResultWriter, StoreSource,
};
use hacivat_values::{Chunk, ColumnMeta, LogicalType, NativeType, ResultSchema, Value};

// ---------------------------------------------------------------------------------------------
// Fixtures

/// Records every `fetch_more`.
#[derive(Default)]
struct Control {
    fetches: Mutex<Vec<(ResultId, u64)>>,
    all: Mutex<Vec<ResultId>>,
}

impl Control {
    fn fetches(&self) -> Vec<(ResultId, u64)> {
        self.fetches.lock().unwrap().clone()
    }

    fn asked(&self) -> u64 {
        self.fetches().iter().map(|(_, rows)| rows).sum()
    }
}

impl FetchControl for Control {
    fn fetch_more(&self, result: ResultId, rows: u64) {
        self.fetches.lock().unwrap().push((result, rows));
    }

    fn close(&self, _result: ResultId, _reason: CloseReason) {}

    fn fetch_all(&self, result: ResultId) {
        self.all.lock().unwrap().push(result);
    }
}

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

fn suspended() -> Completeness {
    let now = Instant::now();
    Completeness::Suspended(CursorInfo {
        opened_at: now,
        last_fetch: now,
        holds_tx: true,
        idle_bound: Duration::from_secs(600),
    })
}

/// A suspended store of `rows` rows (default policy: a 1,000-row margin, 10,000 rows read
/// ahead), its writer and a recording control.
fn suspended_store(rows: u64) -> (Arc<ResultStore>, ResultWriter, Arc<Control>) {
    let (store, mut w) = ResultStore::new(
        ResultId(7),
        schema(),
        MemoryBudget::new(1 << 30, 1 << 28),
        1,
    );
    w.append(chunk(rows));
    w.set_state(suspended());
    (store, w, Arc::new(Control::default()))
}

fn source(store: &Arc<ResultStore>, ctl: &Arc<Control>) -> Arc<StoreSource> {
    StoreSource::new(store.clone(), ctl.clone(), FetchPolicy::default())
}

// ---------------------------------------------------------------------------------------------
// The plan's tests (Task 8, Step 1)

#[test]
fn source_cap_raised_follows_the_store() {
    let (store, _w, ctl) = suspended_store(10);
    let src = source(&store, &ctl);
    assert!(!src.cap_raised());
    // "Fetch more anyway" from the source: the cap rises, then the session fetches all.
    src.raise_cap();
    assert!(src.cap_raised());
    assert_eq!(*ctl.all.lock().unwrap(), vec![ResultId(7)]);
    // Raised on the store by another holder: every source over it sees it.
    let (other, _w2, ctl2) = suspended_store(10);
    let src2 = source(&other, &ctl2);
    other.raise_cap_to_global();
    assert!(src2.cap_raised());
    assert_eq!(src2.cap_raised(), other.cap_raised());
}

/// Two asks before the session answers (the snapshot version has not moved): the second asks
/// only for the rows past those already asked, so what the session receives adds up to the
/// farthest need, never twice the same rows; the session's queue then merges the queued
/// `FetchMore`s of one result into one fetch of the sum (`Ctl::coalesce`, M3a Task 18).
#[test]
fn fetch_mores_from_one_source_coalesce() {
    let (store, _w, ctl) = suspended_store(10_000);
    let src = source(&store, &ctl);
    src.ensure_rows(9_500);
    src.ensure_rows(25_000);
    // The first ask: the read-ahead (10,000 rows, through row 20,000). The second: what row
    // 25,000 and its margin need past that, at least a read-ahead (through row 30,000).
    assert_eq!(
        ctl.fetches(),
        vec![(ResultId(7), 10_000), (ResultId(7), 10_000)]
    );
    assert_eq!(ctl.asked(), 20_000);
    // Anything already covered asks nothing more.
    src.ensure_rows(9_999);
    src.ensure_rows(25_000);
    src.ensure_rows(28_999);
    assert_eq!(ctl.asked(), 20_000);
    assert_eq!(ctl.fetches().len(), 2);
}

// ---------------------------------------------------------------------------------------------
// Coalescing per store (ledger 109 M4)

/// Two sources over one store (sibling views, R-M3-12) share the ledger: the same need is
/// asked once, never once per source.
#[test]
fn sibling_sources_over_one_store_ask_once() {
    let (store, _w, ctl) = suspended_store(10_000);
    let a = source(&store, &ctl);
    let b = source(&store, &ctl);
    a.ensure_rows(9_999);
    b.ensure_rows(9_999);
    assert_eq!(ctl.fetches(), vec![(ResultId(7), 10_000)]);
    // A farther need from the sibling asks only the rest.
    b.ensure_rows(29_500);
    assert_eq!(ctl.asked(), 20_500);
}

/// The ledger starts over once the session has answered (a new snapshot version).
#[test]
fn the_ledger_starts_over_after_an_answer() {
    let (store, mut w, ctl) = suspended_store(10_000);
    let src = source(&store, &ctl);
    src.ensure_rows(9_999);
    // The session serves the fetch: streaming, the rows, suspended again.
    w.set_state(Completeness::Streaming);
    src.ensure_rows(19_999);
    w.append(chunk(10_000));
    w.set_state(suspended());
    assert_eq!(ctl.fetches().len(), 1, "nothing is asked while it streams");
    src.ensure_rows(19_500);
    assert_eq!(
        ctl.fetches(),
        vec![(ResultId(7), 10_000), (ResultId(7), 10_000)]
    );
}

/// Many threads, many sources, one need: one ask.
#[test]
fn many_sources_on_many_threads_ask_once() {
    let (store, _w, ctl) = suspended_store(10_000);
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let src = source(&store, &ctl);
            std::thread::spawn(move || {
                for _ in 0..100 {
                    src.ensure_rows(9_999);
                }
            })
        })
        .collect();
    for t in threads {
        t.join().expect("a caller panicked");
    }
    assert_eq!(ctl.fetches(), vec![(ResultId(7), 10_000)]);
}

// ---------------------------------------------------------------------------------------------
// Visibility

/// `set_visible` on the source reaches the budget's eviction: hidden, a finished result may
/// go; shown again, it may not.
#[test]
fn source_visibility_reaches_eviction() {
    let budget = MemoryBudget::new(1 << 30, 1 << 28);
    let (store, mut w) = ResultStore::new(ResultId(8), schema(), budget.clone(), 1);
    w.append(chunk(1_000));
    w.append(chunk(1_000));
    w.set_state(Completeness::Complete);
    let ctl = Arc::new(Control::default());
    let src = source(&store, &ctl);
    src.set_visible(false);
    src.set_visible(true);
    assert_eq!(budget.evict_for(1), 0, "a shown result is never evicted");
    src.set_visible(false);
    assert!(budget.evict_for(1) > 0);
    assert_eq!(
        *src.snapshot().completeness(),
        Completeness::Closed {
            reason: CloseReason::Evicted
        }
    );
    assert_eq!(src.snapshot().row_count(), 1_000);
}

// ---------------------------------------------------------------------------------------------
// The cell cap from `results.max_value_bytes` (R-M3-82)

#[test]
fn the_cell_cap_comes_from_the_value_limit() {
    let p = FetchPolicy::default();
    assert_eq!(p.cell_cap, 1 << 20, "M3a's 1 MiB stays the default");
    assert_eq!(p.with_cell_cap(4 << 20).cell_cap, 4 << 20);
    // "Re-run with a 64 MiB value limit".
    assert_eq!(p.with_cell_cap(MAX_CELL_CAP).cell_cap, 64 << 20);
    // The setting's range (64 KiB ..= 64 MiB), held here too.
    assert_eq!(MIN_CELL_CAP, 64 << 10);
    assert_eq!(p.with_cell_cap(1).cell_cap, MIN_CELL_CAP);
    assert_eq!(p.with_cell_cap(u32::MAX).cell_cap, MAX_CELL_CAP);
    // Nothing else changes.
    assert_eq!(
        FetchPolicy {
            cell_cap: p.cell_cap,
            ..p.with_cell_cap(8 << 20)
        },
        p
    );
}
