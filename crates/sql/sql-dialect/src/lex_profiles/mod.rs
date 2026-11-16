//! Table-driven lexer profiles, one per dialect (the lexical half of SQL-LEX-01, read by the
//! SQL-LEX-02 lexer; Task 4).
//!
//! A profile is data: which quotes, comments, numbers, sigils and operators a server's lexer
//! knows. The `sql-lexer` engine reads it and holds no per-dialect behaviour of its own.
//! Version gates select a static profile (PostgreSQL 14, 15 and 16+ numbers); session modes
//! (`LexModes`) are applied by the lexer on top of the profile. `SqlServer`, `ClickHouse` and
//! `Oracle` have no profile in M3a (`UnsupportedDialect`).

mod generic;
mod mariadb;
mod mysql;
mod pg;
mod sqlite;

use crate::id::{DialectCtx, DialectId};

/// One dialect's lexical rules.
#[non_exhaustive]
#[derive(Debug, PartialEq, Eq)]
pub struct LexDialect {
    /// The profile's name, for diagnostics (`pg-16`, `mysql`, …).
    pub name: &'static str,
    /// The keyword table words are looked up in.
    pub keywords: DialectId,
    pub space: SpaceRule,
    pub comments: CommentRule,
    pub quotes: QuoteRule,
    pub numbers: NumberRule,
    pub idents: IdentRule,
    pub sigils: SigilRule,
    pub operators: OperatorRule,
    /// ASCII bytes that start no token in this dialect: each one is an `Unknown` token.
    pub illegal: &'static [u8],
}

impl LexDialect {
    /// The profile's name, for diagnostics.
    pub fn name(&self) -> &'static str {
        self.name
    }
}

/// Whitespace beyond space, tab, form feed and the line breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpaceRule {
    /// A vertical tab (U+000B) is whitespace (not in SQLite).
    pub vertical_tab: bool,
    /// A UTF-8 byte-order mark is whitespace anywhere (SQLite), not only at offset 0.
    pub bom_anywhere: bool,
}

/// Comments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommentRule {
    /// A `--` comment also ends at a carriage return (PostgreSQL's `non_newline` is `[^\n\r]`);
    /// otherwise only at a line feed (MySQL, MariaDB, SQLite).
    pub line_ends_at_cr: bool,
    /// `--` starts a comment only before whitespace, a control character or the end (MySQL,
    /// MariaDB): `1--1` is `1 - -1`.
    pub dash_needs_space: bool,
    /// `#` starts a line comment (MySQL, MariaDB).
    pub hash: bool,
    /// Block comments nest, counted (PostgreSQL); otherwise the first `*/` closes.
    pub nested: bool,
    /// A block comment may run to the end of the text without an error (SQLite).
    pub unterminated_ok: bool,
    /// `/*` with nothing after it is the operators `/` and `*` (SQLite).
    pub open_at_end_is_operators: bool,
    /// Executable comments (`/*!`, MariaDB also `/*M!`; SQL-LEX-05).
    pub executable: Option<ExecVendor>,
    /// `/*+ … */` is an optimizer hint (MySQL, MariaDB).
    pub hints: bool,
}

/// Whose executable comments a dialect runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecVendor {
    /// `/*! … */`.
    MySql,
    /// `/*! … */` and `/*M! … */`.
    MariaDb,
}

/// Quoted strings and identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuoteRule {
    pub double: DoubleQuote,
    /// `` `…` `` is a quoted identifier (doubled backticks escape one).
    pub backtick_idents: bool,
    /// `[…]` is a quoted identifier (SQLite; no escape, the first `]` closes).
    pub bracket_idents: bool,
    /// When a plain `'…'` string reads backslash escapes.
    pub backslash: Backslash,
    pub prefixes: Prefixes,
    /// `$tag$ … $tag$` strings (PostgreSQL).
    pub dollar: bool,
    /// `X'…'` must hold an even number of hex digits, else the token is an error (SQLite).
    pub hex_blob_checked: bool,
}

/// What `"…"` is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoubleQuote {
    /// A quoted identifier (doubled quotes escape one).
    Ident,
    /// A string, unless `ANSI_QUOTES` is set (MySQL, MariaDB).
    StringUnlessAnsiQuotes,
}

/// When a plain string reads backslash escapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backslash {
    Never,
    /// PostgreSQL with `standard_conforming_strings` off.
    UnlessStandardConformingStrings,
    /// MySQL and MariaDB without `NO_BACKSLASH_ESCAPES`.
    UnlessNoBackslashEscapes,
}

