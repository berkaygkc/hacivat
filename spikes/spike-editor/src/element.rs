//! The editor view and its custom element.
//!
//! The element paints only the visible lines. Each line is shaped over a window of at most
//! `MAX_SHAPED_COLUMNS` columns around the viewport, never the whole line, and the shaped
//! result is cached per line under `(line, version_of_line)`, so an edit reshapes only the
//! lines it touched.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::time::Instant;

use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::frames::LatencyProbe;
use hacivat_ui_core::gpui_surface::{
    App, AvailableSpace, Bounds, ContentMask, Context, Element, ElementId, ElementInputHandler,
    Entity, FocusHandle, Focusable, Font, GlobalElementId, InspectorElementId, InteractiveElement,
    IntoElement, KeyDownEvent, LayoutId, ParentElement, Pixels, Point, Render, ScrollEvent,
    ShapedLine, Style, Styled, TextAlign, TextRun, UnderlineStyle, Window, div, fill, point, px,
    relative, size,
};
use hacivat_ui_core::theme::{Colors, Theme};

use crate::buffer::{Buffer, is_identifier_char};
use crate::completion::{self, CompletionItem};
use crate::input::ImeCore;

/// Hard cap on the characters shaped per line (Review Focus 5: a 100k-char line).
pub const MAX_SHAPED_COLUMNS: usize = 512;
/// The window start moves in steps of up to this many columns, so horizontal caret movement
/// reuses the cached shape until the viewport leaves the window.
const WINDOW_STEP: usize = 128;
/// Columns kept between the caret and the viewport edge when revealing it.
const REVEAL_MARGIN: usize = 8;

/// The column range of a line to shape: at most `MAX_SHAPED_COLUMNS` long, covering
/// `scroll_col..scroll_col + visible_cols` whenever that fits in the cap.
pub fn column_window(scroll_col: usize, visible_cols: usize, line_len: usize) -> Range<usize> {
    let step = MAX_SHAPED_COLUMNS
        .saturating_sub(visible_cols)
        .min(WINDOW_STEP);
    let start = scroll_col
        .checked_div(step)
        .map_or(scroll_col, |q| q * step);
    let start = start.min(line_len);
    start..(start + MAX_SHAPED_COLUMNS).min(line_len)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Plain,
    Keyword,
    Identifier,
    Function,
    String,
    Number,
    Comment,
    Operator,
}

/// Spike-only keyword set; a real lexer replaces this in M1.
const KEYWORDS: &[&str] = &[
    "ALL",
    "AND",
    "AS",
    "ASC",
    "BETWEEN",
    "BY",
    "CASE",
    "CREATE",
    "CURRENT_DATE",
    "DELETE",
    "DESC",
    "DISTINCT",
    "ELSE",
    "END",
    "EXISTS",
    "FROM",
    "GROUP",
    "HAVING",
    "IN",
    "INNER",
    "INSERT",
    "INTERVAL",
    "INTO",
    "IS",
    "JOIN",
    "LEFT",
    "LIKE",
    "LIMIT",
    "NOT",
    "NULL",
    "OFFSET",
    "ON",
    "OR",
    "ORDER",
    "OUTER",
    "RIGHT",
    "SELECT",
    "SET",
    "TABLE",
    "THEN",
    "UNION",
    "UPDATE",
    "VALUES",
    "WHEN",
    "WHERE",
    "WITH",
];

fn is_keyword(word: &str) -> bool {
    KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(word))
}

