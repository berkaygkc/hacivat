//! The lexer's highlight vocabulary (R-M3-42; the editor adapter maps it to its own classes).

use sql_dialect::KwEntry;

use crate::token::{TokenFlags, TokenKind};

/// What a token looks like to a reader.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenClass {
    Keyword,
    Identifier,
    QuotedIdentifier,
    String,
    Number,
    Comment,
    Operator,
    Punctuation,
    Parameter,
    /// The `$tag$` delimiters of a dollar-quoted string (the adapter splits them off a
    /// `DollarString`, whose class is `String`).
    DollarTag,
    ExecutableComment,
    OptimizerHint,
    ClientCommand,
    Error,
    /// Spaces and line breaks: nothing to paint (Task 4).
    Whitespace,
}

/// The class of a token. A word is a keyword when the dialect's table has it (`kw`); a token
/// the server rejects (`Unknown`, an `ERROR` or `JUNK` flag) is `Error`; an unterminated
/// string or comment keeps its own class, so its colour shows how far it runs (the error is in
/// `Lexed::errors`).
pub fn token_class(kind: TokenKind, flags: TokenFlags, kw: Option<KwEntry>) -> TokenClass {
    if flags.intersects(TokenFlags::ERROR | TokenFlags::JUNK) {
        return TokenClass::Error;
    }
    match kind {
        TokenKind::Whitespace | TokenKind::Newline => TokenClass::Whitespace,
        TokenKind::LineComment | TokenKind::BlockComment => TokenClass::Comment,
        TokenKind::HintComment => TokenClass::OptimizerHint,
        TokenKind::ExecutableComment => TokenClass::ExecutableComment,
        TokenKind::Word if kw.is_some() => TokenClass::Keyword,
        TokenKind::Word => TokenClass::Identifier,
        TokenKind::QuotedIdent | TokenKind::UnicodeQuotedIdent => TokenClass::QuotedIdentifier,
        TokenKind::String
        | TokenKind::DollarString
        | TokenKind::BitString
        | TokenKind::HexString
        | TokenKind::InlineData => TokenClass::String,
        TokenKind::Number => TokenClass::Number,
        TokenKind::PositionalParam
        | TokenKind::QuestionParam
        | TokenKind::NamedParam
        | TokenKind::UserVar
        | TokenKind::SystemVar
        | TokenKind::ClientVar => TokenClass::Parameter,
        TokenKind::Op => TokenClass::Operator,
        TokenKind::LParen
        | TokenKind::RParen
        | TokenKind::LBracket
        | TokenKind::RBracket
        | TokenKind::LBrace
        | TokenKind::RBrace
        | TokenKind::Comma
        | TokenKind::Semicolon
        | TokenKind::Dot => TokenClass::Punctuation,
        TokenKind::ClientCommand => TokenClass::ClientCommand,
        TokenKind::Unknown => TokenClass::Error,
    }
}
