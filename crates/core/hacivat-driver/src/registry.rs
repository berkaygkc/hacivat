//! The driver registry: compiled-in drivers, found through the catalog's fallback chain
//! (HV-DRV-01).

use std::sync::Arc;

use hacivat_proto::DbmsKind;

use crate::catalog::EngineCatalog;
use crate::driver::Driver;

/// The registered drivers (HV-DRV-01).
#[derive(Default)]
pub struct DriverRegistry {
    drivers: Vec<Arc<dyn Driver>>,
}

impl DriverRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `d`. A later registration for the same kind replaces the earlier one in lookups.
    pub fn register(&mut self, d: Arc<dyn Driver>) {
        self.drivers.push(d);
    }

    /// The driver for `kind`: its own registration first, then each engine of the embedded
    /// catalog's fallback chain (`MariaDb → MySql`), then a driver registered for `Unknown`.
    pub fn resolve(&self, kind: DbmsKind) -> Option<Arc<dyn Driver>> {
        EngineCatalog::embedded()
            .chain(kind)
            .into_iter()
            .chain(std::iter::once(DbmsKind::Unknown))
            .find_map(|k| self.registered(k))
    }

    fn registered(&self, kind: DbmsKind) -> Option<Arc<dyn Driver>> {
        self.drivers
            .iter()
            .rev()
            .find(|d| d.descriptor().kind == kind)
            .cloned()
    }
}
