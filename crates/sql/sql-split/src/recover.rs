//! Editor-mode recovery (SQL-SPLIT-03 step 1; SPLIT-02 step 8; Task 8):
//!
//! - **Missing semicolons.** Inside a statement at block depth 0, a statement-start word that is
//!   the first significant token on its line is a heuristic boundary, unless the token before it
//!   or the statement's head says it continues the statement (`sql_dialect::SplitProfile`).
//!   Blank lines alone never split (D-05): `select a,`↵↵`b from t` stays one statement. Editor
//!   mode splits there into `Heuristic` items; execution mode keeps one item and reports an
//!   `AmbiguousBoundary` (`MissingSemicolon`), so Run asks (for PostgreSQL, the parser oracle
//!   may settle it, R-M3-24).
//! - **The stability rule.** A string, comment, dollar quote or block that is still open at the
//!   text's end would swallow everything after it. Editor mode instead ends the statement that
//!   holds it at the end of that line, and splits the rest of the text again on its own, as
//!   provisional `Heuristic` items: the statements below an unterminated quote keep their
//!   boundaries while it is being typed. Execution mode never does this.
//! - **Comments.** A statement's smart range takes the comments directly above it (no blank
//!   line between) and a comment on its terminator's line.

use sql_dialect::DialectId;
use sql_lexer::{LexState, TokenKind, lex};
use sql_text::{TextRange, TextSize};

use crate::client;
use crate::item::{ClientSegment, Confidence, Item, Split, Stmt, StmtHead};
use crate::splitter::{Analysis, Part, RawItem, SplitStart, Tok, analyze, lexer_for, size};

/// How many times the stability rule re-splits a tail that is itself unterminated.
const MAX_RECOVERY: u32 = 4;

/// The trivia between two significant tokens (or an item and the next token).
#[derive(Debug, Clone, Default)]
pub(crate) struct Trivia {
    /// Each comment, with the line breaks between it and what precedes it in the run.
    pub comments: Vec<(TextRange, u32)>,
    /// Line breaks after the last comment (or since the run's start).
    pub tail_breaks: u32,
    /// Line breaks in the whole run, those inside block comments included: the next token is
    /// the first significant token on its line when this is not 0.
    pub breaks: u32,
}

impl Trivia {
    pub(crate) fn push(&mut self, text: &str, t: &Tok) {
        let breaks = line_breaks(&text[t.range]);
        if t.is_comment() {
            self.comments.push((t.range, self.tail_breaks));
            self.tail_breaks = 0;
            self.breaks += breaks;
        } else if t.kind == TokenKind::Newline {
            self.tail_breaks += breaks;
            self.breaks += breaks;
        }
    }
}

/// `\n`s, and `\r`s not followed by `\n`.
fn line_breaks(s: &str) -> u32 {
    let b = s.as_bytes();
    let mut n = 0;
    for (i, c) in b.iter().enumerate() {
        if *c == b'\n' || (*c == b'\r' && b.get(i + 1) != Some(&b'\n')) {
            n += 1;
        }
    }
    n
}

/// Splits a trivia run between what precedes it and the next token: the end of the comments
/// that trail the preceding item (on its line, when `after_item` and the next token is on a
/// later line), and the comments that lead the next token (no blank line between them and it).
pub(crate) fn attach(
    trivia: &Trivia,
    after_item: bool,
    next_on_same_line: bool,
) -> (Option<TextSize>, Option<TextRange>) {
    let cs = &trivia.comments;
    let mut trailing = 0;
    if after_item && !next_on_same_line {
        while trailing < cs.len() && cs[trailing].1 == 0 {
            trailing += 1;
        }
    }
    let trailing_end = (trailing > 0).then(|| cs[trailing - 1].0.end());
    let mut first = cs.len();
    let mut gap = trivia.tail_breaks;
    while first > trailing && gap <= 1 {
        first -= 1;
        gap = cs[first].1;
    }
    let leading = (first < cs.len()).then(|| {
        let last = cs[cs.len() - 1].0;
        TextRange::new(cs[first].0.start(), last.end())
    });
    (trailing_end, leading)
}

/// A heuristic boundary inside a statement: part A ends before it, part B starts at `at`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Cut {
    pub a_end: TextSize,
    pub a_smart_end: TextSize,
    pub b_leading: Option<TextRange>,
    pub b_head: StmtHead,
    /// The start word.
    pub at: TextRange,
}

/// A `WITH` that is a boundary only if a CTE name follows.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PendingWith {
    pub cut: Cut,
}

/// The boundary before `t`, whose statement's last significant token ends at `a_end`.
pub(crate) fn cut_at(trivia: &Trivia, a_end: TextSize, t: &Tok) -> Cut {
    let (trailing, leading) = attach(trivia, true, false);
    Cut {
        a_end,
        a_smart_end: trailing.unwrap_or(a_end).max(a_end),
        b_leading: leading,
        b_head: StmtHead {
            first_keyword: t.keyword.map(|k| k.id),
            first_token: Some(t.range),
        },
        at: t.range,
    }
}

