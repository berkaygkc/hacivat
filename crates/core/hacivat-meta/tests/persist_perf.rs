//! The HV-MODEL-03 load budget (M4 Task 11; the first measurement of the 300 ms G1 exit, C-3).
//!
//! Perf-gated: `HACIVAT_PERF=1 cargo test --release -p hacivat-meta --test persist_perf --
//! --ignored --nocapture`. Synthetic catalog: 15,000 tables, 160,000 columns, 15,000 primary
//! keys, 5,000 foreign keys, 20,000 indexes, in one PostgreSQL database over five schemas (one
//! of 10,000 tables). "Load" is open the file, read every row, verify, decode, validate the
//! arena, rebuild the indexes and publish one generation; p95 of 20 runs ≤ 200 ms plaintext on
//! the CI runner (a relative signal; the authoritative 300 ms end to end is Task 29). The
//! encrypted variant is printed, not asserted.

#![cfg(feature = "persist")]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_meta::codec::{decode_namespace, encode_namespace};
use hacivat_meta::fixture::{DB, LARGE_SCHEMAS, table_subtree};
use hacivat_meta::ids::{NsKey, ServerId};
use hacivat_meta::kind::ObjectKind;
use hacivat_meta::name::{DialectId, FoldRule, IdentCtx};
use hacivat_meta::namespace::{Level, NamespaceSnapshot, NsMeta, Obj, Subtree};
use hacivat_meta::persist::{CacheStore, Encryption, LoadAll, RowKey, WriterId};
use hacivat_meta::props::{IndexProps, KeyColumn, KeyProps, Props};
use hacivat_meta::snapshot::{Catalog, CatalogChange, ServerFacts};
use hacivat_proto::DbmsKind;

const TABLES: usize = 15_000;
const COLUMNS: usize = 160_000;
const RUNS: usize = 20;

fn perf_enabled() -> bool {
    std::env::var_os("HACIVAT_PERF").is_some_and(|v| v == "1")
}

/// The plan's shape: `table_subtree` gives each table its columns, a primary key, one index
/// and (every third table) a foreign key; every third table gets a second index.
fn catalog() -> Vec<NamespaceSnapshot> {
    let base = COLUMNS / TABLES;
    let extra = COLUMNS % TABLES;
    let mut global = 0usize;
    let mut oid = 16_384u32;
    let mut out = Vec::new();
    for &(schema, count) in &LARGE_SCHEMAS {
        let mut t = Subtree::with_capacity(Obj::new(ObjectKind::Schema, schema), count * 18);
        let root = t.root();
        let mut prev: Option<String> = None;
        for i in 0..count {
            let name = format!("tbl_{i:05}");
            let columns = base + usize::from(global < extra);
            // Every third table of the whole catalog (5,000): to the previous table of its
            // schema, or to itself when it is the schema's first.
            let fk = (global % 3 == 2).then(|| prev.as_deref().unwrap_or(&name));
            let node = t.graft_under(root, table_subtree(schema, &name, columns, oid, fk));
            if global.is_multiple_of(3) {
                t.add(
                    node,
                    Obj::new(ObjectKind::Index, format!("{name}_status_idx").as_str())
                        .with_server_id(ServerId::Oid(oid + 3))
                        .with_props(Props::Index(Arc::new(IndexProps {
                            key: KeyProps {
                                columns: vec![KeyColumn {
                                    ordinal: 5,
                                    name: Some(Arc::from("status")),
                                }],
                                desc: vec![false],
                                enforced: true,
                                ..Default::default()
                            },
                            method: Some(Arc::from("btree")),
                            ..Default::default()
                        }))),
                );
            }
            oid += 4;
            global += 1;
            prev = Some(name);
        }
        out.push(
            NamespaceSnapshot::build(
                NsKey::schema([DB, schema]),
                DbmsKind::Postgres,
                FoldRule::Exact,
                NsMeta {
                    level: Level::Structure,
                    ..Default::default()
                },
                t,
            )
            .unwrap(),
        );
    }
    out
}

fn count(nss: &[NamespaceSnapshot], kind: ObjectKind) -> usize {
    nss.iter()
        .map(|n| n.objs().iter().filter(|o| o.kind() == kind).count())
        .sum()
}

fn p95(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[(v.len() * 95).div_ceil(100) - 1]
}

/// Publishes a cache load. `Catalog::publish` is Task 9's (a stub on this base); until it
/// lands, the stand-in is what R-M4-2's publish costs: the namespace map rebuilt with `Arc`s.
fn publish(all: LoadAll) -> (usize, &'static str) {
    let catalog = Catalog::new(Arc::new(ServerFacts::new(
        DbmsKind::Postgres,
        IdentCtx::new(DialectId::Postgres),
    )));
    let changes = all.into_changes();
    let n = changes.len();
    if let Some(publisher) = catalog.publisher()
        && publisher.publish(changes.clone()).is_ok()
    {
        return (n, "Catalog::publish");
    }
    let mut map: BTreeMap<NsKey, Arc<NamespaceSnapshot>> = BTreeMap::new();
    for c in changes {
        if let CatalogChange::Put(ns) = c {
            map.insert(ns.key().clone(), ns);
        }
    }
    let map = Arc::new(map);
    (map.len(), "map stand-in (Task 9 publish is a stub)")
}

