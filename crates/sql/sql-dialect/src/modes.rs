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
    /// A dialect's defaults before the session reads the server's settings (`alg/10` LEX-01
    /// "Session lexical modes"; working since Task 1b's fix round 1): the PostgreSQL family has
    /// `standard_conforming_strings` on (the server default since PostgreSQL 9.1; CockroachDB,
    /// YugabyteDB and Timescale inherit it; Redshift's is Inferred, the connect-time read
    /// corrects it); MySQL's and MariaDB's default `sql_mode` holds none of `ANSI_QUOTES`,
    /// `NO_BACKSLASH_ESCAPES`, `PIPES_AS_CONCAT`, `HIGH_NOT_PRECEDENCE` or `ORACLE`; the other
    /// dialects have no session lexical mode (their quoting is the lexer profile's).
    pub fn default_for(d: DialectId) -> Self {
        match d {
            DialectId::Postgres | DialectId::PgWire(_) => LexModes {
                standard_conforming_strings: true,
                ..LexModes::default()
            },
            _ => LexModes::default(),
        }
    }

    /// The modes of a MySQL/MariaDB `@@SESSION.sql_mode` value (`ANSI` implies `ANSI_QUOTES`
    /// and `PIPES_AS_CONCAT`; Task 3). Task 1b stub: every mode off.
    pub fn from_mysql_sql_mode(sql_mode: &str) -> Self {
        let _ = sql_mode;
        LexModes::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::PgWireFlavor;

    #[test]
    fn dialect_defaults_follow_the_servers() {
        assert!(LexModes::default_for(DialectId::Postgres).standard_conforming_strings);
        for flavor in [
            PgWireFlavor::Redshift,
            PgWireFlavor::Cockroach,
            PgWireFlavor::Yugabyte,
            PgWireFlavor::Timescale,
            PgWireFlavor::Other,
        ] {
            assert!(LexModes::default_for(DialectId::PgWire(flavor)).standard_conforming_strings);
        }
        for d in [DialectId::MySql, DialectId::MariaDb] {
            let m = LexModes::default_for(d);
            assert!(!m.standard_conforming_strings);
            assert!(!(m.ansi_quotes || m.no_backslash_escapes || m.pipes_as_concat));
            assert!(!(m.high_not_precedence || m.mariadb_oracle_mode));
        }
        assert_eq!(
            LexModes::default_for(DialectId::Sqlite),
            LexModes::default()
        );
        assert_eq!(
            LexModes::default_for(DialectId::Generic),
            LexModes::default()
        );
    }
}
