//! M4 pre-work spike (HV-INTRO-16, L7, D-017): the catalog read of `xtask minicat`. It reads the
//! *system* schemas of every server line of the active set, and the built-ins of the bundled
//! SQLite, and writes one raw JSON file per line; `xtask minicat build` turns them into the
//! sorted, zstd-compressed snapshots of `assets/system-catalogs/`. Nothing here sorts or shapes: the
//! rows are written as the server answered them (the build owns determinism).
//!
//! - PostgreSQL 14–18: `pg_class`/`pg_attribute`, `pg_proc`, `pg_operator`, `pg_type` and
//!   `pg_settings`, restricted to `pg_catalog` and `information_schema` objects with
//!   OID < 16384 (made by initdb: never an extension's or a test's object), on database
//!   `postgres` with `search_path = pg_catalog`, so type names print the same everywhere.
//! - MySQL 8.0/8.4, MariaDB 10.6/10.11/11.4: `information_schema` `TABLES`, `COLUMNS`,
//!   `ROUTINES`, `PARAMETERS` for `information_schema`, `mysql`, `performance_schema` and `sys`;
//!   the system variable names; the built-in function names (a `PREPARE`-only probe of the
//!   hand-curated `sql-safety/data/mysql/manual-index.txt`, plus MariaDB's
//!   `information_schema.SQL_FUNCTIONS` when present). Never `mysql.help_*`, never a
//!   routine or variable comment (GPL documentation text).
//! - SQLite: `pragma_function_list` (built-ins), `pragma_pragma_list`, `pragma_collation_list`,
//!   `pragma_module_list` and the columns of `sqlite_schema`, on an in-memory database of the
//!   bundled SQLite.
//!
//! The MySQL-family reader is `hv_catalog` (read-only grants on the system schemas, made by
//! `docker/m2a/init/mysql.sql`) unless `HACIVAT_MINICAT_READER=test`, which reads as `hv_test`:
//! that account sees `information_schema` only, so its snapshots are partial (the build records
//! the coverage). PostgreSQL's `hv_test` is the bootstrap superuser and sees everything.
//!
//! Run through `cargo xtask minicat collect` (it sets `HACIVAT_MINICAT=1` and
//! `HACIVAT_MINICAT_RAW=<dir>`). Without `HACIVAT_MINICAT=1` it does nothing, so the crate's
//! other Docker runs never read the catalogs.

mod common;

use std::path::{Path, PathBuf};

use common::matrix::{Line, Side, Table, docker_enabled, line_of, must_exist, server_lines};
use hacivat_proto::DbmsKind;
use serde_json::{Value as Json, json};

/// The raw file format `xtask minicat build` reads.
const RAW_FORMAT: &str = "hacivat-minicat-raw/1";
/// The most rows one catalog statement may answer (PostgreSQL 18 has about 3,500 columns of
/// system relations and 3,300 routines).
const MAX_ROWS: usize = 200_000;

fn wanted() -> bool {
    docker_enabled() && std::env::var("HACIVAT_MINICAT").is_ok_and(|v| v == "1")
}

fn raw_dir() -> PathBuf {
    let dir = std::env::var("HACIVAT_MINICAT_RAW")
        .expect("HACIVAT_MINICAT_RAW names the raw output directory");
    PathBuf::from(dir)
}

/// `catalog` (the default) or `test`.
fn reader() -> String {
    std::env::var("HACIVAT_MINICAT_READER").unwrap_or_else(|_| "catalog".into())
}

// -----------------------------------------------------------------------------------------------
// PostgreSQL

const PG_SCHEMAS: &str = "('pg_catalog', 'information_schema')";

