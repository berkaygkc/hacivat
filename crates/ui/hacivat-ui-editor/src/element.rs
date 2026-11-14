//! The editor element (UI-EDIT-07 paint, -08 scroll, -09 layout cache; Task 19): one custom
//! element for the text and the gutter area. It paints only the rows in view, shapes each through
//! the content-keyed cache (`layout_cache`), and keeps each frame's layout for
//! `EditorView::last_layout` (IME bounds, hit testing, the chooser's anchor).
//!
//! - `request_layout`: the [`A11yFrame`] (only while assistive technology is active);
//! - the accessibility hooks: `a11y::{role, write_info, synthetic_children}`;
//! - `prepaint` ([`prepaint_frame`]): `ime::on_frame`, the geometry (the gutter grows with the
//!   document's last line number: 58 px for three digits plus one advance per extra digit, NOTES
//!   open question 4), the scroll clamp and the caret reveal, the visible rows (decorations
//!   queried for the visible bytes only, `compose_line`, the shape cache), the hit targets
//!   (the text's I-beam, the scroll area, `marks::prepaint`), the frame's [`FrameLayout`];
//! - `paint` ([`paint_frame`]): the input handler, then the harman order (`paint.rs`): the
//!   panel ground, `gutter::paint`, the line backgrounds (`preview::paint_band`), the selection,
//!   the text runs, the IME underline, `marks::paint`, the carets, `scrollbar::paint`, the
//!   horizontal thumb; the pointer listeners (`mouse.rs`); the probe's `painted`.
//!
//! **Rows.** A row is one buffer line (no wrapping in M3a). Its text and tabs (expanded to
//! spaces at their tab stops) are shaped as one line; a control character is a chip of its own
//! (NOTES "Control chip": Fragment Mono 11 `text_2` on `seam_soft`, its name such as `NUL` or
//! `RLO`, taking its own width), so a row is a short list of pieces. A long line (more than
//! 10,000 columns or 64 KB) is shaped only through a window of 512 columns around the viewport
//! (`layout_cache::column_window`), placed at the window's column × the monospace advance; x
//! outside the window is that estimate.

use std::ops::Range;
use std::sync::Arc;

use hacivat_text::display::{
    Decoration, Fragment, HeightMap, RangeSet, ScrollAnchor, compose_line, control_name,
    is_visible_control,
};
use hacivat_text::units::snap_to_grapheme;
use hacivat_text::{Bias, Snapshot, TextBuffer};
use hacivat_ui_core::gpui_surface::{
    A11ySubtreeBuilder, App, Bounds, Context, Element, ElementId, ElementInputHandler, Entity,
    FocusHandle, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, LayoutId, Pixels,
    Point, Role, ShapedLine, Style, TextRun, UnderlineStyle, Window, accesskit, point, px,
    relative, size,
};
use smallvec::SmallVec;
use unicode_segmentation::UnicodeSegmentation;

use crate::a11y::{self, A11yFrame, A11yState};
use crate::decor::HighlightClass;
use crate::gutter::{self, GutterCtx};
use crate::layout_cache::{LONG_LINE_BYTES, LONG_LINE_COLUMNS, LayoutCache, column_window};
use crate::marks::{self, MarkHitboxes};
use crate::paint;
use crate::preview;
use crate::scroll::{REVEAL_MARGIN_COLS, ScrollState};
use crate::scrollbar::{self, ScrollbarCtx};
use crate::service::DecorationSource;
use crate::theme::{ClassInk, EditorTheme};
use crate::view::EditorView;

/// The painted rows of one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisibleRows {
    /// The text area (right of the gutter), in window coordinates.
    pub bounds: Bounds<Pixels>,
    pub line_height: Pixels,
    pub first_line: u32,
    pub last_line: u32,
    /// The top of `first_line`, in window coordinates.
    pub first_y: Pixels,
}

/// One buffer span of a text piece and where its display text sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Seg {
    /// The buffer byte it starts at.
    pub buf: usize,
    /// The display byte it starts at (in the piece's shaped text).
    pub disp: usize,
    /// Its buffer bytes.
    pub len: usize,
    /// Its display bytes.
    pub disp_len: usize,
    /// A tab, shown as `disp_len` spaces.
    pub tab: bool,
}

/// What a piece of a row is.
#[derive(Debug, Clone)]
pub(crate) enum PieceKind {
    /// Text and tabs, shaped as one line.
    Text {
        shaped: ShapedLine,
        segs: SmallVec<[Seg; 2]>,
    },
    /// A control character's chip and its shaped label.
    Chip { label: ShapedLine },
}

