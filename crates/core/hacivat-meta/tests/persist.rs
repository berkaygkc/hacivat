//! HV-MODEL-03 (M4 Task 11): the codec and the SQLite catalog cache — round trips, format
//! versioning, server identity, compare-and-set, eviction, files. alg/02 HV-MODEL-03 tests 2–5
//! and 8, edge cases 4–8, 11–13; the model spike's persist tests (spike origin `d86313b`).

#![cfg(feature = "persist")]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_meta::codec::testing::{
    decode_as_build, encode_as_minor, encode_with_foreign_kind, reseal_checksum,
};
use hacivat_meta::codec::{
    DecodeError, FORMAT_MAJOR, FORMAT_MINOR, HEADER_LEN, decode, decode_namespace,
    encode_namespace, payload_version,
};
use hacivat_meta::fixture::{DB, pg_catalog, table_subtree};
use hacivat_meta::ids::{NsKey, ObjectKey, ServerId, SourceRef};
use hacivat_meta::kind::ObjectKind;
use hacivat_meta::name::{FoldRule, Name};
use hacivat_meta::namespace::{
    Level, NamespaceSnapshot, NsMeta, Obj, ObjFlags, PgIdentity, Subtree, Watermark,
};
use hacivat_meta::persist::{
    CacheError, CacheStore, Encryption, IdentityCheck, LoadAll, RebuildReason, SaveDebouncer,
    ServerFingerprint, WriterId, cache_file_name, source_refs,
};
use hacivat_meta::snapshot::{CatalogChange, LoadState};
use hacivat_proto::{DbmsKind, ProfileId};
use proptest::prelude::*;

const W1: WriterId = WriterId(1);
const W2: WriterId = WriterId(2);

fn small() -> Vec<NamespaceSnapshot> {
    pg_catalog(&[("public", 300), ("sales", 40)], 3_500)
}

fn assert_same(a: &NamespaceSnapshot, b: &NamespaceSnapshot) {
    assert_eq!(a.key(), b.key());
    assert_eq!(a.dbms(), b.dbms());
    assert_eq!(a.fold(), b.fold());
    assert_eq!(a.meta(), b.meta());
    assert_eq!(a.objs(), b.objs());
}

fn open(path: &Path) -> CacheStore {
    CacheStore::open(path, Encryption::None, W1).unwrap().store
}

fn pg_wm(w: u64) -> Watermark {
    Watermark::PgXid {
        w,
        identity: PgIdentity {
            system_identifier: 7_301,
            start_time_micros: 1,
        },
    }
}

/// `app.<schema>` with one table `t` of `columns` columns, at watermark `w`, loaded at `at`.
fn schema_at(schema: &str, columns: usize, w: Watermark, at: i64) -> NamespaceSnapshot {
    let mut t = Subtree::new(Obj::new(ObjectKind::Schema, schema));
    let root = t.root();
    t.graft_under(root, table_subtree(schema, "t", columns, 16_400, None));
    NamespaceSnapshot::build(
        NsKey::schema([DB, schema]),
        DbmsKind::Postgres,
        FoldRule::Exact,
        NsMeta {
            level: Level::Structure,
            watermark: w,
            loaded_at_ms: at,
        },
        t,
    )
    .unwrap()
}

fn raw_payload(path: &Path, key: &NsKey) -> Vec<u8> {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.query_row(
        "SELECT payload FROM namespace WHERE tag = ?1",
        [hacivat_meta::codec::encode_ns_key(key)],
        |r| r.get(0),
    )
    .unwrap()
}

