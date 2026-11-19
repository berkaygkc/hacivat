//! The catalog delta of one publish (HV-INTRO-11; R-M4-36 keeps the name `CatalogDelta`, M5's
//! statement-effect delta is `DdlDelta`): what was created, altered, dropped, renamed or moved.
//! The explorer maps it to tree updates keyed by `ObjectKey`. Declared by M4 Task 1a; computed
//! by Task 10's [`crate::merge::merge`].
//!
//! **Granularity.** `created` and `dropped` name the roots of whole subtrees: a created table's
//! columns come with it and are not listed, a dropped table's go with it. A child created,
//! dropped or altered under an existing parent (a re-read or a kept one) is listed itself, and
//! its parent is listed in `altered` with [`ChangedFacets::children`].
//!
//! **Renames.** An **identity** rename ([`Confidence::is_identity`]) kept the object, its
//! children, facets and tree state: by server id, or by the folded name of a caseless engine (a
//! case-only rename on MySQL `lower_case_table_names` 1/2 or SQLite; the merge decided it with
//! the engine's fold rule, so the delta records it rather than re-deriving it from the keys).
//! Any other rename is a **hint** from the structural heuristic of name-only engines: the model
//! holds a new object, its pair is also listed in `dropped` and `created`, and the UI may use the
//! hint only to keep expansion state ([`CatalogDelta::is_hint`]).

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::ids::{NsKey, ObjectKey, ServerId};
use crate::kind::ObjectKind;
use crate::namespace::NamespaceSnapshot;
use crate::snapshot::CatalogEvent;

/// A delta naming more objects than this reaches subscribers as one coarse
/// `CatalogEvent::NamespaceReplaced` instead of per-object updates (alg/02 HV-INTRO-11 edge 13).
pub const COARSE_DELTA_OBJECTS: usize = 10_000;

/// Which parts of an altered object changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChangedFacets {
    /// Properties other than the comment and the source, or flags.
    pub definition: bool,
    /// The child list (columns, keys, indexes, …), compared structurally.
    pub children: bool,
    pub comment: bool,
    /// The source text reference (views, routines, triggers): a pass that did not re-read
    /// sources invalidates it (step 8).
    pub source: bool,
    /// Grants live in a facet side table (R-M4-30) that the merge never sees: always `false`.
    pub grants: bool,
}

impl ChangedFacets {
    /// Whether any part changed.
    pub fn any(&self) -> bool {
        self.definition || self.children || self.comment || self.source || self.grants
    }
}

/// How sure a rename is, and what kind it is: [`Confidence::IDENTITY`] for a rename that kept
/// the object (server id, or a caseless engine's folded name); for a hint, thousandths:
/// [`Confidence::ONE`] when the column signatures are identical, below one from ≥ 90 % similar
/// signatures (alg/02 HV-INTRO-11 step 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Confidence {
    per_mille: u16,
    identity: bool,
}

impl Confidence {
    /// A hint with identical column signatures.
    pub const ONE: Confidence = Confidence {
        per_mille: 1000,
        identity: false,
    };

    /// An identity rename (certain, and the model kept the object).
    pub const IDENTITY: Confidence = Confidence {
        per_mille: 1000,
        identity: true,
    };

    /// A hint of `per_mille` thousandths (saturated at one).
    pub const fn from_per_mille(per_mille: u16) -> Self {
        Confidence {
            per_mille: if per_mille > 1000 { 1000 } else { per_mille },
            identity: false,
        }
    }

    /// The confidence in thousandths.
    pub const fn per_mille(self) -> u16 {
        self.per_mille
    }

    /// Whether the rename kept the object (not a hint).
    pub const fn is_identity(self) -> bool {
        self.identity
    }
}

/// The changes of one publish.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CatalogDelta {
    pub created: Vec<ObjectKey>,
    pub altered: Vec<(ObjectKey, ChangedFacets)>,
    pub dropped: Vec<ObjectKey>,
    /// `(before, after, confidence)`.
    pub renamed: Vec<(ObjectKey, ObjectKey, Confidence)>,
    /// `(from namespace, to namespace, object)`; the object's key is its key in `to`.
    pub moved: Vec<(NsKey, NsKey, ObjectKey)>,
}

