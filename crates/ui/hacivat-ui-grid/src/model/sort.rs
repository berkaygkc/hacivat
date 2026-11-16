//! Local sort keys (HV-GRID-07; M3b Task 36). The grid only names what to sort by
//! (`GridEvent::SortRequested`); the app sorts off the UI thread with the column's collation
//! and publishes the order as `ViewMap::Permuted`.
//!
//! ## m3b-t01b-seams: the types (Task 1b). Task 36 fills the module.

use smallvec::SmallVec;

use crate::source::ColId;

/// One sort key: a column and its direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SortKey {
    pub col: ColId,
    pub desc: bool,
}

/// The keys of a sort, most significant first (empty: no sort, source order).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct SortSpec {
    pub keys: SmallVec<[SortKey; 3]>,
}
