//! A console's result tabs (Task 26): Output first, then one tab per unit result keyed
//! `TabKey`; a re-run of the same key reuses its tab; titles `Result N`.
//!
//! - **Order.** The Output tab leads; result tabs follow in the order their keys first arrived.
//! - **Reuse (HV-EXEC-07 steps 2–3).** [`ConsoleResults::apply`] takes a run's tabs. A tab
//!   whose key the console already shows keeps its place, its number and its adapter: the
//!   adapter moves to the new result (`GridAdapter::retarget`, a new generation, so the grid
//!   clears its selection). The widths the tab kept stay when the new result's column names
//!   are the same in the same order, and are dropped otherwise.
//! - **Titles** are `Result N` (`hv-results-result-n`), N counting the console's result tabs
//!   from 1; a reused tab keeps its N. Comment titles and pinning are M3b.
//! - **Widths** live here per tab, so a tab's grid view can be rebuilt with them; the grid
//!   reports them (`GridEvent::ColumnResized`) and Task 30's tile records them with
//!   [`ConsoleResults::set_width`].
//!
//! - **The memory banner (R-M3-33).** A result that stopped at a memory limit is `Suspended`
//!   with `StoreSnapshot::paused()` set; [`ResultEntry::memory_banner`] is that state for the
//!   cursor strip. "Fetch more anyway" goes through [`ConsoleResults::raise_cap`], which
//!   remembers the raise: once the cap is raised, a pause for the global budget has nothing
//!   further to raise, so the banner stops offering it.
//!
//! Every method that changes what the tile shows notifies the entity.

use std::rc::Rc;
use std::sync::Arc;

use hacivat_exec::{ResultTab, SessionLog, TabKey};
use hacivat_l10n::{Count, ids};
use hacivat_proto::DbmsKind;
use hacivat_results::{Completeness, PauseReason, ResultSource};
use hacivat_session::session::UnitId;
use hacivat_ui_core::gpui_surface::{App, Context, SharedString};
use hacivat_ui_grid::{ColId, GridDataSource};
use hacivat_values::valfmt::DisplayZone;

use crate::l10n_global::tr;
use crate::results::adapter::{GridAdapter, ResultDisplay};

/// The tab a console's results show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ActiveResult {
    /// The output log.
    #[default]
    Output,
    /// The result tab at this index (after Output).
    Tab(usize),
}

/// The memory banner of a result paused at a memory limit (R-M3-33).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MemoryBanner {
    /// The limit the result stopped at.
    pub reason: PauseReason,
    /// Whether "Fetch more anyway" is offered: not after a raise when the global budget is
    /// what stopped the result.
    pub can_raise: bool,
}

/// One result tab: its key and unit, its adapter over the result, its number, the widths it
/// keeps and its timings.
pub struct ResultEntry {
    key: TabKey,
    unit: UnitId,
    adapter: Rc<GridAdapter>,
    /// N of `Result N`.
    number: u32,
    /// The column names the kept widths belong to.
    names: Vec<SharedString>,
    widths: Vec<(ColId, f32)>,
    exec_ms: Option<u64>,
    fetch_ms: Option<u64>,
    /// "Fetch more anyway" was taken for the result the tab shows now.
    cap_raised: bool,
}

impl std::fmt::Debug for ResultEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResultEntry")
            .field("key", &self.key)
            .field("number", &self.number)
            .field("widths", &self.widths)
            .finish_non_exhaustive()
    }
}

impl ResultEntry {
    pub fn key(&self) -> TabKey {
        self.key
    }

    /// The unit whose result this is (Re-run names it).
    pub fn unit(&self) -> UnitId {
        self.unit
    }

    /// The tab's grid source. It stays the same object across re-runs.
    pub fn adapter(&self) -> &Rc<GridAdapter> {
        &self.adapter
    }

    /// The result's source: snapshots, the cursor's commands.
    pub fn source(&self) -> Arc<dyn ResultSource> {
        self.adapter.source()
    }

    /// N of `Result N`.
    pub fn number(&self) -> u32 {
        self.number
    }

    /// The widths this tab keeps (columns the user sized), for a grid view built anew.
    pub fn widths(&self) -> &[(ColId, f32)] {
        &self.widths
    }

