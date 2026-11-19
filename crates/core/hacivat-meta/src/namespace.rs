//! Namespace arenas (HV-MODEL-01 "Per-namespace arenas"; M4 Task 3, spike origin `d86313b`).
//!
//! A [`NamespaceSnapshot`] owns every object of one schema (or database, or the server level)
//! in a `Vec<Obj>`. Index 0 is the namespace's root object. The arena is in breadth-first
//! order, so **every parent's children are contiguous** and iterating them allocates nothing;
//! a parent always precedes its children, and insertion order is kept within a parent
//! (introspectors insert columns by ordinal). The layout is computed in O(n) by a counting sort
//! and re-validated, with the kind schema, on every build. A namespace is immutable once built:
//! an incremental refresh builds a new arena ([`NamespaceSnapshot::replace_subtree`],
//! [`NamespaceSnapshot::upsert_child`]) and the catalog swaps it in (HV-MODEL-02).
//!
//! Names are looked up through one `IdentKey` index per [`KindGroup`] (namespace-scoped kinds
//! only), keyed by the namespace's stored [`FoldRule`] (R-M4-41); parent-scoped kinds (columns,
//! constraints) are found by a scan of their parent's contiguous children. Typed probes go
//! through [`NamespaceSnapshot::match_named`] and [`NamespaceSnapshot::match_child`], which
//! fold them with `sql_ident::key::match_key` and answer `Found`, `Missing` or
//! `Unknown(ServerDependent)` — never `Missing` where the server's rule is not pinned.

use std::collections::HashMap;
use std::fmt;

use bitflags::bitflags;
use hacivat_proto::DbmsKind;
use smallvec::SmallVec;

use crate::ids::{NsKey, NsKind, ObjectKey, ServerId, StateMark};
use crate::kind::{KindGroup, ObjectKind, mysql_family, pg_family};
use crate::name::{
    DialectId, FoldRule, IdentCtx, IdentKey, Name, NameClass, has_invisible_chars,
    has_non_ascii_cased, lossy_name, match_key,
};
use crate::props::Props;
use crate::view::{Lookup, MissingReason, UnknownReason};

bitflags! {
    /// Per-object flags (alg/02 HV-MODEL-01 `Obj.flags`). Bits are part of the cache format:
    /// never reassign one.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct ObjFlags: u32 {
        /// A system object (`pg_catalog`, `information_schema`, `sqlite_*`).
        const SYSTEM = 1;
        /// A session-scoped object; a namespace whose root is temp is never persisted.
        const TEMP = 1 << 1;
        /// A server-generated name (`t_pkey`, `SYS_C0012`).
        const SERVER_GENERATED_NAME = 1 << 2;
        /// Invalid or not validated (PG `NOT VALID` constraint, invalid index).
        const INVALID = 1 << 3;
        /// Inferred, not declared (Mongo fields).
        const INFERRED = 1 << 4;
        /// The name was not valid UTF-8 (or held NUL) and was stored lossily; never used to
        /// generate DDL. Set by [`Obj::from_server_bytes`]; a NUL name is flagged on build.
        const LOSSY_NAME = 1 << 5;
        /// An inherited column (PG inheritance and partitions); not repeated in DDL.
        const INHERITED = 1 << 6;
        /// Served from a pre-introspected system-catalog snapshot (HV-INTRO-16).
        const PRE_INTROSPECTED = 1 << 7;
        /// Broken (an invalid view or routine; a column reference that does not resolve).
        const BROKEN = 1 << 8;
        /// The name holds bidi controls or zero-width characters (the explorer badge). Derived:
        /// every build sets it from `name::has_invisible_chars`.
        const INVISIBLE_CHARS = 1 << 9;
        /// An implicit object (an index that backs a key, a sequence of an identity column).
        const IMPLICIT = 1 << 10;
    }
}

/// No parent: the namespace root.
pub(crate) const NO_PARENT: u32 = u32::MAX;

/// One catalog object. The layout (parent and children range) is set by the arena; fields are
/// read through accessors.
#[derive(Debug, Clone, PartialEq)]
pub struct Obj {
    kind: ObjectKind,
    name: Name,
    server_id: Option<ServerId>,
    state: StateMark,
    flags: ObjFlags,
    props: Props,
    pub(crate) parent: u32,
    pub(crate) first_child: u32,
    pub(crate) child_count: u32,
}

impl Obj {
    /// A new object with no server id, no state mark, no flags and no props.
    pub fn new(kind: ObjectKind, name: impl Into<Name>) -> Self {
        Obj {
            kind,
            name: name.into(),
            server_id: None,
            state: StateMark::None,
            flags: ObjFlags::empty(),
            props: Props::None,
            parent: NO_PARENT,
            first_child: 0,
            child_count: 0,
        }
    }

    /// A new object named by server bytes that should be UTF-8: an invalid sequence is
    /// replaced and the object flagged [`ObjFlags::LOSSY_NAME`] (alg/02 edge 12).
    pub fn from_server_bytes(kind: ObjectKind, bytes: &[u8]) -> Self {
        let (name, lossy) = lossy_name(bytes);
        let obj = Obj::new(kind, name);
        if lossy {
            obj.with_flags(ObjFlags::LOSSY_NAME)
        } else {
            obj
        }
    }

