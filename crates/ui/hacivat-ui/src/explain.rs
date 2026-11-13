//! Explanation popover (NOTES "NEW primitives": popover, surface 7): a titled card explaining a
//! capsule badge (the "unencrypted" segment), opened with `Workspace::open_anchored`.
//!
//! The popover surface (raised, `seam-strong`, radius 6, popover shadow), 300 px wide with
//! padding 10/12. The title is 13 px weight 600 `text` after a 12 px icon; each body entry is a
//! paragraph at 12.5 px `text-2` (Raised Ink Rule) that wraps to the card's width. The caller
//! passes every string (Fluent). Debug selectors: `explain-popover`, `explain-title`.
//!
//! Focus: the popover takes the focus on its first frame and remembers where it was (the
//! trigger). Escape hands the focus back and emits [`ExplainEvent::Dismissed`]; it does not stop
//! the key, so the workspace's Escape pops the layer (whose restore focus is the same trigger,
//! recorded by `open_anchored`). The workspace is the only close path inside the layer stack:
//! hosts there must not also pop on `Dismissed` (see [`ExplainEvent`]).

use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, Role,
    SharedString, Styled, Window, div, px,
};
use hacivat_ui_core::theme::{Theme, popover_surface};

use crate::icon::{Icon, IconName, IconSize};
use crate::popover::A11yNode;
use crate::text::{TextSource, paragraph};

/// The card's width (NOTES: 300 px).
const WIDTH_PX: f32 = 300.0;
/// The gap between the title's icon and its text (mock `.pop b { gap: 7px }`, between the `sm`
/// (6) and `md` (8) tokens).
const TITLE_GAP_PX: f32 = 7.0;

/// What the popover tells its host.
///
/// Contract: `Dismissed` is a notification, not a close request. The popover does not stop the
/// Escape key, so it keeps propagating to the workspace, whose Escape handler pops the top layer
/// (the `open_anchored` host) and restores that layer's focus. A host inside the workspace's
/// layer stack must therefore **not** also pop the layer on `Dismissed`: it would close a second
/// layer. Only a host outside the layer stack (a test, or a parent that draws the popover
/// itself) closes it on `Dismissed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplainEvent {
    /// Escape was pressed and the focus is back on the trigger. See the enum's contract: the
    /// workspace closes the layer on the same key.
    Dismissed,
}

/// An explanation card (NOTES surface 7).
pub struct ExplainPopover {
    title: SharedString,
    icon: IconName,
    body: Vec<SharedString>,
    focus: FocusHandle,
    /// Whether the popover has taken the focus yet (on its first frame).
    opened: bool,
    /// The focus before the popover took it: the trigger.
    restore: Option<FocusHandle>,
}

impl EventEmitter<ExplainEvent> for ExplainPopover {}

impl Focusable for ExplainPopover {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ExplainPopover {
    /// The popover as an entity, for a host that listens to [`ExplainEvent`].
    pub fn new(
        title: SharedString,
        icon: IconName,
        body: Vec<SharedString>,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| ExplainPopover {
            title,
            icon,
            body,
            focus: cx.focus_handle(),
            opened: false,
            restore: None,
        })
    }

    /// The popover as a view, ready for a layer.
    pub fn view(
        title: SharedString,
        icon: IconName,
        body: Vec<SharedString>,
        cx: &mut App,
    ) -> AnyView {
        Self::new(title, icon, body, cx).into()
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        if stroke.key != "escape" || stroke.modifiers.modified() {
            return;
        }
        if let Some(handle) = self.restore.take() {
            window.focus(&handle, cx);
        }
        cx.emit(ExplainEvent::Dismissed);
    }
}

impl Render for ExplainPopover {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.opened {
            // Take the focus once the first frame is drawn, remembering the trigger.
            self.opened = true;
            let this = cx.entity().downgrade();
            window.defer(cx, move |window, cx| {
                let Some(this) = this.upgrade() else {
                    return;
                };
                let previous = window.focused(cx);
                let focus = this.update(cx, |this, _| {
                    this.restore = previous;
                    this.focus.clone()
                });
                window.focus(&focus, cx);
            });
        }
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let last = self.body.len().saturating_sub(1);
        let paragraphs: Vec<AnyElement> = self
            .body
            .iter()
            .enumerate()
            .map(|(i, line)| {
                // ## m2a-t21-shots: recorded at paint, after the title, with its real fit.
                let p = paragraph(
                    line.clone(),
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    c.text_2.to_hsla(),
                );
                let gap = if i == last { 0.0 } else { t.spacing.sm };
                div().pb(px(gap)).child(p).into_any_element()
            })
            .collect();
        let card = popover_surface(&theme)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .debug_selector(|| "explain-popover".to_string())
            .flex()
            .flex_col()
            .w(px(WIDTH_PX))
            .px(px(t.spacing.xl))
            .py(px(t.spacing.lg))
            .child(
                div()
                    .debug_selector(|| "explain-title".to_string())
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(px(TITLE_GAP_PX))
                    .mb(px(t.spacing.xs))
                    .child(
                        // ## m2a-t21-shots: the icon sits on the first line of a title
                        // that wraps.
                        div()
                            .flex_none()
                            .mt(px((t.type_ramp.body_strong.line_height
                                - IconSize::Chrome.px())
                                / 2.0))
                            .child(
                                Icon::new(self.icon)
                                    .size(IconSize::Chrome)
                                    .color(c.text.to_hsla()),
                            ),
                    )
                    .child(div().flex_1().min_w(px(0.0)).child(paragraph(
                        self.title.clone(),
                        TextSource::Fluent,
                        t.type_ramp.body_strong,
                        c.text.to_hsla(),
                    ))),
            )
            .children(paragraphs);
        let description: SharedString = self.body.join(" ").into();
        A11yNode::new(
            "explain-node",
            Role::Dialog,
            self.title.clone(),
            false,
            card,
        )
        .description(description)
    }
}
