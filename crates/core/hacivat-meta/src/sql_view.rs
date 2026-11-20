//! `sql_catalog::CatalogView` over a [`SnapshotView`] (A-22): what M5's completion and the
//! offline proof of R-M4-5 call, over the live, system and virtual layers (M4 Task 26).
//!
//! - **Places.** A schema key names the system layer's namespace (the snapshot's
//!   `pg_catalog`, `information_schema`, …, unless the user chose to introspect it live) or a
//!   live namespace of the current database (PostgreSQL: the search path's database; no
//!   database known → `Unknown(NotLoaded)`).
//! - **Hits allocate nothing** on `relation` and `columns` (alg/02 "completion lookups never
//!   allocate on hits"): the namespace is found by comparing keys, the relation through the
//!   arena's name index, and the answers are `Arc` clones of names or of slices built once per
//!   namespace ([`crate::overlay::LayerCache`]). Every step that may allocate is counted
//!   (`view::note_slow_path`, read by the test-support counter).
//! - **Misses** go through [`ModelView`] (with the probe's key as a quoted name), so `Missing`
//!   follows exactly its rules: a complete, in-scope namespace, a pinned matching rule, the
//!   system layer's patch rule; everything else is `Unknown` with its reason
//!   (`ServerDependent` passed through, R-M4-41).
//! - **Lists** (`relations`, `functions_in`, `types`, `columns`) are `Found` only from a
//!   complete, unfiltered namespace; a system namespace is complete only at a patch release
//!   its snapshot covers (a newer patch may have added a relation or a column, so the list
//!   is `Unknown(ServerDependent)` there; alg/02 HV-INTRO-16 edge 1). A system relation the
//!   snapshot holds without columns is `Unknown`, never "no columns".
//! - **Built-ins.** PostgreSQL built-ins are `pg_catalog`'s routines, with their arguments;
//!   MySQL/MariaDB built-ins are known by name only (`args = None`, X-8): `functions` and
//!   `functions_in` add them to every schema's answer with an empty `schema` (they are called
//!   unqualified). Their names are the generator's probe of the documented function index, a
//!   subset of what the server accepts, so a MySQL/MariaDB function miss is never `Missing`
//!   (`Unknown(ServerDependent)`).
//! - `settings()` from the snapshot, else `Unknown(NotLoaded)` (no live setting list is kept in
//!   the catalog yet); `row_estimate` from the `Statistics` facet once the catalog stores
//!   facet values (Task 15b), `None` until then; `foreign_keys` real keys both ways,
//!   `virtual_fks` the virtual layer's edges both ways (broken ones left out).

use std::sync::Arc;

use smallvec::SmallVec;
use sql_catalog::types::{
    ColumnInfo, FkEdge, FnSig, GeneratedKind as SqlGenerated, IdentityKind, Name, RelInfo, RelKind,
    RelRef, SchemaInfo, SettingInfo, TypeInfo, TypeKind,
};
use sql_catalog::view::CatalogView;

use crate::ids::{NsKey, NsKind, ObjectKey};
use crate::kind::{KindGroup, ObjectKind, mysql_family, pg_family};
use crate::name::{
    DialectId, FoldRule, IdentCtx, IdentKey, NameClass, NamePart, has_non_ascii_cased,
};
use crate::namespace::{NamespaceSnapshot, ObjFlags};
use crate::overlay::system_ns;
use crate::props::{GeneratedKind, Identity, Props, TypeBody};
use crate::resolve::{NsStatus, ns_status, schema_list_loaded};
use crate::snapshot::{ContainerKey, LoadState, ObjHandle};
use crate::view::{Lookup, MissingReason, ModelView, SnapshotView, UnknownReason, note_slow_path};

/// How much a placed namespace can say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    /// Loaded at `Structure` or above (or the system layer at a patch it covers).
    Complete,
    /// Hits from the arena, if any; a miss is `Unknown(reason)`.
    Partial(UnknownReason),
    /// Nothing at all.
    OutOfScope,
}

/// A namespace a schema key names.
struct Placed {
    arena: Option<Arc<NamespaceSnapshot>>,
    status: Status,
    /// The live key (`[database, schema]` on PostgreSQL).
    key: NsKey,
    system: bool,
}

enum Place {
    At(Placed),
    Missing,
    Unknown(UnknownReason),
}

