// ## m4-t23-minicat
//! The catalog read of `cargo xtask minicat` (HV-INTRO-16, L7, D-017; R-M4-33, R-M4-44). It is
//! the minicat spike's raw-file boundary (`cd27ec1`) with the read rebuilt over the
//! introspectors' own packs: for each shipped line (`pg14`–`pg18`, `mysql80`, `mysql84`,
//! `mariadb106`, `mariadb1011`, `mariadb114`) it runs the system-mode sections
//! (`packs/<dialect>/40-system.hvq.sql`) through `minicat::run_system_sections` on a
//! `MetaConn` (PostgreSQL: database `postgres`, `search_path` pinned by the snapshot;
//! MySQL/MariaDB: `hv_catalog` with `mysql` as the default database), and writes one raw JSON
//! file per section, rows in server order, through `hacivat_testkit::artifact`:
//! `<HACIVAT_MINICAT_RAW>/<target>/{target,<schema>.<Section>,ListSettings,builtins}.json`.
//! Nothing here sorts or shapes: the build (`xtask minicat build`) owns determinism.
//!
//! **The generator-only built-in name probe** (R-M4-44, the single exception to R-M4-7,
//! recorded in ADR 0008 `m4-t23`): MySQL/MariaDB built-in function names cannot be read from
//! a catalog, so this file prepares `SELECT <name>()` for each name of the hand-curated
//! `sql-safety/data/mysql/manual-index.txt` (names only, `[A-Za-z0-9_]`; nothing runs, the
//! statement is deallocated) on a separate `hv_catalog` connection whose default database is
//! the empty `hv_catalog_probe` (R-C1-1: a missing function answers 1305, never 1370), plus
//! MariaDB's `information_schema.SQL_FUNCTIONS` names. A sentinel name must come back absent
//! or the read fails. It lives only under `tests/`, never in the runtime crate, and never runs
//! on a `MetaConn`. Comments and help text are never read (GPL documentation).
//!
//! Active only with `HACIVAT_MINICAT=1` (set by `xtask minicat collect`), so the crate's other
//! Docker runs (`docker-pr`, `docker-full`) never read the catalogs.

#![cfg(feature = "session")]

mod common;

use std::path::{Component, Path, PathBuf};

use common::pg::{Pg, admin_on};
use hacivat_driver::{ExecOpts, Protocol, ServerInfo};
use hacivat_introspect::minicat::{run_system_sections, system_schemas, system_sections};
use hacivat_introspect::pack::{DialectId, Mode, Pack, PackCtx, PackFlags, Params};
use hacivat_meta::Level;
use hacivat_proto::DbmsKind;
use hacivat_session::meta::{MetaConn, Rows};
use hacivat_testkit::{DockerTarget, artifact, docker_enabled, require_target};
use hacivat_values::{BindValue, LogicalType};
use serde_json::{Value, json};

/// The raw format `xtask minicat build` reads.
const RAW_FORMAT: &str = "hacivat-minicat-raw/2";
/// The shipped lines (R-M4-6), in `docker/m2a/targets.toml` order.
const TARGETS: [&str; 10] = [
    "pg14",
    "pg15",
    "pg16",
    "pg17",
    "pg18",
    "mysql80",
    "mysql84",
    "mariadb106",
    "mariadb1011",
    "mariadb114",
];
/// A name no server has: the probe must answer it absent.
const SENTINEL: &str = "hv_minicat_no_such_function";
/// "No such function" (`ER_SP_DOES_NOT_EXIST`, and `ER_FUNC_INEXISTENT_NAME_COLLISION`).
const ABSENT: [&str; 2] = ["1305", "1630"];
/// `ER_PROCACCESS_DENIED_ERROR`: the probe cannot tell (R-C1-1).
const DENIED: &str = "1370";

fn wanted() -> bool {
    docker_enabled() && std::env::var("HACIVAT_MINICAT").is_ok_and(|v| v == "1")
}

