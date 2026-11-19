//! Shared fixtures of the `hacivat-introspect` tests. Append-only: each task adds a fenced
//! block `// ## m4-tNN-<slug>` and never edits another task's.

#![allow(dead_code, reason = "each test file uses a subset of the fixtures")]

// ## m4-t01c-core

use std::sync::Arc;

use hacivat_meta::{Name, NsKey};
use hacivat_proto::DbmsKind;
use hacivat_session::profile::Profile;

/// The PostgreSQL schema namespace `[db, schema]`.
pub fn pg_ns(db: &str, schema: &str) -> NsKey {
    NsKey::schema([db, schema])
}

/// A name.
pub fn name(s: &str) -> Name {
    Name::from(s)
}

/// A saved profile of `engine` (no connection is ever opened from it here).
pub fn profile(engine: DbmsKind) -> Arc<Profile> {
    Arc::new(Profile {
        name: "seams".into(),
        engine,
        ..Profile::default()
    })
}

// ## m4-t16-pg
/// Real PostgreSQL sessions for the Docker tests (`docker_pg_intro.rs`): the real drivers, a
/// memory secret store, no credential files, no SSH; a connected profile's metadata
/// connections; a writable admin connection for fixtures; schema drop guards.
pub mod pg {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use hacivat_driver::{EngineCatalog, ExecOpts, Protocol};
    use hacivat_net::TlsMode;
    use hacivat_proto::Department;
    use hacivat_secrets::{
        AuthProviderId, CredentialFiles, MemoryBackend, Retention, SecretBackend, SecretStore,
        SecretString,
    };
    use hacivat_session::meta::{MetaConn, MetaConnSpec};
    use hacivat_session::pipeline::{
        ApprovedRoots, ConnectDeps, ConnectRequest, Interaction, LiveConnection, Progress, Purpose,
        connect,
    };
    use hacivat_session::profile::{Profile, ProfileEndpoint};
    use hacivat_session::{ConnectionManager, LiveState, ManagerConfig, default_drivers};
    use hacivat_testkit::{DockerTarget, TlsDefault};
    use tokio::sync::watch;
    use tokio_util::sync::CancellationToken;

    /// The PostgreSQL targets (13 is exercised, not supported: R-M4-47).
    pub const PG: [&str; 6] = ["pg13", "pg14", "pg15", "pg16", "pg17", "pg18"];

