//! `ExplorerModel`: the explorer's [`TreeModel`] over catalog snapshots (Task 14): profiles →
//! contributed nodes → databases → schemas → kind groups → objects → child groups → leaves.
//! `TreeKey`s are derived from `ObjectKey` with a domain-separated XXH3-128, never from a
//! display string.
//!
//! ## m4-t14-explorer
//!
//! **Inputs.** [`ExplorerInputs`]: the profiles (file order) with their root rows' live parts
//! and each one's `Option<Arc<CatalogSnapshot>>`, the localization, the contributed nodes and
//! the filter result. The model reads the snapshots synchronously and never does I/O (spec
//! §5.4 rule 2): `children` is computed lazily the first time the tree asks (only expanded
//! nodes are asked) and kept for the generation; the sorted members of a kind group are kept
//! across generations in an [`ExplorerCache`] while their namespace arena is the same `Arc`.
//!
//! **Shape** (NOTES M4 surface 1). Roots: the M2a environment headers (labels: no chevron, not
//! cursor stops) and the profile rows (R-M4-18). Under a profile:
//! - PostgreSQL and the other database-level engines: its databases (the profile's database
//!   first, then by collation; one listed out of scope is dim with "Add to scope", R-M4-14),
//!   then server-level objects. Under a database: contributed nodes (R-M4-19), its schemas,
//!   database-level objects, and a collapsed **Built-in** group last when the catalog holds
//!   system schemas of that database (`ObjFlags::SYSTEM` / `PRE_INTROSPECTED` on the schema;
//!   its "version" badge is Task 26's).
//! - MySQL/MariaDB: its schemas, labelled "database" (`model-kind-schema-mysql`), each with
//!   its contributed nodes first; then a Built-in group for the system schemas.
//! - SQLite: contributed nodes, then `main`, `temp` and the attached schemas (no Built-in:
//!   SQLite never uses a snapshot, R-M4-6).
//! - Under a schema or an object: the kind groups in a fixed order ([`DisplayGroup`]), then
//!   objects of kinds without a group.
//!
//! **Fail-closed** (F31; T1c review m-B5; T1a m9). No catalog, or a catalog at generation 0,
//! shows the child row "Not loaded", never an empty database. A namespace the catalog lists
//! only by its load state shows that state (Not loaded / Loads when you open it, Loading…,
//! Couldn't read with Retry, dim out of scope); a relation in a `Names`-level namespace has no
//! columns yet and shows "Not loaded", never zero columns.
//!
//! **Keys.** `xxh3_128("hv-object\0" ‖ profile uuid ‖ kind code ‖ raw kind ‖ (database ‖
//! server id) or (path ‖ overload signature))`, every part length-prefixed: two profiles never
//! share a key, and an object with a server id (a PostgreSQL OID) keeps its key through a
//! rename, so its expansion survives. Namespace roots (databases, schemas) are keyed by their
//! path only, so a schema's key does not change when it is first loaded. Group headers hash
//! `"hv-group\0"`, their owner's key and the group; profile roots `"hv-profile\0"`; the
//! Built-in group `"hv-builtin\0"`; environment headers `"hv-env\0"`. A rename hint of a
//! name-only engine (MySQL, `CatalogDelta::renamed`) moves the expansion from the old key to
//! the new one ([`set_catalog`]).
//!
//! **Filter** (D-074; surface 4). [`FilterJob`] matches the profiles' names and hosts (as M2a
//! did) and every loaded name (`NamespaceSnapshot::iter_named` over every kind) with the
//! `hacivat-l10n` search key in `Contains` mode, Turkish-tolerant when the filter's toggle is
//! on. Each match reveals its ancestors (the explorer expands them while the filter is on).
//! The job is `Send`: the explorer runs it off the UI thread when more than
//! [`SYNC_FILTER_NAMES`] names would be matched, latest wins.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::hash::BuildHasherDefault;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use hacivat_l10n::{ArgValue, L10n, MatchMode, PreparedQuery, SearchOpts, build_key};
use hacivat_meta::kind::{DbLevel, database_level};
use hacivat_meta::snapshot::ContainerKey;
use hacivat_meta::{
    CatalogDelta, CatalogSnapshot, Level, LoadState, Name, NamespaceSnapshot, NsKey, NsKind,
    ObjFlags, ObjectKey, ObjectKind, ServerId,
};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_session::profile::Profile;
use hacivat_ui::icon::IconName;
use hacivat_ui::text::TextSource;
use hacivat_ui::tree::flat::KeyHasher;
use hacivat_ui::tree::{
    ChildState, Fold, KeySet, MetaMark, RowAction, RowFlag, TreeKey, TreeModel, TreeRowData,
};
use hacivat_ui_core::gpui_surface::{App, Global, SharedString};
use twox_hash::XxHash3_128;

use super::badges::{self, Badge};
// ## m4-t20-levels (named exception)
use super::contrib::{ContributedChildren, ContributedNode};
use super::level_pill;
use super::rows::{self, DisplayGroup, NameOrder};
use hacivat_meta::userdata::ProfileUserData;

/// At most this many names are matched on the UI thread; more go to the background executor.
pub const SYNC_FILTER_NAMES: usize = 2_000;

type KeyMap<V> = HashMap<TreeKey, V, BuildHasherDefault<KeyHasher>>;

// ---- keys -------------------------------------------------------------------------------------

const D_OBJECT: &[u8] = b"hv-object\0";
const D_PROFILE: &[u8] = b"hv-profile\0";
const D_GROUP: &[u8] = b"hv-group\0";
const D_BUILTIN: &[u8] = b"hv-builtin\0";
const D_ENV: &[u8] = b"hv-env\0";

/// Length-prefixed parts, hashed once.
struct KeyBytes(Vec<u8>);

impl KeyBytes {
    fn new(domain: &[u8], buf: Vec<u8>) -> KeyBytes {
        let mut k = KeyBytes(buf);
        k.0.clear();
        k.part(domain);
        k
    }

    fn part(&mut self, bytes: &[u8]) {
        let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
        self.0.extend_from_slice(&len.to_le_bytes());
        self.0.extend_from_slice(bytes);
    }

    fn finish(self) -> (TreeKey, Vec<u8>) {
        (TreeKey(XxHash3_128::oneshot(&self.0)), self.0)
    }
}

