//! The results tile (Task 30): the result tab strip, the grid of the active tab, the cursor
//! strip, the output log view, and the events the console's run UI handles.
//!
//! - **Which console (R-M3-41).** The tile shows the results of the **focused pane's active
//!   console**: the workspace's active tab, when it is a registered console. Anything else (no
//!   console, a console that never ran) keeps the **unchanged M1 tile** (`tiles::results`), so
//!   the M1 and M2 shots do not move. A console has run once its log has had a line or its
//!   results a tab.
//! - **Layout** (NOTES surface 9): the tab strip (Output first; `toolbar.rs`), the toolbar of
//!   the active tab, the 2,000-column hint, then the body: the active result's grid with the
//!   cursor strip under it (`strip.rs`), or the output log (`output_view.rs`).
//! - **One cached `GridView` per result tab**, kept across re-runs (its adapter retargets; the
//!   grid sees the new generation) and dropped with its tab. The grid is embedded as a cached
//!   view: the fork no longer reuses a cached view's prepaint while assistive technology is
//!   attached, so its accessibility node survives (ledger, fork carry). Its configuration is
//!   the app's: auto-sized columns (`GridConfig::auto_size`, ledger T26 note), Fluent labels,
//!   the tab's title as its accessible name, the widths the tab kept.
//! - **Notifications.** A tab's adapter calls back at most once per painted frame; the
//!   callback repaints the grid and the tile (the strip and the toolbar count). The console's
//!   log is awaited on its `watch` (a `tokio::sync` channel, allowed on the UI thread, Q4); a
//!   line marks the Output tab unread while another tab is active.
//! - **Events.** `ResultsEvent`s are emitted on the console's `ConsoleResults` entity, which
//!   the run UI (Task 33) subscribes to: `Rerun` from the strip, `GoToSource` from an error
//!   line of the log.
//! - **Copy.** A grid's `CopyRequested` goes to `copy::copy_region` with the generation the
//!   region was cut from (ledger T26 note); a refusal to `copy::copy_refused`.
//! - **Commands** ([`register_commands`]): `results.copy`, `results.fetch_more`,
//!   `results.fetch_all`, `results.close_cursor`, `results.rerun`, and every `grid.*` id of
//!   `hacivat_ui_grid::GRID_COMMANDS`, dispatched to the focused grid (else the shown one).
//!
//! ## m3b-t25-results (design/m3b-safe surface 10)
//! - **The tab strip** shows each tab's title (`ConsoleResults::title_with_source`), its pin
//!   and its untyped note; a right press opens the tab menu (`pin.rs`). The strip is measured
//!   each frame: when its tabs would not fit, it takes its short form (NOTES ruling 30).
//! - **The log is read incrementally** (`LogRows::take_new`): a change costs its new lines; the
//!   strip facts of each tab merge what the new lines say. ↑/↓ and Enter work on the log while
//!   it has the focus (`output_view.rs`).
//! - **Fetch all** (the strip's, `results.fetch_all`) asks first when the count says the rows
//!   would pass the result's memory limit ([`fetch_all_or_ask`]).
//! - **A copy past the rows a result kept** (after `Closed { Evicted }`) is refused with a
//!   notice, never clamped.
//! - **The toolbar's `+`** follows the strip (ledger 342): a result whose further sets were
//!   drained (`Truncated(FollowingResultSets)`) has all its rows, so no `+`.

use std::cell::Cell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use hacivat_exec::{LogLine, LogMsg, SessionLog, TabKey, copy_invariant};
use hacivat_l10n::{Count, ExactDecimal, Text, ids};
use hacivat_results::{Completeness, PauseReason, StoreSnapshot, TruncReason};
use hacivat_session::session::UnitId;
use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui::popover::Declare as _;
use hacivat_ui::text::TextFn;
use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, AppContext as _, ClipboardItem, Context, Entity, EventEmitter,
    FocusHandle, Focusable, Global, InteractiveElement, IntoElement, ParentElement, Render, Role,
    SharedString, StyleRefinement, Styled, Subscription, Task, WeakEntity, Window, div,
};
use hacivat_ui_core::theme::Theme;
use hacivat_ui_grid::model::{DEFAULT_ROW_HEIGHT, ViewMap};
use hacivat_ui_grid::{
    GRID_COMMANDS, GRID_CONTEXT, GridCommand, GridConfig, GridDataSource, GridEvent, GridLabels,
    GridView, VirtualList,
};

use crate::connections::ConnectionsModel;
use crate::console::{ConsoleId, Consoles};
use crate::l10n_global::{text_fn, tr};
use crate::results::adapter::GridAdapter;
use crate::results::copy;
use crate::results::output_view::{LOG_ROW_PX, LogRows};
use crate::results::strip::{self, StripActions, StripFacts, StripModel};
use crate::results::tabs::{
    ActiveResult, ConsoleResults, CountState, MemoryBanner, ResultEntry, result_title,
};
use crate::results::toolbar::{
    self, OnActivate, OnTabMenu, SHORT_TITLE_PX, TITLE_PX, TabFacts, TabsModel, WIDE_RESULT_COLUMNS,
};
use crate::shell::Workspace;
// ## m3a-f5-demo
use crate::shell::register_focus_context;

/// What the results tile asks of the console's run UI (Task 33 handles them). The tile emits
/// them on the console's [`ConsoleResults`] entity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResultsEvent {
    /// Run the unit behind a closed result again.
    Rerun {
        console: ConsoleId,
        unit_ordinal: u32,
    },
    /// Select the statement an output-log error line points at.
    GoToSource {
        console: ConsoleId,
        range: Range<usize>,
    },
}

// ## m3a-t30-results-view: the run UI subscribes to a console's results for these.
impl EventEmitter<ResultsEvent> for ConsoleResults {}

/// One result tab's grid.
struct GridSlot {
    adapter: Rc<GridAdapter>,
    view: Entity<GridView>,
    _events: Subscription,
}

/// What the tile keeps for one console.
struct ConsoleTile {
    results: Entity<ConsoleResults>,
    log: Arc<SessionLog>,
    list: Entity<VirtualList<LogRows>>,
    /// The log has had a line (or been cleared): the console ran.
    ran: bool,
    grids: Vec<GridSlot>,
    // ## m3a-t30 fix round 1 (I2): the strip's facts per tab, kept when the log is cleared or
    // its ring drops the lines they came from; reset when the tab shows another unit.
    // ## m3b-t25-results: keyed by the tab's key and unit (a pinned tab shares its key).
    facts: HashMap<(TabKey, UnitId), StripFacts>,
    // ## m3b-t25-results
    /// The log list's focus (↑/↓ and Enter select and follow its lines).
    log_focus: FocusHandle,
    /// The tab strip's width in the last frame (its short form follows it).
    strip_width: Rc<Cell<Option<f32>>>,
    /// Fix round 1 (m12): the laid-out widths of the strip's texts (titles, notes), so a frame
    /// shapes only what is new.
    text_widths: Rc<std::cell::RefCell<HashMap<SharedString, f32>>>,
    /// Where each result tab was painted (its menu opens under it).
    tab_bounds: TabBounds,
    _observe: Subscription,
    _events: Subscription,
    _log: Task<()>,
}

impl ConsoleTile {
    /// The grid of result tab `ix` of `results`, once built.
    fn grid(&self, results: &ConsoleResults, ix: usize) -> Option<Entity<GridView>> {
        let adapter = results.tab(ix)?.adapter();
        self.grids
            .iter()
            .find(|s| Rc::ptr_eq(&s.adapter, adapter))
            .map(|s| s.view.clone())
    }
}

/// The results tile's view.
pub struct ResultsHost {
    shown: Option<ConsoleId>,
    tiles: HashMap<ConsoleId, ConsoleTile>,
    // ## m3b-t01b-seams (P-M3b-21): the tile root's focus, registered as the `Results` key
    // context.
    focus: FocusHandle,
}

impl std::fmt::Debug for ResultsHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResultsHost")
            .field("shown", &self.shown)
            .field("consoles", &self.tiles.len())
            .finish()
    }
}

/// Every results tile (one per window), for the commands and tests.
#[derive(Default)]
struct ResultsHosts(Vec<WeakEntity<ResultsHost>>);

impl Global for ResultsHosts {}

/// The live results tiles.
pub fn hosts(cx: &App) -> Vec<Entity<ResultsHost>> {
    cx.try_global::<ResultsHosts>()
        .map(|h| h.0.iter().filter_map(WeakEntity::upgrade).collect())
        .unwrap_or_default()
}

/// The console the focused pane shows, with its results: the workspace's active tab when it is
/// a registered console that has results.
pub(crate) fn console_in(window: &Window, cx: &App) -> Option<(ConsoleId, Entity<ConsoleResults>)> {
    let ws = window.root::<Workspace>().flatten()?;
    let tab = ws.read(cx).active_tab()?;
    let id = Consoles::by_tab(cx, tab)?;
    Some((id, Consoles::get(cx, id)?.results.clone()?))
}

/// The grid configuration of result `number` over `adapter`.
fn grid_config(number: u32, adapter: &Rc<GridAdapter>) -> GridConfig {
    let wide = Rc::downgrade(adapter);
    let too_many: TextFn = Rc::new(move |cx: &App| {
        let n = wide.upgrade().map_or(0, |a| a.columns().len()) as u64;
        tr(
            cx,
            ids::hv::GRID_TOO_MANY_COLUMNS,
            ids::hv::HvGridTooManyColumns { n: Count(n) },
        )
    });
    GridConfig {
        row_height: DEFAULT_ROW_HEIGHT,
        zebra: false,
        // Ledger T26 note: the app sizes its columns from sampled rows.
        auto_size: true,
        labels: GridLabels {
            loading: text_fn(ids::hv::GRID_LOADING),
            too_many_columns: too_many,
            no_rows: text_fn(ids::hv::GRID_NO_ROWS),
            // ## m3c-t35 (named exception): Task 35's frozen tooltip and resize tag.
            frozen_tip: text_fn(ids::hv::GRID_FROZEN_TIP),
            resize_tip: text_fn(ids::hv::GRID_RESIZE_TIP),
            width: Rc::new(|width: f32, cx: &App| {
                tr(
                    cx,
                    ids::hv::GRID_WIDTH,
                    ids::hv::HvGridWidth {
                        px: Count(width.max(0.0).round() as u64),
                    },
                )
            }),
        },
        a11y_label: Rc::new(move |cx: &App| result_title(number, cx)),
        // ## m3b-t01b-seams: a table until Task 38's views.
        transposed: false,
    }
}

