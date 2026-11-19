// ## m4-t04-packs (seam: m4-t01c-core)
//! Catalog query packs (HV-INTRO-04, R-M4-7, R-M4-32, R-M4-37): the only source of catalog SQL.
//!
//! A pack is the `packs/<dialect>/NN-<topic>.hvq.sql` files of one dialect, merged in file-name
//! order (`10-core`, `20-facets`, `30-state`, `40-system`; later milestones add `70-…`); a
//! section name defined twice is a parse error. Tasks 16–18 write the sections.
//!
//! # The grammar (line-oriented)
//!
//! ```text
//! -- a comment line outside a section is ignored
//! --- name: ListNamespaces ---           starts a section (ASCII letter, then [A-Za-z0-9_])
//! --- returns: oid u32, name text ---    required, before the first SQL line
//! --- joinable ---                       optional: may run on a joined connection (R-M4-29)
//! --- show-create ---                    optional, MySQL/MariaDB only (R-M4-37)
//! SELECT n.oid, n.nspname FROM pg_catalog.pg_namespace n WHERE n.oid = :ns
//! --# if version >= 150000 and mode == incremental
//!   …
//! --# elif not flag(joined)
//! --# else
//! --# end
//! ```
//!
//! - **Directives** are lines whose trimmed text starts with `--#` (blocks) or `---` (section
//!   header); nothing else is a directive, so directive-looking text inside a literal is plain
//!   SQL (and a lint error). Blocks nest at most [`MAX_NESTING`] deep. Conditions are a
//!   closed boolean language over `version` (an integer: PG `server_version_num`, MySQL
//!   `80046`), `mode` (`full`, `incremental`, `brief`, `system`), `level` (`names`,
//!   `structure`, `sources`), `flavor` (a `DbmsKind` id) and `flag(mariadb|joined)`, with
//!   `and`, `or`, `not` and parentheses.
//! - **Binds** (R-M4-3): `:name` is one bind; `:[name]` is a list bind and renders as `(…)`
//!   with one bind per item, duplicates removed, chunked at [`MAX_IN_LIST`] items (one query
//!   per chunk; an empty list renders no query). A list bind is legal only when the
//!   top-level `AND` conjunct of the `WHERE` clause that holds it is exactly
//!   `<column-ref> IN :[name]` — outside every `(…)`, `[…]` and `CASE … END`, with no `NOT`,
//!   `!`, `IS …`, `OR` or `BETWEEN … AND` around it — used once per section, in a query whose
//!   own level has no grouping, paging, `DISTINCT`, window, `FILTER`/`WITHIN GROUP` or set
//!   operation and calls only allowlisted row-by-row functions (checked in every `--# if`
//!   variant; fix rounds 1 and 2, I3 and I5). Then each chunk selects a disjoint part of the
//!   answer and the chunks' union is the answer. PostgreSQL gets `$n`, every other dialect
//!   `?`. `::` (a PostgreSQL cast) is never a bind.
//! - **Macros** are a closed enum ([`Macro`]): `#{max_in_list}` (the chunk size) and
//!   `#{ident:<bind>}`, which only a `show-create` section may use (a parse error elsewhere):
//!   it renders a text-typed bind through `sql_ident::quote::quote(DialectId::MySql, …)` and
//!   refuses a NUL. Names are never macros (alg/02 HV-INTRO-04 edge 6).
//! - **Fail-closed lexing.** Literals, quoted identifiers and block comments close on the line
//!   they open. These are parse errors: a backslash inside a literal; a raw placeholder (`$`
//!   on PostgreSQL and SQLite, `?` on MySQL and SQLite, `@`, `#name` and `[` on SQLite, `:`
//!   followed by a digit or a non-ASCII byte everywhere); a backtick in a PostgreSQL pack
//!   (an operator character there, not a quote); `/*` anywhere inside a block comment,
//!   its closing `*/` included (PostgreSQL nests comments); a MySQL `/*!` or MariaDB `/*M!`
//!   executable comment; a control character other than the tab; a `;`; a MySQL `#`
//!   comment or a MySQL `--` without a following space; a macro inside a literal or
//!   comment. Each rule closes one known way the server would read code where this lexer
//!   reads a literal or comment (or the reverse) on the server's default settings; the
//!   review's probes H1–H4 (`tests/hostile_t04_review.rs`) pin them. A `show-create` section
//!   holds no literal, comment or bind, and each identifier macro stands alone between
//!   whitespace, `.`, `(`, `)` or `,`.
//! - **Provenance** (R-M4-7; fix round 1, I2). A [`Pack`] comes only from `&'static` text
//!   ([`Pack::parse`], [`Pack::parse_all`], [`Pack::builtin`]); runtime text parses into a
//!   [`DraftPack`], whose [`DraftQuery`]s cannot become a `MetaQuery`. Check-deps
//!   `PackParseOnlyFromPacks` limits naming `Pack::parse`/`parse_all` to `pack/**`, the
//!   introspector modules, `tests/` and `fuzz/meta/`.
//!
//! This module builds **without** the `session` feature (the fuzzed parser, renderer and
//! lint, ADR 0008 `m4-t01a`, `m4-t04`). The one function that turns a rendered query into a
//! `hacivat_session::meta::MetaQuery`, [`RenderedQuery::as_meta_query`], is `session`-gated
//! inside [`render`] (check-deps `MetaQueryOnlyFromPacks` allows `MetaQuery::new` only under
//! `pack/`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use hacivat_meta::{Level, Name};