fn files_bytes(path: &Path) -> Vec<u8> {
    let mut all = Vec::new();
    for suffix in ["", "-wal", "-shm"] {
        let mut p = path.as_os_str().to_owned();
        p.push(suffix);
        if let Ok(b) = std::fs::read(PathBuf::from(p)) {
            all.extend_from_slice(&b);
        }
    }
    all
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

// ---------------------------------------------------------------------------------------------
// Codec.

#[test]
fn every_namespace_round_trips_through_the_codec() {
    for ns in small() {
        let bytes = encode_namespace(&ns).unwrap();
        assert_eq!(payload_version(&bytes), Some((FORMAT_MAJOR, FORMAT_MINOR)));
        let back = decode(&bytes).unwrap();
        assert_same(&ns, &back.ns);
        assert!(!back.from_newer_minor);
        assert_eq!(back.minor, FORMAT_MINOR);
        // Indexes are rebuilt identically.
        for (i, o) in ns.objs().iter().enumerate() {
            if let Some(g) = o.kind().group(DbmsKind::Postgres) {
                assert_eq!(ns.lookup(g, o.name()), back.ns.lookup(g, o.name()), "{i}");
            }
        }
        // Encoding is deterministic.
        assert_eq!(encode_namespace(&back.ns).unwrap(), bytes);
    }
}

#[test]
fn decoding_interns_repeated_strings() {
    let ns = &small()[2];
    let back = decode_namespace(&encode_namespace(ns).unwrap()).unwrap();
    let types: Vec<&Name> = back
        .objs()
        .iter()
        .filter_map(|o| o.props().as_column())
        .map(|c| &c.raw_type)
        .filter(|t| &***t == "bigint")
        .collect();
    assert!(types.len() > 10);
    assert!(types.windows(2).all(|w| Arc::ptr_eq(w[0], w[1])));
}

#[test]
fn newer_minor_trailing_fields_are_skipped() {
    let ns = schema_at("public", 4, pg_wm(9), 5);
    let bytes = encode_as_minor(&ns, FORMAT_MINOR + 1, &[0xAB, 0xCD, 0x01]).unwrap();
    let back = decode(&bytes).unwrap();
    assert!(back.from_newer_minor);
    assert_eq!(back.minor, FORMAT_MINOR + 1);
    assert_eq!(back.ns.objs(), ns.objs());
    // Loaded with no watermark: its next pass is full, never incremental on dropped fields.
    assert_eq!(back.ns.meta().watermark, Watermark::None);
    assert_eq!(back.ns.meta().level, ns.meta().level);
    // The same bytes are corruption to a build of that minor (it knows every field).
    assert!(matches!(
        decode_as_build(&bytes, FORMAT_MINOR + 1),
        Err(DecodeError::Malformed(_))
    ));
    // And an unpadded payload of our own minor with leftovers is refused too.
    let mut own = encode_namespace(&ns).unwrap();
    own.push(0);
    reseal_checksum(&mut own);
    assert!(decode(&own).is_err());
}

/// T3 review m6: an unknown kind code keeps the newer build's machine id as its raw kind, so a
/// user-data key that build wrote still finds the object here.
#[test]
fn an_unknown_kind_code_keeps_the_newer_builds_kind_id() {
    let ns = small()[1].clone(); // the database namespace, with extensions
    let bytes = encode_with_foreign_kind(&ns, ObjectKind::Extension, 999, "pg-future").unwrap();
    let back = decode_namespace(&bytes).unwrap();
    let (idx, obj) = back
        .objs()
        .iter()
        .enumerate()
        .find(|(_, o)| &**o.name() == "pgcrypto")
        .unwrap();
    assert_eq!(obj.kind(), ObjectKind::Unknown);
    assert_eq!(obj.raw_kind().map(|r| &**r), Some("pg-future"));
    let key = back.object_key(idx as u32).unwrap();
    assert_eq!(key.raw_kind.as_deref(), Some("pg-future"));
    // A key a newer build wrote (its own kind id; serde reads it as `Unknown` plus that raw
    // id, T3) finds the object.
    let theirs = ObjectKey::unknown(key.path.clone(), None, Some(Name::from("pg-future")));
    assert_eq!(back.find(&theirs), Some(idx as u32));
    // Another unknown kind never matches.
    let other = ObjectKey::unknown(key.path.clone(), None, Some(Name::from("pg-other")));
    assert_eq!(back.find(&other), None);
    // Our own unknown objects keep their server text as before.
    let mut t = Subtree::new(Obj::new(ObjectKind::Schema, "s"));
    t.add(
        t.root(),
        Obj::new(ObjectKind::Unknown, "x").with_props(hacivat_meta::props::Props::Generic(
            Arc::new(hacivat_meta::props::GenericProps {
                raw_kind: Some(Name::from("q")),
                ..Default::default()
            }),
        )),
    );
    let ns = NamespaceSnapshot::build(
        NsKey::schema([DB, "s"]),
        DbmsKind::Postgres,
        FoldRule::Exact,
        NsMeta::default(),
        t,
    )
    .unwrap();
    let back = decode_namespace(&encode_namespace(&ns).unwrap()).unwrap();
    assert_eq!(back.objs()[1].raw_kind().map(|r| &**r), Some("q"));
}

/// The spike's robustness test, kept: every truncation is an error; every single-byte
/// corruption is never a panic; in the body it is an error when the checksum is left as
/// written (the header's version bytes are outside the checksum by format: a flipped minor
/// reads as a newer minor, which is never rewritten); with a recomputed checksum it either
/// fails or decodes to a namespace that re-encodes cleanly.
#[test]
fn every_truncation_and_every_single_byte_corruption_with_recomputed_checksum_is_an_error_not_a_panic()
 {
    let ns = schema_at("public", 3, pg_wm(1), 2);
    let bytes = encode_namespace(&ns).unwrap();
    for cut in 0..bytes.len() {
        assert!(decode_namespace(&bytes[..cut]).is_err(), "cut at {cut}");
    }
    for cut in HEADER_LEN..bytes.len() {
        let mut b = bytes[..cut].to_vec();
        reseal_checksum(&mut b);
        assert!(decode_namespace(&b).is_err(), "body cut at {cut}");
    }
    for at in 0..bytes.len() {
        for flip in [0x01u8, 0x80, 0xFF] {
            let mut b = bytes.clone();
            b[at] ^= flip;
            let unsealed = decode_namespace(&b);
            if !(4..HEADER_LEN).contains(&at) || at == 8 {
                assert!(unsealed.is_err(), "unsealed flip at {at}");
            }
            reseal_checksum(&mut b);
            if let Ok(ns) = decode_namespace(&b) {
                let again = encode_namespace(&ns).unwrap();
                assert_eq!(decode_namespace(&again).unwrap().objs(), ns.objs());
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn decoding_arbitrary_bytes_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        let _ = decode(&bytes);
        let _ = payload_version(&bytes);
        let _ = hacivat_meta::codec::decode_ns_key(&bytes);
        let _ = hacivat_meta::codec::decode_watermark(&bytes);
        // The same bytes behind a valid header and checksum reach the body decoder.
        let mut framed = b"HVNS".to_vec();
        framed.extend_from_slice(&FORMAT_MAJOR.to_le_bytes());
        framed.extend_from_slice(&FORMAT_MINOR.to_le_bytes());
        framed.push(0);
        framed.extend_from_slice(&[0; 8]);
        framed.extend_from_slice(&bytes);
        reseal_checksum(&mut framed);
        let _ = decode(&framed);
    }

    #[test]
    fn mutated_payloads_never_panic(edits in proptest::collection::vec((any::<usize>(), any::<u8>()), 1..8)) {
        let ns = schema_at("public", 3, pg_wm(1), 2);
        let mut b = encode_namespace(&ns).unwrap();
        for (at, v) in edits {
            let i = HEADER_LEN + at % (b.len() - HEADER_LEN);
            b[i] = v;
        }
        reseal_checksum(&mut b);
        let _ = decode(&b);
    }
}

#[test]
fn a_corrupt_count_cannot_allocate_hugely() {
    // A body claiming u32::MAX strings in a 10-byte payload: refused before any allocation.
    let mut framed = b"HVNS".to_vec();
    framed.extend_from_slice(&FORMAT_MAJOR.to_le_bytes());
    framed.extend_from_slice(&FORMAT_MINOR.to_le_bytes());
    framed.push(0);
    framed.extend_from_slice(&[0; 8]);
    framed.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0, 0, 0, 0, 0]);
    reseal_checksum(&mut framed);
    assert_eq!(decode(&framed).unwrap_err(), DecodeError::Truncated);
}

// ---------------------------------------------------------------------------------------------
// The store.

#[test]
fn save_then_load_round_trips_a_namespace() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let nss = small();
    {
        let mut store = open(&path);
        for ns in &nss {
            store.save_namespace(ns).unwrap();
        }
        // Saving again replaces the row.
        store.save_namespace(&nss[2]).unwrap();
        assert_eq!(store.row_info(nss[2].key()).unwrap().unwrap().generation, 2);
    }
    let opened = CacheStore::open(&path, Encryption::None, W1).unwrap();
    assert!(opened.rebuilt.is_none());
    let mut store = opened.store;
    let all = store.load_all().unwrap();
    assert!(all.dropped.is_empty());
    assert!(all.from_newer_minor.is_empty());
    assert_eq!(all.namespaces.len(), nss.len());
    for ns in &nss {
        let got = all.namespaces.iter().find(|n| n.key() == ns.key()).unwrap();
        assert_same(ns, got);
    }
    let one = store
        .load_namespace(&NsKey::schema([DB, "sales"]))
        .unwrap()
        .unwrap();
    assert_same(&one, &nss[3]);
    assert!(
        store
            .load_namespace(&NsKey::schema([DB, "nope"]))
            .unwrap()
            .is_none()
    );
    // Removing a namespace.
    assert!(store.remove_namespace(nss[3].key()).unwrap());
    assert!(!store.remove_namespace(nss[3].key()).unwrap());
    assert!(store.load_namespace(nss[3].key()).unwrap().is_none());
}