/// One piece of a painted row.
#[derive(Debug, Clone)]
pub(crate) struct Piece {
    /// Its left edge, from the line's start, before horizontal scrolling.
    pub x: f32,
    pub width: f32,
    /// The buffer bytes it shows.
    pub bytes: Range<usize>,
    pub kind: PieceKind,
}

/// One painted row: a buffer line.
#[derive(Debug, Clone)]
pub(crate) struct RowLayout {
    pub line: u32,
    /// Its top, in window coordinates.
    pub top: Pixels,
    /// The line's bytes, without its break.
    pub line_bytes: Range<usize>,
    /// The bytes the pieces cover: the line, or a long line's window.
    pub shaped: Range<usize>,
    /// The line's width (estimated past a long line's window).
    pub width: f32,
    pub pieces: SmallVec<[Piece; 1]>,
}

/// The last painted frame: what IME bounds, hit testing and the chooser's anchor read.
#[derive(Debug, Clone)]
pub struct FrameLayout {
    rows: VisibleRows,
    pub(crate) lines: Vec<RowLayout>,
    /// The frame's text (an O(1) clone).
    pub(crate) buffer: TextBuffer,
    pub(crate) version: u64,
    /// The window x of a line's start (the text area's left + 8 px − `scroll_x`), in `f64`.
    pub(crate) origin_x: f64,
    /// The monospace advance.
    pub(crate) advance: f32,
}

impl FrameLayout {
    /// The rows the frame painted.
    pub fn rows(&self) -> &VisibleRows {
        &self.rows
    }

    /// The text version the frame painted.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The painted row of `line`.
    pub(crate) fn row(&self, line: u32) -> Option<&RowLayout> {
        let first = self.lines.first()?.line;
        let row = self.lines.get(line.checked_sub(first)? as usize)?;
        (row.line == line).then_some(row)
    }

    /// The x of byte `offset` on a painted line, in window coordinates; `None` off the painted
    /// lines (or past this frame's text).
    pub fn x_for_offset(&self, offset: usize) -> Option<Pixels> {
        if offset > self.buffer.len_bytes() {
            return None;
        }
        let line = u32::try_from(self.buffer.byte_to_line(offset)).ok()?;
        let row = self.row(line)?;
        Some(px(
            (self.origin_x + f64::from(self.row_x(row, offset))) as f32
        ))
    }

    /// The bounds of `r` on the row of its start (to that row's end when `r` runs past it), in
    /// window coordinates; `None` when that row is not painted.
    pub fn bounds_for_range(&self, r: Range<usize>) -> Option<Bounds<Pixels>> {
        let start = r.start.min(self.buffer.len_bytes());
        let line = u32::try_from(self.buffer.byte_to_line(start)).ok()?;
        let row = self.row(line)?;
        let x0 = self.row_x(row, start);
        let end = r.end.clamp(start, row.line_bytes.end);
        let x1 = self.row_x(row, end).max(x0);
        Some(Bounds::new(
            point(px((self.origin_x + f64::from(x0)) as f32), row.top),
            size(px(x1 - x0), self.rows.line_height),
        ))
    }

    /// The byte offset at `p`, snapped to a grapheme boundary; `None` off the painted lines.
    pub fn offset_for_point(&self, p: Point<Pixels>) -> Option<usize> {
        let b = self.rows.bounds;
        if p.x < b.left() || p.x > b.right() {
            return None;
        }
        let row = self
            .lines
            .iter()
            .find(|r| p.y >= r.top && p.y < r.top + self.rows.line_height)?;
        Some(self.offset_in_row(row, p.x))
    }

    /// The byte offset nearest to `p`: above the painted rows the first, below them the last
    /// (the document's end when that is the last line), left or right of the text the line's
    /// edge. `None` when nothing was painted.
    pub(crate) fn clamped_offset_for_point(&self, p: Point<Pixels>) -> Option<usize> {
        let first = self.lines.first()?;
        let last = self.lines.last()?;
        if p.y < first.top {
            return Some(self.offset_in_row(first, p.x));
        }
        if p.y >= last.top + self.rows.line_height {
            if last.line as usize + 1 >= self.buffer.len_lines() {
                return Some(self.buffer.len_bytes());
            }
            return Some(self.offset_in_row(last, p.x));
        }
        let row = self
            .lines
            .iter()
            .find(|r| p.y < r.top + self.rows.line_height)
            .unwrap_or(last);
        Some(self.offset_in_row(row, p.x))
    }

