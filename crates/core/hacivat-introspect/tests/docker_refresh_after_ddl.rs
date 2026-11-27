// ## m4-t21-refresh
//! Refresh after DDL on real servers (M4 Task 21; alg/02 HV-INTRO-12 test cases 6–9, the
//! explorer side; ver/01 `#mysql-ro`). A user session runs the statements; each statement's
//! transaction status before and after, read from the session, drives the run tracker exactly as
//! a run's `TxChanged`/`UnitEnded` events do; the profile's coordinator refreshes the hub's
//! catalog through the engine and the engine's real introspector.
//!
//! Run: `<main>/target/docker/run-docker-tests.sh --set pr -p hacivat-introspect --test
//! docker_refresh_after_ddl` (targets `pg17`, `mysql84`; PR set). Every test is
//! `#[ignore = "docker"]`, returns early unless `HACIVAT_DOCKER=1`, takes its targets only from
//! `require_target`, and creates its own `hvm4_t21_*` schema (PostgreSQL) or database (MySQL),
//! dropped afterwards, also when the test panics.
//!
//! These tests need the real PostgreSQL and MySQL introspectors (Tasks 16 and 17): before those
//! merge, `introspector_for` answers `NotImplemented` and the catalog never shows the objects.

#![cfg(feature = "session")]

use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use hacivat_driver::{EngineCatalog, ExecOpts, TxStatus};
use hacivat_introspect::hub::{CatalogHub, HubSettings, ProfileCatalog, SessionLookup};
use hacivat_introspect::refresh_coord::{RefreshCoordinator, RunTracker, TrackedUnit, TxPhase};
use hacivat_meta::{Catalog, NsKey, ObjectKind};
use hacivat_net::TlsMode;
use hacivat_proto::DbmsKind;
use hacivat_secrets::{
    AuthProviderId, CredentialFiles, MemoryBackend, Retention, SecretBackend, SecretStore,
    SecretString,
};
use hacivat_session::pipeline::{ApprovedRoots, ConnectDeps, Interaction};
use hacivat_session::profile::{Profile, ProfileEndpoint};
use hacivat_session::session::{
    ExecErrorKind, ExecOrigin, Request, RunId, SessionHandle, UnitEnd, UnitId,
};
use hacivat_session::{ConnectionManager, LiveState, ManagerConfig, default_drivers};
use hacivat_settings::{LocalPaths, MetadataSettings, RefreshAfterDdl};
use hacivat_testkit::{DockerTarget, TlsDefault, docker_enabled, require_target};
use sql_dialect::{ClientFlavor, DialectCtx, DialectId};
use sql_safety::{ClassifyCtx, Expect, classify};

const LIMIT: Duration = Duration::from_secs(120);

// ---- harness ---------------------------------------------------------------------------------

fn deps() -> ConnectDeps {
    let backend: Arc<dyn SecretBackend> = Arc::new(MemoryBackend::new());
    let known_hosts = hacivat_net::ssh::KnownHosts::load(
        Vec::new(),
        std::path::PathBuf::from("/nonexistent/hacivat/known_hosts"),
    );
    let tunnels = hacivat_net::ssh::TunnelManager::new(known_hosts.clone(), Default::default());
    ConnectDeps {
        drivers: Arc::new(default_drivers()),
        catalog: EngineCatalog::embedded(),
        secrets: Arc::new(SecretStore::new(Some(backend))),
        files: CredentialFiles {
            pgpass: None,
            pg_service: None,
        },
        transport_wrap: None,
        extra_steps: Vec::new(),
        approved_roots: ApprovedRoots::for_user(),
        timeout_override: None,
        tunnels,
        known_hosts,
    }
}

fn profile_for(t: &DockerTarget) -> Profile {
    let mut p = Profile {
        name: format!("t21 {}", t.id),
        engine: t.engine,
        endpoint: ProfileEndpoint::Server {
            host: t.host.clone(),
            port: Some(t.port),
        },
        database: Some(t.database.clone()),
        user: Some(t.user.clone()),
        auth: AuthProviderId::Password,
        retention: Retention::Never,
        ..Profile::default()
    };
    p.tls.mode = match t.tls_default {
        TlsDefault::SelfSigned => TlsMode::Require,
        TlsDefault::None => TlsMode::Auto,
    };
    p
}

