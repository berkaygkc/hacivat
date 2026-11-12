//! The production-tag prompt before a first connect (NOTES surface 4, D-004, R22, Task 20).
//!
//! A quiet question, not an alarm: a neutral title, no warning icon, the consequences in plain
//! words. A 480 px modal layer `production-prompt` with:
//! - the title "How careful should Hacivat be with <name>?" and a line saying the host isn't
//!   on this Mac and the choice can change later;
//! - when `production_hint` matched (P7), "Suggested because the host name contains “prod”."
//!   (or "…the name contains…") and Production preselected;
//! - four choice rows: Production, Staging, Test, Dev, each with its mark. Local is not offered:
//!   the host was already judged not local;
//! - with Production chosen, what changes (the capsule word in red small caps with the
//!   swatch, never a second filled badge; confirmations; manual commit); with any other, one
//!   calm line;
//! - Cancel and the primary, which names the result ("Save as production"), or reads "Pick an
//!   environment", disabled, while nothing is chosen.
//!
//! Enter confirms; Escape (or Cancel) answers `None`. Tab is trapped in the prompt.

use std::panic::Location;

use hacivat_l10n::{NoArgs, Text, ids};
use hacivat_proto::EnvTag;
use hacivat_session::profile::HintSource;
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::capsule::{EnvKind, env_swatch};
use hacivat_ui::choice_list::{Choice, ChoiceEvent, ChoiceList};
use hacivat_ui::dialog::{DialogSize, dialog_frame};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::popover::A11yNode;
use hacivat_ui::text::{Label, TextSource};
use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext as _, Context, Div, Entity, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement, Render, Role,
    SharedString, Styled, Subscription, Window, div, px,
};
use hacivat_ui_core::theme::Theme;
use tokio::sync::oneshot;

use crate::connections::error_view::{paragraph, record_text};
use crate::l10n_global::{text_fn, tr};
use crate::shell::Workspace;
use crate::shell::layers::{DIALOG_CONTEXT, Layer};

/// The prompt's layer ID.
pub const PROD_LAYER_ID: &str = "production-prompt";

/// The tags offered, in row order (Local is never offered).
const TAGS: [EnvTag; 4] = [
    EnvTag::Production,
    EnvTag::Staging,
    EnvTag::Test,
    EnvTag::Dev,
];

/// The production-tag prompt (NOTES surface 4).
pub struct ProdPrompt {
    name: String,
    host: String,
    hint: Option<(&'static str, HintSource)>,
    choices: Entity<ChoiceList>,
    reply: Option<oneshot::Sender<Option<EnvTag>>>,
    root: FocusHandle,
    cancel_focus: FocusHandle,
    save_focus: FocusHandle,
    _subscription: Subscription,
}

impl Focusable for ProdPrompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root.clone()
    }
}

