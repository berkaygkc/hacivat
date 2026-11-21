//! Object filter timing (alg/02 HV-INTRO-03, "O(items) per object"; M4 Task 12): 160,000 names
//! through a typical filter in under 50 ms. `#[ignore = "perf"]`, self-gated on
//! `HACIVAT_PERF=1`; run in release:
//! `HACIVAT_PERF=1 cargo test --release -p hacivat-introspect --test filter_perf -- --ignored --nocapture`.
//! The gate is the median of the runs (R-W2-3).

use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_introspect::filter::{FilterRole, parse_filter};
use hacivat_meta::ObjectKind;
use hacivat_meta::name::{IdentCtx, Name};
use sql_dialect::DialectId;

fn perf_enabled() -> bool {
    let on = std::env::var_os("HACIVAT_PERF").is_some_and(|v| v == "1");
    if !on {
        eprintln!("skipped: set HACIVAT_PERF=1");
    }
    on
}

#[test]
#[ignore = "perf"]
fn filter_matches_160k_names_under_50ms() {
    if !perf_enabled() {
        return;
    }
    const KINDS: [ObjectKind; 5] = [
        ObjectKind::Table,
        ObjectKind::View,
        ObjectKind::Routine,
        ObjectKind::Sequence,
        ObjectKind::Column,
    ];
    let names: Vec<(ObjectKind, Name)> = (0..160_000u32)
        .map(|i| {
            let stem = match i % 6 {
                0 => "OLD_orders",
                1 => "audit_log",
                2 => "DO_work",
                3 => "V_sales",
                4 => "tmp_17",
                _ => "customer_address_history",
            };
            (
                KINDS[i as usize % KINDS.len()],
                Arc::from(format!("{stem}_{i}")),
            )
        })
        .collect();
    // Prefix, suffix and infix globs, a class, a `?`, a regex, typed and untyped items; MySQL
    // so that some classes compare caselessly.
    let f = parse_filter(
        r"table:-OLD_*||table:-*_bak||routine:DO*||view:V_?*||*_[0-9]*||-*audit*||-{tmp_\d+_\d+}",
        DialectId::MySql,
        FilterRole::Introspect,
    )
    .expect("the filter parses")
    .with_ident(IdentCtx::new(DialectId::MySql));
    let mut runs = Vec::new();
    let mut passed = 0usize;
    for _ in 0..7 {
        let t = Instant::now();
        passed = names.iter().filter(|(k, n)| f.matches(*k, n)).count();
        runs.push(t.elapsed());
    }
    runs.sort();
    let median = runs[runs.len() / 2];
    eprintln!(
        "filter over {} names: median {median:?}, min {:?}, max {:?}; {passed} pass",
        names.len(),
        runs[0],
        runs[runs.len() - 1]
    );
    assert!(passed > 0 && passed < names.len());
    assert!(median < Duration::from_millis(50), "median {median:?}");
}
