//! Overlays above the introspected catalog (HV-INTRO-16): the system-catalog snapshot of the
//! server's version, stored virtual foreign keys (local only, D-057) and annotations.
//!
//! M4 Task 26 layers them under and over [`SnapshotView`]:
//!
//! - **The system layer.** A system namespace (PostgreSQL `pg_catalog`, `information_schema`
//!   in every database; MySQL/MariaDB `information_schema`, `mysql`, `performance_schema`,
//!   `sys`) resolves from the snapshot of [`Overlays::system`] (objects flagged
//!   `PRE_INTROSPECTED`, the snapshot's line as the version label), unless the user chose
//!   "Introspect live" for it ([`Overlays::live_system`]): then from the live namespace like
//!   any other. Without a snapshot the live namespace answers, or nothing (`Unknown`).
//!   MySQL/MariaDB `information_schema` names match ASCII-caselessly (review I-1). An object
//!   the snapshot lacks is `Missing` only when the server is proven to hold nothing beyond
//!   initdb in its system schemas ([`crate::system::objects_complete`], ruling R-W5-6:
//!   PostgreSQL with `ServerFacts::system_schema_extras == Some(false)`, never MySQL/MariaDB);
//!   before that, a live in-scope arena of the same namespace is consulted, so an extension's
//!   object in `pg_catalog` is found. The children of an object the snapshot holds (a system
//!   table's columns) are complete where the version rule covers the server
//!   ([`crate::system::miss_is_fact`]: PostgreSQL any release of its major, R-W5-3;
//!   MySQL/MariaDB a patch up to the snapshot's own). Everything else is
//!   `Unknown(ServerDependent)`, never `Missing` (alg/02 HV-INTRO-16 edge 1). A system
//!   relation the snapshot holds without columns answers `Unknown`, never "no columns".
//!   PostgreSQL system objects are keyed in the live form `[database, schema, …]`: the view
//!   hands out arenas re-keyed per database (built once per database and shared,
//!   [`LayerCache`]).
//! - **The virtual layer.** User virtual foreign keys ([`Overlays::virtual_fks`], from
//!   `ProfileUserData.virtual_fks`) and the shipped catalog virtual foreign keys
//!   ([`Overlays::shipped_fks`], `assets/system-catalogs/virtual-fks.toml`) are reported by
//!   [`SnapshotView::virtual_fks_of`] / [`SnapshotView::virtual_fks_to`] with `layer =
//!   Virtual`; hidden when a real foreign key with the same columns to the same relation
//!   exists; kept and flagged `BROKEN` when the target is certainly gone (`Missing`, never on
//!   `Unknown`); matched by path when server ids differ (an imported overlay), by server id
//!   when the path differs (a rename). They never enter the model: `children(_, ForeignKey)`
//!   and the DDL producers see only real keys.
//! - **Annotations** keyed by [`ObjectKey`] ([`Overlays::annotation`]: by the structural key,
//!   else by kind and path).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::sync::{Arc, PoisonError, RwLock, Weak};

use hacivat_proto::DbmsKind;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use sql_catalog::types::{ColumnInfo, RelInfo};

use crate::ids::{NsKey, NsKind, ObjId, ObjectKey};
use crate::kind::{KindGroup, ObjectKind, mysql_family, pg_family};
use crate::name::{Name, NamePart};
use crate::namespace::{NamespaceSnapshot, ObjFlags};
use crate::props::Props;
use crate::snapshot::{CatalogSnapshot, ObjHandle};
use crate::system::SystemSnapshot;
use crate::userdata::ProfileUserData;
use crate::view::{Lookup, MissingReason, ModelView, SnapshotView, UnknownReason, note_slow_path};

/// A virtual foreign key: columns in FK order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualFk {
    pub from: ObjectKey,
    pub from_cols: Vec<String>,
    pub to: ObjectKey,
    pub to_cols: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A shipped virtual foreign key between two system relations (`virtual-fks.toml`, R-M4-12):
