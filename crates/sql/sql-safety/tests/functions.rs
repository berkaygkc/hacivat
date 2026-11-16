//! SQL-SAFE-03 data: the per-dialect function allow and deny lists, operators, casts and types,
//! and the offline freshness digest (M3b Task 3).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use sql_dialect::{DialectId, ServerVersion};
use sql_ident::parse_qualified;
use sql_safety::functions::data::{DIGEST_FILES, digest_of_files};
use sql_safety::functions::{
    DenyGroup, FnClass, cast_known, denied_function_names, lookup_fn, operator_known,
};

fn pg() -> DialectId {
    DialectId::Postgres
}

fn sqlite() -> DialectId {
    DialectId::Sqlite
}

/// `text` read as a (qualified, quoted) name of `d`, looked up with no server version.
fn lookup(d: DialectId, text: &str) -> FnClass {
    lookup_at(d, text, None)
}

fn lookup_at(d: DialectId, text: &str, v: Option<ServerVersion>) -> FnClass {
    let name = parse_qualified(d, text).unwrap_or_else(|| panic!("{text:?} is a name"));
    lookup_fn(d, &name, v)
}

/// `hacivat_guard::PG_NON_TRANSACTIONAL_FUNCTIONS`, copied by name (`crates/sql` may not reach a
/// core crate; Task 18's `cross_crate_equalities` checks the copy).
fn hacivat_guard_list_fixture() -> [&'static str; 27] {
    [
        "pg_logical_emit_message",
        "pg_switch_wal",
        "pg_create_restore_point",
        "pg_log_standby_snapshot",
        "pg_backup_start",
        "pg_backup_stop",
        "pg_start_backup",
        "pg_stop_backup",
        "pg_promote",
        "pg_wal_replay_pause",
        "pg_wal_replay_resume",
        "pg_create_physical_replication_slot",
        "pg_create_logical_replication_slot",
        "pg_copy_physical_replication_slot",
        "pg_copy_logical_replication_slot",
        "pg_drop_replication_slot",
        "pg_replication_slot_advance",
        "pg_logical_slot_get_changes",
        "pg_logical_slot_get_binary_changes",
        "pg_sync_replication_slots",
        "pg_replication_origin_create",
        "pg_replication_origin_drop",
        "pg_replication_origin_advance",
        "pg_replication_origin_session_setup",
        "pg_replication_origin_session_reset",
        "pg_replication_origin_xact_setup",
        "pg_replication_origin_xact_reset",
    ]
}

#[test]
fn sleep_is_denied_in_every_spelling() {
    for n in [
        "pg_sleep",
        "PG_SLEEP",
        "pg_catalog.pg_sleep",
        "public.pg_sleep",
        r#""pg_sleep""#,
    ] {
        assert!(
            matches!(lookup(pg(), n), FnClass::Denied(DenyGroup::Sleep)),
            "{n}"
        );
    }
}

#[test]
fn denial_ignores_quoting_case_schema_and_version() {
    for n in [
        r#""PG_SLEEP""#,
        r#"mydb."Public"."Pg_Sleep_For""#,
        "PG_CATALOG.PG_SLEEP_UNTIL",
    ] {
        for v in [
            None,
            Some(ServerVersion::new(9, 6, 0)),
            Some(ServerVersion::new(18, 1, 0)),
        ] {
            assert!(
                matches!(lookup_at(pg(), n, v), FnClass::Denied(DenyGroup::Sleep)),
                "{n} {v:?}"
            );
        }
    }
}

#[test]
fn turkish_i_lookalike_is_unknown_not_allowed() {
    assert!(matches!(lookup(pg(), "PG_READ_FİLE"), FnClass::Unknown));
    // Dotless ı in `lower` never reaches the allowlisted `lower`.
    assert!(matches!(lookup(pg(), "LOWER"), FnClass::Allowed));
    assert!(matches!(lookup(pg(), "LOWER_ı"), FnClass::Unknown));
    assert!(matches!(lookup(pg(), "upper"), FnClass::Allowed));
    assert!(matches!(lookup(pg(), "UPPİR"), FnClass::Unknown));
}

