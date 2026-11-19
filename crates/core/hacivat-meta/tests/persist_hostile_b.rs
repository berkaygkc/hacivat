//! Security review B's hostile tests for M4 Task 11 (`task-m4-11-review-b-hostile.rs`),
//! committed in fix round 1, each finding test flipped to assert the fix.
//!
//! The review's counting global allocator is left out: it needs `unsafe`, which the workspace
//! allows only in `hacivat-ui-core/src/platform/**`. H1 and H2 assert the decoder's bounds
//! directly (refusal, and the expansion budget) instead of measuring peak allocation.

#![cfg(feature = "persist")]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

// ## m4-t11 (controller fix, CI 44600951505): wall-clock bounds are perf assertions, so they
// self-gate on HACIVAT_PERF=1 (test-timing rule); the structural checks (refused, TooLarge,
// nothing decoded) always run. CI debug runners took 4.4 s for a bound of 2 s.
fn perf_bound(elapsed: Duration, limit: Duration, what: &str) {
    if std::env::var_os("HACIVAT_PERF").is_some() {
        assert!(elapsed < limit, "{what}: {elapsed:?}");
    }
}

use hacivat_meta::codec::testing::encode_unchecked;
use hacivat_meta::codec::{
    DecodeError, EXPANSION_FACTOR, EXPANSION_SLACK, HEADER_LEN, MAX_NAME_BYTES, decode_namespace,
    encode_namespace, encode_ns_key, encode_watermark,
};
use hacivat_meta::fixture::{DB, ns_pg};
use hacivat_meta::ids::NsKey;
use hacivat_meta::kind::ObjectKind;
use hacivat_meta::name::{FoldRule, Name};
use hacivat_meta::namespace::{
    Level, NamespaceSnapshot, NsMeta, Obj, PgIdentity, Subtree, Watermark,
};
use hacivat_meta::persist::crypt::{OVERHEAD, padme};
use hacivat_meta::persist::{
    CATALOG_DOMAIN, CacheError, CacheStore, CryptError, Encryption, RebuildReason, RowCipher,
    RowError, RowKey, WriterId,
};
use hacivat_proto::DbmsKind;

const W: WriterId = WriterId(1);

/// A SQLite-style schema `main` with `n` tables, every one named the same `len`-byte string.
fn many_big_names(n: usize, len: usize) -> NamespaceSnapshot {
    let big: Name = Arc::from("a".repeat(len));
    let mut t = Subtree::with_capacity(Obj::new(ObjectKind::Schema, "main"), n + 1);
    let root = t.root();
    for _ in 0..n {
        t.add(root, Obj::new(ObjectKind::Table, big.clone()));
    }
    NamespaceSnapshot::build(
        NsKey::schema(["main"]),
        DbmsKind::Sqlite,
        FoldRule::AsciiCaseless,
        NsMeta::default(),
        t,
    )
    .unwrap()
}

/// Writes `payload` as `key`'s plaintext row behind the store's back.
fn plant_plain_row(path: &Path, key: &NsKey, payload: &[u8]) {
    let conn = rusqlite::Connection::open(path).unwrap();
    let k = encode_ns_key(key);
    conn.execute(
        "INSERT OR REPLACE INTO namespace(tag, ns_key, minor, generation, writer_id, epoch,
             watermark, payload)
         VALUES (?1, ?1, 0, 1, 1, 0, ?2, ?3)",
        rusqlite::params![k, encode_watermark(&Watermark::None), payload],
    )
    .unwrap();
}

