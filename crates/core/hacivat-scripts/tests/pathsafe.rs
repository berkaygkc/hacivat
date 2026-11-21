//! M4b Task 4: names from a library and every later open are validated (HV-SCRIPT-03 steps 4
//! and 8): no name joins outside the root, no link on the way is followed, the root is compared
//! canonically on both sides.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use hacivat_scripts::pathsafe::{self, Host, NameProblem, component, component_on, join};
use hacivat_scripts::{LibraryId, LibraryRoot, PathError, RelPath};

fn rel(s: &str) -> RelPath {
    RelPath::parse(s).unwrap()
}

fn canon(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap()
}

/// A library rooted at `dir`, registered through `path` (canonical form computed here).
fn root_at(path: &Path) -> LibraryRoot {
    LibraryRoot {
        id: LibraryId::new(),
        path: path.to_path_buf(),
        canonical: canon(path),
    }
}

fn lib() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn name_problem(r: Result<&str, PathError>) -> NameProblem {
    match r {
        Err(PathError::Name(p)) => p,
        other => panic!("expected a name problem, got {other:?}"),
    }
}

#[test]
fn pathsafe_component_table() {
    for bad in ["", ".", "..", "a/b", "a\\b", "a\0b", "a\u{7}b"] {
        assert!(component(OsStr::new(bad)).is_err(), "{bad:?}");
    }
    for good in [
        "ünïcode.sql",
        "rapor İ.sql",
        "a.sql",
        "@production",
        "email@host",
        "x.SQL",
    ] {
        assert_eq!(component(OsStr::new(good)), Ok(good), "{good:?}");
    }
}

#[test]
fn each_refusal_names_its_problem() {
    let p = |s: &str| name_problem(component(OsStr::new(s)));
    assert_eq!(p(""), NameProblem::Empty);
    assert_eq!(p("."), NameProblem::Dot);
    assert_eq!(p(".."), NameProblem::DotDot);
    assert_eq!(p("a/b"), NameProblem::Separator);
    assert_eq!(p("/"), NameProblem::Separator);
    assert_eq!(p("a\\b"), NameProblem::Separator);
    assert_eq!(p("a\0b"), NameProblem::Nul);
    assert_eq!(p("\0"), NameProblem::Nul);
}

#[test]
fn nul_and_control_characters_are_refused() {
    // C0, DEL and C1 (NEL, CSI), wherever they stand.
    for bad in [
        "a\nb.sql",
        "a\rb.sql",
        "\tlead.sql",
        "trail.sql\n",
        "a\u{1}b",
        "a\u{1b}[31mb.sql",
        "a\u{7f}b",
        "a\u{85}b",
        "a\u{9b}b",
        "a\u{9f}b",
    ] {
        assert_eq!(
            name_problem(component(OsStr::new(bad))),
            NameProblem::Control,
            "{bad:?}"
        );
    }
    // Bidi and invisible characters are allowed as names; display escapes them.
    assert!(component(OsStr::new("a\u{202E}lqs.sql")).is_ok());
    assert!(component(OsStr::new("a\u{200B}.sql")).is_ok());
}

#[test]
fn names_over_255_bytes_are_refused() {
    let ok = "a".repeat(251) + ".sql";
    assert_eq!(ok.len(), 255);
    assert!(component(OsStr::new(&ok)).is_ok());
    let long = "a".repeat(252) + ".sql";
    assert_eq!(
        name_problem(component(OsStr::new(&long))),
        NameProblem::TooLong
    );
    // Bytes, not characters: 128 two-byte letters are 256 bytes.
    let wide = "ş".repeat(128);
    assert_eq!(wide.chars().count(), 128);
    assert_eq!(
        name_problem(component(OsStr::new(&wide))),
        NameProblem::TooLong
    );
}

#[cfg(unix)]
#[test]
fn non_utf8_names_are_refused() {
    use std::os::unix::ffi::OsStrExt;
    for bytes in [&b"a\xffb.sql"[..], b"\xc3\x28.sql", b"\xed\xa0\x80.sql"] {
        assert_eq!(
            name_problem(component(OsStr::from_bytes(bytes))),
            NameProblem::NotUtf8,
            "{bytes:?}"
        );
    }
}

