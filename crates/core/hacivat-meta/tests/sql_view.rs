//! M4 Task 26: `sql_catalog::CatalogView` for `SnapshotView` (R-M4-5, R-M4-27): what M5's
//! completion calls, over the live, system and virtual layers. A hit on `relation` or
//! `columns` allocates nothing (Arc-shared slices built once per namespace); built-ins known by
//! name only have `args = None` (X-8); settings come from the snapshot offline.

// Most tests read the shipped snapshots (feature `system`); without it their helpers idle.
#![cfg_attr(not(feature = "system"), allow(unused_imports, dead_code))]

use std::sync::Arc;
use std::time::SystemTime;

use hacivat_meta::fixture::{ns_mysql, ns_pg};
use hacivat_meta::ids::NsKey;
use hacivat_meta::name::{DialectId, IdentCtx, IdentKey, Name, NameClass, match_key};
use hacivat_meta::namespace::{Level, NamespaceSnapshot, Watermark};
use hacivat_meta::overlay::{Overlays, VirtualFk};
use hacivat_meta::search_path::SearchPath;
use hacivat_meta::snapshot::{
    Catalog, CatalogChange, CatalogSnapshot, ContainerKey, LoadState, ServerFacts,
};
use hacivat_meta::view::{Lookup, SnapshotView, UnknownReason, slow_lookups_on_this_thread};
use hacivat_proto::DbmsKind;
use sql_catalog::types::{RelKind, RelRef};
use sql_catalog::view::CatalogView;

const DB: &str = "app";

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

fn pg_ctx() -> IdentCtx {
    IdentCtx {
        encoding: Some(Arc::from("UTF8")),
        ..IdentCtx::new(DialectId::Postgres)
    }
}

fn my_ctx() -> IdentCtx {
    IdentCtx::new(DialectId::MySql)
}

fn k(ctx: &IdentCtx, text: &str, class: NameClass) -> IdentKey {
    match_key(ctx.dialect, ctx, text, false, class).expect("a pinned key")
}

fn rel(schema: &str, n: &str) -> RelRef {
    RelRef {
        schema: name(schema),
        name: name(n),
    }
}

fn snapshot(
    dbms: DbmsKind,
    ctx: IdentCtx,
    version: &str,
    nss: Vec<(NamespaceSnapshot, LoadState)>,
    states: Vec<(NsKey, LoadState)>,
) -> Arc<CatalogSnapshot> {
    let mut facts = ServerFacts::new(dbms, ctx);
    facts.version = name(version);
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
    for (key, state) in states {
        changes.push(CatalogChange::SetState(ContainerKey::Namespace(key), state));
    }
    let list = if dbms == DbmsKind::Postgres {
        ContainerKey::schema_list(Some(&name(DB)))
    } else {
        ContainerKey::schema_list(None)
    };
    changes.push(CatalogChange::SetState(list, loaded(Level::Names)));
    publisher.publish(changes).unwrap();
    catalog.current()
}

#[cfg(feature = "system")]
fn shipped() -> hacivat_meta::system::SystemCatalogs {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../assets/system-catalogs");
    hacivat_meta::system::SystemCatalogs::open(&dir).unwrap()
}

fn pg_view(layers: Overlays, public_state: LoadState) -> SnapshotView {
    let public = ns_pg(DB, "public")
        .table_oid("orders", 100, &["id", "customer_id", "total"])
        .table_oid("customers", 200, &["id", "name"])
        .finish();
    let snap = snapshot(
        DbmsKind::Postgres,
        pg_ctx(),
        "17.12",
        vec![(public, public_state)],
        vec![(NsKey::schema([DB, "pg_catalog"]), LoadState::OutOfScope)],
    );
    SnapshotView::new(snap)
        .with_layers(Arc::new(layers))
        .with_search_path(SearchPath::for_pg(
            &name(DB),
            &[name("public")],
            Some(&name("alice")),
            None,
        ))
}

