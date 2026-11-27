//! HV-MODEL-02 steps 1, 3, 4 (M4 Task 9): the Arc-swapped catalog, its single writer, the
//! all-or-nothing publish, on-demand references and reverse FK edges (alg/02 HV-MODEL-02 test
//! case 3, edge cases 1–3). The model spike's five catalog-level tests (spike origin
//! `d86313b`, deferred by Task 3) are here: publish sharing, subtree changes through the
//! catalog, all-or-nothing publish, references surviving a reload, and torn reads.

use std::sync::Arc;
use std::time::SystemTime;

use hacivat_meta::fixture::ns_pg;
use hacivat_meta::ids::{NsKey, ObjectKey, Ref, ServerId};
use hacivat_meta::kind::ObjectKind;
use hacivat_meta::name::{DialectId, IdentCtx, Name};
use hacivat_meta::namespace::{Level, ModelError, NamespaceSnapshot, Obj, Subtree, Watermark};
use hacivat_meta::props::{ColumnProps, FkAction, FkMatch, FkProps, Props, TableProps};
use hacivat_meta::snapshot::{
    Catalog, CatalogChange, CatalogSnapshot, ContainerKey, LoadState, Publisher, ServerFacts,
};
use hacivat_meta::view::{Lookup, MissingReason, ModelView, SnapshotView, UnknownReason};
use hacivat_proto::DbmsKind;
use smallvec::SmallVec;

const DB: &str = "db";

fn pg_ctx() -> IdentCtx {
    IdentCtx {
        encoding: Some(Arc::from("UTF8")),
        ..IdentCtx::new(DialectId::Postgres)
    }
}

fn name(s: &str) -> Name {
    Arc::from(s)
}

fn path(parts: &[&str]) -> SmallVec<[Name; 4]> {
    parts.iter().map(|s| name(s)).collect()
}

fn loaded(level: Level) -> LoadState {
    LoadState::Loaded {
        level,
        at: SystemTime::UNIX_EPOCH,
        watermark: Watermark::None,
        stale: false,
    }
}

fn col(n: &str, ordinal: u32) -> Obj {
    Obj::new(ObjectKind::Column, n).with_props(Props::Column(Arc::new(ColumnProps {
        ordinal,
        raw_type: name("integer"),
        nullable: true,
        ..Default::default()
    })))
}

fn table(n: &str, oid: u32, cols: &[&str]) -> Subtree {
    let mut t = Subtree::new(
        Obj::new(ObjectKind::Table, n)
            .with_server_id(ServerId::Oid(oid))
            .with_props(Props::Table(Arc::new(TableProps::default()))),
    );
    for (i, c) in cols.iter().enumerate() {
        t.add(t.root(), col(c, i as u32 + 1));
    }
    t
}

fn fk(n: &str, target: &[&str], target_oid: Option<u32>) -> Obj {
    Obj::new(ObjectKind::ForeignKey, n).with_props(Props::ForeignKey(Arc::new(FkProps {
        columns: vec![name("owner_id")],
        target: Ref {
            kind: ObjectKind::Table,
            path: path(target),
            server_id: target_oid.map(ServerId::Oid),
        },
        target_columns: vec![name("id")],
        on_update: FkAction::default(),
        on_delete: FkAction::default(),
        match_kind: FkMatch::default(),
        deferrable: false,
        initially_deferred: false,
        not_valid: false,
        enforced: true,
        rare: None,
    })))
}

/// A PostgreSQL schema `[db, name]` holding `tables`.
fn pg_schema(n: &str, tables: Vec<Subtree>) -> NamespaceSnapshot {
    let mut b = ns_pg(DB, n);
    for t in tables {
        b = b.subtree(t);
    }
    b.finish()
}

fn put(ns: NamespaceSnapshot, state: LoadState) -> [CatalogChange; 2] {
    [
        CatalogChange::SetState(ContainerKey::Namespace(ns.key().clone()), state),
        CatalogChange::Put(Arc::new(ns)),
    ]
}

fn catalog() -> Catalog {
    Catalog::new(Arc::new(ServerFacts::new(DbmsKind::Postgres, pg_ctx())))
}

