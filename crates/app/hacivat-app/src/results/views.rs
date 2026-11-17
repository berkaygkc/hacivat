//! The result views (HV-GRID-12; M3c Task 38): Table / Transposed / Text, and the record
//! inspector, with their commands.
//!
//! ## m3b-t01b-seams: the seam (Task 1b). Task 38 fills the bodies and their `enabled`.
//!
//! ## m3c-t38 (M3c Task 38; NOTES surface 21, M3c-28, M3c-29, M3c-30, M3c-40, M3c-42)
//! - **The view switch** (Table / Transposed / Text) sits in the result toolbar's right group,
//!   after rows · timing; its words go first when the bar is narrow. The view is per result tab
//!   (the table grid's), never chosen for the user (never auto-transposed), and kept when the
//!   user comes back to the tab. **Transposed is disabled above 100,000 shown rows**, with the
//!   reason in its tooltip; it stays focusable.
//! - **Transposed** shows the table's rows as columns through a grid of its own over
//!   `hacivat_ui_grid::TransposedSource` (the axis swap; the grid crate's `element/transpose.rs`).
//!   Its columns follow **the order the table shows** (the sort's and the filter's view map,
//!   read through the map's one owner, never set from here) and its rows the table's shown
//!   columns. It is rebuilt when the table's rows, order or columns change (not while a result
//!   streams). The table grid keeps its selection, scroll, sort and frozen columns for when the
//!   user comes back. Rulings (carries-w10 T38): the table's frozen columns are not applied (the
//!   name column is frozen instead; `transpose.rs`); sorting by a row is not offered and no sort
//!   indicator shows; **find, Filter rows and go to row are turned off while transposed** (the
//!   bar closes, the commands are disabled): their marks and rows are the table's, and a scan in
//!   the other axis would number rows the user does not see. Copy copies what the transposed
//!   grid shows, raw (the name column copies the column names).
//! - **Text** is a Pretty ASCII table of the shown rows (view order) and columns: raw text (what
//!   copy puts on the clipboard; control characters escaped so a row is one line, NULL as
//!   `NULL`), widths capped at 255 characters, numbers aligned right. It is built on the
//!   background executor with progress and Cancel; **above 100,000 rows it asks first** in place
//!   (Back to the table, Show as text). Whole lines select (click, Shift-click, ↑/↓) and copy (the
//!   platform's copy keys, the toolbar's Copy, `results.copy`): the lines as shown.
//! - **The record inspector** (closed by default; the record toggle, `grid.toggle_record`; its
//!   visibility is saved with the tile layout) shows the focused row, read as a **model** row
//!   (a sort or a filter never changes which row it reads), one field per shown column: the name
//!   (a duplicate name gets "(2)" with a tooltip), the type, the value clamped to three lines
//!   with "Show more" (the value viewer), and the raw value in the field's tooltip. ▲ ▼ and
//!   Alt+↑ / Alt+↓ (keys of the focused inspector; no keymap row) move to the previous or next
//!   row as shown. Above 200 columns the fields are virtualized (one line each). A one-row result
//!   shows the toggle with its word.

use std::cell::Cell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use hacivat_l10n::ArgValue;
use hacivat_results::Completeness;
use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::popover::{A11yNode, Declare as _, Tip};
use hacivat_ui::step_trail::run_marker;
use hacivat_ui::text::{Label, TextFn, TextSource, paragraph};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui::toolbar::ToolbarItem;
use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, AppContext as _, ClipboardItem, Context, Entity, EntityId,
    FocusHandle, Focusable as _, Global, InteractiveElement as _, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, ParentElement as _, Render, Role, ScrollEvent, SharedString,
    StatefulInteractiveElement as _, StyleRefinement, Styled as _, Subscription, Task, WeakEntity,
    Window, accesskit, canvas, div, hsla, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode, focus_outline};
use hacivat_ui_grid::model::{CopyRegion, ViewMap};
use hacivat_ui_grid::{
    ColId, ColumnKind, GRID_CONTEXT, GridCommand, GridDataSource as _, GridEvent, GridView,
    ModelCell, ModelRow, NULL_MARKER, RowRenderer, TransposedSource, ViewRow, VirtualList,
};
use hacivat_values::valfmt::{DisplayKind, Purpose, RawText, format_cell, raw_text_in};
use hacivat_values::{CellFlags, ValueRef};

use crate::console::ConsoleId;
use crate::exec_ui::marks::message;
use crate::results::adapter::{GridAdapter, RawView, Texts};
use crate::results::copy::{self, CopyOutcome, TooLarge, Tsv};
use crate::shell::Workspace;

/// The view commands: `(id, title)`.
const COMMANDS: [(&str, &str); 4] = [
    ("grid.view_table", "hv-cmd-grid-view-table"),
    ("grid.view_transposed", "hv-cmd-grid-view-transposed"),
    ("grid.view_text", "hv-cmd-grid-view-text"),
    ("grid.toggle_record", "hv-cmd-grid-toggle-record"),
];

/// The most shown rows the Transposed view takes (one column per row).
pub const TRANSPOSED_MAX_ROWS: u64 = 100_000;
/// Above this many rows the Text view asks first.
pub const TEXT_ASK_ROWS: u64 = 100_000;
/// A text-view column is at most this many characters wide.
pub const TEXT_MAX_WIDTH: usize = 255;
/// Above this many columns the record inspector's fields are virtualized.
pub const RECORD_VIRTUAL_ABOVE: usize = 200;
/// A transposed row (NOTES surface 21: 34 px rows).
const TRANSPOSED_ROW_PX: f32 = 34.0;
/// Up to this many rows a transposed grid sizes its columns from their cells.
const TRANSPOSED_AUTOSIZE_ROWS: usize = 200;
/// A text-view line (Fragment Mono 11.5/17).
const TEXT_LINE_PX: f32 = 17.0;
/// Rows formatted per background batch of the text build.
const TEXT_BATCH_ROWS: usize = 20_000;
/// The record inspector's header and a virtualized field's height.
const RECORD_HEADER_PX: f32 = 30.0;
const RECORD_FIELD_PX: f32 = 44.0;
/// A field's value shows at most three lines (about this many characters each).
const RECORD_LINES: usize = 3;
const RECORD_LINE_CHARS: usize = 34;
/// The view switch's and the record toggle's height (NOTES: a 22 px Segmented).
const SWITCH_PX: f32 = 22.0;

/// How a result tab shows its result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ViewMode {
    #[default]
    Table,
    Transposed,
    Text,
}

// ---- state ---------------------------------------------------------------------------------------

/// What a transposed grid was built from: a change rebuilds it.
#[derive(Clone, PartialEq)]
struct BuildKey {
    generation: Option<u64>,
    loaded: u64,
    map: ViewMap,
    cols: Vec<ColId>,
}

/// A tab's Transposed view.
struct Transposed {
    grid: Entity<GridView>,
    source: Rc<TransposedSource>,
    key: BuildKey,
    // ## m3c-int (I1)
    /// The adapter generation it was built over (a re-run retargets the adapter in place).
    generation: u64,
    _events: Subscription,
}

/// One result tab's views (keyed by its table grid).
#[derive(Default)]
struct TabViews {
    mode: ViewMode,
    transposed: Option<Transposed>,
    text: Option<Entity<TextView>>,
}

#[derive(Default)]
struct Views {
    tabs: HashMap<EntityId, TabViews>,
    /// Each console's virtualized record fields (above 200 columns).
    records: HashMap<ConsoleId, Entity<VirtualList<FieldRows>>>,
    /// Each console's record inspector focus (its keys).
    record_focus: HashMap<ConsoleId, FocusHandle>,
    // ## m3c-t38 fix round 1
    /// Each console's view switch segments and record toggle (m5).
    switch_focus: HashMap<ConsoleId, [FocusHandle; 4]>,
    /// Each console's last record, by what it was read from (m2).
    record_cache: HashMap<ConsoleId, (RecordKey, RecordView)>,
}

/// What a record was read from (fix round 1, m2): the same key reads the same fields.
#[derive(Clone, PartialEq)]
struct RecordKey {
    grid: EntityId,
    generation: Option<u64>,
    adapter_generation: u64,
    row: ModelRow,
    view_row: u64,
    rows: u64,
    cols: Vec<ColId>,
    display: u32,
    l10n: Option<u64>,
}

impl Global for Views {}

fn tab_of(grid: EntityId, cx: &App) -> Option<&TabViews> {
    cx.try_global::<Views>()?.tabs.get(&grid)
}

/// The table grid and the adapter of console `console`'s active result.
fn active(console: ConsoleId, cx: &App) -> Option<(Entity<GridView>, Rc<GridAdapter>)> {
    super::find::active_target(console, cx)
}

fn notify_hosts(cx: &mut App) {
    for host in super::view::hosts(cx) {
        host.update(cx, |_, cx| cx.notify());
    }
}

/// Focuses `handle` after this update (in the active window).
fn focus_later(handle: FocusHandle, cx: &mut App) {
    cx.defer(move |cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, cx| window.focus(&handle, cx));
        }
    });
}

// ## m3c-t38 fix round 1 (I3): the focus follows the view shown.

/// The focus handle of the view `table`'s tab shows: the Transposed view's grid, the text
/// view, or the table grid.
pub(crate) fn shown_focus(table: &Entity<GridView>, cx: &App) -> FocusHandle {
    let shown = tab_of(table.entity_id(), cx).and_then(|t| match t.mode {
        ViewMode::Table => None,
        ViewMode::Transposed => t
            .transposed
            .as_ref()
            .map(|tr| tr.grid.read(cx).focus_handle(cx)),
        ViewMode::Text => t.text.as_ref().map(|x| x.read(cx).focus.clone()),
    });
    shown.unwrap_or_else(|| table.read(cx).focus_handle(cx))
}

/// Whether a view that is not shown holds the focus: a Transposed view's grid or a text view
/// other than the one `shown`'s tab shows, or `shown` (the table grid) while another of its
/// views is shown.
pub(crate) fn hidden_view_focused(
    shown: Option<&Entity<GridView>>,
    window: &Window,
    cx: &App,
) -> bool {
    let target = shown.map(|g| shown_focus(g, cx));
    let Some(views) = cx.try_global::<Views>() else {
        return false;
    };
    for tab in views.tabs.values() {
        let handles = tab
            .transposed
            .as_ref()
            .map(|t| t.grid.read(cx).focus_handle(cx))
            .into_iter()
            .chain(tab.text.as_ref().map(|t| t.read(cx).focus.clone()));
        for h in handles {
            if Some(&h) != target.as_ref() && h.contains_focused(window, cx) {
                return true;
            }
        }
    }
    shown.is_some_and(|g| {
        let table = g.read(cx).focus_handle(cx);
        Some(&table) != target.as_ref() && table.contains_focused(window, cx)
    })
}

