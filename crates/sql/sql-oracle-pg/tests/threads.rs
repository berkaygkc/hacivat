//! The parser thread (Task 9; R-M3-3, R-M3-35).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use sql_oracle_pg::{on_parser_stack, split_with_parser, statement_count};

#[test]
fn eight_threads_parse_in_parallel() {
    std::thread::scope(|s| {
        for i in 0..8 {
            s.spawn(move || {
                for j in 0..1_000 {
                    assert_eq!(statement_count("SELECT 1").unwrap(), 1);
                    if (i + j) % 100 == 0 {
                        assert_eq!(split_with_parser("SELECT 1; SELECT 2").unwrap().len(), 2);
                        assert!(statement_count("SELEC 1").is_err());
                    }
                }
            });
        }
    });
}

#[test]
fn on_parser_stack_returns_the_result_and_contains_a_panic() {
    assert_eq!(on_parser_stack(|| 1), Some(1));
    assert_eq!(on_parser_stack(|| -> i32 { panic!("boom") }), None);
    // The thread outlives the panic.
    assert_eq!(on_parser_stack(|| 2), Some(2));
    assert_eq!(statement_count("SELECT 1; SELECT 2"), Ok(2));
}

#[test]
fn jobs_from_many_threads_never_overlap() {
    // libpg_query swaps the process's stderr during each call; two at once can leave it
    // broken, so the parser thread runs one job at a time.
    static ACTIVE: AtomicUsize = AtomicUsize::new(0);
    static MOST: AtomicUsize = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| {
                for _ in 0..200 {
                    on_parser_stack(|| {
                        let now = ACTIVE.fetch_add(1, Ordering::SeqCst) + 1;
                        MOST.fetch_max(now, Ordering::SeqCst);
                        for _ in 0..50 {
                            thread::yield_now();
                        }
                        ACTIVE.fetch_sub(1, Ordering::SeqCst);
                    })
                    .unwrap();
                }
            });
        }
    });
    assert_eq!(MOST.load(Ordering::SeqCst), 1);
}

#[test]
fn every_job_runs_on_the_one_parser_thread() {
    let name = || thread::current().name().map(str::to_string);
    let first = on_parser_stack(move || (thread::current().id(), name())).unwrap();
    assert_eq!(first.1.as_deref(), Some("hacivat-pg-parser"));
    let ids: Vec<_> = (0..4)
        .map(|_| thread::spawn(|| on_parser_stack(|| thread::current().id()).unwrap()))
        .map(|h| h.join().unwrap())
        .collect();
    assert!(
        ids.iter().all(|id| *id == first.0),
        "{ids:?} vs {:?}",
        first.0
    );
}

#[test]
fn a_job_may_call_the_oracle_again() {
    // `hacivat-exec` wraps `split_with_parser` in `on_parser_stack` (Task 23); the inner call
    // runs inline instead of waiting for the thread it is on.
    let r = on_parser_stack(|| split_with_parser("SELECT 1; SELECT 2").map(|r| r.len()));
    assert_eq!(r, Some(Ok(2)));
    assert_eq!(on_parser_stack(|| on_parser_stack(|| 3)), Some(Some(3)));
    assert_eq!(
        on_parser_stack(|| on_parser_stack(|| -> u8 { panic!("inner") })),
        Some(None)
    );
}

/// Recurses until the stack below `top` holds `want` bytes of frames, and returns how many it
/// holds then (the stack grows down on every supported target).
fn dig(top: usize, want: usize) -> usize {
    let pad = std::hint::black_box([0u8; 1024]);
    let used = top.saturating_sub(std::ptr::addr_of!(pad) as usize);
    if used >= want {
        return used;
    }
    dig(top, want) + usize::from(std::hint::black_box(pad)[0])
}

#[test]
fn the_parser_thread_has_a_16_mib_stack() {
    // 12 MiB of frames: six times a test thread's default 2 MiB stack.
    let used = on_parser_stack(|| {
        let top = 0u8;
        dig(std::ptr::addr_of!(top) as usize, 12 << 20)
    });
    assert!(used.is_some_and(|u| u >= 12 << 20), "{used:?}");
}
