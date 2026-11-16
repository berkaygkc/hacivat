//! The confirm-in-transaction prompt (R-M3-55, D-014; M3b Task 21, `design/m3b-safe` surface
//! 2): a production change ran inside a transaction; Commit or Roll back (Keep or Undo) before
//! the deadline.
//!
//! ## m3b-t21-confirm
//!
//! - **The form follows `CommitQuestion::savepoint`** (the session's choice, P-M3b-1; RC-5): the
//!   savepoint form (inside an open transaction: Manual, Smart, or Auto inside the user's
//!   transaction) says Keep joins the change to the open transaction and Undo removes only this
//!   statement's change, the transaction staying open; the own form (Auto-commit, nothing open:
//!   Hacivat's own `BEGIN`) says Commit or Roll back. Confirm-in-transaction exists only on
//!   production (P-M3b-30), so there is no other variant.
//! - **The safe answer is the default**: Roll back / Undo is Primary with the initial focus;
//!   Commit / Keep is Default, never Danger (the destructive question, if any, was asked before
//!   the run). Enter acts on key down on the focused button only. **Escape, the close button,
//!   ⌘W (Ctrl+W), closing the window and quitting answer Roll back** (a prompt dropped without
//!   an answer answers it; the quit guard answers every open one).
//! - **The countdown** ("Rolls back in 4:32" / "Undoes this statement in 4:51") counts down to
//!   the question's deadline on the GPUI executor's clock and repaints once a second, when its
//!   `m:ss` value changes; no colour or weight changes near the end. At 0:00 the prompt answers
//!   Roll back and closes (the session's own deadline undoes the statement, D-014).
//! - **It follows the session** ([`CommitContext::follow`], the run's `RunTicket::choices`): once
//!   its question is no longer the one the run waits on (`CommitDecided`, the unit or the run
//!   ended), it closes. The outcome is the session's event, never the dialog's (T1c M7).
//! - **Connection lost** ([`connection_lost`]): the prompt says the server rolled the changes
//!   back (in the savepoint form, the user's whole transaction), with Close only and no
//!   countdown.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_exec::{CommitAnswer, CommitQuestion};
use hacivat_l10n::ArgValue;
use hacivat_proto::EnvTag;
use hacivat_session::session::RunEvent;
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::dialog::DialogSize;
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::popover::A11yNode;
use hacivat_ui::text::{Label, TextSource, paragraph};
use hacivat_ui_core::gpui_surface::{
    App, AppContext as _, Context, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, Role, SharedString, Styled, Task, WeakEntity, Window, div,
    px,
};
use hacivat_ui_core::theme::Theme;
use tokio::sync::{oneshot, watch};

use super::confirm::{
    OpenPrompts, closes_dialog, grouped_footer, scroll_body, sql_inset, subtitle, text_arg,
};
use super::{answered, in_workspace_window, workspace_window};
use crate::connections::prompt_kit::{
    acting_enter, card, cycle_focus, header, push_modal, register_part, remove_modal, trapped_tab,
    visible,
};
use crate::console::ConsoleId;
use crate::exec_ui::marks::message;

/// The prompt's layer ID.
pub const COMMIT_LAYER_ID: &str = "commit-prompt";

/// At most this many characters of the connection's name are drawn.
const NAME_MAX_CHARS: usize = 200;

/// What the prompt shows and follows beyond its [`CommitQuestion`].
#[derive(Debug, Clone, Default)]
pub struct CommitContext {
    /// The environment mark; `None` with `production` set draws production.
    pub env: Option<EnvTag>,
    /// The engine and its major version (data).
    pub engine: Option<SharedString>,
    /// The statement's masked text (the SQL inset), when known.
    pub sql: Option<Arc<str>>,
    /// The console the question is about (a lost connection is reported per console).
    pub console: Option<ConsoleId>,
    /// The run's `RunTicket::choices`: the prompt closes once its question is decided.
    pub follow: Option<watch::Receiver<Option<(u64, RunEvent)>>>,
}