/// One left-to-right scan that splits `text` into contiguous, char-aligned token spans
/// (byte ranges). Whitespace joins the preceding span to keep the run count low.
pub fn highlight(text: &str) -> Vec<(Range<usize>, Token)> {
    let mut spans: Vec<(Range<usize>, Token)> = Vec::new();
    let mut i = 0;
    while let Some(c) = text[i..].chars().next() {
        let start = i;
        let rest = &text[i..];
        let token = if rest.starts_with("--") {
            i = text.len();
            Token::Comment
        } else if let Some(body) = rest.strip_prefix("/*") {
            i = body.find("*/").map_or(text.len(), |e| i + 2 + e + 2);
            Token::Comment
        } else if c == '\'' {
            i = rest[1..].find('\'').map_or(text.len(), |e| i + 1 + e + 1);
            Token::String
        } else if c.is_ascii_digit() {
            i += rest
                .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '.'))
                .unwrap_or(rest.len());
            Token::Number
        } else if is_identifier_char(c) {
            i += rest
                .find(|ch: char| !is_identifier_char(ch))
                .unwrap_or(rest.len());
            if is_keyword(&text[start..i]) {
                Token::Keyword
            } else if text[i..].starts_with('(') {
                Token::Function
            } else {
                Token::Identifier
            }
        } else if c.is_whitespace() {
            i += c.len_utf8();
            Token::Plain
        } else {
            i += c.len_utf8();
            Token::Operator
        };
        match spans.last_mut() {
            Some((last, kind)) if *kind == token || token == Token::Plain => last.end = i,
            _ => spans.push((start..i, token)),
        }
    }
    spans
}

fn token_color(token: Token, colors: &Colors) -> hacivat_ui_core::gpui_surface::Hsla {
    match token {
        Token::Plain | Token::Identifier => colors.syn_identifier,
        Token::Keyword => colors.syn_keyword,
        Token::Function => colors.syn_function,
        Token::String => colors.syn_string,
        Token::Number => colors.syn_number,
        Token::Comment => colors.syn_comment,
        Token::Operator => colors.syn_operator,
    }
    .to_hsla()
}

/// Highlight runs for `text`. `marked` is the byte range of an IME composition inside
/// `text`; it is drawn with an underline, so runs are split at its edges.
fn text_runs(
    text: &str,
    font: &Font,
    colors: &Colors,
    marked: Option<Range<usize>>,
) -> Vec<TextRun> {
    let underline = UnderlineStyle {
        thickness: px(1.),
        color: Some(colors.text.to_hsla()),
        wavy: false,
    };
    let mut runs = Vec::new();
    for (range, token) in highlight(text) {
        let color = token_color(token, colors);
        let mut push = |part: Range<usize>, underlined: bool| {
            if !part.is_empty() {
                runs.push(TextRun {
                    len: part.len(),
                    font: font.clone(),
                    color,
                    background_color: None,
                    underline: underlined.then_some(underline),
                    strikethrough: None,
                });
            }
        };
        match &marked {
            Some(m) if m.start < range.end && m.end > range.start => {
                let (s, e) = (m.start.max(range.start), m.end.min(range.end));
                push(range.start..s, false);
                push(s..e, true);
                push(e..range.end, false);
            }
            _ => push(range, false),
        }
    }
    runs
}

struct CachedLine {
    version: u64,
    columns: Range<usize>,
    /// The composition range within the line (line-relative chars) this shape was made for.
    marked: Option<Range<usize>>,
    shaped: ShapedLine,
}

/// Geometry of the last laid-out frame. `bounds_for_char_range` reads it, and so will the
/// IME handler's `bounds_for_range` (Task 7).
#[derive(Debug, Clone, Copy)]
pub struct EditorLayout {
    pub text_bounds: Bounds<Pixels>,
    pub line_height: Pixels,
    pub char_width: Pixels,
    pub top_line: usize,
    pub scroll_col: usize,
    pub visible_lines: usize,
}

pub struct CompletionState {
    /// The identifier being completed (char indices).
    pub replace: Range<usize>,
    pub items: Vec<CompletionItem>,
    pub selected: usize,
}

/// Where a bench's latency probe takes its frame timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeTarget {
    /// Any frame that paints the editor (typing latency).
    EditorFrame,
    /// A frame that paints the completion popup (completion latency).
    CompletionPopup,
}

