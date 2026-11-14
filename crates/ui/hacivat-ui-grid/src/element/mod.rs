//! The grid element (HV-GRID-01, -02 fast-scroll, -03; Task 20): one custom element with a
//! hitbox, the focus of its view and the accessibility declaration; regions painted body →
//! header → gutter → corner → scrollbars, each inside its content mask.
//!
//! - [`grid`]: the element and its frame (one snapshot per frame, the geometry, the visible
//!   region, `ensure_rows`, fast-scroll);
//! - [`layers`]: the paint layers (background, zebra, selection, content, hairlines, outlines,
//!   resize guide) and the text roles;
//! - [`text_cache`]: the frame-LRU of shaped cells;
//! - [`truncate`]: the width cap, the ellipsis and the visible controls;
//! - [`scrollbar`]: the tracks, thumbs and the loaded band;
//! - [`pointer`]: hit testing, clicks, drags, column resizing.
//!
//! **Fast-scroll (R-M3-12).** While the scroll moves more than [`FAST_SCROLL_ROWS_PER_FRAME`]
//! rows in a frame (a thumb jump counts as infinite for one frame):
//! - cached shapes paint as usual;
//! - an uncached cell whose text is printable ASCII within 24 characters is shaped at full
//!   quality; any other uncached cell paints its ASCII-safe prefix followed by `…`, or the
//!   skeleton bar when it has no such prefix or the prefix would end at a number's group
//!   separator ([`truncate::fast_cut`]): a prefix never reads as a different value;
//! - chips are skipped;
//! - the body shapes at most [`FAST_SHAPES_PER_FRAME`] layouts, spent **row by row from the
//!   top**, and a cell past the budget paints the **skeleton bar**: no cell is ever blank;
//! - the header is never in fast-scroll and is not charged to the body's budget.
//!
//! A frame that painted anything below full quality asks for the next frame, which is full
//! quality as soon as the velocity is back under the threshold: one frame after the scroll
//! slows.

mod autosize;
mod grid;
pub mod layers;
mod pointer;
pub mod scrollbar;
pub mod text_cache;
pub mod truncate;

use hacivat_ui_core::gpui_surface::{Bounds, Pixels, SharedString};

use crate::source::ColId;
use text_cache::{FrameLru, Shaped, TEXT_CACHE_CAPACITY, TextKey};

pub use autosize::AUTOSIZE_MEASURES_PER_FRAME;
pub(crate) use autosize::AutosizeState;
pub(crate) use grid::GridElement;
pub(crate) use pointer::PointerState;

/// Above this many rows per frame the grid paints in fast-scroll mode (R-M3-12).
pub const FAST_SCROLL_ROWS_PER_FRAME: f64 = 40.0;

/// The most layouts a fast-scroll frame shapes (R-M3-12).
pub const FAST_SHAPES_PER_FRAME: u32 = 150;

/// The scrollbar tracks' thickness, in logical px.
pub const SCROLLBAR_PX: f32 = 9.0;

/// The row-number gutter's narrowest width (up to 5 digits; one digit advance more per digit
/// beyond, NOTES open question 4).
pub const ROW_NUMBER_MIN_PX: f32 = 46.0;

/// Digits the narrowest gutter holds.
pub const ROW_NUMBER_MIN_DIGITS: u32 = 5;

/// Within this many px of a header's right edge, a drag resizes the column.
pub const RESIZE_GRAB_PX: f32 = 4.0;

/// What one frame painted.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FrameStats {
    /// Body layouts shaped (cache misses, a truncation's second shape included): what the
    /// fast-scroll budget caps.
    pub shapes: u32,
    /// Header layouts shaped (never fast-scroll, never charged to the body's budget).
    pub header_shapes: u32,
    /// Body cells painted (text, a marker or a skeleton bar).
    pub cells: u32,
    /// Cells painted below full quality: a fast-scroll prefix (ending in `…`) or the skeleton
    /// bar.
    pub degraded: u32,
    /// Loaded cells painted as the skeleton bar (the shaping budget was spent, or the text had
    /// no safe fast-scroll prefix).
    pub skeletons: u32,
    /// The frame was in fast-scroll mode.
    pub fast: bool,
    /// Rows the user moved the view since the last frame (`f32::MAX` for a thumb jump).
    pub velocity: f32,
    /// `ensure_rows` calls in this frame (at most one).
    pub ensure_rows: u32,
    /// The cell paint path's debug allocation counter (cells, headers, row numbers): one per
    /// shaped layout, one per new cache entry, one per debug record. A frame of cache hits
    /// counts 0. (Allocations inside GPUI, and the frame's few listeners, are not counted: the
    /// workspace denies the `unsafe` a counting allocator needs.)
    pub cell_allocs: u32,
    /// Cells measured for the column auto-size in this frame (at most
    /// [`AUTOSIZE_MEASURES_PER_FRAME`] plus one column's samples; 0 once the widths are set).
    pub autosize_measures: u32,
}

