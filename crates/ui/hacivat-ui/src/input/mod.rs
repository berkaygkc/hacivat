//! Single-line text input with IME composition.
//!
//! Plain typing and IME composition share one path: GPUI delivers all text to
//! [`EntityInputHandler::replace_text_in_range`] (and marked text to
//! `replace_and_mark_text_in_range`); the key handler only edits with named keys (Backspace,
//! arrows) and never inserts printable characters itself. While a composition is in progress,
//! every key belongs to the IME. The model is [`ime::ImeState`].
//!
//! A masked input (M2a, passwords) paints `•` per grapheme, never copies or cuts, and exposes an
//! empty accessible value; [`TextInput::value`] still returns the text. A paste reaches the app
//! only through [`TextInput::set_paste_filter`] (P16), which a masked input never calls.

pub mod ime;

use std::ops::Range;
use std::rc::Rc;

use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::gpui_surface::FluentBuilder as _;
use hacivat_ui_core::gpui_surface::{
    App, Bounds, ClipboardItem, ContentMask, Context, Element, ElementId, ElementInputHandler,
    Entity, EntityId, EntityInputHandler, EventEmitter, FocusHandle, Focusable, Global,
    GlobalElementId, InspectorElementId, InteractiveElement as _, IntoElement, KeyDownEvent,
    LayoutId, MouseButton, MouseDownEvent, ParentElement as _, Pixels, Point, Render, Role,
    ShapedLine, SharedString, Style, Styled as _, Subscription, TextAlign, TextRun, UTF16Selection,
    UnderlineStyle, Window, div, fill, point, px, relative, size,
};
use hacivat_ui_core::theme::{Theme, ThemeMode, focus_outline};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::popover::A11yNode;
use crate::text::{RenderedText, TextFn, TextRegistry, TextSource};
use ime::ImeState;

/// Events a `TextInput` emits (F15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputEvent {
    Changed,
    Submitted,
    Cancelled,
    /// `true` on the first mark of an IME composition, `false` when it commits or is cancelled.
    CompositionChanged(bool),
}

/// The focused text input and whether it is composing. Every `TextInput` keeps it current on
/// focus, blur, mark and unmark (Task 11); the key dispatcher reads `composing` (Task 16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FocusedInput {
    pub entity: Option<EntityId>,
    pub composing: bool,
}

impl Global for FocusedInput {}

/// What a masked input paints for each grapheme.
const MASK: char = '•';

/// DESIGN.md `palette-field` padding (`0 6px 0 9px`); fields share it (`TextInput`, `Select`).
pub(crate) const FIELD_PADDING_LEFT_PX: f32 = 9.0;
pub(crate) const FIELD_PADDING_RIGHT_PX: f32 = 6.0;
const CARET_WIDTH_PX: f32 = 1.0;

/// The text line as last painted: what `bounds_for_range` and hit-testing measure against.
struct InputLayout {
    line: ShapedLine,
    /// Top-left of the line (scroll applied).
    origin: Point<Pixels>,
    line_height: Pixels,
    /// For a masked input: maps value offsets to and from the painted bullets.
    mask: Option<Mask>,
}

/// The grapheme boundaries of a masked value: grapheme `g` starts at `bounds[g]` in the value and
/// at `g * MASK.len_utf8()` in the painted bullets.
#[derive(Clone)]
struct Mask {
    bounds: Vec<usize>,
}

impl Mask {
    fn new(value: &str) -> Self {
        let mut bounds: Vec<usize> = value.grapheme_indices(true).map(|(i, _)| i).collect();
        bounds.push(value.len());
        Mask { bounds }
    }

    /// The bullets, one per grapheme.
    fn text(&self) -> String {
        std::iter::repeat_n(MASK, self.bounds.len() - 1).collect()
    }

    /// A value byte offset as a painted byte offset (inside a grapheme: its start).
    fn to_painted(&self, byte: usize) -> usize {
        let grapheme = self
            .bounds
            .partition_point(|&b| b <= byte)
            .saturating_sub(1);
        grapheme * MASK.len_utf8()
    }

    /// A painted byte offset as a value byte offset.
    fn to_value(&self, byte: usize) -> usize {
        let grapheme = (byte / MASK.len_utf8()).min(self.bounds.len() - 1);
        self.bounds[grapheme]
    }
}

