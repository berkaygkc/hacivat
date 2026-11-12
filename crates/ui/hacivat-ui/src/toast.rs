//! Toasts: short notices at the bottom right, above the status bar. At most three show at once
//! (the rest wait their turn), each lives its TTL (6 s by default) unless dismissed, and none
//! animates (named rule). The text is a [`TextFn`], so a visible toast follows a language
//! switch (F16).
//!
//! A sticky toast (NOTES "NEW primitives": toast, sticky + actions) has no TTL and is announced
//! as an alert. It may carry a title (weight 600), the text as a `text-2` body line, a list of
//! `text-2` bullets with 3 px square marks, and a row of buttons; an action runs, then dismisses
//! the toast. A `wide` toast is 380 px instead of 320.
//!
//! [`ToastQueue`] is the model (a `Global`); [`ToastStack`] draws it and is placed by the host
//! (Task 13's `Layers`). Debug selectors: `toast-<id>`, `toast-bullet-<id>-<i>` and the action
//! buttons `button-toast-action-<id>-<i>`.

use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use hacivat_ui_core::gpui_surface::{
    AnyElement, App, ElementId, Global, InteractiveElement, IntoElement, ParentElement, Role,
    SharedString, StatefulInteractiveElement, Styled, Window, accesskit, div, px,
};
use hacivat_ui_core::theme::{Theme, popover_surface};

use crate::actions::ActionFn;
use crate::button::{Button, ButtonKind};
use crate::icon::{Icon, IconName, IconSize};
use crate::l10n::ui_text;
use crate::popover::{A11yNode, Declare, lazy};
use crate::text::{Label, TextFn, TextSource};

/// How many toasts show at once.
pub const MAX_VISIBLE_TOASTS: usize = 3;
/// The default lifetime of a toast.
pub const DEFAULT_TOAST_TTL: Duration = Duration::from_secs(6);
/// Gap between the toast stack and the window edge, and above the status bar.
const EDGE_PX: f32 = 12.0;
/// The toast widths (NOTES: 320 px; 380 px when the content is wide).
const WIDTH_PX: f32 = 320.0;
const WIDE_WIDTH_PX: f32 = 380.0;
/// The square bullet of a list line (NOTES: 3 px, `text-2`).
const BULLET_PX: f32 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    /// Announced politely (`Role::Status`).
    Info,
    /// Announced assertively (`Role::Alert`).
    Error,
}

#[derive(Clone)]
pub struct Toast {
    /// The message; with a `title`, the body line under it (left out when empty).
    pub text: TextFn,
    pub kind: ToastKind,
    pub ttl: Duration,
    // M2a (Task 1; drawn from Task 9 on).
    /// Stays until dismissed or an action runs (no TTL), announced as an alert.
    pub sticky: bool,
    pub title: Option<TextFn>,
    pub bullets: Vec<TextFn>,
    pub actions: Vec<ToastAction>,
    /// The wide card (NOTES sticky toast).
    pub wide: bool,
}

/// A button on a toast (M2a, NOTES surface 4).
#[derive(Clone)]
pub struct ToastAction {
    pub label: TextFn,
    /// `Rc<dyn Fn(&mut Window, &mut App)>`.
    pub run: ActionFn,
}

impl Toast {
    /// A toast with the default TTL.
    pub fn new(kind: ToastKind, text: TextFn) -> Self {
        Self {
            text,
            kind,
            ttl: DEFAULT_TOAST_TTL,
            sticky: false,
            title: None,
            bullets: Vec::new(),
            actions: Vec::new(),
            wide: false,
        }
    }

    /// How long the toast lives on its own; `None` for a sticky toast.
    fn lifetime(&self) -> Option<Duration> {
        (!self.sticky).then_some(self.ttl)
    }
}

/// Pushes one wide sticky toast with `title`, `bullets` and one `action` (which only dismisses
/// it), for the screenshot harness (R28, P19): the caller passes the app's strings, so the
/// widget owns no copy.
pub fn sticky_demo_fixture(
    title: TextFn,
    bullets: Vec<TextFn>,
    action: TextFn,
    _window: &mut Window,
    cx: &mut App,
) {
    let mut toast = Toast::new(ToastKind::Info, Rc::new(|_: &App| SharedString::default()));
    toast.sticky = true;
    toast.wide = true;
    toast.title = Some(title);
    toast.bullets = bullets;
    toast.actions = vec![ToastAction {
        label: action,
        run: Rc::new(|_, _| {}),
    }];
    ToastQueue::push(cx, toast);
}

struct Entry {
    id: u64,
    toast: Toast,
}

/// The toasts on screen and those waiting.
#[derive(Default)]
pub struct ToastQueue {
    next_id: u64,
    visible: Vec<Entry>,
    waiting: VecDeque<Entry>,
}