// The types this API takes, re-exported so a caller (the `fuzz/meta` target) needs no other
// crate to build a `PackCtx`.
pub use hacivat_meta::snapshot::Flavor;
pub use hacivat_values::{BindValue, LogicalType};
pub use sql_dialect::DialectId;

mod lint;
mod parse;
pub mod render;

pub use lint::{LintError, LintKind, lint};
pub use parse::Macro;
pub use render::{DraftQuery, RenderError, RenderedQuery};

use parse::Family;

/// A list bind renders at most this many items per query (R-M4-3); longer lists chunk.
pub const MAX_IN_LIST: usize = 1000;

/// `--# if` blocks nest at most this deep (alg/02 HV-INTRO-04 edge 2).
pub const MAX_NESTING: usize = 4;

/// How a pass reads a namespace (HV-INTRO-01): the `mode` a pack section is rendered for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Mode {
    /// Every object of the namespace.
    #[default]
    Full,
    /// Only what changed since the stored watermark.
    Incremental,
    /// Names only, the first paint (`Brief`, HV-INTRO-01 step 3).
    Brief,
    /// The system schemas, for the shipped snapshots (R-M4-44).
    System,
}

/// One pack file: its name (`10-core`) and its text.
pub type PackFile = (&'static str, &'static str);

/// The PostgreSQL pack files, in merge order.
pub const PG_FILES: [PackFile; 4] = [
    ("10-core", include_str!("../../packs/pg/10-core.hvq.sql")),
    (
        "20-facets",
        include_str!("../../packs/pg/20-facets.hvq.sql"),
    ),
    ("30-state", include_str!("../../packs/pg/30-state.hvq.sql")),
    (
        "40-system",
        include_str!("../../packs/pg/40-system.hvq.sql"),
    ),
];

/// The MySQL and MariaDB pack files, in merge order (one pack; flavor gates inside).
pub const MYSQL_FILES: [PackFile; 4] = [
    ("10-core", include_str!("../../packs/mysql/10-core.hvq.sql")),
    (
        "20-facets",
        include_str!("../../packs/mysql/20-facets.hvq.sql"),
    ),
    (
        "30-state",
        include_str!("../../packs/mysql/30-state.hvq.sql"),
    ),
    (
        "40-system",
        include_str!("../../packs/mysql/40-system.hvq.sql"),
    ),
];

/// The SQLite pack files, in merge order (no facets, no system snapshot: R-M4-6).
pub const SQLITE_FILES: [PackFile; 2] = [
    (
        "10-core",
        include_str!("../../packs/sqlite/10-core.hvq.sql"),
    ),
    (
        "30-state",
        include_str!("../../packs/sqlite/30-state.hvq.sql"),
    ),
];

/// The compiled-in pack files of `dialect`; `None` for a dialect M4 does not introspect
/// (R-M4-4).
pub fn files_of(dialect: DialectId) -> Option<&'static [PackFile]> {
    match dialect {
        DialectId::Postgres => Some(&PG_FILES),
        DialectId::MySql | DialectId::MariaDb => Some(&MYSQL_FILES),
        DialectId::Sqlite => Some(&SQLITE_FILES),
        _ => None,
    }
}