fn server_id_bytes(id: ServerId) -> Vec<u8> {
    let mut out = Vec::with_capacity(17);
    match id {
        ServerId::Oid(v) => {
            out.push(1);
            out.extend_from_slice(&v.to_le_bytes());
        }
        ServerId::ObjectId(v) => {
            out.push(2);
            out.extend_from_slice(&v.to_le_bytes());
        }
        ServerId::ObjNum(v) => {
            out.push(3);
            out.extend_from_slice(&v.to_le_bytes());
        }
        ServerId::Uuid(v) => {
            out.push(4);
            out.extend_from_slice(&v);
        }
        ServerId::RowId(v) => {
            out.push(5);
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

/// The parts of an object key, in the order [`object_tree_key`] hashes them.
struct ObjectParts<'a, I: Iterator<Item = &'a str> + Clone> {
    kind: ObjectKind,
    raw_kind: Option<&'a str>,
    server_id: Option<ServerId>,
    path: I,
    overload: Option<&'a str>,
}

fn object_bytes<'a, I: Iterator<Item = &'a str> + Clone>(
    profile: ProfileId,
    parts: ObjectParts<'a, I>,
    buf: Vec<u8>,
) -> (TreeKey, Vec<u8>) {
    let mut k = KeyBytes::new(D_OBJECT, buf);
    k.part(profile.0.as_bytes());
    k.part(&parts.kind.code().to_le_bytes());
    match parts.raw_kind {
        Some(raw) => {
            k.part(b"R");
            k.part(raw.as_bytes());
        }
        None => k.part(b"-"),
    }
    match parts.server_id {
        Some(id) => {
            // Server ids are unique per database (PostgreSQL OIDs): the database is the first
            // path step.
            k.part(b"I");
            k.part(parts.path.clone().next().unwrap_or_default().as_bytes());
            k.part(&server_id_bytes(id));
        }
        None => {
            k.part(b"P");
            let n = parts.path.clone().count();
            k.part(&u32::try_from(n).unwrap_or(u32::MAX).to_le_bytes());
            for name in parts.path {
                k.part(name.as_bytes());
            }
            match parts.overload {
                Some(sig) => {
                    k.part(b"O");
                    k.part(sig.as_bytes());
                }
                None => k.part(b"-"),
            }
        }
    }
    k.finish()
}

/// The tree key of catalog object `key` of `profile`.
pub fn object_tree_key(profile: ProfileId, key: &ObjectKey) -> TreeKey {
    object_bytes(
        profile,
        ObjectParts {
            kind: key.kind,
            raw_kind: key.raw_kind.as_deref(),
            server_id: key.server_id,
            path: key.path.iter().map(|n| n.as_ref()),
            overload: key.overload_sig.as_deref(),
        },
        Vec::new(),
    )
    .0
}

/// The tree key of a profile's root row.
pub fn profile_key(profile: ProfileId) -> TreeKey {
    let mut k = KeyBytes::new(D_PROFILE, Vec::new());
    k.part(profile.0.as_bytes());
    k.finish().0
}

/// The tree key of an environment header (`None`: "No environment").
pub fn env_key(tag: Option<EnvTag>) -> TreeKey {
    let code = tag
        .and_then(|t| EnvTag::ALL.iter().position(|x| *x == t))
        .map_or(0u8, |i| u8::try_from(i + 1).unwrap_or(u8::MAX));
    let mut k = KeyBytes::new(D_ENV, Vec::new());
    k.part(&[code]);
    k.finish().0
}

/// The tree key of a namespace root (a database or a schema): by kind and path only.
pub fn namespace_key(profile: ProfileId, ns: &NsKey) -> TreeKey {
    object_bytes(
        profile,
        ObjectParts {
            kind: ns.kind.root_kind(),
            raw_kind: None,
            server_id: None,
            path: ns.path.iter().map(|n| n.as_ref()),
            overload: None,
        },
        Vec::new(),
    )
    .0
}

/// The tree key of database `db` of `profile` (the database row).
pub fn database_key(profile: ProfileId, db: &str) -> TreeKey {
    namespace_key(profile, &NsKey::database(db))
}

/// The tree key of the kind-group header `group` under the row `owner`.
pub fn group_key(owner: TreeKey, group: DisplayGroup) -> TreeKey {
    let mut k = KeyBytes::new(D_GROUP, Vec::new());
    k.part(&owner.0.to_le_bytes());
    k.part(&[group.code()]);
    k.finish().0
}

/// The tree key of the Built-in group of database `db` (`None`: the profile's, MySQL).
pub fn builtin_key(profile: ProfileId, db: Option<&str>) -> TreeKey {
    let mut k = KeyBytes::new(D_BUILTIN, Vec::new());
    k.part(profile.0.as_bytes());
    match db {
        Some(db) => {
            k.part(b"D");
            k.part(db.as_bytes());
        }
        None => k.part(b"-"),
    }
    k.finish().0
}

/// The tree key of the object at `idx` of `ns` (equal to [`object_tree_key`] of
/// `ns.object_key(idx)`, without building the key). Index 0 is the namespace root:
/// [`namespace_key`].
fn arena_key(profile: ProfileId, ns: &NamespaceSnapshot, idx: u32, buf: &mut Vec<u8>) -> TreeKey {
    if idx == 0 {
        return namespace_key(profile, ns.key());
    }
    let objs = ns.objs();
    let mut chain: smallvec::SmallVec<[u32; 8]> = smallvec::SmallVec::new();
    let mut cur = idx;
    while cur != 0 {
        chain.push(cur);
        cur = objs[cur as usize].parent().unwrap_or(0);
    }
    let obj = &objs[idx as usize];
    let path = ns.key().path.iter().map(|n| n.as_ref()).chain(
        chain
            .iter()
            .rev()
            .map(|&i| objs[i as usize].name().as_ref()),
    );
    let (key, back) = object_bytes(
        profile,
        ObjectParts {
            kind: obj.kind(),
            raw_kind: obj.raw_kind().map(|r| r.as_ref()),
            server_id: obj.server_id(),
            path,
            overload: obj.props().overload_sig().map(|s| s.as_ref()),
        },
        std::mem::take(buf),
    );
    *buf = back;
    key
}

/// The type-ahead fold of the explorer tree (R-M4-50): Turkish-tolerant while the filter's
/// Turkish-tolerant toggle is on (its default is on for a Turkish UI language or region,
/// D-074); otherwise the access-key fold of the UI language.
pub fn typeahead_fold(tolerant: bool, ui_turkic: bool) -> Fold {
    if tolerant {
        Fold::Tolerant
    } else {
        Fold::Locale { turkic: ui_turkic }
    }
}

// ---- inputs -----------------------------------------------------------------------------------

/// One profile root: the profile, what its row shows besides the name, and its catalog.
#[derive(Clone)]
pub struct ProfileInput {
    pub profile: Arc<Profile>,
    /// Connected: the name in `body_strong` (NOTES M4 surface 1).
    pub connected: bool,
    pub mark: Option<MetaMark>,
    /// The latency while connected, else the engine (M2a, mock root meta).
    pub meta: Option<SharedString>,
    /// The flags after the name: the environment mark, the read-only lock, the SSH route and
    /// the plaintext lock (M2a).
    pub flags: Vec<RowFlag>,
    /// The environment, read-only and flag words (the widget adds none).
    pub a11y: Option<SharedString>,
    /// The tooltip of the row's live mark (an idle disconnect).
    pub tooltip: Option<SharedString>,
    /// The catalog the tree shows below the root (`None`: none loaded).
    pub catalog: Option<Arc<CatalogSnapshot>>,
    // ## m4-t20-levels (named exception): the profile's user data, for the level pills
    // (`None` until its file was read: no pill is drawn then).
    pub userdata: Option<Arc<ProfileUserData>>,
}

impl fmt::Debug for ProfileInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProfileInput")
            .field("profile", &self.profile.id)
            .field("connected", &self.connected)
            .field("catalog", &self.catalog.as_ref().map(|c| c.generation))
            .finish_non_exhaustive()
    }
}

impl ProfileInput {
    /// A disconnected root with no live parts, showing `catalog`.
    pub fn new(profile: Arc<Profile>, catalog: Option<Arc<CatalogSnapshot>>) -> ProfileInput {
        ProfileInput {
            profile,
            connected: false,
            mark: None,
            meta: None,
            flags: Vec::new(),
            a11y: None,
            tooltip: None,
            catalog,
            // ## m4-t20-levels (named exception)
            userdata: None,
        }
    }
}

/// The nodes one contributor puts under one database of one profile.
#[derive(Debug, Clone)]
pub struct ContributedPlace {
    pub profile: ProfileId,
    pub database: String,
    pub nodes: Vec<ContributedNode>,
}

/// The children of an expanded contributed node, asked for on the UI thread.
#[derive(Debug, Clone)]
pub struct ContributedLoad {
    pub state: ChildState,
    /// The rows of `state`'s keys.
    pub nodes: Vec<ContributedNode>,
}

/// Everything one generation of the model is built from.
#[derive(Clone, Default)]
pub struct ExplorerInputs {
    pub profiles: Vec<ProfileInput>,
    pub l10n: Option<Arc<L10n>>,
    /// The UI language (the display collation).
    pub lang: String,
    pub contributed: Vec<ContributedPlace>,
    pub contributed_children: HashMap<TreeKey, ContributedLoad>,
    pub filter: Option<Arc<FilterResult>>,
}

impl fmt::Debug for ExplorerInputs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExplorerInputs")
            .field("profiles", &self.profiles)
            .field("lang", &self.lang)
            .field("filter", &self.filter.as_ref().map(|r| r.count))
            .finish_non_exhaustive()
    }
}

// ---- layout -----------------------------------------------------------------------------------

/// One schema row (or a MySQL database): its namespace if loaded, its load state.
#[derive(Debug)]
pub(crate) struct SchemaEntry {
    key: NsKey,
    ns: Option<Arc<NamespaceSnapshot>>,
    state: Option<LoadState>,
    system: bool,
}

impl SchemaEntry {
    fn name(&self) -> &str {
        self.key.root_name()
    }

    fn out_of_scope(&self) -> bool {
        matches!(self.state, Some(LoadState::OutOfScope))
    }
}

/// One database row (PostgreSQL).
#[derive(Debug)]
pub(crate) struct DbEntry {
    name: Name,
    in_scope: bool,
    schemas: Vec<Arc<SchemaEntry>>,
    system: Vec<Arc<SchemaEntry>>,
    db_ns: Option<Arc<NamespaceSnapshot>>,
    /// Schemas in scope, schemas listed.
    scope: (usize, usize),
}

/// What one catalog shows under its profile root.
#[derive(Debug)]
pub(crate) struct Layout {
    level: DbLevel,
    dbms: DbmsKind,
    databases: Vec<Arc<DbEntry>>,
    schemas: Vec<Arc<SchemaEntry>>,
    system: Vec<Arc<SchemaEntry>>,
    server_ns: Option<Arc<NamespaceSnapshot>>,
}

fn is_system(ns: &NamespaceSnapshot) -> bool {
    ns.root()
        .flags()
        .intersects(ObjFlags::SYSTEM | ObjFlags::PRE_INTROSPECTED)
}

