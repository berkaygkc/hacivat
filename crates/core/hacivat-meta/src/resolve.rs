//! On-demand reference resolution and the per-namespace completeness rules (HV-MODEL-01
//! "References", HV-MODEL-02 step 5; N-9; M4 Task 9). A [`Ref`] stores the target's kind, exact
//! path and server id, never an arena index; it is resolved against one generation, so it
//! cannot go stale when either side's namespace is reloaded (R-M4-26, spike origin `d86313b`).
//!
//! What a namespace's [`LoadState`] allows:
//!
//! | State | Found | A miss |
//! |---|---|---|
//! | `Loaded` at `Structure` or `Sources`, with an arena | yes | `Missing` |
//! | `Loaded` at `Names`; `NotLoaded`; no state | from an arena, if any | `Unknown(NotLoaded)` |
//! | `Loading` | from a cached arena, if any | `Unknown(Loading)` |
//! | `Failed`, `Partial` | from a cached arena, if any | `Unknown(Failed)` |
//! | `OutOfScope` | never | `Unknown(OutOfScope)` |
//!
//! `Names` reads "major object names" (alg/02 HV-INTRO-01); which kinds that covers per engine
//! is the introspectors' (Tasks 16–18), so a miss at `Names` is not a fact here.

use std::sync::Arc;

use smallvec::SmallVec;

use crate::ids::{NsKey, NsKind, Ref};
use crate::kind::pg_family;
use crate::name::Name;
use crate::namespace::{Level, NamespaceSnapshot};
use crate::snapshot::{CatalogSnapshot, ContainerKey, LoadState, ObjHandle};
use crate::view::{Lookup, MissingReason, UnknownReason};

/// How complete one namespace is (the table in the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NsStatus {
    /// `Loaded` at `Structure` or above, with an arena: a miss is a fact.
    Complete,
    /// Hits come from the arena if there is one; a miss answers `Unknown(reason)`.
    /// `wants_load`: not loaded at a sufficient level, so the view asks for it (step 5).
    Partial {
        reason: UnknownReason,
        wants_load: bool,
    },
    /// Outside the introspection scope: nothing is said, not even a hit.
    OutOfScope,
}

/// One namespace of a generation as a lookup sees it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NsView<'a> {
    /// The arena to search (never for an out-of-scope namespace).
    pub arena: Option<&'a Arc<NamespaceSnapshot>>,
    pub status: NsStatus,
    /// The catalog lists the namespace (an arena or a state), so it exists on the server.
    pub listed: bool,
}

/// The completeness of namespace `key` in `snap`.
pub(crate) fn ns_status<'a>(snap: &'a CatalogSnapshot, key: &NsKey) -> NsView<'a> {
    let arena = snap.namespaces.get(key);
    let state = snap.states.get(&ContainerKey::Namespace(key.clone()));
    let not_loaded = NsStatus::Partial {
        reason: UnknownReason::NotLoaded,
        wants_load: true,
    };
    let status = match state {
        Some(LoadState::OutOfScope) => NsStatus::OutOfScope,
        Some(LoadState::Loaded { level, .. }) if arena.is_some() && *level >= Level::Structure => {
            NsStatus::Complete
        }
        Some(LoadState::Loaded { .. } | LoadState::NotLoaded) | None => not_loaded,
        Some(LoadState::Loading { .. }) => NsStatus::Partial {
            reason: UnknownReason::Loading,
            wants_load: false,
        },
        // A failed load backs off by itself (`retry_after`); the view does not ask again.
        Some(LoadState::Failed { .. } | LoadState::Partial { .. }) => NsStatus::Partial {
            reason: UnknownReason::Failed,
            wants_load: false,
        },
    };
    NsView {
        arena: if status == NsStatus::OutOfScope {
            None
        } else {
            arena
        },
        status,
        listed: arena.is_some() || state.is_some(),
    }
}

/// Whether the schema list that would hold namespace `key` is loaded
/// ([`ContainerKey::schema_list`]): PostgreSQL schemas `[db, schema]` by their database,
/// everything else by the server's list.
pub(crate) fn list_known_for(snap: &CatalogSnapshot, key: &NsKey) -> bool {
    let database = (pg_family(snap.server.dbms) && key.kind == NsKind::Schema)
        .then(|| key.path.first())
        .flatten();
    if pg_family(snap.server.dbms) && database.is_none() {
        return false;
    }
    schema_list_loaded(snap, database)
}

/// Whether the schema list of `database` (or of the server) is loaded.
pub(crate) fn schema_list_loaded(snap: &CatalogSnapshot, database: Option<&Name>) -> bool {
    matches!(
        snap.states.get(&ContainerKey::schema_list(database)),
        Some(LoadState::Loaded { .. })
    )
}

