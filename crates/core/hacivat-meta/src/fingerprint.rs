//! Structural fingerprints (HV-MODEL-03, R-M4-26): XXH3-128 over an object's subtree in a
//! canonical form, for drift checks (M7) and change detection (M8). M4 Task 11.
//!
//! - **What counts:** every object's kind, name, flags and typed properties, in the codec's
//!   encoding (`codec::Enc::canonical`): the same bytes the cache stores, with strings written
//!   in place instead of as table indexes.
//! - **What does not:** `StateMark`s (change markers, not structure), server ids (OIDs; also
//!   the ids inside references such as an FK target), the arena layout and the namespace
//!   metadata (watermark, level, load time). Statistics live in the `Statistics` facet, never
//!   in an `Obj`, so they cannot reach a fingerprint.
//! - **Order:** a node's hash covers its own record and the **sorted** hashes of its children
//!   (a Merkle tree over a multiset), so the order in which a server listed children does not
//!   matter; a column's position still does, through its `ordinal` property.
//! - Locale-free: bytes only; no casing, no collation (tested under `tr_TR`).
//! - Untrusted arenas: iterative (no recursion on tree depth) and linear in the subtree size.

use std::collections::HashMap;

use crate::codec::Enc;
use crate::ids::ObjId;
use crate::namespace::NamespaceSnapshot;

/// An XXH3-128 structural fingerprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Fingerprint(pub u128);

/// The fingerprint of the subtree of `id` (its arena index `id.idx`) in `ns`, or `None` when
/// `ns` holds no such object.
pub fn subtree(ns: &NamespaceSnapshot, id: ObjId) -> Option<Fingerprint> {
    let root = id.idx;
    ns.obj(root)?;
    // Preorder: every child comes after its parent, so the reverse visits children first.
    let mut order = Vec::new();
    let mut stack = vec![root];
    while let Some(i) = stack.pop() {
        order.push(i);
        stack.extend(ns.children(i).map(|(c, _)| c));
    }
    let mut hashes: HashMap<u32, u128> = HashMap::with_capacity(order.len());
    let mut buf = Vec::new();
    let mut kids: Vec<u128> = Vec::new();
    for &i in order.iter().rev() {
        let obj = ns.obj(i)?;
        buf.clear();
        Enc::canonical().obj(&mut buf, obj, &[]);
        kids.clear();
        for (c, _) in ns.children(i) {
            kids.push(*hashes.get(&c)?);
        }
        kids.sort_unstable();
        buf.extend_from_slice(&(kids.len() as u64).to_le_bytes());
        for k in &kids {
            buf.extend_from_slice(&k.to_le_bytes());
        }
        hashes.insert(i, twox_hash::XxHash3_128::oneshot(&buf));
    }
    hashes.get(&root).map(|&h| Fingerprint(h))
}
