//! The M4 Task 9 reviewer's hostile probes (H1–H11, `task-m4-09-review.md`), committed in fix
//! round 1. Each test asserts the fail-closed answer; H1–H7 and H11 failed on `eb05a7e`
//! (C1, I1–I5) and pass after the fix.

use std::sync::Arc;
use std::time::SystemTime;

use hacivat_meta::fixture::{ns_mysql, ns_pg, pg_catalog};
use hacivat_meta::ids::{NsKey, ObjectKey, Ref};
use hacivat_meta::kind::{KindGroup, ObjectKind};
use hacivat_meta::name::{DialectId, IdentCtx, Name, NamePart};
use hacivat_meta::namespace::{Level, NamespaceSnapshot, Watermark};
use hacivat_meta::search_path::SearchPath;
use hacivat_meta::snapshot::{
    Catalog, CatalogChange, CatalogSnapshot, ContainerKey, LoadState, ServerFacts,
};
use hacivat_meta::view::{Lookup, ModelView, SnapshotView, UnknownReason};
use hacivat_proto::DbmsKind;

fn loaded() -> LoadState {
    LoadState::Loaded {
        level: Level::Structure,
        at: SystemTime::UNIX_EPOCH,
        watermark: Watermark::None,
        stale: false,
    }
}

fn pg_ctx(enc: &str) -> IdentCtx {
    IdentCtx {
        encoding: Some(Arc::from(enc)),
        ..IdentCtx::new(DialectId::Postgres)
    }
}

fn my_ctx(lctn: u8) -> IdentCtx {
    IdentCtx {
        lower_case_table_names: lctn,
        ..IdentCtx::new(DialectId::MySql)
    }
}

fn part(t: &str, quoted: bool) -> NamePart {
    NamePart {
        text: Arc::from(t),
        quoted,
    }
}

fn n(s: &str) -> Name {
    Arc::from(s)
}

/// Publishes every namespace `Loaded` at `Structure`, plus extra states.
fn publish(
    dbms: DbmsKind,
    ident: IdentCtx,
    nss: Vec<NamespaceSnapshot>,
    extra: Vec<(ContainerKey, LoadState)>,
) -> (Catalog, Arc<CatalogSnapshot>) {
    let catalog = Catalog::new(Arc::new(ServerFacts::new(dbms, ident)));
    let w = catalog.publisher().unwrap();
    let mut ch = Vec::new();
    for ns in nss {
        ch.push(CatalogChange::SetState(
            ContainerKey::Namespace(ns.key().clone()),
            loaded(),
        ));
        ch.push(CatalogChange::Put(Arc::new(ns)));
    }
    for (k, s) in extra {
        ch.push(CatalogChange::SetState(k, s));
    }
    w.publish(ch).unwrap();
    let snap = catalog.current();
    (catalog, snap)
}

fn brief<T: std::fmt::Debug>(l: &Lookup<T>) -> String {
    match l {
        Lookup::Found(_) => "Found".into(),
        other => format!("{other:?}"),
    }
}

// H1. A reference into a schema the catalog does not list, while the schema list is NOT loaded
// and the database namespace (which holds no schema objects, T3's own fixture shape) is
// complete: the server may well have `app.sales.orders`. Must not be `Missing`.
#[test]
fn h1_resolve_into_unlisted_schema_is_never_missing() {
    let (_c, snap) = publish(
        DbmsKind::Postgres,
        pg_ctx("UTF8"),
        pg_catalog(&[("public", 2)], 4),
        vec![],
    );
    let r = Ref {
        kind: ObjectKind::Table,
        path: smallvec::smallvec![n("app"), n("sales"), n("orders")],
        server_id: None,
    };
    let got = snap.resolve(&r);
    assert!(
        matches!(got, Lookup::Unknown(_)),
        "resolve app.sales.orders with schema list unknown: {}",
        brief(&got)
    );
}