/// The rows of `entry` in `snapshot` and their time, as the toolbar shows them ("10,000+ rows ·
/// 131 ms"). ## m3a-t30 fix round 1: Task 34's messages and rules, so the toolbar and the status
/// bar say the same thing: "+" while more can come (streaming, suspended, truncated), the time
/// is execution plus fetching, whole milliseconds under a second, else seconds with two
/// decimals (`hv-status-duration-*`). The snapshot is the frame's one load of the tab (M10).
fn rows_timing(entry: &ResultEntry, snapshot: &StoreSnapshot, cx: &App) -> SharedString {
    let rows = Count(snapshot.row_count());
    // ## m3b-t25-results (ledger 342): the strip's count is the one source: a result whose
    // further sets were drained has every row of its own, so no `+`.
    let more = matches!(
        snapshot.completeness(),
        Completeness::Streaming
            | Completeness::Suspended(_)
            | Completeness::Truncated(TruncReason::ResultCap)
    );
    let duration = match entry.timing() {
        (None, None) => None,
        (e, f) => Some(Text(
            duration_label(e.unwrap_or(0).saturating_add(f.unwrap_or(0)), cx).to_string(),
        )),
    };
    match (more, duration) {
        (false, None) => tr(cx, ids::hv::RESULTS_ROWS, ids::hv::HvResultsRows { rows }),
        (true, None) => tr(
            cx,
            ids::hv::RESULTS_ROWS_MORE,
            ids::hv::HvResultsRowsMore { rows },
        ),
        (false, Some(duration)) => tr(
            cx,
            ids::hv::RESULTS_ROWS_TIMING,
            ids::hv::HvResultsRowsTiming { rows, duration },
        ),
        (true, Some(duration)) => tr(
            cx,
            ids::hv::RESULTS_ROWS_MORE_TIMING,
            ids::hv::HvResultsRowsMoreTiming { rows, duration },
        ),
    }
}

/// A duration as the status bar writes it (Task 34's `duration_label`, which is private to its
/// file): whole milliseconds under a second, else seconds with two decimals.
fn duration_label(ms: u64, cx: &App) -> SharedString {
    if ms < 1000 {
        return tr(
            cx,
            ids::hv::STATUS_DURATION_MS,
            ids::hv::HvStatusDurationMs { ms: Count(ms) },
        );
    }
    tr(
        cx,
        ids::hv::STATUS_DURATION_S,
        ids::hv::HvStatusDurationS {
            seconds: ExactDecimal(format!("{}.{:02}", ms / 1000, (ms % 1000) / 10)),
        },
    )
}

/// The memory banner of `entry` as `snapshot` shows it (M10: the strip, the banner and the
/// toolbar read one snapshot). Whether "Fetch more anyway" is still offered comes from the tab,
/// which alone knows the cap was raised.
fn banner_of(entry: &ResultEntry, snapshot: &StoreSnapshot) -> Option<MemoryBanner> {
    if !matches!(snapshot.completeness(), Completeness::Suspended(_)) {
        return None;
    }
    let reason = snapshot.paused()?;
    let raised = entry.memory_banner().is_some_and(|b| !b.can_raise);
    Some(MemoryBanner {
        reason,
        can_raise: !(raised && reason == PauseReason::GlobalBudget),
    })
}

/// The size of `grid`'s selection over `loaded` rows: `rows × cols` when it has one block;
/// ## m3c-t35 (named exception): "2 ranges · 10 cells" when it holds several ranges (NOTES
/// M3c-22), counted through the grid's view map.
fn selection_dims(grid: &GridView, loaded: u64) -> Option<strip::StripSelection> {
    let selection = grid.selection();
    if selection.is_empty() {
        return None;
    }
    let map = match grid.view_map() {
        ViewMap::Identity { .. } => ViewMap::identity(loaded),
        other => other.clone(),
    };
    let order = grid.column_layout().visible();
    if selection.ranges() > 1 {
        let cells = selection.cell_count_in(&map, order);
        return Some(strip::StripSelection::Ranges {
            n: selection.ranges() as u64,
            cells: u64::try_from(cells).unwrap_or(u64::MAX),
        });
    }
    let region = selection.normalize_for_copy(&map, order).ok()?;
    Some(strip::StripSelection::Dims {
        rows: region.row_count(),
        cols: region.cols.len() as u64,
    })
}

impl ResultsHost {
    fn new(cx: &mut Context<Self>) -> Self {
        ResultsHost {
            shown: None,
            tiles: HashMap::new(),
            focus: cx.focus_handle(),
        }
    }

    /// The console the tile showed in its last frame.
    pub fn shown(&self) -> Option<ConsoleId> {
        self.shown
    }

    /// The grid of `console`'s result tab `ix`, once the tile has shown it.
    pub fn grid_of(&self, console: ConsoleId, ix: usize, cx: &App) -> Option<Entity<GridView>> {
        let tile = self.tiles.get(&console)?;
        tile.grid(tile.results.read(cx), ix)
    }

    // ## m3b-t25-results
    /// Where `console`'s result tab `ix` was last painted.
    pub fn tab_bounds(
        &self,
        console: ConsoleId,
        ix: usize,
    ) -> Option<hacivat_ui_core::gpui_surface::Bounds<hacivat_ui_core::gpui_surface::Pixels>> {
        self.tiles
            .get(&console)?
            .tab_bounds
            .borrow()
            .get(&ix)
            .copied()
    }

    /// `console`'s output list, once the tile has shown the console.
    pub fn log_list(&self, console: ConsoleId) -> Option<Entity<VirtualList<LogRows>>> {
        self.tiles.get(&console).map(|t| t.list.clone())
    }

