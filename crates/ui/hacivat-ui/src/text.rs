//! Text sources, stored label closures, the `Label` element and the text registry.
//!
//! `Label` shapes its text with the window's text system, ellipsizes it with `…` when asked to
//! truncate, and otherwise clips it to its bounds. With the [`TextRegistry`] enabled, every
//! label records what it drew, so tests can check that visible text comes from Fluent and
//! that nothing is clipped (Task 13, Task 18).

use std::cell::Cell;
use std::ops::Range;
use std::panic::Location;
use std::rc::Rc;

use hacivat_ui_core::fonts::{font_for, small_caps, small_caps_glyphs, small_caps_tracking_px};
use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, AppContext as _, AvailableSpace, Bounds, ContentMask, Context,
    Element, ElementId, Global, GlobalElementId, Hsla, InspectorElementId, InteractiveElement,
    IntoElement, LayoutId, ParentElement, Pixels, Render, ShapedLine, SharedString,
    StatefulInteractiveElement as _, Style, Styled, TextAlign, TextRun, UnderlineStyle, Window,
    div, px, relative, size,
};
use hacivat_ui_core::theme::{Caps, Theme, TypeStyle, popover_surface};
use unicode_segmentation::UnicodeSegmentation as _;

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

/// One string drawn in the last frame.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedText {
    /// The full text, before any ellipsis.
    pub text: SharedString,
    pub source: TextSource,
    pub size_px: f32,
    /// Wider than its bounds and not truncating: part of the text is cut off.
    pub clipped: bool,
    /// Wider than its bounds and drawn ellipsized.
    pub truncated: bool,
    /// Where the text was created (`file:line:column`, plus `#id` for a label with an ID).
    pub path: SharedString,
}

/// What the last frame drew, when `enabled` (tests and debug tooling; off by default). Each
/// frame replaces `frame`; with several windows, the last drawn window wins.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextRegistry {
    pub enabled: bool,
    pub frame: Vec<RenderedText>,
}

impl Global for TextRegistry {}

/// Set by the first record of a frame and cleared once that frame is done (`App::defer`).
#[derive(Default)]
struct RecordingFrame(bool);

impl Global for RecordingFrame {}

impl TextRegistry {
    /// Records `text` into the current frame when the registry is installed and enabled.
    pub fn record(cx: &mut App, text: impl FnOnce() -> RenderedText) {
        let Some(reg) = cx.try_global::<TextRegistry>() else {
            return;
        };
        if !reg.enabled {
            return;
        }
        let text = text();
        let first_of_frame = !std::mem::replace(&mut cx.default_global::<RecordingFrame>().0, true);
        let reg = cx.global_mut::<TextRegistry>();
        if first_of_frame {
            reg.frame.clear();
        }
        reg.frame.push(text);
        if first_of_frame {
            // Deferred effects run once the current update (and with it this draw) is done.
            cx.defer(|cx| cx.default_global::<RecordingFrame>().0 = false);
        }
    }
}

/// One line of text in a type style.
#[derive(Debug, Clone)]
pub struct Label {
    text: SharedString,
    source: TextSource,
    style: TypeStyle,
    color: Hsla,
    truncate: bool,
    id: Option<ElementId>,
    highlight: Vec<Range<usize>>,
    highlight_color: Option<Hsla>,
    location: &'static Location<'static>,
}

impl Label {
    #[track_caller]
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
            highlight: Vec::new(),
            highlight_color: None,
            location: Location::caller(),
        }
    }

    /// Ellipsize with `…` instead of overflowing; the full text becomes the tooltip.
    pub fn truncate(mut self, truncate: bool) -> Self {
        self.truncate = truncate;
        self
    }

    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Underlines the byte `ranges` of the text in `color` (matched characters in the palette).
    /// Ranges are clamped to the text and to character boundaries.
    pub fn highlight(mut self, ranges: Vec<Range<usize>>, color: Hsla) -> Self {
        self.highlight = ranges;
        self.highlight_color = Some(color);
        self
    }

    pub fn text(&self) -> &SharedString {
        &self.text
    }

    pub fn source(&self) -> TextSource {
        self.source
    }

    fn path(&self) -> SharedString {
        let l = self.location;
        match &self.id {
            Some(id) => format!("{}:{}:{}#{id}", l.file(), l.line(), l.column()).into(),
            None => format!("{}:{}:{}", l.file(), l.line(), l.column()).into(),
        }
    }
}

impl IntoElement for Label {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        if !self.truncate {
            return LabelElement {
                label: self,
                truncated: None,
            }
            .into_any_element();
        }
        // A truncating label sits in a stateful wrapper that carries the tooltip; the tooltip
        // shows the full text only while the label is actually ellipsized.
        let truncated = Rc::new(Cell::new(false));
        let id = self.id.clone().unwrap_or_else(|| {
            ElementId::Name(format!("label:{}:{}", self.path(), self.text).into())
        });
        let tooltip = TooltipText {
            text: self.text.clone(),
            source: self.source,
            shown: truncated.clone(),
        };
        InteractiveElement::id(div(), id)
            .flex()
            .min_w(px(0.0))
            .tooltip(move |_, cx| AnyView::from(cx.new(|_| tooltip.clone())))
            .child(LabelElement {
                label: self,
                truncated: Some(truncated),
            })
            .into_any_element()
    }
}

