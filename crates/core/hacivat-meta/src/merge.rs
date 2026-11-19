//! The existence-diff merge (HV-INTRO-11; M4 Task 10): an introspection result plus the
//! previous namespace give the next namespace and its [`CatalogDelta`]. A pure function: it
//! reads no clock, does no I/O and never changes `prev`.
//!
//! - **Identity** is [`ObjectKey::same_object`]: the server id when both sides have one, else
//!   kind, exact path and overload signature (and an unknown kind's raw kind). On engines whose
//!   name class compares ASCII-caselessly (MySQL/MariaDB table and database names under
//!   `lower_case_table_names` 1 or 2, their column, routine and event names; SQLite), a miss
//!   falls back to the folded key of `sql_ident::key::match_key` when that key is certain and no
//!   server ids conflict (R-W2-4): the server forbids case twins there, so a case-only spelling
//!   change keeps the object. A server id the cache holds twice (T3 m9) matches only by id **and**
//!   path, so a stale twin is never kept.
//! - **Deletions** come only from a successful existence set of the object's own
//!   [`ObjectKind`], for the scope it covers ([`ExistSet::Ok`] the whole namespace,
//!   [`ExistSet::Scoped`] the objects under its scope), at any depth (C-1, I-1, R-W2-5). A kind
//!   whose set is `Failed`, absent from the result, or out of the set's scope is never deleted:
//!   its cached objects are carried over, under re-read parents too, in full and incremental
//!   passes alike. Every deletion is in `delta.dropped`.
//! - **Changed objects** replace their cached counterpart's own properties; their children are
//!   merged the same way (a re-read child replaces its cached twin; a cached child the pass did
//!   not return stays unless an in-scope set of its kind decides it is gone). So a pass that read
//!   less than the cache (a lower level, a failed section) never erases what it did not read.
//! - **The level** never goes down (I-2): a full pass raises it to the pass's level, nothing else
//!   changes it. Objects a lower-level pass created or re-read are listed in
//!   [`MergeReport::below_level`]; lowering a namespace's level is the engine's job (T15).
//! - **The guard** (D-039, R-M4-22, I-4) counts top-level objects only, per kind and in total:
//!   more than `guard_ratio_pct` % of at least `guard_min_objects` decided objects vanishing is
//!   [`MergeOutcome::PendingConfirmation`], whatever the trigger. "Keep cached" re-merges the same
//!   result with [`Deletions::Suppress`] (I-5): the pass's other changes apply, the vanished
//!   objects stay and are listed in [`MergeReport::suppressed`].
//! - **Rename hints** for name-only engines (step 6): a dropped and a created top-level object of
//!   one kind with ≥ 90 % identical column signatures. Moves across namespaces are paired by
//!   [`CatalogDelta::combine`]; huge deltas are coarse ([`crate::delta::event`]).
//! - `NamespaceResult::categories_to_reload` is not read by the merge.
//!
//! Work is O(|M| + |C| + |E|) with hash maps (children are matched positionally when the pass
//! returns them in cached order, else through per-parent maps): no `find` per column, no pairwise
//! name matching (hint pairs are bounded by [`HINT_PAIR_BUDGET`]). Two fast paths run as dry
//! walks of the same algorithm and fall back to the general one when they do not apply: a full
//! pass that leaves nothing cached standing on its own reuses the changed arena; a pass that
//! changed and deleted nothing reuses the cached arena with the new metadata.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use hacivat_proto::DbmsKind;

use crate::delta::{CatalogDelta, ChangedFacets, Confidence};
use crate::ids::{NsKey, ObjectKey, ServerId, SourceRef, StateMark};
use crate::kind::ObjectKind;
use crate::name::{DialectId, FoldRule, IdentCtx, Name, NameClass, match_key};
use crate::namespace::{
    Level, ModelError, NamespaceSnapshot, NodeId, NsMeta, Obj, ObjFlags, Subtree, Watermark,
};
use crate::props::Props;
use crate::snapshot::IntroErrorCode;

/// Above this many dropped × created pairs of one kind, rename hints are looked for only among
/// identical column signatures (a hash match), so a mass rename stays linear.
pub const HINT_PAIR_BUDGET: usize = 4_096;

/// The least similarity, in thousandths, of a rename hint (step 6).
const HINT_MIN_PER_MILLE: u16 = 900;

/// One existing object as an existence query reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// The object's path inside the namespace (its own name last).
    pub path: Vec<Name>,
    pub server_id: Option<ServerId>,
    pub overload_sig: Option<Name>,
    /// The raw server kind of an [`ObjectKind::Unknown`] object (`None` for a known kind).
    pub raw_kind: Option<Name>,
    pub state: StateMark,
}

impl Identity {
    /// The identity at `path` with `server_id` (no overload signature, raw kind or state mark).
    pub fn of(path: Vec<Name>, server_id: Option<ServerId>) -> Self {
        Identity {
            path,
            server_id,
            overload_sig: None,
            raw_kind: None,
            state: StateMark::None,
        }
    }
}

/// The objects of one kind that exist, or why the existence query failed. Only `Ok` and
/// `Scoped` sets delete, and only inside what they cover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExistSet {
    /// Every object of the kind in the namespace: complete for the whole namespace.
    Ok(Vec<Identity>),
    /// Complete only within `scope` (R-W2-5): it decides a cached object of its kind when the
    /// object's own namespace-relative path, or one of its ancestors', is in `scope`. A fragment
    /// of table `t` reports `Table: Scoped { scope: [["t"]], .. }`; the columns of re-read
    /// tables `a` and `b` are `Column: Scoped { scope: [["a"], ["b"]], .. }`.
    Scoped {
        scope: Vec<Vec<Name>>,
        ids: Vec<Identity>,
    },
    /// The query failed: nothing of the kind is deleted, and the kind is stale.
    Failed(IntroErrorCode),
}

/// A changed (or new) object re-read with its subtree.
#[derive(Debug, Clone)]
pub struct ChangedObject {
    /// The key of the object's parent in the namespace.
    pub parent: ObjectKey,
    pub subtree: Subtree,
}

/// A category of parent-scoped rows a pass re-read for a whole kind (section cascade,
/// R-M4-48).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    Columns,
    Keys,
    ForeignKeys,
    Indexes,
    Checks,
    Triggers,
    Comments,
    Sources,
}

