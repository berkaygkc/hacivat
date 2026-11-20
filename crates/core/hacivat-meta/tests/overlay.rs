//! M4 Task 26: the virtual layer (HV-INTRO-16 overlays; R-M4-12, D-057). User virtual foreign
//! keys (`ProfileUserData.virtual_fks`) and the shipped catalog virtual foreign keys
//! (`assets/system-catalogs/virtual-fks.toml`) are layered above the catalog with `layer =
//! Virtual`: hidden when a real foreign key with the same columns exists, kept and flagged
//! `BROKEN` when the target is certainly gone, matched by path when server ids differ
//! (imported overlays). Annotations are keyed by `ObjectKey`.

use std::sync::Arc;
use std::time::SystemTime;

use hacivat_meta::fixture::ns_pg;
use hacivat_meta::ids::{NsKey, ObjectKey, Ref, ServerId};
use hacivat_meta::kind::ObjectKind;
use hacivat_meta::name::{DialectId, IdentCtx, Name};
use hacivat_meta::namespace::{Level, NamespaceSnapshot, Obj, ObjFlags, Subtree, Watermark};
use hacivat_meta::overlay::{FkOrigin, Layer, Overlays, VirtualFk};
use hacivat_meta::props::{ColumnProps, FkAction, FkMatch, FkProps, Props, TableProps};
use hacivat_meta::search_path::SearchPath;
use hacivat_meta::snapshot::{
    Catalog, CatalogChange, CatalogSnapshot, ContainerKey, LoadState, ServerFacts,
};
use hacivat_meta::userdata::ProfileUserData;
use hacivat_meta::view::{Lookup, ModelView, SnapshotView};
use hacivat_proto::DbmsKind;

const DB: &str = "shop";

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

fn key(kind: ObjectKind, path: &[&str], oid: Option<u32>) -> ObjectKey {
    ObjectKey {
        kind,
        path: path.iter().map(|s| name(s)).collect(),
        server_id: oid.map(ServerId::Oid),
        overload_sig: None,
        raw_kind: None,
    }
}

fn table(path: &[&str], oid: Option<u32>) -> ObjectKey {
    key(ObjectKind::Table, path, oid)
}

/// `orders(id, customer_id)` (oid 100) with, when `real_fk`, a real FK `customer_id →
/// customers(id)`; `customers(id)` (oid 200) unless `dropped`.
fn shop(real_fk: bool, dropped: bool) -> NamespaceSnapshot {
    let mut orders = Subtree::new(
        Obj::new(ObjectKind::Table, "orders")
            .with_server_id(ServerId::Oid(100))
            .with_props(Props::Table(Arc::new(TableProps::default()))),
    );
    let root = orders.root();
    for (i, c) in ["id", "customer_id"].iter().enumerate() {
        orders.add(
            root,
            Obj::new(ObjectKind::Column, *c).with_props(Props::Column(Arc::new(ColumnProps {
                ordinal: i as u32 + 1,
                raw_type: name("integer"),
                nullable: true,
                ..ColumnProps::default()
            }))),
        );
    }
    if real_fk {
        orders.add(
            root,
            Obj::new(ObjectKind::ForeignKey, "orders_customer_id_fkey").with_props(
                Props::ForeignKey(Arc::new(FkProps {
                    columns: vec![name("customer_id")],
                    target: Ref {
                        kind: ObjectKind::Table,
                        path: [DB, "public", "customers"].into_iter().map(name).collect(),
                        server_id: Some(ServerId::Oid(200)),
                    },
                    target_columns: vec![name("id")],
                    on_update: FkAction::NoAction,
                    on_delete: FkAction::NoAction,
                    match_kind: FkMatch::Simple,
                    deferrable: false,
                    initially_deferred: false,
                    not_valid: false,
                    enforced: true,
                    rare: None,
                })),
            ),
        );
    }
    let mut b = ns_pg(DB, "public").subtree(orders);
    if !dropped {
        b = b.table_oid("customers", 200, &["id", "name"]);
    }
    b.finish()
}

fn snapshot(ns: NamespaceSnapshot) -> Arc<CatalogSnapshot> {
    let ctx = IdentCtx {
        encoding: Some(Arc::from("UTF8")),
        ..IdentCtx::new(DialectId::Postgres)
    };
    let mut facts = ServerFacts::new(DbmsKind::Postgres, ctx);
    facts.version = name("17.12");
    let catalog = Catalog::new(Arc::new(facts));
    let publisher = catalog.publisher().unwrap();
    publisher
        .publish(vec![
            CatalogChange::SetState(
                ContainerKey::Namespace(ns.key().clone()),
                loaded(Level::Structure),
            ),
            CatalogChange::Put(Arc::new(ns)),
            CatalogChange::SetState(
                ContainerKey::schema_list(Some(&name(DB))),
                loaded(Level::Names),
            ),
        ])
        .unwrap();
    catalog.current()
}