fn catalog_with(nss: Vec<NamespaceSnapshot>) -> (Catalog, Publisher) {
    let c = catalog();
    let w = c.publisher().unwrap();
    let mut changes: Vec<CatalogChange> = nss
        .into_iter()
        .flat_map(|n| put(n, loaded(Level::Structure)))
        .collect();
    changes.push(CatalogChange::SetState(
        ContainerKey::schema_list(Some(&name(DB))),
        loaded(Level::Names),
    ));
    w.publish(changes).unwrap();
    (c, w)
}

fn public() -> NsKey {
    NsKey::schema([DB, "public"])
}

fn sales() -> NsKey {
    NsKey::schema([DB, "sales"])
}

fn ns<'a>(snap: &'a CatalogSnapshot, key: &NsKey) -> &'a Arc<NamespaceSnapshot> {
    snap.namespaces
        .get(key)
        .expect("namespace in the generation")
}

#[test]
fn publish_shares_untouched_namespaces_ptr_eq() {
    // alg/02 test 3; spike `publish_shares_untouched_namespaces_and_pinned_readers_keep_their_generation`.
    let (c, w) = catalog_with(vec![
        pg_schema("public", vec![table("t", 1, &["a"])]),
        pg_schema("sales", vec![table("s", 2, &["b"])]),
    ]);
    let g1 = c.current();
    assert_eq!(g1.generation, 1);
    let published = w
        .publish(vec![CatalogChange::Put(Arc::new(pg_schema(
            "public",
            vec![table("t", 1, &["a", "a2"])],
        )))])
        .unwrap();
    assert_eq!(published.generation, 2);
    assert_eq!(published.changed, vec![public()]);
    let g2 = c.current();
    assert!(Arc::ptr_eq(ns(&g1, &sales()), ns(&g2, &sales())));
    assert!(!Arc::ptr_eq(ns(&g1, &public()), ns(&g2, &public())));
    // The states map is shared too when no state changed.
    assert!(Arc::ptr_eq(&g1.states, &g2.states));
    assert!(Arc::ptr_eq(&g1.overlays, &g2.overlays));
    assert_eq!(ns(&g2, &public()).generation(), 2);
    assert_eq!(ns(&g2, &sales()).generation(), 1);
    // A publish never copies the objects of a unique arena it is handed: the arena is moved
    // out of its `Arc` to be stamped, so its object buffer is the very one handed over (the
    // `Arc` allocation itself is new).
    let fresh = Arc::new(pg_schema("sales", vec![table("s", 2, &["b"])]));
    let objs = fresh.objs().as_ptr();
    drop(w.publish(vec![CatalogChange::Put(fresh)]).unwrap());
    assert_eq!(ns(&c.current(), &sales()).objs().as_ptr(), objs);
    // A shared one is copied (the documented cost; T15 hands over unique arenas).
    let shared = Arc::new(pg_schema("sales", vec![table("s", 2, &["b"])]));
    let objs = shared.objs().as_ptr();
    w.publish(vec![CatalogChange::Put(Arc::clone(&shared))])
        .unwrap();
    assert_ne!(ns(&c.current(), &sales()).objs().as_ptr(), objs);
}

#[test]
fn old_readers_keep_their_generation() {
    let (c, w) = catalog_with(vec![pg_schema("public", vec![table("t", 1, &["a"])])]);
    let g1 = c.current();
    w.publish(vec![CatalogChange::Put(Arc::new(pg_schema(
        "public",
        vec![table("t", 1, &["a", "b", "c"])],
    )))])
    .unwrap();
    assert_eq!(ns(&g1, &public()).column_count(), 1);
    assert_eq!(ns(&c.current(), &public()).column_count(), 3);
    assert_eq!(g1.generation, 1);
    assert_eq!(c.current().generation, 2);
    // Edge 1: the pinned generation is reported until its last reader lets go.
    assert_eq!(c.pinned_generations(), vec![1, 2]);
    drop(g1);
    assert_eq!(c.pinned_generations(), vec![2]);
}

