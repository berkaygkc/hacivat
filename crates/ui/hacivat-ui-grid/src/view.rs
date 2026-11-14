//! `GridView` (HV-GRID-01, -02, -05; Task 20): the grid's view over a source, its commands and
//! events.
//!
//! - **Frames.** The view renders a focusable area (key context `Grid`) holding the grid
//!   element; the element reads one snapshot of the source per frame (`begin_frame`), applies
//!   the frame's coalesced scroll deltas, and paints only the visible rows × columns.
//! - **Data changes.** A new generation (a re-run) clears the selection and starts the scroll
//!   over (`ScrollState::on_generation_changed` and `Selection::on_generation_changed`, keeping a
//!   pending focus until the new rows arrive); an append keeps both, places a pending focus
//!   (`Selection::on_rows_loaded`, then [`GridEvent::FocusChanged`]) and follows the tail when
//!   the view was at the bottom.
//! - **Keys** reach the grid as [`GridCommand`]s through the app's commands ([`GridView::dispatch`]);
//!   the grid binds no keys itself. **The pointer** selects cells, rows (the gutter), columns
//!   (the header) and everything (the corner), drags the scrollbar thumbs and resizes columns.
//! - **Events.** [`GridEvent::SelectionChanged`] fires at most once per frame;
//!   [`GridEvent::VisibleRegion`] once the region has been still for 50 ms.

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use hacivat_ui::text::TextFn;
use hacivat_ui_core::gpui_surface::{
    App, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, Role, ScrollEvent, SharedString, Styled, Task, Window, div, px,
};

use crate::element::{self, FrameLog, FrameStats, Geometry, PaintState};
use crate::model::{self, ColumnLayout, KeyCtx, KeyOutcome, ScrollState, ViewMap};
use crate::source::{ColId, ColumnSpec, GRID_CONTEXT, GridDataSource, ModelCell, RowCount};

/// The grid's own text (the app passes Fluent text).
#[derive(Clone)]
pub struct GridLabels {
    /// A placeholder row's label.
    pub loading: TextFn,
    /// More than 2,000 columns.
    pub too_many_columns: TextFn,
}

impl std::fmt::Debug for GridLabels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GridLabels").finish_non_exhaustive()
    }
}

/// How a grid looks and is announced.
#[derive(Clone)]
pub struct GridConfig {
    /// 21 px.
    pub row_height: f32,
    /// Off in M3a.
    pub zebra: bool,
    pub labels: GridLabels,
    /// Role `Grid` with this label (R-M3-32).
    pub a11y_label: TextFn,
}

impl std::fmt::Debug for GridConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GridConfig")
            .field("row_height", &self.row_height)
            .field("zebra", &self.zebra)
            .finish_non_exhaustive()
    }
}

/// A grid command (keymap rows `grid.*`, the app's commands).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GridCommand {
    Move { dir: model::Dir, extend: bool },
    JumpEdge { dir: model::Dir, extend: bool },
    Home { extend: bool },
    End { extend: bool },
    FirstRow { extend: bool },
    LastRow { extend: bool },
    Page { up: bool, extend: bool },
    NextCell { back: bool },
    SelectAll,
    SelectRow,
    SelectColumn,
    Copy { headers: bool },
    Escape,
}

/// What a grid tells its host.
#[derive(Debug, Clone, PartialEq)]
pub enum GridEvent {
    SelectionChanged,
    FocusChanged(Option<ModelCell>),
    VisibleRegion {
        rows: Range<u64>,
        cols: Range<usize>,
    },
    CopyRequested {
        region: model::CopyRegion,
        headers: bool,
    },
    CopyRefused(model::CopyRefusal),
    ColumnResized {
        col: ColId,
        width: f32,
    },
}

/// How long the visible region stays still before [`GridEvent::VisibleRegion`] fires.
pub const VISIBLE_REGION_DEBOUNCE: Duration = Duration::from_millis(50);

/// Rows shown as skeleton bars while a result's first rows are arriving (the mock's
/// "First rows" state).
pub const SKELETON_ROWS: u64 = 8;

/// What the view knows of its source's data.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DataState {
    /// The generation of the last frame; `None` before the first frame.
    pub generation: Option<u64>,
    /// `set_source` replaced the source: the next frame is a new generation whatever its
    /// number.
    pub replaced: bool,
    pub count: RowCount,
    /// The rows the grid scrolls over: the exact total when known, the loaded rows otherwise,
    /// and the skeleton rows while the first rows are arriving.
    pub rows: u64,
}

impl DataState {
    /// The total the gutter and the scrollbar count to (`None` while unknown).
    pub fn known_total(&self) -> Option<u64> {
        self.count
            .exact_total
            .map(|t| t.max(self.count.loaded))
            .or((!self.count.more).then_some(self.count.loaded))
    }
}