/// Statistics over the frames since the log was reset.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FrameLog {
    pub frames: u64,
    pub fast_frames: u64,
    pub degraded_frames: u64,
    /// The most layouts shaped in one frame.
    pub max_shapes: u32,
    /// The most layouts shaped in one fast-scroll frame.
    pub max_fast_shapes: u32,
    pub ensure_rows: u64,
    /// The cell paint path's debug allocation counter, summed.
    pub cell_allocs: u64,
    /// Cells measured for the column auto-size, summed, and the most in one frame.
    pub autosize_measures: u64,
    pub max_autosize_measures: u32,
    pub last: FrameStats,
}

impl FrameLog {
    fn push(&mut self, s: FrameStats) {
        self.frames += 1;
        self.fast_frames += u64::from(s.fast);
        self.degraded_frames += u64::from(s.degraded > 0);
        self.max_shapes = self.max_shapes.max(s.shapes);
        if s.fast {
            self.max_fast_shapes = self.max_fast_shapes.max(s.shapes);
        }
        self.ensure_rows += u64::from(s.ensure_rows);
        self.cell_allocs += u64::from(s.cell_allocs);
        self.autosize_measures += u64::from(s.autosize_measures);
        self.max_autosize_measures = self.max_autosize_measures.max(s.autosize_measures);
        self.last = s;
    }
}

/// The regions of one frame, in window coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct Geometry {
    pub bounds: Bounds<Pixels>,
    pub corner: Bounds<Pixels>,
    pub header: Bounds<Pixels>,
    pub gutter: Bounds<Pixels>,
    pub body: Bounds<Pixels>,
    pub vtrack: Bounds<Pixels>,
    pub htrack: Option<Bounds<Pixels>>,
    pub row_h: f32,
    /// Visible rows and columns (indices into the visible columns).
    pub rows: std::ops::Range<u64>,
    pub cols: std::ops::Range<usize>,
    pub scale: f32,
}

/// One cell a recorded frame painted (`test-support`).
#[cfg(feature = "test-support")]
#[derive(Debug, Clone, PartialEq)]
pub struct PaintedCell {
    pub row: u64,
    pub col: ColId,
    /// The painted text, controls shown as their chips, before any ellipsis (empty for a
    /// skeleton bar).
    pub text: SharedString,
    /// The control chips' texts, in order.
    pub chips: Vec<SharedString>,
    /// Ends in `…` (a cut text or a fast-scroll prefix).
    pub truncated: bool,
    pub degraded: bool,
    /// Painted as the skeleton bar.
    pub skeleton: bool,
    /// The bytes the painted runs cover (an ellipsis counts three).
    pub painted_bytes: usize,
}

#[cfg(feature = "test-support")]
impl PaintedCell {
    pub(crate) fn of(row: u64, col: ColId, s: &Shaped) -> Self {
        PaintedCell {
            row,
            col,
            text: s.display.clone(),
            chips: s
                .chips()
                .map(|c| SharedString::from(c.to_string()))
                .collect(),
            truncated: s.truncated,
            degraded: s.degraded,
            skeleton: false,
            painted_bytes: s.painted_bytes(),
        }
    }

    pub(crate) fn skeleton(row: u64, col: ColId) -> Self {
        PaintedCell {
            row,
            col,
            text: SharedString::default(),
            chips: Vec::new(),
            truncated: false,
            degraded: true,
            skeleton: true,
            painted_bytes: 0,
        }
    }
}

/// One header line a recorded frame painted (`test-support`).
#[cfg(feature = "test-support")]
#[derive(Debug, Clone, PartialEq)]
pub struct PaintedHeader {
    pub col: ColId,
    /// The column's name or type label.
    pub text: SharedString,
    pub truncated: bool,
    pub degraded: bool,
}

