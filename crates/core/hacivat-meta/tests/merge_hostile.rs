//! The T10 review's hostile probes (H1–H12, `task-m4-10-review-hostile.rs`), each turned to the
//! behaviour the fix round rules (C-1, I-1..I-6): deletions only from a successful existence set
//! of that kind for that scope; a lower-level pass erases nothing it did not read; scoped sets;
//! a per-kind top-level guard; caseless identity on caseless engines. H6 and H11 keep the
//! behaviour the controller accepted and carried (M-1 to T15/T22, M-2 to T15).

use std::sync::Arc;

use hacivat_meta::delta::{apply_delta, top_level_keys};
use hacivat_meta::fixture::{ns_mysql, ns_pg, ns_sqlite, table_subtree};
use hacivat_meta::merge::Deletions;
use hacivat_meta::merge::{
    ChangedObject, ExistSet, Identity, MergeOutcome, MergePolicy, NamespaceResult, merge,
};
use hacivat_meta::name::Name;
use hacivat_meta::namespace::{Level, NamespaceSnapshot, Obj, Subtree, Watermark};
use hacivat_meta::props::{ColumnProps, Props, TableProps, ViewProps};
use hacivat_meta::snapshot::IntroErrorCode;
use hacivat_meta::{ObjectKey, ObjectKind, ServerId};

fn n(s: &str) -> Name {
    Arc::from(s)
}

fn ident(path: &[&str], sid: Option<u32>) -> Identity {
    Identity::of(path.iter().map(|s| n(s)).collect(), sid.map(ServerId::Oid))
}

fn res(
    existent: Vec<(ObjectKind, ExistSet)>,
    changed: Vec<ChangedObject>,
    full: bool,
    level: Level,
) -> NamespaceResult {
    NamespaceResult {
        existent: existent.into_iter().collect(),
        changed,
        categories_to_reload: Default::default(),
        new_watermark: Watermark::None,
        full,
        level,
        read_at_ms: 1,
    }
}

fn top(ns: &NamespaceSnapshot, subtree: Subtree) -> ChangedObject {
    ChangedObject {
        parent: ns.object_key(0).unwrap(),
        subtree,
    }
}

fn col(name: &str, ord: u32) -> Obj {
    Obj::new(ObjectKind::Column, name).with_props(Props::Column(Arc::new(ColumnProps {
        ordinal: ord,
        raw_type: n("int"),
        nullable: true,
        ..Default::default()
    })))
}

fn bare_table(name: &str, oid: Option<u32>, cols: &[&str]) -> Subtree {
    let mut root =
        Obj::new(ObjectKind::Table, name).with_props(Props::Table(Arc::new(TableProps::default())));
    if let Some(o) = oid {
        root = root.with_server_id(ServerId::Oid(o));
    }
    let mut t = Subtree::new(root);
    for (i, c) in cols.iter().enumerate() {
        t.add(t.root(), col(c, i as u32 + 1));
    }
    t
}

fn view(name: &str, oid: u32) -> Subtree {
    Subtree::new(
        Obj::new(ObjectKind::View, name)
            .with_server_id(ServerId::Oid(oid))
            .with_props(Props::View(Arc::new(ViewProps::default()))),
    )
}

fn count_kind(ns: &NamespaceSnapshot, kind: ObjectKind) -> usize {
    ns.objs().iter().filter(|o| o.kind() == kind).count()
}

fn top_names(ns: &NamespaceSnapshot) -> Vec<String> {
    ns.children(0).map(|(_, o)| o.name().to_string()).collect()
}

fn child_names(ns: &NamespaceSnapshot, name: &str) -> Vec<String> {
    let i = ns
        .children(0)
        .find(|(_, o)| &**o.name() == name)
        .map(|(i, _)| i)
        .unwrap_or_else(|| panic!("{name} missing"));
    ns.children(i).map(|(_, o)| o.name().to_string()).collect()
}

