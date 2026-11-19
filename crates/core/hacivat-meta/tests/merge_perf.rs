//! Merge timings (HV-INTRO-11 "160k-column schemas merge in tens of milliseconds"; M4 Task 10).
//! `#[ignore = "perf"]`, self-gated on `HACIVAT_PERF=1`; run in release:
//! `HACIVAT_PERF=1 cargo test --release -p hacivat-meta --test merge_perf -- --ignored --nocapture`.

use std::time::{Duration, Instant};

use hacivat_meta::fixture::{pg_catalog, table_subtree};
use hacivat_meta::merge::{ChangedObject, ExistSet, Identity, MergePolicy, NamespaceResult, merge};
use hacivat_meta::namespace::{Level, NamespaceSnapshot, Subtree, Watermark};
use hacivat_meta::{ObjectKind, StateMark};

fn perf_enabled() -> bool {
    let on = std::env::var_os("HACIVAT_PERF").is_some_and(|v| v == "1");
    if !on {
        eprintln!("skipped: set HACIVAT_PERF=1");
    }
    on
}

/// Object `i` of `ns` with its subtree, as a pass would return it.
fn subtree_of(ns: &NamespaceSnapshot, i: u32) -> Subtree {
    let mut t = Subtree::new(ns.objs()[i as usize].clone());
    let mut stack = vec![(i, t.root())];
    while let Some((s, node)) = stack.pop() {
        for (d, o) in ns.children(s) {
            let n = t.add(node, o.clone());
            stack.push((d, n));
        }
    }
    t
}

/// The existence sets of every kind the namespace holds at the top level and of its indexes.
fn existence(ns: &NamespaceSnapshot) -> Vec<(ObjectKind, ExistSet)> {
    let mut by_kind: std::collections::BTreeMap<ObjectKind, Vec<Identity>> = Default::default();
    let ns_len = ns.key().path.len();
    for (i, o) in ns.objs().iter().enumerate().skip(1) {
        let top = o.parent() == Some(0);
        if top || o.kind() == ObjectKind::Index {
            let key = ns.object_key(i as u32).expect("key");
            by_kind.entry(o.kind()).or_default().push(Identity {
                path: key.path[ns_len..].to_vec(),
                server_id: o.server_id(),
                overload_sig: o.props().overload_sig().cloned(),
                state: StateMark::None,
            });
        }
    }
    by_kind
        .into_iter()
        .map(|(k, ids)| (k, ExistSet::Ok(ids)))
        .collect()
}

fn result(ns: &NamespaceSnapshot, changed: Vec<ChangedObject>, full: bool) -> NamespaceResult {
    NamespaceResult {
        existent: existence(ns).into_iter().collect(),
        changed,
        categories_to_reload: Default::default(),
        new_watermark: Watermark::None,
        full,
        level: Level::Structure,
        read_at_ms: 0,
    }
}

/// The p50 and p95 of `runs` merges of `r` into `prev` (the result is cloned outside the timer).
fn time(prev: &NamespaceSnapshot, r: &NamespaceResult, runs: usize) -> (Duration, Duration) {
    let mut t = Vec::with_capacity(runs);
    for _ in 0..runs {
        let input = r.clone();
        let start = Instant::now();
        let o = merge(Some(prev), input, &MergePolicy::default()).expect("merge");
        t.push(start.elapsed());
        assert!(!o.is_pending());
        drop(o);
    }
    t.sort();
    (t[runs / 2], t[runs * 95 / 100])
}

/// One schema of 15,000 tables and 160,000 columns (with keys, indexes, foreign keys, views,
/// functions): a full pass re-reading everything, and an incremental pass re-reading 1 %.
#[test]
#[ignore = "perf"]
fn merge_160k_columns_under_50ms() {
    if !perf_enabled() {
        return;
    }
    let ns = pg_catalog(&[("public", 15_000)], 160_000)
        .pop()
        .expect("the schema namespace");
    assert_eq!(ns.column_count(), 160_000);
    let root = ns.object_key(0).expect("root");
    let tops: Vec<u32> = ns.children(0).map(|(i, _)| i).collect();
    let all: Vec<ChangedObject> = tops
        .iter()
        .map(|&i| ChangedObject {
            parent: root.clone(),
            subtree: subtree_of(&ns, i),
        })
        .collect();
    let some: Vec<ChangedObject> = tops
        .iter()
        .step_by(100)
        .map(|&i| ChangedObject {
            parent: root.clone(),
            subtree: subtree_of(&ns, i),
        })
        .collect();
    let full = result(&ns, all, true);
    let inc = result(&ns, some, false);
    let none = result(&ns, Vec::new(), false);
    let (fp50, fp95) = time(&ns, &full, 20);
    let (ip50, ip95) = time(&ns, &inc, 20);
    let (np50, np95) = time(&ns, &none, 20);
    eprintln!(
        "merge, {} objects / 160,000 columns: full p50 {fp50:?} p95 {fp95:?}; \
         incremental 1 % p50 {ip50:?} p95 {ip95:?}; nothing changed p50 {np50:?} p95 {np95:?}",
        ns.len()
    );
    // The gate is the median (alg/02 names no percentile; the p95 of 20 runs on a shared
    // machine measures the scheduler). Both are printed for the report.
    assert!(fp50 <= Duration::from_millis(50), "full pass p50 {fp50:?}");
    assert!(
        ip50 <= Duration::from_millis(50),
        "incremental p50 {ip50:?}"
    );
}

/// The time of a 16,000-column table's merge is about ten times a 1,600-column one's, kept or
/// re-read (T3 carry: never a `find` per column).
#[test]
#[ignore = "perf"]
fn wide_table_merge_time_is_linear() {
    if !perf_enabled() {
        return;
    }
    let measure = |cols: usize, changed: bool| {
        let ns = hacivat_meta::fixture::ns_pg("app", "public")
            .subtree(table_subtree("public", "wide", cols, 9, None))
            .finish();
        let c = if changed {
            vec![ChangedObject {
                parent: ns.object_key(0).expect("root"),
                subtree: table_subtree("public", "wide", cols, 9, None),
            }]
        } else {
            Vec::new()
        };
        let r = result(&ns, c, false);
        time(&ns, &r, 21).0
    };
    for changed in [false, true] {
        let small = measure(1_600, changed);
        let big = measure(16_000, changed);
        eprintln!("wide table (changed = {changed}): 1,600 columns {small:?}, 16,000 {big:?}");
        assert!(
            big <= small * 30,
            "16,000 columns {big:?} vs 1,600 {small:?}"
        );
    }
}