/// A single-line text input, 26 px tall (DESIGN.md `field-height`).
pub struct TextInput {
    focus_handle: FocusHandle,
    ime: ImeState,
    placeholder: TextFn,
    layout: Option<InputLayout>,
    scroll_x: Pixels,
    /// The composition state last reported through `CompositionChanged`.
    reported_composing: bool,
    /// Focus and blur listeners, installed by the first render (they need the window).
    focus_subscriptions: Vec<Subscription>,
    masked: bool,
    /// Draws the field on its own well (`panel` / `ground`).
    well: bool,
    paste_filter: Option<PasteFilter>,
    /// The text of the line last painted (the bullets when masked, the placeholder when empty).
    painted: SharedString,
    // ## m2a-t17-dialog
    /// Where the placeholder text comes from: a Fluent message (default) or data, such as a
    /// suggested name or an example host.
    placeholder_source: TextSource,
}

/// Sees a paste's clipboard text before a non-masked input inserts it; returning `true`
/// consumes the paste (nothing is inserted). A masked input never calls it (P16).
///
/// The filter runs while the input is being updated: it must not read or update this input (it
/// is leased); defer edits with `window.defer`.
pub type PasteFilter = Rc<dyn Fn(&str, &mut Window, &mut App) -> bool>;

impl EventEmitter<InputEvent> for TextInput {}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TextInput {
    /// An empty input. `placeholder` is re-read on every render (F16, F33).
    pub fn new(cx: &mut Context<Self>, placeholder: TextFn) -> Self {
        // An input dropped while focused (a closed palette) gets no blur: clear its entry so
        // the key dispatcher never waits on a composition that no longer exists.
        let me = Some(cx.entity_id());
        cx.on_release(move |_, cx| {
            let global = cx.default_global::<FocusedInput>();
            if global.entity == me {
                *global = FocusedInput::default();
            }
        })
        .detach();
        Self {
            focus_handle: cx.focus_handle(),
            ime: ImeState::new(""),
            placeholder,
            layout: None,
            scroll_x: px(0.0),
            reported_composing: false,
            focus_subscriptions: Vec::new(),
            masked: false,
            well: false,
            paste_filter: None,
            painted: SharedString::default(),
            placeholder_source: TextSource::Fluent,
        }
    }

    // ## m2a-t17-dialog
    /// Marks the placeholder as data (`TextSource::Data`): a suggestion or an example, not a
    /// message (pseudo-locales leave it alone).
    pub fn set_placeholder_source(&mut self, source: TextSource) {
        self.placeholder_source = source;
    }

    /// Paints `•` per grapheme, refuses copy and cut, is announced as a password field with an
    /// empty value, and hands the platform text services (IME, dictation) only bullets;
    /// `value()` is unchanged. Call it inside an `update` and `cx.notify()` to repaint.
    ///
    /// Secure event input (macOS `EnableSecureEventInput`, which hides keystrokes from other
    /// processes while the field is focused) is not enabled yet: deferred to a later security
    /// pass by controller ruling (M2a T6 fix round 1).
    pub fn set_masked(&mut self, masked: bool) {
        self.masked = masked;
    }

    /// Draws the field on its own well (`panel` in dark, `ground` in light), so its text-3
    /// placeholder never sits on a raised surface (DESIGN.md Raised Ink Rule, NOTES cross-cutting
    /// #4). Inputs inside a dialog or any other raised surface must enable it.
    pub fn set_well(&mut self, well: bool) {
        self.well = well;
    }

    pub fn is_masked(&self) -> bool {
        self.masked
    }

    /// Installs (or removes) the paste filter (P16), called on every paste into a non-masked
    /// input.
    pub fn set_paste_filter(&mut self, filter: Option<PasteFilter>) {
        self.paste_filter = filter;
    }

    /// The text of the line last painted: the value, its bullets when masked, or the
    /// placeholder when empty.
    pub fn painted_text(&self) -> &SharedString {
        &self.painted
    }

    pub fn value(&self) -> &str {
        self.ime.text()
    }

    /// Replaces the value and puts the caret at its end; emits nothing. Call it inside an
    /// `update` and `cx.notify()` to repaint. A composition in progress ends; the next render
    /// reports `CompositionChanged(false)`.
    pub fn set_value(&mut self, s: impl Into<SharedString>) {
        self.ime.set_text(&s.into());
    }

    pub fn is_composing(&self) -> bool {
        self.ime.is_composing()
    }

