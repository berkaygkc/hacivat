//! The result store (HV-RES-02, -03, -05 types, -07 v1): columnar chunks published through an
//! `ArcSwap` snapshot on every append (R-M3-48), one writer (the session actor), per-result and
//! global memory budgets, console cursors and the `ResultSource` the grid adapter reads.
//!
//! Types are complete since M3a Task 1b; bodies arrive in Task 7 (they panic naming it until
//! then). Working since Task 1b: `FetchPolicy::default` and the `MemoryBudget` constructors.

pub mod budget;
pub mod cursor;
#[cfg(feature = "test-support")]
mod fixture;
pub mod snapshot;
pub mod store;
pub mod window;
pub mod writer;

pub use budget::MemoryBudget;
pub use cursor::{
    CloseReason, Completeness, CursorInfo, FetchControl, ResultSource, SourceCaps, StoreSource,
    TruncReason,
};
pub use snapshot::StoreSnapshot;
pub use store::{ResultId, ResultStore};
pub use window::FetchPolicy;
pub use writer::{Admission, PauseReason, ResultWriter};
