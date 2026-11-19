//! On-demand reference resolution (HV-MODEL-01 "References", N-9): a [`Ref`] stores the
//! target's path and server id, never an arena index; it is resolved against one generation.
//!
//! M4 Task 1a seam: answers `Unknown(NotLoaded)` until Task 9.

use crate::ids::Ref;
use crate::snapshot::{CatalogSnapshot, ObjHandle};
use crate::view::{Lookup, UnknownReason};

/// Resolves `r` in `snapshot`: `Found` when the target namespace is loaded and holds it,
/// `Missing` only when it is loaded, in scope and absent, `Unknown` otherwise.
///
/// Task 1a stub: `Unknown(NotLoaded)`.
pub fn resolve_ref(snapshot: &CatalogSnapshot, r: &Ref) -> Lookup<ObjHandle> {
    let _ = (snapshot, r);
    Lookup::Unknown(UnknownReason::NotLoaded)
}