/// The tooltip of a truncating label: its full text on a floating surface.
#[derive(Clone)]
struct TooltipText {
    text: SharedString,
    source: TextSource,
    shown: Rc<Cell<bool>>,
}

impl Render for TooltipText {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.shown.get() {
            return div();
        }
        let theme = Theme::get(cx);
        let t = theme.tokens;
        popover_surface(theme)
            .rounded(px(t.radii.xs))
            .px(px(t.spacing.sm))
            .py(px(t.spacing.s_2xs))
            .child(Label::new(
                self.text.clone(),
                self.source,
                t.type_ramp.body_sm,
                t.colors.text.to_hsla(),
            ))
    }
}

/// The custom element behind [`Label`].
struct LabelElement {
    label: Label,
    /// Set when this frame ellipsized the text (truncating labels only).
    truncated: Option<Rc<Cell<bool>>>,
}

/// Natural (unconstrained) width of the text.
struct Measured {
    width: Pixels,
    /// The full line, for plain styles.
    line: Option<ShapedLine>,
    /// Per-char advance including tracking, for synthesized small caps.
    glyphs: Vec<(char, Pixels)>,
}

enum Painted {
    Line(Box<ShapedLine>),
    Caps(AnyElement),
}

const ELLIPSIS: &str = "…";

/// The largest prefix (in bytes, at a grapheme boundary) of `text` that fits `max_width` with
/// an ellipsis of `ellipsis_width` after it; `x_for(byte)` gives the prefix width.
pub(crate) fn fitting_prefix(
    text: &str,
    max_width: Pixels,
    ellipsis_width: Pixels,
    x_for: impl Fn(usize) -> Pixels,
) -> usize {
    let mut fit = 0;
    for (start, g) in text.grapheme_indices(true) {
        let end = start + g.len();
        if x_for(end) + ellipsis_width > max_width {
            break;
        }
        fit = end;
    }
    fit
}

impl LabelElement {
    fn runs(&self, text_len: usize, highlight: &[Range<usize>]) -> Vec<TextRun> {
        let font = font_for(&self.label.style);
        let run = |len, underline: Option<UnderlineStyle>| TextRun {
            len,
            font: font.clone(),
            color: self.label.color,
            background_color: None,
            underline,
            strikethrough: None,
        };
        let mut runs = Vec::new();
        let mut at = 0;
        let underline = self.label.highlight_color.map(|color| UnderlineStyle {
            thickness: px(1.0),
            color: Some(color),
            wavy: false,
        });
        for r in highlight {
            if r.start > at {
                runs.push(run(r.start - at, None));
            }
            runs.push(run(r.end - r.start, underline));
            at = r.end;
        }
        if text_len > at || runs.is_empty() {
            runs.push(run(text_len - at, None));
        }
        runs
    }

    /// Highlight ranges clamped to `text`, on char boundaries, sorted and disjoint.
    fn highlight_in(&self, text: &str) -> Vec<Range<usize>> {
        let floor = |mut i: usize| {
            i = i.min(text.len());
            while !text.is_char_boundary(i) {
                i -= 1;
            }
            i
        };
        let mut ranges: Vec<Range<usize>> = self
            .label
            .highlight
            .iter()
            .map(|r| floor(r.start)..floor(r.end))
            .filter(|r| r.start < r.end)
            .collect();
        ranges.sort_by_key(|r| r.start);
        let mut out: Vec<Range<usize>> = Vec::new();
        for r in ranges {
            match out.last_mut() {
                Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
                _ => out.push(r),
            }
        }
        out
    }

    fn shape(&self, text: SharedString, window: &mut Window) -> ShapedLine {
        let highlight = self.highlight_in(&text);
        let runs = self.runs(text.len(), &highlight);
        window
            .text_system()
            .shape_line(text, px(self.label.style.size_px), &runs, None)
    }

