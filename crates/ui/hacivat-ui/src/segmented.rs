//! Segmented controls (DESIGN.md Components, Segmented Controls): a 26 px group with a
//! `seam_strong` outline and dividers; the pressed segment sits on `raised` (`panel` in the
//! light theme) with text ink, the others in `text_2` (hover lifts them to `text`). Every segment
//! has the debug selector `segmented-<id>-<index>` for tests.

use hacivat_ui_core::gpui_surface::{
    AnyElement, ElementId, InteractiveElement, IntoElement, ParentElement, Role, SharedString,
    StatefulInteractiveElement, Styled, accesskit, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode, focus_outline};

use crate::button::{ClickHandler, focus_visible, hover_listener, interaction_state};
use crate::icon::{Icon, IconName, IconSize};
use crate::popover::Declare;
use crate::popover::{Tip, lazy};
use crate::text::{Label, TextSource};

pub enum SegmentContent {
    /// A rendered Fluent message ([`TextSource::Fluent`]): bracketed under pseudo-locales.
    Label(SharedString),
    /// Text shown as is, never translated: a language's short name (`EN`, `TR`). Marked
    /// [`TextSource::Data`], so pseudo-locales leave it alone and the registry check accepts it.
    Verbatim(SharedString),
    Icon(IconName),
}

pub struct SegmentItem {
    pub content: SegmentContent,
    pub pressed: bool,
    /// The tooltip, and the accessible name of an icon segment.
    pub tooltip: SharedString,
    pub on_click: ClickHandler,
}

pub struct Segmented {
    id: ElementId,
    items: Vec<SegmentItem>,
    mono: bool,
}

impl Segmented {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            items: Vec::new(),
            mono: false,
        }
    }

    pub fn item(mut self, item: SegmentItem) -> Self {
        self.items.push(item);
        self
    }

    /// Labels in 11.5 px mono (the language segments).
    pub fn mono(mut self, mono: bool) -> Self {
        self.mono = mono;
        self
    }
}

impl IntoElement for Segmented {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        lazy(move |window, cx| {
            let theme = *Theme::get(cx);
            let t = theme.tokens;
            let c = &t.colors;
            let pressed_fill = match theme.mode {
                ThemeMode::Dark => c.raised,
                ThemeMode::Light => c.panel,
            };
            let selector = format!("segmented-{}", self.id);
            let mut group = div()
                .debug_selector(move || selector)
                .flex()
                .flex_row()
                .items_center()
                .flex_none()
                .h(px(t.spacing.field_height))
                .border_1()
                .border_color(c.seam_strong.to_hsla())
                .rounded(px(t.radii.sm))
                .overflow_hidden();
            for (i, item) in self.items.into_iter().enumerate() {
                let id = ElementId::from((self.id.clone(), format!("{i}")));
                let (hover, handle) = interaction_state(&id, window, cx);
                let focused = focus_visible(&handle, window);
                let hovered = *hover.read(cx);
                // Harman: hover lifts an unpressed segment's ink to `text`; the fill is only for
                // the pressed segment.
                let ink = if item.pressed || hovered {
                    c.text
                } else {
                    c.text_2
                }
                .to_hsla();
                let is_icon = matches!(item.content, SegmentContent::Icon(_));
                let name = match &item.content {
                    SegmentContent::Label(label) | SegmentContent::Verbatim(label) => label.clone(),
                    SegmentContent::Icon(_) => item.tooltip.clone(),
                };
                let source = match item.content {
                    SegmentContent::Verbatim(_) => TextSource::Data,
                    _ => TextSource::Fluent,
                };
                let content: AnyElement = match item.content {
                    SegmentContent::Label(text) | SegmentContent::Verbatim(text) => {
                        let mut style = if self.mono {
                            t.type_ramp.mono_data
                        } else {
                            t.type_ramp.body
                        };
                        if item.pressed && self.mono {
                            style.weight = 600;
                        }
                        Label::new(text, source, style, ink).into_any_element()
                    }
                    SegmentContent::Icon(name) => Icon::new(name)
                        .size(IconSize::Chrome)
                        .color(ink)
                        .into_any_element(),
                };
                let selector = format!("segmented-{}-{i}", self.id);
                let mut segment = div()
                    .id(id)
                    .debug_selector(move || selector)
                    .declare(Role::Button, name, None)
                    .aria_toggled(accesskit::Toggled::from(item.pressed))
                    .flex()
                    .flex_row()
                    .items_center()
                    .h_full()
                    .px(px(match (is_icon, self.mono) {
                        (true, _) => 7.0,
                        (false, true) => 8.0,
                        (false, false) => 9.0,
                    }))
                    .gap(px(t.spacing.sm))
                    .when_divider(i)
                    .border_color(c.seam_strong.to_hsla())
                    .cursor_pointer()
                    .child(content);
                if item.pressed {
                    segment = segment.bg(pressed_fill.to_hsla());
                }
                let on_click = item.on_click;
                let tooltip = item.tooltip;
                let segment =
                    focus_outline(segment.track_focus(&handle).tab_stop(true), &theme, focused)
                        .on_hover(hover_listener(hover))
                        .on_click(move |event, window, cx| on_click(event, window, cx))
                        .tooltip(move |_, cx| Tip::view(tooltip.clone(), cx));
                group = group.child(segment);
            }
            group.into_any_element()
        })
        .into_any_element()
    }
}

/// Dividers: every segment but the first draws a left seam.
trait Divider: Styled + Sized {
    fn when_divider(self, index: usize) -> Self {
        if index > 0 { self.border_l_1() } else { self }
    }
}

impl<T: Styled + Sized> Divider for T {}
