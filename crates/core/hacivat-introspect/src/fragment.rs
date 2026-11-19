// ## m4-t01c-core
//! Fragment introspection on a caller's session (R-M4-29; Task 15b): a joined `MetaConn` reads
//! `keys` of `ns` inside a transaction Hacivat owns (M7's drift check inside the apply
//! transaction). Only `joinable` pack sections run there; the connection never begins, commits
//! or rolls back.
//!
//! **Fail-closed stub (M4 Task 1c).** [`on_session`] answers `IntroError::NotImplemented`.

use futures::future::BoxFuture;
use hacivat_meta::{Level, NamespaceSnapshot, NsKey, ObjectKey};
use hacivat_session::session::SessionHandle;
use tokio_util::sync::CancellationToken;

use crate::introspector::IntroError;

/// Reads `keys` of `ns` at `level` on `handle`'s open, Hacivat-owned transaction.
///
/// Stub: [`IntroError::NotImplemented`].
pub fn on_session<'a>(
    handle: &'a SessionHandle,
    ns: &'a NsKey,
    keys: &'a [ObjectKey],
    level: Level,
    cancel: &'a CancellationToken,
) -> BoxFuture<'a, Result<NamespaceSnapshot, IntroError>> {
    let _ = (handle, ns, keys, level, cancel);
    Box::pin(async { Err(IntroError::NotImplemented) })
}
