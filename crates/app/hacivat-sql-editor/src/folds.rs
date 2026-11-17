//! The foldable ranges of a script (UI-EDIT-10 candidates; M3c Task 32; NOTES surface 15),
//! computed by the worker from one analysis: the editor split's statements and the lexing.
//!
//! - **Statements:** from the first token (a leading comment stays above the fold) to the
//!   terminator.
//! - **Blocks:** a parenthesised group (from its `(` to before its `)`, so the closing line stays
//!   visible); `BEGIN … END`, `CASE … END`, `IF … END IF` and `LOOP … END LOOP` (to before the
//!   `END`); and a dollar-quoted body (the whole string), whose text is lexed again so the
//!   PL/pgSQL blocks inside it fold too (fix round 1, M6). A parenthesis left open at a `;`
//!   folds nothing; a `BEGIN` before `;` or `TRANSACTION`, `WORK`, … is a transaction command;
//!   an `IF` folds only once its `THEN` is seen (`IF(` and `IF EXISTS` never do); a `CASE` left
//!   open at a `;` never pairs with a later plain `END` (fix round 1, M12: only `END CASE`
//!   closes a `CASE` statement across `;`).
//! - **Comments:** a block comment, and a run of lines holding only a `--` comment; a blank
//!   line ends a run (fix round 1, I4: the lexer folds consecutive breaks into one token).
//! - **Regions:** `-- region <name>` to its `-- endregion` (`#region` too), nested; one left
//!   open folds nothing.
//!
//! Not folded (carried to the M5 lexer work): `INSERT … VALUES` rows, and a MySQL procedure's
//! `BEGIN … END` under a client `DELIMITER` (the whole-text lexing reads `END$$` as a dollar
//! string's start).
//!
//! Each spans at least [`MIN_FOLD_LINES`] lines (NOTES ruling M3c-43). Sorted by start, the
//! outer one first on a shared start; a range found twice is kept once.

use std::ops::Range;
use std::sync::Arc;

use hacivat_text::fold::{MIN_FOLD_LINES, map_fold};
use hacivat_text::{FoldKind, FoldRange};
use sql_dialect::DialectCtx;
use sql_lexer::{LexState, Lexed, TokenFlags, TokenKind, lex};
use sql_split::DocSnapshot;

use crate::service::DocShared;

fn range(r: sql_text::TextRange) -> Range<usize> {
    usize::from(r.start())..usize::from(r.end())
}

/// `r` spans at least [`MIN_FOLD_LINES`] lines of `text`.
fn tall(text: &str, r: &Range<usize>) -> bool {
    text.get(r.clone())
        .is_some_and(|s| s.bytes().filter(|&b| b == b'\n').count() + 1 >= MIN_FOLD_LINES as usize)
}

/// The line breaks in `s` (CRLF counts once, a lone CR once).
fn breaks(s: &str) -> usize {
    let b = s.as_bytes();
    (0..b.len())
        .filter(|&i| b[i] == b'\n' || (b[i] == b'\r' && b.get(i + 1) != Some(&b'\n')))
        .count()
}

/// The region marker a line comment is: `Some(true)` opens, `Some(false)` closes.
fn region_marker(comment: &str) -> Option<bool> {
    let body = comment.strip_prefix("--")?.trim_start();
    let body = body.strip_prefix('#').unwrap_or(body);
    let word: String = body
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .collect::<String>()
        .to_ascii_lowercase();
    match word.as_str() {
        "region" => Some(true),
        "endregion" => Some(false),
        _ => None,
    }
}

fn is(word: &str, any: &[&str]) -> bool {
    any.iter().any(|w| word.eq_ignore_ascii_case(w))
}

/// A word that, right after `BEGIN`, makes it a transaction command rather than a block.
fn begins_a_transaction(word: &str) -> bool {
    is(
        word,
        &[
            "TRANSACTION",
            "WORK",
            "TRAN",
            "ISOLATION",
            "READ",
            "DEFERRABLE",
            "NOT",
        ],
    )
}

/// What opened a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Opener {
    Begin,
    Case,
    If,
    Loop,
}

/// An open block: what opened it, where, how many `;` had been seen then, and (an `IF`)
/// whether its `THEN` came.
#[derive(Debug, Clone, Copy)]
struct Open {
    kind: Opener,
    start: usize,
    semis: u32,
    confirmed: bool,
}

/// Collects foldable ranges over one text (the document, or a dollar-quoted body lexed again).
struct Scan<'a> {
    /// The whole document (ranges are checked against it).
    doc: &'a str,
    out: Vec<FoldRange>,
}