fn status_of(s: NsStatus) -> Status {
    match s {
        NsStatus::Complete => Status::Complete,
        NsStatus::Partial { reason, .. } => Status::Partial(reason),
        NsStatus::OutOfScope => Status::OutOfScope,
    }
}

/// Whether `arena`'s stored rule is the one `ctx` keys probes with (as the view's own check).
fn agrees(arena: &NamespaceSnapshot, ctx: &IdentCtx) -> bool {
    let family = match ctx.dialect {
        DialectId::Postgres | DialectId::PgWire(_) => pg_family(arena.dbms()),
        DialectId::MySql | DialectId::MariaDb => mysql_family(arena.dbms()),
        DialectId::Sqlite => arena.dbms() == hacivat_proto::DbmsKind::Sqlite,
        _ => false,
    };
    family && FoldRule::for_names(ctx.dialect, ctx, NameClass::Table) == Some(arena.fold())
}

fn rel_kind(obj_kind: ObjectKind, props: &Props) -> Option<RelKind> {
    Some(match obj_kind {
        ObjectKind::Table | ObjectKind::Partition | ObjectKind::Subpartition => {
            match props.as_table() {
                Some(t) if t.partitioned => RelKind::Partitioned,
                _ => RelKind::Table,
            }
        }
        ObjectKind::VirtualTable | ObjectKind::Sequence => RelKind::Table,
        ObjectKind::View => RelKind::View,
        ObjectKind::MaterializedView => RelKind::MaterializedView,
        ObjectKind::ForeignTable => RelKind::Foreign,
        _ => return None,
    })
}

fn rel_info(arena: &NamespaceSnapshot, idx: u32) -> Option<RelInfo> {
    let o = arena.obj(idx)?;
    Some(RelInfo {
        name: RelRef {
            schema: arena.root().name().clone(),
            name: o.name().clone(),
        },
        kind: rel_kind(o.kind(), o.props())?,
        temp: arena.is_temp() || o.flags().contains(ObjFlags::TEMP),
        comment: o.props().comment().cloned(),
    })
}

/// The first relation named `key` (stored-key text) right under the root of `arena`.
fn relation_idx(arena: &NamespaceSnapshot, key: &str) -> Option<u32> {
    arena
        .lookup(KindGroup::Relations, key)
        .iter()
        .copied()
        .find(|&i| {
            arena
                .obj(i)
                .is_some_and(|o| o.parent() == Some(0) && rel_kind(o.kind(), o.props()).is_some())
        })
}

fn build_columns(arena: &NamespaceSnapshot, rel: u32) -> Arc<[ColumnInfo]> {
    let mut pk: SmallVec<[&str; 4]> = SmallVec::new();
    let mut fk: SmallVec<[&str; 4]> = SmallVec::new();
    for (_, o) in arena.children(rel) {
        match (o.kind(), o.props()) {
            (ObjectKind::PrimaryKey, Props::Key(k)) => {
                pk.extend(k.columns.iter().filter_map(|c| c.name.as_deref()));
            }
            (ObjectKind::ForeignKey, Props::ForeignKey(f)) => {
                fk.extend(f.columns.iter().map(|c| &**c));
            }
            _ => {}
        }
    }
    let mut cols: Vec<ColumnInfo> = arena
        .children_of_kind(rel, ObjectKind::Column)
        .filter_map(|(_, o)| {
            let p = o.props().as_column()?;
            Some(ColumnInfo {
                name: o.name().clone(),
                ordinal: p.ordinal,
                type_name: p.raw_type.clone(),
                nullable: p.nullable,
                default_raw: p.default_raw.clone(),
                identity: p.identity().map(|i| match i {
                    Identity::Always => IdentityKind::Always,
                    Identity::ByDefault => IdentityKind::ByDefault,
                    Identity::AutoIncrement => IdentityKind::AutoIncrement,
                    Identity::RowidAlias => IdentityKind::RowidAlias,
                }),
                generated: p.generated().map(|g| match g.kind {
                    GeneratedKind::Stored => SqlGenerated::Stored,
                    GeneratedKind::Virtual => SqlGenerated::Virtual,
                }),
                collation: p.collation().cloned(),
                comment: p.comment.clone(),
                in_pk: pk.contains(&&**o.name()),
                in_fk: fk.contains(&&**o.name()),
            })
        })
        .collect();
    cols.sort_by_key(|c| c.ordinal);
    cols.into()
}

