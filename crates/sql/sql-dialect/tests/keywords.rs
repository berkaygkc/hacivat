//! Keyword tables and lookup (SQL-LEX-01 tables, SQL-LEX-04 lookup; Task 3). The lookup is
//! ASCII-only: these tests also run under `LANG=tr_TR.UTF-8` (the turkey-test job), where a
//! locale-aware upper-casing of `limit` would give `LİMİT`.

use sql_dialect::keywords::{keyword_count, keyword_lookup_at};
use sql_dialect::{
    DialectId, KeywordId, KwCategory, PgWireFlavor, ServerVersion, keyword_lookup, keyword_text, kw,
};

const TABLED: [DialectId; 5] = [
    DialectId::Postgres,
    DialectId::MySql,
    DialectId::MariaDb,
    DialectId::Sqlite,
    DialectId::Generic,
];

fn v(major: u16, minor: u16, patch: u16) -> Option<ServerVersion> {
    Some(ServerVersion::new(major, minor, patch))
}

#[test]
fn keyword_lookup_is_ascii_case_insensitive() {
    for d in TABLED {
        for w in ["LIMIT", "limit", "Limit", "lImIt"] {
            assert!(keyword_lookup(d, w).is_some(), "{d:?} {w}");
        }
    }
}

#[test]
fn turkish_letters_never_match_keywords() {
    for d in TABLED {
        for w in [
            "lımıt",
            "lİmİt",
            "LİMİT",
            "SELECT\u{0307}",
            "ınsert",
            "İNSERT",
        ] {
            assert_eq!(keyword_lookup(d, w), None, "{d:?} {w}");
        }
    }
}

#[test]
fn cyrillic_and_full_width_lookalikes_are_not_keywords() {
    for d in TABLED {
        // U+0415 CYRILLIC CAPITAL LETTER IE in place of `E`.
        assert_eq!(keyword_lookup(d, "S\u{0415}LECT"), None, "{d:?}");
        // Full-width letters: NFKC would fold them to `SELECT`; servers reject them.
        assert_eq!(keyword_lookup(d, "ＳＥＬＥＣＴ"), None, "{d:?}");
    }
}

#[test]
fn word_over_32_bytes_is_none() {
    for d in TABLED {
        assert_eq!(keyword_lookup(d, &"A".repeat(33)), None, "{d:?}");
        assert_eq!(keyword_lookup(d, &"SELECT".repeat(64)), None, "{d:?}");
    }
    assert_eq!(keyword_lookup(DialectId::MySql, ""), None);
}

#[test]
fn pg_categories_follow_kwlist() {
    let cat = |w| keyword_lookup(DialectId::Postgres, w).map(|e| e.category);
    assert_eq!(cat("select"), Some(KwCategory::Reserved));
    assert_eq!(cat("abort"), Some(KwCategory::Unreserved));
    assert_eq!(cat("between"), Some(KwCategory::ColName));
    assert_eq!(cat("authorization"), Some(KwCategory::TypeFuncName));
    // PostgreSQL 18's kwlist.h, read from the vendored libpg_query 18.1.0.
    assert_eq!(keyword_count(DialectId::Postgres), 494);
}

#[test]
fn pg_wire_flavours_share_the_postgres_table() {
    for f in [
        PgWireFlavor::Redshift,
        PgWireFlavor::Cockroach,
        PgWireFlavor::Yugabyte,
        PgWireFlavor::Timescale,
        PgWireFlavor::Other,
    ] {
        let d = DialectId::PgWire(f);
        for w in ["select", "abort", "authorization", "zone"] {
            assert_eq!(
                keyword_lookup(d, w),
                keyword_lookup(DialectId::Postgres, w),
                "{w}"
            );
        }
        assert_eq!(keyword_count(d), keyword_count(DialectId::Postgres));
    }
}

#[test]
fn sqlite_list_has_147_keywords() {
    assert_eq!(keyword_count(DialectId::Sqlite), 147);
    assert!(keyword_lookup(DialectId::Sqlite, "pragma").is_some());
    assert!(keyword_lookup(DialectId::Sqlite, "returning").is_some());
    // `SHOW` is MySQL's, not SQLite's.
    assert_eq!(keyword_lookup(DialectId::Sqlite, "show"), None);
}

#[test]
fn mysql_reserved_flag_comes_from_the_server() {
    let cat = |d, w| keyword_lookup(d, w).map(|e| e.category);
    assert_eq!(cat(DialectId::MySql, "select"), Some(KwCategory::Reserved));
    // `STATUS` is a keyword MySQL lets you use as a column name.
    assert_eq!(
        cat(DialectId::MySql, "status"),
        Some(KwCategory::Unreserved)
    );
}

