//! The error-policy bar (HV-EXEC-08, R-M3-61; M3b Task 23, `design/m3b-safe` surface 7): a
//! failed unit under the Ask policy offers Retry, Ignore, Ignore all, Stop and (in a
//! transaction) Roll back and continue.
//!
//! ## m3b-t01b-seams: the seams (Task 1b; P-M3b-17). [`on_choice`] is what Task 24's
//! orchestrator calls on every run event; Task 23 fills it in this file.
//!
//! **One answer per question.** A `RunEvent::NeedsChoice` carries its question id; the
//! session takes exactly one answer to it (`SessionHandle::choose(run_seq, question, c)`, a
//! second is refused). The bar answers each question at most once: the question is noted
//! before the answer goes out, and a question already noted is never answered again; a stale
//! or duplicate answer returns `false` ([`answer`]).
//!
//! ## m3b-t23-escalation
//!
//! - **The bar.** Inline at the foot of the editor tile, above the seam to the results, never a
//!   modal: 32 px on raised with a seam above, `role=alert`; the ink triangle-alert glyph,
//!   "Statement 2 of 5 failed" (12.5/600), the SQLSTATE chip, "3 more wait" (text-2), then
//!   Retry, Ignore, Ignore all, Stop (Default buttons). Inside a failed PostgreSQL transaction
//!   (`rollback_and_continue`, P-M3b-19): "…, and the transaction failed with it", **Roll back
//!   and continue** in place of Ignore and no Retry; its tooltip counts the user's whole open
//!   transaction and says how the rest runs from the live mode. A narrow pane drops "n more
//!   wait" first, then wraps; buttons never truncate.
//! - **The focus.** The bar never takes it by itself (typing must not answer it): F6 moves it to
//!   the bar's first button ([`focus_cycle`]); Tab and Shift-Tab cycle the buttons in visual
//!   order (Retry → Stop); **Escape answers Stop** and never cancels anything running; Enter
//!   acts on the focused button.
//! - **While a question waits** the console's transaction badge, its idle toast and the
//!   palette's Commit and Roll back wait (`tx_badge::set_choice_pending`); it is cleared when
//!   the question is answered or the run ends.
//! - **Closing the console answers Stop** ([`console_closed`]).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tokio::sync::oneshot;

use hacivat_exec::{LogMsg, RunChoice};
use hacivat_l10n::ArgValue;
use hacivat_session::session::{ExecError, RunEvent, RunId, SessionHandle, TxMode, UnitId};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::popover::{A11yNode, Tip};
use hacivat_ui::text::{Label, TextSource, paragraph};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, FocusHandle, Global, InteractiveElement as _, IntoElement, ParentElement as _,
    Role, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use hacivat_ui_core::theme::Theme;

use super::marks::message;
use super::{answered, console_exec};
use crate::console::tx_badge::{self, TxView};
use crate::console::{ConsoleId, Consoles};

/// What the bar asks about: the failed unit, the question's id (the answer names it), its
/// error, and whether Roll back and continue is offered (a transaction is open; Retry then
/// hides, P-M3b-19).
#[derive(Debug, Clone)]
pub struct ErrorAsk {
    pub unit: UnitId,
    pub question: u64,
    pub error: ExecError,
    pub rollback_and_continue: bool,
}

/// The questions answered, per console, of its current run (`(console, run_seq, question)`).
#[derive(Default)]
struct Answered(HashSet<(ConsoleId, u64, u64)>);

impl Global for Answered {}

/// Notes question `question` of run `run_seq` on `console` as answered; `false` when it already
/// was. Questions of the console's earlier runs are forgotten.
pub(crate) fn first_answer(cx: &mut App, console: ConsoleId, run_seq: u64, question: u64) -> bool {
    let answered = &mut cx.default_global::<Answered>().0;
    answered.retain(|(c, seq, _)| *c != console || *seq == run_seq);
    answered.insert((console, run_seq, question))
}

// ## m3b-t23-escalation

