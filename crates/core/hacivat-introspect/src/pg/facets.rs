// ## m4-t01c-core (seam), filled by m4-t16b-pg-facets
//! PostgreSQL facets and the server-level namespace (R-M4-30; Task 16b): grants, dependencies
//! and row estimates of the objects of one schema, selected by oid, and the roles of the
//! `NsKind::Server` namespace. Every read is one pass of Task 16's (`pass::in_snapshot`:
//! `Identity` first, the sections of `packs/pg/20-facets.hvq.sql`, the canary, `COMMIT`) on a
//! read-only metadata connection, with the pinned `search_path` (R-M4-46).
//!
//! **What a read answers.** [`read`] answers [`FacetRows`] for the keys it could read
//! completely; a key it leaves out is not loaded (the engine keeps the facet `NotLoaded`, so a
//! DDL script says "grants not loaded", never a script that silently lacks them). A key is
//! left out when:
//! - it is not an object of `ns` the facet applies to, or has no oid (objects are selected by
//!   oid only, inside the namespace: no name reaches the server);
//! - its oid now names another object (the row's name or kind differs: a rename, a drop and
//!   a reused oid; the namespace's next pass sees it);
//! - for grants, its ACL says something a GRANT-only [`GrantsFacet`] cannot carry: a default
//!   privilege it revokes (`REVOKE EXECUTE … FROM PUBLIC` on a function: re-granting the
//!   facet's entries would hand `PUBLIC` back its default), a grantee without a role row, a
//!   role named exactly `PUBLIC` (the DDL renderer writes that name as the keyword), or a
//!   privilege that is not a keyword. [`grants`] reports these per object
//!   ([`ObjGrants::revoked`], [`ObjGrants::opaque`]).
//!
//! A refused read (`42501`: a hardened server that revoked `SELECT` on a catalog) is an
//! error for the whole read ([`IntroError::Permission`]), never an empty answer; so is a
//! refused role list ([`server_namespace`]).
//!
//! **Grants** are never parsed from ACL text (a quoted role name may hold `=`, `/`, `,` or
//! `"`): `aclexplode` returns the items as rows (grantee oid, privilege keyword, grant
//! option) and the grantee is named by its `pg_roles` row (`0` is `PUBLIC`). A NULL ACL means
//! the default privileges (`acldefault`). The facet holds what the ACL adds to the defaults,
//! the way a dump restores privileges: the owner's own default privileges and `PUBLIC`'s
//! default `EXECUTE`/`USAGE` are not entries, and `CREATE` recreates them. Grantors are not
//! kept (one entry per grantee, privilege and column set; with grant option when any grantor
//! gave it). Column privileges (`attacl`) are entries of the table's facet, their columns in
//! `attnum` order. Owners are `Props.owner`, read with the structure by Task 16.
//!
//! **Dependencies** are the `pg_depend` rows (deptype not `e`, `x` or `p`) of the object and
//! of its parts (a relation's view rules, constraints, column defaults, user triggers and
//! indexes; a type's domain constraints and a composite type's attributes), each referenced
//! object as a [`Ref`] (kind, exact path, oid; a column by its path only), self references
//! and references to the object's own parts left out, one reference per object
//! ([`DepsKind::Catalog`]). Built-in objects are pinned and never recorded by the server.
//!
//! **Statistics** re-read `reltuples` on demand: VACUUM and ANALYZE update it in place, so an
//! incremental pass never sees it move (Task 16 review m-5).
//!
//! **Invalidation.** A `GRANT` rewrites the object's own catalog row (a column `GRANT` its
//! `pg_attribute` row, which Task 16 cascades to the relation), so the object is re-read by
//! the next pass; [`invalidated`] names the objects whose loaded [`INVALIDATED_FACETS`] a pass
//! result makes stale (Task 15b drops them, and the next `load_facet` re-reads).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::SystemTime;

use futures::future::BoxFuture;
use hacivat_meta::merge::{ChangedObject, ExistSet, NamespaceResult};
use hacivat_meta::props::{DepsFacet, DepsKind, FacetKind, Grant, GrantsFacet, StatsFacet};
use hacivat_meta::{
    Level, Name, NsKey, Obj, ObjectKey, ObjectKind, Ref, ServerId, Subtree, Watermark,
};
use hacivat_session::meta::MetaConn;
use smallvec::SmallVec;
use tokio_util::sync::CancellationToken;

use super::build::{self, BASE_TYPE_RAW_KIND, Ns};
use super::decode::{IdentityRow, Row, row_estimate};
use super::pass::{PassHooks, PassReport, in_snapshot, now_ms};
use super::sections::{Binds, QueryLog, Reader};
use crate::introspector::{FacetRows, IntroError};
use crate::pack::Mode;

/// The section names of `packs/pg/20-facets.hvq.sql`.
pub mod name {
    pub const GRANTS_RELATION: &str = "GrantsRelation";
    pub const GRANTS_ROUTINE: &str = "GrantsRoutine";
    pub const GRANTS_TYPE: &str = "GrantsType";
    pub const GRANTS_SCHEMA: &str = "GrantsSchema";
    pub const DEPENDENCIES: &str = "Dependencies";
    pub const STATISTICS_RELATION: &str = "StatisticsRelation";
    pub const LIST_ROLES: &str = "ListRoles";
    /// Every section of the file.
    pub const ALL: [&str; 7] = [
        GRANTS_RELATION,
        GRANTS_ROUTINE,
        GRANTS_TYPE,
        GRANTS_SCHEMA,
        DEPENDENCIES,
        STATISTICS_RELATION,
        LIST_ROLES,
    ];
}

/// The facets the PostgreSQL introspector reads (`IntroCaps.facets`). `Sources` is a level
/// (`Level::Sources`), not a facet read, on PostgreSQL.
pub const FACETS: [FacetKind; 3] = [
    FacetKind::Grants,
    FacetKind::Dependencies,
    FacetKind::Statistics,
];

/// The loaded facets a re-read of their object makes stale ([`invalidated`]).
pub const INVALIDATED_FACETS: [FacetKind; 2] = [FacetKind::Grants, FacetKind::Dependencies];

/// The grantee `aclexplode` reports as oid `0`, and the keyword the DDL renderer writes.
pub const PUBLIC: &str = "PUBLIC";

/// The catalog an object's facets are read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Class {
    /// `pg_class`: tables, views, materialized views, foreign tables, sequences.
    Relation,
    /// `pg_proc`.
    Routine,
    /// `pg_type`: domains, enum, composite, range and base types.
    Type,
    /// `pg_namespace`: the namespace's own schema.
    Schema,
}

/// What a facet read selects `key` by: its catalog and its oid (`None` for the namespace's
/// schema, which the namespace oid selects). `None` when `key` is not an object of `ns` that
/// `facet` applies to, or has no oid.
pub fn target(ns: &NsKey, key: &ObjectKey, facet: FacetKind) -> Option<(Class, Option<u32>)> {
    Ns::new(ns)?;
    if key.kind == ObjectKind::Schema {
        let own = key.path.as_slice() == ns.path.as_slice();
        return (own && facet == FacetKind::Grants).then_some((Class::Schema, None));
    }
    let top = key.path.len() == ns.path.len() + 1 && key.path.starts_with(&ns.path);
    let Some(ServerId::Oid(oid)) = key.server_id else {
        return None;
    };
    if !top {
        return None;
    }
    let class = match key.kind {
        ObjectKind::Table
        | ObjectKind::View
        | ObjectKind::MaterializedView
        | ObjectKind::ForeignTable
        | ObjectKind::Sequence => Class::Relation,
        ObjectKind::Routine | ObjectKind::Procedure | ObjectKind::Aggregate => Class::Routine,
        ObjectKind::Domain
        | ObjectKind::EnumType
        | ObjectKind::CompositeType
        | ObjectKind::RangeType => Class::Type,
        ObjectKind::Unknown if key.raw_kind.as_deref() == Some(BASE_TYPE_RAW_KIND) => Class::Type,
        _ => return None,
    };
    let applies = match facet {
        FacetKind::Grants | FacetKind::Dependencies => true,
        FacetKind::Statistics => matches!(
            key.kind,
            ObjectKind::Table | ObjectKind::MaterializedView | ObjectKind::ForeignTable
        ),
        FacetKind::Sources => false,
    };
    applies.then_some((class, Some(oid)))
}

