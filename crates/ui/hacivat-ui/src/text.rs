//! Text sources, stored label closures and the `Label` element.
//!
//! `Label` is the working minimal version (F4): a `div` with the text in its type style and
//! colour. Task 11 replaces the body with the custom element and the text registry, keeping
//! this constructor and builder API.

use std::rc::Rc;

use hacivat_ui_core::fonts::{font_for, small_caps};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, Div, ElementId, Hsla, InteractiveElement, IntoElement, ParentElement,
    SharedString, Styled, div, px,
};
use hacivat_ui_core::theme::{Caps, TypeStyle};

/// Where a visible string comes from (INV-1, I18N-23).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextSource {
    /// A rendered Fluent message.
    Fluent,
    /// Database catalog names (schemas, tables, columns).
    Catalog,
    /// User or query data.
    Data,
    /// A generated shortcut label (`⇧⌘P`, `Ctrl+Shift+7`).
    Shortcut,
}

/// A label re-read on every render, so stateful overlays (dialog, popup menu, toast) follow a
/// language switch (F16).
pub type TextFn = Rc<dyn Fn(&App) -> SharedString>;

/// An access key re-read on every render (F16).
pub type KeyFn = Rc<dyn Fn(&App) -> Option<char>>;

/// One line of text in a type style.
#[derive(Debug, Clone)]
pub struct Label {
    text: SharedString,
    source: TextSource,
    style: TypeStyle,
    color: Hsla,
    truncate: bool,
    id: Option<ElementId>,
}

impl Label {
    pub fn new(
        text: impl Into<SharedString>,
        source: TextSource,
        style: TypeStyle,
        color: Hsla,
    ) -> Self {
        Self {
            text: text.into(),
            source,
            style,
            color,
            truncate: false,
            id: None,
        }
    }

    /// Ellipsize with `…` instead of overflowing.
    pub fn truncate(mut self, truncate: bool) -> Self {
        self.truncate = truncate;
        self
    }

    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn text(&self) -> &SharedString {
        &self.text
    }

    pub fn source(&self) -> TextSource {
        self.source
    }

    fn body(&self) -> Div {
        let s = &self.style;
        let el = if s.caps == Caps::SynthesizedSmallCaps {
            small_caps(s, self.color, &self.text)
        } else {
            div()
                .font(font_for(s))
                .text_size(px(s.size_px))
                .line_height(px(s.line_height))
                .text_color(self.color)
                .child(self.text.clone())
        };
        if self.truncate {
            el.min_w(px(0.0)).truncate()
        } else {
            el.whitespace_nowrap()
        }
    }
}

impl IntoElement for Label {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        let body = self.body();
        match self.id {
            Some(id) => body.id(id).into_any_element(),
            None => body.into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, point, size};
    use hacivat_ui_core::theme::DARK;

    #[gpui::test]
    fn label_lays_out_one_line_of_its_style(cx: &mut TestAppContext) {
        let vcx = cx.add_empty_window();
        let style = DARK.type_ramp.body;
        let color = DARK.colors.text.to_hsla();
        vcx.draw(
            point(px(0.0), px(0.0)),
            size(px(400.0), px(100.0)),
            |_, _| {
                div()
                    .flex()
                    .debug_selector(|| "label".into())
                    .child(Label::new("Save", TextSource::Fluent, style, color).id("save"))
            },
        );
        // One line of the style's line height (layout snaps to the device pixel grid).
        let height = f32::from(vcx.debug_bounds("label").unwrap().size.height);
        assert!(
            (height - style.line_height).abs() < 0.5,
            "{height} vs {}",
            style.line_height
        );
    }

    #[test]
    fn label_keeps_its_text_and_source() {
        let l = Label::new(
            "orders",
            TextSource::Catalog,
            DARK.type_ramp.body,
            DARK.colors.text.to_hsla(),
        )
        .truncate(true);
        assert_eq!(
            (l.text().as_ref(), l.source(), l.truncate),
            ("orders", TextSource::Catalog, true)
        );
    }
}