/// Where the value viewer anchors for table cell `cell` of console `console`, and where the
/// focus goes back when it closes: the view shown (the table grid and the cell; the Transposed
/// grid and its cell; the text view, which has no cells).
pub(crate) fn anchor_for(
    console: ConsoleId,
    cell: ModelCell,
    cx: &App,
) -> (
    Option<(WeakEntity<GridView>, ModelCell)>,
    Option<FocusHandle>,
) {
    let Some((grid, _)) = active(console, cx) else {
        return (None, None);
    };
    let back = Some(shown_focus(&grid, cx));
    let mode = tab_of(grid.entity_id(), cx)
        .map(|t| t.mode)
        .unwrap_or_default();
    let anchor = match mode {
        ViewMode::Table => Some((grid.downgrade(), cell)),
        ViewMode::Transposed => tab_of(grid.entity_id(), cx)
            .and_then(|t| t.transposed.as_ref())
            .and_then(|t| Some((t.grid.downgrade(), t.source.transposed_cell(cell)?))),
        ViewMode::Text => None,
    };
    (anchor, back)
}

// ---- the mode ------------------------------------------------------------------------------------

/// The view console `console`'s active result shows.
pub fn mode(console: ConsoleId, cx: &App) -> ViewMode {
    active(console, cx)
        .and_then(|(g, _)| tab_of(g.entity_id(), cx).map(|t| t.mode))
        .unwrap_or_default()
}

/// The rows the table shows (a filter's kept rows; else the loaded rows).
fn shown_rows(grid: &Entity<GridView>, cx: &App) -> u64 {
    grid.read(cx).view_map().len()
}

/// Why Transposed is off for `grid` (its shown rows, past [`TRANSPOSED_MAX_ROWS`]).
fn transposed_refused(grid: &Entity<GridView>, cx: &App) -> Option<u64> {
    let rows = shown_rows(grid, cx);
    (rows > TRANSPOSED_MAX_ROWS).then_some(rows)
}

/// The disabled Transposed segment's reason for console `console`'s active result.
pub fn transposed_reason(console: ConsoleId, cx: &App) -> Option<SharedString> {
    let (grid, _) = active(console, cx)?;
    let rows = transposed_refused(&grid, cx)?;
    Some(message(
        cx,
        "hv-view-transposed-off",
        vec![
            ("max", ArgValue::Count(TRANSPOSED_MAX_ROWS)),
            ("rows", ArgValue::Count(rows)),
        ],
    ))
}

/// Shows console `console`'s active result as `mode` (the switch, the commands); `false` when
/// refused (Transposed above its rows).
pub fn set_mode(console: ConsoleId, mode: ViewMode, cx: &mut App) -> bool {
    let Some((grid, adapter)) = active(console, cx) else {
        return false;
    };
    if mode == ViewMode::Transposed && transposed_refused(&grid, cx).is_some() {
        return false;
    }
    let before = tab_of(grid.entity_id(), cx)
        .map(|t| t.mode)
        .unwrap_or_default();
    if before == mode {
        return true;
    }
    // Leaving the Transposed view: the table's focus follows the transposed one.
    if before == ViewMode::Transposed
        && let Some(cell) = transposed_focus(&grid, cx)
    {
        grid.update(cx, |g, cx| g.focus_cell(cell, cx));
    }
    cx.default_global::<Views>()
        .tabs
        .entry(grid.entity_id())
        .or_default()
        .mode = mode;
    super::value_viewer::close(console, cx);
    match mode {
        ViewMode::Table => {
            focus_later(grid.read(cx).focus_handle(cx), cx);
        }
        ViewMode::Transposed => {
            // Find, Filter rows and go to row are the table's (module docs).
            super::find::close(console, cx);
            super::goto::close(console, cx);
            ensure_transposed(console, &grid, &adapter, true, cx);
            if let Some(t) = transposed_grid_of(&grid, cx) {
                focus_later(t.read(cx).focus_handle(cx), cx);
            }
        }
        ViewMode::Text => {
            super::find::close(console, cx);
            super::goto::close(console, cx);
            let text = ensure_text(console, &grid, &adapter, true, cx);
            focus_later(text.read(cx).focus.clone(), cx);
        }
    }
    notify_hosts(cx);
    true
}

/// Whether the shown console's active result shows its table (the find and go-to commands'
/// `enabled`: they act on the table).
pub fn table_shown(cx: &App) -> bool {
    let Some((console, _, _)) = super::view::active_entry(cx) else {
        return true;
    };
    mode(console, cx) == ViewMode::Table
}

// ## m3c-int (I3, M4)
/// Whether `grid`'s tab shows its table (find, Filter rows and go to row act only then).
pub(crate) fn shows_table(grid: &Entity<GridView>, cx: &App) -> bool {
    tab_of(grid.entity_id(), cx).is_none_or(|t| t.mode == ViewMode::Table)
}

/// Whether the shown console's active result shows as text (the grid commands other than Copy
/// do nothing there: their `enabled`).
pub fn text_shown(cx: &App) -> bool {
    super::view::active_entry(cx).is_some_and(|(console, _, _)| mode(console, cx) == ViewMode::Text)
}

/// The tile shows `grid` over `adapter` for console `console` (each frame): the Transposed and
/// Text views follow the table (rebuilt when its rows, order or columns change); a Transposed
/// view past its rows goes back to the table.
pub(crate) fn sync(
    console: ConsoleId,
    grid: &Entity<GridView>,
    adapter: &Rc<GridAdapter>,
    cx: &mut App,
) {
    super::value_viewer::sync(console, adapter, cx);
    let Some(mode) = tab_of(grid.entity_id(), cx).map(|t| t.mode) else {
        return;
    };
    // ## m3c-t38 fix round 1 (I4): the table grid taking frames under another view is hidden
    // from assistive technology.
    let hidden = mode != ViewMode::Table;
    if grid.read(cx).a11y_hidden() != hidden {
        grid.update(cx, |g, cx| g.set_a11y_hidden(hidden, cx));
    }
    match mode {
        ViewMode::Table => {}
        ViewMode::Transposed => {
            if transposed_refused(grid, cx).is_some() {
                let grid = grid.clone();
                cx.defer(move |cx| {
                    if let Some(t) = cx.default_global::<Views>().tabs.get_mut(&grid.entity_id()) {
                        t.mode = ViewMode::Table;
                    }
                    notify_hosts(cx);
                });
                return;
            }
            ensure_transposed(console, grid, adapter, false, cx);
        }
        ViewMode::Text => {
            ensure_text(console, grid, adapter, false, cx);
        }
    }
}

/// `grid` is gone (its tab or console closed): its views go.
pub(crate) fn grid_closed(grid: EntityId, cx: &mut App) {
    if cx
        .try_global::<Views>()
        .is_some_and(|v| v.tabs.contains_key(&grid))
    {
        cx.default_global::<Views>().tabs.remove(&grid);
    }
}

/// The key the table's state gives a view built now.
fn build_key(grid: &Entity<GridView>, cx: &App) -> BuildKey {
    let g = grid.read(cx);
    BuildKey {
        generation: g.generation(),
        loaded: g.view_map().len(),
        map: g.view_map().clone(),
        cols: g.column_layout().visible().to_vec(),
    }
}

/// Whether `adapter`'s result is still streaming rows (a view is not rebuilt for each append).
fn streaming(adapter: &GridAdapter) -> bool {
    matches!(adapter.snapshot().completeness(), Completeness::Streaming)
}

// ---- the Transposed view -------------------------------------------------------------------------

/// The Transposed view's grid of `table`, once built.
pub(crate) fn transposed_grid_of(table: &Entity<GridView>, cx: &App) -> Option<Entity<GridView>> {
    tab_of(table.entity_id(), cx)?
        .transposed
        .as_ref()
        .map(|t| t.grid.clone())
}

/// The Transposed view's grid of console `console`'s active result, once built.
pub fn transposed_grid(console: ConsoleId, cx: &App) -> Option<Entity<GridView>> {
    let (grid, _) = active(console, cx)?;
    transposed_grid_of(&grid, cx)
}

/// The Transposed view's source of console `console`'s active result.
pub fn transposed_source(console: ConsoleId, cx: &App) -> Option<Rc<TransposedSource>> {
    let (grid, _) = active(console, cx)?;
    tab_of(grid.entity_id(), cx)?
        .transposed
        .as_ref()
        .map(|t| t.source.clone())
}

/// The table cell the Transposed view's focus shows.
fn transposed_focus(table: &Entity<GridView>, cx: &App) -> Option<ModelCell> {
    let t = tab_of(table.entity_id(), cx)?.transposed.as_ref()?;
    let focus = t.grid.read(cx).selection().focus()?;
    t.source.table_cell(focus)
}

/// Builds (or rebuilds) `table`'s Transposed view when its key changed; `force` builds it even
/// while the result streams.
fn ensure_transposed(
    console: ConsoleId,
    table: &Entity<GridView>,
    adapter: &Rc<GridAdapter>,
    force: bool,
    cx: &mut App,
) {
    // ## m3c-int (I1): a re-run retargets the adapter in place: the view is rebuilt at once,
    // also while the new result streams, so the old rows and column names never read the new
    // result's cells. Until the table grid paints the re-run, its map and columns are the old
    // result's: the view is built as the table will show the new one (its rows in the source
    // order, every column).
    let generation = adapter.generation();
    let key = if table.read(cx).generation().is_some_and(|g| g != generation) {
        let loaded = adapter.snapshot().row_count();
        BuildKey {
            generation: Some(generation),
            loaded,
            map: ViewMap::identity(loaded),
            cols: hacivat_ui_grid::GridDataSource::columns(&**adapter)
                .iter()
                .map(|c| c.id)
                .collect(),
        }
    } else {
        build_key(table, cx)
    };
    let current = tab_of(table.entity_id(), cx).and_then(|t| {
        t.transposed.as_ref().map(|tr| {
            (
                tr.key == key && tr.generation == generation,
                tr.grid.clone(),
                tr.generation != generation,
            )
        })
    });
    match current {
        Some((true, _, _)) => return,
        Some((false, _, false)) if !force && streaming(adapter) => return,
        _ => {}
    }
    let current = current.map(|(same, grid, _)| (same, grid));
    let name_title = message(cx, "hv-transposed-col", vec![]);
    let inner: Rc<dyn hacivat_ui_grid::GridDataSource> = adapter.clone();
    let source = Rc::new(TransposedSource::new(
        inner, &key.map, &key.cols, name_title,
    ));
    // The focus the view starts on: the table's focus, as the transposed grid shows it.
    let start = table
        .read(cx)
        .selection()
        .focus()
        .and_then(|f| source.transposed_cell(f));
    let number = crate::console::Consoles::get(cx, console)
        .and_then(|s| s.results.clone())
        .and_then(|r| r.read(cx).active_tab().map(|t| t.number()))
        .unwrap_or(1);
    let dyn_source: Rc<dyn hacivat_ui_grid::GridDataSource> = source.clone();
    if let Some((_, grid)) = current {
        // ## m3c-t38 fix round 1 (m3): the record it was on (a table cell), wherever the new
        // order puts it.
        let kept = transposed_focus(table, cx).and_then(|c| source.transposed_cell(c));
        grid.update(cx, |g, cx| {
            g.set_source(dyn_source, false, cx);
            g.set_frozen(1, cx);
            if let Some(cell) = kept {
                g.focus_cell(cell, cx);
            }
        });
        if let Some(t) = cx
            .default_global::<Views>()
            .tabs
            .get_mut(&table.entity_id())
            .and_then(|t| t.transposed.as_mut())
        {
            t.source = source;
            t.key = key;
            t.generation = generation;
        }
        return;
    }
    let mut config = super::view::grid_config(number, adapter);
    config.row_height = TRANSPOSED_ROW_PX;
    config.transposed = true;
    config.auto_size = key.map.len() as usize <= TRANSPOSED_AUTOSIZE_ROWS;
    let grid = cx.new(|cx| GridView::new(dyn_source, config, cx));
    let handle = grid.read(cx).focus_handle(cx);
    crate::shell::register_focus_context(cx, handle, GRID_CONTEXT);
    if let Some(cell) = start {
        grid.update(cx, |g, cx| g.focus_cell(cell, cx));
    }
    let events = cx.subscribe(&grid, move |grid, e: &GridEvent, cx| {
        on_transposed_event(console, &grid, e, cx)
    });
    cx.default_global::<Views>()
        .tabs
        .entry(table.entity_id())
        .or_default()
        .transposed = Some(Transposed {
        grid,
        source,
        key,
        generation,
        _events: events,
    });
}

