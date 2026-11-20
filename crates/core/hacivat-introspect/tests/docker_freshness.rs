// ## m4-t22-freshness
//! Freshness on real servers (M4 Task 22; HV-INTRO-15): state checks compared by equality with
//! the stored marks, and the cheap validation of a cached namespace.
//!
//! Run: `<main>/target/docker/run-docker-tests.sh --set pr -p hacivat-introspect --test
//! docker_freshness` (targets `pg17`, `mysql84`; the full set adds `pg14`, `pg18`, `mysql80`).
//! Every test is `#[ignore = "docker"]`, returns early unless `HACIVAT_DOCKER=1`, takes its
//! targets only from `require_target`, and creates its own `hvm4_t22_*` schema (PostgreSQL) or
//! database (MySQL), dropped afterwards, also when the test panics. Server facts are printed as
//! `HV-FACT <target> <key>=<value>` lines (CI uploads no artifact directory, ruling R-W3-14).
//!
//! The catalog is the hub's (the real engine and introspector); state checks are also called
//! directly on a metadata connection, so a test sees exactly what `check_state` answered.

#![cfg(feature = "session")]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::pg::{Pg, admin, unique};
use hacivat_introspect::freshness::{self, NsVerdict};
use hacivat_introspect::hub::{CatalogHub, HubSettings, ProfileCatalog, SessionLookup};
use hacivat_introspect::{Introspector, Staleness, introspector_for};
use hacivat_meta::snapshot::{ContainerKey, LoadState};
use hacivat_meta::{NsKey, ObjectKey, ObjectKind};
use hacivat_proto::DbmsKind;
use hacivat_secrets::SecretStore;
use hacivat_settings::{LocalPaths, MetadataSettings};
use hacivat_testkit::{DockerTarget, docker_enabled, require_target};
use tokio_util::sync::CancellationToken;

const LIMIT: Duration = Duration::from_secs(120);
const PG: [&str; 3] = ["pg17", "pg14", "pg18"];
const MYSQL: [&str; 2] = ["mysql84", "mysql80"];

fn targets(ids: &[&str]) -> Vec<DockerTarget> {
    if !docker_enabled() {
        return Vec::new();
    }
    ids.iter().filter_map(|id| require_target(id)).collect()
}

/// One server fact on the CI log (see the module docs). Values are server facts only.
fn fact(t: &DockerTarget, kv: &str) {
    use std::io::Write as _;
    let line = format!("HV-FACT {} {}\n", t.id, kv.replace('\n', " "));
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(line.as_bytes());
    let _ = out.flush();
}

/// A connected profile of `t`, the hub's catalog of it, and its introspector.
struct Live {
    t: DockerTarget,
    pg: Pg,
    cat: ProfileCatalog,
    intro: Arc<dyn Introspector>,
    _hub: Arc<CatalogHub>,
    _dir: tempfile::TempDir,
}

async fn live(t: &DockerTarget) -> Live {
    let pg = Pg::connect(t).await;
    let dir = tempfile::tempdir().expect("a temp dir");
    let lookup: Arc<dyn SessionLookup> = pg.m.clone();
    let hub = CatalogHub::new(
        lookup,
        HubSettings {
            paths: LocalPaths::under(dir.path()),
            metadata: MetadataSettings::default(),
        },
        Arc::new(SecretStore::new(None)),
        Arc::new(hacivat_meta::system::SystemCatalogs::empty()),
    );
    let cat = hub.ensure(pg.profile.clone()).await.expect("a catalog");
    tokio::time::timeout(LIMIT, cat.handle.on_connect())
        .await
        .expect("on connect in time")
        .expect("the server is read");
    Live {
        t: t.clone(),
        intro: introspector_for(t.engine).expect("an M4 engine"),
        pg,
        cat,
        _hub: hub,
        _dir: dir,
    }
}

impl Live {
    /// Fixture DDL on its own writable connection (never the metadata connection).
    async fn ddl(&self, sql: &str) {
        let mut a = admin(&self.t).await;
        a.batch(sql).await;
        a.close().await;
    }

    async fn idle(&self) {
        let mut rx = self.cat.handle.progress();
        tokio::time::timeout(LIMIT, rx.wait_for(|p| p.tasks.is_empty()))
            .await
            .expect("idle in time")
            .expect("the engine runs");
    }