/// What one introspection pass found in one namespace.
#[derive(Debug, Clone)]
pub struct NamespaceResult {
    /// Existence sets by kind. A kind missing here is never deleted; a pass that wants its
    /// re-read objects' dropped children (a dropped column, a dropped index) removed reports a
    /// set of the child kind, scoped to the re-read parents ([`ExistSet::Scoped`]).
    pub existent: BTreeMap<ObjectKind, ExistSet>,
    pub changed: Vec<ChangedObject>,
    pub categories_to_reload: BTreeSet<Category>,
    pub new_watermark: Watermark,
    /// A full pass (every object re-read), not an incremental one. It may raise the namespace's
    /// level; it deletes nothing more than an incremental pass would.
    pub full: bool,
    /// The level the pass read at (M4 Task 10). A full pass raises the namespace's level to it;
    /// no pass lowers it.
    pub level: Level,
    /// When the pass read the catalog, milliseconds since the Unix epoch (display only; M4
    /// Task 10). The merge is pure and reads no clock.
    pub read_at_ms: i64,
}

impl NamespaceResult {
    /// A full pass that found nothing.
    pub fn empty_full() -> Self {
        NamespaceResult {
            existent: BTreeMap::new(),
            changed: Vec::new(),
            categories_to_reload: BTreeSet::new(),
            new_watermark: Watermark::None,
            full: true,
            level: Level::Names,
            read_at_ms: 0,
        }
    }
}

/// Who asked for the pass: a user refresh asks before a mass deletion; a background pass marks
/// the namespace stale and keeps the result pending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Trigger {
    User,
    #[default]
    Background,
}

/// What the merge does with objects an existence set says are gone (I-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Deletions {
    /// Remove them (behind the guard).
    #[default]
    Apply,
    /// Keep them (the user chose "Keep cached"): they are listed in
    /// [`MergeReport::suppressed`], the rest of the pass applies, and the guard does not fire.
    Suppress,
}

/// The mass-deletion guard (D-039) and the deletion mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergePolicy {
    pub trigger: Trigger,
    pub guard_ratio_pct: u8,
    pub guard_min_objects: u32,
    pub deletions: Deletions,
}

impl Default for MergePolicy {
    /// A background pass with D-039's thresholds (50 %, 20 objects) that applies deletions.
    fn default() -> Self {
        MergePolicy {
            trigger: Trigger::Background,
            guard_ratio_pct: 50,
            guard_min_objects: 20,
            deletions: Deletions::Apply,
        }
    }
}

/// The guard's count of one top-level kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KindCount {
    /// Cached top-level objects of the kind whose existence the pass decided.
    pub checked: u32,
    /// Of those, how many vanished.
    pub vanished: u32,
}

/// What the merge found besides the next namespace and its delta.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MergeReport {
    /// Kinds whose existence query failed: none of their cached objects was deleted, and they
    /// are stale until a pass reads them (step 2; edge 1).
    pub stale_kinds: BTreeMap<ObjectKind, IntroErrorCode>,
    /// Objects to re-read with a fragment pass, sorted: the parent of a changed child the merge
    /// could not place (edge 9), an object the existence query lists that neither the cache nor
    /// the pass holds (edge 11), a cached object the existence query lists under another name
    /// that the pass did not re-read. The merge never invents them.
    pub fragments: Vec<ObjectKey>,
    /// Changed objects ignored because an in-scope existence set of their kind does not list
    /// them (edge 10).
    pub ignored: u32,
    /// Changed objects that repeated an earlier one's identity; the last one won (edge 12).
    pub duplicates: u32,
    /// Top-level cached objects whose existence this pass decided, all kinds (the guard's
    /// population; nested objects never count, I-4).
    pub checked: u32,
    /// Of those, how many vanished.
    pub vanished: u32,
    /// The same counts per top-level kind.
    pub by_kind: BTreeMap<ObjectKind, KindCount>,
    /// Objects an existence set says are gone that [`Deletions::Suppress`] kept.
    pub suppressed: Vec<ObjectKey>,
    /// Top-level objects a pass below the namespace's level created or re-read: they hold only
    /// what that pass read (the namespace keeps its level; I-2).
    pub below_level: Vec<ObjectKey>,
    /// Objects, identities and pairs the merge visited: O(|M| + |C| + |E|); tests use it as a
    /// linearity witness.
    pub steps: u64,
}

/// The merge's answer. Both variants carry the same computed `next`; the merge never changes
/// `prev` (it cannot: `prev` is borrowed).
#[derive(Debug, Clone)]
pub enum MergeOutcome {
    Applied {
        next: NamespaceSnapshot,
        delta: CatalogDelta,
        report: MergeReport,
    },
    /// The guard fired: nothing is applied until the user confirms. The engine keeps the
    /// result pending and marks the namespace stale; "Remove from cache" publishes `next`,
    /// "Keep cached" re-merges the result with [`Deletions::Suppress`] (R-M4-22).
    PendingConfirmation {
        next: NamespaceSnapshot,
        delta: CatalogDelta,
        vanished: u32,
        report: MergeReport,
    },
}

impl MergeOutcome {
    /// The namespace the merge computed (applied or pending).
    pub fn next(&self) -> &NamespaceSnapshot {
        match self {
            MergeOutcome::Applied { next, .. } | MergeOutcome::PendingConfirmation { next, .. } => {
                next
            }
        }
    }

    pub fn delta(&self) -> &CatalogDelta {
        match self {
            MergeOutcome::Applied { delta, .. }
            | MergeOutcome::PendingConfirmation { delta, .. } => delta,
        }
    }

    pub fn report(&self) -> &MergeReport {
        match self {
            MergeOutcome::Applied { report, .. }
            | MergeOutcome::PendingConfirmation { report, .. } => report,
        }
    }

    /// Whether the mass-deletion guard fired.
    pub fn is_pending(&self) -> bool {
        matches!(self, MergeOutcome::PendingConfirmation { .. })
    }

    /// The parts, whichever the variant.
    pub fn into_parts(self) -> (NamespaceSnapshot, CatalogDelta, MergeReport) {
        match self {
            MergeOutcome::Applied {
                next,
                delta,
                report,
            }
            | MergeOutcome::PendingConfirmation {
                next,
                delta,
                report,
                ..
            } => (next, delta, report),
        }
    }
}

/// Why a merge failed (nothing is published).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeError {
    /// The result is for another namespace than `prev`.
    WrongNamespace,
    /// No previous namespace: a result carries no engine or fold rule, so a first load merges
    /// into [`base`] (M4 Task 10).
    NoBase,
    /// The merged arena is invalid.
    Model(crate::namespace::ModelError),
}