/// A pack parsed from **runtime** text (tests, the fuzz target, a pack author's tooling): it
/// parses, lints and renders exactly like a [`Pack`], but its queries are [`DraftQuery`]s,
/// which cannot become a `MetaQuery` (fix round 1, I2: catalog SQL that runs comes only from
/// `&'static` pack text). Cloning is cheap (shared).
#[derive(Clone)]
pub struct DraftPack {
    inner: Arc<Inner>,
}

/// A pack parsed from `&'static` text (the `include_str!` pack files): the only pack whose
/// rendered queries ([`RenderedQuery`]) become a `MetaQuery` (R-M4-7). Check-deps
/// `PackParseOnlyFromPacks` limits where [`Pack::parse`] and [`Pack::parse_all`] may be named
/// (`pack/**`, the introspector modules, `tests/`, `fuzz/meta/`), so `Box::leak`ed runtime
/// text cannot be laundered into one either. Cloning is cheap (shared).
#[derive(Clone, PartialEq, Eq)]
pub struct Pack {
    draft: DraftPack,
}

struct Inner {
    dialect: DialectId,
    family: Family,
    names: Vec<Name>,
    sections: Vec<Section>,
    index: HashMap<Name, usize>,
    /// Section templates per `(section, ctx without params)` (alg/02 HV-INTRO-04 step 3);
    /// binds are per call.
    cache: Mutex<HashMap<render::TemplateKey, Arc<render::Template>>>,
}

impl std::fmt::Debug for DraftPack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DraftPack")
            .field("dialect", &self.inner.dialect)
            .field("sections", &self.inner.names)
            .finish()
    }
}

impl std::fmt::Debug for Pack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pack")
            .field("dialect", &self.draft.inner.dialect)
            .field("sections", &self.draft.inner.names)
            .finish()
    }
}

impl PartialEq for DraftPack {
    fn eq(&self, other: &Self) -> bool {
        self.inner.dialect == other.inner.dialect && self.inner.sections == other.inner.sections
    }
}

impl Eq for DraftPack {}

impl AsRef<DraftPack> for DraftPack {
    fn as_ref(&self) -> &DraftPack {
        self
    }
}

impl AsRef<DraftPack> for Pack {
    fn as_ref(&self) -> &DraftPack {
        &self.draft
    }
}

impl DraftPack {
    /// Parses and merges `files` into one pack. The files are merged in **file-name order**
    /// (byte order), whatever order they are given in; a section name defined twice is
    /// [`PackErrorKind::DuplicateSection`] at the later file (R-M4-32).
    pub fn parse_all(
        dialect: DialectId,
        files: &[(&'static str, &str)],
    ) -> Result<DraftPack, PackError> {
        let family = Family::of(dialect).ok_or(PackError {
            file: files.first().map_or("", |f| f.0),
            line: 0,
            kind: PackErrorKind::UnsupportedDialect,
        })?;
        let sections = parse::parse_files(dialect, family, files)?;
        let names: Vec<Name> = sections.iter().map(|s| s.name.clone()).collect();
        let index = names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.clone(), i))
            .collect();
        Ok(DraftPack {
            inner: Arc::new(Inner {
                dialect,
                family,
                names,
                sections,
                index,
                cache: Mutex::new(HashMap::new()),
            }),
        })
    }

    /// Parses one pack text (file name `""`).
    pub fn parse(dialect: DialectId, text: &str) -> Result<DraftPack, PackError> {
        DraftPack::parse_all(dialect, &[("", text)])
    }

    pub fn dialect(&self) -> DialectId {
        self.inner.dialect
    }

    /// The section names, in definition order (files in name order).
    pub fn sections(&self) -> &[Name] {
        &self.inner.names
    }

    /// One section's declaration (`returns`, flags, where it is defined).
    pub fn section(&self, name: &str) -> Option<&Section> {
        let i = *self.inner.index.get(name)?;
        Some(&self.inner.sections[i])
    }
}

