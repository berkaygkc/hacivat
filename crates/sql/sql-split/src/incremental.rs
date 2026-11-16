//! Incremental resplit (SQL-SPLIT-05; Task 5): `SqlDocument::apply` reuses the relexed tokens
//! and resplits only a window of the text around the edit, keeping every other statement (and
//! its `StmtKey`, moved by the edit).
//!
//! **Anchors.** The structural pass reads one statement after another, and at some places
//! nothing it carries forward depends on what came before but the client delimiter: no
//! statement is open, no `COPY` data is due, no trivia is pending. An *anchor* is such a place:
//! the start of the line of a statement's (or empty item's) smart range, with only blanks
//! before the item on that line and only white space since the last item, client command or
//! `COPY` data before it, and no byte-order mark there. Split from an anchor with the delimiter
//! in force there, the text reads as the whole pass reads it from that line on (its leading
//! comments start at the anchor; the trailing comments of the item before ended on an earlier
//! line).
//!
//! **The window.** The edit `[e0, e1)` lies between an anchor `A` at or before it and an anchor
//! `B` after it. The new text's `[A, B')` (`B' = B` moved by the edit) is split on its own; when
//! that split ends where the old pass stood at `B` (nothing open, no `COPY` data due, the same
//! delimiter, only white space before `B'`), the pass over the new text is the old pass before
//! `A`, the window's, and the old pass from `B` moved by the edit. If it does not, a later `B`
//! is tried, and last the end of the text (`B' = len`: the window is the rest of the text).
//!
//! **What reaches across a window, and closes the fast path** (the full analysis runs instead,
//! as in M3a): the editor's stability rule (an unterminated construct before the window, or one
//! the window opens and the window ends before the text does); an executable comment or hint
//! whose end is ambiguous before `B` (its refusal runs forward to where the server ends it); the
//! MySQL stray-`END` backstop in or after the window (it refuses back to the last
//! block-tracked statement); a psql `\if` depth that differs at the window's ends. All of these
//! are rare while typing; the property `incremental_resplit_equals_full_split` reaches them.

use std::collections::HashMap;
use std::sync::Arc;

use sql_dialect::split_profile::{BlockFamily, ClientProfile, client_profile};
use sql_dialect::{ClientFlavor, DialectCtx, split_profile};
use sql_text::{TextRange, TextSize};

use crate::document::DocDelta;
use crate::item::{
    AmbiguityReason, AmbiguousBoundary, ClientSegment, CopyData, Diagnostic, Item, Split,
    SplitMode, Stmt, StmtHead,
};
use crate::key::StmtKey;
use crate::recover::Cut;
use crate::splitter::{Analysis, RawItem, RawStmt, SplitStart, analyze, size};

const BOM: &[u8] = b"\xEF\xBB\xBF";

/// How many items an anchor is looked for across, before the window falls back to the text's
/// start (for `A`) or a further `B` is tried.
const ANCHOR_SCAN: usize = 64;

/// How many window ends are tried before the window runs to the end of the text.
const WINDOW_TRIES: u32 = 6;

/// The old revision a resplit starts from.
pub(crate) struct Old<'a> {
    pub text: &'a str,
    pub ctx: DialectCtx,
    pub flavor: ClientFlavor,
    pub analysis: &'a Analysis,
    pub editor: &'a Split,
    pub execution: Option<&'a Split>,
}

/// A resplit's result: the new pass and views, and what changed.
pub(crate) struct Resplit {
    pub analysis: Analysis,
    pub editor: Split,
    pub execution: Option<Split>,
    pub delta: DocDelta,
}

/// The edit as one change: `[p, old_end)` of the old text became `[p, new_end)` of the new.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Change {
    pub p: usize,
    pub old_end: usize,
    pub new_end: usize,
}

impl Change {
    fn delta(&self) -> isize {
        self.new_end as isize - self.old_end as isize
    }
}

