//! Structural fingerprints (HV-MODEL-03, R-M4-26): XXH3-128 over an object's subtree in a
//! canonical order, for drift checks (M7) and change detection.
//!
//! M4 Task 1a seam: no fingerprint is computed until Task 11. The stub answers `None` rather
//! than a constant, so two objects can never compare as "unchanged" by accident.

use crate::ids::ObjId;
use crate::namespace::NamespaceSnapshot;

/// An XXH3-128 structural fingerprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Fingerprint(pub u128);

/// The fingerprint of the subtree of `id` in `ns`, or `None` when `ns` holds no such object.
/// Task 1a stub: `None`.
pub fn subtree(ns: &NamespaceSnapshot, id: ObjId) -> Option<Fingerprint> {
    let _ = (ns, id);
    None
}