/// `from`/`to` are `[schema, relation]` (system schemas exist in every database), `source` the
/// vendor documentation section the row is taken from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShippedFk {
    pub fk: VirtualFk,
    pub dbms: DbmsKind,
    pub source: Name,
}

/// The layer an answer comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layer {
    /// The introspected catalog.
    Live,
    /// A shipped system-catalog snapshot.
    System,
    /// A stored or shipped virtual object.
    Virtual,
}

/// Where a virtual foreign key comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FkOrigin {
    /// The profile's user data (D-057).
    User,
    /// `virtual-fks.toml`.
    Shipped,
}

/// One virtual foreign key as the view layers it.
#[derive(Debug, Clone)]
pub struct LayeredFk {
    /// The row, its relations in the live form (`[database, schema, relation]` on
    /// PostgreSQL) for a shipped row.
    pub fk: VirtualFk,
    pub origin: FkOrigin,
    /// Always [`Layer::Virtual`].
    pub layer: Layer,
    /// `BROKEN` when the target is certainly gone.
    pub flags: ObjFlags,
    /// The target relation in this generation.
    pub target: Lookup<ObjHandle>,
    /// The documentation section of a shipped row.
    pub source: Option<Name>,
}

/// The overlays of one generation.
#[derive(Debug, Clone, Default)]
pub struct Overlays {
    pub system: Option<Arc<SystemSnapshot>>,
    /// The user's virtual foreign keys.
    pub virtual_fks: Vec<VirtualFk>,
    pub annotations: BTreeMap<ObjectKey, Name>,
    // ## m4-t26-overlays
    /// The shipped virtual foreign keys of the server's engine (set with `system`).
    pub shipped_fks: Arc<[ShippedFk]>,
    /// The system namespaces (live keys, `[database, schema]` on PostgreSQL) the user chose to
    /// introspect live: they answer from the live catalog, not the snapshot.
    pub live_system: BTreeSet<NsKey>,
    /// Slices and re-keyed arenas derived from the layers, shared by every clone.
    pub cache: LayerCache,
}

impl Overlays {
    /// The same overlays with `data`'s virtual foreign keys and annotations (the profile's
    /// user data; its file I/O is `hacivat_settings::profile_data`).
    pub fn with_user_data(mut self, data: &ProfileUserData) -> Self {
        self.virtual_fks = data.virtual_fks.clone();
        self.annotations = data
            .annotations
            .iter()
            .map(|(k, v)| (k.clone(), Name::from(v.as_str())))
            .collect();
        // ## m4-t27-refresh-ui (named exception, T26 carry): the persisted "Introspect live"
        // choices.
        self.live_system = data.live_system.iter().cloned().collect();
        self
    }

    /// The same overlays with the system layer `set` gives `dbms` at `version` (its snapshot
    /// and shipped virtual foreign keys; none when the set has no snapshot for it). The user
    /// layers, the live choices and the cache are kept.
    pub fn with_system_of(
        mut self,
        set: &crate::system::SystemCatalogs,
        dbms: DbmsKind,
        version: &str,
    ) -> Self {
        let chosen = set.overlays_for(dbms, version);
        self.system = chosen.system;
        self.shipped_fks = chosen.shipped_fks;
        self
    }

    /// The annotation of `key`: stored under the same key, else under a key of the same kind
    /// and path (an annotation imported from another server, whose ids differ).
    pub fn annotation(&self, key: &ObjectKey) -> Option<&Name> {
        self.annotations.get(key).or_else(|| {
            self.annotations
                .iter()
                .find(|(k, _)| {
                    k.kind == key.kind
                        && k.raw_kind == key.raw_kind
                        && k.path == key.path
                        && k.overload_sig == key.overload_sig
                })
                .map(|(_, v)| v)
        })
    }

    /// The version label of the system layer (the snapshot's line: `17`, `8.4`).
    pub fn system_label(&self) -> Option<&Name> {
        self.system.as_ref().map(|s| &s.line)
    }
}

