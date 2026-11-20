//! The tri-state model view (HV-MODEL-02 step 5 and 6; R-M4-27 renames M4's generic trait to
//! `ModelView`). `Lookup` and its reasons are `sql_catalog`'s (one definition).
//!
//! M4 Task 9: [`SnapshotView`] answers over one pinned generation. A lookup walks the
//! dialect's search path ([`SearchPath`]). It answers `Found` from a namespace that holds the
//! name only when every earlier namespace was a certain miss; the first namespace that cannot
//! answer (not loaded, loading, failed, out of scope, server-dependent, an unnamed entry) stops
//! the walk with `Unknown` and its reason (ruling R-W2-6, amending alg/02 step 5's "Found if
//! any": the server could resolve the name in that earlier namespace). `Missing` comes only
//! from a walk of certain misses (loaded at `Structure` or above, in scope, no scope filter
//! excluding the name, the server's matching rule pinned, R-M4-41). The completeness rules are
//! [`crate::resolve`]'s.
//!
//! - **Hooks** ([`LookupHooks`], shared by every view of a catalog): a namespace the catalog
//!   lists but has not loaded at a sufficient level is reported once per namespace per
//!   generation (`on_unknown`, step 5: the engine schedules a background load); a `Missing`
//!   answer schedules at most one `on_missing_probe` per namespace per 30 s (step 6, the
//!   negative cache: a name created by another session appears without a full refresh).
//! - **Scope filters** ([`ScopeFilter`]): object filters in the scope role leave objects
//!   unread inside loaded namespaces, so what they exclude is `Unknown(OutOfScope)`, never
//!   `Missing`. The explorer's view filter never reaches the view (edge 6).
//! - **No lock and no allocation on a hit** (alg/02 "completion lookups never allocate on
//!   hits"): a probe whose key is its own text (checked against `match_key` in debug builds)
//!   goes straight to the namespace's name index; anything else takes `match_key`'s path. The
//!   hooks' mutex is taken only on an `Unknown` or `Missing` answer.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use smallvec::SmallVec;
pub use sql_catalog::lookup::{Lookup, MissingReason, UnknownReason};

use crate::ids::{NsKey, NsKind, ObjectKey, ServerId};
use crate::kind::{KindGroup, ObjectKind, mysql_family, pg_family};
use crate::name::{
    DialectId, FoldRule, IdentCtx, Name, NameClass, NamePart, has_non_ascii_cased, match_key,
};
use crate::namespace::{NamespaceSnapshot, Watermark};
use crate::overlay::Overlays;
use crate::resolve::{
    NsStatus, NsView, list_known_for, locate_ns_of, miss_is_fact, ns_status, schema_list_loaded,
};
use crate::search_path::SearchPath;
use crate::snapshot::{CatalogSnapshot, ContainerKey, LoadState, ObjHandle};

/// How fresh a namespace is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Freshness {
    /// Not loaded, failed, or out of scope: nothing can be said.
    Unknown,
    Loaded {
        at: SystemTime,
        watermark: Watermark,
        stale: bool,
    },
}

/// Lookups over the object model (M7 implements it for its working model too, X-6).
pub trait ModelView: Send + Sync {
    type Def;
    /// The object `path` names in `group`, along the view's search path.
    fn lookup(&self, path: &[NamePart], group: KindGroup) -> Lookup<Self::Def>;
    /// The children of `parent` of `kind`.
    fn children(&self, parent: &ObjectKey, kind: ObjectKind) -> Lookup<Vec<Self::Def>>;
    /// The object of `database` with server id `id` (M6's `OriginCatalog`, A-12).
    fn find_by_server_id(&self, database: &Name, id: ServerId) -> Lookup<Self::Def>;
    /// The freshness of `ns`.
    fn freshness(&self, ns: &NsKey) -> Freshness;
}

/// What the introspection scope's object filters (HV-INTRO-03, scope role; Task 12) leave
/// unread inside a loaded namespace. Implemented by the engine's adapter (Task 24).
pub trait ScopeFilter: Send + Sync {
    /// Whether an object of `kind` (`None`: any kind) named `name` (`None`: any name; the
    /// probe text as typed otherwise) in `ns` may have been left unread. Answer `true` when
    /// unsure: it only turns a `Missing` into `Unknown(OutOfScope)`.
    fn excludes(&self, ns: &NsKey, kind: Option<ObjectKind>, name: Option<&str>) -> bool;
}

