// ## m4-t29-g1-bench
//! G1's real fixture and the D-038 evidence (M4 Task 29; R-M4-21): `hv_m4_big` on `pg17`
//! (Task 16's fixture: 15,000 tables, 160,000 columns; shared, dropped at the gate) read cold at
//! `Names` and at `Structure`, the times printed with the D-038 thresholds, and the `Structure`
//! namespace persisted as a plaintext catalog cache copied out through
//! `hacivat_testkit::artifact`, so the G1 bench can load the real catalog too
//! (`catalog_cold --fixture <artifacts>/g1-hv_m4_big.v1.sqlite`).
//!
//! `#[ignore = "docker"]`, returns early unless `HACIVAT_DOCKER=1`. Run:
//! `<main>/target/docker/run-docker-tests.sh --set pr -p hacivat-introspect --test
//! docker_big_catalog`. The times are recorded, never asserted (a debug build on a shared
//! runner; the plan's G1 number is the bench's).

mod common;

use std::time::Instant;

use common::pg::{Pg, admin};
use hacivat_introspect::engine::{NsRole, Thresholds, auto_level, levels};
use hacivat_introspect::pack::Mode;
use hacivat_introspect::pg::pass::{self, PassOpts};
use hacivat_introspect::testing::big_catalog;
use hacivat_meta::merge::{MergeOutcome, MergePolicy, Trigger, base, merge};
use hacivat_meta::name::FoldRule;
use hacivat_meta::persist::{CacheStore, Encryption, WriterId};
use hacivat_meta::{Level, NamespaceSnapshot, NsKey, ObjectKind};
use hacivat_proto::DbmsKind;
use hacivat_session::meta::MetaConn;
use hacivat_testkit::{docker_enabled, require_target};
use tokio_util::sync::CancellationToken;

const BIG_TABLES: u32 = 15_000;
const BIG_COLUMNS: u32 = 160_000;
/// The artifact the bench can take as `--fixture`.
const ARTIFACT: &str = "g1-hv_m4_big.v1.sqlite";

/// A fresh full pass of `ns` at `level`, merged into an empty base; its time.
async fn cold(c: &mut MetaConn, ns: &NsKey, level: Level) -> (NamespaceSnapshot, u128) {
    let started = Instant::now();
    let p = pass::introspect(
        c,
        ns,
        PassOpts {
            mode: Mode::Full,
            level,
            prev: None,
            ..PassOpts::default()
        },
        &CancellationToken::new(),
    )
    .await
    .unwrap_or_else(|e| panic!("{ns:?} {level:?}: {e:?}"));
    let b = base(ns.clone(), DbmsKind::Postgres, FoldRule::Exact).unwrap();
    let policy = MergePolicy {
        trigger: Trigger::User,
        ..MergePolicy::default()
    };
    let next = match merge(Some(&b), p.result, &policy).expect("merge") {
        MergeOutcome::Applied { next, .. } | MergeOutcome::PendingConfirmation { next, .. } => next,
    };
    (next, started.elapsed().as_millis())
}

fn tables(ns: &NamespaceSnapshot) -> usize {
    ns.objs()
        .iter()
        .filter(|o| o.kind() == ObjectKind::Table)
        .count()
}

fn thresholds(t: Thresholds) -> String {
    format!(
        "Sources<={} Structure<={}",
        t.sources_max.map_or("never".to_string(), |s| s.to_string()),
        t.structure_max
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn big_catalog_cold_times_recorded_and_cache_artifact_written() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    // Task 16's fixture, created unless it is there (idempotent batches).
    {
        let mut a = admin(&t).await;
        big_catalog::ensure_pg(a.live(), BIG_TABLES, BIG_COLUMNS)
            .await
            .unwrap_or_else(|e| panic!("hv_m4_big: {e:?}"));
        a.close().await;
    }
    let ns = NsKey::schema([t.database.as_str(), big_catalog::SCHEMA]);
    // Fix round 1 (m-4): `Structure` first, each level on a fresh connection, so neither pass
    // runs on a backend whose catalog caches the other warmed. The server's shared buffers stay
    // warm (the fixture was just ensured): the HV-FACT line says so.
    let mut times = Vec::new();
    for level in [Level::Structure, Level::Names] {
        let pg = Pg::connect(&t).await;
        let mut c = pg.meta().await;
        times.push(cold(&mut c, &ns, level).await);
        drop(c);
        pg.shutdown().await;
    }
    let (names, names_ms) = times.pop().expect("Names");
    let (structure, structure_ms) = times.pop().expect("Structure");

    // The fixture is the stated size (review probe: "fixture smaller than stated").
    assert_eq!(tables(&names), BIG_TABLES as usize);
    assert_eq!(tables(&structure), BIG_TABLES as usize);
    assert_eq!(structure.column_count(), BIG_COLUMNS as usize);
    assert_eq!(names.column_count(), 0, "Names reads no columns");

    // D-038: 15,000 major objects is Structure as the current namespace, Names as any other.
    let major = BIG_TABLES;
    assert_eq!(auto_level(NsRole::Current, major), Level::Structure);
    assert_eq!(auto_level(NsRole::Other, major), Level::Names);

    // The cache the G1 bench loads, copied out through the artifact folder.
    let path = hacivat_testkit::artifact(ARTIFACT);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
    let started = Instant::now();
    {
        let mut store = CacheStore::open(&path, Encryption::None, WriterId(1))
            .expect("the artifact cache")
            .store;
        store.save_namespace(&structure).expect("saved");
    }
    let save_ms = started.elapsed().as_millis();
    let loaded = {
        let mut store = CacheStore::open(&path, Encryption::None, WriterId(1))
            .expect("the artifact cache")
            .store;
        store.load_all().expect("read back")
    };
    assert!(loaded.dropped.is_empty());
    assert_eq!(loaded.namespaces.len(), 1);
    assert_eq!(loaded.namespaces[0].column_count(), BIG_COLUMNS as usize);
    let bytes = std::fs::metadata(&path).map_or(0, |m| m.len());

    eprintln!(
        "HV-FACT g1 hv_m4_big pg17 {} tables {} columns (debug build; fresh connection per level, Structure first, server buffers warm): cold Structure {structure_ms} ms, cold Names {names_ms} ms; \
         D-038 current [{}] -> {:?}, other [{}] -> {:?}; cache saved in {save_ms} ms, {bytes} bytes at {}",
        BIG_TABLES,
        BIG_COLUMNS,
        thresholds(levels::CURRENT),
        auto_level(NsRole::Current, major),
        thresholds(levels::OTHER),
        auto_level(NsRole::Other, major),
        path.display()
    );
}
