//! The grid find bar (HV-GRID-09, R-M3-10; M3c Task 37): it replaces the result toolbar's left
//! group, scans display strings off the UI thread, highlights matches
//! (`GridView::set_highlights`) and filters rows (`ViewMap::Filtered`).
//!
//! ## m3b-t01b-seams: the seam (Task 1b). STUB: opens nothing (`GridEvent::FindRequested` is
//! not answered yet). Task 37 fills it.
//!
//! ## m3c-t37
//!
//! **The bar** (NOTES surface 19) takes the result toolbar's left group (Copy, Count rows) in
//! the 34 px toolbar: the find field (Task 33's well and toggles, `console::find_bar`) with five
//! toggles (Match case, Whole words, Regular expression, Filter rows, Turkish-tolerant), the
//! count, previous / next, Select all occurrences, close; the right group (rows · timing) stays
//! and is the first to give way. One bar per console; it follows the console's active result
//! tab (a find on another tab starts anew there).
//!
//! **The matcher is Task 33's** (`find_bar::build_matcher`; an invalid pattern searches
//! nothing and shows "Invalid pattern" in ink). It runs over **what the grid shows**: each
//! visible column's display string (`valfmt::format_cell`, `Purpose::Cell`: grouped numbers,
//! the display zone, the grapheme cap), never the raw value; NULL and a decode error never
//! match. A match is a **cell** (its ranges are all marked; a step goes cell to cell); a value
//! the store cut at its cell cap is searched up to the cut, and the count says "may be
//! incomplete".
//!
//! **The scan runs on the background executor**, in batches of 1,000 rows from the focus cell
//! (the view rows from there to the end, then from the top to there), in slices of about 8 ms
//! that each report to the bar; a new query (or toggle, or a re-run) drops the old scan (a
//! generation number, and its flag stops the slice running). The first match at or after where
//! find started becomes the grid's focus when its slice lands: one selection change per
//! keystroke. At most 100,000 matching cells are kept (steps, marks); the count goes on. Rows
//! fetched later are scanned when they arrive (they come after the scanned ones in the view).
//!
//! **Marks** (layer 5 of the grid): the kept cells' matching text ranges, outlined; the
//! current one filled. **Select all occurrences** selects each matching cell as its own range,
//! at most 10,000 (`find_bar::SELECT_ALL_CAP`); past that a notice says "Selected the first
//! 10,000 of N matches" (the grid has no status-bar sentence of its own).
//!
//! **Filter rows** shows only the rows with a match: `ViewMap::Filtered` over the scanned rows,
//! in the order shown, built on the background executor when the scan is done; the grid then
//! scrolls over the kept rows only, numbered 1…n, and a note row under the toolbar says so with
//! Show all rows (NOTES M3c-26). A focus the filter hides moves to the current match (logged
//! ruling). **The view map has one owner here** ([`show_base`]): the local sort hands it its
//! order (Task 36 review m6) and the filter composes over it, so a sort under a filter orders
//! the kept rows and keeps the filter, Clear sort restores the filter in the source order, and
//! turning the filter off shows the sort again.
//!
//! **Keys.** `grid.find` (⌘F / Ctrl+F in a grid) opens the bar with the focus in its field;
//! Enter / Shift+Enter step; `grid.select_all_occurrences` (⌃⌘G / Ctrl+Shift+L) selects; Escape
//! (or close) ends find, drops the marks and the filter, and gives the focus back to the grid.
//! The field carries the `Grid` key context: `grid.find` and `grid.select_all_occurrences`
//! act on the bar while it has the focus ([`run_in_bar`]); every other grid command is
//! disabled there, so its keys stay the field's ([`field_focused`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use hacivat_l10n::ArgValue;
use hacivat_results::StoreSnapshot;
use hacivat_settings::FindTolerancePref;
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::input::{FocusedInput, InputEvent, TextInput};
use hacivat_ui::popover::{A11yNode, Tip};
use hacivat_ui::step_trail::run_marker;
use hacivat_ui::text::{Label, TextFn, TextSource, paragraph};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui::toolbar::{Toolbar, ToolbarItem};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext as _, Context, Entity, EntityId, FocusHandle, Focusable as _,
    Global, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task, WeakEntity,
    Window, div, px,
};
use hacivat_ui_core::theme::{Theme, focus_outline};
use hacivat_ui_editor::FindMatcher;
use hacivat_ui_grid::model::ViewMap;
use hacivat_ui_grid::{
    CellMarks, GRID_CONTEXT, GridView, Highlights, ModelCell, ModelRow, ViewRow,
};
use hacivat_values::CellFlags;
use hacivat_values::valfmt::{DisplayKind, Purpose, format_cell};
use smallvec::SmallVec;

use crate::console::ConsoleId;
use crate::console::find_bar::{
    FindOpts, RegexMatcher, SELECT_ALL_CAP, ToggleSpec, build_matcher, count_parts, field_well,
    find_pasteboard, toggle_element, tolerant_default, write_find_pasteboard,
};
use crate::exec_ui::marks::message;
use crate::results::adapter::{ColFacts, GridAdapter, ResultDisplay, Texts};

// ---- the scan (pure, `Send`) --------------------------------------------------------------------

/// Rows per batch: the scan looks at its stop flag between batches (the plan's 1,000).
pub const BATCH_ROWS: u64 = 1_000;
/// A slice of batches runs about this long on the background executor, then reports.
const SLICE: Duration = Duration::from_millis(8);
/// The most matching cells kept for the steps and the marks; the count goes on past it.
pub const MAX_KEPT: usize = 100_000;
/// The most matching ranges marked in one cell.
const MAX_RANGES: usize = 32;

/// A matching cell: its view row (in the order the scan read), model row, column (and its
/// place among the scanned columns), and the matching byte ranges of its display string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub view: u64,
    pub row: u64,
    pub col_ix: u32,
    pub col: u32,
    pub ranges: SmallVec<[Range<u32>; 2]>,
}

impl Hit {
    /// Its place in the view: (view row, column place).
    fn key(&self) -> (u64, u32) {
        (self.view, self.col_ix)
    }

    fn cell(&self) -> ModelCell {
        ModelCell {
            row: ModelRow(self.row),
            col: hacivat_ui_grid::ColId(self.col),
        }
    }
}

/// What a scan reads (every part `Send`; cloned cheaply).
#[derive(Clone)]
pub struct ScanInput {
    snapshot: Arc<StoreSnapshot>,
    facts: Arc<[ColFacts]>,
    display: Arc<ResultDisplay>,
    /// The visible columns, in the order shown.
    cols: Arc<[u32]>,
    /// The order the rows are shown in (the sort's, else the source order).
    order: ViewMap,
    matcher: Arc<RegexMatcher>,
}

/// View rows to scan, at or after the scan's start (`after`) or before it.
#[derive(Debug, Clone)]
struct Segment {
    rows: Range<u64>,
    after: bool,
}

/// Where a scan is, and the model rows with a match it has found (in view order: those at or
/// after its start, then those before). Moved to the background for each slice and back.
#[derive(Debug, Default)]
pub(crate) struct ScanRun {
    segments: Vec<Segment>,
    at: usize,
    pos: u64,
    rows_after: Vec<u64>,
    rows_before: Vec<u64>,
}

impl ScanRun {
    fn new(segments: Vec<Segment>) -> Self {
        ScanRun {
            segments,
            ..ScanRun::default()
        }
    }

    /// The matching model rows in view order.
    fn rows(&self) -> Vec<u64> {
        let mut rows = Vec::with_capacity(self.rows_before.len() + self.rows_after.len());
        rows.extend_from_slice(&self.rows_before);
        rows.extend_from_slice(&self.rows_after);
        rows
    }
}

/// What one slice found.
#[derive(Debug, Default)]
struct SliceOut {
    after: Vec<Hit>,
    before: Vec<Hit>,
    count_after: u64,
    count_before: u64,
    incomplete: u64,
    done: bool,
}

/// The matching ranges of `text` (at most [`MAX_RANGES`]).
fn match_ranges(matcher: &RegexMatcher, text: &str) -> SmallVec<[Range<u32>; 2]> {
    let mut out = SmallVec::new();
    let mut at = 0;
    while out.len() < MAX_RANGES {
        let Some(r) = matcher.find(text, at) else {
            break;
        };
        out.push(r.start as u32..r.end as u32);
        at = r.end;
    }
    out
}

