//! The SQL-facing catalog view (R-M4-27; M5 Task 1a's trait). Lookups never block: a container
//! that is not loaded answers `Unknown` (the implementation may enqueue one background load).

use std::sync::Arc;

use sql_ident::key::{IdentCtx, IdentKey};

use crate::lookup::Lookup;
use crate::types::{
    ColumnInfo, FkEdge, FnSig, Name, RelInfo, RelRef, SchemaInfo, SettingInfo, TypeInfo,
};

/// What completion, hover, diagnostics and the tools read from a catalog.
pub trait CatalogView: Send + Sync {
    /// The catalog generation this view reads (monotonic).
    fn generation(&self) -> u64;
    /// The server's identifier matching context; probes are keyed with it.
    fn ident_ctx(&self) -> &IdentCtx;
    /// The schemas of the current database.
    fn schemas(&self) -> Lookup<Arc<[SchemaInfo]>>;
    /// The relation `name` in `schema`.
    fn relation(&self, schema: &IdentKey, name: &IdentKey) -> Lookup<RelInfo>;
    /// Every relation of `schema`.
    fn relations(&self, schema: &IdentKey) -> Lookup<Arc<[RelInfo]>>;
    /// The columns of `rel`, in ordinal order.
    fn columns(&self, rel: &RelRef) -> Lookup<Arc<[ColumnInfo]>>;
    /// The overloads of `name` in `schema`.
    fn functions(&self, schema: &IdentKey, name: &IdentKey) -> Lookup<Arc<[FnSig]>>;
    /// Every routine of `schema`.
    fn functions_in(&self, schema: &IdentKey) -> Lookup<Arc<[FnSig]>>;
    /// Every type of `schema`.
    fn types(&self, schema: &IdentKey) -> Lookup<Arc<[TypeInfo]>>;
    /// The foreign keys of `rel`, both directions (A-13).
    fn foreign_keys(&self, rel: &RelRef) -> Lookup<Arc<[FkEdge]>>;
    /// The stored virtual foreign keys of `rel` (local only, D-057); empty when none.
    fn virtual_fks(&self, rel: &RelRef) -> Arc<[FkEdge]>;
    /// The databases the connection can see.
    fn databases(&self) -> Lookup<Arc<[Name]>>;
    /// The server's setting names.
    fn settings(&self) -> Lookup<Arc<[SettingInfo]>>;
    /// The `Statistics` facet's row estimate of `rel` (display only), if loaded.
    fn row_estimate(&self, rel: &RelRef) -> Option<u64>;
}
