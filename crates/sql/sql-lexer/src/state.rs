//! The lexer's resumable state (Task 4).

use smallvec::SmallVec;

/// Where lexing stands at a point of the text: resuming from an end state lexes the rest as a
/// full lex would.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum LexState {
    #[default]
    Initial,
    InBlockComment {
        depth: u32,
    },
    InString {
        rule: StringRule,
    },
    InDollar {
        tag: SmallVec<[u8; 16]>,
    },
    InCopyData,
}

/// How a string literal's backslashes and quotes read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StringRule {
    Standard,
    /// PostgreSQL `E'…'`.
    Escape,
    /// MySQL without `NO_BACKSLASH_ESCAPES`.
    Backslash,
    /// PostgreSQL `U&'…'`.
    Unicode,
}
