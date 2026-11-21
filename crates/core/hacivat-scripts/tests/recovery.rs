//! M4b Task 5: recovery copies of unsaved script text (HV-SCRIPT-07 step 5; R-M4b-5, Q1).
//! They live under the app's data folder, never inside a library (L14), 0700/0600.
//!
//! ## m4b-t05-fileio fix round 1 (I3, m7; rulings R-T5-3, R-T5-4): copies are keyed by the
//! tab's binding and by the app instance; a running instance's copies are never offered or
//! deleted by another.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hacivat_proto::ProfileId;
use hacivat_scripts::recovery::{
    CopyMeta, Instance, RecoveryEntry, RecoveryError, RecoveryKey, discard, discard_entry,
    discard_library, store, store_at, take_all, with_locks_unsupported,
};
use hacivat_scripts::{
    DatabaseName, DiskStamp, FileMeta, LibraryId, LineEnding, RelPath, TextEncoding,
};
use hacivat_settings::LocalPaths;

fn rel(s: &str) -> RelPath {
    RelPath::parse(s).unwrap()
}

fn lib_id() -> LibraryId {
    LibraryId(uuid::Uuid::from_u128(
        0x5ca1ab1e_0000_4000_8000_000000000001,
    ))
}

fn key(path: &str) -> RecoveryKey {
    RecoveryKey {
        library: lib_id(),
        rel: rel(path),
        profile: None,
        database: None,
    }
}

fn bound(path: &str, profile: ProfileId, db: &str) -> RecoveryKey {
    RecoveryKey {
        profile: Some(profile),
        database: Some(DatabaseName::new(db)),
        ..key(path)
    }
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
        let Ok(entries) = fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
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

fn plain() -> CopyMeta {
    CopyMeta::default()
}

fn texts(found: &[RecoveryEntry]) -> Vec<(String, String)> {
    let mut v: Vec<_> = found
        .iter()
        .map(|r| (r.key.rel.as_str().to_owned(), r.text.clone()))
        .collect();
    v.sort();
    v
}

#[test]
fn recovery_lives_outside_the_library_and_is_private() {
    let (paths, app, lib) = env();
    let me = Instance::start(&paths).unwrap();
    store(&paths, &me, &key("a.sql"), "x", &plain()).unwrap();
    // Nothing was written into the library (L14).
    assert_eq!(walk(lib.path()), [lib.path().join("a.sql")]);
    assert_eq!(fs::read(lib.path().join("a.sql")).unwrap(), b"SELECT 1;");
    let dir = dir_of(&paths);
    assert!(dir.starts_with(app.path().join("data").join("script-recovery")));
    let files = walk(&dir);
    assert_eq!(files.len(), 2, "{files:?}: the copy and its sidecar");
    // The names say nothing about the script: 32 hex digits.
    for f in &files {
        let stem = f.file_stem().unwrap().to_str().unwrap();
        assert_eq!(stem.len(), 32);
        assert!(stem.bytes().all(|b| b.is_ascii_hexdigit()), "{stem}");
        // (The stem is hex only, so it holds no part of the script's name. A substring check
        // on the whole path flaked: a stem ending in `a` before `.sql` contains "a.sql".)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let lock = walk(&paths.script_recovery_dir("instances").unwrap());
        for f in files.iter().chain(&lock) {
            assert_eq!(mode(f), 0o600, "{f:?}");
        }
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(dir.parent().unwrap()), 0o700);
    }
}

/// alg/28's name for the test above.
#[test]
fn recovery_copy_is_outside_library_and_0600() {
    recovery_lives_outside_the_library_and_is_private();
}

