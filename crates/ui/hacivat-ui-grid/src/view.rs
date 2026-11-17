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

use hacivat_ui::text::{Label, TextFn, TextSource};
use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, AppContext as _, Context, Div, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Pixels, Point, Render, Role, ScrollEvent,
    SharedString, Stateful, StatefulInteractiveElement, Styled, Subscription, Task, Window, div,
    point, px,
};
use hacivat_ui_core::theme::{Theme, popover_surface};

use crate::element::{self, FrameLog, FrameStats, Geometry, PaintState};
use crate::model::{self, ColumnLayout, KeyCtx, KeyOutcome, ScrollState, SortSpec, ViewMap};
use crate::source::{
    ColId, ColumnSpec, GRID_CONTEXT, GridDataSource, ModelCell, ModelRow, RowCount,
};

/// The grid's own text (the app passes Fluent text).
#[derive(Clone)]
pub struct GridLabels {
    /// A placeholder row's label.
    pub loading: TextFn,
    /// More than 2,000 columns.
    pub too_many_columns: TextFn,
    // ## m3a-t30-results-view (ledger: the "No rows" text is built in Task 30)
    /// A finished result without rows: "No rows" (12.5 `text-3`) in the body, under the header.
    pub no_rows: TextFn,
    // ## m3c-t35
    /// A frozen column's header tooltip: "Frozen: stays in place when you scroll sideways".
    pub frozen_tip: TextFn,
    /// The resize tag's hint after the width: "Drag to resize · double-click to fit".
    pub resize_tip: TextFn,
    /// A column width in the resize tag: "136 px" (a Fluent number with its unit).
    pub width: WidthFn,
}

// ## m3c-t35
/// Text for a width in logical px (the resize tag's "136 px").
pub type WidthFn = Rc<dyn Fn(f32, &App) -> SharedString>;

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
    /// Sizes the columns from sampled rows once the first rows are loaded (HV-GRID-04; module
    /// docs of `element::autosize`). Off: columns keep their source's default widths.
    pub auto_size: bool,
    pub labels: GridLabels,
    /// Role `Grid` with this label (R-M3-32).
    pub a11y_label: TextFn,
    // ## m3b-t01b-seams
    /// Rows as columns (Task 38's Transposed view; `GridView::set_transposed` sets it, Task
    /// 38's `element/transpose.rs` hook swaps the axes). Off: M3a.
    pub transposed: bool,
}

impl std::fmt::Debug for GridConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GridConfig")
            .field("row_height", &self.row_height)
            .field("zebra", &self.zebra)
            .field("auto_size", &self.auto_size)
            .field("transposed", &self.transposed)
            .finish_non_exhaustive()
    }
}

