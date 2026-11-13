//! SSH passwords, SSH key passphrases and TLS key passphrases asked during a connect (NOTES
//! surface 3, Task 21): the M2a password prompt's anatomy, per hop.
//!
//! A 480 px modal layer `secret-prompt`:
//! - an SSH password: "SSH password for <hop host>", the subtitle naming the hop ("<host>, hop
//!   n of total"), "Enter the SSH password for <user>@<host>. It's kept only if the connection
//!   works.", then the rows Route (the asking hop boxed), SSH user, Password and Remember;
//! - a key passphrase (SSH or TLS): "Passphrase for <key file>", the key file instead of the
//!   user (and the route for an SSH hop);
//! - after a rejected attempt, the ink alert line and "Attempt n of 3" right under the field.
//!
//! **Remember** (fix round 1, controller ruling on the reviewer's seam): for an SSH secret, a
//! 240 px Select over the dialog's retention choices (the vault's label for Forever when the
//! vault holds Forever secrets), defaulting to the profile's `ssh_retention` (or the choice an
//! earlier prompt of this profile made in this run). The answer is `Answer::SshSecret { secret,
//! remember }`: the connect stores the typed secret under it and, after it worked, reports the
//! new retention, which the app saves (`ConnectionsModel::apply_changes`), moving the other
//! hops' stored secrets with it. A TLS key passphrase keeps a static row (the DB retention).
//!
//! Keys: the focus starts in the field; Enter (in the field or on Continue) answers with the
//! secret, which becomes a `SecretString` at once while the field is cleared; an empty field
//! answers nothing. The Select opens with Space, ↓ or Enter (Q34: Escape and Tab bubble out of
//! the closed Select). Escape, Cancel or the close button answer `Cancelled`. Tab is trapped.

// ## m2b-t21-prompts

use std::path::Path;

use hacivat_l10n::{NoArgs, Text, ids};
use hacivat_secrets::Retention;
use hacivat_session::pipeline::{Answer, Question, SecretPrompt};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::dialog::DialogSize;
use hacivat_ui::input::{InputEvent, TextInput};
use hacivat_ui::popover::A11yNode;
use hacivat_ui::select::Select;
use hacivat_ui::text::{Label, TextSource, paragraph};
use hacivat_ui_core::gpui_surface::{
    App, AppContext as _, Context, Div, Entity, FocusHandle, Focusable, Global, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, Role, Styled, Subscription, Window, div, px,
};
use hacivat_ui_core::theme::Theme;
use tokio::sync::oneshot;

use crate::connections::ConnectionsModel;
use crate::connections::dialog::RETENTIONS;
use crate::connections::dialog::form::retention_options;
use crate::connections::prompt_kit::{
    self, EnterGate, Part, Subject, alert_line, card, cycle_focus, footer, header, meta_item,
    meta_line, register_part, remove_modal, route_row, row, secret_input, static_value, subject,
    subtitle, take_secret, trapped_tab, under_field,
};
use crate::connections::vault_dialog;
use crate::l10n_global::{text_fn, tr};

/// The prompt's layer ID.
pub const SECRET_LAYER_ID: &str = "secret-prompt";

/// The width of the secret field and of the Remember value (NOTES: a 240 px Select).
const FIELD_PX: f32 = 240.0;

/// The prompt (NOTES surface 3).
pub struct SecretPromptView {
    prompt: SecretPrompt,
    attempt: u8,
    profile: Option<Subject>,
    input: Entity<TextInput>,
    reply: Option<oneshot::Sender<Answer>>,
    root: FocusHandle,
    cancel_focus: FocusHandle,
    continue_focus: FocusHandle,
    enter: EnterGate,
    _subscription: Subscription,
    // ## m2b-t21-prompts (fix round 1): the Remember select (SSH secrets only).
    remember: Option<Entity<Select>>,
    profile_id: hacivat_proto::ProfileId,
}

/// The Remember an SSH secret prompt last answered with, per profile, in this run: a later
/// prompt of the same profile starts from it ("the last answer wins").
#[derive(Default)]
struct LastRemember(Vec<(hacivat_proto::ProfileId, Retention)>);

impl Global for LastRemember {}