/// Where a bar's answer goes.
enum Reply {
    /// The session's run gate (`SessionHandle::choose`).
    Session(SessionHandle),
    /// [`show`]'s caller.
    Channel(Option<oneshot::Sender<RunChoice>>),
    /// A shots fixture: nowhere.
    #[cfg(feature = "shots")]
    Nowhere,
}

/// One console's open question.
struct Bar {
    run_seq: u64,
    ask: ErrorAsk,
    /// The failed statement's number in the run, and the run's statements.
    index: u32,
    total: u32,
    /// The user's open transaction's statements, and whether the session runs in Auto-commit
    /// (Roll back and continue's tooltip).
    tx_statements: u32,
    auto: bool,
    reply: Reply,
    /// The buttons, in visual (and Tab) order.
    buttons: Vec<(RunChoice, FocusHandle)>,
    _end: Option<hacivat_ui_core::gpui_surface::Task<()>>,
}

/// The open bars, by console.
#[derive(Default)]
struct Bars(HashMap<ConsoleId, Bar>, bool);

impl Global for Bars {}

/// The choices the bar offers for `ask`, in visual order: Retry, Ignore, Ignore all, Stop; in
/// a failed PostgreSQL transaction Roll back and continue, Ignore all, Stop.
pub fn offered(rollback_and_continue: bool) -> Vec<RunChoice> {
    if rollback_and_continue {
        vec![
            RunChoice::RollbackAndContinue,
            RunChoice::IgnoreAll,
            RunChoice::Stop,
        ]
    } else {
        vec![
            RunChoice::Retry,
            RunChoice::Ignore,
            RunChoice::IgnoreAll,
            RunChoice::Stop,
        ]
    }
}

/// Opens console `console`'s bar for `ask` (`index` of `total`), answered through `reply`.
fn open(
    console: ConsoleId,
    run_seq: u64,
    ask: ErrorAsk,
    (index, total): (u32, u32),
    reply: Reply,
    cx: &mut App,
) {
    let snap = Consoles::get(cx, console)
        .and_then(|s| s.session.as_ref())
        .map(|h| h.state().borrow().clone());
    let tx_statements = snap
        .as_ref()
        .and_then(|s| TxView::of(s).user)
        .map_or(0, |u| u.statements);
    let auto = snap.as_ref().is_none_or(|s| s.tx.mode == TxMode::Auto);
    let buttons = offered(ask.rollback_and_continue)
        .into_iter()
        .map(|c| (c, cx.focus_handle().tab_stop(true)))
        .collect();
    // The run's end takes the bar away (a cancel while it waits answers Stop in the session).
    let end = console_exec(cx, console)
        .and_then(|e| e.run.as_ref())
        .filter(|r| r.seq == run_seq)
        .map(|r| r.status.clone())
        .map(|mut status| {
            let question = ask.question;
            cx.spawn(async move |cx| {
                let _ = status.wait_for(|s| s.summary.is_some()).await;
                cx.update(|cx| close(console, run_seq, question, cx));
            })
        });
    // Closing the console answers Stop, however it closes (the registry forgets it).
    if !std::mem::replace(&mut cx.default_global::<Bars>().1, true) {
        cx.observe_global::<Consoles>(answer_closed).detach();
    }
    let old = cx.default_global::<Bars>().0.insert(
        console,
        Bar {
            run_seq,
            ask,
            index,
            total,
            tx_statements,
            auto,
            reply,
            buttons,
            _end: end,
        },
    );
    if let Some(old) = old {
        answer_dropped(old);
    }
    tx_badge::set_choice_pending(console, true, cx);
    cx.refresh_windows();
}

/// A bar that went without an answer answers its caller Stop (the session's own question is
/// answered `Stop` by the session when the run goes).
fn answer_dropped(bar: Bar) {
    if let Reply::Channel(Some(tx)) = bar.reply {
        let _ = tx.send(RunChoice::Stop);
    }
}