/// The model kind a facet section's object row stands for (`kind` is the row's `relkind`,
/// `prokind` or `typtype`).
fn row_kind(class: Class, kind: &str) -> Option<ObjectKind> {
    match class {
        Class::Relation if kind == "S" => Some(ObjectKind::Sequence),
        Class::Relation => build::existent_kind("rel", Some(kind)),
        Class::Routine => build::existent_kind("fn", Some(kind)),
        Class::Type => build::existent_kind("typ", Some(kind)),
        Class::Schema => Some(ObjectKind::Schema),
    }
}

/// Whether an object row (`own`, `kind`) is still `key`'s object.
fn same_object(key: &ObjectKey, class: Class, own: &str, kind: &str) -> bool {
    key.name() == own && row_kind(class, kind) == Some(key.kind)
}

// ---- grants -----------------------------------------------------------------------------------

/// Where a Grants row comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AclSrc {
    /// The object's own row (no item): it exists, with this name and kind.
    Obj,
    /// An item of its ACL (the defaults where the ACL is NULL).
    Acl,
    /// An item of its default ACL.
    Default,
    /// An item of a column's ACL.
    Column,
}

/// One `aclexplode` item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AclItem {
    /// The grantee's oid; `0` is `PUBLIC`.
    pub grantee: u32,
    /// The grantee's `pg_roles.rolname` (`None`: no role row).
    pub grantee_name: Option<String>,
    pub privilege: String,
    pub grantable: bool,
}

/// One row of a Grants section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AclRow {
    pub oid: u32,
    pub src: AclSrc,
    /// The object's name.
    pub own: String,
    /// Its `relkind`, `prokind`, `typtype` (`n` for a schema).
    pub kind: String,
    /// A column item's `(attnum, attname)`.
    pub col: Option<(i32, String)>,
    /// `None` on the object row.
    pub item: Option<AclItem>,
}

/// Decodes a Grants row (`oid, src, own, kind, col, colnum, grantee, grantee_name, privilege,
/// grantable`).
pub fn acl_row(r: &Row<'_>) -> Result<AclRow, IntroError> {
    let src = match r.text(1)? {
        "obj" => AclSrc::Obj,
        "acl" => AclSrc::Acl,
        "dflt" => AclSrc::Default,
        "col" => AclSrc::Column,
        _ => return Err(r.err(1, "obj, acl, dflt or col")),
    };
    let item = if src == AclSrc::Obj {
        None
    } else {
        Some(AclItem {
            grantee: r.num(6)?,
            grantee_name: r.opt_string(7),
            privilege: r.string(8)?,
            grantable: r.bool(9)?,
        })
    };
    let col = if src == AclSrc::Column {
        Some((r.num(5)?, r.string(4)?))
    } else {
        None
    };
    Ok(AclRow {
        oid: r.num(0)?,
        src,
        own: r.string(2)?,
        kind: r.string(3)?,
        col,
        item,
    })
}

/// One object's grants as read, with what the facet cannot carry.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjGrants {
    pub key: ObjectKey,
    /// What the ACL adds to the defaults (module docs).
    pub facet: GrantsFacet,
    /// Default privileges the ACL takes away (a GRANT-only facet cannot say so).
    pub revoked: Vec<Grant>,
    /// An item the facet cannot carry faithfully: a grantee without a role row, a role named
    /// exactly `PUBLIC`, a privilege that is not a keyword.
    pub opaque: bool,
}

impl ObjGrants {
    /// Whether the facet says everything about the object's privileges ([`read`] answers only
    /// complete objects).
    pub fn complete(&self) -> bool {
        self.revoked.is_empty() && !self.opaque
    }
}

/// The grants of `key` from its rows (its object row matched): module docs.
pub fn decide_grants(key: ObjectKey, rows: &[AclRow]) -> ObjGrants {
    let mut opaque = false;
    // The grantee's name and the privilege, or `None` (and opaque) when the facet cannot
    // carry them faithfully.
    let mut carry = |it: &AclItem| -> Option<(Name, Name)> {
        let privilege_ok = !it.privilege.is_empty()
            && it
                .privilege
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b == b' ')
            && !it.privilege.starts_with(' ')
            && !it.privilege.ends_with(' ');
        let who = match (it.grantee, it.grantee_name.as_deref()) {
            (0, _) => Some(PUBLIC),
            (_, Some(n)) if n != PUBLIC => Some(n),
            _ => None,
        };
        match who {
            Some(who) if privilege_ok => Some((Name::from(who), Name::from(it.privilege.as_str()))),
            _ => {
                opaque = true;
                None
            }
        }
    };
    let items = |src: AclSrc| {
        rows.iter()
            .filter(move |r| r.src == src)
            .filter_map(|r| r.item.as_ref())
    };
    let same = |a: &AclItem, b: &AclItem| a.grantee == b.grantee && a.privilege == b.privilege;
    // Object level: what the ACL adds to the defaults, one entry per grantee and privilege
    // (with grant option when any grantor gave it).
    let mut object: BTreeMap<(Name, Name), bool> = BTreeMap::new();
    for a in items(AclSrc::Acl) {
        let in_default = items(AclSrc::Default).any(|d| same(a, d) && d.grantable == a.grantable);
        if in_default {
            continue;
        }
        if let Some(k) = carry(a) {
            *object.entry(k).or_default() |= a.grantable;
        }
    }
    // A default the ACL no longer holds (or holds without the default's grant option).
    let mut revoked = Vec::new();
    for d in items(AclSrc::Default) {
        let kept = items(AclSrc::Acl).any(|a| same(a, d) && (a.grantable || !d.grantable));
        if kept {
            continue;
        }
        if let Some((grantee, privilege)) = carry(d) {
            revoked.push(Grant {
                grantee,
                privilege,
                columns: Vec::new(),
                grantable: d.grantable,
            });
        }
    }
    // Column level: per grantee, privilege and column, then the columns of each
    // (grantee, privilege, grant option) in attnum order.
    let mut per_column: BTreeMap<(Name, Name, i32, Name), bool> = BTreeMap::new();
    for r in rows.iter().filter(|r| r.src == AclSrc::Column) {
        let (Some(it), Some((num, col))) = (r.item.as_ref(), r.col.as_ref()) else {
            continue;
        };
        if let Some((who, privilege)) = carry(it) {
            *per_column
                .entry((who, privilege, *num, Name::from(col.as_str())))
                .or_default() |= it.grantable;
        }
    }
    let mut columns: BTreeMap<(Name, Name, bool), Vec<Name>> = BTreeMap::new();
    for ((who, privilege, _, col), grantable) in per_column {
        columns
            .entry((who, privilege, grantable))
            .or_default()
            .push(col);
    }
    let mut entries: Vec<Grant> = object
        .into_iter()
        .map(|((grantee, privilege), grantable)| Grant {
            grantee,
            privilege,
            columns: Vec::new(),
            grantable,
        })
        .chain(
            columns
                .into_iter()
                .map(|((grantee, privilege, grantable), columns)| Grant {
                    grantee,
                    privilege,
                    columns,
                    grantable,
                }),
        )
        .collect();
    // A stable order: grantee, privilege, then the object entry before its column entries.
    entries.sort_by(|a, b| {
        (&a.grantee, &a.privilege, &a.columns, a.grantable).cmp(&(
            &b.grantee,
            &b.privilege,
            &b.columns,
            b.grantable,
        ))
    });
    revoked.sort_by(|a, b| (&a.grantee, &a.privilege).cmp(&(&b.grantee, &b.privilege)));
    revoked.dedup();
    ObjGrants {
        key,
        facet: GrantsFacet { entries },
        revoked,
        opaque,
    }
}

