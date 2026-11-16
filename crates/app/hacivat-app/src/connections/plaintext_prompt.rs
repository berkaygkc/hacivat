//! The public-plaintext question (NOTES M2a surface 5, "Public host without TLS"; Task 19, B7):
//! under Auto, a host on a public network offered no TLS, so the password and the results
//! would cross the network unencrypted.
//!
//! A 420 px modal layer `plaintext-prompt`: "`host` doesn't offer encryption", what that means,
//! the checkbox "Don't ask again for this connection", and Cancel (left), Connect without
//! encryption and **Use an SSH tunnel…** (primary).
//!
//! The safe answer has the focus: Use an SSH tunnel. Connect without encryption weakens the
//! connection, so it is never the default. Enter acts on key down only (never held, never with a
//! modifier; its key up is suppressed) and presses the focused button, else the primary;
//! Escape cancels. Use an SSH tunnel answers `UseSsh` and opens the connection's SSH section
//! (`dialog::ssh::open_with_ssh`).

use std::net::IpAddr;

use hacivat_l10n::{NoArgs, Text, ids};
use hacivat_proto::{EnvTag, ProfileId};
use hacivat_session::pipeline::{Answer, Question};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::capsule::env_swatch_on_raised;
use hacivat_ui::checkbox::Checkbox;
use hacivat_ui::dialog::{DialogSize, dialog_frame};
use hacivat_ui::icon::IconName;
use hacivat_ui::popover::A11yNode;
use hacivat_ui::text::{Label, TextSource, paragraph};
use hacivat_ui_core::gpui_surface::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, Global, InteractiveElement,
    IntoElement, KeyDownEvent, MouseButton, ParentElement, Render, Role, Styled, WeakEntity,
    Window, div, px,
};
use hacivat_ui_core::theme::Theme;
use tokio::sync::oneshot;

use crate::connections::cert_dialog::{close_view, footer_row, forward_answer};
use crate::connections::dialog;
use crate::connections::error_view::env_kind;
use crate::connections::model::ConnectionsModel;
use crate::l10n_global::tr;
use crate::shell::Workspace;
use crate::shell::layers::{DIALOG_CONTEXT, Layer};

/// The question's layer ID.
pub const PLAINTEXT_LAYER_ID: &str = "plaintext-prompt";

/// What was asked (`Question::PlaintextPublic`).
struct Asked {
    profile: ProfileId,
    name: String,
    host: String,
    ip: Option<IpAddr>,
}

/// The public-plaintext question (NOTES M2a surface 5).
pub struct PlaintextPrompt {
    asked: Asked,
    env: Option<EnvTag>,
    remember: bool,
    reply: Option<oneshot::Sender<Answer>>,
    root: FocusHandle,
    remember_focus: FocusHandle,
    cancel: FocusHandle,
    connect: FocusHandle,
    ssh: FocusHandle,
}

impl Focusable for PlaintextPrompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root.clone()
    }
}

/// The question open now (tests, the shots harness).
struct OpenQuestion(WeakEntity<PlaintextPrompt>);

impl Global for OpenQuestion {}

/// The public-plaintext question open now, if any.
pub fn open_view(cx: &App) -> Option<Entity<PlaintextPrompt>> {
    cx.try_global::<OpenQuestion>().and_then(|o| o.0.upgrade())
}

impl PlaintextPrompt {
    fn new(
        asked: Asked,
        env: Option<EnvTag>,
        reply: oneshot::Sender<Answer>,
        cx: &mut Context<Self>,
    ) -> Self {
        let stop = |cx: &mut Context<Self>| cx.focus_handle().tab_stop(true);
        PlaintextPrompt {
            asked,
            env,
            remember: false,
            reply: Some(reply),
            root: cx.focus_handle(),
            remember_focus: stop(cx),
            cancel: stop(cx),
            connect: stop(cx),
            ssh: stop(cx),
        }
    }