/// H1 (flipped, fix round 1 B-I3). One interned 1 MiB name referenced by many 8-byte object
/// records: the encoder refuses to write it, and a crafted row of it is dropped fast as
/// malformed instead of costing O(objects × name length).
#[test]
fn h1_an_interned_big_name_fails_fast() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    // 250 objects: the review measured 2.3 s for this load before the fix (debug); building
    // the fixture itself hashes every name, so larger counts only slow the test down.
    let ns = many_big_names(250, 1 << 20);
    let mut s = CacheStore::open(&path, Encryption::None, W).unwrap().store;
    assert!(matches!(
        s.save_namespace(&ns),
        Err(CacheError::Encode(DecodeError::Malformed(_)))
    ));
    let bytes = encode_unchecked(&ns).unwrap();
    drop(ns);
    plant_plain_row(&path, &NsKey::schema(["main"]), &bytes);
    let t0 = Instant::now();
    let all = s.load_all().unwrap();
    let dt = t0.elapsed();
    assert!(all.namespaces.is_empty());
    assert!(
        matches!(
            all.dropped.as_slice(),
            [(_, RowError::Decode(DecodeError::Malformed(_)))]
        ),
        "{:?}",
        all.dropped
    );
    perf_bound(dt, Duration::from_secs(2), "load");
    // At the cap a name still round-trips; one byte over is refused both ways.
    let at_cap = many_big_names(2, MAX_NAME_BYTES);
    assert!(decode_namespace(&encode_namespace(&at_cap).unwrap()).is_ok());
    let over = many_big_names(2, MAX_NAME_BYTES + 1);
    assert!(encode_namespace(&over).is_err());
    assert!(decode_namespace(&encode_unchecked(&over).unwrap()).is_err());
}

/// H1b (fix round 1 B-I3). Names under the cap still cannot expand a small payload without
/// bound: the total string bytes one payload expands to are capped.
#[test]
fn h1b_total_string_expansion_is_capped() {
    let ns = many_big_names(20_000, MAX_NAME_BYTES);
    let bytes = encode_unchecked(&ns).unwrap();
    let body = bytes.len() - HEADER_LEN;
    let expanded = 20_000 * MAX_NAME_BYTES;
    assert!(expanded > body * EXPANSION_FACTOR + EXPANSION_SLACK);
    let t0 = Instant::now();
    assert_eq!(
        decode_namespace(&bytes).unwrap_err(),
        DecodeError::Malformed("strings expand past the budget")
    );
    perf_bound(t0.elapsed(), Duration::from_secs(2), "load");
}

/// H2 (flipped). The decoder's allocation stays linear: a 200k-minimal-object payload (the
/// densest the format allows) decodes, inside the expansion budget.
#[test]
fn h2_minimal_objects_decode_within_the_budget() {
    let mut t = Subtree::with_capacity(Obj::new(ObjectKind::Schema, "main"), 200_001);
    let root = t.root();
    for _ in 0..200_000 {
        t.add(root, Obj::new(ObjectKind::Table, "x"));
    }
    let ns = NamespaceSnapshot::build(
        NsKey::schema(["main"]),
        DbmsKind::Sqlite,
        FoldRule::AsciiCaseless,
        NsMeta::default(),
        t,
    )
    .unwrap();
    let bytes = encode_namespace(&ns).unwrap();
    let back = decode_namespace(&bytes).unwrap();
    assert_eq!(back.len(), 200_001);
}

fn pg_ns(table: &str, w: u64) -> NamespaceSnapshot {
    let mut t = Subtree::new(Obj::new(ObjectKind::Schema, "gizli"));
    let r = t.root();
    t.add(r, Obj::new(ObjectKind::Table, table));
    NamespaceSnapshot::build(
        NsKey::schema([DB, "gizli"]),
        DbmsKind::Postgres,
        FoldRule::Exact,
        NsMeta {
            level: Level::Structure,
            watermark: Watermark::PgXid {
                w,
                identity: PgIdentity {
                    system_identifier: 5,
                    start_time_micros: 6,
                },
            },
            loaded_at_ms: 1,
        },
        t,
    )
    .unwrap()
}

fn names(ns: &NamespaceSnapshot) -> Vec<String> {
    ns.objs().iter().map(|o| o.name().to_string()).collect()
}