/// A grid command (keymap rows `grid.*`, the app's commands).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GridCommand {
    Move {
        dir: model::Dir,
        extend: bool,
    },
    JumpEdge {
        dir: model::Dir,
        extend: bool,
    },
    Home {
        extend: bool,
    },
    End {
        extend: bool,
    },
    FirstRow {
        extend: bool,
    },
    LastRow {
        extend: bool,
    },
    Page {
        up: bool,
        extend: bool,
    },
    NextCell {
        back: bool,
    },
    SelectAll,
    SelectRow,
    SelectColumn,
    Copy {
        headers: bool,
    },
    Escape,
    // ## m3b-t01b-seams: Task 35 fills grow/shrink, freeze/unfreeze and auto-fit; Find,
    // GotoRow, OpenValue and ContextMenu tell the host (`GridEvent`), which Tasks 37, 38 and 39
    // answer.
    /// Cell → row segment (the on-screen columns around the focus) → row → all (## m3c-t35).
    GrowSelection,
    /// The grow steps back down to the focus cell.
    ShrinkSelection,
    Find,
    GotoRow,
    OpenValue,
    ContextMenu,
    /// Fits the selected whole columns, else the focus column, to their sampled cells.
    AutoFit,
    /// Freezes the columns up to and including the focus column (refused when they would leave
    /// less than `MIN_WIDTH` of the body to scroll).
    Freeze,
    Unfreeze,
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
        /// The source generation the region was cut from (the frame the grid last painted).
        /// The host copies only while its source is still at this generation: a re-run that
        /// arrived since has other rows under the same positions.
        generation: u64,
    },
    CopyRefused(model::CopyRefusal),
    ColumnResized {
        col: ColId,
        width: f32,
    },
    // ## m3b-t01b-seams
    /// Sort by these keys (Task 36 sorts and publishes `ViewMap::Permuted`).
    SortRequested(SortSpec),
    /// `grid.find` (Task 37 opens the find bar).
    FindRequested,
    /// `grid.goto_row` (Task 37 opens the go-to field).
    GotoRequested,
    /// `grid.open_value` on the focus cell (Task 38 opens the value viewer).
    ValueRequested(ModelCell),
    /// `grid.context_menu` (Shift+F10: under the focus cell) or a right click (Task 39's menu).
    ContextMenuRequested {
        at: Point<Pixels>,
        cell: Option<ModelCell>,
        header: Option<ColId>,
    },
    /// The frozen column count changed (Task 35): `grid.freeze`, `grid.unfreeze`, never
    /// `GridView::set_frozen` (the host's own change).
    FrozenChanged(usize),
    // ## m3c-t35 fix round 1 (M3)
    /// `grid.freeze` was refused: the columns up to the focus would leave less than
    /// `MIN_WIDTH` of the body to scroll (the host says so).
    FreezeRefused,
}

// ## m3c-t35
/// While the pointer drags a selection past the body's edge and stays still, the grid scrolls
/// on at this interval (ledger 213).
pub const AUTOSCROLL_TICK: Duration = Duration::from_millis(16);

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
    /// The sampled column auto-size of the current column set.
    pub(crate) autosize: element::AutosizeState,
    visible: VisibleState,
    /// The label was read this many times (only while assistive technology is active).
    a11y_reads: u64,
    // ## m3b-t01b-seams
    /// The view map the host set (`set_view_map`); `None`: the identity over the loaded rows.
    /// The frame, the keys and the pointer read it (## m3c-t35).
    view_map: Option<ViewMap>,
    /// The identity over the loaded rows, kept for [`GridView::view_map`] while no map is set.
    identity_map: ViewMap,
    /// The cells a find marks (Task 37), as model row and column.
    pub(crate) highlights: Option<Rc<dyn Fn(ModelRow, ColId) -> bool>>,
    // ## m3c-t35
    /// The edge autoscroll's timer while a drag rests past the body's edge.
    pub(crate) autoscroll: Option<Task<()>>,
    // ## m3c-t35 fix round 1 (I3)
    /// Blur and window-deactivation subscriptions that end a drag (set by the first frame).
    pub(crate) drag_hooks: Option<(Subscription, Subscription)>,
    // ## m3c-t36
    /// The sort the header shows (`set_sort`).
    pub(crate) sort: SortShown,
}

// ## m3c-t36
/// The sort the header shows: the keys, each key's tooltip, and the level numbers (two or more
/// keys only, NOTES M3c-19), shaped once per `set_sort`.
#[derive(Clone, Default)]
pub(crate) struct SortShown {
    pub spec: SortSpec,
    pub tips: Vec<TextFn>,
    pub levels: Vec<SharedString>,
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
        let mut autosize = element::AutosizeState::default();
        autosize.restart();
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
            autosize,
            visible: VisibleState::default(),
            a11y_reads: 0,
            view_map: None,
            identity_map: ViewMap::identity(0),
            highlights: None,
            autoscroll: None,
            drag_hooks: None,
            sort: SortShown::default(),
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

