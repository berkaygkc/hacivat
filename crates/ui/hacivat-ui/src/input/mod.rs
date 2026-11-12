//! Text input. Task 1 commits the event and focus types; Task 11 adds the IME-correct
//! `TextInput` (and `ime.rs`).

use hacivat_ui_core::gpui_surface::{EntityId, Global};

/// Events a `TextInput` emits (F15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputEvent {
    Changed,
    Submitted,
    Cancelled,
    /// `true` on the first mark of an IME composition, `false` when it commits or is cancelled.
    CompositionChanged(bool),
}

/// The focused text input and whether it is composing. Every `TextInput` keeps it current on
/// focus, blur, mark and unmark (Task 11); the key dispatcher reads `composing` (Task 16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FocusedInput {
    pub entity: Option<EntityId>,
    pub composing: bool,
}

impl Global for FocusedInput {}
