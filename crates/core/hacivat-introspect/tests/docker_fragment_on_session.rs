// ## m4-t15b-facets
//! Fragment introspection on a caller's session and facets through the engine, on real servers
//! (M4 Task 15b; R-M4-29, R-M4-30, R-M4-46, R-W3-11).
//!
//! - A `Job(DdlApply)` session opens a transaction, changes a table and writes a row; the
//!   joined read (`fragment::on_session`) sees the change, the job's transaction is still open
//!   afterwards, the job's own `ROLLBACK` removes both (so the joined read sent no `COMMIT`),
//!   and the last statement the job's backend ran was the joined read's `SELECT`
//!   (`pg_stat_activity.query`: no `BEGIN`, `COMMIT`, `ROLLBACK` or `SET`).
//! - Joined PostgreSQL columns carry their type's schema and name, never `format_type` text;
//!   a job whose `search_path` puts a schema before `pg_catalog` is refused (R-W3-11).
//! - A PostgreSQL facet read on a joined connection answers `NotJoinable` (T16b m-7).
//! - The engine loads a PostgreSQL grants facet, and a re-read of its table drops it.
//!
//! Every test is `#[ignore = "docker"]`, returns early unless `HACIVAT_DOCKER=1`, takes its
//! targets only from `require_target`, and creates its own `hvm4_t15b_*` schema or database,
//! dropped by a guard, also on panic. Facts are printed as `HV-FACT` lines (R-W3-14).
//!
//! Run: `<main>/target/docker/run-docker-tests.sh --set pr -p hacivat-introspect --test
//! docker_fragment_on_session` (PR set: pg17, mysql84; full adds the other targets).

#![allow(
    clippy::result_large_err,
    reason = "the session's `Reply` and the seam's `IntroError` are other crates' types"
)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::pg::{DropSchemas, PG, Pg, admin, unique};
use futures::future::BoxFuture;
use hacivat_driver::TxStatus;
use hacivat_introspect::ddl_effects::Target;
use hacivat_introspect::engine::{
    Engine, EngineConfig, EngineLimits, Filters, IntroHandle, LevelPolicy, MetaSource,
};
use hacivat_introspect::fragment::on_session;
use hacivat_introspect::pg::PgIntrospector;
use hacivat_introspect::scope::{CurrentCtx, parse_scope};
use hacivat_introspect::{IntroError, Introspector};
use hacivat_meta::merge::Trigger;
use hacivat_meta::name::IdentCtx;
use hacivat_meta::props::{FacetKind, Props, TypeRef};
use hacivat_meta::snapshot::{ContainerKey, LoadState};
use hacivat_meta::{
    Catalog, Level, Name, NamespaceSnapshot, NsKey, ObjectKey, ObjectKind, ServerFacts,
};
use hacivat_proto::{DbmsKind, Department, ProfileId};
use hacivat_session::meta::{MetaConn, MetaConnSpec, MetaError};
use hacivat_session::pipeline::Interaction;
use hacivat_session::session::{
    ExecOrigin, JobKind, Reply, Request, SessionHandle, SessionKind, SessionManager, SessionSpec,
};
use hacivat_testkit::{DockerTarget, docker_enabled, require_target};
use sql_dialect::DialectId;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------------------------
// Helpers

fn pg_targets() -> Vec<DockerTarget> {
    PG.into_iter().filter_map(require_target).collect()
}

fn n(s: &str) -> Name {
    Name::from(s)
}

