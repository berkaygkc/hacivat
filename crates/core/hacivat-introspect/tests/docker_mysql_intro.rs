// ## m4-t17-mysql
//! The MySQL and MariaDB introspector on real servers (M4 Task 17; alg/02 HV-INTRO-07 tests
//! 1–7, HV-INTRO-13 tests 1 and 7, HV-INTRO-14 test 3, HV-INTRO-15 test 3, HV-INTRO-01 test 6;
//! R-M4-37, R-M4-39, R-M4-41; spike findings F1, F2, F3, F9).
//!
//! Run with `<main>/target/docker/run-docker-tests.sh --set pr|full -p hacivat-introspect
//! --test docker_mysql_intro` after `cargo test -p hacivat-introspect --no-run`. Every test is
//! `#[ignore = "docker"]` and returns early unless `HACIVAT_DOCKER=1`; targets come only from
//! `require_target`. Each test works in its own database `hvm4_t17_<slug>` (R-M4-43: `hv_test`
//! has `ALL` on `hvm4\_%`), dropped by a guard, also on panic. `SET GLOBAL` goes through the
//! `hv_fixture` definer procedures and is restored by a guard (the wrapper's lock gives
//! exclusivity). Waits are conditions on the server clock (`UTC_TIMESTAMP(6)`), never a
//! fixed sleep.
//!
//! **Harness (NEEDS_CONTEXT, T17 report).** A `MetaConn` comes only from a connected
//! `SessionManager`, whose `ConnectDeps` need `hacivat_net::ssh::{KnownHosts, TunnelManager}`;
//! `hacivat-net` is not a dependency of this crate and only Task 1a edits manifests. The
//! [`harness`] functions are therefore fail-loud stubs: every test here compiles and fails
//! with that message until the controller adds the dev-dependency and fills them (the T17
//! report gives the bodies, after `hacivat-exec/tests/common/mod.rs`'s `deps()` and `Side`).

#![cfg(feature = "session")]

use std::fmt::Write as _;
use std::sync::Arc;

use hacivat_introspect::mysql::{MySqlIntrospector, PassStats};
use hacivat_introspect::{IntroError, Introspector, Mode};
use hacivat_meta::merge::{MergePolicy, NamespaceResult, base, merge};
use hacivat_meta::name::{DialectId, FoldRule, IdentCtx, NameClass, match_key};
use hacivat_meta::props::{ColumnExt, FkAction, Props};
use hacivat_meta::{
    Level, Name, NamespaceSnapshot, NsKey, ObjFlags, ObjectKey, ObjectKind, StateMark,
};
use hacivat_proto::DbmsKind;
use hacivat_session::meta::MetaConn;
use hacivat_testkit::{DockerTarget, docker_enabled, require_target};
use smallvec::smallvec;
use tokio_util::sync::CancellationToken;

const MYSQL: [&str; 2] = ["mysql80", "mysql84"];
const ALL: [&str; 6] = [
    "mysql80",
    "mysql84",
    "mysql84-x509",
    "mariadb106",
    "mariadb1011",
    "mariadb114",
];
const MARIADB: [&str; 3] = ["mariadb106", "mariadb1011", "mariadb114"];
const FOLD: [&str; 3] = ["mysql80", "mysql84", "mariadb114"];

/// The connections a test needs. See the module docs: stubs until the dev-dependency lands.
mod harness {
    use super::*;

    const MISSING: &str = "docker_mysql_intro needs a MetaConn on a connected SessionManager: \
        hacivat-introspect lacks the `hacivat-net` dev-dependency that ConnectDeps needs \
        (T17 NEEDS_CONTEXT; see task-m4-17-report.md for the harness bodies)";

    /// Which account a statement runs as.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Login {
        /// `hv_test`: the fixtures' DDL in `hvm4_*` databases.
        Test,
        /// `hv_fixture`: only the `hv_fixture_admin` definer procedures (`SET GLOBAL`, users).
        Fixture,
    }

    /// A read-only metadata connection of `t` on database `db` (Task 5's `meta_conn`).
    pub async fn meta(t: &DockerTarget, db: &str) -> MetaConn {
        let _ = (t, db);
        panic!("{MISSING}")
    }

    /// Runs `sql` as `login` on `db` (`None`: the login's default); the server's error code
    /// (`1054`, `1419`, …) when it fails.
    pub async fn exec(
        t: &DockerTarget,
        login: Login,
        db: Option<&str>,
        sql: &str,
    ) -> Result<(), String> {
        let _ = (t, login, db, sql);
        panic!("{MISSING}")
    }

    /// The rows of `sql` as `hv_test` on `db`, every value as text (NULL as `""`).
    pub async fn rows(t: &DockerTarget, db: Option<&str>, sql: &str) -> Vec<Vec<String>> {
        let _ = (t, db, sql);
        panic!("{MISSING}")
    }

    /// [`exec`] on a fresh runtime thread, for drop guards.
    pub fn exec_blocking(t: &DockerTarget, login: Login, sql: &str) {
        let (t, sql) = (t.clone(), sql.to_string());
        let _ = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime");
            rt.block_on(async { exec(&t, login, None, &sql).await.ok() })
        })
        .join();
    }
}

use harness::Login;

// ---- fixtures ---------------------------------------------------------------------------------

fn n(s: &str) -> Name {
    Arc::from(s)
}

/// `hvm4_t17_<slug>`, dropped when the guard goes (also on panic).
struct Db {
    t: DockerTarget,
    name: String,
}

impl Db {
    async fn create(t: &DockerTarget, slug: &str) -> Db {
        let name = format!("hvm4_t17_{slug}");
        harness::exec(
            t,
            Login::Test,
            None,
            &format!("DROP DATABASE IF EXISTS `{name}`"),
        )
        .await
        .expect("drop leftover");
        harness::exec(t, Login::Test, None, &format!("CREATE DATABASE `{name}`"))
            .await
            .unwrap_or_else(|e| panic!("{}: create {name}: {e}", t.id));
        Db { t: t.clone(), name }
    }

    async fn run(&self, sql: &str) {
        harness::exec(&self.t, Login::Test, Some(&self.name), sql)
            .await
            .unwrap_or_else(|e| panic!("{}: `{sql}`: {e}", self.t.id));
    }

