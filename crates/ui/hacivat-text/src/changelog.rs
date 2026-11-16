//! The change log: the last 256 committed changes, to rebase ranges computed at an older
//! version (UI-EDIT-01, UI-EDIT-03 step 9; R-M3-22 rebases off-thread highlighting, marks and
//! the preview through it).
//!
//! Versions count text changes: every commit that changes the text makes the next version and
//! one entry; a selection-only commit makes neither. So the entries are consecutive versions,
//! and finding where a version starts is O(1).

use std::collections::VecDeque;
use std::ops::Range;

use crate::change::{ChangeDesc, RangeKind};

/// The number of changes the log keeps.
pub const CHANGE_LOG_LEN: usize = 256;

/// What a rebased range became.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Rebased {
    Mapped(Range<usize>),
    /// An edit dropped it (by the range kind's rules).
    Deleted,
    /// Its version is older than the log, or newer than the text: recompute it.
    TooOld,
}

/// The recent changes, newest last.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChangeLog {
    /// `(the version the change made, the change)`, consecutive versions.
    entries: VecDeque<(u64, ChangeDesc)>,
    version: u64,
}

impl ChangeLog {
    /// Maps `r`, computed at `from_version`, to the current version through every change since,
    /// in order, by `kind`'s rules (`ChangeDesc::map_range`). O(versions × sections).
    pub fn rebase(&self, from_version: u64, r: Range<usize>, kind: RangeKind) -> Rebased {
        let Some(descs) = self.descs_since(from_version) else {
            return Rebased::TooOld;
        };
        let mut r = r;
        for d in descs {
            match d.map_range(r, kind) {
                Some(next) => r = next,
                None => return Rebased::Deleted,
            }
        }
        Rebased::Mapped(r)
    }

    /// The changes made since `from_version`, oldest first (none at the current version);
    /// `None` when the log no longer holds them all, or `from_version` is in the future. For
    /// mapping many positions at once (an `AnchorSet` per change).
    pub fn descs_since(&self, from_version: u64) -> Option<impl Iterator<Item = &ChangeDesc>> {
        if from_version > self.version {
            return None;
        }
        let first = match self.entries.front() {
            Some((v, _)) => *v,
            None => self.version + 1,
        };
        // The change that made `from_version + 1` must still be here (or nothing happened).
        if from_version + 1 < first {
            return None;
        }
        let skip = usize::try_from(from_version + 1 - first).ok()?;
        Some(self.entries.iter().skip(skip).map(|(_, d)| d))
    }

    /// The version after the newest change.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Records the change that made `version` (the next version).
    pub(crate) fn record(&mut self, version: u64, desc: ChangeDesc) {
        debug_assert_eq!(version, self.version + 1, "versions are consecutive");
        if self.entries.len() == CHANGE_LOG_LEN {
            self.entries.pop_front();
        }
        self.entries.push_back((version, desc));
        self.version = version;
    }
}