/// The `ObjId::ns` of a handle into the system layer (its arena is not one of the
/// generation's namespaces).
pub const SYSTEM_LAYER_NS: u32 = u32::MAX;

/// Derived data of the layers, shared by every clone of an [`Overlays`]: PostgreSQL system
/// arenas re-keyed per database, and the `sql_catalog` slices of each namespace (relations,
/// columns per relation), built on first use and handed out as `Arc`s, so a repeated hit
/// allocates nothing. Entries of a namespace that is gone are dropped on the next insert.
#[derive(Clone, Default)]
pub struct LayerCache(Arc<RwLock<CacheInner>>);

#[derive(Default)]
struct CacheInner {
    /// Live system key → (the snapshot it was built from, the re-keyed arena).
    system: HashMap<NsKey, (Arc<SystemSnapshot>, Arc<NamespaceSnapshot>)>,
    /// `Arc::as_ptr` of a namespace → its slices. The `Weak` keeps the allocation, so the
    /// address is not reused while the entry exists.
    slices: HashMap<usize, NsSlices>,
}

struct NsSlices {
    ns: Weak<NamespaceSnapshot>,
    relations: Option<Arc<[RelInfo]>>,
    columns: HashMap<u32, Arc<[ColumnInfo]>>,
}

impl fmt::Debug for LayerCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LayerCache").finish_non_exhaustive()
    }
}

fn ptr_of(ns: &Arc<NamespaceSnapshot>) -> usize {
    Arc::as_ptr(ns) as usize
}

impl LayerCache {
    fn read(&self) -> std::sync::RwLockReadGuard<'_, CacheInner> {
        self.0.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, CacheInner> {
        self.0.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// The system arena `src` of `sys` keyed as `live` (`[database, schema]`).
    fn system_arena(
        &self,
        sys: &Arc<SystemSnapshot>,
        live: &NsKey,
        src: &Arc<NamespaceSnapshot>,
    ) -> Option<Arc<NamespaceSnapshot>> {
        if let Some((built_from, arena)) = self.read().system.get(live)
            && Arc::ptr_eq(built_from, sys)
        {
            return Some(Arc::clone(arena));
        }
        note_slow_path();
        let arena = Arc::new(
            NamespaceSnapshot::from_arena(
                live.clone(),
                src.dbms(),
                src.fold(),
                src.meta().clone(),
                src.objs().to_vec(),
            )
            .ok()?,
        );
        self.write()
            .system
            .insert(live.clone(), (Arc::clone(sys), Arc::clone(&arena)));
        Some(arena)
    }

    fn slices_mut<'a>(inner: &'a mut CacheInner, ns: &Arc<NamespaceSnapshot>) -> &'a mut NsSlices {
        let key = ptr_of(ns);
        if !inner.slices.contains_key(&key) {
            inner.slices.retain(|_, s| s.ns.strong_count() > 0);
        }
        inner.slices.entry(key).or_insert_with(|| NsSlices {
            ns: Arc::downgrade(ns),
            relations: None,
            columns: HashMap::new(),
        })
    }

    /// The relations of `ns`, built by `build` once.
    pub(crate) fn relations(
        &self,
        ns: &Arc<NamespaceSnapshot>,
        build: impl FnOnce() -> Arc<[RelInfo]>,
    ) -> Arc<[RelInfo]> {
        if let Some(r) = self
            .read()
            .slices
            .get(&ptr_of(ns))
            .and_then(|s| s.relations.clone())
        {
            return r;
        }
        note_slow_path();
        let built = build();
        let mut inner = self.write();
        Arc::clone(
            Self::slices_mut(&mut inner, ns)
                .relations
                .get_or_insert(built),
        )
    }

    /// The columns of relation `idx` of `ns`, built by `build` once.
    pub(crate) fn columns(
        &self,
        ns: &Arc<NamespaceSnapshot>,
        idx: u32,
        build: impl FnOnce() -> Arc<[ColumnInfo]>,
    ) -> Arc<[ColumnInfo]> {
        if let Some(c) = self
            .read()
            .slices
            .get(&ptr_of(ns))
            .and_then(|s| s.columns.get(&idx).cloned())
        {
            return c;
        }
        note_slow_path();
        let built = build();
        let mut inner = self.write();
        Arc::clone(
            Self::slices_mut(&mut inner, ns)
                .columns
                .entry(idx)
                .or_insert(built),
        )
    }
}