/// H1 (I-1). A full pass with a Table set but no View set keeps the cached view: a kind the
/// pass carries no set for is never deleted, in full passes too.
#[test]
fn h1_full_pass_without_a_view_set_keeps_the_cached_view() {
    let prev = ns_pg("db", "public")
        .subtree(bare_table("a", Some(100), &["id"]))
        .subtree(view("v", 200))
        .finish();
    let r = res(
        vec![(
            ObjectKind::Table,
            ExistSet::Ok(vec![ident(&["a"], Some(100))]),
        )],
        vec![top(&prev, bare_table("a", Some(100), &["id"]))],
        true,
        Level::Structure,
    );
    let o = merge(Some(&prev), r, &MergePolicy::default()).unwrap();
    assert_eq!(top_names(o.next()), ["a", "v"]);
    assert!(o.delta().dropped.is_empty());
}

/// H2 (I-2). A Names-level full pass over a Structure cache keeps the cached columns and the
/// namespace's level; the objects it re-read are reported below the level.
#[test]
fn h2_names_level_full_pass_keeps_cached_columns() {
    let prev = ns_pg("db", "public")
        .subtree(bare_table("a", Some(100), &["id", "v", "w"]))
        .subtree(bare_table("b", Some(101), &["id", "v"]))
        .finish();
    let structure = merge(
        Some(&prev),
        res(Vec::new(), Vec::new(), true, Level::Structure),
        &MergePolicy::default(),
    )
    .unwrap()
    .into_parts()
    .0;
    assert_eq!(structure.meta().level, Level::Structure);
    assert_eq!(count_kind(&structure, ObjectKind::Column), 5);
    let r = res(
        vec![(
            ObjectKind::Table,
            ExistSet::Ok(vec![ident(&["a"], Some(100)), ident(&["b"], Some(101))]),
        )],
        vec![
            top(&structure, bare_table("a", Some(100), &[])),
            top(&structure, bare_table("b", Some(101), &[])),
        ],
        true,
        Level::Names,
    );
    let o = merge(Some(&structure), r, &MergePolicy::default()).unwrap();
    assert_eq!(count_kind(o.next(), ObjectKind::Column), 5);
    assert_eq!(o.next().meta().level, Level::Structure);
    assert!(o.delta().dropped.is_empty());
    assert_eq!(o.report().below_level.len(), 2);
}

/// H3 (C-1). A failed index section while tables are re-read without their indexes: the cached
/// indexes stay (alg/02 step 2), in incremental and full passes.
#[test]
fn h3_failed_index_section_with_re_read_tables_keeps_cached_indexes() {
    for full in [false, true] {
        let prev = ns_pg("db", "public")
            .subtree(table_subtree("public", "a", 3, 100, None))
            .subtree(table_subtree("public", "b", 3, 200, None))
            .finish();
        let before = count_kind(&prev, ObjectKind::Index);
        assert!(before >= 2);
        let a = bare_table("a", Some(100), &["id", "created_at", "updated_at"]);
        let b = bare_table("b", Some(200), &["id", "created_at", "updated_at"]);
        let r = res(
            vec![
                (
                    ObjectKind::Table,
                    ExistSet::Ok(vec![ident(&["a"], Some(100)), ident(&["b"], Some(200))]),
                ),
                (
                    ObjectKind::Index,
                    ExistSet::Failed(IntroErrorCode::PermissionDenied),
                ),
            ],
            vec![top(&prev, a), top(&prev, b)],
            full,
            Level::Structure,
        );
        let o = merge(Some(&prev), r, &MergePolicy::default()).unwrap();
        assert_eq!(
            count_kind(o.next(), ObjectKind::Index),
            before,
            "full = {full}"
        );
        assert!(o.delta().dropped.is_empty(), "full = {full}");
        assert_eq!(
            o.report().stale_kinds.get(&ObjectKind::Index),
            Some(&IntroErrorCode::PermissionDenied)
        );
    }
}

/// H4 (I-3). A column set scoped to table `a` decides only `a`'s columns: `b`'s stay. An
/// unscoped set is namespace-complete by contract, so the same ids unscoped do drop `b`'s
/// columns, and the delta says so.
#[test]
fn h4_scoped_column_set_decides_only_its_scope() {
    let prev = ns_pg("db", "public")
        .subtree(bare_table("a", Some(100), &["id", "v"]))
        .subtree(bare_table("b", Some(101), &["id", "v"]))
        .finish();
    let ids = || vec![ident(&["a", "id"], None), ident(&["a", "v"], None)];
    let scoped = res(
        vec![(
            ObjectKind::Column,
            ExistSet::Scoped {
                scope: vec![vec![n("a")]],
                ids: ids(),
            },
        )],
        vec![],
        false,
        Level::Structure,
    );
    let o = merge(Some(&prev), scoped, &MergePolicy::default()).unwrap();
    assert_eq!(child_names(o.next(), "b"), ["id", "v"]);
    assert!(o.delta().is_empty());

    let unscoped = res(
        vec![(ObjectKind::Column, ExistSet::Ok(ids()))],
        vec![],
        false,
        Level::Structure,
    );
    let o = merge(Some(&prev), unscoped, &MergePolicy::default()).unwrap();
    assert!(child_names(o.next(), "b").is_empty());
    assert_eq!(o.delta().dropped.len(), 2);
}