    /// Loads `ns` and waits until its full read has published and the engine is idle.
    ///
    /// Fix round 3 (CI 44667350956): an expansion's ticket resolves after the names-first
    /// pass; the full pass at the namespace's level follows, queued. Waiting for "idle" alone
    /// could see the gap between the two and let the test's DDL run before the full pass,
    /// which then read the post-DDL server (both `Current` failures). Wait for the level.
    async fn expand(&self, ns: &NsKey) {
        tokio::time::timeout(LIMIT, self.cat.handle.expand(ns.clone()))
            .await
            .expect("loaded in time")
            .expect("loaded");
        let start = tokio::time::Instant::now();
        loop {
            let st = self
                .cat
                .catalog
                .current()
                .state(&ContainerKey::Namespace(ns.clone()))
                .clone();
            if matches!(st, LoadState::Loaded { level, .. } if level >= hacivat_meta::Level::Structure)
            {
                break;
            }
            assert!(
                start.elapsed() < LIMIT,
                "{}: {ns:?} stays {st:?}",
                self.t.id
            );
            let mut rx = self.cat.catalog.subscribe();
            let _ = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await;
        }
        self.idle().await;
    }

    /// The key (with its server id) of the top-level object `name` of `kind` in `ns`.
    fn key(&self, ns: &NsKey, kind: ObjectKind, name: &str) -> ObjectKey {
        let snap = self.cat.catalog.current();
        let a = snap.namespaces.get(ns).expect("loaded");
        a.children(0)
            .find(|(_, o)| o.kind() == kind && &**o.name() == name)
            .and_then(|(i, _)| a.object_key(i))
            .unwrap_or_else(|| panic!("{}: {kind:?} {name} is loaded", self.t.id))
    }

    /// `check_state` of `keys`, called directly on a metadata connection of `ns`'s database.
    async fn check(&self, keys: &[ObjectKey]) -> Vec<Staleness> {
        let db = (self.t.engine == DbmsKind::Postgres).then_some(self.t.database.as_str());
        let mut c = self.pg.meta_on(db).await;
        freshness::check_state(&*self.intro, &mut c, &self.cat.catalog.current(), keys)
            .await
            .unwrap_or_else(|e| panic!("{}: check_state: {e:?}", self.t.id))
            .staleness
    }

    /// The engine's `check_state` (re-reads what changed and stores the stamps as marks).
    async fn engine_check(&self, keys: Vec<ObjectKey>) {
        tokio::time::timeout(LIMIT, self.cat.handle.check_state(keys))
            .await
            .expect("checked in time")
            .expect("checked");
        self.idle().await;
    }

    /// The engine's check, then a direct one that must read `Current` everywhere: every
    /// object's stamp is now its stored mark (PostgreSQL passes store none; the first check
    /// re-reads and records them).
    async fn prime(&self, keys: &[ObjectKey]) {
        self.engine_check(keys.to_vec()).await;
        let refreshed: Vec<ObjectKey> = keys
            .iter()
            .map(|k| {
                let ns = NsKey::schema(k.path[..k.path.len() - 1].iter().cloned());
                self.key(&ns, k.kind, k.name())
            })
            .collect();
        assert_eq!(
            self.check(&refreshed).await,
            vec![Staleness::Current; keys.len()],
            "{}: stamps equal the stored marks once recorded",
            self.t.id
        );
    }

    async fn shutdown(self) {
        self.cat.handle.shutdown();
        self.pg.shutdown().await;
    }
}

/// The PostgreSQL tests of this file run one at a time (fix round 1 I-7): a concurrent test's
/// open transaction would hold the snapshot xmin back across another test's pass.
static PG_TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Runs `sql` on `t`'s admin connection when it goes, also on panic or timeout (review m-6).
struct DropOnExit {
    t: DockerTarget,
    sql: String,
}

impl Drop for DropOnExit {
    fn drop(&mut self) {
        let (t, sql) = (self.t.clone(), self.sql.clone());
        let rt = tokio::runtime::Handle::current();
        std::thread::scope(|s| {
            s.spawn(|| {
                rt.block_on(async {
                    let mut a = admin(&t).await;
                    a.batch(&sql).await;
                    a.close().await;
                });
            });
        });
    }
}

