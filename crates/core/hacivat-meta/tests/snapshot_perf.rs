//! HV-MODEL-02 performance (M4 Task 9; R-M4-2's trigger): a publish into a catalog of 5,000
//! namespaces, and lookups that hit.
//!
//! `HACIVAT_PERF=1 cargo test --release -p hacivat-meta --test snapshot_perf -- --ignored
//! --nocapture`.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use hacivat_meta::fixture::ns_pg;
use hacivat_meta::ids::NsKey;
use hacivat_meta::kind::KindGroup;
use hacivat_meta::name::{DialectId, IdentCtx, Name, NamePart};
use hacivat_meta::namespace::{Level, NamespaceSnapshot, Watermark};
use hacivat_meta::search_path::SearchPath;
use hacivat_meta::snapshot::{Catalog, CatalogChange, ContainerKey, LoadState, ServerFacts};
use hacivat_meta::view::{Lookup, ModelView, SnapshotView, slow_lookups_on_this_thread};
use hacivat_proto::DbmsKind;

fn perf_enabled() -> bool {
    std::env::var_os("HACIVAT_PERF").is_some_and(|v| v == "1")
}

fn ctx() -> IdentCtx {
    IdentCtx {
        encoding: Some(Arc::from("UTF8")),
        ..IdentCtx::new(DialectId::Postgres)
    }
}

fn schema(i: usize) -> NamespaceSnapshot {
    ns_pg("db", &format!("s{i:05}"))
        .table_oid("t", 20_000 + i as u32, &["a", "b"])
        .finish()
}

fn loaded() -> LoadState {
    LoadState::Loaded {
        level: Level::Structure,
        at: SystemTime::UNIX_EPOCH,
        watermark: Watermark::None,
        stale: false,
    }
}

fn p95(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[(v.len() * 95).div_ceil(100) - 1]
}

#[test]
#[ignore = "perf"]
fn publish_5000_namespaces_under_1ms() {
    if !perf_enabled() {
        return;
    }
    const NAMESPACES: usize = 5_000;
    const PUBLISHES: usize = 200;
    let catalog = Catalog::new(Arc::new(ServerFacts::new(DbmsKind::Postgres, ctx())));
    let w = catalog.publisher().unwrap();
    let mut changes = Vec::with_capacity(NAMESPACES * 2);
    for i in 0..NAMESPACES {
        let ns = schema(i);
        changes.push(CatalogChange::SetState(
            ContainerKey::Namespace(ns.key().clone()),
            loaded(),
        ));
        changes.push(CatalogChange::Put(Arc::new(ns)));
    }
    w.publish(changes).unwrap();
    // Index building is excluded from the timed publishes and reported on its own (the
    // spike's 2.6–3.4 ms "put one prebuilt namespace" included it).
    let t = Instant::now();
    let prebuilt: Vec<Arc<NamespaceSnapshot>> = (0..PUBLISHES)
        .map(|k| Arc::new(schema(k * (NAMESPACES / PUBLISHES))))
        .collect();
    let build_each = t.elapsed() / PUBLISHES as u32;
    // The real shape of an engine publish: the arena and its `Loaded` state, so both the
    // namespace map and the states map (5,000 entries each) are copied.
    let mut times = Vec::with_capacity(PUBLISHES);
    for ns in prebuilt {
        let state = CatalogChange::SetState(ContainerKey::Namespace(ns.key().clone()), loaded());
        let t = Instant::now();
        let p = w.publish(vec![CatalogChange::Put(ns), state]).unwrap();
        times.push(t.elapsed());
        assert_eq!(p.changed.len(), 1);
    }
    let snap = catalog.current();
    assert_eq!(snap.namespaces.len(), NAMESPACES);
    let p95 = p95(times.clone());
    let max = times.iter().max().copied().unwrap_or_default();
    println!(
        "publish into {NAMESPACES} namespaces: p95 {p95:?}, max {max:?} over {PUBLISHES} \
         publishes; building one prebuilt namespace: {build_each:?}"
    );
    assert!(
        p95 < Duration::from_millis(1),
        "p95 {p95:?} ≥ 1 ms: R-M4-2's trigger (two-level map first)"
    );
}

#[test]
#[ignore = "perf"]
fn lookup_hit_does_not_allocate() {
    if !perf_enabled() {
        return;
    }
    // `unsafe` is confined to the UI platform layer, so no counting allocator: the view
    // counts, per thread, every lookup that leaves the allocation-free hit path (a typed key
    // built by `match_key`, a namespace index built for the generation). A hit after the
    // warm-up must count none.
    //
    // **Limit:** the counter proves the instrumented fast path was taken, not that nothing
    // else allocates; an uninstrumented allocation added later would pass. The hit path is
    // `SnapshotView::lookup` → `qualify`/`key_text` (identity keys only) → `walk` →
    // `resolve::ns_status` (inline `NsKey` clones, `BTreeMap::get`) → `in_arena`
    // (`key_is_identity`, `agrees`, `NamespaceSnapshot::lookup`) → `CatalogSnapshot::handle`
    // (`Arc` clone, the generation's index): each must stay allocation-free on a hit, and a
    // change to any of them is checked by review, not by this test.
    let catalog = Catalog::new(Arc::new(ServerFacts::new(DbmsKind::Postgres, ctx())));
    let w = catalog.publisher().unwrap();
    let mut changes = Vec::new();
    for i in 0..200 {
        let ns = schema(i);
        changes.push(CatalogChange::SetState(
            ContainerKey::Namespace(ns.key().clone()),
            loaded(),
        ));
        changes.push(CatalogChange::Put(Arc::new(ns)));
    }
    w.publish(changes).unwrap();
    let view = SnapshotView::new(catalog.current()).with_search_path(SearchPath::exact(
        Some(Name::from("db")),
        [NsKey::schema(["db", "s00150"])],
    ));
    let unqualified = [NamePart {
        text: Name::from("t"),
        quoted: false,
    }];
    let qualified = [
        NamePart {
            text: Name::from("s00042"),
            quoted: false,
        },
        NamePart {
            text: Name::from("t"),
            quoted: true,
        },
    ];
    // Warm-up: the generation's namespace index is built once.
    assert!(matches!(
        view.lookup(&unqualified, KindGroup::Relations),
        Lookup::Found(_)
    ));
    let before = slow_lookups_on_this_thread();
    let t = Instant::now();
    for _ in 0..100_000 {
        assert!(matches!(
            view.lookup(&unqualified, KindGroup::Relations),
            Lookup::Found(_)
        ));
        assert!(matches!(
            view.lookup(&qualified, KindGroup::Relations),
            Lookup::Found(_)
        ));
    }
    let per = t.elapsed() / 200_000;
    println!("lookup hit: {per:?} each");
    assert_eq!(slow_lookups_on_this_thread(), before);
    // The counter is live: a probe that needs folding takes the slow path.
    view.lookup(
        &[NamePart {
            text: Name::from("T"),
            quoted: false,
        }],
        KindGroup::Relations,
    );
    assert_eq!(slow_lookups_on_this_thread(), before + 1);
}