/// H3 (flipped, fix round 1 B-M1). An older sealed row restored with forged bookkeeping
/// (generation, writer) fails authentication: the bookkeeping is bound into the payload's
/// associated data. (Restoring every column of an older row stays possible and is documented:
/// see review A's H5.)
#[test]
fn h3_rollback_with_forged_bookkeeping_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let key = || Encryption::Row(RowKey::from_bytes(&[9; 32]));
    let c = RowCipher::new(RowKey::from_bytes(&[9; 32]), CATALOG_DOMAIN).unwrap();
    let tag = c
        .tag(&encode_ns_key(&NsKey::schema([DB, "gizli"])))
        .to_vec();
    let mut s = CacheStore::open(&path, key(), W).unwrap().store;
    s.save_namespace(&pg_ns("old_table", 1)).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    type Row = (Vec<u8>, Vec<u8>, Vec<u8>);
    let old: Row = conn
        .query_row(
            "SELECT ns_key, watermark, payload FROM namespace WHERE tag = ?1",
            [&tag],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    s.save_namespace(&pg_ns("new_table", 2)).unwrap();
    drop(s);
    conn.execute(
        "UPDATE namespace SET ns_key = ?1, watermark = ?2, payload = ?3, generation = 999,
             writer_id = 4242 WHERE tag = ?4",
        rusqlite::params![old.0, old.1, old.2, tag],
    )
    .unwrap();
    drop(conn);
    let mut s = CacheStore::open(&path, key(), W).unwrap().store;
    let all = s.load_all().unwrap();
    assert!(all.namespaces.is_empty(), "the rolled-back row was served");
    assert_eq!(
        all.dropped,
        vec![(
            Some(NsKey::schema([DB, "gizli"])),
            RowError::Crypt(CryptError::Auth)
        )]
    );
}

/// H4 (flipped, fix round 1 B-I2). Ciphertext lengths show only padding buckets: every
/// namespace key seals to one length whatever the schema name, payloads and sources to Padmé
/// buckets, and no source length is stored in the clear.
#[test]
fn h4_lengths_show_only_padding_buckets() {
    fn profile(dir: &Path, name: &str, k: u8) -> Vec<(usize, usize)> {
        let path = dir.join(name);
        let mut s = CacheStore::open(&path, Encryption::Row(RowKey::from_bytes(&[k; 32])), W)
            .unwrap()
            .store;
        for (schema, tables) in [("public", 3usize), ("musteri_hesaplari_2024", 7)] {
            let mut b = ns_pg(DB, schema);
            for i in 0..tables {
                b = b.table(&format!("t{i}"), &["id", "ad"]);
            }
            s.save_namespace(&b.finish()).unwrap();
        }
        s.put_source("CREATE VIEW v AS SELECT secret_column FROM payroll")
            .unwrap();
        drop(s);
        let conn = rusqlite::Connection::open(&path).unwrap();
        let cols: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info('source')")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(cols, ["tag", "text"], "a clear source length column");
        let mut out: Vec<(usize, usize)> = conn
            .prepare("SELECT length(ns_key), length(payload) FROM namespace")
            .unwrap()
            .query_map([], |r| {
                Ok((r.get::<_, i64>(0)? as usize, r.get::<_, i64>(1)? as usize))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let src: i64 = conn
            .query_row("SELECT length(text) FROM source", [], |r| r.get(0))
            .unwrap();
        out.push((0, src as usize));
        out.sort();
        out
    }
    let dir = tempfile::tempdir().unwrap();
    let a = profile(dir.path(), "a.v1.sqlite", 1);
    let b = profile(dir.path(), "b.v1.sqlite", 2);
    // Same content, same buckets: what remains is the bucket, never a name length.
    assert_eq!(a, b);
    let keys: Vec<usize> = a.iter().filter(|r| r.0 > 0).map(|r| r.0).collect();
    assert_eq!(keys.len(), 2);
    assert!(keys.iter().all(|&k| k == 256 + OVERHEAD), "{keys:?}");
    for &(_, sealed) in &a {
        let padded = sealed - OVERHEAD;
        assert_eq!(padded, padme(padded), "{sealed} is not a bucket");
        assert!(padded >= 256);
    }
}

/// H5. Downgrade attempts. A plaintext file (attacker rows) put where an encrypted profile's
/// cache lives is scrubbed, never read; so is an encrypted file whose mode rows were deleted;
/// and (fix round 1, A-I1) an encrypted file opened without a key is refused, not rebuilt.
#[test]
fn h5_downgrade_to_plaintext_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    {
        let mut s = CacheStore::open(&path, Encryption::None, W).unwrap().store;
        s.save_namespace(&pg_ns("planted", 1)).unwrap();
    }
    let o = CacheStore::open(&path, Encryption::Row(RowKey::from_bytes(&[3; 32])), W).unwrap();
    assert_eq!(o.rebuilt, Some(RebuildReason::WrongKey));
    let mut s = o.store;
    assert!(s.load_all().unwrap().namespaces.is_empty());
    s.save_namespace(&pg_ns("real", 1)).unwrap();
    drop(s);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute(
        "DELETE FROM header WHERE key IN ('encryption', 'key_check')",
        [],
    )
    .unwrap();
    drop(conn);
    let o = CacheStore::open(&path, Encryption::Row(RowKey::from_bytes(&[3; 32])), W).unwrap();
    assert_eq!(o.rebuilt, Some(RebuildReason::WrongKey));
    drop(o);
    assert!(matches!(
        CacheStore::open(&path, Encryption::None, W),
        Err(CacheError::Disabled)
    ));
}

/// H6a (flipped, fix round 1 B-I1). A planted trigger makes the schema foreign: the file is
/// rotated and rebuilt, and the next save is really stored.
#[test]
fn h6a_planted_trigger_is_rebuilt_not_obeyed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let key = || Encryption::Row(RowKey::from_bytes(&[9; 32]));
    {
        let mut s = CacheStore::open(&path, key(), W).unwrap().store;
        s.save_namespace(&pg_ns("old_table", 1)).unwrap();
    }
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER pin BEFORE UPDATE ON namespace BEGIN SELECT RAISE(IGNORE); END;",
        )
        .unwrap();
    let o = CacheStore::open(&path, key(), W).unwrap();
    assert_eq!(o.rebuilt, Some(RebuildReason::Corrupt));
    let mut s = o.store;
    s.save_namespace(&pg_ns("new_table", 2)).unwrap();
    let mut s2 = CacheStore::open(&path, key(), W).unwrap().store;
    let all = s2.load_all().unwrap();
    assert_eq!(all.namespaces.len(), 1);
    assert!(names(&all.namespaces[0]).contains(&"new_table".to_string()));
    drop(s);
}