#[test]
fn subtree_changes_go_through_the_catalog() {
    // Spike `subtree_changes_go_through_the_catalog`.
    let (c, w) = catalog_with(vec![pg_schema("public", vec![table("t", 1, &["a"])])]);
    let schema_key = ns(&c.current(), &public()).object_key(0).unwrap();
    w.publish(vec![CatalogChange::UpsertSubtree {
        ns: public(),
        parent: schema_key.clone(),
        subtree: table("t", 1, &["a", "b"]),
    }])
    .unwrap();
    let snap = c.current();
    let t_key = ObjectKey {
        kind: ObjectKind::Table,
        path: path(&[DB, "public", "t"]),
        server_id: Some(ServerId::Oid(1)),
        overload_sig: None,
        raw_kind: None,
    };
    let h = snap.find(&t_key).expect("the upserted table");
    assert_eq!(h.ns.children(h.id.idx).count(), 2);
    assert_eq!(snap.obj(h.id).unwrap().name().as_ref(), "t");
    assert_eq!(h.ns.generation(), 2);

    w.publish(vec![CatalogChange::RemoveSubtree {
        ns: public(),
        target: t_key.clone(),
    }])
    .unwrap();
    let snap = c.current();
    assert_eq!(ns(&snap, &public()).column_count(), 0);
    assert!(snap.find(&t_key).is_none());
}

#[test]
fn a_failing_change_publishes_nothing() {
    // Spike `a_failing_change_publishes_nothing`: all or nothing, states included.
    let (c, w) = catalog_with(vec![pg_schema("public", vec![table("t", 1, &["a"])])]);
    let before = c.current();
    #[cfg(feature = "catalog")]
    let mut rx = c.subscribe();
    for bad in [
        CatalogChange::Remove(NsKey::schema([DB, "missing"])),
        CatalogChange::UpsertSubtree {
            ns: NsKey::schema([DB, "missing"]),
            parent: ns(&before, &public()).object_key(0).unwrap(),
            subtree: table("x", 9, &[]),
        },
        CatalogChange::RemoveSubtree {
            ns: public(),
            target: ObjectKey {
                kind: ObjectKind::Table,
                path: path(&[DB, "public", "nope"]),
                server_id: None,
                overload_sig: None,
                raw_kind: None,
            },
        },
        // A column directly under a schema breaks the kind schema.
        CatalogChange::UpsertSubtree {
            ns: public(),
            parent: ns(&before, &public()).object_key(0).unwrap(),
            subtree: Subtree::new(col("c", 1)),
        },
    ] {
        let err = w
            .publish(vec![
                CatalogChange::Put(Arc::new(pg_schema("sales", vec![]))),
                CatalogChange::SetState(ContainerKey::Namespace(sales()), loaded(Level::Structure)),
                bad,
            ])
            .unwrap_err();
        assert!(
            matches!(
                err,
                ModelError::NoSuchNamespace(_)
                    | ModelError::NoSuchObject
                    | ModelError::KindNotAllowed { .. }
            ),
            "{err:?}"
        );
        let after = c.current();
        assert!(Arc::ptr_eq(&before, &after));
        assert_eq!(after.namespaces.len(), 1);
        assert!(!after.states.contains_key(&ContainerKey::Namespace(sales())));
    }
    #[cfg(feature = "catalog")]
    assert!(
        rx.try_recv().is_err(),
        "a refused publish announces nothing"
    );
}

#[test]
fn an_empty_change_list_publishes_no_generation() {
    let (c, w) = catalog_with(vec![]);
    let before = c.current();
    let p = w.publish(Vec::new()).unwrap();
    assert_eq!(p.generation, before.generation);
    assert!(p.changed.is_empty());
    assert!(Arc::ptr_eq(&before, &c.current()));
}

#[test]
fn the_catalog_has_one_writer() {
    let c = catalog();
    let w = c.publisher();
    assert!(w.is_some());
    assert!(c.publisher().is_none());
    assert!(c.publisher().is_none());
}