#[derive(Clone)]
pub struct BenchProbe {
    pub probe: Rc<RefCell<LatencyProbe>>,
    pub target: ProbeTarget,
    /// When the key handler last marked an input.
    last_mark: Rc<Cell<Option<Instant>>>,
}

impl BenchProbe {
    pub fn new(probe: Rc<RefCell<LatencyProbe>>, target: ProbeTarget) -> Self {
        Self {
            probe,
            target,
            last_mark: Rc::default(),
        }
    }

    pub fn mark_input(&self, at: Instant) {
        self.last_mark.set(Some(at));
        self.probe.borrow_mut().mark_input(at);
    }

    /// Called while painting: records a frame once this frame has been drawn and handed to
    /// the platform (deferred effects flush after `Window::draw` and `present` return).
    ///
    /// GPUI also draws synchronously, without presenting, when a key arrives while the window
    /// is dirty. The deferred callback then runs after that key's handler; such a draw does
    /// not show the new input, so it is skipped and the input pairs with the next frame.
    pub fn record_after_present(&self, cx: &mut App) {
        let painted_at = Instant::now();
        let this = self.clone();
        cx.defer(move |_| {
            if frame_shows_last_input(this.last_mark.get(), painted_at) {
                this.probe.borrow_mut().on_frame(Instant::now());
            }
        });
    }
}

/// A frame painted before the latest input mark cannot show that input.
fn frame_shows_last_input(last_mark: Option<Instant>, painted_at: Instant) -> bool {
    last_mark.is_none_or(|mark| mark <= painted_at)
}

/// Text for keys the platform does not deliver through the input handler.
fn control_key_text(key: &str) -> Option<&'static str> {
    (key == "tab").then_some("\t")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    Identifier,
    Delete,
    Other,
    Move,
}

pub struct EditorView {
    pub buffer: Buffer,
    pub focus_handle: FocusHandle,
    /// Selection and IME composition, char indices; the caret is `ime.selection.end`.
    pub ime: ImeCore,
    pub top_line: usize,
    pub scroll_col: usize,
    pub completion: Option<CompletionState>,
    pub layout: Option<EditorLayout>,
    pub bench: Option<BenchProbe>,
    /// Lines shaped since start (cache misses); the bench reports it.
    pub shaped_lines: u64,
    reveal_caret: bool,
    scroll_remainder: (Pixels, Pixels),
    char_width: Option<Pixels>,
    cache: HashMap<usize, CachedLine>,
}

impl Focusable for EditorView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EditorView {
    pub fn new(buffer: Buffer, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        // Focus loss mid-composition (Review Focus 4): an app switch leaves the composition
        // to the platform IME; a focus move inside the active window commits it.
        // `ImeCore::blur` has the reasoning. No `notify_text_input_changed` here: the handle
        // is no longer focused at blur time, so GPUI would skip it.
        cx.on_blur(&focus_handle, window, |this, window, cx| {
            if this.ime.blur(window.is_window_active()) {
                cx.notify();
            }
        })
        .detach();
        Self {
            buffer,
            focus_handle,
            ime: ImeCore::caret_at(0),
            top_line: 0,
            scroll_col: 0,
            completion: None,
            layout: None,
            bench: None,
            shaped_lines: 0,
            reveal_caret: false,
            scroll_remainder: (px(0.), px(0.)),
            char_width: None,
            cache: HashMap::new(),
        }
    }

    pub fn caret(&self) -> usize {
        self.ime.selection.end
    }

    /// Moves the caret and scrolls so it sits `context_lines` below the top of the view.
    pub fn place_caret(&mut self, char_idx: usize, context_lines: usize) {
        let char_idx = char_idx.min(self.buffer.len_chars());
        self.ime = ImeCore::caret_at(char_idx);
        self.top_line = self
            .buffer
            .char_to_line(char_idx)
            .saturating_sub(context_lines);
        self.reveal_caret = true;
    }