/// H6b (flipped). A table of the wrong shape is rotated and rebuilt once; the cache then works.
#[test]
fn h6b_wrong_shaped_table_is_rebuilt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE namespace(tag BLOB PRIMARY KEY, junk INTEGER);")
        .unwrap();
    for round in 0..2 {
        let o = CacheStore::open(&path, Encryption::None, W).unwrap();
        let expected = (round == 0).then_some(RebuildReason::Corrupt);
        assert_eq!(o.rebuilt, expected, "round {round}");
        let mut s = o.store;
        s.save_namespace(&pg_ns("t", 1)).unwrap();
        assert_eq!(s.load_all().unwrap().namespaces.len(), 1);
    }
    // Every other shape change is caught too: an extra column, a changed type, a view, an
    // index.
    for planted in [
        "ALTER TABLE usage ADD COLUMN extra INTEGER",
        "CREATE VIEW v AS SELECT 1",
        "CREATE INDEX i ON namespace(minor)",
    ] {
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch(planted)
            .unwrap();
        let o = CacheStore::open(&path, Encryption::None, W).unwrap();
        assert_eq!(o.rebuilt, Some(RebuildReason::Corrupt), "{planted}");
    }
}

/// H6c (flipped). A planted trigger with an endless recursive CTE is never run: the foreign
/// schema is rotated before any write, and the load returns.
#[test]
fn h6c_planted_trigger_cannot_hang_a_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    {
        let mut s = CacheStore::open(&path, Encryption::None, W).unwrap().store;
        s.save_namespace(&pg_ns("t", 1)).unwrap();
    }
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER spin AFTER INSERT ON usage BEGIN
               SELECT count(*) FROM (WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c)
                                     SELECT x FROM c);
             END;
             CREATE TRIGGER spin2 AFTER UPDATE ON usage BEGIN
               SELECT count(*) FROM (WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c)
                                     SELECT x FROM c);
             END;",
        )
        .unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let p = path.clone();
    std::thread::spawn(move || {
        let o = CacheStore::open(&p, Encryption::None, W).unwrap();
        let rebuilt = o.rebuilt;
        let mut s = o.store;
        let r = s.load_all().map(|a| a.namespaces.len());
        let _ = tx.send((rebuilt, format!("{r:?}")));
    });
    let got = rx.recv_timeout(Duration::from_secs(20));
    assert_eq!(
        got.ok(),
        Some((Some(RebuildReason::Corrupt), "Ok(0)".to_string()))
    );
}

