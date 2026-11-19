// ## m4-t01c-core
//! Catalog query packs (HV-INTRO-04, R-M4-7, R-M4-32, R-M4-37): the only source of catalog SQL.
//!
//! A pack is the `packs/<dialect>/NN-<topic>.hvq.sql` files of one dialect, merged in file-name
//! order (`10-core`, `20-facets`, `30-state`, `40-system`; later milestones add `70-…`). Task 4
//! writes the grammar (`--- name: <Section> ---`, `--- returns: … ---`, `--- joinable ---`,
//! `--- show-create ---`, `--# if/elif/else/end`, `:name`, `:[name]`, the closed `#{macro}`
//! set), the parser, the lint and the renderer; Tasks 16–18 write the sections.
//!
//! This module builds **without** the `session` feature (the fuzzed parser, ADR 0008
//! `m4-t01a`). The renderer that turns a section into a `hacivat_session::meta::MetaQuery` is
//! the `session`-gated submodule [`render`] (check-deps `MetaQueryOnlyFromPacks` allows
//! `MetaQuery::new` only under `pack/`).
//!
//! **Fail-closed stub (M4 Task 1c).** The pack files are compiled in (header only); parsing
//! answers [`PackErrorKind::NotImplemented`], so no catalog SQL exists to run.

use hacivat_meta::snapshot::Flavor;
use hacivat_meta::{Level, Name};
use hacivat_values::BindValue;
use sql_dialect::DialectId;

#[cfg(feature = "session")]
pub mod render;

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

/// A parsed pack: its sections, merged from every file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pack {
    dialect: DialectId,
    sections: Vec<Name>,
}

impl Pack {
    /// Parses and merges `files` (in the order given) into one pack (R-M4-32: a section name
    /// defined twice is an error).
    ///
    /// Stub: [`PackErrorKind::NotImplemented`] at the first file.
    pub fn parse_all(dialect: DialectId, files: &[PackFile]) -> Result<Pack, PackError> {
        let _ = dialect;
        Err(PackError {
            file: files.first().map_or("", |f| f.0),
            line: 0,
            kind: PackErrorKind::NotImplemented,
        })
    }

    /// The compiled-in pack of `dialect`.
    pub fn builtin(dialect: DialectId) -> Result<Pack, PackError> {
        let files = files_of(dialect).ok_or(PackError {
            file: "",
            line: 0,
            kind: PackErrorKind::UnsupportedDialect,
        })?;
        Pack::parse_all(dialect, files)
    }

    pub fn dialect(&self) -> DialectId {
        self.dialect
    }

    /// The section names, in definition order.
    pub fn sections(&self) -> &[Name] {
        &self.sections
    }
}

/// Why a pack does not parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackError {
    /// The pack file (`10-core`).
    pub file: &'static str,
    /// The 1-based line; 0 when the error has no line.
    pub line: u32,
    pub kind: PackErrorKind,
}

/// The closed set of pack errors (codes only; Task 4 adds the grammar's).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackErrorKind {
    /// The parser is not written yet (M4 Task 1c stub).
    NotImplemented,
    /// M4 introspects PostgreSQL, MySQL, MariaDB and SQLite only (R-M4-4).
    UnsupportedDialect,
    /// A section name is defined twice across the files (R-M4-32).
    DuplicateSection(Name),
}

bitflags::bitflags! {
    /// Facts a section's `--# if` directives may test besides the version (Task 4).
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
    /// `:name` binds.
    pub scalars: Vec<(Name, BindValue)>,
    /// `:[name]` list binds, expanded to `IN (…)` chunks of at most 1,000 items.
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
                assert!(
                    text.lines().all(|l| l.starts_with("-- ")),
                    "{d:?} {name}: only comment lines until Tasks 16–18"
                );
            }
        }
        assert!(files_of(DialectId::SqlServer).is_none());
    }

    #[test]
    fn the_stub_parses_nothing() {
        let e = Pack::builtin(DialectId::Postgres).unwrap_err();
        assert_eq!((e.file, e.kind), ("10-core", PackErrorKind::NotImplemented));
        assert_eq!(
            Pack::builtin(DialectId::Oracle).unwrap_err().kind,
            PackErrorKind::UnsupportedDialect
        );
    }
}
