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

/// An engine over a catalog made as the hub makes it: the facts read on connect (PostgreSQL
/// 17.13) choose the pg17 snapshot, and a plain view of the published generation resolves a
/// built-in function and a system view through the layer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_published_facts_resolve_a_pg_builtin_through_the_layer() {
    use hacivat_introspect::engine::{Engine, EngineConfig, EngineLimits, Filters, LevelPolicy};
    use hacivat_introspect::scope::parse_scope;
    use hacivat_introspect::testing::FakeIntrospector;
    use hacivat_introspect::testing::fake::FakeMetaSource;
    use hacivat_meta::props::Props;

    let ctx = IdentCtx {
        encoding: Some(Arc::from("UTF8")),
        ..IdentCtx::new(DialectId::Postgres)
    };
    let fake = FakeIntrospector::new(DbmsKind::Postgres);
    fake.put_ns(NsKey::schema(["db", "public"]), FakeIntrospector::tables(2));
    let mut facts = ServerFacts::new(DbmsKind::Postgres, ctx.clone());
    facts.version = Name::from("17.13");
    fake.set_facts(Some(Ok(facts)));
    let meta = FakeMetaSource::new(DbmsKind::Postgres);
    fake.link_meta(meta.clone());
    // As `CatalogHub` makes it: the placeholder facts and the hub's set.
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
    assert!(catalog.current().overlays.system.is_none());
    common::bounded(handle.on_connect()).await.unwrap();
    common::idle(&handle).await;
    let snap = catalog.current();
    assert_eq!(&*snap.server.version, "17.13");
    assert_eq!(snap.overlays.system_label().map(|l| &**l), Some("17"));
    let view = SnapshotView::new(snap).with_search_path(SearchPath::for_pg(
        &Name::from("db"),
        &[Name::from("public")],
        None,
        None,
    ));
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
    handle.shutdown();
}
