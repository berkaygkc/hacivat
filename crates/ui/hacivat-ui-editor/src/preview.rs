//! The Run preview band (UI-EDIT-21; Task 28): queried 50 ms after the last change or move; no
//! band from a stale (`None`) answer; the chooser's override wins; frozen while composing.
//!
//! Task 1b: the state keeps the last preview and the override; `paint_band` paints nothing.

use std::ops::Range;

use hacivat_ui_core::gpui_surface::{App, Window};
use smallvec::SmallVec;

use crate::element::VisibleRows;
use crate::service::ExecPreview;

/// The band's module-owned state (P11; `PreviewState` stays the exported verdict).
#[derive(Debug, Clone, Default)]
pub struct BandState {
    /// The last preview the service answered at the editor's version.
    pub last: Option<ExecPreview>,
    /// The chooser's hover override.
    pub override_ranges: Option<SmallVec<[Range<usize>; 2]>>,
    /// No update while an IME composition is open.
    pub frozen: bool,
}

/// Sets (or, with `None`, clears) the chooser's override.
pub fn set_override(s: &mut BandState, ranges: Option<SmallVec<[Range<usize>; 2]>>) {
    s.override_ranges = ranges;
}

/// Paints the band over `rows` (Task 28). Task 1b: nothing.
pub fn paint_band(p: &BandState, rows: &VisibleRows, window: &mut Window, cx: &mut App) {
    let _ = (p, rows, window, cx);
}
