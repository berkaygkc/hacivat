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
    DollarTag,
    ExecutableComment,
    OptimizerHint,
    ClientCommand,
    Error,
}

/// The class of a token (Task 4). Task 1b stub: every token is `Error` (the stub lexer only
/// makes `Unknown` tokens).
pub fn token_class(kind: TokenKind, flags: TokenFlags, kw: Option<KwEntry>) -> TokenClass {
    let _ = (kind, flags, kw);
    TokenClass::Error
}
