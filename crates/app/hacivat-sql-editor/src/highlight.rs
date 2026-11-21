//! Lexical highlighting (UI-EDIT-11 L1; Task 24): `sql_lexer::TokenClass` to the editor's
//! `HighlightClass` (R-M3-42), rebased through the editor's change log.
//!
//! **Classes.** A word directly followed by `(` is a `Function`, unless it is a reserved
//! keyword (`IN(`, `VALUES(`… stay keywords; `count(`, `coalesce(`, `left(` are calls). A
//! dollar-quoted string is split into its two `DollarTag`s and its `String` body (the lexer
//! never returns `DollarTag`). A running MySQL/MariaDB executable comment (`/*! … */`, gate
//! active or unknown) is marked `ExecComment` whole, which the element paints as the inlay
//! well with comment ink, and the tokens of its body get their own classes on top; one whose
//! gate the server is below is a plain `Comment`. White space is not marked.
//!
//! **Regions.** psql/mysql/sqlite3 client commands are `MetaCommand` and `COPY … FROM STDIN`
//! rows are data (`String`, their `\.` a `MetaCommand`), from the editor split, whatever the
//! whole-text lexing reads inside them.
//!
//! **Known divergence (cosmetic).** The colours come from `DocSnapshot::lexed()`, one lexing
//! of the whole text. After a client command, `COPY` data or a client delimiter the splitter
//! lexes again from its own state, so past such a region the colours can differ from the split
//! (a quote in `COPY` rows colours the SQL after them as a string). Run and the preview use the
//! split, never these colours.
//!
//! **Rebasing (R-M3-22).** Marks are computed for the analysed version's text. When the editor
//! is ahead, the visible range is mapped back conservatively, the marks of that older range
//! are mapped forward through the editor's `ChangeLog` as `RangeKind::Content` (a token an
//! edit touched since is dropped, so it paints plain until the next analysis), and an analysis
//! older than the log paints nothing.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use hacivat_text::display::{Decoration, RangeSet};
use hacivat_text::{ChangeDesc, ChangeLog, RangeKind, Rebased, Snapshot};
use hacivat_ui_editor::{DecorationSource, HighlightClass, SourceId};
use sql_dialect::{DialectCtx, DialectId, KwCategory, lex_profile};
use sql_lexer::{
    GateStatus, LexState, Lexed, TokenClass, TokenFlags, TokenKind, exec_comment,
    exec_comment_gate, lex, token_class,
};
use sql_split::DocSnapshot;
use sql_text::{TextRange, TextSize};

use crate::service::DocShared;
use crate::worker::Analysis;

/// The highlight source's ID ("24": its task).
pub(crate) const SOURCE_ID: SourceId = SourceId(24);

type Spans = Vec<(Range<usize>, HighlightClass)>;

fn size(n: usize) -> TextSize {
    TextSize::from(u32::try_from(n).unwrap_or(u32::MAX))
}

fn range(r: TextRange) -> Range<usize> {
    usize::from(r.start())..usize::from(r.end())
}

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

/// The lexer's class of a token, as the editor's class (`None`: nothing painted).
fn editor_class(class: TokenClass) -> Option<HighlightClass> {
    Some(match class {
        TokenClass::Keyword => HighlightClass::Keyword,
        TokenClass::Identifier => HighlightClass::Ident,
        TokenClass::QuotedIdentifier => HighlightClass::QuotedIdent,
        TokenClass::String => HighlightClass::String,
        TokenClass::Number => HighlightClass::Number,
        TokenClass::Comment => HighlightClass::Comment,
        TokenClass::Operator => HighlightClass::Operator,
        TokenClass::Punctuation => HighlightClass::Punct,
        TokenClass::Parameter => HighlightClass::Param,
        TokenClass::DollarTag => HighlightClass::DollarTag,
        TokenClass::ExecutableComment => HighlightClass::ExecComment,
        TokenClass::OptimizerHint => HighlightClass::Hint,
        TokenClass::ClientCommand => HighlightClass::MetaCommand,
        TokenClass::Error => HighlightClass::ErrorToken,
        _ => return None,
    })
}

/// Token `i` is directly followed by `(`.
fn called(lexed: &Lexed, i: usize) -> bool {
    i + 1 < lexed.len() && lexed.kind(i + 1) == TokenKind::LParen
}