    /// Edits for non-text keys (backspace, enter) and the bench setup. Text input, plain or
    /// composed, goes through the `EntityInputHandler` impl in `input.rs`.
    pub fn replace(&mut self, range: Range<usize>, text: &str) {
        self.buffer.replace(range.clone(), text);
        self.ime = ImeCore::caret_at(range.start + text.chars().count());
        self.reveal_caret = true;
    }

    /// Shared tail of every edit: completion, bench probe mark, repaint. `input_at` is when
    /// the input reached the editor.
    pub fn after_edit(&mut self, edit: EditKind, input_at: Instant, cx: &mut Context<Self>) {
        self.reveal_caret = true;
        self.refresh_completion(edit);
        if let Some(bench) = &self.bench {
            let measured = match bench.target {
                ProbeTarget::EditorFrame => true,
                ProbeTarget::CompletionPopup => {
                    edit == EditKind::Identifier && self.completion.is_some()
                }
            };
            if measured {
                bench.mark_input(input_at);
            }
        }
        cx.notify();
    }

    fn move_caret(&mut self, key: &str) -> bool {
        let caret = self.caret();
        let line = self.buffer.char_to_line(caret);
        let col = caret - self.buffer.line_to_char(line);
        let to_line = |b: &Buffer, l: usize| b.line_to_char(l) + col.min(b.line_len_chars(l));
        let target = match key {
            "left" => caret.saturating_sub(1),
            "right" => (caret + 1).min(self.buffer.len_chars()),
            "up" if line > 0 => to_line(&self.buffer, line - 1),
            "down" if line + 1 < self.buffer.line_count() => to_line(&self.buffer, line + 1),
            "home" => self.buffer.line_to_char(line),
            "end" => self.buffer.line_to_char(line) + self.buffer.line_len_chars(line),
            "up" | "down" => caret,
            _ => return false,
        };
        self.ime = ImeCore::caret_at(target);
        self.reveal_caret = true;
        true
    }

    fn refresh_completion(&mut self, edit: EditKind) {
        let keep_open = match edit {
            EditKind::Identifier => true,
            EditKind::Delete => self.completion.is_some(),
            EditKind::Other | EditKind::Move => false,
        };
        let caret = self.caret();
        let start = self.buffer.identifier_start(caret);
        if !keep_open || start == caret {
            self.completion = None;
            return;
        }
        let prefix: String = (start..caret).map(|i| self.buffer.char(i)).collect();
        let items = completion::complete(&prefix);
        self.completion = (!items.is_empty()).then_some(CompletionState {
            replace: start..caret,
            items,
            selected: 0,
        });
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let input_at = Instant::now();
        let keystroke = &event.keystroke;
        // While composing, every key belongs to the IME (the platform sends it there first;
        // this guards the platforms that do not).
        if keystroke.modifiers.platform || keystroke.modifiers.control || self.ime.marked.is_some()
        {
            return;
        }
        let caret = self.caret();
        let edit = match keystroke.key.as_str() {
            "backspace" => {
                if caret > 0 {
                    self.replace(caret - 1..caret, "");
                }
                EditKind::Delete
            }
            "enter" => {
                self.replace(caret..caret, "\n");
                EditKind::Other
            }
            // Tab is not delivered through `insertText` on macOS, so insert it here.
            key if control_key_text(key).is_some() => {
                let text = control_key_text(key).unwrap_or_default();
                self.replace(self.ime.selection.clone(), text);
                EditKind::Other
            }
            "escape" => EditKind::Move,
            key if self.move_caret(key) => EditKind::Move,
            // Printable text is not handled here: leaving the event unhandled lets the
            // platform deliver it (through the IME when one is active) to
            // `replace_text_in_range`, the one text input path.
            _ => return,
        };
        self.after_edit(edit, input_at, cx);
        cx.stop_propagation();
    }