    pub fn with_server_id(mut self, id: ServerId) -> Self {
        self.server_id = Some(id);
        self
    }

    pub fn with_state(mut self, state: StateMark) -> Self {
        self.state = state;
        self
    }

    /// Adds `flags` to the object's flags (a flag set earlier, such as `LOSSY_NAME`, stays).
    pub fn with_flags(mut self, flags: ObjFlags) -> Self {
        self.flags |= flags;
        self
    }

    pub fn with_props(mut self, props: Props) -> Self {
        self.props = props;
        self
    }

    pub fn kind(&self) -> ObjectKind {
        self.kind
    }

    pub fn name(&self) -> &Name {
        &self.name
    }

    /// The parent's arena index; `None` for the namespace root.
    pub fn parent(&self) -> Option<u32> {
        (self.parent != NO_PARENT).then_some(self.parent)
    }

    pub fn server_id(&self) -> Option<ServerId> {
        self.server_id
    }

    pub fn state(&self) -> StateMark {
        self.state
    }

    pub fn flags(&self) -> ObjFlags {
        self.flags
    }

    pub fn props(&self) -> &Props {
        &self.props
    }

    pub fn child_count(&self) -> u32 {
        self.child_count
    }

    /// The raw server kind of an [`ObjectKind::Unknown`] object (`None` for a known kind).
    pub fn raw_kind(&self) -> Option<&Name> {
        if self.kind == ObjectKind::Unknown {
            self.props.raw_kind()
        } else {
            None
        }
    }

    /// Whether `self` and `other` are the same server object (the object-level form of
    /// [`ObjectKey::same_object`] for two siblings).
    fn same_identity(&self, other: &Obj) -> bool {
        if self.kind != other.kind || self.raw_kind() != other.raw_kind() {
            return false;
        }
        match (self.server_id, other.server_id) {
            (Some(a), Some(b)) => a == b,
            _ => self.name == other.name && self.props.overload_sig() == other.props.overload_sig(),
        }
    }

    /// Whether this object is the last element of `key` (kind, raw kind, overload signature).
    fn is_keyed_by(&self, key: &ObjectKey) -> bool {
        self.kind == key.kind
            && self.raw_kind() == key.raw_kind.as_ref()
            && self.props.overload_sig() == key.overload_sig.as_ref()
    }
}

/// How much of a namespace was introspected (HV-INTRO-01 levels; R-M4-21 pills).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum Level {
    /// Major object names and routine signatures.
    #[default]
    Names,
    /// Everything but source text.
    Structure,
    /// Structure plus source text.
    Sources,
}

/// The PostgreSQL cluster a watermark belongs to: a restore or a swapped server changes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PgIdentity {
    /// `pg_control_system().system_identifier`.
    pub system_identifier: u64,
    /// `pg_postmaster_start_time()`, microseconds since the Unix epoch (a restart).
    pub start_time_micros: i64,
}

/// The namespace-level change watermark incremental introspection starts from
/// (HV-INTRO-05..08).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Watermark {
    #[default]
    None,
    /// PostgreSQL: `W`, the 64-bit snapshot xmin of the pass (R-M4-8), and the cluster.
    PgXid { w: u64, identity: PgIdentity },
    /// MySQL: `UTC_TIMESTAMP(6)` read inside the pass's snapshot (R-M4-39), `@@server_uuid`,
    /// `@@version` and a hash of the login's privileges.
    MyTime {
        at_utc_micros: i64,
        server_uuid: Name,
        version: Name,
        priv_hash: u64,
    },
    /// SQLite: `schema_version`, the file identity (device, inode) and the XXH3-128 digest of
    /// `sqlite_schema`.
    Sqlite {
        schema_version: u32,
        file_identity: Option<(u64, u64)>,
        digest: u128,
    },
}

/// Namespace metadata kept with the arena.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NsMeta {
    pub level: Level,
    pub watermark: Watermark,
    /// When the namespace was introspected, milliseconds since the Unix epoch (display only;
    /// correctness relies on the watermark).
    pub loaded_at_ms: i64,
}

/// Why an arena operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    /// The root object's kind does not match the namespace kind.
    RootKind {
        expected: ObjectKind,
        found: ObjectKind,
    },
    /// The root object's name is not the namespace path's last element.
    RootName { expected: Name, found: Name },
    /// The kind schema does not allow `child` under `parent` in this DBMS.
    KindNotAllowed {
        parent: ObjectKind,
        child: ObjectKind,
    },
    /// A decoded arena is not a valid breadth-first layout.
    BadLayout(&'static str),
    /// More objects than an arena index can address.
    TooManyObjects,
    /// The subtree target is not in the namespace (or is its root).
    NoSuchObject,
    /// The namespace is not in the catalog.
    NoSuchNamespace(NsKey),
    /// A second publisher, or a publish through a closed one.
    NotTheWriter,
    /// Not written yet (an M4 Task 1a stub of a later task, such as the catalog's publish).
    NotImplemented,
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelError::RootKind { expected, found } => write!(
                f,
                "namespace root is a {}, expected a {}",
                found.id(),
                expected.id()
            ),
            ModelError::RootName { expected, found } => {
                write!(
                    f,
                    "namespace root is named {found:?}, expected {expected:?}"
                )
            }
            ModelError::KindNotAllowed { parent, child } => {
                write!(f, "a {} cannot contain a {}", parent.id(), child.id())
            }
            ModelError::BadLayout(why) => write!(f, "bad arena layout: {why}"),
            ModelError::TooManyObjects => write!(f, "too many objects in one namespace"),
            ModelError::NoSuchObject => write!(f, "no such object in the namespace"),
            ModelError::NoSuchNamespace(k) => write!(f, "no such namespace: {k:?}"),
            ModelError::NotTheWriter => write!(f, "not the catalog's single writer"),
            ModelError::NotImplemented => write!(f, "not implemented"),
        }
    }
}