/// What the Transposed view's grid asks of the tile.
fn on_transposed_event(console: ConsoleId, grid: &Entity<GridView>, e: &GridEvent, cx: &mut App) {
    let Some((table, adapter)) = active(console, cx) else {
        return;
    };
    let Some(source) = transposed_source(console, cx) else {
        return;
    };
    match e {
        GridEvent::CopyRequested {
            region, headers, ..
        } => {
            copy_transposed(&adapter, &source, region, *headers, cx).detach();
        }
        GridEvent::CopyRefused(refusal) => copy::copy_refused(*refusal, cx),
        GridEvent::ValueRequested(cell) => {
            if let Some(table_cell) = source.table_cell(*cell) {
                let back = Some(grid.read(cx).focus_handle(cx));
                super::value_viewer::open_anchored(
                    console,
                    table_cell,
                    Some((grid.downgrade(), *cell)),
                    back,
                    cx,
                );
            }
        }
        // ## m3c-t39 (named exception): a cell menu over the table cell (no header or row
        // number menu here: their rows and columns are the table's).
        GridEvent::ContextMenuRequested {
            at, cell, pointer, ..
        } => {
            let cell = cell.and_then(|c| source.table_cell(c));
            let request = super::context_menu::MenuRequest {
                at: *at,
                cell,
                header: None,
                row_number: false,
                pointer: *pointer,
            };
            super::context_menu::open(console, grid, request, cx);
        }
        GridEvent::SelectionChanged | GridEvent::FocusChanged(_) => {
            let _ = table;
            cx.defer(notify_hosts);
        }
        // Find, go to row and sorting by a row are not offered here (module docs).
        _ => {}
    }
}

/// The TSV of `region` of the Transposed view over `view`: raw values (copy's channel), the
/// name column's cells as the column names; `headers` adds "Column" and the row numbers.
pub(crate) fn transposed_tsv(
    view: &RawView,
    source: &TransposedSource,
    region: &CopyRegion,
    headers: bool,
    max_bytes: usize,
) -> Result<Tsv, TooLarge> {
    let snapshot = &view.snapshot;
    let engine = view.display.engine();
    let zone = view.display.zone();
    let rows = source.table_rows();
    let cols = source.table_cols();
    let names = &snapshot.schema().columns;
    let loaded = snapshot.row_count();
    let mut out = String::new();
    let mut incomplete = 0u64;
    if headers {
        for (i, c) in region.cols.iter().enumerate() {
            if i > 0 {
                out.push('\t');
            }
            let title = source
                .columns()
                .get(c.0 as usize)
                .map(|s| s.name.to_string())
                .unwrap_or_default();
            copy::push_cell(&mut out, &title);
        }
        out.push('\n');
    }
    for r in region.rows.iter().flat_map(|r| r.clone()) {
        let Some(table_col) = cols.get(r as usize) else {
            continue;
        };
        for (i, c) in region.cols.iter().enumerate() {
            if i > 0 {
                out.push('\t');
            }
            if c.0 == 0 {
                if let Some(meta) = names.get(table_col.0 as usize) {
                    copy::push_cell(&mut out, &meta.name);
                }
                continue;
            }
            let Some(row) = rows.get(c.0 as usize - 1).filter(|r| r.0 < loaded) else {
                continue;
            };
            let Some(facts) = view.facts.get(table_col.0 as usize) else {
                continue;
            };
            let value = snapshot.cell(row.0, table_col.0);
            match value {
                ValueRef::Error(_) => {
                    incomplete += 1;
                    continue;
                }
                ValueRef::Truncated(_) => incomplete += 1,
                _ => {}
            }
            if out.len().saturating_add(copy::known_len(value)) > max_bytes {
                return Err(TooLarge);
            }
            if let RawText::Text(text) = raw_text_in(value, &facts.view(engine), zone) {
                copy::push_cell(&mut out, &text);
            }
        }
        out.push('\n');
        if out.len() > max_bytes {
            return Err(TooLarge);
        }
    }
    Ok(Tsv {
        text: out,
        incomplete,
    })
}

/// Copies `region` of the Transposed view (the grid's `CopyRequested`): the guards and the
/// toasts of a table copy; a large one is built on the background executor.
fn copy_transposed(
    adapter: &GridAdapter,
    source: &Rc<TransposedSource>,
    region: &CopyRegion,
    headers: bool,
    cx: &mut App,
) -> Task<CopyOutcome> {
    let cells = region.cell_count();
    if cells > copy::MAX_COPY_CELLS {
        copy::refuse_too_much(cells, cx);
        return Task::ready(CopyOutcome::TooManyCells { cells });
    }
    let view = adapter.raw_view();
    let rows = region.row_count();
    let cols = region.cols.len() as u64;
    if cells <= copy::BACKGROUND_COPY_CELLS {
        let text = transposed_tsv(&view, source, region, headers, copy::MAX_COPY_BYTES);
        return Task::ready(copy::finish(text, rows, cols, cells, cx));
    }
    // The source is not `Send`: copy what the build needs.
    let table_rows: Vec<ModelRow> = source.table_rows().to_vec();
    let table_cols: Vec<ColId> = source.table_cols().to_vec();
    let titles: Vec<SharedString> = source.columns().iter().map(|c| c.name.clone()).collect();
    let region = region.clone();
    let job = cx.background_spawn(async move {
        let source = SendableTranspose {
            rows: table_rows,
            cols: table_cols,
            titles,
        };
        source.tsv(&view, &region, headers, copy::MAX_COPY_BYTES)
    });
    cx.spawn(async move |cx| {
        let text = job.await;
        cx.update(|cx| copy::finish(text, rows, cols, cells, cx))
    })
}

/// [`transposed_tsv`]'s inputs, without the table's (UI-thread) source.
struct SendableTranspose {
    rows: Vec<ModelRow>,
    cols: Vec<ColId>,
    titles: Vec<SharedString>,
}

impl SendableTranspose {
    fn tsv(
        &self,
        view: &RawView,
        region: &CopyRegion,
        headers: bool,
        max_bytes: usize,
    ) -> Result<Tsv, TooLarge> {
        let snapshot = &view.snapshot;
        let engine = view.display.engine();
        let zone = view.display.zone();
        let names = &snapshot.schema().columns;
        let loaded = snapshot.row_count();
        let mut out = String::new();
        let mut incomplete = 0u64;
        if headers {
            for (i, c) in region.cols.iter().enumerate() {
                if i > 0 {
                    out.push('\t');
                }
                if let Some(t) = self.titles.get(c.0 as usize) {
                    copy::push_cell(&mut out, t);
                }
            }
            out.push('\n');
        }
        for r in region.rows.iter().flat_map(|r| r.clone()) {
            let Some(table_col) = self.cols.get(r as usize) else {
                continue;
            };
            for (i, c) in region.cols.iter().enumerate() {
                if i > 0 {
                    out.push('\t');
                }
                if c.0 == 0 {
                    if let Some(meta) = names.get(table_col.0 as usize) {
                        copy::push_cell(&mut out, &meta.name);
                    }
                    continue;
                }
                let Some(row) = self.rows.get(c.0 as usize - 1).filter(|r| r.0 < loaded) else {
                    continue;
                };
                let Some(facts) = view.facts.get(table_col.0 as usize) else {
                    continue;
                };
                let value = snapshot.cell(row.0, table_col.0);
                match value {
                    ValueRef::Error(_) => {
                        incomplete += 1;
                        continue;
                    }
                    ValueRef::Truncated(_) => incomplete += 1,
                    _ => {}
                }
                if out.len().saturating_add(copy::known_len(value)) > max_bytes {
                    return Err(TooLarge);
                }
                if let RawText::Text(text) = raw_text_in(value, &facts.view(engine), zone) {
                    copy::push_cell(&mut out, &text);
                }
            }
            out.push('\n');
            if out.len() > max_bytes {
                return Err(TooLarge);
            }
        }
        Ok(Tsv {
            text: out,
            incomplete,
        })
    }
}

// ---- the Text view -------------------------------------------------------------------------------

/// Where a text view is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextState {
    /// Above [`TEXT_ASK_ROWS`]: asks first (`bytes` the text would take, about).
    Asking {
        rows: u64,
        bytes: u64,
    },
    /// Building: `done` of `rows`.
    Building {
        done: u64,
        rows: u64,
    },
    Ready,
}

/// The text view's lines (a `VirtualList`'s rows).
#[derive(Clone, Default)]
struct TextRows {
    lines: Arc<[String]>,
    selected: Option<Range<usize>>,
    /// The horizontal scroll, in px.
    x: f32,
    view: Option<WeakEntity<TextView>>,
}

impl RowRenderer for TextRows {
    fn len(&self) -> u64 {
        self.lines.len() as u64
    }

    fn row(&self, ix: u64, _window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let ix = ix as usize;
        let Some(line) = self.lines.get(ix) else {
            return div().into_any_element();
        };
        let selected = self.selected.as_ref().is_some_and(|s| s.contains(&ix));
        let rule = line.starts_with('+');
        let style = t.type_ramp.mono_data;
        let view = self.view.clone();
        let mut row = div()
            .id(("text-view-line", ix))
            .h(px(TEXT_LINE_PX))
            .w_full()
            .pl(px(12.0 - self.x))
            .whitespace_nowrap()
            .font(font_for(&style))
            .text_size(px(style.size_px))
            .line_height(px(TEXT_LINE_PX))
            .text_color(if rule { c.text_2 } else { c.text }.to_hsla())
            .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, window, cx| {
                if let Some(view) = view.as_ref().and_then(WeakEntity::upgrade) {
                    view.update(cx, |v, cx| v.click(ix, e.modifiers.shift, window, cx));
                }
            })
            .child(SharedString::from(line.clone()));
        if selected {
            row = row.bg(c.blue_tint_strong.to_hsla());
        }
        row.into_any_element()
    }
}