#[test]
fn windows_rules_on_any_host() {
    // The Windows rules, checked on every runner through `component_on`.
    for bad in [
        "CON",
        "con.sql",
        "LPT1.txt",
        "a.",
        "a ",
        "a:b",
        "COM9",
        "Aux",
        "nul.tar.gz",
        "PRN.sql",
        "com1",
        "lpt9.md",
        "COM0",
        "LPT0.sql",
        "COM\u{b9}",
        "LPT\u{b2}.sql",
        "CONIN$",
        "conout$.sql",
        "con .sql",
        "a<b",
        "a>b",
        "a|b",
        "a?b",
        "a*b",
        "a\"b",
        "C:",
    ] {
        assert!(
            component_on(OsStr::new(bad), Host::Windows).is_err(),
            "{bad:?} on Windows"
        );
    }
    for good in [
        "CONSOLE.sql",
        "COM10.sql",
        "nulled.sql",
        "a.b.sql",
        "LPT.sql",
        "con_x.sql",
    ] {
        assert_eq!(
            component_on(OsStr::new(good), Host::Windows),
            Ok(good),
            "{good:?}"
        );
    }
    // The same names are ordinary on Unix (a macOS teammate may commit `con.sql`).
    for unix_ok in ["CON", "con.sql", "a.", "a ", "a:b", "C:", "a?b"] {
        assert_eq!(
            component_on(OsStr::new(unix_ok), Host::Unix),
            Ok(unix_ok),
            "{unix_ok:?}"
        );
    }
    assert_eq!(
        name_problem(component_on(OsStr::new("con.sql"), Host::Windows)),
        NameProblem::Reserved
    );
    assert_eq!(
        name_problem(component_on(OsStr::new("a."), Host::Windows)),
        NameProblem::TrailingDotOrSpace
    );
    assert_eq!(
        name_problem(component_on(OsStr::new("a:b"), Host::Windows)),
        NameProblem::WindowsCharacter
    );
}

#[cfg(windows)]
#[test]
fn windows_reserved_names_are_refused() {
    for bad in ["CON", "con.sql", "LPT1.txt", "a.", "a ", "a:b"] {
        assert!(component(OsStr::new(bad)).is_err(), "{bad}");
    }
}

#[test]
fn an_accepted_component_is_one_rel_path_component() {
    for good in ["a.sql", "rapor İ.sql", "@production", "a\u{202E}b", "x y"] {
        let s = component(OsStr::new(good)).unwrap();
        let r = RelPath::parse(s).unwrap();
        assert_eq!(r.components().count(), 1);
    }
}

#[test]
fn dot_dot_and_absolute_paths_never_parse() {
    for bad in [
        "../x.sql",
        "a/../../b.sql",
        "/etc/passwd",
        "//server/share/a.sql",
        "a/./b.sql",
        "C:\\Windows\\a.sql",
        "\\\\?\\C:\\a.sql",
        "a/..",
        "",
    ] {
        assert!(RelPath::parse(bad).is_err(), "{bad:?}");
        let json = format!("{bad:?}");
        assert!(
            serde_json_like(&json).is_err(),
            "deserializing {bad:?} must fail too"
        );
    }
}

/// `RelPath`'s `Deserialize` through `toml` (a value in a table), the shape `workspace.json`
/// readers use.
fn serde_json_like(quoted: &str) -> Result<RelPath, toml::de::Error> {
    #[derive(serde::Deserialize)]
    struct W {
        p: RelPath,
    }
    let doc = format!("p = {quoted}");
    toml::from_str::<W>(&doc).map(|w| w.p)
}

#[test]
fn join_a_plain_file_a_nested_one_and_a_new_one() {
    let d = lib();
    fs::create_dir(d.path().join("x")).unwrap();
    fs::write(d.path().join("a.sql"), "SELECT 1;").unwrap();
    fs::write(d.path().join("x/b.sql"), "SELECT 2;").unwrap();
    let root = root_at(d.path());
    assert_eq!(
        join(&root, &rel("a.sql")).unwrap(),
        root.canonical.join("a.sql")
    );
    assert_eq!(
        join(&root, &rel("x/b.sql")).unwrap(),
        root.canonical.join("x/b.sql")
    );
    // A name that does not exist yet (New script, Save as) joins under an existing folder.
    assert_eq!(
        join(&root, &rel("x/new.sql")).unwrap(),
        root.canonical.join("x/new.sql")
    );
    assert_eq!(join(&root, &rel("x")).unwrap(), root.canonical.join("x"));
    // A missing folder on the way is refused, never created.
    assert_eq!(
        join(&root, &rel("nope/new.sql")),
        Err(PathError::Io(std::io::ErrorKind::NotFound))
    );
    // A file on the way is not a folder.
    assert_eq!(join(&root, &rel("a.sql/b.sql")), Err(PathError::NotAFolder));
}

