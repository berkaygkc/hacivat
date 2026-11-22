//! M4b Task 1a: the script-workspace seams fail closed (F31) until their tasks. Nothing is
//! shown, bound, read, written, watched or passed: a stub that answered "visible", "bound",
//! "saved" or "no secret" would be a silent weakening.

use std::path::Path;
use std::sync::{Arc, Mutex};

use hacivat_proto::{EnvTag, ProfileId};
use hacivat_scripts::testing::FakeEventSource;
// ## m4b-t04-scanner (named exception): `envscope`, `pathsafe` and `visible` are Task 4's now.
// ## m4b-t10-service (named exception): the service stubs' imports are gone with their tests.
use hacivat_scripts::{
    DatabaseName, DbNode, EntryKind, EventSource, FileOpError, LibraryId, LibraryService,
    LocalMapping, PathError, RawHint, RelPath, SecretKind, SpanKind, WatchError, binding,
    scan_secrets,
};
use hacivat_settings::LocalPaths;

fn rel(s: &str) -> RelPath {
    RelPath::parse(s).unwrap()
}

/// Every file name and its bytes under `dir`, recursively (the "nothing was written" check).
fn tree(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                todo.push(p);
            } else {
                let name = p.strip_prefix(dir).unwrap().to_string_lossy().into_owned();
                out.push((name, std::fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// A service over app directories in one temp dir, and a "library" folder with one script in
/// another (the stub cannot attach, so the folder is only what a test points at).
fn service_with_library() -> (LibraryService, tempfile::TempDir, tempfile::TempDir) {
    let app = tempfile::tempdir().unwrap();
    let lib = tempfile::tempdir().unwrap();
    std::fs::write(lib.path().join("b.sql"), b"SELECT 1;").unwrap();
    let s = LibraryService::open(
        &LocalPaths::under(app.path()),
        &tokio::runtime::Handle::current(),
    );
    (s, app, lib)
}

// ## m4b-t04-scanner (named exception): `visible_stub_hides_everything` is gone; Task 4 wrote
// the rules (`tests/envscope.rs`).

// ## m4b-t03-registry (named exception): Task 3 filled `binding::resolve`, `LogicalDb::parse`,
// `names::same_db` and `manifest::{read, read_bytes}`; their stub tests (`resolve_stub_binds_nothing`,
// `same_db_stub_matches_nothing` and the manifest half of the test below) moved to
// `tests/{binding,manifest}.rs` as the real behaviour. A mapping to a library that is not there
// still binds nothing:
#[test]
fn resolve_binds_nothing_without_libraries() {
    let profile = ProfileId::new();
    let node = DbNode {
        profile,
        database: DatabaseName::new("orders"),
        logical: hacivat_scripts::testing::logical("orders"),
    };
    let map = LocalMapping {
        profile,
        database: DatabaseName::new("orders"),
        library: LibraryId::new(),
    };
    assert!(binding::resolve(&node, &[], &[map]).is_empty());
}
// ## m4b-t03-registry

// ## m4b-t01a-fr1 (review M3): one distinct bit per tag, so no tag can stand for another.
#[test]
fn env_set_has_one_bit_per_tag() {
    for tag in EnvTag::ALL {
        let set = hacivat_scripts::EnvSet::of(tag);
        for other in EnvTag::ALL {
            assert_eq!(set.contains(other), other == tag, "{tag:?} {other:?}");
        }
    }
    let all = EnvTag::ALL
        .iter()
        .fold(hacivat_scripts::EnvSet::empty(), |s, t| s.with(*t));
    assert!(EnvTag::ALL.iter().all(|t| all.contains(*t)));
    assert!(
        hacivat_scripts::EnvSet::of(EnvTag::Production)
            .intersect(hacivat_scripts::EnvSet::of(EnvTag::Local))
            .is_empty()
    );
}

#[test]
fn secret_stub_reports_the_whole_text() {
    let f = scan_secrets("SELECT 1", &[]);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].range, 0..8);
    assert_eq!(f[0].kind, SecretKind::Unscanned);
    // Spans that say "nothing here" do not make the stub pass either.
    let f = scan_secrets("SELECT password_hash", &[(7..20, SpanKind::Other)]);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].range, 0..20);
}

// ## m4b-t03-registry / m4b-t04-scanner (named exceptions, merged as a union): the manifest
// and pathsafe stub assertions are gone; Task 3 wrote `tests/manifest.rs`, Task 4 `tests/pathsafe.rs`.

