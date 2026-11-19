//! Encryption at rest (M4 Task 11, R-M4-15, D-016): `RowCipher` and the encrypted catalog
//! cache (alg/02 HV-MODEL-03 test 6, edge 9).

#![cfg(feature = "persist")]

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use hacivat_meta::codec::encode_ns_key;
use hacivat_meta::fixture::{DB, pg_catalog, table_subtree};
use hacivat_meta::ids::{NsKey, SourceRef};
use hacivat_meta::kind::ObjectKind;
use hacivat_meta::name::FoldRule;
use hacivat_meta::namespace::{
    Level, NamespaceSnapshot, NsMeta, Obj, PgIdentity, Subtree, Watermark,
};
use hacivat_meta::persist::crypt::{NONCE_LEN, OVERHEAD};
use hacivat_meta::persist::{
    CATALOG_DOMAIN, CacheError, CacheStore, CryptError, Encryption, IdentityCheck, RebuildReason,
    RowCipher, RowError, RowKey, ServerFingerprint, WriterId,
};
use hacivat_proto::DbmsKind;
use proptest::prelude::*;

const W1: WriterId = WriterId(1);
const SECRET_NAME: &str = "musteri_hesap_ozet";

fn key(byte: u8) -> RowKey {
    RowKey::from_bytes([byte; 32])
}

fn sealed(byte: u8) -> Encryption {
    Encryption::Row(key(byte))
}

fn open_with(path: &Path, enc: Encryption) -> hacivat_meta::persist::Opened {
    CacheStore::open(path, enc, W1).unwrap()
}

/// A schema `gizli` holding table `musteri_hesap_ozet` (and the generated catalog besides).
fn catalog() -> Vec<NamespaceSnapshot> {
    let mut nss = pg_catalog(&[("public", 50), ("sales", 10)], 600);
    let mut t = Subtree::new(Obj::new(ObjectKind::Schema, "gizli"));
    let root = t.root();
    t.graft_under(root, table_subtree("gizli", SECRET_NAME, 6, 90_000, None));
    nss.push(
        NamespaceSnapshot::build(
            NsKey::schema([DB, "gizli"]),
            DbmsKind::Postgres,
            FoldRule::Exact,
            NsMeta {
                level: Level::Sources,
                watermark: Watermark::PgXid {
                    w: 77,
                    identity: PgIdentity {
                        system_identifier: 5,
                        start_time_micros: 6,
                    },
                },
                loaded_at_ms: 8,
            },
            t,
        )
        .unwrap(),
    );
    nss
}