/// alg/02 step 3: a cache load publishes every namespace `Loaded { stale: true }`.
#[test]
fn a_cache_load_publishes_every_namespace_stale() {
    let all = LoadAll {
        namespaces: small(),
        ..Default::default()
    };
    let n = all.namespaces.len();
    let changes = all.into_changes();
    assert_eq!(changes.len(), 2 * n);
    let mut puts = 0;
    for c in &changes {
        match c {
            CatalogChange::SetState(_, LoadState::Loaded { stale, level, .. }) => {
                assert!(*stale);
                assert_eq!(*level, Level::Structure);
            }
            CatalogChange::Put(_) => puts += 1,
            other => panic!("unexpected change {other:?}"),
        }
    }
    assert_eq!(puts, n);
}

/// alg/02 test 2: a build of minor 1 reads a minor-0 row (missing fields default), migrates it
/// and saves it as minor 1.
#[test]
fn older_minor_is_migrated_then_saved_as_current() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let ns = schema_at("public", 5, pg_wm(10), 1);
    open(&path).save_namespace(&ns).unwrap();
    let mut newer_build = CacheStore::open_as_minor(&path, Encryption::None, W1, FORMAT_MINOR + 1)
        .unwrap()
        .store;
    let all = newer_build.load_all().unwrap();
    assert!(all.from_newer_minor.is_empty());
    assert_same(&all.namespaces[0], &ns);
    newer_build.save_namespace(&all.namespaces[0]).unwrap();
    let info = newer_build.row_info(ns.key()).unwrap().unwrap();
    assert_eq!(info.minor, FORMAT_MINOR + 1);
    assert_eq!(info.generation, 2);
}

