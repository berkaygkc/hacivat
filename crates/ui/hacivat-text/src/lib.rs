//! The headless editor core (UI-EDIT-01..08, -12 model): a rope buffer with byte offsets as
//! the canonical positions, change sets and anchors with mapping modes, a change log for
//! rebasing, transactions and grouped linear history, selections and text units, the IME core,
//! and the display model. No GPUI (`TextCoreNotHeadless`).
//!
//! Types complete since M3a Task 1b. Working since Task 1b: [`TextBuffer`] and
//! [`EditorModel::new`], [`EditorModel::dispatch`] (apply the changes, then the selection, given
//! or mapped) and [`EditorModel::snapshot`]. Bodies: Task 5 (`change`, `anchor`, `changelog`,
//! `model`, `transaction`, `history`), Task 10 (`selection`, `units`, `ime`), Task 11
//! (`display`). Until then the other bodies are simple or panic naming their task.

pub mod anchor;
pub mod buffer;
pub mod change;
pub mod changelog;
pub mod display;
pub mod history;
pub mod ime;
pub mod model;
pub mod selection;
pub mod transaction;
pub mod units;

pub use anchor::AnchorSet;
pub use buffer::{Bias, BufferMeta, LineEnding, LineEndingStats, TextBuffer};
pub use change::{ChangeDesc, ChangeSet, MapMode, OverlapError, RangeKind, Section};
pub use changelog::{ChangeLog, Rebased};
pub use ime::ImeCore;
pub use model::{
    BracketId, Committed, EditorModel, Field, FieldId, ModelConfig, Snapshot, TransactionFilter,
    Veto,
};
pub use selection::{SelectionRange, SelectionSet};
pub use transaction::{Annotations, Origin, Transaction, UserEvent};
pub use units::{Dir, TokenOracle, Unit, move_offset};