impl ProdPrompt {
    fn new(
        name: String,
        host: String,
        hint: Option<(&'static str, HintSource)>,
        reply: oneshot::Sender<Option<EnvTag>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let rows = vec![
            choice(
                EnvKind::Production,
                ids::hv::CONN_PROD_OPT_PRODUCTION,
                ids::hv::CONN_PROD_OPT_PRODUCTION_D,
            ),
            choice(
                EnvKind::Staging,
                ids::hv::CONN_PROD_OPT_STAGING,
                ids::hv::CONN_PROD_OPT_STAGING_D,
            ),
            choice(
                EnvKind::Test,
                ids::hv::CONN_PROD_OPT_TEST,
                ids::hv::CONN_PROD_OPT_TEST_D,
            ),
            choice(
                EnvKind::Dev,
                ids::hv::CONN_PROD_OPT_DEV,
                ids::hv::CONN_PROD_OPT_DEV_D,
            ),
        ];
        // P7: Production is preselected only when the host or the name matched.
        let preselected = hint.map(|_| 0);
        let choices = cx.new(|cx| {
            let mut list = ChoiceList::new(rows, preselected, cx);
            list.set_label(text_fn(ids::hv::CONN_PROD_CHOICES));
            list
        });
        let subscription = cx.subscribe(&choices, |_, _, _: &ChoiceEvent, cx| cx.notify());
        ProdPrompt {
            name,
            host,
            hint,
            choices,
            reply: Some(reply),
            root: cx.focus_handle(),
            cancel_focus: cx.focus_handle().tab_stop(true),
            save_focus: cx.focus_handle().tab_stop(true),
            _subscription: subscription,
        }
    }

    fn selected(&self, cx: &App) -> Option<EnvTag> {
        self.choices
            .read(cx)
            .selected()
            .and_then(|i| TAGS.get(i).copied())
    }

    fn answer(&mut self, tag: Option<EnvTag>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(tag);
        }
        window.defer(cx, close_layer);
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tag) = self.selected(cx) {
            self.answer(Some(tag), window, cx);
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        let m = &stroke.modifiers;
        let plain = !(m.control || m.platform || m.alt || m.shift);
        match stroke.key.as_str() {
            "escape" => self.answer(None, window, cx),
            // Enter confirms, except on Cancel (which it presses); a held key never confirms.
            "enter" if plain && !event.is_held => {
                if self.cancel_focus.is_focused(window) {
                    self.answer(None, window, cx);
                } else {
                    self.confirm(window, cx);
                }
            }
            "tab" if !m.control && !m.platform => {
                let mut order = vec![
                    self.choices.read(cx).focus_handle(cx),
                    self.cancel_focus.clone(),
                ];
                if self.selected(cx).is_some() {
                    order.push(self.save_focus.clone());
                }
                let n = order.len();
                let at = order.iter().position(|h| h.is_focused(window));
                let next = match (at, m.shift) {
                    (Some(i), false) => (i + 1) % n,
                    (Some(i), true) => (i + n - 1) % n,
                    (None, false) => 0,
                    (None, true) => n - 1,
                };
                window.focus(&order[next], cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    /// What changes with Production (NOTES §4): the capsule's word in red small caps beside the
    /// production swatch (the swatch is the one filled mark), confirmations, manual commit.
    fn consequences(&self, cx: &mut Context<Self>) -> Div {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let word = tr(cx, ids::hv::CONN_PROD_C1_WORD, NoArgs);
        let c1 = tr(
            cx,
            ids::hv::CONN_PROD_C1,
            ids::hv::HvConnProdC1 {
                word: Text(word.to_string()),
            },
        );
        let line = |mark: AnyElement, text: AnyElement| {
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(t.spacing.md))
                .child(
                    div()
                        .w(px(16.0))
                        .h(px(t.type_ramp.body_sm.line_height))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .child(mark),
                )
                .child(div().flex_1().min_w_0().child(text))
        };
        let icon = |name| {
            Icon::new(name)
                .size(IconSize::Chrome)
                .color(c.text_2.to_hsla())
                .into_any_element()
        };
        let text = |s: SharedString, id: &'static str, cx: &mut App| {
            paragraph(
                s,
                TextSource::Fluent,
                t.type_ramp.body_sm,
                c.text.to_hsla(),
                Some(id),
                cx,
            )
        };
        // The word is the message's one placeable: split on its isolation marks so it can be
        // drawn in red small caps (the word is pre-cased in the FTL, never cased here).
        // The sentence wraps word by word (a long locale never clips), and is recorded once as
        // the whole message; the word itself is drawn in red small caps.
        let c1_el = match split_isolated(&c1) {
            Some((before, word, after)) => {
                record_text(
                    &c1,
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    Some("prod-c1"),
                    Location::caller(),
                    cx,
                );
                let style = t.type_ramp.body_sm;
                let plain = |token: &str| {
                    div()
                        .flex_none()
                        .font(font_for(&style))
                        .text_size(px(style.size_px))
                        .line_height(px(style.line_height))
                        .text_color(c.text.to_hsla())
                        .child(SharedString::from(token.to_string()))
                        .into_any_element()
                };
                let mut tokens: Vec<AnyElement> = before.split_inclusive(' ').map(plain).collect();
                tokens.push(
                    Label::new(
                        SharedString::from(word),
                        TextSource::Fluent,
                        t.type_ramp.label_sc,
                        c.red.to_hsla(),
                    )
                    .id("prod-word")
                    .into_any_element(),
                );
                tokens.extend(after.split_inclusive(' ').map(plain));
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_baseline()
                    .children(tokens)
                    .into_any_element()
            }
            None => text(c1, "prod-c1", cx),
        };
        let c2 = tr(cx, ids::hv::CONN_PROD_C2, NoArgs);
        let c3 = tr(cx, ids::hv::CONN_PROD_C3, NoArgs);
        div()
            .debug_selector(|| "prod-changes".to_string())
            .flex()
            .flex_col()
            .gap(px(t.spacing.md))
            .child(paragraph(
                tr(cx, ids::hv::CONN_PROD_CHANGES, NoArgs),
                TextSource::Fluent,
                t.type_ramp.body_sm,
                c.text_2.to_hsla(),
                Some("prod-changes"),
                cx,
            ))
            .child(line(
                env_swatch(EnvKind::Production, &theme).into_any_element(),
                c1_el,
            ))
            .child(line(icon(IconName::Check), text(c2, "prod-c2", cx)))
            .child(line(icon(IconName::GitCommit), text(c3, "prod-c3", cx)))
    }
}

impl Render for ProdPrompt {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let title = tr(
            cx,
            ids::hv::CONN_PROD_TITLE,
            ids::hv::HvConnProdTitle {
                name: Text(self.name.clone()),
            },
        );
        let body = tr(
            cx,
            ids::hv::CONN_PROD_BODY,
            ids::hv::HvConnProdBody {
                host: Text(self.host.clone()),
            },
        );
        let suggest = self.hint.map(|(token, source)| {
            let token = Text(token.to_string());
            let line = match source {
                HintSource::Host => tr(
                    cx,
                    ids::hv::CONN_PROD_SUGGEST,
                    ids::hv::HvConnProdSuggest { token },
                ),
                HintSource::Name => tr(
                    cx,
                    ids::hv::CONN_PROD_SUGGEST_NAME,
                    ids::hv::HvConnProdSuggestName { token },
                ),
            };
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(t.spacing.sm))
                .child(
                    Icon::new(IconName::Info)
                        .size(IconSize::Chrome)
                        .color(c.text_2.to_hsla()),
                )
                .child(
                    Label::new(
                        line,
                        TextSource::Fluent,
                        t.type_ramp.caption,
                        c.text_2.to_hsla(),
                    )
                    .id("prod-suggest"),
                )
        });
        let selected = self.selected(cx);
        let after: Option<AnyElement> = match selected {
            Some(EnvTag::Production) => Some(self.consequences(cx).into_any_element()),
            Some(_) => Some(paragraph(
                tr(cx, ids::hv::CONN_PROD_OTHER, NoArgs),
                TextSource::Fluent,
                t.type_ramp.body_sm,
                c.text_2.to_hsla(),
                Some("prod-calm"),
                cx,
            )),
            None => None,
        };
        let primary_label = match selected {
            Some(tag) => {
                use ids::hv::HvConnProdSaveEnv as E;
                let env = match tag {
                    EnvTag::Production => E::Production,
                    EnvTag::Staging => E::Staging,
                    EnvTag::Test => E::Test,
                    EnvTag::Dev | EnvTag::Local => E::Dev,
                };
                tr(cx, ids::hv::CONN_PROD_SAVE, ids::hv::HvConnProdSave { env })
            }
            None => tr(cx, ids::hv::CONN_PROD_PICK, NoArgs),
        };
        let cancel = Button::new("prod-cancel", tr(cx, ids::hv::CONN_PROD_CANCEL, NoArgs))
            .kind(ButtonKind::Ghost)
            .focus_handle(self.cancel_focus.clone())
            .on_click(cx.listener(|this, _, window, cx| this.answer(None, window, cx)));
        let save = Button::new("prod-save", primary_label)
            .kind(ButtonKind::Primary)
            .disabled(selected.is_none())
            .focus_handle(self.save_focus.clone())
            .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx)));
        let card = dialog_frame(&theme, DialogSize::Wide)
            .id(PROD_LAYER_ID)
            .debug_selector(|| PROD_LAYER_ID.to_string())
            .track_focus(&self.root)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            // Enter acts on key down only; the focused button's key-up click is suppressed (as
            // in `Dialog`).
            .capture_key_up(|event, window, _| {
                let stroke = &event.keystroke;
                if stroke.key == "enter" && !stroke.modifiers.modified() {
                    window.prevent_default();
                }
            })
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
                c.text.to_hsla(),
                Some("prod-body"),
                cx,
            ))
            .children(suggest)
            .child(self.choices.clone())
            .children(after)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(px(t.spacing.md))
                    .child(cancel)
                    .child(save),
            );
        A11yNode::new("production-prompt-node", Role::Dialog, title, true, card).description(body)
    }
}

