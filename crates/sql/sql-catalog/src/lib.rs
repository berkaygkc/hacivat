//! The SQL-facing catalog view (R-M4-27): the `CatalogView` trait M5's completion and analysis
//! call, the tri-state `Lookup`, and its payload types. `hacivat-meta` implements it.

pub mod empty;
pub mod lookup;
pub mod types;
pub mod view;