fn layout_of(profile: &Profile, cat: &CatalogSnapshot, order: &NameOrder) -> Layout {
    let level = database_level(profile.engine);
    let state_of = |k: &NsKey| cat.states.get(&ContainerKey::Namespace(k.clone())).cloned();
    // Every schema the catalog knows: loaded, or listed by its load state.
    let mut keys: Vec<NsKey> = cat
        .namespaces
        .keys()
        .filter(|k| k.kind == NsKind::Schema)
        .cloned()
        .collect();
    for k in cat.states.keys() {
        if let ContainerKey::Namespace(k) = k
            && k.kind == NsKind::Schema
            && !cat.namespaces.contains_key(k)
        {
            keys.push(k.clone());
        }
    }
    let entries: Vec<Arc<SchemaEntry>> = keys
        .into_iter()
        .map(|key| {
            let ns = cat.namespaces.get(&key).cloned();
            let system = ns.as_deref().is_some_and(is_system);
            let state = state_of(&key);
            Arc::new(SchemaEntry {
                key,
                ns,
                state,
                system,
            })
        })
        .collect();
    let by_name = |a: &Arc<SchemaEntry>, b: &Arc<SchemaEntry>| order.cmp(a.name(), b.name());
    let server_ns = cat.namespaces.get(&NsKey::server()).cloned();
    if level != DbLevel::Database {
        let (mut system, mut schemas): (Vec<_>, Vec<_>) =
            entries.into_iter().partition(|e| e.system);
        system.sort_by(by_name);
        schemas.sort_by(|a, b| {
            if level == DbLevel::File {
                // `main`, then `temp`, then the attached schemas.
                let rank = |e: &SchemaEntry| match e.name() {
                    "main" => 0,
                    "temp" => 1,
                    _ => 2,
                };
                rank(a).cmp(&rank(b)).then_with(|| by_name(a, b))
            } else {
                by_name(a, b)
            }
        });
        return Layout {
            level,
            dbms: profile.engine,
            databases: Vec::new(),
            schemas,
            system,
            server_ns,
        };
    }
    // Database-level engines: schemas by their database (the first path step).
    let mut names: Vec<Name> = Vec::new();
    let mut push = |n: &Name| {
        if !names.iter().any(|x| x == n) {
            names.push(n.clone());
        }
    };
    if let Some(db) = &profile.database {
        push(&Name::from(db.as_str()));
    }
    for e in &entries {
        if let Some(db) = e.key.path.first() {
            push(db);
        }
    }
    for k in cat.namespaces.keys() {
        if k.kind == NsKind::Database
            && let Some(db) = k.path.first()
        {
            push(db);
        }
    }
    for k in cat.states.keys() {
        if let ContainerKey::Namespace(k) = k
            && k.kind == NsKind::Database
            && let Some(db) = k.path.first()
        {
            push(db);
        }
    }
    let connected = profile.database.as_deref();
    names.sort_by(|a, b| {
        let first = |n: &Name| connected != Some(n.as_ref());
        first(a).cmp(&first(b)).then_with(|| order.cmp(a, b))
    });
    let databases = names
        .into_iter()
        .map(|name| {
            let db_key = NsKey::database(name.clone());
            let in_scope = !matches!(state_of(&db_key), Some(LoadState::OutOfScope));
            let mine = entries
                .iter()
                .filter(|e| e.key.path.first() == Some(&name))
                .cloned();
            let (mut system, mut schemas): (Vec<_>, Vec<_>) = mine.partition(|e| e.system);
            system.sort_by(by_name);
            schemas.sort_by(by_name);
            let scope = (
                schemas.iter().filter(|e| !e.out_of_scope()).count(),
                schemas.len(),
            );
            Arc::new(DbEntry {
                db_ns: cat.namespaces.get(&db_key).cloned(),
                name,
                in_scope,
                schemas,
                system,
                scope,
            })
        })
        .collect();
    Layout {
        level,
        dbms: profile.engine,
        databases,
        schemas: Vec::new(),
        system: Vec::new(),
        server_ns,
    }
}

// ---- cache ------------------------------------------------------------------------------------

/// What survives from one model generation to the next: the display order, each profile's
/// layout (while its snapshot is the same `Arc`), and the sorted members of kind groups (while
/// their namespace arena is the same `Arc`).
#[derive(Default)]
pub struct ExplorerCache {
    inner: RefCell<CacheInner>,
}

impl fmt::Debug for ExplorerCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = self.inner.borrow();
        f.debug_struct("ExplorerCache")
            .field("layouts", &inner.layouts.len())
            .field("sorted", &inner.sorted.len())
            .finish()
    }
}

/// A kind group's members: `(profile, namespace arena address, owner index, group)`.
type SortKey = (ProfileId, usize, u32, DisplayGroup);

/// A kind group's members in display order: `(arena index, key)`.
type Sorted = Arc<[(u32, TreeKey)]>;

#[derive(Default)]
struct CacheInner {
    order: Option<Rc<NameOrder>>,
    layouts: HashMap<ProfileId, (Arc<CatalogSnapshot>, Arc<Layout>)>,
    /// The arena is kept alive with its entry, so its address is not reused while cached.
    sorted: HashMap<SortKey, (Arc<NamespaceSnapshot>, Sorted)>,
}

impl ExplorerCache {
    fn order(&self, lang: &str) -> Rc<NameOrder> {
        let mut inner = self.inner.borrow_mut();
        if let Some(o) = inner
            .order
            .as_ref()
            .filter(|o| o.lang() == lang_or_root(lang))
        {
            return o.clone();
        }
        // Another language: every display order changes.
        inner.layouts.clear();
        inner.sorted.clear();
        let o = Rc::new(NameOrder::new(lang));
        inner.order = Some(o.clone());
        o
    }

    fn layout(
        &self,
        profile: &Profile,
        cat: &Arc<CatalogSnapshot>,
        order: &NameOrder,
    ) -> Arc<Layout> {
        if let Some((snap, layout)) = self.inner.borrow().layouts.get(&profile.id)
            && Arc::ptr_eq(snap, cat)
            && layout.dbms == profile.engine
        {
            return layout.clone();
        }
        let layout = Arc::new(layout_of(profile, cat, order));
        self.inner
            .borrow_mut()
            .layouts
            .insert(profile.id, (cat.clone(), layout.clone()));
        layout
    }

    /// Drops what no current snapshot holds any more.
    fn retain(&self, live: &[(ProfileId, Option<Arc<CatalogSnapshot>>)]) {
        let mut inner = self.inner.borrow_mut();
        inner.layouts.retain(|id, (snap, _)| {
            live.iter()
                .any(|(p, c)| p == id && c.as_ref().is_some_and(|c| Arc::ptr_eq(c, snap)))
        });
        let arenas: std::collections::HashSet<usize> = live
            .iter()
            .filter_map(|(_, c)| c.as_ref())
            .flat_map(|c| c.namespaces.values().map(|n| Arc::as_ptr(n) as usize))
            .collect();
        inner.sorted.retain(|k, _| arenas.contains(&k.1));
    }
}

fn lang_or_root(lang: &str) -> &str {
    if lang.trim().is_empty() { "und" } else { lang }
}

// ---- nodes ------------------------------------------------------------------------------------

/// The row a group's members or an object's children hang from.
#[derive(Clone)]
struct Owner {
    ns: Arc<NamespaceSnapshot>,
    idx: u32,
    key: TreeKey,
}

#[derive(Clone)]
enum Node {
    Env {
        tag: Option<EnvTag>,
        count: usize,
    },
    Profile(usize),
    Database {
        p: usize,
        db: Arc<DbEntry>,
    },
    Schema {
        p: usize,
        entry: Arc<SchemaEntry>,
    },
    BuiltIn {
        p: usize,
        db: Option<Arc<DbEntry>>,
    },
    Group {
        p: usize,
        group: DisplayGroup,
        owner: Owner,
        members: Arc<[u32]>,
    },
    Object {
        p: usize,
        ns: Arc<NamespaceSnapshot>,
        idx: u32,
    },
    Contributed {
        p: usize,
        node: Box<ContributedNode>,
    },
}

/// What a row of the explorer is (for the host's actions and the tests).
#[derive(Debug, Clone, PartialEq)]
pub enum NodeInfo {
    /// An environment header (a label).
    Environment(Option<EnvTag>),
    /// A profile root.
    Profile(ProfileId),
    /// A database row (PostgreSQL).
    Database { profile: ProfileId, name: Name },
    /// A schema row (a MySQL/MariaDB database; SQLite `main`, `temp`, attached).
    Schema { profile: ProfileId, ns: NsKey },
    /// The Built-in group of a database (`None`: the profile's).
    BuiltIn {
        profile: ProfileId,
        database: Option<Name>,
    },
    /// A kind-group header.
    Group {
        profile: ProfileId,
        group: DisplayGroup,
    },
    /// A catalog object.
    Object { profile: ProfileId, key: ObjectKey },
    /// A contributed node (R-M4-19).
    Contributed { profile: ProfileId },
}

struct Inner {
    inputs: ExplorerInputs,
    cache: Rc<ExplorerCache>,
    order: Option<Rc<NameOrder>>,
    layouts: Vec<Option<Arc<Layout>>>,
    roots: Arc<[TreeKey]>,
    nodes: RefCell<KeyMap<Node>>,
    kids: RefCell<KeyMap<ChildState>>,
    texts: RefCell<HashMap<&'static str, SharedString>>,
}

