//! Template variables (D-053; Task 6): `${var}`, `%var%` and `<var>` are detected, never
//! substituted; [`crate::params::scan_params`] reports them with `ParamDiag::TemplateVariable`.
//!
//! Detection reads code only: a pattern whose bytes touch a string, a quoted identifier, a
//! dollar body, a comment, `COPY` data or a client command is not a template (text there is
//! data, and Hacivat has no "substitute inside strings" switch). An executable comment's body
//! is code to the server, so it is read too. `%var%` and `<var>` also read as arithmetic and
//! comparisons (`10%n%3`, `a<b>c`), so they count only in operand position: after the start,
//! an operator, `(`, `,`, `[` or an expression keyword, and before the end, an operator, a
//! closing bracket, `,`, `;` or a keyword. `${` has no other meaning in any dialect.

use std::sync::Arc;

use sql_dialect::LexDialect;
use sql_lexer::{LexState, Lexed, TokenKind, exec_comment, lex};
use sql_text::TextRange;

use crate::params::{expression_keyword, size_of};

/// The template syntax a variable was written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TemplatePattern {
    /// `${var}`
    DollarBrace,
    /// `%var%`
    Percent,
    /// `<var>`
    Angle,
}

/// One template variable in a unit.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TemplateVar {
    pub range: TextRange,
    pub name: Arc<str>,
    pub pattern: TemplatePattern,
}

// ## m3b-t06-params

/// Whether token kind `k` holds data or commentary rather than code.
fn is_data(k: TokenKind) -> bool {
    matches!(
        k,
        TokenKind::String
            | TokenKind::DollarString
            | TokenKind::BitString
            | TokenKind::HexString
            | TokenKind::QuotedIdent
            | TokenKind::UnicodeQuotedIdent
            | TokenKind::LineComment
            | TokenKind::BlockComment
            | TokenKind::HintComment
            | TokenKind::InlineData
            | TokenKind::ClientCommand
            | TokenKind::ExecutableComment
    )
}

/// A `%var%`/`<var>` name byte (as the lexer's identifiers: ASCII letters, digits, `_`, and
/// every non-ASCII byte).
fn name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// The template variables of `text` (lexed as `lexed`), in text order; `base` is `text`'s
/// offset in the unit. Executable comment bodies are lexed with `profile` and read as code.
pub(crate) fn find(
    text: &str,
    lexed: &Lexed,
    profile: &LexDialect,
    modes: sql_dialect::LexModes,
    base: usize,
    out: &mut Vec<TemplateVar>,
) {
    let b = text.as_bytes();
    let mut code = vec![false; b.len()];
    for i in 0..lexed.len() {
        let r = lexed.range(i);
        let (s, e) = (usize::from(r.start()), usize::from(r.end()));
        if !is_data(lexed.kind(i)) {
            code[s..e].fill(true);
        }
    }
    let all_code = |s: usize, e: usize| code[s..e].iter().all(|c| *c);
    let mut at = 0;
    while at < b.len() {
        let found = match b[at] {
            b'$' if code[at] => dollar_brace(b, at).filter(|(e, _)| all_code(at, *e)),
            b'%' | b'<' if code[at] => delimited(text, lexed, at),
            _ => None,
        };
        match found {
            Some((end, pattern)) => {
                let name = match pattern {
                    TemplatePattern::DollarBrace => &text[at + 2..end - 1],
                    _ => &text[at + 1..end - 1],
                };
                out.push(TemplateVar {
                    range: TextRange::new(size_of(base + at), size_of(base + end)),
                    name: Arc::from(name),
                    pattern,
                });
                at = end;
            }
            None => at += 1,
        }
    }
    // Executable comment bodies run on the server: read them as code.
    for i in 0..lexed.len() {
        if let Some(ec) = exec_comment(text, lexed, i) {
            let (s, e) = (usize::from(ec.body.start()), usize::from(ec.body.end()));
            let body = &text[s..e];
            let sub = lex(body, profile, modes, &LexState::Initial);
            find(body, &sub, profile, modes, base + s, out);
        }
    }
}

/// `${…}` at `at`: the end offset (after `}`); the name holds no line break and may be empty.
fn dollar_brace(b: &[u8], at: usize) -> Option<(usize, TemplatePattern)> {
    if b.get(at + 1) != Some(&b'{') {
        return None;
    }
    let mut i = at + 2;
    while let Some(&c) = b.get(i) {
        match c {
            b'}' => return Some((i + 1, TemplatePattern::DollarBrace)),
            b'\n' | b'\r' => return None,
            _ => i += 1,
        }
    }
    None
}

/// `%name%` or `<name>` at `at`, in operand position.
fn delimited(text: &str, lexed: &Lexed, at: usize) -> Option<(usize, TemplatePattern)> {
    let b = text.as_bytes();
    let (close, pattern) = match b[at] {
        b'%' => (b'%', TemplatePattern::Percent),
        _ => (b'>', TemplatePattern::Angle),
    };
    let first = *b.get(at + 1)?;
    if !(first.is_ascii_alphabetic() || first == b'_' || first >= 0x80) {
        return None;
    }
    let mut i = at + 1;
    while b.get(i).is_some_and(|c| name_byte(*c)) {
        i += 1;
    }
    if b.get(i) != Some(&close) {
        return None;
    }
    let end = i + 1;
    // The name is one word token, and the delimiters are operator bytes.
    let name_tok = lexed.token_at(size_of(at + 1))?;
    let r = lexed.range(name_tok);
    if lexed.kind(name_tok) != TokenKind::Word
        || usize::from(r.start()) != at + 1
        || usize::from(r.end()) != i
    {
        return None;
    }
    let open_tok = lexed.token_at(size_of(at))?;
    let close_tok = lexed.token_at(size_of(i))?;
    if lexed.kind(open_tok) != TokenKind::Op || lexed.kind(close_tok) != TokenKind::Op {
        return None;
    }
    (operand_before(text, lexed, open_tok, at) && operand_after(lexed, close_tok, end))
        .then_some((end, pattern))
}

/// The token before `at` (the opener's own operator token when it started earlier, else the
/// previous significant token) leaves an operand position.
fn operand_before(text: &str, lexed: &Lexed, open_tok: usize, at: usize) -> bool {
    if usize::from(lexed.range(open_tok).start()) < at {
        return true;
    }
    let prev = (0..open_tok).rev().find(|j| !lexed.is_trivia(*j));
    match prev {
        None => true,
        Some(j) => match lexed.kind(j) {
            TokenKind::Op | TokenKind::LParen | TokenKind::Comma | TokenKind::LBracket => true,
            TokenKind::Word => expression_keyword(text, lexed, j),
            _ => false,
        },
    }
}

/// What follows `end` (the closer's own operator token when it runs on, else the next
/// significant token) ends an operand.
fn operand_after(lexed: &Lexed, close_tok: usize, end: usize) -> bool {
    if usize::from(lexed.range(close_tok).end()) > end {
        return true;
    }
    let next = (close_tok + 1..lexed.len()).find(|j| !lexed.is_trivia(*j));
    match next {
        None => true,
        Some(j) => match lexed.kind(j) {
            TokenKind::Op
            | TokenKind::RParen
            | TokenKind::RBracket
            | TokenKind::Comma
            | TokenKind::Semicolon => true,
            TokenKind::Word => lexed.keyword(j).is_some(),
            _ => false,
        },
    }
}