impl Global for ToastQueue {}

impl ToastQueue {
    /// Shows `toast` now, or after one of the three visible ones goes.
    pub fn push(cx: &mut App, toast: Toast) {
        let started = {
            let q = cx.default_global::<ToastQueue>();
            q.next_id += 1;
            let entry = Entry {
                id: q.next_id,
                toast,
            };
            if q.visible.len() < MAX_VISIBLE_TOASTS {
                let started = (entry.id, entry.toast.lifetime());
                q.visible.push(entry);
                Some(started)
            } else {
                q.waiting.push_back(entry);
                None
            }
        };
        if let Some((id, Some(ttl))) = started {
            Self::start_timer(cx, id, ttl);
        }
        cx.refresh_windows();
    }

    /// Removes a toast (its TTL ran out, the user dismissed it or one of its actions ran) and
    /// promotes a waiting one.
    pub fn dismiss(cx: &mut App, id: u64) {
        let promoted = {
            let q = cx.default_global::<ToastQueue>();
            let before = q.visible.len() + q.waiting.len();
            q.visible.retain(|e| e.id != id);
            q.waiting.retain(|e| e.id != id);
            if q.visible.len() + q.waiting.len() == before {
                return;
            }
            if q.visible.len() < MAX_VISIBLE_TOASTS
                && let Some(entry) = q.waiting.pop_front()
            {
                let started = (entry.id, entry.toast.lifetime());
                q.visible.push(entry);
                Some(started)
            } else {
                None
            }
        };
        if let Some((id, Some(ttl))) = promoted {
            Self::start_timer(cx, id, ttl);
        }
        cx.refresh_windows();
    }

    fn start_timer(cx: &mut App, id: u64, ttl: Duration) {
        cx.spawn(async move |cx| {
            cx.background_executor().timer(ttl).await;
            cx.update(|cx| Self::dismiss(cx, id));
        })
        .detach();
    }

    /// The ids of the visible toasts, oldest first.
    pub fn visible_ids(cx: &App) -> Vec<u64> {
        cx.try_global::<ToastQueue>()
            .map(|q| q.visible.iter().map(|e| e.id).collect())
            .unwrap_or_default()
    }

    /// How many toasts wait for a free slot.
    pub fn waiting(cx: &App) -> usize {
        cx.try_global::<ToastQueue>().map_or(0, |q| q.waiting.len())
    }
}

/// Draws the visible toasts, anchored bottom right above the status bar. The host places it
/// over the whole window.
pub struct ToastStack {
    status_bar_px: f32,
}

impl ToastStack {
    pub fn new() -> Self {
        Self { status_bar_px: 0.0 }
    }

    /// The height of the status bar the stack clears.
    pub fn above(mut self, status_bar_px: f32) -> Self {
        self.status_bar_px = status_bar_px;
        self
    }
}

impl Default for ToastStack {
    fn default() -> Self {
        Self::new()
    }
}

/// One visible toast, with its texts read for this frame.
struct Shown {
    id: u64,
    kind: ToastKind,
    sticky: bool,
    wide: bool,
    title: Option<SharedString>,
    text: SharedString,
    bullets: Vec<SharedString>,
    actions: Vec<(SharedString, ActionFn)>,
}

