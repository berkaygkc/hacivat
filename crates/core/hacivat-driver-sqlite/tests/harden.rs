//! The read-only open of a SQLite profile (Task 11, B21): `SQLITE_OPEN_READ_ONLY`, defensive
//! mode, attach off, `LIMIT_ATTACHED` 0, `query_only` and the default-deny authorizer, through
//! `SqliteDriver::connect` with `DriverConnect.read_only`.

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

mod common;

use common::*;
use hacivat_driver::RawErrorKind;

/// `SQLITE_AUTH`: the authorizer refused the statement while it was prepared.
const SQLITE_AUTH: i64 = 23;

/// Asserts that the authorizer refused `sql` while it was prepared: `SQLITE_AUTH`, or for a
/// function SQLite's own "not authorized to use function" (its resolver reports a denied
/// function as `SQLITE_ERROR`). The disabled extension loader says only "not authorized",
/// and only once the statement runs.
async fn authorizer_denies(c: &mut Conn, sql: &str) {
    let server = err(c, sql).await.server.expect("an engine error");
    let function =
        server.vendor_code == Some(1) && server.text.starts_with("not authorized to use function");
    assert!(
        server.vendor_code == Some(SQLITE_AUTH) || function,
        "{sql}: {server:?}"
    );
}

/// A database in `dir` with table `t` (one row), index `i`, view `v` and trigger `tr`.
fn rich_db_in(dir: &tempfile::TempDir) -> Db {
    let db = db_in(dir);
    let c = rusqlite::Connection::open(&db.path).unwrap();
    c.execute_batch(
        "CREATE INDEX i ON t(a); CREATE VIEW v AS SELECT a FROM t;
         CREATE TRIGGER tr AFTER INSERT ON t BEGIN SELECT 1; END;",
    )
    .unwrap();
    drop(c);
    db
}

/// The files in `dir` other than the database file itself (side files such as `-wal`, `-shm`
/// and `-journal` are listed: review I3).
fn stray_files(dir: &tempfile::TempDir) -> Vec<String> {
    std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != "db.sqlite")
        .collect()
}

#[tokio::test]
async fn read_only_open_applies_hardening() {
    let mut c = ro(db_with_table()).await;
    // Allowed by Q16's pragma list.
    assert_eq!(text(&mut c, "PRAGMA query_only").await, "1");
    // M2a's probe still reads it.
    assert!(!text(&mut c, "PRAGMA journal_mode").await.is_empty());
    assert_eq!(text(&mut c, "PRAGMA trusted_schema").await, "0");
    assert!(!text(&mut c, "PRAGMA foreign_keys").await.is_empty());
}

#[tokio::test]
async fn attach_and_vacuum_into_leave_no_file() {
    let dir = tempdir();
    let mut c = ro(db_in(&dir)).await;
    let new = dir.path().join("new.db");
    let copy = dir.path().join("copy.db");
    for sql in [
        format!("ATTACH '{}' AS n", new.display()),
        format!("VACUUM INTO '{}'", copy.display()),
    ] {
        assert!(c.execute(&sql, simple()).await.is_err(), "{sql}");
    }
    assert!(!new.exists() && !copy.exists());
}

#[tokio::test]
async fn ro_denials() {
    let mut c = ro(db_with_table()).await;
    for sql in [
        "PRAGMA query_only = 0",
        "INSERT INTO t VALUES (1)",
        "CREATE TEMP TABLE x(a)",
        "SELECT load_extension('x')",
        "PRAGMA writable_schema = 1",
        "BEGIN",
    ] {
        assert!(c.execute(sql, simple()).await.is_err(), "{sql}");
    }
    assert_eq!(text(&mut c, "SELECT count(*) FROM t").await, "1");
}

#[tokio::test]
async fn read_write_profile_has_no_authorizer() {
    let mut c = rw(db_with_table()).await;
    assert!(
        c.execute("INSERT INTO t VALUES (2)", simple())
            .await
            .is_ok()
    );
    // Nothing the read-only open sets is set here.
    assert_eq!(text(&mut c, "PRAGMA query_only").await, "0");
    c.execute("CREATE TEMP TABLE x(a)", simple()).await.unwrap();
    c.execute("PRAGMA user_version = 7", simple())
        .await
        .unwrap();
    assert_eq!(text(&mut c, "PRAGMA user_version").await, "7");
}