/// String prefixes (ASCII case-insensitive, directly before the quote).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prefixes {
    /// `E'…'`: backslash escapes (PostgreSQL).
    pub escape: bool,
    /// `B'…'`: a bit string.
    pub bit: bool,
    /// `X'…'`: a hex string.
    pub hex: bool,
    /// `N'…'`: a national string.
    pub national: bool,
    /// `U&'…'` and `U&"…"` (PostgreSQL).
    pub unicode: bool,
}

/// Numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumberRule {
    pub radix: Radix,
    pub separators: Separators,
    /// What directly after a number (or a `$n` parameter) makes it an error (`JUNK`).
    pub junk: Junk,
    /// A word may start with digits (`1e5x`, `123abc` and `0X12` are identifiers): MySQL,
    /// MariaDB.
    pub digit_leading_idents: bool,
    /// `1..2` is `1`, `..`, `2` (PostgreSQL).
    pub dot_dot: bool,
}

/// Non-decimal integer prefixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Radix {
    Decimal,
    /// `0x`, `0o`, `0b`, either case, `_` after the prefix allowed (PostgreSQL 16).
    Postgres,
    /// `0x` and `0b`, lower case only (MySQL, MariaDB).
    LowerHexBin,
    /// `0x` or `0X` (SQLite).
    Hex,
}

/// `_` digit separators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Separators {
    None,
    /// One `_` between two digits (PostgreSQL 16).
    BetweenDigits,
    /// Any `_` after the first digit (SQLite 3.46; the parser checks placement).
    Loose,
}

/// Trailing junk after a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Junk {
    /// None (PostgreSQL ≤ 14: `123abc` is `123 AS abc`).
    Allowed,
    /// An identifier start (PostgreSQL ≥ 15).
    IdentStart,
    /// Any identifier character, `$` included (SQLite).
    IdentChar,
}

/// Unquoted identifiers. ASCII letters and `_` start one and digits continue one in every
/// dialect; every non-ASCII character is an identifier character (the servers' rule: PostgreSQL
/// takes any byte ≥ 0x80, MySQL U+0080..U+FFFF, SQLite any byte ≥ 0x80).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentRule {
    /// `$` may start an identifier (MySQL, MariaDB).
    pub dollar_start: bool,
    /// `$` may continue an identifier (`x$$y` is one word).
    pub dollar_continue: bool,
}

/// Parameters and variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SigilRule {
    pub question: Question,
    pub colon: Colon,
    pub at: At,
    pub dollar: Dollar,
    /// `#name` is a parameter (SQLite).
    pub hash_param: bool,
}

/// What `?` is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Question {
    /// A parameter.
    Param,
    /// A parameter, optionally numbered (`?3`, SQLite).
    NumberedParam,
    /// An operator character (PostgreSQL: `?|`, `?&`).
    Operator,
}

/// What `:` starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colon {
    /// `::`, `:=`, psql `:'v'`/`:"v"`/`:{?v}` client variables, `:name` parameters, else `:`.
    Postgres,
    /// `:=` or `:` (MySQL, MariaDB).
    Operator,
    /// `::` and `:=` operators, `:name` parameters, else `:`.
    NamedParam,
    /// SQLite's variable syntax (with Tcl `::` segments and a `(…)` suffix), else illegal.
    Tcl,
}

/// What `@` starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum At {
    /// An operator character (PostgreSQL).
    Operator,
    /// `@user` and `@@system` variables (MySQL, MariaDB).
    Variables,
    /// SQLite's variable syntax.
    Tcl,
    /// Not a token.
    Illegal,
}

/// What `$` starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dollar {
    /// `$n` parameters (dollar quotes are `QuoteRule::dollar`); else illegal.
    Positional,
    /// SQLite's variable syntax.
    Tcl,
    /// An identifier character (MySQL, MariaDB).
    Ident,
    /// Not a token.
    Illegal,
}

/// Operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorRule {
    /// PostgreSQL: maximal munch over `chars`, cut before an embedded `--` or `/*`; a
    /// multi-character operator ends in `+`/`-` only if it holds one of `sign_keepers`
    /// (otherwise its trailing signs are separate operators); `max_len` bytes or more is an
    /// error (`NAMEDATALEN`).
    Postgres {
        chars: &'static [u8],
        sign_keepers: &'static [u8],
        max_len: usize,
    },
    /// A fixed set: the longest of `multi` that matches, else one of `single`.
    Table {
        multi: &'static [&'static [u8]],
        single: &'static [u8],
    },
}