/// alg/02 test 3 and edge 4: a row from a newer minor is read, never written back as read;
/// a fresh introspection replaces it with our own minor.
#[test]
fn newer_minor_is_read_and_never_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let ns = schema_at("public", 5, pg_wm(10), 1);
    CacheStore::open_as_minor(&path, Encryption::None, W2, FORMAT_MINOR + 1)
        .unwrap()
        .store
        .save_namespace(&ns)
        .unwrap();
    let before = raw_payload(&path, ns.key());
    let mut store = open(&path);
    let all = store.load_all().unwrap();
    assert_eq!(all.from_newer_minor, vec![ns.key().clone()]);
    let read = &all.namespaces[0];
    assert_eq!(read.objs(), ns.objs());
    assert_eq!(read.meta().watermark, Watermark::None);
    assert!(matches!(
        store.save_namespace(read),
        Err(CacheError::NewerMinor)
    ));
    // Also through a single load.
    let one = store.load_namespace(ns.key()).unwrap().unwrap();
    assert!(matches!(
        store.save_namespace(&one),
        Err(CacheError::NewerMinor)
    ));
    assert_eq!(raw_payload(&path, ns.key()), before);
    assert_eq!(
        store.row_info(ns.key()).unwrap().unwrap().minor,
        FORMAT_MINOR + 1
    );
    // A fresh introspection (new watermark and load time) is saved with our minor.
    let fresh = schema_at("public", 6, pg_wm(11), 2);
    store.save_namespace(&fresh).unwrap();
    assert_eq!(
        store.row_info(ns.key()).unwrap().unwrap().minor,
        FORMAT_MINOR
    );
}

/// alg/02 test 4 and edge 6: another major lives in another file, untouched; a hand-copied
/// file of another major at our name is moved aside, not read.
#[test]
fn new_major_uses_a_new_file_and_leaves_the_old_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let profile = ProfileId::new();
    let ours = dir.path().join(cache_file_name(&profile));
    assert!(
        ours.to_string_lossy()
            .ends_with(&format!(".v{FORMAT_MAJOR}.sqlite"))
    );
    // Another major's file of the same profile.
    let theirs = dir
        .path()
        .join(format!("{}.v{}.sqlite", profile.0, FORMAT_MAJOR + 1));
    {
        let c = rusqlite::Connection::open(&theirs).unwrap();
        c.execute_batch(
            "CREATE TABLE header(key TEXT PRIMARY KEY, value BLOB NOT NULL);
             INSERT INTO header VALUES ('format_major', 2);",
        )
        .unwrap();
    }
    let theirs_bytes = std::fs::read(&theirs).unwrap();
    let mut store = open(&ours);
    for ns in small() {
        store.save_namespace(&ns).unwrap();
    }
    assert_eq!(store.load_all().unwrap().namespaces.len(), 4);
    drop(store);
    assert_eq!(std::fs::read(&theirs).unwrap(), theirs_bytes);

    // That file copied over ours: moved aside intact, and ours starts empty.
    std::fs::remove_file(&ours).unwrap();
    for s in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{s}", ours.display()));
    }
    std::fs::copy(&theirs, &ours).unwrap();
    let opened = CacheStore::open(&ours, Encryption::None, W1).unwrap();
    assert_eq!(opened.rebuilt, Some(RebuildReason::MajorChanged));
    let moved = opened.moved_to.clone().unwrap();
    assert_eq!(std::fs::read(&moved).unwrap(), theirs_bytes);
    let mut store = opened.store;
    assert!(store.load_all().unwrap().namespaces.is_empty());
}

