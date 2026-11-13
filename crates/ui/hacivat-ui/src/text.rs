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

use hacivat_ui_core::fonts::{font_for, small_caps_glyphs, small_caps_tracking_px};
use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, AppContext as _, Bounds, ContentMask, Context, Element, ElementId,
    FontStyle, Global, GlobalElementId, Hsla, InspectorElementId, InteractiveElement, IntoElement,
    LayoutId, ParentElement, Pixels, Point, Render, ShapedLine, SharedString,
    StatefulInteractiveElement as _, Style, Styled, TextAlign, TextRun, UnderlineStyle, Window,
    canvas, div, point, px, relative,
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
    /// The ink width of the run actually painted (for small caps without the gap after the last
    /// glyph). Measuring and painting are the same numbers, so this never exceeds the width the
    /// label was laid out with.
    pub painted_px: f32,
    /// Wider than its bounds and not truncating: part of the text is cut off.
    pub clipped: bool,
    /// Wider than its bounds and drawn ellipsized.
    pub truncated: bool,
    /// Where the text was created (`file:line:column`, plus `#id` for a label with an ID). The
    /// file uses `/` separators on every platform.
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
        begin_record(cx);
        cx.global_mut::<TextRegistry>().frame.push(text);
    }

    // ## m2a-t21-shots
    /// Notes the clip of a scroll viewport (`mask`, recorded by [`scroll_viewport`]) for this
    /// frame, when the registry is enabled: text that this clip cuts at its top or bottom is
    /// scrolling, not clipped.
    pub fn note_scroll_viewport(cx: &mut App, mask: Bounds<Pixels>) {
        if !cx.try_global::<TextRegistry>().is_some_and(|r| r.enabled) {
            return;
        }
        begin_record(cx);
        cx.default_global::<ScrollViewports>().0.push(mask);
    }
}

// ## m2a-t21-shots
/// The scroll viewports' clips of the current frame (see [`TextRegistry::note_scroll_viewport`]).
#[derive(Default)]
struct ScrollViewports(Vec<Bounds<Pixels>>);

impl Global for ScrollViewports {}

/// Starts a frame's debug records on the first record of the frame (a label, a paragraph, a
/// scroll viewport or the capsule probe): the text frame, the scroll viewports and the capsule
/// probe start empty, so nothing from an earlier frame is ever read as this frame's. Records
/// happen only while the registry is enabled or the probe installed (tests and the shots).
pub(crate) fn begin_record(cx: &mut App) {
    let first_of_frame = !std::mem::replace(&mut cx.default_global::<RecordingFrame>().0, true);
    if !first_of_frame {
        return;
    }
    if cx.has_global::<TextRegistry>() {
        cx.global_mut::<TextRegistry>().frame.clear();
    }
    if cx.has_global::<ScrollViewports>() {
        cx.global_mut::<ScrollViewports>().0.clear();
    }
    if cx.has_global::<crate::capsule::CapsuleProbe>() {
        cx.global_mut::<crate::capsule::CapsuleProbe>().bounds = None;
    }
    // Deferred effects run once the current update (and with it this draw) is done.
    cx.defer(|cx| cx.default_global::<RecordingFrame>().0 = false);
}