#[test]
fn recovery_round_trip_and_discard() {
    let (paths, _app, _lib) = env();
    let before = SystemTime::now();
    let crashed = Instance::start(&paths).unwrap();
    let tr = CopyMeta {
        meta: FileMeta {
            encoding: hacivat_scripts::fileio::lookup_encoding("windows-1254").unwrap(),
            bom: false,
            line_ending: LineEnding::CrLf,
            partial: false,
        },
        base: Some(stamp()),
        secret_suspected: true,
        damaged: false,
    };
    store(&paths, &crashed, &key("a.sql"), "SELECT 'ğüş';\r\n", &tr).unwrap();
    store(&paths, &crashed, &key("@dev/b.md"), "notes", &plain()).unwrap();
    // The newer copy of a tab replaces its older one.
    store(&paths, &crashed, &key("@dev/b.md"), "notes 2", &plain()).unwrap();
    // While that instance runs, another one sees none of its copies.
    let next = Instance::start(&paths).unwrap();
    assert!(take_all(&paths, &next, lib_id()).is_empty());
    drop(crashed);
    let found = take_all(&paths, &next, lib_id());
    assert_eq!(
        texts(&found),
        [
            ("@dev/b.md".into(), "notes 2".into()),
            ("a.sql".into(), "SELECT 'ğüş';\r\n".into()),
        ]
    );
    let a = found
        .iter()
        .find(|r| r.key.rel.as_str() == "a.sql")
        .unwrap();
    // m7: the encoding, BOM and line ending come back with the stamp and the flags.
    assert_eq!(a.copy, tr);
    assert_eq!(a.copy.meta.encoding.name(), "windows-1254");
    assert!(a.saved_at >= before - Duration::from_secs(1));
    let b = found
        .iter()
        .find(|r| r.key.rel.as_str() == "@dev/b.md")
        .unwrap();
    assert_eq!(b.copy.base, None);
    assert_eq!(b.copy.meta.encoding, TextEncoding::UTF_8);
    // Offering them back does not delete them: a second crash still loses nothing.
    assert_eq!(take_all(&paths, &next, lib_id()).len(), 2);
    // Another library sees none of them.
    assert!(take_all(&paths, &next, LibraryId::new()).is_empty());
    discard_entry(&paths, a).unwrap();
    assert_eq!(
        texts(&take_all(&paths, &next, lib_id())),
        [("@dev/b.md".into(), "notes 2".into())]
    );
    assert_eq!(walk(&dir_of(&paths)).len(), 2);
    discard_entry(&paths, b).unwrap();
    discard_entry(&paths, b).unwrap();
    assert!(take_all(&paths, &next, lib_id()).is_empty());
    assert!(walk(&dir_of(&paths)).is_empty());
}

/// I3: two tabs of one file under two connections (Q2) keep two copies; a save in one removes
/// only its own.
#[test]
fn two_dirty_tabs_of_one_file_keep_two_copies() {
    let (paths, _app, _lib) = env();
    let me = Instance::start(&paths).unwrap();
    let (p1, p2) = (ProfileId::new(), ProfileId::new());
    let tab_a = bound("cleanup.sql", p1, "orders");
    let tab_b = bound("cleanup.sql", p2, "orders");
    let unbound = key("cleanup.sql");
    store(&paths, &me, &tab_a, "DELETE FROM a;", &plain()).unwrap();
    store(&paths, &me, &tab_b, "DELETE FROM b;", &plain()).unwrap();
    store(&paths, &me, &unbound, "-- draft", &plain()).unwrap();
    // Tab A saves: only its copy goes.
    discard(&paths, &me, &tab_a).unwrap();
    drop(me);
    let next = Instance::start(&paths).unwrap();
    let mut found = take_all(&paths, &next, lib_id());
    found.sort_by(|x, y| x.text.cmp(&y.text));
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].text, "-- draft");
    assert_eq!(found[0].key, unbound);
    assert_eq!(found[1].text, "DELETE FROM b;");
    assert_eq!(found[1].key, tab_b);
}

/// I3: two instances at once (no single-instance lock before M8): one never offers or deletes
/// the other's copies.
#[test]
fn a_running_instance_keeps_its_copies() {
    let (paths, _app, _lib) = env();
    let first = Instance::start(&paths).unwrap();
    let second = Instance::start(&paths).unwrap();
    assert_ne!(first.id(), second.id());
    store(&paths, &first, &key("a.sql"), "first", &plain()).unwrap();
    store(&paths, &second, &key("a.sql"), "second", &plain()).unwrap();
    assert!(take_all(&paths, &second, lib_id()).is_empty());
    assert!(take_all(&paths, &first, lib_id()).is_empty());
    // The second one's "Don't save" leaves the first one's copy alone.
    discard(&paths, &second, &key("a.sql")).unwrap();
    drop(first);
    assert_eq!(
        texts(&take_all(&paths, &second, lib_id())),
        [("a.sql".into(), "first".into())]
    );
}

