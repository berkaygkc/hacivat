//! Server versions and the lexer profile's version string (Task 3).

use crate::id::{DialectCtx, DialectId};

/// A server version; versions order by major, minor, patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ServerVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl ServerVersion {
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        ServerVersion {
            major,
            minor,
            patch,
        }
    }
}

/// The release of the profile data: the keyword tables and the lexer rules. Bump it whenever
/// either changes, so caches and audit records keyed on [`profile_version`] see the change;
/// `keyword_data_matches_the_release` fails when a keyword table changes without a bump.
macro_rules! release {
    () => {
        "2026.11.1"
    };
}

/// One profile string per server line Hacivat's tables tell apart.
macro_rules! profile {
    ($name:literal) => {
        concat!("lex:", $name, "@", release!())
    };
}

/// The PostgreSQL lines, oldest first: majors 13 to 18 each get their own profile (the lexer's
/// version rules change at major boundaries: trailing junk after numbers from 15, separators
/// and `0x`/`0o`/`0b` from 16). Older servers use the oldest, newer and unknown ones the newest.
const PG_LINES: [(u16, &str); 6] = [
    (13, profile!("pg-13")),
    (14, profile!("pg-14")),
    (15, profile!("pg-15")),
    (16, profile!("pg-16")),
    (17, profile!("pg-17")),
    (18, profile!("pg-18")),
];

/// The MySQL lines with a keyword snapshot (`data/keywords/mysql80.toml`, `mysql84.toml`), as
/// `(major, minor)` of their first version, oldest first. The 8.1–8.3 innovation releases use
/// the 8.0 snapshot; 9.x and unknown versions the newest.
pub(crate) const MYSQL_LINES: [((u16, u16), &str); 2] = [
    ((8, 0), profile!("mysql-8.0")),
    ((8, 4), profile!("mysql-8.4")),
];

/// The MariaDB lines with a keyword snapshot (`data/keywords/mariadb.toml`), oldest first.
pub(crate) const MARIADB_LINES: [((u16, u16), &str); 3] = [
    ((10, 6), profile!("mariadb-10.6")),
    ((10, 11), profile!("mariadb-10.11")),
    ((11, 4), profile!("mariadb-11.4")),
];

const SQLITE: &str = profile!("sqlite-3");
const GENERIC: &str = profile!("generic");
const UNSUPPORTED: &str = profile!("unsupported");