/// The time left as the countdown draws it: `m:ss` in ASCII digits, whole seconds rounded up (a
/// prompt opened 5 minutes before its deadline reads 5:00).
pub fn countdown_text(left: Duration) -> String {
    let ms = left.as_millis();
    let secs = ms.div_ceil(1_000);
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// How long until the countdown's text changes (`left` rounded up to whole seconds goes down by
/// one): the fraction of a second over a whole second, else a second.
fn until_next_tick(left: Duration) -> Duration {
    let over = left.as_millis() % 1_000;
    if over == 0 {
        Duration::from_secs(1)
    } else {
        Duration::from_millis(u64::try_from(over).unwrap_or(1_000))
    }
}

/// Asks `question` (its form from `question.savepoint`); any close answers `RollBack`. Without
/// a workspace window it answers `RollBack` at once.
pub fn open(question: CommitQuestion, cx: &mut App) -> oneshot::Receiver<CommitAnswer> {
    let ctx = CommitContext {
        env: question.production.then_some(EnvTag::Production),
        ..CommitContext::default()
    };
    open_in(question, ctx, cx)
}

/// Asks `question` with `ctx` (module docs).
pub fn open_in(
    question: CommitQuestion,
    ctx: CommitContext,
    cx: &mut App,
) -> oneshot::Receiver<CommitAnswer> {
    if workspace_window(cx).is_none() {
        return answered(CommitAnswer::RollBack);
    }
    // The time left is read once from the session's deadline; the countdown then runs on the
    // executor's clock.
    let left = question.deadline.saturating_duration_since(Instant::now());
    let (reply, rx) = oneshot::channel();
    cx.defer(move |cx| {
        let Some(handle) = workspace_window(cx) else {
            let _ = reply.send(CommitAnswer::RollBack);
            return;
        };
        let mut reply = Some(reply);
        let opened = handle.update(cx, |_, window, cx| {
            if let Some(reply) = reply.take() {
                show(question, ctx, left, reply, window, cx);
            }
        });
        if opened.is_err()
            && let Some(reply) = reply.take()
        {
            let _ = reply.send(CommitAnswer::RollBack);
        }
    });
    rx
}

/// Pushes the prompt over `window`'s workspace, `left` before its deadline.
fn show(
    question: CommitQuestion,
    ctx: CommitContext,
    left: Duration,
    reply: oneshot::Sender<CommitAnswer>,
    window: &mut Window,
    cx: &mut App,
) {
    let console = ctx.console;
    let view = cx.new(|cx| CommitPrompt::new(question, ctx, left, reply, window, cx));
    {
        let open = cx.default_global::<OpenPrompts>();
        open.commits.retain(|(_, p)| p.upgrade().is_some());
        open.commits.push((console, view.downgrade()));
    }
    let safe = view.read(cx).safe.clone();
    if !push_modal(COMMIT_LAYER_ID, view.clone().into(), &safe, window, cx) {
        view.update(cx, |p, _| p.finish(CommitAnswer::RollBack));
    }
    for (name, handle) in view.read(cx).parts() {
        register_part(cx, name, &handle, None);
    }
}

/// Console `console`'s connection was lost while its prompt waited (`None`: every open prompt):
/// the prompt switches to the lost form (module docs).
pub(crate) fn connection_lost(
    console: Option<ConsoleId>,
    statements_before: Option<u32>,
    cx: &mut App,
) {
    let Some(open) = cx.try_global::<OpenPrompts>() else {
        return;
    };
    let prompts: Vec<_> = open
        .commits
        .iter()
        .filter(|(c, _)| console.is_none() || *c == console)
        .filter_map(|(_, p)| p.upgrade())
        .collect();
    if prompts.is_empty() {
        return;
    }
    in_workspace_window(cx, move |_, window, cx| {
        for p in prompts {
            p.update(cx, |p, cx| {
                p.lost = Some(statements_before);
                p.finish(CommitAnswer::RollBack);
                window.focus(&p.close, cx);
                cx.notify();
            });
        }
    });
}

/// The view IDs of the open commit prompts (tests).
pub fn open_commit_views(cx: &App) -> Vec<hacivat_ui_core::gpui_surface::EntityId> {
    cx.try_global::<OpenPrompts>()
        .map(|o| {
            o.commits
                .iter()
                .filter_map(|(_, p)| p.upgrade())
                .map(|p| p.entity_id())
                .collect()
        })
        .unwrap_or_default()
}

/// The countdown text an open commit prompt draws now, and how many times it repainted on its
/// own (tests).
pub fn countdown_of(cx: &App) -> Option<(String, u64)> {
    let open = cx.try_global::<OpenPrompts>()?;
    let p = open.commits.iter().rev().find_map(|(_, p)| p.upgrade())?;
    let p = p.read(cx);
    Some((countdown_text(p.left(cx)), p.ticks))
}

/// The confirm-in-transaction prompt (module docs).
pub struct CommitPrompt {
    q: CommitQuestion,
    ctx: CommitContext,
    reply: Option<oneshot::Sender<CommitAnswer>>,
    /// The deadline on the executor's clock.
    end: Instant,
    /// The connection was lost: `Some(statements before this one)`, if known.
    lost: Option<Option<u32>>,
    root: FocusHandle,
    /// Roll back / Undo (Primary, focused).
    safe: FocusHandle,
    /// Commit / Keep.
    keep: FocusHandle,
    /// Close (the lost form).
    close: FocusHandle,
    /// The countdown's own repaints (tests: once a second).
    ticks: u64,
    _tasks: Vec<Task<()>>,
}

impl Focusable for CommitPrompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root.clone()
    }
}

