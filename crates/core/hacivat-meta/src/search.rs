//! Object search over a snapshot (A-14; M8's `search_objects`; M4 Task 9): ASCII-caseless,
//! locale-free and bounded. Exact matches (ASCII-caseless) come first, then prefix matches, then
//! substring matches; within each, namespace key order, then arena order. At most `limit` hits
//! are kept while scanning (three buckets of `limit`), so memory is bounded too. The explorer
//! filter keeps its own Turkish-tolerant matcher over the same iterator
//! ([`crate::namespace::NamespaceSnapshot::iter_named`]).

use crate::ids::NsKey;
use crate::kind::ObjectKind;
use crate::name::Name;
use crate::namespace::Obj;
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
    /// Only these kinds; empty = every kind (but the server namespace's root).
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

/// How well a name matches: the bucket it goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rank {
    Exact = 0,
    Prefix = 1,
    Substring = 2,
}

/// ASCII letters compare caselessly, every other byte exactly (no locale, no Unicode folding:
/// `I` is never `ı` or `İ`). UTF-8 keeps a match of whole characters: an ASCII byte never
/// matches inside a multi-byte character, and a non-ASCII byte matches only itself.
fn rank(name: &str, needle: &str, mode: SearchMode) -> Option<Rank> {
    let (n, q) = (name.as_bytes(), needle.as_bytes());
    if n.len() < q.len() {
        return None;
    }
    if n.eq_ignore_ascii_case(q) {
        return Some(Rank::Exact);
    }
    if n[..q.len()].eq_ignore_ascii_case(q) {
        return Some(Rank::Prefix);
    }
    if mode == SearchMode::Substring
        && n.windows(q.len().max(1))
            .skip(1)
            .any(|w| w.eq_ignore_ascii_case(q))
    {
        return Some(Rank::Substring);
    }
    None
}

/// The objects of `snapshot` that match `query`, best first, at most `query.limit`.
pub fn search(snapshot: &CatalogSnapshot, query: &SearchQuery) -> Vec<ObjHit> {
    if query.limit == 0 {
        return Vec::new();
    }
    let mut buckets: [Vec<ObjHit>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let wanted = |o: &Obj| {
        if query.kinds.is_empty() {
            o.kind() != ObjectKind::Root
        } else {
            query.kinds.contains(&o.kind())
        }
    };
    'scan: for (key, ns) in snapshot.namespaces.iter() {
        for (i, o) in ns.objs().iter().enumerate() {
            if !wanted(o) {
                continue;
            }
            let Some(r) = rank(o.name(), &query.text, query.mode) else {
                continue;
            };
            let bucket = &mut buckets[r as usize];
            if bucket.len() < query.limit {
                bucket.push(ObjHit {
                    ns: key.clone(),
                    id: i as u32,
                    kind: o.kind(),
                    name: o.name().clone(),
                });
            }
            if buckets[Rank::Exact as usize].len() >= query.limit {
                break 'scan;
            }
        }
    }
    let [exact, prefix, substring] = buckets;
    exact
        .into_iter()
        .chain(prefix)
        .chain(substring)
        .take(query.limit)
        .collect()
}