impl Default for Inner {
    fn default() -> Self {
        Inner {
            inputs: ExplorerInputs::default(),
            cache: Rc::default(),
            order: None,
            layouts: Vec::new(),
            roots: Arc::from([]),
            nodes: RefCell::default(),
            kids: RefCell::default(),
            texts: RefCell::default(),
        }
    }
}

/// The explorer's tree model: one immutable generation of [`ExplorerInputs`], with its nodes
/// computed lazily as the tree asks for them.
#[derive(Clone, Default)]
pub struct ExplorerModel {
    generation: u64,
    inner: Rc<Inner>,
}

impl fmt::Debug for ExplorerModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExplorerModel")
            .field("generation", &self.generation)
            .field("roots", &self.inner.roots.len())
            .field("nodes", &self.inner.nodes.borrow().len())
            .finish_non_exhaustive()
    }
}

/// The environment groups in M2a order: by severity, then "No environment" last (R31).
fn env_order() -> Vec<Option<EnvTag>> {
    let mut order: Vec<Option<EnvTag>> = EnvTag::ALL.iter().copied().map(Some).collect();
    order.sort_by_key(|t| t.map(EnvTag::severity_rank));
    order.push(None);
    order
}

impl ExplorerModel {
    /// Generation `generation` over `inputs` (the host bumps it on every change).
    pub fn build(inputs: ExplorerInputs, cache: Rc<ExplorerCache>, generation: u64) -> Self {
        let has_catalog = inputs.profiles.iter().any(|p| p.catalog.is_some());
        let order = has_catalog.then(|| cache.order(&inputs.lang));
        let live: Vec<(ProfileId, Option<Arc<CatalogSnapshot>>)> = inputs
            .profiles
            .iter()
            .map(|p| (p.profile.id, p.catalog.clone()))
            .collect();
        cache.retain(&live);
        let layouts = inputs
            .profiles
            .iter()
            .map(|p| {
                let cat = p.catalog.as_ref().filter(|c| c.generation > 0)?;
                Some(cache.layout(&p.profile, cat, order.as_deref()?))
            })
            .collect();
        let mut nodes = KeyMap::default();
        let mut roots = Vec::new();
        let filter = inputs.filter.clone();
        for tag in env_order() {
            let shown: Vec<usize> = inputs
                .profiles
                .iter()
                .enumerate()
                .filter(|(_, p)| p.profile.environment == tag)
                .filter(|(_, p)| {
                    filter
                        .as_ref()
                        .is_none_or(|f| f.keep.contains(&profile_key(p.profile.id)))
                })
                .map(|(i, _)| i)
                .collect();
            if shown.is_empty() {
                continue;
            }
            let ek = env_key(tag);
            nodes.insert(
                ek,
                Node::Env {
                    tag,
                    count: shown.len(),
                },
            );
            roots.push(ek);
            for i in shown {
                let k = profile_key(inputs.profiles[i].profile.id);
                nodes.insert(k, Node::Profile(i));
                roots.push(k);
            }
        }
        ExplorerModel {
            generation,
            inner: Rc::new(Inner {
                inputs,
                cache,
                order,
                layouts,
                roots: Arc::from(roots),
                nodes: RefCell::new(nodes),
                kids: RefCell::default(),
                texts: RefCell::default(),
            }),
        }
    }

    /// The inputs this generation was built from.
    pub fn inputs(&self) -> &ExplorerInputs {
        &self.inner.inputs
    }

    /// The cache shared with the next generation.
    pub fn cache(&self) -> Rc<ExplorerCache> {
        self.inner.cache.clone()
    }

    /// The profile a row belongs to (`None` for an environment header or an unknown key).
    pub fn profile_of(&self, key: TreeKey) -> Option<ProfileId> {
        let p = match self.inner.nodes.borrow().get(&key)? {
            Node::Env { .. } => return None,
            Node::Profile(p)
            | Node::Database { p, .. }
            | Node::Schema { p, .. }
            | Node::BuiltIn { p, .. }
            | Node::Group { p, .. }
            | Node::Object { p, .. }
            | Node::Contributed { p, .. } => *p,
        };
        Some(self.inner.inputs.profiles[p].profile.id)
    }

    /// The profile whose root row `key` is.
    pub fn profile_root(&self, key: TreeKey) -> Option<ProfileId> {
        match self.inner.nodes.borrow().get(&key)? {
            Node::Profile(p) => Some(self.inner.inputs.profiles[*p].profile.id),
            _ => None,
        }
    }

    /// What row `key` is (`None` for a key this generation has not shown).
    pub fn info(&self, key: TreeKey) -> Option<NodeInfo> {
        let nodes = self.inner.nodes.borrow();
        let node = nodes.get(&key)?;
        let id = |p: usize| self.inner.inputs.profiles[p].profile.id;
        Some(match node {
            Node::Env { tag, .. } => NodeInfo::Environment(*tag),
            Node::Profile(p) => NodeInfo::Profile(id(*p)),
            Node::Database { p, db } => NodeInfo::Database {
                profile: id(*p),
                name: db.name.clone(),
            },
            Node::Schema { p, entry } => NodeInfo::Schema {
                profile: id(*p),
                ns: entry.key.clone(),
            },
            Node::BuiltIn { p, db } => NodeInfo::BuiltIn {
                profile: id(*p),
                database: db.as_ref().map(|d| d.name.clone()),
            },
            Node::Group { p, group, .. } => NodeInfo::Group {
                profile: id(*p),
                group: *group,
            },
            Node::Object { p, ns, idx } => NodeInfo::Object {
                profile: id(*p),
                key: ns.object_key(*idx)?,
            },
            Node::Contributed { p, .. } => NodeInfo::Contributed { profile: id(*p) },
        })
    }

    /// The match count of the filter (`None` while no filter is on).
    pub fn filter(&self) -> Option<&Arc<FilterResult>> {
        self.inner.inputs.filter.as_ref()
    }

    /// The filter's work for `query` (Turkish-tolerant when `tolerant`): every name this
    /// generation's catalogs and contributed nodes hold.
    pub fn filter_job(&self, query: &str, tolerant: bool) -> FilterJob {
        let inputs = &self.inner.inputs;
        let profiles = inputs
            .profiles
            .iter()
            .zip(&self.inner.layouts)
            .map(|(p, layout)| FilterProfile {
                id: p.profile.id,
                name: p.profile.name.clone(),
                host: crate::connections::explorer::endpoint_text(&p.profile),
                layout: layout.clone(),
            })
            .collect();
        let mut contributed = Vec::new();
        for place in &inputs.contributed {
            let root = profile_key(place.profile);
            let Some((p, layout)) = inputs
                .profiles
                .iter()
                .zip(&self.inner.layouts)
                .find(|(p, _)| p.profile.id == place.profile)
            else {
                continue;
            };
            let mut chain = vec![root];
            match layout.as_deref().map(|l| l.level) {
                Some(DbLevel::Database) => chain.push(database_key(p.profile.id, &place.database)),
                Some(DbLevel::SchemaIsDatabase) => chain.push(namespace_key(
                    p.profile.id,
                    &NsKey::schema([place.database.as_str()]),
                )),
                _ => {}
            }
            for node in &place.nodes {
                collect_contributed(node, &chain, inputs, &mut contributed);
            }
        }
        FilterJob {
            query: query.to_string(),
            tolerant,
            profiles,
            contributed,
            cancel: None,
        }
    }

    // ---- texts ----------------------------------------------------------------------------

    fn t(&self, id: &'static str) -> SharedString {
        if let Some(s) = self.inner.texts.borrow().get(id) {
            return s.clone();
        }
        let s = self.t_args(id, Vec::new());
        self.inner.texts.borrow_mut().insert(id, s.clone());
        s
    }