/// Scans `run` on from where it is, batch by batch, until `deadline` (none: to the end) or
/// until `cancel` is raised; keeps at most `room` hits. Pure: it runs on any thread.
fn scan_slice(
    input: &ScanInput,
    run: &mut ScanRun,
    room: usize,
    deadline: Option<Instant>,
    cancel: &AtomicBool,
) -> SliceOut {
    let engine = input.display.engine();
    let settings = input.display.settings();
    let facts: Vec<_> = input
        .cols
        .iter()
        .map(|c| input.facts.get(*c as usize).map(|f| f.view(engine)))
        .collect();
    let loaded = input.snapshot.row_count();
    let mut out = SliceOut::default();
    let mut kept = 0usize;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return out;
        }
        let Some(seg) = run.segments.get(run.at).cloned() else {
            out.done = true;
            return out;
        };
        let pos = run.pos.max(seg.rows.start);
        if pos >= seg.rows.end {
            run.at += 1;
            run.pos = 0;
            continue;
        }
        let end = pos.saturating_add(BATCH_ROWS).min(seg.rows.end);
        for v in pos..end {
            let Some(model) = input.order.view_to_model(ViewRow(v)) else {
                continue;
            };
            if model.0 >= loaded {
                continue;
            }
            let mut row_hit = false;
            for (ix, (col, f)) in input.cols.iter().zip(&facts).enumerate() {
                let Some(f) = f else { continue };
                let value = input.snapshot.cell(model.0, *col);
                let shown = format_cell(value, f, settings, Purpose::Cell);
                // NULL never matches; a decode error's text is chrome, not the value.
                if matches!(shown.kind, DisplayKind::Null | DisplayKind::Error) {
                    continue;
                }
                if CellFlags::from_bits_truncate(shown.flags).contains(CellFlags::TRUNCATED) {
                    out.incomplete += 1;
                }
                let ranges = match_ranges(&input.matcher, &shown.text);
                if ranges.is_empty() {
                    continue;
                }
                row_hit = true;
                if seg.after {
                    out.count_after += 1;
                } else {
                    out.count_before += 1;
                }
                if kept < room {
                    kept += 1;
                    let hit = Hit {
                        view: v,
                        row: model.0,
                        col_ix: ix as u32,
                        col: *col,
                        ranges,
                    };
                    if seg.after {
                        out.after.push(hit);
                    } else {
                        out.before.push(hit);
                    }
                }
            }
            if row_hit {
                if seg.after {
                    run.rows_after.push(model.0);
                } else {
                    run.rows_before.push(model.0);
                }
            }
        }
        run.pos = end;
        if deadline.is_some_and(|d| Instant::now() >= d) {
            return out;
        }
    }
}

/// A whole scan's outcome ([`scan_snapshot`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanSummary {
    /// Matching cells.
    pub total: u64,
    /// Cut values scanned.
    pub incomplete: u64,
    /// The rows with a match, in the order shown.
    pub rows: Vec<u64>,
}

/// Scans every column of `snapshot` shown with `display` for `query` (the job the background
/// executor runs, whole; the perf line).
pub fn scan_snapshot(
    snapshot: Arc<StoreSnapshot>,
    display: &ResultDisplay,
    query: &str,
    opts: FindOpts,
) -> ScanSummary {
    let facts = crate::results::adapter::col_facts(snapshot.schema(), display.engine());
    let cols: Arc<[u32]> = (0..facts.len() as u32).collect();
    let rows = snapshot.row_count();
    let Ok(Some(matcher)) = build_matcher(query, opts) else {
        return ScanSummary {
            total: 0,
            incomplete: 0,
            rows: Vec::new(),
        };
    };
    let input = ScanInput {
        snapshot,
        facts,
        display: Arc::new(display.clone()),
        cols,
        order: ViewMap::identity(rows),
        matcher: Arc::new(matcher),
    };
    let mut run = ScanRun::new(vec![Segment {
        rows: 0..rows,
        after: true,
    }]);
    let out = scan_slice(&input, &mut run, 0, None, &AtomicBool::new(false));
    ScanSummary {
        total: out.count_after + out.count_before,
        incomplete: out.incomplete,
        rows: run.rows(),
    }
}

/// The filter's view map over `rows` (in the order shown) of `loaded` fetched rows.
pub fn filtered_map(rows: &[u64], loaded: u64) -> ViewMap {
    ViewMap::filtered(Arc::from(rows), loaded)
}

// ---- the view map's owner (the sort's order, the filter over it) ------------------------------

/// A filter's rows and its maps.
#[derive(Clone)]
struct FilterRows {
    /// The kept model rows, ascending.
    sorted: Arc<[u64]>,
    /// The filter in the source order (Clear sort).
    plain: ViewMap,
    /// The filter in the order of base version `for_base`.
    shown: Option<(u64, ViewMap)>,
    /// The fetched rows it ran over.
    rows: u64,
    // ## m3c-t37 fix round 1 (m1)
    /// The grid generation it was built for: a filter of an older run is refused.
    generation: Option<u64>,
}

/// What a grid shows: a sort's order and a filter over it.
struct GridViews {
    /// The grid's generation these belong to (a re-run drops them).
    generation: Option<u64>,
    /// The sort's order (`None`: the source order), and its version.
    base: Option<ViewMap>,
    base_version: u64,
    filter: Option<FilterRows>,
    /// A composition running on the background executor.
    job: Option<Task<()>>,
}

#[derive(Default)]
struct Views(HashMap<EntityId, GridViews>);

impl Global for Views {}

fn views_entry<'a>(grid: &Entity<GridView>, cx: &'a mut App) -> &'a mut GridViews {
    let generation = grid.read(cx).generation();
    let views = cx.default_global::<Views>();
    let entry = views
        .0
        .entry(grid.entity_id())
        .or_insert_with(|| GridViews {
            generation,
            base: None,
            base_version: 0,
            filter: None,
            job: None,
        });
    if entry.generation != generation {
        entry.generation = generation;
        entry.base = None;
        entry.filter = None;
        entry.job = None;
        entry.base_version += 1;
    }
    entry
}

/// The local sort hands the grid its order (`None`: back to the source order; Task 36's
/// publish and Clear sort): the grid shows it, under the filter when one is on (composed on
/// the background executor).
pub(crate) fn show_base(grid: &Entity<GridView>, base: Option<ViewMap>, cx: &mut App) {
    let entry = views_entry(grid, cx);
    entry.base = base;
    entry.base_version += 1;
    refresh_view(grid, cx);
}

/// The order a scan reads: the sort's (following the loaded rows), else the source order.
fn base_of(grid: &Entity<GridView>, loaded: u64, cx: &App) -> (ViewMap, u64) {
    let generation = grid.read(cx).generation();
    match cx
        .try_global::<Views>()
        .and_then(|v| v.0.get(&grid.entity_id()))
        .filter(|e| e.generation == generation)
    {
        Some(e) => (
            e.base
                .as_ref()
                .map_or_else(|| ViewMap::identity(loaded), |b| b.with_loaded(loaded)),
            e.base_version,
        ),
        None => (ViewMap::identity(loaded), 0),
    }
}

/// Sets (or drops) `grid`'s filter.
fn set_filter(grid: &Entity<GridView>, filter: Option<FilterRows>, cx: &mut App) {
    // ## m3c-t37 fix round 1 (m1): a filter of another run never lands on these rows.
    if filter
        .as_ref()
        .is_some_and(|f| f.generation != grid.read(cx).generation())
    {
        return;
    }
    let entry = views_entry(grid, cx);
    entry.filter = filter;
    refresh_view(grid, cx);
}

/// Whether `grid` shows a filter, and its rows (shown, fetched).
fn filter_of(grid: &Entity<GridView>, cx: &App) -> Option<(u64, u64)> {
    let e = cx.try_global::<Views>()?.0.get(&grid.entity_id())?;
    let f = e.filter.as_ref()?;
    (e.generation == grid.read(cx).generation()).then_some((f.sorted.len() as u64, f.rows))
}

/// Shows what `grid`'s state says: the base, or the filter in the base's order (composed off
/// the UI thread when it is not built yet).
fn refresh_view(grid: &Entity<GridView>, cx: &mut App) {
    let key = grid.entity_id();
    let Some(entry) = cx.try_global::<Views>().and_then(|v| v.0.get(&key)) else {
        return;
    };
    let version = entry.base_version;
    let ready = match (&entry.filter, &entry.base) {
        (None, base) => Some(base.clone().unwrap_or_else(|| ViewMap::identity(0))),
        (Some(f), None) => Some(f.plain.clone()),
        (Some(f), Some(_)) => match &f.shown {
            Some((v, map)) if *v == version => Some(map.clone()),
            _ => None,
        },
    };
    let compose = (entry.filter.clone(), entry.base.clone());
    if let Some(map) = ready {
        if let Some(e) = cx.default_global::<Views>().0.get_mut(&key) {
            e.job = None;
        }
        grid.update(cx, |g, cx| g.set_view_map(map, cx));
        return;
    }
    // Compose the filter in the base's order on the background executor.
    let (Some(f), Some(base)) = compose else {
        return;
    };
    let job = cx.background_spawn(async move { filter_in_order(&f.sorted, f.rows, &base) });
    let weak = grid.downgrade();
    let task = cx.spawn(async move |cx| {
        let map = job.await;
        cx.update(|cx| {
            let Some(grid) = weak.upgrade() else { return };
            let Some(e) = cx.default_global::<Views>().0.get_mut(&key) else {
                return;
            };
            if e.base_version != version {
                return;
            }
            if let Some(f) = e.filter.as_mut() {
                f.shown = Some((version, map.clone()));
            }
            e.job = None;
            grid.update(cx, |g, cx| g.set_view_map(map, cx));
        });
    });
    if let Some(e) = cx.default_global::<Views>().0.get_mut(&key) {
        e.job = Some(task);
    }
}