impl Scan<'_> {
    fn push(&mut self, r: Range<usize>, kind: FoldKind) {
        if r.start < r.end && tall(self.doc, &r) {
            self.out.push(FoldRange { range: r, kind });
        }
    }

    /// The blocks, parentheses, block comments and dollar bodies of `lexed` (a lexing of
    /// `text`, which starts at `base` in the document); at the top level (`top`) also the
    /// comment runs and regions.
    fn tokens(&mut self, text: &str, lexed: &Lexed, base: usize, ctx: &DialectCtx, top: bool) {
        let word = |i: usize| text.get(range(lexed.range(i))).unwrap_or("");
        let next_significant = |i: usize| (i + 1..lexed.len()).find(|&j| !lexed.is_trivia(j));
        let mut parens: Vec<usize> = Vec::new();
        let mut blocks: Vec<Open> = Vec::new();
        let mut regions: Vec<usize> = Vec::new();
        let mut semis = 0u32;
        // A token an `END x` already read (its `x`).
        let mut skip: Option<usize> = None;
        // A run of comment-only lines: its first comment's start, its last comment's end, its
        // line count; and whether the current line holds code, or anything at all, so far.
        let mut run: Option<(usize, usize, u32)> = None;
        let mut line_has_code = false;
        let mut line_is_blank = true;
        let close_run = |run: &mut Option<(usize, usize, u32)>, scan: &mut Self| {
            if let Some((s, e, n)) = run.take()
                && n >= MIN_FOLD_LINES
            {
                scan.out.push(FoldRange {
                    range: s..e,
                    kind: FoldKind::Comment,
                });
            }
        };
        for i in 0..lexed.len() {
            let local = range(lexed.range(i));
            let r = base + local.start..base + local.end;
            match lexed.kind(i) {
                TokenKind::Newline => {
                    // A blank line ends a run of comment lines: an empty line before this
                    // break, or more than one break in the token.
                    if line_is_blank || breaks(word(i)) > 1 {
                        close_run(&mut run, self);
                    }
                    line_has_code = false;
                    line_is_blank = true;
                    continue;
                }
                TokenKind::Whitespace => continue,
                TokenKind::LineComment if top => {
                    line_is_blank = false;
                    match region_marker(word(i)) {
                        Some(true) => {
                            regions.push(r.start);
                            close_run(&mut run, self);
                        }
                        Some(false) => {
                            if let Some(s) = regions.pop() {
                                self.push(s..r.end, FoldKind::Region);
                            }
                            close_run(&mut run, self);
                        }
                        None if !line_has_code => {
                            run = match run {
                                Some((s, _, n)) => Some((s, r.end, n + 1)),
                                None => Some((r.start, r.end, 1)),
                            };
                        }
                        None => close_run(&mut run, self),
                    }
                    line_has_code = true;
                    continue;
                }
                _ => {}
            }
            close_run(&mut run, self);
            line_has_code = true;
            line_is_blank = false;
            if skip == Some(i) {
                skip = None;
                continue;
            }
            match lexed.kind(i) {
                TokenKind::BlockComment => self.push(r, FoldKind::Comment),
                TokenKind::DollarString => {
                    self.push(r.clone(), FoldKind::Block);
                    if top {
                        self.dollar_body(word(i), lexed.flags(i), r.start, ctx);
                    }
                }
                TokenKind::LParen => parens.push(r.start),
                TokenKind::RParen => {
                    if let Some(s) = parens.pop() {
                        self.push(s..r.start, FoldKind::Block);
                    }
                }
                TokenKind::Semicolon => {
                    parens.clear();
                    semis += 1;
                    // An `IF` whose `THEN` never came (`DROP … IF EXISTS`) is no block.
                    while blocks
                        .last()
                        .is_some_and(|b| b.kind == Opener::If && !b.confirmed)
                    {
                        blocks.pop();
                    }
                }
                TokenKind::Word => {
                    let w = word(i);
                    let next = next_significant(i);
                    let open = |kind, confirmed| Open {
                        kind,
                        start: r.start,
                        semis,
                        confirmed,
                    };
                    if is(w, &["BEGIN"]) {
                        let tx = next.is_none_or(|j| {
                            lexed.kind(j) == TokenKind::Semicolon || begins_a_transaction(word(j))
                        });
                        if !tx {
                            blocks.push(open(Opener::Begin, true));
                        }
                    } else if is(w, &["CASE"]) {
                        blocks.push(open(Opener::Case, true));
                    } else if is(w, &["IF"]) {
                        if next.is_some_and(|j| lexed.kind(j) != TokenKind::LParen) {
                            blocks.push(open(Opener::If, false));
                        }
                    } else if is(w, &["LOOP"]) {
                        blocks.push(open(Opener::Loop, true));
                    } else if is(w, &["THEN"]) {
                        if let Some(b) = blocks.last_mut()
                            && b.kind == Opener::If
                        {
                            b.confirmed = true;
                        }
                    } else if is(w, &["END"]) {
                        let closer = next.filter(|&j| lexed.kind(j) == TokenKind::Word);
                        let target = match closer.map(word) {
                            Some(x) if is(x, &["IF"]) => Some(Opener::If),
                            Some(x) if is(x, &["LOOP"]) => Some(Opener::Loop),
                            Some(x) if is(x, &["CASE"]) => Some(Opener::Case),
                            Some(x) if is(x, &["WHILE", "REPEAT", "FOR"]) => {
                                skip = closer;
                                continue;
                            }
                            _ => None,
                        };
                        let found = match target {
                            Some(kind) => {
                                skip = closer;
                                blocks.iter().rposition(|b| b.kind == kind)
                            }
                            // A plain `END` closes a `BEGIN`, or a `CASE` of this statement.
                            None => blocks.iter().rposition(|b| {
                                b.kind == Opener::Begin
                                    || (b.kind == Opener::Case && b.semis == semis)
                            }),
                        };
                        if let Some(k) = found {
                            let s = blocks[k].start;
                            blocks.truncate(k);
                            self.push(s..r.start, FoldKind::Block);
                        }
                    }
                }
                _ => {}
            }
        }
        close_run(&mut run, self);
    }

    /// A dollar-quoted string's body (between its tags), lexed alone: its blocks fold too.
    fn dollar_body(&mut self, token: &str, flags: TokenFlags, at: usize, ctx: &DialectCtx) {
        let Some(tag_len) = token[1..].find('$').map(|k| k + 2) else {
            return;
        };
        let end = if flags.contains(TokenFlags::UNTERMINATED) || token.len() < tag_len * 2 {
            token.len()
        } else {
            token.len() - tag_len
        };
        let Some(body) = token.get(tag_len..end) else {
            return;
        };
        let inner = lex(
            body,
            crate::highlight::profile(ctx),
            ctx.modes,
            &LexState::Initial,
        );
        self.tokens(body, &inner, at + tag_len, ctx, false);
    }
}

