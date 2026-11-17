//! Namespace arenas (HV-MODEL-01 "Per-namespace arenas").
//!
//! A [`NamespaceSnapshot`] owns every object of one schema (or database, or the server level)
//! in a `Vec<Obj>`. Index 0 is the namespace's root object (the schema itself). The arena is
//! in breadth-first order, so **every parent's children are contiguous** and iterating them
//! allocates nothing; a parent always precedes its children. A namespace is immutable once
//! built: an incremental refresh builds a new arena ([`NamespaceSnapshot::replace_subtree`],
//! [`NamespaceSnapshot::upsert_child`]) and the catalog swaps it in (HV-MODEL-02).

use std::collections::HashMap;
use std::fmt;

use bitflags::bitflags;
use hacivat_proto::DbmsKind;
use smallvec::SmallVec;

use crate::ids::{NsKey, ObjectKey, ServerId, StateMark};
use crate::kind::{KindGroup, ObjectKind};
use crate::name::{FoldPolicy, FoldRule, IdentKey, Name, name_eq};
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
        /// A server-generated name (`t_pkey`, `SYS_C0012`); diff may match it structurally.
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
        /// Broken (an invalid view or routine the server reports as such).
        const BROKEN = 1 << 8;
        /// The name holds bidi controls or zero-width characters (the explorer badge).
        const INVISIBLE_CHARS = 1 << 9;
    }
}

/// No parent: the namespace root.
pub(crate) const NO_PARENT: u32 = u32::MAX;

/// One catalog object. The layout fields (parent and children range) are set by the arena.
#[derive(Debug, Clone, PartialEq)]
pub struct Obj {
    pub kind: ObjectKind,
    pub name: Name,
    pub server_id: Option<ServerId>,
    pub state: StateMark,
    pub flags: ObjFlags,
    pub props: Props,
    pub(crate) parent: u32,
    pub(crate) first_child: u32,
    pub(crate) child_count: u32,
}

impl Obj {
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

    /// The parent's arena index (`None` for the namespace root).
    pub fn parent(&self) -> Option<u32> {
        (self.parent != NO_PARENT).then_some(self.parent)
    }

    pub fn child_count(&self) -> u32 {
        self.child_count
    }

    /// Whether `self` and `other` are the same server object (HV-MODEL-01 identity, the
    /// object-level form of [`ObjectKey::same_object`] for two siblings).
    fn same_identity(&self, other: &Obj) -> bool {
        if self.kind != other.kind {
            return false;
        }
        match (self.server_id, other.server_id) {
            (Some(a), Some(b)) => a == b,
            _ => self.name == other.name && self.props.overload_sig() == other.props.overload_sig(),
        }
    }
}

/// How much of a namespace was introspected (HV-INTRO-01 levels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum Level {
    /// Major object names and routine signatures.
    #[default]
    Names,
    /// Everything but source text and expensive facets.
    Structure,
    /// Structure plus source text.
    Sources,
}

impl Level {
    pub(crate) fn code(self) -> u8 {
        match self {
            Level::Names => 1,
            Level::Structure => 2,
            Level::Sources => 3,
        }
    }

    pub(crate) fn from_code(c: u8) -> Option<Self> {
        Some(match c {
            1 => Level::Names,
            2 => Level::Structure,
            3 => Level::Sources,
            _ => return None,
        })
    }
}

/// The namespace-level change watermark incremental introspection starts from
/// (HV-INTRO-05..08).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Watermark {
    #[default]
    None,
    /// PostgreSQL: the snapshot xmin (64-bit, epoch-extended) and the cluster's
    /// `system_identifier`.
    PgXid { xid: u64, system_id: u64 },
    /// MySQL: the newest `CREATE_TIME` seen, microseconds since the Unix epoch (UTC), and
    /// `@@server_uuid`.
    MyTime {
        at_utc_micros: i64,
        server_uuid: Option<Name>,
    },
    /// SQLite: `schema_version` and a digest of `sqlite_schema`.
    SqliteSchema { version: u32, digest: u64 },
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
    /// The namespace (or object) is not in the catalog.
    NoSuchNamespace(NsKey),
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
        }
    }
}

impl std::error::Error for ModelError {}

/// A node handle inside a [`Subtree`] being built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(u32);

/// A tree of new objects: a whole namespace (root = the schema), or one table with its
/// columns, indexes and constraints for a fragment refresh. Children keep insertion order
/// (introspectors insert columns by ordinal).
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

    pub fn with_capacity(root: Obj, capacity: usize) -> Self {
        let mut objs = Vec::with_capacity(capacity);
        let mut parents = Vec::with_capacity(capacity);
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

    pub fn len(&self) -> usize {
        self.objs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objs.is_empty()
    }

    /// Adds every node of `other` under `parent`, keeping `other`'s shape; returns the node
    /// of `other`'s root.
    pub fn graft_under(&mut self, parent: NodeId, other: Subtree) -> NodeId {
        let root = u32::try_from(self.objs.len()).unwrap_or(NO_PARENT);
        self.graft(parent.0, other);
        NodeId(root)
    }

    fn graft(&mut self, parent: u32, other: Subtree) {
        let base = u32::try_from(self.objs.len()).unwrap_or(NO_PARENT);
        for (obj, p) in other.objs.into_iter().zip(other.parents) {
            self.objs.push(obj);
            self.parents
                .push(if p == NO_PARENT { parent } else { base + p });
        }
    }
}

