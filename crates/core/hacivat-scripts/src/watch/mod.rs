//! The library watcher (HV-SCRIPT-08; R-M4b-14, -16): OS events through `notify` and
//! `notify-debouncer-full`, or polling, reduced to dirty-path hints that the service reconciles
//! against the disk (`lstat`, `DiskStamp`, settle). No `notify` type is public.
//!
//! ## m4b-t01a-seams
//! Task 1a: the [`EventSource`] seam (`testing::FakeEventSource` drives the deterministic
//! tests) and the one way the OS debouncer is built, fixed by R-T0-2 (alg/28 C5, ADR 0009 D3):
//! `new_debouncer_opt::<_, RecommendedWatcher, NoCache>` with
//! `Config::default().with_follow_symlinks(false)`. Never `new_debouncer`, whose default
//! `FileIdMap` cache walks every added path with `follow_links(true)`, and never a following
//! config (`WatcherFollowsLinks`, `cargo xtask check-deps`). [`OsEventSource::watch`] is a
//! fail-closed stub that watches nothing until Task 9 (`debounce`, `poll`, `limits`, `echo`).

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::RecommendedWatcher;
use notify_debouncer_full::{DebounceEventResult, Debouncer, NoCache, new_debouncer_opt};

/// The debounce window (alg/28 HV-SCRIPT-08 Decision).
pub const DEBOUNCE: Duration = Duration::from_millis(150);

/// One hint from an event source. Kinds are never trusted (alg/28 C3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawHint {
    /// Something may have changed at this absolute path.
    Path(PathBuf),
    /// Events were dropped or coalesced under this absolute path: rescan it.
    Rescan(PathBuf),
    /// The backend stopped working; the service falls back to polling.
    Lost,
}

/// Where an event source delivers its batches of hints (one batch per debounce window).
pub type HintSink = Box<dyn FnMut(Vec<RawHint>) + Send>;

/// Why a watch did not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchError {
    /// Task 1a's stub watches nothing.
    NotImplemented,
    /// The backend refused (the text is the backend's, never file contents).
    Backend(String),
}

/// A source of hints for library roots: the OS watcher, the poller, or a test fake.
pub trait EventSource: Send {
    /// Starts delivering hints for everything under `root` to `sink`.
    fn watch(&mut self, root: &Path, sink: HintSink) -> Result<(), WatchError>;
    /// Stops delivering hints for `root`; unknown roots are ignored.
    fn unwatch(&mut self, root: &Path);
}

/// The configuration every OS and poll watcher is built with: directory links are never
/// followed (R-T0-2).
pub(crate) fn watcher_config() -> notify::Config {
    notify::Config::default().with_follow_symlinks(false)
}

/// The OS event source: one debouncer over the platform's recommended watcher.
pub struct OsEventSource {
    debouncer: Debouncer<RecommendedWatcher, NoCache>,
}

impl OsEventSource {
    /// Builds the debouncer the only allowed way (R-T0-2). It watches nothing yet.
    pub fn new() -> Result<Self, WatchError> {
        let handler = |_batch: DebounceEventResult| {};
        let debouncer = new_debouncer_opt::<_, RecommendedWatcher, NoCache>(
            DEBOUNCE,
            None,
            handler,
            NoCache,
            watcher_config(),
        )
        .map_err(|e| WatchError::Backend(e.to_string()))?;
        Ok(OsEventSource { debouncer })
    }
}

impl fmt::Debug for OsEventSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OsEventSource").finish_non_exhaustive()
    }
}

impl EventSource for OsEventSource {
    /// Task 1a stub (fail closed): watches nothing and never calls `sink`.
    fn watch(&mut self, root: &Path, sink: HintSink) -> Result<(), WatchError> {
        let _ = (&mut self.debouncer, root, sink);
        Err(WatchError::NotImplemented)
    }

    fn unwatch(&mut self, root: &Path) {
        let _ = root;
    }
}
// ## m4b-t01a-seams

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn follow_symlinks_is_off() {
        assert!(!watcher_config().follow_symlinks());
        assert_eq!(DEBOUNCE, Duration::from_millis(150));
    }

    #[test]
    fn os_source_builds_and_its_stub_watches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let mut src = OsEventSource::new().unwrap();
        let r = src.watch(
            dir.path(),
            Box::new(move |_| {
                seen.fetch_add(1, Ordering::SeqCst);
            }),
        );
        assert_eq!(r, Err(WatchError::NotImplemented));
        std::fs::write(dir.path().join("a.sql"), "SELECT 1").unwrap();
        src.unwatch(dir.path());
        drop(src);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
