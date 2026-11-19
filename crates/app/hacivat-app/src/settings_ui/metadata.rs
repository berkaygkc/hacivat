//! Settings › Metadata (Task 27; design/m4-explorer surface 13): refresh after DDL (Always, Ask,
//! Never), the background check interval, the cache limits and "Clear cached metadata for this
//! connection…".
//!
//! ## m4-t01b-seams: stub. The section has no rows, so the settings page and its navigation are
//! unchanged (no shot changes) until Task 27 adds them.

use super::rows::SettingRow;

/// The Metadata section's rows. Stub: none (hidden until Task 27).
pub fn metadata_rows() -> Vec<SettingRow> {
    Vec::new()
}
