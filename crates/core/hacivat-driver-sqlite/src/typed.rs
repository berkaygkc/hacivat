//! Typed streaming on SQLite (HV-RES-04 SQLite; Task 15): a typed job on the connection's
//! blocking thread, stepping a prepared statement into `hacivat_values::Chunk`s through
//! [`crate::decode`], with the declared column types (`column_decltype`) as the native type.
//!
//! Task 1b (fix round 1): the module line only; the `RawConn` methods still answer
//! `Unsupported` (the trait's defaults).
