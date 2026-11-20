// ## m4-t29-g1-bench
//! The G1 bench fixture (M4 Task 29; spec §3.3 "Cached catalog load (160k columns) < 300 ms",
//! HV-MODEL-03): 15,000 tables, 160,000 columns, 15,000 primary keys, 5,000 foreign keys and
//! 20,000 indexes in **one** schema (`app.public`), plus the server and database namespaces.
//! The shape is Task 11's `persist_perf` generator (`fixture::table_subtree`, a second index on
//! every third table, a foreign key on every third table) with all tables in one schema, the
//! worst case for the explorer: one namespace whose 15,000 tables sit under one kind group.
//! The generator is deterministic (no random source), so every run writes the same catalog.
//!
//! The bench (`hacivat-app` example `catalog_cold`) reads the cache file this test writes:
//!
//! ```text
//! HACIVAT_G1_FIXTURE=$PWD/target/bench/g1-160k.v1.sqlite \
//!   cargo test --release -p hacivat-meta --test bench_fixture -- --ignored write_g1_fixture
//! ```

#![cfg(feature = "persist")]

use std::path::Path;
use std::sync::Arc;

use hacivat_meta::fixture::{DB, pg_catalog, table_subtree};
use hacivat_meta::ids::{NsKey, ServerId};
use hacivat_meta::kind::ObjectKind;
use hacivat_meta::name::FoldRule;
use hacivat_meta::namespace::{Level, NamespaceSnapshot, NsMeta, Obj, Subtree};
use hacivat_meta::persist::{CacheStore, Encryption, WriterId};
use hacivat_meta::props::{IndexProps, KeyColumn, KeyProps, Props};
use hacivat_proto::DbmsKind;

const SCHEMA: &str = "public";
const TABLES: usize = 15_000;
const COLUMNS: usize = 160_000;
const PKS: usize = 15_000;
const FKS: usize = 5_000;
const INDEXES: usize = 20_000;

// ## m4-t33c-facts (named exception)
/// Prints one `HV-FACT …` line (`format!` arguments) straight to the process's stdout, on one
/// line: libtest captures only the `print!` family, so the line reaches the CI log of a passing
/// test too (R-W3-14).
macro_rules! fact {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let line = format!($($arg)*).replace('\n', " ");
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }};
}

/// The G1 catalog: the server and database namespaces of the shared generator, then one schema
/// with every table.
fn g1_catalog() -> Vec<NamespaceSnapshot> {
    // No schema: the server and the database namespaces only.
    let mut out = pg_catalog(&[], 0);
    let base = COLUMNS / TABLES;
    let extra = COLUMNS % TABLES;
    let mut oid = 16_384u32;
    let mut t = Subtree::with_capacity(Obj::new(ObjectKind::Schema, SCHEMA), TABLES * 18);
    let root = t.root();
    let mut prev: Option<String> = None;
    for i in 0..TABLES {
        let name = format!("tbl_{i:05}");
        let columns = base + usize::from(i < extra);
        // Every third table (5,000): to the previous table, or to itself when it is the first.
        let fk = (i % 3 == 2).then(|| prev.as_deref().unwrap_or(&name));
        let node = t.graft_under(root, table_subtree(SCHEMA, &name, columns, oid, fk));
        if i.is_multiple_of(3) {
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
        prev = Some(name);
    }
    out.push(
        NamespaceSnapshot::build(
            NsKey::schema([DB, SCHEMA]),
            DbmsKind::Postgres,
            FoldRule::Exact,
            NsMeta {
                level: Level::Structure,
                ..Default::default()
            },
            t,
        )
        .expect("a valid generated namespace"),
    );
    out
}

fn count(nss: &[NamespaceSnapshot], kind: ObjectKind) -> usize {
    nss.iter()
        .map(|n| n.objs().iter().filter(|o| o.kind() == kind).count())
        .sum()
}

/// The stated shape, exactly (review probe: "fixture smaller than stated").
fn assert_shape(nss: &[NamespaceSnapshot]) {
    assert_eq!(count(nss, ObjectKind::Table), TABLES);
    assert_eq!(count(nss, ObjectKind::Column), COLUMNS);
    assert_eq!(count(nss, ObjectKind::PrimaryKey), PKS);
    assert_eq!(count(nss, ObjectKind::ForeignKey), FKS);
    assert_eq!(count(nss, ObjectKind::Index), INDEXES);
    // All of them in one schema namespace.
    let schemas: Vec<&NamespaceSnapshot> = nss
        .iter()
        .filter(|n| n.key() == &NsKey::schema([DB, SCHEMA]))
        .collect();
    assert_eq!(schemas.len(), 1);
    assert_eq!(schemas[0].column_count(), COLUMNS);
    assert_eq!(
        schemas[0]
            .objs()
            .iter()
            .filter(|o| o.kind() == ObjectKind::Table)
            .count(),
        TABLES
    );
}

#[test]
fn bench_fixture_is_15k_tables_160k_columns() {
    let a = g1_catalog();
    assert_shape(&a);
    // Deterministic: a second build is the same catalog, object for object.
    let b = g1_catalog();
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(x.key(), y.key());
        assert_eq!(x.len(), y.len());
        assert!(
            x.objs()
                .iter()
                .zip(y.objs())
                .all(|(p, q)| p.kind() == q.kind() && p.name() == q.name())
        );
    }
    // ## m4-t33c-facts: the shape just asserted, as the G1 fixture's record.
    fact!(
        "HV-FACT g1 bench-fixture schema={DB}.{SCHEMA} namespaces={} tables={} columns={} \
         primary_keys={} foreign_keys={} indexes={} deterministic=yes",
        a.len(),
        count(&a, ObjectKind::Table),
        count(&a, ObjectKind::Column),
        count(&a, ObjectKind::PrimaryKey),
        count(&a, ObjectKind::ForeignKey),
        count(&a, ObjectKind::Index),
    );
}

/// Writes `nss` as a plaintext cache at `path` (any older file there is replaced).
fn write_cache(path: &Path, nss: &[NamespaceSnapshot]) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    let mut store = CacheStore::open(path, Encryption::None, WriterId(1))
        .unwrap()
        .store;
    for ns in nss {
        store.save_namespace(ns).unwrap();
    }
}

/// The fixture writer (not a check): `HACIVAT_G1_FIXTURE=<file>` names the cache file to write.
/// The file is read back and its shape checked before the test passes.
#[test]
#[ignore = "bench fixture writer"]
fn write_g1_fixture() {
    let Some(path) = std::env::var_os("HACIVAT_G1_FIXTURE") else {
        eprintln!("write_g1_fixture: set HACIVAT_G1_FIXTURE=<cache file> to write the fixture");
        return;
    };
    let path = std::path::PathBuf::from(path);
    let nss = g1_catalog();
    assert_shape(&nss);
    write_cache(&path, &nss);
    let mut store = CacheStore::open(&path, Encryption::None, WriterId(1))
        .unwrap()
        .store;
    let all = store.load_all().unwrap();
    assert!(all.dropped.is_empty());
    assert_shape(&all.namespaces);
    println!("write_g1_fixture: wrote {}", path.display());
}