/// Resplits `new_text` from `old` after `change`, or `None` when the change reaches across
/// every window (module documentation): the caller runs the whole pass.
pub(crate) fn resplit(old: &Old, new_text: &str, change: Change) -> Option<Resplit> {
    let a = old.analysis;
    let cp = client_profile(old.flavor);
    let mysql = split_profile(&old.ctx).family == BlockFamily::MySql;
    let (k, start) = window_start(old.text, a, change.p);
    // A stray `END` at or after the window's start refuses back past it.
    if mysql && a.items[k..].iter().any(|it| stray_end(old.text, it)) {
        return None;
    }
    if if_depth(old.text, cp, &a.client, start) != Some(0) {
        return None;
    }
    // The edit may start at the anchor: a byte-order mark it puts there would read as white
    // space at the window's offset 0 and as text in the document.
    if start > 0 && new_text.as_bytes()[start..].starts_with(BOM) {
        return None;
    }
    let delim = a.delimiter_at(size(start));
    let first = a
        .items
        .partition_point(|it| usize::from(smart_start(it)) <= change.old_end);
    let mut skip = 0usize;
    for _ in 0..WINDOW_TRIES {
        let Some((m, end)) = anchor_from(old.text, a, first + skip, change.old_end) else {
            break;
        };
        if let Some(r) = window(old, new_text, change, (k, start), (m, end), &delim) {
            return Some(r);
        }
        skip = skip * 2 + 1;
    }
    // The rest of the text.
    window(
        old,
        new_text,
        change,
        (k, start),
        (a.items.len(), old.text.len()),
        &delim,
    )
}

/// The window's start: the last anchor at or before `e0` and its item, else the text's start.
fn window_start(text: &str, a: &Analysis, e0: usize) -> (usize, usize) {
    let mut k = a
        .items
        .partition_point(|it| usize::from(smart_start(it)) <= e0);
    for _ in 0..ANCHOR_SCAN {
        let Some(c) = k.checked_sub(1) else {
            break;
        };
        k = c;
        if let Some(line) = anchor(text, a, c)
            && line <= e0
        {
            return (c, line);
        }
    }
    (0, 0)
}

/// The first anchor after `after` among the items from `from` on: its item and line.
fn anchor_from(text: &str, a: &Analysis, from: usize, after: usize) -> Option<(usize, usize)> {
    (from..a.items.len().min(from + ANCHOR_SCAN))
        .find_map(|m| anchor(text, a, m).filter(|l| *l > after).map(|l| (m, l)))
}