/// H6d (flipped). A `format_major` of the wrong SQLite type is corrupt: rotated and rebuilt.
#[test]
fn h6d_wrong_typed_header_is_rebuilt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    drop(CacheStore::open(&path, Encryption::None, W).unwrap());
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE header SET value = X'01' WHERE key = 'format_major'",
            [],
        )
        .unwrap();
    let o = CacheStore::open(&path, Encryption::None, W).unwrap();
    assert_eq!(o.rebuilt, Some(RebuildReason::Corrupt));
    assert!(o.moved_to.is_some());
    let o = CacheStore::open(&path, Encryption::None, W).unwrap();
    assert!(o.rebuilt.is_none());
}

/// B-M3. A row larger than the per-profile cap is not read: dropped as too large.
#[test]
fn rows_over_the_read_cap_are_not_read() {
    use hacivat_meta::persist::CacheCaps;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let mut s = CacheStore::open(&path, Encryption::None, W).unwrap().store;
    s.save_namespace(&pg_ns("t", 1)).unwrap();
    s.set_caps(CacheCaps {
        per_profile: 64,
        global: 64,
    });
    let all = s.load_all().unwrap();
    assert!(all.namespaces.is_empty());
    assert_eq!(all.dropped, vec![(None, RowError::TooLarge)]);
}

// ---------------------------------------------------------------------------------------------
// Re-review of fix round 1 (`task-m4-11-review-b-rereview.rs`, R1–R7), committed in fix round
// 2 with each finding flipped.

fn rr_key() -> Encryption {
    Encryption::Row(RowKey::from_bytes(&[9; 32]))
}

fn attacker(path: &Path, sql: &str) {
    rusqlite::Connection::open(path)
        .unwrap()
        .execute_batch(sql)
        .unwrap();
}

/// R1 (flipped, fix round 2 N1). A trigger planted while the store is open is never obeyed:
/// the next transaction sees the schema cookie changed, rotates the file and rebuilds it; the
/// old row is never served again.
#[test]
fn r1_trigger_planted_after_open_is_never_obeyed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let mut s = CacheStore::open(&path, rr_key(), W).unwrap().store;
    s.save_namespace(&pg_ns("old_table", 1)).unwrap();
    attacker(
        &path,
        "CREATE TRIGGER pin BEFORE UPDATE ON namespace BEGIN SELECT RAISE(IGNORE); END;",
    );
    let r = s.save_namespace(&pg_ns("new_table", 2));
    let Err(CacheError::Corrupt { renamed_to }) = r else {
        panic!("the planted trigger was not caught: {r:?}");
    };
    assert!(renamed_to.exists());
    // The rebuilt store is empty (no pinned row) and works.
    assert!(
        s.load_namespace(&NsKey::schema([DB, "gizli"]))
            .unwrap()
            .is_none()
    );
    s.save_namespace(&pg_ns("new_table", 2)).unwrap();
    let got = s
        .load_namespace(&NsKey::schema([DB, "gizli"]))
        .unwrap()
        .unwrap();
    assert!(names(&got).contains(&"new_table".to_string()));
    // A fresh open sees an exact schema.
    assert!(
        CacheStore::open(&path, rr_key(), W)
            .unwrap()
            .rebuilt
            .is_none()
    );
}

/// R2 (flipped, fix round 2 N1). A hang trigger planted while the store is open: `load_all`
/// returns (the file is rotated), well within the timeout.
#[test]
fn r2_hang_trigger_planted_after_open_cannot_hang_a_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let mut s = CacheStore::open(&path, Encryption::None, W).unwrap().store;
    s.save_namespace(&pg_ns("t", 1)).unwrap();
    attacker(
        &path,
        "CREATE TRIGGER spin AFTER UPDATE ON usage BEGIN
           SELECT count(*) FROM (WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c)
                                 SELECT x FROM c);
         END;
         CREATE TRIGGER spin2 AFTER INSERT ON usage BEGIN
           SELECT count(*) FROM (WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c)
                                 SELECT x FROM c);
         END;",
    );
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let first = s.load_all().map(|a| a.namespaces.len());
        let second = s.load_all().map(|a| a.namespaces.len());
        let saved = s.save_namespace(&pg_ns("t", 2)).is_ok();
        let _ = tx.send((
            matches!(first, Err(CacheError::Corrupt { .. })),
            format!("{second:?}"),
            saved,
        ));
    });
    let got = rx.recv_timeout(Duration::from_secs(15));
    assert_eq!(got.ok(), Some((true, "Ok(0)".to_string(), true)));
}

