//! Namespace arenas (HV-MODEL-01 "Per-namespace arenas"). M4 Task 1a seam (spike origin
//! `d86313b`, declarations only): every arena body fails closed until Task 3 —
//! [`NamespaceSnapshot::build`] refuses, so no arena exists and every lookup above answers
//! `Unknown`.
//!
//! A [`NamespaceSnapshot`] owns every object of one schema (or database, or the server level)
//! in a `Vec<Obj>`: index 0 is the namespace root, breadth-first, a parent before its
//! children, children contiguous, insertion order kept within a parent.

use std::fmt;

use bitflags::bitflags;
use hacivat_proto::DbmsKind;

use crate::ids::{NsKey, ObjectKey, ServerId, StateMark};
use crate::kind::{KindGroup, ObjectKind};
use crate::name::{FoldRule, Name};
use crate::props::Props;

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
        /// The name was not valid UTF-8 and was stored lossily; never used to generate DDL.
        const LOSSY_NAME = 1 << 5;
        /// An inherited column (PG inheritance and partitions); not repeated in DDL.
        const INHERITED = 1 << 6;
        /// Served from a pre-introspected system-catalog snapshot (HV-INTRO-16).
        const PRE_INTROSPECTED = 1 << 7;
        /// Broken (an invalid view or routine; a column reference that does not resolve).
        const BROKEN = 1 << 8;
        /// The name holds bidi controls or zero-width characters (the explorer badge).
        const INVISIBLE_CHARS = 1 << 9;
        /// An implicit object (an index that backs a key, a sequence of an identity column).
        const IMPLICIT = 1 << 10;
    }
}

/// One catalog object. The layout (parent, children range) is set by the arena; fields are
/// read through accessors.
#[derive(Debug, Clone, PartialEq)]
pub struct Obj {
    kind: ObjectKind,
    name: Name,
    server_id: Option<ServerId>,
    state: StateMark,
    flags: ObjFlags,
    props: Props,
    parent: Option<u32>,
    child_count: u32,
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
            parent: None,
            child_count: 0,
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

    pub fn with_flags(mut self, flags: ObjFlags) -> Self {
        self.flags = flags;
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
        self.parent
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
    /// Not written yet (M4 Task 1a stub).
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
/// [`NamespaceSnapshot::build`] lays them out and validates them.
#[derive(Debug, Clone)]
pub struct Subtree {
    nodes: Vec<(Option<u32>, Obj)>,
}

impl Subtree {
    pub fn new(root: Obj) -> Self {
        Subtree {
            nodes: vec![(None, root)],
        }
    }

    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    /// Adds `obj` under `parent`.
    pub fn add(&mut self, parent: NodeId, obj: Obj) -> NodeId {
        let id = u32::try_from(self.nodes.len()).unwrap_or(u32::MAX);
        self.nodes.push((Some(parent.0), obj));
        NodeId(id)
    }

    /// Adds every node of `other` under `parent`, keeping `other`'s shape; returns the node of
    /// `other`'s root.
    pub fn graft_under(&mut self, parent: NodeId, other: Subtree) -> NodeId {
        let base = u32::try_from(self.nodes.len()).unwrap_or(u32::MAX);
        for (p, obj) in other.nodes {
            let p = match p {
                None => parent.0,
                Some(p) => base.saturating_add(p),
            };
            self.nodes.push((Some(p), obj));
        }
        NodeId(base)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
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
}

impl NamespaceSnapshot {
    /// Builds a namespace from `tree` (root = the namespace's root object), checking the root
    /// against `key`, the layout, and every edge against the kind schema of `dbms`.
    ///
    /// Task 1a stub: `Err(ModelError::NotImplemented)`.
    pub fn build(
        key: NsKey,
        dbms: DbmsKind,
        fold: FoldRule,
        meta: NsMeta,
        tree: Subtree,
    ) -> Result<Self, ModelError> {
        let _ = (key, dbms, fold, meta, tree);
        Err(ModelError::NotImplemented)
    }

    pub fn key(&self) -> &NsKey {
        &self.key
    }

    pub fn dbms(&self) -> DbmsKind {
        self.dbms
    }

    /// The stored fold policy of this namespace's names.
    pub fn fold(&self) -> FoldRule {
        self.fold
    }

    pub fn meta(&self) -> &NsMeta {
        &self.meta
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn len(&self) -> usize {
        self.objs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objs.is_empty()
    }

    pub fn obj(&self, idx: u32) -> Option<&Obj> {
        self.objs.get(idx as usize)
    }

    /// The children of `idx`, in arena order. Task 1a stub: none.
    pub fn children(&self, idx: u32) -> impl Iterator<Item = (u32, &Obj)> + '_ {
        let _ = idx;
        std::iter::empty()
    }

    /// The children of `idx` of `kind`. Task 1a stub: none.
    pub fn children_of_kind(
        &self,
        idx: u32,
        kind: ObjectKind,
    ) -> impl Iterator<Item = (u32, &Obj)> + '_ {
        let _ = (idx, kind);
        std::iter::empty()
    }

    /// The namespace-scoped objects of `group` named `name` (folded by the namespace's rule).
    /// Task 1a stub: none.
    pub fn lookup(&self, group: KindGroup, name: &str) -> &[u32] {
        let _ = (group, name);
        &[]
    }

    /// The objects of `kind` named `name`. Task 1a stub: none.
    pub fn lookup_kind(&self, kind: ObjectKind, name: &str) -> impl Iterator<Item = u32> + '_ {
        let _ = (kind, name);
        std::iter::empty()
    }

    /// The child of `parent` of `kind` named `name` (parent-scoped kinds). Task 1a stub: none.
    pub fn child_by_name(&self, parent: u32, kind: ObjectKind, name: &str) -> Option<u32> {
        let _ = (parent, kind, name);
        None
    }

    /// The stable key of the object at `idx`. Task 1a stub: none.
    pub fn object_key(&self, idx: u32) -> Option<ObjectKey> {
        let _ = idx;
        None
    }

    /// The object `key` denotes (server-id index first, then exact path). Task 1a stub: none.
    pub fn find(&self, key: &ObjectKey) -> Option<u32> {
        let _ = key;
        None
    }

    /// Every object of `kinds`, in arena order. Task 1a stub: none.
    pub fn iter_named<'a>(
        &'a self,
        kinds: &'a [ObjectKind],
    ) -> impl Iterator<Item = (u32, &'a Obj)> + 'a {
        let _ = kinds;
        std::iter::empty()
    }

    /// A new arena with the subtree at `target` replaced by `with` (or removed).
    /// Task 1a stub: `Err(ModelError::NotImplemented)`.
    pub fn replace_subtree(&self, target: u32, with: Option<Subtree>) -> Result<Self, ModelError> {
        let _ = (target, with);
        Err(ModelError::NotImplemented)
    }

    /// A new arena with `subtree` inserted under `parent`, replacing the child that is the
    /// same object as its root. Task 1a stub: `Err(ModelError::NotImplemented)`.
    pub fn upsert_child(&self, parent: u32, subtree: Subtree) -> Result<Self, ModelError> {
        let _ = (parent, subtree);
        Err(ModelError::NotImplemented)
    }
}