/// alg/02 test 5: another server marks every row stale (loads with no watermark, so the next
/// pass is full) and keeps the rows for offline use.
#[test]
fn server_change_marks_all_stale_and_keeps_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let nss = small();
    let mut store = open(&path);
    for ns in &nss {
        store.save_namespace(ns).unwrap();
    }
    let a = ServerFingerprint(0xA);
    let b = ServerFingerprint(0xB);
    assert_eq!(store.check_identity(a).unwrap(), IdentityCheck::Same);
    assert_eq!(store.check_identity(a).unwrap(), IdentityCheck::Same);
    assert!(
        nss.iter()
            .all(|n| !store.row_info(n.key()).unwrap().unwrap().stale)
    );
    assert_eq!(store.check_identity(b).unwrap(), IdentityCheck::Changed);
    assert!(
        nss.iter()
            .all(|n| store.row_info(n.key()).unwrap().unwrap().stale)
    );
    let all = store.load_all().unwrap();
    assert_eq!(all.namespaces.len(), nss.len());
    for ns in &nss {
        let got = all.namespaces.iter().find(|n| n.key() == ns.key()).unwrap();
        assert_eq!(got.objs(), ns.objs(), "rows kept");
        assert_eq!(got.meta().watermark, Watermark::None, "next pass is full");
    }
    // A fresh save clears the mark; the server is now the recorded one.
    store.save_namespace(&nss[2]).unwrap();
    assert!(!store.row_info(nss[2].key()).unwrap().unwrap().stale);
    assert_eq!(
        store.load_namespace(nss[2].key()).unwrap().unwrap().meta(),
        nss[2].meta()
    );
    drop(store);
    assert_eq!(open(&path).check_identity(b).unwrap(), IdentityCheck::Same);
}

/// alg/02 step 5: two stores on one file; the save with the older watermark loses.
#[test]
fn older_writer_never_clobbers_newer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let mut one = CacheStore::open(&path, Encryption::None, W1).unwrap().store;
    let mut two = CacheStore::open(&path, Encryption::None, W2).unwrap().store;
    let newer = schema_at("public", 4, pg_wm(200), 2);
    let older = schema_at("public", 3, pg_wm(100), 1);
    two.save_namespace(&newer).unwrap();
    assert!(matches!(
        one.save_namespace(&older),
        Err(CacheError::Superseded)
    ));
    let info = one.row_info(newer.key()).unwrap().unwrap();
    assert_eq!((info.writer, info.generation), (W2, 1));
    assert_same(&one.load_namespace(newer.key()).unwrap().unwrap(), &newer);
    // A namespace with no watermark never replaces one that has it.
    let unknown = schema_at("public", 3, Watermark::None, 9);
    assert!(matches!(
        one.save_namespace(&unknown),
        Err(CacheError::Superseded)
    ));
    // Equal or newer wins.
    let newest = schema_at("public", 5, pg_wm(300), 3);
    one.save_namespace(&newest).unwrap();
    let info = two.row_info(newer.key()).unwrap().unwrap();
    assert_eq!((info.writer, info.generation), (W1, 2));
    // Another cluster is not comparable: its save wins.
    let elsewhere = schema_at(
        "public",
        2,
        Watermark::PgXid {
            w: 5,
            identity: PgIdentity {
                system_identifier: 99,
                start_time_micros: 0,
            },
        },
        4,
    );
    two.save_namespace(&elsewhere).unwrap();
    assert_same(
        &one.load_namespace(newer.key()).unwrap().unwrap(),
        &elsewhere,
    );
}

#[test]
fn temp_namespaces_are_never_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(&dir.path().join("p.v1.sqlite"));
    let t = Subtree::new(Obj::new(ObjectKind::Schema, "temp").add_flags(ObjFlags::TEMP));
    let ns = NamespaceSnapshot::build(
        NsKey::schema(["temp"]),
        DbmsKind::Sqlite,
        FoldRule::AsciiCaseless,
        NsMeta::default(),
        t,
    )
    .unwrap();
    assert!(matches!(
        store.save_namespace(&ns),
        Err(CacheError::TempNamespace)
    ));
    assert!(store.load_all().unwrap().namespaces.is_empty());
    // Nor queued for a save.
    let mut queue = SaveDebouncer::default();
    queue.offer(Arc::new(ns), Instant::now());
    assert!(queue.is_empty());
}