/// A monotonic clock (the negative cache's TTL; tests move it by hand).
pub trait MonoClock: Send + Sync {
    fn now(&self) -> Instant;
}

/// The process clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl MonoClock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

type OnUnknown = Box<dyn Fn(&NsKey) + Send + Sync>;
type OnMissingProbe = Box<dyn Fn(&NsKey, &NamePart) + Send + Sync>;

/// The engine's ears (alg/02 HV-MODEL-02 steps 5 and 6), shared by every view of one catalog.
/// The callbacks run on the looking-up thread, after the hooks' own lock is released; they
/// must only enqueue work (a channel send), never block or look up again.
pub struct LookupHooks {
    on_unknown: OnUnknown,
    on_missing_probe: OnMissingProbe,
    clock: Arc<dyn MonoClock>,
    state: Mutex<HookState>,
}

#[derive(Default)]
struct HookState {
    /// The newest generation reported so far; older entries of `asked` are dropped then.
    newest: u64,
    asked: HashSet<(u64, NsKey)>,
    /// When each namespace was last probed for a missing name.
    probed: HashMap<NsKey, Instant>,
}

impl fmt::Debug for LookupHooks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LookupHooks").finish_non_exhaustive()
    }
}

impl LookupHooks {
    /// The negative cache's TTL per namespace (alg/02 step 6).
    pub const PROBE_TTL: Duration = Duration::from_secs(30);

    /// Hooks calling `on_unknown(ns)` for a listed namespace a lookup needed but found not
    /// loaded at a sufficient level, and `on_missing_probe(ns, name)` for a name a lookup
    /// answered `Missing` in `ns`.
    pub fn new(
        on_unknown: impl Fn(&NsKey) + Send + Sync + 'static,
        on_missing_probe: impl Fn(&NsKey, &NamePart) + Send + Sync + 'static,
    ) -> Self {
        LookupHooks {
            on_unknown: Box::new(on_unknown),
            on_missing_probe: Box::new(on_missing_probe),
            clock: Arc::new(SystemClock),
            state: Mutex::new(HookState::default()),
        }
    }

    /// The same hooks reading `clock`.
    pub fn with_clock(mut self, clock: Arc<dyn MonoClock>) -> Self {
        self.clock = clock;
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HookState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Once per namespace per generation.
    fn unknown(&self, generation: u64, ns: &NsKey) {
        let fire = {
            let mut s = self.lock();
            if generation > s.newest {
                s.newest = generation;
                s.asked.retain(|(g, _)| *g >= generation);
            }
            s.asked.insert((generation, ns.clone()))
        };
        if fire {
            (self.on_unknown)(ns);
        }
    }

    /// At most once per namespace per [`LookupHooks::PROBE_TTL`].
    fn missing(&self, ns: &NsKey, name: &NamePart) {
        let now = self.clock.now();
        let fire = {
            let mut s = self.lock();
            match s.probed.get(ns) {
                Some(&at) if now.saturating_duration_since(at) < Self::PROBE_TTL => false,
                _ => {
                    s.probed.insert(ns.clone(), now);
                    true
                }
            }
        };
        if fire {
            (self.on_missing_probe)(ns, name);
        }
    }
}

#[cfg(feature = "test-support")]
thread_local! {
    static SLOW_PATH: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Counts a lookup step that may allocate (a key built by `match_key`, a per-generation index
/// built); the perf test `lookup_hit_does_not_allocate` reads it. A no-op without
/// `test-support`.
#[inline]
pub(crate) fn note_slow_path() {
    #[cfg(feature = "test-support")]
    SLOW_PATH.with(|c| c.set(c.get() + 1));
}

/// How many lookup steps on this thread left the allocation-free hit path (test-support).
#[cfg(feature = "test-support")]
pub fn slow_lookups_on_this_thread() -> u64 {
    SLOW_PATH.with(std::cell::Cell::get)
}

/// A model view over one pinned generation.
#[derive(Clone)]
pub struct SnapshotView {
    snapshot: Arc<CatalogSnapshot>,
    search_path: SearchPath,
    /// The overlays layered above the snapshot (the plan's `SnapshotView { snap, search_path,
    /// layers }`; HV-INTRO-16): the generation's own overlays unless a caller sets others.
    /// Task 26 consumes them: `lookup` and `children` answer a system namespace from the
    /// system layer (`crate::overlay`), and the virtual foreign keys are layered on demand.
    layers: Arc<Overlays>,
    scope: Option<Arc<dyn ScopeFilter>>,
    hooks: Option<Arc<LookupHooks>>,
}

impl fmt::Debug for SnapshotView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SnapshotView")
            .field("generation", &self.snapshot.generation)
            .field("search_path", &self.search_path)
            .field("system_layer", &self.layers.system.is_some())
            .field("scope_filter", &self.scope.is_some())
            .field("hooks", &self.hooks.is_some())
            .finish()
    }
}

/// The namespace a qualified name points into.
enum Qualified {
    Ns(NsKey),
    Unknown(UnknownReason),
}

/// One namespace a lookup walks.
#[derive(Clone, Copy)]
struct Candidate<'k> {
    /// `None`: an entry the session could not name.
    ns: Option<&'k NsKey>,
    /// Exists whatever the schema list says (`pg_catalog`, a reported temporary schema).
    must_exist: bool,
}

