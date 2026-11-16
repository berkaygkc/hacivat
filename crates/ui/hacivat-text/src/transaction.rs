//! Transactions: a change set, the selection after it, and what it was for (UI-EDIT-01).
//!
//! History (UI-EDIT-04, `history.rs`) reads the annotations: `add_to_history` decides whether
//! the change is recorded or the stored groups are rebased over it, `user_event` which group
//! it may join, and `time` the 500 ms window. The model never reads the wall clock for
//! grouping; the caller stamps `time`.

use std::time::Instant;

use crate::change::ChangeSet;
use crate::selection::SelectionSet;

/// Who made a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Origin {
    User,
    Ime,
    /// `set_text`, a restored console.
    Program,
    /// The language service (M5).
    Service,
}

/// What the user did: history grouping and the input handler read it.
///
/// Only `InputType`, `DeleteBackward` and `DeleteForward` ever join an open undo group (each
/// its own family; a Backspace also joins the typing it corrects). Every other event starts a
/// group of its own that nothing joins, except inside a bracket, where all join the bracket's
/// group (`InputTypeCompose` is meant for one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UserEvent {
    InputType,
    InputTypeCompose,
    InputPaste,
    InputDrop,
    DeleteBackward,
    DeleteForward,
    DeleteSelection,
    DeleteCut,
    Move,
    Select,
    SelectPointer,
    /// The model's own undo and redo transactions.
    Undo,
    Redo,
    Command(&'static str),
}

/// A transaction's annotations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Annotations {
    pub user_event: UserEvent,
    /// Recorded as (part of) an undo group; when `false` the change is not undoable and the
    /// stored groups are rebased over it (a group it overlaps is dropped, with older ones).
    pub add_to_history: bool,
    /// When it happened (the grouping window).
    pub time: Instant,
    pub origin: Origin,
}

/// One atomic edit of the model: the changes, the selection after them (`None`: the old
/// selection mapped), and its annotations.
#[derive(Debug, Clone, PartialEq)]
pub struct Transaction {
    pub changes: ChangeSet,
    pub selection: Option<SelectionSet>,
    pub ann: Annotations,
}
