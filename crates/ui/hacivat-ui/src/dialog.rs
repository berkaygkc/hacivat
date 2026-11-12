//! Modal dialogs: a title, a body and buttons on a popover surface. The host (Task 13's
//! `Layers`) places the view and dims what is behind it.
//!
//! The dialog traps focus: Tab and Shift-Tab cycle its buttons, the `primary` button has the
//! initial focus, Escape cancels, Enter presses the focused button, and on Windows and Linux
//! Alt plus a button's access key presses it. Every string is a [`TextFn`] re-read on each
//! render, so an open dialog follows a language switch (F16).
//!
//! Keys are handled in the bubble phase, after the focused widget: a widget inside the dialog
//! (an open `Select` menu) that handles Escape or Enter stops them, and the dialog stays open.
//! Enter presses a button only on key down; the dialog prevents the default on Enter's key up so
//! GPUI's keyboard click on the focused `Button` never presses it a second time.

use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, AppContext, Context, Div, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement, Render,
    Role, StatefulInteractiveElement, Styled, Window, div, px,
};
use hacivat_ui_core::theme::{Theme, popover_surface};

use crate::accesskey::{AccessKey, AccessKeyAssigner, AccessKeyLocale, typed_matches};
use crate::button::{Button, ButtonKind};
use crate::icon::{Icon, IconName, IconSize};
use crate::l10n::ui_text;
use crate::popover::A11yNode;
use crate::popover::Declare;
use crate::text::{KeyFn, Label, TextFn, TextSource};

/// Width of the dialog card.
pub const DIALOG_WIDTH_PX: f32 = 420.0;

pub struct DialogButton {
    pub label: TextFn,
    pub kind: ButtonKind,
    /// The catalog's access key for this button (Windows and Linux).
    pub accesskey: Option<KeyFn>,
    /// The English message's access key: the fallback when the label has no usable letter.
    pub english_accesskey: Option<char>,
}

pub struct DialogSpec {
    pub title: TextFn,
    pub body: TextFn,
    pub buttons: Vec<DialogButton>,
    /// The button Escape stands for.
    pub cancel: usize,
    /// The button with the initial focus.
    pub primary: usize,
    pub size: DialogSize,
}

/// A dialog card's width class (NOTES "NEW primitives": dialog sizes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DialogSize {
    /// 420 px: confirmations and short notices.
    Standard,
    /// 480 px: the error and production-tag dialogs.
    Wide,
    /// 640 px: the connection form.
    Form,
}

impl DialogSize {
    pub fn width_px(self) -> f32 {
        match self {
            DialogSize::Standard => DIALOG_WIDTH_PX,
            DialogSize::Wide => 480.0,
            DialogSize::Form => 640.0,
        }
    }
}

/// The card of a dialog of `size` (NOTES "NEW primitives": dialog sizes): the popover surface
/// (raised, 1 px `seam-strong`, radius 6, popover shadow), 16 px padding and a 12 px gap between
/// its parts, stacked in a column no taller than its host.
pub fn dialog_frame(theme: &Theme, size: DialogSize) -> Div {
    let t = theme.tokens;
    popover_surface(theme)
        .w(px(size.width_px()))
        .max_h_full()
        .p(px(t.spacing.s_3xl))
        .flex()
        .flex_col()
        .gap(px(t.spacing.xl))
}

