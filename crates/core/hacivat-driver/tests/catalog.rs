//! Engine catalog tests (Task 2): the embedded catalog, fallback chains and cycles, driver
//! resolution, loose versions, MySQL-family detection and the support floors (HV-DRV-01, C10).

use std::sync::Arc;

use hacivat_driver::fake::FakeDriver;
use hacivat_driver::{
    CatalogError, DriverRegistry, EngineCatalog, SemVerLoose, ServerInfo, Support,
    detect_mysql_family, support_status,
};
use hacivat_proto::DbmsKind;

/// `mariadb → mysql → mariadb`.
const CYCLE: &str = r#"
schema = 1

[[engine]]
id = "mariadb"
fallback = "mysql"
default_budget = 4

[[engine]]
id = "mysql"
fallback = "mariadb"
default_budget = 4
"#;

fn info(kind: DbmsKind, raw: &str) -> ServerInfo {
    ServerInfo {
        kind,
        product: String::new(),
        version: SemVerLoose::parse(raw),
        raw_version: raw.into(),
        is_replica: None,
    }
}

#[test]
fn embedded_catalog_has_the_v1_engines() {
    let c = EngineCatalog::embedded();
    assert_eq!(c.get(DbmsKind::Postgres).unwrap().default_port, Some(5432));
    assert_eq!(c.get(DbmsKind::MySql).unwrap().default_port, Some(3306));
    assert_eq!(c.get(DbmsKind::Sqlite).unwrap().default_port, None);
    assert_eq!(c.get(DbmsKind::Postgres).unwrap().default_budget, 4);
    assert_eq!(c.get(DbmsKind::Sqlite).unwrap().default_budget, 1);
    assert_eq!(
        c.chain(DbmsKind::MariaDb),
        vec![DbmsKind::MariaDb, DbmsKind::MySql]
    );
    assert_eq!(
        c.engines().map(|e| e.kind).collect::<Vec<_>>(),
        [
            DbmsKind::Postgres,
            DbmsKind::MySql,
            DbmsKind::MariaDb,
            DbmsKind::Sqlite
        ]
    );
}

#[test]
fn embedded_catalog_content_is_normative() {
    let c = EngineCatalog::embedded();
    let pg = c.get(DbmsKind::Postgres).unwrap();
    assert_eq!(pg.floor.as_deref(), Some("14"));
    assert_eq!(pg.tested, ["14", "15", "16", "17", "18"]);
    assert_eq!(pg.auth, ["password", "pgpass", "client_cert", "none"]);
    assert_eq!(
        pg.conn_types
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>(),
        ["default", "unix_socket", "url_only"]
    );
    assert_eq!(
        pg.defaults
            .iter()
            .map(|d| (d.key.as_str(), d.value.as_str()))
            .collect::<Vec<_>>(),
        [
            ("application_name", "Hacivat"),
            ("client_encoding", "UTF8"),
            ("extra_float_digits", "3")
        ]
    );
    assert!(pg.defaults.iter().all(|d| !d.rationale.is_empty()));
    assert_eq!(
        pg.option_allow,
        [
            "sslmode",
            "application_name",
            "connect_timeout",
            "options",
            "target_session_attrs",
            "sslrootcert",
            "sslcert",
            "sslkey"
        ]
    );
    assert_eq!(
        pg.safe_gucs,
        [
            "search_path",
            "statement_timeout",
            "application_name",
            "DateStyle",
            "TimeZone",
            "lc_messages"
        ]
    );
    assert_eq!(
        pg.path_keys,
        ["sslrootcert", "sslcert", "sslkey", "sslcrl", "passfile"]
    );

    let my = c.get(DbmsKind::MySql).unwrap();
    assert_eq!((my.floor.as_deref(), my.default_budget), (Some("8.0"), 4));
    assert_eq!(my.tested, ["8.0", "8.4"]);
    assert_eq!(my.auth, ["password", "client_cert", "none"]);
    assert_eq!(
        my.always_deny,
        [
            "allowLoadLocalInfile",
            "allowUrlInLocalInfile",
            "autoDeserialize"
        ]
    );
    assert_eq!(
        my.option_allow,
        ["sslMode", "useSSL", "connectTimeout", "characterEncoding"]
    );

    let maria = c.get(DbmsKind::MariaDb).unwrap();
    assert_eq!(maria.fallback, Some(DbmsKind::MySql));
    assert_eq!(maria.floor.as_deref(), Some("10.6"));
    assert_eq!(maria.tested, ["10.6", "10.11", "11.4"]);

    let lite = c.get(DbmsKind::Sqlite).unwrap();
    assert_eq!((lite.floor.as_deref(), lite.default_budget), (None, 1));
    assert_eq!(lite.auth, ["none"]);
    assert_eq!(
        lite.conn_types
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>(),
        ["file", "memory"]
    );
    assert_eq!(lite.always_deny, ["vfs", "load_extension"]);
    assert_eq!(lite.option_allow, ["mode"]);
}

