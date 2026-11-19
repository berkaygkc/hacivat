//! The profile's object filters as the engine applies them (HV-INTRO-03; Task 12 carries).
//!
//! - **Scope role** (objects never read): the introspectors take no filter (the Task 1c seam),
//!   so the engine applies it to every pass result on the client before the merge
//!   ([`Filters::apply`]): a top-level object the filter does not keep is dropped from the
//!   changed set **and** from its kind's existence set, so the merge neither stores it nor asks
//!   for a fragment of it (edge 11). It never applies inside a system namespace
//!   ([`CompiledFilter::applies_to`]), whose objects resolve types and functions.
//! - **Lookups** answer `Unknown(OutOfScope)` for what the scope filter may have left unread:
//!   [`ScopeFilterAdapter`] is the `hacivat_meta::view::ScopeFilter` the explorer and M5 hand to
//!   `SnapshotView::with_scope_filter`.
//! - **Pushdown** (an optimisation only: the client filter decides): a section may push a
//!   filter only through [`bounded_pushdown`], which refuses a predicate of more than
//!   [`MAX_PUSHED_RUNS_PER_SECTION`] runs. Task 12 caps one call at 200 runs; one run is one
//!   query, so the per-profile total is bounded here: at most 16 runs per section of one
//!   namespace pass (T12 review M5). The engine itself pushes nothing in M4, because the
//!   introspector seam carries no filter (ADR 0008 `m4-t15`).

use std::collections::HashSet;
use std::sync::Arc;

use hacivat_meta::merge::{ExistSet, NamespaceResult};
use hacivat_meta::name::IdentCtx;
use hacivat_meta::view::ScopeFilter;
use hacivat_meta::{Name, NsKey, ObjectKind};
use sql_dialect::DialectId;

use crate::filter::{CompiledFilter, FilterRole, SqlPredicate};
use crate::scope::NsInfo;

/// The most runs (queries) one section may issue for a pushed filter.
pub const MAX_PUSHED_RUNS_PER_SECTION: usize = 16;

/// A pushdown predicate for a section returning rows of `kinds`, or `None` when it would take
/// more than [`MAX_PUSHED_RUNS_PER_SECTION`] runs (the client filter then decides alone).
pub fn bounded_pushdown(
    filter: &CompiledFilter,
    kinds: &[ObjectKind],
    dialect: DialectId,
    ident: &IdentCtx,
) -> Option<SqlPredicate> {
    filter
        .pushdown_kinds(kinds, dialect, ident)
        .filter(|p| p.binds.len() <= MAX_PUSHED_RUNS_PER_SECTION)
}

/// The profile's filters.
#[derive(Debug, Clone, Default)]
pub struct Filters {
    /// The scope filter (`FilterRole::Introspect`): objects it does not keep are never read.
    pub scope: Option<CompiledFilter>,
}

impl Filters {
    /// The scope filter, when it is one (a view filter here is ignored: it never cuts a read).
    fn scope(&self) -> Option<&CompiledFilter> {
        self.scope
            .as_ref()
            .filter(|f| f.role() == FilterRole::Introspect && !f.is_empty())
    }

    /// Drops from `r` every top-level object of `ns` the scope filter does not keep: from the
    /// changed set and from its kind's existence set (so the merge neither stores it nor asks
    /// for a fragment of it). `system` namespaces are never filtered. Answers how many changed
    /// objects were dropped.
    pub fn apply(&self, ns: &NsKey, system: bool, r: &mut NamespaceResult) -> usize {
        let Some(f) = self.scope() else {
            return 0;
        };
        let info = NsInfo {
            key: ns.clone(),
            system,
        };
        if !f.applies_to(&info) {
            return 0;
        }
        let depth = ns.path.len();
        let mut dropped_names: HashSet<(ObjectKind, Name)> = HashSet::new();
        let before = r.changed.len();
        r.changed.retain(|c| {
            // Only objects right under the namespace root are filtered.
            if c.parent.path.len() != depth {
                return true;
            }
            let o = c.subtree.root_obj();
            let keep = f.matches(o.kind(), o.name());
            if !keep {
                dropped_names.insert((o.kind(), o.name().clone()));
            }
            keep
        });
        for (kind, set) in r.existent.iter_mut() {
            let ids = match set {
                ExistSet::Ok(ids) => ids,
                ExistSet::Scoped { ids, .. } => ids,
                ExistSet::Failed(_) => continue,
            };
            ids.retain(|id| id.path.len() != 1 || f.matches(*kind, &id.path[0]));
        }
        // A changed child under a dropped parent goes with it.
        if !dropped_names.is_empty() {
            r.changed.retain(|c| {
                let p = &c.parent;
                p.path.len() <= depth
                    || !dropped_names.iter().any(|(k, n)| {
                        p.path[depth] == *n && (p.path.len() > depth + 1 || p.kind == *k)
                    })
            });
        }
        before - r.changed.len()
    }
}

/// The scope filter as lookups see it (`hacivat_meta::view::ScopeFilter`): for every namespace
/// but the system ones, whatever the filter may have left unread answers `Unknown(OutOfScope)`.
#[derive(Debug, Clone)]
pub struct ScopeFilterAdapter {
    filter: CompiledFilter,
    system: Arc<HashSet<NsKey>>,
}

impl ScopeFilterAdapter {
    /// The adapter of `filter` (`with_ident(server facts)` already applied by the caller), with
    /// the system namespaces `system` exempt.
    pub fn new(filter: CompiledFilter, system: HashSet<NsKey>) -> Self {
        ScopeFilterAdapter {
            filter,
            system: Arc::new(system),
        }
    }
}

impl ScopeFilter for ScopeFilterAdapter {
    fn excludes(&self, ns: &NsKey, kind: Option<ObjectKind>, name: Option<&str>) -> bool {
        if self.system.contains(ns) {
            return false;
        }
        self.filter.excludes(kind, name)
    }
}