fn files_bytes(path: &Path) -> Vec<u8> {
    let mut all = Vec::new();
    for suffix in ["", "-wal", "-shm"] {
        if let Ok(b) = std::fs::read(PathBuf::from(format!("{}{suffix}", path.display()))) {
            all.extend_from_slice(&b);
        }
    }
    all
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn assert_same(a: &NamespaceSnapshot, b: &NamespaceSnapshot) {
    assert_eq!(a.key(), b.key());
    assert_eq!(a.meta(), b.meta());
    assert_eq!(a.objs(), b.objs());
}

// ---------------------------------------------------------------------------------------------
// RowCipher.

#[test]
fn rows_sealed_in_one_domain_fail_in_another() {
    let catalog = RowCipher::new(key(1), CATALOG_DOMAIN);
    let other = RowCipher::new(key(1), "hv-drift-v1");
    let row = catalog.seal(b"ad", b"payload").unwrap();
    assert_eq!(catalog.open(b"ad", &row).unwrap(), b"payload");
    assert_eq!(other.open(b"ad", &row), Err(CryptError::Auth));
    assert_eq!(catalog.open(b"other ad", &row), Err(CryptError::Auth));
    assert_eq!(
        RowCipher::new(key(2), CATALOG_DOMAIN).open(b"ad", &row),
        Err(CryptError::Auth)
    );
}

#[test]
fn row_cipher_debug_is_redacted() {
    let c = RowCipher::new(
        RowKey::from_bytes(*b"hv_test_pw-hv_test_pw-hv_test_pw"),
        CATALOG_DOMAIN,
    );
    for shown in [format!("{c:?}"), format!("{c:#?}")] {
        assert!(!shown.contains("hv_test_pw"), "{shown}");
        assert!(shown.contains("redacted"), "{shown}");
        assert!(shown.contains(CATALOG_DOMAIN), "{shown}");
    }
    let k = RowKey::from_bytes([0xAB; 32]);
    let shown = format!("{:?}{:?}", k, Encryption::Row(k.clone()));
    for leak in ["171", "ab", "AB"] {
        assert!(!shown.contains(leak), "{shown}");
    }
    // A store's `Debug` names no key either.
    let dir = tempfile::tempdir().unwrap();
    let opened = open_with(
        &dir.path().join("p.v1.sqlite"),
        Encryption::Row(RowKey::from_bytes(*b"hv_test_pw-hv_test_pw-hv_test_pw")),
    );
    let shown = format!("{opened:?}");
    assert!(!shown.contains("hv_test_pw"), "{shown}");
}

#[test]
fn tampered_ciphertext_is_an_error_not_a_panic() {
    let c = RowCipher::new(key(3), CATALOG_DOMAIN);
    let row = c.seal(b"ad", b"some catalog payload").unwrap();
    assert_eq!(row.len(), b"some catalog payload".len() + OVERHEAD);
    for at in 0..row.len() {
        for flip in [0x01u8, 0x80] {
            let mut b = row.clone();
            b[at] ^= flip;
            assert_eq!(c.open(b"ad", &b), Err(CryptError::Auth), "flip at {at}");
        }
    }
    for cut in 0..row.len() {
        let r = c.open(b"ad", &row[..cut]);
        assert!(r.is_err(), "cut at {cut}");
        if cut < OVERHEAD {
            assert_eq!(r, Err(CryptError::Truncated));
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn opening_arbitrary_bytes_never_panics(ad in proptest::collection::vec(any::<u8>(), 0..64),
                                            bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        let c = RowCipher::new(key(4), CATALOG_DOMAIN);
        prop_assert!(c.open(&ad, &bytes).is_err());
    }
}

#[test]
fn every_seal_draws_a_fresh_nonce() {
    let c = RowCipher::new(key(5), CATALOG_DOMAIN);
    let mut seen = HashSet::new();
    for _ in 0..10_000 {
        let row = c.seal(b"ad", b"same plaintext").unwrap();
        assert!(seen.insert(row[..NONCE_LEN].to_vec()), "nonce reused");
    }
}

// ---------------------------------------------------------------------------------------------
// The encrypted store.

#[test]
fn an_encrypted_store_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let nss = catalog();
    {
        let mut store = open_with(&path, sealed(9)).store;
        assert!(store.is_encrypted());
        for ns in &nss {
            store.save_namespace(ns).unwrap();
        }
        let r = store.put_source("SELECT 1 AS \"İ\"").unwrap();
        assert_eq!(
            store.source(r).unwrap().as_deref(),
            Some("SELECT 1 AS \"İ\"")
        );
        assert_eq!(
            store.check_identity(ServerFingerprint(1)).unwrap(),
            IdentityCheck::Same
        );
    }
    let opened = open_with(&path, sealed(9));
    assert!(opened.rebuilt.is_none());
    let mut store = opened.store;
    let all = store.load_all().unwrap();
    assert!(all.dropped.is_empty());
    assert_eq!(all.namespaces.len(), nss.len());
    for ns in &nss {
        assert_same(
            ns,
            all.namespaces.iter().find(|n| n.key() == ns.key()).unwrap(),
        );
    }
    let gizli = NsKey::schema([DB, "gizli"]);
    assert_same(&store.load_namespace(&gizli).unwrap().unwrap(), &nss[4]);
    assert_eq!(
        store.check_identity(ServerFingerprint(1)).unwrap(),
        IdentityCheck::Same
    );
    assert_eq!(
        store.check_identity(ServerFingerprint(2)).unwrap(),
        IdentityCheck::Changed
    );
    // Eviction reads the sealed keys to protect the current database.
    let out = store
        .evict(0, &|k: &NsKey| k.path.first().is_some_and(|d| &**d == DB))
        .unwrap();
    assert!(
        out.evicted
            .iter()
            .all(|k| k.as_ref().is_some_and(|k| k.path.is_empty()))
    );
    // Sources survive a collection only while referenced.
    assert_eq!(store.gc_sources(&HashSet::new()).unwrap(), 1);
}

/// The acceptance test of D-016's encrypted mode: no object name, no namespace name, no source
/// text in any byte of the file or its sidecars.
#[test]
fn encrypted_file_contains_no_object_name_in_clear() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let source = format!("SELECT * FROM {SECRET_NAME}");
    {
        let mut store = open_with(&path, sealed(11)).store;
        for ns in catalog() {
            store.save_namespace(&ns).unwrap();
        }
        store.put_source(&source).unwrap();
        store.check_identity(ServerFingerprint(3)).unwrap();
        let mut s = open_with(&path, sealed(11)).store;
        s.load_all().unwrap();
        // While the WAL is still open, and after close.
        let bytes = files_bytes(&path);
        for needle in [
            SECRET_NAME,
            "gizli",
            "tbl_00001",
            "app_owner",
            "external_ref",
            "Business records",
        ] {
            assert!(
                !contains(&bytes, needle.as_bytes()),
                "{needle} in the open files"
            );
        }
    }
    let bytes = files_bytes(&path);
    for needle in [
        SECRET_NAME,
        "gizli",
        "tbl_00001",
        "app_owner",
        "external_ref",
        "Business records",
        "SELECT",
    ] {
        assert!(!contains(&bytes, needle.as_bytes()), "{needle} in the file");
    }
    // The control: a plaintext cache does hold the names.
    let plain = dir.path().join("q.v1.sqlite");
    {
        let mut store = open_with(&plain, Encryption::None).store;
        for ns in catalog() {
            store.save_namespace(&ns).unwrap();
        }
    }
    assert!(contains(&files_bytes(&plain), SECRET_NAME.as_bytes()));
}

/// alg/02 test 6: a wrong key rebuilds the cache, without a panic and without reading a row.
#[test]
fn wrong_key_rebuilds_without_panic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    {
        let mut store = open_with(&path, sealed(1)).store;
        for ns in catalog() {
            store.save_namespace(&ns).unwrap();
        }
    }
    let opened = open_with(&path, sealed(2));
    assert_eq!(opened.rebuilt, Some(RebuildReason::WrongKey));
    assert_eq!(RebuildReason::WrongKey.ftl_reason(), "wrong-key");
    let mut store = opened.store;
    let all = store.load_all().unwrap();
    assert!(all.namespaces.is_empty() && all.dropped.is_empty());
    // The rebuilt file works under the new key.
    store.save_namespace(&catalog()[2]).unwrap();
    drop(store);
    let opened = open_with(&path, sealed(2));
    assert!(opened.rebuilt.is_none());
    assert_eq!(
        opened
            .store
            .row_info(catalog()[2].key())
            .unwrap()
            .unwrap()
            .generation,
        1
    );
}

