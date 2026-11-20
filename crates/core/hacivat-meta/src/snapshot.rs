//! Catalog snapshots (HV-MODEL-02; R-M4-2): an immutable map of namespace arenas published
//! through `arc-swap` with a monotonically increasing generation. Readers call
//! [`Catalog::current`] (one atomic load, no lock) and may keep the `Arc` as long as they like;
//! a publish shares every untouched namespace by `Arc` (`Arc::ptr_eq` holds across
//! generations), so its cost is O(namespaces) pointer copies plus the rebuilt namespaces.
//!
//! M4 Task 9 (spike origin `d86313b`): [`Publisher::publish`] applies a change list all or
//! nothing (alg/02 HV-MODEL-02 steps 1, 3, 4); references resolve on demand
//! ([`CatalogSnapshot::resolve`], [`crate::resolve`]); reverse FK edges are built lazily once
//! per generation ([`CatalogSnapshot::fk_referencing`]). alg/02's publish-time reverse-index
//! re-resolution (step 2) is not built (R-M4-26).
//!
//! **Completeness.** A miss is a fact only where the catalog holds the whole container: a
//! namespace whose [`LoadState`] is `Loaded` at [`Level::Structure`] or above, and, for "no
//! such namespace", a loaded schema list ([`ContainerKey::schema_list`]). Everything else
//! answers `Unknown` (Global Constraints, "No silent weakening").

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::time::{Duration, SystemTime};

use arc_swap::ArcSwap;
use hacivat_proto::DbmsKind;

use crate::delta::CatalogDelta;
use crate::ids::{NsKey, NsKind, ObjId, ObjectKey, Ref, SourceRef};
use crate::kind::{ObjectKind, pg_family};
use crate::name::{IdentCtx, Name};
use crate::namespace::{Level, ModelError, NamespaceSnapshot, Obj, Subtree, Watermark};
use crate::overlay::Overlays;
use crate::props::{DepsFacet, Facet, FacetKind, GrantsFacet, Props, StatsFacet};
use crate::resolve::{
    NsStatus, locate_ns_of, locate_ref, miss_is_fact, ns_status, schema_list_loaded,
};
use crate::view::{Lookup, MissingReason, UnknownReason, note_slow_path};

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
    /// The list of schemas of one database (`ns` = the database namespace) or of the server
    /// (`ns` = the server namespace: MySQL/MariaDB databases, SQLite's attached files). Built by
    /// [`ContainerKey::schema_list`]. Distinct from every `Children` key, so "the schema list
    /// is loaded" never reads as "this database has no schema children" (T9 fix round 1, I5).
    SchemaList { ns: NsKey },
}

impl ContainerKey {
    /// The container of the schema list: the schemas of `database` (PostgreSQL), or of the
    /// server where schemas are top level (MySQL/MariaDB databases, SQLite's attached files:
    /// `None`).
    ///
    /// When it is `Loaded`, every schema the server has is in the catalog's namespaces or
    /// states, so a schema that neither holds does not exist: a qualified name into it is
    /// `Missing`, and a search-path entry naming it is skipped (PostgreSQL skips a `search_path`
    /// schema that does not exist, `$user` most often). Otherwise such a lookup is
    /// `Unknown(NotLoaded)`. `children(database, Schema)` answers from the listed schema
    /// namespaces. The engine sets it after each scope refresh (Task 15).
    pub fn schema_list(database: Option<&Name>) -> ContainerKey {
        ContainerKey::SchemaList {
            ns: match database {
                Some(db) => NsKey::database(db.clone()),
                None => NsKey::server(),
            },
        }
    }

    /// The namespace the container belongs to.
    pub fn ns(&self) -> &NsKey {
        match self {
            ContainerKey::Namespace(ns)
            | ContainerKey::Children { ns, .. }
            | ContainerKey::Facet { ns, .. }
            | ContainerKey::SchemaList { ns } => ns,
        }
    }
}

/// A handle to one object of one generation: the namespace it lives in and its arena index.
#[derive(Debug, Clone)]
pub struct ObjHandle {
    pub ns: Arc<NamespaceSnapshot>,
    pub id: ObjId,
}

