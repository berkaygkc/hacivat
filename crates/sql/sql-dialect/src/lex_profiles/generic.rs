//! The generic (ANSI) lexer profile, for a dialect without its own (`alg/10` LEX-01 edge
//! cases): `'…'` strings and `"…"` identifiers with doubled quotes, `--` comments, non-nesting
//! block comments, no backslash escapes, `?` and `:name` parameters.

use super::{
    At, Backslash, Colon, CommentRule, Dollar, DoubleQuote, IdentRule, Junk, LexDialect,
    NumberRule, OperatorRule, Prefixes, Question, QuoteRule, Radix, Separators, SigilRule,
    SpaceRule,
};
use crate::id::DialectId;

/// Multi-character operators, longest first.
const MULTI: &[&[u8]] = &[
    b"->>", b"<=", b">=", b"<>", b"!=", b"||", b"::", b":=", b"=>", b"->",
];
/// Single-character operators.
const SINGLE: &[u8] = b"+-*/%=<>!~^&|:";

pub(super) static GENERIC: LexDialect = LexDialect {
    name: "generic",
    keywords: DialectId::Generic,
    space: SpaceRule {
        vertical_tab: true,
        bom_anywhere: false,
    },
    comments: CommentRule {
        line_ends_at_cr: true,
        dash_needs_space: false,
        hash: false,
        nested: false,
        unterminated_ok: false,
        open_at_end_is_operators: false,
        executable: None,
        hints: false,
    },
    quotes: QuoteRule {
        double: DoubleQuote::Ident,
        backtick_idents: false,
        bracket_idents: false,
        backslash: Backslash::Never,
        prefixes: Prefixes {
            escape: false,
            bit: true,
            hex: true,
            national: true,
            unicode: false,
        },
        dollar: false,
        hex_blob_checked: false,
    },
    numbers: NumberRule {
        radix: Radix::Decimal,
        separators: Separators::None,
        junk: Junk::Allowed,
        digit_leading_idents: false,
        dot_dot: false,
    },
    idents: IdentRule {
        dollar_start: false,
        dollar_continue: true,
    },
    sigils: SigilRule {
        question: Question::Param,
        colon: Colon::NamedParam,
        at: At::Illegal,
        dollar: Dollar::Illegal,
        hash_param: false,
    },
    operators: OperatorRule::Table {
        multi: MULTI,
        single: SINGLE,
    },
    illegal: b"\\#`",
};