#[test]
fn saves_are_debounced_and_coalesced_per_namespace() {
    let mut queue = SaveDebouncer::default();
    let t0 = Instant::now();
    let a1 = Arc::new(schema_at("a", 1, pg_wm(1), 1));
    let a2 = Arc::new(schema_at("a", 2, pg_wm(2), 2));
    let b = Arc::new(schema_at("b", 1, pg_wm(1), 1));
    queue.offer(a1, t0);
    queue.offer(b.clone(), t0 + Duration::from_millis(500));
    queue.offer(a2.clone(), t0 + Duration::from_millis(900));
    assert_eq!(queue.len(), 2);
    assert!(queue.take_due(t0 + Duration::from_millis(999)).is_empty());
    assert_eq!(queue.next_due(), Some(t0 + Duration::from_secs(1)));
    // `a` is due one second after its first offer, with its latest snapshot.
    let due = queue.take_due(t0 + Duration::from_secs(1));
    assert_eq!(due.len(), 1);
    assert!(Arc::ptr_eq(&due[0], &a2));
    let due = queue.take_due(t0 + Duration::from_millis(1_500));
    assert!(Arc::ptr_eq(&due[0], &b));
    assert!(queue.is_empty());
    queue.offer(b.clone(), t0);
    queue.cancel(b.key());
    assert!(queue.take_all().is_empty());
}

/// R-M4-25 / alg/02 step 7: least recently loaded first; the protected (current database)
/// namespaces never go.
#[test]
fn eviction_is_lru_and_never_evicts_the_current_database() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(&dir.path().join("p.v1.sqlite"));
    let ns = |db: &str, schema: &str| {
        let mut t = Subtree::new(Obj::new(ObjectKind::Schema, schema));
        let root = t.root();
        t.graft_under(root, table_subtree(schema, "t", 40, 16_400, None));
        NamespaceSnapshot::build(
            NsKey::schema([db, schema]),
            DbmsKind::Postgres,
            FoldRule::Exact,
            NsMeta::default(),
            t,
        )
        .unwrap()
    };
    let current = ns("cur", "x");
    let (a, b, c) = (ns("old", "a"), ns("old", "b"), ns("old", "c"));
    for n in [&current, &a, &b, &c] {
        store.save_namespace(n).unwrap();
    }
    // Last use: current (save), c (save), b (load), a (load).
    store.load_namespace(b.key()).unwrap().unwrap();
    store.load_namespace(a.key()).unwrap().unwrap();
    let protect = |k: &NsKey| &*k.path[0] == "cur";
    let total = store.total_bytes().unwrap();
    let under = store.evict(total, &protect).unwrap();
    assert!(under.evicted.is_empty());
    let out = store.evict(total - 1, &protect).unwrap();
    assert_eq!(out.evicted, vec![Some(c.key().clone())]);
    assert!(out.bytes_after < out.bytes_before);
    assert!(out.bytes_after < total);
    let out = store.evict(0, &protect).unwrap();
    assert_eq!(
        out.evicted,
        vec![Some(b.key().clone()), Some(a.key().clone())]
    );
    let left = store.load_all().unwrap();
    assert_eq!(left.namespaces.len(), 1);
    assert_eq!(left.namespaces[0].key(), current.key());
}

/// Edge 8: a deleted profile's files go, every major, sidecar and rotation; nothing else does,
/// and a link is removed without touching its target.
#[test]
fn profile_deletion_deletes_its_cache_files() {
    let dir = tempfile::tempdir().unwrap();
    let (p, q) = (ProfileId::new(), ProfileId::new());
    let mine = dir.path().join(cache_file_name(&p));
    let other = dir.path().join(cache_file_name(&q));
    {
        let mut s = open(&mine);
        s.save_namespace(&small()[2]).unwrap();
        open(&other).save_namespace(&small()[3]).unwrap();
        std::fs::write(
            dir.path().join(format!("{}.v1.sqlite.corrupt-1", p.0)),
            b"x",
        )
        .unwrap();
        std::fs::write(dir.path().join(format!("{}.v2.sqlite", p.0)), b"x").unwrap();
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            let target = outside.path().join("keep.txt");
            std::fs::write(&target, b"keep").unwrap();
            let link = dir.path().join(format!("{}.v3.sqlite", p.0));
            std::os::unix::fs::symlink(&target, &link).unwrap();
            drop(s);
            CacheStore::delete_profile_files(dir.path(), &p).unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), b"keep");
            assert!(std::fs::symlink_metadata(&link).is_err());
        }
    }
    CacheStore::delete_profile_files(dir.path(), &p).unwrap();
    let left: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(!left.is_empty());
    assert!(
        left.iter().all(|n| n.starts_with(&q.0.to_string())),
        "{left:?}"
    );
    // A missing directory is not an error.
    CacheStore::delete_profile_files(&dir.path().join("none"), &p).unwrap();
}

