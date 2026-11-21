//! M4b Task 1a: the script-workspace seams fail closed (F31) until their tasks. Nothing is
//! shown, bound, read, written, watched or passed: a stub that answered "visible", "bound",
//! "saved" or "no secret" would be a silent weakening.

use std::ffi::OsStr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use hacivat_proto::{EnvTag, ProfileId};
use hacivat_scripts::testing::FakeEventSource;
use hacivat_scripts::{
    AttachError, DatabaseName, DbNode, EntryKind, EnvScope, EventSource, FileOpError, LibraryId,
    LibraryRoot, LibraryService, LibraryState, LinkState, LocalMapping, PathError, RawHint,
    ReadError, RegistryError, RelPath, SaveError, SaveText, ScriptEntry, SecretKind, SpanKind,
    UnavailableReason, WatchError, binding, envscope, fileio, manifest, pathsafe, scan_secrets,
    visible,
};
use hacivat_settings::LocalPaths;

fn rel(s: &str) -> RelPath {
    RelPath::parse(s).unwrap()
}

fn entry(s: &str) -> ScriptEntry {
    ScriptEntry {
        rel: rel(s),
        kind: EntryKind::Sql,
        scope: EnvScope::Any,
        link: LinkState::Plain,
        dataless: false,
        len: 8,
        mtime: SystemTime::UNIX_EPOCH,
    }
}

fn text(s: &str) -> SaveText {
    SaveText {
        text: s.into(),
        meta: Default::default(),
    }
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
    let (s, _rx) = LibraryService::open(
        &LocalPaths::under(app.path()),
        &tokio::runtime::Handle::current(),
    );
    (s, app, lib)
}

#[test]
fn visible_stub_hides_everything() {
    assert!(!visible(&entry("a.sql"), Some(EnvTag::Dev)));
    assert!(!visible(&entry("a.sql"), None));
    for tag in EnvTag::ALL {
        assert!(!visible(&entry("a.sql"), Some(tag)), "{tag:?}");
    }
    assert!(matches!(
        envscope::scope_of(&rel("a.sql")),
        EnvScope::Hidden(_)
    ));
}

#[test]
fn resolve_stub_binds_nothing() {
    let profile = ProfileId::new();
    let lib = LibraryId::new();
    let node = DbNode {
        profile,
        database: DatabaseName::new("orders"),
        logical: logical_for_test(),
    };
    let map = LocalMapping {
        profile,
        database: DatabaseName::new("orders"),
        library: lib,
    };
    // Even an explicit mapping for exactly this node binds nothing.
    assert!(binding::resolve(&node, &[], &[map]).is_empty());
}

/// A `LogicalDb` for the binding stub: `LogicalDb::parse` itself refuses every name until
/// Task 3, so the test double builds one.
fn logical_for_test() -> hacivat_scripts::LogicalDb {
    assert!(matches!(
        hacivat_scripts::LogicalDb::parse("orders"),
        Err(hacivat_scripts::NameError::NotImplemented)
    ));
    hacivat_scripts::testing::logical("orders")
}

// ## m4b-t01a-fr1 (review M4): binding compares with `same_db`, never `==`; the stub matches
// nothing (Task 3 writes NFC + ASCII case-insensitive).
#[test]
fn same_db_stub_matches_nothing() {
    let a = hacivat_scripts::testing::logical("orders");
    assert!(!a.same_db(&a));
    assert!(!hacivat_scripts::names::same_db("orders", "orders"));
}

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