/// H5 (I-4). 30 tables, 25 vanish (83 %): the guard counts top-level objects per kind, so the
/// survivors' columns do not dilute it.
#[test]
fn h5_guard_is_not_diluted_by_a_nested_kind() {
    let names: Vec<String> = (0..30).map(|i| format!("t{i:02}")).collect();
    let cols: Vec<String> = (0..20).map(|i| format!("c{i:02}")).collect();
    let colrefs: Vec<&str> = cols.iter().map(String::as_str).collect();
    let mut b = ns_pg("db", "public");
    for (i, t) in names.iter().enumerate() {
        b = b.subtree(bare_table(t, Some(1000 + i as u32), &colrefs));
    }
    let prev = b.finish();
    let survivors = &names[..5];
    let tables = survivors
        .iter()
        .enumerate()
        .map(|(i, t)| ident(&[t.as_str()], Some(1000 + i as u32)))
        .collect();
    let mut colids = Vec::new();
    for t in survivors {
        for c in &cols {
            colids.push(ident(&[t.as_str(), c.as_str()], None));
        }
    }
    let r = res(
        vec![
            (ObjectKind::Table, ExistSet::Ok(tables)),
            (ObjectKind::Column, ExistSet::Ok(colids)),
        ],
        vec![],
        false,
        Level::Structure,
    );
    let o = merge(Some(&prev), r, &MergePolicy::default()).unwrap();
    assert!(matches!(
        o,
        MergeOutcome::PendingConfirmation { vanished: 25, .. }
    ));
    let t = o.report().by_kind[&ObjectKind::Table];
    assert_eq!((t.checked, t.vanished), (30, 25));
    assert_eq!((o.report().checked, o.report().vanished), (30, 25));
}

/// H6 (M-1, accepted and carried to T15/T22). The guard is per pass (D-039 "in one pass"):
/// 40 → 20 → 10 → 0 over three passes never asks. Counting since a confirmed baseline is the
/// engine's.
#[test]
fn h6_guard_is_per_pass() {
    let mut cur = {
        let mut b = ns_pg("db", "public");
        for i in 0..40u32 {
            b = b.subtree(bare_table(&format!("t{i:02}"), Some(1000 + i), &["id"]));
        }
        b.finish()
    };
    let mut asked = false;
    for keep in [20u32, 10, 0] {
        let ids = (0..keep)
            .map(|i| ident(&[format!("t{i:02}").as_str()], Some(1000 + i)))
            .collect();
        let r = res(
            vec![(ObjectKind::Table, ExistSet::Ok(ids))],
            vec![],
            false,
            Level::Structure,
        );
        let o = merge(Some(&cur), r, &MergePolicy::default()).unwrap();
        asked |= o.is_pending();
        cur = o.into_parts().0;
    }
    assert_eq!(cur.children(0).count(), 0);
    assert!(!asked);
}

/// H7 (I-6). A case-only spelling change on a caseless engine (MySQL lctn = 2, SQLite) is one
/// object: an identity rename, no drop, no create.
#[test]
fn h7_case_only_spelling_change_on_caseless_engines_keeps_the_object() {
    for (label, prev) in [
        (
            "mysql lctn2",
            ns_mysql("shop", 2).table("Orders", &["id", "v"]).finish(),
        ),
        (
            "sqlite",
            ns_sqlite("main").table("Orders", &["id", "v"]).finish(),
        ),
    ] {
        let mut t = Subtree::new(
            Obj::new(ObjectKind::Table, "orders")
                .with_props(Props::Table(Arc::new(TableProps::default()))),
        );
        for (_, c) in prev.children(prev.children(0).next().unwrap().0) {
            t.add(t.root(), c.clone());
        }
        let r = res(
            vec![(
                ObjectKind::Table,
                ExistSet::Ok(vec![ident(&["orders"], None)]),
            )],
            vec![top(&prev, t)],
            false,
            Level::Structure,
        );
        let o = merge(Some(&prev), r, &MergePolicy::default()).unwrap();
        assert!(o.delta().dropped.is_empty(), "{label}");
        assert!(o.delta().created.is_empty(), "{label}");
        assert_eq!(o.delta().renamed.len(), 1, "{label}");
        assert!(o.delta().renamed[0].2.is_identity(), "{label}");
        assert_eq!(top_names(o.next()), ["orders"], "{label}");
    }
}