impl fmt::Display for MergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MergeError::WrongNamespace => write!(f, "result for another namespace"),
            MergeError::NoBase => write!(f, "no previous namespace to merge into"),
            MergeError::Model(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for MergeError {}

/// The empty namespace `key` (its root only) that a first load merges into.
pub fn base(key: NsKey, dbms: DbmsKind, fold: FoldRule) -> Result<NamespaceSnapshot, ModelError> {
    let root = Obj::new(key.kind.root_kind(), key.root_name());
    NamespaceSnapshot::build(key, dbms, fold, NsMeta::default(), Subtree::new(root))
}

/// Merges `r` into `prev` (alg/02 HV-INTRO-11 steps 1–10; see the module docs). `prev = None`
/// is refused with [`MergeError::NoBase`]: a first load merges into [`base`].
pub fn merge(
    prev: Option<&NamespaceSnapshot>,
    r: NamespaceResult,
    policy: &MergePolicy,
) -> Result<MergeOutcome, MergeError> {
    let prev = prev.ok_or(MergeError::NoBase)?;
    Merger::run(prev, r, policy)
}

/// Marks a server id two siblings share.
const AMBIGUOUS: u32 = u32::MAX;

/// Marks a folded key two existence identities share.
const AMBIGUOUS_ID: usize = usize::MAX;

/// Flags derived from the name; a rename changes them, not the definition.
const NAME_FLAGS: ObjFlags = ObjFlags::INVISIBLE_CHARS.union(ObjFlags::LOSSY_NAME);

/// A namespace-relative path, an overload signature and a raw kind: one existence identity.
type PathKey = (Vec<Name>, Option<Name>, Option<Name>);

/// A sibling's kind, name, overload signature and raw kind.
type SibKey = (ObjectKind, Name, Option<Name>, Option<Name>);

/// The folded-key fallback of identity on caseless engines (R-W2-4).
struct Folder {
    dialect: DialectId,
    ctx: IdentCtx,
    dbms: DbmsKind,
}

impl Folder {
    /// The fallback of `ns`'s engine: MySQL, MariaDB (by the namespace's stored table rule) and
    /// SQLite; none elsewhere (PostgreSQL names are exact: case twins exist there).
    fn of(ns: &NamespaceSnapshot) -> Option<Self> {
        let dialect = match ns.dbms() {
            DbmsKind::MySql => DialectId::MySql,
            DbmsKind::MariaDb => DialectId::MariaDb,
            DbmsKind::Sqlite => DialectId::Sqlite,
            _ => return None,
        };
        // `lower_case_table_names` 1 and 2 share every rule; 0 is the exact table rule.
        let ctx = IdentCtx {
            lower_case_table_names: u8::from(ns.fold() == FoldRule::AsciiCaseless),
            ..IdentCtx::new(dialect)
        };
        Some(Folder {
            dialect,
            ctx,
            dbms: ns.dbms(),
        })
    }

    /// The certain folded key of `name` of `class`: `None` when the class compares exactly or
    /// the server's rule for the text is not pinned (`ServerDependent`).
    fn key(&self, class: NameClass, name: &str) -> Option<Name> {
        if FoldRule::for_names(self.dialect, &self.ctx, class) != Some(FoldRule::AsciiCaseless) {
            return None;
        }
        let key = match_key(self.dialect, &self.ctx, name, true, class).ok()?;
        Some(Name::from(key.as_str()))
    }

    fn name(&self, kind: ObjectKind, name: &str) -> Option<Name> {
        self.key(kind.name_class(self.dbms), name)
    }

    /// The folded relative path of an object of `kind`: its own name by its class, ancestors by
    /// the table class (the parents of nested objects are relations), kept exact where that
    /// class compares exactly.
    fn path(&self, kind: ObjectKind, path: &[Name]) -> Option<Vec<Name>> {
        let (last, ancestors) = path.split_last()?;
        let mut out = Vec::with_capacity(path.len());
        for a in ancestors {
            out.push(match self.key(NameClass::Table, a) {
                Some(k) => k,
                None if FoldRule::for_names(self.dialect, &self.ctx, NameClass::Table)
                    == Some(FoldRule::Exact) =>
                {
                    a.clone()
                }
                None => return None,
            });
        }
        out.push(self.name(kind, last)?);
        Some(out)
    }
}

/// One kind's successful existence set, indexed.
struct ExistIndex {
    kind: ObjectKind,
    ids: Vec<Identity>,
    by_sid: HashMap<ServerId, usize>,
    by_path: HashMap<PathKey, usize>,
    by_fold: HashMap<PathKey, usize>,
    /// `None`: the whole namespace.
    scope: Option<HashSet<Vec<Name>>>,
    /// Identities some cached or changed object answered for (the rest are edge 11).
    seen: Vec<bool>,
}

impl ExistIndex {
    fn new(
        kind: ObjectKind,
        ids: Vec<Identity>,
        scope: Option<Vec<Vec<Name>>>,
        folder: Option<&Folder>,
    ) -> Self {
        let mut by_sid = HashMap::with_capacity(ids.len());
        let mut by_path = HashMap::with_capacity(ids.len());
        let mut by_fold = HashMap::new();
        for (k, id) in ids.iter().enumerate() {
            if let Some(sid) = id.server_id {
                by_sid.insert(sid, k);
            }
            let tail = (id.overload_sig.clone(), id.raw_kind.clone());
            if let Some(f) = folder
                && let Some(p) = f.path(kind, &id.path)
            {
                by_fold
                    .entry((p, tail.0.clone(), tail.1.clone()))
                    .and_modify(|v| *v = AMBIGUOUS_ID)
                    .or_insert(k);
            }
            by_path.insert((id.path.clone(), tail.0, tail.1), k);
        }
        let seen = vec![false; ids.len()];
        ExistIndex {
            kind,
            ids,
            by_sid,
            by_path,
            by_fold,
            scope: scope.map(|s| s.into_iter().collect()),
            seen,
        }
    }

    /// Whether the set decides an object at the relative `path`.
    fn in_scope(&self, path: &[Name]) -> bool {
        match &self.scope {
            None => true,
            Some(s) => (1..=path.len()).any(|k| s.contains(&path[..k])),
        }
    }

    /// The identity that is the same object as one with `sid` at `key` ([`ObjectKey::same_object`],
    /// then the folded fallback); an `ambiguous` server id matches only together with its path.
    fn find(
        &self,
        sid: Option<ServerId>,
        key: &PathKey,
        ambiguous: bool,
        folder: Option<&Folder>,
    ) -> Option<usize> {
        if let Some(s) = sid
            && !ambiguous
            && let Some(&k) = self.by_sid.get(&s)
            && self.ids[k].raw_kind == key.2
        {
            return Some(k);
        }
        let compatible = |k: usize| match (self.ids[k].server_id, sid) {
            (Some(a), Some(b)) if a != b => None,
            _ => Some(k),
        };
        if let Some(&k) = self.by_path.get(key) {
            return compatible(k);
        }
        let folded = folder?.path(self.kind, &key.0)?;
        match self.by_fold.get(&(folded, key.1.clone(), key.2.clone())) {
            Some(&k) if k != AMBIGUOUS_ID => compatible(k),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Rule {
    /// Deletions from this successful set (an index into `Merger::sets`).
    Exist(usize),
    /// The existence query failed: keep, stale.
    Failed,
}

/// A sibling's identity under one parent, for duplicates in the changed set (edge 12).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum SiblingId {
    Sid(ObjectKind, ServerId),
    Name(SibKey),
}

impl SiblingId {
    fn of(o: &Obj) -> Self {
        match o.server_id() {
            Some(s) => SiblingId::Sid(o.kind(), s),
            None => SiblingId::Name(sib_key(o, o.name().clone())),
        }
    }
}

fn sib_key(o: &Obj, name: Name) -> SibKey {
    (
        o.kind(),
        name,
        o.props().overload_sig().cloned(),
        o.raw_kind().cloned(),
    )
}

/// The children of one cached parent, indexed by server id, by name and by folded name.
struct Siblings {
    by_sid: HashMap<(ObjectKind, ServerId), u32>,
    by_name: HashMap<SibKey, u32>,
    by_fold: HashMap<SibKey, u32>,
}

impl Siblings {
    fn of(ns: &NamespaceSnapshot, parent: u32, folder: Option<&Folder>, steps: &mut u64) -> Self {
        let mut by_sid = HashMap::new();
        let mut by_name = HashMap::new();
        let mut by_fold = HashMap::new();
        for (i, o) in ns.children(parent) {
            *steps += 1;
            if let Some(s) = o.server_id() {
                by_sid
                    .entry((o.kind(), s))
                    .and_modify(|v| *v = AMBIGUOUS)
                    .or_insert(i);
            }
            if let Some(f) = folder
                && let Some(k) = f.name(o.kind(), o.name())
            {
                by_fold
                    .entry(sib_key(o, k))
                    .and_modify(|v| *v = AMBIGUOUS)
                    .or_insert(i);
            }
            by_name.entry(sib_key(o, o.name().clone())).or_insert(i);
        }
        Siblings {
            by_sid,
            by_name,
            by_fold,
        }
    }

    /// The sibling that is the same object as `o` ([`ObjectKey::same_object`] among siblings,
    /// then the folded fallback).
    fn find(&self, ns: &NamespaceSnapshot, o: &Obj, folder: Option<&Folder>) -> Option<u32> {
        if let Some(s) = o.server_id()
            && let Some(&i) = self.by_sid.get(&(o.kind(), s))
            && i != AMBIGUOUS
            && ns.objs()[i as usize].raw_kind() == o.raw_kind()
        {
            return Some(i);
        }
        let compatible = |i: u32| match (ns.objs()[i as usize].server_id(), o.server_id()) {
            (Some(a), Some(b)) if a != b => None,
            _ => Some(i),
        };
        if let Some(&i) = self.by_name.get(&sib_key(o, o.name().clone())) {
            return compatible(i);
        }
        let folded = folder?.name(o.kind(), o.name())?;
        match self.by_fold.get(&sib_key(o, folded)) {
            Some(&i) if i != AMBIGUOUS => compatible(i),
            _ => None,
        }
    }
}

/// Whether two siblings are the same object spelled the same way (the positional fast path).
fn same_sibling(a: &Obj, b: &Obj) -> bool {
    a.kind() == b.kind()
        && a.server_id() == b.server_id()
        && a.name() == b.name()
        && a.raw_kind() == b.raw_kind()
        && a.props().overload_sig() == b.props().overload_sig()
}

/// A changed object (an index into the changed arena) and the cached sibling it replaces.
#[derive(Debug, Clone, Copy)]
struct Slot {
    ci: u32,
    mi: Option<u32>,
}

/// One column's signature for rename hints: name, raw type, nullability.
type ColSig = (Name, Name, bool);

/// What an existence set says about a cached object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    /// No set of its kind covers it (none, `Failed`, out of scope): it stays.
    Undecided,
    Present,
    Absent,
}

/// A node of the merge walk.
#[derive(Debug, Clone, Copy)]
enum Node {
    /// The namespace root: cached top level in arena order, then the top-level creations.
    Root,
    /// A cached object the pass did not re-read (its children may still change).
    Kept(u32),
    /// A cached object and its re-read version.
    Pair(u32, u32),
}

/// A dry walk (no output) that takes a fast path when it applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dry {
    /// A full pass: the changed arena becomes the next namespace unless a cached object must
    /// stay on its own (or the pass holds an object the merge would not place).
    Reuse,
    /// No changed objects: the cached arena stays unless something is deleted.
    Unchanged,
}

/// The fast path does not apply; run the general walk.
struct Abort;

struct Merger<'a> {
    /// The cached namespace (`M`).
    m: &'a NamespaceSnapshot,
    /// The changed objects (`C`) as one arena: the root, the top-level changes, then one carrier
    /// chain (copies of a cached parent's ancestors) per cached parent with changed children.
    c: NamespaceSnapshot,
    folder: Option<Folder>,
    rules: Vec<(ObjectKind, Rule)>,
    sets: Vec<ExistIndex>,
    /// `(kind, server id)`s the cache holds more than once.
    ambiguous: HashSet<(ObjectKind, ServerId)>,
    /// Cached parent → its carrier leaf in `c`.
    carriers: HashMap<u32, u32>,
    /// The top-level changed objects (the root's children in `c`, carriers excluded).
    c_top: Vec<u32>,
    suppress: bool,
    /// The pass reads less than the namespace holds (I-2).
    below: bool,
    dry: Option<Dry>,
    /// The relative path of the node being walked, in the **next** namespace's names (a re-read
    /// parent's new name), so its children's existence identities are looked up where the pass
    /// reports them.
    stack: Vec<Name>,
    dropped_top: Vec<u32>,
    created_top: Vec<u32>,
    fragments: BTreeSet<ObjectKey>,
    delta: CatalogDelta,
    report: MergeReport,
}