/// Changing "Encrypt cached metadata" either way never reads rows of the other mode: the file
/// is scrubbed and rebuilt (a plaintext file's names do not survive into the encrypted one).
#[test]
fn switching_modes_scrubs_the_old_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    {
        let mut store = open_with(&path, Encryption::None).store;
        for ns in catalog() {
            store.save_namespace(&ns).unwrap();
        }
    }
    assert!(contains(&files_bytes(&path), SECRET_NAME.as_bytes()));
    let opened = open_with(&path, sealed(3));
    assert_eq!(opened.rebuilt, Some(RebuildReason::WrongKey));
    let mut store = opened.store;
    assert!(store.load_all().unwrap().namespaces.is_empty());
    store.save_namespace(&catalog()[4]).unwrap();
    assert!(!contains(&files_bytes(&path), SECRET_NAME.as_bytes()));
    drop(store);
    // And back: an encrypted file opened without a key is rebuilt, never read.
    let opened = open_with(&path, Encryption::None);
    assert_eq!(opened.rebuilt, Some(RebuildReason::WrongKey));
}

/// The associated data binds a row to its namespace key: a payload moved under another key
/// fails authentication and only that row is dropped.
#[test]
fn row_moved_to_another_ns_key_fails_authentication() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let nss = catalog();
    {
        let mut store = open_with(&path, sealed(7)).store;
        for ns in &nss {
            store.save_namespace(ns).unwrap();
        }
    }
    let c = RowCipher::new(key(7), CATALOG_DOMAIN);
    let tag = |k: &NsKey| c.tag(&encode_ns_key(k)).to_vec();
    let (public, gizli) = (NsKey::schema([DB, "public"]), NsKey::schema([DB, "gizli"]));
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        let payload: Vec<u8> = conn
            .query_row(
                "SELECT payload FROM namespace WHERE tag = ?1",
                [tag(&gizli)],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "UPDATE namespace SET payload = ?1 WHERE tag = ?2",
            rusqlite::params![payload, tag(&public)],
        )
        .unwrap();
    }
    let mut store = open_with(&path, sealed(7)).store;
    let all = store.load_all().unwrap();
    assert_eq!(
        all.dropped,
        vec![(Some(public.clone()), RowError::Crypt(CryptError::Auth))]
    );
    assert_eq!(all.namespaces.len(), nss.len() - 1);
    assert!(all.namespaces.iter().any(|n| n.key() == &gizli));
    // A sealed key column moved to another row's tag fails too.
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        let col: Vec<u8> = conn
            .query_row(
                "SELECT ns_key FROM namespace WHERE tag = ?1",
                [tag(&gizli)],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "UPDATE namespace SET ns_key = ?1 WHERE tag = ?2",
            rusqlite::params![col, tag(&NsKey::schema([DB, "sales"]))],
        )
        .unwrap();
    }
    let all = store.load_all().unwrap();
    assert_eq!(all.dropped, vec![(None, RowError::Crypt(CryptError::Auth))]);
    // A minor column edited under a sealed payload fails too.
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute(
            "UPDATE namespace SET minor = 1 WHERE tag = ?1",
            [tag(&gizli)],
        )
        .unwrap();
    }
    assert!(matches!(
        store.load_namespace(&gizli),
        Err(CacheError::Row(RowError::Crypt(CryptError::Auth)))
    ));
    // A source moved under another hash is refused.
    let a = store.put_source("SELECT 'a'").unwrap();
    let b = store.put_source("SELECT 'b'").unwrap();
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        let src_tag = |r: SourceRef| {
            let mut id = r.hash.to_le_bytes().to_vec();
            id.extend_from_slice(&r.len.to_le_bytes());
            c.tag(&id).to_vec()
        };
        let text: Vec<u8> = conn
            .query_row(
                "SELECT text FROM source WHERE tag = ?1",
                [src_tag(a)],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "UPDATE source SET text = ?1 WHERE tag = ?2",
            rusqlite::params![text, src_tag(b)],
        )
        .unwrap();
    }
    assert!(store.source(b).unwrap().is_none());
    assert_eq!(store.source(a).unwrap().as_deref(), Some("SELECT 'a'"));
}
