//! Catalog snapshots (HV-MODEL-02; R-M4-2): an immutable map of namespace arenas published
//! through `arc-swap` with a monotonically increasing generation. Readers call
//! [`Catalog::current`] and may keep the `Arc` as long as they like; a publish shares every
//! untouched namespace by `Arc`.
//!
//! M4 Task 1a seam (spike origin `d86313b`, declarations only): [`Publisher::publish`] refuses
//! (nothing is published), [`CatalogSnapshot::fk_referencing`] and
//! [`CatalogSnapshot::resolve`] answer `Unknown`, until Task 9.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use arc_swap::ArcSwap;
use hacivat_proto::DbmsKind;

use crate::delta::CatalogDelta;
use crate::ids::{NsKey, ObjId, ObjectKey, Ref};
use crate::kind::ObjectKind;
use crate::name::{IdentCtx, Name};
use crate::namespace::{Level, ModelError, NamespaceSnapshot, Obj, Subtree, Watermark};
use crate::overlay::Overlays;
use crate::props::FacetKind;
use crate::view::{Lookup, UnknownReason};

/// The precise engine a catalog was read from (MySQL vs MariaDB, PostgreSQL vs its wire
/// forks): renderers need it where texts differ per flavor (F9, X-10).
pub type Flavor = DbmsKind;

/// What a catalog knows about its server: read once per connection, carried by every
/// generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerFacts {
    pub dbms: DbmsKind,
    pub flavor: Flavor,
    /// The server's version text, verbatim.
    pub version: Name,
    /// The version as a number (PostgreSQL `server_version_num`, MySQL `80400`); 0 if unknown.
    pub version_num: u32,
    pub ident: IdentCtx,
    /// The default collation per database.
    pub default_collation: BTreeMap<Name, Name>,
    pub standard_conforming_strings: Option<bool>,
    pub no_backslash_escapes: Option<bool>,
    pub ansi_quotes: Option<bool>,
    /// PostgreSQL `pg_is_in_recovery()` (R-M4-9: replicas refresh `Full`).
    pub in_recovery: Option<bool>,
}

impl ServerFacts {
    /// The facts of a server of which only the engine and identifier context are known yet.
    pub fn new(dbms: DbmsKind, ident: IdentCtx) -> Self {
        ServerFacts {
            dbms,
            flavor: dbms,
            version: Name::from(""),
            version_num: 0,
            ident,
            default_collation: BTreeMap::new(),
            standard_conforming_strings: None,
            no_backslash_escapes: None,
            ansi_quotes: None,
            in_recovery: None,
        }
    }
}

/// A stable, locale-free code for an introspection failure (the UI maps it to Fluent; the
/// engine's own error type lives in `hacivat-introspect`, which `hacivat-meta` may not see).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntroErrorCode {
    PermissionDenied,
    Timeout,
    Connection,
    Cancelled,
    ServerError,
    Unsupported,
    NotImplemented,
    Unknown,
}

impl IntroErrorCode {
    /// The stable machine code.
    pub const fn code(self) -> &'static str {
        match self {
            IntroErrorCode::PermissionDenied => "permission-denied",
            IntroErrorCode::Timeout => "timeout",
            IntroErrorCode::Connection => "connection",
            IntroErrorCode::Cancelled => "cancelled",
            IntroErrorCode::ServerError => "server-error",
            IntroErrorCode::Unsupported => "unsupported",
            IntroErrorCode::NotImplemented => "not-implemented",
            IntroErrorCode::Unknown => "unknown",
        }
    }
}

/// The load state of one container (a namespace, a kind group of children, a facet).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum LoadState {
    #[default]
    NotLoaded,
    Loading {
        generation: u64,
        level: Level,
    },
    Loaded {
        level: Level,
        at: SystemTime,
        watermark: Watermark,
        stale: bool,
    },
    /// Loaded, but the login could not read everything (a facet the login sees partly).
    Partial {
        reason: IntroErrorCode,
    },
    Failed {
        at: SystemTime,
        error: IntroErrorCode,
        retry_after: Duration,
    },
    OutOfScope,
}

/// What a [`LoadState`] belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ContainerKey {
    /// A whole namespace.
    Namespace(NsKey),
    /// The children of `kind` under `parent` (a lazily loaded kind group).
    Children {
        ns: NsKey,
        parent: ObjectKey,
        kind: ObjectKind,
    },
    /// One facet of one object.
    Facet {
        ns: NsKey,
        object: ObjectKey,
        facet: FacetKind,
    },
}

/// A handle to one object of one generation: the namespace it lives in and its arena index.
#[derive(Debug, Clone)]
pub struct ObjHandle {
    pub ns: Arc<NamespaceSnapshot>,
    pub id: ObjId,
}

/// One immutable generation of a connection's catalog.
#[derive(Debug, Clone)]
pub struct CatalogSnapshot {
    pub generation: u64,
    pub server: Arc<ServerFacts>,
    pub namespaces: Arc<BTreeMap<NsKey, Arc<NamespaceSnapshot>>>,
    pub states: Arc<BTreeMap<ContainerKey, LoadState>>,
    pub overlays: Arc<Overlays>,
}