/// A header whose name the last frame ellipsized: the grid shows the whole name in a tooltip
/// over `bounds` (window coordinates).
#[derive(Debug, Clone, PartialEq)]
pub struct HeaderTip {
    pub col: ColId,
    /// The column's whole name (catalog text).
    pub name: SharedString,
    pub bounds: Bounds<Pixels>,
}

/// A visible column's paint geometry for one frame (reused buffer).
#[derive(Debug, Clone, Copy)]
pub(crate) struct ColPaint {
    pub ix: usize,
    pub col: crate::source::ColId,
    pub kind: crate::source::ColumnKind,
    pub left: f32,
    pub width: f32,
}

/// The bench's per-frame CPU samples (feature `bench`; the spike's `PaintStats`).
#[cfg(feature = "bench")]
#[derive(Debug, Clone, Default)]
pub struct BenchStats {
    /// Time inside the grid's paint, in ms.
    pub paint_ms: Vec<f64>,
    /// From the grid's prepaint to the end of its paint, in ms.
    pub frame_cpu_ms: Vec<f64>,
    pub shapes: Vec<u32>,
    pub cells: Vec<u32>,
    pub fast: Vec<bool>,
}

/// What the grid keeps across frames to paint.
pub(crate) struct PaintState {
    pub cache: FrameLru<TextKey, Shaped>,
    pub roles: Option<layers::Roles>,
    /// The scale the cache was shaped at; a change clears it.
    pub scale: f32,
    pub scratch: text_cache::Scratch,
    /// The visible columns' geometry, rebuilt each frame in a kept buffer.
    pub col_scratch: Vec<ColPaint>,
    pub log: FrameLog,
    /// The headers whose names the last painted frame ellipsized, and those of the frame
    /// before it (two kept buffers, swapped each frame).
    pub header_tips: Vec<HeaderTip>,
    pub header_tips_before: Vec<HeaderTip>,
    #[cfg(feature = "bench")]
    pub bench: BenchStats,
    #[cfg(feature = "bench")]
    pub prepaint_started: Option<std::time::Instant>,
    #[cfg(feature = "test-support")]
    pub recorded: Option<Vec<PaintedCell>>,
    #[cfg(feature = "test-support")]
    pub recorded_headers: Option<Vec<PaintedHeader>>,
}

impl PaintState {
    pub fn new() -> Self {
        PaintState {
            cache: FrameLru::new(TEXT_CACHE_CAPACITY),
            roles: None,
            scale: 0.0,
            scratch: text_cache::Scratch::default(),
            col_scratch: Vec::new(),
            log: FrameLog::default(),
            header_tips: Vec::new(),
            header_tips_before: Vec::new(),
            #[cfg(feature = "bench")]
            bench: BenchStats::default(),
            #[cfg(feature = "bench")]
            prepaint_started: None,
            #[cfg(feature = "test-support")]
            recorded: None,
            #[cfg(feature = "test-support")]
            recorded_headers: None,
        }
    }

    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }
}

// The test and bench seams of the view live here, next to the state they read.
impl crate::view::GridView {
    /// Records every painted cell from the next frame on (`test-support`).
    #[cfg(feature = "test-support")]
    pub fn set_paint_recording(&mut self, on: bool) {
        self.paint.recorded = on.then(Vec::new);
        self.paint.recorded_headers = on.then(Vec::new);
    }

    /// The cells the last recorded frame painted, skeleton bars included (`test-support`).
    #[cfg(feature = "test-support")]
    pub fn painted_cells(&self) -> &[PaintedCell] {
        self.paint.recorded.as_deref().unwrap_or(&[])
    }

    /// The header lines the last recorded frame painted (`test-support`).
    #[cfg(feature = "test-support")]
    pub fn painted_headers(&self) -> &[PaintedHeader] {
        self.paint.recorded_headers.as_deref().unwrap_or(&[])
    }

    /// The hovered body cell (view row, visible column index) and whether a pointer drag is in
    /// progress (`test-support`).
    #[cfg(feature = "test-support")]
    pub fn pointer_state(&self) -> (Option<(u64, usize)>, bool) {
        (self.pointer.hover, self.pointer.drag.is_some())
    }

    /// The bench's per-frame samples (feature `bench`).
    #[cfg(feature = "bench")]
    pub fn bench_stats(&mut self) -> &mut BenchStats {
        &mut self.paint.bench
    }

    /// The last frame's geometry.
    pub fn geometry(&self) -> Option<&Geometry> {
        self.geometry.as_ref()
    }
}