// ---- dependencies -----------------------------------------------------------------------------

/// One row of the Dependencies section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepRow {
    pub oid: u32,
    /// `rel`, `fn` or `typ`.
    pub cls: String,
    pub own: String,
    pub kind: String,
    /// The depender's own relation (a relation's oid, a composite type's `typrelid`; else 0).
    pub selfrel: u32,
    /// `class`, `column`, `proc`, `type`, `namespace`, `constraint`, `other`, or `none` (the
    /// part depends on nothing: the row only proves the object exists).
    pub refcat: String,
    pub refid: u32,
    pub refsub: i32,
    /// `relkind`, `prokind`, `typtype`, `contype`, or `pg_identify_object`'s type.
    pub refkind: Option<String>,
    pub refschema: Option<String>,
    pub refname: Option<String>,
    /// A column's or an index's table, a constraint's table or domain.
    pub refparent: Option<String>,
    /// The relation or type that owns the referenced object (0: none).
    pub refowner: u32,
}

/// Decodes a Dependencies row.
pub fn dep_row(r: &Row<'_>) -> Result<DepRow, IntroError> {
    Ok(DepRow {
        oid: r.num(0)?,
        cls: r.string(1)?,
        own: r.string(2)?,
        kind: r.string(3)?,
        selfrel: r.num(4)?,
        refcat: r.string(5)?,
        refid: r.opt_num(6)?.unwrap_or(0),
        refsub: r.opt_num(7)?.unwrap_or(0),
        refkind: r.opt_string(8),
        refschema: r.opt_string(9),
        refname: r.opt_string(10),
        refparent: r.opt_string(11),
        refowner: r.num(12)?,
    })
}

/// The dependencies of the object `oid` of database `db` from its rows: module docs.
pub fn decide_deps(db: &Name, oid: u32, rows: &[DepRow]) -> Result<DepsFacet, IntroError> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for row in rows {
        if is_own(oid, row) {
            continue;
        }
        if let Some(r) = dep_ref(db, row)?
            && seen.insert(r.clone())
        {
            out.push(r);
        }
    }
    out.sort_by(|a, b| {
        let oid = |r: &Ref| match r.server_id {
            Some(ServerId::Oid(o)) => o,
            _ => 0,
        };
        (a.kind.code(), a.path.as_slice(), oid(a)).cmp(&(b.kind.code(), b.path.as_slice(), oid(b)))
    });
    Ok(DepsFacet {
        depends_on: out,
        kind: DepsKind::Catalog,
    })
}

/// Whether `row` refers back to its depender `oid` or one of its parts: a relation's own
/// row, columns, indexes and constraints; a type's own row and domain constraints, and a
/// composite type's relation row and attributes (`selfrel`); a routine itself.
fn is_own(oid: u32, row: &DepRow) -> bool {
    let rel_side = matches!(row.refcat.as_str(), "class" | "column" | "constraint");
    match row.cls.as_str() {
        "rel" => rel_side && row.refowner == oid,
        "typ" => {
            (matches!(row.refcat.as_str(), "type" | "constraint") && row.refowner == oid)
                || (row.selfrel != 0
                    && matches!(row.refcat.as_str(), "class" | "column")
                    && row.refowner == row.selfrel)
        }
        "fn" => row.refcat == "proc" && row.refid == oid,
        _ => false,
    }
}

/// The model kind of a relation's `relkind` (`None` for a TOAST table, which is never a
/// dependency a user sees).
fn relkind_kind(k: &str) -> Option<ObjectKind> {
    Some(match k {
        "r" | "p" => ObjectKind::Table,
        "v" => ObjectKind::View,
        "m" => ObjectKind::MaterializedView,
        "f" => ObjectKind::ForeignTable,
        "S" => ObjectKind::Sequence,
        "i" | "I" => ObjectKind::Index,
        "c" => ObjectKind::CompositeType,
        "t" => return None,
        _ => ObjectKind::Unknown,
    })
}

/// The model kind of a `pg_identify_object` type of the catalogs the section does not join.
fn identified_kind(t: &str) -> ObjectKind {
    match t {
        "collation" => ObjectKind::Collation,
        "operator" => ObjectKind::Operator,
        "operator class" => ObjectKind::OperatorClass,
        "operator family" => ObjectKind::OperatorFamily,
        "access method" => ObjectKind::AccessMethod,
        "language" => ObjectKind::Language,
        "extension" => ObjectKind::Extension,
        "cast" => ObjectKind::Cast,
        "foreign-data wrapper" => ObjectKind::ForeignDataWrapper,
        "server" => ObjectKind::ForeignServer,
        "event trigger" => ObjectKind::EventTrigger,
        "trigger" => ObjectKind::Trigger,
        "rule" => ObjectKind::Rule,
        "policy" => ObjectKind::Policy,
        "default value" => ObjectKind::Default,
        _ => ObjectKind::Unknown,
    }
}

/// The reference a dependency row names (`None`: nothing, or a TOAST table). A referenced
/// object without a name was dropped while the pass read it (`CatalogRace`, retried); a
/// schema-scoped object without a schema is a protocol error.
fn dep_ref(db: &Name, row: &DepRow) -> Result<Option<Ref>, IntroError> {
    if row.refcat == "none" {
        return Ok(None);
    }
    let name = row.refname.as_deref().ok_or(IntroError::CatalogRace)?;
    let kind_text = row.refkind.as_deref().unwrap_or("");
    let schema = || {
        row.refschema
            .as_deref()
            .ok_or_else(|| IntroError::Protocol(Name::from("pg/Dependencies: no schema")))
    };
    let parent = || {
        row.refparent
            .as_deref()
            .ok_or_else(|| IntroError::Protocol(Name::from("pg/Dependencies: no parent")))
    };
    let first = kind_text.chars().next();
    let (kind, path): (ObjectKind, Vec<&str>) = match row.refcat.as_str() {
        "class" => {
            let Some(kind) = relkind_kind(kind_text) else {
                return Ok(None);
            };
            if kind == ObjectKind::Index {
                (kind, vec![schema()?, parent()?, name])
            } else {
                (kind, vec![schema()?, name])
            }
        }
        "column" => (ObjectKind::Column, vec![schema()?, parent()?, name]),
        "proc" => (
            build::existent_kind("fn", Some(kind_text)).unwrap_or(ObjectKind::Routine),
            vec![schema()?, name],
        ),
        "type" => (
            match first {
                Some('d') => ObjectKind::Domain,
                Some('e') => ObjectKind::EnumType,
                Some('c') => ObjectKind::CompositeType,
                Some('r') => ObjectKind::RangeType,
                _ => ObjectKind::Unknown,
            },
            vec![schema()?, name],
        ),
        "namespace" => (ObjectKind::Schema, vec![name]),
        "constraint" => (
            match first {
                Some('p') => ObjectKind::PrimaryKey,
                Some('u') => ObjectKind::UniqueKey,
                Some('f') => ObjectKind::ForeignKey,
                Some('c') => ObjectKind::Check,
                Some('x') => ObjectKind::Exclusion,
                _ => ObjectKind::Unknown,
            },
            vec![schema()?, parent()?, name],
        ),
        "other" => {
            let mut p = Vec::new();
            if let Some(s) = row.refschema.as_deref() {
                p.push(s);
            }
            p.push(name);
            (identified_kind(kind_text), p)
        }
        _ => {
            return Err(IntroError::Protocol(Name::from(
                "pg/Dependencies: unknown reference class",
            )));
        }
    };
    let mut full: SmallVec<[Name; 4]> = SmallVec::new();
    full.push(db.clone());
    full.extend(path.into_iter().map(Name::from));
    Ok(Some(Ref {
        kind,
        path: full,
        server_id: (row.refcat != "column").then_some(ServerId::Oid(row.refid)),
    }))
}

