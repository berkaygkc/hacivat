// ## m4-t01c-core
//! PostgreSQL facets and server-level sections (R-M4-30; Task 16b): grants, dependencies and
//! statistics by oid, and the `NsKind::Server` namespace (roles, tablespaces).
//!
//! **Fail-closed stub (M4 Task 1c).** Both answer `IntroError::NotImplemented`, so a facet stays
//! `NotLoaded` and the server namespace is never published empty.

use futures::future::BoxFuture;
use hacivat_meta::merge::NamespaceResult;
use hacivat_meta::props::FacetKind;
use hacivat_meta::{NsKey, ObjectKey};
use hacivat_session::meta::MetaConn;

use crate::introspector::{FacetRows, IntroError};

/// One facet of `keys` in `ns`.
///
/// Stub: [`IntroError::NotImplemented`].
pub fn read<'a>(
    c: &'a mut MetaConn,
    ns: &'a NsKey,
    keys: &'a [ObjectKey],
    kind: FacetKind,
) -> BoxFuture<'a, Result<FacetRows, IntroError>> {
    let _ = (c, ns, keys, kind);
    Box::pin(async { Err(IntroError::NotImplemented) })
}

/// The server-level namespace (roles, tablespaces).
///
/// Stub: [`IntroError::NotImplemented`].
pub fn server_namespace(c: &mut MetaConn) -> BoxFuture<'_, Result<NamespaceResult, IntroError>> {
    let _ = c;
    Box::pin(async { Err(IntroError::NotImplemented) })
}