/// One result tab's text view.
pub struct TextView {
    console: ConsoleId,
    grid: WeakEntity<GridView>,
    adapter: Weak<GridAdapter>,
    key: Option<BuildKey>,
    state: TextState,
    /// The model row each line shows (`None`: a rule or the header).
    rows: Arc<[Option<ModelRow>]>,
    anchor: Option<usize>,
    list: Entity<VirtualList<TextRows>>,
    focus: FocusHandle,
    job: Option<Task<()>>,
    /// The widest line, in characters.
    widest: usize,
    /// The list's width in the last frame.
    width: Rc<Cell<f32>>,
    advance: Cell<Option<f32>>,
}

/// The table's rows as the text view reads them: model rows in view order and the shown
/// columns' places.
#[derive(Clone)]
struct TextInput {
    view: RawView,
    rows: Arc<[u64]>,
    cols: Arc<[u32]>,
}

/// One cell's text in the text view: the raw text, control characters escaped (`\n`, `\t`,
/// `\r`), NULL as `NULL`, a decode error empty, cut at [`TEXT_MAX_WIDTH`] characters (with `…`).
fn text_cell(view: &RawView, row: u64, col: u32) -> String {
    let Some(facts) = view.facts.get(col as usize) else {
        return String::new();
    };
    let value = view.snapshot.cell(row, col);
    let raw = match value {
        ValueRef::Error(_) => return String::new(),
        v => raw_text_in(v, &facts.view(view.display.engine()), view.display.zone()),
    };
    let text = match raw {
        RawText::Null => return NULL_MARKER.to_string(),
        RawText::Text(t) => t,
    };
    let mut out = String::with_capacity(text.len().min(TEXT_MAX_WIDTH * 4));
    let mut n = 0;
    for ch in text.chars() {
        let esc = match ch {
            '\n' => Some("\\n"),
            '\t' => Some("\\t"),
            '\r' => Some("\\r"),
            _ => None,
        };
        let w = esc.map_or(1, str::len);
        if n + w > TEXT_MAX_WIDTH {
            out.push('…');
            return out;
        }
        match esc {
            Some(e) => out.push_str(e),
            None if ch.is_control() => out.push('\u{FFFD}'),
            None => out.push(ch),
        }
        n += w;
    }
    out
}

/// Whether column `col` aligns its cells right (numbers).
fn right_aligned(view: &RawView, col: u32) -> bool {
    view.facts
        .get(col as usize)
        .is_some_and(|f| f.align == hacivat_ui_grid::Align::End)
}

/// The widths (characters) of `input`'s columns over rows `rows` (and the header names).
fn text_widths(input: &TextInput, rows: Range<usize>, mut widths: Vec<usize>) -> Vec<usize> {
    for r in &input.rows[rows] {
        for (i, c) in input.cols.iter().enumerate() {
            let w = text_cell(&input.view, *r, *c).chars().count();
            if w > widths[i] {
                widths[i] = w;
            }
        }
    }
    widths
}

/// A rule line for `widths`: `+----+------+`.
fn rule_line(widths: &[usize]) -> String {
    let mut out = String::from("+");
    for w in widths {
        out.push_str(&"-".repeat(w + 2));
        out.push('+');
    }
    out
}

/// One table line of `cells` padded to `widths` (right-aligned where `right`).
fn table_line(cells: &[String], widths: &[usize], right: &[bool]) -> String {
    let mut out = String::from("|");
    for (i, cell) in cells.iter().enumerate() {
        let pad = widths[i].saturating_sub(cell.chars().count());
        out.push(' ');
        if right[i] {
            out.push_str(&" ".repeat(pad));
            out.push_str(cell);
        } else {
            out.push_str(cell);
            out.push_str(&" ".repeat(pad));
        }
        out.push_str(" |");
    }
    out
}

fn header_widths(input: &TextInput) -> Vec<usize> {
    input
        .cols
        .iter()
        .map(|c| {
            input
                .view
                .snapshot
                .schema()
                .columns
                .get(*c as usize)
                .map_or(0, |m| m.name.chars().count().min(TEXT_MAX_WIDTH))
        })
        .collect()
}

fn header_lines(input: &TextInput, widths: &[usize]) -> Vec<String> {
    let names: Vec<String> = input
        .cols
        .iter()
        .map(|c| {
            input
                .view
                .snapshot
                .schema()
                .columns
                .get(*c as usize)
                .map(|m| m.name.chars().take(TEXT_MAX_WIDTH).collect())
                .unwrap_or_default()
        })
        .collect();
    let rule = rule_line(widths);
    vec![
        rule.clone(),
        table_line(&names, widths, &vec![false; names.len()]),
        rule,
    ]
}

/// The rows `grid` shows (model rows in view order) and its shown columns.
fn text_input(grid: &Entity<GridView>, adapter: &GridAdapter, cx: &App) -> TextInput {
    let g = grid.read(cx);
    let map = g.view_map();
    let view = adapter.raw_view();
    let loaded = view.snapshot.row_count();
    let rows: Arc<[u64]> = (0..map.len())
        .filter_map(|v| map.view_to_model(ViewRow(v)).map(|m| m.0))
        .filter(|m| *m < loaded)
        .collect();
    let cols: Arc<[u32]> = g.column_layout().visible().iter().map(|c| c.0).collect();
    TextInput { view, rows, cols }
}

/// About the bytes the text of `input` takes (the first rows' mean line, times the rows).
fn text_bytes_estimate(input: &TextInput) -> u64 {
    let sample = input.rows.len().min(1_000);
    if sample == 0 {
        return 0;
    }
    let widths = text_widths(input, 0..sample, header_widths(input));
    let line: usize = widths.iter().map(|w| w + 3).sum::<usize>() + 1;
    // A line's text and its `String`.
    (line as u64 + 24) * input.rows.len() as u64
}

fn ensure_text(
    console: ConsoleId,
    grid: &Entity<GridView>,
    adapter: &Rc<GridAdapter>,
    force: bool,
    cx: &mut App,
) -> Entity<TextView> {
    let existing = tab_of(grid.entity_id(), cx).and_then(|t| t.text.clone());
    let key = build_key(grid, cx);
    let text = match existing {
        Some(text) => text,
        None => {
            let text = cx.new(|cx| TextView::new(console, grid, adapter, cx));
            cx.default_global::<Views>()
                .tabs
                .entry(grid.entity_id())
                .or_default()
                .text = Some(text.clone());
            text
        }
    };
    let stale = text.read(cx).key.as_ref() != Some(&key);
    let waiting = matches!(text.read(cx).state, TextState::Asking { .. });
    if stale && !waiting && (force || !streaming(adapter)) {
        let (grid, adapter) = (grid.clone(), adapter.clone());
        let t = text.clone();
        cx.defer(move |cx| t.update(cx, |t, cx| t.start(&grid, &adapter, key, false, cx)));
    }
    text
}

impl TextView {
    fn new(
        console: ConsoleId,
        grid: &Entity<GridView>,
        adapter: &Rc<GridAdapter>,
        cx: &mut Context<Self>,
    ) -> Self {
        let weak = cx.entity().downgrade();
        let list = cx.new(|_| {
            VirtualList::new(
                TextRows {
                    view: Some(weak),
                    ..TextRows::default()
                },
                TEXT_LINE_PX,
            )
        });
        TextView {
            console,
            grid: grid.downgrade(),
            adapter: Rc::downgrade(adapter),
            key: None,
            state: TextState::Ready,
            rows: Arc::from([]),
            anchor: None,
            list,
            focus: cx.focus_handle(),
            job: None,
            widest: 0,
            width: Rc::new(Cell::new(0.0)),
            advance: Cell::new(None),
        }
    }

    /// Builds the text for `key` (asks first above [`TEXT_ASK_ROWS`] unless `asked`).
    fn start(
        &mut self,
        grid: &Entity<GridView>,
        adapter: &Rc<GridAdapter>,
        key: BuildKey,
        asked: bool,
        cx: &mut Context<Self>,
    ) {
        let input = text_input(grid, adapter, cx);
        let rows = input.rows.len() as u64;
        self.key = Some(key);
        if rows > TEXT_ASK_ROWS && !asked {
            self.job = None;
            self.state = TextState::Asking {
                rows,
                bytes: text_bytes_estimate(&input),
            };
            cx.defer(notify_hosts);
            cx.notify();
            return;
        }
        self.state = TextState::Building { done: 0, rows };
        let n = input.rows.len();
        self.job = Some(cx.spawn(async move |this, cx| {
            // Pass 1: the widths, in batches (half the progress).
            let mut widths = header_widths(&input);
            let mut at = 0;
            while at < n {
                let end = (at + TEXT_BATCH_ROWS).min(n);
                let part = input.clone();
                let w = widths.clone();
                widths = cx
                    .background_spawn(async move { text_widths(&part, at..end, w) })
                    .await;
                at = end;
                let done = (at / 2) as u64;
                if this.update(cx, |t, cx| t.progress(done, rows, cx)).is_err() {
                    return;
                }
            }
            // Pass 2: the lines.
            let mut lines = header_lines(&input, &widths);
            let mut of: Vec<Option<ModelRow>> = vec![None; lines.len()];
            let right: Arc<[bool]> = input
                .cols
                .iter()
                .map(|c| right_aligned(&input.view, *c))
                .collect();
            let widths: Arc<[usize]> = widths.into();
            let mut at = 0;
            while at < n {
                let end = (at + TEXT_BATCH_ROWS).min(n);
                let part = input.clone();
                let (w, r) = (widths.clone(), right.clone());
                let batch = cx
                    .background_spawn(async move {
                        part.rows[at..end]
                            .iter()
                            .map(|row| {
                                let cells: Vec<String> = part
                                    .cols
                                    .iter()
                                    .map(|c| text_cell(&part.view, *row, *c))
                                    .collect();
                                (table_line(&cells, &w, &r), ModelRow(*row))
                            })
                            .collect::<Vec<_>>()
                    })
                    .await;
                for (line, row) in batch {
                    lines.push(line);
                    of.push(Some(row));
                }
                at = end;
                let done = ((n + at) / 2) as u64;
                if this.update(cx, |t, cx| t.progress(done, rows, cx)).is_err() {
                    return;
                }
            }
            lines.push(rule_line(&widths));
            of.push(None);
            let _ = this.update(cx, |t, cx| t.finish(lines, of, cx));
        }));
        cx.defer(notify_hosts);
        cx.notify();
    }

    fn progress(&mut self, done: u64, rows: u64, cx: &mut Context<Self>) {
        self.state = TextState::Building { done, rows };
        cx.defer(notify_hosts);
        cx.notify();
    }