/// Splits the window `[start, end)` of the old text (items `k..m`) again over the new text,
/// and splices it in when the pass leaves it as the old pass stood at `end`.
fn window(
    old: &Old,
    new_text: &str,
    change: Change,
    (k, start): (usize, usize),
    (m, end): (usize, usize),
    delim: &Option<Arc<str>>,
) -> Option<Resplit> {
    let a = old.analysis;
    let delta = change.delta();
    let to_eof = end == old.text.len();
    let new_end = end.wrapping_add_signed(delta);
    // What reaches across the window's end into it, or from before its start over it.
    let reaches = |u: TextSize| usize::from(u) < if to_eof { start } else { end };
    if a.unterminated.is_some_and(reaches) {
        return None;
    }
    if a.ambiguous
        .iter()
        .any(|x| x.reason == AmbiguityReason::AmbiguousCommentEnd && reaches(x.at.start()))
    {
        return None;
    }
    let piece = &new_text[start..new_end];
    let begin = SplitStart {
        lex: sql_lexer::LexState::Initial,
        delimiter: delim.clone(),
    };
    let w = analyze(piece, None, &old.ctx, old.flavor, &begin);
    // A restart at a byte-order mark (after a client command): the pass reads it as white space
    // when it lexes from there, as text when the document's lexing has a token there; which one
    // it does depends on lexing the window cannot see.
    if w.restarts.iter().any(|(p, _)| {
        let p = usize::from(*p);
        start + p > 0 && piece.as_bytes()[p..].starts_with(BOM)
    }) {
        return None;
    }
    let cp = client_profile(old.flavor);
    if split_profile(&old.ctx).family == BlockFamily::MySql
        && w.items.iter().any(|it| stray_end(piece, it))
    {
        return None;
    }
    // Short of the end, the window's pass must end as the old pass stood at `end`.
    let open_at_end = w.unterminated.is_some()
        || w.ambiguous
            .iter()
            .any(|x| x.reason == AmbiguityReason::AmbiguousCommentEnd)
        || w.copies.iter().any(|(_, d)| d.end.is_none())
        || matches!(w.items.last(), Some(RawItem::Stmt(s)) if s.terminator.is_none())
        || w.delimiter_at(size(piece.len())) != a.delimiter_at(size(end))
        || if_depth(piece, cp, &w.client, piece.len()) != if_depth(old.text, cp, &a.client, end)
        || !window_ends_clean(piece, &w);
    if !to_eof && open_at_end {
        return None;
    }
    // The window to the end replaces everything from its start on (a restart or a `COPY`'s
    // data may sit exactly at the old text's end).
    let cut = if to_eof { usize::MAX } else { end };
    let editor = splice_split(
        old.editor,
        w.to_split(piece, SplitMode::Editor),
        start,
        cut,
        delta,
    );
    let execution = old.execution.map(|x| {
        splice_split(
            x,
            w.to_split(piece, SplitMode::Execution),
            start,
            cut,
            delta,
        )
    });
    let doc_delta = doc_delta(old.editor, &editor, start, cut, new_end, change);
    let analysis = splice_analysis(a, w, (k, m), start, cut, delta);
    Some(Resplit {
        analysis,
        editor,
        execution,
        delta: doc_delta,
    })
}

// ---- anchors -----------------------------------------------------------------------------------

/// Where item `it`'s smart range starts.
fn smart_start(it: &RawItem) -> TextSize {
    match it {
        RawItem::Stmt(s) => s
            .leading
            .map_or(s.strict.start(), |l| l.start().min(s.strict.start())),
        RawItem::Empty(r) => r.start(),
    }
}

/// Where item `it` ends: its smart range, terminator and trailing comments included.
fn item_end(it: &RawItem) -> TextSize {
    match it {
        RawItem::Stmt(s) => s
            .smart_end
            .max(s.strict.end())
            .max(s.terminator.map_or(s.strict.end(), |t| t.end())),
        RawItem::Empty(r) => r.end(),
    }
}

/// White space that never starts or ends an item in any dialect.
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0c)
}

/// The line of item `i` when it is an anchor (module documentation).
pub(crate) fn anchor(text: &str, a: &Analysis, i: usize) -> Option<usize> {
    let s = usize::from(smart_start(&a.items[i]));
    let b = text.as_bytes();
    let line = b[..s]
        .iter()
        .rposition(|c| *c == b'\n')
        .map_or(0, |lf| lf + 1);
    if !b[line..s].iter().all(|c| *c == b' ' || *c == b'\t') {
        return None;
    }
    if line > 0 && b[line..].starts_with(BOM) {
        return None;
    }
    let before = last_end(a, i, line);
    clean_before(text, line, before).then_some(line)
}

/// The end of the last thing the pass read before `line` among items `..i`, client commands
/// and `COPY` data (0 when there is none).
fn last_end(a: &Analysis, i: usize, line: usize) -> usize {
    let item = i
        .checked_sub(1)
        .map_or(0, |p| usize::from(item_end(&a.items[p])));
    let c = a
        .client
        .partition_point(|c| usize::from(c.range.start()) < line);
    let client = c
        .checked_sub(1)
        .map_or(0, |c| usize::from(a.client[c].range.end()));
    let copy = a
        .copies
        .iter()
        .rev()
        .find(|(_, d)| usize::from(d.rows.start()) < line)
        .map_or(0, |(_, d)| {
            usize::from(d.end.map_or(d.rows.end(), |e| e.end()))
        });
    item.max(client).max(copy)
}