// ## m3c-int (M2)
/// The filter's kept rows `sorted` (model rows, ascending; scanned over `fetched` rows) in
/// `base`'s order. A sort's base holds the rows of its snapshot: rows fetched while it ran join
/// its unsorted tail, so the kept ones among them stay shown (no gap until the rescan lands).
pub fn filter_in_order(sorted: &[u64], fetched: u64, base: &ViewMap) -> ViewMap {
    let base = base.with_loaded(fetched.max(base.len()));
    let mut rows = Vec::with_capacity(sorted.len());
    for v in 0..base.len() {
        if let Some(m) = base.view_to_model(ViewRow(v))
            && sorted.binary_search(&m.0).is_ok()
        {
            rows.push(m.0);
        }
    }
    ViewMap::filtered(Arc::from(rows), fetched)
}

/// `grid` is gone (its tab or console closed): its view state goes.
pub(crate) fn grid_closed(grid: EntityId, cx: &mut App) {
    if let Some(v) = cx.try_global::<Views>()
        && v.0.contains_key(&grid)
    {
        cx.default_global::<Views>().0.remove(&grid);
    }
}

// ---- the bar ------------------------------------------------------------------------------------

/// The bar's toggles, in their order inside the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Toggle {
    Case,
    Words,
    Regex,
    Filter,
    Tolerant,
}

const TOGGLES: [Toggle; 5] = [
    Toggle::Case,
    Toggle::Words,
    Toggle::Regex,
    Toggle::Filter,
    Toggle::Tolerant,
];

impl Toggle {
    fn icon(self) -> IconName {
        match self {
            Toggle::Case => IconName::CaseSensitive,
            Toggle::Words => IconName::WholeWord,
            Toggle::Regex => IconName::Regex,
            Toggle::Filter => IconName::Filter,
            Toggle::Tolerant => IconName::Languages,
        }
    }

    fn name_id(self) -> &'static str {
        match self {
            Toggle::Case => "hv-find-match-case",
            Toggle::Words => "hv-find-words",
            Toggle::Regex => "hv-find-regex",
            Toggle::Filter => "hv-gfind-filter",
            Toggle::Tolerant => "hv-find-tolerant",
        }
    }

    fn tip_id(self) -> Option<&'static str> {
        match self {
            Toggle::Filter => Some("hv-gfind-filter-tip"),
            Toggle::Tolerant => Some("hv-find-tolerant-tip"),
            _ => None,
        }
    }

    fn element_id(self) -> &'static str {
        match self {
            Toggle::Case => "grid-find-match-case",
            Toggle::Words => "grid-find-words",
            Toggle::Regex => "grid-find-regex",
            Toggle::Filter => "grid-find-filter",
            Toggle::Tolerant => "grid-find-tolerant",
        }
    }
}

/// The find's state as the tests and the count see it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FindView {
    /// Matching cells, once the scan is done (`None` while it runs).
    pub total: Option<u64>,
    /// Matching cells counted so far.
    pub counted: u64,
    /// The current match's place, 1-based, among the matches in the order shown.
    pub current: Option<u64>,
    /// Cut values scanned ("may be incomplete").
    pub incomplete: u64,
    /// The pattern does not compile.
    pub invalid: bool,
    /// The rows a filter shows.
    pub filtered: Option<u64>,
}

/// The grid a bar finds in.
struct Target {
    grid: WeakEntity<GridView>,
    grid_id: EntityId,
    adapter: Weak<GridAdapter>,
    /// The adapter generation it scanned.
    generation: u64,
}

/// The current scan and what it found.
#[derive(Default)]
struct Scan {
    id: u64,
    input: Option<ScanInput>,
    base_version: u64,
    task: Option<Task<()>>,
    cancel: Arc<AtomicBool>,
    /// The view rows it covers.
    covered: u64,
    after: Vec<Hit>,
    before: Vec<Hit>,
    count_after: u64,
    count_before: u64,
    incomplete: u64,
    done: bool,
    /// Back from the background once a run ends (an extension moves it out again).
    run: Option<ScanRun>,
    /// The rows with a match, in view order (built when a run ends).
    rows: Option<Arc<[u64]>>,
}

impl Scan {
    fn stop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.task = None;
    }

    fn kept(&self) -> usize {
        self.before.len() + self.after.len()
    }

    fn hit(&self, i: usize) -> Option<&Hit> {
        if i < self.before.len() {
            self.before.get(i)
        } else {
            self.after.get(i - self.before.len())
        }
    }

    /// The place of the hit at `key` among the kept hits.
    fn index_of(&self, key: (u64, u32)) -> Option<usize> {
        if let Ok(i) = self.before.binary_search_by_key(&key, Hit::key) {
            return Some(i);
        }
        self.after
            .binary_search_by_key(&key, Hit::key)
            .ok()
            .map(|i| self.before.len() + i)
    }
}

/// The marks the grid paints (shared with its highlights closure).
#[derive(Default)]
struct MarkTable {
    cells: HashMap<(u64, u32), SmallVec<[Range<u32>; 2]>>,
    /// The current match (model row, column).
    current: Option<(u64, u32)>,
}

/// One console's grid find bar.
pub struct GridFindBar {
    console: ConsoleId,
    input: Entity<TextInput>,
    toggles: [FocusHandle; 5],
    prev: FocusHandle,
    next: FocusHandle,
    all: FocusHandle,
    close: FocusHandle,
    opts: FindOpts,
    filter: bool,
    open: bool,
    /// The field takes the focus at the next frame (the grid's event carries no window).
    take_focus: bool,
    error: Option<SharedString>,
    target: Option<Target>,
    // ## m3c-t37 fix round 1 (I1, I2): the origin is a cell; its place is taken in the order
    // the scan reads (the source or sort order, never a filter's).
    /// Where find started.
    origin: Option<ModelCell>,
    /// The origin's (view row, column place) in the current scan's order.
    origin_key: (u64, u32),
    /// The cell find last selected (a focus elsewhere is a new origin).
    made: Option<ModelCell>,
    scan: Scan,
    /// The current match: (view row, column place).
    current: Option<(u64, u32)>,
    /// The first match at or after the origin becomes the focus when it lands.
    auto: bool,
    select_all_pending: bool,
    marks: Rc<RefCell<MarkTable>>,
    // ## m3c-t37 fix round 1
    /// A rescan of the same query (a new order or columns): the current match is found again
    /// by its cell, and nothing moves (I1, m6).
    relocate: Option<ModelCell>,
    /// The last settled count, shown (pending) until a new scan's first slice lands (I4).
    last: Option<FindView>,
    /// A new scan has not landed a slice yet: the last count and marks stay (I4).
    pending_first: bool,
    /// The filter is being built: a Select all waits for it (I3).
    filter_building: bool,
    /// The count's accessible name: changed only when a count settles (announced once).
    announced: SharedString,
    _subscriptions: Vec<Subscription>,
}

/// Every console's bar (made when find first opens).
#[derive(Default)]
struct Bars(HashMap<ConsoleId, Entity<GridFindBar>>);

impl Global for Bars {}

fn bar(console: ConsoleId, cx: &App) -> Option<Entity<GridFindBar>> {
    cx.try_global::<Bars>()?.0.get(&console).cloned()
}

/// Opens the find bar over console `console`'s active result (the tile calls it on
/// `GridEvent::FindRequested`); again: the field takes the focus back with its query selected.
pub fn open(console: ConsoleId, cx: &mut App) {
    let bar = match bar(console, cx) {
        Some(bar) => bar,
        None => {
            let bar = cx.new(|cx| GridFindBar::new(console, cx));
            cx.default_global::<Bars>().0.insert(console, bar.clone());
            bar
        }
    };
    bar.update(cx, |b, cx| b.open(cx));
    if let Some((grid, adapter)) = active_target(console, cx) {
        bar.update(cx, |b, cx| b.follow(&grid, &adapter, cx));
    }
    notify_hosts(cx);
}