impl Focusable for SecretPromptView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root.clone()
    }
}

/// A path as the prompt shows it: `~` for the home directory.
fn shown_path(path: &Path) -> String {
    match std::env::home_dir()
        .as_deref()
        .and_then(|home| path.strip_prefix(home).ok())
    {
        Some(rest) => format!("~/{}", rest.to_string_lossy()),
        None => path.to_string_lossy().into_owned(),
    }
}

impl SecretPromptView {
    fn new(
        asked: (SecretPrompt, u8, hacivat_proto::ProfileId),
        profile: Option<Subject>,
        reply: oneshot::Sender<Answer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (prompt, attempt, profile_id) = asked;
        let input = secret_input(cx);
        // ## m2b-t21-prompts (fix round 1): Remember, for an SSH secret.
        let ssh = !matches!(prompt, SecretPrompt::TlsKeyPassphrase { .. });
        let remember = ssh.then(|| {
            let start = cx
                .try_global::<LastRemember>()
                .and_then(|l| {
                    l.0.iter()
                        .find(|(id, _)| *id == profile_id)
                        .map(|(_, r)| *r)
                })
                .or(profile.as_ref().map(|s| s.ssh_retention))
                .unwrap_or(Retention::Forever);
            let selected = RETENTIONS.iter().position(|r| *r == start).unwrap_or(3);
            cx.new(|cx| {
                let mut select = Select::new(retention_options(true), selected, cx);
                select.set_label(text_fn(ids::hv::CONN_DLG_REMEMBER));
                select
            })
        });
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Submitted => {
                    if this.enter.take() {
                        this.submit(window, cx);
                    }
                }
                // A focused field swallows Escape (P16): it cancels the prompt.
                InputEvent::Cancelled => this.cancel(window, cx),
                _ => {}
            },
        );
        SecretPromptView {
            prompt,
            attempt,
            profile,
            input,
            reply: Some(reply),
            root: cx.focus_handle(),
            cancel_focus: cx.focus_handle().tab_stop(true),
            continue_focus: cx.focus_handle().tab_stop(true),
            enter: EnterGate::default(),
            _subscription: subscription,
            remember,
            profile_id,
        }
    }

    // ## m2b-t21-prompts (fix round 1)
    /// Asks the core runtime once whether the keyring can store secrets (P36), as the dialog
    /// does: without one (and without the vault), Forever is offered disabled with its reason.
    fn probe_keyring(&self, cx: &mut Context<Self>) {
        let (Some(select), Some(model)) =
            (self.remember.clone(), cx.try_global::<ConnectionsModel>())
        else {
            return;
        };
        let secrets = model.secrets().clone();
        let (tx, rx) = oneshot::channel();
        model.runtime().spawn_blocking(move || {
            let _ = tx.send(secrets.keyring_available());
        });
        cx.spawn(async move |_, cx| {
            if let Ok(false) = rx.await {
                select.update(cx, |s, cx| {
                    s.set_options(retention_options(false), cx);
                    if RETENTIONS.get(s.selected()) == Some(&Retention::Forever) {
                        let fallback = RETENTIONS
                            .iter()
                            .position(|r| *r == Retention::UntilRestart)
                            .unwrap_or(1);
                        s.set_selected(fallback, cx);
                    }
                });
            }
        })
        .detach();
    }

    /// The retention the Remember select shows.
    fn chosen(&self, cx: &App) -> Option<Retention> {
        let select = self.remember.as_ref()?;
        RETENTIONS.get(select.read(cx).selected()).copied()
    }

    fn answer(&mut self, answer: Answer, window: &mut Window, cx: &mut Context<Self>) {
        prompt_kit::clear_all(std::slice::from_ref(&self.input), cx);
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(answer);
        }
        let view = cx.entity_id();
        window.defer(cx, move |window, cx| remove_modal(view, window, cx));
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.answer(Answer::Cancelled, window, cx);
    }

    /// Answers with the typed secret; an empty field answers nothing.
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.reply.is_none() {
            return;
        }
        let Some(secret) = take_secret(&self.input, cx) else {
            return;
        };
        let answer = match self.chosen(cx) {
            // ## m2b-t21-prompts (fix round 1): an SSH secret carries its Remember.
            Some(remember) => {
                let id = self.profile_id;
                let last = cx.default_global::<LastRemember>();
                last.0.retain(|(p, _)| *p != id);
                last.0.push((id, remember));
                Answer::SshSecret { secret, remember }
            }
            None => Answer::Secret(secret),
        };
        self.answer(answer, window, cx);
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => self.cancel(window, cx),
            "enter" if prompt_kit::acting_enter(event) => {
                if self.cancel_focus.is_focused(window) {
                    self.cancel(window, cx);
                } else if self.continue_focus.is_focused(window) {
                    self.submit(window, cx);
                }
            }
            "enter" => {}
            _ if trapped_tab(event) => {
                let mut order = vec![self.input.read(cx).focus_handle(cx)];
                if let Some(select) = &self.remember {
                    order.push(select.read(cx).focus_handle(cx));
                }
                order.push(self.cancel_focus.clone());
                order.push(self.continue_focus.clone());
                cycle_focus(&order, event.keystroke.modifiers.shift, window, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    /// The hop that asks (0-based) and its host, for an SSH secret.
    fn hop(&self) -> Option<(usize, String)> {
        match &self.prompt {
            SecretPrompt::SshPassword { hop, host, .. } => Some((usize::from(*hop), host.clone())),
            SecretPrompt::SshPassphrase { hop, .. } => {
                let hop = usize::from(*hop);
                let host = self
                    .profile
                    .as_ref()
                    .and_then(|s| s.hops.get(hop))
                    .map(|h| h.host.clone())
                    .unwrap_or_default();
                Some((hop, host))
            }
            SecretPrompt::TlsKeyPassphrase { .. } => None,
        }
    }

    /// How the secret will be kept, as the Remember row shows it.
    fn retention(&self) -> Retention {
        let profile = self.profile.as_ref();
        match self.prompt {
            SecretPrompt::TlsKeyPassphrase { .. } => {
                profile.map_or(Retention::Forever, |s| s.retention)
            }
            _ => profile.map_or(Retention::Forever, |s| s.ssh_retention),
        }
    }

    /// The Remember row's value. An SSH secret: the Select (240 px) and a line saying the choice
    /// applies to every hop. A TLS key passphrase: the retention's label and hint (the vault's
    /// for Forever when the vault holds Forever secrets).
    fn remember(&self, theme: &Theme, cx: &App) -> Div {
        let t = theme.tokens;
        let c = &t.colors;
        if let Some(select) = &self.remember {
            return div()
                .flex()
                .flex_col()
                .gap(px(t.spacing.xs))
                .child(
                    div()
                        .debug_selector(|| "secret-remember-select".to_string())
                        .w(px(FIELD_PX))
                        .max_w_full()
                        .child(select.clone()),
                )
                .child(
                    paragraph(
                        tr(cx, ids::hv::CONN_SSH_SECRET_REMEMBER_ALL, NoArgs),
                        TextSource::Fluent,
                        t.type_ramp.caption,
                        c.text_2.to_hsla(),
                    )
                    .id("secret-remember-all"),
                );
        }
        let (label, hint) = match self.retention() {
            Retention::Never => (
                tr(cx, ids::hv::CONN_RET_NEVER, NoArgs),
                tr(cx, ids::hv::CONN_RET_NEVER_H, NoArgs),
            ),
            Retention::UntilRestart => (
                tr(cx, ids::hv::CONN_RET_RESTART, NoArgs),
                tr(cx, ids::hv::CONN_RET_RESTART_H, NoArgs),
            ),
            Retention::ForSession => (
                tr(cx, ids::hv::CONN_RET_SESSION, NoArgs),
                tr(cx, ids::hv::CONN_RET_SESSION_H, NoArgs),
            ),
            Retention::Forever => (
                tr(cx, vault_dialog::forever_label(cx), NoArgs),
                tr(cx, vault_dialog::forever_hint(cx), NoArgs),
            ),
        };
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(t.spacing.sm))
            .child(
                Label::new(
                    label,
                    TextSource::Fluent,
                    t.type_ramp.body,
                    c.text.to_hsla(),
                )
                .id("secret-remember"),
            )
            .child(Label::new(
                hint,
                TextSource::Fluent,
                t.type_ramp.caption,
                c.text_2.to_hsla(),
            ))
    }
}

