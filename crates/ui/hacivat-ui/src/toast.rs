//! Toasts: short notices at the bottom right, above the status bar. At most three transient
//! toasts show at once (the rest wait their turn), each lives its TTL (6 s by default) unless
//! dismissed, and none animates (named rule). The text is a [`TextFn`], so a visible toast
//! follows a language switch (F16).
//!
//! A sticky toast (NOTES "NEW primitives": toast, sticky + actions) has no TTL and is announced
//! as an alert. It may carry a title (weight 600), the text as a `text-2` body line, a list of
//! `text-2` bullets with 3 px square marks, and a row of buttons; an action runs, then dismisses
//! the toast. A `wide` toast is 380 px instead of 320. Sticky toasts have their own two slots,
//! so they never keep a transient toast from showing.
//!
//! Keyboard: the actions and the dismiss ✕ are buttons and tab stops; [`focus_newest_sticky`]
//! moves the focus to the newest sticky toast (the host binds a key to it), Escape dismisses a
//! focused toast, and dismissing it gives the focus back to where it was.
//!
//! [`ToastQueue`] is the model (a `Global`); [`ToastStack`] draws it and is placed by the host
//! (Task 13's `Layers`). Debug selectors: `toast-<id>`, `toast-bullet-<id>-<i>`, the action
//! buttons `button-toast-action-<id>-<i>` and the ✕ `button-toast-dismiss-<id>`.

use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use hacivat_ui_core::gpui_surface::{
    AnyElement, App, ElementId, FocusHandle, Global, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Role, SharedString, Styled, Window, accesskit, div, px,
};
use hacivat_ui_core::theme::{Theme, popover_surface};

use crate::actions::ActionFn;
use crate::button::{Button, ButtonKind};
use crate::icon::{Icon, IconName, IconSize};
use crate::l10n::ui_text;
use crate::popover::{A11yNode, lazy};
use crate::text::{Label, TextFn, TextSource};

/// How many transient toasts show at once.
pub const MAX_VISIBLE_TOASTS: usize = 3;
/// How many sticky toasts show at once, besides the transient ones.
pub const MAX_STICKY_TOASTS: usize = 2;
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
    // ## m2a-t17-dialog
    /// Machine text shown verbatim under the title in mono (`TextSource::Data`), e.g. the URL
    /// "Copy as URL" put on the clipboard (NOTES surface 2).
    pub data: Option<SharedString>,
    // ## m2b-t22-live
    /// The glyph in place of the kind's (the session toasts: rotate-cw, unplug); it keeps the
    /// kind's ink.
    pub icon: Option<IconName>,
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
            data: None,
            icon: None,
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

/// Moves the focus to the newest visible sticky toast: its first action, or its ✕. Returns
/// whether a sticky toast took the focus. Dismissing that toast gives the focus back. The host
/// binds a key to this (GPUI key bindings belong to `menus::rebuild`).
pub fn focus_newest_sticky(window: &mut Window, cx: &mut App) -> bool {
    let previous = window.focused(cx);
    let target = {
        let Some(q) = cx.try_global::<ToastQueue>() else {
            return false;
        };
        let Some(entry) = q
            .visible
            .iter()
            .filter(|e| e.toast.sticky)
            .max_by_key(|e| e.id)
        else {
            return false;
        };
        let handle = entry
            .actions_focus
            .first()
            .unwrap_or(&entry.dismiss_focus)
            .clone();
        (entry.id, handle)
    };
    let (id, handle) = target;
    if let Some(entry) = cx
        .global_mut::<ToastQueue>()
        .visible
        .iter_mut()
        .find(|e| e.id == id)
        && entry.restore.is_none()
    {
        entry.restore = previous;
    }
    window.focus(&handle, cx);
    true
}

struct Entry {
    id: u64,
    toast: Toast,
    /// One focus handle per action, then the ✕'s.
    actions_focus: Vec<FocusHandle>,
    dismiss_focus: FocusHandle,
    /// Where the focus was before [`focus_newest_sticky`] moved it here.
    restore: Option<FocusHandle>,
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
    /// Shows `toast` now, or after a visible toast of its kind (sticky or transient) goes.
    pub fn push(cx: &mut App, toast: Toast) {
        Self::push_id(cx, toast);
    }