#[test]
fn catalog_cycle_is_a_load_error() {
    let err = EngineCatalog::parse(CYCLE).unwrap_err();
    assert_eq!(
        err,
        CatalogError::Cycle(vec!["mariadb".into(), "mysql".into(), "mariadb".into()])
    );
}

#[test]
fn catalog_load_errors_name_the_problem() {
    let unknown_target =
        "schema = 1\n[[engine]]\nid = \"mariadb\"\nfallback = \"mysql\"\ndefault_budget = 4\n";
    assert_eq!(
        EngineCatalog::parse(unknown_target).unwrap_err(),
        CatalogError::UnknownTarget("mysql".into())
    );
    let bogus_target =
        "schema = 1\n[[engine]]\nid = \"mariadb\"\nfallback = \"mysqll\"\ndefault_budget = 4\n";
    assert_eq!(
        EngineCatalog::parse(bogus_target).unwrap_err(),
        CatalogError::UnknownTarget("mysqll".into())
    );
    assert!(matches!(
        EngineCatalog::parse("schema = 2\n"),
        Err(CatalogError::Parse(_))
    ));
    assert!(matches!(
        EngineCatalog::parse("schema = 1\n[[engine]]\nid = \"nosuch\"\ndefault_budget = 1\n"),
        Err(CatalogError::Parse(_))
    ));
    let duplicate = "schema = 1\n[[engine]]\nid = \"sqlite\"\ndefault_budget = 1\n[[engine]]\nid = \"sqlite\"\ndefault_budget = 1\n";
    assert!(matches!(
        EngineCatalog::parse(duplicate),
        Err(CatalogError::Parse(_))
    ));
}

#[test]
fn an_engine_without_a_fallback_or_outside_the_catalog_is_its_own_chain() {
    let c = EngineCatalog::embedded();
    // In the catalog, no fallback edge.
    assert_eq!(c.chain(DbmsKind::Postgres), vec![DbmsKind::Postgres]);
    // Not in the catalog at all.
    assert_eq!(c.chain(DbmsKind::Redis), vec![DbmsKind::Redis]);
}

#[test]
fn the_mariadb_handshake_prefix_is_stripped_only_before_a_digit() {
    let v = SemVerLoose::parse("5.5.5-10.6.18-MariaDB").unwrap();
    assert_eq!((v.major, v.minor, v.patch), (10, 6, 18));
    // Not followed by a digit: `5.5.5` is the version itself.
    let v = SemVerLoose::parse("5.5.5-MariaDB").unwrap();
    assert_eq!((v.major, v.minor, v.patch), (5, 5, 5));
    let v = SemVerLoose::parse("5.5.5-").unwrap();
    assert_eq!((v.major, v.minor, v.patch), (5, 5, 5));
}

#[test]
fn pre_releases_are_not_tested() {
    let c = EngineCatalog::embedded();
    for raw in [
        "14beta1",
        "17devel",
        "16rc1",
        "18beta2 (Debian 18~beta2-1)",
        "15RC2",
        "16-rc1",
    ] {
        assert_eq!(
            support_status(c, &info(DbmsKind::Postgres, raw)),
            Support::NotTested,
            "{raw}"
        );
    }
    // Below the floor, a pre-release is still unsupported.
    assert_eq!(
        support_status(c, &info(DbmsKind::Postgres, "13beta1")),
        Support::Unsupported { floor: "14".into() }
    );
    // Distribution suffixes are not pre-release markers.
    assert_eq!(
        support_status(
            c,
            &info(DbmsKind::Postgres, "16.4 (Debian 16.4-1.pgdg120+2)")
        ),
        Support::Supported
    );
    assert_eq!(
        support_status(c, &info(DbmsKind::MySql, "8.0.46-0ubuntu0.24.04.1")),
        Support::Supported
    );
}

#[test]
fn mariadb_resolves_to_mysql_driver_unless_registered() {
    let mut r = DriverRegistry::new();
    r.register(Arc::new(FakeDriver::new(DbmsKind::MySql)));
    assert_eq!(
        r.resolve(DbmsKind::MariaDb).unwrap().descriptor().kind,
        DbmsKind::MySql
    );
    r.register(Arc::new(FakeDriver::new(DbmsKind::MariaDb)));
    assert_eq!(
        r.resolve(DbmsKind::MariaDb).unwrap().descriptor().kind,
        DbmsKind::MariaDb
    );
    assert!(r.resolve(DbmsKind::Redis).is_none());
}

