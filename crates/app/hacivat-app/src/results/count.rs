//! Σ Count rows (HV-RES-10; M3b Task 25): the result toolbar's count of the statement's rows
//! (estimate vs exact, a cancellable spinner, the refusal reason).
//!
//! ## m3b-t01b-seams: the slot (Task 1b). STUB: renders nothing, so the result toolbar stays
//! M3a's. Task 25 fills it.
//!
//! ## m3b-t25-results (R-M3-76; design/m3b-safe surface 10)
//! - **What is counted.** The statement behind the tab, as it ran (the run UI keeps each tab's
//!   unit, `ConsoleExec::tab_units`), lexed alone under the console's known SQL context and
//!   wrapped by `hacivat_exec::count_sql`: one `SELECT`, `WITH`, `VALUES` or `TABLE` statement
//!   without a locking clause, `INTO` or a parameter. Anything else, a context not read yet or
//!   a statement the run UI no longer knows is "This query can't be counted": nothing is sent.
//! - **How.** A separate, explicit query only this button sends: `ExecService::count` (Task 16)
//!   classifies it again and refuses it unless the read-only check allows it, then runs it on a
//!   pooled `Service` session (on the console when it holds the user's transaction), for at most
//!   30 s, cancellable. The console's guard applies to it as to any statement.
//! - **What it shows** beside the button: nothing until asked; while counting the static amber
//!   run marker, "Counting…" and a ghost Cancel (design ruling 21: no spinner, no ambient
//!   motion); then "1,048,213 rows (exact)" in mono `text`, or why there is no number. A
//!   planner's estimate is shown only when the count service supplies one (it does not yet).
//!   The button is disabled while counting. A re-run of the tab forgets its count.
//! - **The count runs through [`Counter`]**, `ExecService::count` unless a test replaced it
//!   ([`set_counter`]).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use hacivat_exec::{CountError, CountRequest, ExecUnit, count_sql};
use hacivat_l10n::{Count, NoArgs, ids};
use hacivat_proto::DbmsKind;
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::icon::IconName;
use hacivat_ui::popover::Tip;
use hacivat_ui::text::{Label, TextFn, TextSource};
use hacivat_ui::toolbar::ToolbarItem;
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, Global, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};
use hacivat_ui_core::theme::Theme;
use hacivat_values::BindValue;
use sql_split::SqlDocument;
use tokio::sync::oneshot;

use crate::console::{ConsoleId, Consoles};
use crate::exec_ui::context::SqlContext;
use crate::l10n_global::tr;
use crate::results::tabs::{ActiveResult, CountState};

// ## m3b-t25-results

/// One count to run: the console it counts for, its engine, the count statement and its
/// binds.
#[derive(Debug, Clone)]
pub struct CountJob {
    pub console: ConsoleId,
    pub engine: DbmsKind,
    pub sql: String,
    pub binds: Arc<[BindValue]>,
}

/// A count under way: its answer, and how to cancel it.
pub struct CountHandle {
    pub done: oneshot::Receiver<Result<u64, CountError>>,
    pub cancel: Rc<dyn Fn()>,
}

/// Runs a count, or says why it can't (a `hv-results-count-*` Fluent ID; nothing sent).
pub type Counter = Rc<dyn Fn(CountJob, &mut App) -> Result<CountHandle, &'static str>>;

/// The counter in place of `ExecService::count`.
struct CounterHook(Counter);

impl Global for CounterHook {}

/// Counts with `counter` from now on (tests; a later task's own service).
pub fn set_counter(cx: &mut App, counter: Counter) {
    cx.set_global(CounterHook(counter));
}

/// The Fluent IDs of a count's refusals.
const UNAVAILABLE: &str = "hv-results-count-unavailable";
const REFUSED: &str = "hv-results-count-refused";
const TIMED_OUT: &str = "hv-results-count-timeout";
const FAILED: &str = "hv-results-count-failed";
const OFFLINE: &str = "hv-results-count-offline";