fn fn_sig(schema: &Name, o: &crate::namespace::Obj) -> Option<FnSig> {
    let Props::Routine(r) = o.props() else {
        return None;
    };
    Some(FnSig {
        schema: schema.clone(),
        name: o.name().clone(),
        args: r.args.clone(),
        returns: r.returns.clone(),
        returns_set: r.returns_set,
        kind: r.kind,
        volatility: r.volatility,
        language: r.language.clone(),
    })
}

fn is_routine(kind: ObjectKind) -> bool {
    matches!(
        kind,
        ObjectKind::Routine | ObjectKind::Procedure | ObjectKind::Aggregate
    )
}

fn type_info(o: &crate::namespace::Obj) -> Option<TypeInfo> {
    let Props::Type(t) = o.props() else {
        return None;
    };
    Some(TypeInfo {
        name: o.name().clone(),
        kind: match &t.body {
            TypeBody::Enum { labels } => TypeKind::Enum {
                labels: labels.clone(),
            },
            TypeBody::Domain { base, .. } => TypeKind::Domain { base: base.clone() },
            TypeBody::Composite { .. } => TypeKind::Composite,
            TypeBody::Range { subtype } => TypeKind::Range {
                subtype: subtype.clone(),
            },
            TypeBody::Base => TypeKind::Base,
        },
    })
}

fn quoted(text: &Name) -> NamePart {
    NamePart {
        text: text.clone(),
        quoted: true,
    }
}

impl SnapshotView {
    fn sql_ctx(&self) -> &IdentCtx {
        &self.snapshot().server.ident
    }

    /// PostgreSQL: the search path's database (`Err` when unknown); elsewhere `None`.
    fn current_db(&self) -> Result<Option<&Name>, UnknownReason> {
        if pg_family(self.snapshot().server.dbms) {
            self.search_path()
                .database
                .as_ref()
                .map(Some)
                .ok_or(UnknownReason::NotLoaded)
        } else {
            Ok(None)
        }
    }

    fn live_key(db: Option<&Name>, schema: &Name) -> NsKey {
        let mut path: SmallVec<[Name; 2]> = SmallVec::new();
        path.extend(db.cloned());
        path.push(schema.clone());
        NsKey {
            kind: NsKind::Schema,
            path,
        }
    }

    fn placed_live(&self, key: NsKey) -> Placed {
        let v = ns_status(self.snapshot(), &key);
        Placed {
            arena: v.arena.cloned(),
            status: status_of(v.status),
            key,
            system: false,
        }
    }

    fn placed_system(key: NsKey, sys: crate::overlay::SystemNs) -> Placed {
        Placed {
            arena: Some(sys.arena),
            status: if sys.miss_is_fact {
                Status::Complete
            } else {
                Status::Partial(UnknownReason::ServerDependent)
            },
            key,
            system: true,
        }
    }

    /// The namespace schema key `schema` names (module docs).
    fn place(&self, schema: &IdentKey) -> Place {
        let snap = self.snapshot();
        let ctx = self.sql_ctx();
        let db = match self.current_db() {
            Ok(db) => db,
            Err(r) => return Place::Unknown(r),
        };
        let Some(rule) = FoldRule::for_names(ctx.dialect, ctx, NameClass::Schema) else {
            note_slow_path();
            return Place::Unknown(UnknownReason::ServerDependent);
        };
        let probe = schema.as_str();
        if let Some(sys) = &self.layers().system
            && sys.dbms == snap.server.dbms
        {
            for k in sys.namespaces.keys() {
                if let [s] = k.path.as_slice()
                    && rule.matches(s, probe)
                {
                    let key = Self::live_key(db, s);
                    if let Some(sns) = system_ns(snap, self.layers(), &key) {
                        return Place::At(Self::placed_system(key, sns));
                    }
                }
            }
        }
        let depth = usize::from(db.is_some());
        let under = |k: &NsKey| {
            k.kind == NsKind::Schema
                && k.path.len() == depth + 1
                && db.is_none_or(|d| k.path[0] == *d)
        };
        let lo = NsKey {
            kind: NsKind::Schema,
            path: db.into_iter().cloned().collect(),
        };
        let mut uncertain = false;
        for k in snap.namespaces.range(&lo..).map(|(k, _)| k) {
            if k.kind != NsKind::Schema || !k.path.starts_with(&lo.path) {
                break;
            }
            if under(k) && rule.matches(k.root_name(), probe) {
                return Place::At(self.placed_live(k.clone()));
            }
        }
        note_slow_path();
        let listed = snap.states.keys().filter_map(|c| match c {
            ContainerKey::Namespace(k) if under(k) => Some(k),
            _ => None,
        });
        for k in listed.chain(snap.namespaces.keys().filter(|k| under(k))) {
            if rule.matches(k.root_name(), probe) {
                return Place::At(self.placed_live(k.clone()));
            }
            uncertain |= rule == FoldRule::AsciiCaseless && has_non_ascii_cased(k.root_name());
        }
        // R-M4-41 at schema level: a stored caseless name with a non-ASCII cased letter may
        // fold onto the probe on the server.
        if uncertain || (rule == FoldRule::AsciiCaseless && has_non_ascii_cased(probe)) {
            return Place::Unknown(UnknownReason::ServerDependent);
        }
        if schema_list_loaded(snap, db) {
            Place::Missing
        } else {
            Place::Unknown(UnknownReason::NotLoaded)
        }
    }

