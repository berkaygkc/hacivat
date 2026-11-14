//! Transactions: a change set, the selection after it, and what it was for (UI-EDIT-01;
//! Task 5).

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
    Undo,
    Redo,
    Command(&'static str),
}

/// A transaction's annotations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Annotations {
    pub user_event: UserEvent,
    pub add_to_history: bool,
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