// H2. The same via the server namespace: a reference into another database.
#[test]
fn h2_resolve_into_unlisted_database_is_never_missing() {
    let (_c, snap) = publish(
        DbmsKind::Postgres,
        pg_ctx("UTF8"),
        pg_catalog(&[("public", 2)], 4),
        vec![],
    );
    let r = Ref {
        kind: ObjectKind::Table,
        path: smallvec::smallvec![n("otherdb"), n("public"), n("t")],
        server_id: None,
    };
    let got = snap.resolve(&r);
    assert!(
        matches!(got, Lookup::Unknown(_)),
        "resolve otherdb.public.t: {}",
        brief(&got)
    );
}

// H3. `children` of a table in an unlisted schema (schema list unknown).
#[test]
fn h3_children_in_unlisted_schema_is_never_missing() {
    let (_c, snap) = publish(
        DbmsKind::Postgres,
        pg_ctx("UTF8"),
        pg_catalog(&[("public", 2)], 4),
        vec![],
    );
    let v = SnapshotView::new(snap);
    let parent = ObjectKey {
        kind: ObjectKind::Table,
        path: smallvec::smallvec![n("app"), n("sales"), n("orders")],
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    };
    let got = v.children(&parent, ObjectKind::Column);
    assert!(
        matches!(got, Lookup::Unknown(_)),
        "children(app.sales.orders): {}",
        brief(&got)
    );
}

/// PostgreSQL `alice` (holds `t`), `public` (no `t`), `pg_catalog` complete (as after T26 or
/// "Introspect live"), schema list loaded.
fn alice_public(public_has_t: bool) -> Arc<CatalogSnapshot> {
    let mut public = ns_pg("db", "public");
    if public_has_t {
        public = public.table_oid("t", 2, &["a"]);
    }
    let (_c, snap) = publish(
        DbmsKind::Postgres,
        pg_ctx("UTF8"),
        vec![
            ns_pg("db", "alice").table_oid("t", 1, &["a"]).finish(),
            public.finish(),
            ns_pg("db", "pg_catalog").finish(),
        ],
        vec![(ContainerKey::schema_list(Some(&n("db"))), loaded())],
    );
    snap
}

// H4. `$user` with an unknown session user (`SchemaCtx.user` is an Option): the server
// searches `alice` first and finds `alice.t`. Must not be `Missing`.
#[test]
fn h4_unknown_user_never_gives_missing() {
    let snap = alice_public(false);
    let path = SearchPath::for_pg(&n("db"), &[n("$user"), n("public")], None, None);
    let v = SnapshotView::new(snap).with_search_path(path);
    let got = v.lookup(&[part("t", false)], KindGroup::Relations);
    assert!(
        !matches!(got, Lookup::Missing(_)),
        "t via $user with unknown user: {}",
        brief(&got)
    );
}

// H5. pg_catalog absent from a loaded schema list (an engine that lists only user schemas,
// R-M4-14 "system schemas are never introspected live"): `pg_class` exists on every server.
#[test]
fn h5_pg_catalog_unlisted_never_gives_missing() {
    let (_c, snap) = publish(
        DbmsKind::Postgres,
        pg_ctx("UTF8"),
        vec![ns_pg("db", "public").table_oid("t", 1, &[]).finish()],
        vec![(ContainerKey::schema_list(Some(&n("db"))), loaded())],
    );
    let path = SearchPath::for_pg(&n("db"), &[n("public")], Some(&n("alice")), None);
    let v = SnapshotView::new(snap).with_search_path(path);
    let got = v.lookup(&[part("pg_class", false)], KindGroup::Relations);
    assert!(
        !matches!(got, Lookup::Missing(_)),
        "pg_class with pg_catalog unlisted: {}",
        brief(&got)
    );
}

