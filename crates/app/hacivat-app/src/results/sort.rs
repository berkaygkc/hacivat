//! Local sort (HV-GRID-07, R-M3-70; M3c Task 36): the sort job off the UI thread by the column's
//! collation, publishing `ViewMap::Permuted`, the "Sorted locally" banners, and the sort
//! commands.
//!
//! ## m3b-t01b-seams: the seam (Task 1b). Task 36 fills the bodies.
//!
//! ## m3c-t36
//! - **Requests.** A grid asks with `GridEvent::SortRequested(spec)` (the tile calls
//!   [`on_requested`]); the commands `grid.sort_asc` / `grid.sort_desc` ask for the focus
//!   column, `grid.clear_sort` for the source order (an empty spec), and Task 39's menu goes
//!   through `GridView::request_sort` too. The header click keeps selecting the column (M3a).
//! - **The job runs on the background executor**: it extracts each key column's cells of one
//!   snapshot into a compact array (one `ValueRef` per row, borrowed from the snapshot), then
//!   `hacivat_ui_grid::model::stable_order` sorts the row numbers (stable: ties keep model
//!   order; split over threads and merged above a million rows; it looks at the job's
//!   cancellation flag every 64k comparisons), and builds `ViewMap::permuted` (its O(n) index)
//!   there too. The UI thread only publishes the map (`GridView::set_view_map`) and the header
//!   indicator (`GridView::set_sort`). Meanwhile the grid keeps the order it shows.
//! - **The generation guard.** Every request, cancel and clear bumps the grid's request number;
//!   a job publishes only while its number is still the current one **and** the adapter is
//!   still at the generation the job sorted (a re-run moves it on). So a stale sort never
//!   replaces a newer one, and never puts its order on a re-run's rows. A newer request also
//!   raises the older job's flag, so it stops at its next look.
//! - **Fix round 1.** A job stops whenever its state goes (`Running`'s drop raises its flag): a
//!   newer sort, Cancel, Clear sort, a re-run (the tile drops the old run's state, `sync`) and
//!   a closed tab or console (`grid_closed`, from the tile's prune). A request is refused,
//!   claiming nothing, while the grid has not painted the adapter's current generation, and a
//!   job publishes only to a grid that shows the generation it sorted (m3). A job that panics
//!   keeps the order shown and says so in a notice (`hv-grid-sort-failed`), never a cancel (m4).
//! - **A re-run drops the sort** (the grid drops its map and indicator at the new generation's
//!   first frame); sorting again is the user's call.
//! - **Collation (R-M3-70).** Each key compares by `valfmt::compare::comparator_for` with the
//!   column's database collation ([`collation_of`]), never the UI locale. Today no engine sends
//!   a column's collation with a result (PostgreSQL's row description has none; MySQL's
//!   column definition carries the *connection's* result character set, not the column's), so
//!   every text column compares by code point and its header tooltip says the collation is not
//!   known (`hv-grid-sort-tip-unknown`). A catalog lookup through the column's origin fills
//!   [`collation_of`] later (ruling request in the Task 36 report).
//! - **The note row** under the result toolbar (NOTES surface 18, M3c-20; [`note_kind`]):
//!   "Sorting N rows…" with Cancel once a sort has run 200 ms; on a partial result "Sorted
//!   locally: …" with Clear sort; rows that arrived after the sort wait in the unsorted tail
//!   (the grid draws a dashed line above it): with Re-sort, and on a partial result both facts
//!   in one row with Re-sort and Clear sort.

use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::time::Duration;

use hacivat_l10n::{Count, NoArgs, Text, ids};
use hacivat_proto::DbmsKind;
use hacivat_results::{Completeness, StoreSnapshot, TruncReason};
use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::text::{TextFn, TextSource, paragraph};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext as _, Entity, EntityId, Global, InteractiveElement, IntoElement,
    ParentElement, SharedString, Styled, Task, WeakEntity, Window, div, px,
};
use hacivat_ui_core::theme::Theme;
use hacivat_ui_grid::model::{Cancelled, SortSpec, ViewMap, stable_order};
use hacivat_ui_grid::{GRID_CONTEXT, GridView};
use hacivat_values::valfmt::compare::{CollationUse, Comparator, comparator_for};
use hacivat_values::valfmt::{BinaryDisplay, ColumnFacts};
use hacivat_values::{ColumnMeta, LogicalType, ValueRef};