impl<'a> Merger<'a> {
    fn run(
        m: &'a NamespaceSnapshot,
        r: NamespaceResult,
        policy: &MergePolicy,
    ) -> Result<MergeOutcome, MergeError> {
        let NamespaceResult {
            existent,
            changed,
            categories_to_reload: _,
            new_watermark,
            full,
            level,
            read_at_ms,
        } = r;
        let key = m.key();
        let root_kind = key.kind.root_kind();
        let mut report = MergeReport::default();
        let mut fragments = BTreeSet::new();
        let nothing_changed = changed.is_empty();

        // Sort the changed objects: the namespace root, top-level objects, children of cached
        // parents (edge 9: a parent neither cached nor re-read is re-read by a fragment).
        let mut root_update = None;
        let mut top = Vec::new();
        let mut groups: Vec<(u32, Vec<Subtree>)> = Vec::new();
        let mut group_of: HashMap<u32, usize> = HashMap::new();
        for ch in changed {
            report.steps += 1;
            let o = ch.subtree.root_obj();
            if o.kind() == root_kind {
                if &**o.name() != key.root_name() {
                    return Err(MergeError::WrongNamespace);
                }
                if root_update.replace(ch.subtree).is_some() {
                    report.duplicates += 1;
                }
                continue;
            }
            if !ch.parent.path.starts_with(&key.path) {
                return Err(MergeError::WrongNamespace);
            }
            if ch.parent.path.len() == key.path.len() {
                if ch.parent.kind != root_kind {
                    return Err(MergeError::WrongNamespace);
                }
                top.push(ch.subtree);
                continue;
            }
            match m.find(&ch.parent) {
                Some(p) if p != 0 => {
                    let g = *group_of.entry(p).or_insert_with(|| {
                        groups.push((p, Vec::new()));
                        groups.len() - 1
                    });
                    groups[g].1.push(ch.subtree);
                }
                _ => {
                    fragments.insert(ch.parent);
                }
            }
        }

        let had_root_update = root_update.is_some();
        let mut ct = root_update.unwrap_or_else(|| Subtree::new(m.root().clone()));
        let croot = ct.root();
        for s in top {
            ct.graft_under(croot, s);
        }
        let mut chains = Vec::with_capacity(groups.len());
        for (p, subs) in groups {
            let mut path = vec![p];
            let mut cur = p;
            while let Some(up) = m.objs()[cur as usize].parent() {
                if up == 0 {
                    break;
                }
                path.push(up);
                cur = up;
            }
            path.reverse();
            let mut chain = Subtree::new(m.objs()[path[0] as usize].clone());
            let mut node = chain.root();
            for &a in &path[1..] {
                node = chain.add(node, m.objs()[a as usize].clone());
            }
            for s in subs {
                chain.graft_under(node, s);
            }
            ct.graft_under(croot, chain);
            chains.push((p, path.len()));
        }
        let prev_level = m.meta().level;
        let meta = NsMeta {
            level: if full {
                level.max(prev_level)
            } else {
                prev_level
            },
            watermark: new_watermark,
            loaded_at_ms: read_at_ms,
        };
        let c = NamespaceSnapshot::build(key.clone(), m.dbms(), m.fold(), meta.clone(), ct)
            .map_err(MergeError::Model)?;

        let kids: Vec<u32> = c.children(0).map(|(i, _)| i).collect();
        let real = kids.len() - chains.len();
        let mut carriers = HashMap::with_capacity(chains.len());
        for (j, &(p, depth)) in chains.iter().enumerate() {
            let mut leaf = kids[real + j];
            for _ in 1..depth {
                leaf = c
                    .children(leaf)
                    .next()
                    .map(|(i, _)| i)
                    .ok_or(MergeError::Model(ModelError::BadLayout("carrier chain")))?;
            }
            carriers.insert(p, leaf);
        }
        let c_top = kids[..real].to_vec();

        // Existence sets by kind (steps 1, 2).
        let folder = Folder::of(m);
        let mut rules = Vec::with_capacity(existent.len());
        let mut sets = Vec::new();
        for (kind, set) in existent {
            let (ids, scope) = match set {
                ExistSet::Ok(ids) => (ids, None),
                ExistSet::Scoped { scope, ids } => (ids, Some(scope)),
                ExistSet::Failed(code) => {
                    rules.push((kind, Rule::Failed));
                    report.stale_kinds.insert(kind, code);
                    continue;
                }
            };
            report.steps += ids.len() as u64;
            rules.push((kind, Rule::Exist(sets.len())));
            sets.push(ExistIndex::new(kind, ids, scope, folder.as_ref()));
        }
        let mut ambiguous = HashSet::new();
        if m.duplicate_server_ids() > 0 {
            let mut once = HashSet::new();
            for o in m.objs() {
                if let Some(s) = o.server_id()
                    && !once.insert((o.kind(), s))
                {
                    ambiguous.insert((o.kind(), s));
                }
            }
        }

        let suppress = policy.deletions == Deletions::Suppress;
        let any_failed = rules.iter().any(|(_, r)| matches!(r, Rule::Failed));
        let mut mg = Merger {
            m,
            c,
            folder,
            rules,
            sets,
            ambiguous,
            carriers,
            c_top,
            suppress,
            below: level < prev_level,
            dry: None,
            stack: Vec::new(),
            dropped_top: Vec::new(),
            created_top: Vec::new(),
            fragments,
            delta: CatalogDelta::default(),
            report,
        };

        // Steps 3–8: one walk from the root; a fast path first when one may apply.
        let fast = if nothing_changed {
            Some(Dry::Unchanged)
        } else if full && !any_failed && !suppress && mg.carriers.is_empty() {
            Some(Dry::Reuse)
        } else {
            None
        };
        let mut taken = None;
        if let Some(dry) = fast {
            let saved = (mg.report.clone(), mg.fragments.clone());
            mg.dry = Some(dry);
            if mg.node(None, None, Node::Root).is_ok() {
                taken = Some(dry);
            } else {
                (mg.report, mg.fragments) = saved;
                mg.delta = CatalogDelta::default();
                mg.dropped_top.clear();
                mg.stack.clear();
                mg.created_top.clear();
            }
            mg.dry = None;
        }
        let mut out = taken
            .is_none()
            .then(|| Subtree::with_capacity(mg.c.root().clone(), m.len() + mg.c.len()));
        if let Some(out) = out.as_mut() {
            // The general walk never aborts (only dry walks do).
            let _ = mg.node(Some(out), None, Node::Root);
        }
        if had_root_update {
            let f = facets(m.root(), mg.c.root());
            if f.any()
                && let Some(k) = m.object_key(0)
            {
                mg.delta.altered.push((k, f));
            }
        }
        mg.hints();
        mg.unseen();

        let Merger {
            c,
            delta,
            mut report,
            fragments,
            ..
        } = mg;
        let next = match (out, taken) {
            (Some(out), _) => NamespaceSnapshot::build(key.clone(), m.dbms(), m.fold(), meta, out)
                .map_err(MergeError::Model)?,
            (None, Some(Dry::Reuse)) => c,
            (None, _) => NamespaceSnapshot::from_arena(
                key.clone(),
                m.dbms(),
                m.fold(),
                meta,
                m.objs().to_vec(),
            )
            .map_err(MergeError::Model)?,
        };
        report.fragments = fragments.into_iter().collect();

        // Step 10: the mass-deletion guard, per top-level kind and over the whole top level.
        let over = |c: &KindCount| {
            c.checked >= policy.guard_min_objects
                && u64::from(c.vanished) * 100
                    > u64::from(c.checked) * u64::from(policy.guard_ratio_pct)
        };
        let total = KindCount {
            checked: report.checked,
            vanished: report.vanished,
        };
        let guarded = !suppress && (over(&total) || report.by_kind.values().any(over));
        Ok(if guarded {
            MergeOutcome::PendingConfirmation {
                next,
                delta,
                vanished: report.vanished,
                report,
            }
        } else {
            MergeOutcome::Applied {
                next,
                delta,
                report,
            }
        })
    }