/// One map per [`KindGroup`] (indexed by [`KindGroup::index`]); `IdentKey: Borrow<str>`, so
/// a probe that needs no folding is looked up without allocating.
type NameIndex = Vec<HashMap<IdentKey, SmallVec<[u32; 2]>>>;

/// One immutable namespace arena (alg/02 HV-MODEL-01 `NamespaceSnapshot`).
#[derive(Debug, Clone)]
pub struct NamespaceSnapshot {
    key: NsKey,
    dbms: DbmsKind,
    fold: FoldPolicy,
    meta: NsMeta,
    generation: u64,
    from_newer_minor: bool,
    objs: Vec<Obj>,
    names: NameIndex,
    by_server_id: HashMap<(ObjectKind, ServerId), u32>,
}

impl NamespaceSnapshot {
    /// Builds a namespace from `tree` (root = the namespace's root object), checking the root
    /// against `key` and every edge against the kind schema of `dbms`.
    pub fn build(
        key: NsKey,
        dbms: DbmsKind,
        fold: FoldPolicy,
        meta: NsMeta,
        tree: Subtree,
    ) -> Result<Self, ModelError> {
        let objs = layout(tree)?;
        Self::from_arena(key, dbms, fold, meta, objs, false)
    }

    /// Wraps an arena already in breadth-first order (the decoder's output) after checking
    /// its layout and kind schema; computes the children ranges and the indexes.
    pub(crate) fn from_arena(
        key: NsKey,
        dbms: DbmsKind,
        fold: FoldPolicy,
        meta: NsMeta,
        mut objs: Vec<Obj>,
        from_newer_minor: bool,
    ) -> Result<Self, ModelError> {
        let n = u32::try_from(objs.len()).map_err(|_| ModelError::TooManyObjects)?;
        if n == 0 || n == NO_PARENT {
            return Err(if n == 0 {
                ModelError::BadLayout("empty arena")
            } else {
                ModelError::TooManyObjects
            });
        }
        let root_kind = key.kind.root_kind();
        if objs[0].kind != root_kind {
            return Err(ModelError::RootKind {
                expected: root_kind,
                found: objs[0].kind,
            });
        }
        if &*objs[0].name != key.root_name() {
            return Err(ModelError::RootName {
                expected: key.root_name().into(),
                found: objs[0].name.clone(),
            });
        }
        if objs[0].parent != NO_PARENT {
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
            from_newer_minor,
            objs,
            names: Vec::new(),
            by_server_id: HashMap::new(),
        };
        ns.index();
        Ok(ns)
    }

    fn index(&mut self) {
        let mut names: NameIndex = vec![HashMap::new(); KindGroup::COUNT];
        let mut by_sid = HashMap::new();
        for (i, o) in self.objs.iter().enumerate().skip(1) {
            let i = i as u32;
            if let Some(g) = o.kind.group(self.dbms) {
                let k = IdentKey::new(self.fold.rule(g), &o.name);
                names[g.index()].entry(k).or_default().push(i);
            }
            if let Some(sid) = o.server_id {
                by_sid.insert((o.kind, sid), i);
            }
        }
        self.names = names;
        self.by_server_id = by_sid;
    }

    pub fn key(&self) -> &NsKey {
        &self.key
    }

    pub fn dbms(&self) -> DbmsKind {
        self.dbms
    }

    pub fn fold(&self) -> FoldPolicy {
        self.fold
    }

    pub fn meta(&self) -> &NsMeta {
        &self.meta
    }

