//! Line commands (M3b Task 34): toggle comment, duplicate, delete and move lines, normalise
//! line endings, and history recall (`editor.history_prev`/`_next`).
//!
//! ## m3b-t01b-seams: the rows are registered (`commands/mod.rs`) and every body is a no-op
//! until Task 34 fills it. A no-op never edits, so a stub can never change the text or what Run
//! sends.
//!
//! ## m3c-t34-editor: the bodies (NOTES surface 17). Each command is one transaction, so one
//! undo group, at every caret (T31 carry); a read-only editor refuses each (`super::change`).
//!
//! - **Comment or uncomment** (⌘/) uses the dialect's tokens (`LanguageService::comment_tokens`,
//!   `--` and `/* */` by default). When every range is a part of one line (not the whole of
//!   its text) the parts are wrapped in a block comment, or unwrapped when each already is one.
//!   Otherwise the lines the ranges touch are commented at their least indentation (`-- `, a
//!   blank line stays blank), or uncommented when every line with text already is (the token
//!   and one space after it go).
//! - **Duplicate line** (⌘D) puts a copy of the touched lines below them; the ranges move onto
//!   the copy. A folded header duplicates its fold's lines with it.
//! - **Delete line** (⇧⌘K) deletes the touched lines with their break (the last line takes the
//!   break before it); the caret keeps its column on the line that takes their place. Deleting
//!   a folded header unfolds its fold (`FoldSet::map` drops a fold whose header went).
//! - **Move line up/down** (⌥⇧↑/↓) swaps the touched lines with the line above or below as one
//!   edit; neighbouring blocks move together. A fold moves with its lines and a folded
//!   neighbour is stepped over whole: the folds the edit covers are re-folded where their text
//!   went (T32 carry: a delete-and-insert drops them otherwise).
//! - **Line endings.** The fan-out tells the host when the text turns mixed or uniform
//!   (`EditorEvent::LineEndingsChanged`, [`after_commit`]); `editor.normalize_line_endings`
//!   rewrites every break to the dominant ending in one undo group, and
//!   [`EditorView::normalize_line_endings_to`] to a chosen one (the status bar's menu).
//! - **History recall** (⌃⌥↑ / ⌃⌥↓) at the last line of an empty or recalled editor asks the
//!   host (`EditorEvent::HistoryRecall`), which owns the history; the host answers with
//!   [`EditorView::show_recalled`] (one transaction, said in the status bar).

use std::ops::Range;
use std::sync::Arc;

use hacivat_text::{
    Bias, ChangeSet, LineEnding, LineEndingStats, Origin, SelectionRange, SelectionSet, TextBuffer,
    Transaction, UserEvent,
};
use hacivat_ui_core::gpui_surface::{Context, SharedString, Window};

use super::{change, say};
use crate::service::CommentTokens;
use crate::view::{EditorEvent, EditorView};

type Cx<'a> = Context<'a, EditorView>;

/// The module's state in `CommandState` (P11).
#[derive(Debug, Default)]
pub struct LineState {
    /// The text's line endings were mixed at the last commit the fan-out saw.
    pub(crate) mixed: bool,
    /// The text version a history recall produced: while the text is still it, a recall goes
    /// on from there.
    pub(crate) recalled: Option<u64>,
}

/// Whether `s` counts more than one kind of line ending.
fn is_mixed(s: LineEndingStats) -> bool {
    [s.lf, s.crlf, s.cr].iter().filter(|n| **n > 0).count() > 1
}

/// The editor opened over its first text: the line endings' state starts from it.
pub(crate) fn opened(v: &mut EditorView) {
    v.commands.lines.mixed = is_mixed(v.model.buffer().meta().line_endings);
}

/// Every commit (the fan-out): the host hears when the line endings turned mixed or uniform.
pub(crate) fn after_commit(v: &mut EditorView, text_changed: bool, cx: &mut Cx<'_>) {
    if !text_changed {
        return;
    }
    let mixed = is_mixed(v.model.buffer().meta().line_endings);
    if mixed != v.commands.lines.mixed {
        v.commands.lines.mixed = mixed;
        cx.emit(EditorEvent::LineEndingsChanged { mixed });
    }
}

// ---- lines and blocks ----------------------------------------------------------------------

