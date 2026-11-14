//! The lexing modes a server's settings imply (SQL-LEX-01; Task 3).

use crate::id::DialectId;

/// Settings that change how text lexes: PostgreSQL `standard_conforming_strings`, MySQL
/// `sql_mode` flags, MariaDB's Oracle mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LexModes {
    pub standard_conforming_strings: bool,
    pub ansi_quotes: bool,
    pub no_backslash_escapes: bool,
    pub pipes_as_concat: bool,
    pub high_not_precedence: bool,
    pub mariadb_oracle_mode: bool,
}

impl LexModes {
    /// A dialect's defaults (Task 3). Task 1b stub: every mode off.
    pub fn default_for(d: DialectId) -> Self {
        let _ = d;
        LexModes::default()
    }

    /// The modes of a MySQL/MariaDB `@@SESSION.sql_mode` value (`ANSI` implies `ANSI_QUOTES`
    /// and `PIPES_AS_CONCAT`; Task 3). Task 1b stub: every mode off.
    pub fn from_mysql_sql_mode(sql_mode: &str) -> Self {
        let _ = sql_mode;
        LexModes::default()
    }
}