    fn install_focus_listeners(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = cx.on_focus(&self.focus_handle, window, |this, _, cx| {
            let current = FocusedInput {
                entity: Some(cx.entity_id()),
                composing: this.ime.is_composing(),
            };
            *cx.default_global::<FocusedInput>() = current;
            cx.notify();
        });
        // Focus loss mid-composition: a focus move inside the active window commits; an app
        // switch leaves the composition to the platform (`ImeState::blur`). No
        // `notify_text_input_changed` here: the handle is no longer focused at blur time.
        let blur = cx.on_blur(&self.focus_handle, window, |this, window, cx| {
            this.ime.blur(window.is_window_active());
            this.report_composition(cx);
            let me = Some(cx.entity_id());
            let global = cx.default_global::<FocusedInput>();
            if global.entity == me {
                *global = FocusedInput::default();
            }
            cx.notify();
        });
        self.focus_subscriptions = vec![focus, blur];
    }

    /// Emits `CompositionChanged` when the composition started or ended since the last report
    /// and mirrors it into `FocusedInput` while this input is the focused one.
    fn report_composition(&mut self, cx: &mut Context<Self>) {
        let composing = self.ime.is_composing();
        if composing != self.reported_composing {
            self.reported_composing = composing;
            cx.emit(InputEvent::CompositionChanged(composing));
        }
        let me = Some(cx.entity_id());
        let global = cx.default_global::<FocusedInput>();
        if global.entity == me {
            global.composing = composing;
        }
    }

    /// After any edit: `Changed` if the text changed, composition reporting, platform update.
    fn edited(&mut self, before: &str, window: &mut Window, cx: &mut Context<Self>) {
        if before != self.ime.text() {
            cx.emit(InputEvent::Changed);
        }
        self.report_composition(cx);
        window.notify_text_input_changed(&self.focus_handle);
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // While composing, every key belongs to the IME (the platform sends it there first;
        // this guards the platforms that do not).
        if self.ime.is_composing() {
            return;
        }
        let keystroke = &event.keystroke;
        let m = &keystroke.modifiers;
        let before = self.ime.text().to_string();
        let clipboard_chord = m.secondary(cx) && !m.alt && !m.shift;
        match keystroke.key.as_str() {
            "a" if clipboard_chord => self.ime.select_all(),
            // A masked input never puts its value on the clipboard.
            "c" | "x" if clipboard_chord && self.masked => {}
            key @ ("c" | "x") if clipboard_chord => {
                let selection = self.ime.selection();
                if !selection.is_empty() {
                    let text = self.ime.text()[selection].to_string();
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    if key == "x" {
                        self.ime.backspace();
                    }
                }
            }
            "v" if clipboard_chord => {
                cx.stop_propagation();
                if let Some(item) = cx.read_from_clipboard() {
                    self.paste(item, window, cx);
                }
                return;
            }
            // Other chords are commands (or AltGr text, which arrives through the input handler).
            _ if m.platform || m.control || m.function => return,
            "backspace" => {
                self.ime.backspace();
            }
            "left" => self.ime.move_left(),
            "right" => self.ime.move_right(),
            "enter" => cx.emit(InputEvent::Submitted),
            "escape" => cx.emit(InputEvent::Cancelled),
            // Printable text is left to the platform, which delivers it (through the IME when
            // one is active) to `replace_text_in_range`.
            _ => return,
        }
        cx.stop_propagation();
        if before != self.ime.text() {
            self.edited(&before, window, cx);
        } else {
            window.notify_text_selection_changed(&self.focus_handle);
            cx.notify();
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        if self.ime.is_composing() {
            return;
        }
        if let Some(layout) = &self.layout {
            let byte = layout
                .line
                .closest_index_for_x(event.position.x - layout.origin.x);
            let byte = layout.mask.as_ref().map_or(byte, |m| m.to_value(byte));
            self.ime.set_caret(byte);
            window.notify_text_selection_changed(&self.focus_handle);
            cx.notify();
        }
    }

    /// Bounds of a byte range (of the value) on the painted line.
    fn byte_bounds(&self, range: Range<usize>) -> Option<Bounds<Pixels>> {
        let l = self.layout.as_ref()?;
        let painted = |byte| l.mask.as_ref().map_or(byte, |m| m.to_painted(byte));
        let x0 = l.line.x_for_index(painted(range.start));
        let x1 = l.line.x_for_index(painted(range.end)).max(x0);
        Some(Bounds::new(
            point(l.origin.x + x0, l.origin.y),
            size(x1 - x0, l.line_height),
        ))
    }
}

/// Single-line input: line breaks and tabs from a paste become spaces; a text that is only
/// control characters (Tab, Return reaching the handler) inserts nothing.
fn sanitize(text: &str) -> Option<String> {
    if !text.is_empty() && text.chars().all(char::is_control) {
        return None;
    }
    Some(
        text.chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect(),
    )
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let text = self.ime.text_for_range_utf16(range, adjusted_range);
        if self.masked {
            // Platform text services get one bullet per UTF-16 unit: the same length, so the
            // IME's range arithmetic holds, and never the password.
            return Some(std::iter::repeat_n(MASK, text.encode_utf16().count()).collect());
        }
        Some(text)
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.ime.selection_utf16(),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.ime.marked_utf16()
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ime.unmark();
        let text = self.ime.text().to_string();
        self.edited(&text, window, cx);
    }