impl std::error::Error for ModelError {}

/// A node handle inside a [`Subtree`] being built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(u32);

/// A tree of new objects: a whole namespace (root = the schema), or one table with its
/// columns, indexes and constraints for a fragment refresh. Children keep insertion order
/// (introspectors insert columns by ordinal). A plain builder: it records nodes and parents;
/// [`NamespaceSnapshot::build`] lays them out and validates them (a [`NodeId`] of another
/// subtree is refused there as a bad layout).
#[derive(Debug, Clone)]
pub struct Subtree {
    objs: Vec<Obj>,
    parents: Vec<u32>,
}

impl Subtree {
    pub fn new(root: Obj) -> Self {
        Subtree {
            objs: vec![root],
            parents: vec![NO_PARENT],
        }
    }

    /// A subtree with room for `capacity` objects.
    pub fn with_capacity(root: Obj, capacity: usize) -> Self {
        let mut objs = Vec::with_capacity(capacity.max(1));
        let mut parents = Vec::with_capacity(capacity.max(1));
        objs.push(root);
        parents.push(NO_PARENT);
        Subtree { objs, parents }
    }

    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    pub fn root_obj(&self) -> &Obj {
        &self.objs[0]
    }

    /// Adds `obj` under `parent`.
    pub fn add(&mut self, parent: NodeId, obj: Obj) -> NodeId {
        let id = u32::try_from(self.objs.len()).unwrap_or(NO_PARENT);
        self.objs.push(obj);
        self.parents.push(parent.0);
        NodeId(id)
    }

    /// Adds every node of `other` under `parent`, keeping `other`'s shape; returns the node of
    /// `other`'s root.
    pub fn graft_under(&mut self, parent: NodeId, other: Subtree) -> NodeId {
        let root = u32::try_from(self.objs.len()).unwrap_or(NO_PARENT);
        self.graft(parent.0, other);
        NodeId(root)
    }

    fn graft(&mut self, parent: u32, other: Subtree) {
        let base = u32::try_from(self.objs.len()).unwrap_or(NO_PARENT);
        for (obj, p) in other.objs.into_iter().zip(other.parents) {
            self.objs.push(obj);
            self.parents.push(if p == NO_PARENT {
                parent
            } else {
                base.saturating_add(p)
            });
        }
    }

    pub fn len(&self) -> usize {
        self.objs.len()
    }

    /// Never true: a subtree always has its root.
    pub fn is_empty(&self) -> bool {
        self.objs.is_empty()
    }
}

/// One immutable namespace arena (alg/02 HV-MODEL-01 `NamespaceSnapshot`).
#[derive(Debug, Clone)]
pub struct NamespaceSnapshot {
    key: NsKey,
    dbms: DbmsKind,
    fold: FoldRule,
    meta: NsMeta,
    generation: u64,
    objs: Vec<Obj>,
    /// One map per [`KindGroup`] (by [`KindGroup::index`]); `IdentKey: Borrow<str>`, so a
    /// probe that needs no folding is looked up without allocating.
    names: [HashMap<IdentKey, SmallVec<[u32; 2]>>; KindGroup::COUNT],
    /// Per group: a stored name holds a non-ASCII cased letter under a rule the server may
    /// extend beyond ASCII (MySQL/MariaDB caseless names, R-M4-41), so a miss is not a fact.
    uncertain: [bool; KindGroup::COUNT],
    by_server_id: HashMap<(ObjectKind, ServerId), u32>,
}

impl NamespaceSnapshot {
    /// Builds a namespace from `tree` (root = the namespace's root object), checking the root
    /// against `key`, the layout, and every edge against the kind schema of `dbms`. `fold` is
    /// the stored rule of the namespace's table-class names (`FoldRule::for_names(d, ctx,
    /// NameClass::Table)`: PostgreSQL exact, SQLite ASCII-caseless, MySQL/MariaDB by
    /// `lower_case_table_names`); MySQL/MariaDB routine, event, index and column names are
    /// always ASCII-caseless.
    pub fn build(
        key: NsKey,
        dbms: DbmsKind,
        fold: FoldRule,
        meta: NsMeta,
        tree: Subtree,
    ) -> Result<Self, ModelError> {
        let objs = layout(tree)?;
        Self::from_arena(key, dbms, fold, meta, objs)
    }

