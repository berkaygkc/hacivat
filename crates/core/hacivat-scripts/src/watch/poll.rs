//! The poll source (HV-SCRIPT-08 step 1; R-M4b-14): network file systems, a small inotify
//! budget, and a lost OS backend.
//!
//! ## m4b-t09-watcher
//! On each pass the source hands each watched root one hint on the root itself; the watcher's
//! reconciler answers a root hint with one scan of the library (names and `lstat` only, never
//! a content read, never through a directory link, skipped folders not entered) compared with
//! what it knows, by length and full-precision modification time.
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
//!
//! ## m4b-t09-watcher fix round 1
//! - **Pace (m5, controller ruling):** a root starts at the base interval ([`POLL_INTERVAL`]);
//!   every pass that changed nothing doubles it, up to [`MAX_POLL_INTERVAL`]; a change resets
//!   it. A pass is never closer than five times what the last one cost ([`next_interval`]),
//!   so a slow share is not scanned back to back. The watcher reports each pass through
//!   [`EventSource::pace`].
//! - **Repository (I4):** each pass also `lstat`s `<root>/.git/{HEAD,index,packed-refs}`
//!   (`.git` itself must be a real folder: a gitfile or a link is left to the status task),
//!   never reading or following anything, and a dataless one never opened (Task 7's rule).
//!   When one of their `(len, mtime, dataless)` moved, the pass also hints `.git/index`, which
//!   the reconciler reports as `RepositoryChanged`.
//!
//! ## m4b-t10-service fix round 3 (G2, R-T10-6; a Task 9 named exception)
//! Each pass also `lstat`s `<root>/.hacivat/workspace.toml` (`.hacivat` must be a real folder;
//! nothing is read or followed); when its `(len, mtime, dataless)` moved, or it came or went,
//! the pass also hints that path, which the reconciler reports as `ManifestChanged`.
//!
//! ## m4b-cifix-3-manifest-poll (CI 45068616161)
//! - **Baselines at `watch`.** The repository's and the manifest's first stamps are taken when
//!   a root is watched, not at its first pass, so a change between the watcher's start (the
//!   service's scan reads the manifest after it) and the first pass is reported.
//! - **Content.** The manifest's stamp also carries the hash of its bytes, read through
//!   `manifest::content_hash` (the service's own reader: no link followed, at most
//!   `MANIFEST_MAX_BYTES`; never a dataless file), so a rewrite that keeps the length and the
//!   modification time (a coarse network clock) is still a change. This is the one read a pass
//!   makes; scripts are still compared by `lstat` only.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use super::{EventSource, HintSink, RawHint, WatchError};

/// How often a polled library is looked at while things change.
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// The slowest pace of an idle polled library.
pub const MAX_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// The repository files whose stamps a pass compares (Task 7 reads them; the pass only
/// `lstat`s them).
///
/// ## m4b-t09-watcher fix round 2 (m-fr1-1): `logs/HEAD` too, appended on every move of
/// `HEAD`'s branch, so a ref-only change (`reset --soft`, `update-ref`) is seen. Its folder
/// `logs` must be a real folder.
const REPOSITORY_FILES: [&str; 4] = ["HEAD", "index", "packed-refs", "logs/HEAD"];

/// The next interval after a pass: back to `base` on a change, doubled (up to `max`) when
/// nothing changed, and never less than five times what the pass cost.
pub fn next_interval(
    current: Duration,
    changed: bool,
    cost: Duration,
    base: Duration,
    max: Duration,
) -> Duration {
    let paced = if changed {
        base
    } else {
        current.saturating_mul(2).clamp(base, max.max(base))
    };
    paced.max(cost.saturating_mul(5))
}

type GitStamp = Option<(u64, SystemTime, bool)>;

/// The `(len, mtime, dataless)` of the repository files, from `lstat` only; `None` when the
/// root has no real `.git` folder.
fn repository_stamps(root: &Path) -> Option<[GitStamp; 4]> {
    let git = root.join(".git");
    let real_dir = |p: &Path| std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_dir());
    if !real_dir(&git) {
        return None;
    }
    let logs = real_dir(&git.join("logs"));
    Some(REPOSITORY_FILES.map(|name| {
        if name.contains('/') && !logs {
            return None;
        }
        let m = std::fs::symlink_metadata(git.join(name)).ok()?;
        m.file_type().is_file().then(|| {
            (
                m.len(),
                m.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                crate::dataless::is_dataless(&m),
            )
        })
    }))
}

/// ## m4b-cifix-3-manifest-poll: the manifest's `(len, mtime, dataless, content hash)`.
type ManifestStamp = Option<(u64, SystemTime, bool, Option<u64>)>;

