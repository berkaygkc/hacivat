// ## m4-t01c-core
//! Facet loading (R-M4-30; Task 15b): single-flight per `(object, facet)`, `Introspector::facet`
//! on a pooled metadata connection, published as a side-table change that never touches the
//! object's structure.
//!
//! **Fail-closed stub (M4 Task 1c).** Resolves to `IntroError::NotImplemented` and publishes
//! nothing, so the facet stays `NotLoaded` (a DDL producer then emits `ddl-grants-not-loaded`,
//! never a script that silently lacks grants).

use hacivat_meta::ObjectKey;
use hacivat_meta::props::FacetKind;

use super::IntroHandle;
use crate::ticket::RefreshTicket;

/// Loads facet `kind` of `keys` through `handle`'s engine.
pub(crate) fn load(handle: &IntroHandle, keys: Vec<ObjectKey>, kind: FacetKind) -> RefreshTicket {
    let _ = (handle, keys, kind);
    RefreshTicket::not_implemented()
}