/// Every write and control action code that SQLite asks the authorizer about, each refused
/// with `SQLITE_AUTH` (so the authorizer refused it, not the read-only file, which answers
/// `SQLITE_READONLY`): codes 1–19 and 22–32. The data is untouched afterwards.
#[tokio::test]
async fn every_write_action_is_refused_by_the_authorizer() {
    let dir = tempdir();
    let mut c = ro(rich_db_in(&dir)).await;
    for sql in [
        "CREATE INDEX j ON t(a)",                                       // 1
        "CREATE TABLE x(a)",                                            // 2
        "CREATE TEMP TABLE x(a)",                                       // 4
        "CREATE INDEX temp.j ON x(a)",                                  // 3 (needs a temp table)
        "CREATE TEMP TRIGGER tt AFTER INSERT ON t BEGIN SELECT 1; END", // 5
        "CREATE TEMP VIEW tv AS SELECT 1",                              // 6
        "CREATE TRIGGER t2 AFTER INSERT ON t BEGIN SELECT 1; END",      // 7
        "CREATE VIEW v2 AS SELECT 1",                                   // 8
        "DELETE FROM t",                                                // 9
        "DROP INDEX i",                                                 // 10
        "DROP TABLE t",                                                 // 11
        "DROP TRIGGER tr",                                              // 16
        "DROP VIEW v",                                                  // 17
        "INSERT INTO t VALUES (2)",                                     // 18
        "PRAGMA user_version = 5",                                      // 19
        "BEGIN",                                                        // 22
        "COMMIT",                                                       // 22
        "UPDATE t SET a = 2",                                           // 23
        "ATTACH ':memory:' AS m",                                       // 24
        "DETACH m",                                                     // 25
        "ALTER TABLE t ADD COLUMN b",                                   // 26
        "ALTER TABLE t RENAME TO u",                                    // 26
        "REINDEX",                                                      // 27
        "ANALYZE",                                                      // 28
        "CREATE VIRTUAL TABLE vt USING fts5(x)",                        // 29
        "SELECT load_extension('x')",                                   // 31
        "SAVEPOINT s",                                                  // 32
        "RELEASE s",                                                    // 32
    ] {
        // `CREATE INDEX temp.j` names a temp table that cannot exist: "no such table" may be
        // the engine's answer before the authorizer is asked. Every other one is the
        // authorizer.
        if sql.contains("temp.j") {
            assert!(c.execute(sql, simple()).await.is_err(), "{sql}");
        } else {
            authorizer_denies(&mut c, sql).await;
        }
    }
    assert_eq!(text(&mut c, "SELECT count(*) FROM t").await, "1");
    assert_eq!(
        text(&mut c, "SELECT count(*) FROM sqlite_schema").await,
        "4"
    );
}

/// What a read-only profile can still do: read tables and views, recursive CTEs, ordinary
/// functions, the Q16 read pragmas, and the six introspection pragmas with a name argument
/// in both forms (controller ruling).
#[tokio::test]
async fn reads_and_introspection_are_allowed() {
    let dir = tempdir();
    let mut c = ro(rich_db_in(&dir)).await;
    assert_eq!(text(&mut c, "SELECT a FROM t").await, "1");
    assert_eq!(text(&mut c, "SELECT a FROM v").await, "1");
    assert_eq!(
        text(
            &mut c,
            "WITH RECURSIVE r(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM r WHERE i < 5) \
             SELECT max(i) FROM r"
        )
        .await,
        "5"
    );
    assert_eq!(
        text(&mut c, "SELECT upper('a') || length('xy')").await,
        "A2"
    );
    for sql in [
        "PRAGMA table_info(t)",
        "PRAGMA main.table_xinfo(t)",
        "PRAGMA index_list(t)",
        "PRAGMA index_info(i)",
        "PRAGMA index_xinfo(i)",
        "PRAGMA foreign_key_list(t)",
        "SELECT name FROM pragma_table_info('t')",
        "SELECT name FROM pragma_index_info('i', 'main')",
        "PRAGMA database_list",
        "PRAGMA schema_version",
        "PRAGMA user_version",
        "PRAGMA page_count",
        "PRAGMA encoding",
    ] {
        c.execute(sql, simple())
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e:?}"));
    }
}

// Review probes (Task 11 "Review" line).

#[tokio::test]
async fn query_only_cannot_be_reset_by_any_spelling() {
    let mut c = ro(db_with_table()).await;
    for sql in [
        "PRAGMA main.query_only = 0",
        "PRAGMA temp.query_only = 0",
        "PRAGMA query_only(0)",
        "PRAGMA QUERY_ONLY = false",
        "PRAGMA main.QUERY_ONLY(off)",
    ] {
        assert_eq!(err_code(&mut c, sql).await, SQLITE_AUTH, "{sql}");
    }
    assert_eq!(text(&mut c, "PRAGMA query_only").await, "1");
}

#[tokio::test]
async fn attach_through_a_file_uri_creates_nothing() {
    let dir = tempdir();
    let mut c = ro(db_in(&dir)).await;
    let new = dir.path().join("uri.db");
    for sql in [
        format!("ATTACH 'file:{}?mode=rwc' AS n", new.display()),
        format!("ATTACH DATABASE 'file:{}?mode=rw' AS n", new.display()),
        format!("ATTACH '{}' || '' AS n", new.display()),
    ] {
        assert_eq!(err_code(&mut c, &sql).await, SQLITE_AUTH, "{sql}");
    }
    assert!(stray_files(&dir).is_empty(), "{:?}", stray_files(&dir));
}