/// The lines a range touches (`first..=last`): one that ends at a line's start does not touch
/// that line.
fn lines_of(buf: &TextBuffer, r: Range<usize>) -> (usize, usize) {
    let first = buf.byte_to_line(r.start);
    let mut last = buf.byte_to_line(r.end);
    if last > first && buf.line_to_byte(last) == r.end {
        last -= 1;
    }
    (first, last)
}

/// A run of lines (`first..=last`) and the ranges of the selection on it (their indices).
#[derive(Debug, Clone)]
struct Block {
    first: usize,
    last: usize,
    members: Vec<usize>,
}

/// The selection's ranges grouped by the lines they touch, in order: ranges whose lines
/// overlap share a block, and so do ranges on neighbouring lines when `touching`.
/// `extend(first, last)` widens a block (over the folds it ends in).
fn blocks(
    v: &EditorView,
    touching: bool,
    extend: impl Fn(usize, usize) -> (usize, usize),
) -> Vec<Block> {
    let buf = v.model.buffer();
    let mut out: Vec<Block> = Vec::new();
    for (i, r) in v.model.selection().ranges().iter().enumerate() {
        let (first, last) = lines_of(buf, r.range());
        let (first, last) = extend(first, last);
        match out.last_mut() {
            Some(b) if first <= b.last || (touching && first == b.last + 1) => {
                b.last = b.last.max(last);
                b.members.push(i);
            }
            _ => out.push(Block {
                first,
                last,
                members: vec![i],
            }),
        }
    }
    out
}

/// The height map's view of the folds: whether a line is hidden, and its fold's header.
struct Hidden(hacivat_text::display::HeightMap);

impl Hidden {
    fn of(v: &mut EditorView) -> Self {
        let lines = u32::try_from(v.model.buffer().len_lines()).unwrap_or(u32::MAX);
        Hidden(crate::fold_gutter::height_map(v, lines, 1.0))
    }

    fn is_hidden(&self, line: usize) -> bool {
        u32::try_from(line).is_ok_and(|l| self.0.is_hidden(l))
    }

    /// The header of the fold hiding `line` (`line` itself when it shows).
    fn header(&self, line: usize) -> usize {
        u32::try_from(line).map_or(line, |l| self.0.visible_line(l) as usize)
    }

    /// The last line of the row `line` shows on (a folded header: its fold's last line).
    fn row_end(&self, line: usize, lines: usize) -> usize {
        let mut last = line;
        while last + 1 < lines && self.is_hidden(last + 1) {
            last += 1;
        }
        last
    }
}

/// The byte where line `l`'s content ends (its break's start).
fn content_end(buf: &TextBuffer, l: usize) -> usize {
    buf.line_range(l, false).end
}

/// The selection `ranges` with each position through `f`, the main one kept.
fn mapped(v: &EditorView, mut f: impl FnMut(usize, usize) -> usize) -> SelectionSet {
    let sel = v.model.selection();
    let ranges: Vec<SelectionRange> = sel
        .ranges()
        .iter()
        .enumerate()
        .map(|(i, r)| SelectionRange::new(f(i, r.anchor), f(i, r.head)))
        .collect();
    SelectionSet::normalized(ranges, sel.main_index())
}

// ---- comment or uncomment ------------------------------------------------------------------

/// The dialect's comment tokens.
fn tokens(v: &EditorView) -> CommentTokens {
    v.service.comment_tokens(v.doc)
}

/// Whether range `r` is a part of one line: not empty, on one line, and not all of its text.
fn partial(buf: &TextBuffer, r: &Range<usize>) -> bool {
    if r.is_empty() || buf.byte_to_line(r.start) != buf.byte_to_line(r.end) {
        return false;
    }
    let line = buf.line_range(buf.byte_to_line(r.start), false);
    let text = buf.slice(line.clone());
    let lead = text.len() - text.trim_start().len();
    let body = line.start + lead..line.start + text.trim_end().len();
    !(r.start <= body.start && body.end <= r.end)
}

pub(super) fn toggle_comment(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let buf = v.model.buffer().clone();
    let tok = tokens(v);
    let ranges: Vec<Range<usize>> = v
        .model
        .selection()
        .ranges()
        .iter()
        .map(SelectionRange::range)
        .collect();
    if ranges.iter().all(|r| partial(&buf, r)) {
        block_comment(v, &buf, &ranges, tok, cx);
    } else {
        line_comment(v, &buf, tok, cx);
    }
}

