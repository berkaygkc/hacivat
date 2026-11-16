//! Lexing modes from MySQL/MariaDB `sql_mode`, dialect contexts and the profile version string
//! (SQL-LEX-01; Task 3).

use sql_dialect::{DialectCtx, DialectId, LexModes, PgWireFlavor, ServerVersion, profile_version};

fn ctx(id: DialectId, version: Option<(u16, u16, u16)>) -> DialectCtx {
    DialectCtx::new(id, version.map(|(a, b, c)| ServerVersion::new(a, b, c)))
}

fn pg18() -> DialectCtx {
    ctx(DialectId::Postgres, Some((18, 1, 0)))
}

#[test]
fn ansi_sql_mode_implies_quotes_and_pipes() {
    let m = LexModes::from_mysql_sql_mode("ANSI");
    assert!(m.ansi_quotes && m.pipes_as_concat);
    assert!(!(m.no_backslash_escapes || m.high_not_precedence || m.mariadb_oracle_mode));
    // What `SELECT @@SESSION.sql_mode` answers after `SET sql_mode = 'ANSI'` (MySQL 8).
    let expanded = "REAL_AS_FLOAT,PIPES_AS_CONCAT,ANSI_QUOTES,IGNORE_SPACE,ONLY_FULL_GROUP_BY,ANSI";
    assert_eq!(LexModes::from_mysql_sql_mode(expanded), m);
}

#[test]
fn no_backslash_escapes_is_read() {
    let m = LexModes::from_mysql_sql_mode("STRICT_TRANS_TABLES,NO_BACKSLASH_ESCAPES");
    assert!(m.no_backslash_escapes);
    assert!(!(m.ansi_quotes || m.pipes_as_concat));
}

#[test]
fn every_lexical_flag_is_read_on_its_own() {
    let one = |s| LexModes::from_mysql_sql_mode(s);
    assert!(one("ANSI_QUOTES").ansi_quotes);
    assert!(one("PIPES_AS_CONCAT").pipes_as_concat);
    assert!(one("HIGH_NOT_PRECEDENCE").high_not_precedence);
    assert!(!one("ANSI_QUOTES").pipes_as_concat);
}

#[test]
fn mariadb_combination_modes_imply_ansi_quoting() {
    let oracle = LexModes::from_mysql_sql_mode("ORACLE");
    assert!(oracle.mariadb_oracle_mode && oracle.ansi_quotes && oracle.pipes_as_concat);
    for mode in ["DB2", "MAXDB", "MSSQL", "POSTGRESQL"] {
        let m = LexModes::from_mysql_sql_mode(mode);
        assert!(m.ansi_quotes && m.pipes_as_concat, "{mode}");
        assert!(!m.mariadb_oracle_mode, "{mode}");
    }
}

#[test]
fn sql_mode_names_are_trimmed_and_ascii_case_insensitive() {
    let m = LexModes::from_mysql_sql_mode(" ansi_quotes , No_Backslash_Escapes ");
    assert!(m.ansi_quotes && m.no_backslash_escapes);
}

#[test]
fn sql_mode_flags_match_whole_names_only() {
    for s in [
        "",
        ",,",
        "ANSI_QUOTESX",
        "XANSI",
        "NO_BACKSLASH",
        "STRICT_ALL_TABLES",
        "ansı_quotes",
        "ANSİ",
    ] {
        assert_eq!(
            LexModes::from_mysql_sql_mode(s),
            LexModes::default(),
            "{s:?}"
        );
    }
}

#[test]
fn a_context_starts_from_the_dialect_defaults() {
    let c = ctx(DialectId::Postgres, None);
    assert_eq!(c.modes, LexModes::default_for(DialectId::Postgres));
    assert!(c.modes.standard_conforming_strings);
    assert_eq!(c.version, None);
    assert!(ServerVersion::new(8, 4, 0) > ServerVersion::new(8, 0, 44));
}

#[test]
fn profile_version_is_stable() {
    assert_eq!(profile_version(&pg18()), "lex:pg-18@2026.11.1");
}

#[test]
fn profile_versions_follow_the_server_lines() {
    let pv = |id, version| profile_version(&ctx(id, version));
    // An unknown version lexes with the newest profile (alg/10 LEX-01 edge cases).
    assert_eq!(pv(DialectId::Postgres, None), "lex:pg-18@2026.11.1");
    assert_eq!(
        pv(DialectId::Postgres, Some((15, 4, 0))),
        "lex:pg-15@2026.11.1"
    );
    assert_eq!(
        pv(DialectId::Postgres, Some((16, 0, 0))),
        "lex:pg-16@2026.11.1"
    );
    assert_eq!(
        pv(DialectId::Postgres, Some((9, 6, 24))),
        "lex:pg-13@2026.11.1"
    );
    assert_eq!(
        pv(DialectId::Postgres, Some((19, 0, 0))),
        "lex:pg-18@2026.11.1"
    );
    // PG-wire flavours lex as PostgreSQL of the version they report.
    assert_eq!(
        pv(DialectId::PgWire(PgWireFlavor::Cockroach), Some((13, 0, 0))),
        "lex:pg-13@2026.11.1"
    );
    assert_eq!(
        pv(DialectId::MySql, Some((8, 0, 44))),
        "lex:mysql-8.0@2026.11.1"
    );
    assert_eq!(
        pv(DialectId::MySql, Some((8, 3, 0))),
        "lex:mysql-8.0@2026.11.1"
    );
    assert_eq!(
        pv(DialectId::MySql, Some((8, 4, 7))),
        "lex:mysql-8.4@2026.11.1"
    );
    assert_eq!(
        pv(DialectId::MySql, Some((9, 1, 0))),
        "lex:mysql-8.4@2026.11.1"
    );
    assert_eq!(pv(DialectId::MySql, None), "lex:mysql-8.4@2026.11.1");
    assert_eq!(
        pv(DialectId::MariaDb, Some((10, 6, 20))),
        "lex:mariadb-10.6@2026.11.1"
    );
    assert_eq!(
        pv(DialectId::MariaDb, Some((10, 11, 9))),
        "lex:mariadb-10.11@2026.11.1"
    );
    assert_eq!(
        pv(DialectId::MariaDb, Some((11, 2, 0))),
        "lex:mariadb-10.11@2026.11.1"
    );
    assert_eq!(
        pv(DialectId::MariaDb, Some((11, 4, 3))),
        "lex:mariadb-11.4@2026.11.1"
    );
    assert_eq!(pv(DialectId::MariaDb, None), "lex:mariadb-11.4@2026.11.1");
    assert_eq!(
        pv(DialectId::Sqlite, Some((3, 46, 0))),
        "lex:sqlite-3@2026.11.1"
    );
    assert_eq!(pv(DialectId::Generic, None), "lex:generic@2026.11.1");
    for id in [
        DialectId::SqlServer,
        DialectId::ClickHouse,
        DialectId::Oracle,
    ] {
        assert_eq!(pv(id, None), "lex:unsupported@2026.11.1", "{id:?}");
    }
}
