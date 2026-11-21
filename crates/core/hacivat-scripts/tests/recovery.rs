//! M4b Task 5: recovery copies of unsaved script text (HV-SCRIPT-07 step 5; R-M4b-5, Q1).
//! They live under the app's data folder, never inside a library (L14), 0700/0600.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hacivat_scripts::recovery::{Recovered, discard, store, take_all};
use hacivat_scripts::{DiskStamp, LibraryId, RelPath};
use hacivat_settings::LocalPaths;

fn rel(s: &str) -> RelPath {
    RelPath::parse(s).unwrap()
}

fn lib_id() -> LibraryId {
    LibraryId(uuid::Uuid::from_u128(
        0x5ca1ab1e_0000_4000_8000_000000000001,
    ))
}

/// App folders in one temp dir, and the library folder the copies are about in another.
fn env() -> (LocalPaths, tempfile::TempDir, tempfile::TempDir) {
    let app = tempfile::tempdir().unwrap();
    let lib = tempfile::tempdir().unwrap();
    fs::write(lib.path().join("a.sql"), b"SELECT 1;").unwrap();
    (LocalPaths::under(app.path()), app, lib)
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if e.file_type().unwrap().is_dir() {
                todo.push(p.clone());
            }
            out.push(p);
        }
    }
    out.sort();
    out
}

fn dir_of(paths: &LocalPaths) -> PathBuf {
    paths.script_recovery_dir(&lib_id().to_string()).unwrap()
}

fn stamp() -> DiskStamp {
    DiskStamp {
        len: 9,
        mtime: UNIX_EPOCH + Duration::new(1_800_000_000, 123_456_789),
        hash: u128::MAX - 7,
    }
}

fn texts(found: &[Recovered]) -> Vec<(String, String)> {
    let mut v: Vec<_> = found
        .iter()
        .map(|r| (r.rel.as_str().to_owned(), r.text.clone()))
        .collect();
    v.sort();
    v
}

#[test]
fn recovery_lives_outside_the_library_and_is_private() {
    let (paths, app, lib) = env();
    store(&paths, lib_id(), &rel("a.sql"), "x", None).unwrap();
    // Nothing was written into the library (L14).
    assert_eq!(walk(lib.path()), [lib.path().join("a.sql")]);
    assert_eq!(fs::read(lib.path().join("a.sql")).unwrap(), b"SELECT 1;");
    let dir = dir_of(&paths);
    assert!(dir.starts_with(app.path().join("data").join("script-recovery")));
    let files = walk(&dir);
    assert_eq!(files.len(), 2, "{files:?}: the copy and its sidecar");
    assert!(files.iter().all(|p| p.parent() == Some(dir.as_path())));
    // The name says nothing about the script: the XXH3-128 of its path, in hex.
    let want = format!("{:032x}", twox_hash::XxHash3_128::oneshot(b"a.sql"));
    let names: Vec<String> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, [format!("{want}.meta"), format!("{want}.sql")]);
    assert_eq!(fs::read(dir.join(format!("{want}.sql"))).unwrap(), b"x");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        for f in &files {
            assert_eq!(mode(f), 0o600, "{f:?}");
        }
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(dir.parent().unwrap()), 0o700);
    }
}

#[test]
fn recovery_round_trip_and_discard() {
    let (paths, _app, _lib) = env();
    let before = SystemTime::now();
    store(
        &paths,
        lib_id(),
        &rel("a.sql"),
        "SELECT 'ğüş';\r\n",
        Some(stamp()),
    )
    .unwrap();
    store(&paths, lib_id(), &rel("@dev/b.md"), "notes", None).unwrap();
    // The newer copy of a path replaces the older one.
    store(&paths, lib_id(), &rel("@dev/b.md"), "notes 2", None).unwrap();
    let found = take_all(&paths, lib_id());
    assert_eq!(
        texts(&found),
        [
            ("@dev/b.md".into(), "notes 2".into()),
            ("a.sql".into(), "SELECT 'ğüş';\r\n".into()),
        ]
    );
    let a = found.iter().find(|r| r.rel.as_str() == "a.sql").unwrap();
    assert_eq!(a.base, Some(stamp()));
    assert!(a.saved_at >= before - Duration::from_secs(1));
    let b = found
        .iter()
        .find(|r| r.rel.as_str() == "@dev/b.md")
        .unwrap();
    assert_eq!(b.base, None);
    // Offering them back does not delete them: a second crash still loses nothing.
    assert_eq!(texts(&take_all(&paths, lib_id())).len(), 2);
    // Another library sees none of them.
    assert!(take_all(&paths, LibraryId::new()).is_empty());
    discard(&paths, lib_id(), &rel("a.sql")).unwrap();
    assert_eq!(
        texts(&take_all(&paths, lib_id())),
        [("@dev/b.md".into(), "notes 2".into())]
    );
    assert_eq!(walk(&dir_of(&paths)).len(), 2);
    // Discarding what is not there is fine.
    discard(&paths, lib_id(), &rel("a.sql")).unwrap();
    discard(&paths, lib_id(), &rel("@dev/b.md")).unwrap();
    assert!(take_all(&paths, lib_id()).is_empty());
    assert!(walk(&dir_of(&paths)).is_empty());
}