/// A connected profile, its user session, its catalog and its coordinator.
struct Live {
    m: Arc<ConnectionManager>,
    s: SessionHandle,
    cat: ProfileCatalog,
    coord: RefreshCoordinator,
    dbms: DbmsKind,
    _hub: Arc<CatalogHub>,
    _dir: tempfile::TempDir,
}

async fn live(t: &DockerTarget) -> Live {
    let m = ConnectionManager::new(
        tokio::runtime::Handle::current(),
        Arc::new(deps()),
        ManagerConfig::m2b(),
    );
    let profile = Arc::new(profile_for(t));
    m.connect(
        profile.clone(),
        Some(SecretString::from(t.password.clone())),
        Interaction::Headless,
    );
    let mut changes = m.changes();
    tokio::time::timeout(LIMIT, async {
        loop {
            match m.state(profile.id) {
                LiveState::Connected(_) => break,
                LiveState::Failed(e) => panic!("{}: the connect failed: {:?}", t.id, e.class),
                _ => {}
            }
            let _ = changes.changed().await;
        }
    })
    .await
    .expect("connected in time");
    let s = m
        .sessions(profile.id)
        .and_then(|sm| sm.default_session())
        .expect("a default session");
    let dir = tempfile::tempdir().expect("a temp dir");
    let lookup: Arc<dyn SessionLookup> = m.clone();
    let hub = CatalogHub::new(
        lookup,
        HubSettings {
            paths: LocalPaths::under(dir.path()),
            metadata: MetadataSettings::default(),
        },
        Arc::new(SecretStore::new(None)),
        Arc::new(hacivat_meta::system::SystemCatalogs::empty()),
    );
    let cat = hub.ensure(profile).await.expect("a catalog");
    tokio::time::timeout(LIMIT, cat.handle.on_connect())
        .await
        .expect("on connect in time")
        .expect("the introspector reads the server (Tasks 16, 17)");
    let coord = RefreshCoordinator::new(
        cat.handle.clone(),
        t.engine,
        RefreshAfterDdl::Always,
        tokio::runtime::Handle::current(),
    );
    Live {
        m,
        s,
        cat,
        coord,
        dbms: t.engine,
        _hub: hub,
        _dir: dir,
    }
}

fn phase(s: TxStatus) -> TxPhase {
    match s {
        TxStatus::Idle => TxPhase::Idle,
        TxStatus::InTx => TxPhase::InTx,
        TxStatus::Failed => TxPhase::Failed,
    }
}

fn dialect(dbms: DbmsKind) -> DialectId {
    match dbms {
        DbmsKind::MySql => DialectId::MySql,
        DbmsKind::MariaDb => DialectId::MariaDb,
        _ => DialectId::Postgres,
    }
}

impl Live {
    /// Runs `sql` on the user session and tells the coordinator what it did, as a run's sink
    /// would: the status before and after the statement come from the session (the server).
    async fn run(&self, sql: &str) {
        let d = dialect(self.dbms);
        let ctx = ClassifyCtx::new(DialectCtx::new(d, None), ClientFlavor::None, Expect::Script);
        let reports = classify(sql, &ctx).items;
        let unit = UnitId {
            run: RunId(1),
            ordinal: 0,
        };
        let before = phase(self.s.state().borrow().tx.status);
        let mut tracker = RunTracker::new(
            self.s.id(),
            self.dbms,
            vec![TrackedUnit::new(unit, Some(&reports), d, sql)],
            self.s.state(),
            before,
            self.coord.inbox(),
        );
        tracker.unit_started(unit);
        let reply = self
            .s
            .submit(Request::Exec {
                sql: sql.to_string(),
                opts: ExecOpts::default(),
                origin: ExecOrigin::User,
            })
            .reply
            .await
            .expect("an answer");
        let after = phase(self.s.state().borrow().tx.status);
        if before != after {
            tracker.tx_changed(before, after);
        }
        let end = match reply {
            Ok(_) => UnitEnd::Succeeded { rows: None },
            Err(e) => {
                assert!(
                    !matches!(e.kind, ExecErrorKind::ConnectionLost { .. }),
                    "{sql}: the connection was lost"
                );
                UnitEnd::Failed(e)
            }
        };
        assert!(matches!(end, UnitEnd::Succeeded { .. }), "{sql}: {end:?}");
        tracker.unit_ended(unit, &end);
    }

