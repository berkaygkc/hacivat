//! M3b Task 16: the auditor chain (R-M3-63): synchronous auditors in order, then `try_send` to
//! the sinks; a full sink counts drops and never waits; `RowsFetched` reaches the sinks at
//! most once per 50 ms per result.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hacivat_exec::{AuditorChain, SINK_CAPACITY, SyncAuditor};
use hacivat_results::ResultId;
use hacivat_session::session::{RunEvent, RunId, RunSummary, UnitId};
use tokio::sync::mpsc;

struct Seen {
    name: &'static str,
    order: Arc<Mutex<Vec<&'static str>>>,
}

impl SyncAuditor for Seen {
    fn on_event(&mut self, _ev: &RunEvent) {
        self.order.lock().unwrap().push(self.name);
    }
}

struct Count(Arc<AtomicU64>);

impl SyncAuditor for Count {
    fn on_event(&mut self, _ev: &RunEvent) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

fn rows(result: u64, total: u64) -> RunEvent {
    RunEvent::RowsFetched {
        result: ResultId(result),
        total,
    }
}

fn started(ordinal: u32) -> RunEvent {
    RunEvent::UnitStarted {
        unit: UnitId {
            run: RunId(1),
            ordinal,
        },
        at: Instant::now(),
    }
}

#[tokio::test(start_paused = true)]
async fn auditors_run_in_order_then_sinks_and_rows_are_coalesced() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let mut chain = AuditorChain::new();
    for name in ["timing", "tx", "cancel", "gutter"] {
        chain.push_sync(Box::new(Seen {
            name,
            order: order.clone(),
        }));
    }
    let (tx, mut rx) = mpsc::channel::<String>(SINK_CAPACITY);
    chain.push_sink(tx, |ev| Some(format!("{ev:?}").chars().take(12).collect()));
    chain.on_event(&started(0));
    assert_eq!(*order.lock().unwrap(), ["timing", "tx", "cancel", "gutter"]);
    assert!(rx.try_recv().is_ok());
    // Within 50 ms only the first RowsFetched of a result reaches the sinks; every auditor
    // still sees each one.
    for total in 1..=5 {
        chain.on_event(&rows(9, total));
    }
    chain.on_event(&rows(10, 1));
    let mut got = 0;
    while rx.try_recv().is_ok() {
        got += 1;
    }
    assert_eq!(got, 2, "one per result");
    assert_eq!(order.lock().unwrap().len(), 4 * 7);
    tokio::time::advance(Duration::from_millis(50)).await;
    chain.on_event(&rows(9, 6));
    assert!(rx.try_recv().is_ok(), "50 ms later the next one goes");
    assert_eq!(chain.dropped(), 0);
}

#[tokio::test]
async fn a_full_sink_counts_drops_and_never_waits() {
    let mut chain = AuditorChain::new();
    let (tx, rx) = mpsc::channel::<u32>(3);
    chain.push_sink(tx, |ev| {
        matches!(ev, RunEvent::UnitStarted { .. }).then_some(1)
    });
    let t0 = Instant::now();
    for i in 0..10 {
        chain.on_event(&started(i));
        // An event the sink takes nothing of is never a drop.
        chain.on_event(&RunEvent::RunEnded(RunSummary::default()));
    }
    assert!(t0.elapsed() < Duration::from_secs(1));
    assert_eq!(chain.dropped(), 7);
    // A closed sink (its reader gone) is no drop either.
    drop(rx);
    chain.on_event(&started(11));
    assert_eq!(chain.dropped(), 7);
}

#[test]
#[ignore = "perf"]
fn sync_chain_under_5us_per_event() {
    if std::env::var_os("HACIVAT_PERF").is_none() {
        return;
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    rt.block_on(async {
        let n = Arc::new(AtomicU64::new(0));
        let mut chain = AuditorChain::new();
        for _ in 0..4 {
            chain.push_sync(Box::new(Count(n.clone())));
        }
        let (tx, mut rx) = mpsc::channel::<u64>(SINK_CAPACITY);
        chain.push_sink(tx, |ev| match ev {
            RunEvent::RowsFetched { total, .. } => Some(*total),
            _ => None,
        });
        const EVENTS: u64 = 200_000;
        let t0 = Instant::now();
        for i in 0..EVENTS {
            chain.on_event(&rows(i % 64, i));
            if i % 1_000 == 0 {
                while rx.try_recv().is_ok() {}
            }
        }
        let per = t0.elapsed() / u32::try_from(EVENTS).unwrap();
        assert_eq!(n.load(Ordering::Relaxed), 4 * EVENTS);
        assert!(per < Duration::from_micros(5), "{per:?} per event");
    });
}