// ---- the server namespace ---------------------------------------------------------------------

/// The `NsKind::Server` namespace of `roles` (`(oid, rolname)`), read at `read_at_ms`: a full
/// `Names`-level result whose `Role` existence set is complete.
pub fn roles_result(roles: Vec<(u32, String)>, read_at_ms: i64) -> NamespaceResult {
    let root = ObjectKey {
        kind: ObjectKind::Root,
        path: SmallVec::new(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    };
    let mut ids = Vec::with_capacity(roles.len());
    let mut changed = Vec::with_capacity(roles.len());
    for (oid, role) in roles {
        let o = Obj::new(ObjectKind::Role, Name::from(role.as_str()))
            .with_server_id(ServerId::Oid(oid));
        ids.push(build::top_identity(&o));
        changed.push(ChangedObject {
            parent: root.clone(),
            subtree: Subtree::new(o),
        });
    }
    NamespaceResult {
        existent: BTreeMap::from([(ObjectKind::Role, ExistSet::Ok(ids))]),
        changed,
        categories_to_reload: BTreeSet::new(),
        new_watermark: Watermark::None,
        full: true,
        level: Level::Names,
        read_at_ms,
    }
}

// ---- invalidation -----------------------------------------------------------------------------

/// The objects of `ns` whose loaded [`INVALIDATED_FACETS`] the pass `result` makes stale:
/// every object it re-read (module docs). A full pass re-reads everything.
pub fn invalidated(ns: &NsKey, result: &NamespaceResult) -> Vec<ObjectKey> {
    let root_kind = ns.kind.root_kind();
    result
        .changed
        .iter()
        .map(|c| {
            let o = c.subtree.root_obj();
            // The namespace's own root (a re-read schema row) is the root key itself.
            let path: SmallVec<[Name; 4]> =
                if o.kind() == root_kind && c.parent.path.as_slice() == ns.path.as_slice() {
                    c.parent.path.clone()
                } else {
                    let mut p = c.parent.path.clone();
                    p.push(o.name().clone());
                    p
                };
            ObjectKey {
                kind: o.kind(),
                path,
                server_id: o.server_id(),
                overload_sig: o.props().overload_sig().cloned(),
                raw_kind: o.raw_kind().cloned(),
            }
        })
        .collect()
}

// ---- the reads --------------------------------------------------------------------------------

/// The keys of one read: `(key, class, oid)`.
struct Targets(Vec<(ObjectKey, Class, Option<u32>)>);

impl Targets {
    fn of(ns: &NsKey, keys: &[ObjectKey], facet: FacetKind) -> Targets {
        Targets(
            keys.iter()
                .filter_map(|k| target(ns, k, facet).map(|(c, o)| (k.clone(), c, o)))
                .collect(),
        )
    }

    fn oids(&self, class: Class) -> Vec<u32> {
        let set: BTreeSet<u32> = self
            .0
            .iter()
            .filter(|(_, c, _)| *c == class)
            .filter_map(|(_, _, o)| *o)
            .collect();
        set.into_iter().collect()
    }

    fn has(&self, class: Class) -> bool {
        self.0.iter().any(|(_, c, _)| *c == class)
    }
}

/// The namespace's oid from the probe (`None`: the schema is gone, so none of its objects
/// is); a probe of another database is the engine's mistake.
fn ns_oid(nsx: &Ns, id: &IdentityRow) -> Result<Option<u32>, IntroError> {
    if *id.db_name != *nsx.db {
        return Err(IntroError::Protocol(Name::from("pg/wrong-database")));
    }
    Ok(id.ns_oid)
}

/// The grants of `keys` (module docs), every object read, complete or not.
pub async fn grants(
    c: &mut MetaConn,
    ns: &NsKey,
    keys: &[ObjectKey],
    cancel: &CancellationToken,
) -> Result<(Vec<ObjGrants>, QueryLog), IntroError> {
    let (Some(nsx), wanted) = (Ns::new(ns), Targets::of(ns, keys, FacetKind::Grants)) else {
        return Ok((Vec::new(), QueryLog::default()));
    };
    if wanted.0.is_empty() {
        return Ok((Vec::new(), QueryLog::default()));
    }
    let mut report = PassReport::default();
    let read = in_snapshot(
        c,
        &nsx.schema,
        cancel,
        &mut report,
        PassHooks::default(),
        async |r: &mut Reader<'_, '_>, id: &IdentityRow| {
            let Some(oid) = ns_oid(&nsx, id)? else {
                return Ok(None);
            };
            let mut rows: BTreeMap<(Class, u32), Vec<AclRow>> = BTreeMap::new();
            for (class, section) in [
                (Class::Relation, name::GRANTS_RELATION),
                (Class::Routine, name::GRANTS_ROUTINE),
                (Class::Type, name::GRANTS_TYPE),
            ] {
                let oids = wanted.oids(class);
                if oids.is_empty() {
                    continue;
                }
                let b = Binds::ns(oid).with_oids(oids);
                for row in r
                    .read(section, Mode::Full, Level::Structure, &b, acl_row)
                    .await?
                {
                    rows.entry((class, row.oid)).or_default().push(row);
                }
            }
            if wanted.has(Class::Schema) {
                for row in r
                    .read(
                        name::GRANTS_SCHEMA,
                        Mode::Full,
                        Level::Structure,
                        &Binds::ns(oid),
                        acl_row,
                    )
                    .await?
                {
                    rows.entry((Class::Schema, row.oid)).or_default().push(row);
                }
            }
            Ok(Some((oid, rows)))
        },
    )
    .await?;
    let Some((ns_oid, rows)) = read else {
        return Ok((Vec::new(), report.log));
    };
    let mut out = Vec::new();
    for (key, class, oid) in wanted.0 {
        let Some(rows) = rows.get(&(class, oid.unwrap_or(ns_oid))) else {
            continue;
        };
        let matched = rows
            .iter()
            .find(|r| r.src == AclSrc::Obj)
            .is_some_and(|r| same_object(&key, class, &r.own, &r.kind));
        if matched {
            out.push(decide_grants(key, rows));
        }
    }
    Ok((out, report.log))
}

/// The dependencies of `keys` (module docs).
pub async fn dependencies(
    c: &mut MetaConn,
    ns: &NsKey,
    keys: &[ObjectKey],
    cancel: &CancellationToken,
) -> Result<(Vec<(ObjectKey, DepsFacet)>, QueryLog), IntroError> {
    let (Some(nsx), wanted) = (Ns::new(ns), Targets::of(ns, keys, FacetKind::Dependencies)) else {
        return Ok((Vec::new(), QueryLog::default()));
    };
    let oids: Vec<u32> = {
        let set: BTreeSet<u32> = wanted.0.iter().filter_map(|(_, _, o)| *o).collect();
        set.into_iter().collect()
    };
    if oids.is_empty() {
        return Ok((Vec::new(), QueryLog::default()));
    }
    let mut report = PassReport::default();
    let read = in_snapshot(
        c,
        &nsx.schema,
        cancel,
        &mut report,
        PassHooks::default(),
        async |r: &mut Reader<'_, '_>, id: &IdentityRow| {
            let Some(oid) = ns_oid(&nsx, id)? else {
                return Ok(Vec::new());
            };
            let b = Binds::ns(oid).with_oids(oids.clone());
            r.read(
                name::DEPENDENCIES,
                Mode::Full,
                Level::Structure,
                &b,
                dep_row,
            )
            .await
        },
    )
    .await?;
    // The rows of one depender (catalog tag, oid). Every live object has at least its own
    // row (`refcat` none when it depends on nothing), so no row means the object is gone.
    let mut by: BTreeMap<(&str, u32), Vec<DepRow>> = BTreeMap::new();
    for row in read {
        let tag = match row.cls.as_str() {
            "rel" => "rel",
            "fn" => "fn",
            "typ" => "typ",
            _ => {
                return Err(IntroError::Protocol(Name::from(
                    "pg/Dependencies: unknown depender class",
                )));
            }
        };
        by.entry((tag, row.oid)).or_default().push(row);
    }
    let mut out = Vec::new();
    for (key, class, oid) in wanted.0 {
        let tag = match class {
            Class::Relation => "rel",
            Class::Routine => "fn",
            Class::Type => "typ",
            Class::Schema => continue,
        };
        let Some(oid) = oid else { continue };
        let Some(rows) = by.get(&(tag, oid)) else {
            continue;
        };
        if !rows
            .first()
            .is_some_and(|r| same_object(&key, class, &r.own, &r.kind))
        {
            continue;
        }
        out.push((key, decide_deps(&nsx.db, oid, rows)?));
    }
    Ok((out, report.log))
}

/// The row estimates of `keys` (tables, materialized views, foreign tables), read now.
pub async fn statistics(
    c: &mut MetaConn,
    ns: &NsKey,
    keys: &[ObjectKey],
    cancel: &CancellationToken,
) -> Result<(Vec<(ObjectKey, StatsFacet)>, QueryLog), IntroError> {
    let (Some(nsx), wanted) = (Ns::new(ns), Targets::of(ns, keys, FacetKind::Statistics)) else {
        return Ok((Vec::new(), QueryLog::default()));
    };
    let oids = wanted.oids(Class::Relation);
    if oids.is_empty() {
        return Ok((Vec::new(), QueryLog::default()));
    }
    let mut report = PassReport::default();
    let read = in_snapshot(
        c,
        &nsx.schema,
        cancel,
        &mut report,
        PassHooks::default(),
        async |r: &mut Reader<'_, '_>, id: &IdentityRow| {
            let Some(oid) = ns_oid(&nsx, id)? else {
                return Ok(Vec::new());
            };
            let b = Binds::ns(oid).with_oids(oids.clone());
            r.read(
                name::STATISTICS_RELATION,
                Mode::Full,
                Level::Structure,
                &b,
                |row| {
                    let tuples: f64 = row.num(3)?;
                    Ok((row.num::<u32>(0)?, row.string(1)?, row.string(2)?, tuples))
                },
            )
            .await
        },
    )
    .await?;
    let at = SystemTime::now();
    let by: BTreeMap<u32, (String, String, f64)> = read
        .into_iter()
        .map(|(o, own, kind, t)| (o, (own, kind, t)))
        .collect();
    let mut out = Vec::new();
    for (key, class, oid) in wanted.0 {
        let Some((own, kind, tuples)) = oid.and_then(|o| by.get(&o)) else {
            continue;
        };
        if same_object(&key, class, own, kind) {
            out.push((
                key,
                StatsFacet {
                    row_estimate: row_estimate(*tuples),
                    at,
                },
            ));
        }
    }
    Ok((out, report.log))
}

/// The roles of the server as the `NsKind::Server` namespace ([`roles_result`]). A refused
/// read is `Permission { kind: Role }`: the namespace fails and lookups answer `Unknown`,
/// never "no roles".
pub async fn roles(
    c: &mut MetaConn,
    cancel: &CancellationToken,
) -> Result<(NamespaceResult, QueryLog), IntroError> {
    let mut report = PassReport::default();
    let rows = in_snapshot(
        c,
        &Name::from("pg_catalog"),
        cancel,
        &mut report,
        PassHooks::default(),
        async |r: &mut Reader<'_, '_>, _id: &IdentityRow| {
            r.read(
                name::LIST_ROLES,
                Mode::Full,
                Level::Names,
                &Binds::default(),
                |row| Ok((row.num::<u32>(0)?, row.string(1)?)),
            )
            .await
        },
    )
    .await
    .map_err(|e| match e {
        IntroError::Permission { .. } => IntroError::Permission {
            kind: ObjectKind::Role,
        },
        e => e,
    })?;
    Ok((roles_result(rows, now_ms()), report.log))
}

/// One facet of `keys` in `ns` (module docs: the keys answered are the ones read completely).
/// `Sources` is not a PostgreSQL facet read ([`FACETS`]): `NotImplemented`.
pub fn read<'a>(
    c: &'a mut MetaConn,
    ns: &'a NsKey,
    keys: &'a [ObjectKey],
    kind: FacetKind,
) -> BoxFuture<'a, Result<FacetRows, IntroError>> {
    Box::pin(async move {
        let cancel = CancellationToken::new();
        match kind {
            FacetKind::Grants => {
                let (all, _) = grants(c, ns, keys, &cancel).await?;
                Ok(FacetRows::Grants(
                    all.into_iter()
                        .filter(ObjGrants::complete)
                        .map(|g| (g.key, g.facet))
                        .collect(),
                ))
            }
            FacetKind::Dependencies => Ok(FacetRows::Dependencies(
                dependencies(c, ns, keys, &cancel).await?.0,
            )),
            FacetKind::Statistics => Ok(FacetRows::Statistics(
                statistics(c, ns, keys, &cancel).await?.0,
            )),
            FacetKind::Sources => Err(IntroError::NotImplemented),
        }
    })
}

