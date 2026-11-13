//! Keyboard-interactive SSH prompts (RFC 4256; NOTES surface 2, Task 21): one-time codes and
//! whatever else a server asks.
//!
//! A 480 px modal layer `kbd-prompt`: "Sign in to <host>" (the host Hacivat dialled), the
//! subtitle with the environment mark, the connection, the SSH user and the hop, then
//! "<host> asks:" and the server's own words. **Server text is never placed in a translated
//! sentence** (NOTES cross-cutting #3, T9 review M3): the name and the instruction sit in their
//! own mono inset captioned "Sent by the server, shown as received", and each prompt's label
//! sits verbatim above its field. All of it is made visible first ([`prompt_kit::visible`]: a
//! backslash doubled, controls as `\ooo`, bidi and invisible format characters as `\u{…}`) and
//! capped in length; the inset's server text scrolls on its own, at most 119 px (7 mono lines).
//! A round with more than [`MAX_PROMPTS`] prompts is refused (answered `Cancelled`) without a
//! dialog, so a hostile server cannot flood the window with fields.
//!
//! One field per prompt, masked unless the server allows echo; "Attempt n of 3" and "Answers go
//! to this server only and aren't kept." (after a rejected round, the ink alert line first).
//!
//! Keys (NOTES): the focus starts in the first field. Enter in a field that isn't the last moves
//! to the next one and never submits; Enter in the last submits only when every masked field
//! holds an answer, else it moves to the first empty masked field (a half-filled round never
//! spends an attempt, a server `MaxAuthTries` slot or a second push). Continue always submits
//! what is there. Tab walks the fields, Cancel and Continue. Escape, Cancel or the close button
//! answer `Cancelled`. The answers leave as `SecretString`s, in the server's order, and every
//! field is cleared.

// ## m2b-t21-prompts

use hacivat_l10n::{NoArgs, Text, ids};
use hacivat_net::ssh::KbdRequest;
use hacivat_session::pipeline::{Answer, Question};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::dialog::DialogSize;
use hacivat_ui::input::{InputEvent, TextInput};
use hacivat_ui::popover::A11yNode;
use hacivat_ui::text::{Label, TextSource, paragraph, scroll_viewport};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, Role, ScrollHandle,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};
use hacivat_ui_core::theme::Theme;
use secrecy::SecretString;
use tokio::sync::oneshot;

use crate::connections::prompt_kit::{
    self, EnterGate, Part, alert_line, card, clear_all, cycle_focus, footer, header, inset,
    is_empty, meta_item, meta_line, plain_input, register_part, remove_modal, secret_input,
    subject, subtitle, trapped_tab, visible,
};
use crate::l10n_global::tr;

/// The prompt's layer ID.
pub const KBD_LAYER_ID: &str = "kbd-prompt";

/// The most prompts one round may hold; a round with more is refused.
pub const MAX_PROMPTS: usize = 8;
/// The longest server name, instruction and prompt label shown (characters of server text).
const MAX_NAME_CHARS: usize = 256;
const MAX_INSTRUCTIONS_CHARS: usize = 2048;
const MAX_PROMPT_CHARS: usize = 256;
/// The server text's own scroll area (NOTES: 7 mono lines, so the inset stays within 160 px).
const SERVER_TEXT_MAX_PX: f32 = 119.0;
/// The scroll thumb (NOTES: 4 px seam-strong, radius 3, no track).
const THUMB_PX: f32 = 4.0;
const THUMB_MIN_PX: f32 = 16.0;

/// The fields' IDs, labels' IDs and debug selectors, by prompt.
const FIELD_IDS: [&str; MAX_PROMPTS] = [
    "kbd-0", "kbd-1", "kbd-2", "kbd-3", "kbd-4", "kbd-5", "kbd-6", "kbd-7",
];
const LABEL_IDS: [&str; MAX_PROMPTS] = [
    "kbd-label-0",
    "kbd-label-1",
    "kbd-label-2",
    "kbd-label-3",
    "kbd-label-4",
    "kbd-label-5",
    "kbd-label-6",
    "kbd-label-7",
];