    /// The ID of the focused control (`plain-remember`, `plain-cancel`, `plain-connect`,
    /// `plain-ssh`), if one is.
    pub fn focused(&self, window: &Window) -> Option<&'static str> {
        self.order()
            .into_iter()
            .find(|(_, h)| h.is_focused(window))
            .map(|(id, _)| id)
    }

    /// Whether "Don't ask again" is ticked.
    pub fn remembers(&self) -> bool {
        self.remember
    }

    fn order(&self) -> [(&'static str, FocusHandle); 4] {
        [
            ("plain-remember", self.remember_focus.clone()),
            ("plain-cancel", self.cancel.clone()),
            ("plain-connect", self.connect.clone()),
            ("plain-ssh", self.ssh.clone()),
        ]
    }

    fn answer(&mut self, answer: Answer, window: &mut Window, cx: &mut Context<Self>) {
        let Some(reply) = self.reply.take() else {
            return;
        };
        let use_ssh = matches!(answer, Answer::UseSsh).then_some(self.asked.profile);
        let _ = reply.send(answer);
        let view = cx.entity_id();
        window.defer(cx, move |window, cx| {
            close_view(view, window, cx);
            // The SSH section of the connection, opened with its tunnel on (Q9).
            if let Some(profile) = use_ssh {
                dialog::ssh::open_with_ssh(profile, window, cx);
            }
        });
    }

    fn connect_plaintext(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let remember = self.remember;
        self.answer(Answer::ConnectPlaintext { remember }, window, cx);
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        let m = &stroke.modifiers;
        let plain = !(m.control || m.platform || m.alt || m.shift || m.function);
        match stroke.key.as_str() {
            "escape" => self.answer(Answer::Cancelled, window, cx),
            "enter" if !plain || event.is_held => {}
            "enter" => match self.focused(window) {
                Some("plain-cancel") => self.answer(Answer::Cancelled, window, cx),
                Some("plain-connect") => self.connect_plaintext(window, cx),
                // Use an SSH tunnel, or the checkbox: the primary (never the plaintext one).
                _ => self.answer(Answer::UseSsh, window, cx),
            },
            "tab" if !(m.control || m.platform || m.alt) => {
                let order = self.order();
                let n = order.len();
                let at = order.iter().position(|(_, h)| h.is_focused(window));
                let next = match (at, m.shift) {
                    (Some(i), false) => (i + 1) % n,
                    (Some(i), true) => (i + n - 1) % n,
                    (None, false) => 0,
                    (None, true) => n - 1,
                };
                window.focus(&order[next].1, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl Render for PlaintextPrompt {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let title = tr(
            cx,
            ids::hv::CONN_PLAIN_TITLE,
            ids::hv::HvConnPlainTitle {
                host: Text(self.asked.host.clone()),
            },
        );
        // The address the server answered from; the host when it was never resolved.
        let address = self
            .asked
            .ip
            .map_or_else(|| self.asked.host.clone(), |ip| ip.to_string());
        let body = tr(
            cx,
            ids::hv::CONN_PLAIN_BODY,
            ids::hv::HvConnPlainBody { ip: Text(address) },
        );
        let me = cx.weak_entity();
        let remember = Checkbox::new(
            "plain-remember",
            tr(cx, ids::hv::CONN_PLAIN_REMEMBER, NoArgs),
        )
        .checked(self.remember)
        .focus_handle(self.remember_focus.clone())
        .on_toggle(move |on, _, cx| {
            let _ = me.update(cx, |p, cx| {
                p.remember = on;
                cx.notify();
            });
        });
        let cancel = Button::new("plain-cancel", tr(cx, ids::hv::CONN_DLG_CANCEL, NoArgs))
            .kind(ButtonKind::Ghost)
            .focus_handle(self.cancel.clone())
            .on_raised(true)
            .on_click(cx.listener(|this, _, window, cx| {
                this.answer(Answer::Cancelled, window, cx);
            }));
        let connect = Button::new("plain-connect", tr(cx, ids::hv::CONN_PLAIN_CONNECT, NoArgs))
            .kind(ButtonKind::Default)
            .icon(IconName::LockOpen)
            .focus_handle(self.connect.clone())
            .on_raised(true)
            .on_click(cx.listener(|this, _, window, cx| this.connect_plaintext(window, cx)));
        let ssh = Button::new("plain-ssh", tr(cx, ids::hv::CONN_PLAIN_SSH, NoArgs))
            .kind(ButtonKind::Primary)
            .icon(IconName::Route)
            .focus_handle(self.ssh.clone())
            .on_raised(true)
            .on_click(cx.listener(|this, _, window, cx| {
                this.answer(Answer::UseSsh, window, cx);
            }));
        let card = dialog_frame(&theme, DialogSize::Standard)
            .id(PLAINTEXT_LAYER_ID)
            .debug_selector(|| PLAINTEXT_LAYER_ID.to_string())
            .track_focus(&self.root)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            // Enter acts on key down only; the focused button's key-up click is suppressed.
            .capture_key_up(|event, window, _| {
                let stroke = &event.keystroke;
                if stroke.key == "enter" && !stroke.modifiers.modified() {
                    window.prevent_default();
                }
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(t.spacing.s_2xs))
                    .child(
                        paragraph(
                            title.clone(),
                            TextSource::Fluent,
                            t.type_ramp.body_strong,
                            c.text.to_hsla(),
                        )
                        .id("plain-title"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(t.spacing.sm))
                            .min_w_0()
                            .children(self.env.map(|e| env_swatch_on_raised(env_kind(e), &theme)))
                            .child(
                                Label::new(
                                    self.asked.name.clone(),
                                    TextSource::Data,
                                    t.type_ramp.mono_data,
                                    c.text_2.to_hsla(),
                                )
                                .truncate(true)
                                .id("plain-connection"),
                            ),
                    ),
            )
            .child(
                paragraph(
                    body.clone(),
                    TextSource::Fluent,
                    t.type_ramp.body,
                    c.text_2.to_hsla(),
                )
                .id("plain-body"),
            )
            .child(div().child(remember))
            // A long locale wraps the buttons one by one, right-aligned.
            .child(footer_row(
                vec![
                    cancel.into_any_element(),
                    connect.into_any_element(),
                    ssh.into_any_element(),
                ],
                &theme,
            ));
        A11yNode::new(
            "plaintext-prompt-node",
            Role::AlertDialog,
            title,
            true,
            card,
        )
        .description(body)
    }
}

/// Asks `q` (`Question::PlaintextPublic`); the answer goes to `reply`: Cancel (or Escape, or a
/// closed layer) → `Cancelled`, Connect without encryption → `ConnectPlaintext { remember }`,
/// Use an SSH tunnel → `UseSsh` (and the connection's SSH section opens).
pub(crate) fn open(q: Question, reply: oneshot::Sender<Answer>, window: &mut Window, cx: &mut App) {
    let Question::PlaintextPublic {
        profile,
        name,
        host,
        ip,
    } = q
    else {
        let _ = reply.send(Answer::Cancelled);
        return;
    };
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    let env = cx
        .try_global::<ConnectionsModel>()
        .and_then(|m| m.get(profile))
        .and_then(|p| p.environment);
    let asked = Asked {
        profile,
        name,
        host,
        ip,
    };
    let (tx, rx) = oneshot::channel::<Answer>();
    let prompt = cx.new(|cx| PlaintextPrompt::new(asked, env, tx, cx));
    let ssh = prompt.read(cx).ssh.clone();
    ws.update(cx, |ws, cx| {
        ws.push_layer(
            Layer {
                id: PLAINTEXT_LAYER_ID,
                context: DIALOG_CONTEXT,
                view: prompt.clone().into(),
                restore_focus: None,
                modal: true,
            },
            window,
            cx,
        );
    });
    // The safe answer has the focus.
    window.focus(&ssh, cx);
    cx.set_global(OpenQuestion(prompt.downgrade()));
    forward_answer(reply, rx, prompt.entity_id(), window.window_handle(), cx);
}

// ## shots fixtures (R28, P19)

/// The answer channel of the shots fixture's question, kept so the question stays open.
#[cfg(feature = "shots")]
struct ShotAnswer(#[allow(dead_code, reason = "held, never read")] oneshot::Receiver<Answer>);

#[cfg(feature = "shots")]
impl Global for ShotAnswer {}

/// The question for `db.acme.io` at a public address (NOTES M2a surface 5).
#[cfg(feature = "shots")]
pub fn fixture(window: &mut Window, cx: &mut App) {
    let (reply, answer) = oneshot::channel();
    // The shot shows the question; nobody answers it, and its asker stays.
    cx.set_global(ShotAnswer(answer));
    open(
        Question::PlaintextPublic {
            profile: ProfileId::new(),
            name: "analytics-eu".to_string(),
            host: "db.acme.io".to_string(),
            ip: "203.0.113.10".parse().ok(),
        },
        reply,
        window,
        cx,
    );
}