/// Closes console `console`'s bar for question `question` of run `run_seq` (the run ended),
/// without an answer.
fn close(console: ConsoleId, run_seq: u64, question: u64, cx: &mut App) {
    let bars = &mut cx.default_global::<Bars>().0;
    if bars
        .get(&console)
        .is_some_and(|b| b.run_seq == run_seq && b.ask.question == question)
        && let Some(bar) = bars.remove(&console)
    {
        answer_dropped(bar);
        tx_badge::set_choice_pending(console, false, cx);
        cx.refresh_windows();
    }
}

/// The bar's statement numbers for `ask` on console `console`: its unit's number and the run's
/// statements (the console's run, when `ask` is about it).
fn index_of(console: ConsoleId, ask: &ErrorAsk, cx: &App) -> (u32, u32) {
    let i = ask.unit.ordinal.saturating_add(1);
    let n = console_exec(cx, console)
        .and_then(|e| e.run.as_ref())
        .filter(|r| r.run == ask.unit.run)
        .map_or(i, |r| u32::try_from(r.units.len()).unwrap_or(u32::MAX));
    (i, n.max(i))
}

/// Shows the bar for `ask` on console `console`; the answer comes back through the receiver.
/// Without the console's run UI it answers `Stop` at once (the safe value: the run ends).
pub fn show(console: ConsoleId, ask: ErrorAsk, cx: &mut App) -> oneshot::Receiver<RunChoice> {
    if console_exec(cx, console).is_none() || super::workspace_window(cx).is_none() {
        return answered(RunChoice::Stop);
    }
    let (tx, rx) = oneshot::channel();
    let index = index_of(console, &ask, cx);
    let run_seq = console_exec(cx, console)
        .and_then(|e| e.run.as_ref())
        .filter(|r| r.run == ask.unit.run)
        .map_or(0, |r| r.seq);
    open(console, run_seq, ask, index, Reply::Channel(Some(tx)), cx);
    rx
}

/// A run event of console `console`'s run (Task 24's orchestrator calls it for every event).
/// A `NeedsChoice` of the console's current run shows the bar, once per question; any other
/// event, a question of another run, or a question already shown or answered changes nothing.
pub fn on_choice(console: ConsoleId, ev: &RunEvent, cx: &mut App) {
    let Some(run) = console_exec(cx, console).and_then(|e| e.run.as_ref()) else {
        return;
    };
    let Some((run_seq, question)) = choice_target(ev, Some((run.run, run.seq))) else {
        return;
    };
    let session = run.session.clone();
    let RunEvent::NeedsChoice {
        unit,
        error,
        rollback_and_continue,
        ..
    } = ev
    else {
        return;
    };
    let shown = cx
        .try_global::<Bars>()
        .and_then(|b| b.0.get(&console))
        .is_some_and(|b| b.run_seq == run_seq && b.ask.question == question);
    let done = cx
        .try_global::<Answered>()
        .is_some_and(|a| a.0.contains(&(console, run_seq, question)));
    if shown || done {
        return;
    }
    let ask = ErrorAsk {
        unit: *unit,
        question,
        error: error.clone(),
        rollback_and_continue: *rollback_and_continue,
    };
    let index = index_of(console, &ask, cx);
    open(console, run_seq, ask, index, Reply::Session(session), cx);
}

/// The question `ev` asks of the console's current run `current` (`(run, run_seq)`), as
/// `(run_seq, question)`: only a `NeedsChoice` whose unit belongs to that run; `None` for any
/// other event, a question of an older run, or a console without a run.
fn choice_target(ev: &RunEvent, current: Option<(RunId, u64)>) -> Option<(u64, u64)> {
    let RunEvent::NeedsChoice { unit, question, .. } = ev else {
        return None;
    };
    let (run, run_seq) = current?;
    (unit.run == run).then_some((run_seq, *question))
}