/// Wraps each range in the block comment, or unwraps them when each is one.
fn block_comment(
    v: &mut EditorView,
    buf: &TextBuffer,
    ranges: &[Range<usize>],
    tok: CommentTokens,
    cx: &mut Cx<'_>,
) {
    let (open, close) = tok.block;
    let wrapped = |r: &Range<usize>| -> Option<(Range<usize>, Range<usize>)> {
        let s = buf.slice(r.clone());
        if s.len() < open.len() + close.len() || !s.starts_with(open) || !s.ends_with(close) {
            return None;
        }
        let mut head = open.len();
        if s[head..].starts_with(' ') && s.len() > head + close.len() {
            head += 1;
        }
        let mut tail = close.len();
        if s.len() > head + tail && s[..s.len() - tail].ends_with(' ') {
            tail += 1;
        }
        Some((r.start..r.start + head, r.end - tail..r.end))
    };
    let all_wrapped = ranges.iter().all(|r| wrapped(r).is_some());
    let mut edits: Vec<(Range<usize>, Arc<str>)> = Vec::new();
    // Per range: its new start and end.
    let mut placed: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
    let mut delta: isize = 0;
    let (open_text, close_text): (Arc<str>, Arc<str>) = (
        Arc::from(format!("{open} ")),
        Arc::from(format!(" {close}")),
    );
    for r in ranges {
        if all_wrapped {
            let Some((a, b)) = wrapped(r) else {
                continue;
            };
            // Before the range everything moved by `delta`; inside it, by `delta` less the
            // opening token.
            let inner = a.start.saturating_add_signed(delta);
            let inner_end = b.start.saturating_add_signed(delta) - a.len();
            placed.push((inner, inner_end.max(inner)));
            delta -= (a.len() + b.len()) as isize;
            edits.push((a, Arc::from("")));
            edits.push((b, Arc::from("")));
        } else {
            let start = r.start.saturating_add_signed(delta) + open_text.len();
            placed.push((start, start + r.len()));
            delta += (open_text.len() + close_text.len()) as isize;
            edits.push((r.start..r.start, open_text.clone()));
            edits.push((r.end..r.end, close_text.clone()));
        }
    }
    let sel = v.model.selection();
    let after = SelectionSet::normalized(
        sel.ranges().iter().zip(&placed).map(|(r, &(s, e))| {
            if r.is_reversed() {
                SelectionRange::new(e, s)
            } else {
                SelectionRange::new(s, e)
            }
        }),
        sel.main_index(),
    );
    change(
        v,
        edits,
        Some(after),
        UserEvent::Command("toggle_comment"),
        cx,
    );
}

/// Comments the touched lines at their least indentation, or uncomments them.
fn line_comment(v: &mut EditorView, buf: &TextBuffer, tok: CommentTokens, cx: &mut Cx<'_>) {
    let mut lines: Vec<usize> = Vec::new();
    for r in v.model.selection().ranges() {
        let (first, last) = lines_of(buf, r.range());
        for l in first..=last {
            if lines.last().is_none_or(|&p| p < l) {
                lines.push(l);
            }
        }
    }
    // Each line with text: its start and the length of its indentation.
    let texts: Vec<(usize, usize, String)> = lines
        .iter()
        .filter_map(|&l| {
            let r = buf.line_range(l, false);
            let text = buf.slice(r.clone()).into_owned();
            let lead = text.len() - text.trim_start().len();
            (lead < text.len()).then_some((r.start, lead, text))
        })
        .collect();
    if texts.is_empty() {
        return;
    }
    let commented = |t: &str, lead: usize| t[lead..].starts_with(tok.line);
    let uncomment = texts.iter().all(|(_, lead, t)| commented(t, *lead));
    // The positions each edit inserts at or deletes, to map the selection through.
    let edits: Vec<(Range<usize>, Arc<str>)> = if uncomment {
        texts
            .iter()
            .map(|(start, lead, t)| {
                let at = start + lead;
                let mut n = tok.line.len();
                if t[lead + n..].starts_with(' ') {
                    n += 1;
                }
                (at..at + n, Arc::from(""))
            })
            .collect()
    } else {
        let indent = texts.iter().map(|(_, lead, _)| *lead).min().unwrap_or(0);
        let ins: Arc<str> = Arc::from(format!("{} ", tok.line));
        texts
            .iter()
            .map(|(start, _, _)| (start + indent..start + indent, ins.clone()))
            .collect()
    };
    // A position maps after an insertion at it (a caret at the indentation moves past the
    // token), except a selection's start, which keeps the token inside the selection.
    let map = |p: usize, start_of_selection: bool| -> usize {
        let mut out = p as isize;
        for (r, ins) in &edits {
            if r.is_empty() {
                if r.start < p || (r.start == p && !start_of_selection) {
                    out += ins.len() as isize;
                }
            } else if r.end <= p {
                out -= r.len() as isize;
            } else if r.start < p {
                out -= (p - r.start) as isize;
            }
        }
        out.max(0) as usize
    };
    let sel = v.model.selection();
    let after = SelectionSet::normalized(
        sel.ranges().iter().map(|r| {
            let lo = r.range().start;
            let empty = r.is_empty();
            let m = |p: usize| map(p, !empty && p == lo);
            SelectionRange::new(m(r.anchor), m(r.head))
        }),
        sel.main_index(),
    );
    change(
        v,
        edits,
        Some(after),
        UserEvent::Command("toggle_comment"),
        cx,
    );
}

