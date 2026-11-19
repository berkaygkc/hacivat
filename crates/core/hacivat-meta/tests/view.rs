//! HV-MODEL-02 step 5 and 6 (M4 Task 9): the tri-state `ModelView` over one pinned
//! generation, the per-dialect search paths and the negative cache (alg/02 HV-MODEL-02 test
//! cases 1, 2, 4, 5; edge cases 3–9; R-M4-41).

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use hacivat_meta::fixture::{ns_mysql, ns_pg, ns_sqlite};
use hacivat_meta::ids::{NsKey, ObjectKey, ServerId};
use hacivat_meta::kind::{KindGroup, ObjectKind};
use hacivat_meta::name::{DialectId, IdentCtx, Name, NamePart};
use hacivat_meta::namespace::{Level, NamespaceSnapshot, Watermark};
use hacivat_meta::props::FacetKind;
use hacivat_meta::search_path::SearchPath;
use hacivat_meta::snapshot::{
    Catalog, CatalogChange, CatalogSnapshot, ContainerKey, IntroErrorCode, LoadState, ServerFacts,
};
use hacivat_meta::view::{
    Freshness, Lookup, LookupHooks, MissingReason, ModelView, MonoClock, ScopeFilter, SnapshotView,
    UnknownReason,
};
use hacivat_proto::DbmsKind;

const DB: &str = "db";

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

fn name(s: &str) -> Name {
    Arc::from(s)
}

/// A typed path: `"a.b"` unquoted; a part written `"X"` (with the double quotes) is quoted.
fn p(dotted: &str) -> Vec<NamePart> {
    dotted
        .split('.')
        .map(
            |part| match part.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                Some(inner) => NamePart {
                    text: Arc::from(inner),
                    quoted: true,
                },
                None => NamePart {
                    text: Arc::from(part),
                    quoted: false,
                },
            },
        )
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

fn failed() -> LoadState {
    LoadState::Failed {
        at: SystemTime::UNIX_EPOCH,
        error: IntroErrorCode::PermissionDenied,
        retry_after: Duration::from_secs(60),
    }
}

/// A catalog of `dbms` with each namespace published at its state, plus `states` for
/// namespaces that have no arena, and the schema list of `DB` (PostgreSQL) or of the server
/// marked loaded when `list_known`.
fn snapshot(
    dbms: DbmsKind,
    ident: IdentCtx,
    nss: Vec<(NamespaceSnapshot, LoadState)>,
    states: Vec<(NsKey, LoadState)>,
    list_known: bool,
) -> Arc<CatalogSnapshot> {
    let catalog = Catalog::new(Arc::new(ServerFacts::new(dbms, ident)));
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
    if list_known {
        let db = name(DB);
        let list = if dbms == DbmsKind::Postgres {
            ContainerKey::schema_list(Some(&db))
        } else {
            ContainerKey::schema_list(None)
        };
        changes.push(CatalogChange::SetState(list, loaded(Level::Names)));
    }
    publisher.publish(changes).unwrap();
    catalog.current()
}

fn public_path() -> SearchPath {
    SearchPath::exact(Some(name(DB)), [NsKey::schema([DB, "public"])])
}

/// alg/02 test 1's snapshot: `public` loaded at `Structure` with `{t}`; resolving along
/// `public` only.
fn view_with(schema: &str, tables: &[&str]) -> SnapshotView {
    let mut b = ns_pg(DB, schema);
    for (i, t) in tables.iter().enumerate() {
        b = b.table_oid(t, 16_400 + i as u32, &["id"]);
    }
    let snap = snapshot(
        DbmsKind::Postgres,
        pg_ctx(),
        vec![(b.finish(), loaded(Level::Structure))],
        vec![],
        true,
    );
    SnapshotView::new(snap).with_search_path(SearchPath::exact(
        Some(name(DB)),
        [NsKey::schema([DB, schema])],
    ))
}

fn found_name(l: Lookup<hacivat_meta::snapshot::ObjHandle>) -> String {
    match l {
        Lookup::Found(h) => format!("{}.{}", h.ns.key().root_name(), h.obj().name()),
        other => panic!("expected Found, got {other:?}"),
    }
}

/// Hooks that count `on_unknown` calls and record `on_missing_probe` calls.
#[derive(Default)]
struct Recorder {
    unknown: AtomicUsize,
    unknown_ns: Mutex<Vec<NsKey>>,
    probes: Mutex<Vec<(NsKey, String)>>,
}

fn hooks(rec: &Arc<Recorder>, clock: Option<Arc<dyn MonoClock>>) -> Arc<LookupHooks> {
    let a = rec.clone();
    let b = rec.clone();
    let h = LookupHooks::new(
        move |ns: &NsKey| {
            a.unknown.fetch_add(1, Ordering::SeqCst);
            a.unknown_ns.lock().unwrap().push(ns.clone());
        },
        move |ns: &NsKey, n: &NamePart| {
            b.probes
                .lock()
                .unwrap()
                .push((ns.clone(), n.text.to_string()));
        },
    );
    Arc::new(match clock {
        Some(c) => h.with_clock(c),
        None => h,
    })
}