/// The newest line at or below `version` (the oldest when `version` is below all of them, the
/// newest when it is unknown).
fn line_for<K: Copy + PartialOrd>(lines: &[(K, &'static str)], version: Option<K>) -> &'static str {
    let newest = lines.len() - 1;
    let i = match version {
        None => newest,
        Some(v) => lines
            .iter()
            .rposition(|&(first, _)| first <= v)
            .unwrap_or(0),
    };
    lines[i].1
}

/// The version of the lexer profile `ctx` selects, e.g. `"lex:pg-18@2026.11.1"`: caches of
/// lexed text and audit records key on it. It names the dialect, the server line Hacivat's
/// tables tell apart, and the profile data release. PG-wire flavours lex as PostgreSQL of the
/// version they report (alg/10 LEX-01), so they share its strings.
///
/// What a cache keyed on it must add:
/// - **The lexing modes.** They are a separate input ([`crate::LexModes`]) and are not part of
///   the string, so a cache of lexed text keys on (this string, `LexModes`).
/// - **The full server version, for executable-comment gates.** The string names a server line,
///   not a patch release. A gate result (`/*!80031 … */` active or not) depends on the exact
///   version, so nothing may cache gate results under this string alone. The full
///   [`ServerVersion`] must be part of that key.
///
/// Equal strings always select the same [`crate::lex_profile`]; the test
/// `equal_profile_versions_select_the_same_lexer_profile` holds the lexer's buckets to this.
pub fn profile_version(ctx: &DialectCtx) -> &'static str {
    let line = ctx.version.map(|v| (v.major, v.minor));
    match ctx.id {
        DialectId::Postgres | DialectId::PgWire(_) => {
            line_for(&PG_LINES, ctx.version.map(|v| v.major))
        }
        DialectId::MySql => line_for(&MYSQL_LINES, line),
        DialectId::MariaDb => line_for(&MARIADB_LINES, line),
        DialectId::Sqlite => SQLITE,
        DialectId::Generic => GENERIC,
        DialectId::SqlServer | DialectId::ClickHouse | DialectId::Oracle => UNSUPPORTED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::{keywords_mariadb, keywords_mysql, keywords_pg, keywords_sqlite};

    /// The keyword data each `release!()` was cut from: the FNV-1a 64 hash of each table's
    /// TOML sources (LF line ends), as `gen-sql-tables` writes it into the generated file.
    const RELEASE_DATA: [(&str, u64); 4] = [
        ("pg", keywords_pg::DATA_HASH),
        ("mysql", keywords_mysql::DATA_HASH),
        ("mariadb", keywords_mariadb::DATA_HASH),
        ("sqlite", keywords_sqlite::DATA_HASH),
    ];

    #[test]
    fn keyword_data_matches_the_release() {
        // When a table changes, bump `release!()` and record the new hashes here.
        let recorded: [(&str, u64); 4] = [
            ("pg", 11_486_560_768_953_523_152),
            ("mysql", 12_293_902_648_759_598_950),
            ("mariadb", 5_712_969_847_804_070_839),
            ("sqlite", 15_006_276_825_970_538_428),
        ];
        assert_eq!(release!(), "2026.11.1");
        assert_eq!(
            RELEASE_DATA, recorded,
            "keyword data changed: bump release!()"
        );
    }

    /// Every dialect over a grid of versions (and no version), with its default modes.
    fn version_grid() -> Vec<DialectCtx> {
        use crate::id::PgWireFlavor;
        let mut pg = vec![None];
        for major in 7..=21 {
            for (minor, patch) in [(0, 0), (4, 3)] {
                pg.push(Some(ServerVersion::new(major, minor, patch)));
            }
        }
        let mut mysql = vec![None, Some(ServerVersion::new(5, 7, 44))];
        for (major, minors) in [(8, 0..=4), (9, 0..=3)] {
            for minor in minors {
                for patch in [0, 30, 46] {
                    mysql.push(Some(ServerVersion::new(major, minor, patch)));
                }
            }
        }
        let mut mariadb = vec![None];
        for (major, minors) in [(10, 0..=11), (11, 0..=8), (12, 0..=1)] {
            for minor in minors {
                for patch in [0, 5] {
                    mariadb.push(Some(ServerVersion::new(major, minor, patch)));
                }
            }
        }
        let sqlite: Vec<_> = std::iter::once(None)
            .chain((30..=50).map(|minor| Some(ServerVersion::new(3, minor, 0))))
            .collect();
        let mut grid = Vec::new();
        let mut add = |id, versions: &[Option<ServerVersion>]| {
            grid.extend(versions.iter().map(|&v| DialectCtx::new(id, v)));
        };
        add(DialectId::Postgres, &pg);
        for f in [
            PgWireFlavor::Redshift,
            PgWireFlavor::Cockroach,
            PgWireFlavor::Yugabyte,
            PgWireFlavor::Timescale,
            PgWireFlavor::Other,
        ] {
            add(DialectId::PgWire(f), &pg);
        }
        add(DialectId::MySql, &mysql);
        add(DialectId::MariaDb, &mariadb);
        add(DialectId::Sqlite, &sqlite);
        add(DialectId::Generic, &sqlite);
        for id in [
            DialectId::SqlServer,
            DialectId::ClickHouse,
            DialectId::Oracle,
        ] {
            add(id, &[None, Some(ServerVersion::new(16, 0, 0))]);
        }
        grid
    }

    #[test]
    fn equal_profile_versions_select_the_same_lexer_profile() {
        // Caches and audit records treat one string as one lexer: the lexer's version buckets
        // (Task 4's `lex_profile`) must never be finer than these strings.
        let mut first: std::collections::BTreeMap<&str, DialectCtx> = Default::default();
        for ctx in version_grid() {
            let pv = profile_version(&ctx);
            let seen = *first.entry(pv).or_insert(ctx);
            let same = match (crate::lex_profile(&seen), crate::lex_profile(&ctx)) {
                (Ok(a), Ok(b)) => std::ptr::eq(a, b),
                (Err(_), Err(_)) => true,
                _ => false,
            };
            assert!(same, "{pv}: {seen:?} and {ctx:?} lex differently");
        }
        // The grid reaches every string.
        let all: Vec<&str> = PG_LINES
            .iter()
            .map(|l| l.1)
            .chain(MYSQL_LINES.iter().map(|l| l.1))
            .chain(MARIADB_LINES.iter().map(|l| l.1))
            .chain([SQLITE, GENERIC, UNSUPPORTED])
            .collect();
        assert_eq!(first.len(), all.len());
        assert!(all.iter().all(|s| first.contains_key(s)));
    }

    #[test]
    fn profile_lines_match_the_keyword_snapshots() {
        let firsts = |lines: &[((u16, u16), &str)]| lines.iter().map(|l| l.0).collect::<Vec<_>>();
        assert_eq!(keywords_mysql::SNAPSHOTS, firsts(&MYSQL_LINES));
        assert_eq!(keywords_mariadb::SNAPSHOTS, firsts(&MARIADB_LINES));
        assert_eq!(keywords_pg::SNAPSHOTS.len(), 1);
        assert_eq!(keywords_sqlite::SNAPSHOTS.len(), 1);
    }
}
