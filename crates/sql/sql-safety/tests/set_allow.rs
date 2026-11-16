//! The harmless-`SET` list of read-only sessions (P-M3b-6, P-M3b-T3-1; M3b Task 3).

use sql_dialect::DialectId;
use sql_safety::set_allowed;

/// A value for a variable whose value does not matter.
const V: Option<&str> = Some("1");

#[test]
fn harmless_set_list() {
    // The plan's values; `search_path` now also needs a value (P-M3b-T3-1).
    assert!(set_allowed(
        DialectId::Postgres,
        "search_path",
        Some("public")
    ));
    assert!(set_allowed(DialectId::Postgres, "STATEMENT_TIMEOUT", V));
    for v in [
        "role",
        "session authorization",
        "transaction_read_only",
        "default_transaction_read_only",
        "idle_in_transaction_session_timeout",
    ] {
        assert!(!set_allowed(DialectId::Postgres, v, V), "{v}");
    }
    assert!(!set_allowed(DialectId::MySql, "sql_mode", V));
}

#[test]
fn search_path_is_harmless_only_while_pg_catalog_resolves_first() {
    let sp = |v: Option<&str>| set_allowed(DialectId::Postgres, "search_path", v);
    // The ruling's cases.
    assert!(!sp(Some("public, pg_catalog")));
    assert!(sp(Some("pg_catalog, public")));
    assert!(sp(Some("public")));
    assert!(!sp(Some(r#""PG_CATALOG", public, pg_catalog"#)));
    assert!(!sp(None));
    // Unquoted names fold; a quoted name is exact; a string literal is one schema.
    assert!(sp(Some("PG_CATALOG, public")));
    assert!(sp(Some(r#""PG_CATALOG", public"#)));
    assert!(!sp(Some("public, PG_Catalog")));
    assert!(!sp(Some(r#""$user", public, "pg_catalog""#)));
    assert!(sp(Some(r#""$user", public"#)));
    assert!(sp(Some("'pg_catalog', public")));
    assert!(!sp(Some("public, 'pg_catalog'")));
    assert!(sp(Some("'public, pg_catalog'")));
    // A Unicode space is part of a name: this is not `pg_catalog`.
    assert!(!sp(Some("\u{a0}pg_catalog, x, pg_catalog")));
    // Values that do not parse are refused.
    for bad in [
        "",
        "public,",
        "public pg_catalog",
        "\"public",
        "public; RESET ALL",
    ] {
        assert!(!sp(Some(bad)), "{bad:?}");
    }
    // The name still matches ASCII-case-insensitively.
    assert!(set_allowed(
        DialectId::Postgres,
        "SEARCH_PATH",
        Some("public")
    ));
    assert!(!set_allowed(
        DialectId::Postgres,
        "Search_Path",
        Some("public, pg_catalog")
    ));
}

#[test]
fn other_variables_ignore_the_value() {
    for v in [None, Some("x"), Some("public, pg_catalog")] {
        assert!(set_allowed(DialectId::Postgres, "work_mem", v), "{v:?}");
        assert!(set_allowed(DialectId::MySql, "time_zone", v), "{v:?}");
        assert!(!set_allowed(DialectId::Postgres, "role", v), "{v:?}");
    }
    // MySQL has no `search_path`.
    assert!(!set_allowed(
        DialectId::MySql,
        "search_path",
        Some("public")
    ));
}

#[test]
fn the_p_m3b_6_lists_exactly() {
    let pg = [
        "search_path",
        "statement_timeout",
        "lock_timeout",
        "work_mem",
        "DateStyle",
        "IntervalStyle",
        "TimeZone",
        "extra_float_digits",
        "application_name",
    ];
    let value = Some("public");
    for v in pg {
        assert!(set_allowed(DialectId::Postgres, v, value), "{v}");
        assert!(
            set_allowed(DialectId::Postgres, &v.to_ascii_uppercase(), value),
            "{v}"
        );
        assert!(
            set_allowed(DialectId::Postgres, &v.to_ascii_lowercase(), value),
            "{v}"
        );
    }
    let my = ["time_zone", "max_execution_time", "div_precision_increment"];
    for d in [DialectId::MySql, DialectId::MariaDb] {
        for v in my {
            assert!(set_allowed(d, v, V), "{d:?} {v}");
            assert!(set_allowed(d, &v.to_ascii_uppercase(), V), "{d:?} {v}");
        }
        for v in pg {
            assert!(!set_allowed(d, v, value) || my.contains(&v), "{d:?} {v}");
        }
        for v in [
            "sql_mode",
            "transaction_read_only",
            "autocommit",
            "sql_log_bin",
            "names",
        ] {
            assert!(!set_allowed(d, v, V), "{d:?} {v}");
        }
    }
    // SQLite has no SET; PRAGMA has its own list.
    for v in pg.iter().chain(my.iter()) {
        assert!(!set_allowed(DialectId::Sqlite, v, value), "{v}");
    }
}

#[test]
fn names_match_exactly_ascii_case_insensitively() {
    // No prefix, suffix, padding, scope or locale folding.
    for v in [
        "search_path ",
        " search_path",
        "search_pat",
        "search_paths",
        "pg_catalog.search_path",
        "session.search_path",
        "SEARCH_PATH;",
        "TİMEZONE",
        "tımezone",
        "",
    ] {
        assert!(
            !set_allowed(DialectId::Postgres, v, Some("public")),
            "{v:?}"
        );
    }
    for v in [
        "@@time_zone",
        "@@session.time_zone",
        "SESSION time_zone",
        "tİme_zone",
    ] {
        assert!(!set_allowed(DialectId::MySql, v, V), "{v:?}");
    }
}

#[test]
fn other_dialects_allow_no_set() {
    for d in [
        DialectId::Generic,
        DialectId::SqlServer,
        DialectId::ClickHouse,
        DialectId::Oracle,
        DialectId::PgWire(sql_dialect::PgWireFlavor::Other),
    ] {
        assert!(!set_allowed(d, "search_path", Some("public")), "{d:?}");
        assert!(!set_allowed(d, "time_zone", V), "{d:?}");
    }
}
