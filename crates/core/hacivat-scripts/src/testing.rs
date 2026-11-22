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

type Sinks = Arc<Mutex<HashMap<PathBuf, HintSink>>>;

/// An event source that delivers only what its [`FakeHandle`] emits.
#[derive(Default)]
pub struct FakeEventSource {
    sinks: Sinks,
    refuse: Option<WatchError>,
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
        lock(&self.sinks).insert(root.to_path_buf(), sink);
        Ok(())
    }

    fn unwatch(&mut self, root: &Path) {
        lock(&self.sinks).remove(root);
    }
}

impl FakeHandle {
    /// Delivers `batch` to the sink watching `root`; `false` when nothing watches it.
    pub fn emit(&self, root: &Path, batch: Vec<RawHint>) -> bool {
        match lock(&self.sinks).get_mut(root) {
            Some(sink) => {
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

fn lock(sinks: &Sinks) -> std::sync::MutexGuard<'_, HashMap<PathBuf, HintSink>> {
    sinks.lock().unwrap_or_else(PoisonError::into_inner)
}
// ## m4b-t01a-seams
