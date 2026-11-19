//! The `include_bytes!` table of `assets/system-catalogs/` (feature `embedded-system`; only
//! `hacivat-app` enables it, `EmbeddedSystemOnlyFromApp`). Empty until Task 23 generates the
//! assets: `(file name, bytes)` rows, one per `manifest.toml` entry.

/// The shipped snapshot files.
pub static SNAPSHOTS: &[(&str, &[u8])] = &[];