impl CatalogSnapshot {
    /// An empty generation 0 of `server`.
    pub fn empty(server: Arc<ServerFacts>) -> Self {
        CatalogSnapshot {
            generation: 0,
            server,
            namespaces: Arc::new(BTreeMap::new()),
            states: Arc::new(BTreeMap::new()),
            overlays: Arc::new(Overlays::default()),
        }
    }

    /// The object `key` denotes in this generation. Task 1a stub: none.
    pub fn find(&self, key: &ObjectKey) -> Option<ObjHandle> {
        let _ = key;
        None
    }

    /// Resolves a reference on demand. Task 1a stub: `Unknown(NotLoaded)`.
    pub fn resolve(&self, r: &Ref) -> Lookup<ObjHandle> {
        crate::resolve::resolve_ref(self, r)
    }

    /// The object behind a generation-local id. Task 1a stub: none.
    pub fn obj(&self, id: ObjId) -> Option<&Obj> {
        let _ = id;
        None
    }

    /// The foreign keys that reference `target` (reverse FK edges, A-13).
    /// Task 1a stub: `Unknown(NotLoaded)`.
    pub fn fk_referencing(&self, target: &ObjectKey) -> Lookup<Vec<ObjHandle>> {
        let _ = target;
        Lookup::Unknown(UnknownReason::NotLoaded)
    }
}

/// An incremental update to the catalog; a publish applies a list all-or-nothing.
#[derive(Debug, Clone)]
pub enum CatalogChange {
    /// Inserts or replaces a whole namespace (a full or incremental pass; a cache load).
    Put(Arc<NamespaceSnapshot>),
    /// Removes a namespace (a dropped schema).
    Remove(NsKey),
    /// Replaces the child of `parent` that is the same object as `subtree`'s root, or inserts
    /// it (a fragment refresh).
    UpsertSubtree {
        ns: NsKey,
        parent: ObjectKey,
        subtree: Subtree,
    },
    /// Removes an object and everything under it.
    RemoveSubtree { ns: NsKey, target: ObjectKey },
    /// Sets the load state of a container.
    SetState(ContainerKey, LoadState),
}

/// What a publish produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    pub generation: u64,
    /// The namespaces that changed, in change order.
    pub changed: Vec<NsKey>,
}

/// What subscribers hear (HV-MODEL-02 step 4).
#[derive(Debug, Clone)]
pub enum CatalogEvent {
    Published {
        generation: u64,
        delta: Arc<CatalogDelta>,
    },
    NamespaceReplaced {
        generation: u64,
        ns: NsKey,
    },
    /// A background pass found a mass deletion and kept it pending (D-039, R-M4-22).
    PendingConfirmation {
        ns: NsKey,
        count: u32,
    },
}

/// The capacity of the event channel; a slow subscriber lags and re-reads `current()`.
#[cfg(feature = "catalog")]
const EVENT_CAPACITY: usize = 64;

/// One connection profile's catalog: the current [`CatalogSnapshot`] behind an `ArcSwap`, a
/// single writer, and (feature `catalog`) a broadcast of [`CatalogEvent`]s.
pub struct Catalog {
    current: Arc<ArcSwap<CatalogSnapshot>>,
    writer_taken: AtomicBool,
    #[cfg(feature = "catalog")]
    events: tokio::sync::broadcast::Sender<CatalogEvent>,
}

impl std::fmt::Debug for Catalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Catalog")
            .field("generation", &self.current.load().generation)
            .finish_non_exhaustive()
    }
}

impl Catalog {
    /// An empty catalog of `server` at generation 0.
    pub fn new(server: Arc<ServerFacts>) -> Self {
        Catalog {
            current: Arc::new(ArcSwap::from_pointee(CatalogSnapshot::empty(server))),
            writer_taken: AtomicBool::new(false),
            #[cfg(feature = "catalog")]
            events: tokio::sync::broadcast::channel(EVENT_CAPACITY).0,
        }
    }

    /// The current generation. Lock-free; keep the `Arc` to pin a generation.
    pub fn current(&self) -> Arc<CatalogSnapshot> {
        self.current.load_full()
    }

    /// The catalog's single writer: `Some` the first time, `None` after.
    pub fn publisher(&self) -> Option<Publisher> {
        if self.writer_taken.swap(true, Ordering::AcqRel) {
            return None;
        }
        Some(Publisher {
            current: Arc::clone(&self.current),
            #[cfg(feature = "catalog")]
            events: self.events.clone(),
        })
    }

    /// Subscribes to the catalog's events.
    #[cfg(feature = "catalog")]
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<CatalogEvent> {
        self.events.subscribe()
    }
}

/// The single writer of one [`Catalog`] (the HV-INTRO-01 scheduler). Not `Clone`.
pub struct Publisher {
    current: Arc<ArcSwap<CatalogSnapshot>>,
    #[cfg(feature = "catalog")]
    events: tokio::sync::broadcast::Sender<CatalogEvent>,
}

impl std::fmt::Debug for Publisher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Publisher")
            .field("generation", &self.current.load().generation)
            .finish_non_exhaustive()
    }
}

impl Publisher {
    /// Applies `changes` atomically: on any error nothing is published.
    ///
    /// Task 1a stub: `Err(ModelError::NotImplemented)`; the current generation stays.
    pub fn publish(&self, changes: Vec<CatalogChange>) -> Result<Published, ModelError> {
        let _ = changes;
        #[cfg(feature = "catalog")]
        let _ = &self.events;
        Err(ModelError::NotImplemented)
    }
}