impl Pack {
    /// Parses and merges the `&'static` `files` (see [`DraftPack::parse_all`]).
    pub fn parse_all(dialect: DialectId, files: &[PackFile]) -> Result<Pack, PackError> {
        let files: Vec<(&'static str, &str)> = files.iter().map(|&(n, t)| (n, t)).collect();
        DraftPack::parse_all(dialect, &files).map(|draft| Pack { draft })
    }

    /// Parses one `&'static` pack text (file name `""`).
    pub fn parse(dialect: DialectId, text: &'static str) -> Result<Pack, PackError> {
        DraftPack::parse_all(dialect, &[("", text)]).map(|draft| Pack { draft })
    }

    /// The compiled-in pack of `dialect`, parsed once per process.
    pub fn builtin(dialect: DialectId) -> Result<Pack, PackError> {
        static PG: OnceLock<Result<Pack, PackError>> = OnceLock::new();
        static MYSQL: OnceLock<Result<Pack, PackError>> = OnceLock::new();
        static MARIADB: OnceLock<Result<Pack, PackError>> = OnceLock::new();
        static SQLITE: OnceLock<Result<Pack, PackError>> = OnceLock::new();
        let (cell, files) = match dialect {
            DialectId::Postgres => (&PG, &PG_FILES[..]),
            DialectId::MySql => (&MYSQL, &MYSQL_FILES[..]),
            DialectId::MariaDb => (&MARIADB, &MYSQL_FILES[..]),
            DialectId::Sqlite => (&SQLITE, &SQLITE_FILES[..]),
            _ => {
                return Err(PackError {
                    file: "",
                    line: 0,
                    kind: PackErrorKind::UnsupportedDialect,
                });
            }
        };
        cell.get_or_init(|| Pack::parse_all(dialect, files)).clone()
    }

    pub fn dialect(&self) -> DialectId {
        self.draft.dialect()
    }

    /// The section names, in definition order (files in name order).
    pub fn sections(&self) -> &[Name] {
        self.draft.sections()
    }

    /// One section's declaration (`returns`, flags, where it is defined).
    pub fn section(&self, name: &str) -> Option<&Section> {
        self.draft.section(name)
    }

    /// See [`DraftPack::reachable_variants`].
    pub fn reachable_variants(&self, section: &str) -> Vec<PackCtx> {
        self.draft.reachable_variants(section)
    }
}

/// A section's declaration: what `--- name/returns/joinable/show-create ---` said, and its
/// body (parsed lines and `--#` blocks).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub(crate) name: Name,
    pub(crate) file: &'static str,
    pub(crate) line: u32,
    pub(crate) returns: Vec<ColDecl>,
    pub(crate) joinable: bool,
    pub(crate) show_create: bool,
    pub(crate) body: Vec<parse::Node>,
}

impl Section {
    pub fn name(&self) -> &Name {
        &self.name
    }

    /// The pack file that defines it.
    pub fn file(&self) -> &'static str {
        self.file
    }

    /// The line of its `--- name: … ---`.
    pub fn line(&self) -> u32 {
        self.line
    }

    /// The declared result columns, in order.
    pub fn returns(&self) -> &[ColDecl] {
        &self.returns
    }

    /// Marked `--- joinable ---` (R-M4-29).
    pub fn joinable(&self) -> bool {
        self.joinable
    }

    /// Marked `--- show-create ---` (R-M4-37).
    pub fn show_create(&self) -> bool {
        self.show_create
    }
}

/// One declared result column (`--- returns: oid u32, name text ---`), checked by the
/// introspectors' CI matrix (alg/02 HV-INTRO-04 step 4).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ColDecl {
    pub name: Name,
    pub ty: ColType,
}

/// The closed set of declared column types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColType {
    Text,
    Bool,
    I16,
    I32,
    I64,
    U32,
    U64,
    F64,
    Bytes,
    Timestamp,
}

impl ColType {
    pub const ALL: [ColType; 10] = [
        ColType::Text,
        ColType::Bool,
        ColType::I16,
        ColType::I32,
        ColType::I64,
        ColType::U32,
        ColType::U64,
        ColType::F64,
        ColType::Bytes,
        ColType::Timestamp,
    ];