use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};

use crate::l10n_global::{text_fn, tr};
use crate::results::adapter::GridAdapter;

/// The sort commands: `(id, title)`.
const COMMANDS: [(&str, &str); 3] = [
    ("grid.sort_asc", "hv-cmd-grid-sort-asc"),
    ("grid.sort_desc", "hv-cmd-grid-sort-desc"),
    ("grid.clear_sort", "hv-cmd-grid-clear-sort"),
];

/// A sort still running after this long shows "Sorting N rows…" with Cancel (NOTES surface 18).
pub const SLOW_AFTER: Duration = Duration::from_millis(200);

/// What the tile calls to repaint (the note row) when a sort's state changes.
pub type OnChange = Rc<dyn Fn(&mut App)>;

// ---- the job ----------------------------------------------------------------------------------

/// One key of a sort job: a schema column, its direction and its comparator (`Send`).
pub struct JobKey {
    col: u32,
    desc: bool,
    cmp: Comparator,
}

impl JobKey {
    /// How the key's text compares (its header tooltip says so).
    pub fn collation(&self) -> &CollationUse {
        self.cmp.collation()
    }
}

/// The column's database collation, when the result names one. `None` today for every
/// engine (module docs): a result's columns carry no collation.
pub fn collation_of(meta: &ColumnMeta) -> Option<Arc<str>> {
    let _ = meta;
    None
}

/// The keys of `spec` over `snapshot`'s columns on `engine`, each with its column's collation
/// ([`collation_of`]). A key past the schema is left out.
pub fn job_keys(snapshot: &StoreSnapshot, spec: &SortSpec, engine: DbmsKind) -> Vec<JobKey> {
    let columns = &snapshot.schema().columns;
    let collations: Vec<Option<Arc<str>>> = columns.iter().map(collation_of).collect();
    let refs: Vec<Option<&str>> = collations.iter().map(|c| c.as_deref()).collect();
    job_keys_with(snapshot, spec, engine, &refs)
}

/// [`job_keys`] with each schema column's collation given (`collations[i]` for column `i`).
pub fn job_keys_with(
    snapshot: &StoreSnapshot,
    spec: &SortSpec,
    engine: DbmsKind,
    collations: &[Option<&str>],
) -> Vec<JobKey> {
    let columns = &snapshot.schema().columns;
    spec.keys
        .iter()
        .filter_map(|key| {
            let meta = columns.get(key.col.0 as usize)?;
            let facts = ColumnFacts {
                logical: &meta.logical,
                native_name: "",
                id_like: false,
                binary: BinaryDisplay::Detect,
                engine,
            };
            let collation = collations.get(key.col.0 as usize).copied().flatten();
            Some(JobKey {
                col: key.col.0,
                desc: key.desc,
                cmp: comparator_for(&facts, collation),
            })
        })
        .collect()
}

/// Every 64k rows extracted, the job looks at its flag too.
const EXTRACT_CHECK: u64 = 1 << 16;

