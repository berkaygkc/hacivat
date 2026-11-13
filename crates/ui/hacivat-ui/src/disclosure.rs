//! Disclosure (NOTES "NEW primitives"): a 32 px row with a 1 px `seam` above; a 12 px `text-2`
//! chevron (right when folded, down when open: swapped, never rotated) in a 14 px column, the
//! title at 13 px weight 600 `text`, and while folded the summary right-aligned at 12.5 px
//! `text-2`, ellipsized. The content is indented 22 px. Space and a click toggle it. Debug
//! selectors: `disclosure-<id>` (the row) and `disclosure-content`.

use std::rc::Rc;

use hacivat_ui_core::gpui_surface::{
    AnyElement, App, ClickEvent, ElementId, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Role, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use hacivat_ui_core::theme::{Theme, focus_outline};

use crate::button::{focus_visible, interaction_state};
use crate::icon::{Icon, IconName, IconSize};
use crate::popover::{Declare, lazy};
use crate::text::{Label, TextSource};

type ToggleHandler = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// The row height.
pub const DISCLOSURE_ROW_PX: f32 = 32.0;
/// How far the content is indented from the row.
pub const DISCLOSURE_INDENT_PX: f32 = 22.0;
/// The chevron column.
const CHEVRON_COLUMN_PX: f32 = 14.0;

/// A folding section (NOTES "NEW primitives").
pub struct Disclosure {
    id: ElementId,
    title: SharedString,
    summary: Option<SharedString>,
    open: bool,
    on_toggle: Option<ToggleHandler>,
    content: Option<AnyElement>,
}

impl Disclosure {
    pub fn new(id: impl Into<ElementId>, title: SharedString) -> Self {
        Disclosure {
            id: id.into(),
            title,
            summary: None,
            open: false,
            on_toggle: None,
            content: None,
        }
    }

    /// The line shown while closed.
    pub fn summary(mut self, summary: SharedString) -> Self {
        self.summary = Some(summary);
        self
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// Called with the new open state.
    pub fn on_toggle(mut self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(handler));
        self
    }

    /// What shows while open.
    pub fn content(mut self, content: AnyElement) -> Self {
        self.content = Some(content);
        self
    }

    fn build(self, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let Disclosure {
            id,
            title,
            summary,
            open,
            on_toggle,
            content,
        } = self;

        let (_, handle) = interaction_state(&id, window, cx);
        let handle = handle.tab_stop(true);
        let focused = focus_visible(&handle, window);
        let chevron = if open {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };
        let selector = format!("disclosure-{id}");
        let mut row = div()
            .id(id)
            .debug_selector(move || selector)
            .track_focus(&handle)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(t.spacing.md))
            .w_full()
            .h(px(DISCLOSURE_ROW_PX))
            .pr(px(t.spacing.s_2xs))
            .border_t_1()
            .border_color(c.seam.to_hsla())
            .cursor_pointer()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .justify_center()
                    .w(px(CHEVRON_COLUMN_PX))
                    .child(
                        Icon::new(chevron)
                            .size(IconSize::Chrome)
                            .color(c.text_2.to_hsla()),
                    ),
            )
            .child(Label::new(
                title.clone(),
                TextSource::Fluent,
                t.type_ramp.body_strong,
                c.text.to_hsla(),
            ));
        if let Some(summary) = summary.filter(|_| !open) {
            row = row.child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_w_0()
                    .justify_end()
                    .child(
                        Label::new(
                            summary,
                            TextSource::Fluent,
                            t.type_ramp.body_sm,
                            c.text_2.to_hsla(),
                        )
                        .truncate(true),
                    ),
            );
        }
        let toggle_on_key = on_toggle.clone();
        let row = focus_outline(row, &theme, focused)
            .declare(Role::Button, title, None)
            .aria_expanded(open)
            .capture_key_down(move |event: &KeyDownEvent, window, cx| {
                let stroke = &event.keystroke;
                if stroke.key != "space" || stroke.modifiers.modified() {
                    return;
                }
                window.prevent_default();
                cx.stop_propagation();
                if let Some(handler) = toggle_on_key.as_ref().filter(|_| !event.is_held) {
                    handler(!open, window, cx);
                }
            })
            .on_click(move |event, window, cx| {
                if matches!(event, ClickEvent::Keyboard(_)) {
                    return;
                }
                if let Some(handler) = &on_toggle {
                    handler(!open, window, cx);
                }
            });

        let mut root = div().flex().flex_col().w_full().child(row);
        if let Some(content) = content.filter(|_| open) {
            root = root.child(
                div()
                    .pl(px(DISCLOSURE_INDENT_PX))
                    .pt(px(t.spacing.xs))
                    .pb(px(t.spacing.xl))
                    .child(
                        div()
                            .debug_selector(|| "disclosure-content".to_string())
                            .child(content),
                    ),
            );
        }
        root.into_any_element()
    }
}

impl IntoElement for Disclosure {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        lazy(move |window, cx| self.build(window, cx)).into_any_element()
    }
}
