//! HV-MODEL-01 (M4 Task 3): namespace arenas, identity, lookups and subtree replacement
//! (alg/02 HV-MODEL-01 test cases 1–6 and edge cases 1–14; the model spike's arena tests,
//! spike origin `d86313b`). The spike's catalog-level tests (publish, resolve, torn reads) are
//! Task 9's, with `snapshot.rs`.

use std::collections::BTreeMap;
use std::sync::Arc;

use hacivat_meta::codec::{DecodeError, encode_namespace};
use hacivat_meta::fixture::{ns_mysql, ns_pg, ns_sqlite, table_subtree};
use hacivat_meta::ids::{NsKey, ObjectKey, Ref, ServerId, SourceRef};
use hacivat_meta::kind::{KindGroup, ObjectKind};
use hacivat_meta::name::{
    DialectId, FoldRule, IdentCtx, Name, NameClass, has_invisible_chars, lossy_name,
    lossy_name_utf16, match_key,
};
use hacivat_meta::namespace::{ModelError, NamespaceSnapshot, NsMeta, Obj, ObjFlags, Subtree};
use hacivat_meta::props::*;
use hacivat_meta::view::{Lookup, MissingReason, UnknownReason};
use hacivat_proto::DbmsKind;
use sql_catalog::types::{FnArg, FnKind};

const PG: DbmsKind = DbmsKind::Postgres;

fn pg_ctx() -> IdentCtx {
    IdentCtx {
        encoding: Some(Arc::from("UTF8")),
        ..IdentCtx::new(DialectId::Postgres)
    }
}

fn my_ctx(lctn: u8) -> IdentCtx {
    IdentCtx {
        lower_case_table_names: lctn,
        ..IdentCtx::new(DialectId::MySql)
    }
}

fn ik_pg(text: &str, quoted: bool) -> String {
    match_key(
        DialectId::Postgres,
        &pg_ctx(),
        text,
        quoted,
        NameClass::Table,
    )
    .unwrap()
    .as_str()
    .to_string()
}