/// R-T5-4: detaching a library deletes its copies; copies older than 14 days are not offered.
#[test]
fn detached_libraries_and_old_copies_leave_nothing() {
    let (paths, _app, _lib) = env();
    let gone = Instance::start(&paths).unwrap();
    store(&paths, &gone, &key("a.sql"), "x", &plain()).unwrap();
    let old = SystemTime::now() - Duration::from_secs(15 * 24 * 60 * 60);
    store_at(&paths, &gone, &key("old.sql"), "stale", &plain(), old).unwrap();
    let recent = SystemTime::now() - Duration::from_secs(13 * 24 * 60 * 60);
    store_at(&paths, &gone, &key("recent.sql"), "kept", &plain(), recent).unwrap();
    drop(gone);
    let me = Instance::start(&paths).unwrap();
    assert_eq!(
        texts(&take_all(&paths, &me, lib_id())),
        [
            ("a.sql".into(), "x".into()),
            ("recent.sql".into(), "kept".into())
        ]
    );
    // The expired copy is deleted, not only hidden.
    assert_eq!(walk(&dir_of(&paths)).len(), 4);
    discard_library(&paths, lib_id()).unwrap();
    assert!(!dir_of(&paths).exists());
    assert!(take_all(&paths, &me, lib_id()).is_empty());
    discard_library(&paths, lib_id()).unwrap();
}

/// R-T5-1: the text of a save that damaged the file is kept with the mark.
#[test]
fn a_damaged_save_is_marked() {
    let (paths, _app, _lib) = env();
    let gone = Instance::start(&paths).unwrap();
    let copy = CopyMeta {
        damaged: true,
        ..plain()
    };
    store(&paths, &gone, &key("a.sql"), "new text", &copy).unwrap();
    drop(gone);
    let me = Instance::start(&paths).unwrap();
    let found = take_all(&paths, &me, lib_id());
    assert!(found[0].copy.damaged);
}

#[test]
fn nothing_to_take_when_nothing_was_stored() {
    let (paths, app, _lib) = env();
    let me = Instance::start(&paths).unwrap();
    let before = walk(app.path());
    assert!(take_all(&paths, &me, lib_id()).is_empty());
    discard(&paths, &me, &key("a.sql")).unwrap();
    // Neither call creates a folder.
    assert_eq!(walk(app.path()), before);
}

#[test]
fn damaged_or_foreign_entries_are_skipped() {
    let (paths, _app, _lib) = env();
    let gone = Instance::start(&paths).unwrap();
    store(&paths, &gone, &key("good.sql"), "kept", &plain()).unwrap();
    store(&paths, &gone, &key("x.sql"), "moved", &plain()).unwrap();
    store(&paths, &gone, &key("lone.sql"), "x", &plain()).unwrap();
    store(&paths, &gone, &key("bin.sql"), "x", &plain()).unwrap();
    store(&paths, &gone, &key("v9.sql"), "x", &plain()).unwrap();
    store(&paths, &gone, &key("evil.sql"), "x", &plain()).unwrap();
    drop(gone);
    let dir = dir_of(&paths);
    // Find each copy by the path its sidecar names.
    let stem_for = |rel: &str| -> String {
        for p in walk(&dir) {
            if p.extension().and_then(|e| e.to_str()) == Some("meta")
                && fs::read_to_string(&p)
                    .unwrap()
                    .contains(&format!("rel_path = \"{rel}\""))
            {
                return p.file_stem().unwrap().to_str().unwrap().to_owned();
            }
        }
        panic!("no copy for {rel}");
    };
    let edit = |rel: &str, from: &str, to: &str| {
        let p = dir.join(format!("{}.meta", stem_for(rel)));
        let t = fs::read_to_string(&p).unwrap();
        assert!(t.contains(from), "{t}");
        fs::write(&p, t.replace(from, to)).unwrap();
    };
    // A sidecar whose binding no longer hashes to its file name (edited by hand).
    edit("x.sql", "rel_path = \"x.sql\"", "rel_path = \"y.sql\"");
    // A path that is not library-relative.
    edit(
        "evil.sql",
        "rel_path = \"evil.sql\"",
        "rel_path = \"../../etc/passwd\"",
    );
    // A newer format.
    edit("v9.sql", "format = 2", "format = 9");
    // A sidecar without its copy; a copy that is not UTF-8.
    fs::remove_file(dir.join(format!("{}.sql", stem_for("lone.sql")))).unwrap();
    fs::write(
        dir.join(format!("{}.sql", stem_for("bin.sql"))),
        b"\xFF\xFE",
    )
    .unwrap();
    // Not TOML; stray files.
    fs::write(dir.join(format!("{}.meta", "0".repeat(32))), b"[[[").unwrap();
    fs::write(dir.join("notes.txt"), b"x").unwrap();
    fs::create_dir(dir.join("sub")).unwrap();
    let me = Instance::start(&paths).unwrap();
    assert_eq!(
        texts(&take_all(&paths, &me, lib_id())),
        [("good.sql".into(), "kept".into())]
    );
}

