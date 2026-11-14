//! The change log: the last 256 committed changes, to rebase ranges computed at an older
//! version (UI-EDIT-01; R-M3-22 rebases off-thread highlighting; Task 5).
//!
//! Task 1b: the log records every commit; `rebase` maps only a range from the current version
//! and answers `TooOld` for any other (fail closed: the caller drops or repaints it).

use std::collections::VecDeque;
use std::ops::Range;

use crate::change::{ChangeDesc, RangeKind};

/// The number of changes the log keeps.
pub const CHANGE_LOG_LEN: usize = 256;

/// What a rebased range became.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Rebased {
    Mapped(Range<usize>),
    /// An edit deleted it.
    Deleted,
    /// Its version is older than the log.
    TooOld,
}

/// The recent changes, newest last.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChangeLog {
    entries: VecDeque<(u64, ChangeDesc)>,
    version: u64,
}

impl ChangeLog {
    /// Maps `r`, computed at `from_version`, to the current version (Task 5).
    pub fn rebase(&self, from_version: u64, r: Range<usize>, kind: RangeKind) -> Rebased {
        let _ = kind;
        if from_version == self.version {
            Rebased::Mapped(r)
        } else {
            Rebased::TooOld
        }
    }

    /// The version after the newest change.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Records the change that made `version`.
    pub(crate) fn record(&mut self, version: u64, desc: ChangeDesc) {
        if self.entries.len() == CHANGE_LOG_LEN {
            self.entries.pop_front();
        }
        self.entries.push_back((version, desc));
        self.version = version;
    }
}