impl ObjHandle {
    /// The object. A handle is only made for an index inside its arena.
    pub fn obj(&self) -> &Obj {
        &self.ns.objs()[self.id.idx as usize]
    }

    /// The object's stable key.
    pub fn key(&self) -> Option<ObjectKey> {
        self.ns.object_key(self.id.idx)
    }
}

/// The namespaces of one generation in key order, and each one's position (an [`ObjId`]'s
/// `ns`). Built once per generation, on first use.
#[derive(Debug)]
struct NsIndex {
    /// `Arc::as_ptr` of the map it was built from: a snapshot whose public `namespaces` field
    /// was replaced after the index was built must not use it.
    src: usize,
    order: Vec<Arc<NamespaceSnapshot>>,
    pos: HashMap<NsKey, u32>,
}

/// Reverse FK edges of one generation: target `(ns position, idx)` → the FK objects.
#[derive(Debug)]
struct FkIndex {
    src: usize,
    edges: HashMap<(u32, u32), Vec<(u32, u32)>>,
}

/// One immutable generation of a connection's catalog.
#[derive(Clone)]
pub struct CatalogSnapshot {
    pub generation: u64,
    pub server: Arc<ServerFacts>,
    pub namespaces: Arc<BTreeMap<NsKey, Arc<NamespaceSnapshot>>>,
    pub states: Arc<BTreeMap<ContainerKey, LoadState>>,
    pub overlays: Arc<Overlays>,
    // ## m4-t15b-facets (named exception in Task 9's file)
    /// Facet values and source texts (R-M4-30): side tables beside the arenas.
    side: Arc<SideTables>,
    // ## m4-t15b-facets
    index: OnceLock<Arc<NsIndex>>,
    fks: OnceLock<Arc<FkIndex>>,
}

impl fmt::Debug for CatalogSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CatalogSnapshot")
            .field("generation", &self.generation)
            .field("dbms", &self.server.dbms)
            .field("namespaces", &self.namespaces.len())
            .field("states", &self.states.len())
            .finish_non_exhaustive()
    }
}

impl CatalogSnapshot {
    /// An empty generation 0 of `server`.
    pub fn empty(server: Arc<ServerFacts>) -> Self {
        CatalogSnapshot::from_parts(
            0,
            server,
            Arc::new(BTreeMap::new()),
            Arc::new(BTreeMap::new()),
            Arc::new(Overlays::default()),
            Arc::new(SideTables::default()),
        )
    }

    fn from_parts(
        generation: u64,
        server: Arc<ServerFacts>,
        namespaces: Arc<BTreeMap<NsKey, Arc<NamespaceSnapshot>>>,
        states: Arc<BTreeMap<ContainerKey, LoadState>>,
        overlays: Arc<Overlays>,
        side: Arc<SideTables>,
    ) -> Self {
        CatalogSnapshot {
            generation,
            server,
            namespaces,
            states,
            overlays,
            side,
            index: OnceLock::new(),
            fks: OnceLock::new(),
        }
    }

    /// The load state of `key` (`NotLoaded` when the catalog holds none).
    pub fn state(&self, key: &ContainerKey) -> &LoadState {
        const NOT_LOADED: LoadState = LoadState::NotLoaded;
        self.states.get(key).unwrap_or(&NOT_LOADED)
    }

    fn src(&self) -> usize {
        Arc::as_ptr(&self.namespaces) as usize
    }

    /// This generation's namespace index, if it was built from the current `namespaces` map.
    fn index(&self) -> Option<&NsIndex> {
        let idx = self.index.get_or_init(|| {
            note_slow_path();
            let order: Vec<_> = self.namespaces.values().cloned().collect();
            let pos = order
                .iter()
                .enumerate()
                .map(|(i, n)| (n.key().clone(), i as u32))
                .collect();
            Arc::new(NsIndex {
                src: self.src(),
                order,
                pos,
            })
        });
        (idx.src == self.src()).then_some(&**idx)
    }

    /// The position of namespace `key` in this generation (`u32::MAX` when it is not here).
    fn position(&self, key: &NsKey) -> u32 {
        match self.index() {
            Some(idx) => idx.pos.get(key).copied().unwrap_or(u32::MAX),
            None => self
                .namespaces
                .keys()
                .position(|k| k == key)
                .map_or(u32::MAX, |p| p as u32),
        }
    }

