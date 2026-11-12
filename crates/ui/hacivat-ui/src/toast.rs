//! Toasts: short notices at the bottom right, above the status bar. At most three show at once
//! (the rest wait their turn), each lives its TTL (6 s by default) unless dismissed, and none
//! animates (named rule). The text is a [`TextFn`], so a visible toast follows a language
//! switch (F16).
//!
//! [`ToastQueue`] is the model (a `Global`); [`ToastStack`] draws it and is placed by the host
//! (Task 13's `Layers`).

use std::collections::VecDeque;
use std::time::Duration;

use hacivat_ui_core::gpui_surface::{
    AnyElement, App, ElementId, Global, InteractiveElement, IntoElement, ParentElement, Role,
    SharedString, StatefulInteractiveElement, Styled, accesskit, div, px,
};
use hacivat_ui_core::theme::{Theme, popover_surface};

use crate::actions::ActionFn;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    /// Announced politely (`Role::Status`).
    Info,
    /// Announced assertively (`Role::Alert`).
    Error,
}

#[derive(Clone)]
pub struct Toast {
    pub text: TextFn,
    pub kind: ToastKind,
    pub ttl: Duration,
    // M2a (Task 1; drawn from Task 9 on).
    /// Stays until dismissed or an action runs (no TTL).
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
                let started = (entry.id, entry.toast.ttl);
                q.visible.push(entry);
                Some(started)
            } else {
                q.waiting.push_back(entry);
                None
            }
        };
        if let Some((id, ttl)) = started {
            Self::start_timer(cx, id, ttl);
        }
        cx.refresh_windows();
    }

    /// Removes a toast (its TTL ran out or the user dismissed it) and promotes a waiting one.
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
                let started = (entry.id, entry.toast.ttl);
                q.visible.push(entry);
                Some(started)
            } else {
                None
            }
        };
        if let Some((id, ttl)) = promoted {
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

impl IntoElement for ToastStack {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        lazy(move |_, cx| {
            let theme = *Theme::get(cx);
            let t = theme.tokens;
            let c = &t.colors;
            let dismiss = ui_text(cx, "hui-toast-dismiss");
            let entries: Vec<(u64, ToastKind, SharedString)> = cx
                .try_global::<ToastQueue>()
                .map(|q| {
                    q.visible
                        .iter()
                        .map(|e| (e.id, e.toast.kind, (e.toast.text)(cx)))
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
                .children(entries.into_iter().map(|(id, kind, text)| {
                    let (role, live, icon, tint) = match kind {
                        ToastKind::Info => (
                            Role::Status,
                            accesskit::Live::Polite,
                            IconName::Info,
                            // Hue Is Meaning: info is no selection, so its icon is text-2.
                            c.text_2,
                        ),
                        ToastKind::Error => (
                            Role::Alert,
                            accesskit::Live::Assertive,
                            IconName::TriangleAlert,
                            c.red,
                        ),
                    };
                    let card = popover_surface(&theme)
                        .id(ElementId::from(("toast", id)))
                        .debug_selector(move || format!("toast-{id}"))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(t.spacing.md))
                        .w(px(320.0))
                        .px(px(t.spacing.xl))
                        .py(px(t.spacing.md))
                        .child(
                            Icon::new(icon)
                                .size(IconSize::Default)
                                .color(tint.to_hsla()),
                        )
                        .child(div().flex_1().min_w(px(0.0)).child(Label::new(
                            text.clone(),
                            TextSource::Fluent,
                            t.type_ramp.body,
                            c.text.to_hsla(),
                        )))
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
                    A11yNode::new(("toast-node", id), role, text.clone(), false, card)
                        .live(live)
                        .value(text)
                }))
                .into_any_element()
        })
        .into_any_element()
    }
}