    fn on_scroll(&mut self, event: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(layout) = self.layout else { return };
        let delta = event.delta.pixel_delta(layout.line_height);
        let (mut dx, mut dy) = self.scroll_remainder;
        dx -= delta.x;
        dy -= delta.y;
        let lines = (dy / layout.line_height).trunc();
        let cols = (dx / layout.char_width).trunc();
        self.scroll_remainder = (
            dx - layout.char_width * cols,
            dy - layout.line_height * lines,
        );
        let max_line = self.buffer.line_count().saturating_sub(1) as f32;
        self.top_line = (self.top_line as f32 + lines).clamp(0., max_line) as usize;
        self.scroll_col = (self.scroll_col as f32 + cols).max(0.) as usize;
        cx.notify();
    }

    fn reveal(&mut self, visible_lines: usize, visible_cols: usize) {
        let caret = self.caret();
        let line = self.buffer.char_to_line(caret);
        let col = caret - self.buffer.line_to_char(line);
        let full_lines = visible_lines.saturating_sub(1).max(1);
        if line < self.top_line {
            self.top_line = line;
        } else if line >= self.top_line + full_lines {
            self.top_line = line + 1 - full_lines;
        }
        let margin = REVEAL_MARGIN.min(visible_cols / 4);
        if col < self.scroll_col + margin {
            self.scroll_col = col.saturating_sub(margin);
        } else if col + margin >= self.scroll_col + visible_cols {
            self.scroll_col = col + margin + 1 - visible_cols;
        }
    }

