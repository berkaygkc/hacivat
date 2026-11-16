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

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use hacivat_exec::{LogLine, LogMsg, SessionLog, TabKey, copy_invariant};
use hacivat_l10n::{Count, ExactDecimal, Text, ids};
use hacivat_results::{Completeness, PauseReason, StoreSnapshot};
use hacivat_session::session::UnitId;
use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui::popover::Declare as _;
use hacivat_ui::text::TextFn;
use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, AppContext as _, ClipboardItem, Context, Entity, EventEmitter,
    Focusable, Global, InteractiveElement, IntoElement, ParentElement, Render, Role, SharedString,
    StyleRefinement, Styled, Subscription, Task, WeakEntity, Window, div,
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
use crate::results::tabs::{ActiveResult, ConsoleResults, MemoryBanner, ResultEntry, result_title};
use crate::results::toolbar::{self, OnActivate, TabsModel, WIDE_RESULT_COLUMNS};
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
    facts: HashMap<TabKey, (UnitId, StripFacts)>,
    /// The newest log line seen (I5: a line of a run without a tab shows Output).
    seen_seq: Option<u64>,
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
fn console_in(window: &Window, cx: &App) -> Option<(ConsoleId, Entity<ConsoleResults>)> {
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
        },
        a11y_label: Rc::new(move |cx: &App| result_title(number, cx)),
    }
}