    /// The namespace of an exact schema name (`RelRef.schema`).
    fn place_exact(&self, schema: &Name) -> Result<Placed, UnknownReason> {
        let db = self.current_db()?;
        let key = Self::live_key(db, schema);
        if let Some(sns) = system_ns(self.snapshot(), self.layers(), &key) {
            return Ok(Self::placed_system(key, sns));
        }
        Ok(self.placed_live(key))
    }

    /// The typed path `[database,] schema, name` of `name` in `p`, quoted (exact).
    fn parts_in(p: &Placed, name: &str) -> Vec<NamePart> {
        let mut parts: Vec<NamePart> = p.key.path.iter().map(quoted).collect();
        parts.push(NamePart {
            text: Name::from(name),
            quoted: true,
        });
        parts
    }

    /// A list answer's status: `Found` only from a complete, unfiltered namespace.
    fn list_status(&self, p: &Placed, kind: Option<ObjectKind>) -> Result<(), UnknownReason> {
        match p.status {
            Status::OutOfScope => Err(UnknownReason::OutOfScope),
            Status::Partial(r) => Err(r),
            Status::Complete if !p.system && self.scope_excludes(&p.key, kind, None) => {
                Err(UnknownReason::OutOfScope)
            }
            Status::Complete if p.arena.is_none() => Err(UnknownReason::NotLoaded),
            Status::Complete => Ok(()),
        }
    }

    /// The handle of relation `rel` (exact names) through the model view.
    fn relation_handle(&self, rel: &RelRef) -> Lookup<ObjHandle> {
        note_slow_path();
        let p = match self.place_exact(&rel.schema) {
            Ok(p) => p,
            Err(r) => return Lookup::Unknown(r),
        };
        self.relation_of(&ObjectKey {
            kind: ObjectKind::Table,
            path: p
                .key
                .path
                .iter()
                .cloned()
                .chain([rel.name.clone()])
                .collect(),
            server_id: None,
            overload_sig: None,
            raw_kind: None,
        })
    }

    /// MySQL/MariaDB built-ins named `name` (all of them for `None`), and whether the server's
    /// built-in names are known (a snapshot of its line); PostgreSQL has none (its built-ins
    /// are `pg_catalog`'s routines). The names are the generator's probe of the documented
    /// function index (R-M4-44), a subset of what the server accepts: a name outside them is
    /// never a fact, so a MySQL/MariaDB function miss is never `Missing`.
    fn builtins(&self, name: Option<&str>) -> (Vec<FnSig>, bool) {
        let snap = self.snapshot();
        if !mysql_family(snap.server.dbms) {
            return (Vec::new(), true);
        }
        let Some(sys) = self
            .layers()
            .system
            .as_ref()
            .filter(|s| s.dbms == snap.server.dbms)
        else {
            return (Vec::new(), false);
        };
        let empty = Name::from("");
        let found = sys
            .builtins
            .iter()
            .filter(|o| name.is_none_or(|n| o.name().eq_ignore_ascii_case(n)))
            .filter_map(|o| fn_sig(&empty, o))
            .collect();
        (found, true)
    }