impl SnapshotView {
    /// A view of `snapshot` with an empty search path, no scope filter and no hooks.
    pub fn new(snapshot: Arc<CatalogSnapshot>) -> Self {
        SnapshotView {
            layers: Arc::clone(&snapshot.overlays),
            snapshot,
            search_path: SearchPath::default(),
            scope: None,
            hooks: None,
        }
    }

    /// The same view with `layers` above the snapshot (Task 26).
    pub fn with_layers(mut self, layers: Arc<Overlays>) -> Self {
        self.layers = layers;
        self
    }

    /// The overlays above the snapshot.
    pub fn layers(&self) -> &Arc<Overlays> {
        &self.layers
    }

    /// The same view resolving unqualified names along `search_path` (edge 8: a session's
    /// new path applies to the next view, no republish).
    pub fn with_search_path(mut self, search_path: SearchPath) -> Self {
        self.search_path = search_path;
        self
    }

    /// The same view with the scope's object filters.
    pub fn with_scope_filter(mut self, scope: Arc<dyn ScopeFilter>) -> Self {
        self.scope = Some(scope);
        self
    }

    /// The same view reporting to `hooks`.
    pub fn with_hooks(mut self, hooks: Arc<LookupHooks>) -> Self {
        self.hooks = Some(hooks);
        self
    }

    /// The pinned generation.
    pub fn snapshot(&self) -> &Arc<CatalogSnapshot> {
        &self.snapshot
    }

    pub fn search_path(&self) -> &SearchPath {
        &self.search_path
    }

    fn ctx(&self) -> &IdentCtx {
        &self.snapshot.server.ident
    }

    fn ask(&self, key: &NsKey) {
        if let Some(h) = &self.hooks {
            h.unknown(self.snapshot.generation, key);
        }
    }

    fn excluded(&self, ns: &NsKey, kind: Option<ObjectKind>, name: Option<&str>) -> bool {
        self.scope
            .as_ref()
            .is_some_and(|s| s.excludes(ns, kind, name))
    }

    // ## m4-t26-overlays (named exception in Task 9's file): `sql_view` asks the scope filter
    // before it answers a list or a miss from a live namespace.
    pub(crate) fn scope_excludes(
        &self,
        ns: &NsKey,
        kind: Option<ObjectKind>,
        name: Option<&str>,
    ) -> bool {
        self.excluded(ns, kind, name)
    }
    // ## m4-t26-overlays