/// ## m4b-t10-service fix round 3 (G2): the manifest's `(len, mtime, dataless)` from `lstat`
/// only; `None` when `.hacivat` is not a real folder or the file is not a plain file.
///
/// ## m4b-cifix-3-manifest-poll: and the hash of its bytes (`manifest::content_hash`, the
/// service's own no-link, size-capped read), so a rewrite that keeps the length and the
/// modification time is still seen. A dataless manifest is never read (it would download).
fn manifest_stamp(root: &Path) -> ManifestStamp {
    let dir = root.join(".hacivat");
    if !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.file_type().is_dir()) {
        return None;
    }
    let m = std::fs::symlink_metadata(dir.join("workspace.toml")).ok()?;
    if !m.file_type().is_file() {
        return None;
    }
    let dataless = crate::dataless::is_dataless(&m);
    let hash = if dataless {
        None
    } else {
        crate::manifest::content_hash(root)
    };
    Some((
        m.len(),
        m.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        dataless,
        hash,
    ))
}

struct Root {
    path: PathBuf,
    sink: HintSink,
    interval: Duration,
    last_pass: Instant,
    next: Instant,
    /// The repository files' stamps; the first ones taken when the root is watched
    /// (m4b-cifix-3-manifest-poll).
    git: Option<[GitStamp; 4]>,
    /// Fix round 3 (G2): the manifest's stamp; the first one taken when the root is watched
    /// (m4b-cifix-3-manifest-poll).
    manifest: ManifestStamp,
}

struct State {
    roots: Vec<Root>,
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
    base: Duration,
    max: Duration,
}

impl PollEventSource {
    /// A source that looks at its roots every [`POLL_INTERVAL`] (slower while idle).
    pub fn new() -> Result<Self, WatchError> {
        Self::with_pace(POLL_INTERVAL, MAX_POLL_INTERVAL)
    }

    /// A source that looks at its roots every `interval`, never backing off.
    pub fn with_interval(interval: Duration) -> Result<Self, WatchError> {
        Self::with_pace(interval, interval)
    }

    /// A source that starts at `base` and backs off to `max` while nothing changes.
    pub fn with_pace(base: Duration, max: Duration) -> Result<Self, WatchError> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                roots: Vec::new(),
                stopped: false,
            }),
            wake: Condvar::new(),
        });
        let looped = shared.clone();
        let thread = std::thread::Builder::new()
            .name("hv-library-poll".into())
            .spawn(move || run(&looped))
            .map_err(|e| WatchError::Backend(e.to_string()))?;
        Ok(PollEventSource {
            shared,
            thread: Some(thread),
            base,
            max,
        })
    }

    /// The interval `root` is polled at now (for tests and diagnostics).
    pub fn interval(&self, root: &Path) -> Option<Duration> {
        let st = self.shared.lock();
        st.roots.iter().find(|r| r.path == root).map(|r| r.interval)
    }
}

fn run(shared: &Shared) {
    let mut st = shared.lock();
    loop {
        if st.stopped {
            return;
        }
        let now = Instant::now();
        let Some(next) = st.roots.iter().map(|r| r.next).min() else {
            st = shared.wake.wait(st).unwrap_or_else(PoisonError::into_inner);
            continue;
        };
        if now < next {
            st = shared
                .wake
                .wait_timeout(st, next - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            continue;
        }
        // ## m4b-t20-adversarial (named exception, the cifix-3 review's note): the due roots'
        // stamps are taken with the lock released, so a share that hangs in an `lstat` or in
        // the manifest read never blocks `watch`, `unwatch` or `pace` (the watcher's thread).
        // A root unwatched, or watched again, meanwhile keeps its own state.
        let due: Vec<(PathBuf, Instant)> = st
            .roots
            .iter()
            .filter(|r| r.next <= now)
            .map(|r| (r.path.clone(), r.last_pass))
            .collect();
        drop(st);
        let looked: Vec<_> = due
            .into_iter()
            .map(|(path, seen)| {
                stall_hook(&path);
                let git = repository_stamps(&path);
                let manifest = manifest_stamp(&path);
                (path, seen, git, manifest)
            })
            .collect();
        st = shared.lock();
        for (path, seen, git, manifest) in looked {
            let Some(root) = st
                .roots
                .iter_mut()
                .find(|r| r.path == path && r.last_pass == seen)
            else {
                continue;
            };
            // ## m4b-t20-adversarial
            let mut batch = vec![RawHint::Path(root.path.clone())];
            if root.git != git {
                batch.push(RawHint::Path(root.path.join(".git").join("index")));
            }
            root.git = git;
            if root.manifest != manifest {
                batch.push(RawHint::Path(
                    root.path.join(".hacivat").join("workspace.toml"),
                ));
            }
            root.manifest = manifest;
            (root.sink)(batch);
            root.last_pass = Instant::now();
            root.next = root.last_pass + root.interval;
        }
    }
}

impl fmt::Debug for PollEventSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PollEventSource").finish_non_exhaustive()
    }
}