#[test]
fn join_refuses_skipped_and_bad_components() {
    let d = lib();
    fs::create_dir(d.path().join(".git")).unwrap();
    fs::write(d.path().join(".git/config"), "[core]\n").unwrap();
    let root = root_at(d.path());
    for hidden in [
        ".git/config",
        ".hacivat/workspace.toml",
        "x/.a.sql",
        ".!123!a.sql",
        "node_modules/x.sql",
        "NODE_MODULES/x.sql",
    ] {
        assert_eq!(
            join(&root, &rel(hidden)),
            Err(PathError::Name(NameProblem::Skipped)),
            "{hidden}"
        );
    }
    let long = "a".repeat(300);
    assert_eq!(
        join(&root, &rel(&long)),
        Err(PathError::Name(NameProblem::TooLong))
    );
}

#[test]
fn root_is_compared_canonically() {
    let d = lib();
    fs::create_dir(d.path().join("Lib")).unwrap();
    fs::write(d.path().join("Lib/a.sql"), "SELECT 1;").unwrap();
    let canonical = canon(&d.path().join("Lib"));
    let through_canonical = root_at(&canonical);
    // The path as chosen: the tempdir spelling (macOS `/var/…`, a link to `/private/var/…`),
    // with a trailing separator and a `.` component.
    let mut spelled = d.path().join("Lib").into_os_string();
    spelled.push(std::path::MAIN_SEPARATOR_STR);
    spelled.push(".");
    spelled.push(std::path::MAIN_SEPARATOR_STR);
    let chosen = LibraryRoot {
        id: LibraryId::new(),
        path: PathBuf::from(spelled),
        canonical: canonical.clone(),
    };
    for root in [&through_canonical, &chosen] {
        assert_eq!(join(root, &rel("a.sql")).unwrap(), canonical.join("a.sql"));
    }
    // A case-only different spelling on a case-insensitive volume (APFS, NTFS default):
    // `realpath` answers the stored case, so it is the same library.
    let upper = d.path().join("LIB");
    if upper.exists() {
        let via_upper = LibraryRoot {
            id: LibraryId::new(),
            path: upper.clone(),
            canonical: canon(&upper),
        };
        assert_eq!(
            via_upper.canonical, canonical,
            "realpath gives the stored case"
        );
        assert_eq!(
            join(&via_upper, &rel("a.sql")).unwrap(),
            canonical.join("a.sql")
        );
    }
}

#[test]
fn a_root_whose_canonical_form_is_stale_is_refused() {
    let d = lib();
    fs::create_dir(d.path().join("a")).unwrap();
    fs::create_dir(d.path().join("b")).unwrap();
    // A stored canonical path that is not canonical (a caller bug or a moved volume).
    let root = LibraryRoot {
        id: LibraryId::new(),
        path: d.path().join("a"),
        canonical: d.path().join("a").join("..").join("b"),
    };
    assert_eq!(join(&root, &rel("x.sql")), Err(PathError::RootChanged));
    // A root folder that is gone.
    let gone = root_at(&d.path().join("a"));
    fs::remove_dir(d.path().join("a")).unwrap();
    assert_eq!(join(&gone, &rel("x.sql")), Err(PathError::RootChanged));
}

#[cfg(unix)]
mod links {
    use std::os::unix::fs::symlink;

    use super::{LibraryId, LibraryRoot, PathError, canon, fs, join, lib, rel, root_at};