/// One prompt of the round: its label (made visible) and its field.
struct Field {
    label: String,
    masked: bool,
    input: Entity<TextInput>,
}

/// The keyboard-interactive prompt (NOTES surface 2).
pub struct KbdPrompt {
    host: String,
    hop: u8,
    attempt: u8,
    profile: Option<prompt_kit::Subject>,
    name: String,
    instructions: String,
    fields: Vec<Field>,
    reply: Option<oneshot::Sender<Answer>>,
    root: FocusHandle,
    cancel_focus: FocusHandle,
    continue_focus: FocusHandle,
    scroll: ScrollHandle,
    enter: EnterGate,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for KbdPrompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root.clone()
    }
}

impl KbdPrompt {
    fn new(
        profile: Option<prompt_kit::Subject>,
        hop: u8,
        request: KbdRequest,
        attempt: u8,
        reply: oneshot::Sender<Answer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut fields = Vec::with_capacity(request.prompts.len());
        let mut subscriptions = Vec::with_capacity(request.prompts.len());
        for (i, p) in request.prompts.iter().enumerate() {
            let input = if p.echo {
                plain_input(cx, false)
            } else {
                secret_input(cx)
            };
            subscriptions.push(cx.subscribe_in(
                &input,
                window,
                move |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Submitted => {
                        if this.enter.take() {
                            this.enter_in(i, window, cx);
                        }
                    }
                    // A focused field swallows Escape (P16): it cancels the prompt.
                    InputEvent::Cancelled => this.cancel(window, cx),
                    _ => {}
                },
            ));
            fields.push(Field {
                label: visible(&p.text, MAX_PROMPT_CHARS),
                masked: !p.echo,
                input,
            });
        }
        KbdPrompt {
            host: request.host,
            hop,
            attempt,
            profile,
            name: visible(&request.name, MAX_NAME_CHARS),
            instructions: visible(&request.instructions, MAX_INSTRUCTIONS_CHARS),
            fields,
            reply: Some(reply),
            root: cx.focus_handle(),
            cancel_focus: cx.focus_handle().tab_stop(true),
            continue_focus: cx.focus_handle().tab_stop(true),
            scroll: ScrollHandle::new(),
            enter: EnterGate::default(),
            _subscriptions: subscriptions,
        }
    }

    fn inputs(&self) -> Vec<Entity<TextInput>> {
        self.fields.iter().map(|f| f.input.clone()).collect()
    }

    fn answer(&mut self, answer: Answer, window: &mut Window, cx: &mut Context<Self>) {
        clear_all(&self.inputs(), cx);
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(answer);
        }
        let view = cx.entity_id();
        window.defer(cx, move |window, cx| remove_modal(view, window, cx));
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.answer(Answer::Cancelled, window, cx);
    }

    /// Continue: the answers, in the server's order, as secrets (an empty field answers "").
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.reply.is_none() {
            return;
        }
        let answers: Vec<SecretString> = self
            .fields
            .iter()
            .map(|f| SecretString::from(f.input.read(cx).value().to_string()))
            .collect();
        self.answer(Answer::Keyboard(answers), window, cx);
    }

    /// Enter in field `i` (NOTES keyboard rules).
    fn enter_in(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        if i + 1 < self.fields.len() {
            let next = self.fields[i + 1].input.read(cx).focus_handle(cx);
            window.focus(&next, cx);
            return;
        }
        let empty_masked = self
            .fields
            .iter()
            .find(|f| f.masked && is_empty(&f.input, cx))
            .map(|f| f.input.read(cx).focus_handle(cx));
        match empty_masked {
            Some(focus) => window.focus(&focus, cx),
            None => self.submit(window, cx),
        }
    }

    fn order(&self, cx: &App) -> Vec<FocusHandle> {
        let mut order: Vec<FocusHandle> = self
            .fields
            .iter()
            .map(|f| f.input.read(cx).focus_handle(cx))
            .collect();
        order.push(self.cancel_focus.clone());
        order.push(self.continue_focus.clone());
        order
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => self.cancel(window, cx),
            // Enter in a field arrives as its `Submitted`; here only the buttons.
            "enter" if prompt_kit::acting_enter(event) => {
                if self.cancel_focus.is_focused(window) {
                    self.cancel(window, cx);
                } else if self.continue_focus.is_focused(window) {
                    self.submit(window, cx);
                }
            }
            "enter" => {}
            _ if trapped_tab(event) => {
                let order = self.order(cx);
                cycle_focus(&order, event.keystroke.modifiers.shift, window, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    /// The server text's scroll thumb, while it overflows (as the sheet's).
    fn thumb(theme: &Theme, scroll: &ScrollHandle) -> Option<AnyElement> {
        let max = f32::from(scroll.max_offset().y);
        let view = f32::from(scroll.bounds().size.height);
        if max < 0.5 || view <= 0.0 {
            return None;
        }
        let t = theme.tokens;
        let length = (view * view / (view + max)).max(THUMB_MIN_PX);
        let at = (-f32::from(scroll.offset().y) / max).clamp(0.0, 1.0) * (view - length);
        Some(
            div()
                .debug_selector(|| "kbd-scroll-thumb".to_string())
                .absolute()
                .right(px(0.0))
                .top(px(at))
                .w(px(THUMB_PX))
                .h(px(length))
                .rounded(px(t.radii.xs))
                .bg(t.colors.seam_strong.to_hsla())
                .into_any_element(),
        )
    }

    /// The server-text inset (NOTES "Server-text inset").
    fn server_text(&self, theme: &Theme, cx: &App) -> AnyElement {
        let t = theme.tokens;
        let c = &t.colors;
        if self.name.is_empty() && self.instructions.is_empty() {
            // Hacivat's own words, in the UI face and without the caption, so they can't be
            // mistaken for server text.
            return inset(theme)
                .child(
                    paragraph(
                        tr(cx, ids::hv::CONN_KBD_NO_TEXT, NoArgs),
                        TextSource::Fluent,
                        t.type_ramp.body_sm,
                        c.text_2.to_hsla(),
                    )
                    .id("kbd-no-text"),
                )
                .into_any_element();
        }
        let mut text = div().flex().flex_col().gap(px(t.spacing.xs)).pr(px(6.0));
        if !self.name.is_empty() {
            text = text.child(
                paragraph(
                    self.name.clone(),
                    TextSource::Data,
                    t.type_ramp.mono_data,
                    c.text.to_hsla(),
                )
                .id("kbd-name"),
            );
        }
        if !self.instructions.is_empty() {
            text = text.child(
                paragraph(
                    self.instructions.clone(),
                    TextSource::Data,
                    t.type_ramp.mono_data,
                    c.text_2.to_hsla(),
                )
                .id("kbd-instructions"),
            );
        }
        let scroll = div()
            .id("kbd-server-text")
            .debug_selector(|| "kbd-server-text".to_string())
            .max_h(px(SERVER_TEXT_MAX_PX))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(scroll_viewport())
            .child(text);
        inset(theme)
            .gap(px(t.spacing.xs))
            .child(Label::new(
                tr(cx, ids::hv::CONN_KBD_FROM_SERVER, NoArgs),
                TextSource::Fluent,
                t.type_ramp.caption,
                c.text_2.to_hsla(),
            ))
            .child(
                div()
                    .relative()
                    .child(scroll)
                    .children(Self::thumb(theme, &self.scroll)),
            )
            .into_any_element()
    }
}

impl Render for KbdPrompt {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let host = Text(self.host.clone());
        let title = tr(
            cx,
            ids::hv::CONN_KBD_TITLE,
            ids::hv::HvConnKbdTitle { host: host.clone() },
        );
        let hop = usize::from(self.hop);
        let mut parts = Vec::new();
        let mut env = None;
        if let Some(s) = &self.profile {
            env = s.env;
            parts.push(Part::Mono(s.name.clone().into()));
            if let Some(h) = s.hops.get(hop).filter(|h| !h.user.is_empty()) {
                parts.push(Part::Mono(h.user.clone().into()));
            }
            if s.hops.len() > hop {
                parts.push(Part::Fluent(tr(
                    cx,
                    ids::hv::CONN_SSH_SECRET_HOP,
                    ids::hv::HvConnSshSecretHop {
                        host: host.clone(),
                        n: prompt_kit::count(hop + 1),
                        total: prompt_kit::count(s.hops.len()),
                    },
                )));
            }
        }
        let asks = paragraph(
            tr(
                cx,
                ids::hv::CONN_KBD_ASKS,
                ids::hv::HvConnKbdAsks { host: host.clone() },
            ),
            TextSource::Fluent,
            t.type_ramp.body,
            c.text_2.to_hsla(),
        )
        .id("kbd-asks");
        let prompts = self.fields.iter().enumerate().map(|(i, f)| {
            div()
                .flex()
                .flex_col()
                .gap(px(t.spacing.xs))
                .child(
                    paragraph(
                        f.label.clone(),
                        TextSource::Data,
                        t.type_ramp.mono_data,
                        c.text.to_hsla(),
                    )
                    .id(LABEL_IDS[i]),
                )
                .child(
                    div()
                        .debug_selector(move || FIELD_IDS[i].to_string())
                        .w_full()
                        .child(f.input.clone()),
                )
        });
        let mut meta = meta_line(&theme);
        if self.attempt > 1 {
            meta = meta.child(alert_line(
                &theme,
                tr(cx, ids::hv::CONN_KBD_REJECTED, NoArgs),
                "kbd-rejected",
            ));
        }
        meta = meta
            .child(meta_item(
                &theme,
                tr(
                    cx,
                    ids::hv::CONN_KBD_ATTEMPT,
                    ids::hv::HvConnKbdAttempt {
                        n: prompt_kit::count(usize::from(self.attempt)),
                    },
                ),
                "kbd-attempt",
            ))
            .child(meta_item(
                &theme,
                tr(cx, ids::hv::CONN_KBD_NOT_KEPT, NoArgs),
                "kbd-not-kept",
            ));
        let cancel = Button::new("kbd-cancel", tr(cx, ids::hv::CONN_PW_CANCEL, NoArgs))
            .kind(ButtonKind::Ghost)
            .focus_handle(self.cancel_focus.clone())
            .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx)));
        let go = Button::new("kbd-continue", tr(cx, ids::hv::CONN_KBD_CONTINUE, NoArgs))
            .kind(ButtonKind::Primary)
            .focus_handle(self.continue_focus.clone())
            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx)));
        let card = card(&theme, DialogSize::Wide, KBD_LAYER_ID)
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
                "kbd-title",
                Some(subtitle(&theme, env, parts)),
                cx.listener(|this, _, window, cx| this.cancel(window, cx)),
                cx,
            ))
            // Fix round 1 (review m5): the body scrolls inside the window, so many long prompts
            // never push the footer (Cancel, Continue) out of the dialog.
            .child(
                div()
                    .id("kbd-body")
                    .debug_selector(|| "kbd-body".to_string())
                    .flex()
                    .flex_col()
                    .gap(px(t.spacing.xl))
                    .flex_shrink(1.0)
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(scroll_viewport())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(t.spacing.sm))
                            .child(asks)
                            .child(self.server_text(&theme, cx)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(t.spacing.lg))
                            .children(prompts),
                    )
                    .child(meta),
            )
            .child(footer(
                &theme,
                Vec::new(),
                vec![cancel.into_any_element(), go.into_any_element()],
            ));
        A11yNode::new("kbd-prompt-node", Role::Dialog, title, true, card)
    }
}