    /// Lays out the visible lines, reshaping only cache misses, and returns what to paint.
    fn layout_frame(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> FramePaint {
        let tokens = Theme::get(cx).tokens;
        let colors = tokens.colors;
        let code = tokens.type_ramp.code;
        let font = font_for(&code);
        let font_size = px(code.size_px);
        let line_height = px(tokens.spacing.code_line_height);
        let plain_run = |len: usize, color| TextRun {
            len,
            font: font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let char_width = *self.char_width.get_or_insert_with(|| {
            let probe = "0".repeat(64);
            let run = plain_run(probe.len(), colors.text.to_hsla());
            window
                .text_system()
                .shape_line(probe.into(), font_size, &[run], None)
                .width()
                / 64.
        });

        let gutter_width = px(tokens.spacing.editor_gutter_width);
        let text_left = bounds.left() + gutter_width + px(tokens.spacing.lg);
        let text_bounds =
            Bounds::from_corners(point(text_left, bounds.top()), bounds.bottom_right());
        let visible_lines = ((bounds.size.height / line_height).ceil() as usize).max(1);
        let visible_cols = ((text_bounds.size.width / char_width).floor() as usize).max(1);
        if std::mem::take(&mut self.reveal_caret) {
            self.reveal(visible_lines, visible_cols);
        }
        let layout = EditorLayout {
            text_bounds,
            line_height,
            char_width,
            top_line: self.top_line,
            scroll_col: self.scroll_col,
            visible_lines,
        };
        self.layout = Some(layout);

        let first = self.top_line.min(self.buffer.line_count());
        let last = (first + visible_lines).min(self.buffer.line_count());
        let mut rows = Vec::with_capacity(last - first);
        let mut gutter = Vec::with_capacity(last - first);
        for line in first..last {
            let version = self.buffer.version_of_line(line);
            let columns = column_window(
                self.scroll_col,
                visible_cols,
                self.buffer.line_len_chars(line),
            );
            let marked = self.marked_columns(line);
            let hit = self.cache.get(&line).is_some_and(|c| {
                c.version == version && c.columns == columns && c.marked == marked
            });
            if !hit {
                let text = self
                    .buffer
                    .line_slice(line)
                    .slice(columns.clone())
                    .to_string();
                let marked_bytes = marked.as_ref().and_then(|m| {
                    let (s, e) = (m.start.max(columns.start), m.end.min(columns.end));
                    let byte = |col: usize| {
                        text.char_indices()
                            .nth(col - columns.start)
                            .map_or(text.len(), |(b, _)| b)
                    };
                    (s < e).then(|| byte(s)..byte(e))
                });
                let runs = text_runs(&text, &font, &colors, marked_bytes);
                let shaped = window
                    .text_system()
                    .shape_line(text.into(), font_size, &runs, None);
                self.shaped_lines += 1;
                self.cache.insert(
                    line,
                    CachedLine {
                        version,
                        columns,
                        marked,
                        shaped,
                    },
                );
            }
            let y = bounds.top() + line_height * (line - first) as f32;
            if let Some(cached) = self.cache.get(&line) {
                let x =
                    text_left + char_width * (cached.columns.start as f32 - self.scroll_col as f32);
                rows.push((point(x, y), cached.shaped.clone()));
            }

            let number = (line + 1).to_string();
            let run = plain_run(number.len(), colors.text_3.to_hsla());
            let shaped = window
                .text_system()
                .shape_line(number.into(), font_size, &[run], None);
            let x = bounds.left() + gutter_width - px(tokens.spacing.lg) - shaped.width();
            gutter.push((point(x, y), shaped));
        }
        self.cache.retain(|line, _| (first..last).contains(line));

        let caret = self
            .bounds_for_char_range(self.caret()..self.caret())
            .map(|b| Bounds::new(b.origin, size(px(2.), b.size.height)));

        if let Some(state) = &self.completion
            && let Some(anchor) = self.bounds_for_char_range(state.replace.clone())
        {
            let probe = self
                .bench
                .as_ref()
                .filter(|b| b.target == ProbeTarget::CompletionPopup)
                .cloned();
            let mut popup = completion::popup(&state.items, state.selected, probe, tokens);
            // `defer_draw` takes an element that has already been laid out.
            let popup_size = popup.layout_as_root(AvailableSpace::min_size(), window, cx);
            // Like harman, the popup opens above the line; it flips below near the top edge.
            let gap = px(tokens.spacing.s_2xs);
            let above = anchor.top() - popup_size.height - gap;
            let y = if above >= bounds.top() {
                above
            } else {
                anchor.bottom() + gap
            };
            let origin = point(anchor.left() - px(tokens.spacing.xs), y);
            window.defer_draw(popup, origin, 1, None);
        }

        FramePaint {
            text_bounds,
            gutter_width,
            rows,
            gutter,
            caret,
            line_height,
        }
    }

    /// The part of the IME composition on `line`, as line-relative columns.
    fn marked_columns(&self, line: usize) -> Option<Range<usize>> {
        let marked = self.ime.marked.as_ref()?;
        let line_start = self.buffer.line_to_char(line);
        let line_end = line_start + self.buffer.line_len_chars(line);
        let (s, e) = (marked.start.max(line_start), marked.end.min(line_end));
        (s < e).then(|| s - line_start..e - line_start)
    }

    /// Monospace estimate of a char range's bounds from the last layout, for when the cached
    /// line shape is stale (an IME query between an edit and the next frame).
    pub fn estimated_bounds_for_char_range(&self, range: Range<usize>) -> Option<Bounds<Pixels>> {
        let layout = self.layout?;
        let line = self.buffer.char_to_line(range.start);
        let row = line.checked_sub(layout.top_line)?;
        if row >= layout.visible_lines {
            return None;
        }
        let line_start = self.buffer.line_to_char(line);
        let line_end = line_start + self.buffer.line_len_chars(line);
        let x_for = |char_idx: usize| {
            let col = char_idx.min(line_end) - line_start;
            layout.text_bounds.left() + layout.char_width * (col as f32 - layout.scroll_col as f32)
        };
        let top = layout.text_bounds.top() + layout.line_height * row as f32;
        Some(Bounds::from_corners(
            point(x_for(range.start), top),
            point(x_for(range.end), top + layout.line_height),
        ))
    }

    /// The char index nearest to a window point, from the cached line layout.
    pub fn char_at_point(&self, p: Point<Pixels>) -> Option<usize> {
        let layout = self.layout?;
        if !layout.text_bounds.contains(&p) {
            return None;
        }
        let row = ((p.y - layout.text_bounds.top()) / layout.line_height).floor() as usize;
        let line = layout.top_line + row;
        if line >= self.buffer.line_count() {
            return None;
        }
        let line_len = self.buffer.line_len_chars(line);
        let col = match self.cache.get(&line) {
            Some(cached) if cached.version == self.buffer.version_of_line(line) => {
                let origin_x = layout.text_bounds.left()
                    + layout.char_width * (cached.columns.start as f32 - layout.scroll_col as f32);
                let byte = cached.shaped.closest_index_for_x(p.x - origin_x);
                cached.columns.start + cached.shaped.text[..byte].chars().count()
            }
            _ => {
                let cols = (p.x - layout.text_bounds.left()) / layout.char_width;
                layout.scroll_col + cols.round().max(0.) as usize
            }
        };
        Some(self.buffer.line_to_char(line) + col.min(line_len))
    }

    /// Screen bounds of a char range on one line, from the cached line layout. `None` if the
    /// line is not laid out (scrolled away or edited since the last frame).
    pub fn bounds_for_char_range(&self, range: Range<usize>) -> Option<Bounds<Pixels>> {
        let layout = self.layout?;
        let line = self.buffer.char_to_line(range.start);
        let row = line.checked_sub(layout.top_line)?;
        let cached = self.cache.get(&line)?;
        if row >= layout.visible_lines || cached.version != self.buffer.version_of_line(line) {
            return None;
        }
        let line_start = self.buffer.line_to_char(line);
        let line_end = line_start + self.buffer.line_len_chars(line);
        let origin_x = layout.text_bounds.left()
            + layout.char_width * (cached.columns.start as f32 - layout.scroll_col as f32);
        let x_for = |char_idx: usize| {
            let col = char_idx.min(line_end) - line_start;
            if col < cached.columns.start || col > cached.columns.end {
                // Outside the shaped window: monospace extrapolation.
                return origin_x + layout.char_width * (col as f32 - cached.columns.start as f32);
            }
            let text = &cached.shaped.text;
            let byte = text
                .char_indices()
                .nth(col - cached.columns.start)
                .map_or(text.len(), |(b, _)| b);
            origin_x + cached.shaped.x_for_index(byte)
        };
        let top = layout.text_bounds.top() + layout.line_height * row as f32;
        Some(Bounds::from_corners(
            point(x_for(range.start), top),
            point(x_for(range.end), top + layout.line_height),
        ))
    }
}

pub struct FramePaint {
    text_bounds: Bounds<Pixels>,
    gutter_width: Pixels,
    rows: Vec<(hacivat_ui_core::gpui_surface::Point<Pixels>, ShapedLine)>,
    gutter: Vec<(hacivat_ui_core::gpui_surface::Point<Pixels>, ShapedLine)>,
    caret: Option<Bounds<Pixels>>,
    line_height: Pixels,
}

impl Render for EditorView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_scroll(cx.listener(Self::on_scroll))
            .child(EditorElement { view: cx.entity() })
    }
}

