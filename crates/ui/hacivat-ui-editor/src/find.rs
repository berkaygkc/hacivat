//! Find in the editor (UI-EDIT-22 without replace; M3b Task 33): the matcher seam, the find
//! state and what the view's find methods do.
//!
//! The matcher is built by the app (R-M3-10: Turkish-tolerant `[iIİı]` and accent classes over
//! the original text, in `console/find_bar.rs`); the editor only takes the compiled
//! [`FindMatcher`] and steps through its matches.
//!
//! ## m3b-t01b-seams: the seam (Task 1b). `EditorView::{set_find, find_step,
//! select_all_matches, find_status}` delegate here in one line (P11), and the fan-out tells
//! this module of every commit ([`on_commit`]), so Task 33 never edits `view.rs`. The stubs
//! keep the matcher and find nothing: no step moves the selection, no match is selected, the
//! count is unknown.

use std::ops::Range;
use std::rc::Rc;

use hacivat_text::Committed;
use hacivat_ui_core::gpui_surface::Context;

use crate::view::EditorView;

/// A compiled find query over the document's text.
pub trait FindMatcher {
    /// The first match starting at or after byte `from`.
    fn find(&self, hay: &str, from: usize) -> Option<Range<usize>>;
    /// The last match ending at or before byte `before`.
    fn find_back(&self, hay: &str, before: usize) -> Option<Range<usize>>;
}

/// What the find panel shows ("3 of 41").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct FindStatus {
    /// The current match, 1-based.
    pub current: Option<u32>,
    /// The match count; `None` while counting.
    pub total: Option<u32>,
    /// The count stopped at the cap.
    pub capped: bool,
}

/// The find module's state in `EditorView` (P11).
#[derive(Default)]
pub struct FindState {
    pub(crate) matcher: Option<Rc<dyn FindMatcher>>,
    pub(crate) status: FindStatus,
}

impl std::fmt::Debug for FindState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FindState")
            .field("active", &self.matcher.is_some())
            .field("status", &self.status)
            .finish()
    }
}

/// Sets (or clears, `None`) the query. STUB: kept, nothing counted.
pub(crate) fn set_find(
    v: &mut EditorView,
    m: Option<Rc<dyn FindMatcher>>,
    cx: &mut Context<EditorView>,
) {
    v.find.matcher = m;
    v.find.status = FindStatus::default();
    cx.notify();
}

/// Selects the next (`back`: previous) match from the caret and answers it. STUB: `None`, the
/// selection unchanged.
pub(crate) fn find_step(
    v: &mut EditorView,
    back: bool,
    cx: &mut Context<EditorView>,
) -> Option<Range<usize>> {
    let _ = (v, back, cx);
    None
}

/// Selects every match (at most `cap`, as ranges of a multi-cursor selection) and answers how
/// many. STUB: 0, the selection unchanged.
pub(crate) fn select_all_matches(
    v: &mut EditorView,
    cap: u32,
    cx: &mut Context<EditorView>,
) -> u32 {
    let _ = (v, cap, cx);
    0
}

/// The panel's status.
pub(crate) fn status(v: &EditorView) -> FindStatus {
    v.find.status
}

/// Every commit, from the fan-out (`EditorView::apply_committed`): the matches go stale.
/// STUB: nothing to recount.
pub(crate) fn on_commit(v: &mut EditorView, committed: &Committed) {
    let _ = (v, committed);
}