// ---- duplicate and delete ------------------------------------------------------------------

pub(super) fn duplicate_line(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let hidden = Hidden::of(v);
    let buf = v.model.buffer().clone();
    let lines = buf.len_lines();
    let blocks = blocks(v, false, |a, b| (a, hidden.row_end(b, lines)));
    let eol = buf.dominant_line_ending().as_str();
    let mut edits: Vec<(Range<usize>, Arc<str>)> = Vec::with_capacity(blocks.len());
    // Per range: how far it moves (onto the copy, and by the copies before it).
    let mut shift = vec![0usize; v.model.selection().ranges().len()];
    let mut added = 0usize;
    for b in &blocks {
        let start = buf.line_to_byte(b.first);
        let end = content_end(&buf, b.last);
        let copy = format!("{eol}{}", buf.slice(start..end));
        for &i in &b.members {
            shift[i] = added + copy.len();
        }
        added += copy.len();
        edits.push((end..end, Arc::from(copy)));
    }
    let after = mapped(v, |i, p| p + shift[i]);
    change(
        v,
        edits,
        Some(after),
        UserEvent::Command("duplicate_line"),
        cx,
    );
}

/// The byte column of `at` on its line, kept on line `l` (clamped to its text, on a char
/// boundary), as an offset in `buf`.
fn same_column(buf: &TextBuffer, at: usize, l: usize) -> usize {
    let col = at - buf.line_to_byte(buf.byte_to_line(at));
    let r = buf.line_range(l, false);
    buf.snap((r.start + col).min(r.end), Bias::Before)
}

pub(super) fn delete_line(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let buf = v.model.buffer().clone();
    let lines = buf.len_lines();
    let blocks = blocks(v, true, |a, b| (a, b));
    let sel = v.model.selection().clone();
    let mut edits: Vec<(Range<usize>, Arc<str>)> = Vec::with_capacity(blocks.len());
    let mut carets = Vec::with_capacity(sel.ranges().len());
    let mut removed = 0usize;
    for b in &blocks {
        let (del, target) = if b.last + 1 < lines {
            (
                buf.line_to_byte(b.first)..buf.line_to_byte(b.last + 1),
                Some(b.last + 1),
            )
        } else if b.first > 0 {
            (
                content_end(&buf, b.first - 1)..buf.len_bytes(),
                Some(b.first - 1),
            )
        } else {
            (0..buf.len_bytes(), None)
        };
        for &i in &b.members {
            let head = sel.ranges()[i].head;
            let at = match target {
                // The next line slides up to where the block began.
                Some(t) if t > b.last => {
                    let to = same_column(&buf, head, t);
                    to - del.len()
                }
                Some(t) => same_column(&buf, head, t),
                None => 0,
            };
            carets.push(SelectionRange::caret(at.saturating_sub(removed)));
        }
        removed += del.len();
        edits.push((del, Arc::from("")));
    }
    let after = SelectionSet::normalized(carets, sel.main_index());
    change(v, edits, Some(after), UserEvent::Command("delete_line"), cx);
}

