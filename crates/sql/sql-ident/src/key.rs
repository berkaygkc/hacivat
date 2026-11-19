//! Identifier keys (HV-INTRO-14, R-M4-28, R-M4-41): how a catalog name and a typed probe turn
//! into the one lookup key the model, the resolver, DDL and the tools compare.
//!
//! An [`IdentKey`] is built only here: by [`match_key`] for a probe or a catalog name, and by
//! [`FoldRule::key`] for a stored catalog name under its namespace's policy (the two agree on
//! every text `match_key` keys). It is never displayed and never persisted. The fold is the
//! server's own rule, never the process locale (ASCII-only casing, INV-3).
//!
//! The rules (HV-INTRO-14 rule 4, R-M4-41; M4 Task 3):
//! - **PostgreSQL** and its wire forks: unquoted ASCII letters are lowered and the text is cut to
//!   63 bytes ([`crate::fold`]); non-ASCII letters are never folded on a UTF-8 server. On another
//!   (or an unknown) server encoding the server lowers high-bit letters by its own locale, so
//!   unquoted non-ASCII input is [`ServerDependent`] (rule 3), and so is a quoted non-ASCII text
//!   longer than the cut (the cut counts server-encoding bytes).
//! - **MySQL/MariaDB**: database and table names (views and triggers too, which the manuals group
//!   with tables) compare exactly under `lower_case_table_names = 0` and ASCII-caselessly under 1
//!   and 2; every other name compares ASCII-caselessly. A caseless text holding a non-ASCII
//!   **cased** letter is [`ServerDependent`]: the server's Unicode-aware comparison is not pinned
//!   until Task 17's probe (`// ## m4-t17-mysql-fold`). Backticks never change the comparison.
//! - **SQLite**: ASCII-caseless, never more (ver/01 `#sqlite-case`).
//! - Every other dialect: no rule is pinned, so every text is [`ServerDependent`].
//!
//! Nothing here calls a Unicode or locale case mapping: the only fold is ASCII.

use std::sync::Arc;

use crate::fold::{PG_MAX_IDENT_BYTES, fold};

/// The dialect a key belongs to (re-exported: [`IdentCtx`] names it).
pub use sql_dialect::DialectId;

/// A lookup key: a name folded by its dialect's rule for its [`NameClass`]. Built only by
/// [`match_key`] and [`FoldRule::key`]; never displayed, never persisted.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IdentKey(Arc<str>);

impl IdentKey {
    /// The folded text (for map lookups and tests; never for display).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// `Hash`/`Eq` of `IdentKey` are those of the `str` it holds, so maps keyed by it can be probed
// with a borrowed `&str` of already-folded text.
impl std::borrow::Borrow<str> for IdentKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// What the matching rule of one server depends on besides its dialect: MySQL/MariaDB
/// `lower_case_table_names` (0, 1 or 2) and the server encoding (PostgreSQL non-UTF-8 encodings
/// make non-ASCII input `ServerDependent`, HV-INTRO-14 rule 3).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdentCtx {
    pub dialect: DialectId,
    pub lower_case_table_names: u8,
    pub encoding: Option<Arc<str>>,
}

impl IdentCtx {
    /// The context of `dialect` with `lower_case_table_names = 0` and no known encoding.
    pub fn new(dialect: DialectId) -> Self {
        IdentCtx {
            dialect,
            lower_case_table_names: 0,
            encoding: None,
        }
    }
}

/// The class of the name being matched: MySQL folds database and table names by
/// `lower_case_table_names` and everything else ASCII-caselessly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NameClass {
    Database,
    Schema,
    Table,
    Column,
    Routine,
    Other,
}

/// The stored per-namespace fold policy of catalog names (the model spike's rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FoldRule {
    /// Byte equality.
    Exact,
    /// ASCII letters compare case-insensitively; every other character exactly.
    AsciiCaseless,
}

impl FoldRule {
    /// The stored rule of catalog names of `class` (R-M4-41): PostgreSQL exact (catalog names
    /// are already folded); MySQL/MariaDB database and table names exact under
    /// `lower_case_table_names = 0` and ASCII-caseless under 1 and 2, every other class
    /// ASCII-caseless; SQLite ASCII-caseless. `None` where no rule is pinned (another dialect, an
    /// unknown `lower_case_table_names`).
    pub fn for_names(d: DialectId, ctx: &IdentCtx, class: NameClass) -> Option<FoldRule> {
        match d {
            DialectId::Postgres | DialectId::PgWire(_) => Some(FoldRule::Exact),
            DialectId::Sqlite => Some(FoldRule::AsciiCaseless),
            DialectId::MySql | DialectId::MariaDb => {
                if !follows_lctn(class) {
                    return Some(FoldRule::AsciiCaseless);
                }
                match ctx.lower_case_table_names {
                    0 => Some(FoldRule::Exact),
                    1 | 2 => Some(FoldRule::AsciiCaseless),
                    _ => None,
                }
            }
            // Generic, SQL Server, ClickHouse, Oracle and any later dialect: not pinned in M4.
            _ => None,
        }
    }