    /// The memory banner, while the result is suspended at a memory limit.
    pub fn memory_banner(&self) -> Option<MemoryBanner> {
        let snapshot = self.adapter.snapshot();
        if !matches!(snapshot.completeness(), Completeness::Suspended(_)) {
            return None;
        }
        let reason = snapshot.paused()?;
        Some(MemoryBanner {
            reason,
            can_raise: !(self.cap_raised && reason == PauseReason::GlobalBudget),
        })
    }

    /// The statement's execution time and the time to the last fetched row, when the run UI
    /// has reported them.
    pub fn timing(&self) -> (Option<u64>, Option<u64>) {
        (self.exec_ms, self.fetch_ms)
    }
}

/// A console's results: its output log, its result tabs, the active one and whether the
/// output has lines the user has not seen.
#[derive(Debug)]
pub struct ConsoleResults {
    output: Arc<SessionLog>,
    tabs: Vec<ResultEntry>,
    active: ActiveResult,
    unread_output: bool,
    /// How this console's results are shown and copied.
    display: ResultDisplay,
    /// The next tab's N.
    next_number: u32,
}

impl ConsoleResults {
    /// No result tabs yet; the Output tab shows `output`. Until the console's host calls
    /// [`set_display`](Self::set_display), results are shown without a region (no grouping
    /// locale), in UTC, as a PostgreSQL console's.
    pub fn new(output: Arc<SessionLog>) -> Self {
        ConsoleResults {
            output,
            tabs: Vec::new(),
            active: ActiveResult::Output,
            unread_output: false,
            display: ResultDisplay::new("und", DisplayZone::utc(), DbmsKind::Postgres),
            next_number: 1,
        }
    }

    pub fn output(&self) -> &Arc<SessionLog> {
        &self.output
    }

    /// The result tabs, in tab order (after Output).
    pub fn tabs(&self) -> &[ResultEntry] {
        &self.tabs
    }

    pub fn tab(&self, ix: usize) -> Option<&ResultEntry> {
        self.tabs.get(ix)
    }

    /// The index of the tab keyed `key`.
    pub fn tab_by_key(&self, key: TabKey) -> Option<usize> {
        self.tabs.iter().position(|t| t.key == key)
    }

    pub fn active(&self) -> ActiveResult {
        self.active
    }

    /// The active result tab (`None` while Output is active).
    pub fn active_tab(&self) -> Option<&ResultEntry> {
        match self.active {
            ActiveResult::Output => None,
            ActiveResult::Tab(ix) => self.tabs.get(ix),
        }
    }

    pub fn unread_output(&self) -> bool {
        self.unread_output
    }

    /// How this console's results are shown and copied.
    pub fn display(&self) -> &ResultDisplay {
        &self.display
    }

    /// The title of tab `ix`: `Result N`.
    pub fn title(&self, ix: usize, cx: &App) -> Option<SharedString> {
        self.tabs.get(ix).map(|t| result_title(t.number, cx))
    }

