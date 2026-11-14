//! The lexer's resumable state (Task 4).

use smallvec::SmallVec;

use crate::token::TokenKind;

/// Where lexing stands at a point of the text: lexing the rest from this state gives the
/// tokens a full lex gives there.
///
/// A full lex ends outside `Initial` only inside an unterminated construct, which then runs to
/// the end of the text. A state is exact at a line start (a checkpoint): no delimiter is half
/// read there (a backslash escape, a doubled quote, `*/` or `$tag$` never spans a line feed
/// that ends the text). Elsewhere it is not: a cut between the quotes of `''`, or after the
/// backslash of `\'`, resumes differently. The kinds and ranges after a line-start resume
/// equal a full lex's (`tests/props.rs`); so do the `AmbiguousCommentEnd` flags, because the
/// comment states carry the running reading of their body.
///
/// Lexing from a state also takes its text's offset 0 as a document start for the
/// byte-order-mark rule (`lex`); a resume inside a document never starts at a BOM.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum LexState {
    #[default]
    Initial,
    /// Inside a block comment, `depth` levels deep (PostgreSQL's nest; others stay at 1).
    InBlockComment { depth: u32 },
    /// Inside a MySQL/MariaDB optimizer hint `/*+ … */`; `body` is where a server that reads
    /// the hint's quotes stands.
    InHintComment { body: BodyReading },
    /// Inside a MySQL/MariaDB executable comment `/*! … */`: `depth` is 1, or 2 inside the one
    /// nested comment it may hold (the skipped reading, which the token follows); `body` is
    /// where a server that runs the comment stands.
    InExecutableComment { depth: u32, body: BodyReading },
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

/// Where a MySQL/MariaDB server that *runs* an executable comment (or reads a hint) stands in
/// its body: it lexes the body as code, so a `*/` inside quotes, an identifier or a line
/// comment does not close it (`comments`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BodyReading {
    /// In code: the next `*/` outside quotes and comments closes the body.
    Code,
    /// Inside a string or quoted identifier opened by `quote` (`'`, `"` or `` ` ``; the
    /// session's modes say whether a backslash escapes in it).
    Quoted { quote: u8 },
    /// Inside a `#` or `-- ` comment, to the line feed.
    LineComment,
    /// Inside the one nested `/* … */` (opaque to its first `*/`).
    Nested,
    /// The running reading no longer matches the token: it closed before the token's `*/`, or
    /// met an inner executable comment the lexer does not follow. The comment's end is
    /// ambiguous.
    Diverged,
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
