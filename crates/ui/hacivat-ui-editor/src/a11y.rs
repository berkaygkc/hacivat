//! The editor's accessibility baseline (UI-EDIT-24 subset, D-073; Task 29): role
//! `MultilineTextInput` with its label, a ±100-line window of runs split at 255 characters
//! with stable node ids, built only while assistive technology is active; polite announcements.
//!
//! Task 1b: nothing is declared or announced. The view calls `declare` from its paint path
//! (`element::paint_frame`) since fix round 1.

use hacivat_text::Snapshot;
use hacivat_ui_core::gpui_surface::{App, Bounds, Pixels, SharedString, Window};

use crate::config::{EditorConfig, Tier};

/// The module-owned state (P11): the stable line anchors behind the node ids (Task 29).
#[derive(Debug, Clone, Default)]
pub struct A11yState {}

/// What one frame declares (the view builds it at its paint point).
#[derive(Debug, Clone)]
pub struct A11yFrame<'a> {
    pub snapshot: &'a Snapshot,
    /// The editor's accessible label (`EditorView::a11y_label`).
    pub label: SharedString,
    /// `a11y_window_lines` (D-073: 100), `a11y_optimized`, `read_only`.
    pub config: &'a EditorConfig,
    /// T1 ("Reduced mode") disables the optimised window growth.
    pub tier: Tier,
    pub focused: bool,
    /// The editor's text area, in window coordinates.
    pub bounds: Bounds<Pixels>,
}

/// Declares the editor's accessibility node for one frame (Task 29; only while
/// `window.is_a11y_active()`). Task 1b: nothing.
pub fn declare(a: &mut A11yState, frame: &A11yFrame<'_>, window: &mut Window, cx: &mut App) {
    let _ = (a, frame, window, cx);
}

/// A polite live announcement (execution status, "Run would execute lines 3–7"; Task 29).
/// Task 1b: nothing.
pub fn announce(text: SharedString, window: &mut Window, cx: &mut App) {
    let _ = (text, window, cx);
}