    /// The namespace `qual` (the qualifier parts of a typed name) points into: PostgreSQL
    /// `schema` (in the search path's database) or `database.schema` (the current database
    /// only: PostgreSQL refuses cross-database references); elsewhere `schema`.
    fn qualify(&self, qual: &[NamePart]) -> Qualified {
        let snap = &*self.snapshot;
        let pg = pg_family(snap.server.dbms);
        let (db_part, schema_part) = match (pg, qual) {
            (true, [s]) => (None, s),
            (true, [d, s]) => (Some(d), s),
            (false, [s]) => (None, s),
            _ => return Qualified::Unknown(UnknownReason::NotLoaded),
        };
        let mut prefix: SmallVec<[Name; 2]> = SmallVec::new();
        if pg {
            let current = self.search_path.database.as_ref();
            let db = match db_part {
                None => match current {
                    Some(db) => db.clone(),
                    None => return Qualified::Unknown(UnknownReason::NotLoaded),
                },
                Some(d) => match self.key_text(d, NameClass::Database) {
                    Ok(k) if current.is_none_or(|c| *c == k) => k,
                    Ok(_) => return Qualified::Unknown(UnknownReason::NotLoaded),
                    Err(()) => return Qualified::Unknown(UnknownReason::ServerDependent),
                },
            };
            prefix.push(db);
        }
        let exact = |text: &Name| {
            let mut path = prefix.clone();
            path.push(text.clone());
            NsKey {
                kind: NsKind::Schema,
                path,
            }
        };
        let listed = |k: &NsKey| {
            snap.namespaces.contains_key(k)
                || snap
                    .states
                    .contains_key(&ContainerKey::Namespace(k.clone()))
        };
        let ctx = self.ctx();
        let key = match self.key_text(schema_part, NameClass::Schema) {
            Ok(k) => k,
            Err(()) => {
                // Only where the server compares case-insensitively is a byte-exact stored name
                // certainly the probe's namespace (as `NamespaceSnapshot::match_named`).
                let candidate = exact(&schema_part.text);
                return if mysql_family(snap.server.dbms) && listed(&candidate) {
                    Qualified::Ns(candidate)
                } else {
                    Qualified::Unknown(UnknownReason::ServerDependent)
                };
            }
        };
        let direct = exact(&key);
        match FoldRule::for_names(ctx.dialect, ctx, NameClass::Schema) {
            Some(FoldRule::AsciiCaseless) if !listed(&direct) => {
                note_slow_path();
                let keys =
                    snap.namespaces
                        .keys()
                        .chain(snap.states.keys().filter_map(|k| match k {
                            ContainerKey::Namespace(ns) => Some(ns),
                            _ => None,
                        }));
                let mut siblings = keys.filter(|k| {
                    k.kind == NsKind::Schema
                        && k.path.len() == prefix.len() + 1
                        && k.path.starts_with(&prefix)
                });
                let mut uncertain = false;
                let found = siblings.find(|k| {
                    uncertain |= has_non_ascii_cased(k.root_name());
                    FoldRule::AsciiCaseless.matches(k.root_name(), &key)
                });
                match found {
                    Some(k) => Qualified::Ns(k.clone()),
                    // R-M4-41 at schema level (T9 fix round 1, I4): a stored MySQL/MariaDB
                    // schema name with a non-ASCII cased letter may fold onto the probe on the
                    // server, so a miss is not a fact (as `match_named` does for tables).
                    None if mysql_family(snap.server.dbms)
                        && (uncertain || siblings.any(|k| has_non_ascii_cased(k.root_name()))) =>
                    {
                        Qualified::Unknown(UnknownReason::ServerDependent)
                    }
                    None => Qualified::Ns(direct),
                }
            }
            _ => Qualified::Ns(direct),
        }
    }

    /// The key text of a typed name part of `class`: the part's own text (no allocation) when
    /// folding changes nothing, else `match_key`'s; `Err` when the server's rule is not pinned.
    fn key_text(&self, part: &NamePart, class: NameClass) -> Result<Name, ()> {
        let ctx = self.ctx();
        if key_is_identity(ctx, &part.text, part.quoted, class) {
            return Ok(part.text.clone());
        }
        note_slow_path();
        match_key(ctx.dialect, ctx, &part.text, part.quoted, class)
            .map(|k| Name::from(k.as_str()))
            .map_err(|_| ())
    }

    /// The schemas of a database (PostgreSQL) or of the server (elsewhere), from the schema
    /// list (T9 fix round 1, I5): `Found` only when the list is loaded and every listed schema
    /// has an arena to hand out (its root), in key order; `Unknown(NotLoaded)` otherwise.
    fn listed_schemas(&self, parent: &ObjectKey) -> Lookup<Vec<ObjHandle>> {
        let snap = &*self.snapshot;
        let pg = pg_family(snap.server.dbms);
        let database = match (pg, parent.kind, parent.path.as_slice()) {
            (true, ObjectKind::Database, [db]) => Some(db),
            (false, ObjectKind::Root, []) => None,
            _ => return Lookup::Unknown(UnknownReason::NotLoaded),
        };
        if !schema_list_loaded(snap, database) {
            return Lookup::Unknown(UnknownReason::NotLoaded);
        }
        let under = |k: &NsKey| {
            k.kind == NsKind::Schema
                && match database {
                    Some(db) => k.path.len() == 2 && k.path[0] == *db,
                    None => k.path.len() == 1,
                }
        };
        let keys: std::collections::BTreeSet<&NsKey> = snap
            .namespaces
            .keys()
            .chain(snap.states.keys().filter_map(|k| match k {
                ContainerKey::Namespace(ns) => Some(ns),
                _ => None,
            }))
            .filter(|k| under(k))
            .collect();
        let mut out = Vec::with_capacity(keys.len());
        for k in keys {
            match snap.namespaces.get(k) {
                Some(arena) => out.push(snap.handle(arena, 0)),
                None => return Lookup::Unknown(UnknownReason::NotLoaded),
            }
        }
        Lookup::Found(out)
    }