impl Render for SecretPromptView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let hop = self.hop();
        let (title, body) = match &self.prompt {
            SecretPrompt::SshPassword { host, user, .. } => (
                tr(
                    cx,
                    ids::hv::CONN_SSH_SECRET_PW_TITLE,
                    ids::hv::HvConnSshSecretPwTitle {
                        host: Text(host.clone()),
                    },
                ),
                tr(
                    cx,
                    ids::hv::CONN_SSH_SECRET_PW_BODY,
                    ids::hv::HvConnSshSecretPwBody {
                        user: Text(format!("{user}@{host}")),
                    },
                ),
            ),
            SecretPrompt::SshPassphrase { key, .. } | SecretPrompt::TlsKeyPassphrase { key } => (
                tr(
                    cx,
                    ids::hv::CONN_SSH_SECRET_PASS_TITLE,
                    ids::hv::HvConnSshSecretPassTitle {
                        key: Text(shown_path(key)),
                    },
                ),
                tr(cx, ids::hv::CONN_SSH_SECRET_PASS_BODY, NoArgs),
            ),
        };

        // The subtitle: env mark, connection, "host, hop n of total".
        let mut parts = Vec::new();
        let env = self.profile.as_ref().and_then(|s| s.env);
        if let Some(s) = &self.profile {
            parts.push(Part::Mono(s.name.clone().into()));
        }
        let total = self.profile.as_ref().map_or(0, |s| s.hops.len());
        if let Some((n, host)) = &hop
            && total > *n
        {
            parts.push(Part::Fluent(tr(
                cx,
                ids::hv::CONN_SSH_SECRET_HOP,
                ids::hv::HvConnSshSecretHop {
                    host: Text(host.clone()),
                    n: prompt_kit::count(n + 1),
                    total: prompt_kit::count(total),
                },
            )));
        }

        let mut rows = div().flex().flex_col().gap(px(t.spacing.md));
        if let (Some((n, _)), Some(s)) = (&hop, &self.profile)
            && !s.hops.is_empty()
        {
            let os = if prompt_kit::on_mac() {
                ids::hv::HvConnSshThisMacOs::Mac
            } else {
                ids::hv::HvConnSshThisMacOs::Other
            };
            let this = tr(
                cx,
                ids::hv::CONN_SSH_THIS_MAC,
                ids::hv::HvConnSshThisMac { os },
            );
            rows = rows.child(row(
                &theme,
                tr(cx, ids::hv::CONN_SSH_ROUTE, NoArgs),
                route_row(&theme, this, &s.hops, &s.database, Some(*n)),
            ));
        }
        let (field_label, wrong) = match &self.prompt {
            SecretPrompt::SshPassword { host, user, .. } => {
                rows = rows.child(row(
                    &theme,
                    tr(cx, ids::hv::CONN_SSH_USER, NoArgs),
                    static_value(&theme, user.clone(), "secret-user"),
                ));
                (
                    tr(cx, ids::hv::CONN_DLG_PASSWORD, NoArgs),
                    tr(
                        cx,
                        ids::hv::CONN_SSH_SECRET_WRONG,
                        ids::hv::HvConnSshSecretWrong {
                            host: Text(host.clone()),
                        },
                    ),
                )
            }
            SecretPrompt::SshPassphrase { key, .. } => {
                rows = rows.child(row(
                    &theme,
                    tr(cx, ids::hv::CONN_SSH_KEY, NoArgs),
                    static_value(&theme, shown_path(key), "secret-key"),
                ));
                (
                    tr(cx, ids::hv::CONN_SSH_PASSPHRASE, NoArgs),
                    tr(cx, ids::hv::CONN_SSH_SECRET_PASS_WRONG, NoArgs),
                )
            }
            SecretPrompt::TlsKeyPassphrase { key } => {
                rows = rows.child(row(
                    &theme,
                    tr(cx, ids::hv::CONN_DLG_KEY, NoArgs),
                    static_value(&theme, shown_path(key), "secret-key"),
                ));
                (
                    tr(cx, ids::hv::CONN_SSH_PASSPHRASE, NoArgs),
                    tr(cx, ids::hv::CONN_SSH_SECRET_PASS_WRONG, NoArgs),
                )
            }
        };
        rows = rows.child(row(
            &theme,
            field_label,
            div()
                .debug_selector(|| "secret-input".to_string())
                .w(px(FIELD_PX))
                .max_w_full()
                .child(self.input.clone()),
        ));
        // The refusal sits right under the field it is about (NOTES M4).
        if self.attempt > 1 {
            rows = rows.child(under_field(
                &theme,
                meta_line(&theme)
                    .child(alert_line(&theme, wrong, "secret-wrong"))
                    .child(meta_item(
                        &theme,
                        tr(
                            cx,
                            ids::hv::CONN_SSH_SECRET_ATTEMPT,
                            ids::hv::HvConnSshSecretAttempt {
                                n: prompt_kit::count(usize::from(self.attempt)),
                            },
                        ),
                        "secret-attempt",
                    )),
            ));
        }
        rows = rows.child(row(
            &theme,
            tr(cx, ids::hv::CONN_DLG_REMEMBER, NoArgs),
            self.remember(&theme, cx),
        ));

        let cancel = Button::new("secret-cancel", tr(cx, ids::hv::CONN_PW_CANCEL, NoArgs))
            .kind(ButtonKind::Ghost)
            .focus_handle(self.cancel_focus.clone())
            .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx)));
        let go = Button::new(
            "secret-continue",
            tr(cx, ids::hv::CONN_SSH_SECRET_CONTINUE, NoArgs),
        )
        .kind(ButtonKind::Primary)
        .focus_handle(self.continue_focus.clone())
        .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx)));
        let subtitle = (!parts.is_empty() || env.is_some()).then(|| subtitle(&theme, env, parts));
        let card = card(&theme, DialogSize::Wide, SECRET_LAYER_ID)
            .track_focus(&self.root)
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, _| {
                this.enter.note(event);
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            .child(header(
                &theme,
                title.clone(),
                "secret-title",
                subtitle,
                cx.listener(|this, _, window, cx| this.cancel(window, cx)),
                cx,
            ))
            .child(
                paragraph(
                    body.clone(),
                    TextSource::Fluent,
                    t.type_ramp.body,
                    c.text_2.to_hsla(),
                )
                .id("secret-body"),
            )
            .child(rows)
            .child(footer(
                &theme,
                Vec::new(),
                vec![cancel.into_any_element(), go.into_any_element()],
            ));
        A11yNode::new("secret-prompt-node", Role::Dialog, title, true, card).description(body)
    }
}