/// The grid and the adapter of console `console`'s active result tab, once the tile built
/// its grid.
pub(crate) fn active_target(
    console: ConsoleId,
    cx: &App,
) -> Option<(Entity<GridView>, Rc<GridAdapter>)> {
    let results = crate::console::Consoles::get(cx, console)?
        .results
        .clone()?;
    let r = results.read(cx);
    let super::ActiveResult::Tab(ix) = r.active() else {
        return None;
    };
    let adapter = r.tab(ix)?.adapter().clone();
    let grid = super::view::hosts(cx)
        .into_iter()
        .find_map(|h| h.read(cx).grid_of(console, ix, cx))?;
    Some((grid, adapter))
}

/// Forgets console `console`'s bar (its console closed).
pub fn console_closed(console: ConsoleId, cx: &mut App) {
    if cx.has_global::<Bars>() {
        cx.global_mut::<Bars>().0.remove(&console);
    }
}

/// Whether console `console`'s bar is open.
pub fn is_open(console: ConsoleId, cx: &App) -> bool {
    bar(console, cx).is_some_and(|b| b.read(cx).open)
}

/// The bar's query.
pub fn query(console: ConsoleId, cx: &App) -> Option<String> {
    Some(bar(console, cx)?.read(cx).input.read(cx).value().to_owned())
}

/// Types `query` into console `console`'s field, as the input would on an edit.
pub fn set_query(console: ConsoleId, query: &str, cx: &mut App) {
    let Some(bar) = bar(console, cx) else { return };
    let query = query.to_owned();
    bar.update(cx, |b, cx| {
        b.input.update(cx, |i, cx| {
            i.set_value(query);
            cx.notify();
        });
        b.apply(cx);
    });
}

/// Flips toggle `t` as a click would.
pub fn flip(console: ConsoleId, t: Toggle, cx: &mut App) {
    if let Some(bar) = bar(console, cx) {
        bar.update(cx, |b, cx| b.flip(t, cx));
    }
}

/// The bar's options and whether Filter rows is on.
pub fn options(console: ConsoleId, cx: &App) -> Option<(FindOpts, bool)> {
    let b = bar(console, cx)?;
    let b = b.read(cx);
    Some((b.opts, b.filter))
}

/// The find's state.
pub fn status(console: ConsoleId, cx: &App) -> Option<FindView> {
    Some(bar(console, cx)?.read(cx).view(cx))
}

/// The count as the bar shows it ("3 of 41", "3 of 41 · may be incomplete").
pub fn count_text(console: ConsoleId, cx: &App) -> Option<SharedString> {
    let b = bar(console, cx)?;
    let b = b.read(cx);
    let (text, ..) = b.count(cx)?;
    Some(text)
}

/// "may be incomplete" after the count, while the scan met values cut at the value-size limit.
pub fn incomplete_text(console: ConsoleId, cx: &App) -> Option<SharedString> {
    let b = bar(console, cx)?;
    let b = b.read(cx);
    b.incomplete(cx).map(|(text, _)| text)
}

/// Whether the step buttons and Select all are enabled (as drawn).
pub fn can_step(console: ConsoleId, cx: &App) -> bool {
    bar(console, cx).is_some_and(|b| b.read(cx).searchable(cx))
}

/// The kept matching cells, in the order shown.
pub fn hits(console: ConsoleId, cx: &App) -> Vec<ModelCell> {
    let Some(b) = bar(console, cx) else {
        return Vec::new();
    };
    let s = &b.read(cx).scan;
    s.before.iter().chain(&s.after).map(Hit::cell).collect()
}

/// Steps to the next (or, `back`, the previous) match.
pub fn step(console: ConsoleId, back: bool, cx: &mut App) {
    if let Some(bar) = bar(console, cx) {
        bar.update(cx, |b, cx| b.step(back, cx));
    }
}

/// Selects every match (at most [`SELECT_ALL_CAP`]).
pub fn select_all(console: ConsoleId, cx: &mut App) {
    if let Some(bar) = bar(console, cx) {
        bar.update(cx, |b, cx| b.select_all(cx));
    }
}

/// Closes the bar (no focus move: the tests' and the commands' close).
pub fn close(console: ConsoleId, cx: &mut App) {
    if let Some(bar) = bar(console, cx) {
        bar.update(cx, |b, cx| b.close(None, cx));
    }
}

/// The console whose grid find field has the keyboard focus.
pub fn focused_bar(cx: &App) -> Option<ConsoleId> {
    let focused = cx.try_global::<FocusedInput>()?.entity?;
    let bars = cx.try_global::<Bars>()?;
    bars.0
        .iter()
        .find(|(_, b)| b.read(cx).input.entity_id() == focused)
        .map(|(c, _)| *c)
}

/// Whether a grid find field has the keyboard focus: the grid commands other than
/// [`ROUTED`] are disabled there, so their keys stay the field's.
pub fn field_focused(cx: &App) -> bool {
    focused_bar(cx).is_some()
}

/// The grid commands that act on the bar while its field has the focus.
pub const ROUTED: [&str; 2] = ["grid.find", "grid.select_all_occurrences"];

/// Runs grid command `id` on the focused find bar; `false` when no bar has the focus or `id`
/// is not one of [`ROUTED`].
pub fn run_in_bar(id: &str, cx: &mut App) -> bool {
    let Some(console) = focused_bar(cx).filter(|_| ROUTED.contains(&id)) else {
        return false;
    };
    let Some(bar) = bar(console, cx) else {
        return false;
    };
    bar.update(cx, |b, cx| match id {
        "grid.find" => b.input.update(cx, |i, cx| {
            i.select_all();
            cx.notify();
        }),
        _ => b.select_all(cx),
    });
    true
}

/// The tile shows `grid` over `adapter` for console `console`: an open bar follows it (another
/// tab, a re-run, rows fetched since its scan); a re-run drops the grid's view state.
pub(crate) fn sync(
    console: ConsoleId,
    grid: &Entity<GridView>,
    adapter: &Rc<GridAdapter>,
    cx: &mut App,
) {
    let stale = cx
        .try_global::<Views>()
        .and_then(|v| v.0.get(&grid.entity_id()))
        .is_some_and(|e| e.generation != grid.read(cx).generation());
    if stale {
        cx.default_global::<Views>().0.remove(&grid.entity_id());
    }
    let Some(bar) = bar(console, cx).filter(|b| b.read(cx).open) else {
        return;
    };
    // ## m3c-int (I3): find and Filter rows are the table's: the bar closes while the tab shows
    // another view, also on a tab switched to that already showed it (it never scans, marks or
    // filters the hidden table grid; the bar's own grid loses its marks and filter).
    if !super::views::shows_table(grid, cx) {
        cx.defer(move |cx| bar.update(cx, |b, cx| b.close(None, cx)));
        return;
    }
    let (grid, adapter) = (grid.clone(), adapter.clone());
    // After this frame: the bar starts work and repaints the grid.
    cx.defer(move |cx| {
        bar.update(cx, |b, cx| b.follow(&grid, &adapter, cx));
    });
}

/// The bar's toolbar for console `console` over its active result: the bar in place of the left
/// group, `rows` (rows · timing) on the right, the first to give way. `None` while closed.
// ## m3c-t38 (named exception): `trailing` (the view switch and the record toggle) follows
// rows · timing in the right group, which stays while the bar is open (NOTES surface 19).
pub(crate) fn toolbar(
    console: ConsoleId,
    rows: SharedString,
    trailing: Vec<ToolbarItem>,
    cx: &App,
) -> Option<AnyElement> {
    let bar = bar(console, cx).filter(|b| b.read(cx).open)?;
    let label: TextFn = {
        let rows = rows.clone();
        Rc::new(move |_| rows.clone())
    };
    let el = bar.clone();
    let item = ToolbarItem::pinned(
        Rc::new(move |_, _| el.clone().into_any_element()),
        Rc::new(|cx: &App| message(cx, "hv-gfind-placeholder", vec![])),
    );
    let rows_item = ToolbarItem::new(
        0,
        Rc::new(move |_, cx| {
            let t = Theme::get(cx).tokens;
            Label::new(
                rows.clone(),
                TextSource::Fluent,
                t.type_ramp.mono_kbd,
                t.colors.text_2.to_hsla(),
            )
            .id("results-rows-timing")
            .into_any_element()
        }),
        label,
    );
    let bar = Toolbar::new("results-toolbar")
        .item(item)
        .trailing(rows_item);
    Some(
        trailing
            .into_iter()
            .fold(bar, |bar, item| bar.trailing(item))
            .into_any_element(),
    )
}

