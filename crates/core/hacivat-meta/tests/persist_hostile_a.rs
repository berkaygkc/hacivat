//! Security review A's hostile tests for M4 Task 11 (`task-m4-11-review-a-hostile.rs`),
//! committed in fix round 1. Each finding test is flipped to assert the fix; the probes that
//! confirmed a defence are kept as they were.

#![cfg(feature = "persist")]

use std::path::{Path, PathBuf};

use hacivat_meta::codec::{
    FORMAT_MAJOR, FORMAT_MINOR, decode, decode_ns_key, decode_watermark, encode_ns_key,
    reseal_checksum,
};
use hacivat_meta::fixture::{DB, pg_catalog, table_subtree};
use hacivat_meta::ids::NsKey;
use hacivat_meta::kind::ObjectKind;
use hacivat_meta::name::FoldRule;
use hacivat_meta::namespace::{
    Level, NamespaceSnapshot, NsMeta, Obj, PgIdentity, Subtree, Watermark,
};
use hacivat_meta::persist::crypt::OVERHEAD;
use hacivat_meta::persist::{
    CATALOG_DOMAIN, CacheError, CacheStore, CryptError, Encryption, IdentityCheck, RebuildReason,
    RowCipher, RowError, RowKey, ServerFingerprint, WriterId, cache_file_name,
};
use hacivat_proto::{DbmsKind, ProfileId};

const W1: WriterId = WriterId(1);

fn key(b: u8) -> RowKey {
    RowKey::from_bytes(&[b; 32])
}

fn files_bytes(path: &Path) -> Vec<u8> {
    let mut all = Vec::new();
    for s in ["", "-wal", "-shm"] {
        if let Ok(b) = std::fs::read(PathBuf::from(format!("{}{s}", path.display()))) {
            all.extend_from_slice(&b);
        }
    }
    all
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn schema(name: &str, table: &str, w: u64) -> NamespaceSnapshot {
    let mut t = Subtree::new(Obj::new(ObjectKind::Schema, name));
    let root = t.root();
    t.graft_under(root, table_subtree(name, table, 4, 90_000, None));
    NamespaceSnapshot::build(
        NsKey::schema([DB, name]),
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
            loaded_at_ms: 8,
        },
        t,
    )
    .unwrap()
}

fn has_table(ns: &NamespaceSnapshot, name: &str) -> bool {
    ns.objs()
        .iter()
        .any(|o| o.kind() == ObjectKind::Table && &**o.name() == name)
}

fn remove_family(path: &Path) {
    for s in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{s}", path.display()));
    }
}

/// H1. Downgrade: an attacker replaces a sensitive profile's encrypted file with a plaintext
/// file of their own content. Opening with the key must refuse (scrub), never load it.
#[test]
fn h1_planted_plaintext_file_is_scrubbed_not_loaded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    {
        let mut s = CacheStore::open(&path, Encryption::Row(key(1)), W1)
            .unwrap()
            .store;
        s.save_namespace(&schema("gizli", "real_table", 10))
            .unwrap();
    }
    remove_family(&path);
    {
        let mut s = CacheStore::open(&path, Encryption::None, W1).unwrap().store;
        s.save_namespace(&schema("gizli", "planted_evil", 999))
            .unwrap();
    }
    let opened = CacheStore::open(&path, Encryption::Row(key(1)), W1).unwrap();
    assert_eq!(opened.rebuilt, Some(RebuildReason::WrongKey));
    let mut s = opened.store;
    let all = s.load_all().unwrap();
    assert!(all.namespaces.is_empty());
    assert!(!contains(&files_bytes(&path), b"planted_evil"));
    // Variant: the 'encryption' header row removed but rows left: still refused.
    drop(s);
    remove_family(&path);
    {
        let mut s = CacheStore::open(&path, Encryption::None, W1).unwrap().store;
        s.save_namespace(&schema("gizli", "planted_evil", 999))
            .unwrap();
    }
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute("DELETE FROM header WHERE key = 'encryption'", [])
            .unwrap();
    }
    let opened = CacheStore::open(&path, Encryption::Row(key(1)), W1).unwrap();
    assert_eq!(opened.rebuilt, Some(RebuildReason::WrongKey));
    let mut s = opened.store;
    assert!(s.load_all().unwrap().namespaces.is_empty());
}