    // ## m2b-t22-live
    /// [`ToastQueue::push`], returning the toast's id (for [`ToastQueue::dismiss`]: a toast
    /// that replaces an older one of the same subject).
    pub fn push_id(cx: &mut App, toast: Toast) -> u64 {
        let entry = Entry {
            id: 0,
            actions_focus: toast.actions.iter().map(|_| cx.focus_handle()).collect(),
            dismiss_focus: cx.focus_handle(),
            restore: None,
            toast,
        };
        let (id, started) = {
            let q = cx.default_global::<ToastQueue>();
            q.next_id += 1;
            let entry = Entry {
                id: q.next_id,
                ..entry
            };
            let id = entry.id;
            if q.has_room(entry.toast.sticky) {
                let started = (entry.id, entry.toast.lifetime());
                q.visible.push(entry);
                (id, Some(started))
            } else {
                q.waiting.push_back(entry);
                (id, None)
            }
        };
        if let Some((id, Some(ttl))) = started {
            Self::start_timer(cx, id, ttl);
        }
        cx.refresh_windows();
        id
    }

    /// Whether a toast of this kind can show now.
    fn has_room(&self, sticky: bool) -> bool {
        let max = if sticky {
            MAX_STICKY_TOASTS
        } else {
            MAX_VISIBLE_TOASTS
        };
        self.visible
            .iter()
            .filter(|e| e.toast.sticky == sticky)
            .count()
            < max
    }

    /// Removes a toast (its TTL ran out, the user dismissed it or one of its actions ran) and
    /// promotes the waiting toasts that now have room.
    pub fn dismiss(cx: &mut App, id: u64) {
        let promoted = {
            let q = cx.default_global::<ToastQueue>();
            let before = q.visible.len() + q.waiting.len();
            q.visible.retain(|e| e.id != id);
            q.waiting.retain(|e| e.id != id);
            if q.visible.len() + q.waiting.len() == before {
                return;
            }
            let mut promoted = Vec::new();
            while let Some(at) = q.waiting.iter().position(|e| q.has_room(e.toast.sticky)) {
                if let Some(entry) = q.waiting.remove(at) {
                    promoted.push((entry.id, entry.toast.lifetime()));
                    q.visible.push(entry);
                }
            }
            promoted
        };
        for (id, ttl) in promoted {
            if let Some(ttl) = ttl {
                Self::start_timer(cx, id, ttl);
            }
        }
        cx.refresh_windows();
    }

    /// Dismisses `id` from a key or a click and gives the focus back to where it was before
    /// [`focus_newest_sticky`] moved it.
    fn dismiss_in(window: &mut Window, cx: &mut App, id: u64) {
        let restore = cx.try_global::<ToastQueue>().and_then(|q| {
            q.visible
                .iter()
                .find(|e| e.id == id)
                .and_then(|e| e.restore.clone())
        });
        Self::dismiss(cx, id);
        if let Some(handle) = restore {
            window.focus(&handle, cx);
        }
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
    inert: bool,
}

impl ToastStack {
    pub fn new() -> Self {
        Self {
            status_bar_px: 0.0,
            inert: false,
        }
    }

    /// The height of the status bar the stack clears.
    pub fn above(mut self, status_bar_px: f32) -> Self {
        self.status_bar_px = status_bar_px;
        self
    }

