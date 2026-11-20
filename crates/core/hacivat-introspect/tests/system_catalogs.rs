// ## m4-t26-overlays (named exception: a new test file of Task 26 in this crate)
//! M4 Task 26's Docker test (plan Task 26, `[docker: pg13]`): a server version without a
//! shipped snapshot (PostgreSQL 13, R-M4-6, R-M4-47) gets no system layer, so its system
//! schemas are introspected live at `Names` level and the view answers from that live read:
//! found names are live objects (never `PRE_INTROSPECTED`) and a miss is never `Missing`.
//!
//! It lives here, not in `hacivat-meta/tests/system_catalogs.rs` as the plan lists it: reading
//! a server needs the introspector and sessions, which `hacivat-meta` may not reach, not even
//! as a dev-dependency (`MetaBelowIntrospect`).
//!
//! Run: `<main>/target/docker/run-docker-tests.sh --set full -p hacivat-introspect --test
//! system_catalogs`.
// ## m4-t33a-hygiene (named exception): the modules this file uses exist only with the
// `session` feature, so `--no-default-features --tests` builds (T16b/T17 carry).
#![cfg(feature = "session")]

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use common::pg::Pg;
use hacivat_introspect::pack::Mode;
use hacivat_introspect::pg::pass::{self, PassOpts};
use hacivat_meta::kind::KindGroup;
use hacivat_meta::merge::{MergeOutcome, MergePolicy, Trigger, base, merge};
use hacivat_meta::name::{DialectId, FoldRule, IdentCtx, Name, NamePart};
use hacivat_meta::search_path::SearchPath;
use hacivat_meta::snapshot::{Catalog, CatalogChange, ContainerKey, LoadState, ServerFacts};
use hacivat_meta::system::SystemCatalogs;
use hacivat_meta::view::{Lookup, ModelView, SnapshotView};
use hacivat_meta::{Level, NsKey, ObjFlags, Watermark};
use hacivat_proto::DbmsKind;
use hacivat_testkit::{docker_enabled, require_target};
use tokio_util::sync::CancellationToken;

fn parts(dotted: &str) -> Vec<NamePart> {
    dotted
        .split('.')
        .map(|t| NamePart {
            text: Arc::from(t),
            quoted: false,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg13_introspects_system_schemas_live_at_names_level() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg13") else {
        println!("HV-FACT t26 pg13 target absent: skipped");
        return;
    };
    // No snapshot for 13 (exact selection): the set gives no system layer.
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../assets/system-catalogs");
    let set = SystemCatalogs::open(&assets).expect("the committed set");
    let version = format!("{}.0", t.major);
    assert!(set.for_server(DbmsKind::Postgres, &version).is_none());
    let layers = set.overlays_for(DbmsKind::Postgres, &version);
    assert!(layers.system.is_none());

    // The live read of `pg_catalog` at `Names` level, in system mode (R-M4-44).
    let pg = Pg::connect(&t).await;
    let mut c = pg.meta().await;
    let ns = NsKey::schema([t.database.as_str(), "pg_catalog"]);
    let cancel = CancellationToken::new();
    let p = pass::introspect(
        &mut c,
        &ns,
        PassOpts {
            mode: Mode::System,
            level: Level::Names,
            ..PassOpts::default()
        },
        &cancel,
    )
    .await
    .expect("a system-mode pass on pg13");
    let b = base(ns.clone(), DbmsKind::Postgres, FoldRule::Exact).unwrap();
    let policy = MergePolicy {
        trigger: Trigger::User,
        ..MergePolicy::default()
    };
    let (MergeOutcome::Applied { next, .. } | MergeOutcome::PendingConfirmation { next, .. }) =
        merge(Some(&b), p.result, &policy).expect("merge");
    drop(c);

    let mut facts = ServerFacts::new(
        DbmsKind::Postgres,
        IdentCtx {
            encoding: Some(Arc::from("UTF8")),
            ..IdentCtx::new(DialectId::Postgres)
        },
    );
    facts.version = Name::from(version.as_str());
    let catalog = Catalog::new(Arc::new(facts));
    let db = Name::from(t.database.as_str());
    catalog
        .publisher()
        .unwrap()
        .publish(vec![
            CatalogChange::SetState(
                ContainerKey::Namespace(ns.clone()),
                LoadState::Loaded {
                    level: Level::Names,
                    at: SystemTime::now(),
                    watermark: Watermark::None,
                    stale: false,
                },
            ),
            CatalogChange::Put(Arc::new(next)),
        ])
        .unwrap();
    let view = SnapshotView::new(catalog.current())
        .with_layers(Arc::new(layers))
        .with_search_path(SearchPath::for_pg(&db, &[Name::from("public")], None, None));
    let hit = view.lookup(&parts("pg_catalog.pg_class"), KindGroup::Relations);
    let Lookup::Found(h) = &hit else {
        panic!("pg13 pg_catalog.pg_class: {hit:?}");
    };
    assert!(h.obj().flags().contains(ObjFlags::SYSTEM));
    assert!(!h.obj().flags().contains(ObjFlags::PRE_INTROSPECTED));
    // A `Names` read is not complete: a miss is never `Missing`.
    assert!(matches!(
        view.lookup(&parts("pg_catalog.hv_t26_absent"), KindGroup::Relations),
        Lookup::Unknown(_)
    ));
    println!(
        "HV-FACT t26 pg13 live system read at Names: pg_catalog objects={}",
        h.ns.len()
    );
}

// ## m4-t26-overlays (ruling R-W5-4): the layers reach the catalogs the hub makes.

fn assets() -> Arc<SystemCatalogs> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../assets/system-catalogs");
    Arc::new(SystemCatalogs::open(&dir).expect("the committed set"))
}