    /// Takes `columns`, keeping the widths when asked and the names match. A column set whose
    /// widths are not kept is auto-sized anew.
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
            // ## m3c-t35: the same columns keep their frozen ones.
            layout.set_frozen(self.layout.frozen());
        } else {
            self.autosize.restart();
        }
        self.columns = columns;
        self.layout = layout;
        self.scroll.set_content_width(self.layout.scroll_width());
    }

    /// Runs `cmd`.
    pub fn dispatch(&mut self, cmd: GridCommand, window: &mut Window, cx: &mut Context<Self>) {
        // ## m3b-t01b-seams: the M3b commands that only tell the host, and Task 35's.
        if self.dispatch_m3b(cmd, cx) {
            return;
        }
        // ## m3c-t35: keys move through the view map (an `Arc` clone, no copy of the rows).
        let map = self.view_map().clone();
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
                Ok(region) => cx.emit(GridEvent::CopyRequested {
                    region,
                    headers,
                    generation: self.data.generation.unwrap_or(0),
                }),
                Err(refusal) => cx.emit(GridEvent::CopyRefused(refusal)),
            },
        }
    }

    // ## m3b-t01b-seams
    /// The M3b commands: `true` when `cmd` is one (handled here, never by the key model).
    fn dispatch_m3b(&mut self, cmd: GridCommand, cx: &mut Context<Self>) -> bool {
        match cmd {
            GridCommand::Find => cx.emit(GridEvent::FindRequested),
            GridCommand::GotoRow => cx.emit(GridEvent::GotoRequested),
            GridCommand::OpenValue => {
                if let Some(cell) = self.selection.focus() {
                    cx.emit(GridEvent::ValueRequested(cell));
                }
            }
            GridCommand::ContextMenu => {
                let cell = self.selection.focus();
                cx.emit(GridEvent::ContextMenuRequested {
                    at: self.menu_point(cell),
                    cell,
                    header: None,
                });
            }
            // ## m3c-t35
            GridCommand::GrowSelection | GridCommand::ShrinkSelection => {
                let grow = cmd == GridCommand::GrowSelection;
                self.grow_or_shrink(grow, cx);
            }
            GridCommand::AutoFit => self.auto_fit(cx),
            GridCommand::Freeze => self.freeze_to_focus(cx),
            GridCommand::Unfreeze => {
                if self.layout.frozen() > 0 {
                    self.set_frozen(0, cx);
                    cx.emit(GridEvent::FrozenChanged(0));
                }
            }
            _ => return false,
        }
        true
    }

    // ## m3c-t35
    /// Grows (or shrinks) the selection one step; the row segment is the pane holding the
    /// focus: the frozen columns, or the columns after them (never the horizontal scroll).
    fn grow_or_shrink(&mut self, grow: bool, cx: &mut Context<Self>) {
        let map = self.view_map().clone();
        let order = self.layout.visible().to_vec();
        let segment = self.focus_segment();
        let before_focus = self.selection.focus();
        let changed = if grow {
            self.selection.grow(&map, &order, segment)
        } else {
            self.selection.shrink(&map, &order, segment)
        };
        if changed {
            self.selection_dirty = true;
            if self.selection.focus() != before_focus {
                if let Some(focus) = self.selection.focus() {
                    self.reveal(focus);
                }
                cx.emit(GridEvent::FocusChanged(self.selection.focus()));
            }
            cx.notify();
        }
    }

    /// The pane of visible columns that holds the focus column (## m3c-t35 fix round 1, M2:
    /// ruling 2 as the controller changed it): the frozen columns, or every column after them.
    /// With nothing frozen it is the whole row, so the grow ladder skips the segment step.
    fn focus_segment(&self) -> Range<usize> {
        let frozen = self.layout.frozen_shown();
        let at = self
            .selection
            .focus()
            .and_then(|f| self.layout.index_of(f.col));
        match at {
            Some(ix) if ix < frozen => 0..frozen,
            _ => frozen..self.layout.len(),
        }
    }

    /// `grid.auto_fit`: the selected whole columns, else the focus column, fitted in the coming
    /// frames (`element::autosize`).
    fn auto_fit(&mut self, cx: &mut Context<Self>) {
        let mut queued = false;
        if !self.selection.is_all() {
            let order: Vec<ColId> = self.layout.visible().to_vec();
            for col in order
                .into_iter()
                .filter(|c| self.selection.col_selected(*c))
            {
                self.autosize.queue_fit(col);
                queued = true;
            }
        }
        if !queued && let Some(focus) = self.selection.focus() {
            self.autosize.queue_fit(focus.col);
            queued = true;
        }
        if queued {
            cx.notify();
        }
    }

    /// `grid.freeze`: the columns up to and including the focus column, when they leave
    /// `MIN_WIDTH` of the last frame's body to scroll; else the host hears why
    /// ([`GridEvent::FreezeRefused`], ## m3c-t35 fix round 1, M3). Without a focus there is
    /// nothing to freeze up to.
    fn freeze_to_focus(&mut self, cx: &mut Context<Self>) {
        let Some(ix) = self
            .selection
            .focus()
            .and_then(|f| self.layout.index_of(f.col))
        else {
            return;
        };
        let n = ix + 1;
        let body_w = self
            .geometry
            .as_ref()
            .map_or(0.0, |g| f32::from(g.body.size.width));
        if n == self.layout.frozen() {
            return;
        }
        if self.layout.frozen_fitting(n, body_w) < n {
            cx.emit(GridEvent::FreezeRefused);
            return;
        }
        self.set_frozen(n, cx);
        cx.emit(GridEvent::FrozenChanged(n));
    }

    /// Freezes the first `n` visible columns (`0`: none), as the host restores or sets them
    /// (Task 39's menu). No event: the host knows. A count that later stops fitting a narrower
    /// pane is kept; the frame shows the frozen columns that fit
    /// (`ColumnLayout::frozen_shown`).
    pub fn set_frozen(&mut self, n: usize, cx: &mut Context<Self>) {
        if self.layout.frozen() != n.min(self.layout.len()) {
            self.layout.set_frozen(n);
            self.scroll.set_content_width(self.layout.scroll_width());
            cx.notify();
        }
    }

    /// The frozen column count the user set (`ColumnLayout::frozen`).
    pub fn frozen(&self) -> usize {
        self.layout.frozen()
    }

    /// The resize tag: the column whose right edge the pointer is on (or drags) and its width.
    /// The view draws it under the header edge ("136 px · Drag to resize · double-click to
    /// fit").
    pub fn resize_tag(&self) -> Option<(ColId, f32)> {
        if let Some(resizing) = self.pointer.resizing() {
            return Some(resizing);
        }
        if self.pointer.drag.is_some() {
            return None;
        }
        let ix = self.pointer.border?;
        Some((*self.layout.visible().get(ix)?, self.layout.width_at(ix)))
    }

    /// Where a keyboard context menu opens: under the focus cell of the last frame, clamped
    /// into the body; the body's corner without a focus; the origin before the first frame.
    fn menu_point(&self, cell: Option<ModelCell>) -> Point<Pixels> {
        let Some(g) = self.geometry.as_ref() else {
            return Point::default();
        };
        let at = cell.and_then(|cell| {
            let ix = self.layout.index_of(cell.col)?;
            let x = f32::from(g.body.left()) + self.layout.screen_x(ix, self.scroll.left_x) as f32;
            let row = self.view_row_of(cell.row);
            let y = f32::from(g.body.top()) + self.scroll.row_top(row) as f32 + g.row_h;
            Some((x, y))
        });
        let (x, y) = at.unwrap_or((f32::from(g.body.left()), f32::from(g.body.top())));
        let x = x.clamp(f32::from(g.body.left()), f32::from(g.body.right()));
        let y = y.clamp(f32::from(g.body.top()), f32::from(g.body.bottom()));
        point(px(x), px(y))
    }

    /// Sets the view map (a local sort, Task 36; filter rows, Task 37); `ViewMap::Identity`
    /// goes back to the source order.
    ///
    /// ## m3c-t35: the frame, the keys and the pointer read it. The selection follows (digest
    /// 04 R15): the focus and the row and column sets stay with their data, rectangles (view
    /// rows of the old order) clear (`Selection::on_view_map_changed`). A sort's map follows the
    /// rows that arrive after it (they join its unsorted tail, `ViewMap::with_loaded`). **A
    /// re-run drops the map** (the new rows have no order yet): the host publishes a new one.
    pub fn set_view_map(&mut self, map: ViewMap, cx: &mut Context<Self>) {
        let map = match map {
            ViewMap::Identity { .. } => None,
            ViewMap::Permuted { .. } => Some(map.with_loaded(self.data.count.loaded)),
            filtered @ ViewMap::Filtered { .. } => Some(filtered),
        };
        if map == self.view_map {
            return;
        }
        self.view_map = map;
        let map = self.view_map().clone();
        self.selection.on_view_map_changed(&map);
        self.data.rows = self.scroll_rows();
        self.selection_dirty = true;
        cx.notify();
    }

    /// The view map: the one set, else the identity over the loaded rows.
    pub fn view_map(&self) -> &ViewMap {
        self.view_map.as_ref().unwrap_or(&self.identity_map)
    }

    /// Follows the loaded rows in the view map [`GridView::view_map`] answers (each frame's
    /// data sync): the identity's count, and a sort's unsorted tail.
    pub(crate) fn note_loaded(&mut self, loaded: u64) {
        if self.identity_map.len() != loaded {
            self.identity_map = ViewMap::identity(loaded);
        }
        // ## m3c-t35
        if let Some(map @ ViewMap::Permuted { .. }) = self.view_map.as_ref()
            && map.len() != loaded
        {
            self.view_map = Some(map.with_loaded(loaded));
        }
    }

    // ## m3c-t35
    /// A re-run's new rows have no order yet: back to the source order.
    pub(crate) fn drop_view_map(&mut self) {
        self.view_map = None;
        // ## m3c-t36: and no sort indicator (the host sorts again only when asked).
        self.sort = SortShown::default();
    }

    // ## m3c-t36
    /// Shows `spec`'s indicators in the header (NOTES surface 18): a 12 px blue arrow after the
    /// sorted columns' names (up ascending, down descending) and, with two or more keys, the
    /// level number; `tips[i]` is key `i`'s tooltip ("Sort level 1, ascending. Text compares
    /// by the column's collation, tr-x-icu."). The host calls it when it publishes the order
    /// (`set_view_map`); an empty spec clears them. A re-run clears them with the map.
    pub fn set_sort(&mut self, spec: SortSpec, tips: Vec<TextFn>, cx: &mut Context<Self>) {
        let levels = if spec.keys.len() >= 2 {
            (1..=spec.keys.len())
                .map(|l| SharedString::from(l.to_string()))
                .collect()
        } else {
            Vec::new()
        };
        self.sort = SortShown { spec, tips, levels };
        cx.notify();
    }

    /// The sort the header shows.
    pub fn sort(&self) -> &SortSpec {
        &self.sort.spec
    }

    /// Each sort key's header tooltip, as the host set them.
    pub fn sort_tips(&self) -> &[TextFn] {
        &self.sort.tips
    }

    /// Asks the host to sort by `spec` (`GridEvent::SortRequested`; an empty spec clears the
    /// sort): the sort commands and the context menu (Task 39) come through here.
    pub fn request_sort(&mut self, spec: SortSpec, cx: &mut Context<Self>) {
        cx.emit(GridEvent::SortRequested(spec));
    }

    /// The view row of the first row of a sort's unsorted tail (rows that arrived after the
    /// sort), when it has one: the dashed boundary line sits above it.
    pub fn tail_boundary(&self) -> Option<u64> {
        match self.view_map.as_ref()? {
            ViewMap::Permuted {
                order,
                tail_from,
                loaded,
                ..
            } if loaded > tail_from => Some(order.len() as u64),
            _ => None,
        }
    }

    /// The rows the grid scrolls over: a filter's kept rows; else the exact total when known,
    /// the skeleton rows while the first rows are arriving, the loaded rows otherwise.
    pub(crate) fn scroll_rows(&self) -> u64 {
        match self.view_map.as_ref() {
            Some(map @ ViewMap::Filtered { .. }) => map.len(),
            _ => element::scroll_rows(&self.data.count),
        }
    }

    /// The view row that shows model row `row` (its own number when the map hides it).
    pub(crate) fn view_row_of(&self, row: ModelRow) -> u64 {
        self.view_map().model_to_view(row).map_or(row.0, |v| v.0)
    }

    /// Shows rows as columns (Task 38's Transposed view): stored in the config; Task 38's
    /// `element/transpose.rs` hook swaps the axes at render.
    pub fn set_transposed(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.config.transposed != on {
            self.config.transposed = on;
            cx.notify();
        }
    }

    /// Whether the grid shows rows as columns.
    pub fn transposed(&self) -> bool {
        self.config.transposed
    }

    /// Marks the cells `cells` answers `true` for (a find's matches, Task 37); `None` clears.
    /// Stored only: no frame paints them yet (the painting task is open, Task 1b report R3).
    pub fn set_highlights(
        &mut self,
        cells: Option<Rc<dyn Fn(ModelRow, ColId) -> bool>>,
        cx: &mut Context<Self>,
    ) {
        self.highlights = cells;
        cx.notify();
    }

    /// The cells a find marks, as last set.
    pub fn highlights(&self) -> Option<&Rc<dyn Fn(ModelRow, ColId) -> bool>> {
        self.highlights.as_ref()
    }

    /// Scrolls `cell` into view, both ways: its view row (## m3c-t35), and its column unless it
    /// is frozen (a scrolling column comes out from under the frozen ones).
    pub(crate) fn reveal(&mut self, cell: ModelCell) {
        self.scroll.scroll_into_view(self.view_row_of(cell.row));
        if let Some(ix) = self.layout.index_of(cell.col)
            && ix >= self.layout.frozen_shown()
        {
            let fw = self.layout.frozen_width();
            self.scroll
                .scroll_x_into_view(self.layout.x_of(ix) - fw, self.layout.x_of(ix + 1) - fw);
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
    /// bounds; `false` for an unknown column. Like a width the user dragged to, it is kept: the
    /// auto-size never changes it.
    pub fn set_column_width(&mut self, col: ColId, width: f32, cx: &mut Context<Self>) -> bool {
        let set = self
            .layout
            .set_width(col, model::autosize::clamp_user_width(width));
        if set {
            self.autosize.mark_user_sized(col);
            self.scroll.set_content_width(self.layout.scroll_width());
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

    // ## m3a-t30-results-view: the node moves to the focused element (ledger T30 note).
    /// Makes `el`, the element that tracks the grid's focus (the view's root), the grid's
    /// accessibility node while assistive technology is `active`: role `Grid`, the label, and
    /// the row and column counts of the last frame (R-M3-32). GPUI gives the platform focus to
    /// the node of the element that tracks the focus, so the node must sit there, not on the
    /// inner grid element (Task 29 moved the editor's the same way). Inactive: `el` unchanged,
    /// and the label is not read.
    pub fn a11y_node(&mut self, el: Stateful<Div>, active: bool, cx: &App) -> Stateful<Div> {
        if !active {
            return el;
        }
        let (role, label) = self.a11y_declaration(cx);
        // ## m3c-t35 fix round 1 (M7): a filter announces the rows it keeps.
        let rows = match self.view_map.as_ref() {
            Some(map @ ViewMap::Filtered { .. }) => map.len(),
            _ => self.data.known_total().unwrap_or(self.data.count.loaded),
        };
        el.role(role)
            .aria_label(label)
            .aria_row_count(usize::try_from(rows).unwrap_or(usize::MAX))
            .aria_column_count(self.layout.len())
    }

    /// Whether the last frame showed a result known to have no rows (its header stays): an exact
    /// total of 0 (## m3a-t30 fix round 1, I4). A result that stopped before its first row
    /// (cancelled, closed) has no exact total, and nobody knows it is empty.
    pub fn shows_no_rows(&self) -> bool {
        self.data.generation.is_some()
            && !self.columns.is_empty()
            && self.data.count.exact_total == Some(0)
    }

    /// "No rows" over the body's first row, while [`shows_no_rows`](Self::shows_no_rows).
    /// The element notifies the view when that changes (`sync_data`), so the label follows
    /// the frame it belongs to.
    fn no_rows_label(&self, cx: &App) -> Option<AnyElement> {
        if !self.shows_no_rows() {
            return None;
        }
        let g = self.geometry.as_ref()?;
        let t = Theme::get(cx).tokens;
        let left = g.body.left() - g.bounds.left();
        let top = g.body.top() - g.bounds.top();
        Some(
            div()
                .absolute()
                .left(left + px(element::layers::CELL_PAD_PX))
                .top(top)
                .h(px(self.scroll.row_height()))
                .flex()
                .items_center()
                .child(Label::new(
                    (self.config.labels.no_rows)(cx),
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    t.colors.text_3.to_hsla(),
                ))
                .into_any_element(),
        )
    }

    /// The headers whose names the last frame ellipsized, each with its tooltip's area: the
    /// whole name shows in a tooltip there.
    pub fn header_tips(&self) -> &[element::HeaderTip] {
        &self.paint.header_tips
    }

    /// Whether the column auto-size still has columns to measure.
    pub fn auto_size_pending(&self) -> bool {
        self.config.auto_size && self.autosize.pending()
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let grid = element::GridElement::new(cx.entity());
        // An ellipsized header name keeps its whole text in a tooltip (NOTES rule 7): one hover
        // area per such header of the last frame (a frame whose headers differ asks for this
        // render again). The areas take no click: the element's own hitbox handles the pointer.
        // ## m3c-t35: a frozen column's header says so ("Frozen: stays in place when you scroll
        // sideways"), after its whole name when that is ellipsized.
        let origin = self.geometry.as_ref().map(|g| g.bounds.origin);
        let frozen_tip = self.config.labels.frozen_tip.clone();
        // ## m3c-t36: a sorted column's header names its level, direction and collation.
        let sort_tips = self.sort.tips.clone();
        let tips: Vec<_> = self
            .paint
            .header_tips
            .iter()
            .filter_map(|tip| {
                let origin = origin?;
                let name = tip.ellipsized.then(|| tip.name.clone());
                let frozen = tip.frozen.then(|| frozen_tip.clone());
                let sorted = tip.sort.and_then(|level| sort_tips.get(level).cloned());
                Some(
                    div()
                        .id(("hv-grid-header-tip", tip.col.0 as usize))
                        .absolute()
                        .left(tip.bounds.left() - origin.x)
                        .top(tip.bounds.top() - origin.y)
                        .w(tip.bounds.size.width)
                        .h(tip.bounds.size.height)
                        .tooltip(move |_, cx| {
                            let frozen = frozen.as_ref().map(|f| f(cx));
                            let sorted = sorted.as_ref().map(|f| f(cx));
                            HeaderNameTip::view(name.clone(), frozen, sorted, cx)
                        }),
                )
            })
            .collect();
        let resize_tag = self.resize_tag_element(cx);
        let root = div()
            .id(("hv-grid-view", cx.entity_id()))
            .key_context(GRID_CONTEXT)
            .track_focus(&self.focus);
        // ## m3a-t30-results-view: the accessibility node is the element that tracks the focus.
        let root = self.a11y_node(root, window.is_a11y_active(), cx);
        let no_rows = self.no_rows_label(cx);
        root.size_full()
            .relative()
            .overflow_hidden()
            .on_scroll(cx.listener(|this, e: &ScrollEvent, _, cx| this.on_scroll(e, cx)))
            .child(grid)
            .children(tips)
            .children(no_rows)
            .children(resize_tag)
    }
}

impl GridView {
    // ## m3c-t35
    /// The resize tag (NOTES surface 18, M3c-23): under the header, beside the edge being
    /// hovered or dragged (on the side with room), the width in mono and the hint ("136 px ·
    /// Drag to resize · double-click to fit"), on the popover anatomy. Never over the header.
    fn resize_tag_element(&self, cx: &App) -> Option<AnyElement> {
        let (col, width) = self.resize_tag()?;
        let g = self.geometry.as_ref()?;
        let ix = self.layout.index_of(col)?;
        let edge = f32::from(g.body.left())
            + self.layout.screen_x(ix, self.scroll.left_x) as f32
            + self.layout.width_at(ix);
        let local = edge - f32::from(g.bounds.left());
        let grid_w = f32::from(g.bounds.size.width);
        let top = f32::from(g.header.bottom()) - f32::from(g.bounds.top()) + RESIZE_TAG_GAP_PX;
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let labels = &self.config.labels;
        // ## m3c-t35 fix round 1 (M4): the tag stays inside the grid: right of an edge in the
        // grid's left half, left of an edge in its right half (it never hangs over a
        // neighbouring pane).
        let tag = popover_surface(&theme).absolute().top(px(top));
        let tag = if local <= grid_w / 2.0 {
            tag.left(px(local + RESIZE_TAG_GAP_PX))
        } else {
            tag.right(px((grid_w - local + RESIZE_TAG_GAP_PX).max(0.0)))
        };
        Some(
            tag.px(px(8.0))
                .py(px(4.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(Label::new(
                    (labels.width)(width.round(), cx),
                    TextSource::Fluent,
                    t.type_ramp.mono_kbd,
                    t.colors.text.to_hsla(),
                ))
                .child(Label::new(
                    (labels.resize_tip)(cx),
                    TextSource::Fluent,
                    t.type_ramp.caption,
                    t.colors.text_2.to_hsla(),
                ))
                .into_any_element(),
        )
    }
}

/// The resize tag's offset from the column edge and the header's bottom.
const RESIZE_TAG_GAP_PX: f32 = 6.0;

/// The tooltip of a header: the column's whole name when it is ellipsized (catalog text, never
/// translated) and, for a frozen column (## m3c-t35), that it is frozen, on a popover surface.
/// ## m3c-t36: a sorted column's tooltip names its sort level, direction and collation.
struct HeaderNameTip {
    name: Option<SharedString>,
    frozen: Option<SharedString>,
    sorted: Option<SharedString>,
}

impl HeaderNameTip {
    fn view(
        name: Option<SharedString>,
        frozen: Option<SharedString>,
        sorted: Option<SharedString>,
        cx: &mut App,
    ) -> AnyView {
        cx.new(|_| HeaderNameTip {
            name,
            frozen,
            sorted,
        })
        .into()
    }
}

impl Render for HeaderNameTip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        popover_surface(&theme)
            .px(px(8.0))
            .py(px(4.0))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .children(self.name.clone().map(|name| {
                Label::new(
                    name,
                    TextSource::Catalog,
                    t.type_ramp.body_sm,
                    t.colors.text.to_hsla(),
                )
            }))
            .children(self.frozen.clone().map(|frozen| {
                Label::new(
                    frozen,
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    t.colors.text_2.to_hsla(),
                )
            }))
            .children(self.sorted.clone().map(|sorted| {
                Label::new(
                    sorted,
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    t.colors.text_2.to_hsla(),
                )
            }))
    }
}