    fn catalog(&self) -> &Arc<Catalog> {
        &self.cat.catalog
    }

    /// Whether namespace `ns` holds a table named `name`.
    fn has_table(&self, ns: &NsKey, name: &str) -> bool {
        self.catalog()
            .current()
            .namespaces
            .get(ns)
            .is_some_and(|a| {
                a.children(0)
                    .any(|(_, o)| o.kind() == ObjectKind::Table && &**o.name() == name)
            })
    }

    async fn wait_for(&self, what: &str, cond: impl Fn(&Self) -> bool) {
        let start = tokio::time::Instant::now();
        while !cond(self) {
            assert!(start.elapsed() < LIMIT, "{what}: not within {LIMIT:?}");
            let mut rx = self.catalog().subscribe();
            let _ = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await;
        }
    }

    async fn shutdown(self) {
        self.cat.handle.shutdown();
        self.m.shutdown().await;
    }
}

/// A unique object name for this process.
fn unique(prefix: &str) -> String {
    format!("{prefix}_{}", std::process::id())
}

/// Runs `body` on `t`'s live profile, then drops `cleanup` (statements run on the session), also
/// when `body` panicked.
async fn with_live(t: &DockerTarget, cleanup: Vec<String>, body: impl AsyncFnOnce(&Live)) {
    let l = live(t).await;
    let outcome = AssertUnwindSafe(body(&l)).catch_unwind().await;
    // A failed transaction must end before the cleanup runs.
    let _ =
        l.s.submit(Request::Exec {
            sql: "rollback".into(),
            opts: ExecOpts::default(),
            origin: ExecOrigin::User,
        })
        .reply
        .await;
    for sql in cleanup {
        let _ =
            l.s.submit(Request::Exec {
                sql,
                opts: ExecOpts::default(),
                origin: ExecOrigin::User,
            })
            .reply
            .await;
    }
    l.shutdown().await;
    if let Err(p) = outcome {
        std::panic::resume_unwind(p);
    }
}

// ---- alg/02 HV-INTRO-12 test cases 6–9 -------------------------------------------------------

/// Test case 7 (explorer side): `BEGIN; CREATE TABLE p` is pending (the explorer does not show
/// `p`, nothing is read for it); `COMMIT` makes it appear.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn pg_ddl_in_a_transaction_is_pending_until_commit() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    let schema = unique("hvm4_t21_p");
    let ns = NsKey::schema([t.database.as_str(), schema.as_str()]);
    with_live(
        &t,
        vec![format!("drop schema if exists {schema} cascade")],
        async |l| {
            l.run(&format!("create schema {schema}")).await;
            l.wait_for("the new schema is listed", |l| {
                l.catalog().current().states.keys().any(
                    |k| matches!(k, hacivat_meta::snapshot::ContainerKey::Namespace(n) if *n == ns),
                )
            })
            .await;
            tokio::time::timeout(LIMIT, l.cat.handle.expand(ns.clone()))
                .await
                .unwrap()
                .unwrap();
            l.run("begin").await;
            l.run(&format!("create table {schema}.p(a int)")).await;
            l.coord.drain();
            assert!(!l.coord.pending(l.s.id()).is_empty());
            assert!(!l.has_table(&ns, "p"));
            l.run("commit").await;
            l.wait_for("p appears after the commit", |l| l.has_table(&ns, "p"))
                .await;
            assert!(l.coord.pending(l.s.id()).is_empty());
        },
    )
    .await;
}