/// Answers console `console`'s open question with `choice`: `false` when no question is open,
/// the choice is not one the bar offers, or the question was already answered (a stale or
/// duplicate answer). The answer goes to the session (or [`show`]'s caller) once; the bar
/// closes and the badge's Commit and Roll back no longer wait.
// ## m3b-t23-escalation (fix round 1, m2): the answer names its question.
pub fn answer(console: ConsoleId, question: u64, choice: RunChoice, cx: &mut App) -> bool {
    let Some(bar) = cx.try_global::<Bars>().and_then(|b| b.0.get(&console)) else {
        return false;
    };
    if bar.ask.question != question || !offered(bar.ask.rollback_and_continue).contains(&choice) {
        return false;
    }
    let (run_seq, question) = (bar.run_seq, bar.ask.question);
    let Some(bar) = cx.default_global::<Bars>().0.remove(&console) else {
        return false;
    };
    tx_badge::set_choice_pending(console, false, cx);
    cx.refresh_windows();
    if !first_answer(cx, console, run_seq, question) {
        return false;
    }
    let line = match choice {
        RunChoice::Stop => Some(LogMsg::RunStopped { n: bar.index }),
        RunChoice::RollbackAndContinue => Some(LogMsg::RolledBackContinue {
            n: bar.index.saturating_add(1),
        }),
        _ => None,
    };
    let unit = bar.ask.unit;
    match bar.reply {
        // Fix round 1 (m1): the line only once the session took the answer (a run that
        // ended meanwhile refuses it, and nothing happened).
        Reply::Session(session) => {
            let taken = session.choose(run_seq, question, choice);
            cx.spawn(async move |cx| {
                if taken.await == Ok(true)
                    && let Some(line) = line
                {
                    cx.update(|cx| log_line(console, line, unit, cx));
                }
            })
            .detach();
        }
        Reply::Channel(Some(tx)) => {
            let _ = tx.send(choice);
        }
        Reply::Channel(None) => {}
        #[cfg(feature = "shots")]
        Reply::Nowhere => {}
    }
    true
}

/// Appends `line` (about `unit`) to console `console`'s Output.
fn log_line(console: ConsoleId, line: LogMsg, unit: UnitId, cx: &mut App) {
    if let Some(state) = Consoles::get(cx, console) {
        state.log.append(line, None, Some(unit));
        if let Some(results) = state.results.clone() {
            results.update(cx, |r, cx| r.note_output(cx));
        }
    }
}

/// Console `console` closes: its open question is answered Stop (R-M3-61).
pub fn console_closed(console: ConsoleId, cx: &mut App) {
    if let Some(question) = cx
        .try_global::<Bars>()
        .and_then(|b| b.0.get(&console))
        .map(|b| b.ask.question)
    {
        answer(console, question, RunChoice::Stop, cx);
    }
}

/// The registry changed: the questions of the consoles it no longer has are answered Stop.
fn answer_closed(cx: &mut App) {
    let gone: Vec<ConsoleId> = cx
        .try_global::<Bars>()
        .map(|b| {
            b.0.keys()
                .copied()
                .filter(|c| Consoles::get(cx, *c).is_none())
                .collect()
        })
        .unwrap_or_default();
    for console in gone {
        console_closed(console, cx);
    }
}

/// The question console `console`'s bar shows: `(run_seq, question, choices offered)` (tests,
/// and the console's Run, which waits while it is open).
pub fn open_question(console: ConsoleId, cx: &App) -> Option<(u64, u64, Vec<RunChoice>)> {
    let bar = cx.try_global::<Bars>()?.0.get(&console)?;
    Some((
        bar.run_seq,
        bar.ask.question,
        offered(bar.ask.rollback_and_continue),
    ))
}