    /// The namespace at `position`.
    fn at(&self, position: u32) -> Option<&Arc<NamespaceSnapshot>> {
        match self.index() {
            Some(idx) => idx.order.get(position as usize),
            None => self.namespaces.values().nth(position as usize),
        }
    }

    /// A handle to object `idx` of `ns` (a namespace of this generation).
    pub(crate) fn handle(&self, ns: &Arc<NamespaceSnapshot>, idx: u32) -> ObjHandle {
        ObjHandle {
            ns: Arc::clone(ns),
            id: ObjId {
                ns: self.position(ns.key()),
                idx,
            },
        }
    }

    /// The object `key` denotes in this generation ([`ObjectKey::same_object`]: by server id,
    /// else by exact path), searched in the namespace that holds its path. `None` says only
    /// that no arena of this generation holds it — not that the server lacks it (the tri-state
    /// answers are [`CatalogSnapshot::resolve`] and the [`crate::view::ModelView`]).
    pub fn find(&self, key: &ObjectKey) -> Option<ObjHandle> {
        for k in (0..=key.path.len()).rev() {
            for kind in [NsKind::Schema, NsKind::Database, NsKind::Server] {
                let ns_key = NsKey {
                    kind,
                    path: key.path[..k].iter().cloned().collect(),
                };
                if let Some(ns) = self.namespaces.get(&ns_key)
                    && let Some(idx) = ns.find(key)
                {
                    return Some(self.handle(ns, idx));
                }
            }
        }
        None
    }

    /// Resolves a reference on demand (R-M4-26): the target namespace by path, then the name
    /// index of the target's kind (or the parent's children for a parent-scoped kind), the
    /// server id preferred among same-named hits. `Found` when an arena holds it; `Missing`
    /// only when the target namespace is complete; `Unknown` otherwise.
    pub fn resolve(&self, r: &Ref) -> Lookup<ObjHandle> {
        crate::resolve::resolve_ref(self, r)
    }

    /// The object behind a generation-local id (`None` for an id of another generation that
    /// points outside this one).
    pub fn obj(&self, id: ObjId) -> Option<&Obj> {
        self.at(id.ns)?.obj(id.idx)
    }

    /// The foreign keys that reference `target` (reverse FK edges, A-13), across namespaces.
    /// The edges are built once per generation, on first use. `Unknown` while the schema list
    /// is not loaded or any in-scope namespace that could hold a referencing FK (PostgreSQL:
    /// the target's database; elsewhere every schema) is below `Structure`, loading or failed;
    /// out-of-scope namespaces are not part of the answer. `Missing` when the target itself is
    /// absent from its complete namespace.
    pub fn fk_referencing(&self, target: &ObjectKey) -> Lookup<Vec<ObjHandle>> {
        let database = if pg_family(self.server.dbms) {
            match target.path.first() {
                Some(db) => Some(db),
                None => return Lookup::Unknown(UnknownReason::NotLoaded),
            }
        } else {
            None
        };
        if !schema_list_loaded(self, database) {
            return Lookup::Unknown(UnknownReason::NotLoaded);
        }
        let in_reach = |k: &NsKey| {
            k.kind == NsKind::Schema && database.is_none_or(|db| k.path.first() == Some(db))
        };
        let listed = self
            .namespaces
            .keys()
            .chain(self.states.keys().filter_map(|k| match k {
                ContainerKey::Namespace(ns) => Some(ns),
                _ => None,
            }));
        for k in listed.filter(|k| in_reach(k)) {
            match ns_status(self, k).status {
                NsStatus::Complete | NsStatus::OutOfScope => {}
                NsStatus::Partial { reason, .. } => return Lookup::Unknown(reason),
            }
        }
        let Some(hit) = self.find(target) else {
            let Some(located) = locate_ns_of(self, &target.path) else {
                return Lookup::Unknown(UnknownReason::NotLoaded);
            };
            return match ns_status(self, &located).status {
                NsStatus::Complete if miss_is_fact(self, &located, &target.path) => {
                    Lookup::Missing(MissingReason::NotFound)
                }
                NsStatus::Complete => Lookup::Unknown(UnknownReason::NotLoaded),
                NsStatus::OutOfScope => Lookup::Unknown(UnknownReason::OutOfScope),
                NsStatus::Partial { reason, .. } => Lookup::Unknown(reason),
            };
        };
        let edges = self.fk_edges();
        let from = edges
            .as_deref()
            .and_then(|e| e.edges.get(&(hit.id.ns, hit.id.idx)));
        Lookup::Found(
            from.into_iter()
                .flatten()
                .filter_map(|&(pos, idx)| self.at(pos).map(|ns| self.handle(ns, idx)))
                .collect(),
        )
    }