    fn t_args(&self, id: &str, args: Vec<(&'static str, ArgValue)>) -> SharedString {
        match &self.inner.inputs.l10n {
            Some(l) => l.t_dynamic(id, args).into(),
            None => SharedString::from(id.to_string()),
        }
    }

    fn count_text(&self, n: usize) -> SharedString {
        self.t_args(
            "hv-explorer-count",
            vec![("n", ArgValue::Count(u64::try_from(n).unwrap_or(u64::MAX)))],
        )
    }

    fn not_loaded(&self) -> ChildState {
        ChildState::Loading {
            label: self.t("hv-explorer-not-loaded"),
            running: false,
            count: None,
        }
    }

    // ---- children ---------------------------------------------------------------------------

    fn register(&self, key: TreeKey, node: Node) {
        self.inner.nodes.borrow_mut().insert(key, node);
    }

    /// `keys` under the filter: only the kept ones.
    fn shown(&self, keys: Vec<TreeKey>) -> ChildState {
        match &self.inner.inputs.filter {
            Some(f) => {
                ChildState::Loaded(keys.into_iter().filter(|k| f.keep.contains(k)).collect())
            }
            None => ChildState::Loaded(Arc::from(keys)),
        }
    }

    fn profile_id(&self, p: usize) -> ProfileId {
        self.inner.inputs.profiles[p].profile.id
    }

    fn compute(&self, key: TreeKey, node: &Node) -> ChildState {
        match node {
            Node::Env { .. } => ChildState::Leaf,
            Node::Profile(p) => self.profile_children(*p, key),
            Node::Database { p, db } => self.database_children(*p, db),
            Node::Schema { p, entry } => self.schema_children(*p, entry, key),
            Node::BuiltIn { p, db } => {
                let list = match db {
                    Some(db) => &db.system,
                    None => match self.inner.layouts[*p].as_deref() {
                        Some(l) => &l.system,
                        None => return self.not_loaded(),
                    },
                };
                let keys = self.schema_rows(*p, list);
                self.shown(keys)
            }
            Node::Group {
                p,
                group,
                owner,
                members,
            } => {
                let sorted = self.sorted(*p, owner, *group, members);
                let keys: Vec<TreeKey> = sorted
                    .iter()
                    .map(|&(idx, k)| {
                        self.register(
                            k,
                            Node::Object {
                                p: *p,
                                ns: owner.ns.clone(),
                                idx,
                            },
                        );
                        k
                    })
                    .collect();
                self.shown(keys)
            }
            Node::Object { p, ns, idx } => {
                let obj = &ns.objs()[*idx as usize];
                if obj.child_count() == 0 {
                    // A relation of a namespace read at `Names` has no columns yet: not a
                    // fact (F31).
                    if ns.meta().level == Level::Names && is_relation(obj.kind()) {
                        return self.not_loaded();
                    }
                    return ChildState::Leaf;
                }
                let owner = Owner {
                    ns: ns.clone(),
                    idx: *idx,
                    key,
                };
                let keys = self.container_keys(*p, &owner);
                self.shown(keys)
            }
            Node::Contributed { p, node } => match self.inner.inputs.contributed_children.get(&key)
            {
                Some(load) => {
                    for n in &load.nodes {
                        self.register(
                            n.key,
                            Node::Contributed {
                                p: *p,
                                node: Box::new(n.clone()),
                            },
                        );
                    }
                    match &load.state {
                        ChildState::Loaded(keys) => self.shown(keys.to_vec()),
                        other => other.clone(),
                    }
                }
                None => match node.children {
                    ContributedChildren::Leaf => ChildState::Leaf,
                    ContributedChildren::Lazy(_) => ChildState::NotLoaded,
                },
            },
        }
    }

    /// The contributed nodes of `database` of profile `p`, registered.
    fn contributed_keys(&self, p: usize, database: &str) -> Vec<TreeKey> {
        let id = self.profile_id(p);
        let mut keys = Vec::new();
        for place in &self.inner.inputs.contributed {
            if place.profile != id || place.database != database {
                continue;
            }
            for n in &place.nodes {
                self.register(
                    n.key,
                    Node::Contributed {
                        p,
                        node: Box::new(n.clone()),
                    },
                );
                keys.push(n.key);
            }
        }
        keys
    }

    fn schema_rows(&self, p: usize, list: &[Arc<SchemaEntry>]) -> Vec<TreeKey> {
        let id = self.profile_id(p);
        list.iter()
            .map(|e| {
                let k = namespace_key(id, &e.key);
                self.register(
                    k,
                    Node::Schema {
                        p,
                        entry: e.clone(),
                    },
                );
                k
            })
            .collect()
    }

    fn builtin_row(&self, p: usize, db: Option<&Arc<DbEntry>>) -> TreeKey {
        let k = builtin_key(self.profile_id(p), db.map(|d| d.name.as_ref()));
        self.register(k, Node::BuiltIn { p, db: db.cloned() });
        k
    }

    fn profile_children(&self, p: usize, key: TreeKey) -> ChildState {
        let Some(layout) = self.inner.layouts[p].clone() else {
            // No catalog, or generation 0: "Not loaded", never an empty database.
            return self.not_loaded();
        };
        let id = self.profile_id(p);
        let mut keys = Vec::new();
        match layout.level {
            DbLevel::Database => {
                for db in &layout.databases {
                    let k = database_key(id, &db.name);
                    self.register(k, Node::Database { p, db: db.clone() });
                    keys.push(k);
                }
            }
            DbLevel::SchemaIsDatabase => keys.extend(self.schema_rows(p, &layout.schemas)),
            DbLevel::File => {
                keys.extend(self.contributed_keys(p, "main"));
                keys.extend(self.schema_rows(p, &layout.schemas));
            }
        }
        if let Some(ns) = &layout.server_ns {
            let owner = Owner {
                ns: ns.clone(),
                idx: 0,
                key,
            };
            keys.extend(self.container_keys(p, &owner));
        }
        if layout.level == DbLevel::SchemaIsDatabase && !layout.system.is_empty() {
            keys.push(self.builtin_row(p, None));
        }
        if keys.is_empty() {
            return self.not_loaded();
        }
        self.shown(keys)
    }

    fn database_children(&self, p: usize, db: &Arc<DbEntry>) -> ChildState {
        if !db.in_scope {
            return ChildState::Leaf;
        }
        let id = self.profile_id(p);
        let mut keys = self.contributed_keys(p, &db.name);
        keys.extend(self.schema_rows(p, &db.schemas));
        if let Some(ns) = &db.db_ns {
            let owner = Owner {
                ns: ns.clone(),
                idx: 0,
                key: database_key(id, &db.name),
            };
            keys.extend(self.container_keys(p, &owner));
        }
        if !db.system.is_empty() {
            keys.push(self.builtin_row(p, Some(db)));
        }
        if keys.is_empty() {
            return self.not_loaded();
        }
        self.shown(keys)
    }

    fn schema_children(&self, p: usize, entry: &Arc<SchemaEntry>, key: TreeKey) -> ChildState {
        let level = self.inner.layouts[p].as_deref().map(|l| l.level);
        let mut keys = if level == Some(DbLevel::SchemaIsDatabase) {
            self.contributed_keys(p, entry.name())
        } else {
            Vec::new()
        };
        let Some(ns) = &entry.ns else {
            if !keys.is_empty() {
                return self.shown(keys);
            }
            return match &entry.state {
                Some(LoadState::OutOfScope) => ChildState::Leaf,
                Some(LoadState::Loading { .. }) => ChildState::Loading {
                    label: self.t("hv-explorer-loading"),
                    running: true,
                    count: None,
                },
                Some(LoadState::Failed { .. }) => ChildState::Failed {
                    label: self.t_args(
                        "hv-explorer-failed",
                        vec![("name", ArgValue::Text(entry.name().to_string()))],
                    ),
                    retry: Some(self.t("hv-explorer-retry")),
                },
                // A MySQL/MariaDB database loads when it is opened (R-M4-14).
                None | Some(LoadState::NotLoaded) if level == Some(DbLevel::SchemaIsDatabase) => {
                    ChildState::Loading {
                        label: self.t("hv-explorer-loads-on-open"),
                        running: false,
                        count: None,
                    }
                }
                _ => self.not_loaded(),
            };
        };
        let owner = Owner {
            ns: ns.clone(),
            idx: 0,
            key,
        };
        keys.extend(self.container_keys(p, &owner));
        self.shown(keys)
    }

    /// The kind groups of `owner`'s children (fixed order), then its ungrouped children.
    fn container_keys(&self, p: usize, owner: &Owner) -> Vec<TreeKey> {
        let ns = &owner.ns;
        let mut buckets: Vec<(DisplayGroup, Vec<u32>)> = Vec::new();
        let mut loose: Vec<u32> = Vec::new();
        for (i, obj) in ns.children(owner.idx) {
            match DisplayGroup::of(obj.kind()) {
                Some(g) => match buckets.iter_mut().find(|(x, _)| *x == g) {
                    Some((_, v)) => v.push(i),
                    None => buckets.push((g, vec![i])),
                },
                None => loose.push(i),
            }
        }
        buckets.sort_by_key(|(g, _)| DisplayGroup::ORDER.iter().position(|x| x == g));
        let mut keys = Vec::with_capacity(buckets.len() + loose.len());
        for (group, members) in buckets {
            let k = group_key(owner.key, group);
            self.register(
                k,
                Node::Group {
                    p,
                    group,
                    owner: owner.clone(),
                    members: Arc::from(members),
                },
            );
            keys.push(k);
        }
        if let Some(order) = &self.inner.order {
            rows::sort_members(&mut loose, ns.objs(), None, order);
        }
        let id = self.profile_id(p);
        let mut buf = Vec::new();
        for idx in loose {
            let k = arena_key(id, ns, idx, &mut buf);
            self.register(
                k,
                Node::Object {
                    p,
                    ns: ns.clone(),
                    idx,
                },
            );
            keys.push(k);
        }
        keys
    }

    /// A kind group's members in display order with their keys (cached across generations).
    fn sorted(&self, p: usize, owner: &Owner, group: DisplayGroup, members: &Arc<[u32]>) -> Sorted {
        let id = self.profile_id(p);
        let sk: SortKey = (id, Arc::as_ptr(&owner.ns) as usize, owner.idx, group);
        if let Some((_, hit)) = self.inner.cache.inner.borrow().sorted.get(&sk) {
            return hit.clone();
        }
        let mut list = members.to_vec();
        if let Some(order) = &self.inner.order {
            rows::sort_members(&mut list, owner.ns.objs(), Some(group), order);
        }
        let mut buf = Vec::new();
        let out: Sorted = list
            .into_iter()
            .map(|idx| (idx, arena_key(id, &owner.ns, idx, &mut buf)))
            .collect();
        self.inner
            .cache
            .inner
            .borrow_mut()
            .sorted
            .insert(sk, (owner.ns.clone(), out.clone()));
        out
    }

    // ---- rows -------------------------------------------------------------------------------

    fn profile_row(&self, p: usize) -> TreeRowData {
        let input = &self.inner.inputs.profiles[p];
        let mut row = TreeRowData::new(input.profile.name.clone(), TextSource::Data);
        row.icon = Some(if input.profile.engine == DbmsKind::Sqlite {
            IconName::FileCode
        } else {
            IconName::Database
        });
        row.strong = input.connected;
        row.flags = input.flags.clone();
        row.meta_mark = input.mark;
        row.meta = input.meta.clone();
        row.a11y_description = input.a11y.clone();
        row.tooltip = input.tooltip.clone();
        row
    }

    fn object_row(&self, ns: &NamespaceSnapshot, idx: u32) -> TreeRowData {
        let obj = &ns.objs()[idx as usize];
        let mut row = TreeRowData::new(rows::isolate(obj.name()), TextSource::Catalog);
        let broken = obj.flags().contains(ObjFlags::BROKEN);
        row.icon = Some(if broken {
            IconName::CircleAlert
        } else {
            rows::kind_icon(obj.kind())
        });
        row.type_text = match rows::fk_target(obj) {
            // ## fix round 1 (m12): the arrow is Fluent's, never code's.
            Some((target, columns)) => Some(self.t_args(
                "hv-explorer-fk-target",
                vec![
                    ("target", ArgValue::Text(target)),
                    ("columns", ArgValue::Text(columns)),
                ],
            )),
            None => rows::type_text(obj),
        };
        let mut words: Vec<SharedString> = Vec::new();
        if obj.flags().contains(ObjFlags::INVISIBLE_CHARS) {
            let word = self.t(Badge::Invisible.message_id());
            row.badge = Some(word.clone());
            row.badge_glyph = Some(Badge::Invisible.glyph());
            if let Some(what) = badges::first_invisible(obj.name()) {
                row.tooltip = Some(self.t_args(
                    "hv-explorer-invisible-tip",
                    vec![("what", ArgValue::Text(what))],
                ));
            }
            words.push(word);
        }
        if broken {
            let kind = match obj.kind() {
                ObjectKind::Index => "index",
                ObjectKind::ForeignKey => "foreign-key",
                ObjectKind::Check | ObjectKind::Exclusion => "check",
                ObjectKind::Trigger => "trigger",
                _ => "other",
            };
            row.tooltip = Some(self.t_args(
                "hv-explorer-broken",
                vec![
                    ("kind", ArgValue::Text(kind.to_string())),
                    ("name", ArgValue::Text(obj.name().to_string())),
                ],
            ));
        }
        row.a11y_description = join_words(&words);
        row
    }

    fn schema_row(&self, p: usize, entry: &SchemaEntry) -> TreeRowData {
        let mut row = TreeRowData::new(rows::isolate(entry.name()), TextSource::Catalog);
        row.icon = Some(rows::kind_icon(ObjectKind::Schema));
        let mysql = self.inner.layouts[p]
            .as_deref()
            .is_some_and(|l| l.level == DbLevel::SchemaIsDatabase);
        // MySQL/MariaDB call a schema a database (`model-kind-schema-mysql`).
        let kind_word = self.t_args(
            if mysql {
                "model-kind-schema-mysql"
            } else {
                "model-kind-schema"
            },
            vec![("count", ArgValue::Count(1))],
        );
        let mut words = vec![kind_word];
        if entry.out_of_scope() {
            row.dim = true;
            // ## fix round 1 (m3): dim is said, not only drawn.
            words.push(self.t("hv-explorer-out-of-scope"));
            row.tooltip = Some(self.t_args(
                "hv-explorer-out-of-scope-tip",
                vec![("name", ArgValue::Text(entry.name().to_string()))],
            ));
            row.row_action = Some(RowAction {
                icon: IconName::Plus,
                label: self.t("hv-explorer-add-scope"),
            });
        }
        if let Some(badge) = badges::namespace_badge(entry.state.as_ref()) {
            let word = self.t(badge.message_id());
            row.badge = Some(word.clone());
            row.badge_glyph = Some(badge.glyph());
            row.tooltip = Some(match badge {
                Badge::Partial => self.t("hv-explorer-partial-title"),
                _ => self.t_args(
                    "hv-explorer-stale-tip",
                    vec![("schema", ArgValue::Text(entry.name().to_string()))],
                ),
            });
            words.push(word);
        }
        // ## m4-t20-levels (named exception): the level pill (set here, inherited from the
        // database, or the level the schema was read at).
        if !entry.out_of_scope()
            && let Some(data) = &self.inner.inputs.profiles[p].userdata
            && let Some((level, source)) =
                level_pill::schema_level(data, &entry.key, entry.ns.as_deref())
        {
            row.pill = Some(self.pill(level, &source));
        }
        row.a11y_description = join_words(&words);
        row
    }

    fn database_row(&self, p: usize, db: &DbEntry) -> TreeRowData {
        let mut row = TreeRowData::new(rows::isolate(&db.name), TextSource::Catalog);
        row.icon = Some(rows::kind_icon(ObjectKind::Database));
        let mut words =
            vec![self.t_args("model-kind-database", vec![("count", ArgValue::Count(1))])];
        if !db.in_scope {
            row.dim = true;
            words.push(self.t("hv-explorer-out-of-scope"));
            row.tooltip = Some(self.t_args(
                "hv-explorer-out-of-scope-db-tip",
                vec![("name", ArgValue::Text(db.name.to_string()))],
            ));
            row.row_action = Some(RowAction {
                icon: IconName::Plus,
                label: self.t("hv-explorer-add-scope"),
            });
        } else if db.scope.0 < db.scope.1 {
            let args = || {
                vec![
                    ("n", ArgValue::Count(db.scope.0 as u64)),
                    ("total", ArgValue::Count(db.scope.1 as u64)),
                ]
            };
            row.meta = Some(self.t_args("hv-explorer-scope-count", args()));
            row.tooltip = Some(self.t_args("hv-explorer-scope-count-tip", args()));
        }
        // ## m4-t20-levels (named exception): the level pill, when set on the database.
        if db.in_scope
            && let Some(data) = &self.inner.inputs.profiles[p].userdata
            && let Some((level, source)) = level_pill::database_level(data, &db.name)
        {
            row.pill = Some(self.pill(level, &source));
        }
        row.a11y_description = join_words(&words);
        row
    }

    // ## m4-t20-levels (named exception)
    /// A level pill, its words from Fluent.
    fn pill(&self, level: Level, source: &level_pill::PillSource) -> hacivat_ui::tree::RowPill {
        level_pill::row_pill(level, source, &|id, args| self.t_args(id, args))
    }

    fn group_row(&self, key: TreeKey, group: DisplayGroup, members: usize) -> TreeRowData {
        let mut row = TreeRowData::new(self.t(group.message_id()), TextSource::Fluent);
        row.group_header = true;
        row.meta = Some(match &self.inner.inputs.filter {
            None => self.count_text(members),
            Some(_) => {
                let shown = match self.children(key) {
                    ChildState::Loaded(k) => k.len(),
                    _ => 0,
                };
                self.t_args(
                    "hv-explorer-filter-group-count",
                    vec![
                        ("n", ArgValue::Count(shown as u64)),
                        ("total", ArgValue::Count(members as u64)),
                    ],
                )
            }
        });
        row
    }

    fn env_title(&self, tag: Option<EnvTag>) -> SharedString {
        self.t(match tag {
            Some(EnvTag::Production) => "hv-explorer-group-production",
            Some(EnvTag::Staging) => "hv-explorer-group-staging",
            Some(EnvTag::Test) => "hv-explorer-group-test",
            Some(EnvTag::Dev) => "hv-explorer-group-dev",
            Some(EnvTag::Local) => "hv-explorer-group-local",
            None => "hv-explorer-group-unset",
        })
    }
}

/// The accessible description: the state words, comma-separated (the profile roots' form,
/// `connections::explorer`; fix round 1, m3).
pub(crate) fn join_words(words: &[SharedString]) -> Option<SharedString> {
    if words.is_empty() {
        return None;
    }
    Some(
        words
            .iter()
            .map(|w| w.as_ref())
            .collect::<Vec<_>>()
            .join(", ")
            .into(),
    )
}

fn is_relation(kind: ObjectKind) -> bool {
    matches!(
        kind,
        ObjectKind::Table
            | ObjectKind::View
            | ObjectKind::MaterializedView
            | ObjectKind::ForeignTable
            | ObjectKind::VirtualTable
    )
}

impl TreeModel for ExplorerModel {
    fn roots(&self) -> Arc<[TreeKey]> {
        self.inner.roots.clone()
    }