    /// `part` in `group` of one arena: `Found(index)`, `Missing`, or `Unknown(ServerDependent)`
    /// (the arena's own fact; the namespace's load state is the caller's).
    fn in_arena(
        &self,
        arena: &NamespaceSnapshot,
        group: KindGroup,
        part: &NamePart,
    ) -> Lookup<u32> {
        let ctx = self.ctx();
        if key_is_identity(ctx, &part.text, part.quoted, group_class(group))
            && agrees(arena, ctx)
            && let Some(&first) = arena.lookup(group, &part.text).first()
        {
            debug_assert!(
                matches!(
                    arena.match_named(group, &part.text, part.quoted, ctx),
                    Lookup::Found(ref h) if h[0] == first
                ),
                "the allocation-free hit path disagrees with match_named"
            );
            return Lookup::Found(first);
        }
        note_slow_path();
        arena
            .match_named(group, &part.text, part.quoted, ctx)
            .map(|hits| hits[0])
    }

    /// Walks `candidates` in order (step 5, amended by R-W2-6): a namespace that holds the
    /// name answers `Found` only when every earlier one was a certain miss; the first
    /// namespace that cannot answer stops the walk with its `Unknown` reason (the server could
    /// resolve the name there, so a later hit may be the wrong object). Only a walk of certain
    /// misses is `Missing`, probing each namespace that answered it. No candidate at all is
    /// `Unknown(NotLoaded)`.
    fn walk<'k>(
        &self,
        candidates: impl Iterator<Item = Candidate<'k>>,
        part: &NamePart,
        group: KindGroup,
    ) -> Lookup<ObjHandle> {
        let snap = &*self.snapshot;
        let mut missing_in: SmallVec<[&NsKey; 4]> = SmallVec::new();
        let mut walked = false;
        for Candidate { ns, must_exist } in candidates {
            walked = true;
            // An entry the session could not name (`$user` of an unknown user).
            let Some(key) = ns else {
                return Lookup::Unknown(UnknownReason::NotLoaded);
            };
            // ## m4-t26-overlays (named exception in Task 9's file): a system namespace the
            // user did not choose to introspect live answers from the system layer; a miss
            // there walks on only when it is a fact (not against a newer patch release).
            if let Some(sys) = crate::overlay::system_ns(snap, &self.layers, key) {
                match self.in_arena(&sys.arena, group, part) {
                    Lookup::Found(idx) => return Lookup::Found(sys.handle(idx)),
                    Lookup::Unknown(reason) => return Lookup::Unknown(reason),
                    Lookup::Missing(_) if sys.miss_is_fact => continue,
                    Lookup::Missing(_) => return Lookup::Unknown(UnknownReason::ServerDependent),
                }
            }
            // ## m4-t26-overlays
            let NsView {
                arena,
                status,
                listed,
            } = ns_status(snap, key);
            if !listed {
                // A namespace the catalog does not list: absent (and skipped, as the server
                // skips it) only when the schema list is loaded and nothing says it exists.
                if must_exist || !list_known_for(snap, key) {
                    return Lookup::Unknown(UnknownReason::NotLoaded);
                }
                continue;
            }
            if status == NsStatus::OutOfScope {
                return Lookup::Unknown(UnknownReason::OutOfScope);
            }
            if let Some(arena) = arena {
                match self.in_arena(arena, group, part) {
                    Lookup::Found(idx) => return Lookup::Found(snap.handle(arena, idx)),
                    Lookup::Unknown(reason) => return Lookup::Unknown(reason),
                    Lookup::Missing(_) => {}
                }
            }
            match status {
                NsStatus::Complete => {
                    if self.excluded(key, None, Some(&part.text)) {
                        return Lookup::Unknown(UnknownReason::OutOfScope);
                    }
                    missing_in.push(key);
                }
                NsStatus::Partial { reason, wants_load } => {
                    if wants_load {
                        self.ask(key);
                    }
                    return Lookup::Unknown(reason);
                }
                NsStatus::OutOfScope => return Lookup::Unknown(UnknownReason::OutOfScope),
            }
        }
        // An empty path (MySQL with no current database) names nowhere to look: no fact.
        if !walked {
            return Lookup::Unknown(UnknownReason::NotLoaded);
        }
        if let Some(h) = &self.hooks {
            for key in missing_in {
                h.missing(key, part);
            }
        }
        Lookup::Missing(MissingReason::NotFound)
    }
}