#[test]
fn removed_namespace_still_seen_by_old_generation() {
    // Edge 3: the reader's generation still contains it; the next one reports `Missing`.
    let (c, w) = catalog_with(vec![
        pg_schema("public", vec![]),
        pg_schema("sales", vec![table("s", 2, &["b"])]),
    ]);
    let db_only = || hacivat_meta::search_path::SearchPath::exact(Some(name(DB)), []);
    let old = SnapshotView::new(c.current()).with_search_path(db_only());
    let published = w.publish(vec![CatalogChange::Remove(sales())]).unwrap();
    assert_eq!(published.changed, vec![sales()]);
    let new = SnapshotView::new(c.current()).with_search_path(db_only());
    let q = [
        hacivat_meta::name::NamePart {
            text: name("sales"),
            quoted: false,
        },
        hacivat_meta::name::NamePart {
            text: name("s"),
            quoted: false,
        },
    ];
    assert!(matches!(
        old.lookup(&q, hacivat_meta::kind::KindGroup::Relations),
        Lookup::Found(_)
    ));
    assert!(matches!(
        new.lookup(&q, hacivat_meta::kind::KindGroup::Relations),
        Lookup::Missing(MissingReason::NotFound)
    ));
    // The namespace's own states went with it.
    assert!(
        !c.current()
            .states
            .contains_key(&ContainerKey::Namespace(sales()))
    );
}

#[test]
fn fk_resolves_after_target_namespace_replaced() {
    // Spike `references_resolve_on_demand_and_survive_a_target_reload`: an FK in `sales` to
    // `public.t`; `public` is reloaded and its arena indexes shift; the reference still
    // resolves (a `Ref` holds a path and a server id, never an arena index).
    let mut s = table("orders", 10, &["owner_id"]);
    s.add(
        s.root(),
        fk("orders_owner_fkey", &[DB, "public", "t"], Some(1)),
    );
    let (c, w) = catalog_with(vec![
        pg_schema("public", vec![table("t", 1, &["id"])]),
        pg_schema("sales", vec![s]),
    ]);
    let fk_ref = |snap: &CatalogSnapshot| {
        let fk = ns(snap, &sales())
            .objs()
            .iter()
            .find(|o| o.kind() == ObjectKind::ForeignKey)
            .unwrap()
            .clone();
        match fk.props() {
            Props::ForeignKey(p) => p.target.clone(),
            _ => unreachable!(),
        }
    };
    let r = fk_ref(&c.current());
    let Lookup::Found(hit) = c.current().resolve(&r) else {
        panic!("the FK target")
    };
    assert_eq!(hit.obj().name().as_ref(), "t");
    assert_eq!(hit.id.idx, 1);
    w.publish(vec![CatalogChange::Put(Arc::new(pg_schema(
        "public",
        vec![table("z", 5, &[]), table("t", 1, &["id", "extra"])],
    )))])
    .unwrap();
    let snap = c.current();
    let Lookup::Found(hit) = snap.resolve(&r) else {
        panic!("the FK target after the reload")
    };
    assert_eq!(hit.obj().name().as_ref(), "t");
    assert_eq!(hit.id.idx, 2);
    assert_eq!(snap.obj(hit.id).unwrap().name().as_ref(), "t");
    // The server id wins among same-named candidates; a stale id still finds the name.
    let mut stale = r.clone();
    stale.server_id = Some(ServerId::Oid(77));
    assert!(matches!(snap.resolve(&stale), Lookup::Found(_)));
    // Target namespace not loaded → unknown; loaded and absent → missing.
    let mut other = r.clone();
    other.path = path(&[DB, "hr", "emp"]);
    assert!(matches!(
        snap.resolve(&other),
        Lookup::Unknown(UnknownReason::NotLoaded)
    ));
    let mut gone = r.clone();
    gone.path = path(&[DB, "public", "gone"]);
    gone.server_id = None;
    assert!(matches!(
        snap.resolve(&gone),
        Lookup::Missing(MissingReason::NotFound)
    ));
}

#[test]
fn resolve_reaches_parent_scoped_targets() {
    // A sequence's owning column, a column of a table: walked by exact names.
    let (c, _w) = catalog_with(vec![pg_schema("public", vec![table("t", 1, &["id", "b"])])]);
    let r = Ref {
        kind: ObjectKind::Column,
        path: path(&[DB, "public", "t", "b"]),
        server_id: None,
    };
    let Lookup::Found(h) = c.current().resolve(&r) else {
        panic!("the column")
    };
    assert_eq!(h.obj().name().as_ref(), "b");
    let mut missing = r.clone();
    missing.path = path(&[DB, "public", "t", "zz"]);
    assert!(matches!(
        c.current().resolve(&missing),
        Lookup::Missing(MissingReason::NotFound)
    ));
}