/// `HACIVAT_MINICAT_RAW`: a relative path under the artifacts directory (`minicat/raw-1`).
fn raw_name() -> PathBuf {
    let v = std::env::var("HACIVAT_MINICAT_RAW")
        .expect("HACIVAT_MINICAT_RAW names the raw directory (xtask minicat collect sets it)");
    let p = PathBuf::from(v);
    assert!(
        p.components().all(|c| matches!(c, Component::Normal(_))) && !p.as_os_str().is_empty(),
        "HACIVAT_MINICAT_RAW must be a relative path of plain names: {}",
        p.display()
    );
    p
}

/// `catalog` (the default) or `test`.
fn reader() -> String {
    let r = std::env::var("HACIVAT_MINICAT_READER").unwrap_or_else(|_| "catalog".into());
    assert!(
        r == "catalog" || r == "test",
        "HACIVAT_MINICAT_READER={r:?}: expected catalog or test"
    );
    r
}

fn text_bind(s: &str) -> BindValue {
    BindValue::Typed {
        hint: LogicalType::Text {
            max_chars: None,
            fixed: false,
        },
        text: s.into(),
    }
}

/// The exact server version of a line: `17.6` (PostgreSQL), `8.4.6`, `11.4.8`.
fn exact_version(id: &str, s: &ServerInfo) -> String {
    let v = s
        .version
        .as_ref()
        .unwrap_or_else(|| panic!("{id}: the server reported no version"));
    if s.kind == DbmsKind::Postgres || id.starts_with("pg") {
        format!("{}.{}", v.major, v.minor)
    } else {
        format!("{}.{}.{}", v.major, v.minor, v.patch)
    }
}

/// The system-mode rendering context of `schema` on `t` (version gates: none in
/// `40-system`; the MariaDB flag follows the flavor).
fn ctx(engine: DbmsKind, version: u32, schema: &str) -> PackCtx {
    let (dialect, flags) = match engine {
        DbmsKind::Postgres => (DialectId::Postgres, PackFlags::empty()),
        DbmsKind::MariaDb => (DialectId::MariaDb, PackFlags::MARIADB),
        _ => (DialectId::MySql, PackFlags::empty()),
    };
    PackCtx {
        dialect,
        version,
        flavor: engine,
        mode: Mode::System,
        level: Level::Structure,
        flags,
        params: Params {
            scalars: vec![("schema".into(), text_bind(schema))],
            lists: Vec::new(),
        },
    }
}

fn version_num(t: &DockerTarget, s: &ServerInfo) -> u32 {
    let v = s
        .version
        .as_ref()
        .map_or((0, 0, 0), |v| (v.major, v.minor, v.patch));
    if t.engine == DbmsKind::Postgres {
        v.0 * 10_000 + v.1
    } else {
        v.0 * 10_000 + v.1 * 100 + v.2
    }
}