    fn fk_edge(h: &ObjHandle) -> Option<FkEdge> {
        let o = h.obj();
        let Props::ForeignKey(p) = o.props() else {
            return None;
        };
        let from = h.ns.obj(o.parent()?)?;
        let n = p.target.path.len();
        if n < 2 {
            return None;
        }
        Some(FkEdge {
            from: RelRef {
                schema: h.ns.root().name().clone(),
                name: from.name().clone(),
            },
            from_cols: p.columns.iter().cloned().collect(),
            to: RelRef {
                schema: p.target.path[n - 2].clone(),
                name: p.target.path[n - 1].clone(),
            },
            to_cols: p.target_columns.iter().cloned().collect(),
            name: Some(o.name().clone()),
            is_virtual: false,
        })
    }
}

impl CatalogView for SnapshotView {
    fn generation(&self) -> u64 {
        self.snapshot().generation
    }

    fn ident_ctx(&self) -> &IdentCtx {
        &self.snapshot().server.ident
    }

    fn schemas(&self) -> Lookup<Arc<[SchemaInfo]>> {
        note_slow_path();
        let snap = self.snapshot();
        let db = match self.current_db() {
            Ok(db) => db,
            Err(r) => return Lookup::Unknown(r),
        };
        if !schema_list_loaded(snap, db) {
            return Lookup::Unknown(UnknownReason::NotLoaded);
        }
        let depth = usize::from(db.is_some());
        let mut keys: Vec<&NsKey> = snap
            .namespaces
            .keys()
            .chain(snap.states.keys().filter_map(|c| match c {
                ContainerKey::Namespace(k) => Some(k),
                _ => None,
            }))
            .filter(|k| {
                k.kind == NsKind::Schema
                    && k.path.len() == depth + 1
                    && db.is_none_or(|d| k.path[0] == *d)
            })
            .collect();
        keys.sort();
        keys.dedup();
        let mut out: Vec<SchemaInfo> = Vec::with_capacity(keys.len());
        for k in keys {
            let info = match system_ns(snap, self.layers(), k) {
                Some(sys) => SchemaInfo {
                    name: sys.arena.root().name().clone(),
                    is_system: true,
                    comment: sys.arena.root().props().comment().cloned(),
                },
                None => match snap.namespaces.get(k) {
                    Some(arena) => SchemaInfo {
                        name: arena.root().name().clone(),
                        is_system: arena.root().flags().contains(ObjFlags::SYSTEM),
                        comment: arena.root().props().comment().cloned(),
                    },
                    None => SchemaInfo {
                        name: Name::from(k.root_name()),
                        is_system: false,
                        comment: None,
                    },
                },
            };
            out.push(info);
        }
        if let Some(sys) = &self.layers().system
            && sys.dbms == snap.server.dbms
        {
            for k in sys.namespaces.keys() {
                if let [s] = k.path.as_slice()
                    && !out.iter().any(|i| i.name == *s)
                    && system_ns(snap, self.layers(), &Self::live_key(db, s)).is_some()
                {
                    out.push(SchemaInfo {
                        name: s.clone(),
                        is_system: true,
                        comment: None,
                    });
                }
            }
        }
        Lookup::Found(out.into())
    }

    fn relation(&self, schema: &IdentKey, name: &IdentKey) -> Lookup<RelInfo> {
        let p = match self.place(schema) {
            Place::At(p) => p,
            Place::Missing => return Lookup::Missing(MissingReason::NotFound),
            Place::Unknown(r) => return Lookup::Unknown(r),
        };
        if let Some(arena) = &p.arena
            && agrees(arena, self.sql_ctx())
            && let Some(idx) = relation_idx(arena, name.as_str())
            && let Some(info) = rel_info(arena, idx)
        {
            return Lookup::Found(info);
        }
        note_slow_path();
        match self.lookup(&Self::parts_in(&p, name.as_str()), KindGroup::Relations) {
            Lookup::Found(h) => match rel_info(&h.ns, h.id.idx) {
                Some(info) => Lookup::Found(info),
                // Another kind of the relations group holds the name (an index, a composite
                // type): no relation by that name, where the namespace is certain.
                None => match p.status {
                    Status::Complete => Lookup::Missing(MissingReason::NotFound),
                    Status::Partial(r) => Lookup::Unknown(r),
                    Status::OutOfScope => Lookup::Unknown(UnknownReason::OutOfScope),
                },
            },
            Lookup::Missing(r) => Lookup::Missing(r),
            Lookup::Unknown(r) => Lookup::Unknown(r),
        }
    }

