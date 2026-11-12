//! The app's drivers (HV-DRV-01).

use hacivat_driver::DriverRegistry;

/// The compiled-in drivers: `PgDriver`, `MySqlDriver` and `SqliteDriver` (Task 16 registers
/// them; until then the registry is empty).
pub fn default_drivers() -> DriverRegistry {
    DriverRegistry::new()
}