// ## m4b-t10-service (named exception): the service is Task 10's now. `save_stub_writes_nothing`,
// `service_open_publishes_an_unavailable_empty_snapshot` and the attach/detach/map/read/stream
// half of the test below moved to `tests/service.rs` as the real behaviour. The file operations
// (create, rename, Move to Trash) stay fail-closed stubs until their task (T13):
#[tokio::test]
async fn file_operation_stubs_refuse_and_touch_no_file() {
    let (s, app, dir) = service_with_library();
    let before = tree(dir.path());
    let id = LibraryId::new();
    assert_eq!(
        s.create(id, &rel("c.sql"), EntryKind::Sql).await,
        Err(FileOpError::NotImplemented)
    );
    assert_eq!(
        s.rename(id, &rel("b.sql"), &rel("d.sql")).await,
        Err(FileOpError::NotImplemented)
    );
    assert_eq!(
        s.trash(id, &rel("b.sql")).await,
        Err(FileOpError::NotImplemented)
    );
    assert_eq!(tree(dir.path()), before);
    assert!(
        tree(app.path())
            .iter()
            .all(|(name, _)| !name.contains("c.sql"))
    );
}
// ## m4b-t10-service

#[test]
fn rel_path_keeps_its_structural_invariant() {
    for good in [
        "a.sql",
        "@production/purge.sql",
        "rapor İ.sql",
        "mu\u{0308}s.sql",
        "a/b/c.md",
    ] {
        assert_eq!(rel(good).as_str(), good);
    }
    for bad in [
        "",
        "/a.sql",
        "a//b.sql",
        "a/",
        "./a.sql",
        "a/../b.sql",
        "..",
        "a\\b.sql",
        "a\0b",
        "a\nb",
        "a\u{7}b",
    ] {
        assert_eq!(RelPath::parse(bad), Err(PathError::Malformed), "{bad:?}");
    }
    assert_eq!(
        rel("a/b/c.md").components().collect::<Vec<_>>(),
        ["a", "b", "c.md"]
    );
}

/// A script tab's binding as a saved file keeps it (Task 1b's `TabKind::Script`).
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct Tab {
    library: LibraryId,
    rel: RelPath,
    database: DatabaseName,
}

#[test]
fn ids_and_names_round_trip_through_serde() {
    let tab = Tab {
        library: LibraryId::new(),
        rel: rel("@dev/a.sql"),
        database: DatabaseName::new("Orders"),
    };
    let text = toml::to_string(&tab).unwrap();
    assert!(
        text.contains(&format!("library = \"{}\"", tab.library)),
        "{text}"
    );
    assert!(text.contains("rel = \"@dev/a.sql\""), "{text}");
    assert!(text.contains("database = \"Orders\""), "{text}");
    assert_eq!(toml::from_str::<Tab>(&text).unwrap(), tab);
    // A path that is not library-relative never deserializes, nor a library ID that is not one.
    for bad in ["../etc/passwd", "/etc/passwd", "a\\\\b"] {
        let t = format!(
            "library = \"{}\"\nrel = \"{bad}\"\ndatabase = \"x\"\n",
            tab.library
        );
        assert!(toml::from_str::<Tab>(&t).is_err(), "{bad}");
    }
    let t = "library = \"not-a-uuid\"\nrel = \"a.sql\"\ndatabase = \"x\"\n";
    assert!(toml::from_str::<Tab>(t).is_err());
}

#[test]
fn fake_event_source_delivers_only_what_the_test_emits() {
    let (mut src, handle) = FakeEventSource::new();
    let got: Arc<Mutex<Vec<Vec<RawHint>>>> = Arc::default();
    let sink = got.clone();
    let root = Path::new("/lib");
    src.watch(root, Box::new(move |b| sink.lock().unwrap().push(b)))
        .unwrap();
    assert_eq!(handle.watched(), [root.to_path_buf()]);
    assert!(handle.emit(root, vec![RawHint::Path(root.join("a.sql"))]));
    assert!(!handle.emit(Path::new("/other"), vec![RawHint::Lost]));
    src.unwatch(root);
    assert!(!handle.emit(root, vec![RawHint::Rescan(root.into())]));
    assert_eq!(
        *got.lock().unwrap(),
        [vec![RawHint::Path(root.join("a.sql"))]]
    );
    let (mut refusing, h) = FakeEventSource::refusing(WatchError::Backend("lost".into()));
    assert_eq!(
        refusing.watch(root, Box::new(|_| {})),
        Err(WatchError::Backend("lost".into()))
    );
    assert!(h.watched().is_empty());
}