#[test]
fn the_global_cap_deletes_the_least_recently_used_profiles() {
    use hacivat_meta::persist::{CacheCaps, evict_global};
    let dir = tempfile::tempdir().unwrap();
    let ids: Vec<ProfileId> = (0..3).map(|_| ProfileId::new()).collect();
    for (i, p) in ids.iter().enumerate() {
        let path = dir.path().join(cache_file_name(p));
        let mut s = open(&path);
        s.save_namespace(&small()[2]).unwrap();
        drop(s);
        // Distinct, ordered modification times.
        let t = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000 + i as u64 * 10);
        for entry in std::fs::read_dir(dir.path()).unwrap() {
            let e = entry.unwrap();
            if e.file_name()
                .to_string_lossy()
                .starts_with(&p.0.to_string())
            {
                std::fs::File::options()
                    .write(true)
                    .open(e.path())
                    .unwrap()
                    .set_modified(t)
                    .unwrap();
            }
        }
    }
    let size_of = |p: &ProfileId| -> u64 {
        std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(&p.0.to_string())
            })
            .map(|e| e.metadata().unwrap().len())
            .sum()
    };
    let total: u64 = ids.iter().map(size_of).sum();
    let caps = CacheCaps {
        per_profile: 1,
        global: total - 1,
    };
    // The oldest (ids[0]) is kept by the caller: the next oldest goes.
    let deleted = evict_global(dir.path(), caps, &[ids[0]]).unwrap();
    assert!(!deleted.is_empty());
    assert_eq!(size_of(&ids[1]), 0);
    assert!(size_of(&ids[0]) > 0 && size_of(&ids[2]) > 0);
}

#[cfg(unix)]
#[test]
fn file_mode_0600_and_directory_0700() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let cat = dir.path().join("data").join("catalog");
    let path = cat.join("p.v1.sqlite");
    let mut store = open(&path);
    store.save_namespace(&small()[0]).unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&cat), 0o700);
    assert_eq!(mode(&dir.path().join("data")), 0o700);
    for s in ["", "-wal", "-shm"] {
        let p = PathBuf::from(format!("{}{s}", path.display()));
        assert_eq!(mode(&p), 0o600, "{s}");
    }
    // A looser existing file is tightened on open.
    drop(store);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let _store = open(&path);
    assert_eq!(mode(&path), 0o600);
}

#[cfg(unix)]
#[test]
fn a_cache_path_that_is_a_symlink_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("elsewhere");
    std::fs::write(&target, b"keep").unwrap();
    let path = dir.path().join("p.v1.sqlite");
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(CacheStore::open(&path, Encryption::None, W1).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"keep");
    // A -wal link too.
    let path = dir.path().join("q.v1.sqlite");
    std::os::unix::fs::symlink(&target, format!("{}-wal", path.display())).unwrap();
    assert!(CacheStore::open(&path, Encryption::None, W1).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"keep");
}

/// alg/02 test 8: secrets never enter the cache; the key material never reaches the file or
/// a `Debug`.
#[test]
fn password_never_in_cache_file() {
    let dir = tempfile::tempdir().unwrap();
    for (name, enc) in [
        ("plain.v1.sqlite", Encryption::None),
        (
            "sealed.v1.sqlite",
            Encryption::Row(hacivat_meta::persist::RowKey::from_bytes(
                *b"hv_test_pw-hv_test_pw-hv_test_pw",
            )),
        ),
    ] {
        let path = dir.path().join(name);
        let opened = CacheStore::open(&path, enc, W1).unwrap();
        let shown = format!("{opened:?}");
        let mut store = opened.store;
        for ns in small() {
            store.save_namespace(&ns).unwrap();
        }
        store.put_source("SELECT 1").unwrap();
        store.check_identity(ServerFingerprint(7)).unwrap();
        store.load_all().unwrap();
        let shown = format!("{shown}{store:?}");
        assert!(!shown.contains("hv_test_pw"), "{name}: Debug");
        let bytes = files_bytes(&path);
        assert!(!bytes.is_empty());
        assert!(!contains(&bytes, b"hv_test_pw"), "{name}: file");
    }
}