    /// The spelling in a `returns` line.
    pub fn as_str(self) -> &'static str {
        match self {
            ColType::Text => "text",
            ColType::Bool => "bool",
            ColType::I16 => "i16",
            ColType::I32 => "i32",
            ColType::I64 => "i64",
            ColType::U32 => "u32",
            ColType::U64 => "u64",
            ColType::F64 => "f64",
            ColType::Bytes => "bytes",
            ColType::Timestamp => "timestamp",
        }
    }

    fn from_str(s: &str) -> Option<ColType> {
        ColType::ALL.into_iter().find(|t| t.as_str() == s)
    }
}

/// Why a pack does not parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackError {
    /// The pack file (`10-core`); `""` for [`Pack::parse`].
    pub file: &'static str,
    /// The 1-based line; 0 when the error has no line.
    pub line: u32,
    pub kind: PackErrorKind,
}

/// The closed set of pack errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackErrorKind {
    /// M4 introspects PostgreSQL, MySQL, MariaDB and SQLite only (R-M4-4).
    UnsupportedDialect,
    /// A section name is defined twice across the files (R-M4-32).
    DuplicateSection(Name),
    /// Two files carry the same name.
    DuplicateFile,
    /// A non-comment line before the first `--- name: … ---`.
    TextOutsideSection,
    /// `--- x ---` or `--# x` with an unknown word.
    UnknownDirective,
    /// A directive line that is not well formed (`---` not closed, trailing text).
    MalformedDirective,
    /// A section name that is not an ASCII letter followed by `[A-Za-z0-9_]`.
    BadSectionName,
    /// `returns`, `joinable` or `show-create` outside a section or after its first SQL line.
    DirectiveOutOfPlace,
    /// `joinable`, `show-create` or `returns` given twice.
    DuplicateFlag,
    /// A section without `--- returns: … ---`.
    MissingReturns,
    /// A `returns` list that is empty, repeats a column or names an unknown type.
    BadReturns,
    /// `--# if` nested deeper than [`MAX_NESTING`].
    NestingTooDeep,
    /// `--# elif`/`else`/`end` without an open `if`, or after its `else`.
    UnbalancedBlock,
    /// An `--# if` still open at the end of its section (reported at the `if`).
    UnclosedIf,
    /// A condition outside the closed language.
    BadCondition,
    /// A condition nested too deep (parentheses or `not`).
    ExprTooDeep,
    /// A literal, quoted identifier or block comment that does not close on its line.
    UnterminatedLiteral,
    /// `/*` inside a block comment (PostgreSQL nests them, MySQL does not).
    NestedComment,
    /// A backslash inside a literal (escape rules differ per server setting).
    BackslashInLiteral,
    /// `#{` inside a literal, quoted identifier or comment.
    MacroInLiteral,
    /// `#{…}` outside the closed set.
    UnknownMacro,
    /// `:[` not followed by a valid name and `]`.
    BadBind,
    /// A server placeholder written in pack text (`$1`, `?`, `@x`, `$$`).
    RawPlaceholder,
    /// `;`: a section is exactly one statement.
    Semicolon,
    /// A MySQL `#` comment.
    HashComment,
    /// A MySQL `--` not followed by a space (MySQL does not read it as a comment).
    AmbiguousDashDash,
    /// A SQLite `[identifier]`.
    BracketIdentifier,
    /// A list bind not written as `<operand> IN :[name]` on one line.
    ListBindNotAfterIn,
    /// `NOT IN :[name]` (a chunked `NOT IN` is wrong).
    NotInListBind,
    /// Two different list binds in one section (chunking needs one).
    MultipleListBinds,
    /// `--- show-create ---` in a pack that is not MySQL/MariaDB (R-M4-37).
    ShowCreateNotMySql,
    /// A literal, comment, quoted identifier or bind in a `show-create` section, or an
    /// identifier macro glued to other text.
    ShowCreateText,
    /// A control character other than the tab inside a section line (a bare `\r` ends a
    /// PostgreSQL `--` comment where the pack lexer would not).
    ControlChar,
    /// A backtick in a PostgreSQL pack (an operator character there, not a quote).
    Backtick,
    /// A MySQL `/*!…*/` or MariaDB `/*M!…*/` executable comment (code to the server).
    ExecutableComment,
    /// `#{ident:…}` in a section not marked `show-create` (R-M4-37).
    IdentOutsideShowCreate,
    /// A list bind used more than once in a section (each use would get the same chunk).
    ListBindReused,
    /// A list bind that is not a top-level `AND` conjunct of the `WHERE` clause written
    /// `<column> IN :[name]` (under parentheses, a negation, `OR`, `IS …`, a `CASE`, an `ON`
    /// clause …): chunking would change the result.
    ListBindNotConjunct,
    /// A construct whose result chunking changes, in a query with a list bind: an aggregate,
    /// `GROUP BY`, `HAVING`, `LIMIT`, `OFFSET`, `FETCH`, `DISTINCT`, a window, `OR`, `XOR`,
    /// MySQL `||`, `UNION`/`INTERSECT`/`EXCEPT` at the list's level. Holds the word.
    ListBindUnchunkable(String),
    /// A list-bind section with so many `--# if` variants that its check would not finish.
    ListBindTooManyVariants,
    /// A `PRAGMA` statement: the session's read check refuses it; SQLite packs read through
    /// the `pragma_*` table-valued functions (T5 carry).
    PragmaStatement,
    /// `INSERT`, `UPDATE`, `DELETE`, `MERGE` or `INTO` anywhere in a `joinable` section's text
    /// (literals and comments included): a joined `MetaConn` refuses the text (T5 carry).
    /// Holds the word as written.
    JoinableWriteWord(String),
    /// A function call in a `joinable` section that is not on its allowlist (no session or
    /// server state change, no lock, no XID, no `search_path`-dependent text; fix round 2).
    /// Holds the name as written.
    JoinableFunction(String),
}

