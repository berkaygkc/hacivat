// ## m4-t05-meta
//! Metadata sessions on real servers (M4 Task 5; R-M4-7, R-M4-46, ver/01 `#pg-canary` and
//! `#mysql-ro`): the read-only transaction, the pinned `search_path`, no XID, the statement
//! timeout and `application_name` on PostgreSQL 13–18; the UTC read-only session, its
//! consistent snapshot, DML and DDL refused, and the read-only session restored after a
//! reconnect on MySQL 8.0/8.4 and MariaDB 10.6/10.11/11.4.
//!
//! Run with `<main>/target/docker/run-docker-tests.sh --set pr|full -p hacivat-session
//! --test docker_meta_conn`. Every test is `#[ignore = "docker"]` and returns early unless
//! `HACIVAT_DOCKER=1`; targets come only from `require_target`. A test that creates objects
//! does so in its own `hvm4_t05_*` database (MySQL/MariaDB), dropped by a guard, also on panic.
//! `MetaQuery::new` is called here directly (test files are exempt from check-deps
//! `MetaQueryOnlyFromPacks`): the writes must reach the connection to be refused there.

mod common;

use std::time::Duration;

use common::conformance::*;
use hacivat_session::meta::{MetaConn, MetaConnSpec, MetaError, MetaQuery, Rows};
use hacivat_session::session::SessionManager;
use hacivat_testkit::{DockerTarget, require_target};
use tokio_util::sync::CancellationToken;

const PG: [&str; 6] = ["pg13", "pg14", "pg15", "pg16", "pg17", "pg18"];
const MYSQL: [&str; 5] = [
    "mysql80",
    "mysql84",
    "mariadb106",
    "mariadb1011",
    "mariadb114",
];

/// `t` connected on its own manager (pg13 is below the floor: allowed for this test).
async fn on(t: &DockerTarget) -> OnManager {
    let mut p = profile_for(t, auto_or_require(t));
    p.allow_unsupported_server = true;
    OnManager::connect(matrix_deps(), p).await
}

fn sessions(on: &OnManager) -> std::sync::Arc<SessionManager> {
    on.m.sessions(on.id()).expect("connected")
}

async fn meta(on: &OnManager, timeout: Duration) -> MetaConn {
    let cancel = CancellationToken::new();
    sessions(on)
        .meta_conn(
            MetaConnSpec {
                profile: on.id(),
                database: None,
                statement_timeout: timeout,
            },
            &cancel,
        )
        .await
        .unwrap_or_else(|e| panic!("a metadata connection: {e:?}"))
}

fn q(sql: &str) -> MetaQuery<'_> {
    MetaQuery::new(sql, &[], false)
}

fn first(rows: &Rows) -> String {
    rows.rows
        .first()
        .and_then(|r| r.first())
        .cloned()
        .flatten()
        .map(|s| s.to_string())
        .unwrap_or_default()
}

