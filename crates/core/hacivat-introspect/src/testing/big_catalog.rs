// ## m4-t01c-core
//! The large PostgreSQL fixture `hv_m4_big` (Task 16; reused by Tasks 28–29, dropped at the
//! gate, C-9): 15,000 tables and 160,000 columns, created and dropped idempotently in batches
//! of 250 tables per server-side `DO` block (PG spike §5). It runs on a writable test session,
//! never on a metadata connection.
//!
//! **Stub (M4 Task 1c).** Both answer [`BigCatalogError::NotImplemented`]; nothing is created.

use futures::future::BoxFuture;
use hacivat_session::session::SessionHandle;

/// Why the fixture could not be built or dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BigCatalogError {
    /// Not written yet (M4 Task 1c stub).
    NotImplemented,
}

/// Creates `hv_m4_big` with `tables` tables and `columns` columns in all, unless it is there.
///
/// Stub: [`BigCatalogError::NotImplemented`].
pub fn ensure_pg(
    session: &SessionHandle,
    tables: u32,
    columns: u32,
) -> BoxFuture<'_, Result<(), BigCatalogError>> {
    let _ = (session, tables, columns);
    Box::pin(async { Err(BigCatalogError::NotImplemented) })
}

/// Drops `hv_m4_big`, in batches.
///
/// Stub: [`BigCatalogError::NotImplemented`].
pub fn drop_pg(session: &SessionHandle) -> BoxFuture<'_, Result<(), BigCatalogError>> {
    let _ = session;
    Box::pin(async { Err(BigCatalogError::NotImplemented) })
}
