//! M4b Task 3: the library registry `<config>/libraries.toml` (HV-SCRIPT-01 steps 1, 2, 5;
//! R-M4b-2, -14). `LibraryRegistry` is its one writer.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hacivat_proto::ProfileId;
use hacivat_scripts::registry::{
    AttachGuard, LibraryRegistry, MAX_LIBRARIES, RegistryDiag, RowProblem,
};
use hacivat_scripts::{DatabaseName, LibraryId, LocalMapping, RegistryError};
use hacivat_settings::LocalPaths;

struct App {
    _dir: tempfile::TempDir,
    paths: LocalPaths,
}

fn app() -> App {
    let dir = tempfile::tempdir().unwrap();
    let paths = LocalPaths::under(dir.path());
    App { _dir: dir, paths }
}

impl App {
    fn file(&self) -> std::path::PathBuf {
        self.paths.libraries_toml()
    }
    fn guard(&self) -> AttachGuard {
        AttachGuard::new(&self.paths, None)
    }
    fn load(&self) -> (LibraryRegistry, Vec<RegistryDiag>) {
        LibraryRegistry::load(&self.file())
    }
    fn write(&self, text: &str) {
        fs::create_dir_all(&self.paths.config).unwrap();
        fs::write(self.file(), text).unwrap();
    }
    /// Names in the config folder (set-aside and temp files show here).
    fn config_names(&self) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(&self.paths.config)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }
}

fn mapping(profile: ProfileId, database: &str, library: LibraryId) -> LocalMapping {
    LocalMapping {
        profile,
        database: DatabaseName::new(database),
        library,
    }
}

/// An absolute path in TOML text on this platform (`/libs/a`, `C:/libs/a`).
fn abs(rel: &str) -> String {
    if cfg!(windows) {
        format!("C:/{rel}")
    } else {
        format!("/{rel}")
    }
}

fn t0() -> SystemTime {
    // 2026-11-22T10:30:05Z
    UNIX_EPOCH + Duration::from_secs(1_795_343_405)
}

#[test]
fn missing_registry_is_empty_and_writable() {
    let a = app();
    let (reg, diags) = a.load();
    assert!(diags.is_empty());
    assert!(reg.libraries().is_empty() && reg.mappings().is_empty());
    assert!(!reg.is_read_only());
    // Loading creates nothing.
    assert!(!a.paths.config.exists());
}

#[test]
fn registry_round_trips() {
    let a = app();
    let lib = tempfile::tempdir().unwrap();
    let (mut reg, _) = a.load();
    let root = reg.add(lib.path(), &a.guard(), t0()).unwrap();
    assert_eq!(root.path, lib.path());
    assert_eq!(root.canonical, lib.path().canonicalize().unwrap());
    let p = ProfileId::new();
    reg.set_mapping(mapping(p, "orders_p", root.id)).unwrap();
    reg.save().unwrap();

    let text = fs::read_to_string(a.file()).unwrap();
    assert!(text.starts_with("version = 1\n"), "{text}");
    assert!(
        text.contains("added_at = \"2026-11-22T10:30:05Z\""),
        "{text}"
    );
    assert!(text.contains(&format!("id = \"{}\"", root.id)), "{text}");

    let (back, diags) = a.load();
    assert!(diags.is_empty(), "{diags:?}");
    assert_eq!(back.libraries().len(), 1);
    let entry = &back.libraries()[0];
    assert_eq!(entry.id, root.id);
    assert_eq!(entry.path, lib.path());
    assert_eq!(entry.added_at, "2026-11-22T10:30:05Z");
    assert_eq!(entry.root().unwrap(), root);
    assert_eq!(back.library(root.id).unwrap().id, root.id);
    assert_eq!(back.mappings(), [mapping(p, "orders_p", root.id)]);
    assert_eq!(
        back.mappings_for(p).cloned().collect::<Vec<_>>(),
        [mapping(p, "orders_p", root.id)]
    );
    assert_eq!(back.mappings_for(ProfileId::new()).count(), 0);
}