/// The one MySQL/MariaDB system schema whose name and table names the server compares
/// ASCII-caselessly whatever `lower_case_table_names` is (review I-1).
const MYSQL_CASELESS_SCHEMA: &str = "information_schema";

/// A system namespace as the view sees it.
pub(crate) struct SystemNs {
    pub arena: Arc<NamespaceSnapshot>,
    /// A miss of a whole object in `arena` is a fact (ruling R-W5-6,
    /// [`crate::system::objects_complete`]).
    pub objects_complete: bool,
    /// The children of an object `arena` holds are all there (the version rule,
    /// [`crate::system::miss_is_fact`]).
    pub children_complete: bool,
    /// Names in it compare ASCII-caselessly (MySQL/MariaDB `information_schema`).
    pub caseless: bool,
    /// The live arena of the same namespace, consulted after a miss in `arena` (an object the
    /// snapshot cannot hold: an extension in `pg_catalog`), when the catalog holds one in scope.
    pub live: Option<Arc<NamespaceSnapshot>>,
}

impl SystemNs {
    pub fn handle(&self, idx: u32) -> ObjHandle {
        ObjHandle {
            ns: Arc::clone(&self.arena),
            id: ObjId {
                ns: SYSTEM_LAYER_NS,
                idx,
            },
        }
    }

    /// An object right under the root named `name` ASCII-caselessly, of `group` (a caseless
    /// namespace's second look after an exact miss).
    pub fn caseless_hit(&self, group: KindGroup, name: &str) -> Option<u32> {
        if !self.caseless || !name.is_ascii() {
            return None;
        }
        let dbms = self.arena.dbms();
        self.arena
            .children(0)
            .find(|(_, o)| {
                o.kind().group(dbms) == Some(group) && o.name().eq_ignore_ascii_case(name)
            })
            .map(|(i, _)| i)
    }

    /// The object `key` names in `arena`: exactly, else (caseless namespace) by the caseless
    /// name of a relation right under the root.
    fn find(&self, key: &ObjectKey) -> Option<u32> {
        if let Some(i) = self.arena.find(key) {
            return Some(i);
        }
        match key.path.as_slice() {
            [_, name] if self.caseless && key.kind != ObjectKind::Schema => {
                self.caseless_hit(KindGroup::Relations, name)
            }
            _ => None,
        }
    }
}

/// The system layer's answer for namespace `key` (a live schema key), if it has one: the
/// layer holds a snapshot of the catalog's engine, the snapshot holds `key`'s schema, and the
/// user did not choose to introspect it live.
pub(crate) fn system_ns(
    snap: &CatalogSnapshot,
    layers: &Overlays,
    key: &NsKey,
) -> Option<SystemNs> {
    let sys = layers.system.as_ref()?;
    if key.kind != NsKind::Schema
        || sys.dbms != snap.server.dbms
        || layers.live_system.contains(key)
    {
        return None;
    }
    let pg = pg_family(sys.dbms);
    let schema = match (pg, key.path.as_slice()) {
        (true, [_, s]) | (false, [s]) => s,
        _ => return None,
    };
    let caseless = mysql_family(sys.dbms) && schema.eq_ignore_ascii_case(MYSQL_CASELESS_SCHEMA);
    let src = if caseless {
        sys.namespace(MYSQL_CASELESS_SCHEMA)?
    } else {
        sys.namespace(schema)?
    };
    let arena = if pg {
        layers.cache.system_arena(sys, key, src)?
    } else {
        Arc::clone(src)
    };
    let live = match crate::resolve::ns_status(snap, key) {
        v if v.status == crate::resolve::NsStatus::OutOfScope => None,
        v => v.arena.cloned(),
    };
    Some(SystemNs {
        arena,
        objects_complete: crate::system::objects_complete(&snap.server, sys),
        children_complete: crate::system::miss_is_fact(
            sys.dbms,
            &snap.server.version,
            &sys.server_version,
        ),
        caseless,
        live,
    })
}