fn user_fk(from_oid: Option<u32>, to_oid: Option<u32>) -> VirtualFk {
    VirtualFk {
        from: table(&[DB, "public", "orders"], from_oid),
        from_cols: vec!["customer_id".into()],
        to: table(&[DB, "public", "customers"], to_oid),
        to_cols: vec!["id".into()],
        name: Some("orders_customer_vfk".into()),
    }
}

fn view(ns: NamespaceSnapshot, fks: Vec<VirtualFk>) -> SnapshotView {
    let layers = Overlays {
        virtual_fks: fks,
        ..Overlays::default()
    };
    SnapshotView::new(snapshot(ns))
        .with_layers(Arc::new(layers))
        .with_search_path(SearchPath::exact(
            Some(name(DB)),
            [NsKey::schema([DB, "public"])],
        ))
}

fn orders() -> ObjectKey {
    table(&[DB, "public", "orders"], Some(100))
}

#[test]
fn virtual_fk_is_on_the_virtual_layer() {
    let v = view(shop(false, false), vec![user_fk(Some(100), Some(200))]);
    let fks = v.virtual_fks_of(&orders());
    assert_eq!(fks.len(), 1);
    let fk = &fks[0];
    assert_eq!(fk.layer, Layer::Virtual);
    assert_eq!(fk.origin, FkOrigin::User);
    assert!(!fk.flags.contains(ObjFlags::BROKEN));
    let Lookup::Found(target) = &fk.target else {
        panic!("{:?}", fk.target)
    };
    assert_eq!(&**target.obj().name(), "customers");
    // A virtual FK is never a real one: the model's FK children do not show it (and so the
    // DDL producers, which read the model, never emit it; Task 19's test).
    let Lookup::Found(real) = v.children(&orders(), ObjectKind::ForeignKey) else {
        panic!("orders' FK children");
    };
    assert!(real.is_empty());
    // The other side sees it too.
    let incoming = v.virtual_fks_to(&table(&[DB, "public", "customers"], Some(200)));
    assert_eq!(incoming.len(), 1);
}

#[test]
fn real_fk_hides_a_duplicate_virtual_fk() {
    let v = view(shop(true, false), vec![user_fk(Some(100), Some(200))]);
    assert!(v.virtual_fks_of(&orders()).is_empty());
    // Other columns are not a duplicate.
    let other = VirtualFk {
        from_cols: vec!["id".into()],
        ..user_fk(Some(100), Some(200))
    };
    let v = view(shop(true, false), vec![other]);
    assert_eq!(v.virtual_fks_of(&orders()).len(), 1);
}

#[test]
fn virtual_fk_to_a_dropped_table_is_kept_broken() {
    let v = view(shop(false, true), vec![user_fk(Some(100), Some(200))]);
    let fks = v.virtual_fks_of(&orders());
    assert_eq!(fks.len(), 1, "kept");
    assert!(fks[0].flags.contains(ObjFlags::BROKEN));
    assert!(matches!(fks[0].target, Lookup::Missing(_)));
    // A target whose namespace is not loaded is not broken: nothing is certain.
    let ns = shop(false, false);
    let snap = {
        let ctx = IdentCtx::new(DialectId::Postgres);
        let catalog = Catalog::new(Arc::new(ServerFacts::new(DbmsKind::Postgres, ctx)));
        let publisher = catalog.publisher().unwrap();
        publisher
            .publish(vec![
                CatalogChange::SetState(
                    ContainerKey::Namespace(ns.key().clone()),
                    LoadState::NotLoaded,
                ),
                CatalogChange::Put(Arc::new(ns)),
            ])
            .unwrap();
        catalog.current()
    };
    let far = VirtualFk {
        to: table(&[DB, "other", "customers"], None),
        ..user_fk(None, None)
    };
    let v = SnapshotView::new(snap).with_layers(Arc::new(Overlays {
        virtual_fks: vec![far],
        ..Overlays::default()
    }));
    let fks = v.virtual_fks_of(&orders());
    assert_eq!(fks.len(), 1);
    assert!(!fks[0].flags.contains(ObjFlags::BROKEN));
    assert!(matches!(fks[0].target, Lookup::Unknown(_)));
}

