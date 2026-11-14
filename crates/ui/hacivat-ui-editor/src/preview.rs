//! The Run preview band (UI-EDIT-21; Task 28): queried 50 ms after the last change or move; no
//! band from a stale (`None`) answer; the chooser's override wins; frozen while composing.
//!
//! Task 1b: the state keeps the last preview and the override; `paint_band` paints nothing;
//! the refresh seams (`on_update`, `on_composition`) are called by the view since fix round 1
//! and do nothing yet; `current_statement` is `None`.

use std::ops::Range;

use hacivat_text::Snapshot;
use hacivat_ui_core::gpui_surface::{App, Context, Window};
use smallvec::SmallVec;

use crate::element::VisibleRows;
use crate::service::ExecPreview;
use crate::view::EditorView;

/// The band's module-owned state (P11; `PreviewState` stays the exported verdict). Task 28 adds
/// its own fields (the 50 ms timer, the version asked); keep `Default`.
#[derive(Debug, Clone, Default)]
pub struct BandState {
    /// The last preview the service answered at the editor's version.
    pub last: Option<ExecPreview>,
    /// The chooser's hover override.
    pub override_ranges: Option<SmallVec<[Range<usize>; 2]>>,
    /// No update while an IME composition is open.
    pub frozen: bool,
}

/// Why the band may need a fresh preview (the view's call points).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Refresh {
    /// A committed change of the text.
    Changed,
    /// The selection or a caret moved, the text unchanged.
    SelectionMoved,
    /// The language service's analysis of a newer version landed (`LanguageService::on_update`).
    AnalysisLanded,
    /// The document context changed (connected, tier).
    ContextChanged,
}

/// Called by the view after every commit, analysis and context change (Task 28: restart the
/// 50 ms timer, then ask `exec_preview` at the editor's version; an answer for an older
/// version, or `None`, draws no band; nothing while `frozen`). Task 1b: nothing.
pub fn on_update(v: &mut EditorView, why: Refresh, cx: &mut Context<EditorView>) {
    let _ = (v, why, cx);
}

/// Called by the view when an IME composition opens or closes (Task 28: freeze the band while
/// composing, refresh when it closes). Task 1b: records the freeze only.
pub fn on_composition(s: &mut BandState, composing: bool) {
    s.frozen = composing;
}

/// The lines (0-based, end exclusive) of the statement at the caret, for the gutter's
/// current-statement bar and the scrollbar marker (Task 28, from the service's statements).
/// Task 1b: none.
pub fn current_statement(s: &BandState, snap: &Snapshot) -> Option<Range<u32>> {
    let _ = (s, snap);
    None
}

/// Sets (or, with `None`, clears) the chooser's override.
pub fn set_override(s: &mut BandState, ranges: Option<SmallVec<[Range<usize>; 2]>>) {
    s.override_ranges = ranges;
}

/// Paints the band over `rows`, mapping its byte ranges to lines through `snap` (Task 28).
/// Task 1b: nothing.
pub fn paint_band(
    p: &BandState,
    rows: &VisibleRows,
    snap: &Snapshot,
    window: &mut Window,
    cx: &mut App,
) {
    let _ = (p, rows, snap, window, cx);
}
