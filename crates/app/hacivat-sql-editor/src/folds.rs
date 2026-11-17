//! The foldable ranges of a script (UI-EDIT-10 candidates; M3c Task 32; NOTES surface 15),
//! computed by the worker from one analysis: the editor split's statements and the lexing.
//!
//! - **Statements:** from the first token (a leading comment stays above the fold) to the
//!   terminator.
//! - **Blocks:** a parenthesised group (from its `(` to before its `)`, so the closing line stays
//!   visible), `BEGIN … END` and `CASE … END` (to before the `END`), and a dollar-quoted body
//!   (the whole string). A parenthesis left open at a `;` folds nothing.
//! - **Comments:** a block comment, and a run of lines holding only a `--` comment.
//! - **Regions:** `-- region <name>` to its `-- endregion` (`#region` too), nested; one left
//!   open folds nothing.
//!
//! Each spans at least [`MIN_FOLD_LINES`] lines (NOTES ruling M3c-43). Sorted by start, the
//! outer one first on a shared start; a range found twice is kept once.

use std::ops::Range;
use std::sync::Arc;

use hacivat_text::fold::MIN_FOLD_LINES;
use hacivat_text::{FoldKind, FoldRange, RangeKind};
use sql_lexer::TokenKind;
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

/// A word that, right after `BEGIN`, makes it a transaction command rather than a block.
fn begins_a_transaction(word: &str) -> bool {
    [
        "TRANSACTION",
        "WORK",
        "TRAN",
        "ISOLATION",
        "READ",
        "DEFERRABLE",
        "NOT",
    ]
    .iter()
    .any(|w| word.eq_ignore_ascii_case(w))
}

/// A word that, right after `END`, closes a compound statement this scan does not open.
fn ends_a_compound(word: &str) -> bool {
    ["IF", "LOOP", "WHILE", "REPEAT", "FOR"]
        .iter()
        .any(|w| word.eq_ignore_ascii_case(w))
}

/// The foldable ranges of `doc` (module docs).
pub fn folding_ranges(doc: &DocSnapshot) -> Vec<FoldRange> {
    let text = doc.text();
    let mut out: Vec<FoldRange> = Vec::new();
    let push = |r: Range<usize>, kind: FoldKind, out: &mut Vec<FoldRange>| {
        if r.start < r.end && tall(text, &r) {
            out.push(FoldRange { range: r, kind });
        }
    };
    for st in doc.editor_split().statements() {
        let start = st.head.first_token.map_or(st.strict.start(), |t| t.start());
        let end = st
            .terminator
            .map_or(st.strict.end(), |t| t.end().max(st.strict.end()));
        push(
            usize::from(start)..usize::from(end),
            FoldKind::Statement,
            &mut out,
        );
    }

    let lexed = doc.lexed();
    let word = |i: usize| text.get(range(lexed.range(i))).unwrap_or("");
    // The next token that is not trivia.
    let next_significant = |i: usize| (i + 1..lexed.len()).find(|&j| !lexed.is_trivia(j));
    let mut parens: Vec<usize> = Vec::new();
    let mut blocks: Vec<usize> = Vec::new();
    let mut regions: Vec<usize> = Vec::new();
    // A run of comment-only lines: its first comment's start, its last comment's end, its
    // line count; and whether the current line holds anything but blanks so far.
    let mut run: Option<(usize, usize, u32)> = None;
    let mut line_has_code = false;
    let mut line_is_blank = true;
    let close_run = |run: &mut Option<(usize, usize, u32)>, out: &mut Vec<FoldRange>| {
        if let Some((s, e, n)) = run.take()
            && n >= MIN_FOLD_LINES
        {
            out.push(FoldRange {
                range: s..e,
                kind: FoldKind::Comment,
            });
        }
    };
    for i in 0..lexed.len() {
        let r = range(lexed.range(i));
        match lexed.kind(i) {
            TokenKind::Newline => {
                // A blank line ends a run of comment lines.
                if line_is_blank {
                    close_run(&mut run, &mut out);
                }
                line_has_code = false;
                line_is_blank = true;
                continue;
            }
            TokenKind::Whitespace => continue,
            TokenKind::LineComment => {
                line_is_blank = false;
                match region_marker(word(i)) {
                    Some(true) => {
                        regions.push(r.start);
                        close_run(&mut run, &mut out);
                    }
                    Some(false) => {
                        if let Some(s) = regions.pop() {
                            push(s..r.end, FoldKind::Region, &mut out);
                        }
                        close_run(&mut run, &mut out);
                    }
                    None if !line_has_code => {
                        run = match run {
                            Some((s, _, n)) => Some((s, r.end, n + 1)),
                            None => Some((r.start, r.end, 1)),
                        };
                    }
                    None => close_run(&mut run, &mut out),
                }
                line_has_code = true;
                continue;
            }
            _ => {}
        }
        close_run(&mut run, &mut out);
        line_has_code = true;
        line_is_blank = false;
        match lexed.kind(i) {
            TokenKind::BlockComment => push(r, FoldKind::Comment, &mut out),
            TokenKind::DollarString => push(r, FoldKind::Block, &mut out),
            TokenKind::LParen => parens.push(r.start),
            TokenKind::RParen => {
                if let Some(s) = parens.pop() {
                    push(s..r.start, FoldKind::Block, &mut out);
                }
            }
            TokenKind::Semicolon => parens.clear(),
            TokenKind::Word => {
                let w = word(i);
                if w.eq_ignore_ascii_case("BEGIN") {
                    let next = next_significant(i);
                    let tx = next.is_none_or(|j| {
                        lexed.kind(j) == TokenKind::Semicolon || begins_a_transaction(word(j))
                    });
                    if !tx {
                        blocks.push(r.start);
                    }
                } else if w.eq_ignore_ascii_case("CASE") {
                    blocks.push(r.start);
                } else if w.eq_ignore_ascii_case("END") {
                    let compound = next_significant(i).is_some_and(|j| {
                        lexed.kind(j) == TokenKind::Word && ends_a_compound(word(j))
                    });
                    if !compound && let Some(s) = blocks.pop() {
                        push(s..r.start, FoldKind::Block, &mut out);
                    }
                }
            }
            _ => {}
        }
    }
    close_run(&mut run, &mut out);
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
/// as containers when the editor is ahead of it (a range one edit deleted goes); none when the
/// analysis is newer or the changes since are no longer known. The editor asks once per
/// version.
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
                    r = d.map_range(r, RangeKind::Container)?;
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