    fn finish(&mut self, lines: Vec<String>, of: Vec<Option<ModelRow>>, cx: &mut Context<Self>) {
        self.widest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        self.rows = of.into();
        self.anchor = None;
        self.state = TextState::Ready;
        self.job = None;
        let lines: Arc<[String]> = lines.into();
        self.list.update(cx, |l, cx| {
            let rows = l.rows_mut(cx);
            rows.lines = lines;
            rows.selected = None;
            rows.x = 0.0;
        });
        cx.defer(notify_hosts);
        cx.notify();
    }

    /// Show as text (the question's primary).
    fn show_anyway(&mut self, cx: &mut Context<Self>) {
        let (Some(grid), Some(adapter)) = (self.grid.upgrade(), self.adapter.upgrade()) else {
            return;
        };
        let key = build_key(&grid, cx);
        self.start(&grid, &adapter, key, true, cx);
    }

    /// Back to the table (the question's default, the build's Cancel, Escape).
    fn back(&mut self, cx: &mut Context<Self>) {
        self.job = None;
        self.key = None;
        self.state = TextState::Ready;
        let console = self.console;
        cx.defer(move |cx| {
            set_mode(console, ViewMode::Table, cx);
        });
    }

    fn click(&mut self, ix: usize, extend: bool, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.select(ix, extend, cx);
    }

    /// Selects line `ix` (from the anchor when `extend`).
    fn select(&mut self, ix: usize, extend: bool, cx: &mut Context<Self>) {
        let n = self.rows.len();
        if n == 0 {
            return;
        }
        let ix = ix.min(n - 1);
        let anchor = match (extend, self.anchor) {
            (true, Some(a)) => a,
            _ => {
                self.anchor = Some(ix);
                ix
            }
        };
        let range = anchor.min(ix)..anchor.max(ix) + 1;
        self.list.update(cx, |l, cx| {
            l.rows_mut(cx).selected = Some(range);
            l.scroll_into_view(ix as u64, cx);
        });
        cx.defer(notify_hosts);
        cx.notify();
    }

    /// The selected lines, as shown.
    fn selected_text(&self, cx: &App) -> Option<(String, usize)> {
        let rows = self.list.read(cx).rows();
        let range = rows.selected.clone()?;
        let n = range.len();
        let mut out = String::new();
        for line in &rows.lines[range] {
            out.push_str(line);
            out.push('\n');
        }
        Some((out, n))
    }

    /// Copies the selected lines (nothing selected: nothing).
    fn copy(&mut self, cx: &mut Context<Self>) {
        let Some((text, n)) = self.selected_text(cx) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        let mut toast = Toast::new(
            ToastKind::Info,
            Rc::new(move |cx: &App| {
                message(cx, "hv-text-copied", vec![("n", ArgValue::Count(n as u64))])
            }),
        );
        toast.icon = Some(IconName::CopyCheck);
        ToastQueue::push(cx, toast);
    }

    /// The model row of the selection's last line (the record inspector reads it).
    fn selected_row(&self, cx: &App) -> Option<ModelRow> {
        let range = self.list.read(cx).rows().selected.clone()?;
        range
            .rev()
            .find_map(|i| self.rows.get(i).copied().flatten())
    }