/// The rows of `snapshot` in the order `keys` give them (stable), or [`Cancelled`] once
/// `cancelled` answers `true`. Pure: it runs on any thread (module docs).
pub fn sort_rows(
    snapshot: &StoreSnapshot,
    keys: &[JobKey],
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Vec<u64>, Cancelled> {
    let rows = snapshot.row_count();
    // The compact arrays: one borrowed value per row and key.
    let mut columns: Vec<Vec<ValueRef<'_>>> = Vec::with_capacity(keys.len());
    for key in keys {
        let mut values = Vec::with_capacity(usize::try_from(rows).unwrap_or(0));
        for row in 0..rows {
            if row % EXTRACT_CHECK == EXTRACT_CHECK - 1 && cancelled() {
                return Err(Cancelled);
            }
            values.push(snapshot.cell(row, key.col));
        }
        columns.push(values);
    }
    let cmp = |a: u64, b: u64| {
        for (key, values) in keys.iter().zip(&columns) {
            let o = key.cmp.cmp(values[a as usize], values[b as usize]);
            let o = if key.desc { o.reverse() } else { o };
            if o != std::cmp::Ordering::Equal {
                return o;
            }
        }
        std::cmp::Ordering::Equal
    };
    stable_order(rows, &cmp, cancelled)
}

// ---- the state --------------------------------------------------------------------------------

/// Each grid's local sort, by the grid's entity.
#[derive(Default)]
struct Sorts(HashMap<EntityId, LocalSort>, TestHooks);

// ## m3c-t36 fix round 1 (m8): test seams (`test-support`): finished jobs can be held and
// published later in a chosen order, and the next job can be made to fail.
#[derive(Default)]
struct TestHooks {
    #[cfg(feature = "test-support")]
    held: Option<Vec<Finished>>,
    #[cfg(feature = "test-support")]
    fail_next: bool,
}

/// A finished job, waiting to be published.
#[cfg_attr(not(feature = "test-support"), allow(dead_code))]
struct Finished {
    key: EntityId,
    request: u64,
    spec: SortSpec,
    map: ViewMap,
    tips: Vec<TextFn>,
}

impl Global for Sorts {}

/// One grid's local sort.
struct LocalSort {
    grid: WeakEntity<GridView>,
    adapter: Weak<GridAdapter>,
    on_change: OnChange,
    /// The adapter generation this state belongs to (a re-run makes it stale).
    generation: u64,
    /// Bumped by every request, cancel and clear: the generation guard.
    request: u64,
    /// The sort the grid shows, and how many rows it sorted (the rest are the tail).
    shown: Option<(SortSpec, u64)>,
    running: Option<Running>,
}

/// A sort job in flight.
struct Running {
    request: u64,
    rows: u64,
    cancel: Arc<AtomicBool>,
    /// It has run past [`SLOW_AFTER`]: the note shows it.
    slow: bool,
    _publish: Task<()>,
    _timer: Task<()>,
}

impl Running {
    fn stop(&self) {
        self.cancel.store(true, AtomicOrdering::Relaxed);
    }
}

// ## m3c-t36 fix round 1 (m2): a job whose state goes (a newer sort, Cancel, Clear sort, a
// re-run, a closed tab or grid) stops at its next look at the flag.
impl Drop for Running {
    fn drop(&mut self) {
        self.stop();
    }
}

/// How a job ended.
enum JobEnd {
    Done(ViewMap),
    Cancelled,
    /// It panicked (a comparator bug): the old order stays and the user is told.
    Failed,
}

/// A grid's sort, as the tests and the commands see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortStatus {
    /// The sort the grid shows (empty: none).
    pub shown: SortSpec,
    /// The rows that sort ordered.
    pub sorted: Option<u64>,
    /// A job runs over this many rows.
    pub running: Option<u64>,
    /// It has run past [`SLOW_AFTER`].
    pub slow: bool,
}

/// `grid`'s local sort, while its adapter is still at the generation it belongs to.
pub fn status(grid: &Entity<GridView>, cx: &App) -> Option<SortStatus> {
    let state = cx.try_global::<Sorts>()?.0.get(&grid.entity_id())?;
    let adapter = state.adapter.upgrade()?;
    if adapter.generation() != state.generation {
        return None;
    }
    Some(SortStatus {
        shown: state
            .shown
            .as_ref()
            .map(|s| s.0.clone())
            .unwrap_or_default(),
        sorted: state.shown.as_ref().map(|s| s.1),
        running: state.running.as_ref().map(|r| r.rows),
        slow: state.running.as_ref().is_some_and(|r| r.slow),
    })
}