/// The hub makes each profile's catalog with its system set (the catalog re-chooses the layer
/// on every change of the server facts its engine publishes).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hub_built_catalogs_carry_the_hubs_system_layer() {
    use hacivat_introspect::hub::{CatalogHub, HubSettings, SessionLookup};
    use hacivat_proto::ProfileId;
    use hacivat_secrets::SecretStore;
    use hacivat_session::session::SessionManager;
    use hacivat_settings::{LocalPaths, MetadataSettings};

    struct NoSessions;
    impl SessionLookup for NoSessions {
        fn sessions(&self, _: ProfileId) -> Option<Arc<SessionManager>> {
            None
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let system = assets();
    let hub = CatalogHub::new(
        Arc::new(NoSessions),
        HubSettings {
            paths: LocalPaths::under(dir.path()),
            metadata: MetadataSettings::default(),
        },
        Arc::new(SecretStore::new(None)),
        Arc::clone(&system),
    );
    let p = common::profile(DbmsKind::Postgres);
    let made = common::bounded(hub.ensure(p.clone())).await.unwrap();
    let carried = made.catalog.system().expect("the hub's set");
    assert!(Arc::ptr_eq(carried, &system));
    assert!(Arc::ptr_eq(carried, hub.system()));
    // No connection yet: the version is unknown, so no snapshot is chosen (never a guess).
    assert!(made.catalog.current().overlays.system.is_none());
    common::bounded(hub.shutdown(p.id)).await;
}

/// An engine over a fake PostgreSQL server whose facts are `facts`, its catalog made as the
/// hub makes it (the placeholder facts and the shipped set).
fn hub_like_engine(
    facts: Option<Result<ServerFacts, hacivat_introspect::IntroError>>,
) -> (
    Arc<Catalog>,
    hacivat_introspect::engine::IntroHandle,
    hacivat_introspect::testing::FakeIntrospector,
) {
    use hacivat_introspect::engine::{Engine, EngineConfig, EngineLimits, Filters, LevelPolicy};
    use hacivat_introspect::scope::parse_scope;
    use hacivat_introspect::testing::FakeIntrospector;
    use hacivat_introspect::testing::fake::FakeMetaSource;

    let fake = FakeIntrospector::new(DbmsKind::Postgres);
    fake.put_ns(NsKey::schema(["db", "public"]), FakeIntrospector::tables(2));
    fake.set_facts(facts);
    let meta = FakeMetaSource::new(DbmsKind::Postgres);
    fake.link_meta(meta.clone());
    let catalog = Arc::new(Catalog::with_system(
        Arc::new(ServerFacts::new(
            DbmsKind::Postgres,
            IdentCtx::new(DialectId::Postgres),
        )),
        assets(),
    ));
    let handle = Engine::start(EngineConfig {
        profile: hacivat_proto::ProfileId::new(),
        dbms: DbmsKind::Postgres,
        meta: meta.clone(),
        introspector: Arc::new(fake.clone()),
        catalog: catalog.clone(),
        catalog_publisher: catalog.publisher().expect("the only writer"),
        cache: None,
        levels: LevelPolicy::default(),
        scope: parse_scope("@:*", sql_dialect::DialectId::Postgres).expect("a scope"),
        current: common::pg_current("db", "public"),
        filters: Filters::default(),
        validate_every: None,
        limits: EngineLimits::default(),
        runtime: tokio::runtime::Handle::current(),
    });
    (catalog, handle, fake)
}

fn pg_facts(version: &str, extras: Option<bool>) -> ServerFacts {
    let mut facts = ServerFacts::new(
        DbmsKind::Postgres,
        IdentCtx {
            encoding: Some(Arc::from("UTF8")),
            ..IdentCtx::new(DialectId::Postgres)
        },
    );
    facts.version = Name::from(version);
    facts.system_schema_extras = extras;
    facts
}

fn db_view(snap: Arc<hacivat_meta::snapshot::CatalogSnapshot>) -> SnapshotView {
    SnapshotView::new(snap).with_search_path(SearchPath::for_pg(
        &Name::from("db"),
        &[Name::from("public")],
        None,
        None,
    ))
}

/// An engine over a catalog made as the hub makes it: the facts read on connect (PostgreSQL
/// 17.13, nothing beyond initdb in the system schemas) choose the pg17 snapshot, and a plain
/// view of the published generation resolves a built-in function and a system view through
/// the layer, and a name the snapshot lacks is a fact.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_published_facts_resolve_a_pg_builtin_through_the_layer() {
    use hacivat_meta::props::Props;

    let (catalog, handle, _fake) = hub_like_engine(Some(Ok(pg_facts("17.13", Some(false)))));
    assert!(catalog.current().overlays.system.is_none());
    common::bounded(handle.on_connect()).await.unwrap();
    common::idle(&handle).await;
    let snap = catalog.current();
    assert_eq!(&*snap.server.version, "17.13");
    assert_eq!(snap.overlays.system_label().map(|l| &**l), Some("17"));
    let view = db_view(snap);
    let hit = view.lookup(&parts("pg_catalog.pg_stat_activity"), KindGroup::Relations);
    let Lookup::Found(h) = &hit else {
        panic!("pg_stat_activity through the layer: {hit:?}");
    };
    assert!(h.obj().flags().contains(ObjFlags::PRE_INTROSPECTED));
    // A built-in function, unqualified (`pg_catalog` is first on the path), with its arguments
    // (`now()` takes none: `Some([])`, never "name only").
    let hit = view.lookup(&parts("now"), KindGroup::Routines);
    let Lookup::Found(now) = &hit else {
        panic!("now() through the layer: {hit:?}");
    };
    let Props::Routine(r) = now.obj().props() else {
        panic!("now() is a routine");
    };
    assert_eq!(r.args.as_deref(), Some(&[][..]));
    assert!(matches!(
        view.lookup(&parts("pg_catalog.hv_t26_absent"), KindGroup::Relations),
        Lookup::Missing(_)
    ));
    handle.shutdown();
}