fn pg_queries() -> Vec<(&'static str, String)> {
    vec![
        (
            "relations",
            format!(
                "SELECT n.nspname::text AS schema, c.relname::text AS name, \
                 c.relkind::text AS kind \
                 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n \
                 ON n.oid = c.relnamespace \
                 WHERE c.oid < 16384 AND n.nspname IN {PG_SCHEMAS} \
                 AND c.relkind IN ('r', 'v', 'm', 'p', 'f')"
            ),
        ),
        (
            "columns",
            format!(
                "SELECT n.nspname::text AS schema, c.relname::text AS relation, \
                 a.attnum::text AS position, a.attname::text AS name, \
                 pg_catalog.format_type(a.atttypid, a.atttypmod) AS type, \
                 (NOT a.attnotnull)::text AS nullable \
                 FROM pg_catalog.pg_attribute a \
                 JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
                 JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                 WHERE c.oid < 16384 AND n.nspname IN {PG_SCHEMAS} \
                 AND c.relkind IN ('r', 'v', 'm', 'p', 'f') \
                 AND a.attnum > 0 AND NOT a.attisdropped"
            ),
        ),
        (
            "routines",
            format!(
                "SELECT n.nspname::text AS schema, p.proname::text AS name, \
                 p.prokind::text AS kind, \
                 pg_catalog.pg_get_function_arguments(p.oid) AS args, \
                 pg_catalog.pg_get_function_identity_arguments(p.oid) AS identity, \
                 pg_catalog.pg_get_function_result(p.oid) AS result, \
                 p.provolatile::text AS volatility, p.proretset::text AS set_returning, \
                 pg_catalog.obj_description(p.oid, 'pg_proc') AS description \
                 FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n \
                 ON n.oid = p.pronamespace \
                 WHERE p.oid < 16384 AND n.nspname IN {PG_SCHEMAS}"
            ),
        ),
        (
            "operators",
            format!(
                "SELECT n.nspname::text AS schema, o.oprname::text AS name, \
                 o.oprkind::text AS kind, \
                 CASE WHEN o.oprleft = 0 THEN NULL \
                   ELSE pg_catalog.format_type(o.oprleft, NULL) END AS \"left\", \
                 CASE WHEN o.oprright = 0 THEN NULL \
                   ELSE pg_catalog.format_type(o.oprright, NULL) END AS \"right\", \
                 pg_catalog.format_type(o.oprresult, NULL) AS result, \
                 o.oprcode::regproc::text AS function, \
                 pg_catalog.obj_description(o.oid, 'pg_operator') AS description \
                 FROM pg_catalog.pg_operator o JOIN pg_catalog.pg_namespace n \
                 ON n.oid = o.oprnamespace \
                 WHERE o.oid < 16384 AND n.nspname IN {PG_SCHEMAS}"
            ),
        ),
        (
            // The row types of system relations (and their arrays) mirror `relations`: left out.
            "types",
            format!(
                "SELECT n.nspname::text AS schema, t.typname::text AS name, \
                 pg_catalog.format_type(t.oid, NULL) AS sql_name, \
                 t.typtype::text AS kind, t.typcategory::text AS category, \
                 CASE WHEN t.typcategory = 'A' \
                   THEN pg_catalog.format_type(t.typelem, NULL) END AS element, \
                 pg_catalog.obj_description(t.oid, 'pg_type') AS description \
                 FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n \
                 ON n.oid = t.typnamespace \
                 WHERE t.oid < 16384 AND n.nspname IN {PG_SCHEMAS} AND t.typrelid = 0 \
                 AND NOT (t.typcategory = 'A' AND EXISTS (SELECT 1 FROM pg_catalog.pg_type e \
                   WHERE e.oid = t.typelem AND e.typrelid <> 0))"
            ),
        ),
        (
            // Dotted names are extension placeholders (`plpgsql.*` appears once plpgsql loads):
            // not part of the server line.
            "variables",
            "SELECT name::text AS name, vartype::text AS type, context::text AS context \
             FROM pg_catalog.pg_settings WHERE name NOT LIKE '%.%'"
                .to_string(),
        ),
    ]
}