/// The grid asked for a sort of its result (`GridEvent::SortRequested`; the tile calls it):
/// an empty `spec` clears the sort, any other starts a job (module docs). `on_change` repaints
/// the tile.
pub fn on_requested(
    grid: &Entity<GridView>,
    adapter: &Rc<GridAdapter>,
    spec: SortSpec,
    on_change: OnChange,
    cx: &mut App,
) {
    if spec.is_empty() {
        clear(grid, cx);
        return;
    }
    let key = grid.entity_id();
    let generation = adapter.generation();
    // ## m3c-t36 fix round 1 (m3): the grid has not painted this run's rows yet (a re-run
    // arrived a frame ago): an order now would land on rows it then drops. Nothing is sorted,
    // and nothing says it is.
    if grid.read(cx).generation() != Some(generation) {
        forget(key, cx);
        return;
    }
    let snapshot = adapter.snapshot();
    let rows = snapshot.row_count();
    let engine = adapter.display().engine();
    let keys = job_keys(&snapshot, &spec, engine);
    let tips = tips_of(&snapshot, &spec, &keys);
    let request = {
        let sorts = cx.default_global::<Sorts>();
        let state = sorts.0.entry(key).or_insert_with(|| LocalSort {
            grid: grid.downgrade(),
            adapter: Rc::downgrade(adapter),
            on_change: on_change.clone(),
            generation,
            request: 0,
            shown: None,
            running: None,
        });
        if state.generation != generation || state.adapter.upgrade().is_none() {
            // A re-run (or a retargeted tile): what was shown belongs to other rows.
            state.generation = generation;
            state.adapter = Rc::downgrade(adapter);
            state.shown = None;
        }
        state.on_change = on_change;
        // A newer sort: the older job's drop raises its flag.
        state.running = None;
        state.request += 1;
        state.request
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    #[cfg(feature = "test-support")]
    let fail = std::mem::take(&mut cx.default_global::<Sorts>().1.fail_next);
    #[cfg(not(feature = "test-support"))]
    let fail = false;
    let job = cx.background_spawn(async move {
        // ## m3c-t36 fix round 1 (m4): a panic is a failure the user sees, not a cancel.
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if fail {
                panic!("a sort job failed on purpose (test seam)");
            }
            sort_rows(&snapshot, &keys, &|| flag.load(AtomicOrdering::Relaxed))
        }));
        match ran {
            Ok(Ok(order)) => JobEnd::Done(ViewMap::permuted(Arc::from(order), rows, rows)),
            Ok(Err(Cancelled)) => JobEnd::Cancelled,
            Err(_) => JobEnd::Failed,
        }
    });
    let publish = cx.spawn(async move |cx| match job.await {
        JobEnd::Done(map) => cx.update(|cx| {
            publish(
                Finished {
                    key,
                    request,
                    spec,
                    map,
                    tips,
                },
                cx,
            )
        }),
        JobEnd::Cancelled => {}
        JobEnd::Failed => cx.update(|cx| failed(key, request, cx)),
    });
    let timer = cx.spawn(async move |cx| {
        cx.background_executor().timer(SLOW_AFTER).await;
        cx.update(|cx| mark_slow(key, request, cx));
    });
    if let Some(state) = cx.default_global::<Sorts>().0.get_mut(&key) {
        state.running = Some(Running {
            request,
            rows,
            cancel,
            slow: false,
            _publish: publish,
            _timer: timer,
        });
    }
}

/// A job finished: the order goes to the grid if the job is still current (the generation
/// guard, module docs).
fn publish(done: Finished, cx: &mut App) {
    #[cfg(feature = "test-support")]
    if let Some(held) = cx.default_global::<Sorts>().1.held.as_mut() {
        held.push(done);
        return;
    }
    let Finished {
        key,
        request,
        spec,
        map,
        tips,
    } = done;
    let Some(state) = cx.try_global::<Sorts>().and_then(|s| s.0.get(&key)) else {
        return;
    };
    let (Some(grid), Some(adapter)) = (state.grid.upgrade(), state.adapter.upgrade()) else {
        forget(key, cx);
        return;
    };
    // The guard: still the current request and the running job, the adapter still at the
    // generation the job sorted, and (## m3c-t36 fix round 1, m3) the grid showing it.
    let current = state.request == request
        && state.running.as_ref().is_some_and(|r| r.request == request)
        && adapter.generation() == state.generation
        && grid.read(cx).generation() == Some(state.generation);
    if !current {
        if adapter.generation() != state.generation {
            // A re-run: the state belongs to rows that are gone.
            forget(key, cx);
        }
        return;
    }
    let rows = map.len();
    let on_change = state.on_change.clone();
    if let Some(state) = cx.default_global::<Sorts>().0.get_mut(&key) {
        state.running = None;
        state.shown = Some((spec.clone(), rows));
    }
    // ## m3c-t37 (named exception; Task 36 review m6): the order goes through the grid's view
    // owner, which keeps a filter over it.
    super::find::show_base(&grid, Some(map), cx);
    grid.update(cx, |g, cx| g.set_sort(spec, tips, cx));
    on_change(cx);
}

