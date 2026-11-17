//! HV-MODEL-01/02: arenas, identity, lookups, subtree replacement and catalog publishing.

use std::sync::Arc;

use hacivat_meta::props::{ColumnProps, ForeignKeyProps, RoutineProps, TableProps};
use hacivat_meta::snapshot::child_key;
use hacivat_meta::*;
use hacivat_proto::DbmsKind;

const PG: DbmsKind = DbmsKind::Postgres;

fn col(name: &str, ordinal: u32) -> Obj {
    Obj::new(ObjectKind::Column, name).with_props(Props::Column(Arc::new(ColumnProps {
        ordinal,
        type_text: Arc::from("integer"),
        nullable: true,
        ..Default::default()
    })))
}

fn table(name: &str, oid: u32, cols: &[&str]) -> Subtree {
    let mut t = Subtree::new(
        Obj::new(ObjectKind::Table, name)
            .with_server_id(ServerId::Oid(oid))
            .with_props(Props::Table(Arc::new(TableProps::default()))),
    );
    for (i, c) in cols.iter().enumerate() {
        t.add(t.root(), col(c, i as u32 + 1));
    }
    t
}

fn pg_schema(name: &str, tables: Vec<Subtree>) -> NamespaceSnapshot {
    let mut t = Subtree::new(Obj::new(ObjectKind::Schema, name));
    for tab in tables {
        t.graft_under(t.root(), tab);
    }
    NamespaceSnapshot::build(
        NsKey::schema(["db", name]),
        PG,
        FoldPolicy::for_dbms(PG),
        NsMeta::default(),
        t,
    )
    .unwrap()
}