#[test]
fn imported_overlay_with_other_server_ids_matches_by_path() {
    // Exported from another server: the same paths, other OIDs.
    let v = view(shop(false, false), vec![user_fk(Some(9_100), Some(9_200))]);
    let fks = v.virtual_fks_of(&orders());
    assert_eq!(fks.len(), 1);
    let Lookup::Found(target) = &fks[0].target else {
        panic!("{:?}", fks[0].target)
    };
    assert_eq!(target.obj().server_id(), Some(ServerId::Oid(200)));
    assert!(!fks[0].flags.contains(ObjFlags::BROKEN));
    // And a real FK still hides it.
    let v = view(shop(true, false), vec![user_fk(Some(9_100), Some(9_200))]);
    assert!(v.virtual_fks_of(&orders()).is_empty());
    // A renamed table keeps its virtual FK by server id.
    let renamed = VirtualFk {
        from: table(&[DB, "public", "orders_old"], Some(100)),
        ..user_fk(Some(100), Some(200))
    };
    let v = view(shop(false, false), vec![renamed]);
    assert_eq!(v.virtual_fks_of(&orders()).len(), 1);
}

#[test]
fn user_data_layers_virtual_fks_and_annotations() {
    let mut data = ProfileUserData::default();
    data.virtual_fks.push(user_fk(Some(100), Some(200)));
    data.annotations.insert(
        table(&[DB, "public", "orders"], Some(9_100)),
        "Siparişler".into(),
    );
    let layers = Overlays::default().with_user_data(&data);
    assert_eq!(layers.virtual_fks.len(), 1);
    // An annotation stored under another server's id is found by path.
    assert_eq!(
        layers.annotation(&orders()).map(|n| &**n),
        Some("Siparişler")
    );
    assert_eq!(
        layers.annotation(&table(&[DB, "public", "customers"], Some(200))),
        None
    );
}

/// Review m-7: MySQL/MariaDB column names are caseless, so a real foreign key whose column
/// spelling differs only in ASCII case still hides the duplicate virtual one.
#[test]
fn mysql_real_fk_hides_a_virtual_fk_of_other_column_case() {
    let mut orders = Subtree::new(
        Obj::new(ObjectKind::Table, "orders")
            .with_props(Props::Table(Arc::new(TableProps::default()))),
    );
    let root = orders.root();
    orders.add(
        root,
        Obj::new(ObjectKind::Column, "Customer_Id").with_props(Props::Column(Arc::new(
            ColumnProps {
                ordinal: 1,
                raw_type: name("int"),
                nullable: true,
                ..ColumnProps::default()
            },
        ))),
    );
    orders.add(
        root,
        Obj::new(ObjectKind::ForeignKey, "fk_customer").with_props(Props::ForeignKey(Arc::new(
            FkProps {
                columns: vec![name("Customer_Id")],
                target: Ref {
                    kind: ObjectKind::Table,
                    path: ["shop", "customers"].into_iter().map(name).collect(),
                    server_id: None,
                },
                target_columns: vec![name("ID")],
                on_update: FkAction::NoAction,
                on_delete: FkAction::NoAction,
                match_kind: FkMatch::Simple,
                deferrable: false,
                initially_deferred: false,
                not_valid: false,
                enforced: true,
                rare: None,
            },
        ))),
    );
    let ns = hacivat_meta::fixture::ns_mysql("shop", 0)
        .subtree(orders)
        .table("customers", &["id"])
        .finish();
    let catalog = Catalog::new(Arc::new(ServerFacts::new(
        DbmsKind::MySql,
        IdentCtx::new(DialectId::MySql),
    )));
    catalog
        .publisher()
        .unwrap()
        .publish(vec![
            CatalogChange::SetState(
                ContainerKey::Namespace(ns.key().clone()),
                loaded(Level::Structure),
            ),
            CatalogChange::Put(Arc::new(ns)),
            CatalogChange::SetState(ContainerKey::schema_list(None), loaded(Level::Names)),
        ])
        .unwrap();
    let fk = VirtualFk {
        from: table(&["shop", "orders"], None),
        from_cols: vec!["customer_id".into()],
        to: table(&["shop", "customers"], None),
        to_cols: vec!["id".into()],
        name: None,
    };
    let v = SnapshotView::new(catalog.current()).with_layers(Arc::new(Overlays {
        virtual_fks: vec![fk],
        ..Overlays::default()
    }));
    assert!(
        v.virtual_fks_of(&table(&["shop", "orders"], None))
            .is_empty()
    );
}