// ## m3c-t36 fix round 1 (m4)
/// A job failed (it panicked): the order shown stays, the note goes, and a notice says so.
fn failed(key: EntityId, request: u64, cx: &mut App) {
    let Some(state) = cx.default_global::<Sorts>().0.get_mut(&key) else {
        return;
    };
    if state.running.as_ref().is_none_or(|r| r.request != request) {
        return;
    }
    state.running = None;
    let on_change = state.on_change.clone();
    let mut toast = Toast::new(ToastKind::Info, text_fn(ids::hv::GRID_SORT_FAILED));
    toast.icon = Some(IconName::CircleAlert);
    toast.wide = true;
    ToastQueue::push(cx, toast);
    on_change(cx);
}

/// Drops `key`'s sort state (its job, if any, stops: `Running`'s drop).
fn forget(key: EntityId, cx: &mut App) {
    if let Some(sorts) = cx.try_global::<Sorts>()
        && sorts.0.contains_key(&key)
    {
        cx.default_global::<Sorts>().0.remove(&key);
    }
}

// ## m3c-t36 fix round 1 (m2)
/// The tile shows `grid` over `adapter` at its current generation: a sort state of an older
/// run (a re-run) goes, and its job stops.
pub(crate) fn sync(grid: EntityId, adapter: &GridAdapter, cx: &mut App) {
    let stale = cx
        .try_global::<Sorts>()
        .and_then(|s| s.0.get(&grid))
        .is_some_and(|s| s.generation != adapter.generation());
    if stale {
        forget(grid, cx);
    }
}

/// `grid` is gone (its tab closed, its console closed): its sort state goes, its job stops.
pub(crate) fn grid_closed(grid: EntityId, cx: &mut App) {
    forget(grid, cx);
}

/// How many grids keep a sort state (the tests check nothing leaks).
#[doc(hidden)]
pub fn tracked(cx: &App) -> usize {
    cx.try_global::<Sorts>().map_or(0, |s| s.0.len())
}

/// Holds every finished job from now on instead of publishing it (`test-support`): a test
/// then publishes them in the order it wants with [`release_held`].
#[cfg(feature = "test-support")]
pub fn hold_publishing(cx: &mut App) {
    cx.default_global::<Sorts>().1.held = Some(Vec::new());
}

/// Publishes the held jobs, the newest first when `newest_first`, through the same guard
/// (`test-support`); later jobs publish as they finish.
#[cfg(feature = "test-support")]
pub fn release_held(newest_first: bool, cx: &mut App) {
    let mut held = cx
        .default_global::<Sorts>()
        .1
        .held
        .take()
        .unwrap_or_default();
    if newest_first {
        held.reverse();
    }
    for done in held {
        publish(done, cx);
    }
}

/// The next job panics (`test-support`).
#[cfg(feature = "test-support")]
pub fn fail_next_job(cx: &mut App) {
    cx.default_global::<Sorts>().1.fail_next = true;
}

/// A job has run past [`SLOW_AFTER`]: the note says so.
fn mark_slow(key: EntityId, request: u64, cx: &mut App) {
    let Some(state) = cx.default_global::<Sorts>().0.get_mut(&key) else {
        return;
    };
    let Some(running) = state.running.as_mut().filter(|r| r.request == request) else {
        return;
    };
    running.slow = true;
    let on_change = state.on_change.clone();
    on_change(cx);
}

/// Stops `grid`'s running sort (the note's Cancel): the order shown stays, and the job never
/// publishes.
pub fn cancel(grid: &Entity<GridView>, cx: &mut App) {
    let Some(state) = cx.default_global::<Sorts>().0.get_mut(&grid.entity_id()) else {
        return;
    };
    state.request += 1;
    // Dropping it raises its flag.
    state.running = None;
    let on_change = state.on_change.clone();
    on_change(cx);
}

/// Back to the source order (Clear sort): any job stops, the map and the indicator go.
pub fn clear(grid: &Entity<GridView>, cx: &mut App) {
    // Dropping the state raises its job's flag.
    let state = cx.default_global::<Sorts>().0.remove(&grid.entity_id());
    // ## m3c-t37 (named exception; Task 36 review m6): back to the source order, under the
    // filter when one is on.
    super::find::show_base(grid, None, cx);
    grid.update(cx, |g, cx| g.set_sort(SortSpec::default(), Vec::new(), cx));
    if let Some(state) = state {
        (state.on_change)(cx);
    }
}

