//! Cursor eligibility from a statement's head (R-M3-23; Task 8). This is a fetch strategy, not
//! a safety layer: read-only safety stays with the guard.

use sql_dialect::DialectId;
use sql_lexer::{LexState, Lexed, TokenFlags, TokenKind, lex};
use sql_text::TextRange;

use crate::document::DocSnapshot;
use crate::splitter::lexer_for;

/// Whether the unit at `range` may stay suspended as a console cursor: its head reads
/// (`SELECT`, `VALUES`, `TABLE`, MySQL/MariaDB `SHOW`, or `WITH` with no `INSERT`, `UPDATE`,
/// `DELETE` or `MERGE` keyword), and it holds no `INTO`, no locking clause (`FOR UPDATE`,
/// `FOR SHARE`, `FOR NO KEY UPDATE`, `FOR KEY SHARE`, `LOCK IN SHARE MODE`) and no executable
/// comment.
///
/// The range is lexed on its own, from the start, exactly as the server receives it, so a
/// keyword hidden in a comment does not count and one in an executable comment does (any
/// executable comment makes the unit ineligible). It also fails closed on what it cannot read:
/// a `;` (more than one statement), an unterminated token, a comment whose end is ambiguous,
/// a range that is not the document's text.
pub fn cursor_eligible(doc: &DocSnapshot, range: TextRange) -> bool {
    let Some(text) = doc
        .text()
        .get(usize::from(range.start())..usize::from(range.end()))
    else {
        return false;
    };
    let ctx = doc.ctx();
    let lexed = lex(text, lexer_for(ctx), ctx.modes, &LexState::Initial);
    if !lexed.errors().is_empty() {
        return false;
    }
    let words: Vec<&str> = (0..lexed.len())
        .filter(|i| !lexed.is_trivia(*i))
        .map(|i| significant(text, &lexed, i))
        .collect();
    if words.contains(&"") {
        return false;
    }
    let mysql = matches!(ctx.id, DialectId::MySql | DialectId::MariaDb);
    let is = |w: &str, x: &str| w.eq_ignore_ascii_case(x);
    let Some(head) = words.first() else {
        return false;
    };
    let reads =
        ["SELECT", "VALUES", "TABLE"].iter().any(|x| is(head, x)) || (mysql && is(head, "SHOW"));
    let with = is(head, "WITH");
    if !(reads || with) {
        return false;
    }
    for (i, w) in words.iter().enumerate() {
        let next = words.get(i + 1).copied().unwrap_or("");
        let after = words.get(i + 2).copied().unwrap_or("");
        if is(w, "INTO") || is(w, ";") {
            return false;
        }
        if with
            && ["INSERT", "UPDATE", "DELETE", "MERGE"]
                .iter()
                .any(|x| is(w, x))
        {
            return false;
        }
        let locking = is(w, "FOR")
            && (is(next, "UPDATE")
                || is(next, "SHARE")
                || (is(next, "NO") && is(after, "KEY"))
                || (is(next, "KEY") && is(after, "SHARE")));
        if locking || (is(w, "LOCK") && is(next, "IN")) {
            return false;
        }
    }
    true
}

/// A significant token's text for the head rules: a word's text, `;`, `-` for any other
/// token; `""` (ineligible) for an executable comment or an unterminated token.
fn significant<'t>(text: &'t str, lexed: &Lexed, i: usize) -> &'t str {
    let kind = lexed.kind(i);
    if kind == TokenKind::ExecutableComment || lexed.flags(i).contains(TokenFlags::UNTERMINATED) {
        return "";
    }
    match kind {
        TokenKind::Word => &text[lexed.range(i)],
        TokenKind::Semicolon => ";",
        _ => "-",
    }
}