#[cfg(feature = "system")]
#[test]
fn shipped_virtual_fks_resolve_against_every_pg_snapshot() {
    use hacivat_meta::system::SystemCatalogs;
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../assets/system-catalogs");
    let set = SystemCatalogs::open(&dir).unwrap();
    let rows = set.shipped_fks(DbmsKind::Postgres);
    assert!(rows.len() >= 20, "{} rows", rows.len());
    for version in ["14.1", "15.1", "16.1", "17.1", "18.1"] {
        let sys = set.for_server(DbmsKind::Postgres, version).unwrap();
        for row in rows.iter() {
            assert!(!row.source.trim().is_empty(), "a row without a source");
            for (rel, cols) in [
                (&row.fk.from, &row.fk.from_cols),
                (&row.fk.to, &row.fk.to_cols),
            ] {
                let [schema, rel_name] = rel.path.as_slice() else {
                    panic!("a shipped row names schema.relation: {rel:?}");
                };
                let ns = sys
                    .namespace(schema)
                    .unwrap_or_else(|| panic!("{version}: no schema {schema}"));
                let idx = *ns
                    .lookup(hacivat_meta::kind::KindGroup::Relations, rel_name)
                    .first()
                    .unwrap_or_else(|| panic!("{version}: no {schema}.{rel_name}"));
                for col in cols {
                    assert!(
                        ns.children_of_kind(idx, ObjectKind::Column)
                            .any(|(_, o)| &**o.name() == col.as_str()),
                        "{version}: {schema}.{rel_name} has no column {col}"
                    );
                }
            }
        }
    }
    // Layered over an offline catalog, `pg_class` shows them with a found target.
    let layers = set.overlays_for(DbmsKind::Postgres, "17.12");
    let v = SnapshotView::new(snapshot(shop(false, false)))
        .with_layers(Arc::new(layers))
        .with_search_path(SearchPath::exact(
            Some(name(DB)),
            [NsKey::schema([DB, "public"])],
        ));
    let pg_class = table(&[DB, "pg_catalog", "pg_class"], None);
    let fks = v.virtual_fks_of(&pg_class);
    assert!(!fks.is_empty());
    for fk in &fks {
        assert_eq!(fk.origin, FkOrigin::Shipped);
        assert!(matches!(fk.target, Lookup::Found(_)), "{:?}", fk.fk);
        assert!(fk.source.as_deref().is_some_and(|s| !s.is_empty()));
    }
    let namespace = fks
        .iter()
        .find(|f| f.fk.from_cols == ["relnamespace"])
        .expect("pg_class.relnamespace → pg_namespace.oid");
    assert_eq!(namespace.fk.to.name(), "pg_namespace");
    assert_eq!(namespace.fk.to_cols, ["oid"]);
}

#[cfg(feature = "system")]
#[test]
fn a_shipped_row_without_a_source_is_refused() {
    use hacivat_meta::system::SystemCatalogs;
    let src = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../assets/system-catalogs");
    for bad in [
        // No source.
        "[[fk]]\ndbms = \"postgres\"\nfrom = \"pg_catalog.pg_class\"\nfrom_columns = [\"relnamespace\"]\n\
         to = \"pg_catalog.pg_namespace\"\nto_columns = [\"oid\"]\n",
        // An empty source.
        "[[fk]]\ndbms = \"postgres\"\nfrom = \"pg_catalog.pg_class\"\nfrom_columns = [\"relnamespace\"]\n\
         to = \"pg_catalog.pg_namespace\"\nto_columns = [\"oid\"]\nsource = \"  \"\n",
        // Column lists of different lengths.
        "[[fk]]\ndbms = \"postgres\"\nfrom = \"pg_catalog.pg_class\"\nfrom_columns = [\"a\", \"b\"]\n\
         to = \"pg_catalog.pg_namespace\"\nto_columns = [\"oid\"]\nsource = \"doc\"\n",
        // An unknown key.
        "[[fk]]\ndbms = \"postgres\"\nfrom = \"pg_catalog.pg_class\"\nfrom_columns = [\"relnamespace\"]\n\
         to = \"pg_catalog.pg_namespace\"\nto_columns = [\"oid\"]\nsource = \"doc\"\nguess = \"x\"\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::copy(src.join("manifest.toml"), dir.path().join("manifest.toml")).unwrap();
        std::fs::write(dir.path().join("virtual-fks.toml"), bad).unwrap();
        assert!(SystemCatalogs::open(dir.path()).is_err(), "{bad}");
    }
    // The control: the same row with its source is read.
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(src.join("manifest.toml"), dir.path().join("manifest.toml")).unwrap();
    std::fs::write(
        dir.path().join("virtual-fks.toml"),
        "[[fk]]\ndbms = \"postgres\"\nfrom = \"pg_catalog.pg_class\"\nfrom_columns = [\"relnamespace\"]\n\
         to = \"pg_catalog.pg_namespace\"\nto_columns = [\"oid\"]\nsource = \"doc\"\n",
    )
    .unwrap();
    let set = SystemCatalogs::open(dir.path()).unwrap();
    assert_eq!(set.shipped_fks(DbmsKind::Postgres).len(), 1);
}