fn rows_json(r: &Rows) -> Value {
    json!({
        "columns": r.columns.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
        "rows": r.rows.iter()
            .map(|row| row.iter().map(|v| v.as_deref().map(str::to_string)).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
    })
}

fn write(dir: &Path, file: &str, v: &Value) {
    let path = artifact(&dir.join(file).to_string_lossy());
    std::fs::create_dir_all(path.parent().expect("a parent"))
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    std::fs::write(&path, serde_json::to_vec(v).expect("raw JSON"))
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

/// Runs every system schema's sections on `c` and writes them under `dir`.
async fn read_sections(t: &DockerTarget, c: &mut MetaConn, version: u32, dir: &Path) {
    let pack = Pack::builtin(match t.engine {
        DbmsKind::Postgres => DialectId::Postgres,
        _ => DialectId::MySql,
    })
    .expect("the built-in pack");
    let mut settings_written = false;
    for schema in system_schemas(t.engine) {
        let got = run_system_sections(c, &pack, &ctx(t.engine, version, schema))
            .await
            .unwrap_or_else(|e| panic!("{} {schema}: {e:?}", t.id));
        for (section, rows) in got {
            if &*section == "ListSettings" {
                // One server-wide list: written once.
                if !settings_written {
                    write(dir, "ListSettings.json", &rows_json(&rows));
                    settings_written = true;
                }
                continue;
            }
            write(dir, &format!("{schema}.{section}.json"), &rows_json(&rows));
        }
    }
    assert!(settings_written, "{}: no ListSettings", t.id);
}

// -----------------------------------------------------------------------------------------------
// The generator-only built-in name probe (MySQL/MariaDB)

/// The names of `sql-safety/data/mysql/manual-index.txt`, in order (comments and blanks
/// out); each is `[A-Za-z0-9_]+`, so it can stand in the probe text.
fn manual_index() -> Vec<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../sql/sql-safety/data/mysql/manual-index.txt");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .map(|l| l.split('#').next().unwrap_or_default().trim())
        .filter(|l| !l.is_empty())
        .inspect(|l| {
            assert!(
                l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
                "manual-index: {l:?}"
            );
        })
        .map(str::to_string)
        .collect()
}

/// The probe's connection: `hv_catalog` on `hv_catalog_probe` (or, for `--reader test`,
/// `hv_test` on its own database). Never a `MetaConn`.
fn probe_target(t: &DockerTarget, reader: &str) -> DockerTarget {
    match reader {
        "catalog" => t.as_catalog().expect("a MySQL-family target"),
        _ => t.clone(),
    }
}

/// Whether `name` is a function (`PREPARE` only; nothing runs), with the server's answer.
async fn probe(live: &mut hacivat_session::pipeline::LiveConnection, name: &str) -> (bool, String) {
    // Only `[A-Za-z0-9_]` ever stands in the probe text (the one R-M4-7 exception).
    assert!(
        !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
        "probe name {name:?}"
    );
    match live
        .raw()
        .batch(&format!("PREPARE hv_minicat FROM 'SELECT {name}()'"))
        .await
    {
        Ok(()) => {
            live.raw()
                .batch("DEALLOCATE PREPARE hv_minicat")
                .await
                .unwrap_or_else(|e| panic!("DEALLOCATE: {:?}", e.kind));
            (true, "ok".into())
        }
        Err(e) => {
            let code = e
                .server
                .as_ref()
                .and_then(|m| m.vendor_code)
                .map(|c| c.to_string())
                .unwrap_or_else(|| panic!("{name}: not a server answer: {:?}", e.kind));
            assert_ne!(code, DENIED, "{name}: the probe cannot tell (1370, R-C1-1)");
            (!ABSENT.contains(&code.as_str()), code)
        }
    }
}

/// The built-in names of `t`: `(name, source)` rows.
async fn builtins(t: &DockerTarget, reader: &str) -> Value {
    let mut admin = admin_on(&probe_target(t, reader), None).await;
    let live = admin.live();
    let (present, answer) = probe(live, SENTINEL).await;
    assert!(
        !present,
        "{}: the probe does not tell functions apart (the sentinel answers {answer})",
        t.id
    );
    let mut rows: Vec<Vec<Option<String>>> = Vec::new();
    let mut codes: std::collections::BTreeMap<String, u32> = Default::default();
    for name in manual_index() {
        let (found, code) = probe(live, &name).await;
        *codes.entry(code).or_default() += 1;
        if found {
            rows.push(vec![Some(name), Some("probe".into())]);
        }
    }
    if t.engine == DbmsKind::MariaDb {
        let answer = live
            .raw()
            .execute(
                "SELECT `FUNCTION` AS `name` FROM information_schema.SQL_FUNCTIONS",
                ExecOpts {
                    max_rows: 10_000,
                    protocol: Protocol::Simple,
                },
            )
            .await;
        if let Ok(answer) = answer {
            for r in answer.rows {
                if let Some(Some(v)) = r.into_iter().next() {
                    rows.push(vec![Some(v.to_string()), Some("sql_functions".into())]);
                }
            }
        }
    }
    let summary: Vec<String> = codes.iter().map(|(c, n)| format!("{c}:{n}")).collect();
    println!(
        "HV-FACT minicat-probe target={} rows={} answers={}",
        t.id,
        rows.len(),
        summary.join(",")
    );
    admin.close().await;
    json!({ "columns": ["name", "source"], "rows": rows })
}

// -----------------------------------------------------------------------------------------------
// One target

async fn read_target(id: &str, raw: &Path, reader: &str) {
    let t = require_target(id)
        .unwrap_or_else(|| panic!("{id} is not in the active set: the read needs --set full"));
    let dir = raw.join(id);
    let (dbms, version) = match t.engine {
        DbmsKind::Postgres => {
            let pg = Pg::connect(&t).await;
            let mut c = pg.meta_on(Some("postgres")).await;
            let (v, n) = (exact_version(id, c.server()), version_num(&t, c.server()));
            read_sections(&t, &mut c, n, &dir).await;
            drop(c);
            pg.shutdown().await;
            ("postgres", v)
        }
        DbmsKind::MySql | DbmsKind::MariaDb => {
            let as_reader = match reader {
                // `hv_catalog` reads the system schemas; `mysql` is its default database.
                "catalog" => DockerTarget {
                    database: "mysql".into(),
                    ..t.as_catalog().expect("a MySQL-family target")
                },
                _ => t.clone(),
            };
            let conn = Pg::connect(&as_reader).await;
            let mut c = conn.meta().await;
            let (v, n) = (exact_version(id, c.server()), version_num(&t, c.server()));
            read_sections(&t, &mut c, n, &dir).await;
            drop(c);
            conn.shutdown().await;
            write(&dir, "builtins.json", &builtins(&t, reader).await);
            let dbms = if t.engine == DbmsKind::MariaDb {
                "mariadb"
            } else {
                "mysql"
            };
            (dbms, v)
        }
        other => panic!("{id}: no snapshot is made for {other:?} (R-M4-6)"),
    };
    assert!(
        version
            .strip_prefix(&t.major)
            .is_some_and(|rest| rest.starts_with('.')),
        "{id}: version {version} is not of line {}",
        t.major
    );
    write(
        &dir,
        "target.json",
        &json!({
            "format": RAW_FORMAT,
            "id": id,
            "dbms": dbms,
            "line": t.major,
            "server_version": version,
            "reader": reader,
        }),
    );
    println!("HV-FACT minicat-read target={id} version={version}");
}

// -----------------------------------------------------------------------------------------------
// Pure tests (every `cargo test` run)

#[test]
fn the_manual_index_is_readable() {
    let names = manual_index();
    assert!(names.len() > 100, "{} names", names.len());
    assert!(!names.iter().any(|n| n.eq_ignore_ascii_case(SENTINEL)));
}

#[test]
fn the_system_sections_are_the_40_system_files() {
    let pg = Pack::builtin(DialectId::Postgres).unwrap();
    let names: Vec<String> = system_sections(&pg).iter().map(|s| s.to_string()).collect();
    assert_eq!(
        names,
        [
            "SystemRelations",
            "SystemColumns",
            "SystemRoutines",
            "SystemOperators",
            "SystemTypes",
            "ListSettings"
        ]
    );
    let my = Pack::builtin(DialectId::MySql).unwrap();
    let names: Vec<String> = system_sections(&my).iter().map(|s| s.to_string()).collect();
    assert_eq!(
        names,
        [
            "SystemRelations",
            "SystemColumns",
            "SystemRoutines",
            "SystemParameters",
            "ListSettings"
        ]
    );
    assert!(system_sections(&Pack::builtin(DialectId::Sqlite).unwrap()).is_empty());
    assert_eq!(
        system_schemas(DbmsKind::Postgres),
        ["information_schema", "pg_catalog"]
    );
    assert_eq!(
        system_schemas(DbmsKind::MariaDb),
        ["information_schema", "mysql", "performance_schema", "sys"]
    );
    assert!(system_schemas(DbmsKind::Sqlite).is_empty());
}

#[tokio::test]
async fn run_system_sections_refuses_other_modes_schemas_and_reads_nothing_offline() {
    let pg = Pack::builtin(DialectId::Postgres).unwrap();
    let t = |engine, schema: &str| ctx(engine, 0, schema);
    let mut c = MetaConn::offline(DbmsKind::Postgres);
    // Another mode.
    let mut full = t(DbmsKind::Postgres, "pg_catalog");
    full.mode = Mode::Full;
    let e = run_system_sections(&mut c, &pg, &full).await.unwrap_err();
    assert!(
        matches!(&e, hacivat_introspect::IntroError::Protocol(n) if &**n == "minicat/mode"),
        "{e:?}"
    );
    // A database that is not a system schema (carry: never `hv_catalog_probe`, never a
    // fixture database), and no schema at all.
    for schema in ["hv_catalog_probe", "public", "hvm4_x", ""] {
        let e = run_system_sections(&mut c, &pg, &t(DbmsKind::Postgres, schema))
            .await
            .unwrap_err();
        assert!(
            matches!(&e, hacivat_introspect::IntroError::Protocol(n) if &**n == "minicat/schema"),
            "{schema}: {e:?}"
        );
    }
    let mut none = t(DbmsKind::Postgres, "pg_catalog");
    none.params = Params::default();
    assert!(run_system_sections(&mut c, &pg, &none).await.is_err());
    // A system schema on a connection with no server: the connection's own refusal.
    let e = run_system_sections(&mut c, &pg, &t(DbmsKind::Postgres, "pg_catalog"))
        .await
        .unwrap_err();
    assert!(
        matches!(
            e,
            hacivat_introspect::IntroError::Meta(hacivat_session::meta::MetaError::Unavailable)
        ),
        "{e:?}"
    );
    // A pack of another dialect than the context's.
    let my = Pack::builtin(DialectId::MySql).unwrap();
    assert!(
        run_system_sections(&mut c, &my, &t(DbmsKind::Postgres, "pg_catalog"))
            .await
            .is_err()
    );
}

// -----------------------------------------------------------------------------------------------
// Docker (only with HACIVAT_MINICAT=1, through `xtask minicat collect`)

/// The read: every shipped line into `<HACIVAT_MINICAT_RAW>`. Two runs build byte-identical
/// snapshots: `xtask minicat collect --twice` (and `verify --twice`) runs this test twice from
/// scratch and compares the built files per target; the `minicat-verify` CI job runs it again
/// on a fresh stack (R-M4-17).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn snapshot_bytes_identical_across_two_runs() {
    if !wanted() {
        eprintln!(
            "minicat_gen: set HACIVAT_MINICAT=1 (xtask minicat collect) to read the catalogs"
        );
        return;
    }
    let raw = raw_name();
    let reader = reader();
    for id in TARGETS {
        read_target(id, &raw, &reader).await;
    }
}

/// The probe tells a missing function from a built-in on every MySQL and MariaDB line: the
/// sentinel is absent, `NOW` is present, and `ABS()` with no argument is present too (1582).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn builtin_name_probe_sentinel_is_absent() {
    if !wanted() {
        return;
    }
    let reader = reader();
    for id in TARGETS.iter().filter(|id| !id.starts_with("pg")) {
        let t = require_target(id).unwrap_or_else(|| panic!("{id}: needs --set full"));
        let mut admin = admin_on(&probe_target(&t, &reader), None).await;
        let live = admin.live();
        let (present, code) = probe(live, SENTINEL).await;
        assert!(!present, "{id}: the sentinel answers {code}");
        assert!(ABSENT.contains(&code.as_str()), "{id}: {code}");
        assert_eq!(probe(live, "NOW").await, (true, "ok".into()), "{id}");
        assert_eq!(probe(live, "ABS").await, (true, "1582".into()), "{id}");
        admin.close().await;
    }
}