    #[test]
    fn a_file_link_out_of_the_library_is_refused() {
        let d = lib();
        let outside = lib();
        fs::write(outside.path().join("secret.sql"), "DROP TABLE x;").unwrap();
        symlink(
            outside.path().join("secret.sql"),
            d.path().join("secret.sql"),
        )
        .unwrap();
        // A relative link climbing out works the same way.
        symlink(
            "../../../../../../../../etc/hosts",
            d.path().join("hosts.sql"),
        )
        .unwrap();
        let root = root_at(d.path());
        assert_eq!(join(&root, &rel("secret.sql")), Err(PathError::OutsideRoot));
        assert_eq!(join(&root, &rel("hosts.sql")), Err(PathError::OutsideRoot));
    }

    #[test]
    fn a_folder_link_on_the_way_is_never_followed() {
        let d = lib();
        let outside = lib();
        fs::write(outside.path().join("a.sql"), "x").unwrap();
        symlink(outside.path(), d.path().join("out")).unwrap();
        fs::create_dir(d.path().join("real")).unwrap();
        fs::write(d.path().join("real/a.sql"), "x").unwrap();
        // Even a link that stays inside is a link: directory links are never followed (Q15).
        symlink(d.path().join("real"), d.path().join("alias")).unwrap();
        let root = root_at(d.path());
        assert_eq!(join(&root, &rel("out/a.sql")), Err(PathError::LinkOnPath));
        assert_eq!(join(&root, &rel("out/new.sql")), Err(PathError::LinkOnPath));
        assert_eq!(join(&root, &rel("alias/a.sql")), Err(PathError::LinkOnPath));
        // The folder link itself, as the last component, is not a file to open either.
        assert_eq!(join(&root, &rel("out")), Err(PathError::OutsideRoot));
        assert_eq!(join(&root, &rel("alias")), Err(PathError::OutsideRoot));
    }

    #[test]
    fn a_file_link_inside_opens_its_canonical_target() {
        let d = lib();
        fs::create_dir(d.path().join("shared")).unwrap();
        fs::write(d.path().join("shared/q.sql"), "SELECT 1;").unwrap();
        symlink("shared/q.sql", d.path().join("q.sql")).unwrap();
        let root = root_at(d.path());
        assert_eq!(
            join(&root, &rel("q.sql")).unwrap(),
            root.canonical.join("shared/q.sql")
        );
    }

    #[test]
    fn a_link_into_a_skipped_folder_or_to_a_non_script_is_refused() {
        let d = lib();
        fs::create_dir(d.path().join(".git")).unwrap();
        fs::write(d.path().join(".git/config"), "[core]\n").unwrap();
        fs::write(d.path().join("notes.txt"), "x").unwrap();
        fs::create_dir(d.path().join("node_modules")).unwrap();
        fs::write(d.path().join("node_modules/x.sql"), "x").unwrap();
        symlink(".git/config", d.path().join("cfg.sql")).unwrap();
        symlink("notes.txt", d.path().join("notes.sql")).unwrap();
        symlink("node_modules/x.sql", d.path().join("nm.sql")).unwrap();
        symlink(".", d.path().join("root.sql")).unwrap();
        let root = root_at(d.path());
        for name in ["cfg.sql", "notes.sql", "nm.sql", "root.sql"] {
            assert_eq!(
                join(&root, &rel(name)),
                Err(PathError::OutsideRoot),
                "{name}"
            );
        }
    }

    #[test]
    fn a_symlink_loop_and_a_dangling_link_are_refused() {
        let d = lib();
        symlink("b.sql", d.path().join("a.sql")).unwrap();
        symlink("a.sql", d.path().join("b.sql")).unwrap();
        symlink("self.sql", d.path().join("self.sql")).unwrap();
        symlink("missing.sql", d.path().join("dangling.sql")).unwrap();
        let root = root_at(d.path());
        for name in ["a.sql", "b.sql", "self.sql", "dangling.sql"] {
            assert_eq!(
                join(&root, &rel(name)),
                Err(PathError::OutsideRoot),
                "{name}"
            );
        }
    }