#[test]
fn recovered_debug_never_shows_the_text() {
    let (paths, _app, _lib) = env();
    let gone = Instance::start(&paths).unwrap();
    store(
        &paths,
        &gone,
        &key("a.sql"),
        "password = 's3cret'",
        &plain(),
    )
    .unwrap();
    drop(gone);
    let me = Instance::start(&paths).unwrap();
    let found = take_all(&paths, &me, lib_id());
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
    let me = Instance::start(&paths).unwrap();
    store(&paths, &me, &key("a.sql"), "x", &plain()).unwrap();
    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(dir.parent().unwrap()), 0o700);
}

/// fr2 mf3 (R-T5-5): where advisory locks are unsupported, the instance still starts and still
/// writes copies (a crash loses nothing); a later launch offers them marked "liveness unknown".
#[test]
fn without_advisory_locks_copies_are_kept_and_offered_marked() {
    let (paths, _app, _lib) = env();
    let unlocked = with_locks_unsupported(|| Instance::start(&paths)).unwrap();
    assert!(!unlocked.is_locked());
    store(&paths, &unlocked, &key("a.sql"), "kept", &plain()).unwrap();
    // Its lock file is not locked while it runs, so a lock test would read "dead": the
    // sidecar's mark keeps the copy from passing as a dead instance's.
    let other = Instance::start(&paths).unwrap();
    assert!(other.is_locked());
    let found = take_all(&paths, &other, lib_id());
    assert_eq!(texts(&found), [("a.sql".into(), "kept".into())]);
    assert!(found[0].liveness_unknown);
    // A locked instance's copies, seen where locks cannot be tested, are offered marked too:
    // the double offer of two instances starting together is the M8 single-instance carry.
    store(&paths, &other, &key("b.sql"), "theirs", &plain()).unwrap();
    let third = Instance::start(&paths).unwrap();
    let seen = with_locks_unsupported(|| take_all(&paths, &third, lib_id()));
    let b = seen.iter().find(|e| e.key.rel.as_str() == "b.sql").unwrap();
    assert!(b.liveness_unknown);
    // A lockable dead instance's copy is not marked.
    drop(other);
    let after = take_all(&paths, &third, lib_id());
    let b = after
        .iter()
        .find(|e| e.key.rel.as_str() == "b.sql")
        .unwrap();
    assert!(!b.liveness_unknown);
}

/// fr2 I-fr1-2: a preview's meta is never stored.
#[test]
fn a_preview_meta_is_not_stored() {
    let (paths, _app, _lib) = env();
    let me = Instance::start(&paths).unwrap();
    let copy = CopyMeta {
        meta: FileMeta {
            partial: true,
            ..FileMeta::default()
        },
        ..plain()
    };
    assert_eq!(
        store(&paths, &me, &key("a.sql"), "x", &copy),
        Err(RecoveryError::PreviewOnly)
    );
    assert!(!dir_of(&paths).exists());
}