async fn read_pg(line: &Line) -> Json {
    let mut side = Side::open(line, Some("postgres")).await;
    side.batch("SET search_path = pg_catalog").await;
    let version = scalar(&mut side, line, "SHOW server_version").await;
    let mut sections = serde_json::Map::new();
    for (name, sql) in pg_queries() {
        let table = side
            .try_rows(&sql, MAX_ROWS)
            .await
            .unwrap_or_else(|e| panic!("{} {name}: {e}", line.id));
        sections.insert(name.to_string(), table_json(table));
    }
    side.close().await;
    raw("postgres", line, &version, "test", sections)
}

// -----------------------------------------------------------------------------------------------
// MySQL and MariaDB

const MY_SCHEMAS: &str = "('information_schema', 'mysql', 'performance_schema', 'sys')";

fn my_queries() -> Vec<(&'static str, String)> {
    vec![
        (
            "relations",
            format!(
                "SELECT TABLE_SCHEMA AS `schema`, TABLE_NAME AS `name`, TABLE_TYPE AS `kind` \
                 FROM information_schema.TABLES WHERE TABLE_SCHEMA IN {MY_SCHEMAS}"
            ),
        ),
        (
            "columns",
            format!(
                "SELECT TABLE_SCHEMA AS `schema`, TABLE_NAME AS `relation`, \
                 ORDINAL_POSITION AS `position`, COLUMN_NAME AS `name`, COLUMN_TYPE AS `type`, \
                 IS_NULLABLE AS `nullable` \
                 FROM information_schema.COLUMNS WHERE TABLE_SCHEMA IN {MY_SCHEMAS}"
            ),
        ),
        (
            "routines",
            format!(
                "SELECT ROUTINE_SCHEMA AS `schema`, ROUTINE_NAME AS `name`, \
                 ROUTINE_TYPE AS `kind`, DTD_IDENTIFIER AS `result`, \
                 IS_DETERMINISTIC AS `deterministic` \
                 FROM information_schema.ROUTINES WHERE ROUTINE_SCHEMA IN {MY_SCHEMAS}"
            ),
        ),
        (
            "parameters",
            format!(
                "SELECT SPECIFIC_SCHEMA AS `schema`, SPECIFIC_NAME AS `name`, \
                 ROUTINE_TYPE AS `kind`, ORDINAL_POSITION AS `position`, PARAMETER_MODE AS `mode`, \
                 PARAMETER_NAME AS `param`, DTD_IDENTIFIER AS `type` \
                 FROM information_schema.PARAMETERS \
                 WHERE SPECIFIC_SCHEMA IN {MY_SCHEMAS} AND ORDINAL_POSITION > 0"
            ),
        ),
    ]
}

/// The names of `sql-safety/data/mysql/manual-index.txt`, in order (comments and blanks out).
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

/// The built-in function names: each manual-index name that `PREPARE` does not answer
/// 1305/1630 (no such function; nothing runs), plus, on MariaDB, the names of
/// `information_schema.SQL_FUNCTIONS` (the functions the parser treats as keywords; not every
/// native function) when the server has it.
async fn my_builtins(side: &mut Side, line: &Line) -> Table {
    // A probe that finds everything present is worthless: a sentinel must come back absent.
    let (present, answer) = probe(side, "hv_minicat_no_such_function").await;
    assert!(
        !present,
        "{}: the probe does not tell functions apart (a missing one answers {answer})",
        line.id
    );
    let mut rows = Vec::new();
    for name in manual_index() {
        if probe(side, &name).await.0 {
            rows.push(vec![Some(name), Some("probe".into())]);
        }
    }
    if line.engine == DbmsKind::MariaDb
        && let Ok((_, found)) = side
            .try_rows(
                "SELECT `FUNCTION` AS `name` FROM information_schema.SQL_FUNCTIONS",
                MAX_ROWS,
            )
            .await
    {
        rows.extend(found.into_iter().map(|mut r| {
            r.push(Some("sql_functions".into()));
            r
        }));
    }
    (vec!["name".into(), "source".into()], rows)
}