fn names<'a>(ns: &'a NamespaceSnapshot, it: impl Iterator<Item = (u32, &'a Obj)>) -> Vec<&'a str> {
    let _ = ns;
    it.map(|(_, o)| &*o.name).collect()
}

#[test]
fn children_are_contiguous_and_in_insertion_order() {
    // Interleave: build two tables, add columns to the first after the second exists.
    let mut t = Subtree::new(Obj::new(ObjectKind::Schema, "public"));
    let a = t.add(t.root(), Obj::new(ObjectKind::Table, "a"));
    let b = t.add(t.root(), Obj::new(ObjectKind::Table, "b"));
    t.add(b, col("x", 1));
    t.add(a, col("c1", 1));
    t.add(a, col("c2", 2));
    t.add(b, col("y", 2));
    let ns = NamespaceSnapshot::build(
        NsKey::schema(["db", "public"]),
        PG,
        FoldPolicy::for_dbms(PG),
        NsMeta::default(),
        t,
    )
    .unwrap();
    let tabs: Vec<u32> = ns.children(0).map(|(i, _)| i).collect();
    assert_eq!(names(&ns, ns.children(0)), ["a", "b"]);
    assert_eq!(names(&ns, ns.children(tabs[0])), ["c1", "c2"]);
    assert_eq!(names(&ns, ns.children(tabs[1])), ["x", "y"]);
    for (i, o) in ns.objs().iter().enumerate().skip(1) {
        assert!(o.parent().unwrap() < i as u32, "parent precedes child");
    }
}

#[test]
fn object_key_and_find_round_trip() {
    // alg/02 HV-MODEL-01 test 1.
    let ns = pg_schema("public", vec![table("t", 16400, &["a", "b"])]);
    let t = ns.lookup_kind(ObjectKind::Table, "t").next().unwrap();
    let key = ns.object_key(t).unwrap();
    assert_eq!(key.kind, ObjectKind::Table);
    assert_eq!(
        key.path.iter().map(|n| &**n).collect::<Vec<_>>(),
        ["db", "public", "t"]
    );
    assert_eq!(key.server_id, Some(ServerId::Oid(16400)));
    let b = ns.child_by_name(t, ObjectKind::Column, "b").unwrap();
    let bkey = ns.object_key(b).unwrap();
    assert_eq!(ns.find(&bkey), Some(b));
    assert_eq!(ns.find(&ns.object_key(0).unwrap()), Some(0));
}

#[test]
fn rename_keeps_identity_and_find_follows_the_server_id() {
    // alg/02 HV-MODEL-01 test 2.
    let before = pg_schema("public", vec![table("t", 16400, &["a"])]);
    let after = pg_schema("public", vec![table("u", 16400, &["a"])]);
    let kb = before.object_key(1).unwrap();
    let ka = after.object_key(1).unwrap();
    assert!(kb.same_object(&ka));
    assert_eq!(after.find(&kb), Some(1));
}

#[test]
fn a_reused_server_id_on_the_path_is_not_the_same_object() {
    let ns = pg_schema("public", vec![table("t", 16401, &["a"])]);
    let mut stale = ns.object_key(1).unwrap();
    stale.server_id = Some(ServerId::Oid(99));
    assert_eq!(ns.find(&stale), None);
}

#[test]
fn names_differing_by_case_are_two_objects_in_postgres() {
    // alg/02 HV-MODEL-01 edge 1.
    let ns = pg_schema(
        "public",
        vec![table("Users", 1, &["a"]), table("users", 2, &["a"])],
    );
    assert_eq!(ns.lookup(KindGroup::Relations, "users").len(), 1);
    assert_eq!(ns.lookup(KindGroup::Relations, "Users").len(), 1);
    assert_eq!(ns.lookup(KindGroup::Relations, "USERS").len(), 0);
}

#[test]
fn mysql_columns_fold_ascii_only() {
    let mut t = Subtree::new(Obj::new(ObjectKind::Schema, "shop"));
    let tab = t.add(t.root(), Obj::new(ObjectKind::Table, "Orders"));
    t.add(tab, col("MÜŞTERİ_Id", 1));
    let ns = NamespaceSnapshot::build(
        NsKey::schema(["shop"]),
        DbmsKind::MySql,
        FoldPolicy::for_dbms(DbmsKind::MySql),
        NsMeta::default(),
        t,
    )
    .unwrap();
    // Relations exact (lower_case_table_names = 0), columns ASCII-caseless.
    assert_eq!(ns.lookup(KindGroup::Relations, "orders").len(), 0);
    assert_eq!(ns.lookup(KindGroup::Relations, "Orders").len(), 1);
    assert!(
        ns.child_by_name(1, ObjectKind::Column, "MÜŞTERİ_ID")
            .is_some()
    );
    assert!(
        ns.child_by_name(1, ObjectKind::Column, "müşteri_id")
            .is_none()
    );
}

#[test]
fn overloads_share_a_name_and_keep_distinct_keys() {
    // alg/02 HV-MODEL-01 test 4.
    let f = |sig: &str| {
        Subtree::new(
            Obj::new(ObjectKind::Routine, "f").with_props(Props::Routine(Arc::new(RoutineProps {
                overload_sig: Some(Arc::from(sig)),
                ..Default::default()
            }))),
        )
    };
    let ns = pg_schema("public", vec![f("integer"), f("text")]);
    let hits = ns.lookup(KindGroup::Routines, "f");
    assert_eq!(hits.len(), 2);
    let k_text = ns.object_key(hits[1]).unwrap();
    assert_eq!(k_text.overload_sig.as_deref(), Some("text"));
    assert_eq!(ns.find(&k_text), Some(hits[1]));
}

#[test]
fn the_kind_schema_rejects_a_column_under_a_schema() {
    let mut t = Subtree::new(Obj::new(ObjectKind::Schema, "public"));
    t.add(t.root(), col("loose", 1));
    let err = NamespaceSnapshot::build(
        NsKey::schema(["db", "public"]),
        PG,
        FoldPolicy::for_dbms(PG),
        NsMeta::default(),
        t,
    )
    .unwrap_err();
    assert_eq!(
        err,
        ModelError::KindNotAllowed {
            parent: ObjectKind::Schema,
            child: ObjectKind::Column
        }
    );
}

#[test]
fn the_root_must_match_the_namespace_key() {
    let t = Subtree::new(Obj::new(ObjectKind::Schema, "other"));
    assert!(matches!(
        NamespaceSnapshot::build(
            NsKey::schema(["db", "public"]),
            PG,
            FoldPolicy::for_dbms(PG),
            NsMeta::default(),
            t
        ),
        Err(ModelError::RootName { .. })
    ));
}

#[test]
fn replace_subtree_swaps_one_table_in_place() {
    let ns = pg_schema(
        "public",
        vec![
            table("a", 1, &["x"]),
            table("b", 2, &["y"]),
            table("c", 3, &["z"]),
        ],
    );
    let b = ns.lookup_kind(ObjectKind::Table, "b").next().unwrap();
    let next = ns
        .replace_subtree(b, Some(table("b", 2, &["y", "y2", "y3"])))
        .unwrap();
    assert_eq!(names(&next, next.children(0)), ["a", "b", "c"]);
    let nb = next.lookup_kind(ObjectKind::Table, "b").next().unwrap();
    assert_eq!(names(&next, next.children(nb)), ["y", "y2", "y3"]);
    // Untouched siblings keep their columns; the old arena is unchanged.
    let na = next.lookup_kind(ObjectKind::Table, "a").next().unwrap();
    assert_eq!(names(&next, next.children(na)), ["x"]);
    assert_eq!(ns.children(b).count(), 1);
    // Removing drops the whole subtree.
    let gone = next.replace_subtree(nb, None).unwrap();
    assert_eq!(names(&gone, gone.children(0)), ["a", "c"]);
    assert_eq!(gone.column_count(), 2);
    assert_eq!(
        ns.replace_subtree(0, None).unwrap_err(),
        ModelError::NoSuchObject
    );
}

#[test]
fn upsert_child_replaces_by_identity_or_appends() {
    let ns = pg_schema("public", vec![table("a", 1, &["x"]), table("b", 2, &["y"])]);
    // Same oid, new name: a rename replaces in place.
    let renamed = ns.upsert_child(0, table("a2", 1, &["x", "w"])).unwrap();
    assert_eq!(names(&renamed, renamed.children(0)), ["a2", "b"]);
    // New oid: appended.
    let added = renamed.upsert_child(0, table("c", 3, &[])).unwrap();
    assert_eq!(names(&added, added.children(0)), ["a2", "b", "c"]);
    assert_eq!(added.lookup(KindGroup::Relations, "c").len(), 1);
    assert_eq!(added.lookup(KindGroup::Relations, "a").len(), 0);
}

fn catalog_with(nss: Vec<NamespaceSnapshot>) -> Catalog {
    let c = Catalog::new(PG);
    c.publish(nss.into_iter().map(CatalogChange::Put)).unwrap();
    c
}

#[test]
fn publish_shares_untouched_namespaces_and_pinned_readers_keep_their_generation() {
    // alg/02 HV-MODEL-02 test 3.
    let c = catalog_with(vec![
        pg_schema("public", vec![table("t", 1, &["a"])]),
        pg_schema("sales", vec![table("s", 2, &["b"])]),
    ]);
    let g1 = c.current();
    assert_eq!(g1.generation(), 1);
    let public = NsKey::schema(["db", "public"]);
    let sales = NsKey::schema(["db", "sales"]);
    let published = c
        .publish([CatalogChange::Put(pg_schema(
            "public",
            vec![table("t", 1, &["a", "a2"])],
        ))])
        .unwrap();
    assert_eq!(published.generation, 2);
    assert_eq!(published.changed, std::slice::from_ref(&public));
    let g2 = c.current();
    assert!(Arc::ptr_eq(
        g1.namespace(&sales).unwrap(),
        g2.namespace(&sales).unwrap()
    ));
    assert_eq!(g1.namespace(&public).unwrap().column_count(), 1);
    assert_eq!(g2.namespace(&public).unwrap().column_count(), 2);
    assert_eq!(g2.namespace(&public).unwrap().generation(), 2);
    assert_eq!(g2.namespace(&sales).unwrap().generation(), 1);
}

#[test]
fn subtree_changes_go_through_the_catalog() {
    let c = catalog_with(vec![pg_schema("public", vec![table("t", 1, &["a"])])]);
    let public = NsKey::schema(["db", "public"]);
    let schema_key = c
        .current()
        .namespace(&public)
        .unwrap()
        .object_key(0)
        .unwrap();
    c.publish([CatalogChange::UpsertSubtree {
        ns: public.clone(),
        parent: schema_key.clone(),
        subtree: table("t", 1, &["a", "b"]),
    }])
    .unwrap();
    let snap = c.current();
    let id = snap
        .find(&child_key(&schema_key, ObjectKind::Table, "t"))
        .unwrap();
    assert_eq!(snap.namespace(&id.ns).unwrap().children(id.idx).count(), 2);

    c.publish([CatalogChange::RemoveSubtree {
        ns: public.clone(),
        target: child_key(&schema_key, ObjectKind::Table, "t"),
    }])
    .unwrap();
    assert_eq!(c.current().column_count(), 0);
}

#[test]
fn a_failing_change_publishes_nothing() {
    let c = catalog_with(vec![pg_schema("public", vec![table("t", 1, &["a"])])]);
    let before = c.current();
    let err = c
        .publish([
            CatalogChange::Put(pg_schema("sales", vec![])),
            CatalogChange::Remove(NsKey::schema(["db", "missing"])),
        ])
        .unwrap_err();
    assert!(matches!(err, ModelError::NoSuchNamespace(_)));
    let after = c.current();
    assert_eq!(after.generation(), before.generation());
    assert_eq!(after.namespace_count(), 1);
}

#[test]
fn references_resolve_on_demand_and_survive_a_target_reload() {
    // A FK in `sales` to `public.t`; `public` is reloaded; the reference still resolves.
    let mut s = table("orders", 10, &["owner_id"]);
    s.add(
        s.root(),
        Obj::new(ObjectKind::ForeignKey, "orders_owner_fkey").with_props(Props::ForeignKey(
            Arc::new(ForeignKeyProps {
                columns: vec![Arc::from("owner_id")],
                target: Ref {
                    kind: ObjectKind::Table,
                    path: ["db", "public", "t"]
                        .iter()
                        .map(|s| Arc::from(*s))
                        .collect(),
                    server_id: Some(ServerId::Oid(1)),
                },
                target_columns: vec![Arc::from("id")],
                on_update: Default::default(),
                on_delete: Default::default(),
                match_kind: Default::default(),
                deferrable: Default::default(),
            }),
        )),
    );
    let c = catalog_with(vec![
        pg_schema("public", vec![table("t", 1, &["id"])]),
        pg_schema("sales", vec![s]),
    ]);
    let fk_ref = |snap: &CatalogSnapshot| {
        let sales = snap.namespace(&NsKey::schema(["db", "sales"])).unwrap();
        let fk = sales
            .objs()
            .iter()
            .find(|o| o.kind == ObjectKind::ForeignKey)
            .unwrap();
        match &fk.props {
            Props::ForeignKey(p) => p.target.clone(),
            _ => unreachable!(),
        }
    };
    let r = fk_ref(&c.current());
    let hit = c.current().resolve(&r).unwrap();
    assert_eq!(c.current().obj(&hit).unwrap().name.as_ref(), "t");
    c.publish([CatalogChange::Put(pg_schema(
        "public",
        vec![table("z", 5, &[]), table("t", 1, &["id", "extra"])],
    ))])
    .unwrap();
    let snap = c.current();
    let hit = snap.resolve(&r).unwrap();
    assert_eq!(snap.obj(&hit).unwrap().name.as_ref(), "t");
    assert_eq!(hit.idx, 2);
    // Target namespace not loaded → unresolved (the tri-state view reports Unknown).
    let mut other = r.clone();
    other.path = ["db", "hr", "emp"].iter().map(|s| Arc::from(*s)).collect();
    assert_eq!(snap.resolve(&other), None);
}

#[test]
fn readers_never_see_a_torn_generation() {
    // alg/02 HV-MODEL-02 test 7 (reduced): each reader's view comes from one generation.
    let c = Arc::new(catalog_with(vec![
        pg_schema("public", vec![table("t", 1, &["a"])]),
        pg_schema("sales", vec![table("s", 2, &["a"])]),
    ]));
    let writer = {
        let c = c.clone();
        std::thread::spawn(move || {
            for n in 2..200usize {
                let cols: Vec<String> = (0..n).map(|i| format!("c{i}")).collect();
                let cols: Vec<&str> = cols.iter().map(String::as_str).collect();
                c.publish([
                    CatalogChange::Put(pg_schema("public", vec![table("t", 1, &cols)])),
                    CatalogChange::Put(pg_schema("sales", vec![table("s", 2, &cols)])),
                ])
                .unwrap();
            }
        })
    };
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let c = c.clone();
            std::thread::spawn(move || {
                for _ in 0..2_000 {
                    let snap = c.current();
                    let p = snap.namespace(&NsKey::schema(["db", "public"])).unwrap();
                    let s = snap.namespace(&NsKey::schema(["db", "sales"])).unwrap();
                    assert_eq!(p.column_count(), s.column_count());
                    assert_eq!(p.generation(), s.generation());
                }
            })
        })
        .collect();
    writer.join().unwrap();
    for r in readers {
        r.join().unwrap();
    }
}

#[test]
fn snapshots_cross_threads() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Catalog>();
    send_sync::<CatalogSnapshot>();
    send_sync::<NamespaceSnapshot>();
}