/// Sorts again by the sort shown (Re-sort): the tail's rows join the order.
pub fn resort(grid: &Entity<GridView>, cx: &mut App) {
    let Some(state) = cx
        .try_global::<Sorts>()
        .and_then(|s| s.0.get(&grid.entity_id()))
    else {
        return;
    };
    let (Some(adapter), Some((spec, _))) = (state.adapter.upgrade(), state.shown.clone()) else {
        return;
    };
    let on_change = state.on_change.clone();
    on_requested(grid, &adapter, spec, on_change, cx);
}

// ---- the header tooltips ------------------------------------------------------------------

/// Whether `logical`'s values compare as text (its tooltip names the collation); an enum
/// compares by its labels' declared order.
fn textual(logical: &LogicalType) -> bool {
    !matches!(
        logical,
        LogicalType::Bool
            | LogicalType::Enum(_)
            | LogicalType::Int { .. }
            | LogicalType::Decimal { .. }
            | LogicalType::Float { .. }
            | LogicalType::Money
            | LogicalType::Bytes { .. }
            | LogicalType::Uuid
            | LogicalType::Date
            | LogicalType::Time { .. }
            | LogicalType::Timestamp { .. }
            | LogicalType::Interval
    )
}

/// Each key's header tooltip (NOTES surface 18).
fn tips_of(snapshot: &StoreSnapshot, spec: &SortSpec, keys: &[JobKey]) -> Vec<TextFn> {
    let columns = &snapshot.schema().columns;
    spec.keys
        .iter()
        .enumerate()
        .map(|(level, key)| {
            let use_ = columns
                .get(key.col.0 as usize)
                .filter(|m| textual(&m.logical))
                .and_then(|_| keys.iter().find(|k| k.col == key.col.0))
                .map(|k| k.collation().clone());
            let desc = key.desc;
            let tip: TextFn = Rc::new(move |cx: &App| tip_text(level, desc, use_.as_ref(), cx));
            tip
        })
        .collect()
}

/// The tooltip of sort level `level` (0: the first key): its level and direction, and for a
/// text column (`collation` given) how its text compares.
pub fn tip_text(
    level: usize,
    desc: bool,
    collation: Option<&CollationUse>,
    cx: &App,
) -> SharedString {
    let lvl = Count(level as u64 + 1);
    let dir = Text(
        tr(
            cx,
            if desc {
                ids::hv::GRID_DIR_DESC
            } else {
                ids::hv::GRID_DIR_ASC
            },
            NoArgs,
        )
        .to_string(),
    );
    match collation {
        None => tr(
            cx,
            ids::hv::GRID_SORT_LEVEL,
            ids::hv::HvGridSortLevel { lvl, dir },
        ),
        Some(CollationUse::Mapped { name } | CollationUse::CodePoint { name }) => tr(
            cx,
            ids::hv::GRID_SORT_TIP,
            ids::hv::HvGridSortTip {
                lvl,
                dir,
                collation: Text(name.to_string()),
            },
        ),
        Some(CollationUse::Fallback { name }) => tr(
            cx,
            ids::hv::GRID_SORT_TIP_CP,
            ids::hv::HvGridSortTipCp {
                lvl,
                dir,
                collation: Text(name.to_string()),
            },
        ),
        Some(CollationUse::Unknown) => tr(
            cx,
            ids::hv::GRID_SORT_TIP_UNKNOWN,
            ids::hv::HvGridSortTipUnknown { lvl, dir },
        ),
    }
}

// ---- the note row ---------------------------------------------------------------------------

/// What the sort note says (NOTES surface 18, M3c-20).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoteKind {
    /// A sort has run past [`SLOW_AFTER`]: "Sorting N rows…" + Cancel.
    Sorting { rows: u64 },
    /// A partial result, sorted: "Sorted locally … Only the N fetched rows are sorted." +
    /// Clear sort.
    Partial { sorted: u64 },
    /// A partial result with rows after the sort: both facts + Re-sort and Clear sort.
    PartialTail { sorted: u64, tail: u64 },
    /// A complete result with rows after the sort: "N new rows at the end (not sorted)" +
    /// Re-sort.
    Tail { tail: u64 },
}