/// Whether `name` is a function (`PREPARE` only; nothing runs), with the server's answer: 1305
/// and 1630 mean no such function; anything else (1582 wrong argument count, 1064 special
/// syntax) means present.
async fn probe(side: &mut Side, name: &str) -> (bool, String) {
    match side
        .try_batch(&format!("PREPARE hv_minicat FROM 'SELECT {name}()'"))
        .await
    {
        Ok(()) => {
            side.batch("DEALLOCATE PREPARE hv_minicat").await;
            (true, "ok".into())
        }
        Err(code) => (code != "1305" && code != "1630", code),
    }
}

async fn read_my(line: &Line, reader: &str) -> Json {
    let target = line.target().clone();
    // The default database matters to the probe: an unknown function must answer 1305 ("no
    // such function in <db>"), which needs a privilege on <db> (in `information_schema` it
    // answers 1044). `hv_catalog` may read `mysql`; `hv_test` owns `hv_test`.
    let (as_reader, db) = match reader {
        // The same throwaway password as every docker/m2a account.
        "catalog" => (
            line_of(hacivat_testkit::DockerTarget {
                user: "hv_catalog".into(),
                ..target
            }),
            Some("mysql"),
        ),
        "test" => (line.clone(), None),
        other => panic!("HACIVAT_MINICAT_READER={other:?}: expected catalog or test"),
    };
    let mut side = Side::open(&as_reader, db).await;
    let version = scalar(&mut side, line, "SELECT VERSION()").await;
    let mut sections = serde_json::Map::new();
    for (name, sql) in my_queries() {
        let table = side
            .try_rows(&sql, MAX_ROWS)
            .await
            .unwrap_or_else(|e| panic!("{} {name}: {e}", line.id));
        sections.insert(name.to_string(), table_json(table));
    }
    let variables = match line.engine {
        DbmsKind::MariaDb => {
            "SELECT VARIABLE_NAME AS `name`, VARIABLE_SCOPE AS `scope`, VARIABLE_TYPE AS `type` \
             FROM information_schema.SYSTEM_VARIABLES"
        }
        _ => "SELECT VARIABLE_NAME AS `name` FROM performance_schema.global_variables",
    };
    let table = side
        .try_rows(variables, MAX_ROWS)
        .await
        .unwrap_or_else(|e| panic!("{} variables: {e}", line.id));
    sections.insert("variables".into(), table_json(table));
    let builtins = my_builtins(&mut side, line).await;
    sections.insert("builtins".into(), table_json(builtins));
    side.close().await;
    let engine = if line.engine == DbmsKind::MariaDb {
        "mariadb"
    } else {
        "mysql"
    };
    raw(engine, line, &version, reader, sections)
}

// -----------------------------------------------------------------------------------------------
// SQLite