/// A dialect without a lexer profile in M3a (R19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnsupportedDialect(pub DialectId);

/// The lexer profile for `ctx`: its dialect, with the version gates its server version selects
/// (an unknown version lexes with the newest profile, `alg/10` LEX-01).
pub fn lex_profile(ctx: &DialectCtx) -> Result<&'static LexDialect, UnsupportedDialect> {
    match ctx.id {
        DialectId::Postgres | DialectId::PgWire(_) => Ok(pg::for_version(ctx.version)),
        DialectId::MySql => Ok(&mysql::MYSQL),
        DialectId::MariaDb => Ok(&mariadb::MARIADB),
        DialectId::Sqlite => Ok(&sqlite::SQLITE),
        DialectId::Generic => Ok(&generic::GENERIC),
        DialectId::SqlServer | DialectId::ClickHouse | DialectId::Oracle => {
            Err(UnsupportedDialect(ctx.id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::PgWireFlavor;
    use crate::modes::LexModes;
    use crate::version::ServerVersion;

    fn ctx(id: DialectId, major: Option<u16>) -> DialectCtx {
        DialectCtx {
            id,
            version: major.map(|major| ServerVersion {
                major,
                minor: 0,
                patch: 0,
            }),
            modes: LexModes::default_for(id),
        }
    }

    fn name(id: DialectId, major: Option<u16>) -> &'static str {
        lex_profile(&ctx(id, major)).map(LexDialect::name).unwrap()
    }

    #[test]
    fn postgres_versions_select_their_number_rules() {
        assert_eq!(name(DialectId::Postgres, None), "pg-16");
        assert_eq!(name(DialectId::Postgres, Some(18)), "pg-16");
        assert_eq!(name(DialectId::Postgres, Some(16)), "pg-16");
        assert_eq!(name(DialectId::Postgres, Some(15)), "pg-15");
        assert_eq!(name(DialectId::Postgres, Some(14)), "pg-14");
        assert_eq!(name(DialectId::Postgres, Some(9)), "pg-14");
        // PostgreSQL-wire servers lex as PostgreSQL of their reported version.
        assert_eq!(
            name(DialectId::PgWire(PgWireFlavor::Redshift), Some(8)),
            "pg-14"
        );
        assert_eq!(
            name(DialectId::PgWire(PgWireFlavor::Cockroach), Some(23)),
            "pg-16"
        );
        let pg16 = lex_profile(&ctx(DialectId::Postgres, None)).unwrap();
        let pg15 = lex_profile(&ctx(DialectId::Postgres, Some(15))).unwrap();
        let pg14 = lex_profile(&ctx(DialectId::Postgres, Some(14))).unwrap();
        assert_eq!(pg16.numbers.separators, Separators::BetweenDigits);
        assert_eq!(pg16.numbers.radix, Radix::Postgres);
        assert_eq!(pg15.numbers.separators, Separators::None);
        assert_eq!(pg15.numbers.junk, Junk::IdentStart);
        assert_eq!(pg14.numbers.junk, Junk::Allowed);
    }

    #[test]
    fn each_supported_dialect_has_its_own_profile() {
        assert_eq!(name(DialectId::MySql, Some(8)), "mysql");
        assert_eq!(name(DialectId::MariaDb, Some(11)), "mariadb");
        assert_eq!(name(DialectId::Sqlite, Some(3)), "sqlite");
        assert_eq!(name(DialectId::Generic, None), "generic");
        let mysql = lex_profile(&ctx(DialectId::MySql, None)).unwrap();
        let mariadb = lex_profile(&ctx(DialectId::MariaDb, None)).unwrap();
        assert_eq!(mysql.comments.executable, Some(ExecVendor::MySql));
        assert_eq!(mariadb.comments.executable, Some(ExecVendor::MariaDb));
        assert_eq!(mysql.keywords, DialectId::MySql);
        assert_eq!(mariadb.keywords, DialectId::MariaDb);
        for id in [DialectId::Postgres, DialectId::Sqlite, DialectId::Generic] {
            assert_eq!(
                lex_profile(&ctx(id, None)).unwrap().comments.executable,
                None
            );
        }
    }

    #[test]
    fn engines_without_a_profile_are_unsupported() {
        for id in [
            DialectId::SqlServer,
            DialectId::ClickHouse,
            DialectId::Oracle,
        ] {
            assert_eq!(lex_profile(&ctx(id, None)), Err(UnsupportedDialect(id)));
        }
    }
}