impl Drop for CommitPrompt {
    /// A prompt that goes without an answer (its window closed, the app quit) answers Roll back.
    fn drop(&mut self) {
        self.finish(CommitAnswer::RollBack);
    }
}

impl CommitPrompt {
    fn new(
        q: CommitQuestion,
        mut ctx: CommitContext,
        left: Duration,
        reply: oneshot::Sender<CommitAnswer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let end = cx.background_executor().now() + left;
        let mut tasks = Vec::new();
        // The countdown: a repaint when its text changes, once a second; at 0:00, Roll back.
        tasks.push(cx.spawn_in(window, async move |this, cx| {
            loop {
                let Ok(left) = this.update(cx, |p, cx| p.left(cx)) else {
                    return;
                };
                if left.is_zero() {
                    let _ = this.update_in(cx, |p, window, cx| {
                        if p.lost.is_none() {
                            p.answer(CommitAnswer::RollBack, window, cx);
                        }
                    });
                    return;
                }
                cx.background_executor().timer(until_next_tick(left)).await;
                let ticked = this.update(cx, |p, cx| {
                    p.ticks += 1;
                    cx.notify();
                    p.lost.is_none()
                });
                if !matches!(ticked, Ok(true)) {
                    return;
                }
            }
        }));
        if let Some(follow) = ctx.follow.take() {
            tasks.push(follow_session(follow, &q, window, cx));
        }
        CommitPrompt {
            q,
            ctx,
            reply: Some(reply),
            end,
            lost: None,
            root: cx.focus_handle(),
            safe: cx.focus_handle().tab_stop(true),
            keep: cx.focus_handle().tab_stop(true),
            close: cx.focus_handle().tab_stop(true),
            ticks: 0,
            _tasks: tasks,
        }
    }

    fn parts(&self) -> Vec<(&'static str, FocusHandle)> {
        vec![
            ("commit-safe", self.safe.clone()),
            ("commit-keep", self.keep.clone()),
            ("commit-close", self.close.clone()),
        ]
    }

    /// The time left on the executor's clock.
    fn left(&self, cx: &App) -> Duration {
        self.end
            .saturating_duration_since(cx.background_executor().now())
    }