#[test]
fn an_unknown_driver_is_the_last_resort() {
    let mut r = DriverRegistry::new();
    assert!(r.resolve(DbmsKind::Postgres).is_none());
    r.register(Arc::new(FakeDriver::new(DbmsKind::Unknown)));
    assert_eq!(
        r.resolve(DbmsKind::Postgres).unwrap().descriptor().kind,
        DbmsKind::Unknown
    );
    r.register(Arc::new(FakeDriver::new(DbmsKind::Postgres)));
    assert_eq!(
        r.resolve(DbmsKind::Postgres).unwrap().descriptor().kind,
        DbmsKind::Postgres
    );
}

#[test]
fn loose_versions_keep_raw() {
    for (raw, v) in [
        ("16.15 (Debian 16.15-1.pgdg13+2)", (16, 15, 0)),
        ("8.0.46-0ubuntu", (8, 0, 46)),
        ("10.11.6-MariaDB-1:10.11.6+maria~ubu2204", (10, 11, 6)),
        ("5.5.5-10.11.6-MariaDB", (10, 11, 6)),
        ("3.50.4", (3, 50, 4)),
    ] {
        let s = SemVerLoose::parse(raw).unwrap();
        assert_eq!((s.major, s.minor, s.patch), v);
        assert_eq!(s.raw, raw);
    }
    assert_eq!(SemVerLoose::parse("devel"), None);
}

#[test]
fn loose_versions_edge_cases() {
    let pg = SemVerLoose::parse("PostgreSQL 17.2 on aarch64-unknown-linux-gnu").unwrap();
    assert_eq!((pg.major, pg.minor, pg.patch), (17, 2, 0));
    let four = SemVerLoose::parse("24.8.14.39").unwrap();
    assert_eq!((four.major, four.minor, four.patch), (24, 8, 14));
    let trailing_dot = SemVerLoose::parse("16.").unwrap();
    assert_eq!((trailing_dot.major, trailing_dot.minor), (16, 0));
    assert_eq!(SemVerLoose::parse(""), None);
    assert_eq!(SemVerLoose::parse("99999999999.1"), None);
    // Only ASCII digits count: Arabic-Indic digits are not a version.
    assert_eq!(SemVerLoose::parse("١٦.٢"), None);
}

#[test]
fn mysql_family_detection() {
    assert_eq!(
        detect_mysql_family("8.4.3", "MySQL Community Server - GPL"),
        DbmsKind::MySql
    );
    assert_eq!(
        detect_mysql_family("10.11.6-MariaDB", "mariadb.org binary distribution"),
        DbmsKind::MariaDb
    );
    assert_eq!(
        detect_mysql_family("11.4.2-MariaDB-ubu2404", ""),
        DbmsKind::MariaDb
    );
    assert_eq!(
        detect_mysql_family("8.0.11-TiDB-v7.5.0", ""),
        DbmsKind::TiDb
    );
}

#[test]
fn support_floors() {
    use Support::*;
    let c = EngineCatalog::embedded();
    for (k, v, want) in [
        (
            DbmsKind::Postgres,
            "13.18",
            Unsupported { floor: "14".into() },
        ),
        (DbmsKind::Postgres, "14.0", Supported),
        (DbmsKind::Postgres, "18.1", Supported),
        (DbmsKind::Postgres, "19.0", NotTested),
        (
            DbmsKind::MySql,
            "5.7.44",
            Unsupported {
                floor: "8.0".into(),
            },
        ),
        (DbmsKind::MySql, "8.0.46", Supported),
        (DbmsKind::MySql, "8.4.3", Supported),
        (DbmsKind::MySql, "9.1.0", NotTested),
        (
            DbmsKind::MariaDb,
            "10.5.27",
            Unsupported {
                floor: "10.6".into(),
            },
        ),
        (DbmsKind::MariaDb, "11.4.4", Supported),
        (DbmsKind::MariaDb, "11.2.6", NotTested),
        (DbmsKind::Sqlite, "3.50.4", Supported),
    ] {
        assert_eq!(support_status(c, &info(k, v)), want, "{k:?} {v}");
    }
    assert_eq!(
        support_status(
            c,
            &ServerInfo {
                version: None,
                ..info(DbmsKind::Postgres, "")
            }
        ),
        NotTested
    );
}

#[test]
fn support_edges() {
    let c = EngineCatalog::embedded();
    // MySQL 8.1 to 8.3 are innovation releases: above the floor, not tested.
    assert_eq!(
        support_status(c, &info(DbmsKind::MySql, "8.3.0")),
        Support::NotTested
    );
    // SQLite has no floor, so even a missing version is supported (P4).
    assert_eq!(
        support_status(
            c,
            &ServerInfo {
                version: None,
                ..info(DbmsKind::Sqlite, "")
            }
        ),
        Support::Supported
    );
    // An engine the catalog does not describe is never claimed as supported.
    assert_eq!(
        support_status(c, &info(DbmsKind::Redis, "7.4.12")),
        Support::NotTested
    );
}