    fn rule(&self, kind: ObjectKind) -> Option<Rule> {
        self.rules
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|&(_, rule)| rule)
    }

    fn is_ambiguous(&self, o: &Obj) -> bool {
        !self.ambiguous.is_empty()
            && o.server_id()
                .is_some_and(|s| self.ambiguous.contains(&(o.kind(), s)))
    }

    /// What the existence sets say about cached object `d`. A present object whose identity
    /// names it differently and that the pass did not re-read (`reread = false`) is re-read by a
    /// fragment. Top-level decisions are counted for the guard.
    fn decide(&mut self, d: u32, reread: bool) -> Decision {
        let m = self.m;
        let o = &m.objs()[d as usize];
        let Some(Rule::Exist(k)) = self.rule(o.kind()) else {
            return Decision::Undecided;
        };
        let mut path = self.stack.clone();
        path.push(o.name().clone());
        if !self.sets[k].in_scope(&path) {
            return Decision::Undecided;
        }
        let amb = self.is_ambiguous(o);
        let key = (
            path,
            o.props().overload_sig().cloned(),
            o.raw_kind().cloned(),
        );
        let found = self.sets[k].find(o.server_id(), &key, amb, self.folder.as_ref());
        let top = o.parent() == Some(0);
        if top {
            let count = self.report.by_kind.entry(o.kind()).or_default();
            count.checked += 1;
            self.report.checked += 1;
            if found.is_none() {
                count.vanished += 1;
                self.report.vanished += 1;
            }
        }
        match found {
            Some(e) => {
                let set = &mut self.sets[k];
                set.seen[e] = true;
                if !reread && set.ids[e].path != key.0 {
                    let fk = identity_key(&m.key().path, set.kind, &set.ids[e]);
                    self.fragments.insert(fk);
                }
                Decision::Present
            }
            None => Decision::Absent,
        }
    }

    /// Edge 10: whether the changed object `ci` may enter (no in-scope set of its kind, or one
    /// that lists it).
    fn admit(&mut self, ci: u32) -> bool {
        let o = &self.c.objs()[ci as usize];
        let Some(Rule::Exist(k)) = self.rule(o.kind()) else {
            return true;
        };
        let path = rel_path(&self.c, ci);
        if !self.sets[k].in_scope(&path) {
            return true;
        }
        let key = (
            path,
            o.props().overload_sig().cloned(),
            o.raw_kind().cloned(),
        );
        match self.sets[k].find(o.server_id(), &key, false, self.folder.as_ref()) {
            Some(e) => {
                self.sets[k].seen[e] = true;
                true
            }
            None => false,
        }
    }

    /// The changed objects `cs` matched to the children of cached `mi`: one slot per identity,
    /// the last duplicate winning (edge 12). Positional when the pass returned the cached
    /// children in order.
    fn slots(&mut self, mi: u32, cs: &[u32]) -> Result<Vec<Slot>, Abort> {
        if cs.is_empty() {
            return Ok(Vec::new());
        }
        let mut admitted = Vec::with_capacity(cs.len());
        for &ci in cs {
            self.report.steps += 1;
            if self.admit(ci) {
                admitted.push(ci);
            } else {
                if self.dry == Some(Dry::Reuse) {
                    return Err(Abort);
                }
                self.report.ignored += 1;
            }
        }
        let m = self.m;
        let kids: Vec<u32> = m.children(mi).map(|(i, _)| i).collect();
        if kids.len() == admitted.len()
            && kids
                .iter()
                .zip(&admitted)
                .all(|(&d, &ci)| same_sibling(&m.objs()[d as usize], &self.c.objs()[ci as usize]))
        {
            self.report.steps += kids.len() as u64;
            return Ok(kids
                .iter()
                .zip(admitted)
                .map(|(&d, ci)| Slot { ci, mi: Some(d) })
                .collect());
        }
        let sibs = Siblings::of(m, mi, self.folder.as_ref(), &mut self.report.steps);
        let mut slots: Vec<Slot> = Vec::with_capacity(admitted.len());
        let mut by_id: HashMap<SiblingId, usize> = HashMap::new();
        let mut by_m: HashMap<u32, usize> = HashMap::new();
        for ci in admitted {
            let o = &self.c.objs()[ci as usize];
            let found = sibs.find(m, o, self.folder.as_ref());
            let id = SiblingId::of(o);
            let earlier = by_id
                .get(&id)
                .copied()
                .or_else(|| found.and_then(|i| by_m.get(&i).copied()));
            match earlier {
                Some(s) => {
                    if self.dry == Some(Dry::Reuse) {
                        return Err(Abort);
                    }
                    self.report.duplicates += 1;
                    let found = found.or(slots[s].mi);
                    slots[s] = Slot { ci, mi: found };
                    by_id.insert(id, s);
                    if let Some(i) = found {
                        by_m.insert(i, s);
                    }
                }
                None => {
                    by_id.insert(id, slots.len());
                    if let Some(i) = found {
                        by_m.insert(i, slots.len());
                    }
                    slots.push(Slot { ci, mi: found });
                }
            }
        }
        Ok(slots)
    }

    /// Cached object `d` vanished: dropped (in the delta), or kept when deletions are
    /// suppressed. Whether it was dropped.
    fn vanish(&mut self, d: u32) -> Result<bool, Abort> {
        let key = self.m.object_key(d);
        if self.suppress {
            self.report.suppressed.extend(key);
            return Ok(false);
        }
        if self.dry == Some(Dry::Unchanged) {
            return Err(Abort);
        }
        self.delta.dropped.extend(key);
        if self.m.objs()[d as usize].parent() == Some(0) {
            self.dropped_top.push(d);
        }
        Ok(true)
    }

    /// Emits changed object `ci` with its subtree as a creation under `parent`, marking the
    /// existence identities its descendants answer for.
    fn created(
        &mut self,
        out: Option<&mut Subtree>,
        parent: Option<NodeId>,
        ci: u32,
        top: bool,
    ) -> Result<(), Abort> {
        if self.dry == Some(Dry::Unchanged) {
            return Err(Abort);
        }
        if let Some(k) = self.c.object_key(ci) {
            if top && self.below {
                self.report.below_level.push(k.clone());
            }
            self.delta.created.push(k);
        }
        if top {
            self.created_top.push(ci);
        }
        let mut out = out.zip(parent);
        let first = out
            .as_mut()
            .map(|(o, p)| o.add(*p, self.c.objs()[ci as usize].clone()));
        let mut stack = vec![(ci, first)];
        while let Some((s, node)) = stack.pop() {
            for (d, o) in self.c.children(s) {
                self.report.steps += 1;
                if let Some((_, Rule::Exist(k))) = self.rules.iter().find(|(k, _)| *k == o.kind()) {
                    let path = rel_path(&self.c, d);
                    let set = &mut self.sets[*k];
                    if set.in_scope(&path) {
                        let key = (
                            path,
                            o.props().overload_sig().cloned(),
                            o.raw_kind().cloned(),
                        );
                        if let Some(e) = set.find(o.server_id(), &key, false, self.folder.as_ref())
                        {
                            set.seen[e] = true;
                        }
                    }
                }
                let n = match (out.as_mut(), node) {
                    (Some((o_, _)), Some(n)) => Some(o_.add(n, o.clone())),
                    _ => None,
                };
                stack.push((d, n));
            }
        }
        Ok(())
    }

    /// Emits `node` under `parent` (nothing in a dry walk) and merges its children; whether
    /// anything in its subtree changed. Recursion depth is the tree's depth, which the kind
    /// schema bounds on the M4 engines.
    fn node(
        &mut self,
        mut out: Option<&mut Subtree>,
        parent: Option<NodeId>,
        node: Node,
    ) -> Result<bool, Abort> {
        let m = self.m;
        self.report.steps += 1;
        let (mi, ci, reread) = match node {
            Node::Root => (0, None, false),
            Node::Kept(d) => {
                if self.dry == Some(Dry::Reuse) {
                    return Err(Abort);
                }
                (d, None, false)
            }
            Node::Pair(d, c) => (d, Some(c), true),
        };
        let me = out.as_deref_mut().map(|o| match (parent, ci) {
            (None, _) => o.root(),
            (Some(p), Some(c)) => o.add(p, self.c.objs()[c as usize].clone()),
            (Some(p), None) => o.add(p, m.objs()[mi as usize].clone()),
        });
        let cs: Vec<u32> = match node {
            // Cloned: an aborted dry walk is followed by the general one.
            Node::Root => self.c_top.clone(),
            Node::Kept(d) => match self.carriers.get(&d) {
                Some(&leaf) => self.c.children(leaf).map(|(i, _)| i).collect(),
                None => Vec::new(),
            },
            Node::Pair(_, c) => self.c.children(c).map(|(i, _)| i).collect(),
        };
        let top = matches!(node, Node::Root);
        // This node's name in the next namespace (an aborted dry walk resets the stack).
        match node {
            Node::Root => {}
            Node::Kept(d) => self.stack.push(m.objs()[d as usize].name().clone()),
            Node::Pair(_, c) => self.stack.push(self.c.objs()[c as usize].name().clone()),
        }
        let slots = self.slots(mi, &cs)?;
        let mut changed = false;
        if reread {
            // The pass's order, then cached children it did not return that stay. A parent's
            // children are contiguous, so "matched" is a bit per child.
            let first = m.children(mi).next().map_or(0, |(i, _)| i);
            let mut matched = vec![false; m.objs()[mi as usize].child_count() as usize];
            for s in &slots {
                match s.mi {
                    Some(d) if self.decide(d, true) != Decision::Absent => {
                        matched[(d - first) as usize] = true;
                        changed |= self.node(out.as_deref_mut(), me, Node::Pair(d, s.ci))?;
                    }
                    Some(d) => {
                        matched[(d - first) as usize] = true;
                        if self.vanish(d)? {
                            changed = true;
                            self.created(out.as_deref_mut(), me, s.ci, false)?;
                        } else {
                            changed |= self.node(out.as_deref_mut(), me, Node::Pair(d, s.ci))?;
                        }
                    }
                    None => {
                        changed = true;
                        self.created(out.as_deref_mut(), me, s.ci, false)?;
                    }
                }
            }
            for (d, _) in m.children(mi) {
                if matched[(d - first) as usize] {
                    continue;
                }
                self.report.steps += 1;
                if self.decide(d, false) == Decision::Absent && self.vanish(d)? {
                    changed = true;
                    continue;
                }
                changed |= self.node(out.as_deref_mut(), me, Node::Kept(d))?;
            }
        } else {
            // Cached order, then creations.
            let by_m: HashMap<u32, u32> = slots
                .iter()
                .filter_map(|s| s.mi.map(|d| (d, s.ci)))
                .collect();
            let mut orphans = HashSet::new();
            for (d, _) in m.children(mi) {
                self.report.steps += 1;
                let replacement = by_m.get(&d).copied();
                if self.decide(d, replacement.is_some()) == Decision::Absent && self.vanish(d)? {
                    changed = true;
                    orphans.extend(replacement);
                    continue;
                }
                let next = match replacement {
                    Some(c) => Node::Pair(d, c),
                    None => Node::Kept(d),
                };
                changed |= self.node(out.as_deref_mut(), me, next)?;
            }
            for s in &slots {
                if s.mi.is_none() || orphans.contains(&s.ci) {
                    changed = true;
                    self.created(out.as_deref_mut(), me, s.ci, top)?;
                }
            }
        }
        if !top {
            self.stack.pop();
        }
        Ok(match node {
            Node::Root => changed,
            Node::Kept(d) => {
                if changed && let Some(k) = m.object_key(d) {
                    let f = ChangedFacets {
                        children: true,
                        ..Default::default()
                    };
                    self.delta.altered.push((k, f));
                }
                changed
            }
            Node::Pair(d, c) => self.compare(d, c, changed),
        })
    }

    /// Records what changed between cached `mi` and its re-read `ci` (an identity rename, the
    /// altered facets; `children` from the walk); whether anything did.
    fn compare(&mut self, mi: u32, ci: u32, children: bool) -> bool {
        let (a, b) = (&self.m.objs()[mi as usize], &self.c.objs()[ci as usize]);
        let mut f = facets(a, b);
        f.children = children;
        let renamed = a.name() != b.name();
        let top = a.parent() == Some(0);
        if renamed
            && let (Some(before), Some(after)) = (self.m.object_key(mi), self.c.object_key(ci))
        {
            self.delta
                .renamed
                .push((before, after, Confidence::IDENTITY));
        }
        if (f.any() || (top && self.below))
            && let Some(k) = self.c.object_key(ci)
        {
            if top && self.below {
                self.report.below_level.push(k.clone());
            }
            if f.any() {
                self.delta.altered.push((k, f));
            }
        }
        renamed || f.any()
    }

    /// Step 6: rename hints between dropped and created top-level objects without server ids.
    fn hints(&mut self) {
        let mut by_kind: BTreeMap<ObjectKind, (Vec<u32>, Vec<u32>)> = BTreeMap::new();
        for &mi in &self.dropped_top {
            let o = &self.m.objs()[mi as usize];
            if o.server_id().is_none() {
                by_kind.entry(o.kind()).or_default().0.push(mi);
            }
        }
        for &ci in &self.created_top {
            let o = &self.c.objs()[ci as usize];
            if o.server_id().is_none() {
                by_kind.entry(o.kind()).or_default().1.push(ci);
            }
        }
        for (_, (ds, cs)) in by_kind {
            if ds.is_empty() || cs.is_empty() {
                continue;
            }
            let dsig: Vec<Vec<ColSig>> = ds.iter().map(|&i| col_sigs(self.m, i)).collect();
            let csig: Vec<Vec<ColSig>> = cs.iter().map(|&i| col_sigs(&self.c, i)).collect();
            let mut cands: Vec<(Confidence, usize, usize)> = Vec::new();
            if ds.len().saturating_mul(cs.len()) <= HINT_PAIR_BUDGET {
                for (x, a) in dsig.iter().enumerate() {
                    for (y, b) in csig.iter().enumerate() {
                        self.report.steps += 1 + a.len() as u64 + b.len() as u64;
                        if let Some(conf) = similarity(a, b) {
                            cands.push((conf, x, y));
                        }
                    }
                }
            } else {
                let mut exact: HashMap<&[ColSig], Vec<usize>> = HashMap::new();
                for (y, b) in csig.iter().enumerate() {
                    if !b.is_empty() {
                        exact.entry(b.as_slice()).or_default().push(y);
                    }
                }
                for (x, a) in dsig.iter().enumerate() {
                    self.report.steps += 1;
                    for &y in exact.get(a.as_slice()).map_or(&[][..], Vec::as_slice) {
                        cands.push((Confidence::ONE, x, y));
                    }
                }
            }
            cands.sort_by(|p, q| q.0.cmp(&p.0).then(p.1.cmp(&q.1)).then(p.2.cmp(&q.2)));
            let mut used_d = vec![false; ds.len()];
            let mut used_c = vec![false; cs.len()];
            for (conf, x, y) in cands {
                if used_d[x] || used_c[y] {
                    continue;
                }
                used_d[x] = true;
                used_c[y] = true;
                if let (Some(before), Some(after)) =
                    (self.m.object_key(ds[x]), self.c.object_key(cs[y]))
                {
                    self.delta.renamed.push((before, after, conf));
                }
            }
        }
    }

    /// Edge 11: identities no cached or changed object answered for are re-read by fragments.
    fn unseen(&mut self) {
        let ns = &self.m.key().path;
        for set in &self.sets {
            for (e, id) in set.ids.iter().enumerate() {
                if !set.seen[e] && !id.path.is_empty() {
                    self.fragments.insert(identity_key(ns, set.kind, id));
                }
            }
        }
    }
}