/// The view's debounced visible region.
#[derive(Default)]
struct VisibleState {
    emitted: Option<(Range<u64>, Range<usize>)>,
    current: Option<(Range<u64>, Range<usize>)>,
    task: Option<Task<()>>,
}

/// One grid.
pub struct GridView {
    pub(crate) source: Rc<dyn GridDataSource>,
    pub(crate) config: GridConfig,
    pub(crate) selection: model::Selection,
    pub(crate) scroll: ScrollState,
    pub(crate) focus: FocusHandle,
    pub(crate) columns: Arc<[ColumnSpec]>,
    pub(crate) layout: ColumnLayout,
    pub(crate) data: DataState,
    pub(crate) paint: PaintState,
    /// The last frame's geometry, for the pointer and the keyboard's page size.
    pub(crate) geometry: Option<Geometry>,
    pub(crate) pointer: element::PointerState,
    /// A selection change waiting for the next frame's event.
    pub(crate) selection_dirty: bool,
    visible: VisibleState,
    /// The label was read this many times (only while assistive technology is active).
    a11y_reads: u64,
}

impl std::fmt::Debug for GridView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GridView")
            .field("config", &self.config)
            .field("selection", &self.selection)
            .field("scroll", &self.scroll)
            .finish_non_exhaustive()
    }
}

impl GridView {
    pub fn new(source: Rc<dyn GridDataSource>, config: GridConfig, cx: &mut Context<Self>) -> Self {
        let columns = source.columns();
        let layout = ColumnLayout::from_specs(&columns);
        GridView {
            scroll: ScrollState::new(config.row_height),
            source,
            config,
            selection: model::Selection::default(),
            focus: cx.focus_handle(),
            columns,
            layout,
            data: DataState::default(),
            paint: PaintState::new(),
            geometry: None,
            pointer: element::PointerState::default(),
            selection_dirty: false,
            visible: VisibleState::default(),
            a11y_reads: 0,
        }
    }

    /// Replaces the source (a re-run): the selection clears; widths stay when `keep_widths`
    /// and the new columns have the same names in the same order.
    pub fn set_source(
        &mut self,
        source: Rc<dyn GridDataSource>,
        keep_widths: bool,
        cx: &mut Context<Self>,
    ) {
        self.source = source;
        let columns = self.source.columns();
        self.adopt_columns(columns, keep_widths);
        self.data.replaced = true;
        self.selection_dirty = true;
        cx.notify();
    }

    /// Takes `columns`, keeping the widths when asked and the names match.
    pub(crate) fn adopt_columns(&mut self, columns: Arc<[ColumnSpec]>, keep_widths: bool) {
        let same_names = columns.len() == self.columns.len()
            && columns
                .iter()
                .zip(self.columns.iter())
                .all(|(a, b)| a.name == b.name && a.id == b.id);
        if Arc::ptr_eq(&columns, &self.columns) {
            return;
        }
        let mut layout = ColumnLayout::from_specs(&columns);
        if keep_widths && same_names {
            for id in self.layout.order() {
                if let Some(w) = self.layout.width_of(*id) {
                    layout.set_width(*id, w);
                }
            }
        }
        self.columns = columns;
        self.layout = layout;
        self.scroll.set_content_width(self.layout.total_width());
    }

    /// Runs `cmd`.
    pub fn dispatch(&mut self, cmd: GridCommand, window: &mut Window, cx: &mut Context<Self>) {
        let map = ViewMap::identity(self.data.count.loaded);
        let ctx = KeyCtx {
            map: &map,
            order: self.layout.visible(),
            page_rows: self.scroll.viewport_rows(),
        };
        let outcome = model::apply_command(cmd, &mut self.selection, &ctx);
        match outcome {
            KeyOutcome::Unchanged => {}
            KeyOutcome::Changed => {
                self.selection_dirty = true;
                cx.notify();
            }
            KeyOutcome::Moved { focus, follow_tail } => {
                self.reveal(focus);
                if follow_tail {
                    self.scroll.scroll_to_end();
                }
                self.selection_dirty = true;
                cx.emit(GridEvent::FocusChanged(Some(focus)));
                cx.notify();
            }
            KeyOutcome::Leave { back } => {
                if back {
                    window.focus_prev(cx);
                } else {
                    window.focus_next(cx);
                }
            }
            KeyOutcome::Copy { region, headers } => match region {
                Ok(region) => cx.emit(GridEvent::CopyRequested { region, headers }),
                Err(refusal) => cx.emit(GridEvent::CopyRefused(refusal)),
            },
        }
    }

    /// Scrolls `cell` into view, both ways.
    pub(crate) fn reveal(&mut self, cell: ModelCell) {
        self.scroll.scroll_into_view(cell.row.0);
        if let Some(ix) = self.layout.index_of(cell.col) {
            self.scroll
                .scroll_x_into_view(self.layout.x_of(ix), self.layout.x_of(ix + 1));
        }
    }

    pub fn selection(&self) -> &model::Selection {
        &self.selection
    }

