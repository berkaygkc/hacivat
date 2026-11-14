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

    /// The modes of a MySQL/MariaDB `@@SESSION.sql_mode` value: comma-separated mode names,
    /// matched whole, trimmed and ASCII case-insensitively (never a locale-aware casing).
    ///
    /// The server reports a combination mode together with its parts (`SET sql_mode = 'ANSI'`
    /// reads back as `REAL_AS_FLOAT,PIPES_AS_CONCAT,ANSI_QUOTES,IGNORE_SPACE,…,ANSI`), but a
    /// value taken from elsewhere may hold the bare name, so the combinations that change
    /// lexing are expanded here too: `ANSI` and MariaDB's `DB2`, `MAXDB`, `MSSQL`, `ORACLE` and
    /// `POSTGRESQL` all include `ANSI_QUOTES` and `PIPES_AS_CONCAT`; `ORACLE` also sets
    /// [`LexModes::mariadb_oracle_mode`]. Every other name (`STRICT_TRANS_TABLES`,
    /// `IGNORE_SPACE`, …) leaves the lexer unchanged.
    pub fn from_mysql_sql_mode(sql_mode: &str) -> Self {
        let mut m = LexModes::default();
        for name in sql_mode.split(',').map(str::trim) {
            let is = |mode: &str| name.eq_ignore_ascii_case(mode);
            if is("ANSI_QUOTES") {
                m.ansi_quotes = true;
            } else if is("NO_BACKSLASH_ESCAPES") {
                m.no_backslash_escapes = true;
            } else if is("PIPES_AS_CONCAT") {
                m.pipes_as_concat = true;
            } else if is("HIGH_NOT_PRECEDENCE") {
                m.high_not_precedence = true;
            } else if ["ANSI", "DB2", "MAXDB", "MSSQL", "ORACLE", "POSTGRESQL"]
                .into_iter()
                .any(is)
            {
                m.ansi_quotes = true;
                m.pipes_as_concat = true;
                m.mariadb_oracle_mode |= is("ORACLE");
            }
        }
        m
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