    /// This generation's reverse FK edges (`None` only if `namespaces` was replaced on a clone
    /// after they were built; then they are rebuilt for the call).
    fn fk_edges(&self) -> Option<Arc<FkIndex>> {
        let cached = self.fks.get_or_init(|| Arc::new(self.build_fk_edges()));
        if cached.src == self.src() {
            Some(Arc::clone(cached))
        } else {
            Some(Arc::new(self.build_fk_edges()))
        }
    }

    fn build_fk_edges(&self) -> FkIndex {
        let mut edges: HashMap<(u32, u32), Vec<(u32, u32)>> = HashMap::new();
        for (pos, ns) in self.namespaces.values().enumerate() {
            for (idx, obj) in ns.iter_named(&[ObjectKind::ForeignKey]) {
                let Props::ForeignKey(fk) = obj.props() else {
                    continue;
                };
                if let Some((target_ns, target_idx)) = locate_ref(self, &fk.target).hit() {
                    let tpos = self.position(target_ns.key());
                    edges
                        .entry((tpos, target_idx))
                        .or_default()
                        .push((pos as u32, idx));
                }
            }
        }
        FkIndex {
            src: self.src(),
            edges,
        }
    }
}

/// An incremental update to the catalog; a publish applies a list all-or-nothing.
#[derive(Debug, Clone)]
pub enum CatalogChange {
    /// Inserts or replaces a whole namespace (a full or incremental pass; a cache load). The
    /// publisher stamps the arena with the new generation; hand it a unique `Arc` (a shared one
    /// is copied — by reference count for its objects — to be stamped).
    Put(Arc<NamespaceSnapshot>),
    /// Removes a namespace (a dropped schema) and every state of its containers.
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
    // ## m4-t15b-facets (named exception in Task 9's file; plan Task 15b)
    /// Sets one facet's load state and value (R-M4-30), a side-table change: no arena is
    /// touched, so no namespace changes generation and none is offered to the cache. `key` must
    /// be a [`ContainerKey::Facet`] whose value (when given) is of its facet kind. `NotLoaded`
    /// removes the state and the value.
    SetFacet {
        key: ContainerKey,
        state: LoadState,
        value: Option<FacetValue>,
    },
    /// Adds source texts read by a pass of `ns` (a view's, routine's or trigger's definition),
    /// each under its own [`SourceRef`] (a text whose hash is not its reference is dropped).
    /// After the publish a namespace keeps only the texts its arena still references.
    PutSources {
        ns: NsKey,
        texts: Vec<(SourceRef, Name)>,
    },
    // ## m4-t15b-facets
    // ## m4-t17-mysql (named exception in Task 9's file; T15 carry m4, R-W3-6): the facts
    // read from the server on connect replace the placeholder the catalog was made with
    // (MySQL `lower_case_table_names`, the version, `sql_mode` arrive with the connection).
    /// Replaces the catalog's server facts (carried by every later generation).
    SetServer(Arc<ServerFacts>),
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

/// What a [`Catalog`] and its [`Publisher`] share.
struct Shared {
    current: ArcSwap<CatalogSnapshot>,
    /// Every published generation, weakly: the ones still alive are pinned by a reader
    /// (alg/02 edge 1). Touched only by the writer and by `pinned_generations`.
    generations: Mutex<Vec<(u64, Weak<CatalogSnapshot>)>>,
    /// Serializes publishes through a shared `&Publisher` (the HV-INTRO-01 scheduler is the
    /// only writer; this guards misuse). Readers never take it.
    writer: Mutex<()>,
    #[cfg(feature = "catalog")]
    events: tokio::sync::broadcast::Sender<CatalogEvent>,
}

impl Shared {
    fn send(&self, event: CatalogEvent) {
        #[cfg(feature = "catalog")]
        {
            // No subscriber is not an error: they re-read `current()` when they come.
            let _ = self.events.send(event);
        }
        #[cfg(not(feature = "catalog"))]
        let _ = event;
    }
}

/// One connection profile's catalog: the current [`CatalogSnapshot`] behind an `ArcSwap`, a
/// single writer, and (feature `catalog`) a broadcast of [`CatalogEvent`]s.
pub struct Catalog {
    shared: Arc<Shared>,
    writer_taken: AtomicBool,
}

impl fmt::Debug for Catalog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Catalog")
            .field("generation", &self.shared.current.load().generation)
            .finish_non_exhaustive()
    }
}