/// The note for a grid whose slow job runs over `running` rows (`None`: none, or not yet
/// slow), whose shown sort ordered `sorted` rows, over `loaded` rows of a result that is
/// `partial` (the database may hold more).
pub fn note_kind(
    running: Option<u64>,
    sorted: Option<u64>,
    loaded: u64,
    partial: bool,
) -> Option<NoteKind> {
    if let Some(rows) = running {
        return Some(NoteKind::Sorting { rows });
    }
    let sorted = sorted?;
    let tail = loaded.saturating_sub(sorted);
    match (partial, tail) {
        (true, 0) => Some(NoteKind::Partial { sorted }),
        (true, tail) => Some(NoteKind::PartialTail { sorted, tail }),
        (false, 0) => None,
        (false, tail) => Some(NoteKind::Tail { tail }),
    }
}

/// Whether a result in `c` may have rows the store does not (a sort of it is partial).
fn partial(c: &Completeness) -> bool {
    !matches!(
        c,
        Completeness::Complete | Completeness::Truncated(TruncReason::FollowingResultSets)
    )
}

/// The sort note of `grid` over `snapshot` of `adapter`'s result, when it has one.
pub(crate) fn note_row(
    grid: &Entity<GridView>,
    adapter: &GridAdapter,
    snapshot: &StoreSnapshot,
    cx: &App,
) -> Option<AnyElement> {
    let state = cx.try_global::<Sorts>()?.0.get(&grid.entity_id())?;
    if state.generation != adapter.generation() {
        return None;
    }
    let running = state.running.as_ref().filter(|r| r.slow).map(|r| r.rows);
    let sorted = state.shown.as_ref().map(|s| s.1);
    let kind = note_kind(
        running,
        sorted,
        snapshot.row_count(),
        partial(snapshot.completeness()),
    )?;
    Some(render_note(kind, grid, cx))
}

/// The note row (`raised`, padding 5/12, 12 px `text-2`; the arrow-up-down glyph, or the static
/// amber marker while sorting).
fn render_note(kind: NoteKind, grid: &Entity<GridView>, cx: &App) -> AnyElement {
    let t = Theme::get(cx).tokens;
    let c = t.colors;
    let locally = || tr(cx, ids::hv::GRID_SORTED_LOCALLY, NoArgs);
    let text: SharedString = match kind {
        NoteKind::Sorting { rows } => tr(
            cx,
            ids::hv::GRID_SORTING,
            ids::hv::HvGridSorting { rows: Count(rows) },
        ),
        NoteKind::Partial { sorted } => format!(
            "{} {}",
            locally(),
            tr(
                cx,
                ids::hv::GRID_SORTED_LOCALLY_MORE,
                ids::hv::HvGridSortedLocallyMore {
                    rows: Count(sorted)
                },
            )
        )
        .into(),
        NoteKind::PartialTail { sorted, tail } => format!(
            "{} {}",
            locally(),
            tr(
                cx,
                ids::hv::GRID_SORTED_TAIL,
                ids::hv::HvGridSortedTail {
                    sorted: Count(sorted),
                    rows: Count(tail),
                },
            )
        )
        .into(),
        NoteKind::Tail { tail } => tr(
            cx,
            ids::hv::GRID_UNSORTED_TAIL,
            ids::hv::HvGridUnsortedTail { rows: Count(tail) },
        ),
    };
    let lead = match kind {
        NoteKind::Sorting { .. } => div()
            .debug_selector(|| "results-sort-marker".to_string())
            .mt(px(4.0))
            .size(px(7.0))
            .flex_none()
            .bg(c.amber.to_hsla())
            .into_any_element(),
        _ => div()
            .pt(px(2.0))
            .child(
                Icon::new(IconName::ArrowUpDown)
                    .size(IconSize::Chrome)
                    .color(c.text_2.to_hsla()),
            )
            .into_any_element(),
    };
    let button =
        |id: &'static str, label: SharedString, action: fn(&Entity<GridView>, &mut App)| {
            let grid = grid.clone();
            Button::new(id, label)
                .kind(ButtonKind::Default)
                .on_raised(true)
                .compact(true)
                .on_click(move |_, _: &mut Window, cx| action(&grid, cx))
        };
    let mut buttons = div().flex().flex_row().flex_none().gap(px(6.0));
    match kind {
        NoteKind::Sorting { .. } => {
            buttons = buttons.child(button(
                "results-sort-cancel",
                tr(cx, ids::hv::GRID_SORT_CANCEL, NoArgs),
                cancel,
            ));
        }
        NoteKind::Partial { .. } => {
            buttons = buttons.child(button(
                "results-sort-clear",
                tr(cx, ids::hv::GRID_CLEAR_SORT, NoArgs),
                clear,
            ));
        }
        NoteKind::PartialTail { .. } => {
            buttons = buttons
                .child(button(
                    "results-sort-resort",
                    tr(cx, ids::hv::GRID_RESORT, NoArgs),
                    resort,
                ))
                .child(button(
                    "results-sort-clear",
                    tr(cx, ids::hv::GRID_CLEAR_SORT, NoArgs),
                    clear,
                ));
        }
        NoteKind::Tail { .. } => {
            buttons = buttons.child(button(
                "results-sort-resort",
                tr(cx, ids::hv::GRID_RESORT, NoArgs),
                resort,
            ));
        }
    }
    div()
        .debug_selector(|| "results-sort-note-row".to_string())
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
        .child(lead)
        .child(
            div().flex_1().min_w(px(0.0)).pt(px(2.0)).child(
                paragraph(
                    text,
                    TextSource::Fluent,
                    t.type_ramp.caption,
                    c.text_2.to_hsla(),
                )
                .id("results-sort-note"),
            ),
        )
        .child(buttons)
        .into_any_element()
}