    /// The real drivers, a memory secret store, no credential files, no SSH.
    pub fn deps() -> ConnectDeps {
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

    /// The profile of `t`: user and database `hv_test` (or `db`), a password kept nowhere;
    /// PostgreSQL 13 is allowed (it is below the floor).
    pub fn profile_for(t: &DockerTarget, db: Option<&str>) -> Profile {
        let mut p = Profile {
            name: format!("t16 {}", t.id),
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

    fn progress() -> watch::Sender<Progress> {
        watch::Sender::new(Progress {
            trail: Vec::new(),
            current: None,
            started: Instant::now(),
        })
    }

    /// A profile of `t` connected on its own manager.
    pub struct Pg {
        pub t: DockerTarget,
        pub m: Arc<ConnectionManager>,
        pub profile: Arc<Profile>,
    }

    impl Pg {
        /// Connects `t` (database `hv_test`) and waits on the condition (60 s at most).
        pub async fn connect(t: &DockerTarget) -> Pg {
            let m = ConnectionManager::new(
                tokio::runtime::Handle::current(),
                Arc::new(deps()),
                ManagerConfig::m2b(),
            );
            let profile = Arc::new(profile_for(t, None));
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
            Pg {
                t: t.clone(),
                m,
                profile,
            }
        }

        /// A metadata connection on database `db` (`None`: the profile's).
        pub async fn meta_on(&self, db: Option<&str>) -> MetaConn {
            let cancel = CancellationToken::new();
            self.m
                .sessions(self.profile.id)
                .expect("connected")
                .meta_conn(
                    MetaConnSpec {
                        profile: self.profile.id,
                        database: db.map(str::to_string),
                        statement_timeout: Duration::from_secs(120),
                    },
                    &cancel,
                )
                .await
                .unwrap_or_else(|e| panic!("{}: a metadata connection: {e:?}", self.t.id))
        }

        pub async fn meta(&self) -> MetaConn {
            self.meta_on(None).await
        }

        pub async fn shutdown(self) {
            self.m.shutdown().await;
        }
    }

    /// A writable connection for fixtures (never a metadata connection).
    pub struct Admin {
        pub id: String,
        live: Option<LiveConnection>,
    }

    /// The admin connection of `t` on database `db` (`None`: `hv_test`).
    pub async fn admin_on(t: &DockerTarget, db: Option<&str>) -> Admin {
        let p = profile_for(t, db);
        let live = connect(
            &deps(),
            ConnectRequest {
                read_only: false,
                profile: Arc::new(p),
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
            progress(),
        )
        .await
        .unwrap_or_else(|e| panic!("{} admin: {:?}", t.id, e.class));
        Admin {
            id: t.id.clone(),
            live: Some(live),
        }
    }

    pub async fn admin(t: &DockerTarget) -> Admin {
        admin_on(t, None).await
    }

    impl Admin {
        fn conn(&mut self) -> &mut LiveConnection {
            self.live.as_mut().expect("open")
        }

        /// Runs fixture text (several statements allowed). A failure names the statement only
        /// up to its first literal, and the server's code.
        pub async fn batch(&mut self, sql: &str) {
            let id = self.id.clone();
            if let Err(e) = self.conn().raw().batch(sql).await {
                let shown = sql.split('\'').next().unwrap_or_default();
                panic!(
                    "{id}: fixture `{shown}…`: {:?} {:?}",
                    e.kind,
                    e.server.as_ref().and_then(|m| m.sqlstate.clone())
                );
            }
        }

        /// The first value of the first row of `sql`.
        pub async fn scalar(&mut self, sql: &str) -> String {
            let id = self.id.clone();
            let answer = self
                .conn()
                .raw()
                .execute(
                    sql,
                    ExecOpts {
                        max_rows: 1000,
                        protocol: Protocol::Simple,
                    },
                )
                .await
                .unwrap_or_else(|e| panic!("{id}: {sql}: {:?}", e.kind));
            answer
                .rows
                .first()
                .and_then(|r| r.first())
                .cloned()
                .flatten()
                .map(|v| v.to_string())
                .unwrap_or_default()
        }

        /// The open connection (the big-catalog builder runs on it).
        pub fn live(&mut self) -> &mut LiveConnection {
            self.conn()
        }

        pub async fn close(mut self) {
            if let Some(l) = self.live.take() {
                l.close().await;
            }
        }
    }

    /// A unique name `prefix_<pid>_<n>`.
    pub fn unique(prefix: &str) -> String {
        static N: AtomicU64 = AtomicU64::new(0);
        format!(
            "{prefix}_{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        )
    }

    /// Drops a schema of `t` holding `tables` tables `t1..` when it goes, also on panic: in
    /// 250-table `DO` blocks first, then the schema (review I-5: one `DROP SCHEMA … CASCADE`
    /// over thousands of tables runs out of lock-table slots).
    pub struct DropBatchedSchema {
        pub t: DockerTarget,
        pub schema: String,
        pub tables: u32,
    }

    impl Drop for DropBatchedSchema {
        fn drop(&mut self) {
            let (t, schema, tables) = (self.t.clone(), self.schema.clone(), self.tables);
            let rt = tokio::runtime::Handle::current();
            std::thread::scope(|s| {
                s.spawn(|| {
                    rt.block_on(async {
                        let mut a = admin(&t).await;
                        hacivat_introspect::testing::big_catalog::drop_schema_tables(
                            a.live(),
                            &schema,
                            tables,
                        )
                        .await
                        .unwrap_or_else(|e| panic!("{}: batched drop of {schema}: {e:?}", t.id));
                        a.close().await;
                    });
                });
            });
        }
    }

    /// Drops schemas of `t` when it goes, also on panic.
    pub struct DropSchemas {
        pub t: DockerTarget,
        pub names: Vec<String>,
    }

    impl Drop for DropSchemas {
        fn drop(&mut self) {
            let t = self.t.clone();
            let sql: String = self
                .names
                .iter()
                .map(|n| format!("DROP SCHEMA IF EXISTS \"{n}\" CASCADE;"))
                .collect();
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
}
// ## m4-t15-engine

use std::future::Future;
use std::time::Duration;

use hacivat_introspect::engine::{
    CacheHandle, Engine, EngineConfig, EngineLimits, Filters, IntroHandle, LevelPolicy,
};
use hacivat_introspect::scope::{CurrentCtx, parse_scope};
use hacivat_introspect::testing::FakeIntrospector;
use hacivat_introspect::testing::fake::FakeMetaSource;
use hacivat_meta::name::IdentCtx;
use hacivat_meta::{Catalog, ObjectKey, ObjectKind, ServerFacts};
use sql_dialect::DialectId;

/// An engine over a [`FakeIntrospector`] and a [`FakeMetaSource`].
pub struct Rig {
    pub fake: FakeIntrospector,
    pub meta: Arc<FakeMetaSource>,
    pub catalog: Arc<Catalog>,
    pub handle: IntroHandle,
}

/// How to build a [`Rig`].
pub struct RigOpts {
    pub dbms: DbmsKind,
    pub incremental: bool,
    pub scope: &'static str,
    pub current: CurrentCtx,
    pub cache: Option<CacheHandle>,
    pub limits: EngineLimits,
    pub validate_every: Option<Duration>,
    pub levels: LevelPolicy,
    pub filters: Filters,
}

impl Default for RigOpts {
    fn default() -> Self {
        RigOpts {
            dbms: DbmsKind::Postgres,
            incremental: false,
            scope: "@:*",
            current: CurrentCtx::default(),
            cache: None,
            limits: EngineLimits::default(),
            validate_every: None,
            levels: LevelPolicy::default(),
            filters: Filters::default(),
        }
    }
}

/// The current PostgreSQL namespace `db.schema`.
pub fn pg_current(db: &str, schema: &str) -> CurrentCtx {
    CurrentCtx {
        database: Some(name(db)),
        schema: Some(name(schema)),
        ident: None,
    }
}

/// Starts an engine on the current runtime with `fake` as the server.
pub fn rig_with(fake: FakeIntrospector, opts: RigOpts) -> Rig {
    let dialect = match opts.dbms {
        DbmsKind::MySql => DialectId::MySql,
        DbmsKind::MariaDb => DialectId::MariaDb,
        DbmsKind::Sqlite => DialectId::Sqlite,
        _ => DialectId::Postgres,
    };
    let fake = fake.with_incremental(opts.incremental);
    let meta = FakeMetaSource::new(opts.dbms);
    fake.link_meta(meta.clone());
    let catalog = Arc::new(Catalog::new(Arc::new(ServerFacts::new(
        opts.dbms,
        IdentCtx::new(dialect),
    ))));
    let handle = Engine::start(EngineConfig {
        profile: hacivat_proto::ProfileId::new(),
        dbms: opts.dbms,
        meta: meta.clone(),
        introspector: Arc::new(fake.clone()),
        catalog: catalog.clone(),
        catalog_publisher: catalog.publisher().expect("the only writer"),
        cache: opts.cache,
        levels: opts.levels,
        scope: parse_scope(opts.scope, dialect).expect("a scope"),
        current: opts.current,
        filters: opts.filters,
        validate_every: opts.validate_every,
        limits: opts.limits,
        runtime: tokio::runtime::Handle::current(),
    });
    Rig {
        fake,
        meta,
        catalog,
        handle,
    }
}

/// The key of table `t` of namespace `ns`, with its OID when `oid` is set.
pub fn table_key(ns: &NsKey, t: &str, oid: Option<u32>) -> ObjectKey {
    let mut path: smallvec::SmallVec<[Name; 4]> = ns.path.iter().cloned().collect();
    path.push(name(t));
    ObjectKey {
        kind: ObjectKind::Table,
        path,
        server_id: oid.map(hacivat_meta::ServerId::Oid),
        overload_sig: None,
        raw_kind: None,
    }
}

/// Waits (on the test's clock) until `cond` holds; panics after `limit` of polling.
pub async fn wait_until(limit: Duration, mut cond: impl FnMut() -> bool) {
    let start = tokio::time::Instant::now();
    while !cond() {
        assert!(
            start.elapsed() < limit,
            "condition not met within {limit:?}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Waits until the engine runs nothing.
pub async fn idle(h: &IntroHandle) {
    let mut rx = h.progress();
    let _ = tokio::time::timeout(
        Duration::from_secs(120),
        rx.wait_for(|p| p.tasks.is_empty()),
    )
    .await
    .expect("the engine went idle");
}

/// Runs `f` with a bound, so a hang fails the test instead of blocking it.
pub async fn bounded<T>(f: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(120), f)
        .await
        .expect("finished in time")
}
// ## m4-t15-engine