    fn row(&self, key: TreeKey) -> Option<TreeRowData> {
        let node = self.inner.nodes.borrow().get(&key).cloned()?;
        Some(match node {
            Node::Env { tag, count } => {
                let mut row = TreeRowData::new(self.env_title(tag), TextSource::Fluent);
                row.group_header = true;
                row.meta = Some(self.count_text(count));
                row
            }
            Node::Profile(p) => self.profile_row(p),
            // ## m4-t20-levels (named exception): the profile, for the level pill.
            Node::Database { p, db } => self.database_row(p, &db),
            Node::Schema { p, entry } => self.schema_row(p, &entry),
            Node::BuiltIn { .. } => {
                let mut row = TreeRowData::new(self.t("hv-explorer-builtin"), TextSource::Fluent);
                row.icon = Some(IconName::Library);
                row
            }
            Node::Group { group, members, .. } => self.group_row(key, group, members.len()),
            Node::Object { ns, idx, .. } => self.object_row(&ns, idx),
            Node::Contributed { node, .. } => node.row.clone(),
        })
    }

    fn children(&self, key: TreeKey) -> ChildState {
        if let Some(state) = self.inner.kids.borrow().get(&key) {
            return state.clone();
        }
        let Some(node) = self.inner.nodes.borrow().get(&key).cloned() else {
            // A key this generation never showed: nothing is claimed about it.
            return ChildState::NotLoaded;
        };
        let state = self.compute(key, &node);
        self.inner.kids.borrow_mut().insert(key, state.clone());
        state
    }