    /// The offset nearest to window x `x` on `row`, on a grapheme boundary.
    fn offset_in_row(&self, row: &RowLayout, x: Pixels) -> usize {
        let rel = (f64::from(f32::from(x)) - self.origin_x) as f32;
        let raw = self.row_offset_at(row, rel);
        // The nearer of the two cluster boundaries around it.
        let before = snap_to_grapheme(&self.buffer, raw, Bias::Before);
        let after = snap_to_grapheme(&self.buffer, raw, Bias::After).min(row.line_bytes.end);
        if before == after || before < row.line_bytes.start {
            return after.max(row.line_bytes.start);
        }
        let (xb, xa) = (self.row_x(row, before), self.row_x(row, after));
        if (rel - xb).abs() <= (xa - rel).abs() {
            before
        } else {
            after
        }
    }

    /// The column (UTF-16 units from the line's start) of `offset`, for the estimates.
    fn col_of(&self, row: &RowLayout, offset: usize) -> usize {
        self.buffer.byte_to_utf16(offset) - self.buffer.byte_to_utf16(row.line_bytes.start)
    }

    /// The byte at column `col` (UTF-16 units from the line's start), clamped to the line.
    fn offset_at_col(&self, row: &RowLayout, col: f32) -> usize {
        let start16 = self.buffer.byte_to_utf16(row.line_bytes.start);
        let end16 = self.buffer.byte_to_utf16(row.line_bytes.end);
        let at = (start16 + col.max(0.0).round() as usize).min(end16);
        self.buffer.utf16_to_byte(at)
    }

    /// The x of `offset` from `row`'s line start (before horizontal scrolling).
    pub(crate) fn row_x(&self, row: &RowLayout, offset: usize) -> f32 {
        let offset = offset.clamp(row.line_bytes.start, row.line_bytes.end);
        let outside = (offset < row.shaped.start) || (offset > row.shaped.end);
        if outside || row.pieces.is_empty() {
            if row.shaped.start == row.line_bytes.start && row.shaped.end == row.line_bytes.end {
                return 0.0; // an empty line
            }
            return self.col_of(row, offset) as f32 * self.advance;
        }
        let piece = row
            .pieces
            .iter()
            .find(|p| offset < p.bytes.end)
            .or(row.pieces.last());
        let Some(piece) = piece else {
            return 0.0;
        };
        if offset <= piece.bytes.start {
            return piece.x;
        }
        match &piece.kind {
            PieceKind::Chip { .. } => piece.x + piece.width,
            PieceKind::Text { shaped, segs } => {
                let seg = segs.iter().find(|s| offset < s.buf + s.len).or(segs.last());
                let disp = match seg {
                    Some(s) if s.tab => {
                        if offset <= s.buf {
                            s.disp
                        } else {
                            s.disp + s.disp_len
                        }
                    }
                    Some(s) => s.disp + (offset.min(s.buf + s.len) - s.buf),
                    None => 0,
                };
                piece.x + f32::from(shaped.x_for_index(disp))
            }
        }
    }

    /// The offset at `x` from `row`'s line start (not yet snapped to a cluster).
    fn row_offset_at(&self, row: &RowLayout, x: f32) -> usize {
        let (Some(first), Some(last)) = (row.pieces.first(), row.pieces.last()) else {
            if row.shaped.is_empty() && row.line_bytes.is_empty() {
                return row.line_bytes.start;
            }
            return self.offset_at_col(row, x / self.advance);
        };
        if x < first.x {
            if row.shaped.start > row.line_bytes.start {
                return self.offset_at_col(row, x / self.advance);
            }
            return first.bytes.start;
        }
        if x >= last.x + last.width {
            if row.shaped.end < row.line_bytes.end {
                return self.offset_at_col(row, x / self.advance);
            }
            return last.bytes.end;
        }
        let piece = row
            .pieces
            .iter()
            .find(|p| x < p.x + p.width)
            .unwrap_or(last);
        match &piece.kind {
            PieceKind::Chip { .. } => {
                if x < piece.x + piece.width / 2.0 {
                    piece.bytes.start
                } else {
                    piece.bytes.end
                }
            }
            PieceKind::Text { shaped, segs } => {
                let idx = shaped.closest_index_for_x(px(x - piece.x));
                let seg = segs
                    .iter()
                    .find(|s| idx < s.disp + s.disp_len)
                    .or(segs.last());
                match seg {
                    Some(s) if s.tab => {
                        if (idx.saturating_sub(s.disp)) * 2 < s.disp_len {
                            s.buf
                        } else {
                            s.buf + s.len
                        }
                    }
                    Some(s) => s.buf + idx.saturating_sub(s.disp).min(s.len),
                    None => piece.bytes.start,
                }
            }
        }
    }
}

