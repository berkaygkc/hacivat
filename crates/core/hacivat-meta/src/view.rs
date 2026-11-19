//! The tri-state model view (HV-MODEL-02 step 5; R-M4-27 renames M4's generic trait to
//! `ModelView`). `Lookup` and its reasons are `sql_catalog`'s (one definition).
//!
//! M4 Task 1a seam: every [`SnapshotView`] lookup answers `Unknown(NotLoaded)` (never `Found`,
//! never `Missing`) until Task 9.

use std::sync::Arc;
use std::time::SystemTime;

pub use sql_catalog::lookup::{Lookup, MissingReason, UnknownReason};

use crate::ids::{NsKey, ObjectKey, ServerId};
use crate::kind::{KindGroup, ObjectKind};
use crate::name::{Name, NamePart};
use crate::namespace::Watermark;
use crate::search_path::SearchPath;
use crate::snapshot::{CatalogSnapshot, ObjHandle};

/// How fresh a namespace is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Freshness {
    /// Not loaded, failed, or out of scope: nothing can be said.
    Unknown,
    Loaded {
        at: SystemTime,
        watermark: Watermark,
        stale: bool,
    },
}

/// Lookups over the object model (M7 implements it for its working model too, X-6).
pub trait ModelView: Send + Sync {
    type Def;
    /// The object `path` names in `group`, along the view's search path.
    fn lookup(&self, path: &[NamePart], group: KindGroup) -> Lookup<Self::Def>;
    /// The children of `parent` of `kind`.
    fn children(&self, parent: &ObjectKey, kind: ObjectKind) -> Lookup<Vec<Self::Def>>;
    /// The object of `database` with server id `id` (M6's `OriginCatalog`, A-12).
    fn find_by_server_id(&self, database: &Name, id: ServerId) -> Lookup<Self::Def>;
    /// The freshness of `ns`.
    fn freshness(&self, ns: &NsKey) -> Freshness;
}

/// A model view over one pinned generation.
#[derive(Debug, Clone)]
pub struct SnapshotView {
    snapshot: Arc<CatalogSnapshot>,
    search_path: SearchPath,
}

impl SnapshotView {
    /// A view of `snapshot` with an empty search path.
    pub fn new(snapshot: Arc<CatalogSnapshot>) -> Self {
        SnapshotView {
            snapshot,
            search_path: SearchPath::default(),
        }
    }

    /// The same view resolving unqualified names along `search_path`.
    pub fn with_search_path(mut self, search_path: SearchPath) -> Self {
        self.search_path = search_path;
        self
    }

    /// The pinned generation.
    pub fn snapshot(&self) -> &Arc<CatalogSnapshot> {
        &self.snapshot
    }

    pub fn search_path(&self) -> &SearchPath {
        &self.search_path
    }
}

impl ModelView for SnapshotView {
    type Def = ObjHandle;

    /// Task 1a stub: `Unknown(NotLoaded)`.
    fn lookup(&self, path: &[NamePart], group: KindGroup) -> Lookup<ObjHandle> {
        let _ = (path, group);
        Lookup::Unknown(UnknownReason::NotLoaded)
    }

    /// Task 1a stub: `Unknown(NotLoaded)`.
    fn children(&self, parent: &ObjectKey, kind: ObjectKind) -> Lookup<Vec<ObjHandle>> {
        let _ = (parent, kind);
        Lookup::Unknown(UnknownReason::NotLoaded)
    }

    /// Task 1a stub: `Unknown(NotLoaded)`.
    fn find_by_server_id(&self, database: &Name, id: ServerId) -> Lookup<ObjHandle> {
        let _ = (database, id);
        Lookup::Unknown(UnknownReason::NotLoaded)
    }

    /// Task 1a stub: `Unknown`.
    fn freshness(&self, ns: &NsKey) -> Freshness {
        let _ = ns;
        Freshness::Unknown
    }
}
