//! SQLite's lexer profile, from its tokenizer (`tokenize.c`, public domain; Hacivat links the
//! bundled SQLite 3.5x through rusqlite): whitespace without the vertical tab and with the BOM
//! anywhere, `--` comments ending at a line feed, non-nesting block comments that may run to
//! the end without an error, `"…"`, `` `…` `` and `[…]` identifiers, `X'…'` blobs, `0x`
//! integers and loose `_` separators, an identifier character after a number is an error,
//! `?NNN` and `$`/`@`/`:`/`#` variables with Tcl `::` segments and `(…)` suffixes.

use super::{
    At, Backslash, Colon, CommentRule, Dollar, DoubleQuote, IdentRule, Junk, LexDialect,
    NumberRule, OperatorRule, Prefixes, Question, QuoteRule, Radix, Separators, SigilRule,
    SpaceRule,
};
use crate::id::DialectId;

/// Multi-character operators, longest first.
const MULTI: &[&[u8]] = &[
    b"->>", b"||", b"<=", b">=", b"<>", b"!=", b"==", b"<<", b">>", b"->",
];
/// Single-character operators (`!` only in `!=`; no `^`).
const SINGLE: &[u8] = b"+-*/%&|~<>=";

pub(super) static SQLITE: LexDialect = LexDialect {
    name: "sqlite",
    keywords: DialectId::Sqlite,
    space: SpaceRule {
        vertical_tab: false,
        bom_anywhere: true,
    },
    comments: CommentRule {
        line_ends_at_cr: false,
        dash_needs_space: false,
        hash: false,
        nested: false,
        unterminated_ok: true,
        open_at_end_is_operators: true,
        executable: None,
        hints: false,
    },
    quotes: QuoteRule {
        double: DoubleQuote::Ident,
        backtick_idents: true,
        bracket_idents: true,
        backslash: Backslash::Never,
        prefixes: Prefixes {
            escape: false,
            bit: false,
            hex: true,
            national: false,
            unicode: false,
        },
        dollar: false,
        hex_blob_checked: true,
    },
    numbers: NumberRule {
        radix: Radix::Hex,
        separators: Separators::Loose,
        junk: Junk::IdentChar,
        digit_leading_idents: false,
        dot_dot: false,
    },
    idents: IdentRule {
        dollar_start: false,
        dollar_continue: true,
    },
    sigils: SigilRule {
        question: Question::NumberedParam,
        colon: Colon::Tcl,
        at: At::Tcl,
        dollar: Dollar::Tcl,
        hash_param: true,
    },
    operators: OperatorRule::Table {
        multi: MULTI,
        single: SINGLE,
    },
    illegal: b"\\]{}^",
};
