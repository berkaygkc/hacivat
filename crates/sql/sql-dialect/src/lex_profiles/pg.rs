//! PostgreSQL's lexer profiles (PostgreSQL-wire servers reuse them). The rules follow the
//! server's scanner (`scan.l`, PostgreSQL License, read in the vendored libpg_query): `--`
//! comments end at `\n` or `\r`, block comments nest, `$tag$` strings, `E''`/`B''`/`X''`/`N''`/
//! `U&''` prefixes, `$n` parameters, the operator rule. Version gates: PostgreSQL 15 makes an
//! identifier right after a number or parameter an error ("trailing junk"); PostgreSQL 16 adds
//! `0x`/`0o`/`0b` integers and `_` separators.

use super::{
    At, Backslash, Colon, CommentRule, Dollar, DoubleQuote, IdentRule, Junk, LexDialect,
    NumberRule, OperatorRule, Prefixes, Question, QuoteRule, Radix, Separators, SigilRule,
    SpaceRule,
};
use crate::id::DialectId;
use crate::version::ServerVersion;

/// `op_chars`: the characters an operator is made of.
const OPERATOR_CHARS: &[u8] = b"~!@#^&|`?+-*/%<>=";
/// A multi-character operator holding one of these may end in `+` or `-`.
const SIGN_KEEPERS: &[u8] = b"~!@#^&|`?%";
/// `NAMEDATALEN`: an operator of this many bytes or more is an error.
const MAX_OPERATOR: usize = 64;

const fn profile(name: &'static str, numbers: NumberRule) -> LexDialect {
    LexDialect {
        name,
        keywords: DialectId::Postgres,
        space: SpaceRule {
            vertical_tab: true,
            bom_anywhere: false,
        },
        comments: CommentRule {
            line_ends_at_cr: true,
            dash_needs_space: false,
            hash: false,
            nested: true,
            unterminated_ok: false,
            open_at_end_is_operators: false,
            executable: None,
            hints: false,
        },
        quotes: QuoteRule {
            double: DoubleQuote::Ident,
            backtick_idents: false,
            bracket_idents: false,
            backslash: Backslash::UnlessStandardConformingStrings,
            prefixes: Prefixes {
                escape: true,
                bit: true,
                hex: true,
                national: true,
                unicode: true,
            },
            dollar: true,
            hex_blob_checked: false,
        },
        numbers,
        idents: IdentRule {
            dollar_start: false,
            dollar_continue: true,
        },
        sigils: SigilRule {
            question: Question::Operator,
            colon: Colon::Postgres,
            at: At::Operator,
            dollar: Dollar::Positional,
            hash_param: false,
        },
        operators: OperatorRule::Postgres {
            chars: OPERATOR_CHARS,
            sign_keepers: SIGN_KEEPERS,
            max_len: MAX_OPERATOR,
        },
        // `\` is the scanner's `{other}`: a lone character the grammar rejects.
        illegal: b"\\",
    }
}

/// PostgreSQL 16 and later (and an unknown version).
pub(super) static PG16: LexDialect = profile(
    "pg-16",
    NumberRule {
        radix: Radix::Postgres,
        separators: Separators::BetweenDigits,
        junk: Junk::IdentStart,
        digit_leading_idents: false,
        dot_dot: true,
    },
);

/// PostgreSQL 15: trailing junk is an error, no separators or radix prefixes yet.
pub(super) static PG15: LexDialect = profile(
    "pg-15",
    NumberRule {
        radix: Radix::Decimal,
        separators: Separators::None,
        junk: Junk::IdentStart,
        digit_leading_idents: false,
        dot_dot: true,
    },
);

/// PostgreSQL 14 and earlier: `123abc` is `123` then `abc`.
pub(super) static PG14: LexDialect = profile(
    "pg-14",
    NumberRule {
        radix: Radix::Decimal,
        separators: Separators::None,
        junk: Junk::Allowed,
        digit_leading_idents: false,
        dot_dot: true,
    },
);

/// The profile a server of `version` lexes with.
pub(super) fn for_version(version: Option<ServerVersion>) -> &'static LexDialect {
    match version.map(|v| v.major) {
        None => &PG16,
        Some(major) if major >= 16 => &PG16,
        Some(15) => &PG15,
        Some(_) => &PG14,
    }
}