/// Whether the window's pass ended before its end, with only white space after the last thing
/// it read: nothing it read ran to the window's end (a `\copy … from stdin` whose data has no
/// `\.` line, a command on a last line), where the document's pass would read on.
fn window_ends_clean(piece: &str, w: &Analysis) -> bool {
    let last = last_end(w, w.items.len(), piece.len());
    (piece.is_empty() || last < piece.len()) && clean_before(piece, piece.len(), last)
}

/// Whether only white space stands between `from` and `line`.
fn clean_before(text: &str, line: usize, from: usize) -> bool {
    let b = text.as_bytes();
    let mut q = line;
    while q > 0 && is_space(b[q - 1]) {
        q -= 1;
    }
    q <= from
}

/// A MySQL/MariaDB statement that starts with `END` (the stray-`END` backstop).
fn stray_end(text: &str, it: &RawItem) -> bool {
    matches!(it, RawItem::Stmt(s) if s.head.first_token
        .is_some_and(|t| text[t].eq_ignore_ascii_case("END")))
}

/// The psql `\if` depth after the client commands of `segments` before `upto`, as the pass
/// counts it (never below 0); `Some(0)` without conditionals.
pub(crate) fn if_depth(
    text: &str,
    cp: &ClientProfile,
    segments: &[ClientSegment],
    upto: usize,
) -> Option<u32> {
    if cp.backslash_conditionals.is_empty() {
        return Some(0);
    }
    let mut depth = 0u32;
    for seg in segments
        .iter()
        .take_while(|s| usize::from(s.range.start()) < upto)
    {
        let t = &text[seg.range];
        let Some(rest) = t.strip_prefix('\\') else {
            continue;
        };
        let name_len = rest
            .find(|c: char| c.is_ascii_whitespace() || c == '\\')
            .unwrap_or(rest.len());
        if let Some((_, n)) = cp
            .backslash_conditionals
            .iter()
            .find(|(c, _)| *c == &rest[..name_len])
        {
            depth = depth.saturating_add_signed((*n).into());
        }
    }
    Some(depth)
}

// ---- splicing ----------------------------------------------------------------------------------

fn mv(s: TextSize, by: isize) -> TextSize {
    size(usize::from(s).wrapping_add_signed(by))
}

fn mv_range(r: TextRange, by: isize) -> TextRange {
    TextRange::new(mv(r.start(), by), mv(r.end(), by))
}

fn mv_head(h: StmtHead, by: isize) -> StmtHead {
    StmtHead {
        first_token: h.first_token.map(|r| mv_range(r, by)),
        ..h
    }
}

fn mv_copy(d: CopyData, by: isize) -> CopyData {
    CopyData {
        rows: mv_range(d.rows, by),
        end: d.end.map(|r| mv_range(r, by)),
    }
}

fn mv_raw(it: &RawItem, by: isize) -> RawItem {
    match it {
        RawItem::Empty(r) => RawItem::Empty(mv_range(*r, by)),
        RawItem::Stmt(s) => RawItem::Stmt(RawStmt {
            strict: mv_range(s.strict, by),
            smart_end: mv(s.smart_end, by),
            terminator: s.terminator.map(|r| mv_range(r, by)),
            leading: s.leading.map(|r| mv_range(r, by)),
            head: mv_head(s.head, by),
            delimiter: s.delimiter.clone(),
            copy_data: s.copy_data.map(|d| mv_copy(d, by)),
            cuts: s
                .cuts
                .iter()
                .map(|c| Cut {
                    a_end: mv(c.a_end, by),
                    a_smart_end: mv(c.a_smart_end, by),
                    b_leading: c.b_leading.map(|r| mv_range(r, by)),
                    b_head: mv_head(c.b_head, by),
                    at: mv_range(c.at, by),
                })
                .collect(),
            uncertain: s.uncertain,
            key: s.key,
            block: s.block,
        }),
    }
}