/// A choice row for `kind`, its label and description.
fn choice(
    kind: EnvKind,
    label: hacivat_l10n::MsgRef<NoArgs>,
    description: hacivat_l10n::MsgRef<NoArgs>,
) -> Choice {
    Choice {
        mark: Some(kind),
        label: text_fn(label),
        description: text_fn(description),
    }
}

/// Splits a rendered message around its first isolated placeable (U+2068 … U+2069): the text
/// before, the placeable, the text after.
fn split_isolated(s: &str) -> Option<(String, String, String)> {
    let open = s.find('\u{2068}')?;
    let rest = &s[open + '\u{2068}'.len_utf8()..];
    let close = rest.find('\u{2069}')?;
    Some((
        s[..open].to_string(),
        rest[..close].to_string(),
        rest[close + '\u{2069}'.len_utf8()..].to_string(),
    ))
}

/// Pops the prompt when it is the top layer.
fn close_layer(window: &mut Window, cx: &mut App) {
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    ws.update(cx, |ws, cx| {
        if ws.layer_ids().last() == Some(&PROD_LAYER_ID) {
            ws.pop_layer(window, cx);
        }
    });
}

/// Asks which environment `name` at `host` is (NOTES surface 4); `hint` is `production_hint`'s
/// result (P7). Resolves with the chosen tag, or `None` when cancelled. The prompt stacks over
/// the window's top layer (the connection dialog); without a workspace it answers `None`.
pub fn ask_environment(
    name: String,
    host: String,
    hint: Option<(&'static str, HintSource)>,
    window: &mut Window,
    cx: &mut App,
) -> oneshot::Receiver<Option<EnvTag>> {
    let (reply, answer) = oneshot::channel();
    let Some(ws) = window.root::<Workspace>().flatten() else {
        let _ = reply.send(None);
        return answer;
    };
    let prompt = cx.new(|cx| ProdPrompt::new(name, host, hint, reply, cx));
    let focus = prompt.read(cx).choices.read(cx).focus_handle(cx);
    ws.update(cx, |ws, cx| {
        ws.push_layer(
            Layer {
                id: PROD_LAYER_ID,
                context: DIALOG_CONTEXT,
                view: prompt.into(),
                restore_focus: None,
                modal: true,
            },
            window,
            cx,
        );
    });
    window.focus(&focus, cx);
    answer
}

// ## shots fixtures (R28, P19)

/// The prompt with Production preselected from the host name.
#[cfg(feature = "shots")]
pub fn preselected_fixture(window: &mut Window, cx: &mut App) {
    // The shot shows the prompt; nobody waits for its answer.
    drop(ask_environment(
        "orders-prod-us".to_string(),
        "db-prod-us.acme.io".to_string(),
        Some(("prod", HintSource::Host)),
        window,
        cx,
    ));
}

/// The prompt with nothing chosen yet (the primary disabled).
#[cfg(feature = "shots")]
pub fn unset_fixture(window: &mut Window, cx: &mut App) {
    drop(ask_environment(
        "crm".to_string(),
        "10.0.4.7".to_string(),
        None,
        window,
        cx,
    ));
}

#[cfg(test)]
mod tests {
    use super::split_isolated;

    #[test]
    fn the_placeable_is_split_out_on_its_isolation_marks() {
        let s = "shows the \u{2068}PRODUCTION\u{2069} capsule";
        assert_eq!(
            split_isolated(s),
            Some((
                "shows the ".to_string(),
                "PRODUCTION".to_string(),
                " capsule".to_string()
            ))
        );
        assert_eq!(split_isolated("no marks"), None);
    }
}