impl IntoElement for ToastStack {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        lazy(move |_, cx| {
            let theme = *Theme::get(cx);
            let t = theme.tokens;
            let c = &t.colors;
            let dismiss = ui_text(cx, "hui-toast-dismiss");
            let entries: Vec<Shown> = cx
                .try_global::<ToastQueue>()
                .map(|q| {
                    q.visible
                        .iter()
                        .map(|e| Shown {
                            id: e.id,
                            kind: e.toast.kind,
                            sticky: e.toast.sticky,
                            wide: e.toast.wide,
                            title: e.toast.title.as_ref().map(|f| f(cx)),
                            text: (e.toast.text)(cx),
                            bullets: e.toast.bullets.iter().map(|f| f(cx)).collect(),
                            actions: e
                                .toast
                                .actions
                                .iter()
                                .map(|a| ((a.label)(cx), a.run.clone()))
                                .collect(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            div()
                .debug_selector(|| "toast-stack".to_string())
                .absolute()
                .right(px(EDGE_PX))
                .bottom(px(self.status_bar_px + EDGE_PX))
                .flex()
                .flex_col()
                .gap(px(t.spacing.md))
                .children(entries.into_iter().map(|shown| {
                    let id = shown.id;
                    let (role, live) = if shown.sticky || shown.kind == ToastKind::Error {
                        (Role::Alert, accesskit::Live::Assertive)
                    } else {
                        (Role::Status, accesskit::Live::Polite)
                    };
                    let (icon, tint) = match shown.kind {
                        // Hue Is Meaning: info is no selection, so its icon is text-2.
                        ToastKind::Info => (IconName::Info, c.text_2),
                        ToastKind::Error => (IconName::TriangleAlert, c.red),
                    };
                    let rich = shown.title.is_some()
                        || !shown.bullets.is_empty()
                        || !shown.actions.is_empty();
                    let headline = shown.title.clone().unwrap_or_else(|| shown.text.clone());
                    let mut content = div().flex().flex_col().flex_1().min_w(px(0.0));
                    match &shown.title {
                        Some(title) => {
                            content = content.child(Label::new(
                                title.clone(),
                                TextSource::Fluent,
                                t.type_ramp.body_strong,
                                c.text.to_hsla(),
                            ));
                            if !shown.text.is_empty() {
                                // Raised Ink Rule: secondary lines on the card are text-2.
                                content = content.child(Label::new(
                                    shown.text.clone(),
                                    TextSource::Fluent,
                                    t.type_ramp.body,
                                    c.text_2.to_hsla(),
                                ));
                            }
                        }
                        None => {
                            content = content.child(Label::new(
                                shown.text.clone(),
                                TextSource::Fluent,
                                t.type_ramp.body,
                                c.text.to_hsla(),
                            ));
                        }
                    }
                    if !shown.bullets.is_empty() {
                        content =
                            content.child(div().flex().flex_col().mt(px(t.spacing.xs)).children(
                                shown.bullets.iter().enumerate().map(|(i, bullet)| {
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap(px(t.spacing.sm))
                                        .child(
                                            div()
                                                .debug_selector(move || {
                                                    format!("toast-bullet-{id}-{i}")
                                                })
                                                .size(px(BULLET_PX))
                                                .flex_none()
                                                .bg(c.text_2.to_hsla()),
                                        )
                                        .child(Label::new(
                                            bullet.clone(),
                                            TextSource::Fluent,
                                            t.type_ramp.body,
                                            c.text_2.to_hsla(),
                                        ))
                                }),
                            ));
                    }
                    if !shown.actions.is_empty() {
                        content = content.child(
                            div()
                                .flex()
                                .flex_row()
                                .gap(px(t.spacing.sm))
                                .mt(px(t.spacing.md))
                                .children(shown.actions.into_iter().enumerate().map(
                                    |(i, (label, run))| {
                                        // One emphasised action; the rest are ghosts (mock).
                                        let kind = if i == 0 {
                                            ButtonKind::Default
                                        } else {
                                            ButtonKind::Ghost
                                        };
                                        Button::new(
                                            ElementId::Name(
                                                format!("toast-action-{id}-{i}").into(),
                                            ),
                                            label,
                                        )
                                        .kind(kind)
                                        .on_click(
                                            move |_, window, cx| {
                                                run(window, cx);
                                                ToastQueue::dismiss(cx, id);
                                            },
                                        )
                                    },
                                )),
                        );
                    }
                    let mut card = popover_surface(&theme)
                        .id(ElementId::from(("toast", id)))
                        .debug_selector(move || format!("toast-{id}"))
                        .flex()
                        .flex_row()
                        .gap(px(t.spacing.md))
                        .w(px(if shown.wide { WIDE_WIDTH_PX } else { WIDTH_PX }))
                        .px(px(t.spacing.xl))
                        .py(px(t.spacing.md));
                    // A one-line toast centres its icon; a rich one keeps it on the first line.
                    card = if rich {
                        card.items_start()
                    } else {
                        card.items_center()
                    };
                    let card = card
                        .child(
                            Icon::new(icon)
                                .size(IconSize::Default)
                                .color(tint.to_hsla()),
                        )
                        .child(content)
                        .child(
                            div()
                                .id(ElementId::from(("toast-dismiss", id)))
                                .declare(Role::Button, dismiss.clone(), None)
                                .flex()
                                .items_center()
                                .cursor_pointer()
                                .on_click(move |_, _, cx| ToastQueue::dismiss(cx, id))
                                .child(
                                    // Raised Ink Rule: text-2 on the raised card.
                                    Icon::new(IconName::X)
                                        .size(IconSize::Chrome)
                                        .color(c.text_2.to_hsla()),
                                ),
                        );
                    // Announced by screen readers: the node's value changes with a live region
                    // (macOS speaks only valued, live nodes).
                    let mut node =
                        A11yNode::new(("toast-node", id), role, headline.clone(), false, card)
                            .live(live)
                            .value(headline);
                    if shown.title.is_some() && !shown.text.is_empty() {
                        node = node.description(shown.text);
                    }
                    node
                }))
                .into_any_element()
        })
        .into_any_element()
    }
}