/// Ruling R-W5-6 through the engine: facts saying the system schemas hold objects beyond
/// initdb keep the layer's hits, and make its misses `Unknown`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_published_extras_make_system_misses_unknown() {
    let (catalog, handle, _fake) = hub_like_engine(Some(Ok(pg_facts("17.13", Some(true)))));
    common::bounded(handle.on_connect()).await.unwrap();
    common::idle(&handle).await;
    let view = db_view(catalog.current());
    assert!(matches!(
        view.lookup(&parts("pg_catalog.pg_class"), KindGroup::Relations),
        Lookup::Found(_)
    ));
    assert!(matches!(
        view.lookup(&parts("pg_catalog.hv_t26_absent"), KindGroup::Relations),
        Lookup::Unknown(_)
    ));
    handle.shutdown();
}

/// Review m-1: a failed facts read clears the version and the initdb-only fact, so the system
/// layer chosen from the old facts goes (the server behind the profile may have changed).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_facts_read_drops_the_system_layer() {
    use hacivat_introspect::IntroError;
    use hacivat_introspect::ddl_effects::Target;

    let (catalog, handle, fake) = hub_like_engine(Some(Ok(pg_facts("17.13", Some(false)))));
    common::bounded(handle.on_connect()).await.unwrap();
    common::idle(&handle).await;
    assert!(catalog.current().overlays.system.is_some());
    fake.set_facts(Some(Err(IntroError::Timeout)));
    common::bounded(handle.refresh(Target::ScopeRefresh, Trigger::User))
        .await
        .unwrap();
    common::idle(&handle).await;
    let snap = catalog.current();
    assert_eq!(&*snap.server.version, "");
    assert_eq!(snap.server.system_schema_extras, None);
    assert!(snap.overlays.system.is_none());
    // Nothing comes from the old pg17 snapshot any more (the fake server lists no live
    // `pg_catalog`, so the answer is the live catalog's own).
    assert!(!matches!(
        db_view(snap).lookup(&parts("pg_catalog.pg_class"), KindGroup::Relations),
        Lookup::Found(_)
    ));
    handle.shutdown();
}