/// H2. A plaintext row injected into a legitimate encrypted store (valid key check kept), under
/// a copied real tag and under a fresh tag: never decoded, dropped as an auth failure.
#[test]
fn h2_plaintext_row_injected_into_encrypted_store_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let real = schema("gizli", "real_table", 10);
    {
        let mut s = CacheStore::open(&path, Encryption::Row(key(2)), W1)
            .unwrap()
            .store;
        s.save_namespace(&real).unwrap();
    }
    let evil = schema("gizli", "planted_evil", 999);
    let evil_payload = hacivat_meta::codec::encode_namespace(&evil).unwrap();
    let k = encode_ns_key(evil.key());
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        // Over the real row (tag kept, columns replaced with plaintext).
        c.execute(
            "UPDATE namespace SET ns_key = ?1, payload = ?2",
            rusqlite::params![k, evil_payload],
        )
        .unwrap();
        // And a new row under an arbitrary tag.
        c.execute(
            "INSERT INTO namespace(tag, ns_key, minor, generation, writer_id, epoch, watermark,
                 payload)
             VALUES (?1, ?2, 0, 1, 1, 0, x'00', ?3)",
            rusqlite::params![vec![0xAAu8; 16], k, evil_payload],
        )
        .unwrap();
    }
    let mut s = CacheStore::open(&path, Encryption::Row(key(2)), W1)
        .unwrap()
        .store;
    let all = s.load_all().unwrap();
    assert!(all.namespaces.is_empty(), "a plaintext row was decoded");
    assert_eq!(all.dropped.len(), 2);
    assert!(
        all.dropped.iter().all(|d| matches!(
            d,
            (
                None,
                RowError::Crypt(CryptError::Auth | CryptError::Truncated)
            )
        )),
        "{:?}",
        all.dropped
    );
    assert!(s.load_namespace(evil.key()).unwrap().is_none());
}

/// H3. A whole row (ns_key, watermark, payload) relabelled under another row's tag: the sealed
/// key column is bound to its tag, so both rows fail.
#[test]
fn h3_whole_rows_swapped_between_tags_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let (a, b) = (schema("alpha", "ta", 1), schema("beta", "tb", 1));
    {
        let mut s = CacheStore::open(&path, Encryption::Row(key(3)), W1)
            .unwrap()
            .store;
        s.save_namespace(&a).unwrap();
        s.save_namespace(&b).unwrap();
    }
    let c = RowCipher::new(key(3), CATALOG_DOMAIN).unwrap();
    let (ta, tb) = (
        c.tag(&encode_ns_key(a.key())),
        c.tag(&encode_ns_key(b.key())),
    );
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        tx.execute(
            "UPDATE namespace SET tag = x'00' WHERE tag = ?1",
            [ta.to_vec()],
        )
        .unwrap();
        tx.execute(
            "UPDATE namespace SET tag = ?1 WHERE tag = ?2",
            rusqlite::params![ta.to_vec(), tb.to_vec()],
        )
        .unwrap();
        tx.execute(
            "UPDATE namespace SET tag = ?1 WHERE tag = x'00'",
            [tb.to_vec()],
        )
        .unwrap();
        tx.commit().unwrap();
    }
    let mut s = CacheStore::open(&path, Encryption::Row(key(3)), W1)
        .unwrap()
        .store;
    let all = s.load_all().unwrap();
    assert!(all.namespaces.is_empty());
    assert_eq!(
        all.dropped,
        vec![(None, RowError::Crypt(CryptError::Auth)); 2]
    );
}

