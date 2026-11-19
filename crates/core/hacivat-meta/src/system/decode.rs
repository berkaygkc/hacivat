//! Decoding one shipped snapshot (R-M4-33): a zstd frame (checksum on) of canonical JSON
//! format 1. Untrusted bytes: decoding never panics (fuzz target `system_snapshot_decode`).
//!
//! M4 Task 1a seam: refuses until Task 23.

use super::{SystemError, SystemSnapshot};

/// The snapshot in `bytes`. Task 1a stub: `Err(SystemError::NotImplemented)`.
pub fn decode(bytes: &[u8]) -> Result<SystemSnapshot, SystemError> {
    let _ = bytes;
    Err(SystemError::NotImplemented)
}
