//! Movement and selection commands (Task 25; UI-EDIT-05 steps 8–10, UI-EDIT-06).
//!
//! - **Horizontal** moves go through `hacivat_text::units::move_offset`: grapheme clusters (a
//!   CRLF is one), words from the language service's tokens (character classes without them),
//!   smart Home and End, the document's edges. Left and Right on a selection collapse it to
//!   that edge. Each resets the goal column.
//! - **Vertical** moves keep the goal column (`SelectionRange::moved_vertically`): the x comes
//!   from the last painted frame when it shows this text and the line, else from grapheme
//!   columns times the monospace advance (the same scale, so a goal set one way is read the
//!   other: after an edit before the next frame, or for a line outside the view).
//! - **Pages** are the viewport's height less one row; the view scrolls by the same rows, so
//!   the caret keeps its place in it.
//!
//! Every move is a selection-only transaction (`UserEvent::Move`, or `Select` with Shift).

use hacivat_text::units::{grapheme_column, offset_at_column};
use hacivat_text::{Dir, SelectionRange, Snapshot, Unit, UserEvent, move_offset};
use hacivat_ui_core::gpui_surface::{Context, Window, point, px};

use super::{select, words};
use crate::element::FrameLayout;
use crate::view::EditorView;

/// The rows of a page before the first frame is painted.
const DEFAULT_PAGE_ROWS: isize = 24;

fn event(extend: bool) -> UserEvent {
    if extend {
        UserEvent::Select
    } else {
        UserEvent::Move
    }
}

/// A move of `unit` in `dir` (never `Up` or `Down`).
fn horizontal(
    v: &mut EditorView,
    dir: Dir,
    unit: Unit,
    extend: bool,
    cx: &mut Context<EditorView>,
) {
    let now = *v.model.selection().main();
    let head = if !extend && !now.is_empty() && unit == Unit::Grapheme {
        // Left and Right leave a selection at its edge.
        match dir {
            Dir::Back => now.range().start,
            _ => now.range().end,
        }
    } else {
        move_offset(v.model.buffer(), now.head, dir, unit, words(v).as_ref())
    };
    select(v, now.moved_to(head, extend), event(extend), cx);
}

/// Where a line's x positions come from.
struct Geometry<'a> {
    snap: &'a Snapshot,
    /// The last frame, when it painted this text.
    layout: Option<&'a FrameLayout>,
    /// The monospace advance (1 before the first frame: x is then the column).
    advance: f32,
}

impl<'a> Geometry<'a> {
    fn of(v: &'a EditorView, snap: &'a Snapshot) -> Self {
        Geometry {
            snap,
            layout: v.layout.as_ref().filter(|l| l.version() == snap.version),
            advance: v
                .layout
                .as_ref()
                .map(|l| l.advance)
                .filter(|a| *a > 0.0)
                .unwrap_or(1.0),
        }
    }

    /// The x of `at` from its line's start.
    fn x_of(&self, at: usize) -> f32 {
        self.layout
            .and_then(|l| {
                let x = l.x_for_offset(at)?;
                Some((f64::from(f32::from(x)) - l.origin_x) as f32)
            })
            .unwrap_or_else(|| grapheme_column(&self.snap.buffer, at) as f32 * self.advance)
    }

    /// The offset on `line` nearest to `x` from its start.
    fn offset_at(&self, line: usize, x: f32) -> usize {
        let painted = self.layout.and_then(|l| {
            let row = l.row(u32::try_from(line).ok()?)?;
            let p = point(px((l.origin_x + f64::from(x)) as f32), row.top);
            l.clamped_offset_for_point(p)
        });
        painted.unwrap_or_else(|| {
            let col = (x / self.advance).round().max(0.0) as usize;
            offset_at_column(&self.snap.buffer, line, col)
        })
    }
}

/// A move of `lines` buffer lines (negative: up) at the goal column.
fn vertical(v: &mut EditorView, lines: isize, extend: bool, cx: &mut Context<EditorView>) {
    let snap = v.model.snapshot();
    let now = *snap.selection.main();
    let next: SelectionRange = {
        let geo = Geometry::of(v, &snap);
        now.moved_vertically(
            &snap.buffer,
            lines,
            extend,
            |at| geo.x_of(at),
            |line, x| geo.offset_at(line, x),
        )
    };
    select(v, next, event(extend), cx);
}

/// A page up or down: the viewport's rows less one, the view scrolled by the same.
fn page(v: &mut EditorView, down: bool, extend: bool, cx: &mut Context<EditorView>) {
    let (rows, line_height) = v.layout.as_ref().map_or((DEFAULT_PAGE_ROWS, 0.0), |l| {
        let r = l.rows();
        let lh = f32::from(r.line_height);
        let fit = if lh > 0.0 {
            (f32::from(r.bounds.size.height) / lh).floor() as isize
        } else {
            DEFAULT_PAGE_ROWS
        };
        ((fit - 1).max(1), lh)
    });
    let lines = if down { rows } else { -rows };
    v.scroll_by(lines as f64 * f64::from(line_height));
    vertical(v, lines, extend, cx);
}

type Cx<'a> = Context<'a, EditorView>;

pub(super) fn move_left(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    horizontal(v, Dir::Back, Unit::Grapheme, false, cx);
}

pub(super) fn move_right(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    horizontal(v, Dir::Forward, Unit::Grapheme, false, cx);
}

pub(super) fn select_left(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    horizontal(v, Dir::Back, Unit::Grapheme, true, cx);
}

pub(super) fn select_right(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    horizontal(v, Dir::Forward, Unit::Grapheme, true, cx);
}

pub(super) fn move_up(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    vertical(v, -1, false, cx);
}

pub(super) fn move_down(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    vertical(v, 1, false, cx);
}

pub(super) fn select_up(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    vertical(v, -1, true, cx);
}

pub(super) fn select_down(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    vertical(v, 1, true, cx);
}

pub(super) fn move_word_left(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    horizontal(v, Dir::Back, Unit::Word, false, cx);
}

pub(super) fn move_word_right(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    horizontal(v, Dir::Forward, Unit::Word, false, cx);
}

pub(super) fn select_word_left(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    horizontal(v, Dir::Back, Unit::Word, true, cx);
}

pub(super) fn select_word_right(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    horizontal(v, Dir::Forward, Unit::Word, true, cx);
}

pub(super) fn move_line_start(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    horizontal(v, Dir::Back, Unit::Line, false, cx);
}

pub(super) fn move_line_end(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    horizontal(v, Dir::Forward, Unit::Line, false, cx);
}

pub(super) fn select_line_start(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    horizontal(v, Dir::Back, Unit::Line, true, cx);
}

pub(super) fn select_line_end(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    horizontal(v, Dir::Forward, Unit::Line, true, cx);
}

pub(super) fn move_document_start(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    horizontal(v, Dir::Back, Unit::Document, false, cx);
}

pub(super) fn move_document_end(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    horizontal(v, Dir::Forward, Unit::Document, false, cx);
}

pub(super) fn select_document_start(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    horizontal(v, Dir::Back, Unit::Document, true, cx);
}

pub(super) fn select_document_end(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    horizontal(v, Dir::Forward, Unit::Document, true, cx);
}

pub(super) fn page_up(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    page(v, false, false, cx);
}

pub(super) fn page_down(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    page(v, true, false, cx);
}

pub(super) fn select_page_up(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    page(v, false, true, cx);
}

pub(super) fn select_page_down(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    page(v, true, true, cx);
}
