//! MySQL's lexer profile (MariaDB shares it, with its own executable comments). From the
//! MySQL reference manual and Docker-verified behaviour (`alg/10` LEX-01, LEX-05; ADR 0004
//! step 1a), never from MySQL's GPL sources: `#` and `-- ` comments ending at a line feed,
//! non-nesting block comments, `/*!` executable comments and `/*+` hints, backtick
//! identifiers, `"…"` strings unless `ANSI_QUOTES`, backslash escapes unless
//! `NO_BACKSLASH_ESCAPES`, `0x`/`0b` (lower case) numbers, digit-leading identifiers, `?`,
//! `@user` and `@@system` variables.

use super::{
    At, Backslash, Colon, CommentRule, Dollar, DoubleQuote, ExecVendor, IdentRule, Junk,
    LexDialect, NumberRule, OperatorRule, Prefixes, Question, QuoteRule, Radix, Separators,
    SigilRule, SpaceRule,
};
use crate::id::DialectId;

/// Multi-character operators, longest first.
const MULTI: &[&[u8]] = &[
    b"<=>", b"->>", b"<=", b">=", b"<>", b"!=", b"<<", b">>", b"&&", b"||", b":=", b"->",
];
/// Single-character operators.
const SINGLE: &[u8] = b"=<>!~^&|+-*/%:";

/// The MySQL-family profile under `name`, its keyword table and executable-comment vendor.
pub(super) const fn family(
    name: &'static str,
    keywords: DialectId,
    executable: ExecVendor,
) -> LexDialect {
    LexDialect {
        name,
        keywords,
        space: SpaceRule {
            vertical_tab: true,
            bom_anywhere: false,
        },
        comments: CommentRule {
            line_ends_at_cr: false,
            dash_needs_space: true,
            hash: true,
            nested: false,
            unterminated_ok: false,
            open_at_end_is_operators: false,
            executable: Some(executable),
            hints: true,
        },
        quotes: QuoteRule {
            double: DoubleQuote::StringUnlessAnsiQuotes,
            backtick_idents: true,
            bracket_idents: false,
            backslash: Backslash::UnlessNoBackslashEscapes,
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
            radix: Radix::LowerHexBin,
            separators: Separators::None,
            junk: Junk::Allowed,
            digit_leading_idents: true,
            dot_dot: false,
        },
        idents: IdentRule {
            dollar_start: true,
            dollar_continue: true,
        },
        sigils: SigilRule {
            question: Question::Param,
            colon: Colon::Operator,
            at: At::Variables,
            dollar: Dollar::Ident,
            hash_param: false,
        },
        operators: OperatorRule::Table {
            multi: MULTI,
            single: SINGLE,
        },
        illegal: b"\\",
    }
}

/// MySQL 8.0 and 8.4 (their lexical rules agree; the keyword tables are Task 3's).
pub(super) static MYSQL: LexDialect = family("mysql", DialectId::MySql, ExecVendor::MySql);