/// F6 (`workspace.focus_notification`, M6): with a bar on the focused console, the first F6
/// moves the focus to its first button and the next one leaves it (`false`: the toasts get it,
/// as without a bar). `true` when the bar took the focus.
pub fn focus_cycle(window: &mut Window, cx: &mut App) -> bool {
    let Some(console) = super::cancel::focused_console(cx) else {
        return false;
    };
    let Some(bar) = cx.try_global::<Bars>().and_then(|b| b.0.get(&console)) else {
        return false;
    };
    // Fix round 1 (I3): from the bar, the toasts; without a sticky toast, the editor (the
    // focus never stays trapped in the bar). A toast entered from here gives it back to the
    // editor.
    if bar.buttons.iter().any(|(_, h)| h.is_focused(window)) {
        focus_editor(console, window, cx);
        hacivat_ui::toast::focus_newest_sticky(window, cx);
        return true;
    }
    let Some((_, first)) = bar.buttons.first() else {
        return false;
    };
    let first = first.clone();
    window.focus(&first, cx);
    true
}

/// Gives the focus back to console `console`'s editor (after the bar's own key answered it).
fn focus_editor(console: ConsoleId, window: &mut Window, cx: &mut App) {
    if let Some(editor) = Consoles::get(cx, console).and_then(|s| s.editor.clone()) {
        let handle = hacivat_ui_core::gpui_surface::Focusable::focus_handle(editor.read(cx), cx);
        window.focus(&handle, cx);
    }
}

/// The words of `choice`'s button.
fn label_id(choice: RunChoice) -> &'static str {
    match choice {
        RunChoice::Retry => "hv-errbar-retry",
        RunChoice::Ignore => "hv-errbar-ignore",
        RunChoice::IgnoreAll => "hv-errbar-ignore-all",
        RunChoice::Stop => "hv-errbar-stop",
        RunChoice::RollbackAndContinue => "hv-errbar-rollback-continue",
    }
}

fn element_id(choice: RunChoice) -> &'static str {
    match choice {
        RunChoice::Retry => "errbar-retry",
        RunChoice::Ignore => "errbar-ignore",
        RunChoice::IgnoreAll => "errbar-ignore-all",
        RunChoice::Stop => "errbar-stop",
        RunChoice::RollbackAndContinue => "errbar-rollback-continue",
    }
}

