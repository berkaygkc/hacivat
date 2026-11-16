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
//!
//! ## m3b-t25-results (HV-EXEC-07 UI; R-M3-66, R-M3-68)
//! - **One tab per result set** (R-M3-66), titled by its statement's comment as the run
//!   service gives it (Task 16 sanitised it; shown as given, as data), else `Result N`. N counts
//!   the console's statements that opened a tab, so a statement's further sets share its N. A
//!   title shown by an earlier tab of the console again gets `(2)`, `(3)` …
//!   (`hv-results-tab-dup`): a statement's further result sets, two statements under one
//!   comment. The run service's own title tells which: it is `Result n[ (k)]` (n counting the
//!   run's statements that opened a tab) when the statement has no comment; a comment that
//!   reads exactly so is shown as the console's `Result N`.
//! - **Pinned tabs** ([`ConsoleResults::set_pinned`]) are never reused: the next run of the same
//!   statement opens a new tab beside it.
//! - **Visibility for eviction** (R-M3-68, the Task 8 carries). Every result tab reports to its
//!   source whether it is on screen (`ResultSource::set_visible`), new tabs included: visible
//!   while it is the console's active tab, pinned, or shown by another view of it
//!   ([`ConsoleResults::set_view_visible`], Task 38's views); hidden only when no view shows it,
//!   and only after [`HIDE_GRACE`] out of sight. A visible result is never evicted.
//! - **After an eviction** (`Closed { Evicted }`) the tab, once shown again, moves its adapter
//!   to a new generation over the same result: the grid clears its selection, so no copy is cut
//!   from rows the result no longer keeps (the tile also refuses such a copy).
//! - **"Fetch more anyway"** reads the raise from the source (`ResultSource::cap_raised`, the
//!   store's flag; the tab keeps no copy).
//! - **Count rows** state per tab ([`CountState`]; the work is `count.rs`'s).

use std::cell::Cell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use hacivat_exec::{ResultTab, SessionLog, TabKey};
use hacivat_l10n::{Count, Text, ids};
use hacivat_proto::{AccessLabel, DbmsKind};
use hacivat_results::{CloseReason, Completeness, PauseReason, ResultSource};
use hacivat_session::session::{RunId, UnitId};
use hacivat_ui::text::TextSource;
use hacivat_ui_core::gpui_surface::{App, Context, SharedString, Task, Window};
use hacivat_ui_grid::{ColId, GridDataSource};
use hacivat_values::NativeType;
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

// ## m3b-t25-results

/// How long a result tab stays visible to eviction after it leaves the screen (HV-RES-07).
pub const HIDE_GRACE: Duration = Duration::from_secs(30);

/// Where a tab's Count rows is (R-M3-76; `count.rs` runs it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CountState {
    /// Not asked (or cancelled): only the button.
    #[default]
    Idle,
    /// The count runs; `ticket` names it (an answer to an older one is dropped).
    Counting { ticket: u64 },
    /// The server's count.
    Exact(u64),
    /// No count: why, a Fluent ID (`hv-results-count-*`).
    Refused(&'static str),
}

/// What a tab's title is made of: its statement's comment, or the console's `Result N`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Base {
    Comment(Arc<str>),
    Number(u32),
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
    /// The access label of the connection when it was last seen connected for this result
    /// (the status bar shows it while the connection is away).
    access: Cell<Option<AccessLabel>>,
    // ## m3b-t25-results
    /// The console-unique ID of this tab (its timers name it).
    id: u64,
    /// The run service's title, whole (the tooltip's text when the tab cuts it).
    service_title: Arc<str>,
    /// The statement's comment title, `None` for `Result N`.
    comment: Option<Arc<str>>,
    pinned: bool,
    /// The views other than the tab strip that show this result now (Task 38).
    views: BTreeSet<u64>,
    /// What the source was last told (`None`: nothing yet; a store starts visible).
    reported: Option<bool>,
    /// The grace timer of a tab that left the screen.
    hide: Option<Task<()>>,
    /// The eviction of the result shown now has been answered (the selection was cleared).
    evicted_seen: bool,
    count: CountState,
    /// Cancels the count under way.
    count_cancel: Option<Rc<dyn Fn()>>,
}