impl EventSource for PollEventSource {
    fn watch(&mut self, root: &Path, sink: HintSink) -> Result<(), WatchError> {
        // ## m4b-cifix-3-manifest-poll (CI 45068616161): the baselines are taken here, before
        // the caller's first scan reads the manifest and the status, never at the first pass:
        // a change between that read and the first pass (up to a whole interval) was recorded
        // as the baseline and never reported.
        let git = repository_stamps(root);
        let manifest = manifest_stamp(root);
        let mut st = self.shared.lock();
        if st.stopped {
            // ## m4b-gatefix-1 (E9): a stopped source says so (the variant had a stub name).
            return Err(WatchError::Stopped);
        }
        st.roots.retain(|r| r.path != root);
        let now = Instant::now();
        st.roots.push(Root {
            path: root.to_path_buf(),
            sink,
            interval: self.base,
            last_pass: now,
            next: now + self.base,
            git,
            manifest,
        });
        drop(st);
        self.shared.wake.notify_all();
        Ok(())
    }

    fn unwatch(&mut self, root: &Path) {
        self.shared.lock().roots.retain(|r| r.path != root);
    }

    fn pace(&mut self, root: &Path, changed: bool, cost: Duration) {
        let (base, max) = (self.base, self.max);
        let mut st = self.shared.lock();
        if let Some(r) = st.roots.iter_mut().find(|r| r.path == root) {
            r.interval = next_interval(r.interval, changed, cost, base, max);
            r.next = r.last_pass + r.interval;
        }
        drop(st);
        self.shared.wake.notify_all();
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

// ## m4b-t20-adversarial (named exception, the cifix-3 review's note)
/// A pass about to look at a root (unit tests stall it there, as a hung share would).
#[cfg(test)]
fn stall_hook(root: &Path) {
    let rx = {
        let mut hook = STALL.lock().unwrap_or_else(PoisonError::into_inner);
        match hook.take() {
            Some((at, entered, rx)) if at == root => {
                let _ = entered.send(());
                Some(rx)
            }
            other => {
                *hook = other;
                None
            }
        }
    };
    if let Some(rx) = rx {
        let _ = rx.recv();
    }
}

#[cfg(not(test))]
fn stall_hook(_: &Path) {}

/// The root a test stalls, its "entered" signal and its release.
#[cfg(test)]
type Stall = (
    PathBuf,
    std::sync::mpsc::Sender<()>,
    std::sync::mpsc::Receiver<()>,
);

#[cfg(test)]
static STALL: Mutex<Option<Stall>> = Mutex::new(None);

#[cfg(test)]
mod t20_tests {
    use super::*;
    use std::sync::mpsc;

    /// A pass stalled in a root's `lstat`s (a hung share) holds no lock: `watch`, `unwatch` and
    /// `pace` of the source go on, and the stalled root's own pass still lands afterwards.
    #[test]
    fn a_stalled_pass_never_blocks_watch_unwatch_or_pace() {
        let hung = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let (entered_tx, entered) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        *STALL.lock().unwrap() = Some((hung.path().to_path_buf(), entered_tx, release_rx));
        let mut src = PollEventSource::with_interval(Duration::from_millis(5)).unwrap();
        let (hint_tx, hints) = mpsc::channel::<Vec<RawHint>>();
        let sink: HintSink = Box::new(move |b| {
            let _ = hint_tx.send(b);
        });
        src.watch(hung.path(), sink).unwrap();
        entered
            .recv_timeout(Duration::from_secs(10))
            .expect("the pass reached the hung root");
        let other_path = other.path().to_path_buf();
        let (done_tx, done) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let noop: HintSink = Box::new(|_| {});
            src.watch(&other_path, noop).unwrap();
            src.pace(&other_path, false, Duration::ZERO);
            src.unwatch(&other_path);
            let _ = done_tx.send(());
            src
        });
        let finished = done.recv_timeout(Duration::from_secs(10));
        let _ = release.send(());
        let src = worker.join().unwrap();
        assert!(
            finished.is_ok(),
            "watch, pace and unwatch waited for a stalled pass"
        );
        hints
            .recv_timeout(Duration::from_secs(10))
            .expect("the stalled root's pass still lands");
        drop(src);
    }
}
// ## m4b-t20-adversarial