pub struct EditorElement {
    view: Entity<EditorView>,
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = FramePaint;

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
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.view
            .update(cx, |view, cx| view.layout_frame(bounds, window, cx))
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        frame: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let colors = Theme::get(cx).tokens.colors;
        let focus_handle = self.view.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        window.paint_quad(fill(bounds, colors.ground.to_hsla()));
        let gutter = Bounds::new(bounds.origin, size(frame.gutter_width, bounds.size.height));
        window.paint_quad(fill(gutter, colors.gutter.to_hsla()));
        let seam = Bounds::new(
            point(gutter.right() - px(1.), bounds.top()),
            size(px(1.), bounds.size.height),
        );
        window.paint_quad(fill(seam, colors.seam.to_hsla()));

        let line_height = frame.line_height;
        for (origin, line) in &frame.gutter {
            if let Err(err) = line.paint(*origin, line_height, TextAlign::Left, None, window, cx) {
                eprintln!("gutter paint failed: {err:#}");
            }
        }
        let mask = ContentMask {
            bounds: frame.text_bounds,
        };
        window.with_content_mask(Some(mask), |window| {
            for (origin, line) in &frame.rows {
                if let Err(err) =
                    line.paint(*origin, line_height, TextAlign::Left, None, window, cx)
                {
                    eprintln!("line paint failed: {err:#}");
                }
            }
            if let Some(caret) = frame.caret {
                window.paint_quad(fill(caret, colors.blue.to_hsla()));
            }
        });

