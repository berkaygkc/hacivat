//! Token words for movement (UI-EDIT-06; Task 24): the current version's tokens on a line.
//!
//! One word per token: a string, a quoted identifier or a dollar-quoted string whole, quotes
//! and tags included (Task 25's double click reads them to select a string's content), an
//! operator such as `::` or `->>` whole. Comments (and a gated-off executable comment, which is
//! a comment) are left out, so the character classes split their prose into words; a running
//! executable comment gives its opener, the tokens of its body and its `*/`. Client commands
//! and `COPY` rows are left to the character classes too: the whole-text lexing is not the
//! split's reading inside them. White space is never listed.
//!
//! `None` (the character classes apply) whenever the analysis is not of the editor's version
//! under the current SQL context.

use std::ops::Range;
use std::rc::Rc;

use hacivat_text::TokenOracle;
use smallvec::SmallVec;
use sql_lexer::{
    GateStatus, TokenClass, TokenFlags, TokenKind, exec_comment, exec_comment_gate, token_class,
};
use sql_text::{TextRange, TextSize};

use crate::service::DocShared;
use crate::worker::Analysis;

fn size(n: usize) -> TextSize {
    TextSize::from(u32::try_from(n).unwrap_or(u32::MAX))
}

fn range(r: TextRange) -> Range<usize> {
    usize::from(r.start())..usize::from(r.end())
}

/// The words of `line` in analysis `a` (module docs).
pub(crate) fn on_line(a: &Analysis, line: Range<usize>) -> SmallVec<[Range<usize>; 16]> {
    let (text, lexed) = (a.doc.text(), a.doc.lexed());
    let mut out = SmallVec::new();
    let line = line.start.min(text.len())..line.end.min(text.len());
    if lexed.is_empty() || line.is_empty() {
        return out;
    }
    let from = a.regions.partition_point(|(x, _)| x.end <= line.start);
    let regions = &a.regions[from..];
    let in_region = |r: &Range<usize>| {
        regions
            .iter()
            .take_while(|(x, _)| x.start < r.end)
            .any(|(x, _)| r.start < x.end)
    };
    for i in lexed.tokens_in(TextRange::new(size(line.start), size(line.end))) {
        let kind = lexed.kind(i);
        let r = range(lexed.range(i));
        if matches!(
            kind,
            TokenKind::Whitespace
                | TokenKind::Newline
                | TokenKind::LineComment
                | TokenKind::BlockComment
                | TokenKind::HintComment
        ) || in_region(&r)
        {
            continue;
        }
        let exec = kind == TokenKind::ExecutableComment
            && token_class(kind, lexed.flags(i), lexed.keyword(i)) == TokenClass::ExecutableComment;
        if !exec {
            out.push(r);
            continue;
        }
        if exec_comment_gate(text, lexed, i, a.doc.ctx()) == GateStatus::Inactive {
            continue;
        }
        let Some(ec) = exec_comment(text, lexed, i) else {
            out.push(r);
            continue;
        };
        let (opener, body) = (range(ec.opener), range(ec.body));
        out.push(opener);
        let first = a.exec_inner.partition_point(|(x, _)| x.end <= body.start);
        out.extend(
            a.exec_inner[first..]
                .iter()
                .take_while(|(x, _)| x.start < body.end)
                .map(|(x, _)| x.clone()),
        );
        if body.end < r.end {
            out.push(body.end..r.end);
        }
    }
    out
}

/// The document's token oracle.
pub(crate) struct TokenWords {
    shared: Rc<DocShared>,
}

impl TokenWords {
    pub(crate) fn new(shared: Rc<DocShared>) -> Self {
        TokenWords { shared }
    }
}

impl TokenOracle for TokenWords {
    fn words_on_line(&self, line: Range<usize>) -> Option<SmallVec<[Range<usize>; 16]>> {
        let a = self.shared.current()?;
        Some(on_line(&a, line))
    }

    // ## m3b-t31-multi (named exception): the IME's plain-text check (R-M3-64).
    fn in_string_or_comment(&self, at: usize) -> Option<bool> {
        let a = self.shared.current()?;
        Some(in_literal(&a, at))
    }
}

// ## m3b-t31-multi
/// A string (of any kind) or a comment (an executable comment too: its body is not plain text
/// to an input method's mirror).
fn is_literal(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::String
            | TokenKind::DollarString
            | TokenKind::BitString
            | TokenKind::HexString
            | TokenKind::LineComment
            | TokenKind::BlockComment
            | TokenKind::HintComment
            | TokenKind::ExecutableComment
    )
}

/// Text typed at `at` lands inside a string or a comment of analysis `a`: `at` strictly inside
/// one, or at the end of a line comment or of an unterminated one (typing there continues it).
fn in_literal(a: &Analysis, at: usize) -> bool {
    let lexed = a.doc.lexed();
    if at > a.doc.text().len() {
        return false;
    }
    let Some(i) = lexed.token_at(size(at)) else {
        return false;
    };
    let r = range(lexed.range(i));
    if r.start < at && at < r.end && is_literal(lexed.kind(i)) {
        return true;
    }
    let ending = if r.end == at {
        Some(i)
    } else if r.start == at {
        i.checked_sub(1)
    } else {
        None
    };
    ending.is_some_and(|j| {
        let kind = lexed.kind(j);
        range(lexed.range(j)).end == at
            && is_literal(kind)
            && (kind == TokenKind::LineComment || lexed.flags(j).contains(TokenFlags::UNTERMINATED))
    })
}