impl Catalog {
    /// An empty catalog of `server` at generation 0.
    pub fn new(server: Arc<ServerFacts>) -> Self {
        let first = Arc::new(CatalogSnapshot::empty(server));
        Catalog {
            shared: Arc::new(Shared {
                generations: Mutex::new(vec![(0, Arc::downgrade(&first))]),
                current: ArcSwap::new(first),
                writer: Mutex::new(()),
                #[cfg(feature = "catalog")]
                events: tokio::sync::broadcast::channel(EVENT_CAPACITY).0,
            }),
            writer_taken: AtomicBool::new(false),
        }
    }

    /// The current generation. Lock-free; keep the `Arc` to pin a generation.
    pub fn current(&self) -> Arc<CatalogSnapshot> {
        self.shared.current.load_full()
    }

    /// The catalog's single writer: `Some` the first time, `None` after.
    pub fn publisher(&self) -> Option<Publisher> {
        if self.writer_taken.swap(true, Ordering::AcqRel) {
            return None;
        }
        Some(Publisher {
            shared: Arc::clone(&self.shared),
        })
    }

    /// Subscribes to the catalog's events.
    #[cfg(feature = "catalog")]
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<CatalogEvent> {
        self.shared.events.subscribe()
    }

    /// The generations still alive, oldest first: the current one, and every older one a
    /// reader still holds (alg/02 edge 1: a schema compare pinning generation N keeps the
    /// namespaces N had until it lets go). A diagnostic; never on a read path.
    pub fn pinned_generations(&self) -> Vec<u64> {
        let mut gens = self
            .shared
            .generations
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        gens.retain(|(_, w)| w.strong_count() > 0);
        gens.iter().map(|(g, _)| *g).collect()
    }
}

/// The single writer of one [`Catalog`] (the HV-INTRO-01 scheduler). Not `Clone`.
pub struct Publisher {
    shared: Arc<Shared>,
}

impl fmt::Debug for Publisher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Publisher")
            .field("generation", &self.shared.current.load().generation)
            .finish_non_exhaustive()
    }
}

impl Publisher {
    /// Applies `changes` atomically (alg/02 HV-MODEL-02 steps 1, 3, 4): a shallow clone of the
    /// current generation, every change applied in order, `generation + 1`, one store, then the
    /// broadcast. On any error nothing is published and nothing is announced. Untouched
    /// namespaces (and the states map, when no state changes) stay `Arc::ptr_eq`. An empty list
    /// publishes nothing and answers the current generation.
    ///
    /// Without a delta from the merge (Task 10), subscribers hear `Published` with an empty
    /// delta and one `NamespaceReplaced` per changed namespace.
    ///
    /// **Cost of a shared arena:** `Put` stamps the arena with the new generation, so a `Put`
    /// whose `Arc` the caller still shares copies the arena (its `Vec<Obj>` and name indexes;
    /// the objects' props by reference count) under the writer lock. The engine hands over
    /// unique arenas (a Task 15 carry); a unique `Arc` is moved, never copied.
    pub fn publish(&self, changes: Vec<CatalogChange>) -> Result<Published, ModelError> {
        self.apply(changes, None)
    }

    /// As [`Publisher::publish`], announcing `delta` (the merge's, HV-INTRO-11) instead of
    /// whole namespaces.
    pub fn publish_with_delta(
        &self,
        changes: Vec<CatalogChange>,
        delta: Arc<CatalogDelta>,
    ) -> Result<Published, ModelError> {
        self.apply(changes, Some(delta))
    }