fn sqlite_table(conn: &rusqlite::Connection, sql: &str) -> Table {
    use rusqlite::types::Value;
    let mut stmt = conn
        .prepare(sql)
        .unwrap_or_else(|e| panic!("sqlite `{sql}`: {e}"));
    let columns: Vec<String> = stmt.column_names().map(str::to_string).collect();
    let n = columns.len();
    let rows = stmt
        .query_map([], |r| {
            (0..n)
                .map(|i| {
                    Ok(match r.get::<_, Value>(i)? {
                        Value::Null => None,
                        Value::Integer(v) => Some(v.to_string()),
                        Value::Real(v) => Some(v.to_string()),
                        Value::Text(v) => Some(v),
                        Value::Blob(_) => panic!("sqlite `{sql}`: a blob"),
                    })
                })
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap_or_else(|e| panic!("sqlite `{sql}`: {e}"))
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap_or_else(|e| panic!("sqlite `{sql}`: {e}"));
    (columns, rows)
}

fn read_sqlite() -> Json {
    let conn = rusqlite::Connection::open_in_memory().expect("an in-memory SQLite");
    let version: String = conn
        .query_row("SELECT sqlite_version()", [], |r| r.get(0))
        .expect("sqlite_version()");
    let queries = [
        (
            // SQLITE_DETERMINISTIC is 0x800. A function registered for several text encodings
            // has one row per encoding: DISTINCT folds them.
            "functions",
            "SELECT DISTINCT name, type AS kind, narg AS arity, \
             (flags & 2048) <> 0 AS deterministic \
             FROM pragma_function_list WHERE builtin = 1",
        ),
        ("pragmas", "SELECT name FROM pragma_pragma_list"),
        ("collations", "SELECT name FROM pragma_collation_list"),
        ("modules", "SELECT name FROM pragma_module_list"),
        (
            "columns",
            "SELECT 'main' AS schema, 'sqlite_schema' AS relation, cid + 1 AS position, name, \
             type, CASE WHEN \"notnull\" = 0 THEN 'true' ELSE 'false' END AS nullable \
             FROM pragma_table_info('sqlite_schema')",
        ),
    ];
    let mut sections = serde_json::Map::new();
    for (name, sql) in queries {
        sections.insert(name.to_string(), table_json(sqlite_table(&conn, sql)));
    }
    sections.insert(
        "relations".into(),
        table_json((
            vec!["schema".into(), "name".into(), "kind".into()],
            vec![vec![
                Some("main".into()),
                Some("sqlite_schema".into()),
                Some("table".into()),
            ]],
        )),
    );
    let major = version.split('.').next().unwrap_or_default().to_string();
    json!({
        "format": RAW_FORMAT,
        "id": "sqlite",
        "engine": "sqlite",
        "line": major,
        "server_version": version,
        "reader": "bundled",
        "sections": Json::Object(sections),
    })
}

// -----------------------------------------------------------------------------------------------
// Shared

async fn scalar(side: &mut Side, line: &Line, sql: &str) -> String {
    let (_, rows) = side
        .try_rows(sql, 1)
        .await
        .unwrap_or_else(|e| panic!("{} `{sql}`: {e}", line.id));
    rows.into_iter()
        .next()
        .and_then(|r| r.into_iter().next().flatten())
        .unwrap_or_else(|| panic!("{} `{sql}`: no value", line.id))
}

fn table_json((columns, rows): Table) -> Json {
    json!({ "columns": columns, "rows": rows })
}

fn raw(
    engine: &str,
    line: &Line,
    version: &str,
    reader: &str,
    sections: serde_json::Map<String, Json>,
) -> Json {
    json!({
        "format": RAW_FORMAT,
        "id": line.id,
        "engine": engine,
        "line": line.target().major,
        "server_version": version,
        "reader": reader,
        "sections": Json::Object(sections),
    })
}

fn write_raw(dir: &Path, doc: &Json) {
    let id = doc["id"].as_str().expect("an id");
    let path = dir.join(format!("{id}.json"));
    let text = serde_json::to_vec(doc).expect("raw JSON");
    std::fs::write(&path, text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    eprintln!("minicat_read: wrote {}", path.display());
}

#[test]
fn the_manual_index_is_readable() {
    let names = manual_index();
    assert!(names.len() > 100, "{} names", names.len());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn minicat_read() {
    if !wanted() {
        eprintln!("minicat_read: set HACIVAT_MINICAT=1 to read the system catalogs");
        return;
    }
    assert!(
        must_exist(),
        "the catalog read needs every line: run it with --set full"
    );
    let dir = raw_dir();
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let reader = reader();
    for line in server_lines() {
        let doc = match line.engine {
            DbmsKind::Postgres => read_pg(&line).await,
            DbmsKind::MySql | DbmsKind::MariaDb => read_my(&line, &reader).await,
            other => panic!("{}: engine {other:?} has no catalog read", line.id),
        };
        write_raw(&dir, &doc);
    }
    write_raw(&dir, &read_sqlite());
}
