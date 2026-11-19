//! Object search over a snapshot (A-14; M8's `search_objects`): ASCII-caseless, locale-free and
//! bounded. The explorer filter keeps its own Turkish-tolerant matcher over the same iterator.
//!
//! M4 Task 1a seam: answers no hits until Task 9 (not wired to any user-visible action).

use crate::ids::NsKey;
use crate::kind::ObjectKind;
use crate::name::Name;
use crate::snapshot::CatalogSnapshot;

/// How the query text matches a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchMode {
    #[default]
    Substring,
    Prefix,
}

/// A search request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    pub text: String,
    /// Only these kinds; empty = every kind.
    pub kinds: Vec<ObjectKind>,
    /// At most this many hits.
    pub limit: usize,
    pub mode: SearchMode,
}

/// One hit: the namespace, the arena index, the kind and the exact name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjHit {
    pub ns: NsKey,
    pub id: u32,
    pub kind: ObjectKind,
    pub name: Name,
}

/// The objects of `snapshot` that match `query`. Task 1a stub: none.
pub fn search(snapshot: &CatalogSnapshot, query: &SearchQuery) -> Vec<ObjHit> {
    let _ = (snapshot, query);
    Vec::new()
}