/// A clock the test moves by hand (alg/02 step 6's 30 s TTL without waiting).
struct ManualClock {
    base: Instant,
    offset_ms: AtomicU64,
}

impl ManualClock {
    fn new() -> Arc<Self> {
        Arc::new(ManualClock {
            base: Instant::now(),
            offset_ms: AtomicU64::new(0),
        })
    }
    fn advance(&self, d: Duration) {
        self.offset_ms
            .fetch_add(d.as_millis() as u64, Ordering::SeqCst);
    }
}

impl MonoClock for ManualClock {
    fn now(&self) -> Instant {
        self.base + Duration::from_millis(self.offset_ms.load(Ordering::SeqCst))
    }
}

/// `public` is loaded with `t`; `sales` is listed but not loaded; `on_unknown` is counted.
fn view_counting_unknown(not_loaded: &str) -> (SnapshotView, Arc<Recorder>) {
    let snap = snapshot(
        DbmsKind::Postgres,
        pg_ctx(),
        vec![(
            ns_pg(DB, "public").table_oid("t", 1, &[]).finish(),
            loaded(Level::Structure),
        )],
        vec![(NsKey::schema([DB, not_loaded]), LoadState::NotLoaded)],
        true,
    );
    let rec = Arc::new(Recorder::default());
    let v = SnapshotView::new(snap)
        .with_search_path(public_path())
        .with_hooks(hooks(&rec, None));
    (v, rec)
}

#[derive(Debug)]
struct ExcludeSchemaObjects {
    ns: NsKey,
    prefix: &'static str,
}

impl ScopeFilter for ExcludeSchemaObjects {
    fn excludes(&self, ns: &NsKey, _kind: Option<ObjectKind>, name: Option<&str>) -> bool {
        *ns == self.ns && name.is_none_or(|n| n.starts_with(self.prefix))
    }
}

/// A scope that leaves schema `hr` out of the introspection scope.
fn scoped_view_excluding(schema: &str) -> SnapshotView {
    let snap = snapshot(
        DbmsKind::Postgres,
        pg_ctx(),
        vec![(
            ns_pg(DB, "public").table_oid("t", 1, &[]).finish(),
            loaded(Level::Structure),
        )],
        vec![(NsKey::schema([DB, schema]), LoadState::OutOfScope)],
        true,
    );
    SnapshotView::new(snap).with_search_path(public_path())
}

#[test]
fn found_and_missing_in_a_loaded_namespace() {
    let v = view_with("public", &["t"]);
    assert!(matches!(
        v.lookup(&p("t"), KindGroup::Relations),
        Lookup::Found(_)
    ));
    assert!(matches!(
        v.lookup(&p("zz"), KindGroup::Relations),
        Lookup::Missing(MissingReason::NotFound)
    ));
    // Qualified, and with the folded and quoted forms PostgreSQL uses.
    assert_eq!(
        found_name(v.lookup(&p("public.t"), KindGroup::Relations)),
        "public.t"
    );
    assert_eq!(
        found_name(v.lookup(&p("PUBLIC.T"), KindGroup::Relations)),
        "public.t"
    );
    assert_eq!(
        found_name(v.lookup(&p("db.public.t"), KindGroup::Relations)),
        "public.t"
    );
    assert!(matches!(
        v.lookup(&p("\"T\""), KindGroup::Relations),
        Lookup::Missing(MissingReason::NotFound)
    ));
    // Another group of the same namespace is loaded too.
    assert!(matches!(
        v.lookup(&p("t"), KindGroup::Routines),
        Lookup::Missing(MissingReason::NotFound)
    ));
    // A schema the loaded schema list does not hold is missing, its objects too.
    assert!(matches!(
        v.lookup(&p("nosuch.t"), KindGroup::Relations),
        Lookup::Missing(MissingReason::NotFound)
    ));
    // An empty path or a four-part name is never a fact.
    assert!(matches!(
        v.lookup(&[], KindGroup::Relations),
        Lookup::Unknown(UnknownReason::NotLoaded)
    ));
    assert!(matches!(
        v.lookup(&p("a.db.public.t"), KindGroup::Relations),
        Lookup::Unknown(UnknownReason::NotLoaded)
    ));
}

