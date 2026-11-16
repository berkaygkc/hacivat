//! The harmless-`SET` list of read-only sessions (P-M3b-6; M3b Task 3).

use sql_dialect::DialectId;
use sql_safety::set_allowed;

#[test]
fn harmless_set_list() {
    assert!(set_allowed(DialectId::Postgres, "search_path"));
    assert!(set_allowed(DialectId::Postgres, "STATEMENT_TIMEOUT"));
    for v in [
        "role",
        "session authorization",
        "transaction_read_only",
        "default_transaction_read_only",
        "idle_in_transaction_session_timeout",
    ] {
        assert!(!set_allowed(DialectId::Postgres, v), "{v}");
    }
    assert!(!set_allowed(DialectId::MySql, "sql_mode"));
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
    for v in pg {
        assert!(set_allowed(DialectId::Postgres, v), "{v}");
        assert!(
            set_allowed(DialectId::Postgres, &v.to_ascii_uppercase()),
            "{v}"
        );
        assert!(
            set_allowed(DialectId::Postgres, &v.to_ascii_lowercase()),
            "{v}"
        );
    }
    let my = ["time_zone", "max_execution_time", "div_precision_increment"];
    for d in [DialectId::MySql, DialectId::MariaDb] {
        for v in my {
            assert!(set_allowed(d, v), "{d:?} {v}");
            assert!(set_allowed(d, &v.to_ascii_uppercase()), "{d:?} {v}");
        }
        for v in pg {
            assert!(!set_allowed(d, v) || my.contains(&v), "{d:?} {v}");
        }
        for v in [
            "sql_mode",
            "transaction_read_only",
            "autocommit",
            "sql_log_bin",
            "names",
        ] {
            assert!(!set_allowed(d, v), "{d:?} {v}");
        }
    }
    // SQLite has no SET; PRAGMA has its own list.
    for v in pg.iter().chain(my.iter()) {
        assert!(!set_allowed(DialectId::Sqlite, v), "{v}");
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
        assert!(!set_allowed(DialectId::Postgres, v), "{v:?}");
    }
    for v in [
        "@@time_zone",
        "@@session.time_zone",
        "SESSION time_zone",
        "tİme_zone",
    ] {
        assert!(!set_allowed(DialectId::MySql, v), "{v:?}");
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
        assert!(!set_allowed(d, "search_path"), "{d:?}");
        assert!(!set_allowed(d, "time_zone"), "{d:?}");
    }
}