#[test]
fn corrupt_file_is_renamed_and_rebuilt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    std::fs::write(&path, vec![0x5Au8; 8192]).unwrap();
    std::fs::write(format!("{}-wal", path.display()), b"old log").unwrap();
    let opened = CacheStore::open(&path, Encryption::None, W1).unwrap();
    assert_eq!(opened.rebuilt, Some(RebuildReason::Corrupt));
    let moved = opened.moved_to.clone().unwrap();
    assert!(moved.to_string_lossy().contains(".corrupt-"));
    assert_eq!(std::fs::read(&moved).unwrap(), vec![0x5Au8; 8192]);
    // The old log never sits beside the fresh file (moved with it, or discarded by SQLite's
    // own open attempt).
    let beside = std::fs::read(format!("{}-wal", path.display())).unwrap_or_default();
    assert!(!contains(&beside, b"old log"));
    let mut store = opened.store;
    assert!(store.load_all().unwrap().namespaces.is_empty());
    store.save_namespace(&small()[0]).unwrap();
    drop(store);
    // A second corruption in the same second takes the next free name.
    std::fs::remove_file(&path).unwrap();
    for s in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{s}", path.display()));
    }
    std::fs::write(&path, vec![0x5Au8; 4096]).unwrap();
    let again = CacheStore::open(&path, Encryption::None, W1).unwrap();
    let second = again.moved_to.unwrap();
    assert_ne!(second, moved);
    assert!(moved.exists() && second.exists());
}

#[test]
fn sources_are_content_addressed_and_collected() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(&dir.path().join("p.v1.sqlite"));
    let text = "SELECT 1 AS \"İ\"";
    let a = store.put_source(text).unwrap();
    let b = store.put_source(text).unwrap();
    assert_eq!(a, b);
    assert_eq!(a, SourceRef::of(text));
    assert_eq!(store.source(a).unwrap().as_deref(), Some(text));
    let other = store.put_source("SELECT 2").unwrap();
    let live: HashSet<SourceRef> = [a].into_iter().collect();
    assert_eq!(store.gc_sources(&live).unwrap(), 1);
    assert!(store.source(other).unwrap().is_none());
    assert!(store.source(a).unwrap().is_some());
    // The live set of a namespace: its views, routines and triggers.
    let refs = source_refs(&small()[2]);
    assert!(!refs.is_empty());
}

#[test]
fn the_file_name_carries_the_format_major() {
    let p = ProfileId::new();
    let name = cache_file_name(&p);
    assert!(name.ends_with(&format!(".v{FORMAT_MAJOR}.sqlite")));
    assert!(name.starts_with(&p.0.to_string()));
}

#[test]
fn server_id_kinds_and_state_marks_round_trip() {
    use hacivat_meta::ids::StateMark;
    let mut t = Subtree::new(Obj::new(ObjectKind::Schema, "s"));
    let root = t.root();
    let ids = [
        ServerId::Oid(1),
        ServerId::ObjectId(-2),
        ServerId::ObjNum(u64::MAX),
        ServerId::Uuid([7; 16]),
        ServerId::RowId(i64::MIN),
    ];
    let states = [
        StateMark::Xmin(3),
        StateMark::Modified(-4),
        StateMark::Created(5),
        StateMark::SchemaVersion(6),
        StateMark::Hash(u64::MAX),
    ];
    for (i, (id, st)) in ids.iter().zip(states).enumerate() {
        t.add(
            root,
            Obj::new(ObjectKind::Table, format!("t{i}").as_str())
                .with_server_id(*id)
                .with_state(st),
        );
    }
    t.add(
        root,
        Obj::new(ObjectKind::Table, "m").with_state(StateMark::MyTable {
            created: 1,
            trigger_max: -1,
            trigger_count: 3,
        }),
    );
    let ns = NamespaceSnapshot::build(
        NsKey::schema(["s"]),
        DbmsKind::MySql,
        FoldRule::AsciiCaseless,
        NsMeta {
            level: Level::Sources,
            watermark: Watermark::MyTime {
                at_utc_micros: 9,
                server_uuid: Name::from("u-1"),
                version: Name::from("8.4.3"),
                priv_hash: 11,
            },
            loaded_at_ms: -1,
        },
        t,
    )
    .unwrap();
    assert_same(
        &ns,
        &decode_namespace(&encode_namespace(&ns).unwrap()).unwrap(),
    );
}
