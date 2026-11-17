//! HV-MODEL-03: the codec round trip and the SQLite catalog cache.

use std::sync::Arc;

use hacivat_meta::codec::payload_version;
use hacivat_meta::fixture::{DB, pg_catalog};
use hacivat_meta::*;
use hacivat_proto::DbmsKind;

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

#[test]
fn every_namespace_round_trips_through_the_codec() {
    for ns in small() {
        let bytes = encode_namespace(&ns);
        assert_eq!(payload_version(&bytes), Some((FORMAT_MAJOR, FORMAT_MINOR)));
        let back = decode_namespace(&bytes).unwrap();
        assert_same(&ns, &back);
        assert!(!back.from_newer_minor());
        // Indexes are rebuilt identically.
        for (i, o) in ns.objs().iter().enumerate() {
            if let Some(g) = o.kind.group(DbmsKind::Postgres) {
                assert_eq!(ns.lookup(g, &o.name), back.lookup(g, &o.name), "{i}");
            }
        }
        // Encoding is deterministic.
        assert_eq!(encode_namespace(&back), bytes);
    }
}

#[test]
fn decoding_interns_repeated_strings() {
    let ns = &small()[2];
    let back = decode_namespace(&encode_namespace(ns)).unwrap();
    let types: Vec<&Name> = back
        .objs()
        .iter()
        .filter_map(|o| o.props.as_column())
        .map(|c| &c.type_text)
        .filter(|t| &***t == "bigint")
        .collect();
    assert!(types.len() > 10);
    assert!(types.windows(2).all(|w| Arc::ptr_eq(w[0], w[1])));
}

#[test]
fn the_store_saves_and_loads_every_namespace() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let nss = small();
    {
        let mut store = CacheStore::open(&path).unwrap().store;
        for ns in &nss {
            store.save_namespace(ns).unwrap();
        }
        // Saving again replaces the row.
        store.save_namespace(&nss[2]).unwrap();
    }
    let opened = CacheStore::open(&path).unwrap();
    assert!(opened.rebuilt.is_none());
    let all = opened.store.load_all().unwrap();
    assert!(all.dropped.is_empty());
    assert_eq!(all.namespaces.len(), nss.len());
    for ns in &nss {
        let got = all.namespaces.iter().find(|n| n.key() == ns.key()).unwrap();
        assert_same(ns, got);
    }
    let (one, err) = opened
        .store
        .load_namespace(&NsKey::schema([DB, "sales"]))
        .unwrap();
    assert!(err.is_none());
    assert_eq!(one.unwrap().column_count(), nss[3].column_count());
    let (none, _) = opened
        .store
        .load_namespace(&NsKey::schema([DB, "nope"]))
        .unwrap();
    assert!(none.is_none());
}

#[cfg(unix)]
#[test]
fn the_cache_file_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let mut store = CacheStore::open(&path).unwrap().store;
    store.save_namespace(&small()[0]).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn a_corrupt_file_is_moved_aside_and_rebuilt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    std::fs::write(&path, vec![0x5Au8; 8192]).unwrap();
    let opened = CacheStore::open(&path).unwrap();
    match opened.rebuilt {
        Some(RebuildReason::Corrupt { moved_to }) => {
            assert!(moved_to.exists());
            assert!(
                moved_to
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .contains(".corrupt-")
            );
        }
        other => panic!("expected a rebuild, got {other:?}"),
    }
    assert!(opened.store.load_all().unwrap().namespaces.is_empty());
}

#[test]
fn a_corrupt_row_is_dropped_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let nss = small();
    {
        let mut store = CacheStore::open(&path).unwrap().store;
        for ns in &nss {
            store.save_namespace(ns).unwrap();
        }
    }
    // Flip one payload byte behind the store's back.
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        let key = hacivat_meta::codec::encode_ns_key(&NsKey::schema([DB, "sales"]));
        let mut payload: Vec<u8> = conn
            .query_row(
                "SELECT payload FROM namespace WHERE ns_key = ?1",
                [&key],
                |r| r.get(0),
            )
            .unwrap();
        let mid = payload.len() / 2;
        payload[mid] ^= 0x01;
        conn.execute(
            "UPDATE namespace SET payload = ?1 WHERE ns_key = ?2",
            rusqlite::params![payload, key],
        )
        .unwrap();
    }
    let store = CacheStore::open(&path).unwrap().store;
    let all = store.load_all().unwrap();
    assert_eq!(all.dropped.len(), 1);
    assert_eq!(all.dropped[0].1, DecodeError::Checksum);
    assert_eq!(all.namespaces.len(), nss.len() - 1);
    // The bad row is gone; the rest stays.
    assert_eq!(store.load_all().unwrap().namespaces.len(), nss.len() - 1);
}

#[test]
fn temp_namespaces_are_never_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = CacheStore::open(&dir.path().join("p.v1.sqlite"))
        .unwrap()
        .store;
    let t = Subtree::new(Obj::new(ObjectKind::Schema, "temp").with_flags(ObjFlags::TEMP));
    let ns = NamespaceSnapshot::build(
        NsKey::schema(["temp"]),
        DbmsKind::Sqlite,
        FoldPolicy::for_dbms(DbmsKind::Sqlite),
        NsMeta::default(),
        t,
    )
    .unwrap();
    assert!(matches!(
        store.save_namespace(&ns),
        Err(CacheError::TempNamespace)
    ));
}

#[test]
fn sources_are_content_addressed() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = CacheStore::open(&dir.path().join("p.v1.sqlite"))
        .unwrap()
        .store;
    let text = "SELECT 1 AS \"İ\"";
    let a = store.put_source(text).unwrap();
    let b = store.put_source(text).unwrap();
    assert_eq!(a, b);
    assert_eq!(a, SourceRef::of(text));
    assert_eq!(store.source(a).unwrap().as_deref(), Some(text));
}

#[test]
fn the_file_name_carries_the_format_major() {
    let p = hacivat_proto::ProfileId::new();
    let name = hacivat_meta::persist::cache_file_name(&p);
    assert!(name.ends_with(&format!(".v{FORMAT_MAJOR}.sqlite")));
    assert!(name.starts_with(&p.to_string()));
}