impl ModelView for SnapshotView {
    type Def = ObjHandle;

    /// `path` is `name`, `schema.name` or (PostgreSQL) `database.schema.name`. An unqualified
    /// name walks the search path for `group` (PostgreSQL's temporary schema only for
    /// relations and types); a qualified one looks in its namespace only. Accounts live in
    /// the server namespace. An empty path, or one with more parts than the dialect has,
    /// answers `Unknown(NotLoaded)`.
    fn lookup(&self, path: &[NamePart], group: KindGroup) -> Lookup<ObjHandle> {
        let Some((last, qual)) = path.split_last() else {
            return Lookup::Unknown(UnknownReason::NotLoaded);
        };
        if group == KindGroup::Accounts {
            if !qual.is_empty() {
                return Lookup::Unknown(UnknownReason::NotLoaded);
            }
            let server = NsKey::server();
            return self.walk(
                std::iter::once(Candidate {
                    ns: Some(&server),
                    must_exist: true,
                }),
                last,
                group,
            );
        }
        if qual.is_empty() {
            return self.walk(
                self.search_path.entries_for(group).map(|e| Candidate {
                    ns: e.ns.as_ref(),
                    must_exist: e.must_exist,
                }),
                last,
                group,
            );
        }
        match self.qualify(qual) {
            Qualified::Ns(key) => self.walk(
                std::iter::once(Candidate {
                    ns: Some(&key),
                    must_exist: false,
                }),
                last,
                group,
            ),
            Qualified::Unknown(reason) => Lookup::Unknown(reason),
        }
    }

    /// The children of `parent` of `kind`, in arena order: `Found` only from a complete
    /// namespace whose container for this kind group is not loading or failed, and that no
    /// scope filter thins; `Missing` when the parent is absent from a complete namespace.
    fn children(&self, parent: &ObjectKey, kind: ObjectKind) -> Lookup<Vec<ObjHandle>> {
        let snap = &*self.snapshot;
        if kind == ObjectKind::Schema
            && matches!(parent.kind, ObjectKind::Database | ObjectKind::Root)
        {
            return self.listed_schemas(parent);
        }
        // ## m4-t26-overlays (named exception in Task 9's file): the system layer.
        if let Some(answer) = crate::overlay::system_children(snap, &self.layers, parent, kind) {
            return answer;
        }
        // ## m4-t26-overlays
        let key = if parent.kind.is_namespace_root() {
            let ns_kind = match parent.kind {
                ObjectKind::Root => NsKind::Server,
                ObjectKind::Database => NsKind::Database,
                _ => NsKind::Schema,
            };
            Some(NsKey {
                kind: ns_kind,
                path: parent.path.iter().cloned().collect(),
            })
        } else {
            locate_ns_of(snap, &parent.path)
        };
        let Some(key) = key else {
            return Lookup::Unknown(UnknownReason::NotLoaded);
        };
        let v = ns_status(snap, &key);
        if !v.listed {
            return if list_known_for(snap, &key) {
                Lookup::Missing(MissingReason::NotFound)
            } else {
                Lookup::Unknown(UnknownReason::NotLoaded)
            };
        }
        let arena = match (v.status, v.arena) {
            (NsStatus::Complete, Some(arena)) => arena,
            (NsStatus::OutOfScope, _) => return Lookup::Unknown(UnknownReason::OutOfScope),
            (NsStatus::Partial { reason, wants_load }, _) => {
                if wants_load {
                    self.ask(&key);
                }
                return Lookup::Unknown(reason);
            }
            (NsStatus::Complete, None) => return Lookup::Unknown(UnknownReason::NotLoaded),
        };
        let container = ContainerKey::Children {
            ns: key.clone(),
            parent: parent.clone(),
            kind,
        };
        if let Some(state) = snap.states.get(&container) {
            let reason = match state {
                LoadState::Loaded { .. } => None,
                LoadState::NotLoaded => Some(UnknownReason::NotLoaded),
                LoadState::Loading { .. } => Some(UnknownReason::Loading),
                LoadState::Failed { .. } | LoadState::Partial { .. } => Some(UnknownReason::Failed),
                LoadState::OutOfScope => Some(UnknownReason::OutOfScope),
            };
            if let Some(reason) = reason {
                return Lookup::Unknown(reason);
            }
        }
        match arena.find(parent) {
            None if self.excluded(&key, Some(parent.kind), Some(parent.name())) => {
                Lookup::Unknown(UnknownReason::OutOfScope)
            }
            // The parent's own schema may simply not be listed yet (T9 fix round 1, C1).
            None if !miss_is_fact(snap, &key, &parent.path) => {
                Lookup::Unknown(UnknownReason::NotLoaded)
            }
            None => Lookup::Missing(MissingReason::NotFound),
            Some(_) if self.excluded(&key, Some(kind), None) => {
                Lookup::Unknown(UnknownReason::OutOfScope)
            }
            Some(at) => Lookup::Found(
                arena
                    .children_of_kind(at, kind)
                    .map(|(i, _)| snap.handle(arena, i))
                    .collect(),
            ),
        }
    }