/// The form dialog (640 px): a header (padding 12/16, `seam` below), a scrolling body (padding
/// 12/16/14), an optional strip (the connection test, which draws its own seam and fill) and a
/// footer (padding 12/16, `seam` above). The body scrolls; the header, strip and footer stay
/// put. Debug selectors: `form-dialog`, `form-dialog-header`, `-body` and `-footer`.
pub fn form_dialog(
    theme: &Theme,
    header: AnyElement,
    body: AnyElement,
    strip: Option<AnyElement>,
    footer: AnyElement,
) -> Div {
    let t = theme.tokens;
    let seam = t.colors.seam.to_hsla();
    let edge = |name: &'static str| {
        div()
            .debug_selector(move || name.to_string())
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(t.spacing.md))
            .py(px(t.spacing.xl))
            .px(px(t.spacing.s_3xl))
            .border_color(seam)
    };
    dialog_frame(theme, DialogSize::Form)
        .debug_selector(|| "form-dialog".to_string())
        .p(px(0.0))
        .gap(px(0.0))
        .child(edge("form-dialog-header").border_b_1().child(header))
        .child(
            div()
                .id("form-dialog-body")
                .debug_selector(|| "form-dialog-body".to_string())
                .flex()
                .flex_col()
                .flex_shrink(1.0)
                .min_h_0()
                .overflow_y_scroll()
                .pt(px(t.spacing.xl))
                .px(px(t.spacing.s_3xl))
                .pb(px(t.spacing.s_2xl))
                .child(body),
        )
        .children(strip)
        .child(edge("form-dialog-footer").border_t_1().child(footer))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogEvent {
    /// A button was pressed (clicked, or chosen with Enter or its access key).
    Pressed(usize),
    /// Escape, or the close button.
    Cancelled,
}

pub struct Dialog {
    spec: DialogSpec,
    buttons: Vec<FocusHandle>,
    root: FocusHandle,
    access_keys: bool,
    content: Option<AnyView>,
}

impl EventEmitter<DialogEvent> for Dialog {}

impl Focusable for Dialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.root.clone()
    }
}

impl Dialog {
    /// Builds the dialog and focuses its `primary` button.
    pub fn new(spec: DialogSpec, window: &mut Window, cx: &mut App) -> Entity<Dialog> {
        cx.new(|cx| {
            debug_assert!(
                !spec.buttons.is_empty(),
                "a dialog needs at least one button"
            );
            debug_assert!(
                spec.primary < spec.buttons.len() && spec.cancel < spec.buttons.len(),
                "dialog primary ({}) and cancel ({}) must index its {} buttons",
                spec.primary,
                spec.cancel,
                spec.buttons.len()
            );
            let buttons: Vec<FocusHandle> =
                spec.buttons.iter().map(|_| cx.focus_handle()).collect();
            // A destructive primary never has the initial focus: a stray Enter must not delete.
            let destructive =
                spec.buttons.get(spec.primary).map(|b| b.kind) == Some(ButtonKind::Danger);
            let initial = if destructive {
                spec.cancel
            } else {
                spec.primary
            };
            if let Some(handle) = buttons.get(initial) {
                window.focus(handle, cx);
            }
            Dialog {
                spec,
                buttons,
                root: cx.focus_handle(),
                access_keys: !cfg!(target_os = "macos"),
                content: None,
            }
        })
    }

    /// Whether access keys are shown and accepted (default: Windows and Linux).
    pub fn set_access_keys(&mut self, on: bool) {
        self.access_keys = on;
    }

    /// A view drawn between the body text and the buttons (a control the answer needs).
    pub fn set_content(&mut self, content: AnyView) {
        self.content = Some(content);
    }

    /// The index of the focused button, if one is.
    pub fn focused_button(&self, window: &Window) -> Option<usize> {
        self.buttons.iter().position(|h| h.is_focused(window))
    }

    fn move_focus(&self, forward: bool, window: &mut Window, cx: &mut App) {
        let order: Vec<&FocusHandle> = self.buttons.iter().collect();
        let n = order.len();
        let at = order.iter().position(|h| h.is_focused(window));
        let next = match (at, forward) {
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
            (None, true) => 0,
            (None, false) => n - 1,
        };
        window.focus(order[next], cx);
    }

    fn access_keys_of_buttons(&self, cx: &App) -> Vec<AccessKey> {
        let labels: Vec<_> = self
            .spec
            .buttons
            .iter()
            .map(|b| {
                (
                    (b.label)(cx),
                    b.accesskey.as_ref().and_then(|k| k(cx)),
                    b.english_accesskey,
                )
            })
            .collect();
        let rows: Vec<(&str, Option<char>, Option<char>)> = labels
            .iter()
            .map(|(l, k, e)| (l.as_ref(), *k, *e))
            .collect();
        AccessKeyAssigner::assign(&rows)
    }