    /// Announces a guarded merge kept pending (D-039, R-M4-22); publishes nothing.
    pub fn announce_pending_confirmation(&self, ns: NsKey, count: u32) {
        self.shared
            .send(CatalogEvent::PendingConfirmation { ns, count });
    }

    fn apply(
        &self,
        changes: Vec<CatalogChange>,
        delta: Option<Arc<CatalogDelta>>,
    ) -> Result<Published, ModelError> {
        let _writer = self
            .shared
            .writer
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let old = self.shared.current.load_full();
        if changes.is_empty() {
            return Ok(Published {
                generation: old.generation,
                changed: Vec::new(),
            });
        }
        let generation = old.generation + 1;
        let mut namespaces = Arc::clone(&old.namespaces);
        let mut states = Arc::clone(&old.states);
        let mut server = Arc::clone(&old.server);
        // ## m4-t15b-facets
        let mut side = Arc::clone(&old.side);
        let mut texts_touched: Vec<NsKey> = Vec::new();
        // ## m4-t15b-facets
        let mut changed = Vec::new();
        let mut seen = HashSet::new();
        let mut mark = |key: &NsKey, changed: &mut Vec<NsKey>| {
            if seen.insert(key.clone()) {
                changed.push(key.clone());
            }
        };
        for change in changes {
            match change {
                CatalogChange::Put(ns) => {
                    let ns = Arc::unwrap_or_clone(ns).with_generation(generation);
                    mark(ns.key(), &mut changed);
                    Arc::make_mut(&mut namespaces).insert(ns.key().clone(), Arc::new(ns));
                }
                CatalogChange::Remove(key) => {
                    let had_arena = namespaces.contains_key(&key);
                    let had_state = states.keys().any(|k| *k.ns() == key);
                    if !had_arena && !had_state {
                        return Err(ModelError::NoSuchNamespace(key));
                    }
                    if had_arena {
                        Arc::make_mut(&mut namespaces).remove(&key);
                    }
                    if had_state {
                        Arc::make_mut(&mut states).retain(|k, _| *k.ns() != key);
                    }
                    // ## m4-t15b-facets: the namespace's facet values and texts go with it.
                    if side.holds(&key) {
                        Arc::make_mut(&mut side).remove_ns(&key);
                    }
                    mark(&key, &mut changed);
                }
                CatalogChange::UpsertSubtree {
                    ns,
                    parent,
                    subtree,
                } => {
                    let cur = namespaces
                        .get(&ns)
                        .ok_or_else(|| ModelError::NoSuchNamespace(ns.clone()))?;
                    let at = cur.find(&parent).ok_or(ModelError::NoSuchObject)?;
                    let next = cur.upsert_child(at, subtree)?.with_generation(generation);
                    Arc::make_mut(&mut namespaces).insert(ns.clone(), Arc::new(next));
                    mark(&ns, &mut changed);
                }
                CatalogChange::RemoveSubtree { ns, target } => {
                    let cur = namespaces
                        .get(&ns)
                        .ok_or_else(|| ModelError::NoSuchNamespace(ns.clone()))?;
                    let at = cur.find(&target).ok_or(ModelError::NoSuchObject)?;
                    let next = cur.replace_subtree(at, None)?.with_generation(generation);
                    Arc::make_mut(&mut namespaces).insert(ns.clone(), Arc::new(next));
                    mark(&ns, &mut changed);
                }
                CatalogChange::SetState(key, state) => {
                    Arc::make_mut(&mut states).insert(key, state);
                }
                // ## m4-t17-mysql
                CatalogChange::SetServer(facts) => server = facts,
                // ## m4-t15b-facets
                CatalogChange::SetFacet { key, state, value } => {
                    let ContainerKey::Facet { facet, .. } = &key else {
                        return Err(ModelError::BadLayout("SetFacet needs a facet key"));
                    };
                    if value.as_ref().is_some_and(|v| v.kind() != *facet) {
                        return Err(ModelError::BadLayout("SetFacet value of another facet"));
                    }
                    if state == LoadState::NotLoaded {
                        Arc::make_mut(&mut states).remove(&key);
                        if side.facets.contains_key(&key) {
                            Arc::make_mut(&mut side).facets.remove(&key);
                        }
                    } else {
                        Arc::make_mut(&mut states).insert(key.clone(), state);
                        match value {
                            Some(v) => {
                                Arc::make_mut(&mut side).facets.insert(key, v);
                            }
                            None => {
                                if side.facets.contains_key(&key) {
                                    Arc::make_mut(&mut side).facets.remove(&key);
                                }
                            }
                        }
                    }
                }
                CatalogChange::PutSources { ns, texts } => {
                    let map = Arc::make_mut(&mut side)
                        .texts
                        .entry(ns.clone())
                        .or_default();
                    let map = Arc::make_mut(map);
                    for (r, text) in texts {
                        if SourceRef::of(&text) == r {
                            map.insert(r, text);
                        }
                    }
                    texts_touched.push(ns);
                } // ## m4-t15b-facets
            }
        }
        // ## m4-t15b-facets: a namespace keeps only the texts its arena references.
        for ns in changed.iter().chain(texts_touched.iter()) {
            if !side.texts.contains_key(ns) {
                continue;
            }
            let keep = namespaces.get(ns).map(|a| referenced_sources(a));
            let side = Arc::make_mut(&mut side);
            match keep {
                Some(keep) => {
                    if let Some(map) = side.texts.get_mut(ns) {
                        Arc::make_mut(map).retain(|r, _| keep.contains(r));
                        if map.is_empty() {
                            side.texts.remove(ns);
                        }
                    }
                }
                None => {
                    side.texts.remove(ns);
                }
            }
        }
        // ## m4-t15b-facets
        let next = Arc::new(CatalogSnapshot::from_parts(
            generation,
            server,
            namespaces,
            states,
            Arc::clone(&old.overlays),
            // ## m4-t15b-facets
            side,
        ));
        // The writer lock makes this a plain store; the compare-and-swap is the misuse guard
        // alg/02 edge 2 asks for (a second writer would have moved `current`).
        let prev = self
            .shared
            .current
            .compare_and_swap(&old, Arc::clone(&next));
        if !Arc::ptr_eq(&prev, &old) {
            return Err(ModelError::NotTheWriter);
        }
        drop(prev);
        {
            let mut gens = self
                .shared
                .generations
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            gens.retain(|(_, w)| w.strong_count() > 0);
            gens.push((generation, Arc::downgrade(&next)));
        }
        let whole = delta.is_none();
        self.shared.send(CatalogEvent::Published {
            generation,
            delta: delta.unwrap_or_default(),
        });
        if whole {
            for ns in &changed {
                self.shared.send(CatalogEvent::NamespaceReplaced {
                    generation,
                    ns: ns.clone(),
                });
            }
        }
        Ok(Published {
            generation,
            changed,
        })
    }
}