/// `children(parent, kind)` from the system layer, when `parent` lies in a namespace it
/// answers for (see [`system_ns`]). `None` hands the question to the live catalog: a parent
/// the snapshot lacks but the live arena of that namespace holds.
pub(crate) fn system_children(
    snap: &CatalogSnapshot,
    layers: &Overlays,
    parent: &ObjectKey,
    kind: ObjectKind,
) -> Option<Lookup<Vec<ObjHandle>>> {
    layers.system.as_ref()?;
    let n = if pg_family(snap.server.dbms) { 2 } else { 1 };
    if parent.path.len() < n || (parent.path.len() == n && parent.kind != ObjectKind::Schema) {
        return None;
    }
    let key = NsKey {
        kind: NsKind::Schema,
        path: parent.path[..n].iter().cloned().collect(),
    };
    let sys = system_ns(snap, layers, &key)?;
    let at = match sys.find(parent) {
        Some(at) => at,
        None if sys.live.as_ref().is_some_and(|l| l.find(parent).is_some()) => return None,
        None if sys.objects_complete => return Some(Lookup::Missing(MissingReason::NotFound)),
        None => return Some(Lookup::Unknown(UnknownReason::ServerDependent)),
    };
    // The objects of the schema (the root's children) are complete only when nothing beyond
    // initdb is there; the children of an object it holds follow the version rule.
    let complete = if at == 0 {
        sys.objects_complete
    } else {
        sys.children_complete
    };
    if !complete {
        return Some(Lookup::Unknown(UnknownReason::ServerDependent));
    }
    let found: Vec<ObjHandle> = sys
        .arena
        .children_of_kind(at, kind)
        .map(|(i, _)| sys.handle(i))
        .collect();
    // A system relation the snapshot holds without columns (a reader that could not see them)
    // is not a relation without columns.
    let relation = sys.arena.obj(at).is_some_and(|o| {
        o.kind() != ObjectKind::Schema
            && o.kind().group(sys.arena.dbms()) == Some(KindGroup::Relations)
    });
    Some(
        if kind == ObjectKind::Column && relation && found.is_empty() {
            Lookup::Unknown(UnknownReason::NotLoaded)
        } else {
            Lookup::Found(found)
        },
    )
}

/// Whether two relation keys denote the same relation for a virtual foreign key: on
/// PostgreSQL first the same database (OIDs are per database, review I-4); then the same
/// server id when both have one (a rename keeps it), else (or when the ids differ: an overlay
/// imported from another server) the same path.
fn same_relation(a: &ObjectKey, b: &ObjectKey, pg: bool) -> bool {
    if pg && a.path.first() != b.path.first() {
        return false;
    }
    matches!((a.server_id, b.server_id), (Some(x), Some(y)) if x == y) || a.path == b.path
}

/// A shipped `[schema, relation]` key in the live form of database `db` (PostgreSQL).
fn live_form(key: &ObjectKey, db: Option<&Name>) -> ObjectKey {
    let mut path: SmallVec<[Name; 4]> = SmallVec::new();
    path.extend(db.cloned());
    path.extend(key.path.iter().cloned());
    ObjectKey {
        path,
        ..key.clone()
    }
}

impl SnapshotView {
    // ## m4-t26-overlays

    /// The version label of the system layer (the snapshot's line), if it has a snapshot.
    pub fn system_label(&self) -> Option<Name> {
        self.layers().system_label().cloned()
    }

    /// The virtual foreign keys whose source relation is `rel` (module docs).
    pub fn virtual_fks_of(&self, rel: &ObjectKey) -> Vec<LayeredFk> {
        self.layered_fks(rel, true)
    }