/// Asks `q` (`Question::Keyboard`); the answer goes to `reply`. A round with no prompt answers
/// at once with no answers; a round with more than [`MAX_PROMPTS`] prompts, any other question
/// or a window without a workspace answers `Cancelled`.
pub(crate) fn open(q: Question, reply: oneshot::Sender<Answer>, window: &mut Window, cx: &mut App) {
    let Question::Keyboard {
        profile,
        hop,
        request,
        attempt,
        ..
    } = q
    else {
        let _ = reply.send(Answer::Cancelled);
        return;
    };
    if request.prompts.is_empty() {
        let _ = reply.send(Answer::Keyboard(Vec::new()));
        return;
    }
    if request.prompts.len() > MAX_PROMPTS {
        tracing::warn!(
            prompts = request.prompts.len(),
            "connections: a keyboard-interactive round asked too much; refused"
        );
        let _ = reply.send(Answer::Cancelled);
        return;
    }
    let subject = subject(cx, profile);
    let (answer_tx, answer_rx) = oneshot::channel();
    let prompt = cx.new(|cx| KbdPrompt::new(subject, hop, request, attempt, answer_tx, window, cx));
    let (first, parts) = {
        let p = prompt.read(cx);
        let parts: Vec<(&'static str, FocusHandle, Entity<TextInput>)> = p
            .fields
            .iter()
            .enumerate()
            .map(|(i, f)| {
                (
                    FIELD_IDS[i],
                    f.input.read(cx).focus_handle(cx),
                    f.input.clone(),
                )
            })
            .collect();
        let first = parts.first().map(|(_, focus, _)| focus.clone());
        let buttons = [
            ("kbd-cancel", p.cancel_focus.clone()),
            ("kbd-continue", p.continue_focus.clone()),
        ];
        (first, (parts, buttons))
    };
    let (fields, buttons) = parts;
    for (name, focus, input) in &fields {
        register_part(cx, name, focus, Some(input));
    }
    for (name, focus) in &buttons {
        register_part(cx, name, focus, None);
    }
    let Some(first) = first else {
        return;
    };
    let view = prompt.entity_id();
    if !prompt_kit::push_modal(KBD_LAYER_ID, prompt.into(), &first, window, cx) {
        return;
    }
    prompt_kit::forward(reply, answer_rx, window.window_handle(), view, cx);
}

// ## m2b-t24-shots: shots fixture (R28, P19).
/// The mock's three-prompt round from `bastion.acme.io` (hop 2 of 2): a masked password, then
/// two plain answers, under the server's name and instruction.
#[cfg(feature = "shots")]
pub fn fixture(window: &mut Window, cx: &mut App) {
    use hacivat_net::ssh::KbdPrompt;
    use hacivat_proto::EnvTag;

    use crate::connections::prompt_kit::shots::{held_reply, save_profile, two_hops};

    let id = save_profile(cx, "orders-prod", EnvTag::Production, two_hops());
    let reply = held_reply(cx);
    let prompt = |text: &str, echo: bool| KbdPrompt {
        text: text.to_string(),
        echo,
    };
    open(
        Question::Keyboard {
            profile: id,
            name: "orders-prod".to_string(),
            hop: 1,
            request: KbdRequest {
                host: "bastion.acme.io".to_string(),
                name: "ACME bastion sign-in".to_string(),
                instructions: "All access to this host is logged.".to_string(),
                prompts: vec![
                    prompt("Password: ", false),
                    prompt("Verification code: ", true),
                    prompt("Ticket or reason for access (logged): ", true),
                ],
            },
            attempt: 1,
        },
        reply,
        window,
        cx,
    );
}