    /// Selects the line that shows model row `row`.
    fn select_row(&mut self, row: ModelRow, cx: &mut Context<Self>) {
        if let Some(ix) = self.rows.iter().position(|r| *r == Some(row)) {
            self.select(ix, false, cx);
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        if k.key == "c" && k.modifiers.secondary(cx) && !k.modifiers.shift && !k.modifiers.alt {
            self.copy(cx);
            cx.stop_propagation();
            return;
        }
        if k.key == "escape" && !k.modifiers.modified() {
            if !matches!(self.state, TextState::Ready) {
                self.back(cx);
                cx.stop_propagation();
            }
            return;
        }
        let at = self.list.read(cx).rows().selected.clone().map(|r| {
            if self.anchor == Some(r.start) {
                r.end - 1
            } else {
                r.start
            }
        });
        let step = match k.key.as_str() {
            "up" => -1i64,
            "down" => 1,
            _ => return,
        };
        if k.modifiers.secondary(cx) || k.modifiers.alt {
            return;
        }
        let next = at.map_or(0, |a| (a as i64 + step).max(0) as usize);
        self.select(next, k.modifiers.shift, cx);
        cx.stop_propagation();
    }

    fn scroll_x(&mut self, dx: f32, advance: f32, cx: &mut Context<Self>) {
        let max = (self.widest as f32 * advance + 24.0 - self.width.get()).max(0.0);
        self.list.update(cx, |l, cx| {
            let rows = l.rows_mut(cx);
            rows.x = (rows.x + dx).clamp(0.0, max);
        });
    }
}

impl Render for TextView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let entity = cx.entity();
        let advance = match self.advance.get() {
            Some(a) => a,
            None => {
                let style = t.type_ramp.mono_data;
                let ts = window.text_system();
                let id = ts.resolve_font(&font_for(&style));
                let a = ts
                    .em_advance(id, px(style.size_px))
                    .map(f32::from)
                    .ok()
                    .filter(|a| *a > 0.0)
                    .unwrap_or(style.size_px * 0.6);
                self.advance.set(Some(a));
                a
            }
        };
        let root = div()
            .id(("text-view", self.console.0))
            .debug_selector(|| "text-view".to_string())
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .bg(c.panel.to_hsla())
            .on_key_down(cx.listener(|t, ev: &KeyDownEvent, _, cx| t.on_key(ev, cx)));
        match self.state {
            TextState::Asking { rows, bytes } => {
                let (e1, e2) = (entity.clone(), entity.clone());
                let size = Texts::of(cx).size(bytes);
                let notice = div()
                    .m(px(12.0))
                    .p(px(12.0))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .bg(c.raised.to_hsla())
                    .border_1()
                    .border_color(c.seam_strong.to_hsla())
                    .rounded(px(t.radii.sm))
                    .child(
                        Label::new(
                            message(
                                cx,
                                "hv-text-big-title",
                                vec![("rows", ArgValue::Count(rows))],
                            ),
                            TextSource::Fluent,
                            t.type_ramp.body_strong,
                            c.text.to_hsla(),
                        )
                        .id("text-big-title"),
                    )
                    .child(paragraph(
                        message(cx, "hv-text-big-body", vec![("size", ArgValue::Text(size))]),
                        TextSource::Fluent,
                        t.type_ramp.body_sm,
                        c.text_2.to_hsla(),
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_end()
                            .gap(px(8.0))
                            .child(
                                Button::new(
                                    "text-big-back",
                                    message(cx, "hv-text-big-back", vec![]),
                                )
                                .kind(ButtonKind::Default)
                                .on_raised(true)
                                .on_click(move |_, _, cx| e1.update(cx, |t, cx| t.back(cx))),
                            )
                            .child(
                                Button::new("text-big-ok", message(cx, "hv-text-big-ok", vec![]))
                                    .kind(ButtonKind::Primary)
                                    .on_raised(true)
                                    .on_click(move |_, _, cx| {
                                        e2.update(cx, |t, cx| t.show_anyway(cx))
                                    }),
                            ),
                    );
                root.child(notice).into_any_element()
            }
            TextState::Building { done, rows } => {
                let e = entity.clone();
                let line = div()
                    .m(px(12.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(run_marker(&theme))
                    .child(
                        Label::new(
                            message(
                                cx,
                                "hv-text-building",
                                vec![
                                    ("done", ArgValue::Count(done.min(rows))),
                                    ("rows", ArgValue::Count(rows)),
                                ],
                            ),
                            TextSource::Fluent,
                            t.type_ramp.caption,
                            c.text.to_hsla(),
                        )
                        .id("text-building"),
                    )
                    .child(
                        Button::new("text-cancel", message(cx, "hv-text-cancel", vec![]))
                            .kind(ButtonKind::Default)
                            .on_click(move |_, _, cx| e.update(cx, |t, cx| t.back(cx))),
                    );
                root.child(line).into_any_element()
            }
            TextState::Ready => {
                let width = self.width.clone();
                let total = self.widest as f32 * advance + 24.0;
                let x = self.list.read(cx).rows().x;
                let view_w = width.get();
                let thumb = (total > view_w && view_w > 0.0).then(|| {
                    let w = (view_w * view_w / total).max(24.0);
                    let left = x / total * view_w;
                    div()
                        .absolute()
                        .bottom(px(1.0))
                        .left(px(left))
                        .w(px(w))
                        .h(px(4.0))
                        .rounded(px(2.0))
                        .bg(c.seam_strong.to_hsla())
                });
                root.on_scroll(cx.listener(move |t, e: &ScrollEvent, _, cx| {
                    let d = e.delta.pixel_delta(px(TEXT_LINE_PX));
                    let dx = -f32::from(d.x);
                    if dx != 0.0 {
                        t.scroll_x(dx, advance, cx);
                    }
                }))
                .declare_label(message(cx, "hv-text-label", vec![]))
                .child(self.list.clone())
                .child(
                    canvas(
                        move |b, _, _| width.set(f32::from(b.size.width)),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
                .children(thumb)
                .into_any_element()
            }
        }
    }
}

/// A list's accessible name (the text view's role is a document).
trait DeclareLabel {
    fn declare_label(self, label: SharedString) -> Self;
}

impl DeclareLabel for hacivat_ui_core::gpui_surface::Stateful<hacivat_ui_core::gpui_surface::Div> {
    fn declare_label(self, label: SharedString) -> Self {
        self.role(Role::Document).aria_label(label)
    }
}

/// The text view of console `console`'s active result.
fn text_view(console: ConsoleId, cx: &App) -> Option<Entity<TextView>> {
    let (grid, _) = active(console, cx)?;
    tab_of(grid.entity_id(), cx)?.text.clone()
}

/// The text view's focus handle (tests).
pub fn text_focus(console: ConsoleId, cx: &App) -> Option<FocusHandle> {
    Some(text_view(console, cx)?.read(cx).focus.clone())
}

/// The text view's state (`None` before it was shown).
pub fn text_state(console: ConsoleId, cx: &App) -> Option<TextState> {
    Some(text_view(console, cx)?.read(cx).state)
}

/// The text view's lines.
pub fn text_lines(console: ConsoleId, cx: &App) -> Vec<String> {
    text_view(console, cx)
        .map(|t| t.read(cx).list.read(cx).rows().lines.to_vec())
        .unwrap_or_default()
}

/// Selects line `line` of the text view (extending from the last click when `extend`).
pub fn text_select(console: ConsoleId, line: usize, extend: bool, cx: &mut App) {
    if let Some(t) = text_view(console, cx) {
        t.update(cx, |t, cx| t.select(line, extend, cx));
    }
}

/// The selected lines of the text view, as Copy copies them.
pub fn text_selection(console: ConsoleId, cx: &App) -> Option<String> {
    Some(text_view(console, cx)?.read(cx).selected_text(cx)?.0)
}

/// Copies the text view's selected lines.
pub fn text_copy(console: ConsoleId, cx: &mut App) {
    if let Some(t) = text_view(console, cx) {
        t.update(cx, |t, cx| t.copy(cx));
    }
}

/// The question's Show as text.
pub fn text_show_anyway(console: ConsoleId, cx: &mut App) {
    if let Some(t) = text_view(console, cx) {
        t.update(cx, |t, cx| t.show_anyway(cx));
    }
}

/// The question's Back to the table (or the build's Cancel).
pub fn text_back(console: ConsoleId, cx: &mut App) {
    if let Some(t) = text_view(console, cx) {
        t.update(cx, |t, cx| t.back(cx));
    }
}

// ---- grid commands under the views ----------------------------------------------------------------

/// Runs grid command `cmd` on the view `window` shows when it is not the table: the Transposed
/// view's grid takes it (but find and go to row, which are the table's), the Text view takes
/// Copy. `false`: the table's grid takes it.
pub(crate) fn route(cmd: GridCommand, window: &mut Window, cx: &mut App) -> bool {
    let Some((console, _)) = super::view::console_in(window, cx) else {
        return false;
    };
    let Some((grid, _)) = active(console, cx) else {
        return false;
    };
    match tab_of(grid.entity_id(), cx)
        .map(|t| t.mode)
        .unwrap_or_default()
    {
        ViewMode::Table => false,
        ViewMode::Transposed => {
            let off = matches!(
                cmd,
                GridCommand::Find | GridCommand::GotoRow | GridCommand::SelectAllOccurrences
            );
            if !off && let Some(t) = transposed_grid_of(&grid, cx) {
                t.update(cx, |g, cx| g.dispatch(cmd, window, cx));
            }
            true
        }
        ViewMode::Text => {
            if let GridCommand::Copy { .. } = cmd {
                text_copy(console, cx);
            }
            true
        }
    }
}

// ---- the tile's parts ----------------------------------------------------------------------------

/// The tile's grid area for console `console`'s active result: the table (`table`, its cached
/// view), the Transposed view's grid, or the text view.
pub(crate) fn main_area(
    console: ConsoleId,
    grid: &Entity<GridView>,
    table: AnyElement,
    cx: &App,
) -> AnyElement {
    let _ = console;
    let Some(tab) = tab_of(grid.entity_id(), cx) else {
        return table;
    };
    let shown = match tab.mode {
        ViewMode::Table => return table,
        ViewMode::Transposed => match &tab.transposed {
            Some(t) => {
                let mut style = StyleRefinement::default();
                style.size.width = Some(hacivat_ui_core::gpui_surface::relative(1.0).into());
                style.size.height = Some(hacivat_ui_core::gpui_surface::relative(1.0).into());
                AnyView::from(t.grid.clone())
                    .cached(style)
                    .into_any_element()
            }
            None => return table,
        },
        ViewMode::Text => match &tab.text {
            Some(t) => t.clone().into_any_element(),
            None => return table,
        },
    };
    // The table grid keeps taking its frames, at no size, so its state (a re-run's rows, the
    // view map that follows them) stays the one every view is built from.
    div()
        .size_full()
        .relative()
        .child(shown)
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_0()
                .overflow_hidden()
                .child(table),
        )
        .into_any_element()
}

/// Whether `window`'s record inspector is open (the tile layout's).
pub fn record_open(window: &Window, cx: &App) -> bool {
    window
        .root::<Workspace>()
        .flatten()
        .is_some_and(|ws| ws.read(cx).layout().1.inspector_visible)
}

/// Opens or closes `window`'s record inspector (the toggle, `grid.toggle_record`).
pub fn toggle_record(window: &mut Window, cx: &mut App) {
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    let open = ws.read(cx).layout().1.inspector_visible;
    ws.update(cx, |ws, cx| ws.set_inspector_visible(!open, cx));
    notify_hosts(cx);
}

/// One field of the record inspector.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordField {
    pub col: ColId,
    /// The column's name (catalog text).
    pub name: SharedString,
    /// `(this one, of how many)` when the result has several columns with this name; the
    /// suffix shows from the second.
    pub dup: Option<(u64, u64)>,
    pub type_label: SharedString,
    /// The value as the cell shows it, clamped to three lines (`cut`).
    pub value: SharedString,
    pub cut: bool,
    pub null: bool,
    /// Aligned to the end (numbers).
    pub end: bool,
    /// In mono (numbers, temporal values, bytes, UUIDs).
    pub mono: bool,
    /// The raw value under its tooltip cap (`Purpose::Tooltip`).
    pub tooltip: SharedString,
}

/// What the record inspector shows.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordView {
    pub row: ModelRow,
    /// "Row 4 of 184" (view rows).
    pub position: SharedString,
    pub fields: Vec<RecordField>,
}

/// The model row the record inspector shows for console `console`: the focus of the view shown
/// (the Transposed view's, mapped; the text view's selected line), else the first row shown.
fn record_row(console: ConsoleId, grid: &Entity<GridView>, cx: &App) -> Option<ModelRow> {
    let mode = tab_of(grid.entity_id(), cx)
        .map(|t| t.mode)
        .unwrap_or_default();
    let focused = match mode {
        ViewMode::Table => grid.read(cx).selection().focus().map(|f| f.row),
        ViewMode::Transposed => transposed_focus(grid, cx).map(|c| c.row),
        ViewMode::Text => {
            let _ = console;
            tab_of(grid.entity_id(), cx)
                .and_then(|t| t.text.as_ref())
                .and_then(|t| t.read(cx).selected_row(cx))
        }
    };
    let map = grid.read(cx).view_map();
    focused
        .filter(|r| map.model_to_view(*r).is_some())
        .or_else(|| map.view_to_model(ViewRow(0)))
}

/// What console `console`'s record inspector shows (`None` without a row).
pub fn record(console: ConsoleId, cx: &App) -> Option<RecordView> {
    let (grid, adapter) = active(console, cx)?;
    record_of(console, &grid, &adapter, cx)
}

/// [`record`] over the table grid `grid` and its `adapter` (the tile has them: it never looks
/// itself up while it is being updated).
fn record_of(
    console: ConsoleId,
    grid: &Entity<GridView>,
    adapter: &Rc<GridAdapter>,
    cx: &App,
) -> Option<RecordView> {
    let row = record_row(console, grid, cx)?;
    RECORD_READS.with(|r| r.set(r.get() + 1));
    let g = grid.read(cx);
    let map = g.view_map();
    let view_row = map.model_to_view(row)?.0;
    let position = message(
        cx,
        "hv-record-pos",
        vec![
            ("i", ArgValue::Count(view_row + 1)),
            ("n", ArgValue::Count(map.len())),
        ],
    );
    let raw = adapter.raw_view();
    if row.0 >= raw.snapshot.row_count() {
        return None;
    }
    let specs = adapter.columns();
    let display = adapter.display();
    // Duplicate names, counted over every column of the result (stable under hiding).
    let mut seen: HashMap<&str, u64> = HashMap::new();
    let mut nth: Vec<u64> = Vec::with_capacity(specs.len());
    for s in specs.iter() {
        let n = seen.entry(s.name.as_ref()).or_default();
        *n += 1;
        nth.push(*n);
    }
    let mut fields = Vec::new();
    for col in g.column_layout().visible() {
        let i = col.0 as usize;
        let (Some(spec), Some(facts)) = (specs.get(i), raw.facts.get(i)) else {
            continue;
        };
        let total = seen.get(spec.name.as_ref()).copied().unwrap_or(1);
        let value = raw.snapshot.cell(row.0, col.0);
        let fv = facts.view(display.engine());
        // ## m3c-t39 (named exception): a raw column's value as its cell shows it.
        let shown = format_cell(value, &fv, display.settings_for(&spec.name), Purpose::Cell);
        let tip = format_cell(value, &fv, display.settings(), Purpose::Tooltip);
        let (text, cut) = clamp_value(&shown.text);
        let cut = cut || shown.flags & CellFlags::TRUNCATED.bits() != 0;
        fields.push(RecordField {
            col: *col,
            name: spec.name.clone(),
            dup: (total > 1).then_some((nth[i], total)),
            type_label: spec.type_label.clone(),
            value: text.into(),
            cut,
            null: shown.kind == DisplayKind::Null,
            end: shown.align_end,
            mono: matches!(
                shown.kind,
                DisplayKind::Number | DisplayKind::Temporal | DisplayKind::Binary
            ) || spec.kind == ColumnKind::Uuid,
            tooltip: tip.text.into(),
        });
    }
    Some(RecordView {
        row,
        position,
        fields,
    })
}

// ## m3c-t38 fix round 1 (m2)
/// [`record_of`], read again only when what it reads changed (the row, the columns shown, the
/// rows, the display settings, the language): a tile frame formats no field otherwise.
fn record_cached(
    console: ConsoleId,
    grid: &Entity<GridView>,
    adapter: &Rc<GridAdapter>,
    cx: &mut App,
) -> Option<RecordView> {
    let row = record_row(console, grid, cx)?;
    let key = {
        let g = grid.read(cx);
        RecordKey {
            grid: grid.entity_id(),
            generation: g.generation(),
            adapter_generation: adapter.generation(),
            row,
            view_row: g.view_map().model_to_view(row).map_or(u64::MAX, |v| v.0),
            rows: g.view_map().len(),
            cols: g.column_layout().visible().to_vec(),
            display: adapter.display().settings().version,
            l10n: cx
                .try_global::<crate::l10n_global::L10nGlobal>()
                .map(|l| l.l10n.generation()),
        }
    };
    if let Some((k, r)) = cx
        .try_global::<Views>()
        .and_then(|v| v.record_cache.get(&console))
        && *k == key
    {
        return Some(r.clone());
    }
    let record = record_of(console, grid, adapter, cx)?;
    cx.default_global::<Views>()
        .record_cache
        .insert(console, (key, record.clone()));
    Some(record)
}

/// How many records the inspector has read since the app started (tests: a frame with the same
/// record reads none).
pub fn record_reads(cx: &App) -> u64 {
    let _ = cx;
    RECORD_READS.with(Cell::get)
}

thread_local! {
    static RECORD_READS: Cell<u64> = const { Cell::new(0) };
}

/// `text` clamped to [`RECORD_LINES`] lines of about [`RECORD_LINE_CHARS`] characters (with
/// `…`), and whether it was cut.
fn clamp_value(text: &str) -> (String, bool) {
    let mut out = String::new();
    let mut lines = 0;
    let mut col = 0;
    for ch in text.chars() {
        if ch == '\n' || col == RECORD_LINE_CHARS {
            lines += 1;
            col = 0;
            if lines == RECORD_LINES {
                out.push('…');
                return (out, true);
            }
            if ch == '\n' {
                out.push(ch);
                continue;
            }
        }
        out.push(ch);
        col += 1;
    }
    (out, false)
}

/// Moves console `console`'s record to the previous (`back`) or next row as shown: the focus of
/// the view shown moves there (in the same column).
pub fn record_step(console: ConsoleId, back: bool, cx: &mut App) {
    let Some((grid, _)) = active(console, cx) else {
        return;
    };
    let Some(row) = record_row(console, &grid, cx) else {
        return;
    };
    let (target, col) = {
        let g = grid.read(cx);
        let map = g.view_map();
        let Some(v) = map.model_to_view(row) else {
            return;
        };
        let next = if back {
            v.0.checked_sub(1)
        } else {
            Some(v.0 + 1).filter(|n| *n < map.len())
        };
        let Some(target) = next.and_then(|n| map.view_to_model(ViewRow(n))) else {
            return;
        };
        let col = g
            .selection()
            .focus()
            .map(|f| f.col)
            .or_else(|| g.column_layout().visible().first().copied());
        (target, col)
    };
    let mode = tab_of(grid.entity_id(), cx)
        .map(|t| t.mode)
        .unwrap_or_default();
    match mode {
        ViewMode::Table => {
            if let Some(col) = col {
                grid.update(cx, |g, cx| g.focus_cell(ModelCell { row: target, col }, cx));
            }
        }
        ViewMode::Transposed => {
            let focus = transposed_focus(&grid, cx);
            let col = focus.map(|f| f.col).or(col);
            if let (Some(col), Some(t), Some(source)) = (
                col,
                transposed_grid_of(&grid, cx),
                transposed_source(console, cx),
            ) && let Some(cell) = source.transposed_cell(ModelCell { row: target, col })
            {
                t.update(cx, |g, cx| g.focus_cell(cell, cx));
            }
        }
        ViewMode::Text => {
            if let Some(t) = text_view(console, cx) {
                t.update(cx, |t, cx| t.select_row(target, cx));
            }
        }
    }
    notify_hosts(cx);
}

/// The virtualized fields (one line each), above [`RECORD_VIRTUAL_ABOVE`] columns.
#[derive(Clone, Default)]
struct FieldRows {
    console: Option<ConsoleId>,
    fields: Rc<Vec<RecordField>>,
}

impl RowRenderer for FieldRows {
    fn len(&self) -> u64 {
        self.fields.len() as u64
    }