#[test]
fn manifest_and_pathsafe_stubs_read_and_accept_nothing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".hacivat")).unwrap();
    std::fs::write(
        dir.path().join(".hacivat/workspace.toml"),
        "format = 1\ndatabase = \"orders\"\n",
    )
    .unwrap();
    for r in [
        manifest::read(dir.path()),
        manifest::read_bytes(b"format = 1\ndatabase = \"orders\"\n", "lib"),
    ] {
        assert!(r.db.is_none());
        assert!(r.aliases.is_empty());
        assert_eq!(r.diags, [manifest::Diag::NotImplemented]);
    }
    for name in ["a.sql", "ok", "rapor.sql"] {
        assert_eq!(
            pathsafe::component(OsStr::new(name)),
            Err(PathError::NotImplemented)
        );
    }
    let root = LibraryRoot {
        id: LibraryId::new(),
        path: dir.path().into(),
        canonical: dir.path().canonicalize().unwrap(),
    };
    assert_eq!(
        pathsafe::join(&root, &rel("a.sql")),
        Err(PathError::NotImplemented)
    );
}

#[test]
fn fileio_stubs_read_and_write_nothing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.sql"), b"SELECT 1;").unwrap();
    let before = tree(dir.path());
    let root = LibraryRoot {
        id: LibraryId::new(),
        path: dir.path().into(),
        canonical: dir.path().canonicalize().unwrap(),
    };
    assert!(matches!(
        fileio::read(&root, &rel("a.sql"), Default::default()),
        Err(ReadError::NotImplemented)
    ));
    assert!(matches!(
        fileio::save(&root, &rel("a.sql"), text("DROP TABLE t;"), None),
        Err(SaveError::NotImplemented)
    ));
    assert!(matches!(
        fileio::save(&root, &rel("new.sql"), text("x"), None),
        Err(SaveError::NotImplemented)
    ));
    assert_eq!(tree(dir.path()), before);
}

#[tokio::test]
async fn save_stub_writes_nothing() {
    let (s, app, dir) = service_with_library();
    let lib = LibraryId::new();
    assert!(matches!(
        s.save(lib, &rel("a.sql"), text("x"), None).await,
        Err(SaveError::NotImplemented)
    ));
    assert!(!dir.path().join("a.sql").exists());
    assert!(matches!(
        s.save(lib, &rel("b.sql"), text("DROP TABLE t;"), None)
            .await,
        Err(SaveError::NotImplemented)
    ));
    assert_eq!(
        std::fs::read(dir.path().join("b.sql")).unwrap(),
        b"SELECT 1;"
    );
    assert!(tree(app.path()).is_empty());
}

#[tokio::test]
async fn service_open_publishes_an_unavailable_empty_snapshot() {
    let app = tempfile::tempdir().unwrap();
    let paths = LocalPaths::under(app.path());
    let (s, mut rx) = LibraryService::open(&paths, &tokio::runtime::Handle::current());
    let snap = s.snapshot();
    assert_eq!(snap.generation, 0);
    assert_eq!(
        snap.state,
        LibraryState::Unavailable(UnavailableReason::NotImplemented)
    );
    assert!(snap.libraries.is_empty() && snap.mappings.is_empty() && snap.git.is_empty());
    assert!(snap.library(LibraryId::new()).is_none());
    // No registry was read or created, and nothing was announced.
    assert!(!paths.libraries_toml().exists());
    assert!(tree(app.path()).is_empty());
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn every_service_call_refuses_and_touches_no_file() {
    let (s, app, dir) = service_with_library();
    let before = tree(dir.path());
    let id = LibraryId::new();
    assert!(matches!(
        s.attach(dir.path().into(), None, true).await,
        Err(AttachError::NotImplemented)
    ));
    assert!(!dir.path().join(".hacivat").exists());
    assert_eq!(s.detach(id).await, Err(RegistryError::NotImplemented));
    let m = LocalMapping {
        profile: ProfileId::new(),
        database: DatabaseName::new("orders"),
        library: id,
    };
    assert_eq!(s.map(m, true).await, Err(RegistryError::NotImplemented));
    assert!(matches!(
        s.read(id, &rel("b.sql")).await,
        Err(ReadError::NotImplemented)
    ));
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
    assert!(matches!(
        s.open_stream(id, &rel("b.sql")),
        Err(ReadError::NotImplemented)
    ));
    assert_eq!(tree(dir.path()), before);
    assert!(tree(app.path()).is_empty());
    assert_eq!(s.snapshot().generation, 0);
}

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