#[test]
fn not_loaded_is_unknown_and_enqueues_once_for_100_lookups() {
    let (v, hits) = view_counting_unknown("sales");
    for _ in 0..100 {
        assert!(matches!(
            v.lookup(&p("sales.x"), KindGroup::Relations),
            Lookup::Unknown(UnknownReason::NotLoaded)
        ));
    }
    assert_eq!(hits.unknown.load(Ordering::SeqCst), 1);
    assert_eq!(
        *hits.unknown_ns.lock().unwrap(),
        vec![NsKey::schema([DB, "sales"])]
    );
    // A clone of the view, or another view of the same generation sharing the hooks, does not
    // ask again; nor does a found lookup.
    let again = v.clone();
    assert!(matches!(
        again.lookup(&p("SALES.y"), KindGroup::Relations),
        Lookup::Unknown(UnknownReason::NotLoaded)
    ));
    assert!(matches!(
        again.lookup(&p("t"), KindGroup::Relations),
        Lookup::Found(_)
    ));
    assert_eq!(hits.unknown.load(Ordering::SeqCst), 1);
}

#[test]
fn a_new_generation_asks_for_a_not_loaded_namespace_again() {
    let rec = Arc::new(Recorder::default());
    let h = hooks(&rec, None);
    let catalog = Catalog::new(Arc::new(ServerFacts::new(DbmsKind::Postgres, pg_ctx())));
    let publisher = catalog.publisher().unwrap();
    let sales = NsKey::schema([DB, "sales"]);
    publisher
        .publish(vec![CatalogChange::SetState(
            ContainerKey::Namespace(sales.clone()),
            LoadState::NotLoaded,
        )])
        .unwrap();
    let v1 = SnapshotView::new(catalog.current())
        .with_search_path(public_path())
        .with_hooks(h.clone());
    v1.lookup(&p("sales.x"), KindGroup::Relations);
    v1.lookup(&p("sales.x"), KindGroup::Relations);
    publisher
        .publish(vec![CatalogChange::SetState(
            ContainerKey::Namespace(NsKey::schema([DB, "hr"])),
            LoadState::NotLoaded,
        )])
        .unwrap();
    let v2 = SnapshotView::new(catalog.current())
        .with_search_path(public_path())
        .with_hooks(h);
    v2.lookup(&p("sales.x"), KindGroup::Relations);
    v2.lookup(&p("sales.x"), KindGroup::Relations);
    assert_eq!(rec.unknown.load(Ordering::SeqCst), 2);
}

#[test]
fn view_filter_never_affects_resolution() {
    // alg/02 test 4: a filter that matches `audit_*` (as the explorer's "hide like this"
    // would) never turns a loaded object into anything but `Found`; the view filter itself
    // never reaches the view (Task 24 checks the explorer end to end).
    let snap = snapshot(
        DbmsKind::Postgres,
        pg_ctx(),
        vec![(
            ns_pg(DB, "public")
                .table_oid("audit_log", 1, &["id"])
                .finish(),
            loaded(Level::Structure),
        )],
        vec![],
        true,
    );
    let public = NsKey::schema([DB, "public"]);
    let plain = SnapshotView::new(snap.clone()).with_search_path(public_path());
    let filtered = SnapshotView::new(snap)
        .with_search_path(public_path())
        .with_scope_filter(Arc::new(ExcludeSchemaObjects {
            ns: public.clone(),
            prefix: "audit_",
        }));
    for v in [&plain, &filtered] {
        assert_eq!(
            found_name(v.lookup(&p("audit_log"), KindGroup::Relations)),
            "public.audit_log"
        );
    }
    // What such a filter leaves unread is never a `Missing` fact, only `Unknown`.
    assert!(matches!(
        plain.lookup(&p("audit_trail"), KindGroup::Relations),
        Lookup::Missing(_)
    ));
    assert!(matches!(
        filtered.lookup(&p("audit_trail"), KindGroup::Relations),
        Lookup::Unknown(UnknownReason::OutOfScope)
    ));
    // Names outside the filter keep their facts.
    assert!(matches!(
        filtered.lookup(&p("orders"), KindGroup::Relations),
        Lookup::Missing(_)
    ));
}

#[test]
fn out_of_scope_is_unknown() {
    assert!(matches!(
        scoped_view_excluding("hr").lookup(&p("hr.emp"), KindGroup::Relations),
        Lookup::Unknown(UnknownReason::OutOfScope)
    ));
    // Even when an old arena of that namespace is still in the catalog.
    let snap = snapshot(
        DbmsKind::Postgres,
        pg_ctx(),
        vec![(
            ns_pg(DB, "hr").table_oid("emp", 1, &[]).finish(),
            LoadState::OutOfScope,
        )],
        vec![],
        true,
    );
    let v = SnapshotView::new(snap).with_search_path(public_path());
    assert!(matches!(
        v.lookup(&p("hr.emp"), KindGroup::Relations),
        Lookup::Unknown(UnknownReason::OutOfScope)
    ));
    assert!(matches!(
        v.children(
            &ObjectKey {
                kind: ObjectKind::Schema,
                path: [name(DB), name("hr")].into_iter().collect(),
                server_id: None,
                overload_sig: None,
                raw_kind: None,
            },
            ObjectKind::Table
        ),
        Lookup::Unknown(UnknownReason::OutOfScope)
    ));
}

