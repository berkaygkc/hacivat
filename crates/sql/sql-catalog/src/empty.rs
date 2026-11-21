//! The fail-closed catalog: every lookup is `Unknown(NotLoaded)`, never `Found` and never
//! `Missing`, so nothing reads it as a fact.

use std::sync::Arc;

use sql_ident::key::{IdentCtx, IdentKey};

use crate::lookup::{Lookup, UnknownReason};
use crate::types::{
    ColumnInfo, FkEdge, FnSig, Name, RelInfo, RelRef, SchemaInfo, SettingInfo, TypeInfo,
};
use crate::view::CatalogView;

/// A catalog that knows nothing.
#[derive(Debug, Clone)]
pub struct EmptyCatalog {
    ident: IdentCtx,
}

impl EmptyCatalog {
    pub fn new(ident: IdentCtx) -> Self {
        EmptyCatalog { ident }
    }
}

fn not_loaded<T>() -> Lookup<T> {
    Lookup::Unknown(UnknownReason::NotLoaded)
}

impl CatalogView for EmptyCatalog {
    fn generation(&self) -> u64 {
        0
    }

    fn ident_ctx(&self) -> &IdentCtx {
        &self.ident
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