/// The filter's note under the toolbar (NOTES surface 19, M3c-26): "Showing the 41 rows that
/// match, of the 10,000 fetched. …" with Show all rows.
pub(crate) fn note_row(
    console: ConsoleId,
    grid: &Entity<GridView>,
    cx: &App,
) -> Option<AnyElement> {
    let (shown, rows) = filter_of(grid, cx)?;
    if !matches!(grid.read(cx).view_map(), ViewMap::Filtered { .. }) {
        return None;
    }
    let t = Theme::get(cx).tokens;
    let c = t.colors;
    let text = message(
        cx,
        "hv-gfind-filter-note",
        vec![
            ("shown", ArgValue::Count(shown)),
            ("rows", ArgValue::Count(rows)),
        ],
    );
    let show_all = Button::new(
        "results-filter-show-all",
        message(cx, "hv-gfind-show-all", vec![]),
    )
    .kind(ButtonKind::Default)
    .on_raised(true)
    .compact(true)
    .on_click(move |_, _, cx| {
        if let Some(bar) = bar(console, cx) {
            bar.update(cx, |b, cx| {
                if b.filter {
                    b.flip(Toggle::Filter, cx);
                }
            });
        }
    });
    Some(
        div()
            .debug_selector(|| "results-filter-note-row".to_string())
            .flex()
            .flex_row()
            .items_start()
            .gap(px(8.0))
            .w_full()
            .flex_none()
            .px(px(12.0))
            .py(px(5.0))
            .bg(c.raised.to_hsla())
            .border_t_1()
            .border_color(c.seam.to_hsla())
            .child(
                div().pt(px(2.0)).child(
                    Icon::new(IconName::Filter)
                        .size(IconSize::Chrome)
                        .color(c.text_2.to_hsla()),
                ),
            )
            .child(
                div().flex_1().min_w(px(0.0)).pt(px(2.0)).child(
                    paragraph(
                        text,
                        TextSource::Fluent,
                        t.type_ramp.caption,
                        c.text_2.to_hsla(),
                    )
                    .id("results-filter-note"),
                ),
            )
            .child(div().flex().flex_none().child(show_all))
            .into_any_element(),
    )
}

/// Repaints every results tile (the bar opened or closed in its toolbar).
fn notify_hosts(cx: &mut App) {
    for host in super::view::hosts(cx) {
        host.update(cx, |_, cx| cx.notify());
    }
}

