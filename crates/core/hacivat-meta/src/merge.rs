//! The existence-diff merge (HV-INTRO-11; M4 Task 10): an introspection result plus the
//! previous namespace give the next namespace and its [`CatalogDelta`]. A pure function: it
//! reads no clock, does no I/O and never changes `prev`.
//!
//! - **Identity** is [`ObjectKey::same_object`]: the server id when both sides have one, else
//!   kind, exact path and overload signature. A server id the cache holds twice (a corrupt or
//!   wrapped catalog, T3 m9) matches only by id **and** path, so a stale twin is never kept.
//! - **Deletions** come only from a successful existence set of the object's own
//!   [`ObjectKind`] (`NamespaceResult::existent`). A `Failed` set keeps every cached object of
//!   that kind and reports the kind stale; a failed index query never disables table deletions.
//!   A kind without a set is never deleted by an incremental pass; a full pass is the whole
//!   namespace, so a top-level object of such a kind that the pass did not return is gone.
//! - **Changed objects** replace their cached counterpart wholesale, children included (step 7,
//!   edge 8). Objects the pass did not return are copied with their children and facets, by
//!   reference count. A changed child whose parent is cached (a fragment) replaces the same
//!   child or is appended to it.
//! - **The guard** (D-039, R-M4-22): more than `guard_ratio_pct` % of at least
//!   `guard_min_objects` checked objects vanishing is [`MergeOutcome::PendingConfirmation`],
//!   whatever the trigger; the engine marks the namespace stale (background) or asks (user),
//!   and publishes the pending `next` only on confirmation.
//! - **Rename hints** for name-only engines (step 6): a dropped and a created top-level object
//!   of one kind with ≥ 90 % identical column signatures. Moves across namespaces are paired by
//!   [`CatalogDelta::combine`]; huge deltas are coarse ([`crate::delta::event`]).
//! - `NamespaceResult::categories_to_reload` is not read: every changed object carries its
//!   complete child list (edge 8), so a category re-read shows up as changed objects.
//!
//! Work is O(|M| + |C| + |E|) with hash maps: no `find` per column, no pairwise name matching
//! (hint pairs are bounded by [`HINT_PAIR_BUDGET`]).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use hacivat_proto::DbmsKind;

use crate::delta::{CatalogDelta, ChangedFacets, Confidence};
use crate::ids::{NsKey, ObjectKey, ServerId, SourceRef, StateMark};
use crate::kind::ObjectKind;
use crate::name::{FoldRule, Name};
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
    pub state: StateMark,
}

/// The objects of one kind that exist, or why the existence query failed (then nothing of
/// that kind may be deleted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExistSet {
    Ok(Vec<Identity>),
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
    pub existent: BTreeMap<ObjectKind, ExistSet>,
    pub changed: Vec<ChangedObject>,
    pub categories_to_reload: BTreeSet<Category>,
    pub new_watermark: Watermark,
    /// A full pass (every object re-read), not an incremental one.
    pub full: bool,
    /// The level the pass read at (M4 Task 10): a full pass sets the namespace's level; an
    /// incremental one keeps or lowers it, since the namespace then holds objects of both
    /// passes.
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

/// The mass-deletion guard (D-039): more than `guard_ratio_pct` % of a namespace with at least
/// `guard_min_objects` objects vanishing in one pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergePolicy {
    pub trigger: Trigger,
    pub guard_ratio_pct: u8,
    pub guard_min_objects: u32,
}

impl Default for MergePolicy {
    /// A background pass with D-039's thresholds (50 %, 20 objects).
    fn default() -> Self {
        MergePolicy {
            trigger: Trigger::Background,
            guard_ratio_pct: 50,
            guard_min_objects: 20,
        }
    }
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
    /// Changed objects ignored because their kind's existence set does not list them (edge 10).
    pub ignored: u32,
    /// Changed objects that repeated an earlier one's identity; the last one won (edge 12).
    pub duplicates: u32,
    /// Cached objects whose existence this pass decided (the guard's population).
    pub checked: u32,
    /// Of those, how many vanished.
    pub vanished: u32,
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
    /// result pending, marks the namespace stale, and publishes `next` only on confirmation
    /// (R-M4-22).
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
    /// Not written yet (M4 Task 1a stub).
    NotImplemented,
}

