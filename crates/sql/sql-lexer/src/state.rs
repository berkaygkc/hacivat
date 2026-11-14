//! The lexer's resumable state (Task 4).

use smallvec::SmallVec;

use crate::token::TokenKind;

/// Where lexing stands at a point of the text: lexing the rest from this state gives the
/// tokens a full lex gives there.
///
/// A full lex ends outside `Initial` only inside an unterminated construct, which then runs to
/// the end of the text. A state is exact at a line start (a checkpoint): no delimiter is half
/// read there (a backslash escape, a doubled quote, `*/` or `$tag$` never spans a line feed
/// that ends the text).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum LexState {
    #[default]
    Initial,
    /// Inside a block comment, `depth` levels deep (PostgreSQL's nest; others stay at 1).
    InBlockComment { depth: u32 },
    /// Inside a MySQL/MariaDB optimizer hint `/*+ … */`.
    InHintComment,
    /// Inside a MySQL/MariaDB executable comment `/*! … */`: `depth` is 1, or 2 inside the one
    /// nested comment it may hold.
    InExecutableComment { depth: u32 },
    /// Inside a string literal of `kind` (`String`, `BitString` or `HexString`) that `quote`
    /// closes, read by `rule`.
    InString {
        rule: StringRule,
        quote: u8,
        kind: TokenKind,
    },
    /// Inside a quoted identifier of `kind` (`QuotedIdent` or `UnicodeQuotedIdent`) that
    /// `close` closes.
    InQuotedIdent { close: u8, kind: TokenKind },
    /// Inside a dollar-quoted string opened by `$tag$` (`tag` without its dollars).
    InDollar { tag: SmallVec<[u8; 16]> },
    /// Inside psql `COPY … FROM STDIN` data, before its `\.` line.
    InCopyData,
}

/// How a string literal's backslashes and quotes read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StringRule {
    /// A doubled quote is one quote; a backslash is text.
    Standard,
    /// PostgreSQL `E'…'` (and plain strings with `standard_conforming_strings` off): a
    /// backslash escapes the next character; a doubled quote is one quote.
    Escape,
    /// MySQL/MariaDB without `NO_BACKSLASH_ESCAPES`: as `Escape`.
    Backslash,
    /// PostgreSQL `U&'…'`: as `Standard` (the parser reads its escapes).
    Unicode,
    /// Bit and hex strings: the first quote closes.
    Raw,
}

impl StringRule {
    /// A backslash escapes the next character.
    pub(crate) fn backslash_escapes(self) -> bool {
        matches!(self, StringRule::Escape | StringRule::Backslash)
    }

    /// A doubled quote is one quote.
    pub(crate) fn doubled_quotes(self) -> bool {
        !matches!(self, StringRule::Raw)
    }
}