/// H8 (I-6). The existence query and the changed row spell a name differently in one pass: the
/// object stays, re-read.
#[test]
fn h8_spelling_mismatch_inside_one_pass_keeps_the_object() {
    let prev = ns_mysql("shop", 2).table("Orders", &["id"]).finish();
    let r = res(
        vec![(
            ObjectKind::Table,
            ExistSet::Ok(vec![ident(&["orders"], None)]),
        )],
        vec![top(&prev, bare_table("Orders", None, &["id", "added"]))],
        false,
        Level::Structure,
    );
    let o = merge(Some(&prev), r, &MergePolicy::default()).unwrap();
    assert_eq!(top_names(o.next()), ["Orders"]);
    assert_eq!(child_names(o.next(), "Orders"), ["id", "added"]);
    assert_eq!(o.report().ignored, 0);
    assert!(o.report().fragments.is_empty());
    assert!(o.delta().dropped.is_empty());
}

/// H9 (correct as found). Drop + recreate with the same name and a new OID, not re-read: the old
/// object goes and the new one is read by a fragment.
#[test]
fn h9_recreate_same_name_new_oid_not_re_read() {
    let prev = ns_pg("db", "public")
        .subtree(bare_table("t", Some(100), &["id"]))
        .finish();
    let r = res(
        vec![(
            ObjectKind::Table,
            ExistSet::Ok(vec![ident(&["t"], Some(200))]),
        )],
        vec![],
        false,
        Level::Structure,
    );
    let o = merge(Some(&prev), r, &MergePolicy::default()).unwrap();
    assert!(top_names(o.next()).is_empty());
    assert_eq!(o.report().fragments.len(), 1);
    assert_eq!(o.report().fragments[0].server_id, Some(ServerId::Oid(200)));
}

/// H10. An OID reused by a new table of the same kind (edge 4: accepted as a rename). With the
/// re-read's column set its children are the re-read ones only; without one, nothing decides
/// the old columns and they are carried (C-1), so strategies report the child sets they read.
#[test]
fn h10_same_kind_oid_reuse_is_a_rename_with_the_re_read_children() {
    let prev = ns_pg("db", "public")
        .subtree(bare_table("t", Some(100), &["old1", "old2"]))
        .finish();
    let tables = || {
        (
            ObjectKind::Table,
            ExistSet::Ok(vec![ident(&["u"], Some(100))]),
        )
    };
    let columns = (
        ObjectKind::Column,
        ExistSet::Scoped {
            scope: vec![vec![n("u")]],
            ids: vec![ident(&["u", "fresh"], None)],
        },
    );
    let r = res(
        vec![tables(), columns],
        vec![top(&prev, bare_table("u", Some(100), &["fresh"]))],
        false,
        Level::Structure,
    );
    let o = merge(Some(&prev), r, &MergePolicy::default()).unwrap();
    assert_eq!(child_names(o.next(), "u"), ["fresh"]);
    assert_eq!(o.delta().renamed.len(), 1);
    assert!(matches!(o, MergeOutcome::Applied { .. }));

    let r = res(
        vec![tables()],
        vec![top(&prev, bare_table("u", Some(100), &["fresh"]))],
        false,
        Level::Structure,
    );
    let o = merge(Some(&prev), r, &MergePolicy::default()).unwrap();
    assert_eq!(child_names(o.next(), "u"), ["fresh", "old1", "old2"]);
}