/// R3. The `usage` and `source` autoindexes swapped through `writable_schema` (the
/// `sqlite_master` rows are unchanged): the file is rotated and rebuilt, and the rebuilt store
/// works.
#[test]
fn r3_swapped_autoindex_roots_are_rebuilt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    {
        let mut s = CacheStore::open(&path, Encryption::None, W).unwrap().store;
        s.save_namespace(&pg_ns("t", 1)).unwrap();
        s.put_source("CREATE VIEW v AS SELECT 1").unwrap();
        s.put_source("CREATE VIEW w AS SELECT 2").unwrap();
    }
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
            .unwrap();
        let root = |name: &str| -> i64 {
            c.query_row(
                "SELECT rootpage FROM sqlite_master WHERE name = ?1",
                [name],
                |r| r.get(0),
            )
            .unwrap()
        };
        let (ru, rs) = (
            root("sqlite_autoindex_usage_1"),
            root("sqlite_autoindex_source_1"),
        );
        c.execute_batch(&format!(
            "PRAGMA writable_schema=ON;
             UPDATE sqlite_master SET rootpage={rs} WHERE name='sqlite_autoindex_usage_1';
             UPDATE sqlite_master SET rootpage={ru} WHERE name='sqlite_autoindex_source_1';
             PRAGMA writable_schema=OFF;"
        ))
        .unwrap();
    }
    let o = CacheStore::open(&path, Encryption::None, W).unwrap();
    assert_eq!(o.rebuilt, Some(RebuildReason::Corrupt));
    let mut s = o.store;
    s.save_namespace(&pg_ns("t2", 2)).unwrap();
    s.put_source("CREATE VIEW z AS SELECT 3").unwrap();
    assert_eq!(s.load_all().unwrap().namespaces.len(), 1);
    drop(s);
    assert!(
        CacheStore::open(&path, Encryption::None, W)
            .unwrap()
            .rebuilt
            .is_none()
    );
}

/// R4 (flipped, fix round 2 N2). A 40 MB plaintext `ns_key` cell (and the same as a tag) is
/// never fetched or decoded: the row is dropped as too large, fast. A key cell that differs
/// from its tag is refused before it is decoded, and the key decoder caps the path.
#[test]
fn r4_huge_ns_key_cell_is_never_decoded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    drop(CacheStore::open(&path, Encryption::None, W).unwrap());
    let mut k = encode_ns_key(&NsKey::schema([DB, "gizli"]));
    k.resize(k.len() + 40_000_000, 0);
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute(
        "INSERT INTO namespace(tag, ns_key, minor, generation, writer_id, epoch, watermark,
             payload) VALUES (X'01', ?1, 0, 1, 1, 0, ?2, X'00')",
        rusqlite::params![k, encode_watermark(&Watermark::None)],
    )
    .unwrap();
    c.execute(
        "INSERT INTO namespace(tag, ns_key, minor, generation, writer_id, epoch, watermark,
             payload) VALUES (?1, ?1, 0, 1, 1, 0, ?2, X'00')",
        rusqlite::params![k, encode_watermark(&Watermark::None)],
    )
    .unwrap();
    drop(c);
    let mut s = CacheStore::open(&path, Encryption::None, W).unwrap().store;
    let t0 = Instant::now();
    let all = s.load_all().unwrap();
    let dt = t0.elapsed();
    assert!(all.namespaces.is_empty());
    assert_eq!(all.dropped, vec![(None, RowError::TooLarge); 2]);
    perf_bound(dt, Duration::from_secs(2), "r4");
    // The rows are gone; a second load and an eviction see nothing.
    assert!(s.load_all().unwrap().dropped.is_empty());
    assert!(s.evict(0, &|_: &NsKey| false).unwrap().evicted.is_empty());
    // A key cell that differs from its tag is refused before it is decoded.
    let mut long_path = vec![2u8];
    long_path.resize(1_001, 0);
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute(
        "INSERT INTO namespace(tag, ns_key, minor, generation, writer_id, epoch, watermark,
             payload) VALUES (X'02', ?1, 0, 1, 1, 0, ?2, X'00')",
        rusqlite::params![long_path, encode_watermark(&Watermark::None)],
    )
    .unwrap();
    drop(c);
    assert_eq!(
        s.load_all().unwrap().dropped,
        vec![(None, RowError::Mismatch)]
    );
    // The key decoder itself caps the path at 16 elements.
    assert!(hacivat_meta::codec::decode_ns_key(&long_path).is_err());
}