#[test]
fn nothing_to_take_when_nothing_was_stored() {
    let (paths, app, _lib) = env();
    assert!(take_all(&paths, lib_id()).is_empty());
    discard(&paths, lib_id(), &rel("a.sql")).unwrap();
    // Neither call creates a folder.
    assert!(walk(app.path()).is_empty());
}

#[test]
fn damaged_or_foreign_entries_are_skipped() {
    let (paths, _app, _lib) = env();
    store(&paths, lib_id(), &rel("good.sql"), "kept", None).unwrap();
    let dir = dir_of(&paths);
    let hex = |s: &str| format!("{:032x}", twox_hash::XxHash3_128::oneshot(s.as_bytes()));
    // A sidecar whose path does not hash to its file name (renamed or edited by hand).
    store(&paths, lib_id(), &rel("x.sql"), "moved", None).unwrap();
    fs::rename(
        dir.join(format!("{}.meta", hex("x.sql"))),
        dir.join(format!("{}.meta", hex("y.sql"))),
    )
    .unwrap();
    fs::rename(
        dir.join(format!("{}.sql", hex("x.sql"))),
        dir.join(format!("{}.sql", hex("y.sql"))),
    )
    .unwrap();
    // A path that is not library-relative, under its own hash.
    let evil = "../../etc/passwd";
    fs::write(
        dir.join(format!("{}.meta", hex(evil))),
        format!("format = 1\nrel_path = \"{evil}\"\nsaved_at_secs = 1\nsaved_at_nanos = 0\n"),
    )
    .unwrap();
    fs::write(dir.join(format!("{}.sql", hex(evil))), b"x").unwrap();
    // Not TOML; a sidecar without its copy; a copy that is not UTF-8; a newer format.
    fs::write(dir.join(format!("{}.meta", hex("n.sql"))), b"[[[").unwrap();
    fs::write(dir.join(format!("{}.sql", hex("n.sql"))), b"x").unwrap();
    store(&paths, lib_id(), &rel("lone.sql"), "x", None).unwrap();
    fs::remove_file(dir.join(format!("{}.sql", hex("lone.sql")))).unwrap();
    store(&paths, lib_id(), &rel("bin.sql"), "x", None).unwrap();
    fs::write(dir.join(format!("{}.sql", hex("bin.sql"))), b"\xFF\xFE").unwrap();
    fs::write(
        dir.join(format!("{}.meta", hex("v2.sql"))),
        "format = 2\nrel_path = \"v2.sql\"\nsaved_at_secs = 1\nsaved_at_nanos = 0\n",
    )
    .unwrap();
    fs::write(dir.join(format!("{}.sql", hex("v2.sql"))), b"x").unwrap();
    // Stray files.
    fs::write(dir.join("notes.txt"), b"x").unwrap();
    fs::create_dir(dir.join("sub")).unwrap();
    assert_eq!(
        texts(&take_all(&paths, lib_id())),
        [("good.sql".into(), "kept".into())]
    );
}

#[test]
fn recovered_debug_never_shows_the_text() {
    let (paths, _app, _lib) = env();
    store(&paths, lib_id(), &rel("a.sql"), "password = 's3cret'", None).unwrap();
    let found = take_all(&paths, lib_id());
    let dbg = format!("{found:?}");
    assert!(!dbg.contains("s3cret"), "{dbg}");
    assert!(dbg.contains("a.sql"), "{dbg}");
}

#[cfg(unix)]
#[test]
fn an_existing_folder_is_made_private() {
    use std::os::unix::fs::PermissionsExt;
    let (paths, _app, _lib) = env();
    let dir = dir_of(&paths);
    fs::create_dir_all(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(dir.parent().unwrap(), fs::Permissions::from_mode(0o755)).unwrap();
    store(&paths, lib_id(), &rel("a.sql"), "x", None).unwrap();
    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(dir.parent().unwrap()), 0o700);
}