    /// The relation (any kind of the `Relations` group) with server id `id` in `database`'s
    /// namespaces (PostgreSQL OIDs are unique per database only). `Missing` only when
    /// `database`'s schema list is loaded and every one of its schemas is complete and
    /// unfiltered.
    fn find_by_server_id(&self, database: &Name, id: ServerId) -> Lookup<ObjHandle> {
        let snap = &*self.snapshot;
        let dbms = snap.server.dbms;
        let pg = pg_family(dbms);
        let kinds: SmallVec<[ObjectKind; 16]> = ObjectKind::ALL
            .iter()
            .copied()
            .filter(|k| k.group(dbms) == Some(KindGroup::Relations))
            .collect();
        let in_db = |k: &NsKey| {
            k.kind == NsKind::Schema
                && if pg {
                    k.path.len() == 2 && k.path[0] == *database
                } else {
                    k.path.len() == 1 && k.path[0] == *database
                }
        };
        let mut keys: Vec<&NsKey> = snap.namespaces.keys().filter(|k| in_db(k)).collect();
        keys.extend(snap.states.keys().filter_map(|k| match k {
            ContainerKey::Namespace(ns) if in_db(ns) && !snap.namespaces.contains_key(ns) => {
                Some(ns)
            }
            _ => None,
        }));
        let mut unknown = None;
        for key in keys {
            let v = ns_status(snap, key);
            if v.status == NsStatus::OutOfScope {
                unknown.get_or_insert(UnknownReason::OutOfScope);
                continue;
            }
            if let Some(arena) = v.arena {
                for &kind in &kinds {
                    let probe = ObjectKey {
                        kind,
                        path: smallvec::smallvec![database.clone()],
                        server_id: Some(id),
                        overload_sig: None,
                        raw_kind: None,
                    };
                    if let Some(idx) = arena.find(&probe) {
                        return Lookup::Found(snap.handle(arena, idx));
                    }
                }
            }
            match v.status {
                NsStatus::Complete if self.excluded(key, None, None) => {
                    unknown.get_or_insert(UnknownReason::OutOfScope);
                }
                NsStatus::Complete | NsStatus::OutOfScope => {}
                NsStatus::Partial { reason, wants_load } => {
                    if wants_load {
                        self.ask(key);
                    }
                    unknown.get_or_insert(reason);
                }
            }
        }
        if let Some(reason) = unknown {
            return Lookup::Unknown(reason);
        }
        let list = if pg { Some(database) } else { None };
        if schema_list_loaded(snap, list) {
            Lookup::Missing(MissingReason::NotFound)
        } else {
            Lookup::Unknown(UnknownReason::NotLoaded)
        }
    }

    /// `Loaded` when the namespace's state is `Loaded` and its arena is here; else `Unknown`.
    fn freshness(&self, ns: &NsKey) -> Freshness {
        let snap = &*self.snapshot;
        match snap.states.get(&ContainerKey::Namespace(ns.clone())) {
            Some(LoadState::Loaded {
                at,
                watermark,
                stale,
                ..
            }) if snap.namespaces.contains_key(ns) => Freshness::Loaded {
                at: *at,
                watermark: watermark.clone(),
                stale: *stale,
            },
            _ => Freshness::Unknown,
        }
    }
}

/// The name class of a kind group's names (as the namespace's own index keys them).
fn group_class(group: KindGroup) -> NameClass {
    match group {
        KindGroup::Relations => NameClass::Table,
        KindGroup::Triggers => NameClass::Trigger,
        KindGroup::Routines | KindGroup::Procedures => NameClass::Routine,
        KindGroup::Accounts => NameClass::Account,
        KindGroup::Types | KindGroup::Other => NameClass::Other,
    }
}