    fn relations(&self, schema: &IdentKey) -> Lookup<Arc<[RelInfo]>> {
        let p = match self.place(schema) {
            Place::At(p) => p,
            Place::Missing => return Lookup::Missing(MissingReason::NotFound),
            Place::Unknown(r) => return Lookup::Unknown(r),
        };
        if let Err(r) = self.list_status(&p, None) {
            return Lookup::Unknown(r);
        }
        let Some(arena) = &p.arena else {
            return Lookup::Unknown(UnknownReason::NotLoaded);
        };
        Lookup::Found(self.layers().cache.relations(arena, || {
            arena
                .children(0)
                .filter_map(|(i, _)| rel_info(arena, i))
                .collect()
        }))
    }

    fn columns(&self, rel: &RelRef) -> Lookup<Arc<[ColumnInfo]>> {
        let p = match self.place_exact(&rel.schema) {
            Ok(p) => p,
            Err(r) => return Lookup::Unknown(r),
        };
        if p.status == Status::Complete
            && let Some(arena) = &p.arena
            && let Some(idx) = relation_idx(arena, &rel.name)
            && arena.obj(idx).is_some_and(|o| *o.name() == rel.name)
            && (if p.system {
                arena
                    .children_of_kind(idx, ObjectKind::Column)
                    .next()
                    .is_some()
            } else {
                self.columns_complete(&p, arena, idx)
            })
        {
            return Lookup::Found(
                self.layers()
                    .cache
                    .columns(arena, idx, || build_columns(arena, idx)),
            );
        }
        note_slow_path();
        let h = match self.relation_handle(rel) {
            Lookup::Found(h) => h,
            Lookup::Missing(r) => return Lookup::Missing(r),
            Lookup::Unknown(r) => return Lookup::Unknown(r),
        };
        let Some(key) = h.key() else {
            return Lookup::Unknown(UnknownReason::NotLoaded);
        };
        match self.children(&key, ObjectKind::Column) {
            Lookup::Found(_) => Lookup::Found(
                self.layers()
                    .cache
                    .columns(&h.ns, h.id.idx, || build_columns(&h.ns, h.id.idx)),
            ),
            Lookup::Missing(r) => Lookup::Missing(r),
            Lookup::Unknown(r) => Lookup::Unknown(r),
        }
    }

    fn functions(&self, schema: &IdentKey, name: &IdentKey) -> Lookup<Arc<[FnSig]>> {
        note_slow_path();
        let p = match self.place(schema) {
            Place::At(p) => p,
            Place::Missing => return Lookup::Missing(MissingReason::NotFound),
            Place::Unknown(r) => return Lookup::Unknown(r),
        };
        let (mut out, builtins_known) = self.builtins(Some(name.as_str()));
        let mut found: Vec<FnSig> = Vec::new();
        if let Some(arena) = &p.arena {
            if !agrees(arena, self.sql_ctx()) {
                return Lookup::Unknown(UnknownReason::ServerDependent);
            }
            let schema_name = arena.root().name();
            for group in [KindGroup::Routines, KindGroup::Procedures] {
                for &i in arena.lookup(group, name.as_str()) {
                    if let Some(o) = arena.obj(i)
                        && o.parent() == Some(0)
                        && is_routine(o.kind())
                    {
                        found.extend(fn_sig(schema_name, o));
                    }
                }
            }
        }
        found.append(&mut out);
        if !found.is_empty() {
            return Lookup::Found(found.into());
        }
        // A miss: the schema's routines through the model view, then the built-ins.
        let routines = self.lookup(&Self::parts_in(&p, name.as_str()), KindGroup::Routines);
        let procedures = if mysql_family(self.snapshot().server.dbms) {
            self.lookup(&Self::parts_in(&p, name.as_str()), KindGroup::Procedures)
        } else {
            Lookup::Missing(MissingReason::NotFound)
        };
        match (routines, procedures) {
            (Lookup::Unknown(r), _) | (_, Lookup::Unknown(r)) => Lookup::Unknown(r),
            // A hit the arena index did not give (another rule): no signature to hand out.
            (Lookup::Found(_), _) | (_, Lookup::Found(_)) => {
                Lookup::Unknown(UnknownReason::ServerDependent)
            }
            // MySQL/MariaDB: the built-in names are a probed subset (`builtins`).
            (Lookup::Missing(_), Lookup::Missing(_)) if !builtins_known => {
                Lookup::Unknown(UnknownReason::NotLoaded)
            }
            (Lookup::Missing(_), Lookup::Missing(_))
                if mysql_family(self.snapshot().server.dbms) =>
            {
                Lookup::Unknown(UnknownReason::ServerDependent)
            }
            (Lookup::Missing(r), Lookup::Missing(_)) => Lookup::Missing(r),
        }
    }

