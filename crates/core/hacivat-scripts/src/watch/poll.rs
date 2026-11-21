//! The poll source (HV-SCRIPT-08 step 1; R-M4b-14): network file systems, a small inotify
//! budget, and a lost OS backend.
//!
//! ## m4b-t09-watcher
//! Every [`POLL_INTERVAL`] the source hands each watched root one hint on the root itself;
//! the watcher's reconciler answers a root hint with one scan of the library (names and
//! `lstat` only, never a content read, never through a directory link, skipped folders not
//! entered) compared with what it knows, by length and full-precision modification time.
//!
//! Not `notify`'s `PollWatcher`: in `notify` 8.2.2 it keeps modification times in whole
//! seconds and reports a file only when the time grew (`poll.rs:83`, `:366`, `:410`), so an
//! edit made in the same second as the previous pass is never reported [Observed: Task 9's
//! `polling_fallback_sees_a_change` failed with it]. It also walks `.git` and `node_modules`
//! every pass. With this source no watcher constructor is called at all, so the
//! `#[expect(clippy::disallowed_methods)]` exception the carry anticipated is not needed.
//!
//! The source's thread waits on a condition variable between passes (no CPU while idle) and
//! is stopped and joined when the source is dropped.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::{EventSource, HintSink, RawHint, WatchError};

/// How often a polled library is looked at.
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Default)]
struct State {
    roots: Vec<(PathBuf, HintSink)>,
    stopped: bool,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The poll event source.
pub struct PollEventSource {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl PollEventSource {
    /// A source that looks at its roots every [`POLL_INTERVAL`].
    pub fn new() -> Result<Self, WatchError> {
        Self::with_interval(POLL_INTERVAL)
    }

    /// A source that looks at its roots every `interval`.
    pub fn with_interval(interval: Duration) -> Result<Self, WatchError> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
        });
        let looped = shared.clone();
        let thread = std::thread::Builder::new()
            .name("hv-library-poll".into())
            .spawn(move || run(&looped, interval))
            .map_err(|e| WatchError::Backend(e.to_string()))?;
        Ok(PollEventSource {
            shared,
            thread: Some(thread),
        })
    }
}

fn run(shared: &Shared, interval: Duration) {
    let mut next = Instant::now() + interval;
    let mut st = shared.lock();
    loop {
        if st.stopped {
            return;
        }
        let now = Instant::now();
        if now < next {
            st = shared
                .wake
                .wait_timeout(st, next - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            continue;
        }
        for (root, sink) in st.roots.iter_mut() {
            sink(vec![RawHint::Path(root.clone())]);
        }
        next = Instant::now() + interval;
    }
}

impl fmt::Debug for PollEventSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PollEventSource").finish_non_exhaustive()
    }
}

impl EventSource for PollEventSource {
    fn watch(&mut self, root: &Path, sink: HintSink) -> Result<(), WatchError> {
        let mut st = self.shared.lock();
        if st.stopped {
            return Err(WatchError::NotImplemented);
        }
        st.roots.retain(|(r, _)| r != root);
        st.roots.push((root.to_path_buf(), sink));
        Ok(())
    }

    fn unwatch(&mut self, root: &Path) {
        self.shared.lock().roots.retain(|(r, _)| r != root);
    }
}

impl Drop for PollEventSource {
    fn drop(&mut self) {
        {
            let mut st = self.shared.lock();
            st.stopped = true;
            st.roots.clear();
        }
        self.shared.wake.notify_all();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
// ## m4b-t09-watcher