// ---- the commands -----------------------------------------------------------------------------

/// The grid the shown tile's active result has (the `enabled` checks get no window).
fn shown_grid(cx: &App) -> Option<Entity<GridView>> {
    let (console, _, ix) = super::view::active_entry(cx)?;
    super::view::hosts(cx)
        .into_iter()
        .find_map(|h| h.read(cx).grid_of(console, ix, cx))
}

/// Sorts the grid `window` shows by its focus column.
fn sort_focus_column(desc: bool, window: &mut Window, cx: &mut App) {
    let Some(grid) = super::view::active_grid(window, cx) else {
        return;
    };
    let Some(focus) = grid.read(cx).selection().focus() else {
        return;
    };
    grid.update(cx, |g, cx| {
        g.request_sort(SortSpec::by(focus.col, desc), cx)
    });
}

/// Registers the sort commands (palette-only: no keymap row; Task 39's menus offer them too).
pub fn register(cx: &mut App) {
    let reg = cx.default_global::<ActionRegistry>();
    for (id, title_id) in COMMANDS {
        let (run, enabled): (hacivat_ui::actions::ActionFn, fn(&App) -> bool) = match id {
            "grid.sort_asc" => (
                Rc::new(|window, cx| sort_focus_column(false, window, cx)),
                |cx| shown_grid(cx).is_some_and(|g| g.read(cx).selection().focus().is_some()),
            ),
            "grid.sort_desc" => (
                Rc::new(|window, cx| sort_focus_column(true, window, cx)),
                |cx| shown_grid(cx).is_some_and(|g| g.read(cx).selection().focus().is_some()),
            ),
            _ => (
                Rc::new(|window, cx| {
                    if let Some(grid) = super::view::active_grid(window, cx) {
                        grid.update(cx, |g, cx| g.request_sort(SortSpec::default(), cx));
                    }
                }),
                |cx| {
                    shown_grid(cx).is_some_and(|g| {
                        !g.read(cx).sort().is_empty()
                            || status(&g, cx).is_some_and(|s| s.running.is_some())
                    })
                },
            ),
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

// ## m3c-t36: the sort notes' shots fixtures are listed for Task 43 (no `shots/m3c.rs` yet).
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_follows_completeness() {
        assert!(!partial(&Completeness::Complete));
        assert!(!partial(&Completeness::Truncated(
            TruncReason::FollowingResultSets
        )));
        assert!(partial(&Completeness::Truncated(TruncReason::ResultCap)));
        assert!(partial(&Completeness::Streaming));
        assert!(partial(&Completeness::Cancelled));
    }
}
