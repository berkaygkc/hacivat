//! M4 Task 26: the system-catalog layer (HV-INTRO-16 consumer side; R-M4-6, R-M4-17). The
//! committed snapshot set in `assets/system-catalogs` is opened, chosen by the exact rule
//! (PostgreSQL by major, MySQL and MariaDB by major.minor, never SQLite), decoded lazily once
//! per file, and layered under `SnapshotView`: a system namespace answers from the snapshot
//! unless the user chose to introspect it live; a miss against a MySQL/MariaDB patch newer
//! than the snapshot's is `Unknown`, never `Missing` (alg/02 HV-INTRO-16 edge 1), while a
//! PostgreSQL snapshot covers every release of its major (ruling R-W5-3).

#![cfg(feature = "system")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use hacivat_meta::fixture::{ns_mysql, ns_pg};
use hacivat_meta::ids::{NsKey, ObjectKey};
use hacivat_meta::kind::{KindGroup, ObjectKind};
use hacivat_meta::name::{DialectId, IdentCtx, Name, NamePart};
use hacivat_meta::namespace::{Level, NamespaceSnapshot, ObjFlags, Watermark};
use hacivat_meta::overlay::Overlays;
use hacivat_meta::search_path::SearchPath;
use hacivat_meta::snapshot::{
    Catalog, CatalogChange, CatalogSnapshot, ContainerKey, LoadState, ObjHandle, ServerFacts,
};
use hacivat_meta::system::SystemCatalogs;
use hacivat_meta::view::{Lookup, MissingReason, ModelView, SnapshotView, UnknownReason};
use hacivat_proto::DbmsKind;

const DB: &str = "app";

fn assets_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../assets/system-catalogs")
}

fn sys() -> SystemCatalogs {
    SystemCatalogs::open(&assets_dir()).expect("the committed snapshot set opens")
}

fn name(s: &str) -> Name {
    Arc::from(s)
}