/// The pieces of token `i` of `lexed` (a lexing of `text`), offsets moved by `base`; an
/// executable comment is left to the caller ([`push_token`]).
fn token_pieces(text: &str, lexed: &Lexed, i: usize, base: usize, out: &mut Spans) {
    let (kind, flags, kw) = (lexed.kind(i), lexed.flags(i), lexed.keyword(i));
    let r = range(lexed.range(i));
    let Some(class) = editor_class(token_class(kind, flags, kw)) else {
        return;
    };
    let class = match class {
        HighlightClass::Ident if called(lexed, i) => HighlightClass::Function,
        HighlightClass::Keyword
            if called(lexed, i) && kw.is_some_and(|k| k.category != KwCategory::Reserved) =>
        {
            HighlightClass::Function
        }
        HighlightClass::String if kind == TokenKind::DollarString => {
            dollar_pieces(text, r, flags, base, out);
            return;
        }
        other => other,
    };
    out.push((base + r.start..base + r.end, class));
}

/// A dollar-quoted string: its opening tag, its body, its closing tag (none when it runs to
/// the end of the text).
fn dollar_pieces(text: &str, r: Range<usize>, flags: TokenFlags, base: usize, out: &mut Spans) {
    let whole = (base + r.start..base + r.end, HighlightClass::String);
    let Some(token) = text.get(r.clone()) else {
        out.push(whole);
        return;
    };
    // The tag is `$`, an identifier without `$`, and `$`: the second `$` is right after it.
    let open = match token.strip_prefix('$').and_then(|rest| rest.find('$')) {
        Some(at) => at + 2,
        None => {
            out.push(whole);
            return;
        }
    };
    let tag = &token[..open];
    let closed = !flags.contains(TokenFlags::UNTERMINATED)
        && token.len() >= 2 * open
        && token.ends_with(tag);
    let (s, e) = (base + r.start, base + r.end);
    out.push((s..s + open, HighlightClass::DollarTag));
    let body_end = if closed { e - open } else { e };
    if s + open < body_end {
        out.push((s + open..body_end, HighlightClass::String));
    }
    if closed {
        out.push((e - open..e, HighlightClass::DollarTag));
    }
}

/// Whether token `i` is an executable comment that runs (or may run) on the server.
fn running_exec_comment(text: &str, lexed: &Lexed, i: usize, ctx: &DialectCtx) -> bool {
    lexed.kind(i) == TokenKind::ExecutableComment
        && token_class(lexed.kind(i), lexed.flags(i), lexed.keyword(i))
            == TokenClass::ExecutableComment
        && exec_comment_gate(text, lexed, i, ctx) != GateStatus::Inactive
}

/// The pieces of the document's token `i` (its own text and lexing), with the inner tokens of
/// a running executable comment from `inner`.
fn push_token(
    doc: &DocSnapshot,
    i: usize,
    inner: &[(Range<usize>, HighlightClass)],
    out: &mut Spans,
) {
    let (text, lexed) = (doc.text(), doc.lexed());
    if lexed.kind(i) == TokenKind::ExecutableComment
        && token_class(lexed.kind(i), lexed.flags(i), lexed.keyword(i))
            == TokenClass::ExecutableComment
    {
        let r = range(lexed.range(i));
        if exec_comment_gate(text, lexed, i, doc.ctx()) == GateStatus::Inactive {
            out.push((r, HighlightClass::Comment));
            return;
        }
        out.push((r.clone(), HighlightClass::ExecComment));
        let from = inner.partition_point(|(x, _)| x.end <= r.start);
        out.extend(
            inner[from..]
                .iter()
                .take_while(|(x, _)| x.start < r.end)
                .cloned(),
        );
        return;
    }
    token_pieces(text, lexed, i, 0, out);
}

/// The lexer profile of `ctx` (the generic one when the dialect has none), as the splitter
/// picks it.
// ## m3c-t32-fold (named exception): the fold scan lexes dollar bodies with it.
pub(crate) fn profile(ctx: &DialectCtx) -> &'static sql_dialect::LexDialect {
    match lex_profile(ctx) {
        Ok(p) => p,
        Err(_) => match lex_profile(&DialectCtx::new(DialectId::Generic, None)) {
            Ok(p) => p,
            Err(_) => unreachable!("the generic dialect always has a lexer profile"),
        },
    }
}

/// The tokens inside every running executable comment of `doc`: each body lexed alone, as the
/// server reads it (worker side).
pub(crate) fn exec_inner(doc: &DocSnapshot) -> Spans {
    let (text, lexed, ctx) = (doc.text(), doc.lexed(), doc.ctx());
    let mut out = Vec::new();
    for i in 0..lexed.len() {
        if !running_exec_comment(text, lexed, i, ctx) {
            continue;
        }
        let Some(ec) = exec_comment(text, lexed, i) else {
            continue;
        };
        let body = range(ec.body);
        let Some(body_text) = text.get(body.clone()) else {
            continue;
        };
        let inner = lex(body_text, profile(ctx), ctx.modes, &LexState::Initial);
        for j in 0..inner.len() {
            token_pieces(body_text, &inner, j, body.start, &mut out);
        }
    }
    out
}