#[test]
fn set_mapping_replaces_the_nodes_mapping_and_needs_a_library() {
    let a = app();
    let (l1, l2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (mut reg, _) = a.load();
    let r1 = reg.add(l1.path(), &a.guard(), t0()).unwrap();
    let r2 = reg.add(l2.path(), &a.guard(), t0()).unwrap();
    let p = ProfileId::new();
    reg.set_mapping(mapping(p, "orders", r1.id)).unwrap();
    reg.set_mapping(mapping(p, "orders", r2.id)).unwrap();
    // The database name is exact: `Orders` is another node.
    reg.set_mapping(mapping(p, "Orders", r1.id)).unwrap();
    assert_eq!(
        reg.mappings(),
        [mapping(p, "orders", r2.id), mapping(p, "Orders", r1.id)]
    );
    assert_eq!(
        reg.set_mapping(mapping(p, "x", LibraryId::new())),
        Err(RegistryError::UnknownLibrary)
    );
    assert!(reg.remove_mapping(p, &DatabaseName::new("Orders")));
    assert!(!reg.remove_mapping(p, &DatabaseName::new("Orders")));
    // Removing a library drops its mappings; the folder is never touched.
    fs::write(l2.path().join("a.sql"), "SELECT 1;").unwrap();
    reg.remove(r2.id).unwrap();
    assert!(reg.mappings().is_empty());
    assert_eq!(reg.libraries().len(), 1);
    assert_eq!(reg.remove(r2.id), Err(RegistryError::UnknownLibrary));
    assert_eq!(
        fs::read_to_string(l2.path().join("a.sql")).unwrap(),
        "SELECT 1;"
    );
}

/// alg/28 `registry_is_0600_and_atomic` and the plan's
/// `registry_file_is_0600_and_holds_no_profile_secret`.
#[test]
fn registry_file_is_0600_and_holds_no_profile_secret() {
    let a = app();
    let lib = tempfile::tempdir().unwrap();
    let (mut reg, _) = a.load();
    let root = reg.add(lib.path(), &a.guard(), t0()).unwrap();
    reg.set_mapping(mapping(ProfileId::new(), "orders", root.id))
        .unwrap();
    reg.save().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(a.file()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dir = fs::metadata(&a.paths.config).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir, 0o700);
    }
    // By type: every key in the file is one of these, so no host, user, keyring slot or
    // password can be there.
    let table: toml::Table = fs::read_to_string(a.file()).unwrap().parse().unwrap();
    let mut keys: Vec<String> = Vec::new();
    for (k, v) in &table {
        keys.push(k.clone());
        if let Some(rows) = v.as_array() {
            for row in rows {
                for rk in row.as_table().unwrap().keys() {
                    keys.push(format!("{k}.{rk}"));
                }
            }
        }
    }
    keys.sort();
    keys.dedup();
    assert_eq!(
        keys,
        [
            "library",
            "library.added_at",
            "library.id",
            "library.path",
            "mapping",
            "mapping.database",
            "mapping.library",
            "mapping.profile",
            "version"
        ]
    );
    // Atomic: no temp file is left beside it, and a second save replaces it whole.
    assert_eq!(a.config_names(), ["libraries.toml"]);
    reg.remove(root.id).unwrap();
    reg.save().unwrap();
    assert_eq!(a.config_names(), ["libraries.toml"]);
    assert_eq!(fs::read_to_string(a.file()).unwrap(), "version = 1\n");
}

// ## m4b-t03-registry fix round 1

