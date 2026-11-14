//! M3a Task 1a (R-M3-13): the TestScheduler teardown deadlock. A quit runs the quit handlers
//! (the core runtime stops inside them) and then blocks on their futures with a timeout; a core
//! worker that wakes a GPUI task from its own thread at that moment took the test scheduler's
//! `state` and `rng` locks in the opposite order to the blocked thread. Fork commit `6607045`
//! fixes the order (`crates/ui/hacivat-ui-core/patches/scheduler-block-until-lock-order.patch`).
//!
//! - `fifty_core_boot_and_quit_cycles_never_hang`: each cycle boots today's app with the core
//!   runtime, connects one profile through `FakeDriver` so core workers are live, and quits for
//!   real. The window is one expression per timed block, so this rarely hits it: 1,000 local
//!   cycles at the old rev `a1eaa20` never hung. It guards the whole teardown path.
//! - `timed_blocks_while_another_thread_schedules_never_hang` hits the window directly: at
//!   `a1eaa20` it hung in 5 of 5 local runs, at `6607045` it passes in about a second.
//!
//! Every cycle runs under a watchdog that ends the test binary when it hangs, since a
//! deadlocked thread cannot fail its own test (`watchdog_fails_a_hung_cycle` proves it fires).

mod support;

use std::io::Write as _;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use futures::future;
use gpui::TestAppContext;
use hacivat_driver::fake::FakeDriver;
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_session::LiveState;
use support::{add_profile, connect, fake, live, shell_with_fake, test_profile, wait_until};

/// Boot-and-quit cycles per run.
const CYCLES: usize = 50;
/// How long one cycle may take before the watchdog calls it hung. A healthy boot-and-quit cycle,
/// or a batch of timed blocks, takes well under a second, and a deadlock never resolves, so the
/// limit only delays a real failure. It stays well above `support::WAIT`, so that a slow but
/// live connect fails through `wait_until`'s own message rather than as a "hang".
const CYCLE_LIMIT: Duration = Duration::from_secs(60);
const _: () = assert!(CYCLE_LIMIT.as_secs() >= 3 * support::WAIT.as_secs());
/// Timed blocks run against the spawning thread, in watched batches.
const TIMED_BATCHES: usize = 5;
const BLOCKS_PER_BATCH: usize = 100;
/// Background tasks the spawning thread keeps queued at most.
const MAX_OUTSTANDING: usize = 256;
/// The test binary's exit code when the watchdog fires.
const HUNG_EXIT: i32 = 86;
/// Set on the child process of [`watchdog_fails_a_hung_cycle`] only.
const HANG_PROBE_VAR: &str = "HACIVAT_TEARDOWN_STRESS_HANG_PROBE";

#[gpui::test]
fn fifty_core_boot_and_quit_cycles_never_hang(cx: &mut TestAppContext) {
    for i in 0..CYCLES {
        with_watchdog(CYCLE_LIMIT, i, || boot_core_then_shutdown(cx));
    }
}

/// The inversion itself, without the app around it: while another thread spawns background
/// tasks without pause (each spawn locks the scheduler's `state`, then its `rng`), the test
/// thread runs timed blocks, each of which computes its tick budget under both locks. Before
/// `6607045` that took `rng`, then `state`, and this hung in every local run.
#[gpui::test]
fn timed_blocks_while_another_thread_schedules_never_hang(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let background = cx.executor();
    let foreground = cx.foreground_executor().clone();
    let stop = Arc::new(AtomicBool::new(false));
    let outstanding = Arc::new(AtomicUsize::new(0));
    let spawner = {
        let (stop, outstanding) = (stop.clone(), outstanding.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                // Bounded, so the queue (and memory) stays small however fast this thread is.
                if outstanding.load(Ordering::Relaxed) >= MAX_OUTSTANDING {
                    std::thread::yield_now();
                    continue;
                }
                outstanding.fetch_add(1, Ordering::Relaxed);
                let done = outstanding.clone();
                background
                    .spawn(async move {
                        done.fetch_sub(1, Ordering::Relaxed);
                    })
                    .detach();
            }
        })
    };
    for batch in 0..TIMED_BATCHES {
        with_watchdog(CYCLE_LIMIT, batch, || {
            for _ in 0..BLOCKS_PER_BATCH {
                let blocked = foreground
                    .block_with_timeout(Duration::from_millis(1), future::pending::<()>());
                assert!(blocked.is_err(), "a pending future never completes");
            }
        });
    }
    stop.store(true, Ordering::Relaxed);
    spawner.join().expect("the spawner thread");
}

/// The watchdog really fails a hung cycle: this test runs itself again as a child process whose
/// cycle never returns, and expects the child to exit with [`HUNG_EXIT`] naming the cycle.
#[test]
fn watchdog_fails_a_hung_cycle() {
    if std::env::var_os(HANG_PROBE_VAR).is_some() {
        with_watchdog(Duration::from_millis(200), 7, || {
            let (_keep_open, never) = mpsc::channel::<()>();
            let _ = never.recv();
        });
        unreachable!("a hung cycle returned");
    }
    let out = Command::new(std::env::current_exe().expect("the test binary"))
        .args(["watchdog_fails_a_hung_cycle", "--exact", "--test-threads=1"])
        .env(HANG_PROBE_VAR, "1")
        .output()
        .expect("run the hang probe");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(HUNG_EXIT), "{stderr}");
    assert!(stderr.contains("cycle 7 hung"), "{stderr}");
}

/// Boots the app with the core runtime and a `FakeDriver` PostgreSQL, connects one profile,
/// then quits through `App::shutdown`, as the bounded quit does (P15).
fn boot_core_then_shutdown(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", FakeDriver::new(DbmsKind::Postgres));
    add_profile(vcx, test_profile("a", Some(EnvTag::Dev), "127.0.0.1"));
    connect(vcx, "a");
    wait_until(vcx, "the connect to finish", |vcx| {
        matches!(live(vcx, "a"), LiveState::Connected(_))
    });
    assert_eq!(fake(vcx).open_connections(), 1);
    vcx.executor().allow_parking();
    vcx.cx.update(|cx| cx.shutdown());
    assert_eq!(
        fake(vcx).open_connections(),
        0,
        "the quit closes the connection"
    );
}

/// Runs `f` (cycle `cycle`); if it has not returned after `limit`, prints which cycle hung and
/// ends the test binary with [`HUNG_EXIT`]. A deadlocked thread cannot fail its own test, and
/// libtest has no per-test timeout.
fn with_watchdog<R>(limit: Duration, cycle: usize, f: impl FnOnce() -> R) -> R {
    // Dropping the sender (when `f` returns or unwinds) releases the watchdog.
    let (done, wait) = mpsc::channel::<()>();
    let dog = std::thread::spawn(move || {
        if let Err(mpsc::RecvTimeoutError::Timeout) = wait.recv_timeout(limit) {
            // Straight to stderr: libtest's output capture would swallow `eprintln!`, and the
            // process ends before it could print captured output.
            let mut err = std::io::stderr();
            let _ = writeln!(
                err,
                "teardown_stress: cycle {cycle} hung for more than {limit:?} \
                 (the TestScheduler lock order? R-M3-13)"
            );
            let _ = err.flush();
            std::process::exit(HUNG_EXIT);
        }
    });
    let out = f();
    drop(done);
    dog.join().expect("the watchdog thread");
    out
}