/// Test case 8: the same, then `ROLLBACK`: the pending targets go, nothing is read and nothing
/// is published.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn pg_rollback_discards_pending_and_queries_nothing() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    let schema = unique("hvm4_t21_r");
    let ns = NsKey::schema([t.database.as_str(), schema.as_str()]);
    with_live(
        &t,
        vec![format!("drop schema if exists {schema} cascade")],
        async |l| {
            l.run(&format!("create schema {schema}")).await;
            l.wait_for("the new schema is listed", |l| {
                l.catalog().current().states.keys().any(
                    |k| matches!(k, hacivat_meta::snapshot::ContainerKey::Namespace(n) if *n == ns),
                )
            })
            .await;
            tokio::time::timeout(LIMIT, l.cat.handle.expand(ns.clone()))
                .await
                .unwrap()
                .unwrap();
            let mut progress = l.cat.handle.progress();
            tokio::time::timeout(LIMIT, progress.wait_for(|p| p.tasks.is_empty()))
                .await
                .unwrap()
                .unwrap();
            // Everything the schema's creation asked for has been handed over by now.
            l.coord.drain();
            let applied = l.coord.applied();
            l.run("begin").await;
            l.run(&format!("create table {schema}.p(a int)")).await;
            l.coord.drain();
            assert!(!l.coord.pending(l.s.id()).is_empty(), "p is pending");
            l.run("rollback").await;
            // `drain` is serialized with the coordinator's own task, so once it returns every
            // message pushed so far was handled; `applied` counts a refresh synchronously,
            // before anything is spawned (fix round 1 m1: the check cannot pass vacuously).
            l.coord.drain();
            assert!(l.coord.pending(l.s.id()).is_empty());
            assert_eq!(l.coord.applied(), applied, "a refresh was issued for p");
            assert!(!l.has_table(&ns, "p"));
        },
    )
    .await;
}

/// Test case 9 (ver/01 `#mysql-ro`): `START TRANSACTION; CREATE TABLE m; ROLLBACK` — the DDL
/// committed on its own, so `m` is refreshed at once and stays visible.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn mysql_rollback_after_ddl_still_refreshes() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("mysql84") else {
        return;
    };
    let db = unique("hvm4_t21_m");
    let ns = NsKey::schema([db.as_str()]);
    with_live(
        &t,
        vec![format!("drop database if exists {db}")],
        async |l| {
            l.run(&format!("create database {db}")).await;
            l.wait_for("the new database is listed", |l| {
                l.catalog().current().states.keys().any(
                    |k| matches!(k, hacivat_meta::snapshot::ContainerKey::Namespace(n) if *n == ns),
                )
            })
            .await;
            tokio::time::timeout(LIMIT, l.cat.handle.expand(ns.clone()))
                .await
                .unwrap()
                .unwrap();
            l.run("start transaction").await;
            l.run(&format!("create table {db}.m(a int)")).await;
            l.run("rollback").await;
            l.wait_for("m appears", |l| l.has_table(&ns, "m")).await;
            assert!(l.coord.pending(l.s.id()).is_empty());
        },
    )
    .await;
}

/// Test case 6: dynamic SQL in a `DO` block refreshes the current namespace, so `z` appears.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn dynamic_sql_do_block_makes_z_appear() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    let schema = unique("hvm4_t21_d");
    let ns = NsKey::schema([t.database.as_str(), schema.as_str()]);
    with_live(
        &t,
        vec![format!("drop schema if exists {schema} cascade")],
        async |l| {
            l.run(&format!("create schema {schema}")).await;
            l.wait_for("the new schema is listed", |l| {
                l.catalog().current().states.keys().any(
                    |k| matches!(k, hacivat_meta::snapshot::ContainerKey::Namespace(n) if *n == ns),
                )
            })
            .await;
            tokio::time::timeout(LIMIT, l.cat.handle.expand(ns.clone()))
                .await
                .unwrap()
                .unwrap();
            // The session moves into the schema; its state (read when a statement ends) follows.
            l.run(&format!("set search_path to {schema}")).await;
            l.run("DO $$ BEGIN EXECUTE 'create table z(a int)'; END $$")
                .await;
            l.wait_for("z appears", |l| l.has_table(&ns, "z")).await;
        },
    )
    .await;
}