bitflags::bitflags! {
    /// Facts a section's `--# if` directives may test besides the version.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct PackFlags: u32 {
        /// The server is MariaDB (the MySQL pack's flavor gate).
        const MARIADB = 1;
        /// The connection is joined to a Hacivat-owned transaction (R-M4-29): only `joinable`
        /// sections render.
        const JOINED = 1 << 1;
    }
}

/// The binds of a rendering: names and ids always travel as binds (R-M4-3), never as text.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Params {
    /// `:name` binds (and the text of `#{ident:name}` in `show-create` sections).
    pub scalars: Vec<(Name, BindValue)>,
    /// `:[name]` list binds, expanded to `IN (…)` chunks of at most [`MAX_IN_LIST`] items.
    pub lists: Vec<(Name, Vec<BindValue>)>,
}

/// What a section is rendered for.
#[derive(Debug, Clone, PartialEq)]
pub struct PackCtx {
    pub dialect: DialectId,
    /// PostgreSQL `server_version_num`, MySQL `80046` style; 0 when unknown.
    pub version: u32,
    pub flavor: Flavor,
    pub mode: Mode,
    pub level: Level,
    pub flags: PackFlags,
    pub params: Params,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pack_file_is_a_header_with_no_section_yet() {
        for d in [DialectId::Postgres, DialectId::MySql, DialectId::Sqlite] {
            for (name, text) in files_of(d).unwrap() {
                assert!(!text.is_empty(), "{d:?} {name}");
                // ## m4-t16-pg (named exception): Task 16 wrote the PostgreSQL core, state and
                // system files; `20-facets` stays a header until Task 16b.
                if d == DialectId::Postgres && *name != "20-facets" {
                    continue;
                }
                assert!(
                    text.lines().all(|l| l.starts_with("-- ")),
                    "{d:?} {name}: only comment lines until Tasks 16–18"
                );
            }
        }
        assert!(files_of(DialectId::SqlServer).is_none());
    }

    #[test]
    fn the_builtin_packs_parse_to_no_section_until_tasks_16_to_18() {
        for d in [
            DialectId::Postgres,
            DialectId::MySql,
            DialectId::MariaDb,
            DialectId::Sqlite,
        ] {
            let p = Pack::builtin(d).unwrap();
            // ## m4-t16-pg (named exception): the PostgreSQL pack has Task 16's sections.
            if d == DialectId::Postgres {
                assert!(p.section("Identity").is_some() && p.section("Canary").is_some());
                continue;
            }
            assert!(p.sections().is_empty(), "{d:?}");
        }
        assert_eq!(
            Pack::builtin(DialectId::Oracle).unwrap_err().kind,
            PackErrorKind::UnsupportedDialect
        );
    }
}