/// Asks `q` (`Question::Secret`); the answer goes to `reply`. Any other question, or a window
/// without a workspace, answers `Cancelled`.
pub(crate) fn open(q: Question, reply: oneshot::Sender<Answer>, window: &mut Window, cx: &mut App) {
    let Question::Secret {
        profile,
        prompt,
        attempt,
        ..
    } = q
    else {
        let _ = reply.send(Answer::Cancelled);
        return;
    };
    let subject = subject(cx, profile);
    let (answer_tx, answer_rx) = oneshot::channel();
    let view = cx.new(|cx| {
        let v = SecretPromptView::new((prompt, attempt, profile), subject, answer_tx, window, cx);
        v.probe_keyring(cx);
        v
    });
    let (field, input, cancel, go, remember) = {
        let v = view.read(cx);
        (
            v.input.read(cx).focus_handle(cx),
            v.input.clone(),
            v.cancel_focus.clone(),
            v.continue_focus.clone(),
            v.remember.as_ref().map(|s| s.read(cx).focus_handle(cx)),
        )
    };
    register_part(cx, "secret-input", &field, Some(&input));
    register_part(cx, "secret-cancel", &cancel, None);
    register_part(cx, "secret-continue", &go, None);
    if let Some(remember) = &remember {
        register_part(cx, "secret-remember", remember, None);
    }
    let id = view.entity_id();
    if !prompt_kit::push_modal(SECRET_LAYER_ID, view.into(), &field, window, cx) {
        return;
    }
    prompt_kit::forward(reply, answer_rx, window.window_handle(), id, cx);
}