/// H4. Cross-profile transplant: profile B's file gets profile A's rows and key check. B opens
/// with its own key: rebuilt, nothing of A loaded.
#[test]
fn h4_rows_and_key_check_from_another_profile_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (pa, pb) = (ProfileId::new(), ProfileId::new());
    let (fa, fb) = (
        dir.path().join(cache_file_name(&pa)),
        dir.path().join(cache_file_name(&pb)),
    );
    {
        let mut s = CacheStore::open(&fa, Encryption::Row(key(0xA)), W1)
            .unwrap()
            .store;
        s.save_namespace(&schema("gizli", "from_profile_a", 1))
            .unwrap();
        let mut s = CacheStore::open(&fb, Encryption::Row(key(0xB)), W1)
            .unwrap()
            .store;
        s.save_namespace(&schema("other", "from_profile_b", 1))
            .unwrap();
    }
    // Rows only (B's key check kept): every transplanted row fails.
    {
        let conn = rusqlite::Connection::open(&fb).unwrap();
        conn.execute(&format!("ATTACH DATABASE '{}' AS a", fa.display()), [])
            .unwrap();
        conn.execute("INSERT INTO main.namespace SELECT * FROM a.namespace", [])
            .unwrap();
    }
    {
        let mut s = CacheStore::open(&fb, Encryption::Row(key(0xB)), W1)
            .unwrap()
            .store;
        let all = s.load_all().unwrap();
        assert_eq!(all.namespaces.len(), 1);
        assert!(has_table(&all.namespaces[0], "from_profile_b"));
        assert_eq!(all.dropped, vec![(None, RowError::Crypt(CryptError::Auth))]);
    }
    // Key check too: B's key does not open A's check, so the file is rebuilt.
    {
        let conn = rusqlite::Connection::open(&fb).unwrap();
        conn.execute(&format!("ATTACH DATABASE '{}' AS a", fa.display()), [])
            .unwrap();
        conn.execute(
            "UPDATE main.header SET value = (SELECT value FROM a.header WHERE key = 'key_check')
             WHERE key = 'key_check'",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO main.namespace SELECT * FROM a.namespace", [])
            .unwrap();
    }
    let opened = CacheStore::open(&fb, Encryption::Row(key(0xB)), W1).unwrap();
    assert_eq!(opened.rebuilt, Some(RebuildReason::WrongKey));
}

/// H5 (flipped, fix round 1 A-m4). The generation, writer and epoch are bound into the
/// payload's associated data: a sealed payload restored without its bookkeeping fails, and so
/// does a reset of the epoch after a server change. Restoring a **whole** older row (every
/// column) still loads: whole-row rollback by someone who can write the file is accepted and
/// documented (ADR 0008 `m4-t11`), mitigated by every cache load being published stale and
/// validated in the background.
#[test]
fn h5_rollback_and_stale_reset_are_detected_or_documented() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let old = schema("gizli", "old_table", 77);
    let new = schema("gizli", "new_table", 99);
    let mut s = CacheStore::open(&path, Encryption::Row(key(5)), W1)
        .unwrap()
        .store;
    s.check_identity(ServerFingerprint(1)).unwrap();
    s.save_namespace(&old).unwrap();
    type Snap = (Vec<u8>, Vec<u8>, Vec<u8>, i64, i64, i64);
    let snap: Snap = {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.query_row(
            "SELECT ns_key, watermark, payload, generation, writer_id, epoch FROM namespace",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .unwrap()
    };
    s.save_namespace(&new).unwrap();
    // The sealed columns alone: the generation no longer matches, so the row fails.
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute(
            "UPDATE namespace SET ns_key = ?1, watermark = ?2, payload = ?3",
            rusqlite::params![snap.0, snap.1, snap.2],
        )
        .unwrap();
    }
    assert!(matches!(
        s.load_namespace(new.key()),
        Err(CacheError::Row(RowError::Crypt(CryptError::Auth)))
    ));
    // The whole row, bookkeeping included: accepted (documented limitation).
    s.save_namespace(&new).unwrap();
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute(
            "UPDATE namespace SET ns_key = ?1, watermark = ?2, payload = ?3, generation = ?4,
                 writer_id = ?5, epoch = ?6",
            rusqlite::params![snap.0, snap.1, snap.2, snap.3, snap.4, snap.5],
        )
        .unwrap();
    }
    let got = s.load_namespace(new.key()).unwrap().unwrap();
    assert!(has_table(&got, "old_table"));
    // Stale reset: a server change starts a new epoch; the attacker sets the row's epoch to
    // it: the row fails authentication instead of loading as fresh.
    s.save_namespace(&new).unwrap();
    assert_eq!(
        s.check_identity(ServerFingerprint(2)).unwrap(),
        IdentityCheck::Changed
    );
    assert_eq!(
        s.load_namespace(new.key())
            .unwrap()
            .unwrap()
            .meta()
            .watermark,
        Watermark::None
    );
    assert!(s.row_info(new.key()).unwrap().unwrap().stale);
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute("UPDATE namespace SET epoch = epoch + 1", [])
            .unwrap();
    }
    assert!(matches!(
        s.load_namespace(new.key()),
        Err(CacheError::Row(RowError::Crypt(CryptError::Auth)))
    ));
}