#[cfg(feature = "system")]
#[test]
fn sql_view_relation_and_columns_found_offline() {
    let view = pg_view(
        shipped().overlays_for(DbmsKind::Postgres, "17.12"),
        loaded(Level::Structure),
    );
    let ctx = pg_ctx();
    let Lookup::Found(r) = view.relation(
        &k(&ctx, "pg_catalog", NameClass::Schema),
        &k(&ctx, "pg_class", NameClass::Table),
    ) else {
        panic!("pg_catalog.pg_class");
    };
    assert_eq!(r.name, rel("pg_catalog", "pg_class"));
    assert_eq!(r.kind, RelKind::Table);
    let Lookup::Found(cols) = view.columns(&rel("pg_catalog", "pg_stat_activity")) else {
        panic!("pg_stat_activity columns");
    };
    assert!(cols.iter().any(|c| &*c.name == "pid"));
    assert!(cols.windows(2).all(|w| w[0].ordinal < w[1].ordinal));
    // The relations of a system schema, and a live one.
    let Lookup::Found(all) = view.relations(&k(&ctx, "pg_catalog", NameClass::Schema)) else {
        panic!("pg_catalog relations");
    };
    assert!(all.iter().any(|r| &*r.name.name == "pg_stat_activity"));
    let Lookup::Found(cols) = view.columns(&rel("public", "orders")) else {
        panic!("orders columns");
    };
    assert_eq!(
        cols.iter().map(|c| &*c.name).collect::<Vec<_>>(),
        ["id", "customer_id", "total"]
    );
    // A missing relation of a complete live schema is a fact; of the snapshot (same patch) too.
    assert!(matches!(
        view.relation(
            &k(&ctx, "public", NameClass::Schema),
            &k(&ctx, "hv_absent", NameClass::Table)
        ),
        Lookup::Missing(_)
    ));
    assert!(matches!(
        view.relation(
            &k(&ctx, "pg_catalog", NameClass::Schema),
            &k(&ctx, "hv_absent", NameClass::Table)
        ),
        Lookup::Missing(_)
    ));
    // The schemas: the live one and the built-in ones.
    let Lookup::Found(schemas) = view.schemas() else {
        panic!("schemas");
    };
    let pg_catalog = schemas
        .iter()
        .find(|s| &*s.name == "pg_catalog")
        .expect("pg_catalog listed");
    assert!(pg_catalog.is_system);
    assert!(schemas.iter().any(|s| &*s.name == "public" && !s.is_system));

    // MySQL 8.4: `information_schema.TABLES` (ver/01 #mysql-is-times reads CREATE_TIME).
    let mctx = my_ctx();
    let shop = ns_mysql("shop", 0).table("orders", &["id"]).finish();
    let snap = snapshot(
        DbmsKind::MySql,
        mctx.clone(),
        "8.4.3",
        vec![(shop, loaded(Level::Structure))],
        vec![],
    );
    let my = SnapshotView::new(snap)
        .with_layers(Arc::new(shipped().overlays_for(DbmsKind::MySql, "8.4.3")))
        .with_search_path(SearchPath::for_mysql(Some(&name("shop"))));
    assert!(matches!(
        my.relation(
            &k(&mctx, "information_schema", NameClass::Schema),
            &k(&mctx, "TABLES", NameClass::Table)
        ),
        Lookup::Found(_)
    ));
    let Lookup::Found(cols) = my.columns(&rel("information_schema", "TABLES")) else {
        panic!("TABLES columns");
    };
    assert!(cols.iter().any(|c| &*c.name == "CREATE_TIME"));

    // Against a newer patch than the snapshot's (8.4.13), the lists are not complete: found
    // names stay found, lists and misses are `Unknown(ServerDependent)`.
    let shop = ns_mysql("shop", 0).table("orders", &["id"]).finish();
    let snap = snapshot(
        DbmsKind::MySql,
        mctx.clone(),
        "8.4.99",
        vec![(shop, loaded(Level::Structure))],
        vec![],
    );
    let newer = SnapshotView::new(snap)
        .with_layers(Arc::new(shipped().overlays_for(DbmsKind::MySql, "8.4.99")))
        .with_search_path(SearchPath::for_mysql(Some(&name("shop"))));
    let is = k(&mctx, "information_schema", NameClass::Schema);
    assert!(matches!(
        newer.relation(&is, &k(&mctx, "TABLES", NameClass::Table)),
        Lookup::Found(_)
    ));
    assert!(matches!(
        newer.relation(&is, &k(&mctx, "HV_ABSENT", NameClass::Table)),
        Lookup::Unknown(UnknownReason::ServerDependent)
    ));
    assert!(matches!(
        newer.columns(&rel("information_schema", "TABLES")),
        Lookup::Unknown(UnknownReason::ServerDependent)
    ));
    assert!(matches!(
        newer.relations(&is),
        Lookup::Unknown(UnknownReason::ServerDependent)
    ));
}

