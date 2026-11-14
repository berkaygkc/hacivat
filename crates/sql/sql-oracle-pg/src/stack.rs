//! The parser's own thread with a 16 MiB stack (R-M3-3 on M3a's one parser call, R-M3-35;
//! Task 9).
//!
//! One long-lived thread runs every job, one at a time, rather than a fresh thread per call:
//! - libpg_query points the process's stderr (fd 2) at a pipe for the length of each parse or
//!   scan and then points it back (`pg_query_raw_parse`, `pg_query_scan`). Two such calls on
//!   two threads at once can restore each other's pipe and leave fd 2 on a closed pipe for
//!   the rest of the process. One thread never overlaps two calls.
//! - libpg_query keeps state per thread (a top memory context, and a `pthread_key_create`
//!   per thread that is never deleted). Measured on macOS while writing this: a fresh thread
//!   per call loses about 8 KiB of resident memory per thread (5,000 threads: +40 MiB; the
//!   same 5,000 calls on one thread: +64 KiB), and on Windows nothing frees it at all.
//! - libpg_query sizes its own stack-depth check from the thread it runs on (at most 2 MB of
//!   its recursion), so 16 MiB leaves it ample room and the rest of the parser more.

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, PoisonError};
use std::thread;

/// The deepest bracket nesting (`(` and `[` together, as PostgreSQL's scanner reads them) a
/// text may have to be sent to the parser; deeper texts get an error without a parse
/// (R-M3-35). Callers holding a `sql_lexer::Lexed` check `max_paren_depth` against it first.
pub const MAX_ORACLE_NESTING: u16 = 256;

/// The parser thread's stack (R-M3-3).
pub(crate) const PARSER_STACK_BYTES: usize = 16 << 20;

type Job = Box<dyn FnOnce() + Send + 'static>;

/// The parser thread's queue; started on first use, and again if the thread is gone.
static QUEUE: Mutex<Option<Sender<Job>>> = Mutex::new(None);

thread_local! {
    static ON_PARSER_THREAD: Cell<bool> = const { Cell::new(false) };
}

/// Runs `f` on the parser thread, whose stack is 16 MiB, and waits for its result; `None` if
/// `f` panicked or the thread could not be started.
///
/// - Jobs from all threads run one at a time, in the order they arrive (module docs say why).
///   A job that blocks delays every later one, and one that waits for a job submitted from
///   another thread never finishes.
/// - Called from inside a job, `f` runs at once on the same thread.
/// - The thread outlives a panicking job.
pub fn on_parser_stack<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> Option<R> {
    if ON_PARSER_THREAD.with(Cell::get) {
        return catch_unwind(AssertUnwindSafe(f)).ok();
    }
    let (tx, rx) = mpsc::sync_channel(1);
    let job: Job = Box::new(move || {
        // The receiver only goes away if the caller's thread is gone.
        let _ = tx.send(catch_unwind(AssertUnwindSafe(f)).ok());
    });
    if !submit(job) {
        return None;
    }
    rx.recv().ok().flatten()
}

/// Queues `job` on the parser thread, starting the thread first if there is none; false if
/// it cannot be started.
fn submit(job: Job) -> bool {
    let mut queue = QUEUE.lock().unwrap_or_else(PoisonError::into_inner);
    let job = match queue.as_ref() {
        Some(tx) => match tx.send(job) {
            Ok(()) => return true,
            // The thread is gone (it never should be): start another below.
            Err(mpsc::SendError(job)) => job,
        },
        None => job,
    };
    let (tx, rx) = mpsc::channel::<Job>();
    let spawned = thread::Builder::new()
        .name("hacivat-pg-parser".to_string())
        .stack_size(PARSER_STACK_BYTES)
        .spawn(move || {
            ON_PARSER_THREAD.with(|on| on.set(true));
            for job in rx {
                // Each job catches its own panic; this keeps the thread even if one escapes.
                let _ = catch_unwind(AssertUnwindSafe(job));
            }
        });
    if spawned.is_err() || tx.send(job).is_err() {
        return false;
    }
    *queue = Some(tx);
    true
}