fn row(rows: &Rows) -> Vec<String> {
    rows.rows
        .first()
        .map(|r| {
            r.iter()
                .map(|c| c.as_deref().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// The server's code of a refused statement: MySQL's error number, else the SQLSTATE.
fn code(e: &MetaError) -> Option<String> {
    match e {
        MetaError::Server { message, .. } => code_of(Some(message)),
        _ => None,
    }
}

/// The session read-only variable of `t` (MariaDB before 11.1 has only `tx_read_only`).
fn ro_var(t: &DockerTarget) -> &'static str {
    match t.id.as_str() {
        "mariadb106" | "mariadb1011" => "@@session.tx_read_only",
        _ => "@@session.transaction_read_only",
    }
}

/// Drops database `db` of `t` when it goes, also on panic.
struct DropDb {
    t: DockerTarget,
    db: String,
}

impl Drop for DropDb {
    fn drop(&mut self) {
        let t = self.t.clone();
        let sql = format!("DROP DATABASE IF EXISTS {}", self.db);
        let rt = tokio::runtime::Handle::current();
        std::thread::scope(|s| {
            s.spawn(|| {
                rt.block_on(async {
                    let mut a = admin(&t).await;
                    batch(&mut a, &t.id, &sql).await;
                    a.close().await;
                });
            });
        });
    }
}

/// A fresh `hvm4_t05_*` database on `t` with table `t(a int)`, and its drop guard.
async fn fixture_db(t: &DockerTarget) -> DropDb {
    let db = unique("hvm4_t05");
    let mut a = admin(t).await;
    batch(&mut a, &t.id, &format!("CREATE DATABASE {db}")).await;
    batch(&mut a, &t.id, &format!("CREATE TABLE {db}.t (a int)")).await;
    a.close().await;
    DropDb { t: t.clone(), db }
}

// ---------------------------------------------------------------------------------------------
// PostgreSQL

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_snapshot_txn_is_repeatable_read_read_only_with_pinned_search_path() {
    if !docker_enabled() {
        return;
    }
    for t in PG.into_iter().filter_map(require_target) {
        let on = on(&t).await;
        let mut c = meta(&on, secs(60)).await;
        let mut snap = c.begin_snapshot().await.expect("a snapshot");
        let got = snap
            .rows(q("SELECT current_setting('transaction_isolation'), \
                 current_setting('transaction_read_only'), current_setting('search_path')"))
            .await
            .unwrap_or_else(|e| panic!("{}: {e:?}", t.id));
        assert_eq!(
            row(&got),
            ["repeatable read", "on", "pg_catalog, pg_temp"],
            "{}",
            t.id
        );
        snap.commit().await.unwrap();
        drop(c);
        on.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_meta_txn_refuses_writes() {
    if !docker_enabled() {
        return;
    }
    for t in PG.into_iter().filter_map(require_target) {
        let on = on(&t).await;
        let table = unique("hvm4_t05");
        let mut c = meta(&on, secs(60)).await;
        for write in [
            format!("CREATE TABLE public.{table} (a int)"),
            "INSERT INTO pg_catalog.pg_description VALUES (0, 0, 0, 'x')".to_string(),
        ] {
            let mut snap = c.begin_snapshot().await.unwrap();
            let e = snap.rows(q(&write)).await.expect_err("refused");
            assert_eq!(
                code(&e).as_deref(),
                Some("25006"),
                "{}: {write}: {e:?}",
                t.id
            );
            drop(snap);
        }
        drop(c);
        let mut a = admin(&t).await;
        assert_eq!(
            count(
                &mut a,
                &format!("SELECT count(*) FROM pg_catalog.pg_class WHERE relname = '{table}'")
            )
            .await,
            0,
            "{}: the table is absent",
            t.id
        );
        a.close().await;
        on.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn no_xid_assigned_by_a_metadata_pass() {
    if !docker_enabled() {
        return;
    }
    for t in PG.into_iter().filter_map(require_target) {
        let on = on(&t).await;
        let mut c = meta(&on, secs(60)).await;
        let mut snap = c.begin_snapshot().await.unwrap();
        let catalog = [
            "SELECT pg_catalog.pg_current_snapshot()::text",
            "SELECT pg_catalog.pg_snapshot_xmin(pg_catalog.pg_current_snapshot())::text",
            "SELECT count(*) FROM pg_catalog.pg_namespace",
            "SELECT count(*) FROM pg_catalog.pg_class",
            "SELECT count(*) FROM pg_catalog.pg_attribute",
            "SELECT count(*) FROM pg_catalog.pg_index",
            "SELECT count(*) FROM pg_catalog.pg_constraint",
            "SELECT count(*) FROM pg_catalog.pg_trigger",
            "SELECT count(*) FROM pg_catalog.pg_proc",
            "SELECT count(*) FROM pg_catalog.pg_type",
            "SELECT count(*) FROM pg_catalog.pg_description",
            "SELECT count(*) FROM pg_catalog.pg_depend",
            "SELECT count(*) FROM pg_catalog.pg_sequence",
            "SELECT count(*) FROM pg_catalog.pg_inherits",
            "SELECT count(*) FROM pg_catalog.pg_rewrite",
            "SELECT count(*) FROM pg_catalog.pg_enum",
            "SELECT count(*) FROM pg_catalog.pg_range",
            "SELECT count(*) FROM pg_catalog.pg_attrdef",
            "SELECT pg_catalog.current_setting('server_version_num')",
            "SELECT extract(epoch FROM pg_catalog.pg_postmaster_start_time())::text",
        ];
        assert_eq!(catalog.len(), 20);
        for sql in catalog {
            snap.rows(q(sql))
                .await
                .unwrap_or_else(|e| panic!("{}: {sql}: {e:?}", t.id));
        }
        let unassigned = snap
            .rows(q("SELECT txid_current_if_assigned() IS NULL"))
            .await
            .unwrap();
        assert_eq!(first(&unassigned), "true", "{}: no XID (#pg-canary)", t.id);
        snap.commit().await.unwrap();
        drop(c);
        on.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn application_name_is_hacivat_metadata() {
    if !docker_enabled() {
        return;
    }
    for t in PG.into_iter().filter_map(require_target) {
        let on = on(&t).await;
        let mut c = meta(&on, secs(60)).await;
        let mut snap = c.begin_snapshot().await.unwrap();
        let pid = first(&snap.rows(q("SELECT pg_backend_pid()")).await.unwrap());
        snap.commit().await.unwrap();
        let mut a = admin(&t).await;
        let name = scalar(
            &mut a,
            &format!("SELECT application_name FROM pg_catalog.pg_stat_activity WHERE pid = {pid}"),
        )
        .await;
        assert_eq!(name, "Hacivat (metadata)", "{}", t.id);
        a.close().await;
        drop(c);
        on.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn statement_timeout_applies() {
    if !docker_enabled() {
        return;
    }
    for t in PG.into_iter().filter_map(require_target) {
        let on = on(&t).await;
        let mut c = meta(&on, secs(1)).await;
        let mut snap = c.begin_snapshot().await.unwrap();
        assert_eq!(
            snap.rows(q("SELECT pg_sleep(2)")).await.map(drop),
            Err(MetaError::Timeout),
            "{}",
            t.id
        );
        drop(snap);
        drop(c);
        on.shutdown().await;
    }
}

// ---------------------------------------------------------------------------------------------
// MySQL and MariaDB

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn mysql_snapshot_sets_utc_read_only_session_and_consistent_snapshot() {
    if !docker_enabled() {
        return;
    }
    for t in MYSQL.into_iter().filter_map(require_target) {
        let on = on(&t).await;
        let mut c = meta(&on, secs(60)).await;
        let mut snap = c
            .begin_snapshot()
            .await
            .expect("the read-only session is confirmed");
        let sql = format!("SELECT @@session.time_zone, {}", ro_var(&t));
        let got = snap.rows(q(&sql)).await.unwrap();
        assert_eq!(row(&got), ["+00:00", "1"], "{}", t.id);
        snap.commit().await.unwrap();
        drop(c);
        on.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn mysql_meta_txn_refuses_dml_1792() {
    if !docker_enabled() {
        return;
    }
    for t in MYSQL.into_iter().filter_map(require_target) {
        let db = fixture_db(&t).await;
        let on = on(&t).await;
        let mut c = meta(&on, secs(60)).await;
        let mut snap = c.begin_snapshot().await.unwrap();
        let e = snap
            .rows(q(&format!("INSERT INTO {}.t VALUES (1)", db.db)))
            .await
            .expect_err("refused");
        assert_eq!(code(&e).as_deref(), Some("1792"), "{}: {e:?}", t.id);
        drop(snap);
        drop(c);
        let mut a = admin(&t).await;
        assert_eq!(
            count(&mut a, &format!("SELECT count(*) FROM {}.t", db.db)).await,
            0,
            "{}",
            t.id
        );
        a.close().await;
        on.shutdown().await;
    }
}

/// `CREATE TABLE hvm4_t05` on `t`'s metadata connection, inside its snapshot (ver/01
/// `#mysql-ro` step k: a read-only transaction alone lets DDL through, which implicitly commits
/// it; the read-only session refuses it): the code the server answered. The table is absent
/// afterwards.
async fn ddl_refused(t: &DockerTarget) -> String {
    let db = fixture_db(t).await;
    let on = on(t).await;
    let mut c = meta(&on, secs(60)).await;
    let ddl = format!("CREATE TABLE {}.hvm4_t05 (a int)", db.db);
    let mut snap = c.begin_snapshot().await.unwrap();
    let e = snap.rows(q(&ddl)).await.expect_err("DDL refused");
    let code = code(&e).unwrap_or_else(|| panic!("{}: {e:?}", t.id));
    drop(snap);
    drop(c);
    let mut a = admin(t).await;
    assert_eq!(
        count(
            &mut a,
            &format!(
                "SELECT count(*) FROM information_schema.tables \
                 WHERE table_schema = '{}' AND table_name = 'hvm4_t05'",
                db.db
            )
        )
        .await,
        0,
        "{}: the table is absent",
        t.id
    );
    a.close().await;
    on.shutdown().await;
    code
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn mysql_meta_session_refuses_ddl() {
    if !docker_enabled() {
        return;
    }
    for t in ["mysql80", "mysql84"]
        .into_iter()
        .filter_map(require_target)
    {
        let code = ddl_refused(&t).await;
        assert_eq!(code, "1792", "{}", t.id);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn mariadb_meta_session_refuses_ddl() {
    if !docker_enabled() {
        return;
    }
    for t in ["mariadb106", "mariadb1011", "mariadb114"]
        .into_iter()
        .filter_map(require_target)
    {
        // Refused, and the table is absent (`ddl_refused`); the observed code per server goes
        // to the report and ADR 0004 `## m4-t05` (ver/01 never ran this on MariaDB).
        let code = ddl_refused(&t).await;
        eprintln!(
            "m4-t05 observed: {} refuses DDL on the metadata session with {code}",
            t.id
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn read_only_flag_is_restored_after_reconnect() {
    if !docker_enabled() {
        return;
    }
    for t in MYSQL.into_iter().filter_map(require_target) {
        let on = on(&t).await;
        let mut c = meta(&on, secs(60)).await;
        let mut snap = c.begin_snapshot().await.unwrap();
        let before = first(&snap.rows(q("SELECT CONNECTION_ID()")).await.unwrap());
        snap.commit().await.unwrap();
        kill_backend(&t, &before).await;
        // The loss is reported, never hidden: the first snapshot after the kill answers it
        // (`ConnectionLost`, or a snapshot when the kill landed later); the next one runs on
        // a new connection, set up again.
        match c.begin_snapshot().await.map(drop) {
            Ok(()) | Err(MetaError::ConnectionLost) => {}
            Err(e) => panic!("{}: {e:?}", t.id),
        }
        let mut snap = c
            .begin_snapshot()
            .await
            .unwrap_or_else(|e| panic!("{}: reconnect: {e:?}", t.id));
        let sql = format!(
            "SELECT CONNECTION_ID(), {}, @@session.time_zone",
            ro_var(&t)
        );
        let after = row(&snap.rows(q(&sql)).await.unwrap());
        assert_ne!(after[0], before, "{}: a new connection", t.id);
        assert_eq!(after[1..], ["1", "+00:00"], "{}: set up again", t.id);
        snap.commit().await.unwrap();
        drop(c);
        on.shutdown().await;
    }
}
