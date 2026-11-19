//! Introspection (HV-INTRO-01..16): catalog query packs, scope and filter grammars, the
//! scheduler and `CatalogHub`, the per-engine introspectors, freshness checks, refresh after
//! DDL and the `minicat` snapshot generator.
//!
//! M4 Task 1a creates the crate and its module lines; Task 1c (Part B) declares the seams with
//! fail-closed stubs.

pub mod assemble;
pub mod ddl_effects;
pub mod engine;
pub mod filter;
pub mod fragment;
pub mod freshness;
pub mod glob;
pub mod hub;
pub mod minicat;
pub mod mysql;
pub mod pack;
pub mod pg;
pub mod refresh_coord;
pub mod scope;
pub mod sqlite;
pub mod testing;
pub mod ticket;