    fn primary_is_destructive(&self) -> bool {
        self.spec.buttons.get(self.spec.primary).map(|b| b.kind) == Some(ButtonKind::Danger)
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        match stroke.key.as_str() {
            "tab" => self.move_focus(!stroke.modifiers.shift, window, cx),
            "escape" => cx.emit(DialogEvent::Cancelled),
            // A held key (auto-repeat) or a modified Enter never presses a button.
            "enter" if event.is_held || has_modifier(stroke) => {}
            "enter" => match self.focused_button(window) {
                Some(i) => cx.emit(DialogEvent::Pressed(i)),
                None if self.primary_is_destructive() => {}
                None => cx.emit(DialogEvent::Pressed(self.spec.primary)),
            },
            key if self.access_keys && stroke.modifiers.alt && key.chars().count() == 1 => {
                let typed = key.chars().next().unwrap_or_default();
                let turkic = AccessKeyLocale::turkic(cx);
                let keys = self.access_keys_of_buttons(cx);
                let hit = keys.iter().enumerate().find(|&(i, k)| {
                    let label = (self.spec.buttons[i].label)(cx);
                    match k {
                        AccessKey::InLabel(at) => label[*at..].chars().next(),
                        AccessKey::Suffix(c) => Some(*c),
                        AccessKey::None => None,
                    }
                    .is_some_and(|c| typed_matches(c, typed, turkic))
                });
                match hit {
                    Some((i, _)) => cx.emit(DialogEvent::Pressed(i)),
                    None => return,
                }
            }
            _ => return,
        }
        cx.notify();
        cx.stop_propagation();
    }
}

fn has_modifier(stroke: &hacivat_ui_core::gpui_surface::Keystroke) -> bool {
    let m = &stroke.modifiers;
    m.control || m.platform || m.shift || m.alt
}

impl Render for Dialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let title = (self.spec.title)(cx);
        let body = (self.spec.body)(cx);
        let body_text = body.clone();
        let keys = if self.access_keys {
            self.access_keys_of_buttons(cx)
        } else {
            vec![AccessKey::None; self.spec.buttons.len()]
        };

        let buttons: Vec<_> = self
            .spec
            .buttons
            .iter()
            .enumerate()
            .map(|(i, b)| {
                Button::new(("dialog-button", i), (b.label)(cx))
                    .kind(b.kind)
                    .focus_handle(self.buttons[i].clone())
                    .access_key(keys[i])
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(DialogEvent::Pressed(i))))
            })
            .collect();

        // The close button is for the pointer and assistive technology; keyboard users have
        // Escape, so it is not a tab stop.
        let close = div()
            .id("dialog-close")
            .debug_selector(|| "dialog-close".to_string())
            .declare(Role::Button, ui_text(cx, "hui-dialog-close"), None)
            .flex()
            .items_center()
            .flex_none()
            .p(px(t.spacing.xs))
            .rounded(px(t.radii.sm))
            .cursor_pointer()
            .on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Cancelled)))
            .child(
                // Raised Ink Rule: text-2 on the raised dialog surface, never text-3.
                Icon::new(IconName::X)
                    .size(IconSize::Chrome)
                    .color(c.text_2.to_hsla()),
            );

        let card =
            dialog_frame(&theme, self.spec.size)
                .id("dialog")
                .debug_selector(|| "dialog".to_string())
                .track_focus(&self.root)
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    this.on_key(event, window, cx)
                }))
                // Enter acts on key down only (`on_key`, with its held, modifier and destructive
                // guards); the key-up keyboard click of the focused button is suppressed.
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
                        .flex_row()
                        .items_start()
                        .justify_between()
                        .gap(px(t.spacing.xl))
                        .child(Label::new(
                            title.clone(),
                            TextSource::Fluent,
                            t.type_ramp.body_strong,
                            c.text.to_hsla(),
                        ))
                        .child(close),
                )
                .child(div().child(Label::new(
                    body,
                    TextSource::Fluent,
                    t.type_ramp.body,
                    c.text_2.to_hsla(),
                )))
                .children(self.content.clone())
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(px(t.spacing.md))
                        .children(buttons),
                );
        A11yNode::new("dialog-node", Role::Dialog, title, true, card).description(body_text)
    }
}
