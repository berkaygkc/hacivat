//! Test doubles (feature `test-support`; never in a product build).
//!
//! ## m4b-t01a-seams
//! [`FakeEventSource`]: the injected [`EventSource`] the watcher's deterministic tests use
//! (Global Constraints "Test timing": real-watcher tests are Task 9's `watch_real` only). A test
//! keeps a [`FakeHandle`] and pushes hint batches by hand; nothing touches the OS.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use crate::names::LogicalDb;
use crate::watch::{EventSource, HintSink, RawHint, WatchError};

// ## m4b-t20-adversarial fix round 2 (named lines): each sink with the id of the source that
// registered it, so a dropped source's late `unwatch` never removes its successor's sink (the
// OS backends are per instance; this fake shared one map by path).
type Sinks = Arc<Mutex<HashMap<PathBuf, (u64, HintSink)>>>;

static NEXT_SOURCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_source() -> u64 {
    NEXT_SOURCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// An event source that delivers only what its [`FakeHandle`] emits.
#[derive(Default)]
pub struct FakeEventSource {
    sinks: Sinks,
    refuse: Option<WatchError>,
    id: u64,
}

/// The test's side of a [`FakeEventSource`].
#[derive(Clone, Default)]
pub struct FakeHandle {
    sinks: Sinks,
}

impl FakeEventSource {
    /// A source and the handle that drives it.
    pub fn new() -> (Self, FakeHandle) {
        let sinks: Sinks = Arc::default();
        (
            FakeEventSource {
                sinks: sinks.clone(),
                refuse: None,
                id: next_source(),
            },
            FakeHandle { sinks },
        )
    }

    /// A source whose every `watch` fails with `err` (a backend that will not start).
    pub fn refusing(err: WatchError) -> (Self, FakeHandle) {
        let (mut src, handle) = Self::new();
        src.refuse = Some(err);
        (src, handle)
    }
}

impl EventSource for FakeEventSource {
    fn watch(&mut self, root: &Path, sink: HintSink) -> Result<(), WatchError> {
        if let Some(err) = &self.refuse {
            return Err(err.clone());
        }
        lock(&self.sinks).insert(root.to_path_buf(), (self.id, sink));
        Ok(())
    }

    fn unwatch(&mut self, root: &Path) {
        let mut sinks = lock(&self.sinks);
        if sinks.get(root).is_some_and(|(id, _)| *id == self.id) {
            sinks.remove(root);
        }
    }
}

impl FakeHandle {
    /// Delivers `batch` to the sink watching `root`; `false` when nothing watches it.
    pub fn emit(&self, root: &Path, batch: Vec<RawHint>) -> bool {
        match lock(&self.sinks).get_mut(root) {
            Some((_, sink)) => {
                sink(batch);
                true
            }
            None => false,
        }
    }

    /// The roots being watched, sorted.
    pub fn watched(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = lock(&self.sinks).keys().cloned().collect();
        roots.sort();
        roots
    }

    // ## m4b-t10-service (named exception; the watcher wiring): the library service starts one
    // `Watcher` per library, each owning its source; these give every one of them a fake
    // source that this handle drives.
    /// Sources for `LibraryService::open_with`: each library's watcher gets a fake source that
    /// delivers what this handle emits.
    pub fn sources(&self) -> crate::service::Sources {
        let sinks = self.sinks.clone();
        crate::service::Sources::Custom(Arc::new(move || {
            Ok(Box::new(FakeEventSource {
                sinks: sinks.clone(),
                refuse: None,
                id: next_source(),
            }) as Box<dyn EventSource>)
        }))
    }

    /// Sources whose every `watch` fails with `err` (a backend that will not start).
    pub fn refusing_sources(&self, err: WatchError) -> crate::service::Sources {
        let sinks = self.sinks.clone();
        crate::service::Sources::Custom(Arc::new(move || {
            Ok(Box::new(FakeEventSource {
                sinks: sinks.clone(),
                refuse: Some(err.clone()),
                id: next_source(),
            }) as Box<dyn EventSource>)
        }))
    }
    // ## m4b-t10-service
}

/// A logical name without the character rules (`LogicalDb::parse` refuses every name until
/// Task 3); for building fixtures only.
pub fn logical(name: &str) -> LogicalDb {
    LogicalDb(name.into())
}

fn lock(sinks: &Sinks) -> std::sync::MutexGuard<'_, HashMap<PathBuf, (u64, HintSink)>> {
    sinks.lock().unwrap_or_else(PoisonError::into_inner)
}
// ## m4b-t01a-seams
