// ## m4-t01c-core
//! The SQLite introspector (HV-INTRO-08; Task 18): `sqlite_schema` and the pragma passes on a
//! separate `mode=ro` connection (R-M4-38), `schema_version` plus a schema digest as the change
//! marker, the `temp` schema read on the user's idle connection with the authorizer's pragmas.
//!
//! **Fail-closed stub (M4 Task 1c).** Every method answers `IntroError::NotImplemented`.

use crate::introspector::{IntroCaps, not_implemented_introspector};
use hacivat_proto::DbmsKind;

/// The SQLite introspector.
#[derive(Debug, Clone, Copy, Default)]
pub struct SqliteIntrospector;

not_implemented_introspector!(SqliteIntrospector, |_s| IntroCaps::planned(
    DbmsKind::Sqlite
));
