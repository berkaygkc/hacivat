//! Canonical JSON format 1 (R-M4-17): byte-order sorted, no server ids, OIDs, state marks,
//! watermarks, timestamps, hostnames or paths, every named exclusion rule applied. The
//! generator (`hacivat-introspect` `minicat`) and the decoder share it, so a double run is
//! byte-identical.
//!
//! M4 Task 1a seam: refuses until Task 23.

use super::SystemError;

/// The canonical JSON bytes of a raw introspection result. Task 1a stub:
/// `Err(SystemError::NotImplemented)`.
pub fn normalize(raw: &serde_json::Value) -> Result<Vec<u8>, SystemError> {
    let _ = raw;
    Err(SystemError::NotImplemented)
}