    fn ns(&self) -> NsKey {
        NsKey::schema([self.name.as_str()])
    }

    fn key(&self, kind: ObjectKind, path: &[&str]) -> ObjectKey {
        let mut p = smallvec![n(&self.name)];
        p.extend(path.iter().map(|s| n(s)));
        ObjectKey {
            kind,
            path: p,
            server_id: None,
            overload_sig: None,
            raw_kind: None,
        }
    }
}

impl Drop for Db {
    fn drop(&mut self) {
        harness::exec_blocking(
            &self.t,
            Login::Test,
            &format!("DROP DATABASE IF EXISTS `{}`", self.name),
        );
    }
}

/// Restores a global variable when it goes.
struct GlobalGuard {
    t: DockerTarget,
    name: &'static str,
    value: String,
}

impl Drop for GlobalGuard {
    fn drop(&mut self) {
        harness::exec_blocking(
            &self.t,
            Login::Fixture,
            &hacivat_testkit::fixture_sql::set_global(self.name, &self.value),
        );
    }
}

fn intro(t: &DockerTarget) -> MySqlIntrospector {
    MySqlIntrospector { flavor: t.engine }
}

fn targets(ids: &[&str]) -> Vec<DockerTarget> {
    if !docker_enabled() {
        return Vec::new();
    }
    ids.iter().filter_map(|id| require_target(id)).collect()
}

/// One pass on `db` merged onto `prev` (or the empty namespace).
async fn pass(
    t: &DockerTarget,
    db: &Db,
    mode: Mode,
    prev: Option<&NamespaceSnapshot>,
) -> (NamespaceSnapshot, NamespaceResult, PassStats) {
    let mut c = harness::meta(t, &db.name).await;
    let (res, stats) = intro(t)
        .introspect_with_stats(
            &mut c,
            &db.ns(),
            mode,
            Level::Structure,
            prev,
            &CancellationToken::new(),
        )
        .await
        .unwrap_or_else(|e| panic!("{}: {mode:?}: {e:?}", t.id));
    let b = match prev {
        Some(p) => p.clone(),
        None => base(db.ns(), t.engine, FoldRule::Exact).expect("base"),
    };
    let next = merge(Some(&b), res.clone(), &MergePolicy::default())
        .unwrap_or_else(|e| panic!("{}: merge: {e:?}", t.id))
        .next()
        .clone();
    (next, res, stats)
}