/// The count statement of console `console`'s statement `unit` and its engine, or why there
/// is none (module docs).
fn count_statement(
    console: ConsoleId,
    unit: hacivat_session::session::UnitId,
    cx: &App,
) -> Result<(String, DbmsKind), &'static str> {
    let exec = crate::exec_ui::console_exec(cx, console).ok_or(UNAVAILABLE)?;
    let (unit, _) = exec.tab_units.get(&unit).ok_or(UNAVAILABLE)?;
    let SqlContext::Known(ctx) = exec.context else {
        return Err(UNAVAILABLE);
    };
    let text = unit.text.to_string();
    let len = text.len();
    let flavor = hacivat_sql_editor::effective_flavor(&ctx, sql_dialect::ClientFlavor::None);
    let doc = SqlDocument::new(text, ctx, flavor).snapshot();
    let alone = ExecUnit {
        doc_range: 0..len,
        ..unit.clone()
    };
    let sql = count_sql(&doc, &alone).ok_or(UNAVAILABLE)?;
    let engine = crate::exec_ui::orchestrator::profile_of(cx, console)
        .map_or(DbmsKind::Unknown, |p| p.engine);
    Ok((sql, engine))
}

/// `ExecService::count` on the console's live session, its connection's pool when it has one.
fn exec_counter(job: CountJob, cx: &mut App) -> Result<CountHandle, &'static str> {
    let session = crate::exec_ui::orchestrator::live_session(cx, job.console).ok_or(OFFLINE)?;
    let sessions = Consoles::get(cx, job.console)
        .and_then(|s| s.profile)
        .and_then(|p| {
            cx.try_global::<crate::connections::ConnectionsModel>()?
                .manager()
                .sessions(p)
        });
    let service = crate::exec_ui::exec_service(cx);
    let ticket = service.count(CountRequest {
        console: session,
        sessions,
        engine: job.engine,
        sql: job.sql,
        binds: job.binds,
    });
    let ticket = Rc::new(RefCell::new(ticket));
    let done = std::mem::replace(&mut ticket.borrow_mut().done, oneshot::channel().1);
    Ok(CountHandle {
        done,
        cancel: Rc::new(move || ticket.borrow().cancel()),
    })
}

/// What a count's answer shows.
fn state_of(answer: Result<Result<u64, CountError>, oneshot::error::RecvError>) -> CountState {
    match answer {
        Ok(Ok(n)) => CountState::Exact(n),
        Ok(Err(CountError::Cancelled)) => CountState::Idle,
        Ok(Err(CountError::Refused(_))) => CountState::Refused(REFUSED),
        Ok(Err(CountError::TimedOut)) => CountState::Refused(TIMED_OUT),
        Ok(Err(CountError::Failed(_))) | Err(_) => CountState::Refused(FAILED),
    }
}

/// Count rows on console `console`'s result tab `ix` (the button): nothing while it counts.
pub fn start(console: ConsoleId, ix: usize, cx: &mut App) {
    let Some(results) = Consoles::get(cx, console).and_then(|s| s.results.clone()) else {
        return;
    };
    let (unit, tab_id, counting) = {
        let r = results.read(cx);
        let Some(entry) = r.tab(ix) else {
            return;
        };
        (
            entry.unit(),
            r.tab_id(ix),
            matches!(entry.count(), CountState::Counting { .. }),
        )
    };
    let Some(tab_id) = tab_id else {
        return;
    };
    if counting {
        return;
    }
    let (sql, engine) = match count_statement(console, unit, cx) {
        Ok(found) => found,
        Err(reason) => {
            results.update(cx, |r, cx| r.set_count(ix, CountState::Refused(reason), cx));
            return;
        }
    };
    let job = CountJob {
        console,
        engine,
        sql,
        binds: Arc::from(Vec::new()),
    };
    let counter = cx.try_global::<CounterHook>().map(|h| h.0.clone());
    let handle = match counter {
        Some(counter) => counter(job, cx),
        None => exec_counter(job, cx),
    };
    let handle = match handle {
        Ok(h) => h,
        Err(reason) => {
            results.update(cx, |r, cx| r.set_count(ix, CountState::Refused(reason), cx));
            return;
        }
    };
    let CountHandle { done, cancel } = handle;
    let Some(ticket) = results.update(cx, |r, cx| r.start_count(ix, cancel, cx)) else {
        return;
    };
    let weak = results.downgrade();
    cx.spawn(async move |cx| {
        let state = state_of(done.await);
        cx.update(|cx| {
            let _ = weak.update(cx, |r, cx| r.finish_count(tab_id, ticket, state, cx));
        });
    })
    .detach();
}

/// Cancels the count on console `console`'s result tab `ix` (the ghost Cancel).
pub fn cancel(console: ConsoleId, ix: usize, cx: &mut App) {
    if let Some(results) = Consoles::get(cx, console).and_then(|s| s.results.clone()) {
        results.update(cx, |r, cx| r.cancel_count(ix, cx));
    }
}

