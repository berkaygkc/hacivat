//! Explanation popover (NOTES "NEW primitives": popover, surface 7): a titled card explaining a
//! capsule badge (the "unencrypted" segment), opened with `Workspace::open_anchored`.
//!
//! The popover surface (raised, `seam-strong`, radius 6, popover shadow), 300 px wide with
//! padding 10/12. The title is 13 px weight 600 `text` after a 12 px icon; each body entry is a
//! paragraph at 12.5 px `text-2` (Raised Ink Rule) that wraps to the card's width. The caller
//! passes every string (Fluent). Debug selectors: `explain-popover`, `explain-title`.

use std::panic::Location;

use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, AppContext as _, Context, InteractiveElement, IntoElement,
    ParentElement, Render, Role, SharedString, Styled, Window, div, px,
};
use hacivat_ui_core::theme::{Theme, TypeStyle, popover_surface};

use crate::icon::{Icon, IconName, IconSize};
use crate::popover::A11yNode;
use crate::text::{Label, RenderedText, TextRegistry, TextSource};

/// The card's width (NOTES: 300 px).
const WIDTH_PX: f32 = 300.0;
/// The gap between the title's icon and its text (mock `.pop b { gap: 7px }`, between the `sm`
/// (6) and `md` (8) tokens).
const TITLE_GAP_PX: f32 = 7.0;

/// An explanation card (NOTES surface 7).
pub struct ExplainPopover {
    title: SharedString,
    icon: IconName,
    body: Vec<SharedString>,
}

impl ExplainPopover {
    /// The popover as a view, ready for a layer.
    pub fn view(
        title: SharedString,
        icon: IconName,
        body: Vec<SharedString>,
        cx: &mut App,
    ) -> AnyView {
        cx.new(|_| ExplainPopover { title, icon, body }).into()
    }
}

/// A paragraph that wraps to its container. `Label` draws one line, so the paragraph uses
/// GPUI's wrapping text and records itself in the [`TextRegistry`] like a label (its source
/// stays checkable under the pseudo-locales).
#[track_caller]
fn paragraph(
    text: SharedString,
    style: TypeStyle,
    color: hacivat_ui_core::gpui_surface::Hsla,
    cx: &mut App,
) -> AnyElement {
    let at = Location::caller();
    TextRegistry::record(cx, || RenderedText {
        text: text.clone(),
        source: TextSource::Fluent,
        size_px: style.size_px,
        painted_px: 0.0,
        clipped: false,
        truncated: false,
        path: format!(
            "{}:{}:{}",
            at.file().replace('\\', "/"),
            at.line(),
            at.column()
        )
        .into(),
    });
    div()
        .w_full()
        .font(font_for(&style))
        .text_size(px(style.size_px))
        .line_height(px(style.line_height))
        .text_color(color)
        .child(text)
        .into_any_element()
}

impl Render for ExplainPopover {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let last = self.body.len().saturating_sub(1);
        let paragraphs: Vec<AnyElement> = self
            .body
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let p = paragraph(line.clone(), t.type_ramp.body_sm, c.text_2.to_hsla(), cx);
                let gap = if i == last { 0.0 } else { t.spacing.sm };
                div().pb(px(gap)).child(p).into_any_element()
            })
            .collect();
        let card = popover_surface(&theme)
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
                    .items_center()
                    .gap(px(TITLE_GAP_PX))
                    .mb(px(t.spacing.xs))
                    .child(
                        Icon::new(self.icon)
                            .size(IconSize::Chrome)
                            .color(c.text.to_hsla()),
                    )
                    .child(Label::new(
                        self.title.clone(),
                        TextSource::Fluent,
                        t.type_ramp.body_strong,
                        c.text.to_hsla(),
                    )),
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