/// Console `console`'s bar, while a question is open: the console view draws it at the foot
/// of the editor tile.
pub fn render(console: ConsoleId, cx: &App) -> Option<AnyElement> {
    let bar = cx.try_global::<Bars>()?.0.get(&console)?;
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let c = &t.colors;
    let num = |n: u32| ArgValue::Text(n.to_string());
    let title_id = if bar.ask.rollback_and_continue {
        "hv-errbar-aborted"
    } else {
        "hv-errbar-failed"
    };
    let title = message(
        cx,
        title_id,
        vec![("i", num(bar.index)), ("n", num(bar.total))],
    );
    let waiting = bar.total.saturating_sub(bar.index);
    let code: Option<SharedString> = bar.ask.error.server.as_ref().and_then(|s| {
        s.sqlstate
            .clone()
            .or_else(|| s.vendor_code.map(|v| v.to_string()))
            .map(SharedString::from)
    });
    let head = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_x(px(t.spacing.sm))
        .gap_y(px(t.spacing.xs))
        .flex_1()
        .min_w(px(0.0))
        // The sentence wraps rather than clip in a narrow pane (the bar grows).
        .child(
            div().max_w_full().child(
                paragraph(
                    title.clone(),
                    TextSource::Fluent,
                    t.type_ramp.body_strong,
                    c.text.to_hsla(),
                )
                .id("errbar-title"),
            ),
        )
        .children(code.clone().map(|code| {
            div()
                .flex_none()
                .px(px(t.spacing.xs))
                .border_1()
                .border_color(c.seam_strong.to_hsla())
                .rounded(px(t.radii.xs))
                .child(
                    Label::new(
                        code,
                        TextSource::Data,
                        t.type_ramp.mono_data,
                        c.text.to_hsla(),
                    )
                    .id("errbar-code"),
                )
        }))
        .children((waiting > 0).then(|| {
            // Raised Ink Rule: text-2 on raised.
            Label::new(
                message(
                    cx,
                    "hv-errbar-waiting",
                    vec![("n", ArgValue::Count(u64::from(waiting)))],
                ),
                TextSource::Fluent,
                t.type_ramp.caption,
                c.text_2.to_hsla(),
            )
            .id("errbar-waiting")
        }));
    let tip = (bar.ask.rollback_and_continue).then(|| {
        let id = if bar.auto {
            "hv-errbar-rollback-continue-tip-auto"
        } else {
            "hv-errbar-rollback-continue-tip"
        };
        message(
            cx,
            id,
            vec![
                ("n", ArgValue::Count(u64::from(bar.tx_statements))),
                ("next", num(bar.index.saturating_add(1))),
            ],
        )
    });
    let mut buttons = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .flex_none()
        .items_center()
        .gap(px(t.spacing.sm));
    let question = bar.ask.question;
    for (choice, handle) in &bar.buttons {
        let choice = *choice;
        let button = Button::new(element_id(choice), message(cx, label_id(choice), vec![]))
            .kind(ButtonKind::Default)
            .compact(true)
            .on_raised(true)
            .focus_handle(handle.clone())
            .on_click(move |event, window, cx| {
                if event.click_count() > 1 {
                    return;
                }
                let had_focus = bar_focused(console, window, cx);
                if answer(console, question, choice, cx) && had_focus {
                    focus_editor(console, window, cx);
                }
            });
        buttons = match (&tip, choice) {
            (Some(tip), RunChoice::RollbackAndContinue) => {
                let tip = tip.clone();
                buttons.child(
                    div()
                        .id("errbar-rollback-continue-tip")
                        .child(button)
                        .tooltip(move |_, cx| Tip::view(tip.clone(), cx)),
                )
            }
            _ => buttons.child(button),
        };
    }
    let row = div()
        .id(("errbar", console.0))
        .debug_selector(|| "errbar".to_string())
        .flex()
        .flex_row()
        .flex_wrap()
        .flex_none()
        .items_center()
        .gap_x(px(t.spacing.lg))
        .gap_y(px(t.spacing.xs))
        .min_h(px(32.0))
        .px(px(t.spacing.lg))
        .py(px(t.spacing.xs))
        .bg(c.raised.to_hsla())
        .border_t_1()
        .border_color(c.seam.to_hsla())
        .on_key_down(move |event, window, cx| {
            let stroke = &event.keystroke;
            match stroke.key.as_str() {
                // Escape answers Stop; it never cancels anything running (R-M3-38).
                // Tab moves on in the window's own focus order (fix round 1, I3).
                "escape" if !stroke.modifiers.modified() => {
                    if answer(console, question, RunChoice::Stop, cx) {
                        focus_editor(console, window, cx);
                    }
                }
                _ => return,
            }
            cx.stop_propagation();
        })
        // The ink glyph stays beside the sentence when it wraps.
        .child(
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(t.spacing.sm))
                .flex_1()
                .min_w(px(0.0))
                .child(
                    div().flex_none().pt(px(2.0)).child(
                        Icon::new(IconName::TriangleAlert)
                            .size(IconSize::Chrome)
                            .color(c.text.to_hsla()),
                    ),
                )
                .child(head),
        )
        .child(buttons);
    let description = [Some(&title), code.as_ref()]
        .into_iter()
        .flatten()
        .map(SharedString::as_ref)
        .collect::<Vec<_>>()
        .join(" ");
    Some(
        A11yNode::new("errbar-node", Role::Alert, title, false, row)
            .description(description.into())
            .into_any_element(),
    )
}

/// Whether a button of console `console`'s bar has the focus.
fn bar_focused(console: ConsoleId, window: &Window, cx: &App) -> bool {
    cx.try_global::<Bars>()
        .and_then(|b| b.0.get(&console))
        .is_some_and(|b| b.buttons.iter().any(|(_, h)| h.is_focused(window)))
}

/// The bar's buttons' focus handles, in order, by choice (tests).
pub fn button_focus(console: ConsoleId, cx: &App) -> Vec<(RunChoice, FocusHandle)> {
    cx.try_global::<Bars>()
        .and_then(|b| b.0.get(&console))
        .map(|b| b.buttons.clone())
        .unwrap_or_default()
}