#[test]
fn failed_namespace_is_unknown_with_reason() {
    let rec = Arc::new(Recorder::default());
    let snap = snapshot(
        DbmsKind::Postgres,
        pg_ctx(),
        vec![(ns_pg(DB, "hr").table_oid("emp", 1, &[]).finish(), failed())],
        vec![(NsKey::schema([DB, "sales"]), failed())],
        true,
    );
    let v = SnapshotView::new(snap)
        .with_search_path(public_path())
        .with_hooks(hooks(&rec, None));
    assert!(matches!(
        v.lookup(&p("sales.x"), KindGroup::Relations),
        Lookup::Unknown(UnknownReason::Failed)
    ));
    // A cached arena of a namespace whose refresh failed still shows what it holds; a miss
    // in it is not a fact.
    assert!(matches!(
        v.lookup(&p("hr.emp"), KindGroup::Relations),
        Lookup::Found(_)
    ));
    assert!(matches!(
        v.lookup(&p("hr.zz"), KindGroup::Relations),
        Lookup::Unknown(UnknownReason::Failed)
    ));
    // A failed namespace backs off on its own: the view does not ask for it.
    assert_eq!(rec.unknown.load(Ordering::SeqCst), 0);
    // `Partial` (the login could not read everything) is not complete either.
    let snap = snapshot(
        DbmsKind::Postgres,
        pg_ctx(),
        vec![(
            ns_pg(DB, "hr").table_oid("emp", 1, &[]).finish(),
            LoadState::Partial {
                reason: IntroErrorCode::PermissionDenied,
            },
        )],
        vec![],
        true,
    );
    assert!(matches!(
        SnapshotView::new(snap)
            .with_search_path(public_path())
            .lookup(&p("hr.zz"), KindGroup::Relations),
        Lookup::Unknown(UnknownReason::Failed)
    ));
}

#[test]
fn first_load_in_progress_is_unknown_never_missing() {
    let loading = LoadState::Loading {
        generation: 1,
        level: Level::Structure,
    };
    let snap = snapshot(
        DbmsKind::Postgres,
        pg_ctx(),
        vec![],
        vec![(NsKey::schema([DB, "public"]), loading.clone())],
        true,
    );
    let v = SnapshotView::new(snap).with_search_path(public_path());
    for path in ["t", "public.t"] {
        assert!(matches!(
            v.lookup(&p(path), KindGroup::Relations),
            Lookup::Unknown(UnknownReason::Loading)
        ));
    }
    // A refresh in progress over a cached arena: found objects stay found, misses wait.
    let snap = snapshot(
        DbmsKind::Postgres,
        pg_ctx(),
        vec![(ns_pg(DB, "public").table_oid("t", 1, &[]).finish(), loading)],
        vec![],
        true,
    );
    let v = SnapshotView::new(snap).with_search_path(public_path());
    assert!(matches!(
        v.lookup(&p("t"), KindGroup::Relations),
        Lookup::Found(_)
    ));
    assert!(matches!(
        v.lookup(&p("zz"), KindGroup::Relations),
        Lookup::Unknown(UnknownReason::Loading)
    ));
}

#[test]
fn names_level_is_never_enough_for_missing() {
    // `Names` reads "major object names"; which kinds that covers per engine is pinned by the
    // introspectors (Tasks 16–18), so a miss at `Names` is not a fact (fail closed).
    let rec = Arc::new(Recorder::default());
    let snap = snapshot(
        DbmsKind::Postgres,
        pg_ctx(),
        vec![(
            ns_pg(DB, "public").table_oid("t", 1, &["id"]).finish(),
            loaded(Level::Names),
        )],
        vec![],
        true,
    );
    let v = SnapshotView::new(snap)
        .with_search_path(public_path())
        .with_hooks(hooks(&rec, None));
    assert!(matches!(
        v.lookup(&p("t"), KindGroup::Relations),
        Lookup::Found(_)
    ));
    assert!(matches!(
        v.lookup(&p("zz"), KindGroup::Relations),
        Lookup::Unknown(UnknownReason::NotLoaded)
    ));
    let t = ObjectKey {
        kind: ObjectKind::Table,
        path: [name(DB), name("public"), name("t")].into_iter().collect(),
        server_id: Some(ServerId::Oid(1)),
        overload_sig: None,
        raw_kind: None,
    };
    assert!(matches!(
        v.children(&t, ObjectKind::Column),
        Lookup::Unknown(UnknownReason::NotLoaded)
    ));
    // Not loaded at a sufficient level: the view asks for the namespace once.
    assert_eq!(rec.unknown.load(Ordering::SeqCst), 1);
}