fn col(name: &str, ordinal: u32) -> Obj {
    Obj::new(ObjectKind::Column, name).with_props(Props::Column(Arc::new(ColumnProps {
        ordinal,
        raw_type: Arc::from("integer"),
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
    let mut b = ns_pg("db", name);
    for t in tables {
        b = b.subtree(t);
    }
    b.finish()
}

fn names<'a>(it: impl Iterator<Item = (u32, &'a Obj)>) -> Vec<&'a str> {
    it.map(|(_, o)| &**o.name()).collect()
}

fn find_table(n: &NamespaceSnapshot, name: &str) -> u32 {
    n.lookup_kind(ObjectKind::Table, name)
        .next()
        .unwrap_or_else(|| panic!("table {name}"))
}

fn key_of(n: &NamespaceSnapshot, table: &str) -> ObjectKey {
    n.object_key(find_table(n, table)).unwrap()
}

fn routine(name: &str, oid: u32, sig: &str) -> Subtree {
    Subtree::new(
        Obj::new(ObjectKind::Routine, name)
            .with_server_id(ServerId::Oid(oid))
            .with_props(Props::Routine(Arc::new(RoutineProps {
                args: Some(vec![FnArg::input(None, Arc::from(sig))]),
                overload_sig: Some(Arc::from(sig)),
                ..Default::default()
            }))),
    )
}

/// alg/02 HV-MODEL-01 test 1.
#[test]
fn pg_table_key_uses_oid() {
    let n = ns_pg("db", "public")
        .table_oid("t", 16400, &["a", "b"])
        .finish();
    let t = find_table(&n, "t");
    let k = n.object_key(t).unwrap();
    assert_eq!(k.kind, ObjectKind::Table);
    assert_eq!(k.server_id, Some(ServerId::Oid(16400)));
    let path: Vec<&str> = k.path.iter().map(|p| &**p).collect();
    assert_eq!(path, ["db", "public", "t"]);
    assert_eq!(names(n.children_of_kind(t, ObjectKind::Column)), ["a", "b"]);
    assert_eq!(n.find(&k), Some(t));
    let b = n.child_by_name(t, ObjectKind::Column, "b").unwrap();
    let bkey = n.object_key(b).unwrap();
    assert_eq!(bkey.path.last().map(|p| &**p), Some("b"));
    assert_eq!(n.find(&bkey), Some(b));
    assert_eq!(n.find(&n.object_key(0).unwrap()), Some(0));
}

/// alg/02 HV-MODEL-01 test 2: a rename keeps identity by OID, and `find` follows the server id.
#[test]
fn rename_keeps_object_key_by_oid() {
    let before = ns_pg("db", "public").table_oid("t", 16400, &["a"]).finish();
    let after = ns_pg("db", "public").table_oid("u", 16400, &["a"]).finish();
    let kb = key_of(&before, "t");
    let ka = key_of(&after, "u");
    assert!(kb.same_object(&ka));
    assert_eq!(after.find(&kb), Some(find_table(&after, "u")));
}

/// `Eq`/`Hash` are structural (a transitive equality); identity across a refresh is
/// `same_object`.
#[test]
fn same_object_prefers_server_id_but_eq_is_structural() {
    let t = key_of(
        &ns_pg("db", "public").table_oid("t", 16400, &[]).finish(),
        "t",
    );
    let u = key_of(
        &ns_pg("db", "public").table_oid("u", 16400, &[]).finish(),
        "u",
    );
    assert_ne!(t, u);
    assert!(t.same_object(&u));
    let mut no_id = t.clone();
    no_id.server_id = None;
    assert!(t.same_object(&no_id), "one side without an id: by path");
    assert!(!u.same_object(&no_id));
    let mut other_kind = t.clone();
    other_kind.kind = ObjectKind::View;
    assert!(!t.same_object(&other_kind));
    let mut set = std::collections::HashSet::new();
    set.insert(t.clone());
    assert!(!set.contains(&u));
}

/// alg/02 HV-MODEL-01 test 3: no server id on MySQL, so a rename is a new object.
#[test]
fn mysql_rename_changes_key() {
    let t = ns_mysql("shop", 0).table("t", &["a"]).finish();
    let u = ns_mysql("shop", 0).table("u", &["a"]).finish();
    let (kt, ku) = (key_of(&t, "t"), key_of(&u, "u"));
    assert_ne!(kt, ku);
    assert!(!kt.same_object(&ku));
    assert_eq!(kt.server_id, None);
    assert_eq!(u.find(&kt), None);
    let path: Vec<&str> = kt.path.iter().map(|p| &**p).collect();
    assert_eq!(path, ["shop", "t"]);
}

/// alg/02 HV-MODEL-01 test 4 and edge 3.
#[test]
fn overloads_are_two_objects_lookup_returns_both() {
    let n = pg_schema(
        "public",
        vec![routine("f", 100, "integer"), routine("f", 101, "text")],
    );
    let hits = n.lookup(KindGroup::Routines, "f");
    assert_eq!(hits.len(), 2);
    let k_int = n.object_key(hits[0]).unwrap();
    let k_text = n.object_key(hits[1]).unwrap();
    assert_eq!(k_int.overload_sig.as_deref(), Some("integer"));
    assert_eq!(k_text.overload_sig.as_deref(), Some("text"));
    assert!(!k_int.same_object(&k_text));
    assert_eq!(n.find(&k_text), Some(hits[1]));
    let mut by_path = k_text.clone();
    by_path.server_id = None;
    assert_eq!(n.find(&by_path), Some(hits[1]), "path + overload signature");
    match n.match_named(KindGroup::Routines, "F", false, &pg_ctx()) {
        Lookup::Found(both) => assert_eq!(both.as_slice(), hits),
        other => panic!("{other:?}"),
    }
}

/// alg/02 HV-MODEL-01 test 5 and edge 5: a reference to an object outside the namespace keeps
/// its path and server id; this arena never resolves it, and the catalog answers `Unknown`.
#[test]
fn fk_out_of_scope_stays_unresolved() {
    let target = Ref {
        kind: ObjectKind::Table,
        path: ["db", "other_schema", "t2"]
            .into_iter()
            .map(Name::from)
            .collect(),
        server_id: Some(ServerId::Oid(777)),
    };
    let mut t = table("orders", 10, &["t2_id"]);
    t.add(
        t.root(),
        Obj::new(ObjectKind::ForeignKey, "orders_t2_fkey").with_props(Props::ForeignKey(Arc::new(
            FkProps {
                columns: vec![Arc::from("t2_id")],
                target: target.clone(),
                target_columns: vec![Arc::from("id")],
                on_update: FkAction::NoAction,
                on_delete: FkAction::Cascade,
                match_kind: FkMatch::Simple,
                deferrable: false,
                initially_deferred: false,
                not_valid: false,
                enforced: true,
                rare: None,
            },
        ))),
    );
    let n = pg_schema("public", vec![t]);
    let fk = n
        .iter_named(&[ObjectKind::ForeignKey])
        .next()
        .map(|(_, o)| o)
        .unwrap();
    let Props::ForeignKey(p) = fk.props() else {
        panic!("fk props")
    };
    assert_eq!(p.target, target);
    let target_key = ObjectKey {
        kind: target.kind,
        path: target.path.iter().cloned().collect(),
        server_id: target.server_id,
        overload_sig: None,
        raw_kind: None,
    };
    assert_eq!(n.find(&target_key), None);
    let catalog =
        hacivat_meta::Catalog::new(Arc::new(hacivat_meta::ServerFacts::new(PG, pg_ctx())));
    assert!(matches!(
        catalog.current().resolve(&target),
        Lookup::Unknown(UnknownReason::NotLoaded)
    ));
}

/// alg/02 HV-MODEL-01 edge 1 and test 6: `"Users"` and `users` are two objects; unquoted
/// `users` (folded by `match_key`) finds only `users`.
#[test]
fn case_twins_are_two_objects() {
    let n = ns_pg("db", "public")
        .table_oid("Users", 1, &[])
        .table_oid("users", 2, &[])
        .finish();
    assert_eq!(
        n.lookup(KindGroup::Relations, &ik_pg("users", false)).len(),
        1
    );
    assert_eq!(
        n.lookup(KindGroup::Relations, &ik_pg("Users", true)).len(),
        1
    );
    assert_eq!(
        n.lookup(KindGroup::Relations, &ik_pg("USERS", false)),
        n.lookup(KindGroup::Relations, "users")
    );
    assert_eq!(n.lookup(KindGroup::Relations, "USERS").len(), 0);
    assert_ne!(
        n.lookup(KindGroup::Relations, "Users"),
        n.lookup(KindGroup::Relations, "users")
    );
    let unquoted = n.match_named(KindGroup::Relations, "USERS", false, &pg_ctx());
    let Lookup::Found(hit) = unquoted else {
        panic!("{unquoted:?}")
    };
    assert_eq!(&**n.obj(hit[0]).unwrap().name(), "users");
    assert!(matches!(
        n.match_named(KindGroup::Relations, "USERS", true, &pg_ctx()),
        Lookup::Missing(MissingReason::NotFound)
    ));
}

/// The spike's interleaved build: children stay contiguous, in insertion (ordinal) order, and a
/// parent precedes its children.
#[test]
fn children_contiguous_in_ordinal_order() {
    let mut t = Subtree::new(Obj::new(ObjectKind::Schema, "public"));
    let a = t.add(t.root(), Obj::new(ObjectKind::Table, "a"));
    let b = t.add(t.root(), Obj::new(ObjectKind::Table, "b"));
    t.add(b, col("x", 1));
    t.add(a, col("c1", 1));
    t.add(a, col("c2", 2));
    t.add(b, col("y", 2));
    t.add(a, col("c3", 3));
    let ns = NamespaceSnapshot::build(
        NsKey::schema(["db", "public"]),
        PG,
        FoldRule::Exact,
        NsMeta::default(),
        t,
    )
    .unwrap();
    let tabs: Vec<u32> = ns.children(0).map(|(i, _)| i).collect();
    assert_eq!(names(ns.children(0)), ["a", "b"]);
    assert_eq!(names(ns.children(tabs[0])), ["c1", "c2", "c3"]);
    assert_eq!(names(ns.children(tabs[1])), ["x", "y"]);
    let first = ns.children(tabs[0]).map(|(i, _)| i).collect::<Vec<_>>();
    assert!(first.windows(2).all(|w| w[1] == w[0] + 1), "contiguous");
    for (i, o) in ns.objs().iter().enumerate().skip(1) {
        assert!(o.parent().unwrap() < i as u32, "parent precedes child");
    }
    assert_eq!(ns.obj(0).unwrap().parent(), None);
    assert_eq!(ns.obj(tabs[0]).unwrap().child_count(), 3);
}

/// edge 14: a 1,600-column table (PostgreSQL's limit) builds, keeps order and finds its last
/// column (one scan of the contiguous children; linearity is by construction, not timed).
#[test]
fn a_1600_column_table_builds_and_finds_its_last_column() {
    let cols: Vec<String> = (1..=1600).map(|i| format!("c{i}")).collect();
    let cols: Vec<&str> = cols.iter().map(String::as_str).collect();
    let n = ns_pg("db", "public").table_oid("wide", 1, &cols).finish();
    let t = find_table(&n, "wide");
    assert_eq!(n.children(t).count(), 1600);
    assert_eq!(
        n.child_by_name(t, ObjectKind::Column, "c1600"),
        Some(t + 1600)
    );
    assert_eq!(n.column_count(), 1600);
}

/// edge 11 and the T1a review carry m10: an unknown server kind is kept with its raw kind text,
/// and its key keeps the raw kind through a serde round trip, so two newer kinds never collide
/// as user-data map keys.
#[test]
fn unknown_kind_kept_with_raw() {
    let unknown = |name: &str, raw: &str| {
        Subtree::new(
            Obj::new(ObjectKind::Unknown, name).with_props(Props::Generic(Arc::new(
                GenericProps {
                    raw_kind: Some(Arc::from(raw)),
                    ..Default::default()
                },
            ))),
        )
    };
    let n = pg_schema(
        "public",
        vec![unknown("x", "relkind-q"), unknown("y", "relkind-z")],
    );
    let kx = n.object_key(1).unwrap();
    let ky = n.object_key(2).unwrap();
    assert_eq!(kx.kind, ObjectKind::Unknown);
    assert_eq!(kx.raw_kind.as_deref(), Some("relkind-q"));
    assert_eq!(ky.raw_kind.as_deref(), Some("relkind-z"));
    assert_eq!(n.find(&kx), Some(1));
    let mut wrong_raw = kx.clone();
    wrong_raw.raw_kind = Some(Arc::from("relkind-z"));
    assert_eq!(n.find(&wrong_raw), None);
    assert!(!kx.same_object(&wrong_raw));
    // Two unknown kinds at one path are two keys.
    let mut map = BTreeMap::new();
    map.insert(kx.clone(), "first");
    map.insert(wrong_raw.clone(), "second");
    assert_eq!(map.len(), 2);
    // A known kind never carries a raw kind.
    let kt = key_of(&ns_pg("db", "public").table_oid("t", 1, &[]).finish(), "t");
    assert_eq!(kt.raw_kind, None);
    #[cfg(feature = "system")]
    {
        // Serde (the user-data form): the raw kind travels as the kind id and reads back
        // exactly, so an older build writing the file back keeps both entries.
        let json = serde_json::to_string(&kx).unwrap();
        assert!(json.contains("\"relkind-q\""), "{json}");
        assert!(!json.contains("\"unknown\""), "{json}");
        let back: ObjectKey = serde_json::from_str(&json).unwrap();
        assert_eq!(back, kx);
        let back_map: Vec<(ObjectKey, String)> =
            serde_json::from_str(&serde_json::to_string(&map.iter().collect::<Vec<_>>()).unwrap())
                .unwrap();
        assert_eq!(back_map.len(), 2);
        assert_ne!(back_map[0].0, back_map[1].0);
        let kt_json = serde_json::to_string(&kt).unwrap();
        assert_eq!(serde_json::from_str::<ObjectKey>(&kt_json).unwrap(), kt);
        // An id this build does not know reads back as `Unknown` with that id kept.
        let newer = r#"{"kind":"graph-label","path":["db","public","g"]}"#;
        let k: ObjectKey = serde_json::from_str(newer).unwrap();
        assert_eq!(k.kind, ObjectKind::Unknown);
        assert_eq!(k.raw_kind.as_deref(), Some("graph-label"));
        assert_eq!(serde_json::to_string(&k).unwrap(), newer);
        // A server's raw kind text that equals a known id still reads back as unknown.
        let mut colliding = kx.clone();
        colliding.raw_kind = Some(Arc::from("table"));
        let json = serde_json::to_string(&colliding).unwrap();
        assert_eq!(
            serde_json::from_str::<ObjectKey>(&json).unwrap(),
            colliding,
            "{json}"
        );
    }
}

/// edge 12: a name that was not valid UTF-8 (or UTF-16 with a lone surrogate), or that holds
/// NUL, is stored lossily and flagged `LOSSY_NAME` (never used to generate DDL).
#[test]
fn lossy_name_flag_on_nul_and_lone_surrogate_text() {
    let (bad_utf8, lossy) = lossy_name(b"caf\xE9");
    assert!(lossy);
    assert_eq!(&*bad_utf8, "caf\u{FFFD}");
    let (fine, lossy) = lossy_name("café".as_bytes());
    assert!(!lossy);
    assert_eq!(&*fine, "café");
    let (nul, lossy) = lossy_name(b"a\0b");
    assert!(lossy);
    assert_eq!(&*nul, "a\0b");
    let (surrogate, lossy) = lossy_name_utf16(&[0x0061, 0xD800, 0x0062]);
    assert!(lossy);
    assert_eq!(&*surrogate, "a\u{FFFD}b");
    let (ok16, lossy) = lossy_name_utf16(&"ışık".encode_utf16().collect::<Vec<_>>());
    assert!(!lossy);
    assert_eq!(&*ok16, "ışık");
    // A genuine U+FFFD in a valid name is not a lossy name.
    assert!(!lossy_name("a\u{FFFD}".as_bytes()).1);

    // In the arena: the constructor's flag survives, and a NUL name is flagged on build.
    let mut t = Subtree::new(Obj::new(ObjectKind::Schema, "public"));
    t.add(
        t.root(),
        Obj::from_server_bytes(ObjectKind::Table, b"caf\xE9"),
    );
    t.add(t.root(), Obj::new(ObjectKind::Table, "a\0b"));
    t.add(t.root(), Obj::new(ObjectKind::Table, "plain"));
    let n = NamespaceSnapshot::build(
        NsKey::schema(["db", "public"]),
        PG,
        FoldRule::Exact,
        NsMeta::default(),
        t,
    )
    .unwrap();
    let flags: Vec<bool> = n
        .children(0)
        .map(|(_, o)| o.flags().contains(ObjFlags::LOSSY_NAME))
        .collect();
    assert_eq!(flags, [true, true, false]);
}

/// edge 13: names are stored byte-exact; no NFC normalization, so a decomposed `é` is not the
/// precomposed one.
#[test]
fn non_nfc_names_stored_exactly() {
    let decomposed = "cafe\u{301}";
    let precomposed = "caf\u{e9}";
    let n = ns_pg("db", "public").table_oid(decomposed, 1, &[]).finish();
    let t = find_table(&n, decomposed);
    assert_eq!(&**n.obj(t).unwrap().name(), decomposed);
    assert_eq!(n.lookup(KindGroup::Relations, precomposed).len(), 0);
    assert_eq!(
        n.object_key(t).unwrap().path.last().map(|p| &**p),
        Some(decomposed)
    );
    let s = ns_sqlite("main").table(decomposed, &[]).finish();
    assert_eq!(s.lookup(KindGroup::Relations, precomposed).len(), 0);
    assert_eq!(s.lookup(KindGroup::Relations, decomposed).len(), 1);
}

/// edge 7: inherited columns keep their flag through the build, beside own columns.
#[test]
fn inherited_columns_flagged() {
    let mut t = table("child", 2, &["own"]);
    t.add(
        t.root(),
        col("from_parent", 2).add_flags(ObjFlags::INHERITED),
    );
    let n = pg_schema("public", vec![t]);
    let child = find_table(&n, "child");
    let inherited: Vec<&str> = n
        .children_of_kind(child, ObjectKind::Column)
        .filter(|(_, o)| o.flags().contains(ObjFlags::INHERITED))
        .map(|(_, o)| &**o.name())
        .collect();
    assert_eq!(inherited, ["from_parent"]);
}

/// edge 8: a temp namespace (root flagged `TEMP`) says so; its objects keep their flag.
#[test]
fn temp_objects_flagged_temp() {
    let mut t = Subtree::new(Obj::new(ObjectKind::Schema, "pg_temp_3").add_flags(ObjFlags::TEMP));
    t.add(
        t.root(),
        Obj::new(ObjectKind::Table, "scratch").add_flags(ObjFlags::TEMP),
    );
    let n = NamespaceSnapshot::build(
        NsKey::schema(["db", "pg_temp_3"]),
        PG,
        FoldRule::Exact,
        NsMeta::default(),
        t,
    )
    .unwrap();
    assert!(n.is_temp());
    assert!(n.obj(1).unwrap().flags().contains(ObjFlags::TEMP));
    assert!(!pg_schema("public", vec![]).is_temp());
}

/// The explorer badge: invisible characters are detected, and the arena sets
/// `INVISIBLE_CHARS` on build (a derived flag, whatever the introspector passed).
#[test]
fn invisible_chars_detected() {
    assert!(has_invisible_chars("a\u{200D}b"));
    assert!(has_invisible_chars("evil\u{202E}txt"));
    assert!(!has_invisible_chars("müşteri"));
    assert!(!has_invisible_chars("İSTANBUL ılık"));
    let n = ns_pg("db", "public")
        .table_oid("a\u{200B}b", 1, &[])
        .table_oid("ab", 2, &[])
        .finish();
    let flagged: Vec<bool> = n
        .children(0)
        .map(|(_, o)| o.flags().contains(ObjFlags::INVISIBLE_CHARS))
        .collect();
    assert_eq!(flagged, [true, false]);
}

/// Task 11 owns the codec body; until it lands, encoding any namespace — here one with every
/// `Props` variant — answers the stub error (reported). Task 11 turns this into the round trip.
#[test]
fn props_round_trip_through_the_codec_for_every_variant() {
    let n = every_props_variant();
    assert_eq!(n.len(), 15);
    assert!(matches!(
        encode_namespace(&n),
        Err(DecodeError::NotImplemented)
    ));
}

fn every_props_variant() -> NamespaceSnapshot {
    let r = Ref {
        kind: ObjectKind::Table,
        path: ["db", "public", "t"].into_iter().map(Name::from).collect(),
        server_id: Some(ServerId::Oid(1)),
    };
    let mut t = Subtree::new(
        Obj::new(ObjectKind::Schema, "public")
            .with_props(Props::Schema(Arc::new(SchemaProps::default()))),
    );
    let root = t.root();
    let tab = t.add(
        root,
        Obj::new(ObjectKind::Table, "t").with_props(Props::Table(Arc::new(TableProps::default()))),
    );
    t.add(tab, col("c", 1));
    t.add(
        tab,
        Obj::new(ObjectKind::Index, "t_idx")
            .with_props(Props::Index(Arc::new(IndexProps::default()))),
    );
    t.add(
        tab,
        Obj::new(ObjectKind::PrimaryKey, "t_pkey")
            .with_props(Props::Key(Arc::new(KeyProps::default()))),
    );
    t.add(
        tab,
        Obj::new(ObjectKind::ForeignKey, "t_fk").with_props(Props::ForeignKey(Arc::new(FkProps {
            columns: vec![Arc::from("c")],
            target: r.clone(),
            target_columns: vec![Arc::from("c")],
            on_update: FkAction::default(),
            on_delete: FkAction::default(),
            match_kind: FkMatch::default(),
            deferrable: false,
            initially_deferred: false,
            not_valid: false,
            enforced: true,
            rare: None,
        }))),
    );
    t.add(
        tab,
        Obj::new(ObjectKind::Check, "t_check")
            .with_props(Props::Check(Arc::new(CheckProps::default()))),
    );
    t.add(
        tab,
        Obj::new(ObjectKind::Trigger, "t_trg")
            .with_props(Props::Trigger(Arc::new(TriggerProps::default()))),
    );
    t.add(
        root,
        Obj::new(ObjectKind::View, "v").with_props(Props::View(Arc::new(ViewProps {
            source: Some(SourceRef::of("SELECT 1")),
            ..Default::default()
        }))),
    );
    t.add(
        root,
        Obj::new(ObjectKind::Routine, "f").with_props(Props::Routine(Arc::new(RoutineProps {
            kind: FnKind::Function,
            ..Default::default()
        }))),
    );
    t.add(
        root,
        Obj::new(ObjectKind::Sequence, "s")
            .with_props(Props::Sequence(Arc::new(SeqProps::default()))),
    );
    t.add(
        root,
        Obj::new(ObjectKind::EnumType, "mood").with_props(Props::Type(Arc::new(TypeProps {
            body: TypeBody::Enum {
                labels: Arc::from([Name::from("sad"), Name::from("ok")]),
            },
            ..Default::default()
        }))),
    );
    t.add(
        root,
        Obj::new(ObjectKind::Collation, "c_tr")
            .with_props(Props::Generic(Arc::new(GenericProps::default()))),
    );
    t.add(root, Obj::new(ObjectKind::Domain, "d"));
    t.add(
        root,
        Obj::new(ObjectKind::Unknown, "u").with_props(Props::Generic(Arc::new(GenericProps {
            raw_kind: Some(Arc::from("relkind-q")),
            ..Default::default()
        }))),
    );
    let n = NamespaceSnapshot::build(
        NsKey::schema(["db", "public"]),
        PG,
        FoldRule::Exact,
        NsMeta::default(),
        t,
    )
    .unwrap();
    // Database props live in the database namespace; the schema root covers `Schema`.
    let db = NamespaceSnapshot::build(
        NsKey::database("db"),
        PG,
        FoldRule::Exact,
        NsMeta::default(),
        Subtree::new(
            Obj::new(ObjectKind::Database, "db")
                .with_props(Props::Database(Arc::new(DatabaseProps::default()))),
        ),
    )
    .unwrap();
    assert!(matches!(db.obj(0).unwrap().props(), Props::Database(_)));
    n
}

/// The kind schema is checked on build: a column cannot sit directly under a schema.
#[test]
fn kind_schema_rejects_disallowed_child() {
    let mut t = Subtree::new(Obj::new(ObjectKind::Schema, "public"));
    t.add(t.root(), col("loose", 1));
    let err = NamespaceSnapshot::build(
        NsKey::schema(["db", "public"]),
        PG,
        FoldRule::Exact,
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
    // An engine-specific kind is refused on another engine: no MySQL event in PostgreSQL.
    let mut t = Subtree::new(Obj::new(ObjectKind::Schema, "public"));
    t.add(t.root(), Obj::new(ObjectKind::Event, "nightly"));
    assert!(matches!(
        NamespaceSnapshot::build(
            NsKey::schema(["db", "public"]),
            PG,
            FoldRule::Exact,
            NsMeta::default(),
            t
        ),
        Err(ModelError::KindNotAllowed { .. })
    ));
}

#[test]
fn the_root_must_match_the_namespace_key() {
    let t = Subtree::new(Obj::new(ObjectKind::Schema, "other"));
    assert!(matches!(
        NamespaceSnapshot::build(
            NsKey::schema(["db", "public"]),
            PG,
            FoldRule::Exact,
            NsMeta::default(),
            t
        ),
        Err(ModelError::RootName { .. })
    ));
    let t = Subtree::new(Obj::new(ObjectKind::Database, "public"));
    assert!(matches!(
        NamespaceSnapshot::build(
            NsKey::schema(["db", "public"]),
            PG,
            FoldRule::Exact,
            NsMeta::default(),
            t
        ),
        Err(ModelError::RootKind { .. })
    ));
}

/// A `NodeId` of another subtree (a parent outside the tree, or a cycle) is a bad layout, never
/// a panic.
#[test]
fn arena_rejects_child_before_parent_and_non_contiguous_children() {
    let mut other = Subtree::new(Obj::new(ObjectKind::Schema, "x"));
    let mut far = other.root();
    for i in 0..5 {
        far = other.add(far, Obj::new(ObjectKind::Table, format!("t{i}").as_str()));
    }
    let mut t = Subtree::new(Obj::new(ObjectKind::Schema, "public"));
    t.add(far, Obj::new(ObjectKind::Table, "dangling"));
    assert!(matches!(
        NamespaceSnapshot::build(
            NsKey::schema(["db", "public"]),
            PG,
            FoldRule::Exact,
            NsMeta::default(),
            t
        ),
        Err(ModelError::BadLayout(_))
    ));
    // The decoder-side checks (parent after child, non-contiguous children) are unit tests in
    // `namespace.rs` (`from_arena` is crate-private).
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
    let b = find_table(&ns, "b");
    let next = ns
        .replace_subtree(b, Some(table("b", 2, &["y", "y2", "y3"])))
        .unwrap();
    assert_eq!(names(next.children(0)), ["a", "b", "c"]);
    let nb = find_table(&next, "b");
    assert_eq!(names(next.children(nb)), ["y", "y2", "y3"]);
    let na = find_table(&next, "a");
    assert_eq!(names(next.children(na)), ["x"]);
    assert_eq!(ns.children(b).count(), 1, "the old arena is unchanged");
    let gone = next.replace_subtree(nb, None).unwrap();
    assert_eq!(names(gone.children(0)), ["a", "c"]);
    assert_eq!(gone.column_count(), 2);
    assert_eq!(gone.lookup(KindGroup::Relations, "b").len(), 0);
    assert_eq!(
        ns.replace_subtree(0, None).unwrap_err(),
        ModelError::NoSuchObject
    );
    assert_eq!(
        ns.replace_subtree(99, None).unwrap_err(),
        ModelError::NoSuchObject
    );
    // The replacement is validated against the kind schema too.
    let mut bad = table("b", 2, &[]);
    bad.add(bad.root(), Obj::new(ObjectKind::Schema, "nested"));
    assert!(matches!(
        ns.replace_subtree(b, Some(bad)),
        Err(ModelError::KindNotAllowed { .. })
    ));
}

#[test]
fn upsert_child_replaces_by_identity_or_appends() {
    let ns = pg_schema("public", vec![table("a", 1, &["x"]), table("b", 2, &["y"])]);
    let renamed = ns.upsert_child(0, table("a2", 1, &["x", "w"])).unwrap();
    assert_eq!(names(renamed.children(0)), ["a2", "b"]);
    let added = renamed.upsert_child(0, table("c", 3, &[])).unwrap();
    assert_eq!(names(added.children(0)), ["a2", "b", "c"]);
    assert_eq!(added.lookup(KindGroup::Relations, "c").len(), 1);
    assert_eq!(added.lookup(KindGroup::Relations, "a").len(), 0);
    let schema_cols = added
        .upsert_child(find_table(&added, "c"), Subtree::new(col("k", 1)))
        .unwrap();
    assert_eq!(
        names(schema_cols.children(find_table(&schema_cols, "c"))),
        ["k"]
    );
    assert_eq!(
        ns.upsert_child(99, table("z", 9, &[])).unwrap_err(),
        ModelError::NoSuchObject
    );
}

/// The fixture's `table_subtree` (shared with Tasks 9–11) grafts as one table.
#[test]
fn table_subtree_fixture_upserts_as_one_table() {
    let ns = pg_schema("public", vec![table("a", 1, &["x"])]);
    let next = ns
        .upsert_child(0, table_subtree("public", "orders", 5, 50_000, Some("a")))
        .unwrap();
    let o = find_table(&next, "orders");
    assert_eq!(next.children_of_kind(o, ObjectKind::Column).count(), 5);
    assert_eq!(next.children_of_kind(o, ObjectKind::PrimaryKey).count(), 1);
    assert_eq!(next.children_of_kind(o, ObjectKind::ForeignKey).count(), 1);
    assert_eq!(
        next.lookup(KindGroup::Relations, "orders_created_at_idx")
            .len(),
        1,
        "PG indexes are relations"
    );
}

/// The spike's MySQL fold check: relations exact under `lower_case_table_names = 0`, columns
/// ASCII-caseless, and nothing non-ASCII is ever case-mapped.
#[test]
fn mysql_columns_fold_ascii_only() {
    let ns = ns_mysql("shop", 0)
        .table("Orders", &["MÜŞTERİ_Id"])
        .finish();
    assert_eq!(ns.lookup(KindGroup::Relations, "orders").len(), 0);
    assert_eq!(ns.lookup(KindGroup::Relations, "Orders").len(), 1);
    let t = find_table(&ns, "Orders");
    assert!(
        ns.child_by_name(t, ObjectKind::Column, "MÜŞTERİ_ID")
            .is_some()
    );
    assert!(
        ns.child_by_name(t, ObjectKind::Column, "müşteri_id")
            .is_none()
    );
    let lctn1 = ns_mysql("shop", 1).table("orders", &[]).finish();
    assert_eq!(lctn1.lookup(KindGroup::Relations, "ORDERS").len(), 1);
}

/// R-M4-41: a MySQL column probe with a non-ASCII cased letter is `Found` only on an exact
/// stored-name match and `Unknown(ServerDependent)` otherwise, never `Missing`; a namespace whose
/// stored names hold non-ASCII cased letters answers `Unknown` instead of `Missing` for ASCII
/// probes too (the server may fold `İD` to `id`).
#[test]
fn mysql_non_ascii_cased_column_is_server_dependent_until_pinned() {
    assert!(match_key(DialectId::MySql, &my_ctx(0), "Äb", false, NameClass::Column).is_err());
    let n = ns_mysql("shop", 0).table("t", &["Äb", "plain"]).finish();
    let t = find_table(&n, "t");
    let ctx = my_ctx(0);
    let lookup_column = |probe: &str| n.match_child(t, ObjectKind::Column, probe, false, &ctx);
    assert!(matches!(
        lookup_column("äb"),
        Lookup::Unknown(UnknownReason::ServerDependent)
    ));
    assert!(matches!(lookup_column("Äb"), Lookup::Found(i) if i == t + 1));
    assert!(matches!(lookup_column("PLAIN"), Lookup::Found(i) if i == t + 2));
    // The stored `Äb` makes an ASCII miss uncertain: never `Missing`.
    assert!(matches!(
        lookup_column("ab"),
        Lookup::Unknown(UnknownReason::ServerDependent)
    ));
    // Without non-ASCII cased names, an ASCII miss is `Missing`.
    let plain = ns_mysql("shop", 0).table("t", &["a", "日本"]).finish();
    let pt = find_table(&plain, "t");
    assert!(matches!(
        plain.match_child(pt, ObjectKind::Column, "b", false, &ctx),
        Lookup::Missing(MissingReason::NotFound)
    ));
    assert!(matches!(
        plain.match_child(pt, ObjectKind::Column, "日本", false, &ctx),
        Lookup::Found(_)
    ));
    // Tables: exact under lctn = 0, so a non-ASCII table probe is decided; under lctn = 1 it is
    // server-dependent.
    let tabs = ns_mysql("shop", 0).table("Müşteri", &[]).finish();
    assert!(matches!(
        tabs.match_named(KindGroup::Relations, "Müşteri", false, &my_ctx(0)),
        Lookup::Found(_)
    ));
    assert!(matches!(
        tabs.match_named(KindGroup::Relations, "müşteri", false, &my_ctx(0)),
        Lookup::Missing(MissingReason::NotFound)
    ));
    let tabs1 = ns_mysql("shop", 1).table("müşteri", &[]).finish();
    assert!(matches!(
        tabs1.match_named(KindGroup::Relations, "Müşteri", false, &my_ctx(1)),
        Lookup::Unknown(UnknownReason::ServerDependent)
    ));
    assert!(matches!(
        tabs1.match_named(KindGroup::Relations, "müşteri", false, &my_ctx(1)),
        Lookup::Found(_)
    ));
}

/// A probe context that disagrees with the namespace's stored policy (a namespace built under
/// `lower_case_table_names = 0` probed with 1, or another dialect) cannot decide: `Unknown`.
#[test]
fn a_probe_context_that_disagrees_with_the_namespace_is_unknown() {
    let n = ns_mysql("shop", 0).table("Items", &[]).finish();
    assert!(matches!(
        n.match_named(KindGroup::Relations, "items", false, &my_ctx(1)),
        Lookup::Unknown(UnknownReason::ServerDependent)
    ));
    assert!(matches!(
        n.match_named(KindGroup::Relations, "Items", false, &pg_ctx()),
        Lookup::Unknown(UnknownReason::ServerDependent)
    ));
}

/// PostgreSQL never answers `Found` for an unpinned (non-UTF-8 server) unquoted non-ASCII probe:
/// the server may fold it beyond ASCII.
#[test]
fn pg_unpinned_probe_is_unknown_never_found() {
    let n = ns_pg("db", "public").table_oid("Müşteri", 1, &[]).finish();
    let latin1 = IdentCtx {
        encoding: Some(Arc::from("LATIN1")),
        ..IdentCtx::new(DialectId::Postgres)
    };
    assert!(matches!(
        n.match_named(KindGroup::Relations, "Müşteri", false, &latin1),
        Lookup::Unknown(UnknownReason::ServerDependent)
    ));
    assert!(matches!(
        n.match_named(KindGroup::Relations, "Müşteri", true, &latin1),
        Lookup::Found(_)
    ));
    assert!(matches!(
        n.match_named(KindGroup::Relations, "Müşteri", true, &pg_ctx()),
        Lookup::Found(_)
    ));
}

/// ver/01 `#sqlite-case` through the arena: `Müşteri` = `müşteri`, `MÜŞTERİ` ≠ `müşteri`.
#[test]
fn sqlite_lookups_are_ascii_caseless() {
    let n = ns_sqlite("main").table("müşteri", &["Ad"]).finish();
    let ctx = IdentCtx::new(DialectId::Sqlite);
    assert!(matches!(
        n.match_named(KindGroup::Relations, "Müşteri", false, &ctx),
        Lookup::Found(_)
    ));
    assert!(matches!(
        n.match_named(KindGroup::Relations, "MÜŞTERİ", false, &ctx),
        Lookup::Missing(MissingReason::NotFound)
    ));
    let t = find_table(&n, "müşteri");
    assert!(matches!(
        n.match_child(t, ObjectKind::Column, "AD", true, &ctx),
        Lookup::Found(_)
    ));
}

#[test]
fn iter_named_walks_arena_order() {
    let n = ns_pg("db", "public")
        .table_oid("a", 1, &["x"])
        .subtree(routine("f", 2, "integer"))
        .table_oid("b", 3, &["y"])
        .finish();
    let tables: Vec<&str> = n
        .iter_named(&[ObjectKind::Table, ObjectKind::Routine])
        .map(|(_, o)| &**o.name())
        .collect();
    assert_eq!(tables, ["a", "f", "b"]);
    assert_eq!(n.iter_named(&[ObjectKind::Column]).count(), 2);
}

#[test]
fn snapshots_cross_threads() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<NamespaceSnapshot>();
    send_sync::<Subtree>();
    send_sync::<ObjectKey>();
}

fn ns_with(dbms: DbmsKind, key: NsKey, fold: FoldRule, tree: Subtree) -> NamespaceSnapshot {
    NamespaceSnapshot::build(key, dbms, fold, NsMeta::default(), tree).unwrap()
}

/// Fix round 1, I2 (MySQL 8.4 §8.2.3: `User` comparisons are case-sensitive): account case
/// twins are two objects, and a probe differing only by case is `Missing`, never `Found`.
#[test]
fn mysql_account_case_twins_are_two_objects() {
    let mut t = Subtree::new(Obj::new(ObjectKind::Root, ""));
    t.add(t.root(), Obj::new(ObjectKind::User, "Bob"));
    t.add(t.root(), Obj::new(ObjectKind::User, "bob"));
    t.add(t.root(), Obj::new(ObjectKind::Role, "Admins"));
    for lctn in [0, 1] {
        let fold = if lctn == 0 {
            FoldRule::Exact
        } else {
            FoldRule::AsciiCaseless
        };
        let n = ns_with(DbmsKind::MySql, NsKey::server(), fold, t.clone());
        let ctx = my_ctx(lctn);
        let found = |probe: &str| -> Vec<String> {
            match n.match_named(KindGroup::Accounts, probe, false, &ctx) {
                Lookup::Found(hits) => hits
                    .iter()
                    .map(|&i| n.obj(i).unwrap().name().to_string())
                    .collect(),
                other => panic!("{probe}: {other:?}"),
            }
        };
        assert_eq!(found("bob"), ["bob"]);
        assert_eq!(found("Bob"), ["Bob"]);
        for probe in ["BOB", "admins"] {
            assert!(
                matches!(
                    n.match_named(KindGroup::Accounts, probe, false, &ctx),
                    Lookup::Missing(MissingReason::NotFound)
                ),
                "{probe}"
            );
        }
        assert_eq!(n.lookup(KindGroup::Other, "bob").len(), 0);
    }
}

/// Fix round 1, I3 (MySQL 8.4 §11.2.3: `lower_case_table_names` "does not affect case
/// sensitivity of trigger identifiers"): exact under 0; under 1 only a byte-exact stored name
/// is `Found`, and every other probe is `Unknown`.
#[test]
fn mysql_trigger_names_exact_under_lctn0_unpinned_above() {
    let tree = || {
        let mut t = Subtree::new(Obj::new(ObjectKind::Table, "t"));
        t.add(t.root(), Obj::new(ObjectKind::Trigger, "Trg"));
        t
    };
    let n0 = ns_mysql("shop", 0).subtree(tree()).finish();
    let ctx0 = my_ctx(0);
    assert!(matches!(
        n0.match_named(KindGroup::Triggers, "Trg", false, &ctx0),
        Lookup::Found(_)
    ));
    assert!(matches!(
        n0.match_named(KindGroup::Triggers, "trg", false, &ctx0),
        Lookup::Missing(MissingReason::NotFound)
    ));
    let n1 = ns_mysql("shop", 1).subtree(tree()).finish();
    let ctx1 = my_ctx(1);
    assert!(matches!(
        n1.match_named(KindGroup::Triggers, "Trg", false, &ctx1),
        Lookup::Found(_)
    ));
    for probe in ["trg", "TRG", "other"] {
        assert!(
            matches!(
                n1.match_named(KindGroup::Triggers, probe, false, &ctx1),
                Lookup::Unknown(UnknownReason::ServerDependent)
            ),
            "{probe}"
        );
    }
    let t1 = find_table(&n1, "t");
    assert!(matches!(
        n1.match_child(t1, ObjectKind::Trigger, "trg", false, &ctx1),
        Lookup::Unknown(UnknownReason::ServerDependent)
    ));
}

/// Fix round 1 (m9): a duplicated `(kind, server id)` is reported, and neither object is found
/// by that id alone; the exact path still finds each one, checking its id.
#[test]
fn duplicate_server_ids_are_reported_not_overwritten() {
    let n = ns_pg("db", "public")
        .table_oid("a", 7, &[])
        .table_oid("b", 7, &[])
        .table_oid("c", 8, &[])
        .finish();
    assert_eq!(n.duplicate_server_ids(), 2);
    let (ka, kb) = (key_of(&n, "a"), key_of(&n, "b"));
    assert_eq!(n.find(&ka), Some(find_table(&n, "a")));
    assert_eq!(n.find(&kb), Some(find_table(&n, "b")));
    let mut moved = ka.clone();
    *moved.path.last_mut().unwrap() = Name::from("renamed");
    assert_eq!(
        n.find(&moved),
        None,
        "an ambiguous id never picks one object"
    );
    let mut renamed_c = key_of(&n, "c");
    *renamed_c.path.last_mut().unwrap() = Name::from("old_c");
    assert_eq!(
        n.find(&renamed_c),
        Some(find_table(&n, "c")),
        "a unique id still follows a rename"
    );
    assert_eq!(pg_schema("public", vec![]).duplicate_server_ids(), 0);
}

/// Ruling R-W1-2: a column object stays within alg/02's 250-byte target (arena slot, props,
/// and the props' `Arc` header): the rare fields and the engine extras sit behind pointers.
#[cfg(target_pointer_width = "64")]
#[test]
fn a_column_stays_within_the_memory_target() {
    let arc_header = 2 * std::mem::size_of::<usize>();
    let column = std::mem::size_of::<Obj>() + std::mem::size_of::<ColumnProps>() + arc_header;
    assert!(column <= 250, "{column} bytes per column");
    assert_eq!(std::mem::size_of::<Option<Arc<ColumnRare>>>(), 8);
    assert_eq!(std::mem::size_of::<Option<Arc<ColumnExt>>>(), 8);
    let rare = ColumnProps {
        rare: Some(Arc::new(ColumnRare {
            identity: Some(Identity::Always),
            collation: Some(Arc::from("tr-x-icu")),
            ..Default::default()
        })),
        ..Default::default()
    };
    assert_eq!(rare.identity(), Some(Identity::Always));
    assert_eq!(rare.collation().map(|c| &**c), Some("tr-x-icu"));
    assert_eq!(rare.generated(), None);
    assert_eq!(*rare.ext(), ColumnExt::None);
}

/// Fix round 1 (m5): only an unknown kind carries a raw kind.
#[test]
fn object_key_raw_kind_only_on_unknown_kinds() {
    let path: smallvec::SmallVec<[Name; 4]> =
        ["db", "public", "x"].into_iter().map(Name::from).collect();
    let k = ObjectKey::unknown(path, None, Some(Name::from("relkind-q")));
    assert!(k.is_well_formed());
    assert_eq!(k.kind, ObjectKind::Unknown);
    let mut bad = k.clone();
    bad.kind = ObjectKind::Table;
    assert!(!bad.is_well_formed());
    let t = ns_pg("db", "public").table_oid("t", 1, &[]).finish();
    assert!(key_of(&t, "t").is_well_formed());
}