/// The gutter's base width (three digits) and the scrollbar track's width.
const GUTTER_BASE: f32 = 58.0;
const TRACK_WIDTH: f32 = 8.0;
/// The text starts this far right of the gutter.
pub(crate) const TEXT_INSET: f32 = 8.0;

/// One frame's geometry and prepaint results, from [`prepaint_frame`] to [`paint_frame`].
#[derive(Debug)]
pub struct FramePrepaint {
    pub snapshot: Snapshot,
    pub rows: VisibleRows,
    pub gutter: Bounds<Pixels>,
    pub track: Bounds<Pixels>,
    pub total_lines: u32,
    pub visible_lines: u32,
    pub caret_line: u32,
    pub digits: u8,
    pub current_statement: Option<std::ops::Range<u32>>,
    /// `marks::prepaint`'s hit targets.
    pub mark_hits: MarkHitboxes,
    /// The whole element (wheel scrolling).
    pub(crate) hitbox: Hitbox,
    /// The text area (the I-beam, clicks and drags).
    pub(crate) text_hitbox: Hitbox,
    /// The element has the focus in the active window.
    pub(crate) focused: bool,
    pub(crate) scroll: ScrollAnchor,
    /// The widest painted row, for the horizontal thumb.
    pub(crate) content_width: f64,
    pub(crate) scroll_x: f64,
}

impl FramePrepaint {
    /// The gutter's paint context over this frame.
    fn gutter(&self) -> GutterCtx<'_> {
        GutterCtx {
            bounds: self.gutter,
            line_height: self.rows.line_height,
            first_line: self.rows.first_line,
            last_line: self.rows.last_line,
            digits: self.digits,
            current_line: self.caret_line,
            scroll: self.scroll,
            snapshot: &self.snapshot,
            current_statement: self.current_statement.clone(),
        }
    }
}

/// What building one frame's rows needs.
struct RowBuild<'a> {
    window: &'a Window,
    cache: &'a mut LayoutCache,
    theme: &'a EditorTheme,
    buffer: &'a TextBuffer,
    advance: f32,
    tab_size: u8,
    /// Classes per byte of the line being built (scratch).
    classes: Vec<u8>,
}

/// One line to lay out: its bytes and, for a long line, its window.
#[derive(Debug, Clone)]
struct RowPlan {
    line: u32,
    top: Pixels,
    bytes: Range<usize>,
    /// A long line's window: its columns and its bytes.
    window: Option<(Range<usize>, Range<usize>)>,
}

impl RowPlan {
    fn shaped(&self) -> Range<usize> {
        self.window
            .as_ref()
            .map_or(self.bytes.clone(), |(_, b)| b.clone())
    }
}

/// The plan of `line` at `scroll_x`.
fn plan_row(
    buf: &TextBuffer,
    line: u32,
    top: Pixels,
    scroll_x: f64,
    advance: f32,
    visible_cols: usize,
) -> RowPlan {
    let bytes = buf.line_range(line as usize, false);
    let start16 = buf.byte_to_utf16(bytes.start);
    let cols = buf.byte_to_utf16(bytes.end) - start16;
    let long = bytes.len() > LONG_LINE_BYTES || cols > LONG_LINE_COLUMNS;
    let window = long.then(|| {
        let scroll_col = (scroll_x / f64::from(advance)).max(0.0) as usize;
        let w = column_window(scroll_col, visible_cols, cols);
        let b0 = snap_to_grapheme(buf, buf.utf16_to_byte(start16 + w.start), Bias::Before)
            .max(bytes.start);
        let b1 = snap_to_grapheme(buf, buf.utf16_to_byte(start16 + w.end), Bias::After)
            .clamp(b0, bytes.end);
        (w, b0..b1)
    });
    RowPlan {
        line,
        top,
        bytes,
        window,
    }
}

/// A text piece being built.
#[derive(Default)]
struct TextPiece {
    text: String,
    runs: Vec<TextRun>,
    /// The class of the last run.
    last: Option<u8>,
    segs: SmallVec<[Seg; 2]>,
    start: Option<usize>,
    end: usize,
}

impl TextPiece {
    fn push_run(&mut self, theme: &EditorTheme, class: u8, len: usize) {
        if len == 0 {
            return;
        }
        if self.last == Some(class)
            && let Some(r) = self.runs.last_mut()
        {
            r.len += len;
            return;
        }
        let ink: &ClassInk = theme.ink(HighlightClass::ALL[usize::from(class).min(15)]);
        self.runs.push(TextRun {
            len,
            font: theme.font_for(ink).clone(),
            color: ink.color,
            background_color: ink.background,
            underline: ink.underline.map(|c| UnderlineStyle {
                thickness: px(1.0),
                color: Some(c),
                wavy: false,
            }),
            strikethrough: None,
        });
        self.last = Some(class);
    }
}

