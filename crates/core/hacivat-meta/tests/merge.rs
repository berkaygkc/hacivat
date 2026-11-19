//! The existence-diff merge (HV-INTRO-11; M4 Task 10): alg/02 tests 1–6 and edge cases 1–13,
//! the mass-deletion guard (D-039, R-M4-22), and the T3 carries (a 1,600-column table merges in
//! linear work; a duplicated server id is never resurrected).

use std::sync::Arc;

use hacivat_meta::delta::{
    COARSE_DELTA_OBJECTS, CatalogDelta, ChangedFacets, Confidence, apply_delta, event,
    top_level_keys,
};
use hacivat_meta::fixture::{ns_mysql, ns_pg, table_subtree};
use hacivat_meta::merge::{
    ChangedObject, ExistSet, Identity, MergeError, MergeOutcome, MergePolicy, NamespaceResult,
    Trigger, base, merge,
};
use hacivat_meta::name::{FoldRule, Name};
use hacivat_meta::namespace::{Level, NamespaceSnapshot, NsMeta, Obj, Subtree, Watermark};
use hacivat_meta::props::{ColumnProps, IndexProps, Props, SchemaProps, TableProps, ViewProps};
use hacivat_meta::snapshot::{CatalogEvent, IntroErrorCode};
use hacivat_meta::{NsKey, ObjectKey, ObjectKind, ServerId, SourceRef, StateMark};
use hacivat_proto::DbmsKind;

// ---- helpers ---------------------------------------------------------------------------------

const DB: &str = "db";
const COLS: &[&str] = &["id", "v"];

/// A stable fake OID per table name.
fn oid(name: &str) -> u32 {
    16_384
        + name
            .bytes()
            .fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)))
            % 2_000_000_000
}

fn n(s: &str) -> Name {
    Arc::from(s)
}

fn column(name: &str, ordinal: u32, raw_type: &str) -> Obj {
    Obj::new(ObjectKind::Column, name).with_props(Props::Column(Arc::new(ColumnProps {
        ordinal,
        raw_type: n(raw_type),
        nullable: true,
        ..Default::default()
    })))
}

/// A table subtree with `cols` (all `integer`, as the PostgreSQL fixture), and a server id when
/// `sid` is set.
fn table(name: &str, sid: Option<u32>, cols: &[&str]) -> Subtree {
    table_typed(name, sid, cols, "integer")
}

/// A table subtree with `cols` of `raw_type` (MySQL fixture columns are `int`).
fn table_typed(name: &str, sid: Option<u32>, cols: &[&str], raw_type: &str) -> Subtree {
    let mut root =
        Obj::new(ObjectKind::Table, name).with_props(Props::Table(Arc::new(TableProps::default())));
    if let Some(o) = sid {
        root = root.with_server_id(ServerId::Oid(o));
    }
    let mut t = Subtree::new(root);
    for (i, c) in cols.iter().enumerate() {
        t.add(t.root(), column(c, i as u32 + 1, raw_type));
    }
    t
}

fn ident(path: &[&str], sid: Option<u32>) -> Identity {
    Identity {
        path: path.iter().map(|s| n(s)).collect(),
        server_id: sid.map(ServerId::Oid),
        overload_sig: None,
        state: StateMark::None,
    }
}

fn result(
    existent: Vec<(ObjectKind, ExistSet)>,
    changed: Vec<ChangedObject>,
    full: bool,
) -> NamespaceResult {
    NamespaceResult {
        existent: existent.into_iter().collect(),
        changed,
        categories_to_reload: Default::default(),
        new_watermark: Watermark::None,
        full,
        level: Level::Structure,
        read_at_ms: 1_700_000_000_000,
    }
}

fn top(ns: &NamespaceSnapshot, subtree: Subtree) -> ChangedObject {
    ChangedObject {
        parent: ns.object_key(0).expect("root"),
        subtree,
    }
}

fn pg_key(name: &str) -> ObjectKey {
    pg_key_in("public", name)
}

fn pg_key_in(schema: &str, name: &str) -> ObjectKey {
    ObjectKey {
        kind: ObjectKind::Table,
        path: [DB, schema, name].into_iter().map(n).collect(),
        server_id: Some(ServerId::Oid(oid(name))),
        overload_sig: None,
        raw_kind: None,
    }
}