    /// While a modal layer is open the toasts stay readable but their buttons do nothing and
    /// are announced as disabled (R26); the host passes `Layers::has_modal()`.
    pub fn inert(mut self, inert: bool) -> Self {
        self.inert = inert;
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
    data: Option<SharedString>,
    text: SharedString,
    bullets: Vec<SharedString>,
    actions: Vec<(SharedString, ActionFn, FocusHandle)>,
    dismiss_focus: FocusHandle,
    icon: Option<IconName>,
}

impl IntoElement for ToastStack {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        lazy(move |_, cx| {
            let theme = *Theme::get(cx);
            let t = theme.tokens;
            let c = &t.colors;
            let inert = self.inert;
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
                            data: e.toast.data.clone(),
                            text: (e.toast.text)(cx),
                            bullets: e.toast.bullets.iter().map(|f| f(cx)).collect(),
                            actions: e
                                .toast
                                .actions
                                .iter()
                                .zip(&e.actions_focus)
                                .map(|(a, h)| ((a.label)(cx), a.run.clone(), h.clone()))
                                .collect(),
                            dismiss_focus: e.dismiss_focus.clone(),
                            icon: e.toast.icon,
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
                    // ## m2b-t22-live: a toast's own glyph keeps the kind's ink.
                    let icon = shown.icon.unwrap_or(icon);
                    let rich = shown.title.is_some()
                        || shown.data.is_some()
                        || !shown.bullets.is_empty()
                        || !shown.actions.is_empty();
                    let headline = shown.title.clone().unwrap_or_else(|| shown.text.clone());
                    // Everything the card says, one line each, for the live announcement.
                    let mut spoken: Vec<SharedString> = vec![headline.clone()];
                    spoken.extend(shown.data.clone());
                    if shown.title.is_some() && !shown.text.is_empty() {
                        spoken.push(shown.text.clone());
                    }
                    spoken.extend(shown.bullets.iter().cloned());
                    let announcement: SharedString = spoken.join("\n").into();
                    let mut content = div().flex().flex_col().flex_1().min_w(px(0.0));
                    // ## m2a-t21-shots: the title and the text wrap to the card; the data
                    // line (a URL) is one line that ellipsizes, its tooltip the whole value.
                    match &shown.title {
                        Some(title) => {
                            content = content.child(crate::text::paragraph(
                                title.clone(),
                                TextSource::Fluent,
                                t.type_ramp.body_strong,
                                c.text.to_hsla(),
                            ));
                            if let Some(data) = &shown.data {
                                content = content.child(
                                    Label::new(
                                        data.clone(),
                                        TextSource::Data,
                                        t.type_ramp.mono_data,
                                        c.text.to_hsla(),
                                    )
                                    .truncate(true),
                                );
                            }
                            if !shown.text.is_empty() {
                                // Raised Ink Rule: secondary lines on the card are text-2.
                                content = content.child(crate::text::paragraph(
                                    shown.text.clone(),
                                    TextSource::Fluent,
                                    t.type_ramp.body,
                                    c.text_2.to_hsla(),
                                ));
                            }
                        }
                        None => {
                            content = content.child(crate::text::paragraph(
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
                                    // ## m2a-t21-shots: a bullet wraps; its square sits on
                                    // the first line.
                                    let line = t.type_ramp.body.line_height;
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_start()
                                        .gap(px(t.spacing.sm))
                                        .child(
                                            div()
                                                .debug_selector(move || {
                                                    format!("toast-bullet-{id}-{i}")
                                                })
                                                .mt(px((line - BULLET_PX) / 2.0))
                                                .size(px(BULLET_PX))
                                                .flex_none()
                                                .bg(c.text_2.to_hsla()),
                                        )
                                        .child(div().flex_1().min_w(px(0.0)).child(
                                            crate::text::paragraph(
                                                bullet.clone(),
                                                TextSource::Fluent,
                                                t.type_ramp.body,
                                                c.text_2.to_hsla(),
                                            ),
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
                                    |(i, (label, run, focus))| {
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
                                        .focus_handle(focus)
                                        .inert(inert)
                                        .on_click(
                                            move |_, window, cx| {
                                                run(window, cx);
                                                ToastQueue::dismiss_in(window, cx, id);
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
                    if !inert {
                        // Escape dismisses the toast that holds the focus.
                        card = card.on_key_down(move |event: &KeyDownEvent, window, cx| {
                            let stroke = &event.keystroke;
                            if stroke.key == "escape" && !stroke.modifiers.modified() {
                                cx.stop_propagation();
                                ToastQueue::dismiss_in(window, cx, id);
                            }
                        });
                    }
                    let card = card
                        .child(
                            Icon::new(icon)
                                .size(IconSize::Default)
                                .color(tint.to_hsla()),
                        )
                        .child(content)
                        .child(
                            // Raised Ink Rule: the icon button's ink is text-2 on the card.
                            Button::new(
                                ElementId::Name(format!("toast-dismiss-{id}").into()),
                                dismiss.clone(),
                            )
                            .kind(ButtonKind::Icon)
                            .icon(IconName::X)
                            .focus_handle(shown.dismiss_focus)
                            .inert(inert)
                            .on_click(move |_, window, cx| ToastQueue::dismiss_in(window, cx, id)),
                        );
                    // Announced by screen readers: the node's value changes with a live region
                    // (macOS speaks only valued, live nodes).
                    let mut node =
                        A11yNode::new(("toast-node", id), role, headline.clone(), false, card)
                            .live(live)
                            .value(announcement);
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