#[cfg(feature = "system")]
#[test]
fn sql_view_lookup_allocates_nothing_on_hits() {
    let view = pg_view(
        shipped().overlays_for(DbmsKind::Postgres, "17.12"),
        loaded(Level::Structure),
    );
    let ctx = pg_ctx();
    let (sys_schema, sys_rel) = (
        k(&ctx, "pg_catalog", NameClass::Schema),
        k(&ctx, "pg_stat_activity", NameClass::Table),
    );
    let (user_schema, user_rel) = (
        k(&ctx, "public", NameClass::Schema),
        k(&ctx, "orders", NameClass::Table),
    );
    let (sys_ref, user_ref) = (
        rel("pg_catalog", "pg_stat_activity"),
        rel("public", "orders"),
    );
    // Warm: the per-namespace slices are built once.
    for _ in 0..2 {
        assert!(matches!(
            view.relation(&sys_schema, &sys_rel),
            Lookup::Found(_)
        ));
        assert!(matches!(
            view.relation(&user_schema, &user_rel),
            Lookup::Found(_)
        ));
        assert!(matches!(view.columns(&sys_ref), Lookup::Found(_)));
        assert!(matches!(view.columns(&user_ref), Lookup::Found(_)));
    }
    // `unsafe` is confined to the UI platform layer, so no counting allocator (Task 9's
    // precedent, `snapshot_perf.rs`): the view counts, per thread, every step of a lookup that
    // may allocate (a key or name built, a slice built, a fallback through the model view).
    // A hit after the warm-up must count none. **Limit:** this proves the instrumented hit
    // path was taken, not that nothing else allocates.
    let before = slow_lookups_on_this_thread();
    let hits = (
        view.relation(&sys_schema, &sys_rel),
        view.relation(&user_schema, &user_rel),
        view.columns(&sys_ref),
        view.columns(&user_ref),
    );
    let n = slow_lookups_on_this_thread() - before;
    assert!(matches!(hits.0, Lookup::Found(_)));
    assert!(matches!(hits.1, Lookup::Found(_)));
    let (Lookup::Found(c1), Lookup::Found(c2)) = (&hits.2, &hits.3) else {
        panic!("columns");
    };
    assert_eq!(
        n, 0,
        "a relation/columns hit left the allocation-free path {n} times"
    );
    // A miss does leave it (the counter is live).
    let before = slow_lookups_on_this_thread();
    let _ = view.relation(&user_schema, &k(&ctx, "hv_absent", NameClass::Table));
    assert!(slow_lookups_on_this_thread() > before);
    // The slices are shared, not rebuilt.
    let Lookup::Found(again) = view.columns(&user_ref) else {
        panic!()
    };
    assert!(Arc::ptr_eq(c2, &again));
    assert!(!c1.is_empty());
}

#[test]
fn sql_view_unknown_when_namespace_not_loaded() {
    let ctx = pg_ctx();
    let (public, orders) = (
        k(&ctx, "public", NameClass::Schema),
        k(&ctx, "orders", NameClass::Table),
    );
    let not_loaded = pg_view(Overlays::default(), LoadState::NotLoaded);
    // A cached arena still answers a hit; a miss and the column list are unknown.
    assert!(matches!(
        not_loaded.relation(&public, &k(&ctx, "hv_absent", NameClass::Table)),
        Lookup::Unknown(UnknownReason::NotLoaded)
    ));
    assert!(matches!(
        not_loaded.columns(&rel("public", "orders")),
        Lookup::Unknown(UnknownReason::NotLoaded)
    ));
    assert!(matches!(
        not_loaded.relations(&public),
        Lookup::Unknown(UnknownReason::NotLoaded)
    ));
    let out = pg_view(Overlays::default(), LoadState::OutOfScope);
    assert!(matches!(
        out.relation(&public, &orders),
        Lookup::Unknown(UnknownReason::OutOfScope)
    ));
    assert!(matches!(
        out.columns(&rel("public", "orders")),
        Lookup::Unknown(UnknownReason::OutOfScope)
    ));
    // Without a snapshot, `pg_catalog` (out of scope) says nothing.
    assert!(matches!(
        out.relation(
            &k(&ctx, "pg_catalog", NameClass::Schema),
            &k(&ctx, "pg_class", NameClass::Table)
        ),
        Lookup::Unknown(_)
    ));
    assert!(matches!(out.settings(), Lookup::Unknown(_)));
    // A schema the catalog does not list, with the list loaded: missing.
    let loaded_view = pg_view(Overlays::default(), loaded(Level::Structure));
    assert!(matches!(
        loaded_view.relation(&k(&ctx, "hv_nowhere", NameClass::Schema), &orders),
        Lookup::Missing(_)
    ));
    assert!(loaded_view.row_estimate(&rel("public", "orders")).is_none());
}