/// Ruling R-W5-6 on real servers: the PostgreSQL facts read on connect carry the version and
/// the initdb-only fact of the system schemas, read in one metadata snapshot. The pinned
/// images hold nothing beyond initdb in `pg_catalog`/`information_schema`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_server_facts_read_the_initdb_only_fact() {
    use hacivat_introspect::Introspector;
    use hacivat_introspect::pg::PgIntrospector;
    if !docker_enabled() {
        return;
    }
    for id in common::pg::PG {
        let Some(t) = require_target(id) else {
            continue;
        };
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        let facts = PgIntrospector
            .server_facts(&mut c)
            .await
            .expect("the facts read")
            .expect("PostgreSQL reports its facts");
        assert!(
            facts.version.starts_with(&t.major),
            "{id}: {}",
            facts.version
        );
        assert_eq!(facts.system_schema_extras, Some(false), "{id}");
        println!(
            "HV-FACT t26 {id} version={} system_schema_extras={:?}",
            facts.version, facts.system_schema_extras
        );
    }
}

// ## m4-t30-gates (named exception: Task 26's test file; T26 carries m-6 and the R-M4-6
// engine path). A server version without a shipped snapshot gets its system schemas through
// the engine: in scope, read live at their role's level (`Names` by default), never served
// from another version's snapshot. A covered or unknown version keeps them out of scope.

fn ns_state(catalog: &Catalog, ns: &NsKey) -> LoadState {
    catalog
        .current()
        .state(&ContainerKey::Namespace(ns.clone()))
        .clone()
}