/// Every object's key with its kind data, sorted (the G4 comparison).
fn canon(s: &NamespaceSnapshot) -> Vec<(ObjectKey, StateMark, ObjFlags, Props)> {
    let mut v: Vec<_> = (0..s.len() as u32)
        .map(|i| {
            let o = s.obj(i).expect("object");
            (
                s.object_key(i).expect("key"),
                o.state(),
                o.flags(),
                o.props().clone(),
            )
        })
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

/// An incremental pass over `prev` equals a fresh full pass; returns it and its log.
async fn incremental_equals_full(
    t: &DockerTarget,
    db: &Db,
    prev: &NamespaceSnapshot,
) -> (NamespaceSnapshot, PassStats) {
    let (inc, _, stats) = pass(t, db, Mode::Incremental, Some(prev)).await;
    let (full, _, _) = pass(t, db, Mode::Full, None).await;
    assert_eq!(canon(&inc), canon(&full), "{}: incremental == full", t.id);
    (inc, stats)
}

fn reread(stats: &PassStats) -> Vec<&str> {
    stats.relations.iter().map(|n| &**n).collect()
}

/// The server's UTC time, microseconds.
async fn server_now(t: &DockerTarget) -> i64 {
    let r = harness::rows(t, None, "SELECT CAST(UTC_TIMESTAMP(6) AS CHAR)").await;
    hacivat_introspect::mysql::rows::parse_time_micros(&r[0][0]).expect("a time")
}

/// Waits until the server clock enters the next second (a condition on `UTC_TIMESTAMP(6)`).
async fn next_server_second(t: &DockerTarget) {
    let start = server_now(t).await / 1_000_000;
    while server_now(t).await / 1_000_000 == start {
        tokio::task::yield_now().await;
    }
}

/// Waits until the server clock is more than one second past `w` (out of the window).
async fn past_window(t: &DockerTarget, w: i64) {
    while server_now(t).await < w + 2_000_000 {
        tokio::task::yield_now().await;
    }
}

fn watermark_of(s: &NamespaceSnapshot) -> i64 {
    match s.meta().watermark {
        hacivat_meta::Watermark::MyTime { at_utc_micros, .. } => at_utc_micros,
        _ => panic!("a MySQL watermark"),
    }
}

async fn base_schema(db: &Db) {
    db.run("CREATE TABLE customers (id INT PRIMARY KEY, name VARCHAR(64) NOT NULL)")
        .await;
    db.run(
        "CREATE TABLE orders (id INT PRIMARY KEY, customer_id INT, \
         status ENUM('new','done') DEFAULT 'new', \
         CONSTRAINT fk_cust FOREIGN KEY (customer_id) REFERENCES customers (id))",
    )
    .await;
    db.run("CREATE VIEW v_orders AS SELECT id, status FROM orders")
        .await;
}

// ---- HV-INTRO-07 (MySQL incremental) -----------------------------------------------------------

#[tokio::test]
#[ignore = "docker"]
async fn instant_add_column_rereads_the_table() {
    for t in targets(&MYSQL) {
        let db = Db::create(&t, "instant").await;
        base_schema(&db).await;
        let (s0, _, _) = pass(&t, &db, Mode::Full, None).await;
        db.run("ALTER TABLE customers ADD COLUMN email VARCHAR(128), ALGORITHM=INSTANT")
            .await;
        let (s1, stats) = incremental_equals_full(&t, &db, &s0).await;
        assert!(reread(&stats).contains(&"customers"), "{}", t.id);
        assert!(
            s1.find(&db.key(ObjectKind::Column, &["customers", "email"]))
                .is_some()
        );
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn add_index_reread() {
    for t in targets(&MYSQL) {
        let db = Db::create(&t, "addindex").await;
        base_schema(&db).await;
        let (s0, _, _) = pass(&t, &db, Mode::Full, None).await;
        db.run("CREATE INDEX ix_name ON customers (name)").await;
        let (s1, stats) = incremental_equals_full(&t, &db, &s0).await;
        assert!(reread(&stats).contains(&"customers"), "{}", t.id);
        assert!(
            s1.find(&db.key(ObjectKind::Index, &["customers", "ix_name"]))
                .is_some()
        );
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn create_trigger_found_although_create_time_unchanged() {
    for t in targets(&MYSQL) {
        let db = Db::create(&t, "trigger").await;
        base_schema(&db).await;
        let (s0, _, _) = pass(&t, &db, Mode::Full, None).await;
        let before = harness::rows(
            &t,
            Some(&db.name),
            "SELECT CAST(CREATE_TIME AS CHAR) FROM INFORMATION_SCHEMA.TABLES \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'orders'",
        )
        .await;
        db.run("CREATE TRIGGER trg_o BEFORE INSERT ON orders FOR EACH ROW SET NEW.status = 'new'")
            .await;
        let after = harness::rows(
            &t,
            Some(&db.name),
            "SELECT CAST(CREATE_TIME AS CHAR) FROM INFORMATION_SCHEMA.TABLES \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'orders'",
        )
        .await;
        // ver/01 #mysql-is-times: CREATE TRIGGER leaves CREATE_TIME alone.
        assert_eq!(before, after, "{}", t.id);
        let (s1, stats) = incremental_equals_full(&t, &db, &s0).await;
        assert!(reread(&stats).contains(&"orders"), "{}", t.id);
        assert!(
            s1.find(&db.key(ObjectKind::Trigger, &["orders", "trg_o"]))
                .is_some()
        );
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn insert_only_rereads_no_table() {
    for t in targets(&MYSQL) {
        let db = Db::create(&t, "insert").await;
        base_schema(&db).await;
        let (s0, _, _) = pass(&t, &db, Mode::Full, None).await;
        past_window(&t, watermark_of(&s0)).await;
        let (s0, _, _) = pass(&t, &db, Mode::Incremental, Some(&s0)).await;
        db.run("INSERT INTO customers VALUES (1, 'a')").await;
        let (_, stats) = incremental_equals_full(&t, &db, &s0).await;
        assert_eq!(stats.full_reason, None, "{}", t.id);
        assert!(reread(&stats).is_empty(), "{}: {:?}", t.id, stats);
        assert!(stats.routines.is_empty() && stats.events.is_empty());
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn ddl_in_the_same_second_as_the_watermark_is_caught() {
    for t in targets(&MYSQL) {
        let db = Db::create(&t, "samesec").await;
        base_schema(&db).await;
        // DDL, pass, DDL inside one server second.
        // (When the three cross a second boundary the markers differ instead: caught too.)
        next_server_second(&t).await;
        db.run("ALTER TABLE customers ADD COLUMN a INT").await;
        let (s0, _, _) = pass(&t, &db, Mode::Full, None).await;
        db.run("ALTER TABLE customers ADD COLUMN b INT").await;
        let (s1, stats) = incremental_equals_full(&t, &db, &s0).await;
        assert!(reread(&stats).contains(&"customers"), "{}", t.id);
        assert!(
            s1.find(&db.key(ObjectKind::Column, &["customers", "b"]))
                .is_some()
        );
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn create_or_replace_view_rehashes_and_rereads_columns() {
    for t in targets(&MYSQL) {
        let db = Db::create(&t, "view").await;
        base_schema(&db).await;
        let (s0, _, _) = pass(&t, &db, Mode::Full, None).await;
        db.run("CREATE OR REPLACE VIEW v_orders AS SELECT id, status, customer_id FROM orders")
            .await;
        let (s1, stats) = incremental_equals_full(&t, &db, &s0).await;
        assert_eq!(reread(&stats), ["v_orders"], "{}", t.id);
        assert!(
            s1.find(&db.key(ObjectKind::Column, &["v_orders", "customer_id"]))
                .is_some()
        );
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn global_time_zone_plus_three_still_compares_markers() {
    for t in targets(&MYSQL) {
        let was = harness::rows(&t, None, "SELECT @@GLOBAL.time_zone").await[0][0].clone();
        let _restore = GlobalGuard {
            t: t.clone(),
            name: "time_zone",
            value: was,
        };
        harness::exec(
            &t,
            Login::Fixture,
            None,
            &hacivat_testkit::fixture_sql::set_global("time_zone", "+03:00"),
        )
        .await
        .unwrap_or_else(|e| panic!("{}: SET GLOBAL: {e}", t.id));
        let db = Db::create(&t, "tz").await;
        base_schema(&db).await;
        let (s0, _, _) = pass(&t, &db, Mode::Full, None).await;
        // The metadata session runs in UTC whatever the global zone: a pass without DDL
        // re-reads nothing once out of the window, and an ALTER is still seen.
        past_window(&t, watermark_of(&s0)).await;
        let (s1, stats) = incremental_equals_full(&t, &db, &s0).await;
        assert_eq!(stats.full_reason, None, "{}", t.id);
        let (s1, _) = (s1, stats);
        past_window(&t, watermark_of(&s1)).await;
        let (s2, stats) = incremental_equals_full(&t, &db, &s1).await;
        assert!(reread(&stats).is_empty(), "{}: {:?}", t.id, stats);
        db.run("ALTER TABLE orders ADD COLUMN note TEXT").await;
        let (_, stats) = incremental_equals_full(&t, &db, &s2).await;
        assert!(reread(&stats).contains(&"orders"), "{}", t.id);
    }
}

// ---- HV-INTRO-13 (assembly) -----------------------------------------------------------------------

#[tokio::test]
#[ignore = "docker"]
async fn index_columns_with_desc_part() {
    for t in targets(&MYSQL) {
        let db = Db::create(&t, "desc").await;
        db.run("CREATE TABLE t (a INT, b INT, c INT, KEY ix (a, b DESC, c))")
            .await;
        let (s, _, _) = pass(&t, &db, Mode::Full, None).await;
        let ix = s
            .find(&db.key(ObjectKind::Index, &["t", "ix"]))
            .expect("ix");
        let Props::Index(p) = s.obj(ix).unwrap().props() else {
            panic!()
        };
        let cols: Vec<&str> = p
            .key
            .columns
            .iter()
            .map(|c| c.name.as_deref().unwrap())
            .collect();
        assert_eq!(
            cols,
            ["a", "b", "c"],
            "{}: key order, not column order",
            t.id
        );
        assert_eq!(p.key.desc, [false, true, false], "{}", t.id);
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn six_hundred_indexed_columns_assemble_complete() {
    for t in targets(&MYSQL) {
        let db = Db::create(&t, "wide").await;
        // 40 tables × 15 indexed columns = 600 key parts, grouped by the client.
        for i in 0..40 {
            let cols: Vec<String> = (0..15).map(|j| format!("c{j} INT")).collect();
            let keys: Vec<String> = (0..15).map(|j| format!("KEY k{j} (c{j})")).collect();
            db.run(&format!(
                "CREATE TABLE w{i} ({}, {})",
                cols.join(", "),
                keys.join(", ")
            ))
            .await;
        }
        let (s, _, _) = pass(&t, &db, Mode::Full, None).await;
        let parts: usize = s
            .iter_named(&[ObjectKind::Index])
            .map(|(_, o)| match o.props() {
                Props::Index(p) => p.key.columns.len(),
                _ => 0,
            })
            .sum();
        assert_eq!(parts, 600, "{}", t.id);
        assert!(
            s.iter_named(&[ObjectKind::Index])
                .all(|(_, o)| !o.flags().contains(ObjFlags::BROKEN))
        );
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn functional_key_part_and_invisible_index_read() {
    for t in targets(&MYSQL) {
        let db = Db::create(&t, "funckey").await;
        db.run("CREATE TABLE t (a VARCHAR(20), b INT, KEY fx ((UPPER(a))), KEY iv (b) INVISIBLE)")
            .await;
        let (s, _, _) = pass(&t, &db, Mode::Full, None).await;
        let fx = s
            .find(&db.key(ObjectKind::Index, &["t", "fx"]))
            .expect("fx");
        let Props::Index(p) = s.obj(fx).unwrap().props() else {
            panic!()
        };
        assert_eq!(p.key.columns[0].name, None, "{}", t.id);
        assert!(!p.key.expressions[0].is_empty(), "{}", t.id);
        let iv = s
            .find(&db.key(ObjectKind::Index, &["t", "iv"]))
            .expect("iv");
        let Props::Index(p) = s.obj(iv).unwrap().props() else {
            panic!()
        };
        assert!(p.key.invisible, "{}", t.id);
    }
}

// ---- HV-INTRO-14 (names) -------------------------------------------------------------------------

#[tokio::test]
#[ignore = "docker"]
async fn turkish_collation_database_items_vs_items_missing() {
    for t in targets(&MYSQL) {
        let name = "hvm4_t17_trdb";
        harness::exec(
            &t,
            Login::Test,
            None,
            &format!("DROP DATABASE IF EXISTS {name}"),
        )
        .await
        .ok();
        harness::exec(
            &t,
            Login::Test,
            None,
            &format!("CREATE DATABASE {name} CHARACTER SET utf8mb4 COLLATE utf8mb4_tr_0900_ai_ci"),
        )
        .await
        .expect("the Turkish database");
        let db = Db {
            t: t.clone(),
            name: name.into(),
        };
        db.run("CREATE TABLE items (id INT PRIMARY KEY)").await;
        let (s, _, _) = pass(&t, &db, Mode::Full, None).await;
        // Exact names under lctn 0: `items` is found, `İTEMS` and `ıtems` are not the same
        // table and a probe never folds through the database collation.
        assert!(
            s.find(&db.key(ObjectKind::Table, &["items"])).is_some(),
            "{}",
            t.id
        );
        assert!(s.find(&db.key(ObjectKind::Table, &["İTEMS"])).is_none());
        assert!(s.find(&db.key(ObjectKind::Table, &["ıtems"])).is_none());
    }
}

/// R-M4-41: records how the server matches non-ASCII column names (artifact
/// `t17-mysql-column-fold.tsv`) and pins that the client never answers a wrong `Missing`.
#[tokio::test]
#[ignore = "docker"]
async fn mysql_column_name_matching_is_pinned() {
    let mut tsv = String::from("target\tcharset_system\tcolumn_collation\tstored\tprobe\tserver\n");
    for t in targets(&FOLD) {
        let db = Db::create(&t, "fold").await;
        let dialect = if t.engine == DbmsKind::MariaDb {
            DialectId::MariaDb
        } else {
            DialectId::MySql
        };
        let sys = harness::rows(&t, None, "SELECT @@character_set_system").await[0][0].clone();
        let coll = harness::rows(
            &t,
            None,
            "SELECT COLLATION_NAME FROM INFORMATION_SCHEMA.COLUMNS \
             WHERE TABLE_SCHEMA = 'information_schema' AND TABLE_NAME = 'COLUMNS' \
             AND COLUMN_NAME = 'COLUMN_NAME'",
        )
        .await
        .first()
        .and_then(|r| r.first().cloned())
        .unwrap_or_default();
        for (k, stored) in ["Äb", "İd", "ıd"].into_iter().enumerate() {
            let table = format!("t{k}");
            if let Err(code) = harness::exec(
                &t,
                Login::Test,
                Some(&db.name),
                &format!("CREATE TABLE {table} (`{stored}` INT)"),
            )
            .await
            {
                let _ = writeln!(tsv, "{}	{sys}	{coll}	{stored}	(create)	{code}", t.id);
                continue;
            }
            for probe in ["äb", "id", "ID", "Id", "İd", "ıd"] {
                let found = harness::exec(
                    &t,
                    Login::Test,
                    Some(&db.name),
                    &format!("SELECT `{probe}` FROM {table} LIMIT 0"),
                )
                .await;
                let verdict = match &found {
                    Ok(()) => "found".to_string(),
                    Err(code) => code.clone(),
                };
                let _ = writeln!(tsv, "{}\t{sys}\t{coll}\t{stored}\t{probe}\t{verdict}", t.id);
                if found.is_ok() {
                    // The server says the same column: the client may say "same" or "not
                    // pinned", never "different" (a wrong Missing, F31).
                    let ctx = IdentCtx::new(dialect);
                    let a = match_key(dialect, &ctx, stored, true, NameClass::Column);
                    let b = match_key(dialect, &ctx, probe, true, NameClass::Column);
                    if let (Ok(a), Ok(b)) = (a, b) {
                        assert_eq!(a, b, "{}: {stored} vs {probe}", t.id);
                    }
                }
            }
        }
    }
    if docker_enabled() {
        std::fs::write(hacivat_testkit::artifact("t17-mysql-column-fold.tsv"), tsv)
            .expect("the artifact");
    }
}

// ---- HV-INTRO-15 (state) ---------------------------------------------------------------------

#[tokio::test]
#[ignore = "docker"]
async fn trigger_creation_changes_table_state() {
    for t in targets(&MYSQL) {
        let db = Db::create(&t, "state").await;
        base_schema(&db).await;
        let key = db.key(ObjectKind::Table, &["orders"]);
        let mut c = harness::meta(&t, &db.name).await;
        let before = intro(&t)
            .stamps(&mut c, std::slice::from_ref(&key))
            .await
            .unwrap();
        db.run("CREATE TRIGGER trg_s BEFORE INSERT ON orders FOR EACH ROW SET NEW.id = NEW.id")
            .await;
        let after = intro(&t).stamps(&mut c, &[key]).await.unwrap();
        assert_ne!(before, after, "{}", t.id);
        let (
            Some(StateMark::MyTable {
                trigger_count: a, ..
            }),
            Some(StateMark::MyTable {
                trigger_count: b, ..
            }),
        ) = (before[0], after[0])
        else {
            panic!("{}: table stamps", t.id);
        };
        assert_eq!((a, b), (0, 1), "{}", t.id);
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn stamp_moves_on_comment_and_second_trigger() {
    for t in targets(&ALL) {
        let db = Db::create(&t, "stamp").await;
        base_schema(&db).await;
        db.run("CREATE TRIGGER trg_1 BEFORE INSERT ON orders FOR EACH ROW SET NEW.id = NEW.id")
            .await;
        let key = db.key(ObjectKind::Table, &["orders"]);
        let mut c = harness::meta(&t, &db.name).await;
        let s0 = intro(&t)
            .stamps(&mut c, std::slice::from_ref(&key))
            .await
            .unwrap()[0];
        next_server_second(&t).await;
        db.run("ALTER TABLE orders COMMENT = 'x'").await;
        let s1 = intro(&t)
            .stamps(&mut c, std::slice::from_ref(&key))
            .await
            .unwrap()[0];
        assert_ne!(s0, s1, "{}: a comment moves CREATE_TIME (F3)", t.id);
        next_server_second(&t).await;
        db.run("CREATE TRIGGER trg_2 AFTER INSERT ON orders FOR EACH ROW SET @x = 1")
            .await;
        let s2 = intro(&t).stamps(&mut c, &[key]).await.unwrap()[0];
        assert_ne!(s1, s2, "{}: a second trigger moves the stamp", t.id);
    }
}

// ---- HV-INTRO-01 test 6 (partial load) ------------------------------------------------------

/// A login with no access to `mysql.*` (every `hv_test`-like login): the schema passes are
/// unaffected and fail no kind; the account kinds (users, roles) are the `Server` namespace,
/// whose sections are Task 17b's and answer `NotImplemented` until then.
#[tokio::test]
#[ignore = "docker"]
async fn revoked_mysql_schema_select_fails_users_kind_only() {
    for t in targets(&MYSQL) {
        let db = Db::create(&t, "partial").await;
        base_schema(&db).await;
        let denied = harness::exec(&t, Login::Test, None, "SELECT COUNT(*) FROM mysql.user").await;
        assert!(
            denied.is_err(),
            "{}: hv_test must not read mysql.user",
            t.id
        );
        let (_, res, _) = pass(&t, &db, Mode::Full, None).await;
        assert!(
            res.existent
                .values()
                .all(|e| matches!(e, hacivat_meta::merge::ExistSet::Ok(_))),
            "{}",
            t.id
        );
        let mut c = harness::meta(&t, &db.name).await;
        let server = intro(&t)
            .introspect(
                &mut c,
                &NsKey::server(),
                Mode::Full,
                Level::Structure,
                None,
                &CancellationToken::new(),
            )
            .await;
        assert!(
            matches!(server, Err(IntroError::NotImplemented)),
            "{}",
            t.id
        );
    }
}

// ---- the pack on every server ------------------------------------------------------------------

/// alg/02 HV-INTRO-04 step 4: every section, in the variant this server gets, prepares on the
/// typed path and returns its declared columns (a `show-create` section by count: the server
/// names those columns).
async fn sections_prepare(ids: &[&str]) {
    use hacivat_introspect::mysql::pass::{SectionReader, read_server};
    use hacivat_introspect::mysql::{TxnReader, show_create};
    use hacivat_introspect::pack::{Pack, Params};
    let pack = Pack::builtin(DialectId::MySql).expect("pack");
    for t in targets(ids) {
        let db = Db::create(&t, "prepare").await;
        base_schema(&db).await;
        db.run("CREATE EVENT ev ON SCHEDULE EVERY 1 DAY DO SET @x = 1")
            .await;
        db.run("CREATE PROCEDURE p() SET @x = 1").await;
        db.run("CREATE TRIGGER trg BEFORE INSERT ON orders FOR EACH ROW SET NEW.id = NEW.id")
            .await;
        let mut c = harness::meta(&t, &db.name).await;
        let mut txn = c.begin_snapshot().await.expect("snapshot");
        let mut r = TxnReader::new(&mut txn, None).expect("reader");
        let srv = read_server(&mut r, t.engine).await.expect("identity");
        for s in pack.sections() {
            let sec = pack.section(s).unwrap();
            let mut params = Params {
                scalars: vec![
                    (n("schema"), hacivat_introspect::mysql::pass::text(&db.name)),
                    (n("name_like"), hacivat_introspect::mysql::pass::text("%")),
                    (
                        n("like_escape"),
                        hacivat_introspect::mysql::pass::text("\\"),
                    ),
                ],
                lists: vec![(
                    n("names"),
                    vec![hacivat_introspect::mysql::pass::text("orders")],
                )],
            };
            let name = match &**s {
                "ShowCreateTable" => "orders",
                "ShowCreateView" => "v_orders",
                "ShowCreateProcedure" => "p",
                "ShowCreateFunction" => continue, // MySQL needs a function: covered by flavor test
                "ShowCreateTrigger" => "trg",
                "ShowCreateEvent" => "ev",
                _ => "orders",
            };
            params
                .scalars
                .push((n("name"), hacivat_introspect::mysql::pass::text(name)));
            let schema = if sec.returns().iter().any(|c| &*c.name == "variable_name")
                || s.starts_with("System")
            {
                "information_schema"
            } else {
                db.name.as_str()
            };
            params.scalars[0].1 = hacivat_introspect::mysql::pass::text(schema);
            for mode in [Mode::Full, Mode::Incremental] {
                let ctx = srv.pack_ctx(mode, Level::Sources, params.clone());
                if pack.render(s, &ctx).is_err() {
                    continue; // gated away for this server (ServerUuid on MariaDB, …)
                }
                let rows = r
                    .read(Box::leak(s.to_string().into_boxed_str()), ctx)
                    .await
                    .unwrap_or_else(|e| panic!("{}: {s} {mode:?}: {e:?}", t.id));
                if sec.show_create() {
                    assert_eq!(rows.columns.len(), sec.returns().len(), "{}: {s}", t.id);
                } else {
                    let got: Vec<&str> = rows.columns.iter().map(|c| &**c).collect();
                    let want: Vec<&str> = sec.returns().iter().map(|c| &*c.name).collect();
                    assert_eq!(got, want, "{}: {s}", t.id);
                }
            }
        }
        let _ = show_create::section_of(ObjectKind::Table);
        drop(r);
        txn.commit().await.expect("commit");
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn every_mysql_section_prepares_and_returns_its_declared_columns() {
    sections_prepare(&MYSQL).await;
}

#[tokio::test]
#[ignore = "docker"]
async fn every_mariadb_section_prepares() {
    sections_prepare(&MARIADB).await;
}

/// F2: inside the snapshot, DDL from another session is invisible; the next pass equals a
/// fresh full pass.
#[tokio::test]
#[ignore = "docker"]
async fn snapshot_hides_later_ddl_and_next_pass_catches_it() {
    use hacivat_introspect::mysql::TxnReader;
    use hacivat_introspect::mysql::pass::{SectionReader, read_server, schema_params};
    for t in targets(&MYSQL) {
        let db = Db::create(&t, "snapshot").await;
        base_schema(&db).await;
        let (s0, _, _) = pass(&t, &db, Mode::Full, None).await;
        let mut c = harness::meta(&t, &db.name).await;
        let mut txn = c.begin_snapshot().await.expect("snapshot");
        let mut r = TxnReader::new(&mut txn, None).expect("reader");
        let srv = read_server(&mut r, t.engine).await.unwrap();
        let ctx = || srv.pack_ctx(Mode::Full, Level::Structure, schema_params(&db.name));
        let before = r.read("TableMarks", ctx()).await.unwrap().rows.len();
        db.run("CREATE TABLE late (id INT)").await;
        db.run("ALTER TABLE customers ADD COLUMN later INT").await;
        let inside = r.read("TableMarks", ctx()).await.unwrap().rows.len();
        assert_eq!(before, inside, "{}: frozen inside the snapshot", t.id);
        drop(r);
        txn.commit().await.unwrap();
        let (s1, stats) = incremental_equals_full(&t, &db, &s0).await;
        assert!(reread(&stats).contains(&"late") && reread(&stats).contains(&"customers"));
        assert!(s1.find(&db.key(ObjectKind::Table, &["late"])).is_some());
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn show_create_sections_quote_hostile_names() {
    for t in targets(&MYSQL) {
        let db = Db::create(&t, "hostile").await;
        for name in ["a``b", "İ", "t ", "a:x?b"] {
            db.run(&format!("CREATE TABLE `{name}` (id INT)")).await;
        }
        let mut c = harness::meta(&t, &db.name).await;
        for name in ["a`b", "İ", "t ", "a:x?b"] {
            let text = intro(&t)
                .show_create(&mut c, ObjectKind::Table, &db.name, name)
                .await
                .unwrap_or_else(|e| panic!("{}: {name:?}: {e:?}", t.id))
                .unwrap_or_else(|| panic!("{}: {name:?}: no row", t.id));
            let quoted = format!("`{}`", name.replace('`', "``"));
            assert!(
                text.starts_with(&format!("CREATE TABLE {quoted}")),
                "{}: {text}",
                t.id
            );
        }
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn mysql_enum_column_labels_parsed_with_doubled_quotes() {
    for t in targets(&MYSQL) {
        let db = Db::create(&t, "enum").await;
        db.run("CREATE TABLE t (e ENUM('a','it''s','İ',''), s SET('x,y','z'))")
            .await;
        let (s, _, _) = pass(&t, &db, Mode::Full, None).await;
        let labels = |col: &str| -> Vec<String> {
            let i = s.find(&db.key(ObjectKind::Column, &["t", col])).unwrap();
            let Props::Column(p) = s.obj(i).unwrap().props() else {
                panic!()
            };
            p.rare
                .as_ref()
                .and_then(|r| r.enum_labels.as_ref())
                .map(|l| l.iter().map(|x| x.to_string()).collect())
                .unwrap_or_default()
        };
        assert_eq!(labels("e"), ["a", "it's", "İ", ""], "{}", t.id);
        assert_eq!(labels("s"), ["x,y", "z"], "{}", t.id);
    }
}

// ---- every server ----------------------------------------------------------------------------

/// F1: an `ALTER … ALGORITHM=COPY` on 400k rows with a pass during it; MySQL stamps the start,
/// MariaDB the end. The next pass re-reads the table on every server.
#[tokio::test]
#[ignore = "docker"]
async fn long_alter_straddling_a_pass_is_seen() {
    for t in targets(&ALL) {
        let db = Db::create(&t, "longalter").await;
        db.run("CREATE TABLE big (id INT PRIMARY KEY AUTO_INCREMENT, v INT)")
            .await;
        db.run("INSERT INTO big (v) VALUES (1),(2),(3),(4),(5),(6),(7),(8)")
            .await;
        for _ in 0..16 {
            db.run("INSERT INTO big (v) SELECT v FROM big").await;
            let n = harness::rows(&t, Some(&db.name), "SELECT COUNT(*) FROM big").await;
            if n[0][0].parse::<u64>().unwrap_or(0) >= 400_000 {
                break;
            }
        }
        let (s0, _, _) = pass(&t, &db, Mode::Full, None).await;
        let alter = {
            let t = t.clone();
            let name = db.name.clone();
            tokio::spawn(async move {
                harness::exec(
                    &t,
                    Login::Test,
                    Some(&name),
                    "ALTER TABLE big ADD COLUMN w INT, ALGORITHM=COPY",
                )
                .await
            })
        };
        // A pass while the ALTER runs (its marker may already be the start time).
        let (mid, _, _) = pass(&t, &db, Mode::Incremental, Some(&s0)).await;
        alter.await.expect("joined").expect("the ALTER");
        let (s1, stats) = incremental_equals_full(&t, &db, &mid).await;
        assert!(
            reread(&stats).contains(&"big") || stats.full_reason.is_some(),
            "{}",
            t.id
        );
        assert!(
            s1.find(&db.key(ObjectKind::Column, &["big", "w"]))
                .is_some(),
            "{}",
            t.id
        );
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn flavor_differences_are_kept_verbatim() {
    for t in targets(&ALL) {
        let maria = t.engine == DbmsKind::MariaDb;
        let db = Db::create(&t, "flavor").await;
        db.run("CREATE TABLE p (id INT PRIMARY KEY)").await;
        db.run(
            "CREATE TABLE f (id INT, code VARCHAR(8) DEFAULT 'eu', \
             at TIMESTAMP DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP, \
             CONSTRAINT ck CHECK (id > 0), \
             CONSTRAINT fk FOREIGN KEY (id) REFERENCES p (id) ON DELETE CASCADE)",
        )
        .await;
        if maria {
            db.run("CREATE SEQUENCE s").await;
        }
        let (s, _, _) = pass(&t, &db, Mode::Full, None).await;
        let col = |c: &str| {
            let i = s.find(&db.key(ObjectKind::Column, &["f", c])).unwrap();
            match s.obj(i).unwrap().props() {
                Props::Column(p) => p.clone(),
                _ => panic!(),
            }
        };
        // Pre-quoted on both: MySQL reports `eu`, MariaDB `'eu'` (F9).
        assert_eq!(col("code").default_raw.as_deref(), Some("'eu'"), "{}", t.id);
        let at = col("at");
        let (default, on_update) = if maria {
            ("current_timestamp()", "on update current_timestamp()")
        } else {
            ("CURRENT_TIMESTAMP", "on update CURRENT_TIMESTAMP")
        };
        assert_eq!(at.default_raw.as_deref(), Some(default), "{}", t.id);
        assert!(
            matches!(
                at.ext.as_deref(),
                Some(ColumnExt::MySql { extra: Some(e), .. }) if e.contains(on_update)
            ),
            "{}",
            t.id
        );
        assert_eq!(
            &*col("id").raw_type,
            if maria { "int(11)" } else { "int" },
            "{}",
            t.id
        );
        let fk = s
            .find(&db.key(ObjectKind::ForeignKey, &["f", "fk"]))
            .unwrap();
        let Props::ForeignKey(fp) = s.obj(fk).unwrap().props() else {
            panic!()
        };
        let unspecified = if maria {
            FkAction::Restrict
        } else {
            FkAction::NoAction
        };
        assert_eq!(fp.on_update, unspecified, "{}", t.id);
        assert_eq!(fp.on_delete, FkAction::Cascade, "{}", t.id);
        let ck = s.find(&db.key(ObjectKind::Check, &["f", "ck"])).unwrap();
        let Props::Check(cp) = s.obj(ck).unwrap().props() else {
            panic!()
        };
        let clause = if maria { "`id` > 0" } else { "(`id` > 0)" };
        assert_eq!(&*cp.expr, clause, "{}", t.id);
        if maria {
            assert!(
                s.find(&db.key(ObjectKind::Sequence, &["s"])).is_some(),
                "{}",
                t.id
            );
        }
    }
}

/// The FK-created index (HV-INTRO-13 / T13 carry): recorded in `t17-fk-auto-index.tsv`;
/// `FkIndexRule` stays `None` until the controller rules on the record, so the index is shown
/// (never hidden) and DDL stays valid.
#[tokio::test]
#[ignore = "docker"]
async fn mysql_fk_auto_index_is_pinned() {
    let mut tsv = String::from("target\tfk\tindex\tcolumns\n");
    for t in targets(&ALL) {
        let db = Db::create(&t, "fkindex").await;
        db.run("CREATE TABLE p (id INT PRIMARY KEY)").await;
        db.run(
            "CREATE TABLE c1 (pid INT, CONSTRAINT fk_named FOREIGN KEY (pid) REFERENCES p (id))",
        )
        .await;
        db.run("CREATE TABLE c2 (pid INT, FOREIGN KEY (pid) REFERENCES p (id))")
            .await;
        let rows = harness::rows(
            &t,
            Some(&db.name),
            "SELECT TABLE_NAME, INDEX_NAME, COLUMN_NAME FROM INFORMATION_SCHEMA.STATISTICS \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME IN ('c1', 'c2') \
             ORDER BY TABLE_NAME, INDEX_NAME, SEQ_IN_INDEX",
        )
        .await;
        for r in &rows {
            let _ = writeln!(tsv, "{}\t{}\t{}\t{}", t.id, r[0], r[1], r[2]);
        }
        // MySQL 8.4 manual, FOREIGN KEY constraints: the implicit index takes the constraint's
        // name when one is given.
        assert!(
            rows.iter()
                .any(|r| r[0] == "c1" && r[1] == "fk_named" && r[2] == "pid"),
            "{}: {rows:?}",
            t.id
        );
        let (s, _, _) = pass(&t, &db, Mode::Full, None).await;
        let ix = s
            .find(&db.key(ObjectKind::Index, &["c1", "fk_named"]))
            .unwrap();
        assert!(
            !s.obj(ix).unwrap().flags().contains(ObjFlags::IMPLICIT),
            "{}",
            t.id
        );
    }
    if docker_enabled() {
        std::fs::write(hacivat_testkit::artifact("t17-fk-auto-index.tsv"), tsv)
            .expect("the artifact");
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn server_facts_filled_per_engine() {
    for t in targets(&ALL) {
        let db = Db::create(&t, "facts").await;
        let mut c = harness::meta(&t, &db.name).await;
        let (srv, facts) = intro(&t).server_facts(&mut c).await.unwrap();
        assert_eq!(facts.dbms, t.engine, "{}", t.id);
        assert!(facts.version_num >= 80_000, "{}", t.id);
        assert_eq!(
            facts.ident.lower_case_table_names, 0,
            "{}: Linux images",
            t.id
        );
        assert!(
            srv.utf8mb4 && srv.utc_session,
            "{}: Task 5's session set-up",
            t.id
        );
        assert_eq!(
            srv.server_uuid.is_some(),
            t.engine == DbmsKind::MySql,
            "{}",
            t.id
        );
        assert!(facts.default_collation.contains_key(&*db.name), "{}", t.id);
        assert_eq!(facts.no_backslash_escapes, Some(false), "{}", t.id);
        // R-W3-3: the identity reads on every server and is stable across connections.
        let a = intro(&t).server_fingerprint(&mut c).await.unwrap();
        let mut c2 = harness::meta(&t, &db.name).await;
        let b = intro(&t).server_fingerprint(&mut c2).await.unwrap();
        assert!(a.is_some() && a == b, "{}", t.id);
    }
}

// ---- MariaDB ----------------------------------------------------------------------------------

#[tokio::test]
#[ignore = "docker"]
async fn mariadb_full_per_namespace_after_each_alter() {
    for t in targets(&MARIADB) {
        let db = Db::create(&t, "mfull").await;
        base_schema(&db).await;
        let (mut s, res, _) = pass(&t, &db, Mode::Full, None).await;
        assert_eq!(res.new_watermark, hacivat_meta::Watermark::None, "{}", t.id);
        for ddl in [
            "ALTER TABLE customers ADD COLUMN email VARCHAR(64)",
            "CREATE INDEX ix ON orders (status)",
            "CREATE TRIGGER trg BEFORE INSERT ON orders FOR EACH ROW SET NEW.id = NEW.id",
            "DROP VIEW v_orders",
        ] {
            db.run(ddl).await;
            let (next, res, stats) = pass(&t, &db, Mode::Incremental, Some(&s)).await;
            assert!(res.full, "{}: {ddl}", t.id);
            assert_eq!(stats.full_reason, Some("flavor"), "{}", t.id);
            let (full, _, _) = pass(&t, &db, Mode::Full, None).await;
            assert_eq!(canon(&next), canon(&full), "{}: {ddl}", t.id);
            s = next;
        }
    }
}

/// R-M4-10: the MariaDB `CREATE_TIME` of a table after each ver/01 ALTER, recorded for v1.x
/// (non-gating artifact `t17-mariadb-create-time.tsv`).
#[tokio::test]
#[ignore = "docker"]
async fn record_mariadb_create_time_behaviour() {
    let mut tsv = String::from("target\tstep\tcreate_time_changed\n");
    for t in targets(&MARIADB) {
        let db = Db::create(&t, "ctime").await;
        db.run("CREATE TABLE m (id INT PRIMARY KEY, a INT)").await;
        let ctime = || async {
            harness::rows(
                &t,
                Some(&db.name),
                "SELECT CAST(CREATE_TIME AS CHAR) FROM INFORMATION_SCHEMA.TABLES \
                 WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'm'",
            )
            .await[0][0]
                .clone()
        };
        for (step, ddl) in [
            ("insert", "INSERT INTO m VALUES (1, 1)"),
            ("add column", "ALTER TABLE m ADD COLUMN b INT"),
            ("add index", "CREATE INDEX ix ON m (a)"),
            ("comment", "ALTER TABLE m COMMENT = 'x'"),
            ("modify type", "ALTER TABLE m MODIFY a BIGINT"),
            ("rename column", "ALTER TABLE m RENAME COLUMN b TO c"),
            ("set default", "ALTER TABLE m ALTER COLUMN a SET DEFAULT 1"),
            ("add check", "ALTER TABLE m ADD CONSTRAINT ck CHECK (a > 0)"),
            (
                "create trigger",
                "CREATE TRIGGER tg BEFORE INSERT ON m FOR EACH ROW SET NEW.a = 1",
            ),
            ("truncate", "TRUNCATE TABLE m"),
        ] {
            let before = ctime().await;
            next_server_second(&t).await;
            db.run(ddl).await;
            let after = ctime().await;
            let _ = writeln!(tsv, "{}\t{step}\t{}", t.id, before != after);
        }
    }
    if docker_enabled() {
        std::fs::write(
            hacivat_testkit::artifact("t17-mariadb-create-time.tsv"),
            tsv,
        )
        .expect("the artifact");
    }
}

/// HV-INTRO-01 edge 4 on a real server: rows read while another session creates a table may
/// name a table the TABLES query did not list; they are skipped, and the next pass is whole.
#[tokio::test]
#[ignore = "docker"]
async fn mariadb_rows_without_a_listed_parent_are_skipped() {
    for t in targets(&MARIADB) {
        let db = Db::create(&t, "racing").await;
        base_schema(&db).await;
        let creator = {
            let t = t.clone();
            let name = db.name.clone();
            tokio::spawn(async move {
                for i in 0..50 {
                    let _ = harness::exec(
                        &t,
                        Login::Test,
                        Some(&name),
                        &format!("CREATE TABLE r{i} (id INT, KEY k (id))"),
                    )
                    .await;
                }
            })
        };
        // Passes while tables appear: each must build (no child without its relation).
        let mut s: Option<NamespaceSnapshot> = None;
        for _ in 0..5 {
            let (next, _, _) = pass(&t, &db, Mode::Full, s.as_ref()).await;
            s = Some(next);
        }
        creator.await.expect("joined");
        let (last, _, _) = pass(&t, &db, Mode::Full, s.as_ref()).await;
        let (fresh, _, _) = pass(&t, &db, Mode::Full, None).await;
        assert_eq!(canon(&last), canon(&fresh), "{}", t.id);
    }
}