#[cfg(feature = "system")]
#[test]
fn sql_view_builtin_name_only_function_has_args_none() {
    let mctx = my_ctx();
    let shop = ns_mysql("shop", 0).table("orders", &["id"]).finish();
    let snap = snapshot(
        DbmsKind::MySql,
        mctx.clone(),
        "8.4.3",
        vec![(shop, loaded(Level::Structure))],
        vec![],
    );
    let my = SnapshotView::new(snap)
        .with_layers(Arc::new(shipped().overlays_for(DbmsKind::MySql, "8.4.3")))
        .with_search_path(SearchPath::for_mysql(Some(&name("shop"))));
    let shop_key = k(&mctx, "shop", NameClass::Schema);
    let Lookup::Found(now) = my.functions(&shop_key, &k(&mctx, "NOW", NameClass::Routine)) else {
        panic!("NOW");
    };
    let sig = now
        .iter()
        .find(|f| f.name.eq_ignore_ascii_case("now"))
        .unwrap();
    assert!(sig.is_name_only(), "{sig:?}");
    assert!(sig.args.is_none());
    // A `sys` routine keeps its real signature.
    let Lookup::Found(sys) = my.functions(
        &k(&mctx, "sys", NameClass::Schema),
        &k(&mctx, "create_synonym_db", NameClass::Routine),
    ) else {
        panic!("sys.create_synonym_db");
    };
    assert_eq!(sys[0].args.as_ref().map(Vec::len), Some(2));
    // PostgreSQL built-ins have their arguments: `now()` takes none (`Some(vec![])`).
    let view = pg_view(
        shipped().overlays_for(DbmsKind::Postgres, "17.12"),
        loaded(Level::Structure),
    );
    let ctx = pg_ctx();
    let Lookup::Found(now) = view.functions(
        &k(&ctx, "pg_catalog", NameClass::Schema),
        &k(&ctx, "now", NameClass::Routine),
    ) else {
        panic!("pg_catalog.now");
    };
    assert_eq!(now[0].args.as_deref(), Some(&[][..]));
    assert!(now[0].returns.is_some());
    // A miss: `pg_catalog`'s routines are the whole of `pg_proc` (a fact at a covered patch);
    // MySQL's built-in names are a probed subset, so a miss there is never `Missing`.
    assert!(matches!(
        view.functions(
            &k(&ctx, "pg_catalog", NameClass::Schema),
            &k(&ctx, "hv_no_such_fn", NameClass::Routine),
        ),
        Lookup::Missing(_)
    ));
    assert!(matches!(
        my.functions(&shop_key, &k(&mctx, "hv_no_such_fn", NameClass::Routine)),
        Lookup::Unknown(UnknownReason::ServerDependent)
    ));
    // The built-ins join every schema's routine list.
    let Lookup::Found(all) = my.functions_in(&shop_key) else {
        panic!("shop routines");
    };
    assert!(all.iter().any(|f| f.is_name_only() && &*f.name == "now"));
}

#[cfg(feature = "system")]
#[test]
fn settings_from_snapshot_offline() {
    let view = pg_view(
        shipped().overlays_for(DbmsKind::Postgres, "17.12"),
        loaded(Level::Structure),
    );
    let Lookup::Found(settings) = view.settings() else {
        panic!("settings");
    };
    let work_mem = settings
        .iter()
        .find(|s| &*s.name == "work_mem")
        .expect("work_mem");
    assert_eq!(work_mem.context.as_deref(), Some("user"));
    assert!(settings.windows(2).all(|w| w[0].name < w[1].name));
}

#[test]
fn sql_view_virtual_fks_are_virtual_edges_and_not_real_ones() {
    let fk = VirtualFk {
        from: hacivat_meta::ids::ObjectKey {
            kind: hacivat_meta::kind::ObjectKind::Table,
            path: [DB, "public", "orders"].into_iter().map(name).collect(),
            server_id: None,
            overload_sig: None,
            raw_kind: None,
        },
        from_cols: vec!["customer_id".into()],
        to: hacivat_meta::ids::ObjectKey {
            kind: hacivat_meta::kind::ObjectKind::Table,
            path: [DB, "public", "customers"].into_iter().map(name).collect(),
            server_id: None,
            overload_sig: None,
            raw_kind: None,
        },
        to_cols: vec!["id".into()],
        name: None,
    };
    let view = pg_view(
        Overlays {
            virtual_fks: vec![fk],
            ..Overlays::default()
        },
        loaded(Level::Structure),
    );
    let edges = view.virtual_fks(&rel("public", "orders"));
    assert_eq!(edges.len(), 1);
    assert!(edges[0].is_virtual);
    assert_eq!(edges[0].to, rel("public", "customers"));
    assert_eq!(&*edges[0].from_cols, [name("customer_id")]);
    // The other side sees the edge as well.
    assert_eq!(view.virtual_fks(&rel("public", "customers")).len(), 1);
    // The real foreign keys do not include it.
    let Lookup::Found(real) = view.foreign_keys(&rel("public", "orders")) else {
        panic!("foreign keys");
    };
    assert!(real.iter().all(|e| !e.is_virtual));
    assert!(real.is_empty());
}