    /// The stored key of the catalog name `name`; shares `name`'s allocation when folding
    /// changes nothing.
    pub fn key(self, name: &Arc<str>) -> IdentKey {
        match self {
            FoldRule::AsciiCaseless if name.bytes().any(|b| b.is_ascii_uppercase()) => {
                IdentKey(Arc::from(name.to_ascii_lowercase()))
            }
            _ => IdentKey(name.clone()),
        }
    }

    /// Whether the stored name `name` has the key text `key` under this rule, without
    /// allocating.
    pub fn matches(self, name: &str, key: &str) -> bool {
        match self {
            FoldRule::Exact => name == key,
            FoldRule::AsciiCaseless => name.eq_ignore_ascii_case(key),
        }
    }
}

/// Unicode titlecase letters (`Lt`, UnicodeData.txt 16.0): cased letters that are neither
/// `Lowercase` nor `Uppercase`.
const TITLECASE: [(u32, u32); 10] = [
    (0x01C5, 0x01C5),
    (0x01C8, 0x01C8),
    (0x01CB, 0x01CB),
    (0x01F2, 0x01F2),
    (0x1F88, 0x1F8F),
    (0x1F98, 0x1F9F),
    (0x1FA8, 0x1FAF),
    (0x1FBC, 0x1FBC),
    (0x1FCC, 0x1FCC),
    (0x1FFC, 0x1FFC),
];

/// Whether `text` holds a non-ASCII **cased** letter (Unicode `Lowercase`, `Uppercase` or `Lt`):
/// a letter a server's Unicode-aware comparison might fold. A property test only; nothing is
/// case-mapped.
pub fn has_non_ascii_cased(text: &str) -> bool {
    !text.is_ascii()
        && text.chars().any(|c| {
            !c.is_ascii()
                && (c.is_lowercase()
                    || c.is_uppercase()
                    || TITLECASE
                        .iter()
                        .any(|&(a, b)| (a..=b).contains(&u32::from(c))))
        })
}

/// The server's matching rule for this text is not pinned (R-M4-41): a lookup answers `Found`
/// only on an exact stored-name match where the server compares case-insensitively (MySQL,
/// MariaDB), and `Unknown(ServerDependent)` otherwise, never `Missing`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ServerDependent;

/// The key of `text` (a catalog name, or user input with its `quoted` flag) of `class` under
/// `d` and `ctx` (the module docs give the rules). `Err(ServerDependent)`: the server's rule for
/// this text is not pinned, so a lookup answers `Found` only on an exact stored-name match where
/// the server compares case-insensitively, and `Unknown(ServerDependent)` otherwise — never
/// `Missing`.
pub fn match_key(
    d: DialectId,
    ctx: &IdentCtx,
    text: &str,
    quoted: bool,
    class: NameClass,
) -> Result<IdentKey, ServerDependent> {
    let rule = FoldRule::for_names(d, ctx, class).ok_or(ServerDependent)?;
    match d {
        DialectId::Postgres | DialectId::PgWire(_) => {
            if !text.is_ascii() && !is_utf8_encoding(ctx.encoding.as_deref()) {
                // The server lowers unquoted high-bit letters by its own locale, and cuts at 63
                // bytes of its own encoding.
                if !quoted || text.len() > PG_MAX_IDENT_BYTES {
                    return Err(ServerDependent);
                }
            }
            Ok(IdentKey(fold(d, text, quoted)))
        }
        DialectId::MySql | DialectId::MariaDb => {
            if rule == FoldRule::AsciiCaseless && has_non_ascii_cased(text) {
                return Err(ServerDependent);
            }
            Ok(rule.key(&Arc::from(text)))
        }
        _ => Ok(rule.key(&Arc::from(text))),
    }
}

/// MySQL/MariaDB name classes whose comparison follows `lower_case_table_names`: databases
/// (schemas) and tables. Callers pass [`NameClass::Table`] for views and triggers too (the
/// manuals: "database, table, and trigger names" depend on the file system's case rule).
fn follows_lctn(class: NameClass) -> bool {
    matches!(
        class,
        NameClass::Database | NameClass::Schema | NameClass::Table
    )
}

/// Whether a PostgreSQL `server_encoding` is UTF-8 (`UTF8`, also accepted as `UTF-8`). An
/// unknown encoding is not.
fn is_utf8_encoding(encoding: Option<&str>) -> bool {
    encoding.is_some_and(|e| e.eq_ignore_ascii_case("UTF8") || e.eq_ignore_ascii_case("UTF-8"))
}