    /// Sends `answer` once; later answers (and the drop) send nothing.
    pub(crate) fn finish(&mut self, answer: CommitAnswer) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(answer);
        }
    }

    /// Answers `answer` and closes.
    fn answer(&mut self, answer: CommitAnswer, window: &mut Window, cx: &mut Context<Self>) {
        self.finish(answer);
        let view = cx.entity_id();
        window.defer(cx, move |window, cx| remove_modal(view, window, cx));
    }

    /// The buttons in visual order with their answers.
    fn buttons(&self) -> Vec<(FocusHandle, CommitAnswer)> {
        if self.lost.is_some() {
            return vec![(self.close.clone(), CommitAnswer::RollBack)];
        }
        vec![
            (self.keep.clone(), CommitAnswer::Commit),
            (self.safe.clone(), CommitAnswer::RollBack),
        ]
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        match stroke.key.as_str() {
            "escape" => self.answer(CommitAnswer::RollBack, window, cx),
            "enter" => {
                if !acting_enter(event) {
                    return cx.stop_propagation();
                }
                let focused = self
                    .buttons()
                    .into_iter()
                    .find(|(h, _)| h.is_focused(window));
                if let Some((_, answer)) = focused {
                    self.answer(answer, window, cx);
                }
            }
            "w" if closes_dialog(stroke, cx) => self.answer(CommitAnswer::RollBack, window, cx),
            _ if trapped_tab(event) => {
                let order: Vec<FocusHandle> = self.buttons().into_iter().map(|(h, _)| h).collect();
                cycle_focus(&order, stroke.modifiers.shift, window, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

/// Follows the run's questions: once this prompt's question is no longer the one waiting (after
/// it was seen), or the run's ticket is gone, the prompt closes, unless it reports a lost
/// connection (the user closes that one).
fn follow_session(
    mut choices: watch::Receiver<Option<(u64, RunEvent)>>,
    q: &CommitQuestion,
    window: &mut Window,
    cx: &mut Context<CommitPrompt>,
) -> Task<()> {
    let (unit, question) = (q.unit, q.question);
    let ours = move |c: &Option<(u64, RunEvent)>| {
        matches!(c, Some((_, RunEvent::NeedsCommit { unit: u, question: id, .. }))
            if *u == unit && *id == question)
    };
    cx.spawn_in(window, async move |view: WeakEntity<CommitPrompt>, cx| {
        let mut seen = false;
        loop {
            let waiting = ours(&choices.borrow_and_update());
            if waiting {
                seen = true;
            } else if seen {
                break;
            }
            if choices.changed().await.is_err() {
                break;
            }
        }
        let _ = view.update_in(cx, |p, window, cx| {
            if p.lost.is_none() {
                p.answer(CommitAnswer::RollBack, window, cx);
            }
        });
    })
}

impl Render for CommitPrompt {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let q = &self.q;
        let name = visible(&q.connection, NAME_MAX_CHARS);
        let env = self
            .ctx
            .env
            .or_else(|| q.production.then_some(EnvTag::Production));
        let rows = q.affected.map(ArgValue::Count);
        let title = match (self.lost, q.savepoint, rows) {
            (Some(_), _, _) => message(
                cx,
                "hv-commit-dropped-title",
                vec![("name", text_arg(&name))],
            ),
            (None, true, Some(rows)) => message(cx, "hv-commit-sp-title", vec![("rows", rows)]),
            (None, true, None) => message(cx, "hv-commit-sp-title-unknown", vec![]),
            (None, false, Some(rows)) => message(
                cx,
                "hv-commit-title",
                vec![("rows", rows), ("name", text_arg(&name))],
            ),
            (None, false, None) => message(
                cx,
                "hv-commit-title-unknown",
                vec![("name", text_arg(&name))],
            ),
        };
        let sub = subtitle(&theme, env, &name, self.ctx.engine.clone(), None);

        let line = |text: SharedString, ink, id: &'static str| {
            paragraph(text, TextSource::Fluent, t.type_ramp.body, ink).id(id)
        };
        let mut body = div().flex().flex_col().gap(px(t.spacing.md));
        let description: SharedString;
        if let Some(before) = self.lost {
            let text = match (q.savepoint, before) {
                (false, _) => message(cx, "hv-commit-dropped-body", vec![]),
                (true, Some(n)) => message(
                    cx,
                    "hv-commit-dropped-sp-body",
                    vec![("n", ArgValue::Count(u64::from(n)))],
                ),
                (true, None) => message(cx, "hv-commit-dropped-sp-body-unknown", vec![]),
            };
            description = text.clone();
            body = body.child(
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(px(t.spacing.sm))
                    .child(
                        div().flex_none().pt(px(2.0)).child(
                            Icon::new(IconName::Unplug)
                                .size(IconSize::Chrome)
                                .color(c.text.to_hsla()),
                        ),
                    )
                    .child(div().flex_1().min_w_0().child(line(
                        text,
                        c.text.to_hsla(),
                        "commit-dropped-body",
                    ))),
            );
        } else {
            if let Some(sql) = &self.ctx.sql {
                body = body.child(sql_inset(&theme, sql, "commit-sql"));
            }
            let (first, second, third) = if q.savepoint {
                (
                    message(cx, "hv-commit-sp-keep-line", vec![]),
                    Some(message(cx, "hv-commit-sp-undo-line", vec![])),
                    message(cx, "hv-commit-sp-locks", vec![]),
                )
            } else {
                (
                    message(cx, "hv-commit-body-own", vec![]),
                    None,
                    message(cx, "hv-commit-locks", vec![]),
                )
            };
            description = [Some(&first), second.as_ref(), Some(&third)]
                .into_iter()
                .flatten()
                .map(SharedString::as_ref)
                .collect::<Vec<_>>()
                .join(" ")
                .into();
            body = body
                .child(line(first, c.text.to_hsla(), "commit-line-1"))
                .children(second.map(|s| line(s, c.text.to_hsla(), "commit-line-2")))
                .child(line(third, c.text_2.to_hsla(), "commit-locks"));
        }

        let button = |id: &'static str,
                      label_id: &'static str,
                      kind: ButtonKind,
                      icon: Option<IconName>,
                      handle: &FocusHandle,
                      answer: CommitAnswer,
                      cx: &mut Context<Self>| {
            let mut b = Button::new(id, message(cx, label_id, vec![]))
                .kind(kind)
                .on_raised(true)
                .focus_handle(handle.clone())
                .on_click(cx.listener(move |this, _, window, cx| this.answer(answer, window, cx)));
            if let Some(icon) = icon {
                b = b.icon(icon);
            }
            b.into_any_element()
        };
        let (left, right) = if self.lost.is_some() {
            (
                Vec::new(),
                vec![button(
                    "commit-close",
                    "hv-commit-close",
                    ButtonKind::Primary,
                    None,
                    &self.close,
                    CommitAnswer::RollBack,
                    cx,
                )],
            )
        } else {
            let time = countdown_text(self.left(cx));
            let countdown_id = if q.savepoint {
                "hv-commit-sp-countdown"
            } else {
                "hv-commit-countdown"
            };
            let words = message(
                cx,
                countdown_id,
                vec![("time", ArgValue::Text(time.clone()))],
            );
            let buttons = if q.savepoint {
                vec![
                    button(
                        "commit-keep",
                        "hv-commit-keep",
                        ButtonKind::Default,
                        None,
                        &self.keep,
                        CommitAnswer::Commit,
                        cx,
                    ),
                    button(
                        "commit-undo",
                        "hv-commit-undo",
                        ButtonKind::Primary,
                        Some(IconName::Undo2),
                        &self.safe,
                        CommitAnswer::RollBack,
                        cx,
                    ),
                ]
            } else {
                vec![
                    button(
                        "commit-commit",
                        "hv-commit-commit",
                        ButtonKind::Default,
                        Some(IconName::Check),
                        &self.keep,
                        CommitAnswer::Commit,
                        cx,
                    ),
                    button(
                        "commit-rollback",
                        "hv-commit-rollback",
                        ButtonKind::Primary,
                        Some(IconName::Undo2),
                        &self.safe,
                        CommitAnswer::RollBack,
                        cx,
                    ),
                ]
            };
            (
                vec![countdown(&theme, words, time).into_any_element()],
                buttons,
            )
        };

        let card = card(&theme, DialogSize::Wide, COMMIT_LAYER_ID)
            .track_focus(&self.root)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            .child(header(
                &theme,
                title.clone(),
                "commit-title",
                Some(sub),
                cx.listener(|this, _, window, cx| this.answer(CommitAnswer::RollBack, window, cx)),
                cx,
            ))
            .child(scroll_body(body))
            .child(grouped_footer(&theme, left, right));
        A11yNode::new("commit-node", Role::AlertDialog, title, true, card).description(description)
    }
}

/// The countdown (NOTES "New primitives"): the 12 px timer glyph, the words in caption 12
/// text-2 and the `m:ss` time in Fragment Mono 11.5 text-2, split out of the message on its
/// isolation marks; `role=timer` whose value is the same tick's time.
fn countdown(theme: &Theme, words: SharedString, time: String) -> impl IntoElement {
    let t = theme.tokens;
    let ink = t.colors.text_2.to_hsla();
    let mut row = div()
        .id("commit-countdown")
        .debug_selector(|| "commit-countdown".to_string())
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(t.spacing.xs))
        .child(
            div()
                .flex_none()
                .child(Icon::new(IconName::Timer).size(IconSize::Chrome).color(ink)),
        );
    row = match split(&words) {
        Some((before, value, after)) => {
            let plain =
                |text: String| Label::new(text, TextSource::Fluent, t.type_ramp.caption, ink);
            row.children((!before.is_empty()).then(|| plain(before)))
                .child(
                    Label::new(value, TextSource::Data, t.type_ramp.mono_data, ink)
                        .id("commit-countdown-time"),
                )
                .children((!after.is_empty()).then(|| plain(after)))
        }
        None => row.child(
            Label::new(words.clone(), TextSource::Fluent, t.type_ramp.caption, ink)
                .id("commit-countdown-words"),
        ),
    };
    A11yNode::new("commit-countdown-node", Role::Timer, words, false, row).value(time.into())
}

/// Splits a rendered message around its first isolated placeable (U+2068 … U+2069).
fn split(s: &str) -> Option<(String, String, String)> {
    let open = s.find('\u{2068}')?;
    let rest = &s[open + '\u{2068}'.len_utf8()..];
    let close = rest.find('\u{2069}')?;
    Some((
        s[..open].to_string(),
        rest[..close].to_string(),
        rest[close + '\u{2069}'.len_utf8()..].to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn countdown_reads_minutes_and_seconds_rounded_up() {
        assert_eq!(countdown_text(Duration::from_secs(300)), "5:00");
        assert_eq!(countdown_text(Duration::from_millis(299_001)), "5:00");
        assert_eq!(countdown_text(Duration::from_secs(272)), "4:32");
        assert_eq!(countdown_text(Duration::from_millis(271_400)), "4:32");
        assert_eq!(countdown_text(Duration::from_millis(1)), "0:01");
        assert_eq!(countdown_text(Duration::ZERO), "0:00");
    }

    #[test]
    fn the_next_tick_is_when_the_text_changes() {
        assert_eq!(
            until_next_tick(Duration::from_secs(272)),
            Duration::from_secs(1)
        );
        assert_eq!(
            until_next_tick(Duration::from_millis(271_400)),
            Duration::from_millis(400)
        );
    }
}

// ---- shots fixtures -----------------------------------------------------------------------------

/// The shots' fixtures of surface 2 (`design/m3b-safe`): each opens one state with a question
/// whose asker never answers; the countdown stands still (the shots' clock does not move).
#[cfg(feature = "shots")]
pub mod fixtures {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use hacivat_exec::{CommitAnswer, CommitQuestion};
    use hacivat_proto::EnvTag;
    use hacivat_session::session::{RunId, UnitId};
    use hacivat_ui_core::gpui_surface::{App, Global, SharedString, Window};
    use tokio::sync::oneshot;

    use super::{CommitContext, show};

    #[derive(Default)]
    struct Held(Vec<oneshot::Receiver<CommitAnswer>>);

    impl Global for Held {}

    /// One state of the prompt.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum State {
        /// Manual · first change: the savepoint form, 1 row.
        Savepoint,
        /// Auto-commit, Hacivat's own transaction: 214 rows.
        Own,
        /// The connection was lost (own form).
        LostOwn,
        /// The connection was lost (savepoint form: the user's transaction, 3 statements before).
        LostSavepoint,
    }

    pub const STATES: [State; 4] = [
        State::Savepoint,
        State::Own,
        State::LostOwn,
        State::LostSavepoint,
    ];

    impl State {
        pub fn name(self) -> &'static str {
            match self {
                State::Savepoint => "commit-savepoint",
                State::Own => "commit-own",
                State::LostOwn => "commit-lost-own",
                State::LostSavepoint => "commit-lost-savepoint",
            }
        }
    }

    /// Opens `state` over `window`'s workspace.
    pub fn open(state: State, window: &mut Window, cx: &mut App) {
        let savepoint = matches!(state, State::Savepoint | State::LostSavepoint);
        let (affected, left, sql) = if savepoint {
            (
                1,
                Duration::from_secs(4 * 60 + 51),
                "UPDATE orders\n   SET status = 'shipped'\n WHERE id = 4211;",
            )
        } else {
            (
                214,
                Duration::from_secs(4 * 60 + 32),
                "UPDATE orders\n   SET status = 'archived'\n WHERE created_at < '2026-01-01';",
            )
        };
        let q = CommitQuestion {
            run: RunId(1),
            unit: UnitId {
                run: RunId(1),
                ordinal: 0,
            },
            question: 1,
            affected: Some(affected),
            deadline: Instant::now() + left,
            savepoint,
            production: true,
            connection: Arc::from("orders-prod"),
        };
        // Connection data (the engine and its major version), not chrome text.
        let engine = "PostgreSQL 16";
        let ctx = CommitContext {
            env: Some(EnvTag::Production),
            engine: Some(SharedString::from(engine.to_string())),
            sql: Some(Arc::from(sql)),
            console: None,
            follow: None,
        };
        let (reply, rx) = oneshot::channel();
        cx.default_global::<Held>().0.push(rx);
        show(q, ctx, left, reply, window, cx);
        if matches!(state, State::LostOwn | State::LostSavepoint) {
            let before = savepoint.then_some(3);
            if let Some(open) = cx.try_global::<super::OpenPrompts>()
                && let Some(p) = open.commits.iter().rev().find_map(|(_, p)| p.upgrade())
            {
                p.update(cx, |p, cx| {
                    p.lost = Some(before);
                    p.finish(CommitAnswer::RollBack);
                    window.focus(&p.close, cx);
                    cx.notify();
                });
            }
        }
    }
}