/// The names from below the namespace root down to object `i` (its path relative to the
/// namespace, as existence identities carry it).
fn rel_path(ns: &NamespaceSnapshot, i: u32) -> Vec<Name> {
    let mut path = Vec::with_capacity(2);
    let mut cur = i;
    while cur != 0 {
        let o = &ns.objs()[cur as usize];
        path.push(o.name().clone());
        match o.parent() {
            Some(p) => cur = p,
            None => break,
        }
    }
    path.reverse();
    path
}

/// The key of the existence identity `id` of `kind` in the namespace at `ns` (an unknown kind
/// keeps its raw kind, M-7).
fn identity_key(ns: &[Name], kind: ObjectKind, id: &Identity) -> ObjectKey {
    ObjectKey {
        kind,
        path: ns.iter().chain(&id.path).cloned().collect(),
        server_id: id.server_id,
        overload_sig: id.overload_sig.clone(),
        raw_kind: if kind == ObjectKind::Unknown {
            id.raw_kind.clone()
        } else {
            None
        },
    }
}

/// What changed between two versions of one object, its children aside.
fn facets(a: &Obj, b: &Obj) -> ChangedFacets {
    let (pa, pb) = (a.props(), b.props());
    let comment = pa.comment() != pb.comment();
    let source = source_of(pa) != source_of(pb);
    let props = if pa == pb {
        false
    } else if !comment && !source {
        true
    } else {
        strip(pa) != strip(pb)
    };
    ChangedFacets {
        definition: props || a.flags().difference(NAME_FLAGS) != b.flags().difference(NAME_FLAGS),
        children: false,
        comment,
        source,
        grants: false,
    }
}

