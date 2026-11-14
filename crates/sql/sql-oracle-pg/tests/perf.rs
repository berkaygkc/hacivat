//! Oracle budgets (Task 9). Perf tests run only with `HACIVAT_PERF=1`, in `--release`, through
//! the gate's perf line; `crates/sql` may not reach `hacivat-testkit`, so the check is inline.
#![cfg_attr(not(unix), allow(dead_code, unused_imports, unused_variables))]

use std::sync::Mutex;
use std::time::{Duration, Instant};

use sql_oracle_pg::{split_with_parser, statement_count};
use sql_text::TextSize;

/// The perf tests share one process: each runs alone, so the RSS window sees no other test.
static ALONE: Mutex<()> = Mutex::new(());

fn perf_enabled() -> bool {
    std::env::var_os("HACIVAT_PERF").is_some_and(|v| v == "1")
}

/// This process's resident set, from `ps` (no `unsafe`).
#[cfg(unix)]
fn rss_kib() -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .expect("ps prints the RSS in KiB")
}

#[cfg(unix)]
#[test]
#[ignore = "perf"]
fn hundred_thousand_parses_keep_rss_flat() {
    if !perf_enabled() {
        return;
    }
    let _alone = ALONE.lock().unwrap_or_else(|e| e.into_inner());
    let mixed = |i: usize| match i % 4 {
        0 => statement_count("SELECT 1").map(|_| ()),
        1 => split_with_parser("SELECT a FROM t; SELECT :b").map(|_| ()),
        2 => statement_count("SELECT 'çğ' FRM t t").map(|_| ()),
        _ => split_with_parser("SELECT x FROM t t t; SELECT 2").map(|_| ()),
    };
    for i in 0..2_000 {
        let _ = mixed(i);
    }
    let before = rss_kib();
    for i in 0..100_000 {
        let _ = mixed(i);
    }
    let after = rss_kib();
    // A fresh thread per call lost about 8 KiB each (≈ 800 MiB here); one parser thread
    // keeps the growth to allocator noise (measured: +560 KiB, macOS arm64, release).
    assert!(after < before + 8 * 1024, "RSS {before} KiB → {after} KiB");
}

/// A one-statement `INSERT … VALUES` of at least `bytes` bytes.
fn insert_of(bytes: usize) -> String {
    let mut sql = String::from("INSERT INTO t (id, name, note) VALUES ");
    let mut i = 0u32;
    while sql.len() < bytes {
        if i > 0 {
            sql.push_str(", ");
        }
        sql.push_str(&format!("({i}, 'name {i}', 'çğ ı İ')"));
        i += 1;
    }
    sql
}

fn median(mut runs: Vec<Duration>) -> Duration {
    runs.sort();
    runs[runs.len() / 2]
}

#[test]
#[ignore = "perf"]
fn one_mib_statement_parses_within_budget() {
    if !perf_enabled() {
        return;
    }
    let _alone = ALONE.lock().unwrap_or_else(|e| e.into_inner());
    let sql = insert_of(1 << 20);
    // Just under 1 MiB with an error whose text (`t`) occurs three times: the cursor takes
    // prefix parses (`error::locate`).
    let broken = format!("{} t t", insert_of((1 << 20) - 200));
    let time = |f: &dyn Fn()| {
        median(
            (0..7)
                .map(|_| {
                    let t = Instant::now();
                    f();
                    t.elapsed()
                })
                .collect(),
        )
    };
    let split = time(&|| assert_eq!(split_with_parser(&sql).unwrap().len(), 1));
    let count = time(&|| assert_eq!(statement_count(&sql), Ok(1)));
    let at = u32::try_from(broken.len() - 3).unwrap();
    let error = time(&|| {
        assert_eq!(
            split_with_parser(&broken).unwrap_err().cursor,
            Some(TextSize::from(at))
        );
    });
    println!("oracle, 1 MiB: split {split:?}, count {count:?}, error with cursor {error:?}");
    // The scan, the nesting check and the parse of 1 MiB, on the parser thread. Measured
    // (release, macOS arm64): split 9 ms, count 8 ms, error with its cursor 29 ms. The budgets
    // leave room for slower CI machines.
    assert!(split < Duration::from_millis(100), "split: {split:?}");
    assert!(count < Duration::from_millis(100), "count: {count:?}");
    assert!(
        error < Duration::from_millis(250),
        "error with cursor: {error:?}"
    );
}