impl fmt::Display for MergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MergeError::WrongNamespace => write!(f, "result for another namespace"),
            MergeError::NoBase => write!(f, "no previous namespace to merge into"),
            MergeError::Model(e) => write!(f, "{e}"),
            MergeError::NotImplemented => write!(f, "not implemented"),
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

/// Flags derived from the name; a rename changes them, not the definition.
const NAME_FLAGS: ObjFlags = ObjFlags::INVISIBLE_CHARS.union(ObjFlags::LOSSY_NAME);

/// A namespace-relative path and an overload signature: one existence identity.
type PathKey = (Vec<Name>, Option<Name>);

/// One kind's successful existence set, indexed.
struct ExistIndex {
    kind: ObjectKind,
    ids: Vec<Identity>,
    by_sid: HashMap<ServerId, usize>,
    by_path: HashMap<PathKey, usize>,
    /// Identities some cached or changed object answered for (the rest are edge 11).
    seen: Vec<bool>,
}

impl ExistIndex {
    fn new(kind: ObjectKind, ids: Vec<Identity>) -> Self {
        let mut by_sid = HashMap::with_capacity(ids.len());
        let mut by_path = HashMap::with_capacity(ids.len());
        for (k, id) in ids.iter().enumerate() {
            if let Some(sid) = id.server_id {
                by_sid.insert(sid, k);
            }
            by_path.insert((id.path.clone(), id.overload_sig.clone()), k);
        }
        let seen = vec![false; ids.len()];
        ExistIndex {
            kind,
            ids,
            by_sid,
            by_path,
            seen,
        }
    }