/// Runs `body` on `t` with a fresh `hvm4_t22_<slug>` schema (PostgreSQL) or database (MySQL),
/// dropped afterwards by a guard (also on panic). PostgreSQL tests hold [`PG_TESTS`].
async fn with_ns(t: &DockerTarget, slug: &str, body: impl AsyncFnOnce(&Live, &NsKey, &str)) {
    let _serial = match t.engine {
        DbmsKind::Postgres => Some(PG_TESTS.lock().await),
        _ => None,
    };
    let name = unique(&format!("hvm4_t22_{slug}"));
    let (create, drop) = match t.engine {
        DbmsKind::Postgres => (
            format!("CREATE SCHEMA {name}"),
            format!("DROP SCHEMA IF EXISTS {name} CASCADE"),
        ),
        _ => (
            format!("CREATE DATABASE `{name}`"),
            format!("DROP DATABASE IF EXISTS `{name}`"),
        ),
    };
    let ns = match t.engine {
        DbmsKind::Postgres => NsKey::schema([t.database.as_str(), name.as_str()]),
        _ => NsKey::schema([name.as_str()]),
    };
    {
        let mut a = admin(t).await;
        a.batch(&create).await;
        a.close().await;
    }
    let _drop = DropOnExit {
        t: t.clone(),
        sql: drop,
    };
    let l = live(t).await;
    body(&l, &ns, &name).await;
    l.shutdown().await;
}

/// Waits (bounded by `LIMIT`, polling) until no transaction of the server holds an XID, so the
/// next pass's watermark is not held back by another session (fix round 1 I-7).
async fn no_open_xids(t: &DockerTarget) {
    let mut a = admin(t).await;
    let start = tokio::time::Instant::now();
    loop {
        let none = a
            .scalar(
                "SELECT NOT EXISTS (SELECT 1 FROM pg_catalog.pg_snapshot_xip(\
                 pg_catalog.pg_current_snapshot()))",
            )
            .await;
        if matches!(none.as_str(), "true" | "t") {
            break;
        }
        assert!(start.elapsed() < LIMIT, "{}: an XID stayed open", t.id);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    a.close().await;
}

/// The rows the section `name` returned in `report` (0 when it did not run).
fn rows_of(report: &hacivat_introspect::pg::pass::PassReport, name: &str) -> usize {
    report
        .log
        .sections
        .iter()
        .filter(|s| s.name == name)
        .map(|s| s.rows)
        .sum()
}

/// The schema-qualified table of a test namespace (PostgreSQL schema or MySQL database).
fn q(t: &DockerTarget, ns: &str, object: &str) -> String {
    match t.engine {
        DbmsKind::Postgres => format!("{ns}.{object}"),
        _ => format!("`{ns}`.`{object}`"),
    }
}

// ---- PostgreSQL ------------------------------------------------------------------------------

/// A comment is part of an object's state: changing only the comment moves the stamp
/// (`pg_description`'s xmin is one of its rows).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn pg_comment_only_change_is_changed() {
    for t in targets(&PG) {
        with_ns(&t, "cmt", async |l, ns, s| {
            l.ddl(&format!("CREATE TABLE {s}.t (a int)")).await;
            l.expand(ns).await;
            let k = l.key(ns, ObjectKind::Table, "t");
            l.prime(std::slice::from_ref(&k)).await;
            l.ddl(&format!("COMMENT ON TABLE {s}.t IS 'only the comment'"))
                .await;
            let k = l.key(ns, ObjectKind::Table, "t");
            assert_eq!(
                l.check(std::slice::from_ref(&k)).await,
                vec![Staleness::Changed],
                "{}",
                t.id
            );
            // A column comment too.
            l.engine_check(vec![k.clone()]).await;
            l.ddl(&format!("COMMENT ON COLUMN {s}.t.a IS 'col'")).await;
            let k = l.key(ns, ObjectKind::Table, "t");
            assert_eq!(l.check(&[k]).await, vec![Staleness::Changed], "{}", t.id);
            fact(&t, "pg_comment_only_change=changed");
        })
        .await;
    }
}

