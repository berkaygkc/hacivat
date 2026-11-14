//! `MemoryBudget` (Task 7): the global budget shared by every result and the per-result cap
//! (R-M3-19, R-M3-33). No eviction in M3a.
//!
//! Working since Task 1b: the constructors and `used` (nothing charges the budget before
//! Task 7's writer).

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// The per-result cap: 256 MiB (R-M3-19).
pub const PER_RESULT_CAP: u64 = 256 << 20;

/// The app's memory budget for results.
pub struct MemoryBudget {
    global: u64,
    per_result: u64,
    used: AtomicU64,
}

impl fmt::Debug for MemoryBudget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryBudget")
            .field("global", &self.global)
            .field("per_result", &self.per_result)
            .field("used", &self.used())
            .finish()
    }
}

impl MemoryBudget {
    /// `global` bytes for every result together, `per_result` for each.
    pub fn new(global: u64, per_result: u64) -> Arc<Self> {
        Arc::new(MemoryBudget {
            global,
            per_result,
            used: AtomicU64::new(0),
        })
    }

    /// min(25 % of `ram`, 4 GiB); 1 GiB when the RAM is unknown; 256 MiB per result.
    pub fn from_ram(ram: Option<u64>) -> Arc<Self> {
        let global = ram.map_or(1 << 30, |ram| (ram / 4).min(4 << 30));
        Self::new(global, PER_RESULT_CAP)
    }

    /// The bytes every result holds now.
    pub fn used(&self) -> u64 {
        self.used.load(Ordering::SeqCst)
    }

    /// The global budget.
    pub fn global(&self) -> u64 {
        self.global
    }

    /// The cap of one result (before "Fetch more anyway").
    pub fn per_result(&self) -> u64 {
        self.per_result
    }
}