    fn generation(&self) -> u64 {
        self.generation
    }

    fn search_text(&self, key: TreeKey) -> Option<SharedString> {
        // Type-ahead matches the stored name, without the isolation marks.
        let node = self.inner.nodes.borrow().get(&key).cloned()?;
        match node {
            Node::Object { ns, idx, .. } => Some(ns.objs()[idx as usize].name().to_string().into()),
            Node::Schema { entry, .. } => Some(entry.name().to_string().into()),
            Node::Database { db, .. } => Some(db.name.to_string().into()),
            Node::Profile(p) => Some(self.inner.inputs.profiles[p].profile.name.clone().into()),
            _ => self.row(key).map(|r| r.label),
        }
    }
}

// ---- filter -----------------------------------------------------------------------------------

/// The filter reveals (expands the rows above) at most this many matches; the others are kept
/// (shown when their ancestors are opened) with their group's "n of total" (fix round 1, I3:
/// applying a broad result stays within a frame).
pub const REVEAL_HITS: usize = 200;

/// A job checks its cancel token after this many names.
const CANCEL_EVERY: usize = 1_024;

/// What the filter found: the matching rows and every row above one.
#[derive(Debug, Clone, Default)]
pub struct FilterResult {
    pub query: String,
    pub tolerant: bool,
    /// The rows whose name (or a profile's host) matches.
    pub hits: KeySet,
    /// The rows above the first [`REVEAL_HITS`] matches (expanded while the filter is on).
    pub ancestors: KeySet,
    /// The rows the tree shows: every match and every row above one. When nothing matched,
    /// also the rows whose catalog the filter could not search, with the rows above them, so
    /// an unsearched catalog is never shown as "no match" (fail-closed, fix round 1, I2); with
    /// matches the filter narrows as M2a's did.
    pub keep: KeySet,
    /// How many rows match.
    pub count: usize,
    /// Relations whose children are not loaded (`Names` level): their columns cannot match.
    pub unloaded: usize,
    /// Connections, databases and schemas with no loaded catalog (none, generation 0, or only
    /// a load state): their names cannot match.
    pub unsearched: usize,
}

impl FilterResult {
    /// Whether "No objects match" is a fact: nothing matched and everything was searched.
    pub fn nothing_matches(&self) -> bool {
        self.count == 0 && self.unloaded == 0 && self.unsearched == 0
    }
}

struct FilterProfile {
    id: ProfileId,
    name: String,
    host: String,
    layout: Option<Arc<Layout>>,
}

/// One contributed node the filter can match: its key, text and the rows above it.
struct FilterContributed {
    key: TreeKey,
    text: String,
    chain: Vec<TreeKey>,
}

fn collect_contributed(
    node: &ContributedNode,
    chain: &[TreeKey],
    inputs: &ExplorerInputs,
    out: &mut Vec<FilterContributed>,
) {
    if let Some(text) = &node.filter_text {
        out.push(FilterContributed {
            key: node.key,
            text: text.clone(),
            chain: chain.to_vec(),
        });
    }
    if let Some(load) = inputs.contributed_children.get(&node.key) {
        let mut below = chain.to_vec();
        below.push(node.key);
        for n in &load.nodes {
            collect_contributed(n, &below, inputs, out);
        }
    }
}

/// The filter's work, made on the UI thread from one model generation and run anywhere
/// (it holds `Arc`s of immutable snapshots only).
pub struct FilterJob {
    query: String,
    tolerant: bool,
    profiles: Vec<FilterProfile>,
    contributed: Vec<FilterContributed>,
    /// The newest job's number and this job's: a job that is no longer the newest stops.
    cancel: Option<(Arc<AtomicU64>, u64)>,
}

impl fmt::Debug for FilterJob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilterJob")
            .field("query", &self.query)
            .field("tolerant", &self.tolerant)
            .field("names", &self.names())
            .finish_non_exhaustive()
    }
}

/// Accumulates matches.
struct Found<'a> {
    r: FilterResult,
    opts: SearchOpts,
    query: &'a PreparedQuery,
    buf: Vec<u8>,
    reveal_left: usize,
    cancel: Option<(&'a AtomicU64, u64)>,
    ticks: usize,
    stopped: bool,
    /// Rows kept only when nothing matches (unsearched catalogs and the rows above them).
    fallback: Vec<TreeKey>,
}

impl Found<'_> {
    fn matches(&mut self, text: &str) -> bool {
        self.ticks += 1;
        if self.ticks.is_multiple_of(CANCEL_EVERY)
            && let Some((latest, mine)) = self.cancel
            && latest.load(Ordering::Relaxed) != mine
        {
            self.stopped = true;
        }
        !self.stopped && self.query.matches(&build_key(text, &self.opts)).is_some()
    }

    /// Whether the next match is revealed (its ancestors expanded).
    fn reveal(&mut self) -> bool {
        let r = self.reveal_left > 0;
        self.reveal_left = self.reveal_left.saturating_sub(1);
        r
    }

    fn hit(&mut self, key: TreeKey, chain: &[TreeKey]) {
        if self.r.hits.insert(key) {
            self.r.count += 1;
        }
        self.r.keep.insert(key);
        let reveal = self.reveal();
        self.above(chain, reveal);
    }

    fn above(&mut self, chain: &[TreeKey], reveal: bool) {
        for k in chain {
            self.r.keep.insert(*k);
            if reveal {
                self.r.ancestors.insert(*k);
            }
        }
    }

    /// A row the filter could not search inside: counted, and kept (with the rows above it)
    /// if nothing matches, never hidden as "no match" (fail-closed).
    fn unsearched(&mut self, key: TreeKey, chain: &[TreeKey]) {
        self.r.unsearched += 1;
        self.fallback.push(key);
        self.fallback.extend_from_slice(chain);
    }

    /// An object of `ns` at `idx` matched: it, its group header and every row above it.
    fn object(
        &mut self,
        id: ProfileId,
        ns: &NamespaceSnapshot,
        idx: u32,
        owner: TreeKey,
        chain: &[TreeKey],
    ) {
        let own = arena_key(id, ns, idx, &mut self.buf);
        if self.r.hits.insert(own) {
            self.r.count += 1;
        }
        self.r.keep.insert(own);
        let reveal = self.reveal();
        let objs = ns.objs();
        let mut cur = idx;
        loop {
            let parent = objs[cur as usize].parent().unwrap_or(0);
            let parent_key = if parent == 0 {
                owner
            } else {
                arena_key(id, ns, parent, &mut self.buf)
            };
            if let Some(g) = DisplayGroup::of(objs[cur as usize].kind()) {
                let gk = group_key(parent_key, g);
                self.r.keep.insert(gk);
                if reveal {
                    self.r.ancestors.insert(gk);
                }
            }
            if parent == 0 {
                break;
            }
            let fresh = self.r.keep.insert(parent_key);
            // The rows above an object already seen were added with it.
            let seen = if reveal {
                !self.r.ancestors.insert(parent_key)
            } else {
                !fresh
            };
            if seen {
                return;
            }
            cur = parent;
        }
        self.above(&[owner], reveal);
        self.above(chain, reveal);
    }

    /// Every object of `ns` below its root whose name matches; `owner` is the root's row.
    fn arena(&mut self, id: ProfileId, ns: &NamespaceSnapshot, owner: TreeKey, chain: &[TreeKey]) {
        let names_only = ns.meta().level == Level::Names;
        let mut unloaded = false;
        for (idx, obj) in ns.objs().iter().enumerate().skip(1) {
            if self.stopped {
                return;
            }
            if names_only && is_relation(obj.kind()) && obj.child_count() == 0 {
                self.r.unloaded += 1;
                unloaded = true;
            }
            if self.matches(obj.name()) {
                self.object(id, ns, idx as u32, owner, chain);
            }
        }
        if unloaded {
            // Its columns could not be searched: the namespace row stays if nothing matches.
            self.fallback.push(owner);
            self.fallback.extend_from_slice(chain);
        }
    }

    fn schema(&mut self, id: ProfileId, e: &SchemaEntry, chain: &[TreeKey]) {
        let key = namespace_key(id, &e.key);
        if e.out_of_scope() {
            return;
        }
        if self.matches(e.name()) {
            self.hit(key, chain);
        }
        match &e.ns {
            // `object` adds the schema row and `chain` above each match.
            Some(ns) => self.arena(id, ns, key, chain),
            None => self.unsearched(key, chain),
        }
    }
}