/// Removing a comment deletes its `pg_description` row: no young row is left, but the stamp
/// (a hash of the object's rows) moves.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn pg_comment_removal_is_changed() {
    for t in targets(&PG) {
        with_ns(&t, "cmtrm", async |l, ns, s| {
            l.ddl(&format!(
                "CREATE TABLE {s}.t (a int); COMMENT ON TABLE {s}.t IS 'x'; \
                 COMMENT ON COLUMN {s}.t.a IS 'y'"
            ))
            .await;
            l.expand(ns).await;
            let k = l.key(ns, ObjectKind::Table, "t");
            l.prime(std::slice::from_ref(&k)).await;
            l.ddl(&format!("COMMENT ON COLUMN {s}.t.a IS NULL")).await;
            let k = l.key(ns, ObjectKind::Table, "t");
            assert_eq!(l.check(&[k]).await, vec![Staleness::Changed], "{}", t.id);
            fact(&t, "pg_comment_removal=changed");
        })
        .await;
    }
}

/// `DROP TABLE t; CREATE TABLE t`: the cached object (its oid) is `Dropped`, and the engine's
/// re-read brings the new one (another oid).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn pg_drop_and_recreate_is_dropped_plus_new() {
    for t in targets(&PG) {
        with_ns(&t, "recreate", async |l, ns, s| {
            l.ddl(&format!("CREATE TABLE {s}.t (a int)")).await;
            l.expand(ns).await;
            let old = l.key(ns, ObjectKind::Table, "t");
            l.ddl(&format!("DROP TABLE {s}.t; CREATE TABLE {s}.t (b text)"))
                .await;
            assert_eq!(
                l.check(std::slice::from_ref(&old)).await,
                vec![Staleness::Dropped],
                "{}",
                t.id
            );
            l.engine_check(vec![old.clone()]).await;
            // The scoped re-read removed the old object; the next pass brings the new one.
            tokio::time::timeout(
                LIMIT,
                l.cat.handle.refresh(
                    hacivat_introspect::ddl_effects::Target::Namespace(ns.clone()),
                    hacivat_meta::merge::Trigger::User,
                ),
            )
            .await
            .unwrap()
            .unwrap();
            let new = l.key(ns, ObjectKind::Table, "t");
            assert_ne!(old.server_id, new.server_id, "{}: a new oid", t.id);
            fact(&t, "pg_drop_and_recreate=dropped+new");
        })
        .await;
    }
}

/// PG spike finding 4 / §7 Q2 (R-M4-48): a view over a column that is renamed reads `Changed`
/// (its deparsed text names the column; the stamp covers one `pg_depend` hop).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn check_state_sees_a_renamed_dependency() {
    for t in targets(&PG) {
        with_ns(&t, "dep", async |l, ns, s| {
            l.ddl(&format!(
                "CREATE TABLE {s}.t (a int); CREATE VIEW {s}.v AS SELECT a FROM {s}.t"
            ))
            .await;
            l.expand(ns).await;
            let v = l.key(ns, ObjectKind::View, "v");
            l.prime(std::slice::from_ref(&v)).await;
            l.ddl(&format!("ALTER TABLE {s}.t RENAME COLUMN a TO b"))
                .await;
            let v = l.key(ns, ObjectKind::View, "v");
            assert_eq!(l.check(&[v]).await, vec![Staleness::Changed], "{}", t.id);
            fact(&t, "pg_renamed_dependency=changed");
        })
        .await;
    }
}

// ---- MySQL -------------------------------------------------------------------------------------

/// ver/01 `#mysql-is-times`: `CREATE TRIGGER` leaves `CREATE_TIME` alone, but the table's
/// R-M4-39 stamp counts its triggers: the table reads `Changed`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn mysql_trigger_creation_makes_the_table_changed() {
    for t in targets(&MYSQL) {
        with_ns(&t, "trg", async |l, ns, s| {
            l.ddl(&format!(
                "CREATE TABLE {} (a int, status varchar(10))",
                q(&t, s, "orders")
            ))
            .await;
            l.expand(ns).await;
            let k = l.key(ns, ObjectKind::Table, "orders");
            assert_eq!(
                l.check(std::slice::from_ref(&k)).await,
                vec![Staleness::Current],
                "{}",
                t.id
            );
            l.ddl(&format!(
                "CREATE TRIGGER {} BEFORE INSERT ON {} FOR EACH ROW SET NEW.status = 'new'",
                q(&t, s, "trg_o"),
                q(&t, s, "orders")
            ))
            .await;
            assert_eq!(l.check(&[k]).await, vec![Staleness::Changed], "{}", t.id);
            fact(&t, "mysql_trigger_creation=changed");
        })
        .await;
    }
}

