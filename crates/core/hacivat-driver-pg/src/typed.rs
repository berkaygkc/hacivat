//! Typed streaming on PostgreSQL (HV-RES-04 PG; Task 12): the typed `RawConn` methods' engine
//! side over the vendored `tokio-postgres` patch §1 (`query_with_formats` for every
//! non-suspendable unit, a named portal with `bind_with_formats` + `query_portal_raw` for a
//! suspendable one, P5), chunked into `hacivat_values::Chunk`s through [`crate::decode`].
//!
//! Task 1b (fix round 1): the module line only; the `RawConn` methods still answer
//! `Unsupported` (the trait's defaults).