impl FilterJob {
    /// Stops the job early once `latest` no longer holds `seq` (a newer query was typed).
    pub fn with_cancel(mut self, latest: Arc<AtomicU64>, seq: u64) -> FilterJob {
        self.cancel = Some((latest, seq));
        self
    }

    /// How many names the job matches (the profiles and every loaded object).
    pub fn names(&self) -> usize {
        let catalog: usize = self
            .profiles
            .iter()
            .filter_map(|p| p.layout.as_deref())
            .map(|l| {
                let schemas = |list: &[Arc<SchemaEntry>]| -> usize {
                    list.iter()
                        .map(|e| 1 + e.ns.as_deref().map_or(0, NamespaceSnapshot::len))
                        .sum()
                };
                schemas(&l.schemas)
                    + schemas(&l.system)
                    + l.server_ns.as_deref().map_or(0, NamespaceSnapshot::len)
                    + l.databases
                        .iter()
                        .map(|d| {
                            1 + schemas(&d.schemas)
                                + schemas(&d.system)
                                + d.db_ns.as_deref().map_or(0, NamespaceSnapshot::len)
                        })
                        .sum::<usize>()
            })
            .sum();
        2 * self.profiles.len() + catalog + self.contributed.len()
    }

    /// Matches every name (off the UI thread for more than [`SYNC_FILTER_NAMES`] names).
    pub fn run(&self) -> FilterResult {
        self.run_cancellable().unwrap_or_default()
    }

    /// [`FilterJob::run`], or `None` when a newer job took over while it ran.
    pub fn run_cancellable(&self) -> Option<FilterResult> {
        let opts = SearchOpts {
            turkish_tolerant: self.tolerant,
            mode: MatchMode::Contains,
        };
        let query = PreparedQuery::new(&self.query, &opts);
        let mut f = Found {
            r: FilterResult {
                query: self.query.clone(),
                tolerant: self.tolerant,
                ..FilterResult::default()
            },
            opts,
            query: &query,
            buf: Vec::new(),
            reveal_left: REVEAL_HITS,
            cancel: self.cancel.as_ref().map(|(a, s)| (a.as_ref(), *s)),
            ticks: 0,
            stopped: false,
            fallback: Vec::new(),
        };
        for p in &self.profiles {
            let root = profile_key(p.id);
            if f.matches(&p.name) || f.matches(&p.host) {
                f.hit(root, &[]);
            }
            let Some(layout) = p.layout.as_deref() else {
                // No catalog, or generation 0: not searched, so never "no match".
                f.unsearched(root, &[]);
                continue;
            };
            let chain = [root];
            if let Some(ns) = &layout.server_ns {
                f.arena(p.id, ns, root, &[]);
            }
            for e in &layout.schemas {
                f.schema(p.id, e, &chain);
            }
            for e in &layout.system {
                let chain = [root, builtin_key(p.id, None)];
                f.schema(p.id, e, &chain);
            }
            for db in &layout.databases {
                let dk = database_key(p.id, &db.name);
                if !db.in_scope {
                    continue;
                }
                if f.matches(&db.name) {
                    f.hit(dk, &chain);
                }
                if db.schemas.is_empty() && db.system.is_empty() && db.db_ns.is_none() {
                    f.unsearched(dk, &chain);
                }
                let below = [root, dk];
                for e in &db.schemas {
                    f.schema(p.id, e, &below);
                }
                let bk = builtin_key(p.id, Some(&db.name));
                for e in &db.system {
                    f.schema(p.id, e, &[root, dk, bk]);
                }
                if let Some(ns) = &db.db_ns {
                    f.arena(p.id, ns, dk, &chain);
                }
            }
        }
        for c in &self.contributed {
            if f.matches(&c.text) {
                f.hit(c.key, &c.chain);
            }
        }
        if f.stopped {
            return None;
        }
        if f.r.count == 0 {
            // Nothing matched: what could not be searched stays in sight (I2).
            let fallback = std::mem::take(&mut f.fallback);
            f.r.keep.extend(fallback);
        }
        Some(f.r)
    }
}

// ---- the catalogs the explorer shows ----------------------------------------------------------

/// The catalog snapshot each profile's tree shows (Task 24 fills it from `CatalogHub`; until
/// then nothing does, and every root shows "Not loaded").
#[derive(Default)]
pub struct ExplorerCatalogs {
    map: HashMap<ProfileId, Arc<CatalogSnapshot>>,
    /// Rename hints not yet applied to the tree's expansion: `(from, to)`.
    renames: Vec<(TreeKey, TreeKey)>,
    generation: u64,
}

impl Global for ExplorerCatalogs {}

/// Shows `catalog` under `profile`'s root (`None`: none). `delta`'s renames move the tree's
/// expansion from the old object's key to the new one (a rename hint of a name-only engine,
/// T10; an object with a server id keeps its key anyway).
pub fn set_catalog(
    profile: ProfileId,
    catalog: Option<Arc<CatalogSnapshot>>,
    delta: Option<&CatalogDelta>,
    cx: &mut App,
) {
    let g = cx.default_global::<ExplorerCatalogs>();
    match catalog {
        Some(c) => {
            g.map.insert(profile, c);
        }
        None => {
            g.map.remove(&profile);
        }
    }
    if let Some(delta) = delta {
        for (from, to, _) in &delta.renamed {
            let (a, b) = (object_tree_key(profile, from), object_tree_key(profile, to));
            if a != b {
                g.renames.push((a, b));
            }
        }
    }
    g.generation += 1;
}

/// The catalog `profile`'s tree shows.
pub fn catalog_of(profile: ProfileId, cx: &App) -> Option<Arc<CatalogSnapshot>> {
    cx.try_global::<ExplorerCatalogs>()?
        .map
        .get(&profile)
        .cloned()
}

/// The catalogs' change counter (the explorer rebuilds its model when it moves).
pub(crate) fn catalogs_generation(cx: &App) -> u64 {
    cx.try_global::<ExplorerCatalogs>()
        .map_or(0, |g| g.generation)
}

/// Takes the rename hints not yet applied.
pub(crate) fn take_renames(cx: &mut App) -> Vec<(TreeKey, TreeKey)> {
    // Only a pending hint takes the global mutably: a mutable access notifies its observers
    // (the explorer, which calls this from its observer).
    if cx
        .try_global::<ExplorerCatalogs>()
        .is_none_or(|g| g.renames.is_empty())
    {
        return Vec::new();
    }
    std::mem::take(&mut cx.global_mut::<ExplorerCatalogs>().renames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacivat_meta::name::FoldRule;
    use hacivat_meta::namespace::{NsMeta, Subtree};
    use hacivat_meta::{Obj, ServerId};

    #[test]
    fn arena_keys_equal_object_keys() {
        let mut t = Subtree::new(Obj::new(ObjectKind::Schema, "public"));
        let table = t.add(
            t.root(),
            Obj::new(ObjectKind::Table, "orders").with_server_id(ServerId::Oid(16_400)),
        );
        t.add(table, Obj::new(ObjectKind::Column, "id"));
        let ns = NamespaceSnapshot::build(
            NsKey::schema(["app", "public"]),
            DbmsKind::Postgres,
            FoldRule::Exact,
            NsMeta::default(),
            t,
        )
        .expect("a namespace");
        let p = ProfileId::new();
        let mut buf = Vec::new();
        for idx in 1..ns.len() as u32 {
            let key = ns.object_key(idx).expect("a key");
            assert_eq!(arena_key(p, &ns, idx, &mut buf), object_tree_key(p, &key));
        }
        assert_eq!(arena_key(p, &ns, 0, &mut buf), namespace_key(p, ns.key()));
        // Domain separation: a group header never equals the row it hangs from.
        let k = namespace_key(p, ns.key());
        assert_ne!(group_key(k, DisplayGroup::Tables), k);
    }
}