#[tokio::test]
async fn vacuum_into_through_an_expression_or_a_parameter_writes_nothing() {
    let dir = tempdir();
    let mut c = ro(db_in(&dir)).await;
    let base = dir.path().join("vx").display().to_string();
    for sql in [
        format!("VACUUM INTO '{base}' || '.db'"),
        format!("VACUUM main INTO '{base}2.db'"),
        "VACUUM INTO ?".to_string(),
        "VACUUM".to_string(),
    ] {
        assert!(c.execute(&sql, simple()).await.is_err(), "{sql}");
    }
    assert!(stray_files(&dir).is_empty(), "{:?}", stray_files(&dir));
}

#[tokio::test]
async fn file_functions_are_refused_in_any_case() {
    let mut c = ro(db_with_table()).await;
    // The authorizer at prepare, not the disabled extension loader when the statement runs,
    // refuses these. (`readfile`, `writefile` and `edit` are not built in: SQLite answers "no
    // such function" before it asks; the adapter's unit tests cover their names in any case.)
    for sql in [
        "SELECT LOAD_EXTENSION('x')",
        "SELECT Load_Extension('x', 'y')",
        "SELECT fts3_tokenizer('simple')",
    ] {
        authorizer_denies(&mut c, sql).await;
    }
}

#[tokio::test]
async fn triggers_on_temp_views_cannot_be_made() {
    let mut c = ro(db_with_table()).await;
    for sql in [
        "CREATE TEMP VIEW tv AS SELECT a FROM t",
        "CREATE TEMPORARY VIEW tv AS SELECT a FROM t",
        "CREATE TEMP TRIGGER tg INSTEAD OF INSERT ON tv BEGIN SELECT 1; END",
        "CREATE TRIGGER temp.tg INSTEAD OF INSERT ON tv BEGIN SELECT 1; END",
    ] {
        assert!(c.execute(sql, simple()).await.is_err(), "{sql}");
    }
}

#[tokio::test]
async fn a_statement_that_passes_the_authorizer_but_writes_is_refused_by_stmt_readonly() {
    let mut c = ro(db_with_table()).await;
    // `VACUUM` asks the authorizer nothing; `sqlite3_stmt_readonly` is false for it.
    let e = err(&mut c, "VACUUM").await;
    assert_eq!(e.kind, RawErrorKind::Db);
    let server = e.server.unwrap();
    assert_eq!(server.text, "readonly-required");
    assert_eq!(server.vendor_code, Some(SQLITE_AUTH));
}

/// The plain `PRAGMA journal_mode` query is the one statement `sqlite3_stmt_readonly` calls a
/// write that a read-only profile may run (Q16); every form that sets the mode is refused.
#[tokio::test]
async fn the_journal_mode_can_be_read_not_set() {
    let mut c = ro(db_with_table()).await;
    let mode = text(&mut c, "PRAGMA journal_mode").await;
    assert_eq!(text(&mut c, "PRAGMA main.journal_mode").await, mode);
    for sql in [
        "PRAGMA journal_mode = wal",
        "PRAGMA journal_mode(off)",
        "PRAGMA main.journal_mode = memory",
        "PRAGMA journal_mode = ''",
    ] {
        assert_eq!(err_code(&mut c, sql).await, SQLITE_AUTH, "{sql}");
    }
    assert_eq!(text(&mut c, "PRAGMA journal_mode").await, mode);
}

/// Review I3, an accepted limitation (ADR 0002 m2b-t11): SQLite's WAL reader needs the `-wal`
/// and `-shm` files, so a read-only session on a WAL database may create them and leave them
/// behind (`immutable=1` would avoid it but is unsafe with a concurrent writer). The database
/// file itself stays byte-identical, and the `-wal` holds no frames.
#[tokio::test]
async fn a_read_only_session_on_a_wal_database_leaves_the_database_file_unchanged() {
    let dir = tempdir();
    let db = db_in(&dir);
    let path = db.path.clone();
    let w = rusqlite::Connection::open(&path).unwrap();
    let mode: String = w
        .query_row("PRAGMA journal_mode = wal", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    w.execute_batch("INSERT INTO t VALUES (2)").unwrap();
    // The last connection to close checkpoints and removes the side files.
    drop(w);
    assert!(stray_files(&dir).is_empty(), "{:?}", stray_files(&dir));
    let before = std::fs::read(&path).unwrap();

    let mut c = ro(db).await;
    assert_eq!(text(&mut c, "PRAGMA journal_mode").await, "wal");
    assert_eq!(text(&mut c, "SELECT count(*) FROM t").await, "2");
    for sql in [
        "INSERT INTO t VALUES (3)",
        "PRAGMA journal_mode = delete",
        "PRAGMA wal_checkpoint(TRUNCATE)",
        "VACUUM",
    ] {
        assert!(c.execute(sql, simple()).await.is_err(), "{sql}");
    }
    c.close().await;

    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "the database file changed"
    );
    let stray = stray_files(&dir);
    assert!(
        stray
            .iter()
            .all(|n| n == "db.sqlite-wal" || n == "db.sqlite-shm"),
        "{stray:?}"
    );
    let wal = dir.path().join("db.sqlite-wal");
    if wal.exists() {
        assert_eq!(
            std::fs::metadata(&wal).unwrap().len(),
            0,
            "WAL frames written"
        );
    }
}