// ---- both ----------------------------------------------------------------------------------------

/// HV-INTRO-15: a cached namespace the server did not change validates `Current` with state
/// queries only (no object row is read), and the engine clears its stale mark on connect.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn unchanged_server_clears_stale_with_state_queries_only() {
    let ids: Vec<&str> = PG.iter().chain(MYSQL.iter()).copied().collect();
    for t in targets(&ids) {
        with_ns(&t, "same", async |l, ns, s| {
            l.ddl(&format!("CREATE TABLE {} (a int)", q(&t, s, "t")))
                .await;
            l.ddl(&format!(
                "CREATE VIEW {} AS SELECT a FROM {}",
                q(&t, s, "v"),
                q(&t, s, "t")
            ))
            .await;
            // I-7: no other session's XID may hold the pass's watermark back.
            if t.engine == DbmsKind::Postgres {
                no_open_xids(&t).await;
            }
            l.expand(ns).await;
            if t.engine != DbmsKind::Postgres {
                // R-M4-39: the fixture's DDL is inside the one-second window of the first
                // pass, which the cached side never vouches for; refresh (a condition wait on
                // the server's clock) until the arena's watermark is past it.
                let start = tokio::time::Instant::now();
                loop {
                    let arena = l.cat.catalog.current().namespaces[ns].clone();
                    if l.intro
                        .cached_check(&arena)
                        .is_some_and(|c| c.max_mark != hacivat_introspect::mysql::state::IN_WINDOW)
                    {
                        break;
                    }
                    assert!(start.elapsed() < LIMIT, "{}: the window never passed", t.id);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    tokio::time::timeout(
                        LIMIT,
                        l.cat.handle.refresh(
                            hacivat_introspect::ddl_effects::Target::Namespace(ns.clone()),
                            hacivat_meta::merge::Trigger::User,
                        ),
                    )
                    .await
                    .unwrap()
                    .unwrap();
                }
            }
            let arena = l.cat.catalog.current().namespaces[ns].clone();
            let db = (t.engine == DbmsKind::Postgres).then_some(t.database.as_str());
            let mut c = l.pg.meta_on(db).await;
            if t.engine == DbmsKind::Postgres {
                let (server, report) = hacivat_introspect::pg::state::check_namespace(
                    &mut c,
                    ns,
                    &arena,
                    &CancellationToken::new(),
                )
                .await
                .expect("the check");
                let cached = hacivat_introspect::pg::state::cached_check(&arena).expect("cached");
                assert_eq!(
                    freshness::verdict(&cached, &server),
                    NsVerdict::Current,
                    "{}",
                    t.id
                );
                let ran = report.log.names();
                fact(&t, &format!("pg_validation_sections={}", ran.join(",")));
                // The precondition made an assertion: nothing was young.
                assert_eq!(rows_of(&report, "ChangedParents"), 0, "{}", t.id);
                assert_eq!(rows_of(&report, "ChangedDependents"), 0, "{}", t.id);
                for s in &ran {
                    assert!(
                        [
                            "Identity",
                            "ListExistent",
                            "ListExistentComments",
                            "ChangedParents",
                            "ChangedDependents",
                            "Canary"
                        ]
                        .contains(s),
                        "{}: {s} reads object rows",
                        t.id
                    );
                }
            } else {
                assert_eq!(
                    freshness::check_namespace(&*l.intro, &mut c, ns, &arena)
                        .await
                        .expect("the check"),
                    NsVerdict::Current,
                    "{}",
                    t.id
                );
            }
            drop(c);
            // Through the engine: stale, then validated on connect.
            l.cat.handle.mark_stale(vec![ns.clone()], false);
            l.idle().await;
            tokio::time::timeout(LIMIT, l.cat.handle.on_connect())
                .await
                .unwrap()
                .unwrap();
            let start = tokio::time::Instant::now();
            loop {
                let st = l
                    .cat
                    .catalog
                    .current()
                    .state(&ContainerKey::Namespace(ns.clone()))
                    .clone();
                if matches!(st, LoadState::Loaded { stale: false, .. }) {
                    break;
                }
                assert!(start.elapsed() < LIMIT, "{}: still {st:?}", t.id);
                let mut rx = l.cat.catalog.subscribe();
                let _ = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await;
            }
        })
        .await;
    }
}

