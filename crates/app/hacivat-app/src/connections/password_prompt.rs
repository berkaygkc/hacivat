//! The password prompt a connect asks through the UI port (NOTES surface 4, R14, Task 20).
//!
//! A 480 px modal layer `password-prompt`: "Sign in to <name>", a line naming the user, the
//! masked field the error dialog's re-entry also uses (with "Attempt n of 3" from the second
//! attempt, P8), and Cancel / Connect. Enter (in the field) or Connect answers with the
//! password, which becomes a `SecretString` at once while the field is cleared; Escape (also
//! from the focused field, P16) or Cancel answers `Cancelled`. A prompt closed any other way
//! drops its reply channel, which the port reads as `Cancelled`. Tab is trapped in the prompt.

use std::rc::Rc;

use hacivat_l10n::{Count, NoArgs, Text, ids};
use hacivat_session::pipeline::{Answer, Question};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::dialog::{DialogSize, dialog_frame};
use hacivat_ui::input::{InputEvent, TextInput};
use hacivat_ui::popover::A11yNode;
use hacivat_ui::text::{Label, TextSource};
use hacivat_ui_core::gpui_surface::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, Render, Role, Styled, Subscription, Window, div, px,
};
use hacivat_ui_core::theme::Theme;
use secrecy::SecretString;
use tokio::sync::oneshot;

use crate::connections::error_view::paragraph;
use crate::connections::interaction::PromptRequest;
use crate::l10n_global::tr;
use crate::shell::Workspace;
use crate::shell::layers::{DIALOG_CONTEXT, Layer};

/// The prompt's layer ID.
pub const PASSWORD_LAYER_ID: &str = "password-prompt";

/// The width of the password field (as in the error dialog's re-entry).
const FIELD_PX: f32 = 240.0;

/// The password prompt (R14).
pub struct PasswordPrompt {
    name: String,
    user: String,
    attempt: u8,
    input: Entity<TextInput>,
    reply: Option<oneshot::Sender<Answer>>,
    root: FocusHandle,
    cancel_focus: FocusHandle,
    connect_focus: FocusHandle,
    _subscription: Subscription,
}

impl Focusable for PasswordPrompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root.clone()
    }
}

/// What a password question asks (`Question::Password` without its profile ID).
struct Asked {
    name: String,
    user: String,
    attempt: u8,
}

impl PasswordPrompt {
    /// A prompt for `question`; its answer goes to `reply` (the forwarding task in [`open`]).
    fn new(
        question: Asked,
        reply: oneshot::Sender<Answer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let Asked {
            name,
            user,
            attempt,
        } = question;
        let placeholder_user = user.clone();
        let placeholder = Rc::new(move |cx: &App| {
            tr(
                cx,
                ids::hv::CONN_ERR_NEW_PW,
                ids::hv::HvConnErrNewPw {
                    user: Text(placeholder_user.clone()),
                },
            )
        });
        let input = cx.new(|cx| {
            let mut i = TextInput::new(cx, placeholder);
            i.set_masked(true);
            i.set_well(true);
            i
        });
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Submitted => this.submit(window, cx),
                // A focused field swallows Escape (P16): it cancels the prompt.
                InputEvent::Cancelled => this.cancel(window, cx),
                _ => {}
            },
        );
        PasswordPrompt {
            name,
            user,
            attempt,
            input,
            reply: Some(reply),
            root: cx.focus_handle(),
            cancel_focus: cx.focus_handle().tab_stop(true),
            connect_focus: cx.focus_handle().tab_stop(true),
            _subscription: subscription,
        }
    }

    fn answer(&mut self, answer: Answer, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(answer);
        }
        window.defer(cx, close_layer);
    }

    /// Answers with the typed password (an empty field answers nothing).
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let password = self.input.update(cx, |i, _| {
            let typed = (!i.value().is_empty()).then(|| SecretString::from(i.value().to_string()));
            i.set_value("");
            typed
        });
        if let Some(password) = password {
            self.answer(Answer::Password(password), window, cx);
        }
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.answer(Answer::Cancelled, window, cx);
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        match stroke.key.as_str() {
            "escape" => self.cancel(window, cx),
            "tab" if !stroke.modifiers.control && !stroke.modifiers.platform => {
                let order = [
                    self.input.read(cx).focus_handle(cx),
                    self.cancel_focus.clone(),
                    self.connect_focus.clone(),
                ];
                let at = order.iter().position(|h| h.is_focused(window));
                let next = match (at, stroke.modifiers.shift) {
                    (Some(i), false) => (i + 1) % order.len(),
                    (Some(i), true) => (i + order.len() - 1) % order.len(),
                    (None, false) => 0,
                    (None, true) => order.len() - 1,
                };
                window.focus(&order[next], cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl Render for PasswordPrompt {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let title = tr(
            cx,
            ids::hv::CONN_PW_TITLE,
            ids::hv::HvConnPwTitle {
                name: Text(self.name.clone()),
            },
        );
        let body = tr(
            cx,
            ids::hv::CONN_PW_BODY,
            ids::hv::HvConnPwBody {
                user: Text(self.user.clone()),
            },
        );
        let attempt = (self.attempt > 1).then(|| {
            Label::new(
                tr(
                    cx,
                    ids::hv::CONN_ERR_ATTEMPT,
                    ids::hv::HvConnErrAttempt {
                        n: Count(u64::from(self.attempt)),
                    },
                ),
                TextSource::Fluent,
                t.type_ramp.caption,
                c.text_2.to_hsla(),
            )
            .id("password-attempt")
        });
        let cancel = Button::new("pw-cancel", tr(cx, ids::hv::CONN_PW_CANCEL, NoArgs))
            .kind(ButtonKind::Ghost)
            .focus_handle(self.cancel_focus.clone())
            .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx)));
        let connect = Button::new("pw-connect", tr(cx, ids::hv::CONN_FIX_CONNECT, NoArgs))
            .kind(ButtonKind::Primary)
            .focus_handle(self.connect_focus.clone())
            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx)));
        let card = dialog_frame(&theme, DialogSize::Wide)
            .id(PASSWORD_LAYER_ID)
            .debug_selector(|| PASSWORD_LAYER_ID.to_string())
            .track_focus(&self.root)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(Label::new(
                title.clone(),
                TextSource::Fluent,
                t.type_ramp.body_strong,
                c.text.to_hsla(),
            ))
            .child(paragraph(
                body.clone(),
                TextSource::Fluent,
                t.type_ramp.body,
                c.text_2.to_hsla(),
                Some("password-body"),
                cx,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(t.spacing.sm))
                    .child(
                        div()
                            .id("password-field")
                            .debug_selector(|| "password-field".to_string())
                            .w(px(FIELD_PX))
                            .child(self.input.clone()),
                    )
                    .children(attempt),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(px(t.spacing.md))
                    .child(cancel)
                    .child(connect),
            );
        A11yNode::new("password-prompt-node", Role::Dialog, title, true, card).description(body)
    }
}