    /// Starts following `console` (its results and its log), once.
    fn ensure_tile(
        &mut self,
        console: ConsoleId,
        results: &Entity<ConsoleResults>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .tiles
            .get(&console)
            .is_some_and(|t| &t.results == results)
        {
            return;
        }
        let log = results.read(cx).output().clone();
        let mut rows = LogRows::new(console, results.downgrade());
        rows.set_lines(log.lines());
        let list = cx.new(|_| VirtualList::new(rows, LOG_ROW_PX));
        // ## m3b-t25-results: a click on a row selects it and focuses the log.
        let log_focus = cx.focus_handle();
        let weak_list = list.downgrade();
        let focus = log_focus.clone();
        list.update(cx, |l, cx| l.rows_mut(cx).attach(weak_list, focus));
        // The newest line, then following new ones.
        list.update(cx, |l, cx| l.scroll_to_end(cx));
        let mut changes = log.changes();
        let ran = *changes.borrow() > 0;
        let observe = cx.observe(results, |_, _, cx| cx.notify());
        // ## m3a-t30 fix round 1 (T33 merge obligation): what the tile asks of the run UI.
        let events = cx.subscribe_in(results, window, |_, _, event: &ResultsEvent, window, cx| {
            crate::exec_ui::handle_results_event(event, window, cx);
        });
        let task = cx.spawn(async move |this, cx| {
            while changes.changed().await.is_ok() {
                if this
                    .update(cx, |host, cx| host.on_log(console, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        self.tiles.insert(
            console,
            ConsoleTile {
                results: results.clone(),
                log,
                list,
                ran,
                grids: Vec::new(),
                facts: HashMap::new(),
                log_focus,
                strip_width: Rc::new(Cell::new(None)),
                text_widths: Rc::default(),
                tab_bounds: TabBounds::default(),
                _observe: observe,
                _events: events,
                _log: task,
            },
        );
    }

    /// `console`'s log changed: the list takes its new lines (## m3b-t25-results: only those,
    /// `LogRows::take_new`); the tabs' strip facts take what the new lines say; a line while
    /// another tab is active marks Output unread; a line of a run that opened no tab shows
    /// Output (I5).
    fn on_log(&mut self, console: ConsoleId, cx: &mut Context<Self>) {
        let Some(tile) = self.tiles.get_mut(&console) else {
            return;
        };
        tile.ran = true;
        let log = tile.log.clone();
        let new = tile.list.update(cx, |l, cx| l.rows_mut(cx).take_new(&log));
        let fresh: Vec<&LogLine> = new
            .iter()
            .filter(|l| !matches!(l.msg, LogMsg::Dropped { .. }))
            .collect();
        let results = tile.results.read(cx);
        let rows = tile.list.read(cx).rows();
        for entry in results.tabs() {
            let key = (entry.key(), entry.unit());
            match tile.facts.get_mut(&key) {
                Some(facts) => facts.merge(StripFacts::from_lines(new.iter(), entry.unit())),
                None => {
                    let facts = StripFacts::from_lines(rows.iter_lines(), entry.unit());
                    tile.facts.insert(key, facts);
                }
            }
        }
        // NOTES surface 9: a run that ends in an error or update counts only (no tab of its
        // own) shows the Output tab, not an earlier run's result. A line of the run whose
        // result is shown leaves it in view, and a later tab of the run takes the strip back
        // (`ConsoleResults::apply` activates a run's first tab once).
        let active_run = match results.active() {
            ActiveResult::Tab(ix) => results.tab(ix).map(|t| t.unit().run),
            ActiveResult::Output => None,
        };
        let show_output = active_run.is_some_and(|active| {
            fresh.iter().any(|l| {
                let run = match (&l.msg, l.unit) {
                    (_, Some(unit)) => Some(unit.run),
                    // A run that never started names no unit.
                    (LogMsg::NotStarted(_), None) => None,
                    _ => return false,
                };
                // ## m3a-t30 fix round 2 (R1-1): only a newer run (`RunId` grows): a late line
                // of an older run (the old cursor's close, read after a re-run took the tab)
                // never hides the new run's result.
                run.is_none_or(|run| {
                    run > active && !results.tabs().iter().any(|t| t.unit().run == run)
                })
            })
        });
        let added = !fresh.is_empty();
        if show_output {
            tile.results
                .update(cx, |r, cx| r.set_active(ActiveResult::Output, cx));
        } else if added {
            tile.results.update(cx, |r, cx| r.note_output(cx));
        }
        cx.notify();
    }

    /// The strip facts of tab `entry` (its memo, built from the log on first sight).
    fn facts_of(
        &mut self,
        console: ConsoleId,
        entry_key: TabKey,
        unit: UnitId,
        cx: &App,
    ) -> StripFacts {
        let Some(tile) = self.tiles.get_mut(&console) else {
            return StripFacts::default();
        };
        // ## m3b-t25-results: keyed by key and unit.
        match tile.facts.get(&(entry_key, unit)) {
            Some(facts) => *facts,
            None => {
                let facts = StripFacts::from_lines(tile.list.read(cx).rows().iter_lines(), unit);
                tile.facts.insert((entry_key, unit), facts);
                facts
            }
        }
    }

    /// Keeps the keyboard focus on what the tile shows (C1): when a grid the tile no longer
    /// shows (another result tab, another console's) holds the focus, the focus moves to the
    /// shown grid, or leaves the hidden one when none is shown. GPUI keeps a focus on an
    /// element that stopped rendering, and the palette restores the focus it found.
    fn guard_focus(
        &self,
        shown: Option<&Entity<GridView>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let hidden_focused = self.tiles.values().any(|t| {
            t.grids.iter().any(|s| {
                shown.is_none_or(|g| g.entity_id() != s.view.entity_id())
                    && s.view
                        .read(cx)
                        .focus_handle(cx)
                        .contains_focused(window, cx)
            })
        });
        if !hidden_focused {
            return;
        }
        let target = shown.map(|g| g.read(cx).focus_handle(cx));
        window.defer(cx, move |window, cx| match target {
            Some(handle) => window.focus(&handle, cx),
            None => window.blur(cx),
        });
    }

    /// Forgets consoles that are gone and grids whose tab closed.
    fn prune(&mut self, cx: &App) {
        let live = Consoles::ids(cx);
        self.tiles.retain(|id, _| live.contains(id));
        for tile in self.tiles.values_mut() {
            let results = tile.results.read(cx);
            tile.grids.retain(|slot| {
                results
                    .tabs()
                    .iter()
                    .any(|t| Rc::ptr_eq(t.adapter(), &slot.adapter))
            });
            // ## m3b-t25-results: the facts of tabs that are gone go with them.
            tile.facts.retain(|(key, unit), _| {
                results
                    .tabs()
                    .iter()
                    .any(|t| t.key() == *key && t.unit() == *unit)
            });
        }
    }

    /// The grid of `console`'s result tab `ix`, built on first sight.
    fn ensure_grid(
        &mut self,
        console: ConsoleId,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> Option<Entity<GridView>> {
        let tile = self.tiles.get(&console)?;
        let results = tile.results.clone();
        if let Some(view) = tile.grid(results.read(cx), ix) {
            return Some(view);
        }
        let (adapter, number, widths) = {
            let entry = results.read(cx).tab(ix)?;
            (
                entry.adapter().clone(),
                entry.number(),
                entry.widths().to_vec(),
            )
        };
        let config = grid_config(number, &adapter);
        let source: Rc<dyn GridDataSource> = adapter.clone();
        let view = cx.new(|cx| GridView::new(source, config, cx));
        // ## m3a-f5-demo (DEMO-6): while the grid has the focus its key context is on the
        // workspace's context stack, so its keys (⌘C, arrows, ⌘A …) resolve through the
        // dispatcher. The registration holds the handle weakly, so it ends with the grid
        // however the grid goes (a closed tab, a closed window; review m-4).
        let focus = view.read(cx).focus_handle(cx);
        register_focus_context(cx, focus, GRID_CONTEXT);
        view.update(cx, |g, cx| {
            for (col, width) in widths {
                g.set_column_width(col, width, cx);
            }
        });
        // The adapter calls back at most once per painted frame: repaint the grid and the
        // strip and toolbar that count its rows.
        let (grid, host) = (view.downgrade(), cx.entity().downgrade());
        adapter.on_change(cx, move |cx| {
            let _ = grid.update(cx, |_, cx| cx.notify());
            let _ = host.update(cx, |_, cx| cx.notify());
        });
        let copied = adapter.clone();
        let events = cx.subscribe(&view, move |_, _, e: &GridEvent, cx| match e {
            GridEvent::CopyRequested {
                region,
                headers,
                generation,
            } => {
                // ## m3b-t25-results: a selection past the rows the result keeps (it was
                // closed to free memory) copies nothing, and says so; never a clamped copy.
                let kept = copied.snapshot().row_count();
                if region.rows.iter().any(|r| r.end > kept) {
                    copy_past_kept_rows(cx);
                    return;
                }
                copy::copy_region(&copied, region.clone(), *headers, *generation, cx).detach();
            }
            GridEvent::CopyRefused(refusal) => copy::copy_refused(*refusal, cx),
            GridEvent::ColumnResized { col, width } => {
                let (col, width) = (*col, *width);
                let adapter = copied.clone();
                results.update(cx, |r, _| {
                    if let Some(ix) = r
                        .tabs()
                        .iter()
                        .position(|t| Rc::ptr_eq(t.adapter(), &adapter))
                    {
                        r.set_width(ix, col, width);
                    }
                });
            }
            GridEvent::SelectionChanged => cx.notify(),
            GridEvent::FocusChanged(_) | GridEvent::VisibleRegion { .. } => {}
            // ## m3b-t01b-seams: each M3b request goes to its module (stubs until Tasks 36–39
            // fill them); the frozen count needs nothing from the tile yet (Task 35).
            GridEvent::SortRequested(spec) => super::sort::on_requested(console, spec.clone(), cx),
            GridEvent::FindRequested => super::find::open(console, cx),
            GridEvent::GotoRequested => super::goto::open(console, cx),
            GridEvent::ValueRequested(cell) => super::value_viewer::open(console, *cell, cx),
            GridEvent::ContextMenuRequested { at, cell, header } => {
                super::context_menu::open(console, *at, *cell, *header, cx);
            }
            GridEvent::FrozenChanged(_) => {}
        });
        self.tiles.get_mut(&console)?.grids.push(GridSlot {
            adapter,
            view: view.clone(),
            _events: events,
        });
        Some(view)
    }

    /// The tile of `console`, which has run.
    fn tile_element(
        &mut self,
        console: ConsoleId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let theme = *Theme::get(cx);
        let c = theme.tokens.colors;
        let tile = self.tiles.get(&console)?;
        let results = tile.results.clone();
        // ## m3b-t25-results: each tab's title, its source, pin and untyped note.
        // ## m3b-t25-results: each tab's title, its source, pin and untyped note; fix round 1:
        // every title in one pass, its repeat apart (m9, m12).
        let (active, unread, titles, facts, names) = {
            let r = results.read(cx);
            let all = r.titles(cx);
            let mut titles = Vec::with_capacity(all.len());
            let mut facts = Vec::with_capacity(all.len());
            let mut names = Vec::with_capacity(all.len());
            for (entry, title) in r.tabs().iter().zip(all) {
                facts.push(TabFacts {
                    data_title: title.source == hacivat_ui::text::TextSource::Data,
                    pinned: entry.pinned(),
                    untyped: entry.untyped(),
                    repeat: title.repeat,
                });
                titles.push(title.base);
                names.push(title.whole);
            }
            (r.active(), r.unread_output(), titles, facts, names)
        };
        let env = Consoles::get(cx, console)
            .and_then(|s| s.profile)
            .and_then(|p| cx.try_global::<ConnectionsModel>()?.get(p))
            .and_then(|p| p.environment);
        let weak = results.downgrade();
        let on_activate: OnActivate = Rc::new(move |a, _, cx| {
            let _ = weak.update(cx, |r, cx| r.set_active(a, cx));
        });
        // ## m3b-t25-results: the short form, then (fix round 1, I1) the "+N" menu, when last
        // frame's strip could not hold the tabs whole.
        let strip_width = tile.strip_width.clone();
        let (short, shown) = strip_fit(
            &StripTexts {
                titles: &titles,
                facts: &facts,
                env,
                unread,
            },
            active,
            strip_width.get(),
            &tile.text_widths,
            window,
            cx,
        );
        let on_menu: OnTabMenu = Rc::new(move |ix, at, window, cx| {
            super::pin::open_tab_menu(console, ix, at, window, cx);
        });
        let bounds = tile.tab_bounds.clone();
        bounds.borrow_mut().clear();
        let on_bounds: toolbar::OnTabBounds = Rc::new(move |ix, b| {
            bounds.borrow_mut().insert(ix, b);
        });
        let hidden: Vec<usize> = match &shown {
            Some(shown) => (0..titles.len()).filter(|ix| !shown.contains(ix)).collect(),
            None => Vec::new(),
        };
        let hidden_names: Vec<(usize, SharedString)> = hidden
            .iter()
            .filter_map(|ix| names.get(*ix).map(|n| (*ix, n.clone())))
            .collect();
        let on_more: toolbar::OnMore = Rc::new(move |at, window, cx| {
            open_more_menu(console, hidden_names.clone(), at, window, cx);
        });
        let strip_tabs = toolbar::tab_strip(
            &TabsModel {
                active,
                unread,
                results: titles,
                env,
                facts,
                short,
                names,
                shown,
            },
            on_activate,
            Some(on_menu),
            Some(on_bounds),
            Some(on_more),
            cx,
        );
        let measured = strip_width.clone();
        let strip_tabs = div()
            .relative()
            .w_full()
            .flex_none()
            .child(strip_tabs)
            .child(
                hacivat_ui_core::gpui_surface::canvas(
                    move |b, window, _| {
                        let w = f32::from(b.size.width);
                        if measured.get() != Some(w) {
                            measured.set(Some(w));
                            window.refresh();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            );
        let mut column = div()
            .id("results-tile")
            // ## m3b-t01b-seams (P-M3b-21): the `Results` key context's root.
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .bg(c.panel.to_hsla())
            .child(strip_tabs);
        match active {
            ActiveResult::Output => {
                self.guard_focus(None, window, cx);
                let tile = self.tiles.get(&console)?;
                let list = tile.list.clone();
                let log = tile.log.clone();
                let lines_empty = list.read(cx).rows().is_empty();
                // ## m3b-t25-results: ↑/↓ and Enter on the focused log.
                let log_focus = tile.log_focus.clone();
                let keys = list.downgrade();
                let emit = results.downgrade();
                let shown = list.clone();
                let cleared = list.clone();
                let ends = list.clone();
                column = column
                    .child(toolbar::output_toolbar(
                        // Fix round 1 (m7): the rows forget their lines with the log, so a line
                        // appended before the change is read never keeps them.
                        Rc::new(move |_, cx| {
                            log.clear();
                            cleared.update(cx, |l, cx| l.rows_mut(cx).clear());
                        }),
                        Rc::new(move |_, cx| {
                            let text = copy_invariant(&shown.read(cx).rows().lines());
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                        }),
                        Rc::new(move |_, cx| ends.update(cx, |l, cx| l.scroll_to_end(cx))),
                        cx,
                    ))
                    .child(
                        div()
                            .id("results-log")
                            .debug_selector(|| "results-log".to_string())
                            // ## m3b-t25-results: element-level keys (no keymap rows).
                            .track_focus(&log_focus)
                            .on_key_down(move |event, window, cx| {
                                on_log_key(&keys, &emit, event, window, cx);
                            })
                            .declare(
                                Role::List,
                                tr(cx, ids::hv::RESULTS_LOG_LABEL, hacivat_l10n::NoArgs),
                                None,
                            )
                            .flex_1()
                            .min_h_0()
                            .overflow_hidden()
                            .child(if lines_empty {
                                toolbar::log_empty(cx)
                            } else {
                                list.into_any_element()
                            }),
                    );
            }
            ActiveResult::Tab(ix) => {
                let grid = self.ensure_grid(console, ix, cx)?;
                self.guard_focus(Some(&grid), window, cx);
                let (key, unit) = {
                    let entry = results.read(cx).tab(ix)?;
                    (entry.key(), entry.unit())
                };
                let facts = self.facts_of(console, key, unit, cx);
                // Fix round 1 (m4): an eviction that landed while this tab was shown clears its
                // selection too (after this frame: the adapter tells this tile).
                if results.read(cx).active_evicted_unanswered() {
                    let weak = results.downgrade();
                    cx.defer(move |cx| {
                        let _ = weak.update(cx, |r, cx| r.on_shown(cx));
                    });
                }
                let r = results.read(cx);
                let entry = r.tab(ix)?;
                let adapter = entry.adapter().clone();
                // M10: one snapshot of the tab for the strip, the banner and the toolbar.
                let snapshot = adapter.snapshot();
                let columns = adapter.columns().len();
                let model = StripModel::of(&snapshot, banner_of(entry, &snapshot), facts);
                let selection = selection_dims(grid.read(cx), snapshot.row_count());
                let rows = rows_timing(entry, &snapshot, cx);
                let unit_ordinal = entry.key().unit_ordinal;
                let copy_grid = grid.downgrade();
                // ## m3b-t25-results: Σ Count rows after Copy.
                let count = super::count::toolbar_items(console, ix, entry.count(), cx);
                column = column.child(toolbar::result_toolbar(
                    rows,
                    Rc::new(move |window, cx| {
                        let _ = copy_grid.update(cx, |g, cx| {
                            g.dispatch(GridCommand::Copy { headers: false }, window, cx)
                        });
                    }),
                    count,
                    cx,
                ));
                if columns > WIDE_RESULT_COLUMNS {
                    column = column.child(toolbar::columns_hint(columns, cx));
                }
                // ## m3b-t25-results: the result-set limit (R-M3-66).
                if let Some((total, limit)) = facts.set_limit {
                    column = column.child(toolbar::set_limit_row(limit, total - limit, cx));
                }
                let mut style = StyleRefinement::default();
                style.size.width = Some(hacivat_ui_core::gpui_surface::relative(1.0).into());
                style.size.height = Some(hacivat_ui_core::gpui_surface::relative(1.0).into());
                column = column.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .overflow_hidden()
                        .child(AnyView::from(grid).cached(style)),
                );
                column = column.child(strip::render(
                    model,
                    selection,
                    strip_actions(console, &results, ix, unit_ordinal, adapter),
                    cx,
                ));
            }
        }
        Some(column.into_any_element())
    }
}

/// The strip's buttons for tab `ix` of `results`.
fn strip_actions(
    console: ConsoleId,
    results: &Entity<ConsoleResults>,
    ix: usize,
    unit_ordinal: u32,
    adapter: Rc<GridAdapter>,
) -> StripActions {
    let (a, b) = (adapter.clone(), adapter);
    let raise = results.downgrade();
    let emit = results.downgrade();
    let ask = results.downgrade();
    StripActions {
        // The reason follows the state (`StoreSource::close_cursor`): `MemoryLimit` on the
        // banner, `UserClosed` otherwise.
        close_cursor: Rc::new(move |_, _| a.source().close_cursor()),
        fetch_more: Rc::new(move |_, _| {
            let source = b.source();
            source.ensure_rows(source.snapshot().row_count());
        }),
        // ## m3b-t25-results: asks first when the rows would pass the memory limit.
        fetch_all: Rc::new(move |window, cx| {
            if let Some(results) = ask.upgrade() {
                fetch_all_or_ask(&results, ix, window, cx);
            }
        }),
        // Only through `ConsoleResults::raise_cap` (ledger T26 note).
        fetch_anyway: Rc::new(move |_, cx| {
            let _ = raise.update(cx, |r, cx| r.raise_cap(ix, cx));
        }),
        rerun: Rc::new(move |_, cx| {
            let _ = emit.update(cx, |_, cx| {
                cx.emit(ResultsEvent::Rerun {
                    console,
                    unit_ordinal,
                })
            });
        }),
    }
}

// ## m3b-t25-results

/// Where each result tab of a console was painted, by index.
type TabBounds = Rc<
    std::cell::RefCell<
        HashMap<
            usize,
            hacivat_ui_core::gpui_surface::Bounds<hacivat_ui_core::gpui_surface::Pixels>,
        >,
    >,
>;

/// About the tab menu's width: its anchor stays this far from the window's right edge.
const TAB_MENU_PX: f32 = 280.0;

/// Opens the menu of console `console`'s result tab `ix` under the tab (where the tile last
/// painted it; at the pointer before it was ever painted).
pub fn open_tab_menu_for(console: ConsoleId, ix: usize, window: &mut Window, cx: &mut App) {
    let at = hosts(cx)
        .into_iter()
        .find_map(|h| h.read(cx).tab_bounds(console, ix))
        .map_or_else(
            || window.mouse_position(),
            |b| {
                let at = b.bottom_left();
                // Kept inside the window (a tab near its right edge).
                let room =
                    window.viewport_size().width - hacivat_ui_core::gpui_surface::px(TAB_MENU_PX);
                hacivat_ui_core::gpui_surface::point(
                    at.x.min(room),
                    at.y + hacivat_ui_core::gpui_surface::px(4.0),
                )
            },
        );
    super::pin::open_tab_menu(console, ix, at, window, cx);
}

/// A copy whose selection reaches past the rows the result keeps: nothing is copied and a
/// notice says why (the Task 8 carry: never a clamped copy).
fn copy_past_kept_rows(cx: &mut App) {
    let mut toast = hacivat_ui::toast::Toast::new(
        hacivat_ui::toast::ToastKind::Info,
        crate::l10n_global::text_fn(ids::hv::RESULTS_COPY_EVICTED),
    );
    toast.wide = true;
    toast.icon = Some(hacivat_ui::icon::IconName::CircleAlert);
    hacivat_ui::toast::ToastQueue::push(cx, toast);
}

/// ↑/↓ move the log's selected line, Enter goes to its statement (element-level keys while the
/// log has the focus; nothing with a modifier).
fn on_log_key(
    list: &WeakEntity<VirtualList<LogRows>>,
    results: &WeakEntity<ConsoleResults>,
    event: &hacivat_ui_core::gpui_surface::KeyDownEvent,
    window: &mut Window,
    cx: &mut App,
) {
    let _ = window;
    let ks = &event.keystroke;
    if ks.modifiers.modified() {
        return;
    }
    let Some(list) = list.upgrade() else {
        return;
    };
    match ks.key.as_str() {
        "up" | "down" => {
            let down = ks.key == "down";
            list.update(cx, |l, cx| {
                if let Some(row) = l.rows_mut(cx).step(down) {
                    l.scroll_into_view(row, cx);
                }
            });
            cx.stop_propagation();
        }
        "enter" => {
            let event = list.read(cx).rows().go_to_selected();
            if let Some(event) = event {
                let _ = results.update(cx, |_, cx| cx.emit(event));
            }
            cx.stop_propagation();
        }
        _ => {}
    }
}

/// The laid-out width of `text` in `style`.
fn text_px(window: &Window, text: &SharedString, style: hacivat_ui_core::theme::TypeStyle) -> f32 {
    let run = hacivat_ui_core::gpui_surface::TextRun {
        len: text.len(),
        font: hacivat_ui_core::fonts::font_for(&style),
        color: hacivat_ui_core::gpui_surface::Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    f32::from(
        window
            .text_system()
            .shape_line(
                text.clone(),
                hacivat_ui_core::gpui_surface::px(style.size_px),
                &[run],
                None,
            )
            .width,
    )
}

// Fix round 1 (I1)

/// What the tab strip draws, for its fit.
struct StripTexts<'a> {
    titles: &'a [SharedString],
    facts: &'a [TabFacts],
    env: Option<hacivat_proto::EnvTag>,
    unread: bool,
}

/// The laid-out width of `text` (the body style), from `cache` when it was shaped before.
fn cached_px(
    cache: &std::cell::RefCell<HashMap<SharedString, f32>>,
    text: &SharedString,
    window: &Window,
    cx: &App,
) -> f32 {
    if let Some(w) = cache.borrow().get(text) {
        return *w;
    }
    let w = text_px(window, text, Theme::get(cx).tokens.type_ramp.body);
    let mut cache = cache.borrow_mut();
    if cache.len() > 4_096 {
        cache.clear();
    }
    cache.insert(text.clone(), w);
    w
}

/// How the tab strip draws its tabs in `width` (last frame's; `None` before the first): whole
/// when they fit (`(false, None)`), else in their short form (NOTES ruling 30), else the short
/// form of as many as fit, the active tab always among them, the rest behind "+N"
/// (`(true, Some(drawn))`). A drawn tab is never narrower than its chrome, so its environment
/// swatch is always whole.
fn strip_fit(
    texts: &StripTexts,
    active: ActiveResult,
    width: Option<f32>,
    cache: &std::cell::RefCell<HashMap<SharedString, f32>>,
    window: &Window,
    cx: &App,
) -> (bool, Option<Vec<usize>>) {
    use toolbar::{TAB_CHROME_PX, TAB_DOT_PX, TAB_GAP_PX, TAB_GLYPH_PX, TAB_SWATCH_PX};
    let Some(avail) = width else {
        return (false, None);
    };
    let px = |text: &SharedString| cached_px(cache, text, window, cx);
    let output = tr(cx, ids::hv::RESULTS_OUTPUT, hacivat_l10n::NoArgs);
    let mut output_w = TAB_CHROME_PX + TAB_GAP_PX + px(&output);
    if texts.unread {
        output_w += TAB_GAP_PX + TAB_DOT_PX;
    }
    let untyped = tr(cx, ids::hv::RESULTS_UNTYPED, hacivat_l10n::NoArgs);
    let untyped_short = tr(cx, ids::hv::RESULTS_UNTYPED_SHORT, hacivat_l10n::NoArgs);
    let staging = match texts.env {
        Some(hacivat_proto::EnvTag::Staging) => Some(tr(
            cx,
            ids::hv::RESULTS_ENV_SUFFIX,
            ids::hv::HvResultsEnvSuffix {
                env: Text(
                    tr(
                        cx,
                        crate::connections::capsule::env_word_id(Some(
                            hacivat_proto::EnvTag::Staging,
                        )),
                        hacivat_l10n::NoArgs,
                    )
                    .to_string(),
                ),
            },
        )),
        _ => None,
    };
    let swatch = matches!(
        texts.env,
        Some(hacivat_proto::EnvTag::Production | hacivat_proto::EnvTag::Staging)
    );
    let widths = |short: bool| -> Vec<f32> {
        let cap = if short { SHORT_TITLE_PX } else { TITLE_PX };
        texts
            .titles
            .iter()
            .enumerate()
            .map(|(ix, title)| {
                let f = texts.facts.get(ix).copied().unwrap_or_default();
                let mut w = TAB_CHROME_PX + TAB_GAP_PX + px(title).min(cap);
                if let Some(n) = f.repeat {
                    w += TAB_GAP_PX + px(&toolbar::repeat_text(n, cx));
                }
                if f.pinned {
                    w += TAB_GAP_PX + TAB_GLYPH_PX;
                }
                if f.untyped {
                    w += TAB_GAP_PX + px(if short { &untyped_short } else { &untyped });
                }
                if swatch {
                    w += TAB_GAP_PX + TAB_SWATCH_PX;
                }
                if let Some(staging) = &staging {
                    w += TAB_GAP_PX + px(staging);
                }
                w
            })
            .collect()
    };
    let full = widths(false);
    if output_w + full.iter().sum::<f32>() <= avail {
        return (false, None);
    }
    let short = widths(true);
    if output_w + short.iter().sum::<f32>() <= avail {
        return (true, None);
    }
    let more = toolbar::more_text(texts.titles.len(), cx);
    let budget = avail - output_w - (TAB_CHROME_PX - TAB_GLYPH_PX + px(&more));
    let mut drawn = Vec::new();
    let mut used = 0.0;
    let active = match active {
        ActiveResult::Tab(ix) if ix < short.len() => Some(ix),
        _ => None,
    };
    if let Some(a) = active {
        drawn.push(a);
        used += short[a];
    }
    for (ix, w) in short.iter().enumerate() {
        if Some(ix) == active {
            continue;
        }
        if used + w > budget {
            break;
        }
        drawn.push(ix);
        used += w;
    }
    drawn.sort_unstable();
    (true, Some(drawn))
}

/// The "+N" menu: every tab the strip left out, by its whole title; choosing one shows it.
fn open_more_menu(
    console: ConsoleId,
    hidden: Vec<(usize, SharedString)>,
    at: hacivat_ui_core::gpui_surface::Point<hacivat_ui_core::gpui_surface::Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    let items: Vec<hacivat_ui::menu::MenuItemSpec> = hidden
        .iter()
        .map(|(ix, name)| {
            let name = name.clone();
            hacivat_ui::menu::MenuItemSpec::new(
                Rc::new(move |_: &App| name.clone()),
                format!("{MORE_ACTION}{ix}"),
            )
        })
        .collect();
    // Kept inside the window.
    let room = window.viewport_size().width - hacivat_ui_core::gpui_surface::px(TAB_MENU_PX);
    let at = hacivat_ui_core::gpui_surface::point(at.x.min(room), at.y);
    let menu = ws.update(cx, |ws, cx| ws.open_popup_menu(at, items, window, cx));
    menu.update(cx, |m: &mut hacivat_ui::menu::PopupMenu, _| {
        m.set_access_keys(false);
    });
    cx.subscribe(
        &menu,
        move |_, event: &hacivat_ui::menu::PopupMenuEvent, cx| {
            if let hacivat_ui::menu::PopupMenuEvent::Chosen(action) = event
                && let Some(ix) = action
                    .strip_prefix(MORE_ACTION)
                    .and_then(|ix| ix.parse::<usize>().ok())
                && let Some(results) = Consoles::get(cx, console).and_then(|s| s.results.clone())
            {
                results.update(cx, |r, cx| r.set_active(ActiveResult::Tab(ix), cx));
            }
        },
    )
    .detach();
}

/// The "+N" menu's action IDs (answered by its subscription; no command has them).
const MORE_ACTION: &str = "results.tabs_more.";

/// Fetch all on tab `ix` of `results`: at once, unless its exact count says the rows would
/// pass the result's memory limit (R-M3-68); then the question first, whose "Fetch up to the
/// memory limit" fetches until the limit pauses the result and keeps its cursor open.
pub fn fetch_all_or_ask(
    results: &Entity<ConsoleResults>,
    ix: usize,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(entry) = results.read(cx).tab(ix) else {
        return;
    };
    let source = entry.source();
    if !fetch_all_passes_cap(entry) {
        source.fetch_all();
        return;
    }
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    let size = crate::results::adapter::Texts::of(cx).size(hacivat_results::budget::PER_RESULT_CAP);
    let button = |label, kind| hacivat_ui::dialog::DialogButton {
        label,
        kind,
        accesskey: None,
        english_accesskey: None,
    };
    let spec = hacivat_ui::dialog::DialogSpec {
        title: crate::l10n_global::text_fn(ids::hv::FETCHALL_TITLE),
        body: Rc::new(move |cx: &App| {
            tr(
                cx,
                ids::hv::FETCHALL_BODY,
                ids::hv::HvFetchallBody {
                    size: Text(size.clone()),
                },
            )
        }),
        buttons: vec![
            button(
                crate::l10n_global::text_fn(ids::hv::FETCHALL_CANCEL),
                hacivat_ui::button::ButtonKind::Default,
            ),
            button(
                crate::l10n_global::text_fn(ids::hv::FETCHALL_OK),
                hacivat_ui::button::ButtonKind::Primary,
            ),
        ],
        cancel: 0,
        // The bounded fetch is the safe choice: it has the focus (NOTES surface 10).
        primary: 1,
        size: hacivat_ui::dialog::DialogSize::Wide,
    };
    let dialog = ws.update(cx, |ws, cx| ws.open_dialog(spec, window, cx));
    cx.subscribe(
        &dialog,
        move |_, event: &hacivat_ui::dialog::DialogEvent, _| {
            if *event == hacivat_ui::dialog::DialogEvent::Pressed(1) {
                source.fetch_all();
            }
        },
    )
    .detach();
}

/// Whether fetching all of `entry`'s rows would pass its result's memory limit: its exact
/// count, at the bytes per row it holds so far, is over the per-result cap (never once "Fetch
/// more anyway" raised the cap, nor while nothing is known of the total).
pub fn fetch_all_passes_cap(entry: &ResultEntry) -> bool {
    let CountState::Exact(total) = entry.count() else {
        return false;
    };
    let source = entry.source();
    if source.cap_raised() {
        return false;
    }
    let snapshot = source.snapshot();
    let (rows, bytes) = (snapshot.row_count(), snapshot.bytes());
    if rows == 0 || total <= rows {
        return false;
    }
    let projected = u128::from(bytes) * u128::from(total) / u128::from(rows);
    projected > u128::from(hacivat_results::budget::PER_RESULT_CAP)
}

// ## m3b-t01b-seams
impl Focusable for ResultsHost {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ResultsHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        self.prune(cx);
        let Some((console, results)) = console_in(window, cx) else {
            self.shown = None;
            self.guard_focus(None, window, cx);
            return crate::shell::tiles::results(&theme, cx).into_any_element();
        };
        self.shown = Some(console);
        self.ensure_tile(console, &results, window, cx);
        let ran =
            self.tiles.get(&console).is_some_and(|t| t.ran) || !results.read(cx).tabs().is_empty();
        if !ran {
            // A console that never ran keeps the M1 tile.
            self.guard_focus(None, window, cx);
            return crate::shell::tiles::results(&theme, cx).into_any_element();
        }
        self.tile_element(console, window, cx)
            .unwrap_or_else(|| crate::shell::tiles::results(&theme, cx).into_any_element())
    }
}

/// The `results_view` hook (a `ViewBuilder`): the results tile, built once per window.
pub fn build(window: &mut Window, cx: &mut App) -> AnyView {
    let _ = window;
    let host = cx.new(ResultsHost::new);
    cx.default_global::<ResultsHosts>().0.push(host.downgrade());
    // ## m3b-t01b-seams (P-M3b-21): `Results` joins the context stack while the tile root has
    // the focus.
    let focus = host.read(cx).focus.clone();
    crate::shell::register_focus_context(cx, focus, super::RESULTS_CONTEXT);
    host.into()
}

// ---- commands -------------------------------------------------------------------------------

/// The palette title of each grid command (`hv-cmd-<id with . and _ as ->`, R5).
static GRID_TITLES: [(&str, &str); 35] = [
    ("grid.move_left", "hv-cmd-grid-move-left"),
    ("grid.move_right", "hv-cmd-grid-move-right"),
    ("grid.move_up", "hv-cmd-grid-move-up"),
    ("grid.move_down", "hv-cmd-grid-move-down"),
    ("grid.select_left", "hv-cmd-grid-select-left"),
    ("grid.select_right", "hv-cmd-grid-select-right"),
    ("grid.select_up", "hv-cmd-grid-select-up"),
    ("grid.select_down", "hv-cmd-grid-select-down"),
    ("grid.jump_left", "hv-cmd-grid-jump-left"),
    ("grid.jump_right", "hv-cmd-grid-jump-right"),
    ("grid.jump_up", "hv-cmd-grid-jump-up"),
    ("grid.jump_down", "hv-cmd-grid-jump-down"),
    ("grid.home", "hv-cmd-grid-home"),
    ("grid.end", "hv-cmd-grid-end"),
    ("grid.first_row", "hv-cmd-grid-first-row"),
    ("grid.last_row", "hv-cmd-grid-last-row"),
    ("grid.page_up", "hv-cmd-grid-page-up"),
    ("grid.page_down", "hv-cmd-grid-page-down"),
    ("grid.next_cell", "hv-cmd-grid-next-cell"),
    ("grid.prev_cell", "hv-cmd-grid-prev-cell"),
    ("grid.select_all", "hv-cmd-grid-select-all"),
    ("grid.select_row", "hv-cmd-grid-select-row"),
    ("grid.select_column", "hv-cmd-grid-select-column"),
    ("grid.copy", "hv-cmd-grid-copy"),
    ("grid.copy_with_headers", "hv-cmd-grid-copy-with-headers"),
    ("grid.escape", "hv-cmd-grid-escape"),
    // ## m3b-t01b-seams
    ("grid.grow_selection", "hv-cmd-grid-grow-selection"),
    ("grid.shrink_selection", "hv-cmd-grid-shrink-selection"),
    ("grid.find", "hv-cmd-grid-find"),
    ("grid.goto_row", "hv-cmd-grid-goto-row"),
    ("grid.open_value", "hv-cmd-grid-open-value"),
    ("grid.context_menu", "hv-cmd-grid-context-menu"),
    ("grid.auto_fit", "hv-cmd-grid-auto-fit"),
    ("grid.freeze", "hv-cmd-grid-freeze"),
    ("grid.unfreeze", "hv-cmd-grid-unfreeze"),
];

/// The palette title of grid command `id`.
pub fn grid_title(id: &str) -> Option<&'static str> {
    GRID_TITLES
        .iter()
        .find(|(name, _)| *name == id)
        .map(|(_, title)| *title)
}

/// The console the shown tile follows, with its results (the first window's tile: the
/// `enabled` checks get no window).
pub(crate) fn shown_results(cx: &App) -> Option<(ConsoleId, Entity<ConsoleResults>)> {
    hosts(cx).into_iter().find_map(|h| {
        let console = h.read(cx).shown?;
        Some((console, Consoles::get(cx, console)?.results.clone()?))
    })
}

// ## m3a-t30 fix round 1 (C1, M7): a command acts only on what `window` shows: the active
// result tab of the focused pane's active console. A grid the tile keeps for another tab or
// another console never answers, even when the focus was left on it (GPUI keeps a focus on an
// element that stopped rendering, and the palette restores it).

/// The active result tab of the console `window` shows, with its index.
fn active_entry_in(
    window: &Window,
    cx: &App,
) -> Option<(ConsoleId, Entity<ConsoleResults>, usize)> {
    let (console, results) = console_in(window, cx)?;
    let ActiveResult::Tab(ix) = results.read(cx).active() else {
        return None;
    };
    results.read(cx).tab(ix)?;
    Some((console, results, ix))
}

/// The grid of the active result tab of the console `window` shows.
fn active_grid(window: &Window, cx: &App) -> Option<Entity<GridView>> {
    let (console, _, ix) = active_entry_in(window, cx)?;
    hosts(cx)
        .into_iter()
        .find_map(|h| h.read(cx).grid_of(console, ix, cx))
}

/// Runs `cmd` on the grid `window` shows.
fn dispatch_grid(cmd: GridCommand, window: &mut Window, cx: &mut App) {
    if let Some(grid) = active_grid(window, cx) {
        grid.update(cx, |g, cx| g.dispatch(cmd, window, cx));
    }
}

/// The active result of the shown console, with its index (the `enabled` checks get no
/// window: with several windows they read the first live tile; the run bodies use their own
/// window's console).
fn active_entry(cx: &App) -> Option<(ConsoleId, Entity<ConsoleResults>, usize)> {
    let (console, results) = shown_results(cx)?;
    let ActiveResult::Tab(ix) = results.read(cx).active() else {
        return None;
    };
    results.read(cx).tab(ix)?;
    Some((console, results, ix))
}

/// Whether the active result's cursor can fetch: open and not paused at a memory limit (M8:
/// there only the banner's "Fetch more anyway" fetches on).
fn active_can_fetch(cx: &App) -> bool {
    active_entry(cx).is_some_and(|(_, results, ix)| {
        results.read(cx).tab(ix).is_some_and(|t| {
            let snapshot = t.adapter().snapshot();
            matches!(snapshot.completeness(), Completeness::Suspended(_))
                && snapshot.paused().is_none()
        })
    })
}

/// Whether the active result has an open cursor.
fn active_has_cursor(cx: &App) -> bool {
    active_entry(cx).is_some_and(|(_, results, ix)| {
        results.read(cx).tab(ix).is_some_and(|t| {
            matches!(
                t.adapter().snapshot().completeness(),
                Completeness::Suspended(_)
            )
        })
    })
}

/// Runs `f` on the source of the active result `window` shows.
fn with_active_source(
    window: &Window,
    cx: &mut App,
    f: impl FnOnce(&dyn hacivat_results::ResultSource),
) {
    if let Some((_, results, ix)) = active_entry_in(window, cx)
        && let Some(tab) = results.read(cx).tab(ix)
    {
        f(&*tab.source());
    }
}

/// Registers the results tile's commands as `ActionDef`s (Task 30: `results.copy`,
/// `results.fetch_more`, `results.fetch_all`, `results.close_cursor`, `results.rerun`, and
/// every `grid.*` id of `hacivat_ui_grid::GRID_COMMANDS`, dispatching to the focused
/// `GridView`). Called once from `results::install`.
pub fn register_commands(cx: &mut App) {
    let reg = cx.default_global::<ActionRegistry>();
    for (id, cmd) in GRID_COMMANDS.iter() {
        let cmd = *cmd;
        let _ = reg.register(ActionDef {
            id,
            title_id: grid_title(id).unwrap_or(id),
            context: GRID_CONTEXT,
            run: Rc::new(move |window, cx| dispatch_grid(cmd, window, cx)),
            // ## m3b-t01b-seams (fix round 2, I3): a stub grid command stays disabled.
            enabled: if hacivat_ui_grid::commands::is_stub(id) {
                |_| false
            } else {
                |_| true
            },
        });
    }
    let _ = reg.register(ActionDef {
        id: "results.copy",
        title_id: ids::hv::CMD_RESULTS_COPY.id,
        context: "Workspace",
        run: Rc::new(|window, cx| dispatch_grid(GridCommand::Copy { headers: false }, window, cx)),
        enabled: |cx| active_entry(cx).is_some(),
    });
    let _ = reg.register(ActionDef {
        id: "results.fetch_more",
        title_id: ids::hv::CMD_RESULTS_FETCH_MORE.id,
        context: "Workspace",
        run: Rc::new(|window, cx| {
            with_active_source(window, cx, |s| s.ensure_rows(s.snapshot().row_count()));
        }),
        enabled: active_can_fetch,
    });
    let _ = reg.register(ActionDef {
        id: "results.fetch_all",
        title_id: ids::hv::CMD_RESULTS_FETCH_ALL.id,
        context: "Workspace",
        // ## m3b-t25-results: asks first when the rows would pass the memory limit.
        run: Rc::new(|window, cx| {
            if let Some((_, results, ix)) = active_entry_in(window, cx) {
                fetch_all_or_ask(&results, ix, window, cx);
            }
        }),
        enabled: active_can_fetch,
    });
    let _ = reg.register(ActionDef {
        id: "results.close_cursor",
        title_id: ids::hv::CMD_RESULTS_CLOSE_CURSOR.id,
        context: "Workspace",
        run: Rc::new(|window, cx| with_active_source(window, cx, |s| s.close_cursor())),
        enabled: active_has_cursor,
    });
    let _ = reg.register(ActionDef {
        id: "results.rerun",
        title_id: ids::hv::CMD_RESULTS_RERUN.id,
        context: "Workspace",
        run: Rc::new(|window, cx| {
            let Some((console, results, ix)) = active_entry_in(window, cx) else {
                return;
            };
            let Some(unit_ordinal) = results.read(cx).tab(ix).map(|t| t.key().unit_ordinal) else {
                return;
            };
            results.update(cx, |_, cx| {
                cx.emit(ResultsEvent::Rerun {
                    console,
                    unit_ordinal,
                })
            });
        }),
        enabled: |cx| {
            active_entry(cx).is_some_and(|(_, results, ix)| {
                results.read(cx).tab(ix).is_some_and(|t| {
                    matches!(
                        t.adapter().snapshot().completeness(),
                        Completeness::Closed { .. } | Completeness::Cancelled
                    )
                })
            })
        },
    });
}

// ## m3a-t30-results-view: shots fixtures for surfaces 9–12 (Task 35 renders them).

#[cfg(feature = "shots")]
pub use fixtures::*;

#[cfg(feature = "shots")]
mod fixtures {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use std::rc::Rc;

    use hacivat_driver::NoticeSeverity;
    use hacivat_exec::{ErrorView, LogMsg, ResultTab, SessionLog, TabKey};
    use hacivat_proto::ProfileId;
    use hacivat_results::{
        CloseReason, Completeness, CursorInfo, FetchControl, FetchPolicy, PauseReason, ResultId,
        ResultStore, StoreSource, TruncReason,
    };
    use hacivat_session::session::{ClientId, DenyLayer, RunId, RunSummary, UnitId};
    use hacivat_ui_core::gpui_surface::{App, AppContext as _, Window};
    use hacivat_values::{ColumnMeta, LogicalType, NativeType, ResultSchema, Value};

    use crate::console::{ConsoleId, ConsoleState, Consoles};
    use crate::results::tabs::{ActiveResult, ConsoleResults, CountState};
    use crate::shell::Workspace;

    /// A results-tile state for the shots (NOTES surfaces 9, 11, 12; surface 10 is the grid
    /// of [`ResultsShot::Rows`]).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum ResultsShot {
        /// First rows arriving: the header and static skeleton rows; "Fetching… 0 rows".
        FirstRows,
        /// Rows of several cell types, complete.
        Rows,
        /// A result with no rows.
        ZeroRows,
        /// An update count only: the Output tab, active, with its line.
        UpdateCount,
        /// An error: the Output tab with the error line and the server's text.
        Error,
        /// Two units: "Result 1", "Result 2"; the Output tab carries the unread dot.
        TwoResults,
        /// More than 2,000 columns: the hint row.
        WideColumns,
        /// The strip while rows arrive.
        Streaming,
        /// The strip of an open cursor.
        Suspended,
        /// The strip of a cursor closed for `reason`, Hacivat's transaction rolled back.
        Closed(CloseReason),
        /// The strip of a cancelled result.
        Cancelled,
        /// The memory banner (R-M3-33).
        Memory,
        /// "2 more result sets not shown" (R-M3-25).
        MoreResultSets,
        /// A run's log: rows, a commit, a summary, a cancel, a read-only denial.
        RunLog,
        /// "n events not shown" at the top of the log.
        EventsDropped,
        // ## m3b-t25-results: design/m3b-safe surface 10.
        /// Comment titles, a `(2)` repeat, `Result N`, and a long pinned title.
        TitledTabs,
        /// A result run as one: "untyped (run as one)".
        Untyped,
        /// "The first 100 result sets are shown; 37 more were read and not kept."
        SetLimit,
        /// Count rows while it counts: the amber marker, "Counting…", Cancel.
        Counting,
        /// Count rows done: "1,048,213 rows (exact)".
        CountExact,
        /// "Closed to free memory; the first 4,096 rows are kept." and Re-run.
        Evicted,
        /// The fetch-all question over a result whose count passes its memory limit.
        FetchAll,
        /// PostgreSQL notices and a warning, the dropped-notices line.
        Notices,
        /// MySQL warnings and an error whose chip shows `1792 · 25006`.
        MysqlWarnings,
        /// Fix round 1 (I1): 30 tabs on a production console; the active one far along; the
        /// tabs that do not fit behind "+N".
        CrowdedTabs,
    }

    /// The Task 25 states (design/m3b-safe surface 10).
    pub const T25_SHOTS: [ResultsShot; 10] = [
        ResultsShot::TitledTabs,
        ResultsShot::Untyped,
        ResultsShot::SetLimit,
        ResultsShot::Counting,
        ResultsShot::CountExact,
        ResultsShot::Evicted,
        ResultsShot::FetchAll,
        ResultsShot::Notices,
        ResultsShot::MysqlWarnings,
        ResultsShot::CrowdedTabs,
    ];

    /// A long comment title: the tab ellipsizes it, the tooltip keeps it whole.
    const LONG_TITLE: &str = "Aylık satış raporu: bölge ve kanal kırılımıyla, iadeler hariç";

    /// Nobody listens: the fixtures' cursors are never fetched.
    struct Inert;

    impl FetchControl for Inert {
        fn fetch_more(&self, _: ResultId, _: u64) {}
        fn close(&self, _: ResultId, _: CloseReason) {}
        fn fetch_all(&self, _: ResultId) {}
    }

    fn column(name: &str, logical: LogicalType, native: &str) -> ColumnMeta {
        ColumnMeta {
            name: name.into(),
            logical,
            native: NativeType::Pg {
                oid: 0,
                typmod: -1,
                name: native.into(),
            },
            origin: None,
        }
    }

    fn int8() -> LogicalType {
        LogicalType::Int {
            bits: 64,
            signed: true,
        }
    }

    fn text() -> LogicalType {
        LogicalType::Text {
            max_chars: None,
            fixed: false,
        }
    }

    /// `id`, `customer`, `note`, `paid`, `token`: an ID column, Turkish text, NULL and `''`,
    /// booleans and UUIDs.
    fn orders(n: u64) -> (Arc<ResultSchema>, Vec<Vec<Value>>) {
        let schema = Arc::new(ResultSchema {
            columns: vec![
                column("id", int8(), "int8"),
                column("customer", text(), "text"),
                column("note", text(), "text"),
                column("paid", LogicalType::Bool, "bool"),
                column("token", LogicalType::Uuid, "uuid"),
            ]
            .into(),
        });
        let names = ["Çağla Öztürk", "İsmail Işık", "Ayşe Gül", "Ümit Şahin"];
        let rows = (0..n)
            .map(|r| {
                let note = match r % 4 {
                    0 => Value::Null,
                    1 => Value::Text("".into()),
                    2 => Value::Text("NULL".into()),
                    _ => Value::Text("teslimat cuma günü, kapıda ödeme".into()),
                };
                let mut token = [0u8; 16];
                token[..8].copy_from_slice(&r.to_be_bytes());
                vec![
                    Value::Int(1_000 + r as i64),
                    Value::Text(names[(r % 4) as usize].into()),
                    note,
                    Value::Bool(r % 3 == 0),
                    Value::Uuid(token),
                ]
            })
            .collect();
        (schema, rows)
    }

    fn cursor() -> CursorInfo {
        let now = Instant::now();
        CursorInfo {
            opened_at: now,
            last_fetch: now,
            holds_tx: true,
            idle_bound: Duration::from_secs(600),
        }
    }

    fn unit(ordinal: u32) -> UnitId {
        UnitId {
            run: RunId(1),
            ordinal,
        }
    }

    fn tab(ordinal: u32, store: Arc<ResultStore>) -> ResultTab {
        ResultTab {
            key: TabKey {
                console: 0,
                unit_ordinal: ordinal,
                result_ordinal: 0,
            },
            title: "".into(),
            unit: unit(ordinal),
            source: StoreSource::new(store, Arc::new(Inert), FetchPolicy::default()),
        }
    }

    fn store(n: u64, completeness: Completeness) -> Arc<ResultStore> {
        let (schema, rows) = orders(n);
        ResultStore::from_rows(schema, rows, completeness)
    }

    fn error_view() -> ErrorView {
        ErrorView {
            title_id: "exec-error-server",
            code: Some("42P01".into()),
            server_text: Some("relation \"order_lines\" does not exist".into()),
            hint_id: None,
            doc_offset: None,
            // ## m3b-t25-results (named exception)
            vendor_code: None,
        }
    }

    // ## m3b-t25-results

    /// Result set `set` of unit `ordinal`, titled `title` by the run service.
    fn titled(ordinal: u32, set: u32, title: &str, store: Arc<ResultStore>) -> ResultTab {
        ResultTab {
            key: TabKey {
                console: 0,
                unit_ordinal: ordinal,
                result_ordinal: set,
            },
            title: title.into(),
            unit: unit(ordinal),
            source: StoreSource::new(store, Arc::new(Inert), FetchPolicy::default()),
        }
    }

    /// The orders columns as a result run as one sends them: text of no known native type.
    fn untyped() -> Arc<ResultSchema> {
        let (schema, _) = orders(0);
        Arc::new(ResultSchema {
            columns: schema
                .columns
                .iter()
                .map(|c| ColumnMeta {
                    name: c.name.clone(),
                    logical: text(),
                    native: NativeType::Unknown,
                    origin: None,
                })
                .collect(),
        })
    }

    /// `rows` as the server's text.
    fn as_text(rows: Vec<Vec<Value>>) -> Vec<Vec<Value>> {
        rows.into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|v| match v {
                        Value::Int(n) => Value::Text(n.to_string().into()),
                        Value::Bool(b) => Value::Text(if b { "t" } else { "f" }.into()),
                        Value::Uuid(u) => Value::Text(
                            u.iter()
                                .map(|b| format!("{b:02x}"))
                                .collect::<String>()
                                .into(),
                        ),
                        other => other,
                    })
                    .collect()
            })
            .collect()
    }

    /// Opens a console tab bound to `profile` in the focused pane, active, and fills its
    /// results and log as `shot` says. The console's ID.
    pub fn open_results_shot(
        shot: ResultsShot,
        profile: Option<ProfileId>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<ConsoleId> {
        let ws = window.root::<Workspace>().flatten()?;
        let tab_id = ws.update(cx, |ws, cx| ws.open_console_tab(profile, window, cx))?;
        let log = SessionLog::new();
        let results = cx.new(|_| ConsoleResults::new(log.clone()));
        let console = Consoles::register(
            cx,
            ConsoleState {
                id: ConsoleId(0),
                tab: tab_id,
                n: 1,
                profile,
                client: ClientId::DEFAULT,
                session: None,
                editor: None,
                results: Some(results.clone()),
                sql: None,
                log: log.clone(),
                service: None,
            },
        );
        let line = |msg: LogMsg, ordinal: Option<u32>| log.append(msg, None, ordinal.map(unit));
        let mut tabs = Vec::new();
        match shot {
            ResultsShot::FirstRows => tabs.push(tab(1, store(0, Completeness::Streaming))),
            ResultsShot::Rows => tabs.push(tab(1, store(40, Completeness::Complete))),
            ResultsShot::ZeroRows => tabs.push(tab(1, store(0, Completeness::Complete))),
            ResultsShot::UpdateCount => line(LogMsg::RowsAffected { rows: 12, ms: 38 }, Some(1)),
            ResultsShot::Error => log.append(
                LogMsg::Error {
                    n: 3,
                    view: error_view(),
                },
                Some(0..10),
                Some(unit(3)),
            ),
            ResultsShot::TwoResults => {
                tabs.push(tab(1, store(3, Completeness::Complete)));
                tabs.push(tab(2, store(12, Completeness::Complete)));
            }
            ResultsShot::WideColumns => {
                let schema = Arc::new(ResultSchema {
                    columns: (0..2_412)
                        .map(|c| column(&format!("c{c}"), int8(), "int8"))
                        .collect(),
                });
                let rows = (0..3)
                    .map(|r| (0..2_412).map(|c| Value::Int(r * 10_000 + c)).collect())
                    .collect();
                tabs.push(tab(
                    1,
                    ResultStore::from_rows(schema, rows, Completeness::Complete),
                ));
            }
            ResultsShot::Streaming => tabs.push(tab(1, store(35, Completeness::Streaming))),
            ResultsShot::Suspended => {
                tabs.push(tab(1, store(100, Completeness::Suspended(cursor()))))
            }
            ResultsShot::Closed(reason) => {
                line(
                    LogMsg::CursorClosed {
                        rows: 100,
                        reason,
                        rolled_back: true,
                        idle_ms: 600_000,
                    },
                    Some(1),
                );
                tabs.push(tab(1, store(100, Completeness::Closed { reason })));
            }
            ResultsShot::Cancelled => tabs.push(tab(1, store(35, Completeness::Cancelled))),
            ResultsShot::Memory => {
                let (schema, rows) = orders(100);
                tabs.push(tab(
                    1,
                    ResultStore::from_rows_paused(schema, rows, cursor(), PauseReason::ResultCap),
                ));
            }
            ResultsShot::MoreResultSets => {
                line(LogMsg::MoreResultSets { skipped: 2 }, Some(1));
                tabs.push(tab(
                    1,
                    store(8, Completeness::Truncated(TruncReason::FollowingResultSets)),
                ));
            }
            ResultsShot::RunLog => {
                line(
                    LogMsg::RowsRetrieved {
                        rows: 10_000,
                        more: true,
                        exec_ms: 131,
                        fetch_ms: 40,
                    },
                    Some(1),
                );
                line(LogMsg::RowsAffected { rows: 3, ms: 12 }, Some(2));
                line(LogMsg::TxCommitted, None);
                line(LogMsg::Cancelled { ms: 2_340 }, Some(3));
                line(LogMsg::ReadOnlyDenied(DenyLayer::Input), Some(4));
                line(
                    LogMsg::Summary(RunSummary {
                        executed: 2,
                        total: 6,
                        failed: 1,
                        duration_ms: 1_020,
                        ..RunSummary::default()
                    }),
                    None,
                );
            }
            ResultsShot::EventsDropped => {
                for ms in 0..(SessionLog::CAPACITY as u64 + 312) {
                    line(LogMsg::Completed { ms }, None);
                }
            }
            // ## m3b-t25-results: design/m3b-safe surface 10.
            ResultsShot::TitledTabs => {
                tabs.push(titled(
                    1,
                    0,
                    "Şehre göre ciro",
                    store(40, Completeness::Complete),
                ));
                tabs.push(titled(
                    1,
                    1,
                    "Şehre göre ciro (2)",
                    store(6, Completeness::Complete),
                ));
                tabs.push(titled(2, 0, "Result 2", store(3, Completeness::Complete)));
                tabs.push(titled(3, 0, LONG_TITLE, store(12, Completeness::Complete)));
            }
            ResultsShot::Untyped => {
                let (_, rows) = orders(40);
                tabs.push(tab(
                    1,
                    ResultStore::from_rows(untyped(), as_text(rows), Completeness::Complete),
                ));
            }
            ResultsShot::SetLimit => {
                line(
                    LogMsg::ResultSetLimit {
                        total: 137,
                        limit: 100,
                    },
                    Some(1),
                );
                tabs.push(tab(1, store(40, Completeness::Complete)));
            }
            ResultsShot::Counting | ResultsShot::CountExact => {
                tabs.push(tab(1, store(500, Completeness::Suspended(cursor()))));
            }
            ResultsShot::Evicted => {
                tabs.push(tab(
                    1,
                    store(
                        4_096,
                        Completeness::Closed {
                            reason: CloseReason::Evicted,
                        },
                    ),
                ));
            }
            ResultsShot::FetchAll => {
                tabs.push(tab(1, store(500, Completeness::Suspended(cursor()))));
            }
            ResultsShot::Notices => {
                let notice = |severity, code: Option<&str>, text: &str| LogMsg::Notice {
                    severity,
                    code: code.map(Arc::from),
                    text: Arc::from(text),
                };
                line(
                    notice(
                        NoticeSeverity::Notice,
                        Some("00000"),
                        "relation \"siparişler_arşiv\" already exists, skipping",
                    ),
                    Some(1),
                );
                line(
                    notice(
                        NoticeSeverity::Warning,
                        Some("01000"),
                        "there is no transaction in progress",
                    ),
                    Some(1),
                );
                line(
                    notice(NoticeSeverity::Info, None, "vacuuming \"public.orders\""),
                    Some(1),
                );
                line(LogMsg::NoticesDropped { n: 97 }, Some(1));
                line(LogMsg::Completed { ms: 41 }, Some(1));
            }
            ResultsShot::CrowdedTabs => {
                for u in 0..30u32 {
                    let title = match u % 3 {
                        0 => format!("Bölge {u} satışları"),
                        1 => "Şehre göre ciro".to_string(),
                        _ => String::new(),
                    };
                    tabs.push(titled(
                        u,
                        0,
                        &title,
                        store(3 + u64::from(u), Completeness::Complete),
                    ));
                }
            }
            ResultsShot::MysqlWarnings => {
                line(
                    LogMsg::Notice {
                        severity: NoticeSeverity::Warning,
                        code: Some(Arc::from("1292")),
                        text: Arc::from("Truncated incorrect DOUBLE value: 'İzmir'"),
                    },
                    Some(1),
                );
                line(
                    LogMsg::Notice {
                        severity: NoticeSeverity::Notice,
                        code: Some(Arc::from("1051")),
                        text: Arc::from("Unknown table 'shop.siparis_tmp'"),
                    },
                    Some(1),
                );
                log.append(
                    LogMsg::Error {
                        n: 2,
                        view: ErrorView {
                            title_id: "exec-error-server",
                            code: Some("25006".into()),
                            server_text: Some(
                                "Cannot execute statement in a READ ONLY transaction.".into(),
                            ),
                            hint_id: None,
                            doc_offset: None,
                            vendor_code: Some("1792".into()),
                        },
                    },
                    Some(0..10),
                    Some(unit(2)),
                );
            }
        }
        if !tabs.is_empty() {
            results.update(cx, |r, cx| {
                r.apply(&tabs, cx);
            });
        }
        // ## m3b-t25-results
        match shot {
            ResultsShot::TitledTabs => {
                results.update(cx, |r, cx| r.set_pinned(3, true, cx));
            }
            ResultsShot::CrowdedTabs => {
                results.update(cx, |r, cx| {
                    r.set_pinned(1, true, cx);
                    r.set_active(ActiveResult::Tab(20), cx);
                });
            }
            ResultsShot::Counting => {
                results.update(cx, |r, cx| r.start_count(0, Rc::new(|| {}), cx));
            }
            ResultsShot::CountExact => {
                results.update(cx, |r, cx| r.set_count(0, CountState::Exact(1_048_213), cx));
            }
            ResultsShot::FetchAll => {
                results.update(cx, |r, cx| {
                    r.set_count(0, CountState::Exact(40_000_000), cx)
                });
                super::fetch_all_or_ask(&results, 0, window, cx);
            }
            _ => {}
        }
        if shot == ResultsShot::TwoResults {
            line(LogMsg::Completed { ms: 4 }, None);
            results.update(cx, |r, cx| r.note_output(cx));
        }
        if matches!(shot, ResultsShot::UpdateCount | ResultsShot::Error) {
            results.update(cx, |r, cx| r.set_active(ActiveResult::Output, cx));
        }
        Some(console)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_grid_command_has_its_title() {
        for (id, _) in GRID_COMMANDS.iter() {
            let title = grid_title(id).unwrap_or_else(|| panic!("{id} has no title"));
            assert_eq!(
                title,
                format!("hv-cmd-{}", id.replace(['.', '_'], "-")),
                "{id}"
            );
        }
        assert_eq!(GRID_TITLES.len(), GRID_COMMANDS.len());
    }
}
