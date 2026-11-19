//! `sql_catalog::CatalogView` over a [`SnapshotView`] (A-22): what M5's completion and the
//! offline proof of R-M4-5 call.
//!
//! M4 Task 1a seam: every method answers `Unknown(NotLoaded)` (virtual FKs empty, row
//! estimates `None`) until Task 26.

use std::sync::Arc;

use sql_catalog::types::{
    ColumnInfo, FkEdge, FnSig, Name, RelInfo, RelRef, SchemaInfo, SettingInfo, TypeInfo,
};
use sql_catalog::view::CatalogView;

use crate::name::{IdentCtx, IdentKey};
use crate::view::{Lookup, SnapshotView, UnknownReason};

fn not_loaded<T>() -> Lookup<T> {
    Lookup::Unknown(UnknownReason::NotLoaded)
}

impl CatalogView for SnapshotView {
    fn generation(&self) -> u64 {
        self.snapshot().generation
    }

    fn ident_ctx(&self) -> &IdentCtx {
        &self.snapshot().server.ident
    }

    fn schemas(&self) -> Lookup<Arc<[SchemaInfo]>> {
        not_loaded()
    }

    fn relation(&self, _schema: &IdentKey, _name: &IdentKey) -> Lookup<RelInfo> {
        not_loaded()
    }

    fn relations(&self, _schema: &IdentKey) -> Lookup<Arc<[RelInfo]>> {
        not_loaded()
    }

    fn columns(&self, _rel: &RelRef) -> Lookup<Arc<[ColumnInfo]>> {
        not_loaded()
    }

    fn functions(&self, _schema: &IdentKey, _name: &IdentKey) -> Lookup<Arc<[FnSig]>> {
        not_loaded()
    }

    fn functions_in(&self, _schema: &IdentKey) -> Lookup<Arc<[FnSig]>> {
        not_loaded()
    }

    fn types(&self, _schema: &IdentKey) -> Lookup<Arc<[TypeInfo]>> {
        not_loaded()
    }

    fn foreign_keys(&self, _rel: &RelRef) -> Lookup<Arc<[FkEdge]>> {
        not_loaded()
    }

    fn virtual_fks(&self, _rel: &RelRef) -> Arc<[FkEdge]> {
        Arc::from([])
    }

    fn databases(&self) -> Lookup<Arc<[Name]>> {
        not_loaded()
    }

    fn settings(&self) -> Lookup<Arc<[SettingInfo]>> {
        not_loaded()
    }

    fn row_estimate(&self, _rel: &RelRef) -> Option<u64> {
        None
    }
}