// H6. Shadowing: `s1` (first on the path) is out of scope, `public` holds `t`. The server
// resolves `t` to `s1.t` when it exists; the view cannot know. alg/02 step 5 says "Found if
// any"; this probe records whether the view answers Found from a later namespace.
#[test]
fn h6_found_behind_an_unknown_earlier_namespace() {
    let (_c, snap) = publish(
        DbmsKind::Postgres,
        pg_ctx("UTF8"),
        vec![
            ns_pg("db", "public").table_oid("t", 2, &[]).finish(),
            ns_pg("db", "pg_catalog").finish(),
        ],
        vec![
            (
                ContainerKey::Namespace(NsKey::schema(["db", "s1"])),
                LoadState::OutOfScope,
            ),
            (ContainerKey::schema_list(Some(&n("db"))), loaded()),
        ],
    );
    let path = SearchPath::for_pg(&n("db"), &[n("s1"), n("public")], None, None);
    let v = SnapshotView::new(snap).with_search_path(path);
    let got = v.lookup(&[part("t", false)], KindGroup::Relations);
    assert!(
        matches!(got, Lookup::Unknown(_)),
        "t behind out-of-scope s1: {}",
        brief(&got)
    );
}

// H7. MySQL lctn = 1, a stored database name with a non-ASCII cased letter (`ılık`): an ASCII
// probe `ILIK.t` has a certain *probe* key, but the stored side is unpinned (T3 treats stored
// non-ASCII cased names under a caseless MySQL rule as uncertain). Must not be `Missing`.
#[test]
fn h7_mysql_caseless_schema_with_non_ascii_stored_name() {
    let (_c, snap) = publish(
        DbmsKind::MySql,
        my_ctx(1),
        vec![ns_mysql("ılık", 1).table("t", &["a"]).finish()],
        vec![(ContainerKey::schema_list(None), loaded())],
    );
    let v = SnapshotView::new(snap);
    let got = v.lookup(
        &[part("ILIK", false), part("t", false)],
        KindGroup::Relations,
    );
    assert!(
        !matches!(got, Lookup::Missing(_)),
        "ILIK.t vs stored database ılık: {}",
        brief(&got)
    );
    // Control: the same at table level is uncertain through T3's `match_named`.
    let (_c, snap) = publish(
        DbmsKind::MySql,
        my_ctx(1),
        vec![ns_mysql("shop", 1).table("ılık", &["a"]).finish()],
        vec![(ContainerKey::schema_list(None), loaded())],
    );
    let v = SnapshotView::new(snap).with_search_path(SearchPath::for_mysql(Some(&n("shop"))));
    let got = v.lookup(&[part("ILIK", false)], KindGroup::Relations);
    assert!(
        matches!(got, Lookup::Unknown(UnknownReason::ServerDependent)),
        "ILIK vs stored table ılık: {}",
        brief(&got)
    );
}

// H8. Turkish names on PostgreSQL (UTF8 folds ASCII only): unquoted İSTANBUL is `İstanbul`;
// quoted is exact; an unknown/LATIN1 encoding is server dependent. `istanbul` is another name.
#[test]
fn h8_turkish_pg_names() {
    let (_c, snap) = publish(
        DbmsKind::Postgres,
        pg_ctx("UTF8"),
        vec![
            ns_pg("db", "public").table_oid("İstanbul", 1, &[]).finish(),
            ns_pg("db", "pg_catalog").finish(),
        ],
        vec![(ContainerKey::schema_list(Some(&n("db"))), loaded())],
    );
    let v = SnapshotView::new(snap).with_search_path(SearchPath::for_pg(
        &n("db"),
        &[n("public")],
        Some(&n("u")),
        None,
    ));
    assert!(matches!(
        v.lookup(&[part("İSTANBUL", false)], KindGroup::Relations),
        Lookup::Found(_)
    ));
    assert!(matches!(
        v.lookup(&[part("İSTANBUL", true)], KindGroup::Relations),
        Lookup::Missing(_)
    ));
    assert!(matches!(
        v.lookup(&[part("istanbul", false)], KindGroup::Relations),
        Lookup::Missing(_)
    ));
    let (_c, snap) = publish(
        DbmsKind::Postgres,
        pg_ctx("LATIN1"),
        vec![
            ns_pg("db", "public").table_oid("İstanbul", 1, &[]).finish(),
            ns_pg("db", "pg_catalog").finish(),
        ],
        vec![(ContainerKey::schema_list(Some(&n("db"))), loaded())],
    );
    let v = SnapshotView::new(snap).with_search_path(SearchPath::for_pg(
        &n("db"),
        &[n("public")],
        Some(&n("u")),
        None,
    ));
    let got = v.lookup(&[part("İSTANBUL", false)], KindGroup::Relations);
    assert!(
        matches!(got, Lookup::Unknown(UnknownReason::ServerDependent)),
        "LATIN1 unquoted İSTANBUL: {}",
        brief(&got)
    );
}

