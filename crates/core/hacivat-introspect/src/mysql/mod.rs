// ## m4-t01c-core
//! The MySQL and MariaDB introspector (HV-INTRO-07; Task 17, facets Task 17b): passes over
//! `INFORMATION_SCHEMA` on a read-only session with a consistent-snapshot read-only transaction
//! (R-M4-7), the R-M4-39 markers on MySQL, `Full` per namespace on MariaDB (R-M4-10), and
//! `SHOW CREATE` text through the quoted-identifier macro (R-M4-37).
//!
//! **Fail-closed stub (M4 Task 1c).** Every method answers `IntroError::NotImplemented`.

pub mod facets;

use crate::introspector::{IntroCaps, not_implemented_introspector};
use hacivat_proto::DbmsKind;

/// The MySQL and MariaDB introspector.
#[derive(Debug, Clone, Copy)]
pub struct MySqlIntrospector {
    /// `DbmsKind::MySql` or `DbmsKind::MariaDb` (one pack; flavor gates inside, F9).
    pub flavor: DbmsKind,
}

not_implemented_introspector!(MySqlIntrospector, |s| IntroCaps::planned(s.flavor));