/// R5 (documented limitation). A whole older row restored with its own bookkeeping, under the
/// same server epoch, still loads: the file alone cannot tell it from the current row. ADR 0008
/// `m4-t11` records it with the mitigation: every cache load is published stale and validated
/// in the background.
#[test]
fn r5_whole_row_rollback_is_the_documented_limitation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let mut s = CacheStore::open(&path, rr_key(), W).unwrap().store;
    s.save_namespace(&pg_ns("old_table", 1)).unwrap();
    let snap = dir.path().join("snap.sqlite");
    drop(s);
    std::fs::copy(&path, &snap).unwrap();
    let _ = std::fs::copy(
        format!("{}-wal", path.display()),
        format!("{}-wal", snap.display()),
    );
    let mut s = CacheStore::open(&path, rr_key(), W).unwrap().store;
    s.save_namespace(&pg_ns("new_table", 2)).unwrap();
    drop(s);
    attacker(
        &path,
        &format!(
            "ATTACH '{}' AS old; DELETE FROM namespace;
             INSERT INTO namespace SELECT * FROM old.namespace;",
            snap.display()
        ),
    );
    let mut s = CacheStore::open(&path, rr_key(), W).unwrap().store;
    let all = s.load_all().unwrap();
    assert!(all.dropped.is_empty());
    assert!(names(&all.namespaces[0]).contains(&"old_table".to_string()));
    // Published stale, as every cache load is.
    assert!(matches!(
        hacivat_meta::persist::cached_state(&all.namespaces[0]),
        hacivat_meta::snapshot::LoadState::Loaded { stale: true, .. }
    ));
}

/// R6. Schema variants planted before open are each rotated.
#[test]
fn r6_schema_variants_are_rotated() {
    let variants = [
        ("view", "CREATE VIEW v AS SELECT * FROM namespace;"),
        ("stat1", "ANALYZE;"),
        ("index", "CREATE INDEX i ON namespace(minor);"),
        (
            "check",
            "DROP TABLE usage; CREATE TABLE usage(tag BLOB PRIMARY KEY, last_used INTEGER NOT NULL \
             CHECK (last_used > 0));",
        ),
        (
            "collate",
            "DROP TABLE header; CREATE TABLE header(key TEXT PRIMARY KEY COLLATE NOCASE, value \
             BLOB NOT NULL); INSERT INTO header VALUES ('format_major', 1);",
        ),
        (
            "generated",
            "DROP TABLE usage; CREATE TABLE usage(tag BLOB PRIMARY KEY, last_used INTEGER NOT \
             NULL, g AS (1/0));",
        ),
    ];
    for (what, sql) in variants {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.v1.sqlite");
        {
            let mut s = CacheStore::open(&path, Encryption::None, W).unwrap().store;
            s.save_namespace(&pg_ns("t", 1)).unwrap();
        }
        attacker(&path, sql);
        let r = CacheStore::open(&path, Encryption::None, W).map(|o| o.rebuilt);
        assert!(
            matches!(r, Ok(Some(RebuildReason::Corrupt))),
            "{what}: {r:?}"
        );
    }
}

/// R7. Names at the 4 KiB cap referenced by many objects, and 1 MiB names: both fail fast.
/// (Smaller than the review's 100k and 4,000 objects, with lower-case names: building the
/// fixture hashes, and for upper-case names folds, every name, which dominated the test.)
#[test]
fn r7_names_at_the_cap_fail_fast() {
    for (n, len) in [(40_000usize, 4096usize), (250, 1 << 20)] {
        let ns = many_big_names(n, len);
        let bytes = encode_unchecked(&ns).unwrap();
        drop(ns);
        let t0 = Instant::now();
        let r = decode_namespace(&bytes);
        assert!(r.is_err(), "n={n} len={len}");
        perf_bound(
            t0.elapsed(),
            Duration::from_secs(2),
            &format!("n={n} len={len}"),
        );
    }
}