impl RowBuild<'_> {
    /// The class of every byte of `range` from the decorations (higher priority wins).
    fn fill_classes(&mut self, range: &Range<usize>, sets: &[(i16, Arc<RangeSet<Decoration>>)]) {
        self.classes.clear();
        self.classes
            .resize(range.len(), HighlightClass::Plain as u8);
        for (_, set) in sets {
            for (r, d) in set.query(range.clone()) {
                let Decoration::Mark { style } = d else {
                    continue;
                };
                let Some(class) = HighlightClass::from_style(*style) else {
                    continue;
                };
                let s = r.start.clamp(range.start, range.end) - range.start;
                let e = r.end.clamp(range.start, range.end) - range.start;
                if e > s {
                    self.classes[s..e].fill(class as u8);
                }
            }
        }
    }

    fn class_at(&self, range_start: usize, byte: usize) -> u8 {
        self.classes
            .get(byte - range_start)
            .copied()
            .unwrap_or(HighlightClass::Plain as u8)
    }

    /// Appends buffer text `bytes` to `piece`, with runs by class.
    fn append_text(&self, piece: &mut TextPiece, range_start: usize, bytes: Range<usize>) {
        if bytes.is_empty() {
            return;
        }
        let text = self.buffer.slice(bytes.clone());
        let disp = piece.text.len();
        let mut run_start = 0;
        let mut run_class = None;
        for (i, _) in text.char_indices() {
            let class = self.class_at(range_start, bytes.start + i);
            if run_class != Some(class) {
                if let Some(c) = run_class {
                    piece.push_run(self.theme, c, i - run_start);
                }
                run_start = i;
                run_class = Some(class);
            }
        }
        if let Some(c) = run_class {
            piece.push_run(self.theme, c, text.len() - run_start);
        }
        piece.text.push_str(&text);
        piece.segs.push(Seg {
            buf: bytes.start,
            disp,
            len: bytes.len(),
            disp_len: text.len(),
            tab: false,
        });
        piece.start.get_or_insert(bytes.start);
        piece.end = bytes.end;
    }

    /// Appends a tab at `byte`, `width` cells wide.
    fn append_tab(&self, piece: &mut TextPiece, range_start: usize, byte: usize, width: u8) {
        let disp = piece.text.len();
        let n = usize::from(width.max(1));
        piece.text.extend(std::iter::repeat_n(' ', n));
        piece.push_run(self.theme, self.class_at(range_start, byte), n);
        piece.segs.push(Seg {
            buf: byte,
            disp,
            len: 1,
            disp_len: n,
            tab: true,
        });
        piece.start.get_or_insert(byte);
        piece.end = byte + 1;
    }

    /// Shapes `piece` (if it has text) into `pieces` at `x`; returns the new `x`.
    fn flush(
        &mut self,
        piece: TextPiece,
        pieces: &mut SmallVec<[Piece; 1]>,
        x: f32,
        window_key: (u32, u32),
    ) -> f32 {
        let Some(start) = piece.start else {
            return x;
        };
        let shaped = self.cache.shape(
            self.window,
            &piece.text,
            &piece.runs,
            self.theme.font_size,
            window_key,
        );
        let width = f32::from(shaped.width());
        pieces.push(Piece {
            x,
            width,
            bytes: start..piece.end,
            kind: PieceKind::Text {
                shaped,
                segs: piece.segs,
            },
        });
        x + width
    }

    /// Pushes the chip of control `c` at `byte` into `pieces` at `x`; returns the new `x`.
    fn chip(&mut self, pieces: &mut SmallVec<[Piece; 1]>, x: f32, byte: usize, c: char) -> f32 {
        let name = control_name(c);
        let run = TextRun {
            len: name.len(),
            font: self.theme.font.clone(),
            color: self.theme.chip_text,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let label = self.cache.shape(
            self.window,
            &name,
            &[run],
            self.theme.chip_font_size,
            (0, 0),
        );
        let width = f32::from(label.width()) + paint::CHIP_PAD * 2.0 + paint::CHIP_MARGIN * 2.0;
        pieces.push(Piece {
            x,
            width,
            bytes: byte..byte + c.len_utf8(),
            kind: PieceKind::Chip { label },
        });
        x + width
    }

    /// The control character at `byte`.
    fn char_at(&self, byte: usize, end: usize) -> Option<char> {
        self.buffer
            .chunks_in(byte..end)
            .next()
            .and_then(|s| s.chars().next())
    }

    /// Lays out `plan`'s row.
    fn build(&mut self, plan: &RowPlan, sets: &[(i16, Arc<RangeSet<Decoration>>)]) -> RowLayout {
        let shaped_range = plan.shaped();
        self.fill_classes(&shaped_range, sets);
        let mut pieces: SmallVec<[Piece; 1]> = SmallVec::new();
        let width = match &plan.window {
            None => {
                let refs: SmallVec<[&RangeSet<Decoration>; 2]> =
                    sets.iter().map(|(_, s)| s.as_ref()).collect();
                let vl = compose_line(self.buffer, plan.line, self.tab_size, &refs);
                let mut x = 0.0;
                let mut piece = TextPiece::default();
                // The byte the next fragment starts at (a tab carries no byte of its own).
                let mut cursor = plan.bytes.start;
                for f in &vl.fragments {
                    match f {
                        Fragment::Text { bytes } => {
                            self.append_text(&mut piece, shaped_range.start, bytes.clone());
                            cursor = bytes.end;
                        }
                        Fragment::Tab { width } => {
                            self.append_tab(&mut piece, shaped_range.start, cursor, *width);
                            cursor += 1;
                        }
                        Fragment::Control { byte, .. } => {
                            x = self.flush(std::mem::take(&mut piece), &mut pieces, x, (0, 0));
                            if let Some(c) = self.char_at(*byte, plan.bytes.end) {
                                x = self.chip(&mut pieces, x, *byte, c);
                                cursor = *byte + c.len_utf8();
                            }
                        }
                        // Folds arrive in M3b; nothing in M3a replaces text.
                        Fragment::Placeholder { .. } => {}
                    }
                }
                self.flush(piece, &mut pieces, x, (0, 0))
            }
            Some((cols, bytes)) => {
                let key = (cols.start as u32, cols.end as u32);
                let mut x = cols.start as f32 * self.advance;
                let mut col = cols.start;
                let tab = usize::from(self.tab_size.max(1));
                let mut piece = TextPiece::default();
                let text = self.buffer.slice(bytes.clone());
                let mut run_start = bytes.start;
                for (i, g) in text.grapheme_indices(true) {
                    let at = bytes.start + i;
                    if !g.chars().any(|c| c == '\t' || is_visible_control(c)) {
                        col += g.encode_utf16().count();
                        continue;
                    }
                    for (j, c) in g.char_indices() {
                        let cat = at + j;
                        if c == '\t' {
                            self.append_text(&mut piece, bytes.start, run_start..cat);
                            let w = tab - col % tab;
                            self.append_tab(&mut piece, bytes.start, cat, w as u8);
                            col += w;
                            run_start = cat + 1;
                        } else if is_visible_control(c) {
                            self.append_text(&mut piece, bytes.start, run_start..cat);
                            x = self.flush(std::mem::take(&mut piece), &mut pieces, x, key);
                            x = self.chip(&mut pieces, x, cat, c);
                            col += 1;
                            run_start = cat + c.len_utf8();
                        } else {
                            col += c.len_utf16();
                        }
                    }
                }
                self.append_text(&mut piece, bytes.start, run_start..bytes.end);
                self.flush(piece, &mut pieces, x, key);
                let cols_total = self.buffer.byte_to_utf16(plan.bytes.end)
                    - self.buffer.byte_to_utf16(plan.bytes.start);
                cols_total as f32 * self.advance
            }
        };
        RowLayout {
            line: plan.line,
            top: plan.top,
            line_bytes: plan.bytes.clone(),
            shaped: shaped_range,
            width,
            pieces,
        }
    }
}

