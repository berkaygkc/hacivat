//! Identifier keys (HV-INTRO-14, R-M4-28, R-M4-41): how a catalog name and a typed probe turn
//! into the one lookup key the model, the resolver, DDL and the tools compare.
//!
//! An [`IdentKey`] is built only here: by [`match_key`] for a probe or a catalog name, and by
//! [`FoldRule::key`] for a stored catalog name under its namespace's policy (the two agree on
//! every text `match_key` keys). It is never displayed and never persisted. The fold is the
//! server's own rule, never the process locale (ASCII-only casing, INV-3).
//!
//! The rules (HV-INTRO-14 rule 4, R-M4-41; M4 Task 3):
//! - **PostgreSQL**, and the forks that run PostgreSQL's own parser (Timescale, an extension;
//!   Yugabyte YSQL): unquoted ASCII letters are lowered and the text is cut to 63 bytes
//!   ([`crate::fold`]); non-ASCII letters are never folded on a UTF-8 server. On another (or an
//!   unknown) server encoding the server lowers high-bit letters by its own locale, so unquoted
//!   non-ASCII input is [`ServerDependent`] (rule 3), and so is a quoted non-ASCII text that the
//!   63-byte cut might reach in the server's encoding (counting 4 bytes per non-ASCII
//!   character, the longest of any server encoding: `EUC_TW`, `MULE_INTERNAL`). Redshift (it
//!   folds quoted ASCII letters too and allows 127 bytes), CockroachDB (Unicode lowercasing of
//!   unquoted names) and unnamed forks have no pinned rule: [`ServerDependent`].
//! - **MySQL/MariaDB**: database and table names (and views) compare exactly under
//!   `lower_case_table_names = 0` and ASCII-caselessly under 1 and 2. Trigger names are exact
//!   under 0 and unpinned above it: the manual says the variable "does not affect case
//!   sensitivity of trigger identifiers", which follow the file system. Account names (users,
//!   roles) are always exact (MySQL 8.4 §8.2.3: `User` comparisons are case-sensitive). Every
//!   other name compares ASCII-caselessly, and a caseless text holding a non-ASCII **cased**
//!   letter is [`ServerDependent`]: the server's Unicode-aware comparison is not pinned until
//!   Task 17's probe (`// ## m4-t17-mysql-fold`). Backticks never change the comparison.
//!   Since that probe, [`pinned_column_eq`] says when two **column** names certainly compare
//!   equal on the server (four probed letters plus ASCII case); a lookup answers `Found` on
//!   it and still never `Missing` for such a text.
//! - **SQLite**: ASCII-caseless, never more (ver/01 `#sqlite-case`).
//! - Every other dialect: no rule is pinned, so every text is [`ServerDependent`].
//!
//! Nothing here calls a Unicode or locale case mapping: the only fold is ASCII.

use std::sync::Arc;

use crate::fold::{PG_MAX_IDENT_BYTES, fold};

