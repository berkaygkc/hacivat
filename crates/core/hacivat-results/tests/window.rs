//! The fetch window and the grid's result source (HV-RES-03, HV-RES-05; Task 7):
//! `ensure_rows` sends one `fetch_more` per snapshot version while the result is suspended and
//! nothing otherwise; `fetch_all`, `close_cursor` and `raise_cap` forward to the session.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hacivat_results::window::MAX_FETCH_ROWS;
use hacivat_results::{
    Admission, CloseReason, Completeness, CursorInfo, FetchControl, FetchPolicy, MemoryBudget,
    ResultId, ResultSource, ResultStore, ResultWriter, StoreSource, TruncReason,
};
use hacivat_values::{Chunk, ColumnMeta, LogicalType, NativeType, ResultSchema, Value};

// ---------------------------------------------------------------------------------------------
// Fixtures

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Call {
    FetchMore(ResultId, u64),
    FetchAll(ResultId),
    Close(ResultId, CloseReason),
}

/// Records what a source sends; `fetch_all` also records whether the store's cap was already
/// raised when it arrived.
#[derive(Default)]
struct Control {
    calls: Mutex<Vec<Call>>,
    store: Mutex<Option<Arc<ResultStore>>>,
    raised_at_fetch_all: Mutex<Vec<bool>>,
}

impl Control {
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn fetches(&self) -> usize {
        self.calls()
            .iter()
            .filter(|c| matches!(c, Call::FetchMore(..)))
            .count()
    }
}

impl FetchControl for Control {
    fn fetch_more(&self, result: ResultId, rows: u64) {
        self.calls
            .lock()
            .unwrap()
            .push(Call::FetchMore(result, rows));
    }

    fn close(&self, result: ResultId, reason: CloseReason) {
        self.calls.lock().unwrap().push(Call::Close(result, reason));
    }