/// The digits of `n` (at least one).
fn digits_of(n: u32) -> u32 {
    n.max(1).ilog10() + 1
}

/// The prepaint half of a frame (module docs).
pub(crate) fn prepaint_frame(
    v: &mut EditorView,
    bounds: Bounds<Pixels>,
    focused: bool,
    window: &mut Window,
    cx: &mut Context<EditorView>,
) -> FramePrepaint {
    crate::ime::on_frame(v, window, cx);
    let theme = EditorTheme::get(cx);
    let snapshot = v.model.snapshot();
    let buf = snapshot.buffer.clone();
    let line_height = theme.line_height;
    let lh = f32::from(line_height);
    let advance = f32::from(v.cache.advance(window, &theme));
    let total_lines = u32::try_from(buf.len_lines()).unwrap_or(u32::MAX);
    let digits = digits_of(total_lines).max(3);
    let gutter_w = GUTTER_BASE + (digits - 3) as f32 * advance;
    let gutter = Bounds::new(bounds.origin, size(px(gutter_w), bounds.size.height));
    let text_area = Bounds::new(
        point(bounds.origin.x + px(gutter_w), bounds.origin.y),
        size(
            (bounds.size.width - px(gutter_w)).max(px(0.0)),
            bounds.size.height,
        ),
    );
    let track = Bounds::new(
        point(
            bounds.origin.x + bounds.size.width - px(TRACK_WIDTH),
            bounds.origin.y,
        ),
        size(px(TRACK_WIDTH), bounds.size.height),
    );
    let map = HeightMap::uniform(total_lines, f64::from(lh));
    let view_h = f64::from(f32::from(text_area.size.height));
    let view_w = f64::from((f32::from(text_area.size.width) - TEXT_INSET).max(0.0));
    let visible_cols = (view_w / f64::from(advance)).ceil().max(1.0) as usize;
    let main = *snapshot.selection.main();
    let caret_line = u32::try_from(buf.byte_to_line(main.head)).unwrap_or(u32::MAX);

    v.scroll.clamp(&map, view_h);
    let reveal = std::mem::take(&mut v.scroll.reveal);
    if reveal {
        v.scroll.reveal_line(&map, caret_line, view_h);
    }

    let sources = v.sources.clone();
    let log = v.model.changelog();
    let mut build = RowBuild {
        window,
        cache: &mut v.cache,
        theme: &theme,
        buffer: &buf,
        advance,
        tab_size: v.config.tab_size,
        classes: Vec::new(),
    };
    let rows_for = |build: &mut RowBuild<'_>, scroll: &ScrollState| -> Vec<RowLayout> {
        let first = scroll.first_line(&map);
        let mut plans = Vec::new();
        let mut line = first;
        while line < total_lines {
            let top = text_area.origin.y + px(scroll.row_offset(&map, line));
            if top >= text_area.bottom() {
                break;
            }
            plans.push(plan_row(
                &buf,
                line,
                top,
                scroll.scroll_x,
                advance,
                visible_cols,
            ));
            line += 1;
        }
        // Decorations for the visible bytes only (a long line at either end only its window).
        let span = match (plans.first(), plans.last()) {
            (Some(a), Some(b)) => a.shaped().start..b.shaped().end,
            _ => 0..0,
        };
        let mut sets: SmallVec<[(i16, Arc<RangeSet<Decoration>>); 2]> = sources
            .iter()
            .map(|s: &std::rc::Rc<dyn DecorationSource>| {
                (s.priority(), s.decorations(&snapshot, log, span.clone()))
            })
            .collect();
        sets.sort_by_key(|(p, _)| *p);
        plans.iter().map(|p| build.build(p, &sets)).collect()
    };
    let mut lines = rows_for(&mut build, &v.scroll);

    let margin = f64::from(REVEAL_MARGIN_COLS * advance);
    let probe_layout = |lines: Vec<RowLayout>, origin_x: f64| FrameLayout {
        rows: VisibleRows {
            bounds: text_area,
            line_height,
            first_line: 0,
            last_line: 0,
            first_y: text_area.origin.y,
        },
        lines,
        buffer: buf.clone(),
        version: snapshot.version,
        origin_x,
        advance,
    };
    let text_left = f64::from(f32::from(text_area.left()) + TEXT_INSET);
    if reveal {
        let layout = probe_layout(lines, text_left - v.scroll.scroll_x);
        let caret_x = layout
            .row(caret_line)
            .map(|row| f64::from(layout.row_x(row, main.head)));
        lines = layout.lines;
        if let Some(x) = caret_x {
            let before = v.scroll.scroll_x;
            v.scroll.reveal_x(x, view_w, margin);
            if v.scroll.scroll_x != before && lines.iter().any(|r| r.shaped != r.line_bytes) {
                // A long line's window follows the new scroll position.
                lines = rows_for(&mut build, &v.scroll);
            }
        }
    }
    let content_width = lines.iter().map(|r| f64::from(r.width)).fold(0.0, f64::max);
    let before = v.scroll.scroll_x;
    v.scroll.clamp_x(content_width, margin, view_w);
    if v.scroll.scroll_x != before && lines.iter().any(|r| r.shaped != r.line_bytes) {
        lines = rows_for(&mut build, &v.scroll);
    }
    let content_width = v.scroll.known_width;

    let first_line = lines.first().map_or(0, |r| r.line);
    let last_line = lines.last().map_or(0, |r| r.line);
    let first_y = lines.first().map_or(text_area.origin.y, |r| r.top);
    let rows = VisibleRows {
        bounds: text_area,
        line_height,
        first_line,
        last_line,
        first_y,
    };
    v.layout = Some(FrameLayout {
        rows,
        lines,
        buffer: buf,
        version: snapshot.version,
        origin_x: text_left - v.scroll.scroll_x,
        advance,
    });

    let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
    let text_hitbox = window.insert_hitbox(text_area, HitboxBehavior::Normal);
    let current_statement = preview::current_statement(&v.band, &snapshot);
    let visible_lines = (view_h / f64::from(lh)).ceil().max(1.0) as u32;
    let mut frame = FramePrepaint {
        snapshot,
        rows,
        gutter,
        track,
        total_lines,
        visible_lines,
        caret_line,
        digits: digits as u8,
        current_statement,
        mark_hits: MarkHitboxes::default(),
        hitbox,
        text_hitbox,
        focused,
        scroll: v.scroll.anchor,
        content_width,
        scroll_x: v.scroll.scroll_x,
    };
    frame.mark_hits = marks::prepaint(&v.marks, &frame.gutter(), window, cx);
    frame
}

