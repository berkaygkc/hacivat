//! The HV-MODEL-03 budget (§3.3): a cached catalog of 160k columns loads in < 300 ms.
//!
//! Perf-gated: `HACIVAT_PERF=1 cargo test --release -p hacivat-meta --test perf -- --ignored
//! --nocapture`. "Load" is everything between a cold app start and a published snapshot the
//! explorer and completion can read: open the cache file, read every row, verify, decode,
//! validate the arena, rebuild the name indexes and publish one generation.

use std::time::{Duration, Instant};

use hacivat_meta::fixture::{DB, LARGE_SCHEMAS, large_pg_catalog, table_subtree};
use hacivat_meta::*;
use hacivat_proto::DbmsKind;

fn perf_enabled() -> bool {
    std::env::var_os("HACIVAT_PERF").is_some_and(|v| v == "1")
}

/// (first run, best run, last value).
fn best_of<T>(runs: usize, mut f: impl FnMut() -> T) -> (Duration, Duration, T) {
    let mut first = None;
    let mut best = Duration::MAX;
    let mut last = None;
    for _ in 0..runs {
        let t = Instant::now();
        let v = f();
        let e = t.elapsed();
        first.get_or_insert(e);
        best = best.min(e);
        last = Some(v);
    }
    (
        first.expect("at least one run"),
        best,
        last.expect("at least one run"),
    )
}

#[test]
#[ignore = "perf"]
fn cached_catalog_of_160k_columns_loads_under_300ms() {
    if !perf_enabled() {
        return;
    }
    let t = Instant::now();
    let nss = large_pg_catalog();
    let build = t.elapsed();
    let columns: usize = nss.iter().map(|n| n.column_count()).sum();
    let objects: usize = nss.iter().map(|n| n.len()).sum();
    assert_eq!(columns, 160_000);
    let tables: usize = LARGE_SCHEMAS.iter().map(|s| s.1).sum();
    assert_eq!(tables, 15_000);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("perf.v1.sqlite");
    let t = Instant::now();
    {
        let mut store = CacheStore::open(&path).unwrap().store;
        for ns in &nss {
            store.save_namespace(ns).unwrap();
        }
    }
    let save = t.elapsed();
    let payload_bytes: usize = nss.iter().map(|n| encode_namespace(n).len()).sum();
    let file_bytes = std::fs::metadata(&path).unwrap().len()
        + std::fs::metadata(dir.path().join("perf.v1.sqlite-wal")).map_or(0, |m| m.len());

    // Decode only (no SQLite), for the breakdown.
    let payloads: Vec<Vec<u8>> = nss.iter().map(encode_namespace).collect();
    let (_, decode, _) = best_of(5, || {
        payloads
            .iter()
            .map(|p| decode_namespace(p).unwrap())
            .collect::<Vec<_>>()
    });

    // The budgeted path: open + read + decode + index + publish.
    let (load_first, load, catalog) = best_of(5, || {
        let store = CacheStore::open(&path).unwrap().store;
        let all = store.load_all().unwrap();
        assert!(all.dropped.is_empty());
        let c = Catalog::new(DbmsKind::Postgres);
        c.publish(all.namespaces.into_iter().map(CatalogChange::Put))
            .unwrap();
        c
    });
    let snap = catalog.current();
    assert_eq!(snap.column_count(), 160_000);

    // Incremental: replace one table subtree in the 10,000-table namespace.
    let public = NsKey::schema([DB, "public"]);
    let parent = snap.namespace(&public).unwrap().object_key(0).unwrap();
    let (_, upsert, _) = best_of(5, || {
        catalog
            .publish([CatalogChange::UpsertSubtree {
                ns: public.clone(),
                parent: parent.clone(),
                subtree: table_subtree("public", "tbl_04242", 14, 900_000, None),
            }])
            .unwrap()
    });
    // Whole-namespace publish of an already-built arena (the cost a reader never pays).
    let other = snap.namespace(&public).unwrap().as_ref().clone();
    let (_, put, _) = best_of(5, || {
        catalog
            .publish([CatalogChange::Put(other.clone())])
            .unwrap()
    });

    println!(
        "hacivat-meta perf: {objects} objects, {columns} columns, {tables} tables, {} namespaces",
        nss.len()
    );
    println!(
        "  size_of::<Obj>() = {} bytes, size_of::<ColumnProps>() = {} bytes (+16 Arc header)",
        std::mem::size_of::<Obj>(),
        std::mem::size_of::<hacivat_meta::props::ColumnProps>()
    );
    println!(
        "  payload {:.2} MB (uncompressed), file {:.2} MB",
        payload_bytes as f64 / 1e6,
        file_bytes as f64 / 1e6
    );
    println!("  build (fixture) {build:?}, save {save:?}");
    println!("  decode only     {decode:?} (best of 5)");
    println!("  LOAD (budget)   {load:?} best of 5, {load_first:?} first run (budget 300 ms)");
    println!("  upsert 1 table in the 10k-table namespace {upsert:?}");
    println!("  put 1 namespace (clone + publish) {put:?}");
    assert!(
        load_first < Duration::from_millis(300),
        "cached catalog load took {load_first:?}"
    );
}
