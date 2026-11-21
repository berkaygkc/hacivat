//! The SQL-facing catalog view (R-M4-27): the `CatalogView` trait M5's completion and analysis
//! call, the tri-state `Lookup`, and its payload types. `hacivat-meta` implements it
//! (`SnapshotView`, Task 26); [`EmptyCatalog`] is the fail-closed view.
//!
//! Core layer, `crates/sql`: no runtime, no `hacivat-l10n`, no core crate.

pub mod empty;
pub mod lookup;
pub mod types;
pub mod view;

pub use empty::EmptyCatalog;
pub use lookup::{Lookup, MissingReason, UnknownReason};
pub use types::{
    ArgMode, ColumnInfo, FkEdge, FnArg, FnKind, FnSig, GeneratedKind, IdentityKind, Name, RelInfo,
    RelKind, RelRef, SchemaInfo, SettingInfo, TypeInfo, TypeKind, Volatility,
};
pub use view::CatalogView;