/// The rows of `entry` in `snapshot` and their time, as the toolbar shows them ("10,000+ rows ·
/// 131 ms"). ## m3a-t30 fix round 1: Task 34's messages and rules, so the toolbar and the status
/// bar say the same thing: "+" while more can come (streaming, suspended, truncated), the time
/// is execution plus fetching, whole milliseconds under a second, else seconds with two
/// decimals (`hv-status-duration-*`). The snapshot is the frame's one load of the tab (M10).
fn rows_timing(entry: &ResultEntry, snapshot: &StoreSnapshot, cx: &App) -> SharedString {
    let rows = Count(snapshot.row_count());
    let more = matches!(
        snapshot.completeness(),
        Completeness::Streaming | Completeness::Suspended(_) | Completeness::Truncated(_)
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

/// The size (`rows × cols`) of `grid`'s selection over `loaded` rows, when it has one block.
fn selection_dims(grid: &GridView, loaded: u64) -> Option<(u64, u64)> {
    let selection = grid.selection();
    if selection.is_empty() {
        return None;
    }
    let region = selection
        .normalize_for_copy(&ViewMap::identity(loaded), grid.column_layout().visible())
        .ok()?;
    Some((region.row_count(), region.cols.len() as u64))
}

impl ResultsHost {
    fn new() -> Self {
        ResultsHost {
            shown: None,
            tiles: HashMap::new(),
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
        // The newest line, then following new ones.
        list.update(cx, |l, cx| l.scroll_to_end(cx));
        let mut changes = log.changes();
        let ran = *changes.borrow() > 0;
        let observe = cx.observe(results, |_, _, cx| cx.notify());
        // ## m3a-t30 fix round 1 (T33 merge obligation): what the tile asks of the run UI.
        let events = cx.subscribe_in(results, window, |_, _, event: &ResultsEvent, window, cx| {
            crate::exec_ui::handle_results_event(event, window, cx);
        });
        let seen_seq = log.lines().last().map(|l| l.seq);
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
                seen_seq,
                _observe: observe,
                _events: events,
                _log: task,
            },
        );
    }

    /// `console`'s log changed: the list takes its lines; the tabs' strip facts take what the
    /// new lines say; a line while another tab is active marks Output unread; a line of a run
    /// that opened no tab shows Output (I5).
    fn on_log(&mut self, console: ConsoleId, cx: &mut Context<Self>) {
        let Some(tile) = self.tiles.get_mut(&console) else {
            return;
        };
        tile.ran = true;
        let lines = tile.log.lines();
        let fresh: Vec<&LogLine> = lines
            .iter()
            .filter(|l| tile.seen_seq.is_none_or(|seen| l.seq > seen))
            .filter(|l| !matches!(l.msg, LogMsg::Dropped { .. }))
            .collect();
        if let Some(last) = lines.last() {
            tile.seen_seq = Some(tile.seen_seq.map_or(last.seq, |s| s.max(last.seq)));
        }
        let results = tile.results.read(cx);
        for entry in results.tabs() {
            let found = StripFacts::from_log(&lines, entry.unit());
            match tile.facts.get_mut(&entry.key()) {
                Some((unit, facts)) if *unit == entry.unit() => facts.merge(found),
                _ => {
                    tile.facts.insert(entry.key(), (entry.unit(), found));
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
        tile.list
            .update(cx, |l, cx| l.rows_mut(cx).set_lines(lines));
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
        match tile.facts.get(&entry_key) {
            Some((u, facts)) if *u == unit => *facts,
            _ => {
                let facts = StripFacts::from_log(tile.list.read(cx).rows().lines(), unit);
                tile.facts.insert(entry_key, (unit, facts));
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
        let (active, unread, titles) = {
            let r = results.read(cx);
            let titles = (0..r.tabs().len())
                .filter_map(|ix| r.title(ix, cx))
                .collect::<Vec<_>>();
            (r.active(), r.unread_output(), titles)
        };
        let env = Consoles::get(cx, console)
            .and_then(|s| s.profile)
            .and_then(|p| cx.try_global::<ConnectionsModel>()?.get(p))
            .and_then(|p| p.environment);
        let weak = results.downgrade();
        let on_activate: OnActivate = Rc::new(move |a, _, cx| {
            let _ = weak.update(cx, |r, cx| r.set_active(a, cx));
        });
        let strip_tabs = toolbar::tab_strip(
            &TabsModel {
                active,
                unread,
                results: titles,
                env,
            },
            on_activate,
            cx,
        );
        let mut column = div()
            .id("results-tile")
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
                let lines_empty = list.read(cx).rows().lines().is_empty();
                let shown = list.clone();
                let ends = list.clone();
                column = column
                    .child(toolbar::output_toolbar(
                        Rc::new(move |_, _| log.clear()),
                        Rc::new(move |_, cx| {
                            let text = copy_invariant(shown.read(cx).rows().lines());
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                        }),
                        Rc::new(move |_, cx| ends.update(cx, |l, cx| l.scroll_to_end(cx))),
                        cx,
                    ))
                    .child(
                        div()
                            .id("results-log")
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
                column = column.child(toolbar::result_toolbar(
                    rows,
                    Rc::new(move |window, cx| {
                        let _ = copy_grid.update(cx, |g, cx| {
                            g.dispatch(GridCommand::Copy { headers: false }, window, cx)
                        });
                    }),
                    cx,
                ));
                if columns > WIDE_RESULT_COLUMNS {
                    column = column.child(toolbar::columns_hint(columns, cx));
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
    let (a, b, d) = (adapter.clone(), adapter.clone(), adapter);
    let raise = results.downgrade();
    let emit = results.downgrade();
    StripActions {
        // The reason follows the state (`StoreSource::close_cursor`): `MemoryLimit` on the
        // banner, `UserClosed` otherwise.
        close_cursor: Rc::new(move |_, _| a.source().close_cursor()),
        fetch_more: Rc::new(move |_, _| {
            let source = b.source();
            source.ensure_rows(source.snapshot().row_count());
        }),
        fetch_all: Rc::new(move |_, _| d.source().fetch_all()),
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
    let host = cx.new(|_| ResultsHost::new());
    cx.default_global::<ResultsHosts>().0.push(host.downgrade());
    host.into()
}

// ---- commands -------------------------------------------------------------------------------

/// The palette title of each grid command (`hv-cmd-<id with . and _ as ->`, R5).
static GRID_TITLES: [(&str, &str); 26] = [
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
fn shown_results(cx: &App) -> Option<(ConsoleId, Entity<ConsoleResults>)> {
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
            enabled: |_| true,
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
        run: Rc::new(|window, cx| with_active_source(window, cx, |s| s.fetch_all())),
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
    use crate::results::tabs::{ActiveResult, ConsoleResults};
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
    }

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
        }
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
        }
        if !tabs.is_empty() {
            results.update(cx, |r, cx| {
                r.apply(&tabs, cx);
            });
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