/// A quoted identifier (fixture text only; product code never builds SQL from names).
fn qi(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn key(ns: &NsKey, kind: ObjectKind, name: &str) -> ObjectKey {
    let mut path: smallvec::SmallVec<[Name; 4]> = ns.path.iter().cloned().collect();
    path.push(n(name));
    ObjectKey {
        kind,
        path,
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    }
}

/// The facts of a catalog of `dbms` (what `on_session`'s caller holds).
fn facts(dbms: DbmsKind) -> ServerFacts {
    let d = match dbms {
        DbmsKind::MySql => DialectId::MySql,
        DbmsKind::MariaDb => DialectId::MariaDb,
        _ => DialectId::Postgres,
    };
    ServerFacts::new(dbms, IdentCtx::new(d))
}

/// A `Job(DdlApply)` session of `sm` (Hacivat's own: the joined read's only home).
async fn job(sm: &Arc<SessionManager>, id: &str) -> SessionHandle {
    sm.open(
        SessionSpec {
            kind: SessionKind::Job(1, JobKind::DdlApply),
            read_only: false,
            department: Department::Executor,
        },
        Interaction::Headless,
    )
    .await
    .unwrap_or_else(|e| panic!("{id}: the job session opens: {:?}", e.class))
}

async fn run(h: &SessionHandle, sql: &str) -> Reply {
    h.submit(Request::Exec {
        sql: sql.to_string(),
        opts: hacivat_driver::ExecOpts::default(),
        origin: ExecOrigin::Internal { idempotent: false },
    })
    .reply
    .await
    .expect("the actor replies")
}

async fn exec(h: &SessionHandle, id: &str, sql: &str) {
    if let Err(e) = run(h, sql).await {
        let shown = sql.split('\'').next().unwrap_or_default();
        panic!("{id}: `{shown}`: {:?}", e.kind);
    }
}

async fn cell(h: &SessionHandle, id: &str, sql: &str) -> String {
    let s = run(h, sql)
        .await
        .unwrap_or_else(|e| panic!("{id}: {sql}: {:?}", e.kind));
    s.rows[0][0].clone().unwrap_or_default()
}

/// Waits until `h` reports a transaction (10 s at most).
async fn in_tx(h: &SessionHandle) -> bool {
    let mut st = h.state();
    tokio::time::timeout(
        Duration::from_secs(10),
        st.wait_for(|s| s.tx.status == TxStatus::InTx),
    )
    .await
    .is_ok()
}

fn tx_of(h: &SessionHandle) -> TxStatus {
    h.state().borrow().tx.status
}

/// The columns of table `t` of `snap`, with their props.
fn columns(snap: &NamespaceSnapshot, t: &ObjectKey) -> Vec<(String, Props)> {
    let Some(i) = snap.find(t) else {
        return Vec::new();
    };
    snap.children(i)
        .filter(|(_, o)| o.kind() == ObjectKind::Column)
        .map(|(_, o)| (o.name().to_string(), o.props().clone()))
        .collect()
}

fn names(cols: &[(String, Props)]) -> Vec<&str> {
    cols.iter().map(|(n, _)| n.as_str()).collect()
}

fn type_of(cols: &[(String, Props)], name: &str) -> (Option<TypeRef>, String) {
    let (_, p) = cols
        .iter()
        .find(|(c, _)| c == name)
        .unwrap_or_else(|| panic!("column {name}"));
    let Props::Column(c) = p else {
        panic!("column props of {name}");
    };
    (c.type_ref.clone(), c.raw_type.to_string())
}

// ---------------------------------------------------------------------------------------------
// PostgreSQL

/// A fresh schema `hvm4_t15b_*` with `sql` run in it (search path set for the fixture only).
async fn pg_schema(t: &DockerTarget, sql: &str) -> (String, DropSchemas) {
    let schema = unique("hvm4_t15b");
    let guard = DropSchemas {
        t: t.clone(),
        names: vec![schema.clone()],
    };
    let mut a = admin(t).await;
    a.batch(&format!(
        "CREATE SCHEMA {s}; SET search_path = {s}; {sql}",
        s = qi(&schema)
    ))
    .await;
    a.close().await;
    (schema, guard)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn joined_fragment_sees_the_apply_txns_uncommitted_column() {
    if !docker_enabled() {
        return;
    }
    for t in pg_targets() {
        let (schema, _guard) = pg_schema(&t, "CREATE TABLE hvm4_t (a integer)").await;
        let pg = Pg::connect(&t).await;
        let sm = pg.m.sessions(pg.profile.id).expect("connected");
        let ns = NsKey::schema([t.database.as_str(), schema.as_str()]);
        let table = key(&ns, ObjectKind::Table, "hvm4_t");
        let j = job(&sm, &t.id).await;
        let pid = cell(&j, &t.id, "SELECT pg_catalog.pg_backend_pid()::text").await;
        exec(&j, &t.id, "BEGIN").await;
        let s = qi(&schema);
        exec(
            &j,
            &t.id,
            &format!("ALTER TABLE {s}.hvm4_t ADD COLUMN c integer"),
        )
        .await;
        exec(
            &j,
            &t.id,
            &format!("INSERT INTO {s}.hvm4_t (a, c) VALUES (1, 2)"),
        )
        .await;
        assert!(in_tx(&j).await, "{}: the job is in its transaction", t.id);
        let xid = cell(
            &j,
            &t.id,
            "SELECT pg_catalog.pg_current_xact_id_if_assigned()::text",
        )
        .await;

        let cancel = CancellationToken::new();
        let got = on_session(
            &j,
            &facts(DbmsKind::Postgres),
            &ns,
            std::slice::from_ref(&table),
            Level::Structure,
            &cancel,
        )
        .await
        .unwrap_or_else(|e| panic!("{}: the joined read: {e:?}", t.id));
        let cols = columns(&got, &table);
        assert_eq!(names(&cols), ["a", "c"], "{}: the uncommitted column", t.id);
        assert_eq!(got.generation(), 0, "{}: never published", t.id);
        assert_eq!(
            tx_of(&j),
            TxStatus::InTx,
            "{}: the transaction is untouched",
            t.id
        );

        // What the job's backend ran last: the joined read's SELECT (nothing after it).
        let mut a = admin(&t).await;
        let last = a
            .scalar(&format!(
                "SELECT query FROM pg_catalog.pg_stat_activity WHERE pid = {pid}"
            ))
            .await;
        a.close().await;
        let first_word = last
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        println!(
            "HV-FACT t15b_joined_last_statement_keyword={first_word} target={}",
            t.id
        );
        assert_eq!(first_word, "SELECT", "{}: {last}", t.id);
        // The same transaction, the same XID: nothing began or ended it.
        assert_eq!(
            cell(
                &j,
                &t.id,
                "SELECT pg_catalog.pg_current_xact_id_if_assigned()::text"
            )
            .await,
            xid,
            "{}",
            t.id
        );

        exec(&j, &t.id, "ROLLBACK").await;
        let mut a = admin(&t).await;
        let after = a
            .scalar(&format!(
                "SELECT count(*)::text FROM pg_catalog.pg_attribute \
                 WHERE attrelid = '{s}.hvm4_t'::regclass AND attname = 'c'"
            ))
            .await;
        let rows = a
            .scalar(&format!("SELECT count(*)::text FROM {s}.hvm4_t"))
            .await;
        a.close().await;
        assert_eq!(
            (after.as_str(), rows.as_str()),
            ("0", "0"),
            "{}: the job's ROLLBACK undid both",
            t.id
        );
        drop(j);
        pg.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn joined_pg_sections_return_type_names_not_format_type_text() {
    if !docker_enabled() {
        return;
    }
    for t in pg_targets() {
        let (schema, _guard) = pg_schema(
            &t,
            "CREATE DOMAIN hvm4_dom AS integer; \
             CREATE TABLE hvm4_types (v varchar(64), m numeric(10, 2), ts timestamptz, \
             arr integer[], d hvm4_dom, i integer GENERATED ALWAYS AS IDENTITY)",
        )
        .await;
        let pg = Pg::connect(&t).await;
        let sm = pg.m.sessions(pg.profile.id).expect("connected");
        let ns = NsKey::schema([t.database.as_str(), schema.as_str()]);
        let table = key(&ns, ObjectKind::Table, "hvm4_types");
        let j = job(&sm, &t.id).await;
        exec(&j, &t.id, "BEGIN").await;
        assert!(in_tx(&j).await);
        let cancel = CancellationToken::new();
        let got = on_session(
            &j,
            &facts(DbmsKind::Postgres),
            &ns,
            std::slice::from_ref(&table),
            Level::Sources,
            &cancel,
        )
        .await
        .unwrap_or_else(|e| panic!("{}: the joined read: {e:?}", t.id));
        let cols = columns(&got, &table);
        let pg_type = |name: &str| {
            (
                Some(TypeRef {
                    schema: Some(n("pg_catalog")),
                    name: n(name),
                }),
                name.to_string(),
            )
        };
        assert_eq!(
            type_of(&cols, "v"),
            pg_type("varchar"),
            "{}: not `character varying(64)`",
            t.id
        );
        assert_eq!(type_of(&cols, "m"), pg_type("numeric"), "{}", t.id);
        assert_eq!(type_of(&cols, "ts"), pg_type("timestamptz"), "{}", t.id);
        assert_eq!(type_of(&cols, "arr"), pg_type("_int4"), "{}", t.id);
        assert_eq!(
            type_of(&cols, "d"),
            (
                Some(TypeRef {
                    schema: Some(n(&schema)),
                    name: n("hvm4_dom")
                }),
                "hvm4_dom".to_string()
            ),
            "{}: a user type by schema and name",
            t.id
        );
        // A joined read is at most `Structure` (no deparsed text).
        assert_eq!(got.meta().level, Level::Structure, "{}", t.id);
        println!(
            "HV-FACT t15b_joined_pg_type_of_varchar64=varchar target={}",
            t.id
        );

        // R-W3-11: a path that puts a schema before `pg_catalog` is refused before any read.
        exec(
            &j,
            &t.id,
            &format!("SET LOCAL search_path = {}, pg_catalog", qi(&schema)),
        )
        .await;
        assert_eq!(
            on_session(
                &j,
                &facts(DbmsKind::Postgres),
                &ns,
                std::slice::from_ref(&table),
                Level::Structure,
                &cancel
            )
            .await
            .map(drop),
            Err(IntroError::Meta(MetaError::NotJoinable)),
            "{}",
            t.id
        );
        exec(&j, &t.id, "ROLLBACK").await;
        drop(j);
        pg.shutdown().await;
    }
}

/// Review I-1 (ruling R-W5-5): another backend's temp schema in the job's path, holding an
/// `=` over `(oid, oid)`, is refused when the job's `current_schemas(true)` lists it before
/// `pg_catalog` (PostgreSQL searches it for operators; only the session's own temp schema is
/// skipped). A login that cannot use that schema never sees it in its path, and then the read
/// proceeds without ever reaching its operator.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn joined_read_refuses_another_backends_temp_schema() {
    if !docker_enabled() {
        return;
    }
    for t in pg_targets() {
        let (schema, _guard) = pg_schema(&t, "CREATE TABLE hvm4_t (a integer)").await;
        // Session A: a live temp schema with an `=` over (oid, oid) that matches everything.
        let mut a = admin(&t).await;
        a.batch(
            "CREATE FUNCTION pg_temp.hvm4_evil(oid, oid) RETURNS boolean \
             LANGUAGE sql AS 'SELECT true'; \
             CREATE OPERATOR pg_temp.= (LEFTARG = oid, RIGHTARG = oid, \
             FUNCTION = pg_temp.hvm4_evil)",
        )
        .await;
        let foreign = a
            .scalar(
                "SELECT nspname::text FROM pg_catalog.pg_namespace \
                 WHERE oid = pg_catalog.pg_my_temp_schema()",
            )
            .await;
        let pg = Pg::connect(&t).await;
        let sm = pg.m.sessions(pg.profile.id).expect("connected");
        let ns = NsKey::schema([t.database.as_str(), schema.as_str()]);
        let table = key(&ns, ObjectKind::Table, "hvm4_t");
        let j = job(&sm, &t.id).await;
        exec(&j, &t.id, "BEGIN").await;
        exec(
            &j,
            &t.id,
            &format!("SET LOCAL search_path = {}, pg_catalog", qi(&foreign)),
        )
        .await;
        assert!(in_tx(&j).await);
        let listed = cell(
            &j,
            &t.id,
            &format!(
                "SELECT ('{}' = ANY (pg_catalog.current_schemas(true)))::text",
                foreign.replace('\'', "''")
            ),
        )
        .await;
        println!(
            "HV-FACT t15b_foreign_temp_in_job_path={listed} target={}",
            t.id
        );
        let got = on_session(
            &j,
            &facts(DbmsKind::Postgres),
            &ns,
            std::slice::from_ref(&table),
            Level::Structure,
            &CancellationToken::new(),
        )
        .await;
        if listed == "true" {
            assert_eq!(
                got.map(drop),
                Err(IntroError::Meta(MetaError::NotJoinable)),
                "{}: another backend's temp schema before pg_catalog",
                t.id
            );
        } else {
            // Not in the job's path (no USAGE): the read proceeds, and the foreign `=` (which
            // matches every oid) was never used: only the asked table is listed.
            let got = got.unwrap_or_else(|e| panic!("{}: {e:?}", t.id));
            assert_eq!(got.root().child_count(), 1, "{}", t.id);
            assert_eq!(names(&columns(&got, &table)), ["a"], "{}", t.id);
        }
        exec(&j, &t.id, "ROLLBACK").await;
        drop(j);
        a.close().await;
        pg.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn pg_facet_on_a_joined_session_is_not_joinable() {
    if !docker_enabled() {
        return;
    }
    for t in pg_targets() {
        let (schema, _guard) = pg_schema(&t, "CREATE TABLE hvm4_t (a integer)").await;
        let pg = Pg::connect(&t).await;
        let sm = pg.m.sessions(pg.profile.id).expect("connected");
        let ns = NsKey::schema([t.database.as_str(), schema.as_str()]);
        let j = job(&sm, &t.id).await;
        exec(&j, &t.id, "BEGIN").await;
        assert!(in_tx(&j).await);
        let mut c = MetaConn::join(&j).expect("joinable");
        let got = PgIntrospector
            .facet(
                &mut c,
                &ns,
                &[key(&ns, ObjectKind::Table, "hvm4_t")],
                FacetKind::Grants,
            )
            .await;
        assert!(
            matches!(got, Err(IntroError::Meta(MetaError::NotJoinable))),
            "{}: {got:?}",
            t.id
        );
        drop(c);
        assert_eq!(tx_of(&j), TxStatus::InTx);
        exec(&j, &t.id, "ROLLBACK").await;
        drop(j);
        pg.shutdown().await;
    }
}

/// The engine's metadata connections from a connected manager.
struct ManagerMeta {
    sm: Arc<SessionManager>,
    profile: ProfileId,
}

impl MetaSource for ManagerMeta {
    fn acquire<'a>(
        &'a self,
        database: Option<Name>,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<MetaConn, MetaError>> {
        Box::pin(async move {
            self.sm
                .meta_conn(
                    MetaConnSpec {
                        profile: self.profile,
                        database: database.map(|d| d.to_string()),
                        statement_timeout: Duration::from_secs(120),
                    },
                    cancel,
                )
                .await
        })
    }
}

fn pg_engine(pg: &Pg, t: &DockerTarget, schema: &str) -> (Arc<Catalog>, IntroHandle) {
    let catalog = Arc::new(Catalog::new(Arc::new(ServerFacts::new(
        DbmsKind::Postgres,
        IdentCtx::new(DialectId::Postgres),
    ))));
    let handle = Engine::start(EngineConfig {
        profile: pg.profile.id,
        dbms: DbmsKind::Postgres,
        meta: Arc::new(ManagerMeta {
            sm: pg.m.sessions(pg.profile.id).expect("connected"),
            profile: pg.profile.id,
        }),
        introspector: Arc::new(PgIntrospector),
        catalog: catalog.clone(),
        catalog_publisher: catalog.publisher().expect("the only writer"),
        cache: None,
        levels: LevelPolicy::default(),
        scope: parse_scope("@:*", DialectId::Postgres).expect("a scope"),
        current: CurrentCtx {
            database: Some(n(&t.database)),
            schema: Some(n(schema)),
            ident: None,
        },
        filters: Filters::default(),
        validate_every: None,
        limits: EngineLimits::default(),
        runtime: tokio::runtime::Handle::current(),
    });
    (catalog, handle)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn pg_grants_facet_loaded_through_the_handle() {
    if !docker_enabled() {
        return;
    }
    for t in pg_targets() {
        let (schema, _guard) = pg_schema(
            &t,
            "CREATE TABLE hvm4_g (a integer); GRANT SELECT ON hvm4_g TO PUBLIC",
        )
        .await;
        let pg = Pg::connect(&t).await;
        let (catalog, handle) = pg_engine(&pg, &t, &schema);
        let ns = NsKey::schema([t.database.as_str(), schema.as_str()]);
        handle
            .expand(ns.clone())
            .await
            .unwrap_or_else(|e| panic!("{}: {e:?}", t.id));
        let (c, at) = (catalog.clone(), ns.clone());
        common::wait_until(Duration::from_secs(60), move || {
            matches!(
                c.current().state(&ContainerKey::Namespace(at.clone())),
                LoadState::Loaded {
                    level: Level::Structure | Level::Sources,
                    ..
                }
            )
        })
        .await;
        let g = key(&ns, ObjectKind::Table, "hvm4_g");
        let before = catalog.current();
        let arena = before.namespaces.get(&ns).expect("loaded").clone();
        handle
            .load_facet(vec![g.clone()], FacetKind::Grants)
            .await
            .unwrap_or_else(|e| panic!("{}: grants: {e:?}", t.id));
        let snap = catalog.current();
        assert!(
            Arc::ptr_eq(&arena, snap.namespaces.get(&ns).unwrap()),
            "{}: no arena moved",
            t.id
        );
        let f = snap.grants_facet(&g);
        assert!(
            matches!(f.state, LoadState::Loaded { stale: false, .. }),
            "{}: {:?}",
            t.id,
            f.state
        );
        let entries: Vec<(String, String)> = f
            .value
            .as_ref()
            .map(|v| {
                v.entries
                    .iter()
                    .map(|e| (e.grantee.to_string(), e.privilege.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        println!("HV-FACT t15b_pg_grants={entries:?} target={}", t.id);
        assert_eq!(
            entries,
            [("PUBLIC".to_string(), "SELECT".to_string())],
            "{}",
            t.id
        );

        // A GRANT rewrites the table's row: its re-read drops the loaded facet; the next load
        // reads the new grants.
        let mut a = admin(&t).await;
        a.batch(&format!("GRANT UPDATE ON {}.hvm4_g TO PUBLIC", qi(&schema)))
            .await;
        a.close().await;
        handle
            .refresh(
                Target::Objects {
                    ns: ns.clone(),
                    keys: vec![g.clone()],
                },
                Trigger::User,
            )
            .await
            .unwrap_or_else(|e| panic!("{}: re-read: {e:?}", t.id));
        assert_eq!(
            catalog.current().grants_facet(&g).state,
            LoadState::NotLoaded,
            "{}",
            t.id
        );
        handle
            .load_facet(vec![g.clone()], FacetKind::Grants)
            .await
            .unwrap_or_else(|e| panic!("{}: grants again: {e:?}", t.id));
        let privileges: Vec<String> = catalog
            .current()
            .grants_facet(&g)
            .value
            .map(|v| v.entries.iter().map(|e| e.privilege.to_string()).collect())
            .unwrap_or_default();
        assert!(
            privileges.contains(&"UPDATE".to_string()),
            "{}: {privileges:?}",
            t.id
        );
        handle.shutdown();
        handle.closed().await;
        pg.shutdown().await;
    }
}

// ---------------------------------------------------------------------------------------------
// MySQL and MariaDB

mod my {
    use std::path::PathBuf;
    use std::time::Instant;

    use hacivat_driver::EngineCatalog;
    use hacivat_net::TlsMode;
    use hacivat_secrets::{
        AuthProviderId, CredentialFiles, MemoryBackend, Retention, SecretBackend, SecretStore,
        SecretString,
    };
    use hacivat_session::pipeline::{
        ApprovedRoots, ConnectDeps, ConnectRequest, Progress, Purpose, connect,
    };
    use hacivat_session::profile::{Profile, ProfileEndpoint};
    use hacivat_session::{ConnectionManager, LiveState, ManagerConfig, default_drivers};
    use hacivat_testkit::TlsDefault;
    use tokio::sync::watch;

    use super::*;

    pub const TARGETS: [&str; 3] = ["mysql84", "mysql80", "mariadb114"];

    fn deps() -> ConnectDeps {
        let backend: Arc<dyn SecretBackend> = Arc::new(MemoryBackend::new());
        let known_hosts = hacivat_net::ssh::KnownHosts::load(
            Vec::new(),
            PathBuf::from("/nonexistent/hacivat/known_hosts"),
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

    fn profile_for(t: &DockerTarget, db: Option<&str>) -> Profile {
        let mut p = Profile {
            name: format!("t15b {}", t.id),
            engine: t.engine,
            endpoint: ProfileEndpoint::Server {
                host: t.host.clone(),
                port: Some(t.port),
            },
            database: Some(db.unwrap_or(&t.database).to_string()),
            user: Some(t.user.clone()),
            auth: AuthProviderId::Password,
            retention: Retention::Never,
            ..Profile::default()
        };
        p.tls.mode = match t.tls_default {
            TlsDefault::SelfSigned => TlsMode::Require,
            TlsDefault::None => TlsMode::Auto,
        };
        p.allow_unsupported_server = true;
        p
    }

    fn password(t: &DockerTarget) -> SecretString {
        SecretString::from(t.password.clone())
    }

    /// A connected manager of `t` on database `db`.
    pub async fn manager(t: &DockerTarget, db: &str) -> (Arc<ConnectionManager>, ProfileId) {
        let m = ConnectionManager::new(
            tokio::runtime::Handle::current(),
            Arc::new(deps()),
            ManagerConfig::m2b(),
        );
        let profile = Arc::new(profile_for(t, Some(db)));
        m.connect(profile.clone(), Some(password(t)), Interaction::Headless);
        let mut changes = m.changes();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            match m.state(profile.id) {
                LiveState::Connected(_) => break,
                LiveState::Failed(e) => panic!("{}: connect failed: {:?}", t.id, e.class),
                _ => {}
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{}: connect timed out",
                t.id
            );
            let _ = tokio::time::timeout(Duration::from_millis(200), changes.changed()).await;
        }
        (m, profile.id)
    }

    /// Runs `sql` as `hv_test` on `db` (`None`: its default).
    pub async fn exec(t: &DockerTarget, db: Option<&str>, sql: &str) -> Result<(), String> {
        let mut live = connect(
            &deps(),
            ConnectRequest {
                read_only: false,
                profile: Arc::new(profile_for(t, db)),
                draft_password: Some(password(t)),
                purpose: Purpose::Main,
                department: Department::Console,
                prior_rejections: 0,
                draft_ssh: Vec::new(),
                overlay_secrets: None,
                plaintext_consent: false,
            },
            Interaction::Headless,
            CancellationToken::new(),
            watch::Sender::new(Progress {
                trail: Vec::new(),
                current: None,
                started: Instant::now(),
            }),
        )
        .await
        .unwrap_or_else(|e| panic!("{} admin: {:?}", t.id, e.class));
        let out = live
            .raw()
            .batch(sql)
            .await
            .map_err(|e| format!("{:?}", e.kind));
        drop(live);
        out
    }

    /// `hvm4_t15b_<slug>`, dropped when the guard goes (also on panic).
    pub struct Db {
        pub t: DockerTarget,
        pub name: String,
    }

    impl Db {
        pub async fn create(t: &DockerTarget, slug: &str) -> Db {
            let name = format!("hvm4_t15b_{slug}");
            exec(t, None, &format!("DROP DATABASE IF EXISTS `{name}`"))
                .await
                .expect("drop leftover");
            exec(t, None, &format!("CREATE DATABASE `{name}`"))
                .await
                .unwrap_or_else(|e| panic!("{}: create {name}: {e}", t.id));
            Db { t: t.clone(), name }
        }
    }

    impl Drop for Db {
        fn drop(&mut self) {
            let (t, name) = (self.t.clone(), self.name.clone());
            let _ = std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("a runtime");
                if let Err(e) =
                    rt.block_on(exec(&t, None, &format!("DROP DATABASE IF EXISTS `{name}`")))
                {
                    eprintln!("{}: dropping {name}: {e}", t.id);
                }
            })
            .join();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn mysql_joined_fragment_reads_inside_the_jobs_transaction() {
    if !docker_enabled() {
        return;
    }
    for t in my::TARGETS.into_iter().filter_map(require_target) {
        let db = my::Db::create(&t, "joined").await;
        my::exec(&t, Some(&db.name), "CREATE TABLE hvm4_t (a INT)")
            .await
            .unwrap_or_else(|e| panic!("{}: fixture: {e}", t.id));
        let (m, profile) = my::manager(&t, &db.name).await;
        let sm = m.sessions(profile).expect("connected");
        let ns = NsKey::schema([db.name.as_str()]);
        let table = key(&ns, ObjectKind::Table, "hvm4_t");
        let j = job(&sm, &t.id).await;
        // MySQL commits a transaction implicitly before DDL: the job changes the table, then
        // opens the transaction its drift check joins, and writes a row in it.
        exec(&j, &t.id, "ALTER TABLE hvm4_t ADD COLUMN c INT").await;
        exec(&j, &t.id, "START TRANSACTION").await;
        exec(&j, &t.id, "INSERT INTO hvm4_t (a, c) VALUES (1, 2)").await;
        assert!(in_tx(&j).await, "{}: the job is in its transaction", t.id);
        let cancel = CancellationToken::new();
        let got = on_session(
            &j,
            &facts(t.engine),
            &ns,
            std::slice::from_ref(&table),
            Level::Structure,
            &cancel,
        )
        .await
        .unwrap_or_else(|e| panic!("{}: the joined read: {e:?}", t.id));
        let cols = columns(&got, &table);
        assert_eq!(names(&cols), ["a", "c"], "{}", t.id);
        assert_eq!(type_of(&cols, "c").1, "int", "{}", t.id);
        assert_eq!(
            tx_of(&j),
            TxStatus::InTx,
            "{}: the transaction is untouched",
            t.id
        );
        println!(
            "HV-FACT t15b_mysql_joined_columns={:?} target={}",
            names(&cols),
            t.id
        );
        // The job's ROLLBACK removes the row: the joined read committed nothing.
        exec(&j, &t.id, "ROLLBACK").await;
        let rows = cell(&j, &t.id, "SELECT CAST(COUNT(*) AS CHAR) FROM hvm4_t").await;
        assert_eq!(rows, "0", "{}", t.id);
        drop(j);
        m.shutdown().await;
        drop(db);
    }
}
