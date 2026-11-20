//! T26 fix-round re-review probes (opus, `task-m4-26-rr1-hostile.rs`), committed in fix round 2:
//! r1 pins ruling R-W5-6 for PostgreSQL; r2 pins MySQL at the exact release, including
//! RR-I-1 (plugin variables are never `Missing`).
#![cfg(feature = "system")]

use std::sync::Arc;
use std::time::SystemTime;

use hacivat_meta::fixture::{ns_mysql, ns_pg};
use hacivat_meta::kind::KindGroup;
use hacivat_meta::name::{DialectId, IdentCtx, IdentKey, Name, NameClass, NamePart, match_key};
use hacivat_meta::namespace::{Level, NamespaceSnapshot, Watermark};
use hacivat_meta::search_path::SearchPath;
use hacivat_meta::snapshot::{
    Catalog, CatalogChange, CatalogSnapshot, ContainerKey, LoadState, ServerFacts,
};
use hacivat_meta::system::SystemCatalogs;
use hacivat_meta::view::{Lookup, ModelView, SnapshotView};
use hacivat_proto::DbmsKind;
use sql_catalog::types::RelRef;
use sql_catalog::view::CatalogView;

fn name(s: &str) -> Name {
    Arc::from(s)
}
fn loaded(level: Level) -> LoadState {
    LoadState::Loaded {
        level,
        at: SystemTime::UNIX_EPOCH,
        watermark: Watermark::None,
        stale: false,
    }
}
fn shipped() -> SystemCatalogs {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../assets/system-catalogs");
    SystemCatalogs::open(&dir).unwrap()
}
fn parts(dotted: &str) -> Vec<NamePart> {
    dotted
        .split('.')
        .map(|t| NamePart {
            text: name(t),
            quoted: false,
        })
        .collect()
}
fn k(ctx: &IdentCtx, text: &str, class: NameClass) -> IdentKey {
    match_key(ctx.dialect, ctx, text, false, class).expect("a pinned key")
}
fn snapshot(
    dbms: DbmsKind,
    ctx: IdentCtx,
    version: &str,
    extras: Option<bool>,
    nss: Vec<(NamespaceSnapshot, LoadState)>,
    lists: Vec<ContainerKey>,
) -> Arc<CatalogSnapshot> {
    let mut facts = ServerFacts::new(dbms, ctx);
    facts.version = name(version);
    facts.system_schema_extras = extras;
    let catalog = Catalog::new(Arc::new(facts));
    let publisher = catalog.publisher().unwrap();
    let mut changes = Vec::new();
    for (ns, state) in nss {
        changes.push(CatalogChange::SetState(
            ContainerKey::Namespace(ns.key().clone()),
            state,
        ));
        changes.push(CatalogChange::Put(Arc::new(ns)));
    }
    for l in lists {
        changes.push(CatalogChange::SetState(l, loaded(Level::Names)));
    }
    publisher.publish(changes).unwrap();
    catalog.current()
}
fn kind<T>(l: &Lookup<T>) -> &'static str {
    match l {
        Lookup::Found(_) => "Found",
        Lookup::Missing(_) => "Missing",
        Lookup::Unknown(_) => "Unknown",
    }
}

fn pg_view(extras: Option<bool>) -> SnapshotView {
    let ctx = IdentCtx {
        encoding: Some(Arc::from("UTF8")),
        ..IdentCtx::new(DialectId::Postgres)
    };
    let public = ns_pg("app", "public").table("orders", &["id"]).finish();
    let snap = snapshot(
        DbmsKind::Postgres,
        ctx,
        "17.2",
        extras,
        vec![(public, loaded(Level::Structure))],
        vec![ContainerKey::schema_list(Some(&name("app")))],
    );
    SnapshotView::new(snap)
        .with_layers(Arc::new(shipped().overlays_for(DbmsKind::Postgres, "17.2")))
        .with_search_path(SearchPath::for_pg(
            &name("app"),
            &[name("public")],
            None,
            None,
        ))
}

/// R1. R-W5-6: a pg_catalog miss is a fact only with `Some(false)`.
#[test]
fn r1_pg_miss_is_a_fact_only_when_extras_proven_false() {
    let ctx = IdentCtx::new(DialectId::Postgres);
    let mut got = Vec::new();
    for extras in [Some(false), Some(true), None] {
        let v = pg_view(extras);
        got.push(format!(
            "{extras:?}: q={} unq_absent={} unq_public={} rels={} cols={}",
            kind(&v.lookup(&parts("pg_catalog.hv_absent"), KindGroup::Relations)),
            kind(&v.lookup(&parts("hv_absent"), KindGroup::Relations)),
            kind(&v.lookup(&parts("orders"), KindGroup::Relations)),
            kind(&v.relations(&k(&ctx, "pg_catalog", NameClass::Schema))),
            kind(&v.columns(&RelRef {
                schema: name("pg_catalog"),
                name: name("pg_class")
            })),
        ));
    }
    println!("HV-FACT rr1 {got:#?}");
    assert_eq!(
        got,
        [
            "Some(false): q=Missing unq_absent=Missing unq_public=Found rels=Found cols=Found",
            "Some(true): q=Unknown unq_absent=Unknown unq_public=Unknown rels=Unknown cols=Found",
            "None: q=Unknown unq_absent=Unknown unq_public=Unknown rels=Unknown cols=Found",
        ]
    );
}

/// R2. MySQL at the snapshot's exact release: a system-schema miss is never a fact (R-W5-6);
/// `information_schema` columns are found caselessly; a plugin variable (no dot, not loaded
/// on the snapshot image) must not be `Missing`.
#[test]
fn r2_mysql_exact_release() {
    let ctx = IdentCtx::new(DialectId::MySql);
    let shop = ns_mysql("shop", 0).table("orders", &["id"]).finish();
    let snap = snapshot(
        DbmsKind::MySql,
        ctx.clone(),
        "8.4.13",
        None,
        vec![(shop, loaded(Level::Structure))],
        vec![ContainerKey::schema_list(None)],
    );
    let v = SnapshotView::new(snap)
        .with_layers(Arc::new(shipped().overlays_for(DbmsKind::MySql, "8.4.13")))
        .with_search_path(SearchPath::for_mysql(Some(&name("shop"))));
    let mut bad = Vec::new();
    for p in [
        "mysql.rds_history",
        "sys.hv_absent",
        "information_schema.hv_absent",
    ] {
        let l = v.lookup(&parts(p), KindGroup::Relations);
        if matches!(l, Lookup::Missing(_)) {
            bad.push(format!("{p}: Missing"));
        }
    }
    let cols = v.columns(&RelRef {
        schema: name("INFORMATION_SCHEMA"),
        name: name("tables"),
    });
    if !matches!(cols, Lookup::Found(_)) {
        bad.push(format!(
            "columns(INFORMATION_SCHEMA.tables): {}",
            kind(&cols)
        ));
    }
    let rel = v.relation(
        &k(&ctx, "information_schema", NameClass::Schema),
        &k(&ctx, "Tables", NameClass::Table),
    );
    if !matches!(rel, Lookup::Found(_)) {
        bad.push(format!(
            "relation(information_schema, Tables): {}",
            kind(&rel)
        ));
    }
    println!("HV-FACT rr1 mysql settings()={}", kind(&v.settings()));
    for var in [
        "rpl_semi_sync_source_enabled",
        "group_replication_group_name",
        "audit_log_file",
    ] {
        let s = v.setting(var);
        if matches!(s, Lookup::Missing(_)) {
            bad.push(format!("setting({var}): Missing (a plugin variable)"));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}