/// Client commands and `COPY … FROM STDIN` data from the editor split, sorted, overlaps
/// clipped (worker side).
pub(crate) fn regions(doc: &DocSnapshot) -> Spans {
    let split = doc.editor_split();
    let mut out: Spans = split
        .client
        .iter()
        .map(|c| (range(c.range), HighlightClass::MetaCommand))
        .collect();
    for st in split.statements() {
        if let Some(data) = st.copy_data {
            out.push((range(data.rows), HighlightClass::String));
            if let Some(end) = data.end {
                out.push((range(end), HighlightClass::MetaCommand));
            }
        }
    }
    out.retain(|(r, _)| !r.is_empty());
    out.sort_by_key(|(r, _)| (r.start, r.end));
    // Not overlapping, so their ends are sorted too (the lookups below rely on it).
    let mut clipped: Spans = Vec::with_capacity(out.len());
    for (r, class) in out {
        let start = clipped
            .last()
            .map_or(r.start, |(last, _)| r.start.max(last.end));
        if start < r.end {
            clipped.push((start..r.end, class));
        }
    }
    clipped
}

/// The entries of a sorted, non-overlapping list that overlap `r`.
fn overlapping<'a>(
    list: &'a [(Range<usize>, HighlightClass)],
    r: &Range<usize>,
) -> &'a [(Range<usize>, HighlightClass)] {
    let from = list.partition_point(|(x, _)| x.end <= r.start);
    let to = list.partition_point(|(x, _)| x.start < r.end);
    &list[from..to.max(from)]
}

/// `piece` without the parts `regions` (sorted, not overlapping) cover.
fn push_outside(
    piece: (Range<usize>, HighlightClass),
    regions: &[(Range<usize>, HighlightClass)],
    out: &mut Spans,
) {
    let (mut r, class) = piece;
    for (x, _) in regions {
        if x.end <= r.start || r.end <= x.start {
            continue;
        }
        if r.start < x.start {
            out.push((r.start..x.start, class));
        }
        r.start = x.end.max(r.start);
        if r.start >= r.end {
            return;
        }
    }
    out.push((r, class));
}

/// The marks of the analysed text over `r` (its coordinates), sorted by start.
pub(crate) fn spans_in(a: &Analysis, r: Range<usize>) -> Spans {
    let lexed = a.doc.lexed();
    let len = a.doc.text().len();
    let r = r.start.min(len)..r.end.min(len);
    let regions = overlapping(&a.regions, &r);
    let mut raw = Spans::new();
    if !lexed.is_empty() {
        for i in lexed.tokens_in(TextRange::new(size(r.start), size(r.end.max(r.start)))) {
            push_token(&a.doc, i, &a.exec_inner, &mut raw);
        }
    }
    let mut out = Spans::with_capacity(raw.len() + regions.len());
    for piece in raw {
        push_outside(piece, regions, &mut out);
    }
    out.extend(regions.iter().cloned());
    out.retain(|(x, _)| overlaps(x, &r) || (r.is_empty() && x.start == r.start));
    out.sort_by_key(|(x, _)| x.start);
    out
}

/// `r` (in the text after `d`) mapped back to the text before it, widened to a range that
/// holds every position that maps into `r`.
pub(crate) fn back_map(r: Range<usize>, d: &ChangeDesc) -> Range<usize> {
    let Some(changed) = d.changed_range() else {
        return r;
    };
    let grow = d.len_new().saturating_sub(d.len_old());
    let shrink = d.len_old().saturating_sub(d.len_new());
    let old = |p: usize| p - grow + shrink;
    let old_changed_end = old(changed.end);
    let lo = if r.start <= changed.start {
        r.start
    } else if r.start >= changed.end {
        old(r.start)
    } else {
        changed.start
    };
    let hi = if r.end <= changed.start {
        r.end
    } else if r.end >= changed.end {
        old(r.end)
    } else {
        old_changed_end
    };
    lo.min(d.len_old())..hi.clamp(lo, d.len_old()).max(lo.min(d.len_old()))
}