/// The dialect a key belongs to (re-exported: [`IdentCtx`] names it).
pub use sql_dialect::DialectId;
use sql_dialect::PgWireFlavor;

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
/// `lower_case_table_names`, keeps triggers and accounts exact, and folds everything else
/// ASCII-caselessly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NameClass {
    Database,
    Schema,
    /// Tables and views (and MariaDB sequences, which are tables).
    Table,
    Column,
    Routine,
    /// Trigger names (MySQL/MariaDB: by the file system, not by `lower_case_table_names`).
    Trigger,
    /// Account names: users and roles (MySQL/MariaDB: always case-sensitive).
    Account,
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
    /// The stored rule of catalog names of `class` (R-M4-41): PostgreSQL (and the forks with its
    /// parser) exact, catalog names being already folded; MySQL/MariaDB database and table
    /// names exact under `lower_case_table_names = 0` and ASCII-caseless under 1 and 2, trigger
    /// names exact under 0, account names always exact, every other class ASCII-caseless;
    /// SQLite ASCII-caseless. `None` where no rule is pinned (another dialect or fork, an unknown
    /// `lower_case_table_names`, MySQL trigger names under 1 or 2).
    pub fn for_names(d: DialectId, ctx: &IdentCtx, class: NameClass) -> Option<FoldRule> {
        match d {
            _ if has_pg_rule(d) => Some(FoldRule::Exact),
            DialectId::Sqlite => Some(FoldRule::AsciiCaseless),
            DialectId::MySql | DialectId::MariaDb => match (class, ctx.lower_case_table_names) {
                (NameClass::Account, _) => Some(FoldRule::Exact),
                (NameClass::Trigger, 0) => Some(FoldRule::Exact),
                (NameClass::Trigger, _) => None,
                (NameClass::Database | NameClass::Schema | NameClass::Table, 0) => {
                    Some(FoldRule::Exact)
                }
                (NameClass::Database | NameClass::Schema | NameClass::Table, 1 | 2) => {
                    Some(FoldRule::AsciiCaseless)
                }
                (NameClass::Database | NameClass::Schema | NameClass::Table, _) => None,
                (NameClass::Column | NameClass::Routine | NameClass::Other, _) => {
                    Some(FoldRule::AsciiCaseless)
                }
            },
            // Redshift, CockroachDB, other PostgreSQL forks, Generic, SQL Server, ClickHouse,
            // Oracle and any later dialect: not pinned in M4.
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
        _ if has_pg_rule(d) => {
            if !text.is_ascii() && !is_utf8_encoding(ctx.encoding.as_deref()) {
                // The server lowers unquoted high-bit letters by its own locale, and cuts at 63
                // bytes of its own encoding.
                if !quoted || server_len_bound(text) > PG_MAX_IDENT_BYTES {
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

/// Whether `d` parses identifiers with PostgreSQL's own rule: PostgreSQL itself, Timescale (an
/// extension inside PostgreSQL) and Yugabyte YSQL (PostgreSQL's parser). Redshift folds quoted
/// ASCII letters too and allows 127 bytes (AWS "Names and identifiers"); CockroachDB lowercases
/// unquoted names with Unicode rules (Inferred from its `lexbase.NormalizeName`); an unnamed
/// fork is unknown. Those have no pinned rule.
fn has_pg_rule(d: DialectId) -> bool {
    matches!(
        d,
        DialectId::Postgres | DialectId::PgWire(PgWireFlavor::Timescale | PgWireFlavor::Yugabyte)
    )
}

/// An upper bound of `text`'s length in a PostgreSQL server encoding: one byte per ASCII
/// character, four per other character (the longest character of any server encoding:
/// `EUC_TW` and `MULE_INTERNAL`). At or under the 63-byte cut, no encoding can cut the name.
fn server_len_bound(text: &str) -> usize {
    text.chars().map(|c| if c.is_ascii() { 1 } else { 4 }).sum()
}

/// Whether a PostgreSQL `server_encoding` is UTF-8 (`UTF8`, also accepted as `UTF-8`). An
/// unknown encoding is not.
fn is_utf8_encoding(encoding: Option<&str>) -> bool {
    encoding.is_some_and(|e| e.eq_ignore_ascii_case("UTF8") || e.eq_ignore_ascii_case("UTF-8"))
}

// ## m4-t17-mysql-fold (written by M4 Task 17b from Task 17's probe, CI 44649745390)

/// The non-ASCII letters whose MySQL/MariaDB column-name comparison the probe pinned, each with
/// the letter it compares equal to (R-M4-41; `t17-docker-facts-44649745390.txt`, lines
/// `column_fold`, `lower_case_table_names = 0`, `@@character_set_system = utf8mb3`):
///
/// - MySQL 8.0 and 8.4 (`utf8mb3_tolower_ci`): `Äb` = `äb`; `İd` = `Id` = `ID` = `id`; `ıd`
///   equals only itself.
/// - MariaDB 11.4 (`utf8mb3_general_ci`): `Äb` = `äb`; `İd` equals only itself; `ıd` equals
///   only itself.
///
/// A letter outside this table has no pinned comparison. This is a closed table of four
/// letters, never a Unicode or locale case mapping (INV-3's exception, recorded in ADR 0008
/// `m4-t17b`): `İ` → `i` is the server's own fold on MySQL, not a Turkish rule (`I` never
/// becomes `ı`).
fn pinned_letter(d: DialectId, c: char) -> Option<char> {
    match (d, c) {
        (_, 'Ä' | 'ä') => Some('ä'),
        (_, 'ı') => Some('ı'),
        (DialectId::MySql, 'İ') => Some('i'),
        (DialectId::MariaDb, 'İ') => Some('İ'),
        _ => None,
    }
}

/// Whether the server certainly compares the column names `a` and `b` equal, by the pinned
/// table above plus ASCII case: `true` only when every non-ASCII cased letter of both is in
/// the table and the folded texts are equal. `false` decides nothing (the server may still
/// fold letters the probe did not cover, or fold accents): a lookup uses it only to answer
/// `Found`, never `Missing`. Column names only, on MySQL and MariaDB under
/// `lower_case_table_names = 0` (the probe's setting); the other caseless classes (index,
/// routine, partition, event names) were not probed and stay unpinned.
pub fn pinned_column_eq(d: DialectId, ctx: &IdentCtx, a: &str, b: &str) -> bool {
    if !matches!(d, DialectId::MySql | DialectId::MariaDb) || ctx.lower_case_table_names != 0 {
        return false;
    }
    let fold = |s: &str| -> Option<Vec<char>> {
        s.chars()
            .map(|c| {
                if c.is_ascii() {
                    Some(c.to_ascii_lowercase())
                } else if has_non_ascii_cased(c.encode_utf8(&mut [0; 4])) {
                    pinned_letter(d, c)
                } else {
                    Some(c)
                }
            })
            .collect()
    };
    matches!((fold(a), fold(b)), (Some(x), Some(y)) if x == y)
}
// ## m4-t17-mysql-fold