    /// Wraps an arena already in breadth-first order (`parent` set on every object; the
    /// decoder's output) after checking its root, its layout (a parent before its children,
    /// contiguous children) and its kind schema; computes the children ranges, the derived
    /// flags and the indexes. A corrupt arena is refused, never trusted.
    pub(crate) fn from_arena(
        key: NsKey,
        dbms: DbmsKind,
        fold: FoldRule,
        meta: NsMeta,
        mut objs: Vec<Obj>,
    ) -> Result<Self, ModelError> {
        let n = u32::try_from(objs.len()).map_err(|_| ModelError::TooManyObjects)?;
        if n == NO_PARENT {
            return Err(ModelError::TooManyObjects);
        }
        let Some(root) = objs.first() else {
            return Err(ModelError::BadLayout("empty arena"));
        };
        let root_kind = key.kind.root_kind();
        if root.kind != root_kind {
            return Err(ModelError::RootKind {
                expected: root_kind,
                found: root.kind,
            });
        }
        if &*root.name != key.root_name() {
            return Err(ModelError::RootName {
                expected: key.root_name().into(),
                found: root.name.clone(),
            });
        }
        if root.parent != NO_PARENT {
            return Err(ModelError::BadLayout("root has a parent"));
        }
        for o in &mut objs {
            o.first_child = 0;
            o.child_count = 0;
        }
        for i in 1..n {
            let p = objs[i as usize].parent;
            if p >= i {
                return Err(ModelError::BadLayout("parent after child"));
            }
            let (pk, ck) = (objs[p as usize].kind, objs[i as usize].kind);
            if !pk.allows_child(ck, dbms) {
                return Err(ModelError::KindNotAllowed {
                    parent: pk,
                    child: ck,
                });
            }
            let parent = &mut objs[p as usize];
            if parent.child_count == 0 {
                parent.first_child = i;
            } else if parent.first_child + parent.child_count != i {
                return Err(ModelError::BadLayout("children not contiguous"));
            }
            parent.child_count += 1;
        }
        let mut ns = NamespaceSnapshot {
            key,
            dbms,
            fold,
            meta,
            generation: 0,
            objs,
            names: Default::default(),
            uncertain: [false; KindGroup::COUNT],
            by_server_id: HashMap::new(),
        };
        ns.index();
        Ok(ns)
    }

    /// Derived flags, the name indexes and the server-id index: one pass.
    fn index(&mut self) {
        let mut names: [HashMap<IdentKey, SmallVec<[u32; 2]>>; KindGroup::COUNT] =
            Default::default();
        let mut uncertain = [false; KindGroup::COUNT];
        let mut by_sid = HashMap::new();
        let rules = KindGroup::ALL.map(|g| self.class_rule(group_class(g)));
        let unpinned = KindGroup::ALL.map(|g| self.unpinned(group_class(g)));
        for (i, o) in self.objs.iter_mut().enumerate() {
            o.flags
                .set(ObjFlags::INVISIBLE_CHARS, has_invisible_chars(&o.name));
            if o.name.contains('\0') {
                o.flags.insert(ObjFlags::LOSSY_NAME);
            }
            if i == 0 {
                continue;
            }
            let i = i as u32;
            if let Some(g) = o.kind.group(self.dbms) {
                let gi = g.index();
                names[gi].entry(rules[gi].key(&o.name)).or_default().push(i);
                if unpinned[gi] && has_non_ascii_cased(&o.name) {
                    uncertain[gi] = true;
                }
            }
            if let Some(sid) = o.server_id {
                by_sid.insert((o.kind, sid), i);
            }
        }
        self.names = names;
        self.uncertain = uncertain;
        self.by_server_id = by_sid;
    }

    /// The stored rule of names of `class` in this namespace: the namespace's rule, except that
    /// MySQL/MariaDB names outside the `lower_case_table_names` classes are ASCII-caseless.
    fn class_rule(&self, class: NameClass) -> FoldRule {
        let lctn_class = matches!(
            class,
            NameClass::Database | NameClass::Schema | NameClass::Table
        );
        if mysql_family(self.dbms) && !lctn_class {
            FoldRule::AsciiCaseless
        } else {
            self.fold
        }
    }

    /// Whether names of `class` compare under a rule the server may extend beyond ASCII
    /// (R-M4-41: MySQL/MariaDB caseless names, until Task 17 pins it).
    fn unpinned(&self, class: NameClass) -> bool {
        mysql_family(self.dbms) && self.class_rule(class) == FoldRule::AsciiCaseless
    }

    /// Whether a probe context can be matched against this namespace's stored keys: the same
    /// engine family and the same stored rule for `class`.
    fn agrees_with(&self, ctx: &IdentCtx, class: NameClass) -> bool {
        let family = match ctx.dialect {
            DialectId::Postgres | DialectId::PgWire(_) => pg_family(self.dbms),
            DialectId::MySql | DialectId::MariaDb => mysql_family(self.dbms),
            DialectId::Sqlite => self.dbms == DbmsKind::Sqlite,
            _ => false,
        };
        family && FoldRule::for_names(ctx.dialect, ctx, class) == Some(self.class_rule(class))
    }