    /// The console's format locale, display zone and engine, for the tabs it has and the ones
    /// to come.
    pub fn set_display(&mut self, display: ResultDisplay, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            tab.adapter.set_display(display.clone(), cx);
        }
        self.display = display;
        cx.notify();
    }

    /// Takes the result tabs of a run (the run service republishes the list as results
    /// arrive, so the same tab may come again). A key the console already shows reuses its tab
    /// (module docs); a new key gets a new tab at the end. The first tab that is new or was
    /// re-run becomes active. Answers the indices of the tabs that are new or were re-run.
    pub fn apply(&mut self, run: &[ResultTab], cx: &mut Context<Self>) -> Vec<usize> {
        let mut changed = Vec::new();
        for tab in run {
            match self.tab_by_key(tab.key) {
                Some(ix) => {
                    let entry = &mut self.tabs[ix];
                    let same_result = std::ptr::addr_eq(
                        Arc::as_ptr(&entry.adapter.source()),
                        Arc::as_ptr(&tab.source),
                    );
                    if same_result {
                        continue;
                    }
                    entry.adapter.retarget(tab.source.clone(), cx);
                    entry.unit = tab.unit;
                    entry.exec_ms = None;
                    entry.fetch_ms = None;
                    entry.cap_raised = false;
                    let names = column_names(&entry.adapter);
                    if names != entry.names {
                        entry.widths.clear();
                        entry.names = names;
                    }
                    changed.push(ix);
                }
                None => {
                    let adapter = GridAdapter::new(tab.source.clone(), self.display.clone());
                    let number = self.next_number;
                    self.next_number += 1;
                    self.tabs.push(ResultEntry {
                        key: tab.key,
                        unit: tab.unit,
                        names: column_names(&adapter),
                        adapter,
                        number,
                        widths: Vec::new(),
                        exec_ms: None,
                        fetch_ms: None,
                        cap_raised: false,
                    });
                    changed.push(self.tabs.len() - 1);
                }
            }
        }
        if let Some(first) = changed.first() {
            self.active = ActiveResult::Tab(*first);
            cx.notify();
        }
        changed
    }

    /// Shows `active`. Showing Output marks its lines as seen; an index past the tabs is
    /// ignored.
    pub fn set_active(&mut self, active: ActiveResult, cx: &mut Context<Self>) {
        match active {
            ActiveResult::Output => self.unread_output = false,
            ActiveResult::Tab(ix) if ix >= self.tabs.len() => return,
            ActiveResult::Tab(_) => {}
        }
        self.active = active;
        cx.notify();
    }

    /// A line arrived in the output log: unread while another tab is active.
    pub fn note_output(&mut self, cx: &mut Context<Self>) {
        if self.active != ActiveResult::Output && !self.unread_output {
            self.unread_output = true;
            cx.notify();
        }
    }

    /// Keeps `width` for column `col` of tab `ix` (the user resized it).
    pub fn set_width(&mut self, ix: usize, col: ColId, width: f32) {
        let Some(tab) = self.tabs.get_mut(ix) else {
            return;
        };
        match tab.widths.iter_mut().find(|(c, _)| *c == col) {
            Some(kept) => kept.1 = width,
            None => tab.widths.push((col, width)),
        }
    }

    /// The timings of the result keyed `key`, from the run's status (the status bar and the
    /// toolbar show them).
    pub fn set_timing(
        &mut self,
        key: TabKey,
        exec_ms: Option<u64>,
        fetch_ms: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        if let Some(ix) = self.tab_by_key(key) {
            let tab = &mut self.tabs[ix];
            if (tab.exec_ms, tab.fetch_ms) != (exec_ms, fetch_ms) {
                tab.exec_ms = exec_ms;
                tab.fetch_ms = fetch_ms;
                cx.notify();
            }
        }
    }

    /// "Fetch more anyway" on tab `ix`'s memory banner: the result's cap rises to the global
    /// budget and the source fetches on (`ResultSource::raise_cap`).
    pub fn raise_cap(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(ix) {
            tab.cap_raised = true;
            tab.adapter.source().raise_cap();
            cx.notify();
        }
    }

    /// Closes tab `ix`; the tab before it (or Output) becomes active when it was the active
    /// one. The other tabs keep their numbers.
    pub fn close_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.tabs.remove(ix);
        self.active = match self.active {
            ActiveResult::Tab(a) if a == ix => match ix.checked_sub(1) {
                Some(before) => ActiveResult::Tab(before),
                None if self.tabs.is_empty() => ActiveResult::Output,
                None => ActiveResult::Tab(0),
            },
            ActiveResult::Tab(a) if a > ix => ActiveResult::Tab(a - 1),
            other => other,
        };
        if self.active == ActiveResult::Output {
            self.unread_output = false;
        }
        cx.notify();
    }
}

/// `Result N`.
pub fn result_title(number: u32, cx: &App) -> SharedString {
    tr(
        cx,
        ids::hv::RESULTS_RESULT_N,
        ids::hv::HvResultsResultN {
            n: Count(u64::from(number)),
        },
    )
}

/// The adapter's column names, in order.
fn column_names(adapter: &GridAdapter) -> Vec<SharedString> {
    adapter.columns().iter().map(|c| c.name.clone()).collect()
}