fn my_key(name: &str) -> ObjectKey {
    ObjectKey {
        kind: ObjectKind::Table,
        path: ["shop", name].into_iter().map(n).collect(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    }
}

fn keys(names: &[&str]) -> Vec<ObjectKey> {
    names.iter().map(|t| pg_key(t)).collect()
}

/// Cached tables (each with [`COLS`]).
struct M(Vec<(String, Vec<String>)>);
/// Names the existence query lists for `Table`.
struct E(Vec<String>);
/// Changed tables (each with its columns).
struct C(Vec<(String, Vec<String>)>);

fn m(names: &[&str]) -> M {
    M(names
        .iter()
        .map(|t| (t.to_string(), COLS.iter().map(|c| c.to_string()).collect()))
        .collect())
}

fn m_n(count: usize) -> M {
    let names: Vec<String> = (0..count).map(|i| format!("t{i:03}")).collect();
    M(names
        .into_iter()
        .map(|t| (t, COLS.iter().map(|c| c.to_string()).collect()))
        .collect())
}

fn m_cols(name: &str, cols: &[&str]) -> M {
    M(vec![(
        name.to_string(),
        cols.iter().map(|c| c.to_string()).collect(),
    )])
}

fn e(names: &[&str]) -> E {
    E(names.iter().map(|t| t.to_string()).collect())
}

fn e_n(count: usize) -> E {
    E((0..count).map(|i| format!("t{i:03}")).collect())
}

fn c(names: &[&str]) -> C {
    C(names
        .iter()
        .map(|t| (t.to_string(), COLS.iter().map(|c| c.to_string()).collect()))
        .collect())
}

fn c_cols(name: &str, cols: &[&str]) -> C {
    C(vec![(
        name.to_string(),
        cols.iter().map(|c| c.to_string()).collect(),
    )])
}

fn strs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

fn pg_prev(m: &M) -> NamespaceSnapshot {
    m.0.iter()
        .fold(ns_pg(DB, "public"), |b, (t, cols)| {
            b.table_oid(t, oid(t), &strs(cols))
        })
        .finish()
}

fn my_prev(m: &M) -> NamespaceSnapshot {
    m.0.iter()
        .fold(ns_mysql("shop", 0), |b, (t, cols)| b.table(t, &strs(cols)))
        .finish()
}

fn pg_result(prev: &NamespaceSnapshot, e: E, c: C) -> NamespaceResult {
    let ids = e.0.iter().map(|t| ident(&[t], Some(oid(t)))).collect();
    let changed =
        c.0.iter()
            .map(|(t, cols)| top(prev, table(t, Some(oid(t)), &strs(cols))))
            .collect();
    result(vec![(ObjectKind::Table, ExistSet::Ok(ids))], changed, false)
}

/// A merge's parts, and the namespace it started from.
struct Out {
    prev: NamespaceSnapshot,
    next: NamespaceSnapshot,
    delta: CatalogDelta,
    report: hacivat_meta::merge::MergeReport,
    pending: Option<u32>,
}

fn out(prev: NamespaceSnapshot, o: MergeOutcome) -> Out {
    let pending = match &o {
        MergeOutcome::PendingConfirmation { vanished, .. } => Some(*vanished),
        MergeOutcome::Applied { .. } => None,
    };
    let (next, delta, report) = o.into_parts();
    Out {
        prev,
        next,
        delta,
        report,
        pending,
    }
}

fn run(prev: NamespaceSnapshot, r: NamespaceResult, policy: &MergePolicy) -> Out {
    let o = merge(Some(&prev), r, policy).expect("merge");
    out(prev, o)
}

fn merge_pg(m: M, e: E, c: C) -> Out {
    let prev = pg_prev(&m);
    let r = pg_result(&prev, e, c);
    run(prev, r, &MergePolicy::default())
}

fn merge_my(m: M, e: E, c: C) -> Out {
    let prev = my_prev(&m);
    let ids = e.0.iter().map(|t| ident(&[t], None)).collect();
    let changed =
        c.0.iter()
            .map(|(t, cols)| top(&prev, table_typed(t, None, &strs(cols), "int")))
            .collect();
    let r = result(vec![(ObjectKind::Table, ExistSet::Ok(ids))], changed, false);
    run(prev, r, &MergePolicy::default())
}

fn merge_with(m: M, e: E, c: C, policy: MergePolicy) -> MergeOutcome {
    let prev = pg_prev(&m);
    let r = pg_result(&prev, e, c);
    merge(Some(&prev), r, &policy).expect("merge")
}

fn merge_bg(m: M, e: E, c: C) -> MergeOutcome {
    merge_with(m, e, c, MergePolicy::default())
}

fn idx_of(ns: &NamespaceSnapshot, kind: ObjectKind, name: &str) -> u32 {
    ns.children(0)
        .find(|(_, o)| o.kind() == kind && &**o.name() == name)
        .map(|(i, _)| i)
        .unwrap_or_else(|| panic!("{name} not in the namespace"))
}

fn top_names(ns: &NamespaceSnapshot) -> Vec<String> {
    ns.children(0).map(|(_, o)| o.name().to_string()).collect()
}

fn child_names(ns: &NamespaceSnapshot, idx: u32) -> Vec<String> {
    ns.children(idx)
        .map(|(_, o)| o.name().to_string())
        .collect()
}

/// Whether two props share their allocation.
fn props_ptr_eq(a: &Props, b: &Props) -> bool {
    match (a, b) {
        (Props::None, Props::None) => true,
        (Props::Column(x), Props::Column(y)) => Arc::ptr_eq(x, y),
        (Props::Table(x), Props::Table(y)) => Arc::ptr_eq(x, y),
        (Props::View(x), Props::View(y)) => Arc::ptr_eq(x, y),
        (Props::Index(x), Props::Index(y)) => Arc::ptr_eq(x, y),
        (Props::Key(x), Props::Key(y)) => Arc::ptr_eq(x, y),
        (Props::ForeignKey(x), Props::ForeignKey(y)) => Arc::ptr_eq(x, y),
        _ => false,
    }
}

/// Whether every child of `name` in `next` shares its props with the same child in `prev`.
fn children_ptr_eq(o: &Out, name: &str) -> bool {
    let a = idx_of(&o.prev, ObjectKind::Table, name);
    let b = idx_of(&o.next, ObjectKind::Table, name);
    let pa: Vec<_> = o.prev.children(a).collect();
    let pb: Vec<_> = o.next.children(b).collect();
    !pa.is_empty()
        && pa.len() == pb.len()
        && pa
            .iter()
            .zip(&pb)
            .all(|((_, x), (_, y))| props_ptr_eq(x.props(), y.props()))
}

// ---- alg/02 HV-INTRO-11 test cases ------------------------------------------------------------

/// Test 1: `M={a,b}`, `E={a,c}`, `C={c}` → created `c`, dropped `b`, kept `a` (same key,
/// `Arc`-shared children).
#[test]
fn created_dropped_kept_and_kept_children_shared() {
    let o = merge_pg(m(&["a", "b"]), e(&["a", "c"]), c(&["c"]));
    assert_eq!(o.delta.created, keys(&["c"]));
    assert_eq!(o.delta.dropped, keys(&["b"]));
    assert!(children_ptr_eq(&o, "a"));
    assert!(o.delta.altered.is_empty() && o.delta.renamed.is_empty());
    assert_eq!(top_names(&o.next), ["a", "c"]);
    assert_eq!(
        o.next.object_key(idx_of(&o.next, ObjectKind::Table, "a")),
        Some(pg_key("a"))
    );
    assert_eq!((o.report.checked, o.report.vanished), (2, 1));
    assert_eq!(o.pending, None);
}

/// Test 2: `M={16400:t}`, `E={16400}`, `C={16400:u}` → renamed `t → u`, children intact.
#[test]
fn rename_via_oid_keeps_children() {
    let prev = ns_pg(DB, "public")
        .table_oid("t", 16400, &["a", "b"])
        .finish();
    let r = result(
        vec![(
            ObjectKind::Table,
            ExistSet::Ok(vec![ident(&["u"], Some(16400))]),
        )],
        vec![top(&prev, table("u", Some(16400), &["a", "b"]))],
        false,
    );
    let o = run(prev, r, &MergePolicy::default());
    let before = o
        .prev
        .object_key(idx_of(&o.prev, ObjectKind::Table, "t"))
        .unwrap();
    let after = o
        .next
        .object_key(idx_of(&o.next, ObjectKind::Table, "u"))
        .unwrap();
    assert_eq!(
        o.delta.renamed,
        vec![(before.clone(), after.clone(), Confidence::ONE)]
    );
    assert!(before.same_object(&after));
    assert!(!CatalogDelta::is_hint(&before, &after));
    assert!(o.delta.created.is_empty() && o.delta.dropped.is_empty());
    assert!(o.delta.altered.is_empty(), "only the name changed");
    assert_eq!(
        child_names(&o.next, idx_of(&o.next, ObjectKind::Table, "u")),
        ["a", "b"]
    );
    // The old key still finds the object by its OID.
    assert_eq!(
        o.next.find(&before),
        Some(idx_of(&o.next, ObjectKind::Table, "u"))
    );
}

/// Test 3 and edge 1: a failed existence query keeps every cached object of its kind and marks
/// the kind stale, even in a full pass that re-read nothing of it.
#[test]
fn failed_existence_disables_deletions_and_marks_stale() {
    for full in [false, true] {
        let prev = pg_prev(&m(&["a", "b"]));
        let r = result(
            vec![(
                ObjectKind::Table,
                ExistSet::Failed(IntroErrorCode::PermissionDenied),
            )],
            Vec::new(),
            full,
        );
        let o = run(prev, r, &MergePolicy::default());
        assert_eq!(top_names(&o.next), ["a", "b"], "full = {full}");
        assert!(o.delta.dropped.is_empty());
        assert_eq!(
            o.report.stale_kinds.get(&ObjectKind::Table),
            Some(&IntroErrorCode::PermissionDenied)
        );
        assert_eq!((o.report.checked, o.report.vanished), (0, 0));
        assert!(children_ptr_eq(&o, "a") && children_ptr_eq(&o, "b"));
    }
}

/// Test 4: a MySQL `RENAME TABLE a TO b` is dropped `a` + created `b`, with a rename hint of
/// confidence one (identical column signatures). The model's identity is new.
#[test]
fn mysql_rename_hint_confidence_one() {
    let o = merge_my(
        m_cols("a", &["x", "y", "z"]),
        e(&["b"]),
        c_cols("b", &["x", "y", "z"]),
    );
    assert_eq!(o.delta.renamed[0].2, Confidence::ONE);
    assert_eq!(o.delta.renamed.len(), 1);
    assert_eq!(o.delta.renamed[0].0, my_key("a"));
    assert_eq!(o.delta.renamed[0].1, my_key("b"));
    assert!(CatalogDelta::is_hint(
        &o.delta.renamed[0].0,
        &o.delta.renamed[0].1
    ));
    assert_eq!(o.delta.dropped, vec![my_key("a")]);
    assert_eq!(o.delta.created, vec![my_key("b")]);
}

/// Step 6: ≥ 90 % identical column signatures is a hint below one; below 90 % is none.
#[test]
fn rename_hint_needs_ninety_percent_of_column_signatures() {
    let ten = ["c0", "c1", "c2", "c3", "c4", "c5", "c6", "c7", "c8", "c9"];
    let mut nine = ten;
    nine[9] = "other";
    let o = merge_my(m_cols("a", &ten), e(&["b"]), c_cols("b", &nine));
    assert_eq!(o.delta.renamed.len(), 1);
    assert_eq!(o.delta.renamed[0].2, Confidence::from_per_mille(900));
    assert!(o.delta.renamed[0].2 < Confidence::ONE);

    let mut eight = nine;
    eight[8] = "else";
    let o = merge_my(m_cols("a", &ten), e(&["b"]), c_cols("b", &eight));
    assert!(o.delta.renamed.is_empty());
    assert_eq!(o.delta.dropped, vec![my_key("a")]);

    // The same columns in another order are not identical: below one.
    let mut shuffled = ten;
    shuffled.swap(0, 1);
    let o = merge_my(m_cols("a", &ten), e(&["b"]), c_cols("b", &shuffled));
    assert_eq!(o.delta.renamed[0].2, Confidence::from_per_mille(999));
}

/// Test 5: 100 cached tables, the existence query lists 10 (background) → pending; the
/// snapshot is unchanged until confirmed.
#[test]
fn mass_drop_background_is_pending_confirmation() {
    assert!(matches!(
        merge_bg(m_n(100), e_n(10), c(&[])),
        MergeOutcome::PendingConfirmation { vanished: 90, .. }
    ));
}

/// R-M4-22: a user-triggered refresh gets the same outcome; the engine asks the UI.
#[test]
fn mass_drop_user_triggered_is_pending_confirmation_too() {
    let policy = MergePolicy {
        trigger: Trigger::User,
        ..MergePolicy::default()
    };
    let o = merge_with(m_n(100), e_n(10), c(&[]), policy);
    assert!(matches!(
        o,
        MergeOutcome::PendingConfirmation { vanished: 90, .. }
    ));
    // The pending result is the full merge: applied on confirmation, as is.
    assert_eq!(o.next().children(0).count(), 10);
    assert_eq!(o.delta().dropped.len(), 90);
}

#[test]
fn under_20_objects_is_never_guarded() {
    for count in 1..20 {
        let o = merge_bg(m_n(count), e_n(0), c(&[]));
        assert!(!o.is_pending(), "{count} objects");
        assert_eq!(o.next().children(0).count(), 0);
    }
}

/// 20 objects, 10 vanish → applied (the rule is > 50 %).
#[test]
fn exactly_half_is_not_guarded() {
    let o = merge_bg(m_n(20), e_n(10), c(&[]));
    assert!(matches!(o, MergeOutcome::Applied { .. }));
    assert_eq!(o.report().vanished, 10);
}

/// 19 objects, 19 vanish → applied (the rule is ≥ 20 objects, D-039).
#[test]
fn nineteen_objects_all_vanishing_is_not_guarded() {
    let o = merge_bg(m_n(19), e_n(0), c(&[]));
    assert!(matches!(o, MergeOutcome::Applied { .. }));
    assert_eq!(o.report().vanished, 19);
}

#[test]
fn twenty_objects_eleven_vanishing_is_guarded() {
    assert!(matches!(
        merge_bg(m_n(20), e_n(9), c(&[])),
        MergeOutcome::PendingConfirmation { vanished: 11, .. }
    ));
}

/// The guard's ratio and floor come from the policy.
#[test]
fn guard_thresholds_come_from_the_policy() {
    let strict = MergePolicy {
        guard_ratio_pct: 10,
        guard_min_objects: 5,
        ..MergePolicy::default()
    };
    assert!(merge_with(m_n(10), e_n(8), c(&[]), strict).is_pending());
    assert!(!merge_with(m_n(10), e_n(9), c(&[]), strict).is_pending());
}

/// A pending merge leaves `prev` as it was (nothing applied), and a pending merge's `next`
/// is what an applied merge with the guard off computes.
#[test]
fn pending_confirmation_leaves_prev_untouched() {
    let prev = pg_prev(&m_n(40));
    let before: Vec<Obj> = prev.objs().to_vec();
    let r = || pg_result(&prev, e_n(5), c(&[]));
    let pending = merge(Some(&prev), r(), &MergePolicy::default()).unwrap();
    assert!(pending.is_pending());
    assert_eq!(prev.objs(), &before[..]);
    let off = MergePolicy {
        guard_min_objects: u32::MAX,
        ..MergePolicy::default()
    };
    let applied = merge(Some(&prev), r(), &off).unwrap();
    assert!(!applied.is_pending());
    assert_eq!(pending.next().objs(), applied.next().objs());
    assert_eq!(pending.delta(), applied.delta());
}

/// Existence is keyed by `ObjectKind`: a failed index query never disables table deletions.
#[test]
fn failed_index_existence_keeps_tables_deletable() {
    let prev = ns_pg(DB, "public")
        .subtree(table_subtree("public", "a", 3, 100, None))
        .subtree(table_subtree("public", "b", 3, 200, None))
        .finish();
    let r = result(
        vec![
            (
                ObjectKind::Table,
                ExistSet::Ok(vec![ident(&["a"], Some(100))]),
            ),
            (ObjectKind::Index, ExistSet::Failed(IntroErrorCode::Timeout)),
        ],
        Vec::new(),
        false,
    );
    let o = run(prev, r, &MergePolicy::default());
    assert_eq!(top_names(&o.next), ["a"]);
    let b = o
        .prev
        .object_key(idx_of(&o.prev, ObjectKind::Table, "b"))
        .unwrap();
    assert_eq!(o.delta.dropped, vec![b]);
    let a = idx_of(&o.next, ObjectKind::Table, "a");
    assert!(child_names(&o.next, a).contains(&"a_created_at_idx".to_string()));
    assert_eq!(
        o.report.stale_kinds.get(&ObjectKind::Index),
        Some(&IntroErrorCode::Timeout)
    );
    assert!(!o.report.stale_kinds.contains_key(&ObjectKind::Table));
}

/// An index the index existence query no longer lists is dropped from its kept table; the
/// table is altered (children) and its other children stay shared.
#[test]
fn index_missing_from_a_successful_index_set_is_dropped_from_its_kept_table() {
    let prev = ns_pg(DB, "public")
        .subtree(table_subtree("public", "a", 3, 100, None))
        .finish();
    let r = result(
        vec![
            (
                ObjectKind::Table,
                ExistSet::Ok(vec![ident(&["a"], Some(100))]),
            ),
            (ObjectKind::Index, ExistSet::Ok(Vec::new())),
        ],
        Vec::new(),
        false,
    );
    let o = run(prev, r, &MergePolicy::default());
    let a = idx_of(&o.next, ObjectKind::Table, "a");
    assert_eq!(
        child_names(&o.next, a),
        ["id", "created_at", "updated_at", "a_pkey"]
    );
    let pa = idx_of(&o.prev, ObjectKind::Table, "a");
    let index = o
        .prev
        .children_of_kind(pa, ObjectKind::Index)
        .map(|(i, _)| o.prev.object_key(i).unwrap())
        .next()
        .unwrap();
    assert_eq!(o.delta.dropped, vec![index]);
    let facets = ChangedFacets {
        children: true,
        ..Default::default()
    };
    assert_eq!(
        o.delta.altered,
        vec![(pg_key_in("public", "a").with_oid(100), facets)]
    );
    assert_eq!((o.report.checked, o.report.vanished), (2, 1));
}

trait WithOid {
    fn with_oid(self, oid: u32) -> Self;
}

impl WithOid for ObjectKey {
    fn with_oid(mut self, oid: u32) -> Self {
        self.server_id = Some(ServerId::Oid(oid));
        self
    }
}

/// Test 6 and step 5: PostgreSQL `SET SCHEMA` with both namespaces merged for one publish is
/// one `moved` entry.
#[test]
fn set_schema_with_both_namespaces_is_one_move() {
    let public = ns_pg(DB, "public")
        .table_oid("t", 500, COLS)
        .table_oid("x", 501, COLS)
        .finish();
    let sales = ns_pg(DB, "sales").table_oid("y", 502, COLS).finish();
    let rp = result(
        vec![(
            ObjectKind::Table,
            ExistSet::Ok(vec![ident(&["x"], Some(501))]),
        )],
        Vec::new(),
        false,
    );
    let rs = result(
        vec![(
            ObjectKind::Table,
            ExistSet::Ok(vec![ident(&["y"], Some(502)), ident(&["t"], Some(500))]),
        )],
        vec![top(&sales, table("t", Some(500), COLS))],
        false,
    );
    let dp = run(public.clone(), rp, &MergePolicy::default());
    let ds = run(sales.clone(), rs, &MergePolicy::default());
    let combined = CatalogDelta::combine(vec![
        (public.key().clone(), dp.delta.clone()),
        (sales.key().clone(), ds.delta.clone()),
    ]);
    let in_sales = pg_key_in("sales", "t").with_oid(500);
    assert_eq!(
        combined.moved,
        vec![(public.key().clone(), sales.key().clone(), in_sales.clone())]
    );
    assert!(combined.created.is_empty() && combined.dropped.is_empty());
    // Applied to each namespace's top-level keys, the move removes and adds the object.
    let p = apply_delta(public.key(), &top_level_keys(&public), &combined).unwrap();
    assert_eq!(p, top_level_keys(&dp.next));
    let s = apply_delta(sales.key(), &top_level_keys(&sales), &combined).unwrap();
    assert_eq!(s, top_level_keys(&ds.next));
    assert!(s.contains(&in_sales));
}

/// Edge 6: a MySQL table replaced by a view of the same name is a drop and a create (and never
/// a rename hint across kinds).
#[test]
fn kind_change_same_name_is_drop_and_create() {
    let prev = my_prev(&m_cols("v", &["x"]));
    let mut view = Subtree::new(
        Obj::new(ObjectKind::View, "v").with_props(Props::View(Arc::new(ViewProps::default()))),
    );
    view.add(view.root(), column("x", 1, "integer"));
    let r = result(
        vec![
            (ObjectKind::Table, ExistSet::Ok(Vec::new())),
            (ObjectKind::View, ExistSet::Ok(vec![ident(&["v"], None)])),
        ],
        vec![top(&prev, view)],
        false,
    );
    let o = run(prev, r, &MergePolicy::default());
    assert_eq!(o.delta.dropped, vec![my_key("v")]);
    let mut v = my_key("v");
    v.kind = ObjectKind::View;
    assert_eq!(o.delta.created, vec![v]);
    assert!(o.delta.renamed.is_empty() && o.delta.altered.is_empty());
    assert_eq!(o.next.objs()[1].kind(), ObjectKind::View);
}

/// Edge 3: an OID reused by an object of another kind is a drop and a create.
#[test]
fn oid_reuse_with_another_kind_is_drop_and_create() {
    let prev = ns_pg(DB, "public").table_oid("t", 900, COLS).finish();
    let view = Subtree::new(
        Obj::new(ObjectKind::View, "w")
            .with_server_id(ServerId::Oid(900))
            .with_props(Props::View(Arc::new(ViewProps::default()))),
    );
    let r = result(
        vec![
            (ObjectKind::Table, ExistSet::Ok(Vec::new())),
            (
                ObjectKind::View,
                ExistSet::Ok(vec![ident(&["w"], Some(900))]),
            ),
        ],
        vec![top(&prev, view)],
        false,
    );
    let o = run(prev, r, &MergePolicy::default());
    assert_eq!(o.delta.dropped, vec![pg_key("t").with_oid(900)]);
    let mut w = pg_key("w").with_oid(900);
    w.kind = ObjectKind::View;
    assert_eq!(o.delta.created, vec![w]);
    assert!(o.delta.renamed.is_empty());
    assert_eq!(top_names(&o.next), ["w"]);
}

/// Edge 8 and step 7: a changed object's children come entirely from the pass (an index-only
/// change returns the table with all its children; the old index is gone).
#[test]
fn changed_child_without_parent_row_replaces_all_children() {
    let index = |name: &str, oid: u32| {
        Obj::new(ObjectKind::Index, name)
            .with_server_id(ServerId::Oid(oid))
            .with_props(Props::Index(Arc::new(IndexProps::default())))
    };
    let mut old = table("t", Some(700), &["a", "b"]);
    old.add(old.root(), index("i1", 701));
    let prev = ns_pg(DB, "public").subtree(old).finish();
    let mut new = table("t", Some(700), &["a", "b"]);
    new.add(new.root(), index("i2", 702));
    let r = result(
        vec![(
            ObjectKind::Table,
            ExistSet::Ok(vec![ident(&["t"], Some(700))]),
        )],
        vec![top(&prev, new)],
        false,
    );
    let o = run(prev, r, &MergePolicy::default());
    let t = idx_of(&o.next, ObjectKind::Table, "t");
    assert_eq!(child_names(&o.next, t), ["a", "b", "i2"]);
    let facets = ChangedFacets {
        children: true,
        ..Default::default()
    };
    assert_eq!(o.delta.altered, vec![(pg_key("t").with_oid(700), facets)]);
    assert!(o.delta.created.is_empty() && o.delta.dropped.is_empty());
}

/// Edge 10: an object in the changed set that the existence query does not list is ignored.
#[test]
fn object_in_c_not_in_e_is_ignored() {
    let o = merge_pg(m(&["a"]), e(&["a"]), c(&["z"]));
    assert_eq!(top_names(&o.next), ["a"]);
    assert!(o.delta.is_empty());
    assert_eq!(o.report.ignored, 1);
}

/// Edge 11: an object the existence query lists that neither the cache nor the pass holds is
/// re-read by a fragment; nothing is invented.
#[test]
fn object_in_e_not_in_m_nor_c_schedules_a_fragment_and_invents_nothing() {
    let o = merge_pg(m(&["a"]), e(&["a", "n"]), c(&[]));
    assert_eq!(top_names(&o.next), ["a"]);
    assert!(o.delta.is_empty());
    assert_eq!(o.report.fragments, vec![pg_key("n")]);
}

/// A cached object the existence query lists under a new name (by its OID) but the pass did not
/// re-read stays as cached, and is re-read by a fragment.
#[test]
fn renamed_in_e_but_not_in_c_is_kept_and_re_read() {
    let prev = ns_pg(DB, "public").table_oid("t", 16400, COLS).finish();
    let r = result(
        vec![(
            ObjectKind::Table,
            ExistSet::Ok(vec![ident(&["u"], Some(16400))]),
        )],
        Vec::new(),
        false,
    );
    let o = run(prev, r, &MergePolicy::default());
    assert_eq!(top_names(&o.next), ["t"]);
    assert!(o.delta.is_empty());
    let mut u = pg_key("u");
    u.server_id = Some(ServerId::Oid(16400));
    assert_eq!(o.report.fragments, vec![u]);
}

/// Edge 12: a duplicated identity in the changed set: the last one wins, once.
#[test]
fn duplicate_identity_in_c_last_wins() {
    let prev = pg_prev(&m(&["a"]));
    let r = result(
        vec![(
            ObjectKind::Table,
            ExistSet::Ok(vec![
                ident(&["a"], Some(oid("a"))),
                ident(&["t"], Some(oid("t"))),
            ]),
        )],
        vec![
            top(&prev, table("t", Some(oid("t")), &["first"])),
            top(&prev, table("t", Some(oid("t")), &["second"])),
            top(&prev, table("a", Some(oid("a")), &["x"])),
            top(&prev, table("a", Some(oid("a")), &["y"])),
        ],
        false,
    );
    let o = run(prev, r, &MergePolicy::default());
    assert_eq!(top_names(&o.next), ["a", "t"]);
    assert_eq!(
        child_names(&o.next, idx_of(&o.next, ObjectKind::Table, "t")),
        ["second"]
    );
    assert_eq!(
        child_names(&o.next, idx_of(&o.next, ObjectKind::Table, "a")),
        ["y"]
    );
    assert_eq!(o.delta.created, keys(&["t"]));
    assert_eq!(o.report.duplicates, 2);
}

/// Edge 13: a delta over 10,000 objects reaches subscribers as one coarse event.
#[test]
fn huge_delta_is_coarse() {
    let key = NsKey::schema([DB, "public"]);
    let empty = base(key.clone(), DbmsKind::Postgres, FoldRule::Exact).unwrap();
    for (count, coarse) in [
        (COARSE_DELTA_OBJECTS, false),
        (COARSE_DELTA_OBJECTS + 1, true),
    ] {
        let changed = (0..count)
            .map(|i| {
                top(
                    &empty,
                    table(&format!("t{i}"), Some(20_000 + i as u32), &[]),
                )
            })
            .collect();
        let o = run(
            empty.clone(),
            result(Vec::new(), changed, true),
            &MergePolicy::default(),
        );
        assert_eq!(o.delta.created.len(), count);
        assert_eq!(o.delta.is_coarse(), coarse);
        let ev = event(7, &key, Arc::new(o.delta));
        if coarse {
            assert!(
                matches!(ev, CatalogEvent::NamespaceReplaced { generation: 7, ns } if ns == key)
            );
        } else {
            assert!(matches!(ev, CatalogEvent::Published { generation: 7, .. }));
        }
    }
}

/// Step 8: facets of an altered object are invalidated unless the pass re-read them; a kept
/// object keeps its facets (shared).
#[test]
fn facets_of_altered_invalidated_of_kept_preserved() {
    let view = |name: &str, oid: u32, src: Option<&str>| {
        Subtree::new(
            Obj::new(ObjectKind::View, name)
                .with_server_id(ServerId::Oid(oid))
                .with_props(Props::View(Arc::new(ViewProps {
                    source: src.map(SourceRef::of),
                    ..Default::default()
                }))),
        )
    };
    let prev = ns_pg(DB, "public")
        .subtree(view("v1", 11, Some("SELECT 1")))
        .subtree(view("v2", 12, Some("SELECT 2")))
        .finish();
    let r = result(
        vec![(
            ObjectKind::View,
            ExistSet::Ok(vec![ident(&["v1"], Some(11)), ident(&["v2"], Some(12))]),
        )],
        vec![top(&prev, view("v1", 11, None))],
        false,
    );
    let o = run(prev, r, &MergePolicy::default());
    let source = |ns: &NamespaceSnapshot, name: &str| match ns.objs()
        [idx_of(ns, ObjectKind::View, name) as usize]
        .props()
    {
        Props::View(v) => v.source,
        _ => unreachable!(),
    };
    assert_eq!(source(&o.next, "v1"), None);
    assert_eq!(source(&o.next, "v2"), Some(SourceRef::of("SELECT 2")));
    let pv2 = &o.prev.objs()[idx_of(&o.prev, ObjectKind::View, "v2") as usize];
    let nv2 = &o.next.objs()[idx_of(&o.next, ObjectKind::View, "v2") as usize];
    assert!(props_ptr_eq(pv2.props(), nv2.props()));
    let mut v1 = pg_key("v1").with_oid(11);
    v1.kind = ObjectKind::View;
    let facets = ChangedFacets {
        source: true,
        ..Default::default()
    };
    assert_eq!(o.delta.altered, vec![(v1, facets)]);
}

/// An object re-read identical but for its state mark is not altered; it takes the new mark.
#[test]
fn identical_re_read_is_not_altered_and_takes_the_new_state_mark() {
    let mk = |xmin: u32| {
        let mut t = Subtree::new(
            Obj::new(ObjectKind::Table, "a")
                .with_server_id(ServerId::Oid(100))
                .with_state(StateMark::Xmin(xmin))
                .with_props(Props::Table(Arc::new(TableProps::default()))),
        );
        t.add(t.root(), column("x", 1, "integer"));
        t
    };
    let prev = ns_pg(DB, "public").subtree(mk(1)).finish();
    let r = result(
        vec![(
            ObjectKind::Table,
            ExistSet::Ok(vec![ident(&["a"], Some(100))]),
        )],
        vec![top(&prev, mk(42))],
        true,
    );
    let o = run(prev, r, &MergePolicy::default());
    assert!(o.delta.is_empty());
    let a = idx_of(&o.next, ObjectKind::Table, "a");
    assert_eq!(o.next.objs()[a as usize].state(), StateMark::Xmin(42));
    assert_eq!(child_names(&o.next, a), ["x"]);
}

// ---- carries (T3 review) ------------------------------------------------------------------------

/// Merging a 1,600-column table never calls `find` per column: the work grows linearly with the
/// columns (16,000 columns cost at most ~10× 1,600), whether the table is kept or re-read.
#[test]
fn wide_table_merge_is_linear() {
    fn steps(cols: usize, changed: bool) -> u64 {
        let names: Vec<String> = (0..cols).map(|i| format!("c{i:05}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let prev = ns_pg(DB, "public").table_oid("wide", 9, &refs).finish();
        let mut renamed = refs.clone();
        let last = format!("renamed_{cols}");
        *renamed.last_mut().unwrap() = &last;
        let c = if changed {
            vec![top(&prev, table("wide", Some(9), &renamed))]
        } else {
            Vec::new()
        };
        let r = result(
            vec![(
                ObjectKind::Table,
                ExistSet::Ok(vec![ident(&["wide"], Some(9))]),
            )],
            c,
            false,
        );
        let o = run(prev, r, &MergePolicy::default());
        let t = idx_of(&o.next, ObjectKind::Table, "wide");
        assert_eq!(o.next.children(t).count(), cols);
        o.report.steps
    }
    for changed in [false, true] {
        let small = steps(1_600, changed);
        let big = steps(16_000, changed);
        assert!(small <= 8 * 1_600, "{small} steps for 1,600 columns");
        assert!(
            big <= 11 * small,
            "{big} steps for 16,000 columns vs {small} for 1,600 (changed = {changed})"
        );
    }
}

/// A server id duplicated in the cache (counted, found by neither object, T3 m9) is matched only
/// by id and path together: the stale twin is dropped, never resurrected.
#[test]
fn duplicate_server_id_in_prev_is_not_resurrected() {
    let prev = ns_pg(DB, "public")
        .table_oid("a", 7, COLS)
        .table_oid("b", 7, COLS)
        .finish();
    assert_eq!(prev.duplicate_server_ids(), 2);
    let set = |ids| vec![(ObjectKind::Table, ExistSet::Ok(ids))];
    let o = run(
        prev.clone(),
        result(set(vec![ident(&["a"], Some(7))]), Vec::new(), false),
        &MergePolicy::default(),
    );
    assert_eq!(top_names(&o.next), ["a"]);
    assert_eq!(o.next.duplicate_server_ids(), 0);
    assert_eq!(o.delta.dropped, vec![pg_key("b").with_oid(7)]);

    // Renamed to `c` under the same OID: neither twin is the object any more.
    let o = run(
        prev.clone(),
        result(
            set(vec![ident(&["c"], Some(7))]),
            vec![top(&prev, table("c", Some(7), COLS))],
            false,
        ),
        &MergePolicy::default(),
    );
    assert_eq!(top_names(&o.next), ["c"]);
    assert_eq!(o.next.duplicate_server_ids(), 0);
    assert_eq!(
        o.delta.dropped,
        vec![pg_key("a").with_oid(7), pg_key("b").with_oid(7)]
    );
    assert_eq!(o.delta.created, vec![pg_key("c").with_oid(7)]);
    assert!(o.delta.renamed.is_empty());
}

// ---- seams: first load, level and meta, nested changes, the root, wrong namespaces -----------

/// A first load merges into [`base`]; without a previous namespace the merge refuses.
#[test]
fn no_previous_namespace_is_an_error_and_base_is_a_first_load() {
    let prev = pg_prev(&m(&[]));
    let r = || {
        let mut r = pg_result(&prev, e(&["a", "b"]), c(&["a", "b"]));
        r.full = true;
        r.new_watermark = Watermark::Sqlite {
            schema_version: 3,
            file_identity: None,
            digest: 9,
        };
        r
    };
    assert_eq!(
        merge(None, r(), &MergePolicy::default()).unwrap_err(),
        MergeError::NoBase
    );
    let empty = base(
        NsKey::schema([DB, "public"]),
        DbmsKind::Postgres,
        FoldRule::Exact,
    )
    .unwrap();
    assert_eq!(empty.len(), 1);
    let o = run(empty, r(), &MergePolicy::default());
    assert_eq!(o.delta.created, keys(&["a", "b"]));
    assert_eq!(o.next.meta().level, Level::Structure);
    assert_eq!(o.next.meta().loaded_at_ms, 1_700_000_000_000);
    assert!(matches!(
        o.next.meta().watermark,
        Watermark::Sqlite {
            schema_version: 3,
            ..
        }
    ));
    assert_eq!(
        o.next.generation(),
        0,
        "the publisher stamps the generation"
    );
}

/// A full pass sets the namespace's level; an incremental pass keeps or lowers it.
#[test]
fn incremental_pass_keeps_or_lowers_the_level() {
    let empty = base(
        NsKey::schema([DB, "public"]),
        DbmsKind::Postgres,
        FoldRule::Exact,
    )
    .unwrap();
    let mut full = result(Vec::new(), Vec::new(), true);
    full.level = Level::Sources;
    let o = run(empty, full, &MergePolicy::default());
    assert_eq!(o.next.meta().level, Level::Sources);
    let mut inc = result(Vec::new(), Vec::new(), false);
    inc.level = Level::Names;
    let o = run(o.next, inc, &MergePolicy::default());
    assert_eq!(o.next.meta().level, Level::Names);
    let mut inc = result(Vec::new(), Vec::new(), false);
    inc.level = Level::Sources;
    let o = run(o.next, inc, &MergePolicy::default());
    assert_eq!(o.next.meta().level, Level::Names);
}

/// A changed child whose parent is cached (a fragment of one index) replaces the same child
/// (by OID: a rename) or is appended; the parent is altered (children).
#[test]
fn changed_child_under_a_kept_parent_is_upserted() {
    let index = |name: &str, oid: u32| {
        Subtree::new(
            Obj::new(ObjectKind::Index, name)
                .with_server_id(ServerId::Oid(oid))
                .with_props(Props::Index(Arc::new(IndexProps::default()))),
        )
    };
    let mut t = table("t", Some(700), &["a", "b"]);
    t.graft_under(t.root(), index("i1", 701));
    let prev = ns_pg(DB, "public").subtree(t).finish();
    let parent = pg_key("t").with_oid(700);
    let r = result(
        Vec::new(),
        vec![
            ChangedObject {
                parent: parent.clone(),
                subtree: index("i1x", 701),
            },
            ChangedObject {
                parent: parent.clone(),
                subtree: index("i2", 702),
            },
        ],
        false,
    );
    let o = run(prev, r, &MergePolicy::default());
    let t = idx_of(&o.next, ObjectKind::Table, "t");
    assert_eq!(child_names(&o.next, t), ["a", "b", "i1x", "i2"]);
    let ikey = |name: &str, oid: u32| ObjectKey {
        kind: ObjectKind::Index,
        path: [DB, "public", "t", name].into_iter().map(n).collect(),
        server_id: Some(ServerId::Oid(oid)),
        overload_sig: None,
        raw_kind: None,
    };
    assert_eq!(
        o.delta.renamed,
        vec![(ikey("i1", 701), ikey("i1x", 701), Confidence::ONE)]
    );
    assert_eq!(o.delta.created, vec![ikey("i2", 702)]);
    let facets = ChangedFacets {
        children: true,
        ..Default::default()
    };
    assert_eq!(o.delta.altered, vec![(parent, facets)]);
}

/// Edge 9: a changed child whose parent is neither cached nor re-read schedules a fragment of
/// the parent; the child is not placed anywhere.
#[test]
fn changed_child_under_a_missing_parent_schedules_the_parent() {
    let prev = pg_prev(&m(&["a"]));
    let ghost = pg_key("ghost");
    let r = result(
        Vec::new(),
        vec![ChangedObject {
            parent: ghost.clone(),
            subtree: Subtree::new(Obj::new(ObjectKind::Index, "i")),
        }],
        false,
    );
    let o = run(prev, r, &MergePolicy::default());
    assert_eq!(top_names(&o.next), ["a"]);
    assert_eq!(o.next.len(), o.prev.len());
    assert!(o.delta.is_empty());
    assert_eq!(o.report.fragments, vec![ghost]);
}

/// The namespace root re-read with new properties replaces the root's; its subtree's children
/// are top-level changed objects.
#[test]
fn root_update_replaces_the_root_props() {
    let schema = |comment: Option<&str>| {
        Obj::new(ObjectKind::Schema, "public").with_props(Props::Schema(Arc::new(SchemaProps {
            owner: None,
            comment: comment.map(n),
        })))
    };
    let mut t = Subtree::new(schema(None));
    t.graft_under(t.root(), table("a", Some(oid("a")), COLS));
    let prev = NamespaceSnapshot::build(
        NsKey::schema([DB, "public"]),
        DbmsKind::Postgres,
        FoldRule::Exact,
        NsMeta::default(),
        t,
    )
    .unwrap();
    let mut root = Subtree::new(schema(Some("Main schema")));
    root.graft_under(root.root(), table("a", Some(oid("a")), &["only"]));
    let r = result(Vec::new(), vec![top(&prev, root)], false);
    let o = run(prev, r, &MergePolicy::default());
    assert_eq!(
        o.next.root().props().comment().map(|c| &**c),
        Some("Main schema")
    );
    let root_key = o.prev.object_key(0).unwrap();
    let comment = ChangedFacets {
        comment: true,
        ..Default::default()
    };
    let children = ChangedFacets {
        children: true,
        ..Default::default()
    };
    assert_eq!(
        o.delta.altered,
        vec![(pg_key("a"), children), (root_key, comment)]
    );
    assert_eq!(
        child_names(&o.next, idx_of(&o.next, ObjectKind::Table, "a")),
        ["only"]
    );
}

/// A result about another namespace is refused, and nothing is computed.
#[test]
fn wrong_namespace_is_refused() {
    let prev = pg_prev(&m(&["a"]));
    let other = ns_pg(DB, "sales").finish();
    let r = result(
        Vec::new(),
        vec![top(&other, table("x", Some(1), COLS))],
        false,
    );
    assert_eq!(
        merge(Some(&prev), r, &MergePolicy::default()).unwrap_err(),
        MergeError::WrongNamespace
    );
    let r = result(
        Vec::new(),
        vec![top(
            &prev,
            Subtree::new(Obj::new(ObjectKind::Schema, "sales")),
        )],
        false,
    );
    assert_eq!(
        merge(Some(&prev), r, &MergePolicy::default()).unwrap_err(),
        MergeError::WrongNamespace
    );
}

/// A changed subtree that breaks the kind schema is refused as a model error.
#[test]
fn invalid_changed_subtree_is_a_model_error() {
    let prev = pg_prev(&m(&["a"]));
    let r = result(
        Vec::new(),
        vec![top(
            &prev,
            Subtree::new(Obj::new(ObjectKind::Column, "loose")),
        )],
        false,
    );
    assert!(matches!(
        merge(Some(&prev), r, &MergePolicy::default()),
        Err(MergeError::Model(_))
    ));
}

/// The delta survives a serde round trip and, applied to the previous top-level keys, gives
/// the next namespace's.
#[cfg(feature = "system")]
#[test]
fn delta_round_trips_through_serde_and_applies() {
    let o = merge_pg(m(&["a", "b", "r"]), e(&["a", "c", "s"]), c(&["c", "a"]));
    let json = serde_json::to_string(&o.delta).unwrap();
    let back: CatalogDelta = serde_json::from_str(&json).unwrap();
    assert_eq!(back, o.delta);
    let applied = apply_delta(o.prev.key(), &top_level_keys(&o.prev), &back).unwrap();
    assert_eq!(applied, top_level_keys(&o.next));
    // A lost delta does not apply twice.
    assert!(apply_delta(o.prev.key(), &applied, &back).is_err());
}
