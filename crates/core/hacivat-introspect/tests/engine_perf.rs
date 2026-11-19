//! The scheduler's own cost per task (M4 Task 15). `#[ignore = "perf"]`, run with
//! `HACIVAT_PERF=1 cargo test --release -p hacivat-introspect --test engine_perf -- --ignored`.

mod common;

use std::time::{Duration, Instant};

use common::*;
use hacivat_introspect::ddl_effects::Target;
use hacivat_introspect::testing::FakeIntrospector;
use hacivat_meta::merge::Trigger;

const TASKS: u32 = 2_000;

/// A task over an empty namespace is all scheduler: queue, worker, merge, publish, ticket.
/// Its median cost stays under 1 ms.
#[test]
#[ignore = "perf"]
fn scheduler_overhead_per_task_under_1ms() {
    if std::env::var("HACIVAT_PERF").as_deref() != Ok("1") {
        eprintln!("skipped: set HACIVAT_PERF=1");
        return;
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (median, answered) = rt.block_on(async {
        let n = pg_ns("db", "public");
        let fake = FakeIntrospector::default();
        fake.put_ns(n.clone(), Vec::new());
        let r = rig_with(fake, RigOpts::default());
        let mut times = Vec::with_capacity(TASKS as usize);
        for _ in 0..TASKS {
            let start = Instant::now();
            bounded(
                r.handle
                    .refresh(Target::Namespace(n.clone()), Trigger::User),
            )
            .await
            .unwrap();
            times.push(start.elapsed());
        }
        // A load the catalog already answers never leaves the actor.
        let start = Instant::now();
        for _ in 0..TASKS {
            bounded(r.handle.expand(n.clone())).await.unwrap();
        }
        let answered = start.elapsed() / TASKS;
        times.sort();
        (times[times.len() / 2], answered)
    });
    eprintln!("median task {median:?}, answered load {answered:?}");
    assert!(median < Duration::from_millis(1), "median {median:?}");
    assert!(answered < Duration::from_millis(1), "answered {answered:?}");
}