impl CatalogDelta {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.created.is_empty()
            && self.altered.is_empty()
            && self.dropped.is_empty()
            && self.renamed.is_empty()
            && self.moved.is_empty()
    }

    /// The number of entries.
    pub fn len(&self) -> usize {
        self.created.len()
            + self.altered.len()
            + self.dropped.len()
            + self.renamed.len()
            + self.moved.len()
    }

    /// Whether the delta is too large for per-object tree updates
    /// ([`COARSE_DELTA_OBJECTS`]).
    pub fn is_coarse(&self) -> bool {
        self.len() > COARSE_DELTA_OBJECTS
    }

    /// Whether a rename entry is a structural hint (its keys are two objects of the model), not
    /// an identity rename (by server id, or fold-aware by a caseless engine's name rule).
    pub fn is_hint(entry: &(ObjectKey, ObjectKey, Confidence)) -> bool {
        !entry.2.is_identity()
    }

    /// The deltas of the namespaces merged for one publish, as one delta: an object dropped from
    /// one namespace and created in another with the same kind and server id is one `moved`
    /// entry (PostgreSQL `ALTER … SET SCHEMA` with both namespaces in scope, step 5). Otherwise
    /// the entries are concatenated in `parts` order. O(entries).
    pub fn combine(parts: Vec<(NsKey, CatalogDelta)>) -> CatalogDelta {
        let mut dropped_by_id: HashMap<(ObjectKind, ServerId), (usize, usize)> = HashMap::new();
        for (p, (_, d)) in parts.iter().enumerate() {
            for (k, key) in d.dropped.iter().enumerate() {
                if let Some(sid) = key.server_id {
                    dropped_by_id.insert((key.kind, sid), (p, k));
                }
            }
        }
        let mut gone: Vec<Vec<bool>> = parts
            .iter()
            .map(|(_, d)| vec![false; d.dropped.len()])
            .collect();
        let mut moved = Vec::new();
        let mut created_moved: Vec<Vec<bool>> = parts
            .iter()
            .map(|(_, d)| vec![false; d.created.len()])
            .collect();
        for (p, (to, d)) in parts.iter().enumerate() {
            for (k, key) in d.created.iter().enumerate() {
                let Some(sid) = key.server_id else { continue };
                let Some(&(fp, fk)) = dropped_by_id.get(&(key.kind, sid)) else {
                    continue;
                };
                if fp == p || gone[fp][fk] {
                    continue;
                }
                gone[fp][fk] = true;
                created_moved[p][k] = true;
                moved.push((parts[fp].0.clone(), to.clone(), key.clone()));
            }
        }
        let mut out = CatalogDelta::default();
        for (p, (_, d)) in parts.into_iter().enumerate() {
            out.created.extend(
                d.created
                    .into_iter()
                    .zip(&created_moved[p])
                    .filter(|(_, m)| !**m)
                    .map(|(k, _)| k),
            );
            out.dropped.extend(
                d.dropped
                    .into_iter()
                    .zip(&gone[p])
                    .filter(|(_, g)| !**g)
                    .map(|(k, _)| k),
            );
            out.altered.extend(d.altered);
            out.renamed.extend(d.renamed);
            out.moved.extend(d.moved);
        }
        out.moved.extend(moved);
        out
    }
}

/// The event subscribers hear for `delta`, published as `generation` of the catalog for `ns`:
/// per-object updates, or one coarse `NamespaceReplaced` above [`COARSE_DELTA_OBJECTS`].
pub fn event(generation: u64, ns: &NsKey, delta: Arc<CatalogDelta>) -> CatalogEvent {
    if delta.is_coarse() {
        CatalogEvent::NamespaceReplaced {
            generation,
            ns: ns.clone(),
        }
    } else {
        CatalogEvent::Published { generation, delta }
    }
}

/// The keys of the objects directly under `ns`'s root: the universe [`apply_delta`] works on.
pub fn top_level_keys(ns: &NamespaceSnapshot) -> BTreeSet<ObjectKey> {
    ns.children(0)
        .filter_map(|(i, _)| ns.object_key(i))
        .collect()
}

/// Why a delta does not apply to a key set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeltaError {
    /// A dropped, renamed, moved or altered object is not in the set.
    Missing(Box<ObjectKey>),
    /// A created, renamed or moved-in object is already in the set.
    Exists(Box<ObjectKey>),
}

impl fmt::Display for DeltaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeltaError::Missing(k) => write!(f, "delta names a missing object: {k:?}"),
            DeltaError::Exists(k) => write!(f, "delta creates an existing object: {k:?}"),
        }
    }
}

impl std::error::Error for DeltaError {}

/// Applies `delta` to `keys`, the top-level object keys of namespace `ns`
/// ([`top_level_keys`]): drops, identity renames, moves out of and into `ns`, then creates;
/// every altered top-level key must be present afterwards. Entries about nested objects and
/// other namespaces, and rename hints, leave the set unchanged. A delta that does not fit the
/// set is an error (a lost or reordered delta), never a silent repair. The delta's serde form
/// carries everything this needs (Karagöz ships deltas, HV-INTRO-17).
pub fn apply_delta(
    ns: &NsKey,
    keys: &BTreeSet<ObjectKey>,
    delta: &CatalogDelta,
) -> Result<BTreeSet<ObjectKey>, DeltaError> {
    let top = |k: &ObjectKey| k.path.len() == ns.path.len() + 1 && k.path.starts_with(&ns.path);
    let mut out = keys.clone();
    for k in delta.dropped.iter().filter(|k| top(k)) {
        if !out.remove(k) {
            return Err(DeltaError::Missing(Box::new(k.clone())));
        }
    }
    for entry @ (before, after, _) in &delta.renamed {
        if CatalogDelta::is_hint(entry) || !top(before) {
            continue;
        }
        if !out.remove(before) {
            return Err(DeltaError::Missing(Box::new(before.clone())));
        }
        if !out.insert(after.clone()) {
            return Err(DeltaError::Exists(Box::new(after.clone())));
        }
    }
    for (from, to, key) in &delta.moved {
        if from == ns {
            let mut old = key.clone();
            let rest: Vec<_> = key.path.iter().skip(to.path.len()).cloned().collect();
            old.path = from.path.iter().cloned().chain(rest).collect();
            if top(&old) && !out.remove(&old) {
                return Err(DeltaError::Missing(Box::new(old)));
            }
        }
        if to == ns && top(key) && !out.insert(key.clone()) {
            return Err(DeltaError::Exists(Box::new(key.clone())));
        }
    }
    for k in delta.created.iter().filter(|k| top(k)) {
        if !out.insert(k.clone()) {
            return Err(DeltaError::Exists(Box::new(k.clone())));
        }
    }
    for (k, _) in delta.altered.iter().filter(|(k, _)| top(k)) {
        if !out.contains(k) {
            return Err(DeltaError::Missing(Box::new(k.clone())));
        }
    }
    Ok(out)
}