    fn measure(&self, window: &mut Window) -> Measured {
        let style = &self.label.style;
        if style.caps == Caps::SynthesizedSmallCaps {
            // `fonts::small_caps` lays every glyph out as its own box followed by the tracking.
            let gap = px(small_caps_tracking_px(style));
            let font = font_for(style);
            let glyphs: Vec<(char, Pixels)> = small_caps_glyphs(&self.label.text, style.size_px)
                .into_iter()
                .map(|(ch, size_px)| {
                    let run = TextRun {
                        len: ch.len_utf8(),
                        font: font.clone(),
                        color: self.label.color,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    let w = window
                        .text_system()
                        .shape_line(ch.to_string().into(), px(size_px), &[run], None)
                        .width;
                    (ch, w + gap)
                })
                .collect();
            let width = glyphs.iter().map(|g| g.1).fold(px(0.0), |a, b| a + b);
            Measured {
                width,
                line: None,
                glyphs,
            }
        } else {
            let line = self.shape(self.label.text.clone(), window);
            Measured {
                width: line.width,
                line: Some(line),
                glyphs: Vec::new(),
            }
        }
    }
}

impl Element for LabelElement {
    type RequestLayoutState = Measured;
    type PrepaintState = Option<Painted>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        Some(self.label.location)
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Measured) {
        let measured = self.measure(window);
        // The natural width, never wider than the parent and free to shrink in a flex row:
        // what does not fit is ellipsized or clipped (and then recorded as clipped).
        let mut style = Style::default();
        style.size.width = measured.width.into();
        style.size.height = px(self.label.style.line_height).into();
        style.max_size.width = relative(1.0).into();
        style.min_size.width = px(0.0).into();
        style.flex_shrink = 1.0;
        (window.request_layout(style, [], cx), measured)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        measured: &mut Measured,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Painted> {
        let overflows = measured.width > bounds.size.width + px(0.5);
        let ellipsize = overflows && self.label.truncate;
        if let Some(flag) = &self.truncated {
            flag.set(ellipsize);
        }
        TextRegistry::record(cx, || RenderedText {
            text: self.label.text.clone(),
            source: self.label.source,
            size_px: self.label.style.size_px,
            clipped: overflows && !self.label.truncate,
            truncated: ellipsize,
            path: self.label.path(),
        });

        let style = self.label.style;
        if style.caps == Caps::SynthesizedSmallCaps {
            let text = if ellipsize {
                let glyphs = &measured.glyphs;
                let x_for = |byte: usize| {
                    let mut x = px(0.0);
                    let mut at = 0;
                    for (ch, w) in glyphs {
                        if at >= byte {
                            break;
                        }
                        x += *w;
                        at += ch.len_utf8();
                    }
                    x
                };
                // `…` is not alphabetic: it keeps the full size, like digits and punctuation.
                let ellipsis =
                    self.shape(ELLIPSIS.into(), window).width + px(small_caps_tracking_px(&style));
                let text = &self.label.text;
                let end = fitting_prefix(text, bounds.size.width, ellipsis, x_for);
                format!("{}{ELLIPSIS}", &text[..end])
            } else {
                self.label.text.to_string()
            };
            let mut child = small_caps(&style, self.label.color, &text).into_any_element();
            child.layout_as_root(
                size(
                    AvailableSpace::MinContent,
                    AvailableSpace::Definite(px(style.line_height)),
                ),
                window,
                cx,
            );
            child.prepaint_at(bounds.origin, window, cx);
            return Some(Painted::Caps(child));
        }

        let line = measured.line.take()?;
        if !ellipsize {
            return Some(Painted::Line(Box::new(line)));
        }
        let ellipsis = self.shape(ELLIPSIS.into(), window).width;
        let text = &self.label.text;
        let end = fitting_prefix(text, bounds.size.width, ellipsis, |b| line.x_for_index(b));
        let shown = format!("{}{ELLIPSIS}", &text[..end]);
        Some(Painted::Line(Box::new(self.shape(shown.into(), window))))
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Measured,
        painted: &mut Option<Painted>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(painted) = painted else { return };
        let line_height = px(self.label.style.line_height);
        window.with_content_mask(Some(ContentMask { bounds }), |window| match painted {
            Painted::Line(line) => {
                // Painting only fails when the glyph atlas cannot take a glyph; the label then
                // misses glyphs for one frame, as GPUI's own text elements do.
                let _ = line.paint(
                    bounds.origin,
                    line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
            Painted::Caps(child) => child.paint(window, cx),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, point};
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

    #[test]
    fn fitting_prefix_keeps_whole_graphemes() {
        // 10 px per char: "ai̇b" is a, i + combining dot (one grapheme), b.
        let text = "ai\u{0307}b";
        let x_for = |byte: usize| px(10.0 * text[..byte].chars().count() as f32);
        assert_eq!(fitting_prefix(text, px(45.0), px(5.0), x_for), text.len());
        assert_eq!(
            fitting_prefix(text, px(44.0), px(5.0), x_for),
            "ai\u{0307}".len()
        );
        assert_eq!(fitting_prefix(text, px(34.0), px(5.0), x_for), 1);
        assert_eq!(fitting_prefix(text, px(4.0), px(5.0), x_for), 0);
    }

    #[test]
    fn highlight_ranges_are_clamped_merged_and_on_char_boundaries() {
        let el = LabelElement {
            label: Label::new(
                "İstanbul",
                TextSource::Data,
                DARK.type_ramp.body,
                DARK.colors.text.to_hsla(),
            )
            .highlight(vec![5..99, 1..2, 3..4], DARK.colors.blue.to_hsla()),
            truncated: None,
        };
        // `İ` is two bytes: 1..2 floors to 0..2; 5..99 clamps to the end.
        assert_eq!(el.highlight_in("İstanbul"), vec![0..2, 3..4, 5..9]);
        let runs = el.runs(9, &el.highlight_in("İstanbul"));
        assert_eq!(
            runs.iter()
                .map(|r| (r.len, r.underline.is_some()))
                .collect::<Vec<_>>(),
            vec![(2, true), (1, false), (1, true), (1, false), (4, true)]
        );
    }
}