/// The source reference of a view, routine or trigger.
fn source_of(p: &Props) -> Option<SourceRef> {
    match p {
        Props::View(v) => v.source,
        Props::Routine(r) => r.source,
        Props::Trigger(t) => t.source,
        _ => None,
    }
}

/// `p` without its comment and source (what is left is the definition).
fn strip(p: &Props) -> Props {
    let mut p = p.clone();
    match &mut p {
        Props::None | Props::Key(_) | Props::ForeignKey(_) | Props::Check(_) => {}
        Props::Database(x) => Arc::make_mut(x).comment = None,
        Props::Schema(x) => Arc::make_mut(x).comment = None,
        Props::Table(x) => Arc::make_mut(x).comment = None,
        Props::View(x) => {
            let x = Arc::make_mut(x);
            x.comment = None;
            x.source = None;
        }
        Props::Column(x) => Arc::make_mut(x).comment = None,
        Props::Index(x) => Arc::make_mut(x).comment = None,
        Props::Routine(x) => {
            let x = Arc::make_mut(x);
            x.comment = None;
            x.source = None;
        }
        Props::Sequence(x) => Arc::make_mut(x).comment = None,
        Props::Trigger(x) => Arc::make_mut(x).source = None,
        Props::Type(x) => Arc::make_mut(x).comment = None,
        Props::Generic(x) => Arc::make_mut(x).comment = None,
    }
    p
}