/// Marks its parent as a scroll viewport for the frame checks: put it first among the children
/// of an `overflow_*_scroll` element. It records the clip it is laid out in, which is the
/// viewport's own clip (not its scrolled content), so text cut at that clip's top or bottom
/// edge counts as scrolled, not clipped.
pub fn scroll_viewport() -> impl IntoElement {
    canvas(
        |_, window, cx| TextRegistry::note_scroll_viewport(cx, window.content_mask().bounds),
        |_, (), _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_0()
}

/// Whether `mask` is the clip of a scroll viewport noted this frame (each edge within 1 px):
/// a nested, non-scrolling clip inside a viewport is smaller and does not count.
fn is_scroll_viewport(cx: &App, mask: Bounds<Pixels>) -> bool {
    let near = |a: Pixels, b: Pixels| (a - b).abs() <= px(1.0);
    cx.try_global::<ScrollViewports>().is_some_and(|v| {
        v.0.iter().any(|v| {
            near(v.left(), mask.left())
                && near(v.right(), mask.right())
                && near(v.top(), mask.top())
                && near(v.bottom(), mask.bottom())
        })
    })
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
    // ## m2b-t22-live
    italic: bool,
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
            italic: false,
        }
    }

    // ## m2b-t22-live
    /// Draws the text in the family's italic face (the placeholders and comments of the SQL
    /// inset, NOTES M2b surface 6). Not applied to synthesized small caps.
    pub fn italic(mut self, italic: bool) -> Self {
        self.italic = italic;
        self
    }

    /// Ellipsize with `…` instead of overflowing; the full text becomes the tooltip.
    ///
    /// A truncating label must be drawn inside a view (any `Render` entity): GPUI's tooltip
    /// machinery reads the current view and panics without one.
    pub fn truncate(mut self, truncate: bool) -> Self {
        self.truncate = truncate;
        self
    }

    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Underlines the byte `ranges` of the text in `color` (matched characters in the palette).
    /// Ranges are clamped to the text and to character boundaries. Highlights are not applied to
    /// synthesized small caps (their glyphs are separate lines); only plain styles underline.
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

    /// `file:line:column` (plus `#id`), with `/` separators on every platform: `Location::file`
    /// uses `\` on Windows, and callers match the path by a `crate/src/file.rs:` suffix.
    fn path(&self) -> SharedString {
        let l = self.location;
        let file = l.file().replace('\\', "/");
        match &self.id {
            Some(id) => format!("{file}:{}:{}#{id}", l.line(), l.column()).into(),
            None => format!("{file}:{}:{}", l.line(), l.column()).into(),
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
    /// Per-glyph runs of synthesized small caps; their advances are the single source of the
    /// run's width, for measuring, ellipsizing and painting alike.
    glyphs: Vec<CapsGlyph>,
}

/// One glyph of a synthesized small-caps run.
struct CapsGlyph {
    ch: char,
    line: ShapedLine,
    /// Glyph width plus the tracking gap after it.
    advance: Pixels,
}

enum Painted {
    Line(Box<ShapedLine>),
    /// Small-caps glyph lines with their offsets from the label origin.
    Caps(Vec<(Point<Pixels>, ShapedLine)>, Pixels),
}

const ELLIPSIS: &str = "…";

/// `at` clamped to `text` and floored to a char boundary.
fn floor_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

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
        let mut font = font_for(&self.label.style);
        if self.label.italic {
            font.style = FontStyle::Italic;
        }
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
    /// Highlight ranges clamped to `text[..limit]`: an ellipsized line underlines only the
    /// kept prefix, never the `…`.
    fn highlight_in(&self, text: &str, limit: usize) -> Vec<Range<usize>> {
        let text = &text[..floor_boundary(text, limit)];
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

    /// Shapes `text`, underlining highlights only within its first `highlight_limit` bytes.
    fn shape(&self, text: SharedString, highlight_limit: usize, window: &mut Window) -> ShapedLine {
        let highlight = self.highlight_in(&text, highlight_limit);
        let runs = self.runs(text.len(), &highlight);
        window
            .text_system()
            .shape_line(text, px(self.label.style.size_px), &runs, None)
    }

    /// Decides what gets painted: the full run, or its ellipsized prefix. The glyph lines
    /// shaped for measuring are the ones painted.
    fn plan(
        &self,
        bounds: Bounds<Pixels>,
        measured: &mut Measured,
        ellipsize: bool,
        window: &mut Window,
    ) -> Option<Painted> {
        let style = self.label.style;
        if style.caps == Caps::SynthesizedSmallCaps {
            let glyphs = std::mem::take(&mut measured.glyphs);
            let mut kept = glyphs.len();
            let mut ellipsis_glyph = None;
            if ellipsize {
                let x_for = |byte: usize| {
                    let mut x = px(0.0);
                    let mut at = 0;
                    for g in &glyphs {
                        if at >= byte {
                            break;
                        }
                        x += g.advance;
                        at += g.ch.len_utf8();
                    }
                    x
                };
                // `…` is not alphabetic: it keeps the full size, like digits and punctuation.
                let ellipsis = self.shape(ELLIPSIS.into(), 0, window);
                let gap = px(small_caps_tracking_px(&style));
                let end = fitting_prefix(
                    &self.label.text,
                    bounds.size.width,
                    ellipsis.width + gap,
                    x_for,
                );
                kept = self.label.text[..end].chars().count();
                ellipsis_glyph = Some(ellipsis);
            }
            let baseline =
                |l: &ShapedLine| (px(style.line_height) - l.ascent - l.descent) / 2.0 + l.ascent;
            let lines: Vec<ShapedLine> = glyphs
                .into_iter()
                .take(kept)
                .map(|g| g.line)
                .chain(ellipsis_glyph)
                .collect();
            // Baseline-align the glyphs of different sizes. The baseline comes from the style's
            // full-size metrics, not from the glyphs present, so `HACIVAT` and `HACIVAT 2` sit
            // at the same height.
            let reference = TextRun {
                len: 1,
                font: font_for(&style),
                color: self.label.color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let top = baseline(&window.text_system().shape_line(
                "x".into(),
                px(style.size_px),
                &[reference],
                None,
            ));
            let mut x = px(0.0);
            let gap = px(small_caps_tracking_px(&style));
            let placed: Vec<_> = lines
                .into_iter()
                .map(|l| {
                    let at = point(x, top - baseline(&l));
                    x += l.width + gap;
                    (at, l)
                })
                .collect();
            // The tracking after the last glyph is spacing, not ink.
            return Some(Painted::Caps(placed, (x - gap).max(px(0.0))));
        }

        let line = measured.line.take()?;
        if !ellipsize {
            return Some(Painted::Line(Box::new(line)));
        }
        let ellipsis = self.shape(ELLIPSIS.into(), 0, window).width;
        let text = &self.label.text;
        let end = fitting_prefix(text, bounds.size.width, ellipsis, |b| line.x_for_index(b));
        let shown = format!("{}{ELLIPSIS}", &text[..end]);
        Some(Painted::Line(Box::new(self.shape(
            shown.into(),
            end,
            window,
        ))))
    }

    fn measure(&self, window: &mut Window) -> Measured {
        let style = &self.label.style;
        if style.caps == Caps::SynthesizedSmallCaps {
            // Every glyph is its own shaped line followed by the tracking (GPUI has no letter-spacing).
            // Painting places these same lines, so measure and paint cannot drift apart.
            let gap = px(small_caps_tracking_px(style));
            let font = font_for(style);
            let glyphs: Vec<CapsGlyph> = small_caps_glyphs(&self.label.text, style.size_px)
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
                    let line = window.text_system().shape_line(
                        ch.to_string().into(),
                        px(size_px),
                        &[run],
                        None,
                    );
                    CapsGlyph {
                        ch,
                        advance: line.width + gap,
                        line,
                    }
                })
                .collect();
            let width = glyphs.iter().map(|g| g.advance).fold(px(0.0), |a, b| a + b);
            Measured {
                width,
                line: None,
                glyphs,
            }
        } else {
            let line = self.shape(self.label.text.clone(), usize::MAX, window);
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
        let painted = self.plan(bounds, measured, ellipsize, window);
        // What is recorded as clipped is what the painted run really overflows by, not what
        // was measured: the two are the same widths by construction, and this keeps it so.
        let painted_width = match &painted {
            Some(Painted::Line(line)) => line.width,
            Some(Painted::Caps(_, width)) => *width,
            None => px(0.0),
        };
        let mask = window.content_mask().bounds;
        let cut = cut_by_clip(bounds, mask, is_scroll_viewport(cx, mask));
        TextRegistry::record(cx, || RenderedText {
            text: self.label.text.clone(),
            source: self.label.source,
            size_px: self.label.style.size_px,
            painted_px: f32::from(painted_width),
            clipped: painted_width > bounds.size.width + px(0.5) || cut,
            truncated: ellipsize,
            path: self.label.path(),
        });
        painted
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
            Painted::Caps(glyphs, _) => {
                for (offset, line) in glyphs {
                    let _ = line.paint(
                        bounds.origin + *offset,
                        line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
            }
        });
    }
}

// ## m2a-t21-shots

/// Wrapping prose (a [`Label`] is one line), recorded in the [`TextRegistry`] while it is laid
/// out, in the tree's order like a label, with what it really drew: `painted_px` is its widest wrapped
/// line, and `clipped` is set when the wrapped text is taller or wider than its box, or when
/// the box runs past the clip of its container on the left or right, or is cut at its top or
/// bottom edge. The frame checks (pseudo-locale
/// bracketing, nothing clipped, the 11 px floor) then see paragraphs too.
pub struct Paragraph {
    text: SharedString,
    source: TextSource,
    style: TypeStyle,
    color: Hsla,
    id: Option<&'static str>,
    centered: bool,
    location: &'static Location<'static>,
}

/// A [`Paragraph`] of `text` in `style` and `color`.
#[track_caller]
pub fn paragraph(
    text: impl Into<SharedString>,
    source: TextSource,
    style: TypeStyle,
    color: Hsla,
) -> Paragraph {
    Paragraph {
        text: text.into(),
        source,
        style,
        color,
        id: None,
        centered: false,
        location: Location::caller(),
    }
}

impl Paragraph {
    /// Appended to the recorded path as `#id`, for tests.
    pub fn id(mut self, id: &'static str) -> Self {
        self.id = Some(id);
        self
    }

    /// Centres each wrapped line (empty states).
    pub fn centered(mut self) -> Self {
        self.centered = true;
        self
    }
}

/// The clip policy shared by [`Label`] and [`Paragraph`]: whether text laid out in `bounds`
/// is cut by its container's clip (`mask`).
/// - Text wholly outside the clip is hidden, not cut (a row scrolled away).
/// - Text that shows and runs past the clip's left or right edge is cut (a control row wider
///   than its card). No Hacivat surface scrolls sideways.
/// - Text that shows and runs past the clip's top or bottom edge is cut, unless the clip is a
///   scroll viewport (`scrolls`, marked with [`scroll_viewport`]): there the rest of it is a
///   scroll away.
///
/// Limits: the check sees the nearest clip only (an outer clip cutting a nested one is seen by
/// the nested text as the intersection, which is what it draws in); a scroll container that is
/// not marked with [`scroll_viewport`] counts as a cutting container; and text painted outside
/// the elements it was laid out in (`absolute` overlays with no clip) is not checked against
/// what it covers. Half a pixel of slack absorbs rounding.
fn cut_by_clip(bounds: Bounds<Pixels>, mask: Bounds<Pixels>, scrolls: bool) -> bool {
    let slack = px(0.5);
    let shows = bounds.top() < mask.bottom()
        && bounds.bottom() > mask.top()
        && bounds.left() < mask.right()
        && bounds.right() > mask.left();
    if !shows {
        return false;
    }
    let sideways = bounds.left() < mask.left() - slack || bounds.right() > mask.right() + slack;
    let vertically = bounds.top() < mask.top() - slack || bounds.bottom() > mask.bottom() + slack;
    sideways || (vertically && !scrolls)
}

/// What a paragraph laid out in `bounds` (clipped to `mask`) drew: its widest line and whether
/// part of it is cut off. `lines` holds each wrapped line's width and height. Half a pixel of
/// slack absorbs rounding.
fn paragraph_fit(
    lines: &[(Pixels, Pixels)],
    bounds: Bounds<Pixels>,
    mask: Bounds<Pixels>,
    scrolls: bool,
) -> (Pixels, bool) {
    let slack = px(0.5);
    let widest = lines.iter().map(|(w, _)| *w).fold(px(0.0), Pixels::max);
    let height = lines.iter().map(|(_, h)| *h).fold(px(0.0), |a, b| a + b);
    let clipped = height > bounds.size.height + slack
        || widest > bounds.size.width + slack
        || cut_by_clip(bounds, mask, scrolls);
    (widest, clipped)
}

impl IntoElement for Paragraph {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        let Paragraph {
            text,
            source,
            style,
            color,
            id,
            centered,
            location,
        } = self;
        let file = location.file().replace('\\', "/");
        let path: SharedString = match id {
            Some(id) => format!("{file}:{}:{}#{id}", location.line(), location.column()),
            None => format!("{file}:{}:{}", location.line(), location.column()),
        }
        .into();
        let recorded = text.clone();
        // Recorded in prepaint, as `Label` records, so the registry keeps the tree's order.
        let probe = canvas(
            move |bounds, window, cx| {
                if !cx.try_global::<TextRegistry>().is_some_and(|r| r.enabled) {
                    return;
                }
                let run = TextRun {
                    len: recorded.len(),
                    font: font_for(&style),
                    color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let line_height = px(style.line_height);
                let lines: Vec<(Pixels, Pixels)> = window
                    .text_system()
                    .shape_text(
                        recorded.clone(),
                        px(style.size_px),
                        &[run],
                        Some(bounds.size.width),
                        None,
                    )
                    .map(|lines| {
                        lines
                            .iter()
                            .map(|l| {
                                let size = l.size(line_height);
                                (size.width, size.height)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let mask = window.content_mask().bounds;
                let scrolls = is_scroll_viewport(cx, mask);
                let (painted, clipped) = paragraph_fit(&lines, bounds, mask, scrolls);
                TextRegistry::record(cx, || RenderedText {
                    text: recorded,
                    source,
                    size_px: style.size_px,
                    painted_px: f32::from(painted),
                    clipped,
                    truncated: false,
                    path,
                });
            },
            |_, (), _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let mut block = div().relative().w_full();
        if centered {
            block = block.text_center();
        }
        block
            .font(font_for(&style))
            .text_size(px(style.size_px))
            .line_height(px(style.line_height))
            .text_color(color)
            .child(text)
            .child(probe)
            .into_any_element()
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

    #[gpui::test]
    fn small_caps_baseline_does_not_depend_on_which_glyphs_are_present(cx: &mut TestAppContext) {
        let vcx = cx.add_empty_window();
        let style = DARK.type_ramp.label_sc;
        let first_y = |text: &'static str, vcx: &mut gpui::VisualTestContext| {
            vcx.update(|window, cx| {
                hacivat_ui_core::fonts::register_fonts(cx).unwrap();
                let el = LabelElement {
                    label: Label::new(text, TextSource::Data, style, DARK.colors.text.to_hsla()),
                    truncated: None,
                };
                let mut measured = el.measure(window);
                let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(300.0), px(30.0)));
                match el.plan(bounds, &mut measured, false, window) {
                    Some(Painted::Caps(placed, _)) => f32::from(placed[0].0.y),
                    _ => panic!("small caps expected"),
                }
            })
        };
        // `H` is a small-capital glyph in both; the digit only adds a full-size glyph.
        assert_eq!(first_y("HACIVAT", vcx), first_y("HACIVAT 2", vcx));
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
        assert_eq!(
            el.highlight_in("İstanbul", usize::MAX),
            vec![0..2, 3..4, 5..9]
        );
        let runs = el.runs(9, &el.highlight_in("İstanbul", usize::MAX));
        assert_eq!(
            runs.iter()
                .map(|r| (r.len, r.underline.is_some()))
                .collect::<Vec<_>>(),
            vec![(2, true), (1, false), (1, true), (1, false), (4, true)]
        );
    }

    #[test]
    fn ellipsized_line_never_underlines_the_ellipsis() {
        let el = LabelElement {
            label: Label::new(
                "İstanbul",
                TextSource::Data,
                DARK.type_ramp.body,
                DARK.colors.text.to_hsla(),
            )
            .highlight(vec![0..3, 3..9], DARK.colors.blue.to_hsla()),
            truncated: None,
        };
        // Cut after "İsta" (5 bytes) and append `…` (3 bytes), as prepaint does.
        let shown = format!("{}{ELLIPSIS}", &"İstanbul"[..5]);
        let highlight = el.highlight_in(&shown, 5);
        assert_eq!(highlight, vec![0..5]);
        let runs = el.runs(shown.len(), &highlight);
        assert_eq!(
            runs.iter()
                .map(|r| (r.len, r.underline.is_some()))
                .collect::<Vec<_>>(),
            vec![(5, true), (3, false)]
        );
        // A cut inside a character floors to its boundary.
        assert_eq!(el.highlight_in(&shown, 1), Vec::<Range<usize>>::new());
    }

    // ## m2a-t21-shots

    #[test]
    fn paragraphs_and_labels_cut_by_their_box_or_their_clip_are_clipped() {
        let b = |x: f32, y: f32, w: f32, h: f32| {
            Bounds::new(
                point(px(x), px(y)),
                hacivat_ui_core::gpui_surface::size(px(w), px(h)),
            )
        };
        let mask = b(0.0, 0.0, 300.0, 1000.0);
        let lines = [(px(120.0), px(18.0)), (px(80.0), px(18.0))];
        // Two 18 px lines in a 36 px box: everything shows.
        assert_eq!(
            paragraph_fit(&lines, b(10.0, 0.0, 276.0, 36.0), mask, false),
            (px(120.0), false)
        );
        // A box one line short; a line wider than its box (a word that cannot wrap).
        assert!(paragraph_fit(&lines, b(10.0, 0.0, 276.0, 18.0), mask, false).1);
        assert!(paragraph_fit(&lines, b(10.0, 0.0, 100.0, 36.0), mask, false).1);
        // Past the clip on the right or the left: cut, scrolling or not.
        for scrolls in [false, true] {
            assert!(cut_by_clip(b(200.0, 0.0, 120.0, 18.0), mask, scrolls));
            assert!(cut_by_clip(b(-4.0, 0.0, 276.0, 18.0), mask, scrolls));
        }
        // Past the clip's bottom: cut by a non-scrolling container, scrolling in a viewport.
        let short = b(0.0, 0.0, 300.0, 10.0);
        assert!(cut_by_clip(b(10.0, 0.0, 200.0, 18.0), short, false));
        assert!(!cut_by_clip(b(10.0, 0.0, 200.0, 18.0), short, true));
        assert!(paragraph_fit(&lines, b(10.0, 0.0, 276.0, 36.0), short, false).1);
        assert!(!paragraph_fit(&lines, b(10.0, 0.0, 276.0, 36.0), short, true).1);
        // Wholly outside the clip: hidden, not cut.
        let above = b(0.0, -100.0, 300.0, 50.0);
        assert!(!cut_by_clip(b(10.0, 0.0, 276.0, 36.0), above, false));
        // Inside: never cut.
        assert!(!cut_by_clip(b(10.0, 0.0, 200.0, 18.0), mask, false));
    }
}