fn mv_stmt(s: Stmt, by: isize) -> Stmt {
    Stmt {
        strict: mv_range(s.strict, by),
        smart: mv_range(s.smart, by),
        terminator: s.terminator.map(|r| mv_range(r, by)),
        leading_comments: s.leading_comments.map(|r| mv_range(r, by)),
        head: mv_head(s.head, by),
        copy_data: s.copy_data.map(|d| mv_copy(d, by)),
        ..s
    }
}

fn mv_item(it: &Item, by: isize) -> Item {
    match it {
        Item::Statement(s) => Item::Statement(mv_stmt(*s, by)),
        Item::Empty { range } => Item::Empty {
            range: mv_range(*range, by),
        },
    }
}

/// `old`'s entries before `start`, `window`'s moved to `start`, and `old`'s from `end` on moved
/// by `delta`, each list in text order by `pos`.
fn splice<T: Clone>(
    old: &[T],
    window: Vec<T>,
    pos: impl Fn(&T) -> TextSize,
    mv: impl Fn(&T, isize) -> T,
    start: usize,
    end: usize,
    delta: isize,
) -> Vec<T> {
    let lo = old.partition_point(|x| usize::from(pos(x)) < start);
    let hi = old.partition_point(|x| usize::from(pos(x)) < end).max(lo);
    let mut out = Vec::with_capacity(lo + window.len() + old.len() - hi);
    out.extend_from_slice(&old[..lo]);
    out.extend(window.iter().map(|x| mv(x, start as isize)));
    out.extend(old[hi..].iter().map(|x| mv(x, delta)));
    out
}

fn item_pos(it: &Item) -> TextSize {
    match it {
        Item::Statement(s) => s.strict.start(),
        Item::Empty { range } => range.start(),
    }
}

/// An editor or execution split spliced (module documentation).
fn splice_split(old: &Split, w: Split, start: usize, end: usize, delta: isize) -> Split {
    Split {
        items: splice(&old.items, w.items, item_pos, mv_item, start, end, delta),
        ambiguous: splice(
            &old.ambiguous,
            w.ambiguous,
            |x| x.at.start(),
            |x, by| AmbiguousBoundary {
                at: mv_range(x.at, by),
                ..*x
            },
            start,
            end,
            delta,
        ),
        client: splice(
            &old.client,
            w.client,
            |x| x.range.start(),
            |x, by| ClientSegment {
                range: mv_range(x.range, by),
                ..*x
            },
            start,
            end,
            delta,
        ),
        diagnostics: splice(
            &old.diagnostics,
            w.diagnostics,
            |x| x.range.start(),
            |x, by| Diagnostic {
                range: mv_range(x.range, by),
                ..*x
            },
            start,
            end,
            delta,
        ),
    }
}