/// H11 (M-2, accepted and carried to T15). A name swap (a ↔ b by OID) of which the pass re-read
/// one side: two tables named `a` for one generation, and a fragment re-reads the other side.
#[test]
fn h11_half_re_read_name_swap_asks_for_the_other_side() {
    let prev = ns_pg("db", "public")
        .subtree(bare_table("a", Some(100), &["x"]))
        .subtree(bare_table("b", Some(200), &["y"]))
        .finish();
    let r = res(
        vec![(
            ObjectKind::Table,
            ExistSet::Ok(vec![ident(&["b"], Some(100)), ident(&["a"], Some(200))]),
        )],
        vec![top(&prev, bare_table("a", Some(200), &["y"]))],
        false,
        Level::Structure,
    );
    let o = merge(Some(&prev), r, &MergePolicy::default()).unwrap();
    assert_eq!(top_names(o.next()), ["a", "a"]);
    let frags: Vec<(String, Option<ServerId>)> = o
        .report()
        .fragments
        .iter()
        .map(|k| (k.name().to_string(), k.server_id))
        .collect();
    assert_eq!(frags, [("b".to_string(), Some(ServerId::Oid(100)))]);
}

/// H12 (I-3). A fragment of one table reports its existence scoped to that table: the other 14
/// tables stay.
#[test]
fn h12_fragment_existence_set_is_scoped_to_the_fragment() {
    let mut b = ns_pg("db", "public");
    for i in 0..15u32 {
        b = b.subtree(bare_table(&format!("t{i:02}"), Some(1000 + i), &["id"]));
    }
    let prev = b.finish();
    let r = res(
        vec![(
            ObjectKind::Table,
            ExistSet::Scoped {
                scope: vec![vec![n("t03")]],
                ids: vec![ident(&["t03"], Some(1003))],
            },
        )],
        vec![top(&prev, bare_table("t03", Some(1003), &["id", "new"]))],
        false,
        Level::Structure,
    );
    let o = merge(Some(&prev), r, &MergePolicy::default()).unwrap();
    assert_eq!(o.next().children(0).count(), 15);
    assert_eq!(child_names(o.next(), "t03"), ["id", "new"]);
    assert!(o.delta().dropped.is_empty());

    // The fragment's table was dropped on the server: the scoped set lists nothing.
    let gone = res(
        vec![(
            ObjectKind::Table,
            ExistSet::Scoped {
                scope: vec![vec![n("t03")]],
                ids: Vec::new(),
            },
        )],
        vec![],
        false,
        Level::Structure,
    );
    let o = merge(Some(&prev), gone, &MergePolicy::default()).unwrap();
    assert_eq!(o.next().children(0).count(), 14);
    assert_eq!(o.delta().dropped.len(), 1);
    assert_eq!(o.delta().dropped[0].name(), "t03");
}

// ---- re-review probes (R1–R7, `task-m4-10-rereview-probes.rs`), asserting the fixed behaviour

/// Keys as dotted paths.
fn keyz(ks: &[ObjectKey]) -> Vec<String> {
    ks.iter()
        .map(|k| {
            k.path
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(".")
        })
        .collect()
}

fn renames(o: &MergeOutcome) -> Vec<(String, String, bool)> {
    o.delta()
        .renamed
        .iter()
        .map(|(a, b, c)| (a.name().to_string(), b.name().to_string(), c.is_identity()))
        .collect()
}

/// R1. A scoped set with no ids drops exactly its scope; an empty scope decides nothing.
#[test]
fn r1_scoped_empty_ids_drop_exactly_the_scope() {
    let prev = ns_pg("db", "public")
        .subtree(bare_table("t", Some(100), &["id"]))
        .subtree(bare_table("u", Some(101), &["id"]))
        .finish();
    let scoped = |scope: Vec<Vec<Name>>| {
        res(
            vec![(ObjectKind::Table, ExistSet::Scoped { scope, ids: vec![] })],
            vec![],
            false,
            Level::Structure,
        )
    };
    let o = merge(
        Some(&prev),
        scoped(vec![vec![n("t")]]),
        &MergePolicy::default(),
    )
    .unwrap();
    assert_eq!(top_names(o.next()), ["u"]);
    assert_eq!(keyz(&o.delta().dropped), ["db.public.t"]);
    let o = merge(Some(&prev), scoped(vec![]), &MergePolicy::default()).unwrap();
    assert_eq!(top_names(o.next()), ["t", "u"]);
    assert!(o.delta().is_empty());
}