fn p(dotted: &str) -> Vec<NamePart> {
    dotted
        .split('.')
        .map(|t| NamePart {
            text: Arc::from(t),
            quoted: false,
        })
        .collect()
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

/// A PostgreSQL catalog of server `version` with `nss` published, `states` for namespaces
/// without an arena, and the schema list of `DB` loaded. The server is proven to hold nothing
/// beyond initdb in its system schemas (R-W5-6); [`pg_snapshot_extras`] sets that fact.
fn pg_snapshot(
    version: &str,
    nss: Vec<(NamespaceSnapshot, LoadState)>,
    states: Vec<(NsKey, LoadState)>,
) -> Arc<CatalogSnapshot> {
    pg_snapshot_extras(version, Some(false), nss, states)
}

fn pg_snapshot_extras(
    version: &str,
    extras: Option<bool>,
    nss: Vec<(NamespaceSnapshot, LoadState)>,
    states: Vec<(NsKey, LoadState)>,
) -> Arc<CatalogSnapshot> {
    let mut facts = ServerFacts::new(DbmsKind::Postgres, pg_ctx());
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
    for (key, state) in states {
        changes.push(CatalogChange::SetState(ContainerKey::Namespace(key), state));
    }
    changes.push(CatalogChange::SetState(
        ContainerKey::schema_list(Some(&name(DB))),
        loaded(Level::Names),
    ));
    publisher.publish(changes).unwrap();
    catalog.current()
}

fn pg_path() -> SearchPath {
    SearchPath::for_pg(
        &name(DB),
        &[name("$user"), name("public")],
        Some(&name("alice")),
        None,
    )
}

/// The view of an offline PostgreSQL catalog of server `version`: `public` loaded and empty
/// but for `orders`, `pg_catalog` out of scope (system schemas are never introspected live by
/// default, R-M4-14), the layers of the shipped set for that version.
fn offline_pg(version: &str) -> SnapshotView {
    let public = ns_pg(DB, "public").table("orders", &["id"]).finish();
    let snap = pg_snapshot(
        version,
        vec![(public, loaded(Level::Structure))],
        vec![
            (NsKey::schema([DB, "pg_catalog"]), LoadState::OutOfScope),
            (
                NsKey::schema([DB, "information_schema"]),
                LoadState::OutOfScope,
            ),
        ],
    );
    let layers = sys().overlays_for(DbmsKind::Postgres, version);
    SnapshotView::new(snap)
        .with_layers(Arc::new(layers))
        .with_search_path(pg_path())
}

fn column_names(l: Lookup<Vec<ObjHandle>>) -> Vec<String> {
    match l {
        Lookup::Found(hs) => hs.iter().map(|h| h.obj().name().to_string()).collect(),
        other => panic!("columns not found: {other:?}"),
    }
}

fn key_of(l: &Lookup<ObjHandle>) -> ObjectKey {
    match l {
        Lookup::Found(h) => h.key().expect("a key"),
        other => panic!("not found: {other:?}"),
    }
}

#[test]
fn pg_stat_activity_columns_found_offline_with_pg_catalog_out_of_scope() {
    let view = offline_pg("17.12");
    let hit = view.lookup(&p("pg_catalog.pg_stat_activity"), KindGroup::Relations);
    let Lookup::Found(h) = &hit else {
        panic!("pg_stat_activity: {hit:?}");
    };
    assert!(
        h.obj()
            .flags()
            .contains(ObjFlags::PRE_INTROSPECTED | ObjFlags::SYSTEM)
    );
    let key = key_of(&hit);
    // The handle's key is the live form of the path, so it can be asked for its children.
    assert_eq!(
        key.path.iter().map(|n| &**n).collect::<Vec<_>>(),
        [DB, "pg_catalog", "pg_stat_activity"]
    );
    let cols = column_names(view.children(&key, ObjectKind::Column));
    assert!(cols.iter().any(|c| c == "pid"), "{cols:?}");
    // Unqualified: `pg_catalog` comes first on the path (implicitly), so it is found there.
    assert!(matches!(
        view.lookup(&p("pg_stat_activity"), KindGroup::Relations),
        Lookup::Found(_)
    ));
    // `information_schema` comes from the snapshot too.
    assert!(matches!(
        view.lookup(&p("information_schema.columns"), KindGroup::Relations),
        Lookup::Found(_)
    ));
    // Without the layer, the same catalog says nothing about `pg_catalog`.
    let bare = SnapshotView::new(Arc::clone(view.snapshot())).with_search_path(pg_path());
    assert!(matches!(
        bare.lookup(&p("pg_catalog.pg_stat_activity"), KindGroup::Relations),
        Lookup::Unknown(_)
    ));
}

#[test]
fn pg17_snapshot_serves_17_11_with_its_version_label() {
    let set = sys();
    let s = set.for_server(DbmsKind::Postgres, "17.11").expect("pg17");
    assert_eq!(&*s.line, "17");
    assert_eq!(s.dbms, DbmsKind::Postgres);
    // The packaged forms of a version string pick the same line.
    let debian = set
        .for_server(DbmsKind::Postgres, "17.2 (Debian 17.2-1.pgdg120+1)")
        .expect("pg17 from a packaged version string");
    assert!(Arc::ptr_eq(&s, &debian));
    // The layer carries the label the explorer's Built-in badge shows.
    let view = offline_pg("17.11");
    assert_eq!(view.system_label().as_deref(), Some("17"));
}

#[test]
fn pg13_has_no_snapshot() {
    assert!(sys().for_server(DbmsKind::Postgres, "13.22").is_none());
}

#[test]
fn mariadb_10_11_uses_its_own_snapshot_not_10_6() {
    let set = sys();
    let s = set
        .for_server(DbmsKind::MariaDb, "10.11.9-MariaDB-ubu2204")
        .expect("mariadb 10.11");
    assert_eq!(&*s.line, "10.11");
    // The replication prefix some MariaDB servers report.
    let prefixed = set
        .for_server(DbmsKind::MariaDb, "5.5.5-10.11.9-MariaDB")
        .expect("mariadb 10.11 behind the 5.5.5- prefix");
    assert!(Arc::ptr_eq(&s, &prefixed));
    let s106 = set.for_server(DbmsKind::MariaDb, "10.6.20").expect("10.6");
    assert_eq!(&*s106.line, "10.6");
    // Exact selection: no closest line for a version between two snapshots.
    assert!(set.for_server(DbmsKind::MariaDb, "10.8.8").is_none());
    assert!(set.for_server(DbmsKind::MariaDb, "11.8.1").is_none());
    // A MariaDB version string is not a MySQL line.
    assert!(set.for_server(DbmsKind::MySql, "10.11.9").is_none());
}

#[test]
fn mysql_9_has_no_snapshot() {
    let set = sys();
    assert!(set.for_server(DbmsKind::MySql, "9.1.0").is_none());
    assert!(set.for_server(DbmsKind::MySql, "8.1.0").is_none());
    assert_eq!(
        &*set.for_server(DbmsKind::MySql, "8.4.3").expect("8.4").line,
        "8.4"
    );
    assert_eq!(
        &*set
            .for_server(DbmsKind::MySql, "8.0.40-log")
            .expect("8.0")
            .line,
        "8.0"
    );
}

#[test]
fn sqlite_never_loads_a_snapshot() {
    let set = sys();
    assert!(set.for_server(DbmsKind::Sqlite, "3.46.0").is_none());
    assert!(set.for_server(DbmsKind::Sqlite, "17.11").is_none());
    assert_eq!(set.decoded_count(), 0);
    let layers = set.overlays_for(DbmsKind::Sqlite, "3.46.0");
    assert!(layers.system.is_none());
    assert!(layers.shipped_fks.is_empty());
}

/// An offline MySQL 8.4 catalog of server `version`: `shop` loaded with `orders`, the layers of
/// the shipped set for that version, `shop` the current database.
fn offline_my(version: &str) -> SnapshotView {
    let ctx = IdentCtx::new(DialectId::MySql);
    let mut facts = ServerFacts::new(DbmsKind::MySql, ctx);
    facts.version = name(version);
    let catalog = Catalog::new(Arc::new(facts));
    let shop = ns_mysql("shop", 0).table("orders", &["id"]).finish();
    catalog
        .publisher()
        .unwrap()
        .publish(vec![
            CatalogChange::SetState(
                ContainerKey::Namespace(shop.key().clone()),
                loaded(Level::Structure),
            ),
            CatalogChange::Put(Arc::new(shop)),
            CatalogChange::SetState(ContainerKey::schema_list(None), loaded(Level::Names)),
        ])
        .unwrap();
    SnapshotView::new(catalog.current())
        .with_layers(Arc::new(sys().overlays_for(DbmsKind::MySql, version)))
        .with_search_path(SearchPath::for_mysql(Some(&name("shop"))))
}

/// MySQL/MariaDB (alg/02 HV-INTRO-16 edge 1; rulings R-W5-3, R-W5-6): a name the snapshot
/// lacks is never `Missing` (plugins and managed services add system tables and routines), at
/// any patch. The column list of a table the snapshot holds is complete only up to the
/// snapshot's patch (a newer patch may add a column).
#[test]
fn newer_patch_missing_column_is_unknown_not_missing() {
    // The MySQL 8.4 snapshot was read from 8.4.13.
    let newer = offline_my("8.4.14");
    let same = offline_my("8.4.13");
    let older = offline_my("8.4.3");
    // A relation the snapshot holds is found whatever the patch.
    for v in [&newer, &same, &older] {
        assert!(matches!(
            v.lookup(&p("information_schema.TABLES"), KindGroup::Relations),
            Lookup::Found(_)
        ));
    }
    // A name the snapshot lacks: never a fact on MySQL (R-W5-6).
    let absent = p("information_schema.HV_NO_SUCH_VIEW");
    for v in [&newer, &same, &older] {
        assert!(matches!(
            v.lookup(&absent, KindGroup::Relations),
            Lookup::Unknown(UnknownReason::ServerDependent)
        ));
        assert!(matches!(
            v.lookup(&p("mysql.rds_heartbeat2"), KindGroup::Relations),
            Lookup::Unknown(UnknownReason::ServerDependent)
        ));
    }
    // The column list of a relation the snapshot holds: complete up to the snapshot's patch;
    // a newer patch may have added a column, so a column the list lacks is not a fact there.
    let tables = key_of(&same.lookup(&p("information_schema.TABLES"), KindGroup::Relations));
    let cols = column_names(same.children(&tables, ObjectKind::Column));
    assert!(cols.iter().any(|c| c == "CREATE_TIME"));
    assert!(matches!(
        newer.children(&tables, ObjectKind::Column),
        Lookup::Unknown(UnknownReason::ServerDependent)
    ));
    // The children of a relation the snapshot lacks, and the schema's object list.
    let ghost = ObjectKey {
        kind: ObjectKind::View,
        path: ["information_schema", "HV_NO_SUCH_VIEW"]
            .into_iter()
            .map(name)
            .collect(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    };
    let schema = ObjectKey {
        kind: ObjectKind::Schema,
        path: ["mysql"].into_iter().map(name).collect(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    };
    for v in [&newer, &same] {
        assert!(matches!(
            v.children(&ghost, ObjectKind::Column),
            Lookup::Unknown(UnknownReason::ServerDependent)
        ));
        assert!(matches!(
            v.children(&schema, ObjectKind::Table),
            Lookup::Unknown(UnknownReason::ServerDependent)
        ));
    }
}

/// Review I-1: MySQL compares the `information_schema` name and its table names
/// case-insensitively whatever `lower_case_table_names` is.
#[test]
fn mysql_information_schema_is_found_in_any_case() {
    let view = offline_my("8.4.3");
    for path in [
        "information_schema.TABLES",
        "information_schema.tables",
        "INFORMATION_SCHEMA.TABLES",
        "Information_Schema.Columns",
    ] {
        let hit = view.lookup(&p(path), KindGroup::Relations);
        let key = key_of(&hit);
        assert_eq!(key.path[0].as_ref(), "information_schema", "{path}");
        assert!(
            column_names(view.children(&key, ObjectKind::Column)).len() > 5,
            "{path}"
        );
    }
    // The other system schemas stay case-sensitive under lctn = 0 (the server has no
    // database `MYSQL` there): not found.
    assert!(!matches!(
        view.lookup(&p("MYSQL.user"), KindGroup::Relations),
        Lookup::Found(_)
    ));
    // The parent key in another case reaches its columns too.
    let upper = ObjectKey {
        kind: ObjectKind::Table,
        path: ["INFORMATION_SCHEMA", "tables"]
            .into_iter()
            .map(name)
            .collect(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    };
    assert!(column_names(view.children(&upper, ObjectKind::Column)).len() > 5);
}

/// The offline PostgreSQL view of a server reporting `version`, with the layers the shipped set
/// gives `layer_version` (a mismatched pair stands for layers attached to the wrong server).
fn offline_pg_layered(version: &str, layer_version: &str) -> SnapshotView {
    let public = ns_pg(DB, "public").table("orders", &["id"]).finish();
    let snap = pg_snapshot(
        version,
        vec![(public, loaded(Level::Structure))],
        vec![(NsKey::schema([DB, "pg_catalog"]), LoadState::OutOfScope)],
    );
    SnapshotView::new(snap)
        .with_layers(Arc::new(
            sys().overlays_for(DbmsKind::Postgres, layer_version),
        ))
        .with_search_path(pg_path())
}

/// Ruling R-W5-3: PostgreSQL keeps its system catalogs fixed within a major version
/// (`catversion` changes only between majors; a minor release never needs `initdb`), so the
/// snapshot of a major answers for every release of it, newer patches included. A server of
/// another major, or a development or beta build, is not covered.
#[test]
fn pg_snapshot_covers_every_release_of_its_major() {
    // The pg17 snapshot was read from 17.12.
    let absent = p("pg_catalog.hv_no_such_view");
    for v in ["17.0", "17.11", "17.12", "17.13", "17.99"] {
        let view = offline_pg(v);
        assert!(
            matches!(
                view.lookup(&absent, KindGroup::Relations),
                Lookup::Missing(MissingReason::NotFound)
            ),
            "{v}"
        );
        let pg_class = key_of(&view.lookup(&p("pg_catalog.pg_class"), KindGroup::Relations));
        let cols = column_names(view.children(&pg_class, ObjectKind::Column));
        assert!(cols.iter().any(|c| c == "relnamespace"), "{v}");
    }
    // A development build or a beta: its catalog version is not the release's.
    for v in ["17devel", "17beta1", "17rc1", ""] {
        assert!(
            matches!(
                offline_pg_layered(v, "17.12").lookup(&absent, KindGroup::Relations),
                Lookup::Unknown(UnknownReason::ServerDependent)
            ),
            "{v:?}"
        );
    }
    // Layers of another major attached to a server: nothing certain.
    let wrong = offline_pg_layered("16.4", "17.12");
    assert!(matches!(
        wrong.lookup(&absent, KindGroup::Relations),
        Lookup::Unknown(UnknownReason::ServerDependent)
    ));
    let pg_class = ObjectKey {
        kind: ObjectKind::Table,
        path: [DB, "pg_catalog", "pg_class"]
            .into_iter()
            .map(name)
            .collect(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    };
    assert!(matches!(
        wrong.children(&pg_class, ObjectKind::Column),
        Lookup::Unknown(UnknownReason::ServerDependent)
    ));
}

#[test]
fn unqualified_user_table_resolves_past_the_system_pg_catalog() {
    // R-W2-6's cost (carries-w2 T26), with R-W5-3: with `pg_catalog` served by the snapshot of
    // the server's major, an unqualified user table walks past it at any release of that major.
    for v in ["17.12", "17.13"] {
        let view = offline_pg(v);
        assert!(matches!(
            view.lookup(&p("orders"), KindGroup::Relations),
            Lookup::Found(_)
        ));
        assert!(matches!(
            view.lookup(&p("hv_absent"), KindGroup::Relations),
            Lookup::Missing(MissingReason::NotFound)
        ));
    }
    // A build the snapshot does not cover: the `pg_catalog` miss is not a fact, so the walk
    // stops there (the server could resolve the name in `pg_catalog`).
    assert!(matches!(
        offline_pg_layered("17devel", "17.12").lookup(&p("orders"), KindGroup::Relations),
        Lookup::Unknown(UnknownReason::ServerDependent)
    ));
}

#[test]
fn pg_catalog_shadows_a_public_table_of_the_same_name() {
    // T9's carry: `pg_catalog` is searched first, so its `pg_class` wins over `public.pg_class`.
    let public = ns_pg(DB, "public").table("pg_class", &["mine"]).finish();
    let snap = pg_snapshot(
        "17.12",
        vec![(public, loaded(Level::Structure))],
        vec![(NsKey::schema([DB, "pg_catalog"]), LoadState::OutOfScope)],
    );
    let view = SnapshotView::new(snap)
        .with_layers(Arc::new(sys().overlays_for(DbmsKind::Postgres, "17.12")))
        .with_search_path(pg_path());
    let hit = view.lookup(&p("pg_class"), KindGroup::Relations);
    let key = key_of(&hit);
    assert_eq!(&*key.path[1], "pg_catalog");
    let cols = column_names(view.children(&key, ObjectKind::Column));
    assert!(cols.iter().any(|c| c == "relname"));
    assert!(!cols.iter().any(|c| c == "mine"));
    // Qualified, the user's table is still there.
    assert!(matches!(
        view.lookup(&p("public.pg_class"), KindGroup::Relations),
        Lookup::Found(_)
    ));
}

#[test]
fn snapshot_is_decoded_once_and_shared() {
    let set = sys();
    assert_eq!(set.decoded_count(), 0, "opening decodes nothing");
    let a = set.for_server(DbmsKind::Postgres, "17.1").unwrap();
    assert_eq!(set.decoded_count(), 1);
    let b = set.for_server(DbmsKind::Postgres, "17.12").unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(set.decoded_count(), 1);
    // A clone of the set shares the decoded files.
    let clone = set.clone();
    assert!(Arc::ptr_eq(
        &a,
        &clone.for_server(DbmsKind::Postgres, "17.5").unwrap()
    ));
    assert_eq!(clone.decoded_count(), 1);
    set.for_server(DbmsKind::Postgres, "16.3").unwrap();
    assert_eq!(set.decoded_count(), 2);
    // Two layers of the same version share the snapshot.
    let l1 = set.overlays_for(DbmsKind::Postgres, "17.12");
    let l2 = set.overlays_for(DbmsKind::Postgres, "17.2");
    assert!(Arc::ptr_eq(
        l1.system.as_ref().unwrap(),
        l2.system.as_ref().unwrap()
    ));
}

#[test]
fn live_override_takes_precedence_over_the_snapshot() {
    // A live read of `pg_catalog` (the user chose "Introspect live" for it).
    let live = ns_pg(DB, "pg_catalog")
        .table("pg_class", &["live_only"])
        .finish();
    let public = ns_pg(DB, "public").finish();
    let snap = pg_snapshot(
        "17.12",
        vec![
            (live, loaded(Level::Structure)),
            (public, loaded(Level::Structure)),
        ],
        vec![],
    );
    let layers = sys().overlays_for(DbmsKind::Postgres, "17.12");
    // Not chosen: the snapshot answers, even though a live arena is in the catalog.
    let from_snapshot = SnapshotView::new(Arc::clone(&snap))
        .with_layers(Arc::new(layers.clone()))
        .with_search_path(pg_path());
    let hit = from_snapshot.lookup(&p("pg_catalog.pg_class"), KindGroup::Relations);
    let key = key_of(&hit);
    let cols = column_names(from_snapshot.children(&key, ObjectKind::Column));
    assert!(cols.iter().any(|c| c == "relname"));
    // Chosen: the live namespace answers, and its objects are not pre-introspected.
    let mut chosen = layers;
    chosen.live_system.insert(NsKey::schema([DB, "pg_catalog"]));
    let live_view = SnapshotView::new(snap)
        .with_layers(Arc::new(chosen))
        .with_search_path(pg_path());
    let hit = live_view.lookup(&p("pg_catalog.pg_class"), KindGroup::Relations);
    let Lookup::Found(h) = &hit else {
        panic!("{hit:?}")
    };
    assert!(!h.obj().flags().contains(ObjFlags::PRE_INTROSPECTED));
    assert_eq!(
        column_names(live_view.children(&key_of(&hit), ObjectKind::Column)),
        ["live_only"]
    );
    // A system relation the live read lacks is a live fact now.
    assert!(matches!(
        live_view.lookup(&p("pg_catalog.pg_stat_activity"), KindGroup::Relations),
        Lookup::Missing(_)
    ));
}

#[test]
fn a_tampered_or_truncated_snapshot_serves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    for entry in std::fs::read_dir(assets_dir()).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), dir.path().join(entry.file_name())).unwrap();
    }
    // One byte changed: the manifest digest no longer matches.
    let pg16 = dir.path().join("postgres-16.hvsnap.zst");
    let mut bytes = std::fs::read(&pg16).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    std::fs::write(&pg16, &bytes).unwrap();
    // Truncated.
    let pg15 = dir.path().join("postgres-15.hvsnap.zst");
    let bytes = std::fs::read(&pg15).unwrap();
    std::fs::write(&pg15, &bytes[..bytes.len() / 2]).unwrap();
    // Gone.
    std::fs::remove_file(dir.path().join("postgres-14.hvsnap.zst")).unwrap();
    let set = SystemCatalogs::open(dir.path()).unwrap();
    assert!(set.for_server(DbmsKind::Postgres, "16.1").is_none());
    assert!(set.for_server(DbmsKind::Postgres, "15.1").is_none());
    assert!(set.for_server(DbmsKind::Postgres, "14.1").is_none());
    assert!(set.for_server(DbmsKind::Postgres, "17.1").is_some());
}

#[test]
fn a_manifest_naming_another_path_is_refused() {
    for bad in [
        "../postgres-17.hvsnap.zst",
        "sub/postgres-17.hvsnap.zst",
        "postgres-16.hvsnap.zst",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let text = format!(
            "format = 1\n\n[[snapshot]]\nfile = \"{bad}\"\ndbms = \"postgres\"\nline = \"17\"\n\
             server_version = \"17.12\"\nformat = 1\nzst_digest = \"xxh3-64:0000000000000000\"\n"
        );
        std::fs::write(dir.path().join("manifest.toml"), text).unwrap();
        assert!(
            SystemCatalogs::open(dir.path()).is_err(),
            "{bad} must be refused"
        );
    }
    // No manifest at all is an error too (an installation without its resources).
    let empty = tempfile::tempdir().unwrap();
    assert!(SystemCatalogs::open(empty.path()).is_err());
}

#[cfg(feature = "embedded-system")]
#[test]
fn embedded_snapshots_decode_lazily_once_per_version() {
    let set = SystemCatalogs::from_embedded();
    let mut lines: Vec<(DbmsKind, String)> = set
        .lines()
        .into_iter()
        .map(|(d, l)| (d, l.to_string()))
        .collect();
    lines.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    let mut opened: Vec<(DbmsKind, String)> = sys()
        .lines()
        .into_iter()
        .map(|(d, l)| (d, l.to_string()))
        .collect();
    opened.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    assert_eq!(lines, opened, "the embedded table holds the committed set");
    assert_eq!(lines.len(), 10);
    assert_eq!(set.decoded_count(), 0);
    let a = set.for_server(DbmsKind::Postgres, "16.4").unwrap();
    assert_eq!(set.decoded_count(), 1);
    let b = set.for_server(DbmsKind::Postgres, "16.9").unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(set.decoded_count(), 1);
    assert_eq!(
        &*set.for_server(DbmsKind::MySql, "8.4.2").unwrap().line,
        "8.4"
    );
    assert_eq!(set.decoded_count(), 2);
    // The embedded shipped virtual foreign keys are the committed file's.
    assert_eq!(
        set.overlays_for(DbmsKind::Postgres, "16.4")
            .shipped_fks
            .len(),
        sys()
            .overlays_for(DbmsKind::Postgres, "16.4")
            .shipped_fks
            .len()
    );
}

#[test]
fn layers_without_a_snapshot_are_empty() {
    let layers = Overlays::default();
    assert!(layers.system.is_none());
    let set = sys();
    let none = set.overlays_for(DbmsKind::Postgres, "13.22");
    assert!(none.system.is_none());
    assert!(none.shipped_fks.is_empty());
}

/// Ruling R-W5-4: a catalog made with a system set carries the layer its server's engine and
/// version get, re-chosen by its one writer when a publish changes the facts (the facts read on
/// connect replace the hub's placeholder); user data replaces only the user layers.
#[test]
fn catalog_with_system_layers_follow_the_server_facts() {
    use hacivat_meta::name::{NameClass, match_key};
    use hacivat_meta::userdata::ProfileUserData;
    use sql_catalog::view::CatalogView;

    let set = Arc::new(sys());
    // The hub's placeholder: no version yet, so no snapshot.
    let catalog = Catalog::with_system(
        Arc::new(ServerFacts::new(DbmsKind::Postgres, pg_ctx())),
        Arc::clone(&set),
    );
    assert!(Arc::ptr_eq(catalog.system().unwrap(), &set));
    assert!(catalog.current().overlays.system.is_none());
    let publisher = catalog.publisher().unwrap();
    let mut facts = ServerFacts::new(DbmsKind::Postgres, pg_ctx());
    facts.version = name("17.13");
    publisher
        .publish(vec![
            CatalogChange::SetServer(Arc::new(facts)),
            CatalogChange::SetState(
                ContainerKey::schema_list(Some(&name(DB))),
                loaded(Level::Names),
            ),
        ])
        .unwrap();
    let snap = catalog.current();
    assert_eq!(snap.overlays.system_label().map(|l| &**l), Some("17"));
    assert!(!snap.overlays.shipped_fks.is_empty());
    // The generation's own overlays: a plain view resolves built-ins through the layer.
    let view = SnapshotView::new(Arc::clone(&snap)).with_search_path(pg_path());
    assert!(matches!(
        view.lookup(&p("pg_catalog.pg_class"), KindGroup::Relations),
        Lookup::Found(_)
    ));
    let ctx = pg_ctx();
    let key = |t: &str, class| match_key(ctx.dialect, &ctx, t, false, class).unwrap();
    let Lookup::Found(now) = view.functions(
        &key("pg_catalog", NameClass::Schema),
        &key("now", NameClass::Routine),
    ) else {
        panic!("pg_catalog.now through the layer");
    };
    assert_eq!(now[0].args.as_deref(), Some(&[][..]));
    // User data replaces the user layers and keeps the system layer.
    let mut data = ProfileUserData::default();
    data.annotations.insert(
        ObjectKey {
            kind: ObjectKind::Table,
            path: [DB, "public", "orders"].into_iter().map(name).collect(),
            server_id: None,
            overload_sig: None,
            raw_kind: None,
        },
        "note".into(),
    );
    publisher
        .publish(vec![CatalogChange::SetUserData(Arc::new(data))])
        .unwrap();
    let snap = catalog.current();
    assert_eq!(snap.overlays.annotations.len(), 1);
    assert!(snap.overlays.system.is_some());
    // A server without a snapshot (another major): the layer goes, the user layers stay.
    let mut pg13 = ServerFacts::new(DbmsKind::Postgres, pg_ctx());
    pg13.version = name("13.22");
    publisher
        .publish(vec![CatalogChange::SetServer(Arc::new(pg13))])
        .unwrap();
    let snap = catalog.current();
    assert!(snap.overlays.system.is_none());
    assert_eq!(snap.overlays.annotations.len(), 1);
    // A catalog made without a set never gets one.
    let plain = Catalog::new(Arc::new(ServerFacts::new(DbmsKind::Postgres, pg_ctx())));
    assert!(plain.system().is_none());
}

/// Ruling R-W5-6: a miss in PostgreSQL's system layer is a fact only when the server is proven
/// to hold nothing beyond initdb in its system schemas (`system_schema_extras == Some(false)`,
/// read on connect). With extras, or unknown, it is `Unknown`; hits stay `Found`.
#[test]
fn pg_system_layer_misses_need_the_initdb_only_fact() {
    let absent = p("pg_catalog.hv_no_such_view");
    let public = || ns_pg(DB, "public").table("orders", &["id"]).finish();
    let out = || vec![(NsKey::schema([DB, "pg_catalog"]), LoadState::OutOfScope)];
    for (extras, fact) in [(Some(false), true), (Some(true), false), (None, false)] {
        let snap = pg_snapshot_extras(
            "17.12",
            extras,
            vec![(public(), loaded(Level::Structure))],
            out(),
        );
        let view = SnapshotView::new(snap)
            .with_layers(Arc::new(sys().overlays_for(DbmsKind::Postgres, "17.12")))
            .with_search_path(pg_path());
        let what = format!("extras={extras:?}");
        assert!(
            matches!(
                view.lookup(&p("pg_catalog.pg_class"), KindGroup::Relations),
                Lookup::Found(_)
            ),
            "{what}"
        );
        let miss = view.lookup(&absent, KindGroup::Relations);
        let unqualified = view.lookup(&p("hv_absent"), KindGroup::Relations);
        let user = view.lookup(&p("orders"), KindGroup::Relations);
        if fact {
            assert!(matches!(miss, Lookup::Missing(_)), "{what}");
            assert!(matches!(unqualified, Lookup::Missing(_)), "{what}");
            assert!(matches!(user, Lookup::Found(_)), "{what}");
        } else {
            assert!(
                matches!(miss, Lookup::Unknown(UnknownReason::ServerDependent)),
                "{what}"
            );
            // The walk stops at `pg_catalog`: it may hold the name.
            assert!(
                matches!(unqualified, Lookup::Unknown(UnknownReason::ServerDependent)),
                "{what}"
            );
            assert!(
                matches!(user, Lookup::Unknown(UnknownReason::ServerDependent)),
                "{what}"
            );
        }
        // The columns of a table the snapshot holds follow the version rule only.
        let pg_class = key_of(&view.lookup(&p("pg_catalog.pg_class"), KindGroup::Relations));
        assert!(
            column_names(view.children(&pg_class, ObjectKind::Column))
                .iter()
                .any(|c| c == "relname"),
            "{what}"
        );
    }
}

/// Ruling R-W5-6: a live, loaded `pg_catalog` arena is consulted after a miss in the snapshot,
/// so an object the snapshot cannot hold (an extension installed into `pg_catalog`) is found
/// there, qualified and unqualified, with its columns.
#[test]
fn a_live_pg_catalog_object_beyond_the_snapshot_is_found() {
    let live = ns_pg(DB, "pg_catalog")
        .table("hv_ext_tbl", &["ext_col"])
        .finish();
    let snap = pg_snapshot_extras(
        "17.12",
        Some(true),
        vec![
            (live, loaded(Level::Structure)),
            (
                ns_pg(DB, "public").table("orders", &["id"]).finish(),
                loaded(Level::Structure),
            ),
        ],
        vec![],
    );
    let view = SnapshotView::new(snap)
        .with_layers(Arc::new(sys().overlays_for(DbmsKind::Postgres, "17.12")))
        .with_search_path(pg_path());
    for path in ["pg_catalog.hv_ext_tbl", "hv_ext_tbl"] {
        let hit = view.lookup(&p(path), KindGroup::Relations);
        let Lookup::Found(h) = &hit else {
            panic!("{path}: {hit:?}");
        };
        assert!(!h.obj().flags().contains(ObjFlags::PRE_INTROSPECTED));
        assert_eq!(
            column_names(view.children(&key_of(&hit), ObjectKind::Column)),
            ["ext_col"]
        );
    }
    // The snapshot still answers what it holds.
    let hit = view.lookup(&p("pg_catalog.pg_class"), KindGroup::Relations);
    let Lookup::Found(h) = &hit else {
        panic!("{hit:?}")
    };
    assert!(h.obj().flags().contains(ObjFlags::PRE_INTROSPECTED));
}

/// Review m-3: an I/O failure reading a snapshot file is retried on the next use; only a file
/// that is not the manifest's (or does not decode) is refused for good.
#[cfg(unix)]
#[test]
fn an_unreadable_snapshot_file_is_tried_again() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    for f in ["manifest.toml", "postgres-16.hvsnap.zst"] {
        std::fs::copy(assets_dir().join(f), dir.path().join(f)).unwrap();
    }
    let file = dir.path().join("postgres-16.hvsnap.zst");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
    let set = SystemCatalogs::open(dir.path()).unwrap();
    // A superuser reads the file anyway; then there is nothing to retry.
    if std::fs::read(&file).is_err() {
        assert!(set.for_server(DbmsKind::Postgres, "16.4").is_none());
        assert_eq!(set.decoded_count(), 0, "an I/O failure is not settled");
    }
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(set.for_server(DbmsKind::Postgres, "16.4").is_some());
    assert_eq!(set.decoded_count(), 1);
}