/// `RUNS` cold opens of the cache at `path`, each loaded and published.
fn load_runs(path: &Path, enc: &Encryption) -> (Vec<Duration>, &'static str) {
    let mut times = Vec::with_capacity(RUNS);
    let mut how = "";
    for _ in 0..RUNS {
        let t = Instant::now();
        let mut store = CacheStore::open(path, enc.clone(), WriterId(1))
            .unwrap()
            .store;
        let all = store.load_all().unwrap();
        assert!(all.dropped.is_empty());
        assert_eq!(all.namespaces.len(), LARGE_SCHEMAS.len());
        let (_, h) = publish(all);
        times.push(t.elapsed());
        how = h;
    }
    (times, how)
}

#[test]
#[ignore = "perf"]
fn load_160k_column_namespace_under_budget() {
    if !perf_enabled() {
        return;
    }
    let t = Instant::now();
    let nss = catalog();
    let build = t.elapsed();
    assert_eq!(count(&nss, ObjectKind::Table), TABLES);
    assert_eq!(count(&nss, ObjectKind::Column), COLUMNS);
    assert_eq!(count(&nss, ObjectKind::PrimaryKey), 15_000);
    assert_eq!(count(&nss, ObjectKind::ForeignKey), 5_000);
    assert_eq!(count(&nss, ObjectKind::Index), 20_000);
    let objects: usize = nss.iter().map(NamespaceSnapshot::len).sum();

    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("plain.v1.sqlite");
    let sealed = dir.path().join("sealed.v1.sqlite");
    let key = Encryption::Row(RowKey::from_bytes(&[42; 32]));
    let t = Instant::now();
    {
        let mut store = CacheStore::open(&plain, Encryption::None, WriterId(1))
            .unwrap()
            .store;
        for ns in &nss {
            store.save_namespace(ns).unwrap();
        }
    }
    let save = t.elapsed();
    let t = Instant::now();
    {
        let mut store = CacheStore::open(&sealed, key.clone(), WriterId(1))
            .unwrap()
            .store;
        for ns in &nss {
            store.save_namespace(ns).unwrap();
        }
    }
    let save_sealed = t.elapsed();
    let payloads: Vec<Vec<u8>> = nss.iter().map(|n| encode_namespace(n).unwrap()).collect();
    let payload_bytes: usize = payloads.iter().map(Vec::len).sum();
    let file_bytes = |p: &Path| -> u64 {
        ["", "-wal"]
            .iter()
            .map(|s| std::fs::metadata(format!("{}{s}", p.display())).map_or(0, |m| m.len()))
            .sum()
    };

    // Decode only (no SQLite), for the breakdown.
    let mut decode = Vec::new();
    for _ in 0..RUNS {
        let t = Instant::now();
        let v: Vec<_> = payloads
            .iter()
            .map(|p| decode_namespace(p).unwrap())
            .collect();
        decode.push(t.elapsed());
        assert_eq!(v.len(), nss.len());
    }

    let (plain_runs, how) = load_runs(&plain, &Encryption::None);
    let (sealed_runs, _) = load_runs(&sealed, &key);
    let (p95_plain, p95_sealed, p95_decode) = (
        p95(plain_runs.clone()),
        p95(sealed_runs.clone()),
        p95(decode.clone()),
    );

    println!(
        "hacivat-meta persist_perf: {objects} objects, {COLUMNS} columns, {TABLES} tables, \
         15,000 PKs, 5,000 FKs, 20,000 indexes, {} namespaces",
        nss.len()
    );
    println!(
        "  payload {:.2} MB, file plain {:.2} MB, sealed {:.2} MB",
        payload_bytes as f64 / 1e6,
        file_bytes(&plain) as f64 / 1e6,
        file_bytes(&sealed) as f64 / 1e6
    );
    println!("  build (fixture) {build:?}, save plain {save:?}, save sealed {save_sealed:?}");
    println!(
        "  decode only        p95 {p95_decode:?}, min {:?}",
        decode.iter().min().unwrap()
    );
    println!(
        "  LOAD plaintext     p95 {p95_plain:?}, min {:?}, first {:?} (budget 200 ms; publish: {how})",
        plain_runs.iter().min().unwrap(),
        plain_runs[0]
    );
    println!(
        "  LOAD encrypted     p95 {p95_sealed:?}, min {:?}, first {:?} (printed, not asserted)",
        sealed_runs.iter().min().unwrap(),
        sealed_runs[0]
    );
    assert!(
        p95_plain <= Duration::from_millis(200),
        "cached catalog load p95 {p95_plain:?} over 200 ms"
    );
}