// ---- test seams ---------------------------------------------------------------------------

/// Runs `units` (their texts, in order) on console `console` under the Ask policy, as Task 24's
/// orchestrator will, and follows the run's questions into [`on_choice`] (feature
/// `test-support`: until Task 24 wires the policy, a test reaches the bar through this).
#[cfg(feature = "test-support")]
pub fn run_asking(console: ConsoleId, units: &[&str], cx: &mut App) -> Option<RunId> {
    use hacivat_exec::{ExecUnit, RunRequest};
    use hacivat_session::session::ErrorPolicy;
    use sql_split::exec_range::UnitKind;

    let state = Consoles::get(cx, console)?.clone();
    let session = state.session.clone()?;
    let editor = console_exec(cx, console)?.editor.upgrade()?;
    let engine = super::orchestrator::profile_of(cx, console)
        .map_or(hacivat_proto::DbmsKind::Unknown, |p| p.engine);
    let mut at = 0;
    let units: Vec<ExecUnit> = units
        .iter()
        .map(|text| {
            let range = at..at + text.len();
            at = range.end + 1;
            ExecUnit {
                text: Arc::from(*text),
                doc_range: range,
                kind: UnitKind::Statement,
                suspendable: false,
                title: None,
            }
        })
        .collect();
    let service = super::exec_service(cx);
    let ticket = service.run(
        &session,
        RunRequest {
            console: console.0,
            engine,
            units: units.clone(),
            fetch: hacivat_results::FetchPolicy::default(),
            log: state.log.clone(),
            prepared: None,
            plan: None,
            policy: ErrorPolicy::Ask,
            run_as_one: false,
            history: None,
            confirm: None,
            // ## m3b-t24 merge reconciliation: a test-support run checks no context.
            context_generation: None,
        },
    );
    let version = editor.read(cx).snapshot().version;
    let keys: Vec<hacivat_ui_editor::MarkKey> = (0..units.len())
        .map(|i| super::marks::mark_key(ticket.run, i))
        .collect();
    editor.update(cx, |e, cx| {
        e.add_exec_marks(
            version,
            keys.iter()
                .copied()
                .zip(units.iter().map(|u| u.doc_range.clone()))
                .collect(),
            cx,
        );
    });
    let status_rx = ticket.status.clone();
    let status = super::marks::follow_status(console, ticket.status, keys.clone(), cx);
    let mut choices = ticket.choices;
    let questions = cx.spawn(async move |cx| {
        loop {
            let ev = choices.borrow_and_update().clone();
            if let Some((_, ev)) = ev {
                cx.update(|cx| on_choice(console, &ev, cx));
            }
            if choices.changed().await.is_err() {
                break;
            }
        }
    });
    let run = ticket.run;
    super::update_console_exec(cx, console, |e| {
        e.run = Some(super::ActiveRun {
            run,
            seq: ticket.seq,
            session,
            units,
            keys,
            ended: false,
            cancel_asked: false,
            status: status_rx,
            // ## m3b-t24 merge reconciliation
            policy: ErrorPolicy::Ask,
            _tasks: vec![status, questions],
        });
    });
    Some(run)
}

// ---- shots fixtures -----------------------------------------------------------------------

/// Shots fixtures for surface 7 (feature `shots`): the bar in its forms, answered nowhere.
#[cfg(feature = "shots")]
pub mod fixtures {
    use super::*;
    use hacivat_driver::ServerMessage;
    use hacivat_session::session::ExecErrorKind;