// ---- move ----------------------------------------------------------------------------------

pub(super) fn move_line_up(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    move_lines(v, false, cx);
}

pub(super) fn move_line_down(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    move_lines(v, true, cx);
}

/// A fold the move covers: its range relative to the moved text it sits in, and that text's
/// new start.
struct Refold {
    rel: Range<usize>,
    new_start: usize,
}

fn move_lines(v: &mut EditorView, down: bool, cx: &mut Cx<'_>) {
    if v.config.read_only {
        return;
    }
    let hidden = Hidden::of(v);
    let buf = v.model.buffer().clone();
    let lines = buf.len_lines();
    let blocks = blocks(v, true, |a, b| (a, hidden.row_end(b, lines)));
    // A block at the edge it moves to does not move; nor does anything else (one gesture).
    let at_edge = blocks.iter().any(|b| {
        if down {
            b.last + 1 >= lines
        } else {
            b.first == 0
        }
    });
    if at_edge || blocks.is_empty() {
        return;
    }
    let folded: Vec<Range<usize>> = v.fold.folds.folded().to_vec();
    let mut edits: Vec<(Range<usize>, Arc<str>)> = Vec::with_capacity(blocks.len());
    let mut shift: Vec<isize> = vec![0; v.model.selection().ranges().len()];
    let mut refolds: Vec<Refold> = Vec::new();
    for b in &blocks {
        let block = buf.line_to_byte(b.first)..content_end(&buf, b.last);
        // The neighbour: the row above or below, a folded one whole.
        let (span, new_text, block_to, other_to, other) = if down {
            let first = b.last + 1;
            let last = hidden.row_end(first, lines);
            let other = buf.line_to_byte(first)..content_end(&buf, last);
            let brk = buf.slice(block.end..other.start).into_owned();
            let text = format!(
                "{}{brk}{}",
                buf.slice(other.clone()),
                buf.slice(block.clone())
            );
            let block_to = block.start + other.len() + brk.len();
            (block.start..other.end, text, block_to, block.start, other)
        } else {
            let last = b.first - 1;
            let first = hidden.header(last);
            let other = buf.line_to_byte(first)..content_end(&buf, last);
            let brk = buf.slice(other.end..block.start).into_owned();
            let text = format!(
                "{}{brk}{}",
                buf.slice(block.clone()),
                buf.slice(other.clone())
            );
            let other_to = other.start + block.len() + brk.len();
            (other.start..block.end, text, other.start, other_to, other)
        };
        for &i in &b.members {
            shift[i] = block_to as isize - block.start as isize;
        }
        // The folds inside the block or the neighbour go with their text.
        for f in &folded {
            for (part, to) in [(&block, block_to), (&other, other_to)] {
                if part.start <= f.start && f.end <= part.end {
                    refolds.push(Refold {
                        rel: f.start - part.start..f.end - part.start,
                        new_start: to,
                    });
                }
            }
        }
        edits.push((span, Arc::from(new_text)));
    }
    let after = mapped(v, |i, p| p.saturating_add_signed(shift[i]));
    // The covered folds open first (the edit would drop them), then fold again where their
    // text went: a move never opens a fold.
    let covered: Vec<Range<usize>> = folded
        .iter()
        .filter(|f| {
            edits
                .iter()
                .any(|(s, _)| s.start <= f.start && f.end <= s.end)
        })
        .cloned()
        .collect();
    for f in &covered {
        v.fold.folds.unfold_range(f);
    }
    let event = UserEvent::Command(if down {
        "move_line_down"
    } else {
        "move_line_up"
    });
    if !change(v, edits, Some(after), event, cx) {
        // Refused: the folds are put back as they were.
        let buf = v.model.buffer().clone();
        for f in covered {
            v.fold.folds.fold(f, &buf);
        }
        return;
    }
    let buf = v.model.buffer().clone();
    for r in refolds {
        let at = r.new_start + r.rel.start..r.new_start + r.rel.end;
        v.fold.folds.fold(at, &buf);
    }
    cx.notify();
}

// ---- line endings ----------------------------------------------------------------------------