    /// Every paste (the clipboard chord and the platform's paste) comes through here: a
    /// non-masked input lets its filter consume it first (P16).
    fn paste(&mut self, item: ClipboardItem, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = item.text() else { return };
        if !self.masked
            && let Some(filter) = self.paste_filter.clone()
            && filter(&text, window, cx)
        {
            return;
        }
        self.replace_text_in_range(None, &text, window, cx);
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = sanitize(text) else { return };
        let before = self.ime.text().to_string();
        self.ime.replace(range, &text);
        self.edited(&before, window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = sanitize(new_text) else {
            return;
        };
        let before = self.ime.text().to_string();
        self.ime.replace_and_mark(range, &text, new_selected_range);
        self.edited(&before, window, cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let start = self.ime.utf16_to_byte_at(range_utf16.start);
        let end = self.ime.utf16_to_byte_at(range_utf16.end);
        self.byte_bounds(start..end)
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let layout = self.layout.as_ref()?;
        let byte = layout.line.index_for_x(point.x - layout.origin.x)?;
        let byte = layout.mask.as_ref().map_or(byte, |m| m.to_value(byte));
        Some(self.ime.byte_to_utf16(byte))
    }

    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ime.set_selection_utf16(range_utf16);
        window.notify_text_selection_changed(&self.focus_handle);
        cx.notify();
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.ime.len_utf16())
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.focus_subscriptions.is_empty() {
            self.install_focus_listeners(window, cx);
        }
        // A `set_value` during a composition ended it without a context to report through.
        if self.reported_composing != self.ime.is_composing() {
            self.report_composition(cx);
        }
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let focused = self.focus_handle.is_focused(window);
        // Focus given before the first render (before the listeners existed) is published here.
        // Not while the window is inactive: then the blur listener has cleared the entry on
        // purpose (app switch), and a background re-render must not bring it back.
        let me = Some(cx.entity_id());
        if focused && window.is_window_active() && cx.default_global::<FocusedInput>().entity != me
        {
            *cx.default_global::<FocusedInput>() = FocusedInput {
                entity: me,
                composing: self.ime.is_composing(),
            };
        }
        let placeholder = (self.placeholder)(cx);
        // A masked input exposes no value to assistive technology.
        let value: SharedString = if self.masked {
            SharedString::default()
        } else {
            self.ime.text().to_string().into()
        };
        let field = div()
            .id(("text-input", cx.entity_id()))
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .flex()
            .items_center()
            .h(px(t.spacing.field_height))
            .pl(px(FIELD_PADDING_LEFT_PX))
            .pr(px(FIELD_PADDING_RIGHT_PX))
            .rounded(px(t.radii.sm))
            .border_1()
            .border_color(t.colors.seam_strong.to_hsla())
            .cursor_text()
            .when(self.well, |field| {
                let fill = match theme.mode {
                    ThemeMode::Dark => t.colors.panel,
                    ThemeMode::Light => t.colors.ground,
                };
                field.bg(fill.to_hsla())
            })
            .child(InputText { input: cx.entity() });
        let role = if self.masked {
            Role::PasswordInput
        } else {
            Role::TextInput
        };
        A11yNode::new(
            ("text-input-node", cx.entity_id()),
            role,
            placeholder,
            false,
            focus_outline(field, &theme, focused),
        )
        .value(value)
    }
}

/// The text line, selection and caret of a [`TextInput`]; registers the input handler.
struct InputText {
    input: Entity<TextInput>,
}

struct InputPaint {
    line: ShapedLine,
    origin: Point<Pixels>,
    selection: Option<Bounds<Pixels>>,
    caret: Option<Bounds<Pixels>>,
}