    /// The virtual foreign keys whose target relation is `rel`.
    pub fn virtual_fks_to(&self, rel: &ObjectKey) -> Vec<LayeredFk> {
        self.layered_fks(rel, false)
    }

    fn layered_fks(&self, rel: &ObjectKey, outgoing: bool) -> Vec<LayeredFk> {
        note_slow_path();
        let layers = self.layers();
        let pg = pg_family(self.snapshot().server.dbms);
        fn side(fk: &VirtualFk, outgoing: bool) -> &ObjectKey {
            if outgoing { &fk.from } else { &fk.to }
        }
        let mut out = Vec::new();
        for fk in &layers.virtual_fks {
            if same_relation(side(fk, outgoing), rel, pg) {
                out.extend(self.layer_fk(fk.clone(), FkOrigin::User, None));
            }
        }
        // Shipped rows are `[schema, relation]`; on PostgreSQL they apply in every database.
        let db = if pg { rel.path.first() } else { None };
        if pg && rel.path.len() < 3 {
            return out;
        }
        for row in layers.shipped_fks.iter() {
            let fk = VirtualFk {
                from: live_form(&row.fk.from, db),
                to: live_form(&row.fk.to, db),
                ..row.fk.clone()
            };
            if side(&fk, outgoing).path == rel.path {
                out.extend(self.layer_fk(fk, FkOrigin::Shipped, Some(row.source.clone())));
            }
        }
        out
    }

    /// `fk` layered: `None` when a real foreign key hides it.
    fn layer_fk(&self, fk: VirtualFk, origin: FkOrigin, source: Option<Name>) -> Option<LayeredFk> {
        let target = self.relation_of(&fk.to);
        if let Lookup::Found(from) = self.relation_of(&fk.from)
            && real_duplicate(&from, &fk, &target)
        {
            return None;
        }
        let flags = if matches!(target, Lookup::Missing(_)) {
            ObjFlags::BROKEN
        } else {
            ObjFlags::empty()
        };
        Some(LayeredFk {
            fk,
            origin,
            layer: Layer::Virtual,
            flags,
            target,
            source,
        })
    }

    /// The relation `key` denotes in this generation: by server id first (a rename), then by
    /// its exact path through the model view (any relation kind; the system layer included).
    pub(crate) fn relation_of(&self, key: &ObjectKey) -> Lookup<ObjHandle> {
        if key.server_id.is_some()
            && let Some(h) = self.snapshot().find(key)
            && h.obj().kind().group(h.ns.dbms()) == Some(KindGroup::Relations)
        {
            return Lookup::Found(h);
        }
        let parts: Vec<NamePart> = key
            .path
            .iter()
            .map(|n| NamePart {
                text: n.clone(),
                quoted: true,
            })
            .collect();
        self.lookup(&parts, KindGroup::Relations)
    }
}

/// Whether `from` has a real foreign key with `fk`'s columns to `fk`'s target relation.
/// MySQL and MariaDB compare column names ASCII-caselessly (review m-7).
fn real_duplicate(from: &ObjHandle, fk: &VirtualFk, target: &Lookup<ObjHandle>) -> bool {
    let caseless = mysql_family(from.ns.dbms());
    let target_id = match target {
        Lookup::Found(t) => t.obj().server_id(),
        _ => None,
    };
    from.ns
        .children_of_kind(from.id.idx, ObjectKind::ForeignKey)
        .any(|(_, o)| {
            let Props::ForeignKey(p) = o.props() else {
                return false;
            };
            let cols_eq = |a: &[Name], b: &[String]| {
                a.len() == b.len()
                    && a.iter().zip(b).all(|(x, y)| {
                        if caseless {
                            x.eq_ignore_ascii_case(y)
                        } else {
                            **x == **y
                        }
                    })
            };
            cols_eq(&p.columns, &fk.from_cols)
                && cols_eq(&p.target_columns, &fk.to_cols)
                && (p.target.path == fk.to.path
                    || (p.target.server_id.is_some() && p.target.server_id == target_id))
        })
}