// ## m4-t15b-facets (named exception in Task 9's file; plan Task 15b, R-M4-30)

/// One loaded facet's value (R-M4-30): what `Introspector::facet` read for one object.
#[derive(Debug, Clone, PartialEq)]
pub enum FacetValue {
    /// The server's own text of the object (MySQL `SHOW CREATE`, SQLite `sqlite_schema.sql`).
    Sources(Name),
    Grants(Arc<GrantsFacet>),
    Dependencies(Arc<DepsFacet>),
    Statistics(Arc<StatsFacet>),
}

impl FacetValue {
    /// The facet this value belongs to.
    pub fn kind(&self) -> FacetKind {
        match self {
            FacetValue::Sources(_) => FacetKind::Sources,
            FacetValue::Grants(_) => FacetKind::Grants,
            FacetValue::Dependencies(_) => FacetKind::Dependencies,
            FacetValue::Statistics(_) => FacetKind::Statistics,
        }
    }
}

/// What a generation keeps beside its arenas: facet values by container key (their load
/// states are in [`CatalogSnapshot::states`]), and source texts per namespace, each under its
/// own [`SourceRef`] (content-addressed: a text is never paired with another object's
/// reference). Shared by `Arc` across generations; a publish that touches neither copies
/// nothing.
#[derive(Debug, Clone, Default)]
struct SideTables {
    facets: BTreeMap<ContainerKey, FacetValue>,
    texts: BTreeMap<NsKey, Arc<HashMap<SourceRef, Name>>>,
}