pub(super) fn normalize_line_endings(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let to = v.model.buffer().dominant_line_ending();
    normalize(v, to, cx);
}

/// Rewrites every line break that is not `to` (one transaction); nothing when none is.
fn normalize(v: &mut EditorView, to: LineEnding, cx: &mut Cx<'_>) -> bool {
    let buf = v.model.buffer().clone();
    let stats = buf.meta().line_endings;
    let others = match to {
        LineEnding::Lf => stats.crlf + stats.cr,
        LineEnding::CrLf => stats.lf + stats.cr,
        LineEnding::Cr => stats.lf + stats.crlf,
    };
    if others == 0 {
        return false;
    }
    let text: Arc<str> = Arc::from(to.as_str());
    let edits: Vec<(Range<usize>, Arc<str>)> = (0..buf.len_lines().saturating_sub(1))
        .filter_map(|l| {
            let brk = content_end(&buf, l)..buf.line_range(l, true).end;
            (*buf.slice(brk.clone()) != *to.as_str()).then(|| (brk, text.clone()))
        })
        .collect();
    change(
        v,
        edits,
        None,
        UserEvent::Command("normalize_line_endings"),
        cx,
    )
}

// ---- history recall ------------------------------------------------------------------------

/// Whether a recall may go on: one caret on the last line of an empty text, or of the text the
/// last recall put in, unedited since.
fn recall_ready(v: &EditorView) -> bool {
    let sel = v.model.selection();
    let buf = v.model.buffer();
    if sel.ranges().len() != 1 || !sel.main().is_empty() {
        return false;
    }
    let last = buf.len_lines().saturating_sub(1);
    if buf.byte_to_line(sel.main().head) != last {
        return false;
    }
    let empty = buf.len_bytes() == 0;
    empty || v.commands.lines.recalled == Some(v.model.version())
}

fn recall(v: &mut EditorView, older: bool, cx: &mut Cx<'_>) {
    super::acted(v);
    if v.config.read_only || v.ime.is_composing() || !recall_ready(v) {
        return;
    }
    cx.emit(EditorEvent::HistoryRecall { older });
}

pub(super) fn history_prev(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    recall(v, true, cx);
}

pub(super) fn history_next(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    recall(v, false, cx);
}

// ---- the host's side -----------------------------------------------------------------------

impl EditorView {
    /// How many line endings of each kind the text has (the status bar's "Line endings: LF 214,
    /// CRLF 12").
    pub fn line_endings(&self) -> LineEndingStats {
        self.model.buffer().meta().line_endings
    }

    /// Rewrites every line break to `to` as one undo group (the status bar's "Normalize to …");
    /// `true` when anything changed. Refused on a read-only editor.
    pub fn normalize_line_endings_to(&mut self, to: LineEnding, cx: &mut Cx<'_>) -> bool {
        normalize(self, to, cx)
    }

    /// Puts a statement recalled from the history in the editor (the answer to
    /// `EditorEvent::HistoryRecall`): the whole text replaced as one transaction, the caret at
    /// its end, `sentence` said in the status bar and to assistive technology. Further recalls
    /// go on from it until it is edited.
    pub fn show_recalled(&mut self, text: &str, sentence: SharedString, cx: &mut Cx<'_>) {
        if self.config.read_only || self.ime.is_composing() {
            return;
        }
        let len = self.model.buffer().len_bytes();
        let Ok(changes) = ChangeSet::from_edits(len, [(0..len, Arc::from(text))]) else {
            return;
        };
        let ann = self.annotations(UserEvent::Command("history_recall"), Origin::User, cx);
        self.model.seal_history();
        self.commit(
            Transaction {
                changes,
                selection: Some(SelectionSet::cursor(text.len())),
                ann,
            },
            cx,
        );
        self.model.seal_history();
        self.commands.lines.recalled = Some(self.model.version());
        say(self, sentence, cx);
    }

    /// Makes `set` the selection (several ranges: the host's "select these"), each end snapped
    /// to a character boundary.
    pub fn select_ranges(&mut self, set: SelectionSet, cx: &mut Cx<'_>) {
        let set = set.snapped_to_graphemes(self.model.buffer());
        super::select_set(self, set, UserEvent::Select, cx);
    }
}