/// R2 (RR-1). A scope that names the old name of an object renamed by OID.
#[test]
fn r2_scope_naming_the_old_name_of_a_renamed_object() {
    let prev = ns_pg("db", "public")
        .subtree(bare_table("t", Some(100), &["id", "v"]))
        .finish();
    let scoped_t = |ids: Vec<Identity>| {
        (
            ObjectKind::Table,
            ExistSet::Scoped {
                scope: vec![vec![n("t")]],
                ids,
            },
        )
    };
    // (a) The fragment of `t` was looked up by name (nothing), and the same result re-reads
    // `u` under OID 100: an identity rename, never a drop plus a create (RR-1 a).
    let r = res(
        vec![scoped_t(vec![])],
        vec![top(&prev, bare_table("u", Some(100), &["id", "v"]))],
        false,
        Level::Structure,
    );
    let o = merge(Some(&prev), r, &MergePolicy::default()).unwrap();
    assert_eq!(top_names(o.next()), ["u"]);
    assert!(o.delta().dropped.is_empty() && o.delta().created.is_empty());
    assert_eq!(renames(&o), [("t".to_string(), "u".to_string(), true)]);
    assert_eq!((o.report().checked, o.report().vanished), (1, 0));
    // (b) The fragment resolved its key by OID (the contract, RR-1 b): the same outcome.
    let r = res(
        vec![scoped_t(vec![ident(&["u"], Some(100))])],
        vec![top(&prev, bare_table("u", Some(100), &["id", "v"]))],
        false,
        Level::Structure,
    );
    let o = merge(Some(&prev), r, &MergePolicy::default()).unwrap();
    assert_eq!(top_names(o.next()), ["u"]);
    assert!(o.delta().dropped.is_empty() && o.delta().created.is_empty());
    assert_eq!(renames(&o), [("t".to_string(), "u".to_string(), true)]);
    // (c) A scope keyed by name that lists nothing and re-reads nothing is that key's drop:
    // on server-id engines the strategy resolves scope keys by OID (documented contract,
    // carried to T15b, T16, T21), so it would have reported `u` as in (b).
    let r = res(vec![scoped_t(vec![])], vec![], false, Level::Structure);
    let o = merge(Some(&prev), r, &MergePolicy::default()).unwrap();
    assert!(top_names(o.next()).is_empty());
    assert_eq!(keyz(&o.delta().dropped), ["db.public.t"]);
    // (d) A child scope naming the renamed parent's old name decides nothing (paths are the
    // next namespace's): the column the re-read omits stays.
    let r = res(
        vec![
            (
                ObjectKind::Table,
                ExistSet::Ok(vec![ident(&["u"], Some(100))]),
            ),
            (
                ObjectKind::Column,
                ExistSet::Scoped {
                    scope: vec![vec![n("t")]],
                    ids: vec![ident(&["t", "id"], None)],
                },
            ),
        ],
        vec![top(&prev, bare_table("u", Some(100), &["id"]))],
        false,
        Level::Structure,
    );
    let o = merge(Some(&prev), r, &MergePolicy::default()).unwrap();
    assert_eq!(child_names(o.next(), "u"), ["id", "v"]);
    assert!(o.delta().dropped.is_empty());
    // (e) Scoped to the new name, it decides: `v` is gone.
    let r = res(
        vec![
            (
                ObjectKind::Table,
                ExistSet::Ok(vec![ident(&["u"], Some(100))]),
            ),
            (
                ObjectKind::Column,
                ExistSet::Scoped {
                    scope: vec![vec![n("u")]],
                    ids: vec![ident(&["u", "id"], None)],
                },
            ),
        ],
        vec![top(&prev, bare_table("u", Some(100), &["id"]))],
        false,
        Level::Structure,
    );
    let o = merge(Some(&prev), r, &MergePolicy::default()).unwrap();
    assert_eq!(child_names(o.next(), "u"), ["id"]);
    // A dropped object is named by its key in the previous generation (as every drop is).
    assert_eq!(keyz(&o.delta().dropped), ["db.public.t.v"]);
}