#[test]
fn volatile_but_pure_builtins_are_allowed() {
    // Hand rows, source = "manual", listed in ADR 0005 (P-M3b-18: pg_proc marks them
    // VOLATILE or STABLE; the generator never proposes a volatile one).
    for n in ["random", "clock_timestamp", "gen_random_uuid", "now"] {
        assert!(matches!(lookup(pg(), n), FnClass::Allowed), "{n}");
    }
}

#[test]
fn query_to_xml_is_denied_dynamic_sql() {
    assert!(matches!(
        lookup(pg(), "query_to_xml"),
        FnClass::Denied(DenyGroup::DynamicSql)
    ));
}

#[test]
fn guard_non_transactional_functions_are_a_subset_of_the_deny_list() {
    // The fixture is copied by name; the equality test lives in Task 18.
    for f in hacivat_guard_list_fixture() {
        assert!(matches!(lookup(pg(), f), FnClass::Denied(_)), "{f}");
    }
    let denied: BTreeSet<&str> = denied_function_names(pg()).collect();
    for f in hacivat_guard_list_fixture() {
        assert!(denied.contains(f), "{f}");
    }
}

#[test]
fn sqlite_builtins_come_from_pragma_function_list() {
    assert!(matches!(
        lookup(sqlite(), "load_extension"),
        FnClass::Denied(DenyGroup::FileIo)
    ));
    assert!(matches!(lookup(sqlite(), "abs"), FnClass::Allowed));
    assert!(matches!(lookup(sqlite(), "ABS"), FnClass::Allowed));
    // A schema-qualified call is never a SQLite builtin.
    assert!(matches!(lookup(sqlite(), "main.abs"), FnClass::Unknown));
}

#[test]
fn the_alg13_deny_catalogue_is_present() {
    use DenyGroup::*;
    let cases: [(DialectId, &str, DenyGroup); 28] = [
        (pg(), "pg_terminate_backend", AdminSignal),
        (pg(), "pg_cancel_backend", AdminSignal),
        (pg(), "pg_reload_conf", AdminSignal),
        (pg(), "pg_stat_reset", AdminSignal),
        (pg(), "set_config", SetConfig),
        (pg(), "pg_advisory_lock", Advisory),
        (pg(), "pg_try_advisory_xact_lock_shared", Advisory),
        (pg(), "nextval", Sequence),
        (pg(), "setval", Sequence),
        (pg(), "txid_current", XidConsume),
        (pg(), "pg_current_xact_id", XidConsume),
        (pg(), "lo_export", FileIo),
        (pg(), "lo_import", FileIo),
        (pg(), "lo_from_bytea", LargeObject),
        (pg(), "lo_unlink", LargeObject),
        (pg(), "pg_read_file", FileIo),
        (pg(), "pg_ls_dir", FileIo),
        (pg(), "dblink_exec", Network),
        (pg(), "dblink", Network),
        (pg(), "http_get", Network),
        (pg(), "ts_stat", DynamicSql),
        (pg(), "cursor_to_xml", DynamicSql),
        (pg(), "pg_create_physical_replication_slot", Replication),
        (DialectId::MySql, "LOAD_FILE", FileIo),
        (DialectId::MySql, "get_lock", Advisory),
        (DialectId::MySql, "SLEEP", Sleep),
        (DialectId::MariaDb, "benchmark", Sleep),
        (DialectId::MariaDb, "NEXTVAL", Sequence),
    ];
    for (d, n, g) in cases {
        match lookup(d, n) {
            FnClass::Denied(got) => assert_eq!(got, g, "{d:?} {n}"),
            other => panic!("{d:?} {n}: {other:?}"),
        }
    }
}

