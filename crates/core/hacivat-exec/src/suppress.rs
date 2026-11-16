// ## m3b-t01c-core-seams
//! "Execute and don't ask again" (R-M3-55; Task 15): the `(console, fingerprint)` pairs whose
//! confirmation the user turned off for the app session (never persisted). Working since Task
//! 1c (a plain set).
//!
//! ## m3b-t15-gate
//! The gate ([`crate::gate`]) consults it for a suppressible `Confirm` or `Warn` only: never
//! for a type-to-confirm unit (the production `DmlUnsafe` and `DROP DATABASE/SCHEMA` cases) nor
//! for a MySQL/MariaDB implicit commit, so a pair recorded off production never lowers a
//! production confirmation that asks for a typed name. On production the dialog names the
//! scope ("don't ask again in this console", `design/m3b-safe/NOTES.md` ruling 4). The
//! fingerprint is the classified text's (`SafetyReport.fingerprint.hash`): a statement of the
//! same shape.

use std::collections::HashSet;

/// The suppressed `(console, fingerprint)` pairs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SuppressSet {
    pairs: HashSet<(u64, u64)>,
}

impl SuppressSet {
    pub fn insert(&mut self, console: u64, fp: u64) {
        self.pairs.insert((console, fp));
    }

    pub fn contains(&self, console: u64, fp: u64) -> bool {
        self.pairs.contains(&(console, fp))
    }
}
