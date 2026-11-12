//! Segmented controls (DESIGN.md Components, Segmented Controls): a 26 px group with a
//! `seam_strong` outline and dividers; the pressed segment sits on `raised` (`panel` in the
//! light theme) with text ink, the others in `text_2` (hover lifts them to `text`). Every segment
//! has the debug selector `segmented-<id>-<index>` for tests.
//!
//! Keyboard (M2a, roving tab stop): the group is one tab stop, the pressed segment (the first when
//! none is); ← and → move the focus to the neighbouring segment and press it, wrapping at the
//! ends. Enter and Space press the focused segment.

use std::rc::Rc;

use hacivat_ui_core::gpui_surface::{
    AnyElement, ClickEvent, ElementId, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Role, SharedString, StatefulInteractiveElement, Styled, accesskit, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode, focus_outline};

use crate::button::{ClickHandler, focus_visible, hover_listener, interaction_state};
use crate::capsule::{EnvKind, env_swatch};
use crate::icon::{Icon, IconName, IconSize};
use crate::popover::Declare;
use crate::popover::{A11yNode, Tip, lazy};
use crate::text::{Label, TextSource};

pub enum SegmentContent {
    /// A rendered Fluent message ([`TextSource::Fluent`]): bracketed under pseudo-locales.
    Label(SharedString),
    /// Text shown as is, never translated: a language's short name (`EN`, `TR`). Marked
    /// [`TextSource::Data`], so pseudo-locales leave it alone and the registry check accepts it.
    Verbatim(SharedString),
    Icon(IconName),
    // ## m2a-t17-dialog
    /// A rendered Fluent message after the environment's 7 px mark (NOTES cross-cutting #2).
    Marked(EnvKind, SharedString),
}

pub struct SegmentItem {
    pub content: SegmentContent,
    pub pressed: bool,
    /// The tooltip, and the accessible name of an icon segment.
    pub tooltip: SharedString,
    /// The accessible name when it differs from the label (a language's short name `TR` is
    /// announced by its native name). `None`: the label, or the tooltip for an icon segment.
    pub a11y_name: Option<SharedString>,
    pub on_click: ClickHandler,
}

pub struct Segmented {
    id: ElementId,
    items: Vec<SegmentItem>,
    mono: bool,
    inert: bool,
    in_well: bool,
}

impl Segmented {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            items: Vec::new(),
            mono: false,
            inert: false,
            in_well: false,
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

    /// Ignores the pointer and the keyboard and announces every segment as disabled, with the
    /// same look (the title bar while a modal layer is open, M2a).
    pub fn inert(mut self, inert: bool) -> Self {
        self.inert = inert;
        self
    }

    // ## m2a-t17-dialog
    /// Inside a dialog (NOTES cross-cutting #5): the group sits in a field well (`panel` in dark,
    /// `ground` in light) and the pressed label is weight 600.
    pub fn in_well(mut self, in_well: bool) -> Self {
        self.in_well = in_well;
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
            if self.in_well {
                let well = match theme.mode {
                    ThemeMode::Dark => c.panel,
                    ThemeMode::Light => c.ground,
                };
                group = group.bg(well.to_hsla());
            }
            let count = self.items.len();
            let ids: Vec<ElementId> = (0..count)
                .map(|i| ElementId::from((self.id.clone(), format!("{i}"))))
                .collect();
            let states: Vec<_> = ids
                .iter()
                .map(|id| interaction_state(id, window, cx))
                .collect();
            // One tab stop per group: the pressed segment, or the first.
            let stop = self.items.iter().position(|i| i.pressed).unwrap_or(0);
            let handles: Rc<Vec<FocusHandle>> = Rc::new(
                states
                    .iter()
                    .enumerate()
                    .map(|(i, (_, handle))| handle.clone().tab_stop(i == stop))
                    .collect(),
            );
            let clicks: Rc<Vec<ClickHandler>> =
                Rc::new(self.items.iter().map(|i| i.on_click.clone()).collect());
            for ((i, item), (id, (hover, handle))) in self
                .items
                .into_iter()
                .enumerate()
                .zip(ids.into_iter().zip(states))
            {
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
                let name = item
                    .a11y_name
                    .clone()
                    .unwrap_or_else(|| match &item.content {
                        SegmentContent::Label(label)
                        | SegmentContent::Verbatim(label)
                        | SegmentContent::Marked(_, label) => label.clone(),
                        SegmentContent::Icon(_) => item.tooltip.clone(),
                    });
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
                        if item.pressed && (self.mono || self.in_well) {
                            style.weight = 600;
                        }
                        Label::new(text, source, style, ink).into_any_element()
                    }
                    SegmentContent::Marked(kind, text) => {
                        let mut style = t.type_ramp.body;
                        if item.pressed && self.in_well {
                            style.weight = 600;
                        }
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(t.spacing.sm))
                            .child(env_swatch(kind, &theme))
                            .child(Label::new(text, source, style, ink))
                            .into_any_element()
                    }
                    SegmentContent::Icon(name) => Icon::new(name)
                        .size(IconSize::Chrome)
                        .color(ink)
                        .into_any_element(),
                };
                let selector = format!("segmented-{}-{i}", self.id);
                let node_id = ElementId::from((self.id.clone(), format!("{i}-node")));
                let mut segment = div().id(id).debug_selector(move || selector);
                if !self.inert {
                    segment = segment
                        .declare(Role::Button, name.clone(), None)
                        .aria_toggled(accesskit::Toggled::from(item.pressed));
                }
                let mut segment = segment
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
                    .child(content);
                if item.pressed {
                    segment = segment.bg(pressed_fill.to_hsla());
                }
                if self.inert {
                    group = group.child(
                        A11yNode::new(node_id, Role::Button, name, false, segment)
                            .toggled(item.pressed)
                            .disabled(true),
                    );
                    continue;
                }
                let segment = segment.cursor_pointer();
                let on_click = item.on_click;
                let tooltip = item.tooltip;
                let (handles, clicks) = (handles.clone(), clicks.clone());
                let segment = focus_outline(segment.track_focus(&handles[i]), &theme, focused)
                    .on_hover(hover_listener(hover))
                    .on_click(move |event, window, cx| on_click(event, window, cx))
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        let stroke = &event.keystroke;
                        if stroke.modifiers.modified() {
                            return;
                        }
                        let next = match stroke.key.as_str() {
                            "right" => (i + 1) % count,
                            "left" => (i + count - 1) % count,
                            _ => return,
                        };
                        cx.stop_propagation();
                        window.focus(&handles[next], cx);
                        clicks[next](&ClickEvent::default(), window, cx);
                    })
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