    fn functions_in(&self, schema: &IdentKey) -> Lookup<Arc<[FnSig]>> {
        note_slow_path();
        let p = match self.place(schema) {
            Place::At(p) => p,
            Place::Missing => return Lookup::Missing(MissingReason::NotFound),
            Place::Unknown(r) => return Lookup::Unknown(r),
        };
        if let Err(r) = self.list_status(&p, None) {
            return Lookup::Unknown(r);
        }
        let (builtins, builtins_known) = self.builtins(None);
        if !builtins_known {
            return Lookup::Unknown(UnknownReason::NotLoaded);
        }
        let Some(arena) = &p.arena else {
            return Lookup::Unknown(UnknownReason::NotLoaded);
        };
        let schema_name = arena.root().name();
        let mut out: Vec<FnSig> = arena
            .children(0)
            .filter(|(_, o)| is_routine(o.kind()))
            .filter_map(|(_, o)| fn_sig(schema_name, o))
            .collect();
        out.extend(builtins);
        Lookup::Found(out.into())
    }

    fn types(&self, schema: &IdentKey) -> Lookup<Arc<[TypeInfo]>> {
        note_slow_path();
        let p = match self.place(schema) {
            Place::At(p) => p,
            Place::Missing => return Lookup::Missing(MissingReason::NotFound),
            Place::Unknown(r) => return Lookup::Unknown(r),
        };
        if let Err(r) = self.list_status(&p, None) {
            return Lookup::Unknown(r);
        }
        if p.system {
            let Some(sys) = &self.layers().system else {
                return Lookup::Unknown(UnknownReason::NotLoaded);
            };
            let schema_name = p.key.root_name();
            let out: Vec<TypeInfo> = sys
                .types
                .iter()
                .filter(|t| &*t.schema == schema_name)
                .map(|t| TypeInfo {
                    name: t.name.clone(),
                    // The snapshot keeps no domain base, range subtype or enum labels.
                    kind: if &*t.kind == "composite" {
                        TypeKind::Composite
                    } else {
                        TypeKind::Base
                    },
                })
                .collect();
            return Lookup::Found(out.into());
        }
        let Some(arena) = &p.arena else {
            return Lookup::Unknown(UnknownReason::NotLoaded);
        };
        let dbms = arena.dbms();
        let out: Vec<TypeInfo> = arena
            .children(0)
            .filter(|(_, o)| o.kind().group(dbms) == Some(KindGroup::Types))
            .filter_map(|(_, o)| type_info(o))
            .collect();
        Lookup::Found(out.into())
    }

    fn foreign_keys(&self, rel: &RelRef) -> Lookup<Arc<[FkEdge]>> {
        note_slow_path();
        let h = match self.relation_handle(rel) {
            Lookup::Found(h) => h,
            Lookup::Missing(r) => return Lookup::Missing(r),
            Lookup::Unknown(r) => return Lookup::Unknown(r),
        };
        // A snapshot holds no constraints: nothing certain about a system relation's keys.
        if h.obj().flags().contains(ObjFlags::PRE_INTROSPECTED) {
            return Lookup::Unknown(UnknownReason::NotLoaded);
        }
        let Some(key) = h.key() else {
            return Lookup::Unknown(UnknownReason::NotLoaded);
        };
        let outgoing = match self.children(&key, ObjectKind::ForeignKey) {
            Lookup::Found(hs) => hs,
            Lookup::Missing(r) => return Lookup::Missing(r),
            Lookup::Unknown(r) => return Lookup::Unknown(r),
        };
        let incoming = match self.snapshot().fk_referencing(&key) {
            Lookup::Found(hs) => hs,
            Lookup::Missing(r) => return Lookup::Missing(r),
            Lookup::Unknown(r) => return Lookup::Unknown(r),
        };
        let mut out: Vec<FkEdge> = outgoing.iter().filter_map(Self::fk_edge).collect();
        for e in incoming.iter().filter_map(Self::fk_edge) {
            if !out.contains(&e) {
                out.push(e);
            }
        }
        Lookup::Found(out.into())
    }