impl std::fmt::Debug for ResultEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResultEntry")
            .field("key", &self.key)
            .field("number", &self.number)
            .field("widths", &self.widths)
            .field("pinned", &self.pinned)
            .field("count", &self.count)
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
    ///
    /// Its `raise_cap()` is what [`ConsoleResults::raise_cap`] calls: the store keeps the raise
    /// (`ResultSource::cap_raised`), which [`memory_banner`](Self::memory_banner) reads.
    pub fn source(&self) -> Arc<dyn ResultSource> {
        self.adapter.source()
    }

    /// The connection's access label when it was last seen connected for this result;
    /// `None` when it never was.
    pub fn last_access(&self) -> Option<AccessLabel> {
        self.access.get()
    }

    /// Records the access label of the live connection.
    pub(crate) fn note_access(&self, access: AccessLabel) {
        self.access.set(Some(access));
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
        // ## m3b-t25-results: the raise is the store's (Task 8, ledger 253 M7).
        let raised = self.source().cap_raised();
        Some(MemoryBanner {
            reason,
            can_raise: !(raised && reason == PauseReason::GlobalBudget),
        })
    }

    /// The statement's execution time and the time to the last fetched row, when the run UI
    /// has reported them.
    pub fn timing(&self) -> (Option<u64>, Option<u64>) {
        (self.exec_ms, self.fetch_ms)
    }

    // ## m3b-t25-results

    /// Whether the tab is pinned (the next run never reuses it).
    pub fn pinned(&self) -> bool {
        self.pinned
    }

    /// The run service's title, whole (`ResultTab::title`).
    pub fn service_title(&self) -> &Arc<str> {
        &self.service_title
    }

    /// The statement's comment title, as the run service gave it; `None` for `Result N`.
    pub fn comment(&self) -> Option<&Arc<str>> {
        self.comment.as_ref()
    }

    /// Whether the server sent the result untyped (run as one, R-M3-60): it has columns, and
    /// every one is text of no known native type (what the simple protocol yields).
    pub fn untyped(&self) -> bool {
        let snapshot = self.adapter.snapshot();
        let columns = &snapshot.schema().columns;
        !columns.is_empty()
            && columns.iter().all(|c| {
                c.native == NativeType::Unknown
                    && matches!(c.logical, hacivat_values::LogicalType::Text { .. })
            })
    }

    /// Where the tab's Count rows is.
    pub fn count(&self) -> CountState {
        self.count
    }

    /// Whether the source was last told the tab is visible (`None`: never told; a result
    /// starts visible).
    pub fn reported_visible(&self) -> Option<bool> {
        self.reported
    }

    /// Whether the result was closed to free memory (R-M3-68).
    pub fn evicted(&self) -> bool {
        matches!(
            self.adapter.snapshot().completeness(),
            Completeness::Closed {
                reason: CloseReason::Evicted
            }
        )
    }

    fn base(&self) -> Base {
        match &self.comment {
            Some(c) => Base::Comment(c.clone()),
            None => Base::Number(self.number),
        }
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
    /// The run whose first result was shown: its later results do not take the tab strip
    /// again.
    shown_run: Option<RunId>,
    // ## m3b-t25-results
    /// The next tab's ID.
    next_id: u64,
    /// The next count's ticket.
    next_count: u64,
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
            shown_run: None,
            next_id: 1,
            next_count: 1,
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

    /// The index of the tab keyed `key`: the one the next run of it reuses (never a pinned
    /// tab), else a pinned one.
    pub fn tab_by_key(&self, key: TabKey) -> Option<usize> {
        self.tabs
            .iter()
            .position(|t| t.key == key && !t.pinned)
            .or_else(|| self.tabs.iter().position(|t| t.key == key))
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

    /// The title tab `ix` shows: its comment title or `Result N`, with `(k)` when an earlier
    /// tab shows the same (module docs).
    pub fn title(&self, ix: usize, cx: &App) -> Option<SharedString> {
        self.title_with_source(ix, cx).map(|(t, _)| t)
    }

    // ## m3b-t25-results
    /// [`title`](Self::title) and where its text comes from: a comment title alone is data,
    /// everything else a Fluent message.
    pub fn title_with_source(&self, ix: usize, cx: &App) -> Option<(SharedString, TextSource)> {
        let entry = self.tabs.get(ix)?;
        let base = entry.base();
        let repeat = 1 + self.tabs[..ix].iter().filter(|t| t.base() == base).count() as u64;
        let (text, source) = match &base {
            Base::Comment(c) => (SharedString::from(c.to_string()), TextSource::Data),
            Base::Number(n) => (result_title(*n, cx), TextSource::Fluent),
        };
        if repeat == 1 {
            return Some((text, source));
        }
        Some((
            tr(
                cx,
                ids::hv::RESULTS_TAB_DUP,
                ids::hv::HvResultsTabDup {
                    title: Text(text.to_string()),
                    n: Count(repeat),
                },
            ),
            TextSource::Fluent,
        ))
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
    /// (module docs); a new key gets a new tab at the end. The first new or re-run tab of a
    /// run (by its `RunId`) becomes active; the run's later results change nothing about the
    /// active tab, so a tab the user chose during the run stays. Answers the indices of the
    /// tabs that are new or were re-run.
    ///
    /// ## m3b-t25-results: a tab already showing the result is left as it is; a pinned tab is
    /// never reused (a new tab opens); every tab reports its visibility.
    pub fn apply(&mut self, run: &[ResultTab], cx: &mut Context<Self>) -> Vec<usize> {
        // The access of the connection the results came over, while it is connected.
        let access = crate::results::context::live_access_of(&cx.entity(), cx);
        let mut changed = Vec::new();
        for tab in run {
            let shown = self.tabs.iter().any(|t| {
                std::ptr::addr_eq(Arc::as_ptr(&t.adapter.source()), Arc::as_ptr(&tab.source))
            });
            if shown {
                continue;
            }
            let comment = comment_of(tab, run);
            match self.tabs.iter().position(|t| t.key == tab.key && !t.pinned) {
                Some(ix) => {
                    let entry = &mut self.tabs[ix];
                    entry.adapter.retarget(tab.source.clone(), cx);
                    entry.unit = tab.unit;
                    entry.exec_ms = None;
                    entry.fetch_ms = None;
                    // A new result: what was known of the old one's connection is not kept.
                    entry.access.set(access);
                    // ## m3b-t25-results
                    entry.service_title = tab.title.clone();
                    entry.comment = comment;
                    entry.reported = None;
                    entry.hide = None;
                    entry.evicted_seen = false;
                    if let Some(cancel) = entry.count_cancel.take() {
                        cancel();
                    }
                    entry.count = CountState::Idle;
                    let names = column_names(&entry.adapter);
                    if names != entry.names {
                        entry.widths.clear();
                        entry.names = names;
                    }
                    changed.push(ix);
                }
                None => {
                    let adapter = GridAdapter::new(tab.source.clone(), self.display.clone());
                    // ## m3b-t25-results: a statement's further result sets share its N.
                    let number = match self.tabs.iter().find(|t| t.unit == tab.unit && !t.pinned) {
                        Some(t) => t.number,
                        None => {
                            let n = self.next_number;
                            self.next_number += 1;
                            n
                        }
                    };
                    let id = self.next_id;
                    self.next_id += 1;
                    self.tabs.push(ResultEntry {
                        key: tab.key,
                        unit: tab.unit,
                        names: column_names(&adapter),
                        adapter,
                        number,
                        widths: Vec::new(),
                        exec_ms: None,
                        fetch_ms: None,
                        access: Cell::new(access),
                        id,
                        service_title: tab.title.clone(),
                        comment,
                        pinned: false,
                        views: BTreeSet::new(),
                        reported: None,
                        hide: None,
                        evicted_seen: false,
                        count: CountState::Idle,
                        count_cancel: None,
                    });
                    changed.push(self.tabs.len() - 1);
                }
            }
        }
        if let Some(first) = changed.first() {
            // Once per run: the run service republishes its list as results arrive, and a
            // later result must not take the strip from the first one, or from a tab the user
            // chose meanwhile.
            let run = self.tabs[*first].unit.run;
            if self.shown_run != Some(run) {
                self.shown_run = Some(run);
                self.active = ActiveResult::Tab(*first);
            }
            self.sync_visibility(cx);
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
        // ## m3b-t25-results
        self.on_shown(cx);
        self.sync_visibility(cx);
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
    /// budget and the source fetches on (`ResultSource::raise_cap`, which the store records).
    pub fn raise_cap(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(ix) {
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
        // ## m3b-t25-results: a count under way stops with its tab.
        if let Some(cancel) = self.tabs[ix].count_cancel.take() {
            cancel();
        }
        self.tabs.remove(ix);
        let was = self.active;
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
        // ## m3b-t25-results: another tab took the strip.
        if was == ActiveResult::Tab(ix) {
            self.on_shown(cx);
        }
        self.sync_visibility(cx);
        cx.notify();
    }

    // ## m3b-t25-results

    /// Closes every result tab but `ix` (the tab menu's "Close other tabs").
    pub fn close_others(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        let keep = self.tabs[ix].id;
        let active = self.active_tab().map(|t| t.id);
        for tab in &mut self.tabs {
            if tab.id != keep
                && let Some(cancel) = tab.count_cancel.take()
            {
                cancel();
            }
        }
        self.tabs.retain(|t| t.id == keep);
        self.active = match self.active {
            ActiveResult::Output => ActiveResult::Output,
            ActiveResult::Tab(_) => ActiveResult::Tab(0),
        };
        if active != Some(keep) && self.active == ActiveResult::Tab(0) {
            self.on_shown(cx);
        }
        self.sync_visibility(cx);
        cx.notify();
    }

    /// Pins (or unpins) tab `ix`: a pinned tab is never reused by a run and never hidden from
    /// eviction's view of the screen.
    pub fn set_pinned(&mut self, ix: usize, pinned: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(ix) else {
            return;
        };
        if tab.pinned != pinned {
            tab.pinned = pinned;
            self.sync_visibility(cx);
            cx.notify();
        }
    }

    /// Another view (`view`, an ID its owner chooses; Task 38's) shows tab `ix`'s result, or
    /// no longer does. The result is hidden to eviction only when no view shows it.
    pub fn set_view_visible(
        &mut self,
        ix: usize,
        view: u64,
        visible: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.get_mut(ix) else {
            return;
        };
        let changed = if visible {
            tab.views.insert(view)
        } else {
            tab.views.remove(&view)
        };
        if changed {
            self.sync_visibility(cx);
        }
    }

    /// Tab `ix`'s Count rows starts: its ticket (an answer for another ticket is dropped).
    pub fn start_count(
        &mut self,
        ix: usize,
        cancel: Rc<dyn Fn()>,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let tab = self.tabs.get_mut(ix)?;
        if let Some(old) = tab.count_cancel.take() {
            old();
        }
        let ticket = self.next_count;
        self.next_count += 1;
        tab.count = CountState::Counting { ticket };
        tab.count_cancel = Some(cancel);
        cx.notify();
        Some(ticket)
    }

    /// The count `ticket` of the tab with ID `tab_id` ended in `state`; dropped when the tab
    /// is gone, re-ran or counts again.
    pub(crate) fn finish_count(
        &mut self,
        tab_id: u64,
        ticket: u64,
        state: CountState,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        if tab.count != (CountState::Counting { ticket }) {
            return;
        }
        tab.count = state;
        tab.count_cancel = None;
        cx.notify();
    }

    /// Sets tab `ix`'s count without running one (a refusal before anything is sent).
    pub fn set_count(&mut self, ix: usize, state: CountState, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(ix) {
            if let Some(cancel) = tab.count_cancel.take() {
                cancel();
            }
            tab.count = state;
            cx.notify();
        }
    }

    /// Cancels tab `ix`'s count: back to the button.
    pub fn cancel_count(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.set_count(ix, CountState::Idle, cx);
    }

    /// The ID of tab `ix` (its count and timers name it).
    pub(crate) fn tab_id(&self, ix: usize) -> Option<u64> {
        self.tabs.get(ix).map(|t| t.id)
    }

    /// The active tab was just shown: a result evicted since it was last answered moves its
    /// adapter to a new generation over the same result, so the grid clears a selection that
    /// may reach past the rows it kept (the Task 8 carry: a copy is never clamped silently).
    fn on_shown(&mut self, cx: &mut Context<Self>) {
        let ActiveResult::Tab(ix) = self.active else {
            return;
        };
        let Some(tab) = self.tabs.get_mut(ix) else {
            return;
        };
        if !tab.evicted_seen && tab.evicted() {
            tab.evicted_seen = true;
            let source = tab.adapter.source();
            tab.adapter.retarget(source, cx);
        }
    }

    /// Whether tab `ix` is on screen to eviction: active, pinned or shown by another view.
    fn wants_visible(&self, ix: usize) -> bool {
        self.tabs.get(ix).is_some_and(|t| {
            t.pinned || self.active == ActiveResult::Tab(ix) || !t.views.is_empty()
        })
    }

    /// Tells every tab's source whether it is visible: at once when it is, after
    /// [`HIDE_GRACE`] out of sight when it is not (module docs).
    fn sync_visibility(&mut self, cx: &mut Context<Self>) {
        for ix in 0..self.tabs.len() {
            let want = self.wants_visible(ix);
            let tab = &mut self.tabs[ix];
            if want {
                tab.hide = None;
                if tab.reported != Some(true) {
                    tab.adapter.source().set_visible(true);
                    tab.reported = Some(true);
                }
            } else if tab.reported != Some(false) && tab.hide.is_none() {
                let id = tab.id;
                tab.hide = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(HIDE_GRACE).await;
                    let _ = this.update(cx, |r, _| r.grace_over(id));
                }));
            }
        }
    }

    /// Tab `id`'s grace ended: hidden to eviction unless something shows it again.
    fn grace_over(&mut self, id: u64) {
        let Some(ix) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let want = self.wants_visible(ix);
        let tab = &mut self.tabs[ix];
        tab.hide = None;
        if !want {
            tab.adapter.source().set_visible(false);
            tab.reported = Some(false);
        }
    }
}

// ## m3b-t25-results

/// The comment title of `tab` (one of `run`'s tabs), or `None` when the run service titled it
/// `Result n[ (k)]` (module docs) or not at all (an M3a title).
fn comment_of(tab: &ResultTab, run: &[ResultTab]) -> Option<Arc<str>> {
    let title: &str = &tab.title;
    if title.is_empty() {
        return None;
    }
    let set = tab.key.result_ordinal;
    let base = match set {
        0 => title,
        _ => title
            .strip_suffix(&format!(" ({})", u64::from(set) + 1))
            .unwrap_or(title),
    };
    // n of the run service's `Result n`: the statement's place among the run's statements that
    // opened a tab, in the order their first tab came.
    let mut units: Vec<u32> = Vec::new();
    for t in run {
        if !units.contains(&t.key.unit_ordinal) {
            units.push(t.key.unit_ordinal);
        }
    }
    let n = units
        .iter()
        .position(|u| *u == tab.key.unit_ordinal)
        .map_or(0, |p| p + 1);
    if base == format!("Result {n}") {
        return None;
    }
    Some(Arc::from(base))
}

// ## m3b-t01b-seams (named exception): the bodies of `results.next_tab`/`prev_tab` (Task 25).

/// Activates the next (`forward`) or previous tab of the results `window` shows, Output
/// included, wrapping at either end.
pub(crate) fn step_tab(forward: bool, window: &mut Window, cx: &mut App) {
    // ## m3b-t25-results
    let Some((_, results)) = crate::results::view::console_in(window, cx) else {
        return;
    };
    results.update(cx, |r, cx| {
        let count = r.tabs.len() + 1;
        let at = match r.active {
            ActiveResult::Output => 0,
            ActiveResult::Tab(ix) => ix + 1,
        };
        let next = if forward {
            (at + 1) % count
        } else {
            (at + count - 1) % count
        };
        let target = match next {
            0 => ActiveResult::Output,
            n => ActiveResult::Tab(n - 1),
        };
        r.set_active(target, cx);
    });
}

/// Whether `results.next_tab`/`prev_tab` can run: the shown console has a result tab to step
/// to.
pub(crate) fn can_step_tab(cx: &App) -> bool {
    // ## m3b-t25-results
    crate::results::view::shown_results(cx).is_some_and(|(_, r)| !r.read(cx).tabs.is_empty())
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