impl SideTables {
    fn holds(&self, ns: &NsKey) -> bool {
        self.texts.contains_key(ns) || self.facets.keys().any(|k| k.ns() == ns)
    }

    fn remove_ns(&mut self, ns: &NsKey) {
        self.texts.remove(ns);
        self.facets.retain(|k, _| k.ns() != ns);
    }
}

/// The source references `arena`'s objects hold (views, routines, triggers).
fn referenced_sources(arena: &NamespaceSnapshot) -> HashSet<SourceRef> {
    arena
        .objs()
        .iter()
        .filter_map(|o| match o.props() {
            Props::View(p) => p.source,
            Props::Routine(p) => p.source,
            Props::Trigger(p) => p.source,
            _ => None,
        })
        .collect()
}

impl CatalogSnapshot {
    /// The facet container of `object` as this generation records it: the namespace that
    /// holds the object and the object's own key there (with its server id when it has one).
    /// `None` when no arena of this generation holds the object.
    pub fn facet_key(&self, object: &ObjectKey, facet: FacetKind) -> Option<ContainerKey> {
        let h = self.find(object)?;
        Some(ContainerKey::Facet {
            ns: h.ns.key().clone(),
            object: h.key()?,
            facet,
        })
    }

    /// Facet `facet` of `object`: its load state and its value. `NotLoaded` and no value when
    /// no arena of this generation holds the object.
    pub fn facet(&self, object: &ObjectKey, facet: FacetKind) -> (LoadState, Option<&FacetValue>) {
        match self.facet_key(object, facet) {
            Some(key) => (self.state(&key).clone(), self.side.facets.get(&key)),
            None => (LoadState::NotLoaded, None),
        }
    }

    /// The value recorded under `key` (a [`ContainerKey::Facet`]).
    pub fn facet_value(&self, key: &ContainerKey) -> Option<&FacetValue> {
        self.side.facets.get(key)
    }

    /// Every facet container that holds a value, in key order.
    pub fn loaded_facets(&self) -> impl Iterator<Item = (&ContainerKey, &FacetValue)> {
        self.side.facets.iter()
    }

    /// The text `r` names, when a pass or a `Sources` facet read of this catalog brought it
    /// (the text whose hash is `r`: never another object's).
    pub fn source_text(&self, r: &SourceRef) -> Option<Name> {
        self.side
            .texts
            .values()
            .find_map(|m| m.get(r))
            .cloned()
            .or_else(|| {
                self.side.facets.values().find_map(|v| match v {
                    FacetValue::Sources(t) if SourceRef::of(t) == *r => Some(t.clone()),
                    _ => None,
                })
            })
    }

    /// `object`'s `Sources` facet (the server's text of a table or an index; `DdlSources::
    /// server_text`), only while it is loaded and not stale.
    pub fn server_text(&self, object: &ObjectKey) -> Option<Name> {
        match self.facet(object, FacetKind::Sources) {
            (LoadState::Loaded { stale: false, .. }, Some(FacetValue::Sources(t))) => {
                Some(t.clone())
            }
            _ => None,
        }
    }

    /// `object`'s `Grants` facet as a DDL producer reads it (`DdlSources::grants`): loaded
    /// (and not stale) or partial, with its value. A stale facet (a role was renamed since)
    /// answers `NotLoaded`, so a script never silently carries grants that may be wrong;
    /// every other state carries no value.
    pub fn grants_facet(&self, object: &ObjectKey) -> Facet<GrantsFacet> {
        match self.facet(object, FacetKind::Grants) {
            (
                state @ (LoadState::Loaded { stale: false, .. } | LoadState::Partial { .. }),
                Some(FacetValue::Grants(g)),
            ) => Facet {
                state,
                value: Some(Arc::clone(g)),
            },
            (LoadState::Loaded { .. } | LoadState::Partial { .. }, _) => Facet::not_loaded(),
            (state, _) => Facet { state, value: None },
        }
    }
}
// ## m4-t15b-facets