    fn virtual_fks(&self, rel: &RelRef) -> Arc<[FkEdge]> {
        note_slow_path();
        let Ok(p) = self.place_exact(&rel.schema) else {
            return Arc::from([]);
        };
        let key = ObjectKey {
            kind: ObjectKind::Table,
            path: p
                .key
                .path
                .iter()
                .cloned()
                .chain([rel.name.clone()])
                .collect(),
            server_id: None,
            overload_sig: None,
            raw_kind: None,
        };
        let edge = |fk: &crate::overlay::VirtualFk| -> Option<FkEdge> {
            let rel_ref = |k: &ObjectKey| {
                let n = k.path.len();
                (n >= 2).then(|| RelRef {
                    schema: k.path[n - 2].clone(),
                    name: k.path[n - 1].clone(),
                })
            };
            Some(FkEdge {
                from: rel_ref(&fk.from)?,
                from_cols: fk
                    .from_cols
                    .iter()
                    .map(|c| Name::from(c.as_str()))
                    .collect(),
                to: rel_ref(&fk.to)?,
                to_cols: fk.to_cols.iter().map(|c| Name::from(c.as_str())).collect(),
                name: fk.name.as_deref().map(Name::from),
                is_virtual: true,
            })
        };
        let mut out: Vec<FkEdge> = Vec::new();
        let layered = self
            .virtual_fks_of(&key)
            .into_iter()
            .chain(self.virtual_fks_to(&key));
        for l in layered {
            if l.flags.contains(ObjFlags::BROKEN) {
                continue;
            }
            if let Some(e) = edge(&l.fk)
                && !out.contains(&e)
            {
                out.push(e);
            }
        }
        out.into()
    }

    fn databases(&self) -> Lookup<Arc<[Name]>> {
        note_slow_path();
        let snap = self.snapshot();
        let dbms = snap.server.dbms;
        if mysql_family(dbms) {
            if !schema_list_loaded(snap, None) {
                return Lookup::Unknown(UnknownReason::NotLoaded);
            }
            let mut names: Vec<Name> = snap
                .namespaces
                .keys()
                .chain(snap.states.keys().filter_map(|c| match c {
                    ContainerKey::Namespace(k) => Some(k),
                    _ => None,
                }))
                .filter(|k| k.kind == NsKind::Schema && k.path.len() == 1)
                .map(|k| k.path[0].clone())
                .collect();
            names.sort();
            names.dedup();
            return Lookup::Found(names.into());
        }
        if pg_family(dbms) {
            let server = NsKey::server();
            let v = ns_status(snap, &server);
            return match (v.status, v.arena) {
                (NsStatus::Complete, Some(arena)) => Lookup::Found(
                    arena
                        .children_of_kind(0, ObjectKind::Database)
                        .map(|(_, o)| o.name().clone())
                        .collect(),
                ),
                (NsStatus::OutOfScope, _) => Lookup::Unknown(UnknownReason::OutOfScope),
                (NsStatus::Partial { reason, .. }, _) => Lookup::Unknown(reason),
                (NsStatus::Complete, None) => Lookup::Unknown(UnknownReason::NotLoaded),
            };
        }
        Lookup::Unknown(UnknownReason::NotLoaded)
    }

    fn settings(&self) -> Lookup<Arc<[SettingInfo]>> {
        let snap = self.snapshot();
        match &self.layers().system {
            Some(sys) if sys.dbms == snap.server.dbms && !sys.settings.is_empty() => {
                Lookup::Found(Arc::clone(&sys.settings))
            }
            _ => Lookup::Unknown(UnknownReason::NotLoaded),
        }
    }

    /// `None` until the catalog stores the `Statistics` facet's values (Task 15b).
    fn row_estimate(&self, _rel: &RelRef) -> Option<u64> {
        None
    }
}

impl SnapshotView {
    /// Whether the columns of live relation `idx` are all in `arena`: no kind-group state of
    /// the relation's columns says otherwise and no scope filter thins them.
    fn columns_complete(&self, p: &Placed, arena: &NamespaceSnapshot, idx: u32) -> bool {
        let Some(key) = arena.object_key(idx) else {
            return false;
        };
        let container = ContainerKey::Children {
            ns: p.key.clone(),
            parent: key,
            kind: ObjectKind::Column,
        };
        let state_ok = match self.snapshot().states.get(&container) {
            None | Some(LoadState::Loaded { .. }) => true,
            Some(_) => false,
        };
        state_ok && !self.scope_excludes(&p.key, Some(ObjectKind::Column), None)
    }
}