    #[test]
    fn save_refuses_a_path_that_became_a_link() {
        // TOCTOU: a folder the scan saw is replaced by a link out before the save joins.
        let d = lib();
        let outside = lib();
        fs::create_dir(d.path().join("x")).unwrap();
        fs::write(d.path().join("x/a.sql"), "x").unwrap();
        let root = root_at(d.path());
        assert!(join(&root, &rel("x/a.sql")).is_ok());
        fs::remove_file(d.path().join("x/a.sql")).unwrap();
        fs::remove_dir(d.path().join("x")).unwrap();
        symlink(outside.path(), d.path().join("x")).unwrap();
        assert_eq!(join(&root, &rel("x/a.sql")), Err(PathError::LinkOnPath));
        // And a file replaced by a link out.
        fs::write(d.path().join("b.sql"), "x").unwrap();
        assert!(join(&root, &rel("b.sql")).is_ok());
        fs::remove_file(d.path().join("b.sql")).unwrap();
        fs::write(outside.path().join("b.sql"), "x").unwrap();
        symlink(outside.path().join("b.sql"), d.path().join("b.sql")).unwrap();
        assert_eq!(join(&root, &rel("b.sql")), Err(PathError::OutsideRoot));
        assert!(outside.path().join("b.sql").exists());
    }

    #[test]
    fn a_root_that_became_a_link_elsewhere_is_refused() {
        let d = lib();
        fs::create_dir(d.path().join("one")).unwrap();
        fs::create_dir(d.path().join("two")).unwrap();
        fs::write(d.path().join("one/a.sql"), "x").unwrap();
        fs::write(d.path().join("two/a.sql"), "x").unwrap();
        symlink(d.path().join("one"), d.path().join("lib")).unwrap();
        let root = LibraryRoot {
            id: LibraryId::new(),
            path: d.path().join("lib"),
            canonical: canon(&d.path().join("lib")),
        };
        assert!(join(&root, &rel("a.sql")).is_ok());
        fs::remove_file(d.path().join("lib")).unwrap();
        symlink(d.path().join("two"), d.path().join("lib")).unwrap();
        assert_eq!(join(&root, &rel("a.sql")), Err(PathError::RootChanged));
    }

    #[test]
    fn a_fifo_is_not_a_file_to_open() {
        let d = lib();
        let status = std::process::Command::new("mkfifo")
            .arg(d.path().join("pipe.sql"))
            .status()
            .unwrap();
        assert!(status.success());
        let root = root_at(d.path());
        assert_eq!(join(&root, &rel("pipe.sql")), Err(PathError::NotAFile));
    }
}

#[test]
fn display_name_is_nfc_with_invisible_characters_escaped() {
    assert_eq!(
        pathsafe::display_name("a\u{202E}lqs.sql"),
        "a<U+202E>lqs.sql"
    );
    assert_eq!(
        pathsafe::display_name("mu\u{0308}s\u{0327}teri.sql"),
        "müşteri.sql"
    );
    assert_eq!(pathsafe::display_name("a\u{200B}b"), "a<U+200B>b");
    assert_eq!(
        pathsafe::display_name("\u{2066}x\u{2069}"),
        "<U+2066>x<U+2069>"
    );
    assert_eq!(pathsafe::display_name("a\u{FEFF}"), "a<U+FEFF>");
    assert_eq!(pathsafe::display_name("a\u{E0041}"), "a<U+E0041>");
    assert_eq!(pathsafe::display_name("a\nb\u{1b}"), "a<U+000A>b<U+001B>");
    assert_eq!(pathsafe::display_name("rapor İ.sql"), "rapor İ.sql");
    let r = rel("@dev/mu\u{0308}s\u{202E}.sql");
    assert_eq!(r.display_text(), "@dev/müs<U+202E>.sql");
    // Identity keeps the bytes.
    assert_eq!(r.as_str(), "@dev/mu\u{0308}s\u{202E}.sql");
}

#[test]
fn a_case_variant_on_a_case_insensitive_volume_stays_inside() {
    // APFS and NTFS default to case-insensitive names. A `RelPath` whose case differs from
    // the disk resolves to the same file; containment is a byte comparison of canonical
    // paths (the stored case on both sides), never a case fold of our own.
    let d = lib();
    fs::create_dir(d.path().join("Shared")).unwrap();
    fs::write(d.path().join("Shared/a.sql"), "x").unwrap();
    let root = root_at(d.path());
    let insensitive = d.path().join("SHARED").exists();
    let r = join(&root, &rel("SHARED/A.SQL"));
    if insensitive {
        let p = r.unwrap();
        assert!(p.starts_with(&root.canonical));
        assert!(canon(&p).starts_with(&root.canonical));
    } else {
        assert_eq!(r, Err(PathError::Io(std::io::ErrorKind::NotFound)));
    }
}
