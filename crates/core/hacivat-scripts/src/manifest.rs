//! `.hacivat/workspace.toml`, untrusted input with a fixed schema (HV-SCRIPT-01 steps 3–4;
//! R-M4b-8, -21, -42).
//!
//! ## m4b-t01a-seams
//! Task 1a: the read seams as fail-closed stubs: they read nothing and bind nothing, and say so
//! in [`Diag::NotImplemented`]. Task 3 writes the reader and the typed writer (exactly
//! `format`, `database`, `aliases`; L14).

use std::path::Path;

use crate::names::LogicalDb;

/// What reading a library's manifest found.
#[derive(Debug, Clone, Default)]
pub struct ManifestRead {
    /// The `database` key, when the manifest was read and the name passed the rules.
    pub db: Option<LogicalDb>,
    pub aliases: Vec<LogicalDb>,
    pub diags: Vec<Diag>,
}

/// A manifest diagnostic (shown escaped; never a reason to run anything).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diag {
    /// Task 1a's stub read nothing.
    NotImplemented,
}

/// Reads `<library_dir>/.hacivat/workspace.toml`.
///
/// Task 1a stub (fail closed): opens nothing and binds nothing.
pub fn read(library_dir: &Path) -> ManifestRead {
    let _ = library_dir;
    stub()
}

/// Reads a manifest's bytes for a library folder named `folder` (the fuzz target's entry,
/// `fuzz/scripts/fuzz_targets/workspace_toml.rs`; [`read`] calls it after its `lstat` checks).
///
/// Task 1a stub (fail closed): parses nothing and binds nothing.
pub fn read_bytes(bytes: &[u8], folder: &str) -> ManifestRead {
    let _ = (bytes, folder);
    stub()
}

fn stub() -> ManifestRead {
    ManifestRead {
        db: None,
        aliases: Vec::new(),
        diags: vec![Diag::NotImplemented],
    }
}
// ## m4b-t01a-seams
