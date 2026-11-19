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
        assert!(start.elapsed() < limit, "condition not met within {limit:?}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Waits until the engine runs nothing.
pub async fn idle(h: &IntroHandle) {
    let mut rx = h.progress();
    let _ = tokio::time::timeout(Duration::from_secs(120), rx.wait_for(|p| p.tasks.is_empty()))
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