/// alg/28 `registry_is_0600_and_atomic` (review m10): the file is 0600 after every save, a
/// save replaces it whole through a rename (a hard link to the old file keeps the old bytes),
/// and no temp file stays.
#[test]
fn registry_is_0600_and_atomic() {
    let a = app();
    let lib = tempfile::tempdir().unwrap();
    let (mut reg, _) = a.load();
    let root = reg.add(lib.path(), &a.guard(), t0()).unwrap();
    reg.save().unwrap();
    let first = fs::read_to_string(a.file()).unwrap();
    let kept = a.paths.data.join("old-registry");
    fs::create_dir_all(&a.paths.data).unwrap();
    fs::hard_link(a.file(), &kept).unwrap();
    reg.set_mapping(mapping(ProfileId::new(), "orders", root.id))
        .unwrap();
    reg.save().unwrap();
    // Renamed into place: the old inode still holds the old text.
    assert_eq!(fs::read_to_string(&kept).unwrap(), first);
    assert!(
        fs::read_to_string(a.file())
            .unwrap()
            .contains("[[mapping]]")
    );
    assert_eq!(a.config_names(), ["libraries.toml"]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(a.file()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}

/// Review m5 (alg/28 HV-SCRIPT-01 edge cases): a case-only different spelling on a
/// case-insensitive volume, and the NFD spelling of an NFC folder on a normalization-insensitive
/// one, are the same library. Where the volume tells them apart, they are different folders.
#[test]
fn case_only_and_nfd_spellings_are_the_same_library() {
    let a = app();
    let top = tempfile::tempdir().unwrap();
    fs::create_dir(top.path().join("Lib")).unwrap();
    fs::create_dir(top.path().join("Lib/sub")).unwrap();
    let (mut reg, _) = a.load();
    let lib = reg.add(&top.path().join("Lib"), &a.guard(), t0()).unwrap();
    if top.path().join("lib").exists() {
        assert_eq!(
            reg.add(&top.path().join("lib"), &a.guard(), t0())
                .map(|_| ()),
            Err(RegistryError::AlreadyRegistered(lib.id))
        );
        assert_eq!(
            reg.add(&top.path().join("LIB/sub"), &a.guard(), t0())
                .map(|_| ()),
            Err(RegistryError::Nested(lib.id))
        );
    }
    let nfc = top.path().join("m\u{fc}\u{15f}teri");
    let nfd = top.path().join("mu\u{308}s\u{327}teri");
    fs::create_dir(&nfc).unwrap();
    let first = reg.add(&nfc, &a.guard(), t0()).unwrap();
    if nfd.exists() {
        assert_eq!(
            reg.add(&nfd, &a.guard(), t0()).map(|_| ()),
            Err(RegistryError::AlreadyRegistered(first.id))
        );
    } else {
        // A normalization-sensitive volume: the NFD name is another (missing) folder.
        assert!(matches!(
            reg.add(&nfd, &a.guard(), t0()),
            Err(RegistryError::Io(_))
        ));
    }
}

/// Review m6: a hand-edited registry with the same folder under two spellings, or a folder
/// inside another library, keeps the first row only.
#[test]
fn load_drops_rows_for_the_same_or_a_nested_folder() {
    let a = app();
    let top = tempfile::tempdir().unwrap();
    for d in ["team", "team/inner", "other"] {
        fs::create_dir_all(top.path().join(d)).unwrap();
    }
    let p = |rel: &str| top.path().join(rel).to_str().unwrap().replace('\\', "/");
    let ids: Vec<LibraryId> = (0..5).map(|_| LibraryId::new()).collect();
    let rows = [
        p("team"),
        p("team/inner/.."),
        p("team/inner"),
        p("other"),
        top.path().to_str().unwrap().replace('\\', "/"),
    ];
    let mut text = String::from("version = 1\n");
    for (id, path) in ids.iter().zip(&rows) {
        text.push_str(&format!("[[library]]\nid = \"{id}\"\npath = \"{path}\"\n"));
    }
    a.write(&text);
    let (reg, diags) = a.load();
    assert_eq!(
        reg.libraries().iter().map(|l| l.id).collect::<Vec<_>>(),
        [ids[0], ids[3]]
    );
    assert_eq!(
        diags,
        [
            RegistryDiag::LibraryDropped {
                index: 1,
                problem: RowProblem::Duplicate
            },
            RegistryDiag::LibraryDropped {
                index: 2,
                problem: RowProblem::Nested
            },
            RegistryDiag::LibraryDropped {
                index: 4,
                problem: RowProblem::Nested
            },
        ]
    );
}

/// Review m7: a FIFO (or any non-regular file) named `libraries.toml` is never opened, so it
/// cannot hang the start; the registry is read-only and the file is left alone.
#[cfg(unix)]
#[test]
fn registry_that_is_not_a_regular_file_is_never_opened() {
    use std::os::unix::fs::FileTypeExt;
    let a = app();
    fs::create_dir_all(&a.paths.config).unwrap();
    let made = std::process::Command::new("mkfifo")
        .arg(a.file())
        .status()
        .unwrap();
    assert!(made.success());
    let (reg, diags) = a.load();
    assert_eq!(diags, [RegistryDiag::NotAFile]);
    assert!(reg.is_read_only());
    assert_eq!(reg.save(), Err(RegistryError::ReadOnly));
    assert!(
        fs::symlink_metadata(a.file())
            .unwrap()
            .file_type()
            .is_fifo()
    );
    // A folder of that name too.
    let a = app();
    fs::create_dir_all(a.file()).unwrap();
    assert_eq!(a.load().1, [RegistryDiag::NotAFile]);
}
// ## m4b-t03-registry fix round 1

#[test]
fn registry_load_of_garbage_sets_it_aside() {
    for garbage in [
        "this is not toml [[[",
        "version = \"one\"\n",
        "library = 3\nversion = 1\n",
        "version = 1\nlibrary = [1, 2]\n",
        "[[library]]\nid = \"x\"\n",
    ] {
        let a = app();
        a.write(garbage);
        let (reg, diags) = a.load();
        assert!(reg.libraries().is_empty(), "{garbage:?}");
        assert!(!reg.is_read_only(), "{garbage:?}");
        let [RegistryDiag::SetAside(moved)] = diags.as_slice() else {
            panic!("{garbage:?}: {diags:?}");
        };
        assert_eq!(fs::read_to_string(moved).unwrap(), garbage);
        assert!(!a.file().exists());
        let names = a.config_names();
        assert_eq!(names.len(), 1);
        assert!(names[0].starts_with("libraries.toml.corrupt-"), "{names:?}");
    }
    // Nested deep enough to exhaust a parser's stack: set aside unparsed.
    let a = app();
    a.write(&format!("version = 1\nx = {}", "[".repeat(200_000)));
    assert!(matches!(a.load().1.as_slice(), [RegistryDiag::SetAside(_)]));
    // Not UTF-8.
    let a = app();
    fs::create_dir_all(&a.paths.config).unwrap();
    fs::write(a.file(), b"version = 1\n\xFF\n").unwrap();
    assert!(matches!(a.load().1.as_slice(), [RegistryDiag::SetAside(_)]));
}

#[test]
fn newer_registry_is_read_only_and_never_rewritten() {
    let a = app();
    let text =
        "version = 2\n[[library]]\nid = \"6f1c2f0e-9a51-4a43-9d2b-0d6e7c1c9a10\"\npath = \"/x\"\n";
    a.write(text);
    let (mut reg, diags) = a.load();
    assert_eq!(diags, [RegistryDiag::NewerVersion(2)]);
    assert!(reg.is_read_only());
    assert!(reg.libraries().is_empty());
    let lib = tempfile::tempdir().unwrap();
    assert_eq!(
        reg.add(lib.path(), &a.guard(), t0()).map(|_| ()),
        Err(RegistryError::ReadOnly)
    );
    assert_eq!(reg.save(), Err(RegistryError::ReadOnly));
    assert_eq!(fs::read_to_string(a.file()).unwrap(), text);
}

#[cfg(unix)]
#[test]
fn unreadable_registry_is_read_only_and_left_alone() {
    use std::os::unix::fs::PermissionsExt;
    let a = app();
    a.write("version = 1\n");
    fs::set_permissions(a.file(), fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read(a.file()).is_ok() {
        return; // running as root: permissions do not apply
    }
    let (reg, diags) = a.load();
    assert!(matches!(diags.as_slice(), [RegistryDiag::Unreadable(_)]));
    assert!(reg.is_read_only());
    assert_eq!(reg.save(), Err(RegistryError::ReadOnly));
    fs::set_permissions(a.file(), fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(fs::read_to_string(a.file()).unwrap(), "version = 1\n");
    assert_eq!(a.config_names(), ["libraries.toml"]);
}

#[test]
fn bad_rows_are_dropped_with_a_diagnostic() {
    let a = app();
    let good = LibraryId::new();
    let other = LibraryId::new();
    let p = ProfileId::new();
    let (a_, b_, d_) = (abs("libs/a"), abs("libs/b"), abs("libs/d"));
    let text = format!(
        "version = 1\n\
         [[library]]\nid = \"{good}\"\npath = \"{a_}\"\nadded_at = \"2026-01-01T00:00:00Z\"\n\
         [[library]]\nid = \"not-a-uuid\"\npath = \"{b_}\"\n\
         [[library]]\nid = \"{other}\"\npath = \"relative/c\"\n\
         [[library]]\nid = \"{good}\"\npath = \"{d_}\"\n\
         [[library]]\nid = \"{other}\"\npath = \"{a_}\"\n\
         [[mapping]]\nprofile = \"{p}\"\ndatabase = \"orders\"\nlibrary = \"{good}\"\n\
         [[mapping]]\nprofile = \"{p}\"\ndatabase = \"orders\"\nlibrary = \"{good}\"\n\
         [[mapping]]\nprofile = \"{p}\"\ndatabase = \"x\"\nlibrary = \"{}\"\n\
         [[mapping]]\nprofile = \"nope\"\ndatabase = \"x\"\nlibrary = \"{good}\"\n",
        LibraryId::new()
    );
    a.write(&text);
    let (reg, diags) = a.load();
    assert_eq!(reg.libraries().len(), 1);
    assert_eq!(reg.libraries()[0].id, good);
    assert_eq!(reg.mappings(), [mapping(p, "orders", good)]);
    assert_eq!(
        diags,
        [
            RegistryDiag::LibraryDropped {
                index: 1,
                problem: RowProblem::BadId
            },
            RegistryDiag::LibraryDropped {
                index: 2,
                problem: RowProblem::RelativePath
            },
            RegistryDiag::LibraryDropped {
                index: 3,
                problem: RowProblem::Duplicate
            },
            RegistryDiag::LibraryDropped {
                index: 4,
                problem: RowProblem::Duplicate
            },
            RegistryDiag::MappingDropped {
                index: 1,
                problem: RowProblem::Duplicate
            },
            RegistryDiag::OrphanMapping { index: 2 },
            RegistryDiag::MappingDropped {
                index: 3,
                problem: RowProblem::BadId
            },
        ]
    );
}

/// alg/28 `nested_library_is_refused`, and the other attach refusals of HV-SCRIPT-01 step 2.
#[test]
fn nested_library_is_refused() {
    let a = app();
    let top = tempfile::tempdir().unwrap();
    let inner = top.path().join("team/orders");
    fs::create_dir_all(&inner).unwrap();
    let (mut reg, _) = a.load();
    let outer = reg.add(&top.path().join("team"), &a.guard(), t0()).unwrap();
    // Inside an existing library.
    assert_eq!(
        reg.add(&inner, &a.guard(), t0()).map(|_| ()),
        Err(RegistryError::Nested(outer.id))
    );
    // Containing one.
    assert_eq!(
        reg.add(top.path(), &a.guard(), t0()).map(|_| ()),
        Err(RegistryError::Nested(outer.id))
    );
    // The same folder spelled differently.
    let again = top.path().join("team/orders/..");
    assert_eq!(
        reg.add(&again, &a.guard(), t0()).map(|_| ()),
        Err(RegistryError::AlreadyRegistered(outer.id))
    );
    // A sibling whose name starts the same is not nested.
    fs::create_dir_all(top.path().join("team2")).unwrap();
    reg.add(&top.path().join("team2"), &a.guard(), t0())
        .unwrap();
    assert_eq!(reg.libraries().len(), 2);
}

#[cfg(unix)]
#[test]
fn a_link_to_a_registered_library_is_the_same_library() {
    let a = app();
    let top = tempfile::tempdir().unwrap();
    fs::create_dir(top.path().join("real")).unwrap();
    std::os::unix::fs::symlink(top.path().join("real"), top.path().join("alias")).unwrap();
    let (mut reg, _) = a.load();
    let real = reg.add(&top.path().join("real"), &a.guard(), t0()).unwrap();
    assert_eq!(
        reg.add(&top.path().join("alias"), &a.guard(), t0())
            .map(|_| ()),
        Err(RegistryError::AlreadyRegistered(real.id))
    );
}

#[test]
fn attach_refuses_home_root_app_folders_and_files() {
    let a = app();
    let home = tempfile::tempdir().unwrap();
    let guard = AttachGuard::new(&a.paths, Some(home.path()));
    let (mut reg, _) = a.load();
    let refused = |reg: &mut LibraryRegistry, p: &Path| reg.add(p, &guard, t0()).map(|_| ());
    assert_eq!(
        refused(&mut reg, home.path()),
        Err(RegistryError::HomeFolder)
    );
    // A folder inside home is fine.
    fs::create_dir(home.path().join("scripts")).unwrap();
    reg.add(&home.path().join("scripts"), &guard, t0()).unwrap();
    let fs_root = std::env::current_dir()
        .unwrap()
        .ancestors()
        .last()
        .unwrap()
        .to_path_buf();
    assert_eq!(
        refused(&mut reg, &fs_root),
        Err(RegistryError::FilesystemRoot)
    );
    // The app's own folders, inside them, or containing them.
    a.paths.ensure_dirs().unwrap();
    fs::create_dir_all(a.paths.data.join("script-recovery")).unwrap();
    for p in [
        a.paths.config.clone(),
        a.paths.data.join("script-recovery"),
        a.paths.cache.clone(),
        a.paths.data.parent().unwrap().to_path_buf(),
    ] {
        assert_eq!(
            refused(&mut reg, &p),
            Err(RegistryError::AppFolder),
            "{p:?}"
        );
    }
    // Not a folder, missing, relative.
    let f = home.path().join("file.sql");
    fs::write(&f, "SELECT 1;").unwrap();
    assert_eq!(refused(&mut reg, &f), Err(RegistryError::NotAFolder));
    assert!(matches!(
        refused(&mut reg, &home.path().join("missing")),
        Err(RegistryError::Io(_))
    ));
    assert_eq!(
        refused(&mut reg, Path::new("relative/dir")),
        Err(RegistryError::NotAbsolute)
    );
    assert_eq!(reg.libraries().len(), 1);
}

#[test]
fn at_most_32_libraries() {
    let a = app();
    let top = tempfile::tempdir().unwrap();
    let (mut reg, _) = a.load();
    for i in 0..MAX_LIBRARIES {
        let d = top.path().join(format!("l{i}"));
        fs::create_dir(&d).unwrap();
        reg.add(&d, &a.guard(), t0()).unwrap();
    }
    let d = top.path().join("one-more");
    fs::create_dir(&d).unwrap();
    assert_eq!(
        reg.add(&d, &a.guard(), t0()).map(|_| ()),
        Err(RegistryError::TooMany)
    );
    // A hand-edited file with more is cut at 32, with a diagnostic.
    reg.save().unwrap();
    let mut text = fs::read_to_string(a.file()).unwrap();
    text.push_str(&format!(
        "\n[[library]]\nid = \"{}\"\npath = \"{}\"\n",
        LibraryId::new(),
        abs("libs/extra")
    ));
    fs::write(a.file(), text).unwrap();
    let (reg, diags) = a.load();
    assert_eq!(reg.libraries().len(), MAX_LIBRARIES);
    assert_eq!(diags, [RegistryDiag::TooManyLibraries]);
}

/// HV-SCRIPT-01 step 5: a missing folder stays registered.
#[test]
fn missing_folder_stays_registered() {
    let a = app();
    let lib = tempfile::tempdir().unwrap();
    let kept = lib.path().to_path_buf();
    let (mut reg, _) = a.load();
    let root = reg.add(&kept, &a.guard(), t0()).unwrap();
    reg.save().unwrap();
    drop(lib);
    let (reg, diags) = a.load();
    assert!(diags.is_empty());
    let entry = reg.library(root.id).unwrap();
    assert_eq!(entry.path, kept);
    assert!(entry.root().is_err());
}