/// The editor view of `a` over `text`: heuristic items at the cuts, uncertain statements
/// `Heuristic`, and the stability rule after an unterminated construct.
pub(crate) fn editor(a: &Analysis, text: &str, depth: u32) -> Split {
    let mut split = a.base_split();
    let overlapped = a.overlapped_items(&split);
    for (n, item) in a.items.iter().enumerate() {
        match item {
            RawItem::Empty(r) => split.items.push(Item::Empty { range: *r }),
            RawItem::Stmt(raw) if raw.cuts.is_empty() => {
                let confidence = if raw.uncertain || overlapped[n] {
                    Confidence::Heuristic
                } else {
                    Confidence::Exact
                };
                split.items.push(Item::Statement(a.stmt(
                    text,
                    raw,
                    Part::whole(raw),
                    confidence,
                )));
            }
            RawItem::Stmt(raw) => {
                let mut part = Part {
                    strict: TextRange::new(raw.strict.start(), raw.cuts[0].a_end),
                    leading: raw.leading,
                    smart_end: raw.cuts[0].a_smart_end,
                    terminator: None,
                    head: raw.head,
                    copy_data: None,
                };
                for (i, cut) in raw.cuts.iter().enumerate() {
                    split.items.push(Item::Statement(a.stmt(
                        text,
                        raw,
                        part,
                        Confidence::Heuristic,
                    )));
                    let next = raw.cuts.get(i + 1);
                    part = Part {
                        strict: TextRange::new(
                            cut.at.start(),
                            next.map_or(raw.strict.end(), |n| n.a_end),
                        ),
                        leading: cut.b_leading,
                        smart_end: next.map_or(raw.smart_end, |n| n.a_smart_end),
                        terminator: if next.is_none() { raw.terminator } else { None },
                        head: cut.b_head,
                        copy_data: if next.is_none() { raw.copy_data } else { None },
                    };
                }
                split.items.push(Item::Statement(a.stmt(
                    text,
                    raw,
                    part,
                    Confidence::Heuristic,
                )));
            }
        }
    }
    if let Some(anchor) = a.unterminated
        && depth < MAX_RECOVERY
    {
        stability(a, text, anchor, depth, &mut split);
    }
    split
}

/// The stability rule: the statement holding `anchor` ends on its line; the text after it is
/// split again, provisionally.
fn stability(a: &Analysis, text: &str, anchor: TextSize, depth: u32, split: &mut Split) {
    let l = client::next_line_start(text, usize::from(anchor));
    if l >= text.len() {
        return;
    }
    let ctx = a
        .ctx
        .unwrap_or_else(|| sql_dialect::DialectCtx::new(DialectId::Generic, None));
    // Everything from the next line on is split again below: drop what the first pass made of
    // it (heuristic pieces of the open statement, client commands after it).
    let next = size(l);
    split.items.retain(|it| match it {
        Item::Statement(s) => s.strict.start() < next,
        Item::Empty { range } => range.start() < next,
    });
    split.client.retain(|c| c.range.start() < next);
    if let Some(Item::Statement(s)) = split.items.last_mut()
        && s.strict.start() <= anchor
        && anchor < s.strict.end()
    {
        let start = usize::from(s.strict.start());
        let line_end = client::line_end(text, usize::from(anchor));
        let end = last_significant_end(text, start, line_end, &ctx).max(start + 1);
        let strict = TextRange::new(s.strict.start(), size(end));
        let delimiter = a.delimiter_at(s.strict.start());
        *s = Stmt {
            strict,
            smart: TextRange::new(s.smart.start(), size(end)),
            terminator: None,
            leading_comments: s.leading_comments,
            confidence: Confidence::Heuristic,
            head: s.head,
            key: crate::key::StmtKey::of(&ctx, delimiter.as_deref(), &text[strict]),
            copy_data: None,
        };
    } else if let Some(Item::Statement(s)) = split.items.last_mut() {
        // An unterminated comment trailing the last statement: its smart range keeps only the
        // comment's first line.
        let line_end = size(client::line_end(text, usize::from(anchor)));
        if s.smart.end() > line_end {
            s.smart = TextRange::new(s.smart.start(), line_end.max(s.strict.end()));
        }
    }
    let tail_text = &text[l..];
    let start = SplitStart {
        lex: LexState::Initial,
        delimiter: a.delimiter_at(anchor),
    };
    let tail = analyze(tail_text, None, &ctx, a.flavor, &start);
    let tail_split = editor(&tail, tail_text, depth + 1);
    let shift = size(l);
    for item in tail_split.items {
        split.items.push(match item {
            Item::Empty { range } => Item::Empty {
                range: range + shift,
            },
            Item::Statement(s) => Item::Statement(Stmt {
                confidence: Confidence::Heuristic,
                ..shift_stmt(s, shift)
            }),
        });
    }
    split
        .client
        .extend(tail_split.client.into_iter().map(|c| ClientSegment {
            range: c.range + shift,
            ..c
        }));
}

/// `s` with every range moved by `by`.
pub(crate) fn shift_stmt(s: Stmt, by: TextSize) -> Stmt {
    Stmt {
        strict: s.strict + by,
        smart: s.smart + by,
        terminator: s.terminator.map(|r| r + by),
        leading_comments: s.leading_comments.map(|r| r + by),
        head: StmtHead {
            first_keyword: s.head.first_keyword,
            first_token: s.head.first_token.map(|r| r + by),
        },
        copy_data: s.copy_data.map(|d| crate::item::CopyData {
            rows: d.rows + by,
            end: d.end.map(|r| r + by),
        }),
        ..s
    }
}

/// The end of the last significant token of `text[start..end]` lexed on its own (`start` if
/// it has none).
fn last_significant_end(
    text: &str,
    start: usize,
    end: usize,
    ctx: &sql_dialect::DialectCtx,
) -> usize {
    let lexed = lex(
        &text[start..end],
        lexer_for(ctx),
        ctx.modes,
        &LexState::Initial,
    );
    (0..lexed.len())
        .rev()
        .find(|i| !lexed.is_trivia(*i))
        .map_or(start, |i| start + usize::from(lexed.range(i).end()))
}