/// The server-level namespace: the roles ([`roles`]).
pub fn server_namespace(c: &mut MetaConn) -> BoxFuture<'_, Result<NamespaceResult, IntroError>> {
    Box::pin(async move {
        let cancel = CancellationToken::new();
        roles(c, &cancel).await.map(|(r, _)| r)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::{DialectId, lint};
    use crate::pg::sections::{ctx, pack};
    use hacivat_meta::merge::{MergeOutcome, MergePolicy, Trigger, base, merge};
    use hacivat_meta::name::FoldRule;
    use hacivat_proto::DbmsKind;

    const OWNER: u32 = 10;
    const READER: u32 = 20;

    fn n(s: &str) -> Name {
        Name::from(s)
    }

    fn ns() -> NsKey {
        NsKey::schema(["hv_test", "s"])
    }

    fn key(kind: ObjectKind, own: &str, oid: Option<u32>) -> ObjectKey {
        ObjectKey {
            kind,
            path: ["hv_test", "s", own].into_iter().map(n).collect(),
            server_id: oid.map(ServerId::Oid),
            overload_sig: None,
            raw_kind: (kind == ObjectKind::Unknown).then(|| n(BASE_TYPE_RAW_KIND)),
        }
    }

    fn row(
        src: AclSrc,
        grantee: u32,
        who: Option<&str>,
        privilege: &str,
        grantable: bool,
    ) -> AclRow {
        AclRow {
            oid: 100,
            src,
            own: "t".into(),
            kind: "r".into(),
            col: None,
            item: Some(AclItem {
                grantee,
                grantee_name: who.map(str::to_string),
                privilege: privilege.into(),
                grantable,
            }),
        }
    }

    fn col(attnum: i32, column: &str, privilege: &str, grantable: bool) -> AclRow {
        AclRow {
            col: Some((attnum, column.into())),
            ..row(
                AclSrc::Column,
                READER,
                Some("app_reader"),
                privilege,
                grantable,
            )
        }
    }

    fn obj_row() -> AclRow {
        AclRow {
            item: None,
            ..row(AclSrc::Obj, 0, None, "", false)
        }
    }

    /// A table owner's default privileges (`acldefault('r', owner)` before PostgreSQL 17).
    fn owner_defaults(src: AclSrc) -> Vec<AclRow> {
        [
            "INSERT",
            "SELECT",
            "UPDATE",
            "DELETE",
            "TRUNCATE",
            "REFERENCES",
            "TRIGGER",
        ]
        .into_iter()
        .map(|p| row(src, OWNER, Some("app_owner"), p, false))
        .collect()
    }

    fn g(grantee: &str, privilege: &str, columns: &[&str], grantable: bool) -> Grant {
        Grant {
            grantee: n(grantee),
            privilege: n(privilege),
            columns: columns.iter().map(|c| n(c)).collect(),
            grantable,
        }
    }

    fn table() -> ObjectKey {
        key(ObjectKind::Table, "t", Some(100))
    }

    #[test]
    fn grants_keep_only_what_the_acl_adds_to_the_defaults() {
        // A NULL ACL is the defaults: nothing to grant, nothing revoked.
        let mut rows = vec![obj_row()];
        rows.extend(owner_defaults(AclSrc::Acl));
        rows.extend(owner_defaults(AclSrc::Default));
        let got = decide_grants(table(), &rows);
        assert!(got.complete(), "{got:?}");
        assert!(got.facet.entries.is_empty(), "{got:?}");
        // GRANT INSERT, UPDATE ... TO PUBLIC; GRANT SELECT ... TO app_reader WITH GRANT OPTION.
        rows.push(row(AclSrc::Acl, 0, None, "UPDATE", false));
        rows.push(row(AclSrc::Acl, READER, Some("app_reader"), "SELECT", true));
        rows.push(row(AclSrc::Acl, 0, None, "INSERT", false));
        let got = decide_grants(table(), &rows);
        assert!(got.complete(), "{got:?}");
        assert_eq!(
            got.facet.entries,
            vec![
                g(PUBLIC, "INSERT", &[], false),
                g(PUBLIC, "UPDATE", &[], false),
                g("app_reader", "SELECT", &[], true),
            ]
        );
        assert_eq!(got.key, table());
    }

    #[test]
    fn a_revoked_default_makes_the_object_incomplete() {
        // REVOKE EXECUTE ON FUNCTION f() FROM PUBLIC: the default PUBLIC item is gone.
        let f = key(ObjectKind::Routine, "f", Some(300));
        let rows = vec![
            obj_row(),
            row(AclSrc::Default, 0, None, "EXECUTE", false),
            row(AclSrc::Default, OWNER, Some("app_owner"), "EXECUTE", false),
            row(AclSrc::Acl, OWNER, Some("app_owner"), "EXECUTE", false),
        ];
        let got = decide_grants(f, &rows);
        assert!(!got.complete());
        assert_eq!(got.revoked, vec![g(PUBLIC, "EXECUTE", &[], false)]);
        assert!(got.facet.entries.is_empty());
        // The owner revoking their own privilege is a revocation too.
        let mut rows = vec![obj_row()];
        rows.extend(owner_defaults(AclSrc::Default));
        rows.extend(
            owner_defaults(AclSrc::Acl)
                .into_iter()
                .filter(|r| r.item.as_ref().is_some_and(|i| i.privilege != "TRUNCATE")),
        );
        let got = decide_grants(table(), &rows);
        assert_eq!(got.revoked, vec![g("app_owner", "TRUNCATE", &[], false)]);
        assert!(!got.complete());
    }

    #[test]
    fn public_is_grantee_zero_and_a_role_named_public_is_opaque() {
        let ok = decide_grants(
            table(),
            &[obj_row(), row(AclSrc::Acl, 0, None, "SELECT", false)],
        );
        assert_eq!(ok.facet.entries, vec![g(PUBLIC, "SELECT", &[], false)]);
        assert!(ok.complete());
        // CREATE ROLE "PUBLIC": the renderer would write the keyword, granting everyone.
        let twin = decide_grants(
            table(),
            &[
                obj_row(),
                row(AclSrc::Acl, 77, Some(PUBLIC), "SELECT", false),
            ],
        );
        assert!(twin.opaque && !twin.complete(), "{twin:?}");
        assert!(twin.facet.entries.is_empty(), "{twin:?}");
        // A grantee oid without a role row has no name to give.
        let ghost = decide_grants(
            table(),
            &[obj_row(), row(AclSrc::Acl, 78, None, "SELECT", false)],
        );
        assert!(ghost.opaque && !ghost.complete(), "{ghost:?}");
        // A privilege that is not a keyword is not carried.
        for bad in ["select", "SELECT;", "", " SELECT"] {
            let got = decide_grants(
                table(),
                &[obj_row(), row(AclSrc::Acl, READER, Some("r"), bad, false)],
            );
            assert!(got.opaque, "{bad:?}");
        }
    }

    #[test]
    fn hostile_role_names_are_kept_verbatim() {
        // Names an ACL text parser would split on: `=`, `/`, `,`, `"`, a space, a trailing
        // space, Turkish dotted capital I. aclexplode returns oids; the names come from
        // pg_roles rows, so nothing is ever split.
        for who in ["a=r/b,c", "q\"uote", "with space ", "\u{130}l", "x}{y"] {
            let got = decide_grants(
                table(),
                &[
                    obj_row(),
                    row(AclSrc::Acl, READER, Some(who), "SELECT", false),
                ],
            );
            assert!(got.complete(), "{who:?}");
            assert_eq!(
                got.facet.entries,
                vec![g(who, "SELECT", &[], false)],
                "{who:?}"
            );
        }
    }

    #[test]
    fn column_grants_merge_columns_in_attnum_order_and_collapse_grantors() {
        let rows = vec![
            obj_row(),
            col(2, "b", "SELECT", false),
            col(1, "a", "SELECT", false),
            col(2, "b", "UPDATE", false),
            // A second grantor gave the grant option on `a`.
            col(1, "a", "SELECT", true),
            col(3, "c", "SELECT", false),
            // The same privilege from two grantors on the object.
            row(AclSrc::Acl, READER, Some("app_reader"), "DELETE", false),
            row(AclSrc::Acl, READER, Some("app_reader"), "DELETE", true),
        ];
        let got = decide_grants(table(), &rows);
        assert!(got.complete());
        assert_eq!(
            got.facet.entries,
            vec![
                g("app_reader", "DELETE", &[], true),
                g("app_reader", "SELECT", &["a"], true),
                g("app_reader", "SELECT", &["b", "c"], false),
                g("app_reader", "UPDATE", &["b"], false),
            ]
        );
    }

    fn dep(refcat: &str, refid: u32, refkind: &str, name: &str) -> DepRow {
        DepRow {
            oid: 500,
            cls: "rel".into(),
            own: "v".into(),
            kind: "v".into(),
            selfrel: 500,
            refcat: refcat.into(),
            refid,
            refsub: 0,
            refkind: Some(refkind.into()),
            refschema: Some("s".into()),
            refname: Some(name.into()),
            refparent: None,
            refowner: refid,
        }
    }

    fn r(kind: ObjectKind, path: &[&str], oid: Option<u32>) -> Ref {
        Ref {
            kind,
            path: path.iter().map(|p| n(p)).collect(),
            server_id: oid.map(ServerId::Oid),
        }
    }

    #[test]
    fn dependencies_name_each_catalog_and_drop_self_references() {
        let db = n("hv_test");
        let rows = vec![
            DepRow {
                refkind: None,
                refschema: None,
                refname: None,
                refowner: 0,
                ..dep("none", 0, "", "")
            },
            dep("class", 400, "r", "t"),
            dep("class", 400, "r", "t"),
            DepRow {
                refsub: 2,
                refparent: Some("t".into()),
                ..dep("column", 400, "r", "a")
            },
            dep("proc", 401, "f", "f"),
            dep("proc", 406, "p", "p"),
            dep("type", 402, "e", "mood"),
            dep("type", 407, "b", "citext"),
            DepRow {
                refschema: None,
                ..dep("namespace", 2200, "", "s")
            },
            DepRow {
                refparent: Some("child".into()),
                refowner: 410,
                ..dep("constraint", 403, "f", "child_fk")
            },
            DepRow {
                refparent: Some("t".into()),
                refowner: 400,
                ..dep("class", 404, "i", "t_pkey")
            },
            dep("class", 405, "c", "pair"),
            dep("other", 408, "collation", "c1"),
            DepRow {
                refschema: None,
                ..dep("other", 409, "language", "plpgsql")
            },
            dep("other", 411, "publication", "pub"),
            // The view itself, its own column, its own constraint: dropped.
            dep("class", 500, "v", "v"),
            DepRow {
                refsub: 1,
                refparent: Some("v".into()),
                ..dep("column", 500, "v", "x")
            },
            DepRow {
                refowner: 500,
                refparent: Some("v".into()),
                ..dep("constraint", 412, "c", "own_check")
            },
        ];
        let got = decide_deps(&db, 500, &rows).unwrap();
        assert_eq!(got.kind, DepsKind::Catalog);
        let want = [
            r(ObjectKind::Table, &["hv_test", "s", "t"], Some(400)),
            r(ObjectKind::Column, &["hv_test", "s", "t", "a"], None),
            r(ObjectKind::Routine, &["hv_test", "s", "f"], Some(401)),
            r(ObjectKind::Procedure, &["hv_test", "s", "p"], Some(406)),
            r(ObjectKind::EnumType, &["hv_test", "s", "mood"], Some(402)),
            r(ObjectKind::Unknown, &["hv_test", "s", "citext"], Some(407)),
            r(ObjectKind::Schema, &["hv_test", "s"], Some(2200)),
            r(
                ObjectKind::ForeignKey,
                &["hv_test", "s", "child", "child_fk"],
                Some(403),
            ),
            r(
                ObjectKind::Index,
                &["hv_test", "s", "t", "t_pkey"],
                Some(404),
            ),
            r(
                ObjectKind::CompositeType,
                &["hv_test", "s", "pair"],
                Some(405),
            ),
            r(ObjectKind::Collation, &["hv_test", "s", "c1"], Some(408)),
            r(ObjectKind::Language, &["hv_test", "plpgsql"], Some(409)),
            r(ObjectKind::Unknown, &["hv_test", "s", "pub"], Some(411)),
        ];
        assert_eq!(got.depends_on.len(), want.len(), "{:#?}", got.depends_on);
        for w in &want {
            assert!(
                got.depends_on.contains(w),
                "{w:?} missing: {:#?}",
                got.depends_on
            );
        }
        // Deterministic order: by kind code, then path.
        let mut sorted = got.depends_on.clone();
        sorted.sort_by_key(|x| (x.kind.code(), x.path.to_vec()));
        assert_eq!(got.depends_on, sorted);
    }

    #[test]
    fn a_composite_types_own_relation_is_not_a_dependency() {
        let db = n("hv_test");
        let mk = |refcat: &str, refowner: u32| DepRow {
            oid: 600,
            cls: "typ".into(),
            own: "pair".into(),
            kind: "c".into(),
            selfrel: 601,
            refowner,
            ..dep(refcat, 700, "e", "mood")
        };
        let rows = vec![
            mk("class", 601),
            DepRow {
                refparent: Some("pair".into()),
                refsub: 1,
                ..mk("column", 601)
            },
            mk("type", 600),
            mk("type", 700),
        ];
        let got = decide_deps(&db, 600, &rows).unwrap();
        assert_eq!(
            got.depends_on,
            vec![r(
                ObjectKind::EnumType,
                &["hv_test", "s", "mood"],
                Some(700)
            )]
        );
    }

    #[test]
    fn an_unnamed_dependency_is_a_catalog_race_never_dropped() {
        let db = n("hv_test");
        let rows = vec![DepRow {
            refname: None,
            ..dep("other", 900, "collation", "x")
        }];
        assert_eq!(decide_deps(&db, 500, &rows), Err(IntroError::CatalogRace));
        let rows = vec![DepRow {
            refschema: None,
            ..dep("class", 400, "r", "t")
        }];
        assert!(matches!(
            decide_deps(&db, 500, &rows),
            Err(IntroError::Protocol(_))
        ));
        let rows = vec![dep("mystery", 400, "r", "t")];
        assert!(matches!(
            decide_deps(&db, 500, &rows),
            Err(IntroError::Protocol(_))
        ));
    }

    #[test]
    fn targets_select_by_oid_inside_the_namespace_only() {
        let ns = ns();
        let t = key(ObjectKind::Table, "t", Some(1));
        for f in [
            FacetKind::Grants,
            FacetKind::Dependencies,
            FacetKind::Statistics,
        ] {
            assert_eq!(
                target(&ns, &t, f),
                Some((Class::Relation, Some(1))),
                "{f:?}"
            );
        }
        assert_eq!(target(&ns, &t, FacetKind::Sources), None);
        let v = key(ObjectKind::View, "v", Some(2));
        assert_eq!(
            target(&ns, &v, FacetKind::Grants),
            Some((Class::Relation, Some(2)))
        );
        assert_eq!(target(&ns, &v, FacetKind::Statistics), None);
        let sq = key(ObjectKind::Sequence, "sq", Some(3));
        assert_eq!(
            target(&ns, &sq, FacetKind::Dependencies),
            Some((Class::Relation, Some(3)))
        );
        assert_eq!(target(&ns, &sq, FacetKind::Statistics), None);
        let f = key(ObjectKind::Procedure, "p", Some(4));
        assert_eq!(
            target(&ns, &f, FacetKind::Grants),
            Some((Class::Routine, Some(4)))
        );
        let d = key(ObjectKind::Domain, "d", Some(5));
        assert_eq!(
            target(&ns, &d, FacetKind::Dependencies),
            Some((Class::Type, Some(5)))
        );
        let b = key(ObjectKind::Unknown, "citext", Some(6));
        assert_eq!(
            target(&ns, &b, FacetKind::Grants),
            Some((Class::Type, Some(6)))
        );
        let mut other_unknown = b.clone();
        other_unknown.raw_kind = Some(n("pg-newer-kind"));
        assert_eq!(target(&ns, &other_unknown, FacetKind::Grants), None);
        // The namespace's own schema: grants only, selected by the namespace oid.
        let s = ObjectKey {
            kind: ObjectKind::Schema,
            path: ns.path.iter().cloned().collect(),
            server_id: None,
            overload_sig: None,
            raw_kind: None,
        };
        assert_eq!(
            target(&ns, &s, FacetKind::Grants),
            Some((Class::Schema, None))
        );
        assert_eq!(target(&ns, &s, FacetKind::Dependencies), None);
        // No oid, another schema, a child, a kind without the facet: left out.
        assert_eq!(
            target(&ns, &key(ObjectKind::Table, "t", None), FacetKind::Grants),
            None
        );
        let mut elsewhere = t.clone();
        elsewhere.path[1] = n("other");
        assert_eq!(target(&ns, &elsewhere, FacetKind::Grants), None);
        let mut column = t.clone();
        column.kind = ObjectKind::Column;
        column.path.push(n("a"));
        assert_eq!(target(&ns, &column, FacetKind::Grants), None);
        let index = key(ObjectKind::Index, "t_pkey", Some(7));
        assert_eq!(target(&ns, &index, FacetKind::Grants), None);
        assert_eq!(target(&NsKey::server(), &t, FacetKind::Grants), None);
    }

    #[test]
    fn a_renamed_or_reused_oid_does_not_answer_for_the_key() {
        let t = table();
        assert!(same_object(&t, Class::Relation, "t", "r"));
        assert!(same_object(&t, Class::Relation, "t", "p"));
        assert!(!same_object(&t, Class::Relation, "t2", "r"), "renamed");
        assert!(
            !same_object(&t, Class::Relation, "t", "v"),
            "an oid reused by a view"
        );
        assert!(!same_object(&t, Class::Relation, "t", "S"));
        let sq = key(ObjectKind::Sequence, "sq", Some(3));
        assert!(same_object(&sq, Class::Relation, "sq", "S"));
        let f = key(ObjectKind::Routine, "f", Some(4));
        assert!(same_object(&f, Class::Routine, "f", "f"));
        assert!(!same_object(&f, Class::Routine, "f", "p"));
    }

    #[test]
    fn roles_merge_into_the_server_namespace() {
        let result = roles_result(
            vec![(10, "hv_test".into()), (16_500, "a=r/b,\"c".into())],
            1_700_000_000_000,
        );
        assert!(result.full);
        assert_eq!(result.level, Level::Names);
        assert_eq!(result.new_watermark, Watermark::None);
        assert_eq!(result.read_at_ms, 1_700_000_000_000);
        match result.existent.get(&ObjectKind::Role) {
            Some(ExistSet::Ok(ids)) => assert_eq!(ids.len(), 2),
            other => panic!("{other:?}"),
        }
        let b = base(NsKey::server(), DbmsKind::Postgres, FoldRule::Exact).unwrap();
        let policy = MergePolicy {
            trigger: Trigger::User,
            ..MergePolicy::default()
        };
        let next = match merge(Some(&b), result, &policy).expect("merge") {
            MergeOutcome::Applied { next, .. } => next,
            other => panic!("{other:?}"),
        };
        let roles: Vec<(String, Option<ServerId>)> = (0..next.len() as u32)
            .filter_map(|i| next.obj(i))
            .filter(|o| o.kind() == ObjectKind::Role)
            .map(|o| (o.name().to_string(), o.server_id()))
            .collect();
        assert_eq!(
            roles
                .iter()
                .map(|(r, _)| r.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["hv_test", "a=r/b,\"c"])
        );
        assert!(
            roles
                .iter()
                .all(|(_, id)| matches!(id, Some(ServerId::Oid(_))))
        );
    }

    fn table_key(oid: u32) -> ObjectKey {
        key(ObjectKind::Table, "t", Some(oid))
    }

    #[test]
    fn a_reread_invalidates_grants_and_dependencies_of_its_objects() {
        let ns = ns();
        let root = Ns::new(&ns).unwrap().root_key();
        let t = Obj::new(ObjectKind::Table, "t").with_server_id(ServerId::Oid(1));
        let s = Obj::new(ObjectKind::Schema, "s").with_server_id(ServerId::Oid(2200));
        let mut result = NamespaceResult::empty_full();
        result.full = false;
        result.changed = vec![
            ChangedObject {
                parent: root.clone(),
                subtree: Subtree::new(t),
            },
            ChangedObject {
                parent: root.clone(),
                subtree: Subtree::new(s),
            },
        ];
        let got = invalidated(&ns, &result);
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got.iter().any(|k| k.same_object(&table_key(1))));
        assert!(got.iter().any(|k| k.kind == ObjectKind::Schema
            && k.path.as_slice() == ns.path.as_slice()
            && k.server_id == Some(ServerId::Oid(2200))));
        assert!(!got.iter().any(|k| k.same_object(&table_key(9))));
        assert_eq!(
            INVALIDATED_FACETS,
            [FacetKind::Grants, FacetKind::Dependencies]
        );
        assert!(invalidated(&ns, &NamespaceResult::empty_full()).is_empty());
    }

    /// R-W3-9 and R-M4-7 for this file: every facet section is in the pack, is not joinable,
    /// renders in every reachable variant, passes Task 5's metadata guard, splices no bind
    /// value, casts every scalar bind, projects no ACL as text, and chunks its oid list.
    #[test]
    fn every_facet_section_renders_and_passes_the_metadata_guard() {
        let p = pack().expect("the PostgreSQL pack");
        assert_eq!(lint(&p, DialectId::Postgres), Vec::new());
        let hostile = Binds {
            ns: Some(987_654),
            w: Some(123_457),
            schema: Some(n("hvm4_x'); DROP SCHEMA public CASCADE; -- \"q\"")),
            oids: Some(vec![555_001, 555_002]),
        };
        let list = regex::Regex::new(r"IN \((\$\d+)(, \$\d+)*\)").unwrap();
        let bind = regex::Regex::new(r"\$\d+").unwrap();
        for section in name::ALL {
            let sec = p
                .section(section)
                .unwrap_or_else(|| panic!("{section} is in the pack"));
            assert!(!sec.joinable(), "{section}");
            let mut rendered = 0;
            for v in p.reachable_variants(section) {
                let c = ctx(v.version, v.mode, v.level, &hostile);
                for q in p.render(section, &c).unwrap() {
                    let sql = q.sql();
                    rendered += 1;
                    assert_eq!(
                        hacivat_session::meta::check_own_text(sql, DbmsKind::Postgres),
                        Ok(()),
                        "{section}: the metadata connection's guard refuses the text"
                    );
                    for needle in ["987654", "123457", "555001", "555002", "hvm4_", "DROP"] {
                        assert!(!sql.contains(needle), "{section}: {needle} spliced");
                    }
                    let rest = list.replace_all(sql, "IN (LIST)");
                    for m in bind.find_iter(&rest) {
                        assert!(
                            rest[m.end()..].starts_with("::pg_catalog."),
                            "{section}: {} is not cast",
                            m.as_str()
                        );
                    }
                    for acl in ["acl::", "acl AS", "array_to_string"] {
                        assert!(!sql.contains(acl), "{section}: an ACL read as text");
                    }
                }
            }
            assert!(rendered > 0, "{section} renders");
        }
        // An oid list chunks at 1,000 per statement; an empty one sends nothing.
        let many = Binds::ns(1).with_oids((1..=2_500).collect());
        let c = ctx(170_000, Mode::Full, Level::Structure, &many);
        let none = Binds::ns(1).with_oids(Vec::new());
        let c0 = ctx(170_000, Mode::Full, Level::Structure, &none);
        for section in [
            name::GRANTS_RELATION,
            name::GRANTS_ROUTINE,
            name::GRANTS_TYPE,
            name::DEPENDENCIES,
            name::STATISTICS_RELATION,
        ] {
            assert_eq!(p.render(section, &c).unwrap().len(), 3, "{section}");
            assert!(p.render(section, &c0).unwrap().is_empty(), "{section}");
        }
        assert_eq!(p.render(name::GRANTS_SCHEMA, &c).unwrap().len(), 1);
        assert_eq!(p.render(name::LIST_ROLES, &c).unwrap().len(), 1);
    }

    #[test]
    fn the_facets_postgresql_reads() {
        assert_eq!(
            FACETS,
            [
                FacetKind::Grants,
                FacetKind::Dependencies,
                FacetKind::Statistics
            ]
        );
    }
}