/// H6 (flipped, fix round 1 A-I1). An encrypted cache reopened with `Encryption::None` is
/// refused (memory-only) and left untouched; no name is ever written in clear.
#[test]
fn h6_encrypted_file_reopened_without_key_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    {
        let mut s = CacheStore::open(&path, Encryption::Row(key(6)), W1)
            .unwrap()
            .store;
        s.save_namespace(&schema("gizli", "musteri_hesap_ozet", 1))
            .unwrap();
    }
    let before = files_bytes(&path);
    assert!(!contains(&before, b"musteri_hesap_ozet"));
    assert!(matches!(
        CacheStore::open(&path, Encryption::None, W1),
        Err(CacheError::Disabled)
    ));
    assert_eq!(files_bytes(&path), before, "the encrypted file was touched");
    // The key still opens it.
    let opened = CacheStore::open(&path, Encryption::Row(key(6)), W1).unwrap();
    assert!(opened.rebuilt.is_none());
}

/// H7 (flipped, fix round 1 A-m2). The sealed namespace key is padded: its ciphertext length
/// no longer shows the name length.
#[test]
fn h7_sealed_ns_key_length_does_not_reveal_the_name_length() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let long = schema("a_rather_long_secret_schema_name", "t", 1);
    let short = schema("s", "t", 1);
    {
        let mut s = CacheStore::open(&path, Encryption::Row(key(7)), W1)
            .unwrap()
            .store;
        s.save_namespace(&long).unwrap();
        s.save_namespace(&short).unwrap();
    }
    let c = rusqlite::Connection::open(&path).unwrap();
    let lens: Vec<i64> = c
        .prepare("SELECT length(ns_key) FROM namespace")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(lens.len(), 2);
    assert_eq!(lens[0], lens[1]);
    assert_eq!(lens[0] as usize, 256 + OVERHEAD);
    assert_ne!(lens[0] as usize, encode_ns_key(long.key()).len() + OVERHEAD);
}

/// H8 (flipped, fix round 1 A-m3; Unix). A profile's cache name hard-linked to a file outside
/// the cache directory: profile deletion removes the link and never writes through it.
#[cfg(unix)]
#[test]
fn h8_scrub_never_writes_through_a_hard_link() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("victim.txt");
    std::fs::write(&victim, b"precious user data").unwrap();
    let p = ProfileId::new();
    let link = dir.path().join(format!("{}.v1.sqlite.corrupt-1", p.0));
    std::fs::hard_link(&victim, &link).unwrap();
    CacheStore::delete_profile_files(dir.path(), &p).unwrap();
    assert_eq!(std::fs::read(&victim).unwrap(), b"precious user data");
    assert!(!link.exists());
    // A hard-linked live cache file is refused at open (never written through either).
    let path = dir.path().join(cache_file_name(&p));
    std::fs::hard_link(&victim, &path).unwrap();
    assert!(CacheStore::open(&path, Encryption::None, W1).is_err());
    assert_eq!(std::fs::read(&victim).unwrap(), b"precious user data");
}

