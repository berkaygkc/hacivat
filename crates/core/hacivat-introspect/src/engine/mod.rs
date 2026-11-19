// ## m4-t01c-core
//! The introspection engine (HV-INTRO-01; Task 15, facet loading Task 15b): one scheduler per
//! profile on the `CoreRuntime`, single-flight requests, levels, atomic publish per namespace,
//! progress. [`IntroHandle`] is how the hub, the explorer and later milestones talk to it.
//!
//! **Fail-closed stub (M4 Task 1c).** The only handle is [`IntroHandle::detached`]: a catalog
//! with no engine behind it. Every request answers a ticket resolved to
//! `IntroError::NotImplemented` and publishes nothing, so every namespace stays `NotLoaded`
//! and no facet is ever shown as loaded.

pub mod facet;

use std::sync::{Arc, Mutex};

use hacivat_meta::merge::Trigger;
use hacivat_meta::props::FacetKind;
use hacivat_meta::snapshot::ContainerKey;
use hacivat_meta::{Catalog, Level, NsKey, ObjectKey};
use tokio::sync::watch;

use crate::ddl_effects::Target;
use crate::scope::ScopePattern;
use crate::ticket::RefreshTicket;

/// What a running task is doing (a closed set: the UI shows a static phase label and a
/// determinate count, never ambient motion).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    ListNamespaces,
    Tables,
    Columns,
    Indexes,
    Constraints,
    Triggers,
    Routines,
    Types,
    Sequences,
    Comments,
    Sources,
    Facets,
    Existence,
    Validate,
}

/// One running task's progress.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TaskProgress {
    pub ns: NsKey,
    pub phase: Phase,
    pub done: u64,
    /// `None` until the total is known.
    pub total: Option<u64>,
}

/// The engine's progress: one entry per running task.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Progress {
    pub tasks: Vec<TaskProgress>,
}

/// A profile's handle on its engine (Clone; cheap).
#[derive(Debug, Clone)]
pub struct IntroHandle {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    catalog: Arc<Catalog>,
    progress: watch::Sender<Progress>,
    scope: Mutex<Option<ScopePattern>>,
}

impl IntroHandle {
    /// A handle with no engine behind it: `catalog` is served as it is (cache only) and every
    /// request is refused with `IntroError::NotImplemented`. The hub's handle until Task 15,
    /// and a test double for the UI.
    pub fn detached(catalog: Arc<Catalog>) -> Self {
        IntroHandle {
            inner: Arc::new(Inner {
                catalog,
                progress: watch::Sender::new(Progress::default()),
                scope: Mutex::new(None),
            }),
        }
    }

    /// The catalog the engine publishes into.
    pub fn catalog(&self) -> &Arc<Catalog> {
        &self.inner.catalog
    }

    /// The on-connect algorithm (HV-INTRO-01 steps 1–3). Stub: `NotImplemented`.
    pub fn on_connect(&self) -> RefreshTicket {
        RefreshTicket::not_implemented()
    }

    /// Loads a namespace on its first expansion. Stub: `NotImplemented`.
    pub fn expand(&self, ns: NsKey) -> RefreshTicket {
        let _ = ns;
        RefreshTicket::not_implemented()
    }

    /// Reads `key` at `level` (and its children). Stub: `NotImplemented`.
    pub fn elevate(&self, key: ObjectKey, level: Level) -> RefreshTicket {
        let _ = (key, level);
        RefreshTicket::not_implemented()
    }

    /// Re-reads `keys` of `ns`. Stub: `NotImplemented`.
    pub fn fragment(&self, ns: NsKey, keys: Vec<ObjectKey>) -> RefreshTicket {
        let _ = (ns, keys);
        RefreshTicket::not_implemented()
    }

    /// Refreshes `target` (a user refresh, refresh after DDL, a background check). Stub:
    /// `NotImplemented`.
    pub fn refresh(&self, target: Target, trigger: Trigger) -> RefreshTicket {
        let _ = (target, trigger);
        RefreshTicket::not_implemented()
    }

    /// Loads facet `kind` of `keys` (R-M4-30; scheduling Task 15b). Stub: `NotImplemented`;
    /// the facet stays `NotLoaded`.
    pub fn load_facet(&self, keys: Vec<ObjectKey>, kind: FacetKind) -> RefreshTicket {
        facet::load(self, keys, kind)
    }

    /// Checks `keys` against the server (HV-INTRO-15); the outcome is read with
    /// `freshness::check_state`. Stub: `NotImplemented`.
    pub fn check_state(&self, keys: Vec<ObjectKey>) -> RefreshTicket {
        let _ = keys;
        RefreshTicket::not_implemented()
    }

    /// Replaces the profile's scope (HV-INTRO-02). The stub keeps it; nothing is read.
    pub fn set_scope(&self, pattern: ScopePattern) {
        *self.inner.scope.lock().unwrap_or_else(|e| e.into_inner()) = Some(pattern);
    }

    /// The scope last set.
    pub fn scope(&self) -> Option<ScopePattern> {
        self.inner
            .scope
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Sets (or clears, `None`) the level of `container` and its children (the pills). Stub:
    /// ignored; nothing is read.
    pub fn set_level_override(&self, container: ContainerKey, level: Option<Level>) {
        let _ = (container, level);
    }

    /// The engine's progress.
    pub fn progress(&self) -> watch::Receiver<Progress> {
        self.inner.progress.subscribe()
    }

    /// Applies (`accept`) or drops a merge the mass-deletion guard held back (D-039). Stub:
    /// `NotImplemented`; nothing was held back.
    pub fn confirm_pending(&self, ns: NsKey, accept: bool) -> RefreshTicket {
        let _ = (ns, accept);
        RefreshTicket::not_implemented()
    }

    /// Stops the engine (nothing runs in the stub).
    pub fn shutdown(&self) {}
}