    pub fn key(&self) -> &NsKey {
        &self.key
    }

    pub fn dbms(&self) -> DbmsKind {
        self.dbms
    }

    /// The stored fold policy of this namespace's table-class names.
    pub fn fold(&self) -> FoldRule {
        self.fold
    }

    pub fn meta(&self) -> &NsMeta {
        &self.meta
    }

    /// The catalog generation that published this arena (0 until published).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// This arena stamped with `generation` (the catalog's publisher does this when it swaps
    /// the arena in, HV-MODEL-02).
    pub fn with_generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }

    pub fn len(&self) -> usize {
        self.objs.len()
    }

    /// Never true: a namespace always has its root.
    pub fn is_empty(&self) -> bool {
        self.objs.is_empty()
    }

    /// Every object, in arena order (index 0 is the root).
    pub fn objs(&self) -> &[Obj] {
        &self.objs
    }

    pub fn obj(&self, idx: u32) -> Option<&Obj> {
        self.objs.get(idx as usize)
    }

    pub fn root(&self) -> &Obj {
        &self.objs[0]
    }

    /// Whether this is a session-scoped namespace (its root is flagged `TEMP`): never
    /// persisted (alg/02 edge 8).
    pub fn is_temp(&self) -> bool {
        self.root().flags.contains(ObjFlags::TEMP)
    }

    /// The number of columns in the namespace.
    pub fn column_count(&self) -> usize {
        self.objs
            .iter()
            .filter(|o| o.kind == ObjectKind::Column)
            .count()
    }

    /// The children of `idx`, in arena order, with their indexes. Allocation-free; none for an
    /// index outside the arena.
    pub fn children(&self, idx: u32) -> impl Iterator<Item = (u32, &Obj)> + '_ {
        let (first, count) = self
            .objs
            .get(idx as usize)
            .map_or((0, 0), |o| (o.first_child as usize, o.child_count as usize));
        self.objs[first..first + count]
            .iter()
            .enumerate()
            .map(move |(k, o)| ((first + k) as u32, o))
    }

    /// The children of `idx` of `kind`.
    pub fn children_of_kind(
        &self,
        idx: u32,
        kind: ObjectKind,
    ) -> impl Iterator<Item = (u32, &Obj)> + '_ {
        self.children(idx).filter(move |(_, o)| o.kind == kind)
    }

    /// The namespace-scoped objects of `group` whose stored key is `name` folded by the group's
    /// rule (`name` is a catalog-form name or an `IdentKey`'s text; for PostgreSQL the caller
    /// folds unquoted input first, as [`NamespaceSnapshot::match_named`] does). Overloads
    /// return several. Allocates only for an ASCII-caseless probe holding an ASCII capital.
    pub fn lookup(&self, group: KindGroup, name: &str) -> &[u32] {
        let map = &self.names[group.index()];
        let hit = match self.class_rule(group_class(group)) {
            FoldRule::AsciiCaseless if name.bytes().any(|b| b.is_ascii_uppercase()) => {
                map.get(name.to_ascii_lowercase().as_str())
            }
            _ => map.get(name),
        };
        hit.map_or(&[][..], |v| v.as_slice())
    }

    /// The namespace-scoped objects of `kind` named `name` (as [`NamespaceSnapshot::lookup`]).
    pub fn lookup_kind(&self, kind: ObjectKind, name: &str) -> impl Iterator<Item = u32> + '_ {
        let hits = kind
            .group(self.dbms)
            .map_or(&[][..], |g| self.lookup(g, name));
        hits.iter()
            .copied()
            .filter(move |&i| self.objs[i as usize].kind == kind)
    }

    /// The first child of `parent` of `kind` whose name matches `name` under the kind's stored
    /// rule (a scan of the contiguous children; columns of a table, …).
    pub fn child_by_name(&self, parent: u32, kind: ObjectKind, name: &str) -> Option<u32> {
        let rule = self.class_rule(kind.name_class(self.dbms));
        self.children_of_kind(parent, kind)
            .find(|(_, o)| rule.matches(&o.name, name))
            .map(|(i, _)| i)
    }

    /// The objects of `group` the typed `probe` (with its `quoted` flag) names under `ctx`
    /// (HV-INTRO-14, R-M4-41): `Found` with every hit (overloads); `Missing` only when the
    /// probe keyed, nothing matched and no stored name of the group is uncertain;
    /// `Unknown(ServerDependent)` when the server's rule is not pinned for the probe or the
    /// group, or `ctx` disagrees with this namespace's stored rule. A fact about this arena
    /// only: the model view (Task 9) adds load levels and scope.
    pub fn match_named(
        &self,
        group: KindGroup,
        probe: &str,
        quoted: bool,
        ctx: &IdentCtx,
    ) -> Lookup<SmallVec<[u32; 2]>> {
        let class = group_class(group);
        if !self.agrees_with(ctx, class) {
            return Lookup::Unknown(UnknownReason::ServerDependent);
        }
        match match_key(ctx.dialect, ctx, probe, quoted, class) {
            Ok(key) => {
                let hits = self.lookup(group, key.as_str());
                if !hits.is_empty() {
                    Lookup::Found(hits.into())
                } else if self.uncertain[group.index()] {
                    Lookup::Unknown(UnknownReason::ServerDependent)
                } else {
                    Lookup::Missing(MissingReason::NotFound)
                }
            }
            Err(_) => {
                // Only where the server compares case-insensitively is a byte-exact stored name
                // certainly the probe's object (PostgreSQL may fold the probe itself).
                let exact: SmallVec<[u32; 2]> = if mysql_family(self.dbms) {
                    self.lookup(group, probe)
                        .iter()
                        .copied()
                        .filter(|&i| *self.objs[i as usize].name == *probe)
                        .collect()
                } else {
                    SmallVec::new()
                };
                if exact.is_empty() {
                    Lookup::Unknown(UnknownReason::ServerDependent)
                } else {
                    Lookup::Found(exact)
                }
            }
        }
    }

    /// The child of `parent` of `kind` the typed `probe` names under `ctx` (columns of a table,
    /// …), with [`NamespaceSnapshot::match_named`]'s answers; `Unknown(NotLoaded)` for a
    /// `parent` outside the arena.
    pub fn match_child(
        &self,
        parent: u32,
        kind: ObjectKind,
        probe: &str,
        quoted: bool,
        ctx: &IdentCtx,
    ) -> Lookup<u32> {
        if parent as usize >= self.objs.len() {
            return Lookup::Unknown(UnknownReason::NotLoaded);
        }
        let class = kind.name_class(self.dbms);
        if !self.agrees_with(ctx, class) {
            return Lookup::Unknown(UnknownReason::ServerDependent);
        }
        let rule = self.class_rule(class);
        match match_key(ctx.dialect, ctx, probe, quoted, class) {
            Ok(key) => {
                let unpinned = self.unpinned(class);
                let mut uncertain = false;
                for (i, o) in self.children_of_kind(parent, kind) {
                    if rule.matches(&o.name, key.as_str()) {
                        return Lookup::Found(i);
                    }
                    uncertain |= unpinned && has_non_ascii_cased(&o.name);
                }
                if uncertain {
                    Lookup::Unknown(UnknownReason::ServerDependent)
                } else {
                    Lookup::Missing(MissingReason::NotFound)
                }
            }
            Err(_) => {
                let exact = mysql_family(self.dbms)
                    .then(|| {
                        self.children_of_kind(parent, kind)
                            .find(|(_, o)| *o.name == *probe)
                    })
                    .flatten();
                match exact {
                    Some((i, _)) => Lookup::Found(i),
                    None => Lookup::Unknown(UnknownReason::ServerDependent),
                }
            }
        }
    }

    /// The stable key of the object at `idx`: the namespace path, then the names down to it.
    pub fn object_key(&self, idx: u32) -> Option<ObjectKey> {
        let obj = self.objs.get(idx as usize)?;
        let mut names: SmallVec<[Name; 4]> = SmallVec::new();
        let mut cur = idx;
        while cur != 0 {
            let o = &self.objs[cur as usize];
            names.push(o.name.clone());
            cur = o.parent;
        }
        let mut path: SmallVec<[Name; 4]> = self.key.path.iter().cloned().collect();
        path.extend(names.into_iter().rev());
        Some(ObjectKey {
            kind: obj.kind,
            path,
            server_id: obj.server_id,
            overload_sig: obj.props.overload_sig().cloned(),
            raw_kind: obj.raw_kind().cloned(),
        })
    }

    /// The arena index of the object `key` denotes ([`ObjectKey::same_object`]): by server id
    /// when both sides have one (within this namespace's database: PostgreSQL OIDs are per
    /// database), else by exact path (and overload signature). A path hit whose server id
    /// differs from the key's (an OID reused for another object) is `None`.
    pub fn find(&self, key: &ObjectKey) -> Option<u32> {
        if let Some(sid) = key.server_id
            && key.path.starts_with(self.id_scope())
            && let Some(&i) = self.by_server_id.get(&(key.kind, sid))
            && self.objs[i as usize].is_keyed_by(key)
        {
            return Some(i);
        }
        let rest = key.path.strip_prefix(self.key.path.as_slice())?;
        let i = match rest.split_first() {
            None => (key.kind == self.objs[0].kind).then_some(0)?,
            Some((head, tail)) => self.walk_top(head, tail, key)?,
        };
        match (self.objs[i as usize].server_id, key.server_id) {
            (Some(a), Some(b)) if a != b => None,
            _ => Some(i),
        }
    }

    /// The path prefix within which server ids are unique: the database part of a schema
    /// namespace's path (PostgreSQL `[db]`), the whole path otherwise.
    fn id_scope(&self) -> &[Name] {
        let path = self.key.path.as_slice();
        match self.key.kind {
            NsKind::Schema => &path[..path.len().saturating_sub(1)],
            NsKind::Database | NsKind::Server => path,
        }
    }

    /// The first path step below the root through the name indexes (so a key in a
    /// 10,000-table schema is not a scan), then a scan for kinds outside every index.
    fn walk_top(&self, head: &Name, tail: &[Name], key: &ObjectKey) -> Option<u32> {
        for group in KindGroup::ALL {
            for &i in self.lookup(group, head) {
                let o = &self.objs[i as usize];
                if o.parent != 0 || o.name != *head {
                    continue;
                }
                if let Some(found) = self.step(i, o, tail, key) {
                    return Some(found);
                }
            }
        }
        self.children(0)
            .filter(|(_, o)| o.kind.group(self.dbms).is_none() && o.name == *head)
            .find_map(|(i, o)| self.step(i, o, tail, key))
    }

    fn step(&self, i: u32, o: &Obj, tail: &[Name], key: &ObjectKey) -> Option<u32> {
        if tail.is_empty() {
            return o.is_keyed_by(key).then_some(i);
        }
        if o.child_count == 0 {
            return None;
        }
        self.walk(i, tail, key)
    }

    fn walk(&self, at: u32, rest: &[Name], key: &ObjectKey) -> Option<u32> {
        let (head, tail) = rest.split_first()?;
        self.children(at)
            .filter(|(_, o)| o.name == *head)
            .find_map(|(i, o)| self.step(i, o, tail, key))
    }

    /// Every object of one of `kinds`, in arena order.
    pub fn iter_named<'a>(
        &'a self,
        kinds: &'a [ObjectKind],
    ) -> impl Iterator<Item = (u32, &'a Obj)> + 'a {
        self.objs
            .iter()
            .enumerate()
            .filter(move |(_, o)| kinds.contains(&o.kind))
            .map(|(i, o)| (i as u32, o))
    }

    /// A new arena with the subtree at `target` replaced by `with` (in the same position among
    /// its siblings), or removed when `with` is `None`. Cost: one pass over this namespace;
    /// objects are cloned by reference count (props are `Arc`s). The result is validated like
    /// any build.
    pub fn replace_subtree(&self, target: u32, with: Option<Subtree>) -> Result<Self, ModelError> {
        if target == 0 || target as usize >= self.objs.len() {
            return Err(ModelError::NoSuchObject);
        }
        self.rebuild(Some(target), with, None)
    }

    /// A new arena where `subtree` replaces the child of `parent` that is the same object as
    /// its root (server id, else kind, name and overload signature), or is appended as
    /// `parent`'s last child when there is none.
    pub fn upsert_child(&self, parent: u32, subtree: Subtree) -> Result<Self, ModelError> {
        if parent as usize >= self.objs.len() {
            return Err(ModelError::NoSuchObject);
        }
        let existing = self
            .children(parent)
            .find(|(_, o)| o.same_identity(subtree.root_obj()))
            .map(|(i, _)| i);
        match existing {
            Some(i) => self.rebuild(Some(i), Some(subtree), None),
            None => self.rebuild(None, None, Some((parent, subtree))),
        }
    }

    fn rebuild(
        &self,
        target: Option<u32>,
        mut with: Option<Subtree>,
        mut append: Option<(u32, Subtree)>,
    ) -> Result<Self, ModelError> {
        let n = self.objs.len();
        let extra =
            with.as_ref().map_or(0, Subtree::len) + append.as_ref().map_or(0, |a| a.1.len());
        let mut tree = Subtree {
            objs: Vec::with_capacity(n + extra),
            parents: Vec::with_capacity(n + extra),
        };
        // Old index → new node; NO_PARENT marks a dropped object (the target's subtree). The
        // arena is breadth-first, so a parent is always mapped before its children.
        let mut map = vec![NO_PARENT; n];
        for (i, o) in self.objs.iter().enumerate() {
            let new_parent = if i == 0 {
                NO_PARENT
            } else {
                match map[o.parent as usize] {
                    NO_PARENT => continue,
                    p => p,
                }
            };
            if Some(i as u32) == target {
                if let Some(sub) = with.take() {
                    tree.graft(new_parent, sub);
                }
                continue;
            }
            map[i] = u32::try_from(tree.objs.len()).map_err(|_| ModelError::TooManyObjects)?;
            let mut copy = o.clone();
            copy.parent = NO_PARENT;
            tree.objs.push(copy);
            tree.parents.push(new_parent);
        }
        if let Some((parent, sub)) = append.take() {
            tree.graft(map[parent as usize], sub);
        }
        let objs = layout(tree)?;
        Self::from_arena(
            self.key.clone(),
            self.dbms,
            self.fold,
            self.meta.clone(),
            objs,
        )
    }
}