    fn row(&self, ix: u64, _window: &mut Window, cx: &mut App) -> AnyElement {
        match (self.console, self.fields.get(ix as usize)) {
            (Some(console), Some(f)) => field_element(console, f, ix as usize, true, cx),
            _ => div().into_any_element(),
        }
    }
}

/// One field of the inspector (`compact`: one value line, for the virtualized list).
fn field_element(
    console: ConsoleId,
    f: &RecordField,
    ix: usize,
    compact: bool,
    cx: &App,
) -> AnyElement {
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let c = &t.colors;
    let mut name_row = div().flex().flex_row().items_center().gap(px(4.0)).child(
        Label::new(
            f.name.clone(),
            TextSource::Catalog,
            t.type_ramp.mono_kbd,
            c.text.to_hsla(),
        )
        .truncate(true),
    );
    if let Some((i, n)) = f.dup.filter(|(i, _)| *i > 1) {
        let tip = message(
            cx,
            "hv-record-dup-tip",
            vec![
                ("n", ArgValue::Count(n)),
                ("name", ArgValue::Text(f.name.to_string())),
                ("i", ArgValue::Count(i)),
            ],
        );
        name_row = name_row.child(
            div()
                .id(("record-dup", ix))
                .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
                .child(Label::new(
                    message(cx, "hv-record-dup", vec![("i", ArgValue::Count(i))]),
                    TextSource::Fluent,
                    t.type_ramp.mono_kbd,
                    c.text_2.to_hsla(),
                )),
        );
    }
    let name_row = name_row.child(div().flex_1()).child(Label::new(
        f.type_label.clone(),
        TextSource::Catalog,
        t.type_ramp.mono_kbd,
        c.text_3.to_hsla(),
    ));
    let style = if f.mono {
        t.type_ramp.mono_data
    } else {
        t.type_ramp.body
    };
    let value: AnyElement = if f.null {
        Label::new(
            NULL_MARKER,
            TextSource::Data,
            t.type_ramp.mono_kbd,
            c.text_3.to_hsla(),
        )
        .into_any_element()
    } else if compact {
        Label::new(f.value.clone(), TextSource::Data, style, c.text.to_hsla())
            .truncate(true)
            .into_any_element()
    } else {
        paragraph(f.value.clone(), TextSource::Data, style, c.text.to_hsla()).into_any_element()
    };
    let tip = f.tooltip.clone();
    let mut value_row = div()
        .id(("record-value", ix))
        .flex()
        .flex_row()
        .tooltip(move |_, cx| Tip::view(tip.clone(), cx));
    if f.end {
        value_row = value_row.justify_end();
    }
    let mut field = div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .pt(px(5.0))
        .pb(px(6.0))
        .px(px(12.0))
        .border_b_1()
        .border_color(c.seam_soft.to_hsla())
        .child(name_row)
        .child(value_row.child(value));
    if compact {
        field = field.h(px(RECORD_FIELD_PX)).overflow_hidden();
    }
    if f.cut && !f.null {
        let col = f.col;
        field = field.child(
            div().flex().flex_row().child(
                Button::new(
                    ("record-show-more", ix),
                    message(cx, "hv-record-show-more", vec![]),
                )
                .kind(ButtonKind::Ghost)
                .compact(true)
                .on_click(move |_, _, cx| {
                    let row = active(console, cx).and_then(|(g, _)| record_row(console, &g, cx));
                    if let Some(row) = row {
                        super::value_viewer::open(console, ModelCell { row, col }, cx);
                    }
                }),
            ),
        );
    }
    field.into_any_element()
}

/// The record inspector of console `console` at `width` px, when `window`'s is open.
pub(crate) fn inspector(
    console: ConsoleId,
    grid: &Entity<GridView>,
    adapter: &Rc<GridAdapter>,
    width: f32,
    window: &Window,
    cx: &mut App,
) -> Option<AnyElement> {
    if !record_open(window, cx) {
        return None;
    }
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let c = &t.colors;
    let record = record_cached(console, grid, adapter, cx);
    let focus = cx
        .default_global::<Views>()
        .record_focus
        .get(&console)
        .cloned();
    let focus = match focus {
        Some(f) => f,
        None => {
            let f = cx.focus_handle();
            cx.default_global::<Views>()
                .record_focus
                .insert(console, f.clone());
            f
        }
    };
    let step = |id: &'static str, icon: IconName, msg: &'static str, back: bool, cx: &App| {
        Button::new(id, message(cx, msg, vec![]))
            .kind(ButtonKind::Icon)
            .icon(icon)
            .on_click(move |_, _, cx| record_step(console, back, cx))
    };
    // ## m3c-shotfix F6: the title and "Row 4 of 184" share one line while both fit; a longer
    // position wraps under the title (chrome wraps, NOTES cross-cutting 4) and the header grows
    // from its 30 px, so the position always shows whole.
    let header = div()
        .min_h(px(RECORD_HEADER_PX))
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .px(px(10.0))
        .py(px(2.0))
        .bg(c.head.to_hsla())
        .border_b_1()
        .border_color(c.seam.to_hsla())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_x(px(6.0))
                .child(
                    Label::new(
                        message(cx, "hv-record", vec![]),
                        TextSource::Fluent,
                        t.type_ramp.label_sc,
                        c.text_2.to_hsla(),
                    )
                    .id("record-title"),
                )
                .child(
                    Label::new(
                        record
                            .as_ref()
                            .map(|r| r.position.clone())
                            .unwrap_or_default(),
                        TextSource::Fluent,
                        t.type_ramp.mono_kbd,
                        c.text_2.to_hsla(),
                    )
                    .id("record-pos"),
                ),
        )
        .child(step(
            "record-prev",
            IconName::ArrowUp,
            "hv-record-prev",
            true,
            cx,
        ))
        .child(step(
            "record-next",
            IconName::ArrowDown,
            "hv-record-next",
            false,
            cx,
        ));
    let fields: AnyElement = match &record {
        Some(r) if r.fields.len() > RECORD_VIRTUAL_ABOVE => {
            let list = cx.default_global::<Views>().records.get(&console).cloned();
            let list = match list {
                Some(l) => l,
                None => {
                    let l = cx.new(|_| VirtualList::new(FieldRows::default(), RECORD_FIELD_PX));
                    cx.default_global::<Views>()
                        .records
                        .insert(console, l.clone());
                    l
                }
            };
            let fields = Rc::new(r.fields.clone());
            list.update(cx, |l, cx| {
                let rows = l.rows_mut(cx);
                rows.console = Some(console);
                rows.fields = fields;
            });
            div().flex_1().min_h_0().child(list).into_any_element()
        }
        Some(r) => div()
            .id("record-fields")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(hacivat_ui::text::scroll_viewport())
            .children(
                r.fields
                    .iter()
                    .enumerate()
                    .map(|(i, f)| field_element(console, f, i, false, cx)),
            )
            .into_any_element(),
        None => div()
            .p(px(12.0))
            .child(Label::new(
                message(cx, "hv-grid-no-rows", vec![]),
                TextSource::Fluent,
                t.type_ramp.body_sm,
                c.text_2.to_hsla(),
            ))
            .into_any_element(),
    };
    let panel = div()
        .id(("record-inspector", console.0))
        .debug_selector(|| "record-inspector".to_string())
        .track_focus(&focus)
        .w(px(width))
        .h_full()
        .flex_none()
        .flex()
        .flex_col()
        .bg(c.gutter.to_hsla())
        .border_l_1()
        .border_color(c.seam.to_hsla())
        .on_key_down(move |ev: &KeyDownEvent, _, cx| {
            let k = &ev.keystroke;
            let back = match k.key.as_str() {
                "up" => true,
                "down" => false,
                _ => return,
            };
            if k.modifiers.alt && !k.modifiers.secondary(cx) && !k.modifiers.shift {
                record_step(console, back, cx);
                cx.stop_propagation();
            }
        })
        .child(header)
        .child(fields);
    Some(
        A11yNode::new(
            "record-inspector-node",
            Role::Group,
            message(cx, "hv-record-label", vec![]),
            false,
            panel,
        )
        .into_any_element(),
    )
}

// ---- the toolbar's right group -------------------------------------------------------------------

/// The view switch's three segments and the record toggle, in that order.
const SWITCH_MODES: [ViewMode; 3] = [ViewMode::Table, ViewMode::Transposed, ViewMode::Text];

/// Console `console`'s switch and toggle focus handles (fix round 1, m5: keyboard reachable;
/// one tab stop for the switch, its pressed segment; one for the toggle).
fn switch_handles(console: ConsoleId, cx: &mut App) -> [FocusHandle; 4] {
    if let Some(h) = cx
        .try_global::<Views>()
        .and_then(|v| v.switch_focus.get(&console))
    {
        return h.clone();
    }
    let h = std::array::from_fn(|_| cx.focus_handle());
    cx.default_global::<Views>()
        .switch_focus
        .insert(console, h.clone());
    h
}

/// The focus handle of the view switch's pressed segment for console `console` (tests; the
/// keyboard reaches it with Tab).
pub fn view_switch_focus(console: ConsoleId, cx: &mut App) -> FocusHandle {
    let shown = mode(console, cx);
    let ix = SWITCH_MODES.iter().position(|m| *m == shown).unwrap_or(0);
    switch_handles(console, cx)[ix].clone()
}

