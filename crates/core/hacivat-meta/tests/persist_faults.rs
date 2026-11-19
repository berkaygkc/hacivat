//! HV-MODEL-03 faults (M4 Task 11): a kill inside a save, a bit flip in one row, a full disk,
//! a read-only data directory (alg/02 HV-MODEL-03 test 1; edge cases 1–3, 12).

#![cfg(feature = "persist")]

use std::path::{Path, PathBuf};

use hacivat_meta::codec::{DecodeError, encode_ns_key};
use hacivat_meta::fixture::{DB, pg_catalog};
use hacivat_meta::ids::NsKey;
use hacivat_meta::namespace::NamespaceSnapshot;
use hacivat_meta::persist::{CacheError, CacheStore, Encryption, Fault, RowError, WriterId};

const W1: WriterId = WriterId(1);

/// The environment variable that turns [`kill_child`] into the crashing save.
const KILL_ENV: &str = "HV_T11_KILL_DB";

fn small() -> Vec<NamespaceSnapshot> {
    pg_catalog(&[("public", 300), ("sales", 40)], 3_500)
}

fn open(path: &Path) -> CacheStore {
    CacheStore::open(path, Encryption::None, W1).unwrap().store
}

/// The child half of [`kill_during_commit_keeps_the_old_row`]: with `HV_T11_KILL_DB` set it
/// saves a changed `sales` with the abort fault armed, so the process dies after writing the
/// row and before `COMMIT` returns. Without the variable it does nothing.
#[test]
#[ignore = "child process of kill_during_commit_keeps_the_old_row"]
fn kill_child() {
    let Some(path) = std::env::var_os(KILL_ENV) else {
        return;
    };
    let mut store = open(Path::new(&path));
    let changed = pg_catalog(&[("public", 300), ("sales", 41)], 3_600);
    store.set_fault(Some(Fault::AbortBeforeCommit));
    let _ = store.save_namespace(&changed[3]);
    unreachable!("the armed save aborts the process");
}

/// alg/02 test 1 and edge 1: kill -9 during a save; on reopen the old row is intact and its
/// generation unchanged.
#[test]
fn kill_during_commit_keeps_the_old_row() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let nss = small();
    let sales = NsKey::schema([DB, "sales"]);
    {
        let mut store = open(&path);
        for ns in &nss {
            store.save_namespace(ns).unwrap();
        }
        assert_eq!(store.row_info(&sales).unwrap().unwrap().generation, 1);
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "kill_child", "--ignored", "--nocapture"])
        .env(KILL_ENV, &path)
        .status()
        .unwrap();
    assert!(!status.success(), "the child must die inside the save");
    let mut store = open(&path);
    let info = store.row_info(&sales).unwrap().unwrap();
    assert_eq!(info.generation, 1);
    let back = store.load_namespace(&sales).unwrap().unwrap();
    assert_eq!(back.objs(), nss[3].objs());
    let all = store.load_all().unwrap();
    assert!(all.dropped.is_empty());
    assert_eq!(all.namespaces.len(), nss.len());
}

/// Edge 12: a flipped bit drops that row only; the others load.
#[test]
fn bit_flip_drops_only_that_row() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let nss = small();
    {
        let mut store = open(&path);
        for ns in &nss {
            store.save_namespace(ns).unwrap();
        }
    }
    let key = encode_ns_key(&NsKey::schema([DB, "sales"]));
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        let mut payload: Vec<u8> = conn
            .query_row(
                "SELECT payload FROM namespace WHERE tag = ?1",
                [&key],
                |r| r.get(0),
            )
            .unwrap();
        let mid = payload.len() / 2;
        payload[mid] ^= 0x01;
        conn.execute(
            "UPDATE namespace SET payload = ?1 WHERE tag = ?2",
            rusqlite::params![payload, key],
        )
        .unwrap();
    }
    let mut store = open(&path);
    let all = store.load_all().unwrap();
    assert_eq!(all.dropped.len(), 1);
    assert_eq!(
        all.dropped[0],
        (
            Some(NsKey::schema([DB, "sales"])),
            RowError::Decode(DecodeError::Checksum)
        )
    );
    assert_eq!(all.namespaces.len(), nss.len() - 1);
    // The bad row is gone; the rest stays.
    let again = store.load_all().unwrap();
    assert!(again.dropped.is_empty());
    assert_eq!(again.namespaces.len(), nss.len() - 1);
}

/// A row whose columns are of the wrong type, or whose key and tag disagree, is dropped alone.
#[test]
fn inconsistent_rows_are_dropped_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let nss = small();
    {
        let mut store = open(&path);
        for ns in &nss {
            store.save_namespace(ns).unwrap();
        }
    }
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        // `sales`'s row now claims to be `public` (key column swapped).
        conn.execute(
            "UPDATE namespace SET ns_key = ?1 WHERE tag = ?2",
            rusqlite::params![
                encode_ns_key(&NsKey::schema([DB, "public"])),
                encode_ns_key(&NsKey::schema([DB, "sales"]))
            ],
        )
        .unwrap();
        // The server row's payload is text, not a blob.
        conn.execute(
            "UPDATE namespace SET payload = 'x' WHERE tag = ?1",
            [encode_ns_key(&NsKey::server())],
        )
        .unwrap();
    }
    let mut store = open(&path);
    let all = store.load_all().unwrap();
    assert_eq!(all.dropped.len(), 2);
    assert!(all.dropped.iter().all(|(_, e)| *e == RowError::Mismatch));
    assert_eq!(all.namespaces.len(), nss.len() - 2);
    assert!(matches!(
        store.load_namespace(&NsKey::schema([DB, "sales"])),
        Ok(None)
    ));
}

/// Edge 2: a full disk fails the save, warns once, keeps the previous row and touches nothing
/// in memory.
#[test]
fn disk_full_warns_once_and_memory_snapshot_is_intact() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.v1.sqlite");
    let nss = small();
    let mut store = open(&path);
    store.save_namespace(&nss[3]).unwrap();
    store.limit_growth().unwrap();
    let before: Vec<_> = nss[2].objs().to_vec();
    let first = store.save_namespace(&nss[2]);
    assert!(
        matches!(first, Err(CacheError::Full { warn: true })),
        "{first:?}"
    );
    let second = store.save_namespace(&nss[2]);
    assert!(
        matches!(second, Err(CacheError::Full { warn: false })),
        "{second:?}"
    );
    assert_eq!(nss[2].objs(), &before[..]);
    // The earlier row is intact, the failed one absent.
    let all = store.load_all().unwrap();
    assert_eq!(all.namespaces.len(), 1);
    assert_eq!(all.namespaces[0].objs(), nss[3].objs());
}

/// Edge 3: a read-only data directory runs memory-only: `open` fails and writes nothing.
#[cfg(unix)]
#[test]
fn read_only_directory_runs_memory_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let ro = dir.path().join("catalog");
    std::fs::create_dir(&ro).unwrap();
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o500)).unwrap();
    let path: PathBuf = ro.join("p.v1.sqlite");
    let opened = CacheStore::open(&path, Encryption::None, W1);
    assert!(matches!(opened, Err(CacheError::Io(_))), "{opened:?}");
    assert_eq!(std::fs::read_dir(&ro).unwrap().count(), 0);
    // A directory that cannot be created either.
    let nested = ro.join("deeper").join("p.v1.sqlite");
    assert!(CacheStore::open(&nested, Encryption::None, W1).is_err());
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o700)).unwrap();
}
