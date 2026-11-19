// ## m4-t01c-core
//! The PostgreSQL introspector (HV-INTRO-05; Task 16, facets Task 16b): passes over
//! `pg_catalog` by oid inside a `REPEATABLE READ READ ONLY` snapshot with the pinned
//! `search_path` (R-M4-46), the `pg_current_snapshot()` watermark (R-M4-8) and one-hop change
//! detection (R-M4-48).
//!
//! **Fail-closed stub (M4 Task 1c).** Every method answers `IntroError::NotImplemented`.

pub mod facets;

use crate::introspector::{IntroCaps, not_implemented_introspector};
use hacivat_proto::DbmsKind;

/// The PostgreSQL introspector.
#[derive(Debug, Clone, Copy, Default)]
pub struct PgIntrospector;

not_implemented_introspector!(PgIntrospector, |_s| IntroCaps::planned(DbmsKind::Postgres));