/// The toolbar item of console `console`'s active result: what its count says beside the
/// button (counting, exact, or why there is none); `None` before a count was asked.
pub fn render(console: ConsoleId, cx: &App) -> Option<AnyElement> {
    let results = Consoles::get(cx, console)?.results.clone()?;
    let r = results.read(cx);
    let ActiveResult::Tab(ix) = r.active() else {
        return None;
    };
    status(console, ix, r.tab(ix)?.count(), cx)
}

/// What count `state` of tab `ix` shows beside the button.
fn status(console: ConsoleId, ix: usize, state: CountState, cx: &App) -> Option<AnyElement> {
    let t = Theme::get(cx).tokens;
    let c = t.colors;
    match state {
        CountState::Idle => None,
        CountState::Counting { .. } => Some(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .debug_selector(|| "results-count-marker".to_string())
                        .size(px(7.0))
                        .flex_none()
                        .bg(c.amber.to_hsla()),
                )
                .child(
                    Label::new(
                        tr(cx, ids::hv::RESULTS_COUNTING, NoArgs),
                        TextSource::Fluent,
                        t.type_ramp.caption,
                        c.text_2.to_hsla(),
                    )
                    .id("results-count-state"),
                )
                .child(
                    Button::new(
                        "results-count-cancel",
                        tr(cx, ids::hv::RESULTS_COUNT_CANCEL, NoArgs),
                    )
                    .kind(ButtonKind::Ghost)
                    .on_raised(true)
                    .compact(true)
                    .on_click(move |_, _, cx| cancel(console, ix, cx)),
                )
                .into_any_element(),
        ),
        CountState::Exact(rows) => Some(
            Label::new(
                tr(
                    cx,
                    ids::hv::RESULTS_COUNT_EXACT,
                    ids::hv::HvResultsCountExact { rows: Count(rows) },
                ),
                TextSource::Fluent,
                t.type_ramp.mono_kbd,
                c.text.to_hsla(),
            )
            .id("results-count-state")
            .into_any_element(),
        ),
        CountState::Refused(reason) => Some(
            Label::new(
                crate::l10n_global::tr_id(cx, reason),
                TextSource::Fluent,
                t.type_ramp.caption,
                c.text_2.to_hsla(),
            )
            .id("results-count-state")
            .into_any_element(),
        ),
    }
}

/// The result toolbar's Count rows items for console `console`'s tab `ix`: the ghost Σ button
/// (disabled while counting), then its state (pinned: a count is never cut).
pub fn toolbar_items(
    console: ConsoleId,
    ix: usize,
    state: CountState,
    cx: &App,
) -> Vec<ToolbarItem> {
    let text: SharedString = tr(cx, ids::hv::RESULTS_COUNT, NoArgs);
    let tip = tr(cx, ids::hv::RESULTS_COUNT_TIP, NoArgs);
    let label: TextFn = {
        let text = text.clone();
        Rc::new(move |_| text.clone())
    };
    let counting = matches!(state, CountState::Counting { .. });
    let button = ToolbarItem::new(
        1,
        Rc::new(move |_, _| {
            let tip = tip.clone();
            div()
                .id("results-count-tip")
                .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
                .child(
                    Button::new("results-count", text.clone())
                        .kind(ButtonKind::Ghost)
                        .icon(IconName::Sigma)
                        .on_raised(true)
                        .disabled(counting)
                        .on_click(move |_, _, cx| start(console, ix, cx)),
                )
                .into_any_element()
        }),
        label,
    );
    let button = if counting {
        button
    } else {
        button.on_choose(move |_, cx| start(console, ix, cx))
    };
    let mut items = vec![button];
    if state != CountState::Idle {
        let words: TextFn = Rc::new(move |cx| match state {
            CountState::Exact(rows) => tr(
                cx,
                ids::hv::RESULTS_COUNT_EXACT,
                ids::hv::HvResultsCountExact { rows: Count(rows) },
            ),
            CountState::Counting { .. } => tr(cx, ids::hv::RESULTS_COUNTING, NoArgs),
            CountState::Refused(id) => crate::l10n_global::tr_id(cx, id),
            CountState::Idle => SharedString::default(),
        });
        items.push(ToolbarItem::pinned(
            Rc::new(move |_, cx| {
                status(console, ix, state, cx).unwrap_or_else(|| div().into_any_element())
            }),
            words,
        ));
    }
    items
}
