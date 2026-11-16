//! The app's drivers (HV-DRV-01).

use std::sync::Arc;

use hacivat_driver::DriverRegistry;
use hacivat_driver_mysql::MySqlDriver;
use hacivat_driver_pg::PgDriver;
use hacivat_driver_sqlite::SqliteDriver;

/// The compiled-in drivers: `PgDriver`, `MySqlDriver` (MariaDB falls back to it through the
/// catalog chain) and `SqliteDriver`.
pub fn default_drivers() -> DriverRegistry {
    let mut drivers = DriverRegistry::new();
    drivers.register(Arc::new(PgDriver::new()));
    drivers.register(Arc::new(MySqlDriver::new()));
    drivers.register(Arc::new(SqliteDriver::new()));
    drivers
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacivat_proto::DbmsKind;

    #[test]
    fn every_m2a_engine_has_a_driver() {
        let drivers = default_drivers();
        for (kind, crate_name) in [
            (DbmsKind::Postgres, "hacivat-driver-pg"),
            (DbmsKind::MySql, "hacivat-driver-mysql"),
            (DbmsKind::MariaDb, "hacivat-driver-mysql"),
            (DbmsKind::Sqlite, "hacivat-driver-sqlite"),
        ] {
            let d = drivers.resolve(kind).expect("a driver is registered");
            assert_eq!(d.descriptor().crate_name, crate_name, "{kind:?}");
        }
        assert!(drivers.resolve(DbmsKind::Redis).is_none());
    }
}
