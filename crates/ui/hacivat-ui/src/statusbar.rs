//! The status bar (DESIGN.md, Status Bar): 24 px on `ground`, 12 px `text_2`, segments divided
//! by seams. Left segments start at the left edge; right segments end at the right edge.

use hacivat_ui_core::gpui_surface::{
    AnyElement, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, div, px,
};
use hacivat_ui_core::theme::{Theme, TypeStyle};

use crate::capsule::{EnvKind, env_swatch};
use crate::icon::{Icon, IconName, IconSize};
use crate::popover::lazy;
use crate::text::{Label, TextSource};

/// A status-bar text in the 12 px `caption` style at `text_2`.
pub fn status_label(theme: &Theme, text: impl Into<SharedString>, source: TextSource) -> Label {
    let t = theme.tokens;
    Label::new(text, source, t.type_ramp.caption, t.colors.text_2.to_hsla())
}

/// A status-bar text in a given style (mono counters, a bold environment name).
pub fn status_label_in(
    theme: &Theme,
    text: impl Into<SharedString>,
    source: TextSource,
    style: TypeStyle,
) -> Label {
    Label::new(text, source, style, theme.tokens.colors.text_2.to_hsla())
}

/// The environment segment, the first on the left (DESIGN.md Status Bar; NOTES §7 "the status
/// bar mirrors the capsule"): the environment mark and `label` (a Fluent text such as
/// "Production · read-only").
///
/// Production is tinted edge to edge in `red-tint`, with the label in `red` at weight 600 and,
/// when `read_only`, a red lock. Other environments draw the mark and the label in `text` on the
/// plain ground (only production has a hue). Debug selector: `status-env`.
pub fn env_segment(
    theme: &Theme,
    kind: EnvKind,
    label: SharedString,
    read_only: bool,
) -> AnyElement {
    let t = theme.tokens;
    let c = &t.colors;
    let segment = div()
        .debug_selector(|| "status-env".to_string())
        .flex()
        .flex_row()
        .items_center()
        .gap(px(t.spacing.sm))
        .h_full()
        .flex_none()
        .child(env_swatch(kind, theme));
    if kind != EnvKind::Production {
        return segment
            .child(Label::new(
                label,
                TextSource::Fluent,
                t.type_ramp.caption,
                c.text.to_hsla(),
            ))
            .into_any_element();
    }
    // The tint covers the whole cell: the negative margins cancel the bar's cell padding, and
    // the segment keeps the mock's 12 px lead (`.envseg { padding-left: 12px }`).
    let mut segment = segment
        .ml(px(-t.spacing.lg))
        .mr(px(-t.spacing.lg))
        .pl(px(t.spacing.xl))
        .pr(px(t.spacing.lg))
        .bg(c.red_tint.to_hsla());
    if read_only {
        segment = segment.child(
            Icon::new(IconName::Lock)
                .size(IconSize::Chrome)
                .color(c.red.to_hsla()),
        );
    }
    segment
        .child(Label::new(
            label,
            TextSource::Fluent,
            TypeStyle {
                weight: 600,
                ..t.type_ramp.caption
            },
            c.red.to_hsla(),
        ))
        .into_any_element()
}

#[derive(Default)]
pub struct StatusBar {
    left: Vec<AnyElement>,
    right: Vec<AnyElement>,
}

impl StatusBar {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a segment after the previous left one.
    pub fn left(mut self, segment: impl IntoElement) -> Self {
        self.left.push(segment.into_any_element());
        self
    }

    /// Adds a segment after the previous right one (right segments read left to right).
    pub fn right(mut self, segment: impl IntoElement) -> Self {
        self.right.push(segment.into_any_element());
        self
    }
}

impl IntoElement for StatusBar {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        lazy(move |_, cx| {
            let theme = *Theme::get(cx);
            let t = theme.tokens;
            let c = &t.colors;
            let segment = |child: AnyElement, left: bool| {
                let d = div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .h_full()
                    .flex_none()
                    .px(px(t.spacing.lg))
                    .gap(px(t.spacing.sm))
                    .border_color(c.seam.to_hsla())
                    .child(child);
                if left { d.border_r_1() } else { d.border_l_1() }
            };
            div()
                .debug_selector(|| "status-bar".to_string())
                .flex()
                .flex_row()
                .items_center()
                .w_full()
                .h(px(t.spacing.statusbar_height))
                .flex_none()
                .border_t_1()
                .border_color(c.seam.to_hsla())
                .bg(c.ground.to_hsla())
                .text_color(c.text_2.to_hsla())
                .children(self.left.into_iter().map(|s| segment(s, true)))
                .child(div().flex_1())
                .children(self.right.into_iter().map(|s| segment(s, false)))
                .into_any_element()
        })
        .into_any_element()
    }
}