    /// Shows console `console`'s bar: statement `index` of `total` failed with `sqlstate`;
    /// `failed_tx`: inside a failed PostgreSQL transaction of `tx_statements` statements
    /// (`auto`: the session runs in Auto-commit).
    pub fn show(
        console: ConsoleId,
        (index, total): (u32, u32),
        sqlstate: &str,
        failed_tx: Option<(u32, bool)>,
        cx: &mut App,
    ) {
        let ask = ErrorAsk {
            unit: UnitId {
                run: RunId(0xF_FFFE),
                ordinal: index.saturating_sub(1),
            },
            question: 1,
            error: ExecError {
                kind: ExecErrorKind::Db,
                server: Some(ServerMessage {
                    sqlstate: Some(sqlstate.to_string()),
                    vendor_code: None,
                    severity: Some("ERROR".to_string()),
                    text: String::new(),
                    detail: None,
                    hint: None,
                    lang_hint: None,
                    position: None,
                }),
            },
            rollback_and_continue: failed_tx.is_some(),
        };
        open(console, 0, ask, (index, total), Reply::Nowhere, cx);
        if let Some((statements, auto)) = failed_tx
            && let Some(bar) = cx.default_global::<Bars>().0.get_mut(&console)
        {
            bar.tx_statements = statements;
            bar.auto = auto;
        }
    }

    /// Moves the focus to console `console`'s bar (F6).
    pub fn focus(window: &mut Window, cx: &mut App) -> bool {
        focus_cycle(window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn a_question_is_answered_once(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let c = ConsoleId(1);
            assert!(first_answer(cx, c, 7, 1));
            assert!(!first_answer(cx, c, 7, 1), "a second answer to question 1");
            assert!(first_answer(cx, c, 7, 2));
            // Another console's question of the same ids is its own.
            assert!(first_answer(cx, ConsoleId(2), 7, 1));
            // A new run forgets the questions of the console's earlier runs.
            assert!(first_answer(cx, c, 8, 1));
            assert!(!first_answer(cx, c, 8, 1));
        });
    }

    // ## m3b-t01b-seams (fix round 2, M3)

    fn needs_choice(run: u64, question: u64) -> RunEvent {
        RunEvent::NeedsChoice {
            unit: UnitId {
                run: RunId(run),
                ordinal: 0,
            },
            question,
            error: ExecError::of(hacivat_session::session::ExecErrorKind::Internal),
            rollback_and_continue: false,
        }
    }

    #[test]
    fn only_a_question_of_the_current_run_is_answered() {
        // The console's current run is run 5, sent as request 9.
        let current = Some((RunId(5), 9));
        assert_eq!(choice_target(&needs_choice(5, 3), current), Some((9, 3)));
        // A question of an older run of the console.
        assert_eq!(choice_target(&needs_choice(4, 3), current), None);
        // A console without a run (another console's event reaches nothing).
        assert_eq!(choice_target(&needs_choice(5, 3), None), None);
        // Any other event.
        let other = RunEvent::CommitDecided {
            unit: UnitId {
                run: RunId(5),
                ordinal: 0,
            },
            answer: hacivat_exec::CommitAnswer::RollBack,
            by_deadline: false,
        };
        assert_eq!(choice_target(&other, current), None);
    }

    #[gpui::test]
    fn an_unattached_console_answers_nothing(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let c = ConsoleId(3);
            on_choice(c, &needs_choice(5, 1), cx);
            // Nothing was noted: the question is still unanswered for any run of the console.
            assert!(cx.try_global::<Answered>().is_none_or(|a| a.0.is_empty()));
            assert!(open_question(c, cx).is_none());
            assert!(!answer(c, 1, RunChoice::Stop, cx), "no question is open");
            assert!(first_answer(cx, c, 9, 1));
        });
    }

    // ## m3b-t23-escalation

    #[test]
    fn a_failed_pg_transaction_offers_roll_back_and_continue_and_no_retry() {
        assert_eq!(
            offered(false),
            [
                RunChoice::Retry,
                RunChoice::Ignore,
                RunChoice::IgnoreAll,
                RunChoice::Stop
            ]
        );
        assert_eq!(
            offered(true),
            [
                RunChoice::RollbackAndContinue,
                RunChoice::IgnoreAll,
                RunChoice::Stop
            ]
        );
    }
}