    /// The identity that is the same object as one with `sid` at `key` (relative path and
    /// overload signature; [`ObjectKey::same_object`]); an `ambiguous` server id matches only
    /// together with its path.
    fn lookup(&self, sid: Option<ServerId>, key: &PathKey, ambiguous: bool) -> Option<usize> {
        if let Some(s) = sid
            && !ambiguous
            && let Some(&k) = self.by_sid.get(&s)
        {
            return Some(k);
        }
        let k = *self.by_path.get(key)?;
        match (self.ids[k].server_id, sid) {
            (Some(a), Some(b)) if a != b => None,
            _ => Some(k),
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
    Name(ObjectKind, Name, Option<Name>, Option<Name>),
}

impl SiblingId {
    fn of(o: &Obj) -> Self {
        match o.server_id() {
            Some(s) => SiblingId::Sid(o.kind(), s),
            None => SiblingId::Name(
                o.kind(),
                o.name().clone(),
                o.props().overload_sig().cloned(),
                o.raw_kind().cloned(),
            ),
        }
    }
}

/// The children of one cached parent, indexed by server id and by name.
struct Siblings {
    by_sid: HashMap<(ObjectKind, ServerId), u32>,
    by_name: HashMap<(ObjectKind, Name, Option<Name>, Option<Name>), u32>,
}

impl Siblings {
    fn of(ns: &NamespaceSnapshot, parent: u32, steps: &mut u64) -> Self {
        let mut by_sid = HashMap::new();
        let mut by_name = HashMap::new();
        for (i, o) in ns.children(parent) {
            *steps += 1;
            if let Some(s) = o.server_id() {
                by_sid
                    .entry((o.kind(), s))
                    .and_modify(|v| *v = AMBIGUOUS)
                    .or_insert(i);
            }
            by_name
                .entry((
                    o.kind(),
                    o.name().clone(),
                    o.props().overload_sig().cloned(),
                    o.raw_kind().cloned(),
                ))
                .or_insert(i);
        }
        Siblings { by_sid, by_name }
    }

    /// The sibling that is the same object as `o` ([`ObjectKey::same_object`] among siblings).
    fn find(&self, ns: &NamespaceSnapshot, o: &Obj) -> Option<u32> {
        if let Some(s) = o.server_id()
            && let Some(&i) = self.by_sid.get(&(o.kind(), s))
            && i != AMBIGUOUS
            && ns.objs()[i as usize].raw_kind() == o.raw_kind()
        {
            return Some(i);
        }
        let key = (
            o.kind(),
            o.name().clone(),
            o.props().overload_sig().cloned(),
            o.raw_kind().cloned(),
        );
        let i = *self.by_name.get(&key)?;
        match (ns.objs()[i as usize].server_id(), o.server_id()) {
            (Some(a), Some(b)) if a != b => None,
            _ => Some(i),
        }
    }
}

/// A changed object (an index into the changed arena) and the cached sibling it replaces.
#[derive(Debug, Clone, Copy)]
struct Slot {
    ci: u32,
    mi: Option<u32>,
}

/// The changed children of one cached parent.
#[derive(Debug, Default)]
struct Nested {
    /// Cached child → changed object.
    replaced: HashMap<u32, u32>,
    /// Changed objects appended under the parent, in pass order.
    created: Vec<u32>,
}

/// One column's signature for rename hints: name, raw type, nullability.
type ColSig = (Name, Name, bool);

struct Merger<'a> {
    /// The cached namespace (`M`).
    m: &'a NamespaceSnapshot,
    /// The changed objects (`C`) as one arena: the root, the top-level changes, then one carrier
    /// chain (copies of a cached parent's ancestors) per cached parent with changed children.
    c: NamespaceSnapshot,
    full: bool,
    rules: Vec<(ObjectKind, Rule)>,
    sets: Vec<ExistIndex>,
    /// `(kind, server id)`s the cache holds more than once.
    ambiguous: HashSet<(ObjectKind, ServerId)>,
    nested: HashMap<u32, Nested>,
    /// Changed objects that were emitted as the replacement of a cached one.
    consumed: HashSet<u32>,
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
        let meta = NsMeta {
            level: if full {
                level
            } else {
                level.min(m.meta().level)
            },
            watermark: new_watermark,
            loaded_at_ms: read_at_ms,
        };
        let c = NamespaceSnapshot::build(key.clone(), m.dbms(), m.fold(), meta.clone(), ct)
            .map_err(MergeError::Model)?;

        let kids: Vec<u32> = c.children(0).map(|(i, _)| i).collect();
        let real = kids.len() - chains.len();
        let mut leaves = Vec::with_capacity(chains.len());
        for (j, &(p, depth)) in chains.iter().enumerate() {
            let mut leaf = kids[real + j];
            for _ in 1..depth {
                leaf = c
                    .children(leaf)
                    .next()
                    .map(|(i, _)| i)
                    .ok_or(MergeError::Model(ModelError::BadLayout("carrier chain")))?;
            }
            leaves.push((p, leaf));
        }

        // Existence sets by kind (steps 1, 2).
        let mut rules = Vec::with_capacity(existent.len());
        let mut sets = Vec::new();
        for (kind, set) in existent {
            match set {
                ExistSet::Ok(ids) => {
                    report.steps += ids.len() as u64;
                    rules.push((kind, Rule::Exist(sets.len())));
                    sets.push(ExistIndex::new(kind, ids));
                }
                ExistSet::Failed(code) => {
                    rules.push((kind, Rule::Failed));
                    report.stale_kinds.insert(kind, code);
                }
            }
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

        let mut mg = Merger {
            m,
            c,
            full,
            rules,
            sets,
            ambiguous,
            nested: HashMap::new(),
            consumed: HashSet::new(),
            dropped_top: Vec::new(),
            created_top: Vec::new(),
            fragments,
            delta: CatalogDelta::default(),
            report,
        };

        let top_slots = mg.slots(0, &kids[..real]);
        for &(p, leaf) in &leaves {
            let cs: Vec<u32> = mg.c.children(leaf).map(|(i, _)| i).collect();
            let slots = mg.slots(p, &cs);
            let mut nested = Nested::default();
            for s in slots {
                match s.mi {
                    Some(mi) => {
                        nested.replaced.insert(mi, s.ci);
                    }
                    None => nested.created.push(s.ci),
                }
            }
            mg.nested.insert(p, nested);
        }

        // Steps 2, 3: which cached top-level objects still exist.
        let repl: HashMap<u32, u32> = top_slots
            .iter()
            .filter_map(|s| s.mi.map(|mi| (mi, s.ci)))
            .collect();
        let mut present = Vec::with_capacity(m.root().child_count() as usize);
        for (i, o) in m.children(0) {
            mg.report.steps += 1;
            let replaced = repl.contains_key(&i);
            let p = match mg.rule(o.kind()) {
                Some(Rule::Exist(k)) => {
                    mg.report.checked += 1;
                    mg.check(k, o, vec![o.name().clone()], replaced)
                }
                Some(Rule::Failed) => true,
                None if mg.full => {
                    mg.report.checked += 1;
                    replaced
                }
                None => true,
            };
            if !p {
                mg.report.vanished += 1;
            }
            present.push((i, p));
        }

        // A full pass that left no cached object standing on its own: the changed arena is the
        // next namespace (in pass order), so nothing is copied or laid out twice.
        let reuse = mg.full
            && leaves.is_empty()
            && mg.report.ignored == 0
            && mg.report.duplicates == 0
            && !mg.rules.iter().any(|(_, r)| matches!(r, Rule::Failed))
            && present.iter().all(|&(i, p)| !p || repl.contains_key(&i));

        // Steps 3–8: the cached top level in arena order, then the creations.
        let mut out =
            (!reuse).then(|| Subtree::with_capacity(mg.c.root().clone(), m.len() + mg.c.len()));
        for &(i, p) in &present {
            if !p {
                mg.drop_cached(i);
                mg.dropped_top.push(i);
                continue;
            }
            let Some(out) = out.as_mut() else {
                // Reused arena: `reuse` holds only when every present object was re-read.
                if let Some(&ci) = repl.get(&i) {
                    mg.compare_replaced(i, ci);
                }
                continue;
            };
            let root = out.root();
            match repl.get(&i) {
                Some(&ci) => {
                    mg.emit_replaced(out, root, i, ci);
                }
                None => mg.emit_kept(out, root, i),
            }
        }
        for s in &top_slots {
            if !mg.consumed.contains(&s.ci) {
                match out.as_mut() {
                    Some(out) => {
                        let root = out.root();
                        mg.emit_created(out, root, s.ci);
                    }
                    None => {
                        if let Some(k) = mg.c.object_key(s.ci) {
                            mg.delta.created.push(k);
                        }
                    }
                }
                mg.created_top.push(s.ci);
            }
        }
        if reuse {
            mg.mark_descendants_seen();
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
        let next = match out {
            Some(out) => NamespaceSnapshot::build(key.clone(), m.dbms(), m.fold(), meta, out)
                .map_err(MergeError::Model)?,
            None => c,
        };
        report.fragments = fragments.into_iter().collect();

        // Step 10: the mass-deletion guard.
        let guarded = report.checked >= policy.guard_min_objects
            && u64::from(report.vanished) * 100
                > u64::from(report.checked) * u64::from(policy.guard_ratio_pct);
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

    /// Whether the cached object `o` at the relative `path` is in existence set `k`. An identity
    /// that names it differently (a rename the pass did not re-read) asks for a fragment.
    fn check(&mut self, k: usize, o: &Obj, path: Vec<Name>, replaced: bool) -> bool {
        let amb = self.is_ambiguous(o);
        let key = (path, o.props().overload_sig().cloned());
        let set = &mut self.sets[k];
        match set.lookup(o.server_id(), &key, amb) {
            Some(e) => {
                set.seen[e] = true;
                if !replaced && set.ids[e].path != key.0 {
                    let key = identity_key(&self.m.key().path, set.kind, &set.ids[e]);
                    self.fragments.insert(key);
                }
                true
            }
            None => false,
        }
    }

    /// Edge 10: whether the changed object `ci` may enter (its kind's existence set lists it).
    fn admit(&mut self, ci: u32) -> bool {
        let o = &self.c.objs()[ci as usize];
        let Some(Rule::Exist(k)) = self.rule(o.kind()) else {
            return true;
        };
        let key = (rel_path(&self.c, ci), o.props().overload_sig().cloned());
        let set = &mut self.sets[k];
        match set.lookup(o.server_id(), &key, false) {
            Some(e) => {
                set.seen[e] = true;
                true
            }
            None => false,
        }
    }

    /// Marks the existence identities the changed arena's nested objects answer for (the
    /// reused-arena path; [`Merger::emit_c`] does it while copying).
    fn mark_descendants_seen(&mut self) {
        for (d, o) in self.c.objs().iter().enumerate().skip(1) {
            self.report.steps += 1;
            if o.parent() == Some(0) {
                continue;
            }
            if let Some((_, Rule::Exist(k))) = self.rules.iter().find(|(k, _)| *k == o.kind()) {
                let key = (
                    rel_path(&self.c, d as u32),
                    o.props().overload_sig().cloned(),
                );
                let set = &mut self.sets[*k];
                if let Some(e) = set.lookup(o.server_id(), &key, false) {
                    set.seen[e] = true;
                }
            }
        }
    }

    /// The changed objects `cs` (children of one carrier) matched to the children of cached
    /// `parent`: one slot per identity, the last duplicate winning (edge 12).
    fn slots(&mut self, parent: u32, cs: &[u32]) -> Vec<Slot> {
        let sibs = Siblings::of(self.m, parent, &mut self.report.steps);
        let mut slots: Vec<Slot> = Vec::with_capacity(cs.len());
        let mut by_id: HashMap<SiblingId, usize> = HashMap::new();
        let mut by_m: HashMap<u32, usize> = HashMap::new();
        for &ci in cs {
            self.report.steps += 1;
            if !self.admit(ci) {
                self.report.ignored += 1;
                continue;
            }
            let o = &self.c.objs()[ci as usize];
            let mi = sibs.find(self.m, o);
            let id = SiblingId::of(o);
            let earlier = by_id
                .get(&id)
                .copied()
                .or_else(|| mi.and_then(|i| by_m.get(&i).copied()));
            match earlier {
                Some(s) => {
                    self.report.duplicates += 1;
                    let mi = mi.or(slots[s].mi);
                    slots[s] = Slot { ci, mi };
                    by_id.insert(id, s);
                    if let Some(i) = mi {
                        by_m.insert(i, s);
                    }
                }
                None => {
                    by_id.insert(id, slots.len());
                    if let Some(i) = mi {
                        by_m.insert(i, slots.len());
                    }
                    slots.push(Slot { ci, mi });
                }
            }
        }
        slots
    }

    fn drop_cached(&mut self, mi: u32) {
        if let Some(k) = self.m.object_key(mi) {
            self.delta.dropped.push(k);
        }
    }

    /// Copies changed object `ci` with its subtree under `parent`, marking the existence
    /// identities its descendants answer for.
    fn emit_c(&mut self, out: &mut Subtree, parent: NodeId, ci: u32) {
        let top = out.add(parent, self.c.objs()[ci as usize].clone());
        let mut stack = vec![(ci, top)];
        while let Some((s, node)) = stack.pop() {
            for (d, o) in self.c.children(s) {
                self.report.steps += 1;
                if let Some((_, Rule::Exist(k))) = self.rules.iter().find(|(k, _)| *k == o.kind()) {
                    let key = (rel_path(&self.c, d), o.props().overload_sig().cloned());
                    let set = &mut self.sets[*k];
                    if let Some(e) = set.lookup(o.server_id(), &key, false) {
                        set.seen[e] = true;
                    }
                }
                let n = out.add(node, o.clone());
                stack.push((d, n));
            }
        }
    }

    fn emit_created(&mut self, out: &mut Subtree, parent: NodeId, ci: u32) {
        if let Some(k) = self.c.object_key(ci) {
            self.delta.created.push(k);
        }
        self.emit_c(out, parent, ci);
    }

    /// Emits changed object `ci` in place of cached `mi` (steps 4, 5, 7, 8); whether anything
    /// about it changed.
    fn emit_replaced(&mut self, out: &mut Subtree, parent: NodeId, mi: u32, ci: u32) -> bool {
        let changed = self.compare_replaced(mi, ci);
        self.emit_c(out, parent, ci);
        changed
    }

    /// Records what changed between cached `mi` and changed `ci` (renamed, altered facets);
    /// whether anything did.
    fn compare_replaced(&mut self, mi: u32, ci: u32) -> bool {
        self.consumed.insert(ci);
        let (a, b) = (&self.m.objs()[mi as usize], &self.c.objs()[ci as usize]);
        let mut f = facets(a, b);
        f.children = !same_children(self.m, mi, &self.c, ci, &mut self.report.steps);
        let renamed = a.name() != b.name();
        if renamed
            && let (Some(before), Some(after)) = (self.m.object_key(mi), self.c.object_key(ci))
        {
            self.delta.renamed.push((before, after, Confidence::ONE));
        }
        if f.any()
            && let Some(k) = self.c.object_key(ci)
        {
            self.delta.altered.push((k, f));
        }
        renamed || f.any()
    }

    /// Copies cached object `mi` with its subtree under `parent`, applying the existence sets
    /// of its descendants' kinds and its changed children.
    fn emit_kept(&mut self, out: &mut Subtree, parent: NodeId, mi: u32) {
        let m = self.m;
        let top = out.add(parent, m.objs()[mi as usize].clone());
        let mut stack = vec![(mi, top)];
        while let Some((s, node)) = stack.pop() {
            let mut nested = self.nested.remove(&s);
            let mut changed = false;
            let mut late = Vec::new();
            for (d, o) in m.children(s) {
                self.report.steps += 1;
                let replacement = nested.as_ref().and_then(|n| n.replaced.get(&d).copied());
                if let Some(Rule::Exist(k)) = self.rule(o.kind()) {
                    self.report.checked += 1;
                    if !self.check(k, o, rel_path(m, d), replacement.is_some()) {
                        self.report.vanished += 1;
                        if let Some(key) = m.object_key(d) {
                            self.delta.dropped.push(key);
                        }
                        changed = true;
                        late.extend(replacement);
                        continue;
                    }
                }
                if let Some(ci) = replacement {
                    changed |= self.emit_replaced(out, node, d, ci);
                    continue;
                }
                let n = out.add(node, o.clone());
                stack.push((d, n));
            }
            let created = nested.take().map(|n| n.created).unwrap_or_default();
            for ci in late.into_iter().chain(created) {
                self.emit_created(out, node, ci);
                changed = true;
            }
            if changed && let Some(k) = m.object_key(s) {
                let f = ChangedFacets {
                    children: true,
                    ..Default::default()
                };
                self.delta.altered.push((k, f));
            }
        }
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

/// The key of the existence identity `id` of `kind` in the namespace at `ns`.
fn identity_key(ns: &[Name], kind: ObjectKind, id: &Identity) -> ObjectKey {
    ObjectKey {
        kind,
        path: ns.iter().chain(&id.path).cloned().collect(),
        server_id: id.server_id,
        overload_sig: id.overload_sig.clone(),
        raw_kind: None,
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

/// Whether two subtrees have the same descendants (kinds, names, server ids, flags, props, in
/// order; state marks aside). Linear, iterative.
fn same_children(
    a: &NamespaceSnapshot,
    i: u32,
    b: &NamespaceSnapshot,
    j: u32,
    steps: &mut u64,
) -> bool {
    let mut stack = vec![(i, j)];
    while let Some((x, y)) = stack.pop() {
        if a.objs()[x as usize].child_count() != b.objs()[y as usize].child_count() {
            return false;
        }
        for ((xi, xo), (yi, yo)) in a.children(x).zip(b.children(y)) {
            *steps += 1;
            if xo.kind() != yo.kind()
                || xo.name() != yo.name()
                || xo.server_id() != yo.server_id()
                || xo.flags() != yo.flags()
                || xo.props() != yo.props()
            {
                return false;
            }
            stack.push((xi, yi));
        }
    }
    true
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