// ---- fix round 1 --------------------------------------------------------------------------------

/// C-1: an older open transaction (an XID assigned before the pass) holds the snapshot xmin
/// back across a later `ALTER TABLE`; the namespace must not validate `Current`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn pg_a_held_back_xmin_never_validates_a_change_current() {
    for t in targets(&PG) {
        with_ns(&t, "heldxmin", async |l, ns, s| {
            l.ddl(&format!("CREATE TABLE {s}.t (a int)")).await;
            // The holder: an XID assigned and kept open (closing the connection aborts it,
            // also on panic).
            let mut side = admin(&t).await;
            side.batch("BEGIN; SELECT pg_catalog.pg_current_xact_id();")
                .await;
            l.expand(ns).await;
            l.ddl(&format!("ALTER TABLE {s}.t ADD COLUMN b int")).await;
            let arena = l.cat.catalog.current().namespaces[ns].clone();
            let mut c = l.pg.meta_on(Some(t.database.as_str())).await;
            let (server, report) = hacivat_introspect::pg::state::check_namespace(
                &mut c,
                ns,
                &arena,
                &CancellationToken::new(),
            )
            .await
            .expect("the check");
            let cached = hacivat_introspect::pg::state::cached_check(&arena).expect("cached");
            fact(
                &t,
                &format!(
                    "pg_held_xmin changed_parents_rows={} snap_xmin_held={}",
                    rows_of(&report, "ChangedParents"),
                    report.snap_xmin
                ),
            );
            assert!(rows_of(&report, "ChangedParents") > 0, "{}", t.id);
            assert_ne!(
                freshness::verdict(&cached, &server),
                NsVerdict::Current,
                "{}: a changed namespace validated Current under a held-back xmin",
                t.id
            );
            drop(c);
            side.batch("COMMIT").await;
            side.close().await;
        })
        .await;
    }
}

/// I-1: a schema holding a collation the namespace's column uses is renamed while another
/// object of the namespace changes too: the incremental pass reads the namespace in full
/// (`ChangedDependents`' schema-rename branch follows referenced collations).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn pg_rename_of_a_schema_holding_a_used_collation_reads_in_full() {
    for t in targets(&PG) {
        let other = unique("hvm4_t22_coll");
        let renamed = format!("{other}_r");
        let _drop = DropOnExit {
            t: t.clone(),
            sql: format!(
                "DROP SCHEMA IF EXISTS {other} CASCADE; DROP SCHEMA IF EXISTS {renamed} CASCADE"
            ),
        };
        with_ns(&t, "collren", async |l, ns, s| {
            l.ddl(&format!(
                "CREATE SCHEMA {other}; CREATE COLLATION {other}.c FROM \"C\"; \
                 CREATE TABLE {s}.t (a text COLLATE {other}.c)"
            ))
            .await;
            no_open_xids(&t).await;
            l.expand(ns).await;
            l.ddl(&format!(
                "ALTER SCHEMA {other} RENAME TO {renamed}; CREATE TABLE {s}.u (x int)"
            ))
            .await;
            let arena = l.cat.catalog.current().namespaces[ns].clone();
            let mut c = l.pg.meta_on(Some(t.database.as_str())).await;
            let p = hacivat_introspect::pg::pass::introspect(
                &mut c,
                ns,
                hacivat_introspect::pg::pass::PassOpts {
                    mode: hacivat_introspect::Mode::Incremental,
                    level: arena.meta().level,
                    prev: Some(&arena),
                    ..Default::default()
                },
                &CancellationToken::new(),
            )
            .await
            .expect("the pass");
            fact(
                &t,
                &format!(
                    "pg_collation_schema_rename full_reason={:?}",
                    p.report.full_reason
                ),
            );
            assert!(
                p.result.full,
                "{}: an incremental pass missed the rename",
                t.id
            );
        })
        .await;
    }
}