/// The paint half of a frame, in the harman order (module docs).
pub(crate) fn paint_frame(
    v: &mut EditorView,
    view: &Entity<EditorView>,
    bounds: Bounds<Pixels>,
    frame: &FramePrepaint,
    window: &mut Window,
    cx: &mut Context<EditorView>,
) {
    let theme = EditorTheme::get(cx);
    paint::background(bounds, &theme, window);
    let g = frame.gutter();
    gutter::paint(&g, window, cx);
    if let Some(layout) = v.layout.as_ref() {
        let marked = v.ime.marked(&v.model);
        paint::text_area(
            layout,
            &frame.snapshot,
            &v.band,
            &theme,
            frame.focused,
            marked,
            window,
            cx,
        );
        marks::paint(&v.marks, &g, layout, &frame.mark_hits, window, cx);
        if frame.focused && v.blink.visible() {
            paint::carets(layout, &frame.snapshot, &theme, window);
        }
    }
    scrollbar::paint(
        &ScrollbarCtx {
            track: frame.track,
            total_lines: frame.total_lines,
            first_line: frame.rows.first_line,
            visible_lines: frame.visible_lines,
            current_statement: frame.current_statement.clone(),
            snapshot: &frame.snapshot,
        },
        &v.marks,
        window,
        cx,
    );
    paint::horizontal_thumb(
        &frame.rows,
        frame.content_width,
        frame.scroll_x,
        &theme,
        window,
    );
    crate::mouse::listen(view, frame, window);
    if let Some(probe) = v.probe.clone() {
        probe.painted(cx);
    }
}

