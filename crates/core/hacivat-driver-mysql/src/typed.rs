//! Typed streaming on MySQL and MariaDB (HV-RES-04 MySQL; Task 14): rows read from the open
//! result through the vendored `mysql_async`, a suspended query held by not reading (the
//! server's `net_write_timeout` bounds it), an early close by `KILL QUERY` on a side
//! connection, chunked into `hacivat_values::Chunk`s through [`crate::decode`].
//!
//! Task 1b (fix round 1): the module line only; the `RawConn` methods still answer
//! `Unsupported` (the trait's defaults).
