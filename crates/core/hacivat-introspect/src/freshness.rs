// ## m4-t01c-core
//! Freshness (HV-INTRO-15; Task 22): `check_state` compares `Introspector::stamps` with the
//! snapshot's marks by equality; `check_namespace` validates a cached namespace cheaply (count,
//! highest mark, existence digest) on connect and periodically (R-M4-20).
//!
//! **Fail-closed stub (M4 Task 1c).** [`check_state`] answers [`Staleness::Unknown`] for every
//! key: never `Current` (a stale definition shown as fresh) and never `Dropped` (a wrong
//! "missing"); editing flows treat `Unknown` as `Changed`.

use hacivat_meta::{CatalogSnapshot, ObjectKey, StateMark};

use crate::introspector::Staleness;

/// The cheap validation result of one namespace.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NsCheck {
    /// How many objects the namespace has.
    pub objects: u64,
    /// The highest state mark of the namespace.
    pub max_mark: StateMark,
    /// XXH3-64 of the sorted existence keys.
    pub existence_digest: u64,
}

/// The freshness of `keys` against the server, as the last check recorded it in `snapshot`.
///
/// Stub: `Unknown` for every key.
pub fn check_state(snapshot: &CatalogSnapshot, keys: &[ObjectKey]) -> Vec<Staleness> {
    let _ = snapshot;
    vec![Staleness::Unknown; keys.len()]
}
