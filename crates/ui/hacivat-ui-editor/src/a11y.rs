//! The editor's accessibility baseline (UI-EDIT-24 subset, D-073; Task 29): role
//! `MultilineTextInput` with its label, a ±100-line window of runs split at 255 characters
//! with stable node ids, built only while assistive technology is active; polite announcements.
//!
//! The GPUI fork builds accessibility nodes only through an element's hooks, during prepaint:
//! `Element::a11y_role`, `write_a11y_info` (before the element's prepaint) and
//! `a11y_synthetic_children` (after it). The editor element (`element::EditorElement`, Task 19)
//! calls [`role`], [`write_info`] and [`synthetic_children`] from those hooks, with an
//! [`A11yFrame`] it builds in `request_layout` only while `window.is_a11y_active()`, so nothing
//! is evaluated without assistive technology (round 2 of Task 1b's review; the pattern is
//! `hacivat_ui::popover`'s `Declare`).
//!
//! Task 1b: no role (so no node), nothing written, no children; [`announce`] queues its text in
//! the state for the live-region child Task 29 builds.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use hacivat_text::Snapshot;
use hacivat_ui_core::gpui_surface::{A11ySubtreeBuilder, Role, SharedString, accesskit};

use crate::config::{EditorConfig, Tier};

/// The most announcements kept for the next frame; older ones are dropped (nothing reads the
/// queue while no assistive technology is active).
pub const MAX_PENDING_ANNOUNCEMENTS: usize = 4;

/// The module-owned state (P11): a shared handle, because the element's accessibility hooks
/// get no window or view. Task 29 keeps its stable line anchors in [`A11yNodes`].
#[derive(Debug, Clone, Default)]
pub struct A11yState {
    inner: Rc<RefCell<A11yNodes>>,
}

/// What the accessibility hooks keep between frames (Task 29 adds the line anchors behind the
/// stable node ids; keep `Default`).
#[derive(Debug, Default)]
pub struct A11yNodes {
    /// Polite announcements waiting for the next frame's live-region child, oldest first.
    pub pending: VecDeque<SharedString>,
}

impl A11yState {
    /// The shared nodes (Task 29's bookkeeping).
    pub fn nodes(&self) -> std::cell::RefMut<'_, A11yNodes> {
        self.inner.borrow_mut()
    }
}

/// What one frame declares; the element builds it in `request_layout`, only while assistive
/// technology is active (`EditorView::a11y_frame`).
#[derive(Debug, Clone)]
pub struct A11yFrame {
    pub snapshot: Snapshot,
    /// The editor's accessible label, evaluated when the UI language changes, not per frame.
    pub label: SharedString,
    /// `a11y_window_lines` (D-073: 100), `a11y_optimized`, `read_only`.
    pub config: EditorConfig,
    /// T1 ("Reduced mode") disables the optimised window growth.
    pub tier: Tier,
    pub focused: bool,
}

/// The editor element's role (`Element::a11y_role`; Task 29: `MultilineTextInput`). `None`
/// leaves the element out of the tree. Task 1b: `None`.
pub fn role(a: &A11yState, frame: &A11yFrame) -> Option<Role> {
    let _ = (a, frame);
    None
}

/// The editor node's properties (`Element::write_a11y_info`; Task 29: the label, the text
/// selection, read-only). Called only when [`role`] is `Some`. Task 1b: nothing.
pub fn write_info(a: &A11yState, frame: &A11yFrame, node: &mut accesskit::Node) {
    let _ = (a, frame, node);
}

/// The editor node's children (`Element::a11y_synthetic_children`; Task 29: the ±100-line run
/// window with stable ids, and a polite live region holding [`A11yNodes::pending`]). Task 1b:
/// nothing.
pub fn synthetic_children(a: &A11yState, frame: &A11yFrame, builder: &mut A11ySubtreeBuilder) {
    let _ = (a, frame, builder);
}

/// A polite live announcement (execution status, "Run would execute lines 3–7"): queued for
/// the next frame's live-region child ([`synthetic_children`], Task 29), at most
/// [`MAX_PENDING_ANNOUNCEMENTS`].
pub fn announce(a: &A11yState, text: SharedString) {
    let mut nodes = a.nodes();
    if nodes.pending.len() == MAX_PENDING_ANNOUNCEMENTS {
        nodes.pending.pop_front();
    }
    nodes.pending.push_back(text);
}