// H9. No stale negative answer across a publish: Missing in generation N, Found in N+1 once
// the name is published; the old view keeps its own (consistent) answer.
#[test]
fn h9_negative_answer_never_outlives_its_generation() {
    let (catalog, snap) = publish(
        DbmsKind::Postgres,
        pg_ctx("UTF8"),
        vec![
            ns_pg("db", "public").finish(),
            ns_pg("db", "pg_catalog").finish(),
        ],
        vec![(ContainerKey::schema_list(Some(&n("db"))), loaded())],
    );
    let path = SearchPath::for_pg(&n("db"), &[n("public")], Some(&n("u")), None);
    let old = SnapshotView::new(snap).with_search_path(path.clone());
    assert!(matches!(
        old.lookup(&[part("n", false)], KindGroup::Relations),
        Lookup::Missing(_)
    ));
    // The test owns the only publisher? No: `publish` consumed it. Re-create the catalog.
    drop(catalog);
    let (_c2, snap2) = publish(
        DbmsKind::Postgres,
        pg_ctx("UTF8"),
        vec![
            ns_pg("db", "public").table_oid("n", 9, &[]).finish(),
            ns_pg("db", "pg_catalog").finish(),
        ],
        vec![(ContainerKey::schema_list(Some(&n("db"))), loaded())],
    );
    let new = SnapshotView::new(snap2).with_search_path(path);
    assert!(matches!(
        new.lookup(&[part("n", false)], KindGroup::Relations),
        Lookup::Found(_)
    ));
    assert!(matches!(
        old.lookup(&[part("n", false)], KindGroup::Relations),
        Lookup::Missing(_)
    ));
}

// H10. Atomicity: a list whose last change fails publishes nothing, including the earlier
// `Put` and `SetState`.
#[test]
fn h10_partial_change_list_publishes_nothing() {
    let catalog = Catalog::new(Arc::new(ServerFacts::new(
        DbmsKind::Postgres,
        pg_ctx("UTF8"),
    )));
    let w = catalog.publisher().unwrap();
    let ns = ns_pg("db", "public").table_oid("t", 1, &[]).finish();
    let key = ns.key().clone();
    let err = w.publish(vec![
        CatalogChange::Put(Arc::new(ns)),
        CatalogChange::SetState(ContainerKey::Namespace(key.clone()), loaded()),
        CatalogChange::Remove(NsKey::schema(["db", "nope"])),
    ]);
    assert!(err.is_err());
    let s = catalog.current();
    assert_eq!(s.generation, 0);
    assert!(s.namespaces.is_empty());
    assert!(s.states.is_empty());
    assert!(catalog.publisher().is_none());
}

// H11. The schema-list container doubles as `children(database, Schema)`: with the list loaded
// and the database arena holding no schema objects (T3's fixture shape), `children` must not
// claim the database has no schemas.
#[test]
fn h11_schema_list_container_vs_children_of_database() {
    let (_c, snap) = publish(
        DbmsKind::Postgres,
        pg_ctx("UTF8"),
        pg_catalog(&[("public", 2)], 4),
        vec![(ContainerKey::schema_list(Some(&n("app"))), loaded())],
    );
    let v = SnapshotView::new(snap);
    let db = ObjectKey {
        kind: ObjectKind::Database,
        path: smallvec::smallvec![n("app")],
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    };
    if let Lookup::Found(h) = v.children(&db, ObjectKind::Schema) {
        assert!(!h.is_empty(), "children(app, Schema) = Found([])");
    }
}