    /// The catalog generation that published this arena (0 until published).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn set_generation(&mut self, g: u64) {
        self.generation = g;
    }

    /// Whether this arena was decoded from a newer format minor: it is never written back
    /// (HV-MODEL-03 edge 4); a fresh introspection replaces it.
    pub fn from_newer_minor(&self) -> bool {
        self.from_newer_minor
    }

    pub fn len(&self) -> usize {
        self.objs.len()
    }

    /// Never true: a namespace always has its root.
    pub fn is_empty(&self) -> bool {
        self.objs.is_empty()
    }

    pub fn objs(&self) -> &[Obj] {
        &self.objs
    }

    pub fn obj(&self, idx: u32) -> Option<&Obj> {
        self.objs.get(idx as usize)
    }

    pub fn root(&self) -> &Obj {
        &self.objs[0]
    }

    /// The children of `idx`, in order, with their indexes. Allocation-free.
    pub fn children(&self, idx: u32) -> impl Iterator<Item = (u32, &Obj)> + '_ {
        let (first, count) = self
            .objs
            .get(idx as usize)
            .map_or((0, 0), |o| (o.first_child, o.child_count));
        let first = first as usize;
        self.objs[first..first + count as usize]
            .iter()
            .enumerate()
            .map(move |(k, o)| ((first + k) as u32, o))
    }

    /// The children of `idx` of one kind.
    pub fn children_of_kind(
        &self,
        idx: u32,
        kind: ObjectKind,
    ) -> impl Iterator<Item = (u32, &Obj)> + '_ {
        self.children(idx).filter(move |(_, o)| o.kind == kind)
    }

    /// Namespace-scoped objects of `group` named `name` (the catalog's form of the name: for
    /// PostgreSQL the caller folds unquoted input first). Overloads return several.
    /// Allocates only for an ASCII-caseless probe that contains an uppercase ASCII letter.
    pub fn lookup(&self, group: KindGroup, name: &str) -> &[u32] {
        let map = &self.names[group.index()];
        let hit = match self.fold.rule(group) {
            FoldRule::AsciiCaseless if name.bytes().any(|b| b.is_ascii_uppercase()) => {
                map.get(name.to_ascii_lowercase().as_str())
            }
            _ => map.get(name),
        };
        hit.map_or(&[][..], |v| v.as_slice())
    }

    /// Namespace-scoped objects of `kind` named `name`.
    pub fn lookup_kind(&self, kind: ObjectKind, name: &str) -> impl Iterator<Item = u32> + '_ {
        let hits = kind
            .group(self.dbms)
            .map_or(&[][..], |g| self.lookup(g, name));
        hits.iter()
            .copied()
            .filter(move |&i| self.objs[i as usize].kind == kind)
    }

    /// A child of `parent` of `kind` named `name` (columns of a table, …): a scan of the
    /// contiguous children, folded by the dialect's non-relation rule.
    pub fn child_by_name(&self, parent: u32, kind: ObjectKind, name: &str) -> Option<u32> {
        let rule = kind
            .group(self.dbms)
            .map_or(self.fold.other, |g| self.fold.rule(g));
        self.children_of_kind(parent, kind)
            .find(|(_, o)| name_eq(rule, &o.name, name))
            .map(|(i, _)| i)
    }

    /// The stable key of the object at `idx`.
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
        })
    }

    /// The arena index of the object `key` denotes ([`ObjectKey::same_object`]): by server id
    /// when both sides have one, else by exact path (and overload signature).
    pub fn find(&self, key: &ObjectKey) -> Option<u32> {
        if let Some(sid) = key.server_id
            && let Some(&i) = self.by_server_id.get(&(key.kind, sid))
        {
            return Some(i);
        }
        let rest = key.path.strip_prefix(self.key.path.as_slice())?;
        if rest.is_empty() {
            return (key.kind == self.objs[0].kind).then_some(0);
        }
        let i = self.walk(0, rest, key)?;
        let o = &self.objs[i as usize];
        match (o.server_id, key.server_id) {
            (Some(a), Some(b)) if a != b => None,
            _ => Some(i),
        }
    }

    fn walk(&self, at: u32, rest: &[Name], key: &ObjectKey) -> Option<u32> {
        let (head, tail) = rest.split_first()?;
        for (i, o) in self.children(at) {
            if o.name != *head {
                continue;
            }
            if tail.is_empty() {
                if o.kind == key.kind && o.props.overload_sig() == key.overload_sig.as_ref() {
                    return Some(i);
                }
            } else if o.child_count > 0
                && let Some(found) = self.walk(i, tail, key)
            {
                return Some(found);
            }
        }
        None
    }

    /// A new arena with the subtree at `target` replaced by `with` (in the same position among
    /// its siblings), or removed when `with` is `None`. Cost: one pass over this namespace;
    /// objects are cloned by reference count (props are `Arc`s).
    pub fn replace_subtree(&self, target: u32, with: Option<Subtree>) -> Result<Self, ModelError> {
        if target == 0 || target as usize >= self.objs.len() {
            return Err(ModelError::NoSuchObject);
        }
        self.rebuild(Some(target), with, None)
    }

    /// A new arena where `subtree` replaces the child of `parent` that is the same object
    /// (server id, else kind + name + overload signature), or is appended as `parent`'s last
    /// child when there is none.
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
        // Old index → new node; NO_PARENT marks a dropped object (the target's subtree).
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
            map[i] = tree.objs.len() as u32;
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
            false,
        )
    }

    /// The number of columns in the namespace.
    pub fn column_count(&self) -> usize {
        self.objs
            .iter()
            .filter(|o| o.kind == ObjectKind::Column)
            .count()
    }
}

/// Lays `tree` out breadth-first (children of every parent contiguous, insertion order kept)
/// and sets each object's `parent`.
fn layout(tree: Subtree) -> Result<Vec<Obj>, ModelError> {
    let n = tree.objs.len();
    if n == 0 {
        return Err(ModelError::BadLayout("empty tree"));
    }
    if u32::try_from(n).map_or(true, |n| n == NO_PARENT) {
        return Err(ModelError::TooManyObjects);
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
    let mut kids = vec![0u32; n.saturating_sub(1)];
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