#[test]
fn children_of_a_loaded_object_and_of_a_missing_one() {
    let v = view_with("public", &["t"]);
    let t = ObjectKey {
        kind: ObjectKind::Table,
        path: [name(DB), name("public"), name("t")].into_iter().collect(),
        server_id: Some(ServerId::Oid(16_400)),
        overload_sig: None,
        raw_kind: None,
    };
    let Lookup::Found(cols) = v.children(&t, ObjectKind::Column) else {
        panic!("columns of a loaded table")
    };
    assert_eq!(
        cols.iter()
            .map(|h| h.obj().name().to_string())
            .collect::<Vec<_>>(),
        ["id"]
    );
    let schema = ObjectKey {
        kind: ObjectKind::Schema,
        path: [name(DB), name("public")].into_iter().collect(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    };
    let Lookup::Found(tables) = v.children(&schema, ObjectKind::Table) else {
        panic!("tables of a loaded schema")
    };
    assert_eq!(tables.len(), 1);
    let mut gone = t.clone();
    gone.path[2] = name("gone");
    gone.server_id = None;
    assert!(matches!(
        v.children(&gone, ObjectKind::Column),
        Lookup::Missing(MissingReason::NotFound)
    ));
    // A kind group whose children load lazily answers by its own state.
    let snap = v.snapshot().clone();
    let catalog = Catalog::new(snap.server.clone());
    let publisher = catalog.publisher().unwrap();
    let mut changes: Vec<CatalogChange> = snap
        .namespaces
        .values()
        .map(|n| CatalogChange::Put(n.clone()))
        .collect();
    changes.extend(
        snap.states
            .iter()
            .map(|(k, s)| CatalogChange::SetState(k.clone(), s.clone())),
    );
    changes.push(CatalogChange::SetState(
        ContainerKey::Children {
            ns: NsKey::schema([DB, "public"]),
            parent: t.clone(),
            kind: ObjectKind::Column,
        },
        LoadState::Loading {
            generation: 2,
            level: Level::Structure,
        },
    ));
    publisher.publish(changes).unwrap();
    assert!(matches!(
        SnapshotView::new(catalog.current()).children(&t, ObjectKind::Column),
        Lookup::Unknown(UnknownReason::Loading)
    ));
}

#[test]
fn pg_search_path_order_is_temp_then_path_then_pg_catalog() {
    let path = SearchPath::for_pg(
        &name(DB),
        &[name("$user"), name("public"), name("pg_catalog")],
        Some(&name("alice")),
        Some(&name("pg_temp_3")),
    );
    let order: Vec<_> = path
        .entries_for(KindGroup::Relations)
        .map(|e| {
            e.ns.path
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(".")
        })
        .collect();
    assert_eq!(
        order,
        ["db.pg_temp_3", "db.alice", "db.public", "db.pg_catalog"]
    );
    // The temporary schema holds relations and types only, never routines.
    let routines: Vec<_> = path
        .entries_for(KindGroup::Routines)
        .map(|e| e.ns.root_name().to_string())
        .collect();
    assert_eq!(routines, ["alice", "public", "pg_catalog"]);
    assert_eq!(
        path.entries_for(KindGroup::Types)
            .next()
            .unwrap()
            .ns
            .root_name(),
        "pg_temp_3"
    );
    // An explicit `pg_temp` keeps its place; without a temporary schema it is skipped.
    let explicit = SearchPath::for_pg(
        &name(DB),
        &[name("public"), name("pg_temp")],
        None,
        Some(&name("pg_temp_3")),
    );
    let order: Vec<_> = explicit
        .entries_for(KindGroup::Relations)
        .map(|e| e.ns.root_name().to_string())
        .collect();
    assert_eq!(order, ["pg_catalog", "public", "pg_temp_3"]);
    let no_temp = SearchPath::for_pg(&name(DB), &[name("public"), name("pg_temp")], None, None);
    let order: Vec<_> = no_temp
        .entries_for(KindGroup::Relations)
        .map(|e| e.ns.root_name().to_string())
        .collect();
    assert_eq!(order, ["pg_catalog", "public"]);

    // Resolution follows the order: a temporary table shadows `public.t`.
    let snap = snapshot(
        DbmsKind::Postgres,
        pg_ctx(),
        vec![
            (
                ns_pg(DB, "public").table_oid("t", 1, &[]).finish(),
                loaded(Level::Structure),
            ),
            (
                ns_pg(DB, "pg_temp_3").table_oid("t", 2, &[]).finish(),
                loaded(Level::Structure),
            ),
            (ns_pg(DB, "pg_catalog").finish(), loaded(Level::Structure)),
        ],
        vec![],
        true,
    );
    let v = SnapshotView::new(snap).with_search_path(path);
    assert_eq!(
        found_name(v.lookup(&p("t"), KindGroup::Relations)),
        "pg_temp_3.t"
    );
    // `$user` (no schema `alice` on the listed server) is skipped; the miss is a fact.
    assert!(matches!(
        v.lookup(&p("zz"), KindGroup::Relations),
        Lookup::Missing(_)
    ));
}

#[test]
fn pg_catalog_implicitly_first_when_absent() {
    let path = SearchPath::for_pg(&name(DB), &[name("public")], None, None);
    let order: Vec<_> = path
        .entries_for(KindGroup::Routines)
        .map(|e| e.ns.root_name().to_string())
        .collect();
    assert_eq!(order, ["pg_catalog", "public"]);
    // `pg_catalog` is not introspected live (it comes from the system snapshot, Task 26): it
    // shadows `public` for anything it might hold, so a miss is unknown, a hit in `public`
    // is found.
    let snap = snapshot(
        DbmsKind::Postgres,
        pg_ctx(),
        vec![(
            ns_pg(DB, "public").table_oid("t", 1, &[]).finish(),
            loaded(Level::Structure),
        )],
        vec![(NsKey::schema([DB, "pg_catalog"]), LoadState::OutOfScope)],
        true,
    );
    let v = SnapshotView::new(snap).with_search_path(path);
    assert_eq!(
        found_name(v.lookup(&p("t"), KindGroup::Relations)),
        "public.t"
    );
    assert!(matches!(
        v.lookup(&p("pg_class"), KindGroup::Relations),
        Lookup::Unknown(UnknownReason::OutOfScope)
    ));
}

#[test]
fn an_unlisted_path_schema_is_skipped_only_when_the_list_is_known() {
    let path = SearchPath::for_pg(
        &name(DB),
        &[name("$user"), name("public")],
        Some(&name("alice")),
        None,
    );
    let public = ns_pg(DB, "public").table_oid("t", 1, &[]).finish();
    let pgcat = ns_pg(DB, "pg_catalog").finish();
    let unknown_list = snapshot(
        DbmsKind::Postgres,
        pg_ctx(),
        vec![
            (public.clone(), loaded(Level::Structure)),
            (pgcat.clone(), loaded(Level::Structure)),
        ],
        vec![],
        false,
    );
    let v = SnapshotView::new(unknown_list).with_search_path(path.clone());
    assert!(matches!(
        v.lookup(&p("zz"), KindGroup::Relations),
        Lookup::Unknown(UnknownReason::NotLoaded)
    ));
    assert!(matches!(
        v.lookup(&p("nosuch.zz"), KindGroup::Relations),
        Lookup::Unknown(UnknownReason::NotLoaded)
    ));
    assert!(matches!(
        v.lookup(&p("t"), KindGroup::Relations),
        Lookup::Found(_)
    ));
    let known_list = snapshot(
        DbmsKind::Postgres,
        pg_ctx(),
        vec![
            (public, loaded(Level::Structure)),
            (pgcat, loaded(Level::Structure)),
        ],
        vec![],
        true,
    );
    let v = SnapshotView::new(known_list).with_search_path(path);
    assert!(matches!(
        v.lookup(&p("zz"), KindGroup::Relations),
        Lookup::Missing(_)
    ));
}

#[test]
fn mysql_resolves_in_the_current_database_only() {
    let snap = snapshot(
        DbmsKind::MySql,
        my_ctx(0),
        vec![
            (
                ns_mysql("shop", 0).table("orders", &["id"]).finish(),
                loaded(Level::Structure),
            ),
            (
                ns_mysql("crm", 0).table("customers", &["id"]).finish(),
                loaded(Level::Structure),
            ),
        ],
        vec![],
        true,
    );
    let v = SnapshotView::new(snap.clone())
        .with_search_path(SearchPath::for_mysql(Some(&name("shop"))));
    assert_eq!(
        found_name(v.lookup(&p("orders"), KindGroup::Relations)),
        "shop.orders"
    );
    // `customers` lives in another database: unqualified, it is missing here.
    assert!(matches!(
        v.lookup(&p("customers"), KindGroup::Relations),
        Lookup::Missing(_)
    ));
    assert_eq!(
        found_name(v.lookup(&p("crm.customers"), KindGroup::Relations)),
        "crm.customers"
    );
    // lower_case_table_names = 0: table names are exact.
    assert!(matches!(
        v.lookup(&p("Orders"), KindGroup::Relations),
        Lookup::Missing(_)
    ));
    // No current database: nothing unqualified can be said.
    let none = SnapshotView::new(snap).with_search_path(SearchPath::for_mysql(None));
    assert!(matches!(
        none.lookup(&p("orders"), KindGroup::Relations),
        Lookup::Unknown(UnknownReason::NotLoaded)
    ));
}

#[test]
fn mysql_lctn1_schema_and_table_names_fold_ascii_only() {
    let snap = snapshot(
        DbmsKind::MySql,
        my_ctx(1),
        vec![(
            ns_mysql("Shop", 1).table("Orders", &["id"]).finish(),
            loaded(Level::Structure),
        )],
        vec![],
        true,
    );
    let v = SnapshotView::new(snap);
    assert_eq!(
        found_name(v.lookup(&p("shop.ORDERS"), KindGroup::Relations)),
        "Shop.Orders"
    );
    assert!(matches!(
        v.lookup(&p("SHOP.nope"), KindGroup::Relations),
        Lookup::Missing(_)
    ));
}

#[test]
fn sqlite_order_is_temp_main_attached() {
    let path = SearchPath::for_sqlite(&[name("main"), name("temp"), name("aux"), name("logs")]);
    let order: Vec<_> = path
        .entries_for(KindGroup::Relations)
        .map(|e| e.ns.root_name().to_string())
        .collect();
    assert_eq!(order, ["temp", "main", "aux", "logs"]);
    let snap = snapshot(
        DbmsKind::Sqlite,
        IdentCtx::new(DialectId::Sqlite),
        vec![
            (
                ns_sqlite("main")
                    .table("t", &["a"])
                    .table("m", &[])
                    .finish(),
                loaded(Level::Structure),
            ),
            (
                ns_sqlite("temp").table("t", &["a"]).finish(),
                loaded(Level::Structure),
            ),
            (
                ns_sqlite("aux").table("x", &[]).table("m", &[]).finish(),
                loaded(Level::Structure),
            ),
            (ns_sqlite("logs").finish(), loaded(Level::Structure)),
        ],
        vec![],
        true,
    );
    let v = SnapshotView::new(snap).with_search_path(path);
    assert_eq!(
        found_name(v.lookup(&p("T"), KindGroup::Relations)),
        "temp.t"
    );
    assert_eq!(
        found_name(v.lookup(&p("m"), KindGroup::Relations)),
        "main.m"
    );
    assert_eq!(found_name(v.lookup(&p("x"), KindGroup::Relations)), "aux.x");
    assert_eq!(
        found_name(v.lookup(&p("AUX.m"), KindGroup::Relations)),
        "aux.m"
    );
    assert!(matches!(
        v.lookup(&p("zz"), KindGroup::Relations),
        Lookup::Missing(_)
    ));
}

#[test]
fn negative_probe_at_most_once_per_ttl_per_namespace() {
    let clock = ManualClock::new();
    let rec = Arc::new(Recorder::default());
    let h = hooks(&rec, Some(clock.clone()));
    let snap = view_with("public", &["t"]).snapshot().clone();
    let v = SnapshotView::new(snap)
        .with_search_path(public_path())
        .with_hooks(h.clone());
    let public = NsKey::schema([DB, "public"]);
    for n in ["n", "n", "m", "n"] {
        assert!(matches!(
            v.lookup(&p(n), KindGroup::Relations),
            Lookup::Missing(_)
        ));
    }
    assert_eq!(
        *rec.probes.lock().unwrap(),
        vec![(public.clone(), "n".into())]
    );
    clock.advance(Duration::from_secs(29));
    v.lookup(&p("m"), KindGroup::Relations);
    assert_eq!(rec.probes.lock().unwrap().len(), 1);
    clock.advance(Duration::from_secs(1));
    v.lookup(&p("m"), KindGroup::Relations);
    v.lookup(&p("m"), KindGroup::Relations);
    assert_eq!(
        *rec.probes.lock().unwrap(),
        vec![(public.clone(), "n".into()), (public, "m".into())]
    );
    // A found name, or an unknown one, never probes.
    clock.advance(Duration::from_secs(31));
    v.lookup(&p("t"), KindGroup::Relations);
    assert_eq!(rec.probes.lock().unwrap().len(), 2);
    assert_eq!(LookupHooks::PROBE_TTL, Duration::from_secs(30));
}

#[test]
fn find_by_oid_is_scoped_by_database() {
    let a = ns_pg("db1", "public").table_oid("a", 16_500, &[]).finish();
    let b = ns_pg("db2", "public").table_oid("b", 16_500, &[]).finish();
    let catalog = Catalog::new(Arc::new(ServerFacts::new(DbmsKind::Postgres, pg_ctx())));
    let publisher = catalog.publisher().unwrap();
    let mut changes = vec![];
    for ns in [a, b] {
        changes.push(CatalogChange::SetState(
            ContainerKey::Namespace(ns.key().clone()),
            loaded(Level::Structure),
        ));
        changes.push(CatalogChange::Put(Arc::new(ns)));
    }
    changes.push(CatalogChange::SetState(
        ContainerKey::schema_list(Some(&name("db1"))),
        loaded(Level::Names),
    ));
    publisher.publish(changes).unwrap();
    let v = SnapshotView::new(catalog.current());
    let Lookup::Found(h) = v.find_by_server_id(&name("db1"), ServerId::Oid(16_500)) else {
        panic!("db1's table")
    };
    assert_eq!(h.obj().name().as_ref(), "a");
    let Lookup::Found(h) = v.find_by_server_id(&name("db2"), ServerId::Oid(16_500)) else {
        panic!("db2's table")
    };
    assert_eq!(h.obj().name().as_ref(), "b");
    // db1's schema list is known and every schema is loaded: a free OID is missing there.
    assert!(matches!(
        v.find_by_server_id(&name("db1"), ServerId::Oid(99)),
        Lookup::Missing(_)
    ));
    // db2's schema list is not: unknown.
    assert!(matches!(
        v.find_by_server_id(&name("db2"), ServerId::Oid(99)),
        Lookup::Unknown(UnknownReason::NotLoaded)
    ));
    assert!(matches!(
        v.find_by_server_id(&name("db3"), ServerId::Oid(16_500)),
        Lookup::Unknown(_)
    ));
}

#[test]
fn facet_not_loaded_never_blocks_object_existence() {
    let catalog = Catalog::new(Arc::new(ServerFacts::new(DbmsKind::Postgres, pg_ctx())));
    let publisher = catalog.publisher().unwrap();
    let public = ns_pg(DB, "public").table_oid("t", 1, &[]).finish();
    let t = public.object_key(1).unwrap();
    publisher
        .publish(vec![
            CatalogChange::SetState(
                ContainerKey::Namespace(public.key().clone()),
                loaded(Level::Structure),
            ),
            CatalogChange::SetState(
                ContainerKey::Facet {
                    ns: public.key().clone(),
                    object: t.clone(),
                    facet: FacetKind::Grants,
                },
                LoadState::NotLoaded,
            ),
            CatalogChange::SetState(
                ContainerKey::Facet {
                    ns: public.key().clone(),
                    object: t,
                    facet: FacetKind::Statistics,
                },
                failed(),
            ),
            CatalogChange::SetState(
                ContainerKey::schema_list(Some(&name(DB))),
                loaded(Level::Names),
            ),
            CatalogChange::Put(Arc::new(public)),
        ])
        .unwrap();
    let v = SnapshotView::new(catalog.current()).with_search_path(public_path());
    assert!(matches!(
        v.lookup(&p("t"), KindGroup::Relations),
        Lookup::Found(_)
    ));
    assert!(matches!(
        v.lookup(&p("zz"), KindGroup::Relations),
        Lookup::Missing(_)
    ));
}

#[test]
fn server_dependent_key_is_unknown_never_missing() {
    // R-M4-41: a MySQL column-class probe with a non-ASCII cased letter is not pinned.
    let snap = snapshot(
        DbmsKind::MySql,
        my_ctx(0),
        vec![(
            ns_mysql("shop", 0)
                .table("orders", &["id"])
                .table("Äb", &[])
                .finish(),
            loaded(Level::Structure),
        )],
        vec![],
        true,
    );
    let v = SnapshotView::new(snap).with_search_path(SearchPath::for_mysql(Some(&name("shop"))));
    // Routines compare caselessly: `äb` vs a stored routine is server dependent.
    assert!(matches!(
        v.lookup(&p("äb"), KindGroup::Routines),
        Lookup::Unknown(UnknownReason::ServerDependent)
    ));
    // Exact table names (lctn 0): a non-ASCII probe is pinned and a miss is a fact.
    assert!(matches!(
        v.lookup(&p("äb"), KindGroup::Relations),
        Lookup::Missing(_)
    ));
    assert!(matches!(
        v.lookup(&p("Äb"), KindGroup::Relations),
        Lookup::Found(_)
    ));
    // A PostgreSQL server of unknown encoding: unquoted non-ASCII is server dependent.
    let snap = snapshot(
        DbmsKind::Postgres,
        IdentCtx::new(DialectId::Postgres),
        vec![(
            ns_pg(DB, "public").table_oid("t", 1, &[]).finish(),
            loaded(Level::Structure),
        )],
        vec![],
        true,
    );
    let v = SnapshotView::new(snap).with_search_path(public_path());
    assert!(matches!(
        v.lookup(&p("çay"), KindGroup::Relations),
        Lookup::Unknown(UnknownReason::ServerDependent)
    ));
    assert!(matches!(
        v.lookup(&p("t"), KindGroup::Relations),
        Lookup::Found(_)
    ));
    // A probe context of another engine family than the namespaces: never a fact.
    let snap = snapshot(
        DbmsKind::Postgres,
        IdentCtx::new(DialectId::Generic),
        vec![(
            ns_pg(DB, "public").table_oid("t", 1, &[]).finish(),
            loaded(Level::Structure),
        )],
        vec![],
        true,
    );
    let v = SnapshotView::new(snap).with_search_path(public_path());
    assert!(matches!(
        v.lookup(&p("zz"), KindGroup::Relations),
        Lookup::Unknown(UnknownReason::ServerDependent)
    ));
}

#[test]
fn freshness_follows_the_namespace_state() {
    let v = view_with("public", &["t"]);
    assert!(matches!(
        v.freshness(&NsKey::schema([DB, "public"])),
        Freshness::Loaded { stale: false, .. }
    ));
    assert_eq!(
        v.freshness(&NsKey::schema([DB, "sales"])),
        Freshness::Unknown
    );
}