        let view = self.view.read(cx);
        if let Some(bench) = view.bench.clone()
            && bench.target == ProbeTarget::EditorFrame
        {
            bench.record_after_present(cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_inserts_a_tab_character() {
        assert_eq!(control_key_text("tab"), Some("\t"));
        assert_eq!(control_key_text("a"), None);
        assert_eq!(control_key_text("enter"), None);
    }

    #[test]
    fn column_window_never_shapes_more_than_512_and_covers_viewport() {
        let w = column_window(50_000, 200, 100_000);
        assert!(w.len() <= MAX_SHAPED_COLUMNS);
        assert!(w.start <= 50_000 && w.end >= 50_200, "{w:?}");
        assert_eq!(column_window(0, 150, 40), 0..40);
        assert_eq!(column_window(300, 150, 40), 40..40);
    }

    #[test]
    fn column_window_is_stable_while_the_viewport_stays_inside() {
        assert_eq!(
            column_window(50_050, 200, 100_000),
            column_window(50_100, 200, 100_000)
        );
        let wide = column_window(1_000, 700, 100_000);
        assert_eq!(wide, 1_000..1_512);
    }

    #[test]
    fn frames_painted_before_the_latest_mark_are_not_counted() {
        let t0 = Instant::now();
        let t1 = t0 + std::time::Duration::from_millis(5);
        assert!(frame_shows_last_input(Some(t0), t1));
        assert!(!frame_shows_last_input(Some(t1), t0));
        assert!(frame_shows_last_input(None, t0));
    }

    #[test]
    fn highlight_keywords_ignore_ascii_case() {
        let text = "select a.id FROM b -- note";
        let spans = highlight(text);
        let kind_of = |s: &str| {
            let at = text.find(s).unwrap();
            spans.iter().find(|(r, _)| r.start == at).map(|(_, k)| *k)
        };
        assert_eq!(kind_of("select"), Some(Token::Keyword));
        assert_eq!(kind_of("FROM"), Some(Token::Keyword));
        assert_eq!(kind_of("-- note"), Some(Token::Comment));
        assert_eq!(kind_of("id"), Some(Token::Identifier));
    }

    #[test]
    fn highlight_covers_text_on_char_boundaries() {
        let text = "WHERE c.city = 'İstanbul' AND n > 42";
        let spans = highlight(text);
        assert_eq!(spans.first().unwrap().0.start, 0);
        assert_eq!(spans.last().unwrap().0.end, text.len());
        for pair in spans.windows(2) {
            assert_eq!(pair[0].0.end, pair[1].0.start);
        }
        assert!(spans.iter().all(|(r, _)| text.is_char_boundary(r.start)));
        let string = spans.iter().find(|(_, k)| *k == Token::String).unwrap();
        // Trailing whitespace joins the preceding span.
        assert_eq!(text[string.0.clone()].trim_end(), "'İstanbul'");
        assert!(
            spans
                .iter()
                .any(|(r, k)| *k == Token::Number && &text[r.clone()] == "42")
        );
    }
}
