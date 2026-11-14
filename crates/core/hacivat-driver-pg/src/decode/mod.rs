//! PostgreSQL wire values to `hacivat_values` (HV-RES-01 PG mapping, R-M3-31; Task 12): the
//! binary format per type OID, the text fallback as `Opaque`. Public (hidden) so
//! `tests/decode.rs` reaches the decoders.
//!
//! Task 1b (fix round 1): the module lines only.

pub mod numeric;
pub mod scalar;
pub mod temporal;