fn sales_with_fks() -> Subtree {
    let mut s = table("orders", 10, &["owner_id"]);
    s.add(
        s.root(),
        fk("orders_owner_fkey", &[DB, "public", "t"], Some(1)),
    );
    s
}

#[test]
fn fk_referencing_finds_cross_namespace_children() {
    let mut local = table("notes", 11, &["owner_id"]);
    local.add(
        local.root(),
        fk("notes_owner_fkey", &[DB, "public", "t"], Some(1)),
    );
    let mut unrelated = table("lines", 12, &["owner_id"]);
    unrelated.add(
        unrelated.root(),
        fk("lines_order_fkey", &[DB, "sales", "orders"], Some(10)),
    );
    let (c, w) = catalog_with(vec![
        pg_schema("public", vec![table("t", 1, &["id"]), local]),
        pg_schema("sales", vec![sales_with_fks(), unrelated]),
    ]);
    let t = ObjectKey {
        kind: ObjectKind::Table,
        path: path(&[DB, "public", "t"]),
        server_id: Some(ServerId::Oid(1)),
        overload_sig: None,
        raw_kind: None,
    };
    let names = |snap: &CatalogSnapshot| match snap.fk_referencing(&t) {
        Lookup::Found(v) => {
            let mut n: Vec<String> = v.iter().map(|h| h.obj().name().to_string()).collect();
            n.sort();
            n
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(
        names(&c.current()),
        ["notes_owner_fkey", "orders_owner_fkey"]
    );
    // Rebuilt for the next generation: the edge from `sales` follows a reload of `public`.
    w.publish(vec![CatalogChange::Put(Arc::new(pg_schema(
        "public",
        vec![table("z", 5, &[]), table("t", 1, &["id"])],
    )))])
    .unwrap();
    assert_eq!(names(&c.current()), ["orders_owner_fkey"]);
    // A loaded table nothing references.
    let mut z = t.clone();
    z.path[2] = name("z");
    z.server_id = Some(ServerId::Oid(5));
    assert!(matches!(c.current().fk_referencing(&z), Lookup::Found(v) if v.is_empty()));
    // A table that is not there.
    let mut gone = t.clone();
    gone.path[2] = name("gone");
    gone.server_id = None;
    assert!(matches!(
        c.current().fk_referencing(&gone),
        Lookup::Missing(MissingReason::NotFound)
    ));
}

#[test]
fn fk_referencing_is_unknown_while_a_namespace_is_names_only() {
    let c = catalog();
    let w = c.publisher().unwrap();
    let mut changes = Vec::new();
    changes.extend(put(
        pg_schema("public", vec![table("t", 1, &["id"])]),
        loaded(Level::Structure),
    ));
    changes.extend(put(
        pg_schema("sales", vec![sales_with_fks()]),
        loaded(Level::Names),
    ));
    changes.push(CatalogChange::SetState(
        ContainerKey::schema_list(Some(&name(DB))),
        loaded(Level::Names),
    ));
    w.publish(changes).unwrap();
    let t = ObjectKey {
        kind: ObjectKind::Table,
        path: path(&[DB, "public", "t"]),
        server_id: Some(ServerId::Oid(1)),
        overload_sig: None,
        raw_kind: None,
    };
    assert!(matches!(
        c.current().fk_referencing(&t),
        Lookup::Unknown(UnknownReason::NotLoaded)
    ));
    // A listed schema that is still loading, or failed, keeps it unknown too.
    w.publish(vec![CatalogChange::SetState(
        ContainerKey::Namespace(sales()),
        LoadState::Loading {
            generation: 3,
            level: Level::Structure,
        },
    )])
    .unwrap();
    assert!(matches!(
        c.current().fk_referencing(&t),
        Lookup::Unknown(UnknownReason::Loading)
    ));
    // Once every in-scope namespace is at `Structure` the answer is a fact; an out-of-scope
    // schema is not part of it.
    w.publish(vec![
        CatalogChange::SetState(ContainerKey::Namespace(sales()), loaded(Level::Structure)),
        CatalogChange::SetState(
            ContainerKey::Namespace(NsKey::schema([DB, "pg_catalog"])),
            LoadState::OutOfScope,
        ),
    ])
    .unwrap();
    assert!(matches!(c.current().fk_referencing(&t), Lookup::Found(v) if v.len() == 1));
    // Without the schema list, another schema could hold a referencing FK.
    let c2 = catalog();
    let w2 = c2.publisher().unwrap();
    let mut changes = Vec::new();
    changes.extend(put(
        pg_schema("public", vec![table("t", 1, &["id"])]),
        loaded(Level::Structure),
    ));
    w2.publish(changes).unwrap();
    assert!(matches!(
        c2.current().fk_referencing(&t),
        Lookup::Unknown(UnknownReason::NotLoaded)
    ));
}

#[cfg(feature = "catalog")]
#[test]
fn publish_broadcasts_after_storing() {
    use hacivat_meta::delta::CatalogDelta;
    use hacivat_meta::snapshot::CatalogEvent;
    let (c, w) = catalog_with(vec![pg_schema("public", vec![])]);
    let mut rx = c.subscribe();
    w.publish(vec![CatalogChange::Put(Arc::new(pg_schema(
        "public",
        vec![table("t", 1, &[])],
    )))])
    .unwrap();
    // Without a delta from the merge, the changed namespaces are announced whole.
    match rx.try_recv().unwrap() {
        CatalogEvent::Published { generation, delta } => {
            assert_eq!(generation, 2);
            assert!(delta.is_empty());
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        rx.try_recv().unwrap(),
        CatalogEvent::NamespaceReplaced { generation: 2, ns } if ns == public()
    ));
    assert!(rx.try_recv().is_err());
    assert_eq!(c.current().generation, 2);
    // With the merge's delta, the delta is the announcement.
    let delta = CatalogDelta {
        created: vec![ObjectKey {
            kind: ObjectKind::Table,
            path: path(&[DB, "public", "u"]),
            server_id: Some(ServerId::Oid(3)),
            overload_sig: None,
            raw_kind: None,
        }],
        ..Default::default()
    };
    w.publish_with_delta(
        vec![CatalogChange::Put(Arc::new(pg_schema(
            "public",
            vec![table("t", 1, &[]), table("u", 3, &[])],
        )))],
        Arc::new(delta.clone()),
    )
    .unwrap();
    match rx.try_recv().unwrap() {
        CatalogEvent::Published {
            generation,
            delta: d,
        } => {
            assert_eq!(generation, 3);
            assert_eq!(*d, delta);
        }
        other => panic!("{other:?}"),
    }
    assert!(rx.try_recv().is_err());
    // A guarded merge is announced without a publish (R-M4-22).
    w.announce_pending_confirmation(public(), 42);
    assert!(matches!(
        rx.try_recv().unwrap(),
        CatalogEvent::PendingConfirmation { count: 42, .. }
    ));
    assert_eq!(c.current().generation, 3);
}

#[test]
fn readers_never_see_a_torn_generation() {
    // Spike `readers_never_see_a_torn_generation` (alg/02 test 7, reduced; the 8-thread
    // property test is `snapshot_props.rs`): each reader's view comes from one generation.
    let (c, w) = catalog_with(vec![
        pg_schema("public", vec![table("t", 1, &["a"])]),
        pg_schema("sales", vec![table("s", 2, &["a"])]),
    ]);
    let c = Arc::new(c);
    let writer = std::thread::spawn(move || {
        for n in 2..200usize {
            let cols: Vec<String> = (0..n).map(|i| format!("c{i}")).collect();
            let cols: Vec<&str> = cols.iter().map(String::as_str).collect();
            w.publish(vec![
                CatalogChange::Put(Arc::new(pg_schema("public", vec![table("t", 1, &cols)]))),
                CatalogChange::Put(Arc::new(pg_schema("sales", vec![table("s", 2, &cols)]))),
            ])
            .unwrap();
        }
    });
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let c = c.clone();
            std::thread::spawn(move || {
                for _ in 0..2_000 {
                    let snap = c.current();
                    let p = ns(&snap, &public());
                    let s = ns(&snap, &sales());
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
    send_sync::<Publisher>();
    send_sync::<CatalogSnapshot>();
    send_sync::<SnapshotView>();
}
