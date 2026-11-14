//! Tokens and the lexed struct of arrays (Task 4).

use std::ops::Range;

use sql_dialect::KwEntry;
use sql_text::{TextRange, TextSize};

use crate::state::LexState;

/// What a token is.
#[repr(u8)]
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Whitespace,
    Newline,
    LineComment,
    BlockComment,
    HintComment,
    /// MySQL/MariaDB `/*! … */`, with its version gate (`exec_comment_gate`).
    ExecutableComment,
    Word,
    QuotedIdent,
    UnicodeQuotedIdent,
    String,
    DollarString,
    Number,
    BitString,
    HexString,
    /// psql `COPY … FROM STDIN` data up to `\.`.
    InlineData,
    PositionalParam,
    QuestionParam,
    NamedParam,
    UserVar,
    SystemVar,
    ClientVar,
    Op,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Semicolon,
    Dot,
    /// psql `\cmd`, mysql `DELIMITER`, sqlite3 `.cmd`.
    ClientCommand,
    Unknown,
}

bitflags::bitflags! {
    /// What a token carries besides its kind.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct TokenFlags: u8 {
        /// A string, comment, dollar string or quoted identifier that never closed.
        const UNTERMINATED = 1;
        /// PostgreSQL ≤ 15: junk after a number.
        const JUNK = 2;
        /// PostgreSQL 16: `_` separators in a number.
        const HAS_SEPARATORS = 4;
        const BOM = 8;
        const ERROR = 16;
    }
}

/// A lexing error, in a token's range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LexError {
    pub range: TextRange,
    pub code: LexErrorCode,
}

/// Why a range did not lex cleanly.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LexErrorCode {
    UnterminatedString,
    UnterminatedComment,
    UnterminatedDollar,
    UnterminatedQuotedIdent,
    InvalidEscape,
    Nul,
}

/// A lexed text: token kinds, sorted start offsets, flags and keywords, one entry per token;
/// the tokens tile the text without gaps (lossless).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lexed {
    pub(crate) kinds: Vec<TokenKind>,
    pub(crate) starts: Vec<TextSize>,
    pub(crate) flags: Vec<TokenFlags>,
    pub(crate) keywords: Vec<Option<KwEntry>>,
    pub(crate) errors: Vec<LexError>,
    pub(crate) end_state: LexState,
    /// The text's length (the end of the last token).
    pub(crate) len: TextSize,
}

impl Lexed {
    /// The number of tokens.
    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    pub fn kind(&self, i: usize) -> TokenKind {
        self.kinds[i]
    }

    /// Token `i`'s range: from its start to the next token's (or the text's end).
    pub fn range(&self, i: usize) -> TextRange {
        let end = self.starts.get(i + 1).copied().unwrap_or(self.len);
        TextRange::new(self.starts[i], end)
    }

    pub fn flags(&self, i: usize) -> TokenFlags {
        self.flags[i]
    }

    pub fn keyword(&self, i: usize) -> Option<KwEntry> {
        self.keywords[i]
    }

    /// Whitespace, newlines and comments (an executable comment is not trivia).
    pub fn is_trivia(&self, i: usize) -> bool {
        matches!(
            self.kinds[i],
            TokenKind::Whitespace
                | TokenKind::Newline
                | TokenKind::LineComment
                | TokenKind::BlockComment
                | TokenKind::HintComment
        )
    }

    pub fn errors(&self) -> &[LexError] {
        &self.errors
    }

    /// The state lexing ended in (an unterminated construct), to resume from.
    pub fn end_state(&self) -> &LexState {
        &self.end_state
    }

    /// The token containing `at` (the last token for the text's end); O(log n).
    pub fn token_at(&self, at: TextSize) -> Option<usize> {
        if self.kinds.is_empty() || at > self.len {
            return None;
        }
        Some(self.starts.partition_point(|s| *s <= at).saturating_sub(1))
    }

    /// The tokens overlapping `r`; O(log n).
    pub fn tokens_in(&self, r: TextRange) -> Range<usize> {
        let first = self
            .starts
            .partition_point(|s| *s <= r.start())
            .saturating_sub(1);
        let last = self.starts.partition_point(|s| *s < r.end());
        first.min(last)..last
    }

    /// The deepest parenthesis nesting inside `tokens` (R-M3-35).
    pub fn max_paren_depth(&self, tokens: Range<usize>) -> u16 {
        let (mut depth, mut max) = (0u16, 0u16);
        for kind in &self.kinds[tokens] {
            match kind {
                TokenKind::LParen => {
                    depth = depth.saturating_add(1);
                    max = max.max(depth);
                }
                TokenKind::RParen => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        max
    }

    /// One `Unknown` token over `len` bytes (none for an empty text): the Task 1b stub.
    pub(crate) fn unknown(len: TextSize) -> Lexed {
        let one = len > TextSize::from(0);
        Lexed {
            kinds: if one {
                vec![TokenKind::Unknown]
            } else {
                Vec::new()
            },
            starts: if one {
                vec![TextSize::from(0)]
            } else {
                Vec::new()
            },
            flags: if one {
                vec![TokenFlags::empty()]
            } else {
                Vec::new()
            },
            keywords: if one { vec![None] } else { Vec::new() },
            errors: Vec::new(),
            end_state: LexState::Initial,
            len,
        }
    }
}