    fn fetch_all(&self, result: ResultId) {
        if let Some(store) = self.store.lock().unwrap().as_ref() {
            self.raised_at_fetch_all
                .lock()
                .unwrap()
                .push(store.cap_raised());
        }
        self.calls.lock().unwrap().push(Call::FetchAll(result));
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

struct Fixture {
    src: Arc<StoreSource>,
    store: Arc<ResultStore>,
    writer: ResultWriter,
    ctl: Arc<Control>,
}

/// A source over a store of `rows` rows in state `c`, with the default policy.
fn source_with(rows: u64, c: Completeness, budget: Arc<MemoryBudget>) -> Fixture {
    let (store, mut writer) = ResultStore::new(ResultId(9), schema(), budget, 1);
    if rows > 0 {
        writer.append(chunk(rows));
    }
    writer.set_state(c);
    let ctl = Arc::new(Control::default());
    *ctl.store.lock().unwrap() = Some(store.clone());
    let src = StoreSource::new(store.clone(), ctl.clone(), FetchPolicy::default());
    Fixture {
        src,
        store,
        writer,
        ctl,
    }
}

fn source_in(c: Completeness) -> Fixture {
    source_with(10_000, c, MemoryBudget::new(1 << 30, 1 << 28))
}

fn suspended_source(rows: u64) -> Fixture {
    source_with(rows, suspended(), MemoryBudget::new(1 << 30, 1 << 28))
}

/// The writer appends `rows` more rows; the result stays suspended.
fn publish_more(f: &mut Fixture, rows: u64) {
    f.writer.append(chunk(rows));
}

fn closed(reason: CloseReason) -> Completeness {
    Completeness::Closed { reason }
}

// ---------------------------------------------------------------------------------------------
// The policy's decision

#[test]
fn nothing_is_asked_while_the_margin_is_loaded() {
    let p = FetchPolicy::default();
    assert_eq!(p.rows_to_fetch(0, 10_000), None);
    assert_eq!(p.rows_to_fetch(9_000, 10_000), None);
    assert_eq!(p.rows_to_fetch(9_001, 10_000), Some(10_000));
}

#[test]
fn the_ask_is_the_read_ahead_or_the_deficit_whichever_is_larger() {
    let p = FetchPolicy::default();
    assert_eq!(p.rows_to_fetch(9_500, 10_000), Some(10_000));
    assert_eq!(p.rows_to_fetch(100_000, 10_000), Some(91_000));
    let p = FetchPolicy {
        prefetch_margin_rows: 0,
        read_ahead_rows: 1,
        ..FetchPolicy::default()
    };
    assert_eq!(p.rows_to_fetch(10, 10), None);
    assert_eq!(p.rows_to_fetch(10, 9), Some(1));
    assert_eq!(p.rows_to_fetch(20, 9), Some(11));
}

/// A sentinel or an estimated far row never asks for more than `MAX_FETCH_ROWS`, so the
/// session can add up queued asks without overflowing (review M3).
#[test]
fn the_ask_is_clamped_so_queued_asks_can_be_summed() {
    let p = FetchPolicy::default();
    assert_eq!(p.rows_to_fetch(u64::MAX, 0), Some(MAX_FETCH_ROWS));
    assert_eq!(p.rows_to_fetch(u64::MAX, u64::MAX - 1), Some(10_000));
    assert_eq!(
        p.rows_to_fetch(MAX_FETCH_ROWS + 5_000, 0),
        Some(MAX_FETCH_ROWS)
    );
    assert_eq!(
        p.rows_to_fetch(MAX_FETCH_ROWS - 2_000, 0),
        Some(MAX_FETCH_ROWS - 1_000)
    );
    // The bound's promise: 2^32 clamped asks still sum without overflow.
    assert!(MAX_FETCH_ROWS.checked_mul(1 << 32).is_some());
    let f = suspended_source(10);
    f.src.ensure_rows(u64::MAX);
    assert_eq!(
        f.ctl.calls(),
        vec![Call::FetchMore(ResultId(9), MAX_FETCH_ROWS)]
    );
}

// ---------------------------------------------------------------------------------------------
// `StoreSource::ensure_rows`

#[test]
fn ensure_rows_sends_one_fetch_per_version_while_suspended() {
    let mut f = suspended_source(10_000);
    f.src.ensure_rows(9_500);
    f.src.ensure_rows(9_900);
    assert_eq!(f.ctl.fetches(), 1);
    assert_eq!(f.ctl.calls(), vec![Call::FetchMore(ResultId(9), 10_000)]);
    publish_more(&mut f, 4_096);
    f.src.ensure_rows(13_900);
    assert_eq!(f.ctl.fetches(), 2);
    f.src.ensure_rows(13_900);
    assert_eq!(f.ctl.fetches(), 2);
}

#[test]
fn ensure_rows_is_silent_unless_suspended() {
    for c in [
        Completeness::Streaming,
        Completeness::Complete,
        closed(CloseReason::UserClosed),
        closed(CloseReason::ConnectionLost),
        Completeness::Cancelled,
        Completeness::Truncated(TruncReason::ResultCap),
        Completeness::Released,
    ] {
        let f = source_in(c);
        f.src.ensure_rows(u64::MAX);
        assert_eq!(f.ctl.fetches(), 0, "{c:?}");
    }
}

#[test]
fn ensure_rows_is_silent_when_the_rows_are_loaded() {
    let f = suspended_source(10_000);
    f.src.ensure_rows(0);
    f.src.ensure_rows(8_999);
    assert!(f.ctl.calls().is_empty());
}

#[test]
fn a_fetch_that_has_not_landed_is_not_sent_again() {
    let mut f = suspended_source(10_000);
    f.src.ensure_rows(9_999);
    // The session starts fetching: the result streams, and nothing is asked meanwhile.
    f.writer.set_state(Completeness::Streaming);
    f.src.ensure_rows(9_999);
    publish_more(&mut f, 4_096);
    f.src.ensure_rows(14_000);
    assert_eq!(f.ctl.fetches(), 1);
    // The cursor suspends again at a new version: one more fetch is allowed.
    f.writer.set_state(suspended());
    f.src.ensure_rows(14_000);
    f.src.ensure_rows(14_095);
    assert_eq!(f.ctl.fetches(), 2);
}

#[test]
fn ensure_rows_from_many_threads_sends_one_fetch() {
    let f = suspended_source(10_000);
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let src = f.src.clone();
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
    assert_eq!(f.ctl.fetches(), 1);
}

/// At the memory limit the banner is the confirmation (R-M3-33): scrolling to the end asks
/// for nothing, however often the result republishes, until "Fetch more anyway" or a cap
/// raise lets the next admission through.
#[test]
fn ensure_rows_is_silent_while_paused_for_memory() {
    let mut f = source_with(
        0,
        Completeness::Streaming,
        MemoryBudget::new(1 << 30, 1 << 16),
    );
    let c = chunk(1_024);
    let bytes = c.bytes();
    f.writer.append(c);
    assert!(matches!(
        f.writer.admit((1 << 16) - bytes + 1),
        Admission::Pause(_)
    ));
    f.writer.set_state(suspended());
    f.src.ensure_rows(1_023);
    f.writer.set_state(suspended());
    f.src.ensure_rows(1_023);
    assert_eq!(f.ctl.fetches(), 0);
    // The cap rises and the session resumes: the next suspension may ask again.
    f.src.raise_cap();
    assert_eq!(f.writer.admit(4_096), Admission::Ok);
    f.writer.set_state(suspended());
    f.src.ensure_rows(1_023);
    assert_eq!(f.ctl.fetches(), 1);
}

// ---------------------------------------------------------------------------------------------
// Commands and capabilities

#[test]
fn fetch_all_and_close_cursor_forward_to_the_session() {
    let f = suspended_source(10_000);
    f.src.fetch_all();
    f.src.close_cursor();
    assert_eq!(
        f.ctl.calls(),
        vec![
            Call::FetchAll(ResultId(9)),
            Call::Close(ResultId(9), CloseReason::UserClosed),
        ]
    );
    assert!(!f.store.cap_raised());
}

/// "Close cursor" on the memory banner closes with its reason, so the log says the memory
/// limit was reached (`cursor-closed-memory-limit`); anywhere else it is the user's close.
#[test]
fn closing_from_the_memory_banner_says_memory_limit() {
    let mut f = source_with(
        0,
        Completeness::Streaming,
        MemoryBudget::new(1 << 30, 1 << 16),
    );
    let c = chunk(2_048);
    f.writer.append(c);
    assert_eq!(
        f.writer.admit(4_096),
        Admission::Pause(hacivat_results::PauseReason::ResultCap)
    );
    f.writer.set_state(suspended());
    f.src.close_cursor();
    assert_eq!(
        f.ctl.calls(),
        vec![Call::Close(ResultId(9), CloseReason::MemoryLimit)]
    );
}

#[test]
fn raise_cap_lifts_the_cap_before_it_fetches_all() {
    let mut f = source_with(0, suspended(), MemoryBudget::new(1 << 30, 1 << 16));
    let held = chunk(1_024);
    let bytes = held.bytes();
    f.writer.append(held);
    assert_eq!(
        f.writer.admit((1 << 16) - bytes + 1),
        Admission::Pause(hacivat_results::PauseReason::ResultCap)
    );
    f.src.raise_cap();
    assert_eq!(f.ctl.calls(), vec![Call::FetchAll(ResultId(9))]);
    assert_eq!(*f.ctl.raised_at_fetch_all.lock().unwrap(), vec![true]);
    assert_eq!(f.writer.admit(1 << 20), Admission::Ok);
}

#[test]
fn caps_report_a_cursor_only_while_suspended() {
    let mut f = suspended_source(10);
    assert!(f.src.caps().has_cursor);
    assert!(!f.src.caps().editable);
    f.writer.set_state(Completeness::Streaming);
    assert!(!f.src.caps().has_cursor);
    f.writer.set_state(Completeness::Complete);
    assert!(!f.src.caps().has_cursor);
}

/// The writer moves into the session actor's task and the store, its snapshots and the source
/// are read from the UI thread and the background executor.
#[test]
fn the_store_types_cross_threads() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<ResultWriter>();
    send_sync::<ResultStore>();
    send_sync::<hacivat_results::StoreSnapshot>();
    send_sync::<StoreSource>();
    send_sync::<MemoryBudget>();
}

#[test]
fn the_source_reads_the_stores_snapshots_and_changes() {
    let mut f = suspended_source(10);
    let mut changes = f.src.changes();
    publish_more(&mut f, 5);
    assert!(changes.has_changed().unwrap());
    let (g, v) = *changes.borrow_and_update();
    let snap = f.src.snapshot();
    assert_eq!(
        (snap.generation(), snap.version(), snap.row_count()),
        (g, v, 15)
    );
}
