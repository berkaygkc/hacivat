//! The driver registry: compiled-in drivers, found through the catalog's fallback chain
//! (HV-DRV-01). `resolve` in Task 2.

use std::sync::Arc;

use hacivat_proto::DbmsKind;

use crate::driver::Driver;

/// The registered drivers (HV-DRV-01).
#[derive(Default)]
pub struct DriverRegistry {
    #[allow(dead_code, reason = "Task 2")]
    drivers: Vec<Arc<dyn Driver>>,
}

impl DriverRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, d: Arc<dyn Driver>) {
        self.drivers.push(d);
    }

    /// The driver for `kind`, walking the catalog fallback chain (Task 2). The stub finds
    /// nothing.
    pub fn resolve(&self, _kind: DbmsKind) -> Option<Arc<dyn Driver>> {
        None
    }
}