/// H9. Hostile length fields and varints in the standalone column decoders and the body.
#[test]
fn h9_hostile_lengths_never_allocate_or_panic() {
    // ns key: kind byte then a string claiming 2^63 bytes.
    let mut b = vec![2u8];
    b.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F]);
    assert!(decode_ns_key(&b).is_err());
    // An over-long varint (11 continuation bytes) and a 10-byte varint overflowing u64.
    assert!(
        decode_ns_key(&[
            2, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01
        ])
        .is_err()
    );
    assert!(
        decode_ns_key(&[
            2, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x02
        ])
        .is_err()
    );
    assert!(decode_watermark(&[0xFF; 64]).is_err());
    // Body: zero strings, then a header record whose length prefix claims usize::MAX.
    let mut framed = b"HVNS".to_vec();
    framed.extend_from_slice(&FORMAT_MAJOR.to_le_bytes());
    framed.extend_from_slice(&FORMAT_MINOR.to_le_bytes());
    framed.push(0);
    framed.extend_from_slice(&[0; 8]);
    framed.push(0);
    framed.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01]);
    reseal_checksum(&mut framed);
    assert!(decode(&framed).is_err());
    // A newer-minor (lenient) payload with a huge object count: still capped.
    let ns = &pg_catalog(&[("public", 3)], 9)[0];
    let mut good = hacivat_meta::codec::encode_namespace(ns).unwrap();
    good[6..8].copy_from_slice(&u16::MAX.to_le_bytes());
    good.extend_from_slice(&[0xFF; 9]);
    reseal_checksum(&mut good);
    let _ = decode(&good);
}

// Re-review of fix round 1 (`task-m4-11-review-a-hostile-rr1.rs`), committed in fix round 2.
// Both probes confirmed a defence and are kept as written.

/// H10. A replayed older sealed `server` header (fingerprint + epoch) after a server change:
/// on the next identity check every row is stale. Replay itself stays possible (the file alone
/// cannot detect it; ADR 0008 `m4-t11`), but it never makes a row fresh.
#[test]
fn h10_replayed_server_header_never_makes_rows_fresh_after_identity_check() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let mut s = CacheStore::open(&path, Encryption::Row(key(10)), W1)
        .unwrap()
        .store;
    s.check_identity(ServerFingerprint(0xA)).unwrap();
    s.save_namespace(&schema("gizli", "from_a", 5)).unwrap();
    let old_header: Vec<u8> = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row("SELECT value FROM header WHERE key = 'server'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        s.check_identity(ServerFingerprint(0xB)).unwrap(),
        IdentityCheck::Changed
    );
    s.save_namespace(&schema("other", "from_b", 6)).unwrap();
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE header SET value = ?1 WHERE key = 'server'",
            [old_header],
        )
        .unwrap();
    // The identity check against the real server (B) starts an epoch past every row's.
    assert_eq!(
        s.check_identity(ServerFingerprint(0xB)).unwrap(),
        IdentityCheck::Changed
    );
    let all = s.load_all().unwrap();
    assert_eq!(all.namespaces.len(), 2);
    assert!(
        all.namespaces
            .iter()
            .all(|n| n.meta().watermark == Watermark::None)
    );
}

/// H11. The tag and subkey construction match the documented HMAC-SHA256 layout, recomputed
/// independently.
#[test]
fn h11_tag_is_documented_hmac_layout() {
    use hmac::{Hmac, Mac};
    type H = Hmac<sha2::Sha256>;
    let master = [0x42u8; 32];
    let d = CATALOG_DOMAIN.as_bytes();
    let mut m = <H as Mac>::new_from_slice(&master).unwrap();
    m.update(b"hv-rowcipher-v1");
    m.update(&[0, 2]);
    m.update(&(d.len() as u16).to_le_bytes());
    m.update(d);
    m.update(&[1]);
    let lookup = m.finalize().into_bytes();
    let mut t = <H as Mac>::new_from_slice(&lookup).unwrap();
    t.update(b"public");
    let want = t.finalize().into_bytes();
    let c = RowCipher::new(RowKey::from_bytes(&master), CATALOG_DOMAIN).unwrap();
    assert_eq!(c.tag(b"public"), want[..16]);
}