/// The column signatures of object `i`, in order.
fn col_sigs(ns: &NamespaceSnapshot, i: u32) -> Vec<ColSig> {
    ns.children_of_kind(i, ObjectKind::Column)
        .map(|(_, o)| match o.props().as_column() {
            Some(c) => (o.name().clone(), c.raw_type.clone(), c.nullable),
            None => (o.name().clone(), Name::from(""), true),
        })
        .collect()
}

/// The hint confidence of two column signature lists: one when identical, else the share of
/// the larger list both hold (as multisets), capped below one; `None` below 90 % or without
/// columns.
fn similarity(a: &[ColSig], b: &[ColSig]) -> Option<Confidence> {
    if a.is_empty() || b.is_empty() {
        return None;
    }
    if a == b {
        return Some(Confidence::ONE);
    }
    let mut counts: HashMap<&ColSig, u32> = HashMap::with_capacity(a.len());
    for s in a {
        *counts.entry(s).or_default() += 1;
    }
    let mut common = 0usize;
    for s in b {
        if let Some(n) = counts.get_mut(s)
            && *n > 0
        {
            *n -= 1;
            common += 1;
        }
    }
    let per_mille = common * 1000 / a.len().max(b.len());
    let per_mille = u16::try_from(per_mille).unwrap_or(1000);
    (per_mille >= HINT_MIN_PER_MILLE).then(|| Confidence::from_per_mille(per_mille.min(999)))
}