#[test]
fn qualified_user_functions_never_borrow_a_builtin_allow() {
    // Schema-qualified calls use their schema (`alg/13` SAFE-03 step 1).
    assert!(matches!(lookup(pg(), "pg_catalog.lower"), FnClass::Allowed));
    assert!(matches!(lookup(pg(), "public.lower"), FnClass::Unknown));
    assert!(matches!(lookup(pg(), r#""LOWER""#), FnClass::Unknown));
    assert!(matches!(lookup(pg(), r#""lower""#), FnClass::Allowed));
    assert!(matches!(
        lookup(DialectId::MySql, "mydb.abs"),
        FnClass::Unknown
    ));
    assert!(matches!(lookup(DialectId::MySql, "ABS"), FnClass::Allowed));
    assert!(matches!(
        lookup(DialectId::MySql, "`abs`"),
        FnClass::Unknown
    ));
}

#[test]
fn version_gates_hold_and_unknown_versions_allow_only_ungated_rows() {
    // `regexp_like` arrived in PostgreSQL 15 (pg_proc of pg14..18).
    let v = |m| Some(ServerVersion::new(m, 0, 0));
    assert!(matches!(
        lookup_at(pg(), "regexp_like", v(15)),
        FnClass::Allowed
    ));
    assert!(matches!(
        lookup_at(pg(), "regexp_like", v(18)),
        FnClass::Allowed
    ));
    assert!(matches!(
        lookup_at(pg(), "regexp_like", v(14)),
        FnClass::Unknown
    ));
    assert!(matches!(
        lookup_at(pg(), "regexp_like", None),
        FnClass::Unknown
    ));
    assert!(matches!(lookup_at(pg(), "lower", None), FnClass::Allowed));
}

#[test]
fn other_dialects_allow_nothing_and_deny_by_every_list() {
    for d in [DialectId::Generic, DialectId::SqlServer, DialectId::Oracle] {
        assert!(matches!(lookup(d, "abs"), FnClass::Unknown), "{d:?}");
        assert!(
            matches!(lookup(d, "pg_sleep"), FnClass::Denied(DenyGroup::Sleep)),
            "{d:?}"
        );
        assert!(
            matches!(lookup(d, "load_file"), FnClass::Denied(DenyGroup::FileIo)),
            "{d:?}"
        );
    }
    let wire = DialectId::PgWire(sql_dialect::PgWireFlavor::Cockroach);
    assert!(matches!(lookup(wire, "lower"), FnClass::Unknown));
    assert!(matches!(
        lookup(wire, "pg_sleep"),
        FnClass::Denied(DenyGroup::Sleep)
    ));
}

#[test]
fn operators_and_casts() {
    for op in ["+", "=", "<>", "||", "->>", "@>", "~~*", "#>>"] {
        assert!(operator_known(pg(), op), "{op}");
    }
    assert!(!operator_known(pg(), "###"));
    for t in [
        "int4",
        "INTEGER",
        "pg_catalog.int8",
        "double precision",
        "varchar(10)",
        "timestamp with time zone",
        "numeric(10, 2)",
        "text[]",
        "jsonb",
        "regclass",
    ] {
        assert!(cast_known(pg(), t), "{t}");
    }
    for t in [
        "my_domain",
        "public.int4",
        "internal",
        "cstring",
        "İnt4",
        "",
    ] {
        assert!(!cast_known(pg(), t), "{t}");
    }
    assert!(operator_known(DialectId::MySql, "<=>"));
    assert!(cast_known(DialectId::MySql, "SIGNED"));
    assert!(cast_known(DialectId::MariaDb, "char(10)"));
    assert!(operator_known(sqlite(), "||"));
    assert!(cast_known(sqlite(), "INTEGER"));
    assert!(!operator_known(DialectId::Oracle, "+"));
}

#[test]
fn word_operators_match_in_any_ascii_case() {
    for (d, op) in [
        (DialectId::MySql, "like"),
        (DialectId::MySql, "Div"),
        (DialectId::MySql, "between"),
        (DialectId::MariaDb, "rlike"),
        (sqlite(), "glob"),
        (sqlite(), "Is"),
    ] {
        assert!(operator_known(d, op), "{d:?} {op}");
    }
    // Never a locale fold.
    assert!(!operator_known(DialectId::MySql, "lİke"));
    assert!(!operator_known(DialectId::MySql, "lıke"));
}

#[test]
fn more_builtin_cast_spellings() {
    for t in [
        "interval day to second",
        "INTERVAL YEAR TO MONTH",
        "interval second(3)",
        "\"char\"",
    ] {
        assert!(cast_known(pg(), t), "{t}");
    }
    assert!(!cast_known(pg(), "interval fortnight"));
    assert!(!cast_known(pg(), "\"CHAR\""));
    assert!(cast_known(
        DialectId::MySql,
        "CHAR(10) CHARACTER SET utf8mb4"
    ));
    assert!(cast_known(DialectId::MariaDb, "char charset latin1"));
    // Multi-valued index casts stay unknown (callers treat the cast as unknown).
    assert!(!cast_known(DialectId::MySql, "UNSIGNED ARRAY"));
}

#[test]
fn nothing_is_proven_below_the_probe_floor() {
    let at = |d, n, a, b, c| lookup_at(d, n, Some(ServerVersion::new(a, b, c)));
    // MySQL 8.0 adds builtins in patch releases: hand `since` gates from the change history.
    let my = DialectId::MySql;
    assert!(matches!(at(my, "FORMAT_BYTES", 8, 0, 12), FnClass::Unknown));
    assert!(matches!(at(my, "FORMAT_BYTES", 8, 0, 16), FnClass::Allowed));
    assert!(matches!(at(my, "JSON_VALUE", 8, 0, 20), FnClass::Unknown));
    assert!(matches!(at(my, "JSON_VALUE", 8, 0, 21), FnClass::Allowed));
    assert!(matches!(at(my, "ST_COLLECT", 8, 0, 23), FnClass::Unknown));
    assert!(matches!(at(my, "ABS", 8, 0, 12), FnClass::Allowed));
    assert!(matches!(at(my, "ABS", 5, 7, 44), FnClass::Unknown));
    // PostgreSQL below its floor (14): nothing.
    assert!(matches!(at(pg(), "date_bin", 13, 22, 0), FnClass::Unknown));
    assert!(matches!(at(pg(), "lower", 13, 22, 0), FnClass::Unknown));
    assert!(matches!(at(pg(), "date_bin", 14, 0, 0), FnClass::Allowed));
    // MariaDB 10.6.0 predates SYS_GUID, TO_CHAR and ADD_MONTHS (10.6.1).
    let maria = DialectId::MariaDb;
    for n in ["SYS_GUID", "TO_CHAR", "ADD_MONTHS"] {
        assert!(matches!(at(maria, n, 10, 6, 0), FnClass::Unknown), "{n}");
        assert!(matches!(at(maria, n, 10, 6, 1), FnClass::Allowed), "{n}");
    }
    assert!(matches!(at(maria, "ABS", 10, 6, 0), FnClass::Allowed));
    assert!(matches!(at(maria, "ABS", 10, 5, 9), FnClass::Unknown));
    // An unknown version reads rows that hold from the floor on.
    assert!(matches!(lookup(my, "ABS"), FnClass::Allowed));
    assert!(matches!(lookup(my, "FORMAT_BYTES"), FnClass::Allowed));
    assert!(matches!(lookup(maria, "CRC32C"), FnClass::Unknown));
}

#[test]
fn json_to_record_coercions_are_unknown() {
    // They coerce into a record's column types, so a domain's CHECK can run a user function
    // (fix round 1, I3; the same reason as `record_in`).
    for n in [
        "json_populate_record",
        "json_populate_recordset",
        "jsonb_populate_record",
        "jsonb_populate_recordset",
        "jsonb_populate_record_valid",
        "json_to_record",
        "json_to_recordset",
        "jsonb_to_record",
        "jsonb_to_recordset",
        "record_in",
        "domain_in",
    ] {
        assert!(
            matches!(
                lookup_at(pg(), n, Some(ServerVersion::new(17, 2, 0))),
                FnClass::Unknown
            ),
            "{n}"
        );
    }
}

#[test]
fn a_unicode_space_never_reaches_pg_catalog() {
    // The server reads `\u{a0}pg_catalog` as another schema (fix round 1, I4).
    for n in ["\u{a0}pg_catalog.lower", "pg_catalog.\u{2003}lower"] {
        assert!(matches!(lookup(pg(), n), FnClass::Unknown), "{n:?}");
    }
}

#[test]
fn no_name_is_both_allowed_and_denied() {
    for d in [pg(), DialectId::MySql, DialectId::MariaDb, sqlite()] {
        for n in denied_function_names(d) {
            assert!(matches!(lookup(d, n), FnClass::Denied(_)), "{d:?} {n}");
        }
        let allow = rows(&engine_dir(d), "allow.toml");
        let denied: BTreeSet<String> = denied_function_names(d).map(str::to_string).collect();
        for r in allow {
            let name = r["name"].as_str().unwrap().to_ascii_lowercase();
            assert!(!denied.contains(&name), "{d:?} {name} is in both lists");
        }
    }
}

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("data")
}

fn engine_dir(d: DialectId) -> PathBuf {
    data_dir().join(match d {
        DialectId::Postgres => "pg",
        DialectId::MySql => "mysql",
        DialectId::MariaDb => "mariadb",
        DialectId::Sqlite => "sqlite",
        other => panic!("{other:?} has no data"),
    })
}

fn rows(dir: &Path, file: &str) -> Vec<toml::Table> {
    let text = std::fs::read_to_string(dir.join(file)).unwrap();
    let doc: toml::Table = toml::from_str(&text).unwrap_or_else(|e| panic!("{file}: {e}"));
    let keys: Vec<&String> = doc.keys().collect();
    if file == "allow.toml" {
        // `probe_floor`: the oldest server the catalog read saw (fix round 1, I2).
        assert_eq!(
            keys,
            ["probe_floor", "rows"],
            "{}: `probe_floor` and `rows`",
            dir.join(file).display()
        );
    } else {
        assert_eq!(keys, ["rows"], "{}: only `rows`", dir.join(file).display());
    }
    doc["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_table().unwrap().clone())
        .collect()
}

#[test]
fn every_row_names_its_source_and_review_state() {
    let groups = [
        "Sleep",
        "FileIo",
        "Network",
        "ProcessExec",
        "AdminSignal",
        "Replication",
        "LargeObject",
        "DynamicSql",
        "Advisory",
        "Sequence",
        "XidConsume",
        "SetConfig",
    ];
    let mut counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for engine in ["pg", "mysql", "mariadb", "sqlite"] {
        let dir = data_dir().join(engine);
        for file in [
            "allow.toml",
            "deny.toml",
            "operators.toml",
            "types.toml",
            "set_allow.toml",
        ] {
            let at = format!("{engine}/{file}");
            let mut seen = BTreeSet::new();
            for r in rows(&dir, file) {
                let name = r.get("name").and_then(|v| v.as_str()).unwrap_or_default();
                assert!(!name.is_empty(), "{at}: a row without a name: {r:?}");
                assert!(seen.insert(name.to_ascii_lowercase()), "{at}: {name} twice");
                for key in r.keys() {
                    assert!(
                        ["name", "since", "until", "source", "reviewed", "group"]
                            .contains(&key.as_str()),
                        "{at}: {name}: unknown column {key}"
                    );
                }
                let source = r.get("source").and_then(|v| v.as_str()).unwrap_or_default();
                let ok = match engine {
                    "pg" => {
                        source == "manual"
                            || ["pg_proc@pg", "pg_operator@pg", "pg_type@pg"]
                                .iter()
                                .any(|p| {
                                    source
                                        .strip_prefix(p)
                                        .is_some_and(|m| m.parse::<u16>().is_ok())
                                })
                    }
                    "mysql" | "mariadb" => ["manual", "manual-index"].contains(&source),
                    _ => ["manual", "pragma_function_list"].contains(&source),
                };
                assert!(ok, "{at}: {name}: source {source:?}");
                assert!(
                    r.get("reviewed").and_then(|v| v.as_bool()).is_some(),
                    "{at}: {name}: no reviewed = true|false"
                );
                for gate in ["since", "until"] {
                    if let Some(v) = r.get(gate) {
                        let v = v.as_str().unwrap_or_default();
                        assert!(
                            !v.is_empty() && v.split('.').all(|p| p.parse::<u16>().is_ok()),
                            "{at}: {name}: {gate} = {v:?}"
                        );
                    }
                }
                let group = r.get("group").and_then(|v| v.as_str());
                if file == "deny.toml" {
                    assert!(
                        group.is_some_and(|g| groups.contains(&g)),
                        "{at}: {name}: group {group:?}"
                    );
                    assert_eq!(r["reviewed"].as_bool(), Some(true), "{at}: {name}");
                } else {
                    assert!(group.is_none(), "{at}: {name}: a group outside deny.toml");
                }
                let c = counts.entry(at.clone()).or_default();
                if r["reviewed"].as_bool() == Some(true) {
                    c.0 += 1;
                } else {
                    c.1 += 1;
                }
            }
        }
    }
    // Every list a lookup reads has rows.
    for at in [
        "pg/allow.toml",
        "pg/deny.toml",
        "pg/operators.toml",
        "pg/types.toml",
    ] {
        assert!(counts[at].0 > 0, "{at} has no reviewed row");
    }
}

#[test]
fn unreviewed_rows_never_count_as_allowed() {
    for d in [pg(), DialectId::MySql, DialectId::MariaDb, sqlite()] {
        for r in rows(&engine_dir(d), "allow.toml") {
            if r["reviewed"].as_bool() == Some(false) {
                let name = r["name"].as_str().unwrap();
                assert!(
                    !matches!(
                        lookup_fn(d, &plain(d, name), Some(ServerVersion::new(17, 2, 0))),
                        FnClass::Allowed
                    ),
                    "{d:?} {name}"
                );
            }
        }
        for r in rows(&engine_dir(d), "operators.toml") {
            if r["reviewed"].as_bool() == Some(false) {
                assert!(!operator_known(d, r["name"].as_str().unwrap()));
            }
        }
        for r in rows(&engine_dir(d), "types.toml") {
            if r["reviewed"].as_bool() == Some(false) {
                assert!(!cast_known(d, r["name"].as_str().unwrap()));
            }
        }
    }
}

/// `name` as one unquoted part.
fn plain(_d: DialectId, name: &str) -> sql_ident::QualifiedName {
    let mut q = sql_ident::QualifiedName::default();
    q.parts.push(sql_ident::Ident {
        text: name.into(),
        quoted: false,
    });
    q
}

/// The digest of the files under `dir` (relative to the crate), in [`DIGEST_FILES`] order.
fn digest_of(dir: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(dir);
    let bytes: Vec<(&str, Vec<u8>)> = DIGEST_FILES
        .iter()
        .map(|f| {
            (
                *f,
                std::fs::read(root.join(f)).unwrap_or_else(|e| panic!("{f}: {e}")),
            )
        })
        .collect();
    digest_of_files(bytes.iter().map(|(f, b)| (*f, b.as_slice())))
}

#[test]
fn data_matches_the_checked_in_digest() {
    assert_eq!(
        digest_of("data"),
        include_str!("../data/functions.digest").trim()
    );
    // The compiled-in tables are the files on disk.
    assert_eq!(
        sql_safety::functions::data::compiled_digest(),
        digest_of("data")
    );
}

#[test]
fn the_digest_covers_every_data_file_once() {
    let mut on_disk = BTreeSet::new();
    for engine in ["pg", "mysql", "mariadb", "sqlite"] {
        for e in std::fs::read_dir(data_dir().join(engine)).unwrap() {
            let name = e.unwrap().file_name().into_string().unwrap();
            if [
                "allow.toml",
                "deny.toml",
                "operators.toml",
                "types.toml",
                "set_allow.toml",
            ]
            .contains(&name.as_str())
            {
                on_disk.insert(format!("{engine}/{name}"));
            }
        }
    }
    on_disk.insert("mysql/manual-index.txt".to_string());
    let listed: BTreeSet<String> = DIGEST_FILES.iter().map(|f| f.to_string()).collect();
    assert_eq!(listed.len(), DIGEST_FILES.len(), "a file listed twice");
    assert_eq!(listed, on_disk);
}

#[test]
fn digest_is_order_and_boundary_sensitive_and_line_ending_blind() {
    let a = digest_of_files([("x", b"ab".as_slice()), ("y", b"c".as_slice())]);
    let b = digest_of_files([("x", b"a".as_slice()), ("y", b"bc".as_slice())]);
    let c = digest_of_files([("y", b"c".as_slice()), ("x", b"ab".as_slice())]);
    let crlf = digest_of_files([("x", b"a\r\nb".as_slice())]);
    let lf = digest_of_files([("x", b"a\nb".as_slice())]);
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert_eq!(crlf, lf);
    assert!(
        a.starts_with("xxh3-64:") && a.len() == "xxh3-64:".len() + 16,
        "{a}"
    );
}

#[test]
fn the_digest_matches_xtask_on_a_known_answer() {
    // The same vector is pinned in `xtask/src/gen_fn_catalog.rs` (R1-M3: one algorithm on both
    // sides; xtask takes no SQL crate).
    assert_eq!(
        digest_of_files([("x", b"ab".as_slice()), ("y", b"c\r\n".as_slice())]),
        "xxh3-64:f1ee6490ac0306c7"
    );
}