/// The fake server lists `pg_catalog` and `pg_toast` as system schemas next to `public`.
fn with_fake_pg_catalog(fake: &hacivat_introspect::testing::FakeIntrospector) {
    for s in ["pg_catalog", "pg_toast"] {
        let ns = NsKey::schema(["db", s]);
        fake.put_ns(
            ns.clone(),
            hacivat_introspect::testing::FakeIntrospector::tables(3),
        );
        fake.edit_ns(&ns, |n| n.system = true);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsupported_version_brings_its_system_schemas_into_scope_live() {
    let (catalog, handle, fake) = hub_like_engine(Some(Ok(pg_facts("13.22", Some(false)))));
    with_fake_pg_catalog(&fake);
    common::bounded(handle.on_connect()).await.unwrap();
    common::idle(&handle).await;
    assert!(
        catalog.current().overlays.system.is_none(),
        "no snapshot for 13"
    );
    let pg_catalog = NsKey::schema(["db", "pg_catalog"]);
    // In scope (never `OutOfScope`), and only the snapshot's system schemas: `pg_toast` stays
    // out.
    assert!(!matches!(
        ns_state(&catalog, &pg_catalog),
        LoadState::OutOfScope
    ));
    assert!(matches!(
        ns_state(&catalog, &NsKey::schema(["db", "pg_toast"])),
        LoadState::OutOfScope
    ));
    common::bounded(handle.expand(pg_catalog.clone()))
        .await
        .unwrap();
    common::idle(&handle).await;
    assert!(matches!(
        ns_state(&catalog, &pg_catalog),
        LoadState::Loaded {
            level: Level::Names,
            ..
        }
    ));
    let view = db_view(catalog.current());
    let hit = view.lookup(&parts("pg_catalog.t0"), KindGroup::Relations);
    let Lookup::Found(h) = &hit else {
        panic!("a live pg_catalog object: {hit:?}");
    };
    assert!(!h.obj().flags().contains(ObjFlags::PRE_INTROSPECTED));
    // Review I-2: read by the system sections, so the objects and the schema root carry
    // `SYSTEM` (the explorer's Built-in group, `CatalogView`'s `is_system`).
    assert!(h.obj().flags().contains(ObjFlags::SYSTEM));
    assert!(h.ns.root().flags().contains(ObjFlags::SYSTEM));
    assert!(fake.calls().iter().any(|c| matches!(
        c,
        hacivat_introspect::testing::fake::FakeCall::Introspect {
            ns,
            mode: Mode::System,
            level: Level::Names,
        } if *ns == pg_catalog
    )));
    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn covered_version_keeps_system_schemas_out_of_scope() {
    let (catalog, handle, fake) = hub_like_engine(Some(Ok(pg_facts("17.13", Some(false)))));
    with_fake_pg_catalog(&fake);
    common::bounded(handle.on_connect()).await.unwrap();
    common::idle(&handle).await;
    assert!(matches!(
        ns_state(&catalog, &NsKey::schema(["db", "pg_catalog"])),
        LoadState::OutOfScope
    ));
    // The snapshot answers: its `pg_class`, not the fake server's live `t0`.
    let view = db_view(catalog.current());
    assert!(matches!(
        view.lookup(&parts("pg_catalog.pg_class"), KindGroup::Relations),
        Lookup::Found(_)
    ));
    assert!(!matches!(
        view.lookup(&parts("pg_catalog.t0"), KindGroup::Relations),
        Lookup::Found(_)
    ));
    handle.shutdown();
}

/// An unknown version (a failed facts read) never brings the system schemas in: the version
/// that chooses between the snapshot and a live read is not known.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_version_keeps_system_schemas_out_of_scope() {
    let (catalog, handle, fake) =
        hub_like_engine(Some(Err(hacivat_introspect::IntroError::Timeout)));
    with_fake_pg_catalog(&fake);
    common::bounded(handle.on_connect()).await.unwrap();
    common::idle(&handle).await;
    assert!(matches!(
        ns_state(&catalog, &NsKey::schema(["db", "pg_catalog"])),
        LoadState::OutOfScope
    ));
    handle.shutdown();
}

/// Task 26 review m-6, on a real PostgreSQL 13 through the engine the hub makes (the shipped
/// set attached): `pg_catalog` comes into scope, an expansion reads it live at `Names` or
/// above, and its objects are live (never `PRE_INTROSPECTED`); no snapshot serves 13.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg13_engine_reads_system_schemas_live_at_names_level() {
    use hacivat_introspect::hub::{CatalogHub, HubSettings, SessionLookup};
    use hacivat_secrets::SecretStore;
    use hacivat_settings::{LocalPaths, MetadataSettings};
    use std::time::Duration;

    const LIMIT: Duration = Duration::from_secs(120);
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg13") else {
        println!("HV-FACT g3 pg13 target absent: skipped");
        return;
    };
    let pg = Pg::connect(&t).await;
    let dir = tempfile::tempdir().expect("a temp dir");
    let lookup: Arc<dyn SessionLookup> = pg.m.clone();
    let hub = CatalogHub::new(
        lookup,
        HubSettings {
            paths: LocalPaths::under(dir.path()),
            metadata: MetadataSettings::default(),
        },
        Arc::new(SecretStore::new(None)),
        assets(),
    );
    let cat = hub.ensure(pg.profile.clone()).await.expect("a catalog");
    tokio::time::timeout(LIMIT, cat.handle.on_connect())
        .await
        .expect("on connect in time")
        .expect("the server is read");
    common::idle(&cat.handle).await;
    let snap = cat.catalog.current();
    assert!(
        snap.server.version.starts_with("13."),
        "{}",
        snap.server.version
    );
    assert!(snap.overlays.system.is_none(), "no snapshot serves 13");
    let ns = NsKey::schema([t.database.as_str(), "pg_catalog"]);
    assert!(
        !matches!(ns_state(&cat.catalog, &ns), LoadState::OutOfScope),
        "pg_catalog is in scope on 13"
    );
    tokio::time::timeout(LIMIT, cat.handle.expand(ns.clone()))
        .await
        .expect("expanded in time")
        .expect("pg_catalog is read");
    common::idle(&cat.handle).await;
    let state = ns_state(&cat.catalog, &ns);
    let LoadState::Loaded { level, .. } = state else {
        panic!("pg_catalog after the expansion: {state:?}");
    };
    assert!(level >= Level::Names);
    let view = SnapshotView::new(cat.catalog.current()).with_search_path(SearchPath::for_pg(
        &Name::from(t.database.as_str()),
        &[Name::from("public")],
        None,
        None,
    ));
    let hit = view.lookup(&parts("pg_catalog.pg_class"), KindGroup::Relations);
    let Lookup::Found(h) = &hit else {
        panic!("pg13 pg_catalog.pg_class through the engine: {hit:?}");
    };
    assert!(!h.obj().flags().contains(ObjFlags::PRE_INTROSPECTED));
    assert!(h.obj().flags().contains(ObjFlags::SYSTEM));
    assert!(h.ns.root().flags().contains(ObjFlags::SYSTEM));
    let miss = view.lookup(&parts("pg_catalog.hv_t30_absent"), KindGroup::Relations);
    assert!(!matches!(miss, Lookup::Found(_)), "{miss:?}");
    println!(
        "HV-FACT g3 pg13 engine live system read: version={} level={level:?} \
         pg_catalog_objects={} system_flag={} miss={}",
        snap.server.version,
        h.ns.len(),
        h.obj().flags().contains(ObjFlags::SYSTEM),
        match miss {
            Lookup::Missing(_) => "Missing",
            Lookup::Unknown(_) => "Unknown",
            Lookup::Found(_) => "Found",
        }
    );
    drop(view);
    drop(snap);
    drop(cat);
    hub.shutdown(pg.profile.id).await;
    pg.shutdown().await;
}
// REVIEW HOSTILE (T30 opus review, I-1): a user's explicit negative clause keeps a system
// schema out of scope on an unsupported version ("removing is always safe", scope.rs docs).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_negative_clause_keeps_pg_catalog_out_on_unsupported_version() {
    use hacivat_introspect::scope::parse_scope;
    let (catalog, handle, fake) = hub_like_engine(Some(Ok(pg_facts("13.22", Some(false)))));
    with_fake_pg_catalog(&fake);
    common::bounded(handle.on_connect()).await.unwrap();
    common::idle(&handle).await;
    let pg_catalog = NsKey::schema(["db", "pg_catalog"]);
    assert!(!matches!(
        ns_state(&catalog, &pg_catalog),
        LoadState::OutOfScope
    ));
    handle.set_scope(
        parse_scope("@:*|!@:pg_catalog", sql_dialect::DialectId::Postgres).expect("a scope"),
    );
    // The scope change is a command: wait on the condition, never on "idle" alone (which may
    // be seen before the command is handled).
    let excluded = || matches!(ns_state(&catalog, &pg_catalog), LoadState::OutOfScope);
    common::wait_until(std::time::Duration::from_secs(10), excluded).await;
    let st = ns_state(&catalog, &pg_catalog);
    assert!(
        matches!(st, LoadState::OutOfScope),
        "the user excluded pg_catalog, yet it is {st:?}"
    );
    handle.shutdown();
}
// ## m4-t30-gates