/// The namespace (listed in `snap`: an arena or a state) that holds objects under `path`: the
/// longest listed prefix, schemas before databases before the server.
pub(crate) fn locate_ns_of(snap: &CatalogSnapshot, path: &[Name]) -> Option<NsKey> {
    for k in (0..path.len()).rev() {
        for kind in [NsKind::Schema, NsKind::Database, NsKind::Server] {
            let key = NsKey {
                kind,
                path: path[..k].iter().cloned().collect(),
            };
            if snap.namespaces.contains_key(&key)
                || snap
                    .states
                    .contains_key(&ContainerKey::Namespace(key.clone()))
            {
                return Some(key);
            }
        }
    }
    None
}

/// Where a reference points in one generation, ignoring load states.
#[derive(Debug)]
pub(crate) struct Located<'a> {
    /// The namespace that would hold the target; `None` when no listed namespace does.
    pub ns: Option<NsKey>,
    /// The target, when an arena of this generation holds it.
    pub hit: Option<(&'a Arc<NamespaceSnapshot>, u32)>,
}

impl<'a> Located<'a> {
    pub fn hit(&self) -> Option<(&'a Arc<NamespaceSnapshot>, u32)> {
        self.hit
    }
}

/// Finds `r`'s target: the namespace by the longest listed prefix of its parent path; then
/// through the name index of its kind's group when it sits right under the namespace root
/// (or anywhere below it, for a kind the index holds), else by a walk of exact names; among
/// same-named hits the one with `r`'s server id wins, else the first.
pub(crate) fn locate_ref<'a>(snap: &'a CatalogSnapshot, r: &Ref) -> Located<'a> {
    if r.kind.is_namespace_root() {
        let kind = match r.kind {
            crate::kind::ObjectKind::Root => NsKind::Server,
            crate::kind::ObjectKind::Database => NsKind::Database,
            _ => NsKind::Schema,
        };
        let key = NsKey {
            kind,
            path: r.path.iter().cloned().collect(),
        };
        let hit = snap.namespaces.get(&key).map(|ns| (ns, 0));
        return Located { ns: Some(key), hit };
    }
    let Some((name, parent)) = r.path.split_last() else {
        return Located {
            ns: None,
            hit: None,
        };
    };
    let Some(key) = locate_ns_of(snap, &r.path) else {
        return Located {
            ns: None,
            hit: None,
        };
    };
    let Some(ns) = snap.namespaces.get(&key) else {
        return Located {
            ns: Some(key),
            hit: None,
        };
    };
    let rest = &parent[key.path.len()..];
    let mut candidates: SmallVec<[u32; 2]> = SmallVec::new();
    if rest.is_empty() && r.kind.group(ns.dbms()).is_some() {
        candidates.extend(
            ns.lookup_kind(r.kind, name)
                .filter(|&i| ns.objs()[i as usize].name() == name),
        );
    } else if let Some(at) = walk_names(ns, rest) {
        candidates.extend(
            ns.children_of_kind(at, r.kind)
                .filter(|(_, o)| o.name() == name)
                .map(|(i, _)| i),
        );
    }
    let pick = candidates
        .iter()
        .copied()
        .find(|&i| r.server_id.is_some() && ns.objs()[i as usize].server_id() == r.server_id)
        .or_else(|| candidates.first().copied());
    Located {
        ns: Some(key),
        hit: pick.map(|i| (ns, i)),
    }
}

/// The object reached from the namespace root by exact child names (the first matching path).
fn walk_names(ns: &NamespaceSnapshot, names: &[Name]) -> Option<u32> {
    fn go(ns: &NamespaceSnapshot, at: u32, names: &[Name]) -> Option<u32> {
        let Some((head, tail)) = names.split_first() else {
            return Some(at);
        };
        ns.children(at)
            .filter(|(_, o)| o.name() == head)
            .find_map(|(i, _)| go(ns, i, tail))
    }
    go(ns, 0, names)
}

/// Resolves `r` in `snapshot`: `Found` when the target namespace is in scope and an arena
/// holds the target, `Missing` only when that namespace is complete and lacks it, `Unknown`
/// otherwise (a namespace the catalog does not list is `Unknown(NotLoaded)`).
pub fn resolve_ref(snapshot: &CatalogSnapshot, r: &Ref) -> Lookup<ObjHandle> {
    let located = locate_ref(snapshot, r);
    let Some(key) = located.ns.as_ref() else {
        return Lookup::Unknown(UnknownReason::NotLoaded);
    };
    match ns_status(snapshot, key).status {
        NsStatus::OutOfScope => Lookup::Unknown(UnknownReason::OutOfScope),
        status => match (located.hit, status) {
            (Some((ns, idx)), _) => Lookup::Found(snapshot.handle(ns, idx)),
            (None, NsStatus::Complete) => Lookup::Missing(MissingReason::NotFound),
            (None, NsStatus::Partial { reason, .. }) => Lookup::Unknown(reason),
            (None, NsStatus::OutOfScope) => Lookup::Unknown(UnknownReason::OutOfScope),
        },
    }
}