impl Element for InputText {
    type RequestLayoutState = ();
    type PrepaintState = InputPaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let line_height = Theme::get(cx).tokens.type_ramp.body.line_height;
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = px(line_height).into();
        style.min_size.width = px(0.0).into();
        style.flex_grow = 1.0;
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> InputPaint {
        let t = Theme::get(cx).tokens;
        let style = t.type_ramp.body;
        let font = font_for(&style);
        let font_size = px(style.size_px);
        let line_height = px(style.line_height);
        let input = self.input.read(cx);
        // A masked input shapes only its bullets; offsets are mapped through the mask.
        let mask = input.masked.then(|| Mask::new(input.ime.text()));
        let value = match &mask {
            Some(mask) => SharedString::from(mask.text()),
            None => SharedString::from(input.ime.text().to_string()),
        };
        let painted = |byte: usize| mask.as_ref().map_or(byte, |m| m.to_painted(byte));
        let selection = input.ime.selection();
        let selection = painted(selection.start)..painted(selection.end);
        let marked = input.ime.marked().map(|m| painted(m.start)..painted(m.end));
        let focused = input.focus_handle.is_focused(window);
        let placeholder = value.is_empty().then(|| (input.placeholder)(cx));
        let placeholder_source = input.placeholder_source;
        let mut scroll_x = input.scroll_x;

        let (ink, placeholder_ink) = (t.colors.text.to_hsla(), t.colors.text_3.to_hsla());
        let run = |len, color, underline| TextRun {
            len,
            font: font.clone(),
            color,
            background_color: None,
            underline,
            strikethrough: None,
        };
        let underline = Some(UnderlineStyle {
            thickness: px(1.0),
            color: Some(t.colors.text.to_hsla()),
            wavy: false,
        });
        let mut runs = Vec::new();
        match &marked {
            Some(m) => {
                if m.start > 0 {
                    runs.push(run(m.start, ink, None));
                }
                runs.push(run(m.end - m.start, ink, underline));
                if value.len() > m.end {
                    runs.push(run(value.len() - m.end, ink, None));
                }
            }
            None => runs.push(run(value.len(), ink, None)),
        }
        let line = window
            .text_system()
            .shape_line(value.clone(), font_size, &runs, None);

        // Keep the caret in view.
        let width = bounds.size.width;
        let caret_x = line.x_for_index(selection.end);
        if caret_x < scroll_x {
            scroll_x = caret_x;
        } else if caret_x + px(CARET_WIDTH_PX) > scroll_x + width {
            scroll_x = caret_x + px(CARET_WIDTH_PX) - width;
        }
        scroll_x = scroll_x.min((line.width + px(CARET_WIDTH_PX) - width).max(px(0.0)));
        let origin = point(
            bounds.origin.x - scroll_x,
            bounds.origin.y + (bounds.size.height - line_height) / 2.0,
        );

        let shown = match &placeholder {
            Some(p) => {
                let runs = [run(p.len(), placeholder_ink, None)];
                window
                    .text_system()
                    .shape_line(p.clone(), font_size, &runs, None)
            }
            None => line.clone(),
        };
        let x = |byte| origin.x + line.x_for_index(byte);
        let selection_bounds = (!selection.is_empty()).then(|| {
            Bounds::new(
                point(x(selection.start), origin.y),
                size(x(selection.end) - x(selection.start), line_height),
            )
        });
        let caret = (focused && selection.is_empty()).then(|| {
            Bounds::new(
                point(x(selection.end), origin.y),
                size(px(CARET_WIDTH_PX), line_height),
            )
        });

        if let Some(p) = &placeholder {
            TextRegistry::record(cx, || RenderedText {
                text: p.clone(),
                source: placeholder_source,
                size_px: style.size_px,
                painted_px: f32::from(shown.width),
                clipped: shown.width > width + px(0.5),
                truncated: false,
                path: "hacivat-ui/src/input/mod.rs#placeholder".into(),
            });
        }

        let shown_text = shown.text.clone();
        self.input.update(cx, |input, _| {
            input.scroll_x = scroll_x;
            input.painted = shown_text;
            input.layout = Some(InputLayout {
                line,
                origin,
                line_height,
                mask,
            });
        });
        InputPaint {
            line: shown,
            origin,
            selection: selection_bounds,
            caret,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        frame: &mut InputPaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let t = Theme::get(cx).tokens;
        let line_height = px(t.type_ramp.body.line_height);
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(sel) = frame.selection {
                window.paint_quad(fill(sel, t.colors.blue_tint_strong.to_hsla()));
            }
            // Painting only fails when the glyph atlas cannot take a glyph (one frame).
            let _ = frame
                .line
                .paint(frame.origin, line_height, TextAlign::Left, None, window, cx);
            if let Some(caret) = frame.caret {
                window.paint_quad(fill(caret, t.colors.blue.to_hsla()));
            }
        });
    }
}