/// Pops the prompt when it is the top layer.
fn close_layer(window: &mut Window, cx: &mut App) {
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    ws.update(cx, |ws, cx| {
        if ws.layer_ids().last() == Some(&PASSWORD_LAYER_ID) {
            ws.pop_layer(window, cx);
        }
    });
}

/// Opens the prompt for `request` in the workspace window (the active one, else the first),
/// and focuses its field. Without a workspace window the request is dropped, which answers
/// `Cancelled`.
pub(crate) fn open(request: PromptRequest, cx: &mut App) {
    let window = cx
        .active_window()
        .into_iter()
        .chain(cx.windows())
        .find(|w| w.downcast::<Workspace>().is_some());
    let Some(window) = window else {
        return;
    };
    let PromptRequest {
        question,
        mut reply,
    } = request;
    // ## m2b-t01-interfaces: only password questions open this prompt; any other is cancelled.
    let question = match question {
        Question::Password {
            name,
            user,
            attempt,
            ..
        } => Asked {
            name,
            user,
            attempt,
        },
        _ => {
            let _ = reply.send(Answer::Cancelled);
            return;
        }
    };
    // The prompt answers on its own channel; a task forwards that answer to the asker, or,
    // when the asker goes away first (the connect or test was cancelled, the profile
    // forgotten), closes the orphaned prompt.
    let (answer_tx, answer_rx) = oneshot::channel::<Answer>();
    let opened = window.update(cx, move |_, window, cx| {
        let ws = window.root::<Workspace>().flatten()?;
        let prompt = cx.new(|cx| PasswordPrompt::new(question, answer_tx, window, cx));
        let field = {
            let p = prompt.read(cx);
            p.input.read(cx).focus_handle(cx)
        };
        ws.update(cx, |ws, cx| {
            ws.push_layer(
                Layer {
                    id: PASSWORD_LAYER_ID,
                    context: DIALOG_CONTEXT,
                    view: prompt.into(),
                    restore_focus: None,
                    modal: true,
                },
                window,
                cx,
            );
        });
        window.focus(&field, cx);
        Some(())
    });
    if !matches!(opened, Ok(Some(()))) {
        // Dropping `reply` answers `Cancelled`.
        return;
    }
    cx.spawn(async move |cx| {
        let answered = {
            let closed = reply.closed();
            futures::pin_mut!(closed);
            match futures::future::select(closed, answer_rx).await {
                futures::future::Either::Left(_) => None,
                futures::future::Either::Right((answer, _)) => Some(answer),
            }
        };
        match answered {
            Some(Ok(answer)) => {
                let _ = reply.send(answer);
            }
            // The prompt went away without an answer: dropping `reply` answers `Cancelled`.
            Some(Err(_)) => {}
            None => {
                cx.update(|cx| {
                    let _ = window.update(cx, |_, window, cx| close_layer(window, cx));
                });
            }
        }
    })
    .detach();
}