/// R3. Keep cached (`Suppress`) then a later `Apply`: the first keeps everything and applies the
/// creation; the second asks again (remembering the decision is T15's).
#[test]
fn r3_suppress_then_apply() {
    let mut b = ns_pg("db", "public");
    for i in 0..30u32 {
        b = b.subtree(bare_table(&format!("t{i:02}"), Some(1000 + i), &["id"]));
    }
    let prev = b.finish();
    let mk = |prev: &NamespaceSnapshot| {
        res(
            vec![(
                ObjectKind::Table,
                ExistSet::Ok(
                    (0..5u32)
                        .map(|i| ident(&[format!("t{i:02}").as_str()], Some(1000 + i)))
                        .chain([ident(&["new"], Some(5000))])
                        .collect(),
                ),
            )],
            vec![top(prev, bare_table("new", Some(5000), &["id"]))],
            false,
            Level::Structure,
        )
    };
    let apply = merge(Some(&prev), mk(&prev), &MergePolicy::default()).unwrap();
    assert!(apply.is_pending());
    let keep = merge(
        Some(&prev),
        mk(&prev),
        &MergePolicy {
            deletions: Deletions::Suppress,
            ..MergePolicy::default()
        },
    )
    .unwrap();
    assert!(!keep.is_pending());
    assert_eq!(keep.next().children(0).count(), 31);
    assert_eq!(keep.report().suppressed.len(), 25);
    assert!(keep.delta().dropped.is_empty());
    assert_eq!(keyz(&keep.delta().created), ["db.public.new"]);
    let kept = keep.into_parts().0;
    let again = merge(Some(&kept), mk(&kept), &MergePolicy::default()).unwrap();
    assert!(again.is_pending());
    assert_eq!(again.report().vanished, 25);
    assert!(again.delta().created.is_empty());
    assert_eq!(
        top_names(again.next())
            .iter()
            .filter(|x| *x == "new")
            .count(),
        1
    );
}

/// R4 (RR-m3). Fold twins in a corrupt caseless cache (`Orders`, `ORDERS` under lctn = 1): one
/// existence identity vouches for one cached object, so the twin goes and a re-read collapses
/// to one object.
#[test]
fn r4_fold_twins_in_a_caseless_cache_collapse() {
    let prev = ns_mysql("shop", 1)
        .table("Orders", &["id"])
        .table("ORDERS", &["id"])
        .finish();
    let set = || {
        vec![(
            ObjectKind::Table,
            ExistSet::Ok(vec![ident(&["orders"], None)]),
        )]
    };
    let r = res(
        set(),
        vec![top(&prev, bare_table("orders", None, &["id"]))],
        false,
        Level::Structure,
    );
    let o = merge(Some(&prev), r, &MergePolicy::default()).unwrap();
    assert_eq!(top_names(o.next()), ["orders"]);
    assert_eq!(keyz(&o.delta().dropped), ["shop.ORDERS"]);
    assert!(o.delta().created.is_empty());
    assert_eq!(
        renames(&o),
        [("Orders".to_string(), "orders".to_string(), true)]
    );
    // Not re-read: the first twin stays, the other goes.
    let o = merge(
        Some(&prev),
        res(set(), vec![], false, Level::Structure),
        &MergePolicy::default(),
    )
    .unwrap();
    assert_eq!(top_names(o.next()), ["Orders"]);
    assert_eq!(keyz(&o.delta().dropped), ["shop.ORDERS"]);
}