/// Whether a probe context matches `arena`'s stored keys: the same engine family and the same
/// pinned table rule (as `NamespaceSnapshot::match_named` checks before keying).
fn agrees(arena: &NamespaceSnapshot, ctx: &IdentCtx) -> bool {
    let family = match ctx.dialect {
        DialectId::Postgres | DialectId::PgWire(_) => pg_family(arena.dbms()),
        DialectId::MySql | DialectId::MariaDb => mysql_family(arena.dbms()),
        DialectId::Sqlite => arena.dbms() == hacivat_proto::DbmsKind::Sqlite,
        _ => false,
    };
    family && FoldRule::for_names(ctx.dialect, ctx, NameClass::Table) == Some(arena.fold())
}

/// Whether `match_key` keys `text` to `text` itself (so the probe needs no key built): a
/// pinned rule, ASCII text, and nothing to fold — PostgreSQL: quoted, or no capital, and
/// within the 63-byte cut; elsewhere an exact rule, or no capital under the caseless one.
/// Conservative: `false` sends the probe down `match_key`'s path, which is always right.
fn key_is_identity(ctx: &IdentCtx, text: &str, quoted: bool, class: NameClass) -> bool {
    let Some(rule) = FoldRule::for_names(ctx.dialect, ctx, class) else {
        return false;
    };
    if text.is_empty() || !text.is_ascii() {
        return false;
    }
    let capital = text.bytes().any(|b| b.is_ascii_uppercase());
    let identity = match ctx.dialect {
        DialectId::Postgres | DialectId::PgWire(_) => (quoted || !capital) && text.len() <= 63,
        _ => match rule {
            FoldRule::Exact => true,
            FoldRule::AsciiCaseless => !capital,
        },
    };
    debug_assert!(
        !identity
            || match_key(ctx.dialect, ctx, text, quoted, class).is_ok_and(|k| k.as_str() == text),
        "key_is_identity disagrees with match_key for {text:?}"
    );
    identity
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn ctxs() -> Vec<IdentCtx> {
        let mut v = Vec::new();
        for d in [
            DialectId::Postgres,
            DialectId::MySql,
            DialectId::MariaDb,
            DialectId::Sqlite,
            DialectId::Generic,
        ] {
            for lctn in [0, 1, 2, 3] {
                for enc in [None, Some(Arc::from("UTF8")), Some(Arc::from("LATIN1"))] {
                    v.push(IdentCtx {
                        dialect: d,
                        lower_case_table_names: lctn,
                        encoding: enc,
                    });
                }
            }
        }
        v
    }

    const CLASSES: [NameClass; 8] = [
        NameClass::Database,
        NameClass::Schema,
        NameClass::Table,
        NameClass::Column,
        NameClass::Routine,
        NameClass::Trigger,
        NameClass::Account,
        NameClass::Other,
    ];

    proptest! {
        /// The allocation-free path never claims a key `match_key` would not build.
        #[test]
        fn identity_keys_agree_with_match_key(text in "[a-zA-Z_0-9$ İıé\"]{0,70}", quoted: bool) {
            for ctx in ctxs() {
                for class in CLASSES {
                    if key_is_identity(&ctx, &text, quoted, class) {
                        let k = match_key(ctx.dialect, &ctx, &text, quoted, class);
                        prop_assert!(k.is_ok_and(|k| k.as_str() == text), "{ctx:?} {class:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn identity_covers_the_common_probes() {
        let pg = IdentCtx::new(DialectId::Postgres);
        assert!(key_is_identity(&pg, "orders", false, NameClass::Table));
        assert!(key_is_identity(&pg, "Orders", true, NameClass::Table));
        assert!(!key_is_identity(&pg, "Orders", false, NameClass::Table));
        assert!(!key_is_identity(
            &pg,
            &"a".repeat(64),
            true,
            NameClass::Table
        ));
        let my = IdentCtx::new(DialectId::MySql);
        assert!(key_is_identity(&my, "Orders", false, NameClass::Table));
        assert!(!key_is_identity(&my, "Total", false, NameClass::Column));
        assert!(key_is_identity(&my, "total", true, NameClass::Column));
        let sqlite = IdentCtx::new(DialectId::Sqlite);
        assert!(!key_is_identity(&sqlite, "T", true, NameClass::Table));
        assert!(!key_is_identity(
            &IdentCtx::new(DialectId::Generic),
            "t",
            false,
            NameClass::Table
        ));
    }
}