    /// The scroll position.
    pub fn scroll_state(&self) -> &ScrollState {
        &self.scroll
    }

    /// The columns in view order with their widths.
    pub fn column_layout(&self) -> &ColumnLayout {
        &self.layout
    }

    /// Sets the width of `col` (the host restoring a result tab's widths), clamped to the user
    /// bounds; `false` for an unknown column.
    pub fn set_column_width(&mut self, col: ColId, width: f32, cx: &mut Context<Self>) -> bool {
        let set = self
            .layout
            .set_width(col, model::autosize::clamp_user_width(width));
        if set {
            self.scroll.set_content_width(self.layout.total_width());
            cx.notify();
        }
        set
    }

    /// Scrolls by pixels (input and benches): queued, applied with the frame's other deltas.
    pub fn scroll_px(&mut self, dy: f64, dx: f64, cx: &mut Context<Self>) {
        self.scroll.queue(dy, dx);
        cx.notify();
    }

    /// Jumps to row `row` at the top of the body (go to row, benches): a jump for fast-scroll.
    pub fn scroll_to_row(&mut self, row: u64, cx: &mut Context<Self>) {
        let max_top = self.scroll.max_top_row();
        let fraction = if max_top == 0 {
            0.0
        } else {
            row.min(max_top) as f64 / max_top as f64
        };
        self.scroll.scroll_to_fraction(fraction);
        cx.notify();
    }

    /// The grid's source.
    pub fn source(&self) -> &Rc<dyn GridDataSource> {
        &self.source
    }

    /// The last frame's paint statistics.
    pub fn frame_stats(&self) -> FrameStats {
        self.paint.log.last
    }

    /// Statistics over the frames since the last [`GridView::reset_frame_log`].
    pub fn frame_log(&self) -> &FrameLog {
        &self.paint.log
    }

    pub fn reset_frame_log(&mut self) {
        self.paint.log = FrameLog::default();
    }

    /// The most layouts shaped in one frame since the last [`GridView::reset_frame_log`].
    pub fn max_shapes_per_frame(&self) -> u32 {
        self.paint.log.max_shapes
    }

    /// The shapes the text cache holds.
    pub fn cached_shapes(&self) -> usize {
        self.paint.cache_len()
    }

    /// The accessibility declaration (R-M3-32): role `Grid` and the configured label. The
    /// element reads it only while assistive technology is active.
    pub fn a11y_declaration(&mut self, cx: &App) -> (Role, SharedString) {
        self.a11y_reads += 1;
        (Role::Grid, (self.config.a11y_label)(cx))
    }

    /// How often the label has been read (it is read only while assistive technology is
    /// active).
    pub fn a11y_label_reads(&self) -> u64 {
        self.a11y_reads
    }

    /// The selection changed: one event in the next frame.
    pub(crate) fn mark_selection_changed(&mut self) {
        self.selection_dirty = true;
    }

    /// Emits the frame's pending selection event (called once per frame from prepaint).
    pub(crate) fn flush_frame_events(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.selection_dirty) {
            cx.emit(GridEvent::SelectionChanged);
        }
    }

    /// Notes the frame's visible region; [`GridEvent::VisibleRegion`] fires once it has been
    /// still for [`VISIBLE_REGION_DEBOUNCE`].
    pub(crate) fn note_visible_region(
        &mut self,
        rows: Range<u64>,
        cols: Range<usize>,
        cx: &mut Context<Self>,
    ) {
        let region = (rows, cols);
        if self.visible.current.as_ref() == Some(&region) {
            return;
        }
        self.visible.current = Some(region);
        // Each change re-arms the timer: dropping the old task cancels it.
        let executor = cx.background_executor().clone();
        self.visible.task = Some(cx.spawn(async move |this, cx| {
            executor.timer(VISIBLE_REGION_DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| {
                this.visible.task = None;
                let current = this.visible.current.clone();
                if current.is_some() && current != this.visible.emitted {
                    this.visible.emitted = current.clone();
                    if let Some((rows, cols)) = current {
                        cx.emit(GridEvent::VisibleRegion { rows, cols });
                    }
                }
            });
        }));
    }

    fn on_scroll(&mut self, e: &ScrollEvent, cx: &mut Context<Self>) {
        // GPUI's deltas are positive when the content should move down (towards row 0).
        let d = e.delta.pixel_delta(px(self.scroll.row_height()));
        self.scroll_px(-f64::from(f32::from(d.y)), -f64::from(f32::from(d.x)), cx);
    }
}

impl Focusable for GridView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventEmitter<GridEvent> for GridView {}

impl Render for GridView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let grid = element::GridElement::new(cx.entity());
        div()
            .id(("hv-grid-view", cx.entity_id()))
            .key_context(GRID_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .overflow_hidden()
            .on_scroll(cx.listener(|this, e: &ScrollEvent, _, cx| this.on_scroll(e, cx)))
            .child(grid)
    }
}
