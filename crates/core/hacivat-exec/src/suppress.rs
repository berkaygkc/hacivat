// ## m3b-t01c-core-seams
//! "Execute and don't ask again" (R-M3-55; Task 15): the `(console, fingerprint)` pairs whose
//! calm non-production confirmation the user turned off for the app session. Never offered
//! with type-to-confirm, never on production. Working since Task 1c (a plain set).

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