/// The record toggle's focus handle for console `console` (tests).
pub fn record_toggle_focus(console: ConsoleId, cx: &mut App) -> FocusHandle {
    switch_handles(console, cx)[3].clone()
}

/// One segment of the view switch.
#[allow(clippy::too_many_arguments)]
fn segment(
    console: ConsoleId,
    ix: usize,
    shown: ViewMode,
    icon: IconName,
    words: Option<SharedString>,
    name: SharedString,
    reason: Option<SharedString>,
    handles: &[FocusHandle; 4],
    window: &Window,
    cx: &App,
) -> AnyElement {
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let c = &t.colors;
    let mode = SWITCH_MODES[ix];
    let pressed = mode == shown;
    let disabled = reason.is_some();
    let ink = if disabled {
        c.text_3
    } else if pressed {
        c.text
    } else {
        c.text_2
    };
    let fill = match theme.mode {
        ThemeMode::Dark => c.raised,
        ThemeMode::Light => c.panel,
    };
    let tip = reason.clone().unwrap_or_else(|| name.clone());
    let id = match mode {
        ViewMode::Table => "view-table",
        ViewMode::Transposed => "view-transposed",
        ViewMode::Text => "view-text",
    };
    let stop = SWITCH_MODES.iter().position(|m| *m == shown).unwrap_or(0);
    let handle = handles[ix].clone().tab_stop(ix == stop);
    let focused = handle.is_focused(window) && window.last_input_was_keyboard();
    let roving: Vec<FocusHandle> = handles[..3].to_vec();
    let mut seg = div()
        .id(id)
        .debug_selector(move || format!("segment-{id}"))
        .track_focus(&handle)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .h_full()
        .px(px(7.0))
        .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
        .on_key_down(move |ev: &KeyDownEvent, window, cx| {
            let k = &ev.keystroke;
            if k.modifiers.modified() {
                return;
            }
            let next = match k.key.as_str() {
                "enter" | "space" => {
                    if !disabled {
                        set_mode(console, mode, cx);
                    }
                    cx.stop_propagation();
                    return;
                }
                "right" => (ix + 1) % 3,
                "left" => (ix + 2) % 3,
                _ => return,
            };
            cx.stop_propagation();
            window.focus(&roving[next], cx);
            set_mode(console, SWITCH_MODES[next], cx);
        })
        .child(Icon::new(icon).size(IconSize::Chrome).color(ink.to_hsla()));
    if let Some(words) = words {
        seg = seg.child(Label::new(
            words,
            TextSource::Fluent,
            t.type_ramp.caption,
            ink.to_hsla(),
        ));
    }
    if pressed {
        seg = seg.bg(fill.to_hsla());
    }
    if !disabled {
        seg = seg
            .cursor_pointer()
            .hover(|s| s.text_color(c.text.to_hsla()))
            .on_click(move |_, _, cx| {
                set_mode(console, mode, cx);
            });
    }
    focus_outline(
        seg.declare(Role::Button, name, None)
            .aria_toggled(accesskit::Toggled::from(pressed)),
        &theme,
        focused,
    )
    .into_any_element()
}

/// The view switch (`words`: with the segments' words) showing `shown`, Transposed disabled
/// for `refused` rows.
fn view_switch(
    console: ConsoleId,
    shown: ViewMode,
    refused: Option<u64>,
    words: bool,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let handles = switch_handles(console, cx);
    let theme = *Theme::get(cx);
    let c = &theme.tokens.colors;
    let reason = refused.map(|rows| {
        message(
            cx,
            "hv-view-transposed-off",
            vec![
                ("max", ArgValue::Count(TRANSPOSED_MAX_ROWS)),
                ("rows", ArgValue::Count(rows)),
            ],
        )
    });
    let segments = [
        (IconName::Table2, "hv-view-table"),
        (IconName::TableProperties, "hv-view-transposed"),
        (IconName::AlignLeft, "hv-view-text"),
    ];
    let mut group = div()
        .id("view-switch")
        .debug_selector(|| "view-switch".to_string())
        .flex()
        .flex_row()
        .items_center()
        .flex_none()
        .h(px(SWITCH_PX))
        .border_1()
        .border_color(c.seam_strong.to_hsla())
        .rounded(px(theme.tokens.radii.sm))
        .overflow_hidden();
    for (i, (icon, msg)) in segments.into_iter().enumerate() {
        if i > 0 {
            group = group.child(div().w(px(1.0)).h_full().bg(c.seam_strong.to_hsla()));
        }
        let name = message(cx, msg, vec![]);
        let why = (SWITCH_MODES[i] == ViewMode::Transposed)
            .then(|| reason.clone())
            .flatten();
        group = group.child(segment(
            console,
            i,
            shown,
            icon,
            words.then(|| name.clone()),
            name,
            why,
            &handles,
            window,
            cx,
        ));
    }
    A11yNode::new(
        "view-switch-node",
        Role::RadioGroup,
        message(cx, "hv-view-label", vec![]),
        false,
        group,
    )
    .into_any_element()
}

/// The record toggle (`word`: with "Record", a one-row result).
fn record_toggle(
    console: ConsoleId,
    open: bool,
    word: bool,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let handle = switch_handles(console, cx)[3].clone().tab_stop(true);
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let c = &t.colors;
    let name = message(cx, "hv-record-tip", vec![]);
    let tip = name.clone();
    let ink = if open { c.text } else { c.text_2 };
    let focused = handle.is_focused(window) && window.last_input_was_keyboard();
    let mut b = div()
        .id("record-toggle")
        .debug_selector(|| "record-toggle".to_string())
        .track_focus(&handle)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .h(px(SWITCH_PX))
        .px(px(5.0))
        .rounded(px(t.radii.sm))
        .border_1()
        .border_color(if open {
            c.blue.to_hsla()
        } else {
            hsla(0.0, 0.0, 0.0, 0.0)
        })
        .cursor_pointer()
        .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
        .on_click(|_, window, cx| toggle_record(window, cx))
        .on_key_down(|ev: &KeyDownEvent, window, cx| {
            let k = &ev.keystroke;
            if !k.modifiers.modified() && matches!(k.key.as_str(), "enter" | "space") {
                cx.stop_propagation();
                toggle_record(window, cx);
            }
        })
        .child(
            Icon::new(IconName::PanelRight)
                .size(IconSize::Chrome)
                .color(ink.to_hsla()),
        );
    if open {
        b = b.bg(c.blue_tint_strong.to_hsla());
    } else {
        b = b.hover(|s| s.bg(c.hover.to_hsla()));
    }
    if word {
        b = b.child(Label::new(
            message(cx, "hv-record", vec![]),
            TextSource::Fluent,
            t.type_ramp.caption,
            ink.to_hsla(),
        ));
    }
    focus_outline(
        b.declare(Role::Button, name, None)
            .aria_toggled(accesskit::Toggled::from(open)),
        &theme,
        focused,
    )
    .into_any_element()
}

/// The result toolbar's right-group items after rows · timing: the view switch (its words go
/// first, M3c-28) and the record toggle (with its word for a one-row result, M3c-29).
pub(crate) fn toolbar_items(
    console: ConsoleId,
    grid: &Entity<GridView>,
    window: &Window,
    cx: &App,
) -> Vec<ToolbarItem> {
    let label: TextFn = Rc::new(|cx: &App| message(cx, "hv-view-label", vec![]));
    let shown = tab_of(grid.entity_id(), cx)
        .map(|t| t.mode)
        .unwrap_or_default();
    let refused = transposed_refused(grid, cx);
    let switch = ToolbarItem::pinned(
        Rc::new(move |window, cx| view_switch(console, shown, refused, true, window, cx)),
        label,
    )
    .compact(Rc::new(move |window, cx| {
        view_switch(console, shown, refused, false, window, cx)
    }))
    // ## m3c-t41 merge (M3c-18): the view items compact before Copy gives way.
    .priority(crate::results::toolbar::VIEW_ITEMS_PRIORITY);
    let open = record_open(window, cx);
    let one_row = shown_rows(grid, cx) == 1;
    let record_label: TextFn = Rc::new(|cx: &App| message(cx, "hv-record-tip", vec![]));
    let record = ToolbarItem::pinned(
        Rc::new(move |window, cx| record_toggle(console, open, one_row, window, cx)),
        record_label,
    )
    .priority(crate::results::toolbar::VIEW_ITEMS_PRIORITY);
    vec![switch, record]
}

// ---- commands ------------------------------------------------------------------------------------

/// The console `window` shows, with a result tab active.
fn console_with_result(window: &Window, cx: &App) -> Option<ConsoleId> {
    let (console, _) = super::view::console_in(window, cx)?;
    active(console, cx).map(|_| console)
}

/// Registers the view commands (palette-only, no keymap row; context `Grid`).
pub fn register(cx: &mut App) {
    let reg = cx.default_global::<ActionRegistry>();
    for (id, title_id) in COMMANDS {
        let (run, enabled): (hacivat_ui::actions::ActionFn, fn(&App) -> bool) = match id {
            "grid.view_table" => (
                Rc::new(|window, cx| {
                    if let Some(c) = console_with_result(window, cx) {
                        set_mode(c, ViewMode::Table, cx);
                    }
                }),
                |cx| super::view::active_entry(cx).is_some(),
            ),
            "grid.view_transposed" => (
                Rc::new(|window, cx| {
                    if let Some(c) = console_with_result(window, cx) {
                        set_mode(c, ViewMode::Transposed, cx);
                    }
                }),
                |cx| {
                    super::view::active_entry(cx)
                        .is_some_and(|(console, _, _)| transposed_reason(console, cx).is_none())
                },
            ),
            "grid.view_text" => (
                Rc::new(|window, cx| {
                    if let Some(c) = console_with_result(window, cx) {
                        set_mode(c, ViewMode::Text, cx);
                    }
                }),
                |cx| super::view::active_entry(cx).is_some(),
            ),
            _ => (Rc::new(toggle_record), |cx| {
                super::view::active_entry(cx).is_some()
            }),
        };
        let _ = reg.register(ActionDef {
            id,
            title_id,
            context: GRID_CONTEXT,
            run,
            enabled,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_keeps_three_lines() {
        assert_eq!(clamp_value("short"), ("short".to_string(), false));
        let (text, cut) = clamp_value("a\nb\nc\nd");
        assert_eq!((text.as_str(), cut), ("a\nb\nc…", true));
        let long = "x".repeat(RECORD_LINE_CHARS * 4);
        let (text, cut) = clamp_value(&long);
        assert!(cut);
        assert_eq!(text.chars().count(), RECORD_LINE_CHARS * 3 + 1);
    }

    #[test]
    fn rules_and_lines_pad_to_the_widths() {
        assert_eq!(rule_line(&[2, 4]), "+----+------+");
        assert_eq!(
            table_line(&["7".into(), "ab".into()], &[2, 4], &[true, false]),
            "|  7 | ab   |"
        );
    }
}
