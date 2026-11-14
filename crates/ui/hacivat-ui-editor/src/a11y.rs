//! The editor's accessibility baseline (UI-EDIT-24 subset, D-073; Task 29): role
//! `MultilineTextInput` with its label, a ±100-line window of runs split at 255 characters
//! with stable node ids, built only while assistive technology is active; polite announcements.
//!
//! Task 1b: nothing is declared or announced.

use hacivat_text::Snapshot;
use hacivat_ui_core::gpui_surface::{App, SharedString, Window};

/// The module-owned state (P11): the stable line anchors behind the node ids (Task 29).
#[derive(Debug, Clone, Default)]
pub struct A11yState {}

/// Declares the editor's accessibility node for `snap` (Task 29). Task 1b: nothing.
pub fn declare(a: &mut A11yState, snap: &Snapshot, window: &mut Window, cx: &mut App) {
    let _ = (a, snap, window, cx);
}

/// A polite live announcement (execution status, "Run would execute lines 3–7"; Task 29).
/// Task 1b: nothing.
pub fn announce(text: SharedString, window: &mut Window, cx: &mut App) {
    let _ = (text, window, cx);
}