#[test]
fn generic_has_the_words_every_dialect_shares() {
    let g = |w| keyword_lookup(DialectId::Generic, w);
    for w in [
        "select", "values", "table", "with", "insert", "update", "delete", "into",
    ] {
        assert_eq!(
            g(w).map(|e| e.id),
            keyword_lookup(DialectId::Postgres, w).map(|e| e.id),
            "{w}"
        );
        assert_eq!(g(w).map(|e| e.category), Some(KwCategory::Reserved), "{w}");
    }
    // PostgreSQL-only (`ABORT` is no MySQL keyword) and MySQL-only (`SHOW` is no SQLite
    // keyword) words are not generic.
    assert_eq!(g("abort"), None);
    assert_eq!(g("show"), None);
    let generic = keyword_count(DialectId::Generic);
    assert!(
        generic > 50 && generic < keyword_count(DialectId::Sqlite),
        "{generic}"
    );
}

#[test]
fn unsupported_dialects_have_no_table() {
    for d in [
        DialectId::SqlServer,
        DialectId::ClickHouse,
        DialectId::Oracle,
    ] {
        assert_eq!(keyword_lookup(d, "select"), None, "{d:?}");
        assert_eq!(keyword_count(d), 0, "{d:?}");
    }
}

#[test]
fn named_keywords_resolve_to_their_constants() {
    let named = [
        (kw::SELECT, "SELECT"),
        (kw::VALUES, "VALUES"),
        (kw::TABLE, "TABLE"),
        (kw::WITH, "WITH"),
        (kw::SHOW, "SHOW"),
        (kw::INSERT, "INSERT"),
        (kw::UPDATE, "UPDATE"),
        (kw::DELETE, "DELETE"),
        (kw::MERGE, "MERGE"),
        (kw::INTO, "INTO"),
        (kw::FOR, "FOR"),
        (kw::LOCK, "LOCK"),
        (kw::SET, "SET"),
        (kw::BEGIN, "BEGIN"),
        (kw::END, "END"),
    ];
    for (id, text) in named {
        assert_eq!(keyword_text(id), text);
        assert_eq!(
            keyword_lookup(DialectId::Postgres, text).map(|e| e.id),
            Some(id),
            "{text}"
        );
        assert_eq!(
            keyword_lookup(DialectId::MySql, text).map(|e| e.id),
            Some(id),
            "{text}"
        );
    }
}

#[test]
fn keyword_text_is_canonical_upper_case_ascii() {
    for d in TABLED {
        for w in ["select", "Limit", "with"] {
            let id = keyword_lookup(d, w).unwrap().id;
            let text = keyword_text(id);
            assert!(text.eq_ignore_ascii_case(w), "{d:?} {w} {text}");
            assert!(
                text.bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            );
            assert_eq!(keyword_lookup(d, text).map(|e| e.id), Some(id));
        }
    }
    assert_eq!(keyword_text(KeywordId(u16::MAX)), "");
}

#[test]
fn mysql_rows_follow_the_server_line() {
    // As the servers report it: 8.4 added `QUALIFY` and dropped the `MASTER_*` words.
    let at = |version, w| keyword_lookup_at(DialectId::MySql, version, w).is_some();
    assert!(!at(v(8, 0, 46), "qualify"));
    // The 8.1–8.3 innovation releases read the 8.0 snapshot.
    assert!(!at(v(8, 3, 0), "qualify"));
    assert!(at(v(8, 4, 0), "qualify"));
    assert!(at(None, "qualify"));
    assert!(at(v(5, 7, 44), "master_host"));
    assert!(at(v(8, 0, 46), "master_host"));
    assert!(!at(v(8, 4, 7), "master_host"));
    assert!(!at(v(9, 1, 0), "master_host"));
    assert!(!at(None, "master_host"));
    // One id per word across lines and dialects.
    assert_eq!(
        keyword_lookup_at(DialectId::MySql, v(8, 0, 46), "select"),
        keyword_lookup_at(DialectId::MySql, v(8, 4, 7), "select")
    );
    assert_eq!(keyword_count(DialectId::MySql), 734);
}

#[test]
fn mariadb_rows_follow_the_server_line() {
    // As the servers report it: `CHANNEL` from 10.11, `VALIDATION` from 11.4.
    let at = |version, w| keyword_lookup_at(DialectId::MariaDb, version, w).is_some();
    assert!(!at(v(10, 6, 28), "channel"));
    assert!(at(v(10, 11, 0), "channel"));
    assert!(at(v(11, 4, 13), "channel"));
    assert!(!at(v(10, 11, 19), "validation"));
    assert!(at(v(11, 4, 0), "validation"));
    assert!(at(None, "validation"));
    assert_eq!(keyword_count(DialectId::MariaDb), 690);
}

#[test]
fn versioned_lookup_without_a_version_is_the_newest_table() {
    for d in TABLED {
        for w in ["select", "limit", "status", "abort", "pragma"] {
            assert_eq!(
                keyword_lookup_at(d, None, w),
                keyword_lookup(d, w),
                "{d:?} {w}"
            );
        }
    }
    // A table with one snapshot answers the same for every version.
    for version in [v(9, 6, 0), v(13, 0, 0), v(18, 1, 0), v(99, 0, 0)] {
        assert_eq!(
            keyword_lookup_at(DialectId::Postgres, version, "merge"),
            keyword_lookup(DialectId::Postgres, "merge")
        );
    }
}