/// The editor's element: the text area, its input handler and its accessibility node.
pub(crate) struct EditorElement {
    view: Entity<EditorView>,
    focus: FocusHandle,
    id: ElementId,
    a11y: A11yState,
    /// Built in `request_layout` only while assistive technology is active.
    a11y_frame: Option<A11yFrame>,
}

impl EditorElement {
    pub(crate) fn new(
        view: Entity<EditorView>,
        focus: FocusHandle,
        id: ElementId,
        a11y: A11yState,
    ) -> Self {
        EditorElement {
            view,
            focus,
            id,
            a11y,
            a11y_frame: None,
        }
    }
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<FramePrepaint>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        self.a11y_frame = if window.is_a11y_active() {
            let focused = self.focus.is_focused(window);
            Some(self.view.update(cx, |v, cx| v.a11y_frame(focused, cx)))
        } else {
            None
        };
        // The rest of the editor's column (the banner sits above it), as `flex_1().w_full()`.
        let mut style = Style {
            flex_grow: 1.0,
            flex_shrink: 1.0,
            flex_basis: relative(0.0).into(),
            ..Style::default()
        };
        style.size.width = relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Option<FramePrepaint> {
        let focused = self.focus.is_focused(window) && window.is_window_active();
        Some(
            self.view
                .update(cx, |v, cx| prepaint_frame(v, bounds, focused, window, cx)),
        )
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        prepaint: &mut Option<FramePrepaint>,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.handle_input(
            &self.focus,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        if let Some(frame) = prepaint.as_ref() {
            let view = self.view.clone();
            self.view
                .update(cx, |v, cx| paint_frame(v, &view, bounds, frame, window, cx));
        }
    }

    fn a11y_role(&self) -> Option<Role> {
        self.a11y_frame
            .as_ref()
            .and_then(|f| a11y::role(&self.a11y, f))
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        if let Some(f) = &self.a11y_frame {
            a11y::write_info(&self.a11y, f, node);
        }
    }

    fn a11y_synthetic_children(
        &mut self,
        _prepaint: &mut Option<FramePrepaint>,
        builder: &mut A11ySubtreeBuilder,
    ) {
        if let Some(f) = &self.a11y_frame {
            a11y::synthetic_children(&self.a11y, f, builder);
        }
    }
}