impl GridFindBar {
    fn new(console: ConsoleId, cx: &mut Context<Self>) -> Self {
        let placeholder: TextFn = Rc::new(|cx: &App| message(cx, "hv-gfind-placeholder", vec![]));
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx, placeholder);
            input.set_mono(true);
            input.set_frameless(true);
            input
        });
        // The field's keys reach `grid.find` and `grid.select_all_occurrences` (routed here,
        // `run_in_bar`); the other grid commands are disabled while it has the focus.
        let field = input.focus_handle(cx);
        crate::shell::register_focus_context(cx, field, GRID_CONTEXT);
        let subscriptions = vec![cx.subscribe(&input, Self::on_input_event)];
        let handle = |cx: &mut Context<Self>| cx.focus_handle().tab_stop(true);
        GridFindBar {
            console,
            input,
            toggles: [handle(cx), handle(cx), handle(cx), handle(cx), handle(cx)],
            prev: handle(cx),
            next: handle(cx),
            all: handle(cx),
            close: handle(cx),
            opts: FindOpts {
                tolerant: tolerant_default(cx),
                ..FindOpts::default()
            },
            filter: false,
            open: false,
            take_focus: false,
            error: None,
            target: None,
            origin: None,
            origin_key: (0, 0),
            made: None,
            scan: Scan::default(),
            current: None,
            auto: false,
            select_all_pending: false,
            marks: Rc::default(),
            relocate: None,
            last: None,
            pending_first: false,
            filter_building: false,
            announced: SharedString::default(),
            _subscriptions: subscriptions,
        }
    }

    fn on_input_event(&mut self, _: Entity<TextInput>, event: &InputEvent, cx: &mut Context<Self>) {
        match event {
            InputEvent::Changed => self.apply(cx),
            // Escape in the field: close, and the focus goes back to the grid (next frame).
            InputEvent::Cancelled => self.close_and_refocus(cx),
            InputEvent::Submitted | InputEvent::CompositionChanged(_) => {}
        }
    }

    /// The grid this bar finds in, and its adapter.
    fn target(&self) -> Option<(Entity<GridView>, Rc<GridAdapter>)> {
        let t = self.target.as_ref()?;
        Some((t.grid.upgrade()?, t.adapter.upgrade()?))
    }

    fn open(&mut self, cx: &mut Context<Self>) {
        if !self.open {
            self.open = true;
            self.opts.tolerant = tolerant_default(cx);
            if self.input.read(cx).value().is_empty()
                && let Some(seed) = find_pasteboard(cx)
            {
                let seed = if self.opts.regex {
                    regex::escape(&seed)
                } else {
                    seed
                };
                self.input.update(cx, |i, cx| {
                    i.set_value(seed);
                    cx.notify();
                });
            }
        }
        self.input.update(cx, |i, cx| {
            i.select_all();
            cx.notify();
        });
        // The tile's next frame targets the active grid (`sync` → `follow`).
        self.take_focus = true;
        cx.notify();
    }

    /// The tile shows `grid` over `adapter`: a new target starts anew; a re-run scans again;
    /// rows fetched since the scan are scanned.
    fn follow(
        &mut self,
        grid: &Entity<GridView>,
        adapter: &Rc<GridAdapter>,
        cx: &mut Context<Self>,
    ) {
        if !self.open {
            return;
        }
        let same = self
            .target
            .as_ref()
            .is_some_and(|t| t.grid_id == grid.entity_id());
        let generation = adapter.generation();
        if !same {
            self.leave_target(cx);
            self.target = Some(Target {
                grid: grid.downgrade(),
                grid_id: grid.entity_id(),
                adapter: Rc::downgrade(adapter),
                generation,
            });
            self.take_origin(grid, cx);
            let marks = self.marks.clone();
            let highlights: Highlights = Rc::new(move |row: ModelRow, col| {
                let t = marks.borrow();
                let ranges = t.cells.get(&(row.0, col.0))?;
                Some(CellMarks {
                    ranges: ranges.clone(),
                    current: (t.current == Some((row.0, col.0))).then_some(0),
                })
            });
            grid.update(cx, |g, cx| g.set_highlights(Some(highlights), cx));
            self.apply(cx);
            return;
        }
        if self
            .target
            .as_ref()
            .is_some_and(|t| t.generation != generation)
        {
            // ## m3c-t37 fix round 1 (m1): a re-run drops the old scan and its marks at once.
            if self.scan.input.is_some() || !self.marks.borrow().cells.is_empty() {
                self.reset_scan(true);
                self.current = None;
                grid.update(cx, |_, cx| cx.notify());
            }
            // Once the grid shows the new rows (and their columns), scan them.
            if grid.read(cx).generation() != Some(generation) {
                return;
            }
            if let Some(t) = self.target.as_mut() {
                t.generation = generation;
            }
            self.origin = None;
            self.made = None;
            self.last = None;
            self.apply(cx);
            return;
        }
        if grid.read(cx).generation() != Some(generation) || self.scan.input.is_none() {
            return;
        }
        let loaded = adapter.snapshot().row_count();
        let (base, version) = base_of(grid, loaded, cx);
        // ## m3c-t37 fix round 1 (I1, m6): a new order (a sort, Clear sort) or other columns
        // shown: scan again in what the grid shows, keeping the current match.
        let cols_changed = self.scan.input.as_ref().is_some_and(|i| {
            let shown = grid.read(cx).column_layout().visible();
            i.cols.len() != shown.len() || i.cols.iter().zip(shown).any(|(a, b)| *a != b.0)
        });
        if version != self.scan.base_version || cols_changed {
            self.rescan(cx);
            return;
        }
        // Rows fetched since the scan: scan them too (they come after it in the view).
        if self.scan.done && base.len() > self.scan.covered {
            self.extend(adapter, base, cx);
        }
    }

    /// Drops the scan (a new id); `marks`: its marks too (else they stay until the new scan's
    /// first slice, I4).
    fn reset_scan(&mut self, marks: bool) {
        self.scan.stop();
        let id = self.scan.id + 1;
        self.scan = Scan {
            id,
            ..Scan::default()
        };
        self.filter_building = false;
        if marks {
            let mut m = self.marks.borrow_mut();
            m.cells.clear();
            m.current = None;
            self.pending_first = false;
        }
    }

    /// The same query again over a new order or other columns: nothing moves, the current
    /// match is found again by its cell.
    fn rescan(&mut self, cx: &mut Context<Self>) {
        let keep = self
            .current
            .and_then(|k| self.scan.index_of(k))
            .and_then(|i| self.scan.hit(i))
            .map(Hit::cell);
        let pending = self.select_all_pending;
        self.apply(cx);
        self.auto = false;
        self.relocate = keep;
        self.select_all_pending = pending;
    }

    /// The previous target loses its marks and its filter.
    fn leave_target(&mut self, cx: &mut Context<Self>) {
        // ## m3c-t37 fix round 1: nothing of this grid's find carries over.
        self.reset_scan(true);
        self.last = None;
        if let Some((grid, _)) = self.target() {
            grid.update(cx, |g, cx| g.set_highlights(None, cx));
            set_filter(&grid, None, cx);
        }
        self.target = None;
    }

    /// Find starts from the grid's focus cell (its place is taken when a scan starts, in the
    /// order the scan reads).
    fn take_origin(&mut self, grid: &Entity<GridView>, cx: &App) {
        self.origin = grid.read(cx).selection().focus();
        self.made = None;
    }

    /// The query or an option changed: a new matcher and a new scan from the origin.
    fn apply(&mut self, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        // ## m3c-t37 fix round 1 (I4): the last settled count (and the marks) stay, pending,
        // until the new scan's first slice lands.
        if !self.pending_first && self.scan.input.is_some() {
            self.last = Some(self.view(cx));
        }
        self.reset_scan(false);
        self.current = None;
        self.relocate = None;
        self.select_all_pending = false;
        let query = self.input.read(cx).value().to_owned();
        let built = build_matcher(&query, self.opts);
        self.error = built.as_ref().err().map(|e| SharedString::from(e.clone()));
        let Some((grid, adapter)) = self.target() else {
            cx.notify();
            return;
        };
        // A focus the user moved since find last selected is the new origin (T33 M14).
        let focus = grid.read(cx).selection().focus();
        if focus.is_some() && focus != self.made {
            self.take_origin(&grid, cx);
        }
        let matcher = match built {
            Ok(Some(m)) => Arc::new(m),
            Ok(None) | Err(_) => {
                // Nothing to find: no marks, no filter.
                self.reset_scan(true);
                self.last = None;
                set_filter(&grid, None, cx);
                grid.update(cx, |_, cx| cx.notify());
                cx.notify();
                return;
            }
        };
        let loaded = adapter.snapshot().row_count();
        let (base, version) = base_of(&grid, loaded, cx);
        let raw = adapter.raw_view();
        let cols: Arc<[u32]> = grid
            .read(cx)
            .column_layout()
            .visible()
            .iter()
            .map(|c| c.0)
            .collect();
        let n = base.len();
        // ## m3c-t37 fix round 1 (I2): the origin's place in the order scanned.
        self.origin_key = self
            .origin
            .map(|o| {
                let view = base.model_to_view(o.row).map_or(0, |v| v.0);
                let col = cols.iter().position(|c| *c == o.col.0).unwrap_or(0) as u32;
                (view, col)
            })
            .unwrap_or((0, 0));
        let start = self.origin_key.0.min(n);
        let input = ScanInput {
            snapshot: raw.snapshot,
            facts: raw.facts,
            display: raw.display,
            cols,
            order: base,
            matcher,
        };
        self.scan.covered = n;
        self.scan.base_version = version;
        self.auto = true;
        self.pending_first = true;
        let run = ScanRun::new(vec![
            Segment {
                rows: start..n,
                after: true,
            },
            Segment {
                rows: 0..start,
                after: false,
            },
        ]);
        self.run_scan(input, run, cx);
        grid.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    /// Scans the view rows past what the last scan covered (rows fetched since).
    fn extend(&mut self, adapter: &Rc<GridAdapter>, base: ViewMap, cx: &mut Context<Self>) {
        let Some(mut input) = self.scan.input.clone() else {
            return;
        };
        let Some(mut run) = self.scan.run.take() else {
            return;
        };
        let from = self.scan.covered;
        let n = base.len();
        input.snapshot = adapter.snapshot();
        input.order = base;
        run.segments.push(Segment {
            rows: from..n,
            after: true,
        });
        self.scan.covered = n;
        self.scan.done = false;
        self.run_scan(input, run, cx);
    }

    /// Runs `run` over `input` slice by slice on the background executor.
    fn run_scan(&mut self, input: ScanInput, run: ScanRun, cx: &mut Context<Self>) {
        let id = self.scan.id;
        let cancel = Arc::new(AtomicBool::new(false));
        self.scan.cancel = cancel.clone();
        self.scan.input = Some(input.clone());
        let task = cx.spawn(async move |this, cx| {
            let mut run = run;
            loop {
                let Ok(room) = this.read_with(cx, |b, _| MAX_KEPT.saturating_sub(b.scan.kept()))
                else {
                    return;
                };
                let (inp, flag) = (input.clone(), cancel.clone());
                let (back, out) = cx
                    .background_spawn(async move {
                        let mut run = run;
                        let out =
                            scan_slice(&inp, &mut run, room, Some(Instant::now() + SLICE), &flag);
                        (run, out)
                    })
                    .await;
                run = back;
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let done = out.done;
                if this.update(cx, |b, cx| b.on_slice(id, out, cx)).is_err() {
                    return;
                }
                if done {
                    // The matching rows in view order, built off the UI thread too.
                    let (run, rows) = cx
                        .background_spawn(async move {
                            let rows: Arc<[u64]> = Arc::from(run.rows());
                            (run, rows)
                        })
                        .await;
                    let _ = this.update(cx, |b, cx| b.on_done(id, run, rows, cx));
                    return;
                }
            }
        });
        self.scan.task = Some(task);
    }

    /// A slice landed: its hits join the kept ones and the marks; the first match at or after
    /// the origin becomes the focus.
    fn on_slice(&mut self, id: u64, out: SliceOut, cx: &mut Context<Self>) {
        if id != self.scan.id {
            return;
        }
        {
            let mut m = self.marks.borrow_mut();
            // ## m3c-t37 fix round 1 (I4): the old marks go with the new scan's first slice.
            if std::mem::take(&mut self.pending_first) {
                m.cells.clear();
                m.current = None;
            }
            for h in out.after.iter().chain(&out.before) {
                m.cells.insert((h.row, h.col), h.ranges.clone());
            }
        }
        self.scan.after.extend(out.after);
        self.scan.before.extend(out.before);
        self.scan.count_after += out.count_after;
        self.scan.count_before += out.count_before;
        self.scan.incomplete += out.incomplete;
        if self.auto && self.current.is_none() {
            let origin = self.origin_key;
            let at = self.scan.after.partition_point(|h| h.key() < origin);
            if at < self.scan.after.len() {
                self.select_index(self.scan.before.len() + at, false, cx);
            }
        }
        self.find_relocated();
        if let Some((grid, _)) = self.target() {
            grid.update(cx, |_, cx| cx.notify());
        }
        notify_hosts(cx);
        cx.notify();
    }

    /// The scan ended: the count is final; the filter, a waiting Select all and a first match
    /// past the end (wrapping) follow.
    fn on_done(&mut self, id: u64, run: ScanRun, rows: Arc<[u64]>, cx: &mut Context<Self>) {
        if id != self.scan.id {
            return;
        }
        self.scan.done = true;
        self.scan.task = None;
        self.scan.run = Some(run);
        self.scan.rows = Some(rows);
        // A scan that found nothing: the old marks go now (I4).
        if std::mem::take(&mut self.pending_first) {
            let mut m = self.marks.borrow_mut();
            m.cells.clear();
            m.current = None;
        }
        self.last = None;
        self.find_relocated();
        self.relocate = None;
        if self.auto && self.current.is_none() && self.scan.kept() > 0 {
            self.select_index(0, false, cx);
        }
        self.auto = false;
        if self.filter {
            self.build_filter(cx);
        }
        // ## m3c-t37 fix round 1 (I3): under Filter rows a waiting Select all runs once the
        // new filter is shown (`build_filter`), else its ranges would be dropped with the map.
        if self.select_all_pending && !self.filter_building {
            self.select_all_pending = false;
            self.select_all(cx);
        }
        if let Some((grid, _)) = self.target() {
            grid.update(cx, |_, cx| cx.notify());
        }
        notify_hosts(cx);
        cx.notify();
    }

    /// Builds the filter from the scan's rows on the background executor, then shows it.
    fn build_filter(&mut self, cx: &mut Context<Self>) {
        let (Some((grid, adapter)), Some(rows)) = (self.target(), self.scan.rows.clone()) else {
            return;
        };
        let loaded = adapter.snapshot().row_count();
        let version = self.scan.base_version;
        let id = self.scan.id;
        let generation = grid.read(cx).generation();
        self.filter_building = true;
        let job = cx.background_spawn(async move {
            let shown = ViewMap::filtered(rows.clone(), loaded);
            let mut sorted = rows.to_vec();
            sorted.sort_unstable();
            let sorted: Arc<[u64]> = Arc::from(sorted);
            let plain = ViewMap::filtered(sorted.clone(), loaded);
            FilterRows {
                sorted,
                plain,
                shown: Some((version, shown)),
                rows: loaded,
                generation,
            }
        });
        let weak = grid.downgrade();
        cx.spawn(async move |this, cx| {
            let filter = job.await;
            let _ = this.update(cx, |b, cx| {
                if b.scan.id != id || !b.filter || !b.open {
                    return;
                }
                b.filter_building = false;
                let Some(grid) = weak.upgrade() else { return };
                set_filter(&grid, Some(filter), cx);
                // A focus the filter hides moves to the current match (logged ruling).
                let hidden = grid
                    .read(cx)
                    .selection()
                    .focus()
                    .is_some_and(|f| !grid.read(cx).view_map().shows(f.row));
                if hidden && let Some(h) = b.current.and_then(|k| b.scan.index_of(k)) {
                    b.select_index(h, false, cx);
                }
                // ## m3c-t37 fix round 1 (I3)
                if std::mem::take(&mut b.select_all_pending) {
                    b.select_all(cx);
                }
                notify_hosts(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Makes kept hit `i` the current match and the grid's focus (one selection change);
    /// `commit`: a step the user asked for (the find pasteboard hears the query).
    fn select_index(&mut self, i: usize, commit: bool, cx: &mut Context<Self>) {
        let Some(hit) = self.scan.hit(i).cloned() else {
            return;
        };
        self.current = Some(hit.key());
        self.auto = false;
        self.marks.borrow_mut().current = Some((hit.row, hit.col));
        self.made = Some(hit.cell());
        if let Some((grid, _)) = self.target() {
            grid.update(cx, |g, cx| g.focus_cell(hit.cell(), cx));
        }
        if commit {
            self.commit_query(cx);
        }
        cx.notify();
    }

    fn step(&mut self, back: bool, cx: &mut Context<Self>) {
        let n = self.scan.kept();
        if n == 0 {
            return;
        }
        let at = self.current.and_then(|k| self.scan.index_of(k));
        let next = match (at, back) {
            (Some(i), false) => (i + 1) % n,
            (Some(i), true) => (i + n - 1) % n,
            (None, false) => {
                let origin = self.origin_key;
                (0..n)
                    .find(|i| self.scan.hit(*i).is_some_and(|h| h.key() >= origin))
                    .unwrap_or(0)
            }
            (None, true) => {
                let origin = self.origin_key;
                (0..n)
                    .rev()
                    .find(|i| self.scan.hit(*i).is_some_and(|h| h.key() < origin))
                    .unwrap_or(n - 1)
            }
        };
        self.select_index(next, true, cx);
    }

    /// Selects every matching cell as its own range (at most [`SELECT_ALL_CAP`]); while the
    /// scan runs it waits for the count.
    fn select_all(&mut self, cx: &mut Context<Self>) {
        if self.error.is_some() || self.scan.input.is_none() {
            return;
        }
        if !self.scan.done || self.filter_building {
            self.select_all_pending = true;
            return;
        }
        let total = self.scan.count_after + self.scan.count_before;
        let cap = SELECT_ALL_CAP as usize;
        let cells: Vec<ModelCell> = (0..self.scan.kept().min(cap))
            .filter_map(|i| self.scan.hit(i).map(Hit::cell))
            .collect();
        if cells.is_empty() {
            return;
        }
        let focus = self
            .current
            .and_then(|k| self.scan.index_of(k))
            .filter(|i| *i < cells.len())
            .unwrap_or(0);
        if let Some((grid, _)) = self.target() {
            grid.update(cx, |g, cx| g.select_cells(&cells, focus, cx));
        }
        self.commit_query(cx);
        if total > cells.len() as u64 {
            let max = cells.len() as u64;
            let text: TextFn = Rc::new(move |cx: &App| {
                message(
                    cx,
                    "hv-find-capped",
                    vec![("max", ArgValue::Count(max)), ("n", ArgValue::Count(total))],
                )
            });
            ToastQueue::push(cx, Toast::new(ToastKind::Info, text));
        }
        cx.notify();
    }

    fn flip(&mut self, t: Toggle, cx: &mut Context<Self>) {
        match t {
            Toggle::Case => self.opts.match_case = !self.opts.match_case,
            Toggle::Words => self.opts.words = !self.opts.words,
            Toggle::Regex => self.opts.regex = !self.opts.regex,
            Toggle::Tolerant => {
                self.opts.tolerant = !self.opts.tolerant;
                let on = self.opts.tolerant;
                crate::shell::update_settings(cx, |s| {
                    s.editor.find_turkish_tolerant = if on {
                        FindTolerancePref::On
                    } else {
                        FindTolerancePref::Off
                    };
                });
            }
            Toggle::Filter => {
                self.filter = !self.filter;
                if self.filter {
                    if self.scan.done {
                        self.build_filter(cx);
                    }
                } else if let Some((grid, _)) = self.target() {
                    set_filter(&grid, None, cx);
                }
                notify_hosts(cx);
                cx.notify();
                return;
            }
        }
        self.apply(cx);
    }

    /// Closes the bar: the scan stops, the marks and the filter go; `window`: the focus goes
    /// back to the grid when the bar had it.
    fn close(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        let had_focus = window.as_ref().is_some_and(|w| self.has_focus(w, cx));
        self.open = false;
        self.commit_query(cx);
        let grid = self.target().map(|(g, _)| g);
        self.leave_target(cx);
        self.scan = Scan {
            id: self.scan.id + 1,
            ..Scan::default()
        };
        self.current = None;
        self.error = None;
        if let (Some(window), Some(grid), true) = (window, grid, had_focus) {
            let handle = grid.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        }
        notify_hosts(cx);
        cx.notify();
    }

    /// Escape from the field (its event carries no window): close, and the grid takes the
    /// focus.
    fn close_and_refocus(&mut self, cx: &mut Context<Self>) {
        let grid = self.target().map(|(g, _)| g);
        self.close(None, cx);
        if let Some(grid) = grid {
            let handle = grid.read(cx).focus_handle(cx);
            cx.defer(move |cx| {
                if let Some(window) = cx.active_window() {
                    let _ = window.update(cx, |_, window, cx| window.focus(&handle, cx));
                }
            });
        }
    }

    fn has_focus(&self, window: &Window, cx: &App) -> bool {
        self.input.focus_handle(cx).is_focused(window)
            || self
                .toggles
                .iter()
                .chain([&self.prev, &self.next, &self.all, &self.close])
                .any(|h| h.is_focused(window))
    }

    /// A committed search (a step, Select all, close) writes its query to the find pasteboard.
    fn commit_query(&self, cx: &App) {
        let query = self.input.read(cx).value();
        if !query.is_empty() && self.error.is_none() {
            write_find_pasteboard(query, cx);
        }
    }

    /// The state the bar draws: while a new scan has not landed a slice, the last settled one
    /// (pending), so the count, the buttons and the marks do not flash (I4).
    fn shown_view(&self, cx: &App) -> (FindView, bool) {
        match self.last {
            Some(last) if self.pending_first && self.error.is_none() => (last, true),
            _ => (self.view(cx), false),
        }
    }

    /// Whether the step buttons and Select all are enabled.
    fn searchable(&self, cx: &App) -> bool {
        let (v, _) = self.shown_view(cx);
        self.error.is_none()
            && !self.input.read(cx).value().is_empty()
            && v.total.is_none_or(|n| n > 0)
            && v.counted > 0
    }

    /// The find's state.
    fn view(&self, cx: &App) -> FindView {
        let counted = self.scan.count_after + self.scan.count_before;
        let current = self.current.and_then(|k| {
            if let Ok(i) = self.scan.before.binary_search_by_key(&k, Hit::key) {
                return Some(i as u64 + 1);
            }
            self.scan
                .after
                .binary_search_by_key(&k, Hit::key)
                .ok()
                .map(|i| self.scan.count_before + i as u64 + 1)
        });
        let filtered = self.target().and_then(|(g, _)| {
            matches!(g.read(cx).view_map(), ViewMap::Filtered { .. })
                .then(|| filter_of(&g, cx).map(|f| f.0))
                .flatten()
        });
        FindView {
            total: (self.scan.done && self.scan.input.is_some()).then_some(counted),
            counted,
            current,
            incomplete: self.scan.incomplete,
            invalid: self.error.is_some(),
            filtered,
        }
    }

    /// What the count shows: (text, tooltip, counting, invalid).
    fn count(&self, cx: &App) -> Option<(SharedString, Option<SharedString>, bool, bool)> {
        let (v, pending) = self.shown_view(cx);
        let empty = self.input.read(cx).value().is_empty();
        let (text, tip, counting, invalid) = count_parts(
            self.error.as_ref(),
            empty,
            v.total,
            v.counted,
            v.current,
            cx,
        )?;
        if pending {
            // The last count, with the amber marker and the counting tooltip.
            let tip = message(cx, "hv-find-counting-tip", vec![]);
            return Some((text, Some(tip), true, invalid));
        }
        Some((text, tip, counting, invalid))
    }

    /// A rescan finds its current match again by cell (no focus move).
    fn find_relocated(&mut self) {
        let Some(cell) = self.relocate else {
            return;
        };
        let found = self
            .scan
            .before
            .iter()
            .chain(&self.scan.after)
            .find(|h| h.cell() == cell)
            .map(Hit::key);
        if let Some(key) = found {
            self.current = Some(key);
            self.relocate = None;
            self.marks.borrow_mut().current = Some((cell.row.0, cell.col.0));
        }
    }

    /// "may be incomplete" and its tooltip, when the scan met values cut at the value-size
    /// limit (text past the cut is not searched).
    fn incomplete(&self, cx: &App) -> Option<(SharedString, SharedString)> {
        let n = self.scan.incomplete;
        if n == 0 || self.error.is_some() || self.input.read(cx).value().is_empty() {
            return None;
        }
        let bytes = cx
            .try_global::<crate::shell::AppState>()
            .map_or_else(hacivat_settings::ValueLimit::default, |s| {
                s.settings.results.max_value_bytes
            })
            .get();
        let size = Texts::of(cx).size(u64::from(bytes));
        Some((
            message(cx, "hv-gfind-incomplete", vec![]),
            message(
                cx,
                "hv-gfind-incomplete-tip",
                vec![("n", ArgValue::Count(n)), ("size", ArgValue::Text(size))],
            ),
        ))
    }

    /// Enter and Shift+Enter in the field step (taken before the input).
    fn on_field_key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let m = &k.modifiers;
        if k.key != "enter" || m.control || m.alt || m.platform || m.function {
            return;
        }
        if self.input.read(cx).is_composing() {
            return;
        }
        cx.stop_propagation();
        self.step(m.shift, cx);
    }

    /// Escape anywhere in the bar (the field answers it through `InputEvent::Cancelled`).
    fn on_bar_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        if k.key == "escape" && !k.modifiers.modified() {
            cx.stop_propagation();
            self.close(Some(window), cx);
        }
    }
}

impl Render for GridFindBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // After this frame: the field's focus listeners are installed by then (its first
        // render), so the focus reaches `FocusedInput`.
        if std::mem::take(&mut self.take_focus) {
            let handle = self.input.focus_handle(cx);
            window.defer(cx, move |window, cx| window.focus(&handle, cx));
        }
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let (v, _) = self.shown_view(cx);
        let count = self.count(cx);
        let searchable = self.searchable(cx);
        let field_focused = self.input.focus_handle(cx).is_focused(window);
        let field_handle = self.input.focus_handle(cx).tab_stop(true);
        let mut field = field_well("grid-find-field", self.error.is_some(), cx)
            .flex_none()
            .w(px(240.0))
            .child(
                div()
                    .id("grid-find-query")
                    .flex_1()
                    .min_w(px(40.0))
                    .track_focus(&field_handle)
                    .capture_key_down(cx.listener(Self::on_field_key))
                    .tooltip({
                        let tip = message(cx, "hv-gfind-null-tip", vec![]);
                        move |_, cx| Tip::view(tip.clone(), cx)
                    })
                    .child(self.input.clone()),
            );
        let entity = cx.entity();
        for (i, toggle) in TOGGLES.into_iter().enumerate() {
            let name = message(cx, toggle.name_id(), vec![]);
            let tip = toggle.tip_id().map(|id| message(cx, id, vec![]));
            let on = match toggle {
                Toggle::Case => self.opts.match_case,
                Toggle::Words => self.opts.words,
                Toggle::Regex => self.opts.regex,
                Toggle::Filter => self.filter,
                Toggle::Tolerant => self.opts.tolerant,
            };
            let e = entity.clone();
            field = field.child(toggle_element(
                ToggleSpec {
                    id: toggle.element_id(),
                    node: ("grid-find-toggle-node", i),
                    icon: toggle.icon(),
                    tip: tip.clone().unwrap_or_else(|| name.clone()),
                    name,
                    description: tip,
                    on,
                    handle: self.toggles[i].clone(),
                },
                window,
                move |_, cx| e.update(cx, |b, cx| b.flip(toggle, cx)),
                cx,
            ));
        }
        let field = focus_outline(field, &theme, field_focused);
        let mut count_el = div()
            .id("grid-find-count")
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .min_w(px(58.0))
            .gap(px(6.0));
        match &count {
            Some((text, _, false, _)) => self.announced = text.clone(),
            None => self.announced = SharedString::default(),
            _ => {}
        }
        if let Some((text, tip, counting, invalid)) = count {
            if counting {
                count_el = count_el.child(run_marker(&theme));
            }
            if invalid {
                count_el = count_el.child(
                    Icon::new(IconName::CircleAlert)
                        .size(IconSize::Chrome)
                        .color(c.text.to_hsla()),
                );
            }
            let (style, ink) = if invalid {
                (t.type_ramp.caption, c.text.to_hsla())
            } else if v.total == Some(0) {
                (t.type_ramp.caption, c.text_2.to_hsla())
            } else {
                (t.type_ramp.mono_data, c.text_2.to_hsla())
            };
            count_el = count_el
                .child(Label::new(text, TextSource::Fluent, style, ink).id("grid-find-count-text"));
            if let Some(tip) = tip {
                count_el = count_el.tooltip(move |_, cx| Tip::view(tip.clone(), cx));
            }
        }
        // "may be incomplete": `text-2`, its own tooltip (NOTES "Find count").
        let incomplete = self.incomplete(cx).map(|(text, tip)| {
            div()
                .id("grid-find-incomplete")
                .flex_none()
                .border_b_1()
                .border_dashed()
                .border_color(c.text_2.to_hsla())
                .child(
                    Label::new(
                        text,
                        TextSource::Fluent,
                        t.type_ramp.caption,
                        c.text_2.to_hsla(),
                    )
                    .id("grid-find-incomplete-text"),
                )
                .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
        });
        let count_el = A11yNode::new(
            "grid-find-count-node",
            Role::Status,
            self.announced.clone(),
            false,
            count_el,
        )
        .live(hacivat_ui_core::gpui_surface::accesskit::Live::Polite);
        let icon_button = |id: &'static str,
                           name: &'static str,
                           icon: IconName,
                           handle: &FocusHandle,
                           disabled: bool,
                           cx: &App| {
            Button::new(id, message(cx, name, vec![]))
                .kind(ButtonKind::Icon)
                .icon(icon)
                .compact(true)
                .on_raised(true)
                .disabled(disabled)
                .focus_handle(handle.clone())
        };
        let (e1, e2, e3, e4) = (
            entity.clone(),
            entity.clone(),
            entity.clone(),
            entity.clone(),
        );
        let prev = icon_button(
            "grid-find-prev",
            "hv-find-prev",
            IconName::ArrowUp,
            &self.prev,
            !searchable,
            cx,
        )
        .on_click(move |_, _, cx| e1.update(cx, |b, cx| b.step(true, cx)));
        let next = icon_button(
            "grid-find-next",
            "hv-find-next",
            IconName::ArrowDown,
            &self.next,
            !searchable,
            cx,
        )
        .on_click(move |_, _, cx| e2.update(cx, |b, cx| b.step(false, cx)));
        let all = Button::new(
            "grid-find-select-all",
            message(cx, "hv-gfind-select-all", vec![]),
        )
        .kind(ButtonKind::Ghost)
        .compact(true)
        .on_raised(true)
        .disabled(!searchable)
        .focus_handle(self.all.clone())
        .on_click(move |_, _, cx| e3.update(cx, |b, cx| b.select_all(cx)));
        let close = icon_button(
            "grid-find-close",
            "hv-gfind-close",
            IconName::X,
            &self.close,
            false,
            cx,
        )
        .on_click(move |_, window, cx| e4.update(cx, |b, cx| b.close(Some(window), cx)));
        let row = div()
            .id(("grid-find-bar", self.console.0))
            .debug_selector(|| "grid-find-bar".to_string())
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(28.0))
            .gap(px(6.0))
            .on_key_down(cx.listener(Self::on_bar_key))
            .child(field)
            .child(count_el)
            .children(incomplete)
            .child(prev)
            .child(next)
            .child(all)
            .child(close);
        A11yNode::new(
            "grid-find-bar-node",
            Role::Search,
            message(cx, "hv-gfind-placeholder", vec![]),
            false,
            row,
        )
    }
}

impl std::fmt::Debug for GridFindBar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GridFindBar")
            .field("console", &self.console)
            .field("open", &self.open)
            .field("opts", &self.opts)
            .field("filter", &self.filter)
            .finish_non_exhaustive()
    }
}