/// The name class a group's names are matched under (MySQL triggers follow
/// `lower_case_table_names` like tables; `ObjectKind::name_class`).
fn group_class(group: KindGroup) -> NameClass {
    match group {
        KindGroup::Relations | KindGroup::Triggers => NameClass::Table,
        KindGroup::Routines | KindGroup::Procedures => NameClass::Routine,
        KindGroup::Types | KindGroup::Other => NameClass::Other,
    }
}

/// Lays `tree` out breadth-first (children of every parent contiguous, insertion order kept)
/// and sets each object's `parent`: a counting sort of children by parent, then one BFS. O(n).
/// A parent outside the tree, a cycle or an unreachable node is a bad layout.
fn layout(tree: Subtree) -> Result<Vec<Obj>, ModelError> {
    let n = tree.objs.len();
    if n == 0 {
        return Err(ModelError::BadLayout("empty tree"));
    }
    if u32::try_from(n).map_or(true, |n| n == NO_PARENT) {
        return Err(ModelError::TooManyObjects);
    }
    for (c, &p) in tree.parents.iter().enumerate().skip(1) {
        if p as usize >= n || p as usize == c {
            return Err(ModelError::BadLayout("parent outside the tree"));
        }
    }
    // Children lists by counting sort (stable: insertion order within a parent).
    let mut start = vec![0u32; n + 1];
    for &p in &tree.parents[1..] {
        start[p as usize + 1] += 1;
    }
    for i in 0..n {
        start[i + 1] += start[i];
    }
    let mut fill = start.clone();
    let mut kids = vec![0u32; n - 1];
    for (c, &p) in tree.parents.iter().enumerate().skip(1) {
        kids[fill[p as usize] as usize] = c as u32;
        fill[p as usize] += 1;
    }
    let mut order: Vec<u32> = Vec::with_capacity(n);
    let mut new_idx = vec![NO_PARENT; n];
    order.push(0);
    new_idx[0] = 0;
    let mut head = 0;
    while head < order.len() {
        let u = order[head] as usize;
        for &c in &kids[start[u] as usize..start[u + 1] as usize] {
            new_idx[c as usize] = order.len() as u32;
            order.push(c);
        }
        head += 1;
    }
    if order.len() != n {
        return Err(ModelError::BadLayout("unreachable node"));
    }
    let mut slots: Vec<Option<Obj>> = tree.objs.into_iter().map(Some).collect();
    let mut out = Vec::with_capacity(n);
    for &old in &order {
        let mut o = slots[old as usize]
            .take()
            .ok_or(ModelError::BadLayout("node visited twice"))?;
        let p = tree.parents[old as usize];
        o.parent = if p == NO_PARENT {
            NO_PARENT
        } else {
            new_idx[p as usize]
        };
        out.push(o);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arena(parents: &[u32], kinds: &[ObjectKind]) -> Vec<Obj> {
        parents
            .iter()
            .zip(kinds)
            .enumerate()
            .map(|(i, (&p, &k))| {
                let mut o = Obj::new(k, if i == 0 { "public" } else { "x" });
                o.parent = p;
                o
            })
            .collect()
    }

    fn decode(objs: Vec<Obj>) -> Result<NamespaceSnapshot, ModelError> {
        NamespaceSnapshot::from_arena(
            NsKey::schema(["db", "public"]),
            DbmsKind::Postgres,
            FoldRule::Exact,
            NsMeta::default(),
            objs,
        )
    }

    /// The decoder-side checks: a corrupt arena is refused, never trusted.
    #[test]
    fn arena_rejects_child_before_parent_and_non_contiguous_children() {
        use ObjectKind::*;
        // Valid: schema, two tables, a column of each (children contiguous).
        assert!(
            decode(arena(
                &[NO_PARENT, 0, 0, 1, 2],
                &[Schema, Table, Table, Column, Column]
            ))
            .is_ok()
        );
        // A parent after its child.
        assert_eq!(
            decode(arena(&[NO_PARENT, 2, 0], &[Schema, Column, Table])).unwrap_err(),
            ModelError::BadLayout("parent after child")
        );
        // Table 1's children are split by table 2's column.
        assert_eq!(
            decode(arena(
                &[NO_PARENT, 0, 0, 1, 2, 1],
                &[Schema, Table, Table, Column, Column, Column]
            ))
            .unwrap_err(),
            ModelError::BadLayout("children not contiguous")
        );
        // A root with a parent; a self-parent.
        assert_eq!(
            decode(arena(&[0], &[Schema])).unwrap_err(),
            ModelError::BadLayout("root has a parent")
        );
        assert_eq!(
            decode(arena(&[NO_PARENT, 1], &[Schema, Table])).unwrap_err(),
            ModelError::BadLayout("parent after child")
        );
        assert_eq!(
            decode(Vec::new()).unwrap_err(),
            ModelError::BadLayout("empty arena")
        );
        // The kind schema is checked on decode too.
        assert_eq!(
            decode(arena(&[NO_PARENT, 0], &[Schema, Column])).unwrap_err(),
            ModelError::KindNotAllowed {
                parent: Schema,
                child: Column
            }
        );
    }

    /// A cycle between two nodes outside the root's reach is a bad layout, not a hang.
    #[test]
    fn layout_refuses_a_cycle() {
        let mut t = Subtree::new(Obj::new(ObjectKind::Schema, "public"));
        t.add(NodeId(2), Obj::new(ObjectKind::Table, "a"));
        t.add(NodeId(1), Obj::new(ObjectKind::Table, "b"));
        assert_eq!(
            layout(t).unwrap_err(),
            ModelError::BadLayout("unreachable node")
        );
        let mut t = Subtree::new(Obj::new(ObjectKind::Schema, "public"));
        t.add(NodeId(1), Obj::new(ObjectKind::Table, "self"));
        assert_eq!(
            layout(t).unwrap_err(),
            ModelError::BadLayout("parent outside the tree")
        );
    }
}