/// R5 (RR-m2). Every listed child has its parent marked `children`, every marked parent has a
/// listed child; a re-read that only reorders children marks the parent `children` too.
#[test]
fn r5_delta_detail_and_reordering() {
    let prev = ns_pg("db", "public")
        .subtree(table_subtree("public", "a", 3, 100, None))
        .finish();
    let a = prev.children(0).next().unwrap().0;
    let mut t = Subtree::new(prev.objs()[a as usize].clone());
    for (i, (_, o)) in prev.children(a).enumerate() {
        if o.kind() == ObjectKind::Index {
            continue;
        }
        let mut o = o.clone();
        if i == 1 {
            o = Obj::new(ObjectKind::Column, o.name().as_ref()).with_props(Props::Column(
                Arc::new(ColumnProps {
                    ordinal: 2,
                    raw_type: n("bigint"),
                    nullable: true,
                    ..Default::default()
                }),
            ));
        }
        t.add(t.root(), o);
    }
    t.add(t.root(), col("extra", 9));
    let r = res(
        vec![
            (
                ObjectKind::Table,
                ExistSet::Ok(vec![ident(&["a"], Some(100))]),
            ),
            (
                ObjectKind::Index,
                ExistSet::Scoped {
                    scope: vec![vec![n("a")]],
                    ids: vec![],
                },
            ),
        ],
        vec![top(&prev, t)],
        false,
        Level::Structure,
    );
    let o = merge(Some(&prev), r, &MergePolicy::default()).unwrap();
    let d = o.delta();
    assert_eq!(keyz(&d.created), ["db.public.a.extra"]);
    assert_eq!(keyz(&d.dropped), ["db.public.a.a_created_at_idx"]);
    let keys = apply_delta(prev.key(), &top_level_keys(&prev), d).unwrap();
    assert_eq!(keys, top_level_keys(o.next()));
    let marked: Vec<Vec<Name>> = d
        .altered
        .iter()
        .filter(|(_, f)| f.children)
        .map(|(k, _)| k.path.to_vec())
        .collect();
    let mut children: Vec<Vec<Name>> = d
        .created
        .iter()
        .chain(&d.dropped)
        .map(|k| k.path.to_vec())
        .collect();
    children.extend(d.altered.iter().map(|(k, _)| k.path.to_vec()));
    for c in children.iter().filter(|c| c.len() > 3) {
        assert!(
            marked.contains(&c[..c.len() - 1].to_vec()),
            "unmarked parent of {c:?}"
        );
    }
    for p in &marked {
        assert!(
            children
                .iter()
                .any(|c| c.len() == p.len() + 1 && c.starts_with(p)),
            "{p:?} marked without a child entry"
        );
    }

    // Reordering only: the arena follows the pass, and the parent is altered (children).
    let mut t = Subtree::new(prev.objs()[a as usize].clone());
    let mut v: Vec<Obj> = prev.children(a).map(|(_, o)| o.clone()).collect();
    v.reverse();
    for o in v {
        t.add(t.root(), o);
    }
    let r = res(
        vec![(
            ObjectKind::Table,
            ExistSet::Ok(vec![ident(&["a"], Some(100))]),
        )],
        vec![top(&prev, t)],
        false,
        Level::Structure,
    );
    let o = merge(Some(&prev), r, &MergePolicy::default()).unwrap();
    let mut expected = child_names(&prev, "a");
    expected.reverse();
    assert_eq!(child_names(o.next(), "a"), expected);
    assert_eq!(o.delta().altered.len(), 1);
    assert_eq!(keyz(&[o.delta().altered[0].0.clone()]), ["db.public.a"]);
    assert!(o.delta().altered[0].1.children);
}

/// R6 (RR-m1). A parent renamed by OID but not re-read: its children follow its identity (the
/// new name), so a namespace-wide column set under the new name keeps them.
#[test]
fn r6_kept_renamed_parent_children_follow_its_identity() {
    let prev = ns_pg("db", "public")
        .subtree(bare_table("t", Some(100), &["id", "v"]))
        .finish();
    let r = res(
        vec![
            (
                ObjectKind::Table,
                ExistSet::Ok(vec![ident(&["u"], Some(100))]),
            ),
            (
                ObjectKind::Column,
                ExistSet::Ok(vec![ident(&["u", "id"], None), ident(&["u", "v"], None)]),
            ),
        ],
        vec![],
        false,
        Level::Structure,
    );
    let o = merge(Some(&prev), r, &MergePolicy::default()).unwrap();
    assert_eq!(top_names(o.next()), ["t"]);
    assert_eq!(child_names(o.next(), "t"), ["id", "v"]);
    assert!(o.delta().dropped.is_empty());
    assert_eq!(keyz(&o.report().fragments), ["db.public.u"]);
}

/// R7. A pass at the namespace's own level is not below it: a table it creates without columns
/// is not reported below level, and the level stays.
#[test]
fn r7_same_level_creation_is_not_below_level() {
    let prev = ns_pg("db", "public")
        .subtree(bare_table("t", Some(100), &["id"]))
        .finish();
    let level = prev.meta().level;
    let r = res(
        vec![],
        vec![top(&prev, bare_table("z", Some(200), &[]))],
        false,
        level,
    );
    let o = merge(Some(&prev), r, &MergePolicy::default()).unwrap();
    assert_eq!(o.next().meta().level, level);
    assert!(o.report().below_level.is_empty());
    assert_eq!(top_names(o.next()), ["t", "z"]);
}