/// The foldable ranges of `doc` (module docs).
pub fn folding_ranges(doc: &DocSnapshot) -> Vec<FoldRange> {
    let text = doc.text();
    let mut scan = Scan {
        doc: text,
        out: Vec::new(),
    };
    for st in doc.editor_split().statements() {
        let start = st.head.first_token.map_or(st.strict.start(), |t| t.start());
        let end = st
            .terminator
            .map_or(st.strict.end(), |t| t.end().max(st.strict.end()));
        scan.push(usize::from(start)..usize::from(end), FoldKind::Statement);
    }
    scan.tokens(text, doc.lexed(), 0, doc.ctx(), true);
    let mut out = scan.out;
    out.sort_by(|a, b| {
        a.range
            .start
            .cmp(&b.range.start)
            .then(b.range.end.cmp(&a.range.end))
    });
    out.dedup_by(|b, a| a.range == b.range);
    out
}

/// The foldable ranges at `version`: the latest analysis's, mapped through the changes since
/// as the editor maps a fold (`hacivat_text::fold::map_fold`: text typed at a range's start goes
/// above it, at its end stays outside; fix round 1, M10); a range whose first character an edit
/// deleted goes. None when the analysis is newer or the changes since are no longer known. The
/// editor asks once per version and again when an analysis lands.
pub(crate) fn at(sh: &DocShared, version: u64) -> Arc<[FoldRange]> {
    let Some(a) = sh.latest() else {
        return Arc::from([]);
    };
    if a.version == version {
        return a.folds.clone();
    }
    if a.version > version {
        return Arc::from([]);
    }
    sh.with_descs(a.version, version, |descs| {
        a.folds
            .iter()
            .filter_map(|f| {
                let mut r = f.range.clone();
                for d in descs {
                    r = map_fold(d, &r)?;
                }
                Some(FoldRange {
                    range: r,
                    kind: f.kind,
                })
            })
            .collect::<Vec<_>>()
    })
    .map_or_else(|| Arc::from([]), Arc::from)
}
