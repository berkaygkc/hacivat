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