/// The marks over `r` at `snap`'s version from analysis `a`, rebased through `log` when `a` is
/// older; empty when it cannot be rebased.
pub(crate) fn marks_at(a: &Analysis, snap: &Snapshot, log: &ChangeLog, r: Range<usize>) -> Spans {
    if a.version == snap.version {
        return spans_in(a, r);
    }
    if a.version > snap.version || log.version() != snap.version {
        return Spans::new();
    }
    let Some(descs) = log.descs_since(a.version) else {
        return Spans::new();
    };
    let descs: Vec<&ChangeDesc> = descs.collect();
    let old = descs.iter().rev().fold(r.clone(), |r, d| back_map(r, d));
    let mut out: Spans = spans_in(a, old)
        .into_iter()
        .filter_map(
            |(x, class)| match log.rebase(a.version, x, RangeKind::Content) {
                Rebased::Mapped(m) if !m.is_empty() && overlaps(&m, &r) => Some((m, class)),
                _ => None,
            },
        )
        .collect();
    out.sort_by_key(|(x, _)| x.start);
    out
}

/// The last answer, for the same analysis, version and range (a repaint of an unchanged frame).
struct Cached {
    analysis: Arc<Analysis>,
    version: u64,
    range: Range<usize>,
    set: Arc<RangeSet<Decoration>>,
}

/// The document's highlight source (`DecorationSource`), reading the latest analysis.
pub(crate) struct HighlightSource {
    shared: Rc<DocShared>,
    cache: RefCell<Option<Cached>>,
}

impl HighlightSource {
    pub(crate) fn new(shared: Rc<DocShared>) -> Self {
        HighlightSource {
            shared,
            cache: RefCell::new(None),
        }
    }
}

impl DecorationSource for HighlightSource {
    fn id(&self) -> SourceId {
        SOURCE_ID
    }

    /// Tokens paint under every other source.
    fn priority(&self) -> i16 {
        0
    }

    fn decorations(
        &self,
        snap: &Snapshot,
        log: &ChangeLog,
        range: Range<usize>,
    ) -> Arc<RangeSet<Decoration>> {
        let Some(a) = self.shared.latest() else {
            return Arc::new(RangeSet::default());
        };
        if let Some(c) = self.cache.borrow().as_ref()
            && Arc::ptr_eq(&c.analysis, &a)
            && c.version == snap.version
            && c.range == range
        {
            return c.set.clone();
        }
        let marks = marks_at(&a, snap, log, range.clone());
        let set = Arc::new(RangeSet::from_sorted(
            marks
                .into_iter()
                .map(|(r, class)| {
                    (
                        r,
                        Decoration::Mark {
                            style: class.style(),
                        },
                    )
                })
                .collect(),
        ));
        *self.cache.borrow_mut() = Some(Cached {
            analysis: a,
            version: snap.version,
            range,
            set: set.clone(),
        });
        set
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc as StdArc;

    use hacivat_text::ChangeSet;

    use super::*;

    fn desc(len: usize, edits: &[(Range<usize>, &str)]) -> ChangeDesc {
        ChangeSet::from_edits(
            len,
            edits.iter().map(|(r, t)| (r.clone(), StdArc::from(*t))),
        )
        .unwrap()
        .desc()
    }

    /// Every old position that maps into the asked range is inside the mapped-back range.
    #[test]
    fn back_map_holds_every_position_that_maps_into_the_range() {
        let len = 40;
        let cases: &[&[(Range<usize>, &str)]] = &[
            &[(5..5, "xyz")],
            &[(5..12, "")],
            &[(5..12, "ab"), (20..21, "long text")],
            &[(0..40, "short")],
            &[(39..40, "")],
        ];
        for edits in cases {
            let d = desc(len, edits);
            for lo in 0..=d.len_new() {
                for hi in lo..=d.len_new() {
                    let back = back_map(lo..hi, &d);
                    assert!(back.start <= back.end && back.end <= len);
                    for p in 0..=len {
                        let m = d
                            .map_pos(p, hacivat_text::Bias::After, hacivat_text::MapMode::Simple)
                            .unwrap();
                        if lo < m && m < hi {
                            assert!(
                                back.start <= p && p <= back.end,
                                "{edits:?}: old {p} maps to {m} inside {lo}..{hi}, back {back:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn non_overlapping_lookup_and_clipping() {
        let list = vec![
            (2..4, HighlightClass::String),
            (6..9, HighlightClass::MetaCommand),
        ];
        assert_eq!(overlapping(&list, &(0..2)).len(), 0);
        assert_eq!(overlapping(&list, &(3..7)).len(), 2);
        assert_eq!(overlapping(&list, &(9..12)).len(), 0);
        let mut out = Spans::new();
        push_outside((0..12, HighlightClass::Ident), &list, &mut out);
        assert_eq!(
            out,
            [
                (0..2, HighlightClass::Ident),
                (4..6, HighlightClass::Ident),
                (9..12, HighlightClass::Ident)
            ]
        );
    }
}
