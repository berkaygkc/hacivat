// ## m4-t01c-core
//! `FakeIntrospector` (body Task 15): an in-memory introspector the engine's tests drive
//! (scripted namespaces, query counting, gated passes).
//!
//! **Stub (M4 Task 1c).** Every method answers `IntroError::NotImplemented`, like the real
//! introspectors' stubs.

use crate::introspector::{IntroCaps, not_implemented_introspector};
use hacivat_proto::DbmsKind;

/// A scripted introspector for tests.
#[derive(Debug, Clone, Copy)]
pub struct FakeIntrospector {
    pub dbms: DbmsKind,
}

impl Default for FakeIntrospector {
    fn default() -> Self {
        FakeIntrospector {
            dbms: DbmsKind::Postgres,
        }
    }
}

not_implemented_introspector!(FakeIntrospector, |s| IntroCaps::planned(s.dbms));
