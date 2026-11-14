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
//! rows in a frame (a thumb jump counts as infinite for one frame), cached shapes paint as
//! usual, an uncached cell paints its ASCII-safe prefix of at most 24 characters as one run,
//! chips are skipped, and the frame shapes at most [`FAST_SHAPES_PER_FRAME`] layouts. A frame
//! that painted anything below full quality asks for the next frame, which is full quality as
//! soon as the velocity is back under the threshold: one frame after the scroll slows (≈ 8 ms at
//! 120 Hz), well inside the 100 ms the plan allows.

mod grid;
pub mod layers;
mod pointer;
pub mod scrollbar;
pub mod text_cache;
pub mod truncate;

#[cfg(feature = "test-support")]
use hacivat_ui_core::gpui_surface::SharedString;
use hacivat_ui_core::gpui_surface::{Bounds, Pixels};

#[cfg(feature = "test-support")]
use crate::source::ColId;
use text_cache::{FrameLru, Shaped, TEXT_CACHE_CAPACITY, TextKey};

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
    /// Layouts shaped (cache misses, a truncation's second shape included).
    pub shapes: u32,
    /// Body cells painted (text, a marker or a placeholder bar).
    pub cells: u32,
    /// Cells painted below full quality: a fast-scroll prefix, or nothing because the frame's
    /// shaping budget was spent.
    pub degraded: u32,
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
    /// The painted text, controls shown as their chips, before any ellipsis.
    pub text: SharedString,
    /// The control chips' texts, in order.
    pub chips: Vec<SharedString>,
    pub truncated: bool,
    pub degraded: bool,
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
    pub log: FrameLog,
    #[cfg(feature = "bench")]
    pub bench: BenchStats,
    #[cfg(feature = "bench")]
    pub prepaint_started: Option<std::time::Instant>,
    #[cfg(feature = "test-support")]
    pub recorded: Option<Vec<PaintedCell>>,
}

impl PaintState {
    pub fn new() -> Self {
        PaintState {
            cache: FrameLru::new(TEXT_CACHE_CAPACITY),
            roles: None,
            scale: 0.0,
            scratch: text_cache::Scratch::default(),
            log: FrameLog::default(),
            #[cfg(feature = "bench")]
            bench: BenchStats::default(),
            #[cfg(feature = "bench")]
            prepaint_started: None,
            #[cfg(feature = "test-support")]
            recorded: None,
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
    }

    /// The cells the last recorded frame painted (`test-support`).
    #[cfg(feature = "test-support")]
    pub fn painted_cells(&self) -> &[PaintedCell] {
        self.paint.recorded.as_deref().unwrap_or(&[])
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