/// The structural pass spliced (module documentation).
fn splice_analysis(
    a: &Analysis,
    w: Analysis,
    (k, m): (usize, usize),
    start: usize,
    end: usize,
    delta: isize,
) -> Analysis {
    let shift = start as isize;
    let mut items = Vec::with_capacity(k + w.items.len() + a.items.len() - m);
    items.extend_from_slice(&a.items[..k]);
    items.extend(w.items.iter().map(|it| mv_raw(it, shift)));
    items.extend(a.items[m..].iter().map(|it| mv_raw(it, delta)));
    // The window's, else one after it (one before it closed the fast path).
    let unterminated = w.unterminated.map(|u| mv(u, shift)).or_else(|| {
        a.unterminated
            .filter(|u| usize::from(*u) >= end)
            .map(|u| mv(u, delta))
    });
    Analysis {
        items,
        client: splice(
            &a.client,
            w.client,
            |x| x.range.start(),
            |x, by| ClientSegment {
                range: mv_range(x.range, by),
                ..*x
            },
            start,
            end,
            delta,
        ),
        ambiguous: splice(
            &a.ambiguous,
            w.ambiguous,
            |x| x.at.start(),
            |x, by| AmbiguousBoundary {
                at: mv_range(x.at, by),
                ..*x
            },
            start,
            end,
            delta,
        ),
        diagnostics: splice(
            &a.diagnostics,
            w.diagnostics,
            |x| x.range.start(),
            |x, by| Diagnostic {
                range: mv_range(x.range, by),
                ..*x
            },
            start,
            end,
            delta,
        ),
        unterminated,
        delimiters: splice(
            &a.delimiters,
            w.delimiters,
            |x| x.0,
            |x, by| (mv(x.0, by), x.1.clone()),
            start,
            end,
            delta,
        ),
        restarts: splice(
            &a.restarts,
            w.restarts,
            |x| x.0,
            |x, by| (mv(x.0, by), x.1.clone()),
            start,
            end,
            delta,
        ),
        checkpoints: splice(
            &a.checkpoints,
            w.checkpoints,
            |x| *x,
            |x, by| mv(*x, by),
            start,
            end,
            delta,
        ),
        copies: splice(
            &a.copies,
            w.copies,
            |x| x.0,
            |x, by| (mv(x.0, by), mv_copy(x.1, by)),
            start,
            end,
            delta,
        ),
        ctx: a.ctx,
        flavor: a.flavor,
        start_delimiter: a.start_delimiter.clone(),
        lexed: a.lexed + w.lexed,
    }
}

/// What the change did to the editor statements: the window's statements matched by key to
/// the old window's (equal text: unchanged or moved), the others changed or removed; every
/// statement after the window moved when the text's length changed.
fn doc_delta(
    old: &Split,
    new: &Split,
    start: usize,
    end: usize,
    new_end: usize,
    change: Change,
) -> DocDelta {
    let in_window = |it: &Item, to: usize| {
        let p = usize::from(item_pos(it));
        start <= p && p < to
    };
    let mut pool: HashMap<StmtKey, Vec<TextSize>> = HashMap::new();
    for s in old
        .items
        .iter()
        .filter(|it| in_window(it, end))
        .filter_map(Item::statement)
    {
        pool.entry(s.key).or_default().push(s.strict.start());
    }
    for v in pool.values_mut() {
        v.reverse();
    }
    let (mut changed, mut moved) = (Vec::new(), Vec::new());
    let mut dirty: Option<TextRange> = None;
    let delta = change.delta();
    for it in &new.items {
        let Some(s) = it.statement() else {
            continue;
        };
        let p = usize::from(s.strict.start());
        if p < start {
            continue;
        }
        if p >= new_end {
            if delta != 0 {
                moved.push(s.key);
            }
            continue;
        }
        match pool.get_mut(&s.key).and_then(Vec::pop) {
            Some(at) if at == s.strict.start() => {}
            Some(_) => moved.push(s.key),
            None => {
                changed.push(s.key);
                dirty = Some(dirty.map_or(s.smart, |d| d.cover(s.smart)));
            }
        }
    }
    let mut removed: Vec<StmtKey> = pool
        .into_iter()
        .flat_map(|(k, left)| std::iter::repeat_n(k, left.len()))
        .collect();
    removed.sort();
    if change.p < change.old_end || change.p < change.new_end {
        let edited = TextRange::new(size(change.p), size(change.new_end));
        dirty = Some(dirty.map_or(edited, |d| d.cover(edited)));
    }
    DocDelta {
        changed,
        moved,
        removed,
        dirty: dirty.unwrap_or_else(|| TextRange::empty(size(change.p))),
    }
}
