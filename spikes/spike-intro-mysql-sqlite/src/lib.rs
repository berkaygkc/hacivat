//! M4 pre-work spike (not merged before M4 starts): the MySQL/MariaDB and SQLite introspectors
//! of `hacivat-introspect` (design spec §5.1), over the existing `RawConn` driver API.
//!
//! - [`conn`]: catalog SQL with bound names → text rows, over any `RawConn`'s typed path.
//! - [`model`]: the plain output structs (mapping onto HV-MODEL-01 in the spike report).
//! - [`mysql`]: `INFORMATION_SCHEMA` introspection, MySQL `CREATE_TIME` incremental passes.
//! - [`sqlite`]: `sqlite_schema` + `pragma_*` introspection, `schema_version` + digest checks.

pub mod conn;
pub mod model;
pub mod mysql;
pub mod sqlite;
