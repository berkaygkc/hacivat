//! M4b Task 5: script file I/O (HV-SCRIPT-07; UI-EDIT-02 decode/save, UI-EDIT-23 tiers).
//!
//! Every fixture is built at run time in a temp dir (no checked-in CRLF or link files). The
//! encodings are written by hand here (UTF-16 from `encode_utf16`, Windows-1254 from its code
//! chart), never through `encoding_rs`, so the oracle does not share the code under test.

use std::fs;
use std::path::Path;

use hacivat_scripts::fileio::{
    self, PREVIEW_BYTES, ReadLimits, STREAM_CHUNK_BYTES, faults, open_stream, read, read_with,
    save, save_with_echo, tier_for,
};
use hacivat_scripts::{
    DiskStamp, LibraryId, LibraryRoot, LineEnding, ReadError, RelPath, SaveError, SaveText,
    SizeTier,
};

fn rel(s: &str) -> RelPath {
    RelPath::parse(s).unwrap()
}

fn root(d: &tempfile::TempDir) -> LibraryRoot {
    LibraryRoot {
        id: LibraryId::new(),
        path: d.path().into(),
        canonical: d.path().canonicalize().unwrap(),
    }
}

fn lib_with_bytes(name: &str, bytes: &[u8]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    if let Some(parent) = Path::new(name).parent() {
        fs::create_dir_all(d.path().join(parent)).unwrap();
    }
    fs::write(d.path().join(name), bytes).unwrap();
    d
}

fn read_default(d: &tempfile::TempDir, name: &str) -> hacivat_scripts::Decoded {
    read(&root(d), &rel(name), None, ReadLimits::default()).unwrap()
}

fn text(s: &str) -> SaveText {
    SaveText {
        text: s.into(),
        meta: Default::default(),
    }
}

fn utf16le_bom(s: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    out.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
    out
}

fn utf16be_bom(s: &str) -> Vec<u8> {
    let mut out = vec![0xFE, 0xFF];
    out.extend(s.encode_utf16().flat_map(u16::to_be_bytes));
    out
}

/// Windows-1254 from its code chart: ASCII, plus the Turkish letters the tests use.
fn cp1254(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| match c {
            c if c.is_ascii() => c as u8,
            'Ç' => 0xC7,
            'Ğ' => 0xD0,
            'Ö' => 0xD6,
            'Ü' => 0xDC,
            'İ' => 0xDD,
            'Ş' => 0xDE,
            'ç' => 0xE7,
            'ğ' => 0xF0,
            'ö' => 0xF6,
            'ü' => 0xFC,
            'ı' => 0xFD,
            'ş' => 0xFE,
            other => panic!("not in the test chart: {other:?}"),
        })
        .collect()
}

/// Every file and folder name under `dir`, recursively, relative and sorted.
/// Every name under `dir`, as library-relative text: `/`-separated on every OS, like a
/// `RelPath` (m4b-cifix-5: the native `\` on Windows, CI 45091594060).
fn names_under(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            let parts: Vec<String> = p
                .strip_prefix(dir)
                .unwrap()
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            out.push(parts.join("/"));
            if e.file_type().unwrap().is_dir() {
                todo.push(p);
            }
        }
    }
    out.sort();
    out
}

fn xxh3(bytes: &[u8]) -> u128 {
    twox_hash::XxHash3_128::oneshot(bytes)
}

#[test]
fn fileio_round_trip_table() {
    // read → save unchanged → bytes identical, and the meta the read recorded.
    let rows: [(&str, Vec<u8>, &str, bool, LineEnding); 9] = [
        (
            "utf8",
            b"SELECT 1;\n".to_vec(),
            "UTF-8",
            false,
            LineEnding::Lf,
        ),
        (
            "utf8-bom",
            [&[0xEF, 0xBB, 0xBF][..], b"SELECT 1;\r\n"].concat(),
            "UTF-8",
            true,
            LineEnding::CrLf,
        ),
        (
            "utf16le-bom",
            utf16le_bom("SELECT 'ğüşiöç';\r\n"),
            "UTF-16LE",
            true,
            LineEnding::CrLf,
        ),
        (
            "utf16be-bom",
            utf16be_bom("SELECT 1;\n"),
            "UTF-16BE",
            true,
            LineEnding::Lf,
        ),
        // One of each: a tie, so LF (UI-EDIT-02 step 3).
        (
            "mixed",
            b"a\r\nb\nc\r".to_vec(),
            "UTF-8",
            false,
            LineEnding::Lf,
        ),
        (
            "crlf-dominant",
            b"a\r\nb\r\nc\n".to_vec(),
            "UTF-8",
            false,
            LineEnding::CrLf,
        ),
        (
            "cr-only",
            b"a\rb\rc".to_vec(),
            "UTF-8",
            false,
            LineEnding::Cr,
        ),
        (
            "no-final-newline",
            b"SELECT 1".to_vec(),
            "UTF-8",
            false,
            LineEnding::Lf,
        ),
        ("empty", Vec::new(), "UTF-8", false, LineEnding::Lf),
    ];
    for (name, bytes, enc, bom, ending) in rows {
        let d = lib_with_bytes("f.sql", &bytes);
        let r = read(&root(&d), &rel("f.sql"), None, ReadLimits::default())
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(r.meta.encoding.name(), enc, "{name}");
        assert_eq!(r.meta.bom, bom, "{name}");
        assert_eq!(r.meta.line_ending, ending, "{name}");
        assert_eq!(r.tier, SizeTier::Full, "{name}");
        assert!(
            !r.text.starts_with('\u{FEFF}'),
            "{name}: the BOM is not text"
        );
        assert_eq!(r.stamp.len, bytes.len() as u64, "{name}");
        assert_eq!(r.stamp.hash, xxh3(&bytes), "{name}");
        save(
            &root(&d),
            &rel("f.sql"),
            None,
            r.save_text().unwrap(),
            Some(r.stamp),
        )
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(fs::read(d.path().join("f.sql")).unwrap(), bytes, "{name}");
        assert_eq!(
            names_under(d.path()),
            ["f.sql"],
            "{name}: no temp file left"
        );
    }
    let d = lib_with_bytes("f.sql", &utf16le_bom("SELECT 'ğüşiöç';\r\n"));
    assert_eq!(read_default(&d, "f.sql").text, "SELECT 'ğüşiöç';\r\n");
}

#[test]
fn windows_1254_needs_an_encoding_then_round_trips() {
    let bytes = cp1254("SELECT 'İĞŞ ğüşıöçİ';\r\n");
    let d = lib_with_bytes("tr.sql", &bytes);
    assert!(matches!(
        read(&root(&d), &rel("tr.sql"), None, ReadLimits::default()),
        Err(ReadError::NeedsEncoding { .. })
    ));
    let r = read_with(&root(&d), &rel("tr.sql"), None, "windows-1254").unwrap();
    assert_eq!(r.text, "SELECT 'İĞŞ ğüşıöçİ';\r\n");
    assert_eq!(r.meta.encoding.name(), "windows-1254");
    assert!(!r.meta.bom);
    // ISO-8859-9 is the same encoding to WHATWG.
    let iso = read_with(&root(&d), &rel("tr.sql"), None, "ISO-8859-9").unwrap();
    assert_eq!(iso.meta.encoding.name(), "windows-1254");
    save(
        &root(&d),
        &rel("tr.sql"),
        None,
        r.save_text().unwrap(),
        Some(r.stamp),
    )
    .unwrap();
    assert_eq!(fs::read(d.path().join("tr.sql")).unwrap(), bytes);
    // An edit is encoded back in the file's encoding, never as UTF-8.
    let r = read_with(&root(&d), &rel("tr.sql"), None, "windows-1254").unwrap();
    let mut edited = r.save_text().unwrap();
    edited.text.push_str("-- Ş\r\n");
    save(&root(&d), &rel("tr.sql"), None, edited, Some(r.stamp)).unwrap();
    assert_eq!(
        fs::read(d.path().join("tr.sql")).unwrap(),
        cp1254("SELECT 'İĞŞ ğüşıöçİ';\r\n-- Ş\r\n")
    );
}

#[test]
fn read_with_refuses_an_unknown_encoding_and_a_lossy_one() {
    let d = lib_with_bytes("a.sql", b"SELECT 1;");
    assert_eq!(
        read_with(&root(&d), &rel("a.sql"), None, "no-such-encoding").unwrap_err(),
        ReadError::UnknownEncoding
    );
    // `replacement` would decode every byte to U+FFFD.
    assert_eq!(
        read_with(&root(&d), &rel("a.sql"), None, "csiso2022kr").unwrap_err(),
        ReadError::UnknownEncoding
    );
    // Shift_JIS 0x87 0x90 (NEC row 13) decodes to U+2252, which encodes back as 0x81 0xE0: a
    // save of the unchanged text would change the file, so the read refuses it.
    let d = lib_with_bytes("sj.sql", &[b'a', 0x87, 0x90, b'b']);
    assert_eq!(
        read_with(&root(&d), &rel("sj.sql"), None, "Shift_JIS").unwrap_err(),
        ReadError::Lossy
    );
    // Bytes the encoding cannot decode at all.
    let d = lib_with_bytes("bad.sql", &[0xFF, 0xFF, 0xFF]);
    assert_eq!(
        read_with(&root(&d), &rel("bad.sql"), None, "UTF-8").unwrap_err(),
        ReadError::Undecodable
    );
}

#[test]
fn unencodable_character_refuses_the_save() {
    let bytes = cp1254("SELECT 'İ';\n");
    let d = lib_with_bytes("tr.sql", &bytes);
    let r = read_with(&root(&d), &rel("tr.sql"), None, "windows-1254").unwrap();
    let mut edited = r.save_text().unwrap();
    edited.text = "SELECT 'İ';\n-- 漢\n".into();
    assert_eq!(
        save(&root(&d), &rel("tr.sql"), None, edited, Some(r.stamp)),
        Err(SaveError::Unencodable {
            first_offset: "SELECT 'İ';\n-- ".len()
        })
    );
    assert_eq!(fs::read(d.path().join("tr.sql")).unwrap(), bytes);
    assert_eq!(names_under(d.path()), ["tr.sql"]);
}

#[test]
fn binary_file_is_refused() {
    // More than 1 % NUL in the first 64 KiB, without a UTF-16 BOM.
    let mut bytes = b"SQLite format 3\0".to_vec();
    bytes.extend(std::iter::repeat_n(0u8, 200));
    bytes.extend(std::iter::repeat_n(b'x', 1000));
    let d = lib_with_bytes("a.sql", &bytes);
    assert_eq!(
        read(&root(&d), &rel("a.sql"), None, ReadLimits::default()).unwrap_err(),
        ReadError::Binary
    );
    // At most 1 %: text with a few NUL characters still opens.
    let mut few = vec![b'x'; 1000];
    few[10] = 0;
    let d = lib_with_bytes("b.sql", &few);
    assert_eq!(read_default(&d, "b.sql").text.len(), 1000);
    // UTF-16 text is half NUL bytes, but its BOM says what it is.
    let d = lib_with_bytes("c.sql", &utf16le_bom("SELECT 1;"));
    assert_eq!(read_default(&d, "c.sql").text, "SELECT 1;");
}

#[test]
fn invalid_utf8_without_a_bom_asks_for_an_encoding() {
    for bytes in [
        b"SELECT '\xC3';".to_vec(),
        // A UTF-8 BOM followed by bytes that are not UTF-8.
        [&[0xEF, 0xBB, 0xBF][..], b"\xFF"].concat(),
        // A UTF-16 BOM over an odd byte count.
        vec![0xFF, 0xFE, b'a'],
    ] {
        let d = lib_with_bytes("a.sql", &bytes);
        assert!(
            matches!(
                read(&root(&d), &rel("a.sql"), None, ReadLimits::default()),
                Err(ReadError::NeedsEncoding { .. })
            ),
            "{bytes:?}"
        );
    }
}

#[test]
fn changed_on_disk_refuses_without_expect_match() {
    let d = lib_with_bytes("f.sql", b"a");
    let r = read_default(&d, "f.sql");
    fs::write(d.path().join("f.sql"), b"b").unwrap();
    match save(&root(&d), &rel("f.sql"), None, text("c"), Some(r.stamp)) {
        Err(SaveError::ChangedOnDisk(now)) => {
            assert_eq!(now.len, 1);
            assert_eq!(now.hash, xxh3(b"b"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(fs::read(d.path().join("f.sql")).unwrap(), b"b");
    // Without `expect` (the user chose Overwrite), the save goes through.
    let stamp = save(&root(&d), &rel("f.sql"), None, text("c"), None).unwrap();
    assert_eq!(fs::read(d.path().join("f.sql")).unwrap(), b"c");
    assert_eq!(stamp.hash, xxh3(b"c"));
}

/// alg/28 `save_detects_change_since_open`: a same-length rewrite is caught by the hash.
#[test]
fn save_detects_change_since_open() {
    let d = lib_with_bytes("f.sql", b"SELECT 1;");
    let r = read_default(&d, "f.sql");
    let mut forged = r.stamp;
    forged.hash ^= 1;
    assert!(matches!(
        save(&root(&d), &rel("f.sql"), None, text("x"), Some(forged)),
        Err(SaveError::ChangedOnDisk(now)) if now == r.stamp
    ));
    assert_eq!(fs::read(d.path().join("f.sql")).unwrap(), b"SELECT 1;");
    // The stamp the save returns is the one a read sees next.
    let saved = save(
        &root(&d),
        &rel("f.sql"),
        None,
        text("SELECT 2;"),
        Some(r.stamp),
    )
    .unwrap();
    assert_eq!(read_default(&d, "f.sql").stamp, saved);
}

#[test]
fn save_creates_a_new_file_and_refuses_one_deleted_since_open() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("sub")).unwrap();
    let s = save(
        &root(&d),
        &rel("sub/new.sql"),
        None,
        text("SELECT 1;\n"),
        None,
    )
    .unwrap();
    assert_eq!(
        fs::read(d.path().join("sub/new.sql")).unwrap(),
        b"SELECT 1;\n"
    );
    assert_eq!(s.len, 10);
    let r = read_default(&d, "sub/new.sql");
    fs::remove_file(d.path().join("sub/new.sql")).unwrap();
    assert_eq!(
        save(
            &root(&d),
            &rel("sub/new.sql"),
            None,
            text("x"),
            Some(r.stamp)
        ),
        Err(SaveError::DeletedOnDisk)
    );
    assert!(!d.path().join("sub/new.sql").exists());
    // "Save to recreate": no expectation.
    save(&root(&d), &rel("sub/new.sql"), None, text("x"), None).unwrap();
    assert_eq!(fs::read(d.path().join("sub/new.sql")).unwrap(), b"x");
    // A missing folder is not created.
    assert!(matches!(
        save(&root(&d), &rel("nope/a.sql"), None, text("x"), None),
        Err(SaveError::Io(std::io::ErrorKind::NotFound))
    ));
    assert!(!d.path().join("nope").exists());
}

// ## demofix-1 (PO demo F4)
/// "Deleted on disk: Save writes the file again" recreates the file, but never over a
/// different file that appeared at the path since (it asks, as any save over a change):
/// nothing there is created (no-replace), the tab's own file back is saved over, anything else
/// is `ChangedOnDisk` with what is there now.
#[test]
fn recreate_never_writes_over_a_different_file() {
    use hacivat_scripts::fileio::{SaveExpect, save_expecting};
    let save_as = |d: &tempfile::TempDir, s: &str, expect: SaveExpect| {
        save_expecting(&root(d), &rel("a.sql"), None, text(s), expect, &mut |_, _| {})
    };
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.sql"), "SELECT 1;\n").unwrap();
    let mine = read_default(&d, "a.sql").stamp;
    let recreate = SaveExpect::Recreate { last: Some(mine) };
    // Gone: created again.
    fs::remove_file(d.path().join("a.sql")).unwrap();
    let s = save_as(&d, "SELECT 2;\n", recreate).unwrap();
    assert_eq!(fs::read(d.path().join("a.sql")).unwrap(), b"SELECT 2;\n");
    assert_eq!(read_default(&d, "a.sql").stamp, s);
    // A different file at the path: refused with its stamp, the file untouched.
    fs::write(d.path().join("a.sql"), "-- someone else's\n").unwrap();
    let theirs = read_default(&d, "a.sql").stamp;
    assert_eq!(
        save_as(&d, "mine", recreate),
        Err(SaveError::ChangedOnDisk(theirs))
    );
    assert_eq!(
        fs::read(d.path().join("a.sql")).unwrap(),
        b"-- someone else's\n"
    );
    // A tab that never had the file (`last: None`) is refused over any file too.
    assert_eq!(
        save_as(&d, "mine", SaveExpect::Recreate { last: None }),
        Err(SaveError::ChangedOnDisk(theirs))
    );
    // The tab's own file back (the same stamp): saved over, as a normal save.
    save_as(&d, "mine", SaveExpect::Recreate { last: Some(theirs) }).unwrap();
    assert_eq!(fs::read(d.path().join("a.sql")).unwrap(), b"mine");
    // The old forms keep their meaning.
    assert!(matches!(
        save_as(&d, "x", SaveExpect::Stamp(mine)),
        Err(SaveError::ChangedOnDisk(_))
    ));
    save_as(&d, "any", SaveExpect::Anything).unwrap();
    assert_eq!(fs::read(d.path().join("a.sql")).unwrap(), b"any");
}
// ## demofix-1

#[test]
fn save_refuses_a_folder_and_hidden_paths() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("x.sql")).unwrap();
    fs::create_dir(d.path().join(".hacivat")).unwrap();
    let before = names_under(d.path());
    assert_eq!(
        save(&root(&d), &rel("x.sql"), None, text("x"), None),
        Err(SaveError::PathChanged)
    );
    // Never listed, so never written (C11): the manifest folder, a dot file.
    for hidden in [".hacivat/workspace.toml", ".a.sql", "sub/.git/config"] {
        assert_eq!(
            save(&root(&d), &rel(hidden), None, text("x"), None),
            Err(SaveError::PathRefused),
            "{hidden}"
        );
        assert_eq!(
            read(&root(&d), &rel(hidden), None, ReadLimits::default()).unwrap_err(),
            ReadError::PathRefused,
            "{hidden}"
        );
    }
    assert_eq!(names_under(d.path()), before);
    assert_eq!(
        read(&root(&d), &rel("x.sql"), None, ReadLimits::default()).unwrap_err(),
        ReadError::PathRefused
    );
}

#[test]
fn failed_save_leaves_original() {
    for fault in [
        faults::Fault::Write,
        faults::Fault::Sync,
        faults::Fault::Rename,
    ] {
        let d = lib_with_bytes("f.sql", b"SELECT 1;");
        let r = read_default(&d, "f.sql");
        faults::inject(fault);
        let got = save(
            &root(&d),
            &rel("f.sql"),
            None,
            text("DROP TABLE t;"),
            Some(r.stamp),
        );
        assert!(matches!(got, Err(SaveError::Io(_))), "{fault:?}: {got:?}");
        assert_eq!(
            fs::read(d.path().join("f.sql")).unwrap(),
            b"SELECT 1;",
            "{fault:?}"
        );
        assert_eq!(names_under(d.path()), ["f.sql"], "{fault:?}: no tmp left");
        // The fault fired once; the next save goes through.
        save(
            &root(&d),
            &rel("f.sql"),
            None,
            text("SELECT 2;"),
            Some(r.stamp),
        )
        .unwrap();
        assert_eq!(fs::read(d.path().join("f.sql")).unwrap(), b"SELECT 2;");
    }
}

#[test]
fn tmp_name_never_collides_with_a_script() {
    for name in ["a.sql", "notes.md", "RAPOR İ.SQL", "x"] {
        let t = fileio::temp_name_for(name);
        assert!(t.starts_with('.'), "{t}: the scanner skips dot files (C11)");
        assert!(t.ends_with(".tmp"), "{t}");
        let lower = t.to_ascii_lowercase();
        assert!(!lower.ends_with(".sql") && !lower.ends_with(".md"), "{t}");
        assert!(!t.contains('/') && !t.contains('\\'), "{t}");
        assert!(fileio::is_temp_name(&t), "{t}");
        assert!(!fileio::is_temp_name(name), "{name}");
        assert_ne!(t, fileio::temp_name_for(name), "a random suffix (R-T1a-4)");
    }
    // m10: a long name is cut on a UTF-8 boundary so the temp name fits 255 bytes.
    let long = format!("{}.sql", "ğ".repeat(124)); // 252 bytes
    let t = fileio::temp_name_for(&long);
    assert!(t.len() <= 255, "{}", t.len());
    assert!(fileio::is_temp_name(&t));
    let d = lib_with_bytes(&long, b"SELECT 1;");
    let r = read_default(&d, &long);
    save(
        &root(&d),
        &rel(&long),
        None,
        text("SELECT 2;"),
        Some(r.stamp),
    )
    .unwrap();
    assert_eq!(names_under(d.path()), std::slice::from_ref(&long));
    // The temp file a save really writes has that shape.
    faults::inject(faults::Fault::Rename);
    let _ = save(&root(&d), &rel(&long), None, text("x"), None);
    let used = faults::last_temp().expect("the save made a temp file");
    assert!(fileio::is_temp_name(&used), "{used}");
    assert!(used.len() <= 255);
}

#[test]
fn tier_thresholds() {
    assert_eq!(tier_for(0, 1), SizeTier::Full);
    assert_eq!(tier_for(4 << 20, 10), SizeTier::Full);
    assert_eq!(tier_for((4 << 20) + 1, 10), SizeTier::Reduced);
    assert_eq!(tier_for(20 << 20, 10), SizeTier::Reduced);
    assert_eq!(tier_for((20 << 20) + 1, 10), SizeTier::Preview);
    // UI-EDIT-23: more than 100,000 lines is Reduced however small the file.
    assert_eq!(tier_for(3 << 20, 100_000), SizeTier::Full);
    assert_eq!(tier_for(3 << 20, 100_001), SizeTier::Reduced);
}

#[test]
fn read_picks_the_tier_from_size_lines_and_line_length() {
    let d = lib_with_bytes("many.sql", &b"x\n".repeat(100_001));
    assert_eq!(read_default(&d, "many.sql").tier, SizeTier::Reduced);
    // A line longer than 64 KiB.
    let d = lib_with_bytes("long.sql", &vec![b'x'; (64 << 10) + 1]);
    assert_eq!(read_default(&d, "long.sql").tier, SizeTier::Reduced);
    let d = lib_with_bytes("ok.sql", &vec![b'x'; 64 << 10]);
    assert_eq!(read_default(&d, "ok.sql").tier, SizeTier::Full);
}

#[test]
fn preview_cuts_at_the_last_line_break_before_2_5_mb() {
    assert_eq!(PREVIEW_BYTES, 2_621_440);
    // 25 MB of 100-byte lines: LF-terminated, with a multi-byte character in each.
    let line = format!("INSERT INTO t VALUES ('ğ'); -- {}\n", "x".repeat(67));
    assert_eq!(line.len(), 100);
    let bytes = line.repeat(250_000).into_bytes();
    let d = lib_with_bytes("dump.sql", &bytes);
    let r = read_default(&d, "dump.sql");
    assert_eq!(r.tier, SizeTier::Preview);
    assert!(r.text.len() <= PREVIEW_BYTES);
    assert!(r.text.ends_with('\n'));
    assert_eq!(r.text.len(), PREVIEW_BYTES / 100 * 100);
    assert!(bytes.starts_with(r.text.as_bytes()));
    // The stamp is the whole file's, so a change anywhere is seen.
    assert_eq!(r.stamp.len, bytes.len() as u64);
    assert_eq!(r.stamp.hash, xxh3(&bytes));
    // UTF-16: the cut is in the text, never inside a character.
    let utf16 = utf16le_bom(&"SELECT 'ğüş';\r\n".repeat(800_000));
    assert!(utf16.len() > 20 << 20);
    let d = lib_with_bytes("u16.sql", &utf16);
    let r = read_default(&d, "u16.sql");
    assert_eq!(r.tier, SizeTier::Preview);
    assert!(r.text.ends_with("\r\n"));
    assert!(r.text.len() <= PREVIEW_BYTES);
    assert!(r.meta.bom);
    // Asked for the whole file (UI-EDIT-23 "Open anyway"): Reduced, all of it.
    let r = read(&root(&d), &rel("u16.sql"), None, ReadLimits { whole: true }).unwrap();
    assert_eq!(r.tier, SizeTier::Reduced);
    assert_eq!(r.text, "SELECT 'ğüş';\r\n".repeat(800_000));
}

#[test]
fn save_with_echo_reports_the_stamp_before_the_file_changes() {
    let d = lib_with_bytes("f.sql", b"old");
    let r = read_default(&d, "f.sql");
    let path = d.path().join("f.sql");
    let mut seen: Vec<(DiskStamp, Vec<u8>)> = Vec::new();
    let got = save_with_echo(
        &root(&d),
        &rel("f.sql"),
        None,
        text("new text"),
        Some(r.stamp),
        &mut |s| seen.push((*s, fs::read(&path).unwrap())),
    )
    .unwrap();
    // HV-SCRIPT-07 step 4: recorded before the rename, when the file still held "old".
    assert_eq!(seen, [(got, b"old".to_vec())]);
    assert_eq!(got.hash, xxh3(b"new text"));
    assert_eq!(read_default(&d, "f.sql").stamp, got);
}

#[test]
fn open_stream_yields_utf8_chunks_cut_at_char_boundaries() {
    // A 200 KiB UTF-16LE file with Turkish text (Task 1a's `open_stream` reads through this).
    let body = "SELECT 'ğüşıöçİ — 漢字 😀';\r\n".repeat(4000);
    let bytes = utf16le_bom(&body);
    assert!(bytes.len() > 200 << 10);
    let d = lib_with_bytes("big.sql", &bytes);
    let whole = read_default(&d, "big.sql");
    let mut s = open_stream(&root(&d), &rel("big.sql"), None, whole.meta.encoding).unwrap();
    let mut joined = String::new();
    let mut n = 0;
    while let Some(chunk) = s.next_chunk() {
        let chunk = chunk.unwrap();
        // A `String` is valid UTF-8 by type; the bound is one 64 KiB read, decoded (UTF-16 → at
        // most 3 bytes per 2, plus one character carried over the cut), so memory is one chunk.
        assert!(!chunk.is_empty());
        assert!(
            chunk.len() <= STREAM_CHUNK_BYTES / 2 * 3 + 4,
            "{}",
            chunk.len()
        );
        joined.push_str(&chunk);
        n += 1;
    }
    assert!(n >= 4, "{n} chunks");
    assert_eq!(joined, whole.text);
    assert!(s.next_chunk().is_none(), "fused at the end");
    // UTF-8 with a BOM: the BOM is not text; a character split by the 64 KiB cut is whole.
    let mut utf8 = vec![0xEF, 0xBB, 0xBF];
    utf8.extend(vec![b'a'; STREAM_CHUNK_BYTES - 4]);
    utf8.extend("ğğğ".as_bytes());
    let d = lib_with_bytes("u8.sql", &utf8);
    let r = read_default(&d, "u8.sql");
    let mut s = open_stream(&root(&d), &rel("u8.sql"), None, r.meta.encoding).unwrap();
    let mut joined = String::new();
    while let Some(c) = s.next_chunk() {
        joined.push_str(&c.unwrap());
    }
    assert_eq!(joined, r.text);
}

#[test]
fn open_stream_stops_on_bytes_that_do_not_decode() {
    let mut bytes = vec![b'a'; STREAM_CHUNK_BYTES];
    bytes.extend(b"\xFF\xFE\xFD");
    let d = lib_with_bytes("bad.sql", &bytes);
    let mut s = open_stream(
        &root(&d),
        &rel("bad.sql"),
        None,
        hacivat_scripts::TextEncoding::UTF_8,
    )
    .unwrap();
    assert_eq!(s.next_chunk().unwrap().unwrap().len(), STREAM_CHUNK_BYTES);
    assert_eq!(
        s.next_chunk(),
        Some(Err(hacivat_scripts::ChunkError::Decode))
    );
    assert_eq!(s.next_chunk(), None);
}

/// HV-SCRIPT-12 step 4 at the reader: a chunk read after the file changed is never handed out.
#[test]
fn open_stream_reports_a_change_on_disk() {
    let bytes = vec![b'a'; 3 * STREAM_CHUNK_BYTES];
    // In place: same file, rewritten after the first chunk.
    let d = lib_with_bytes("f.sql", &bytes);
    let mut s = open_stream(
        &root(&d),
        &rel("f.sql"),
        None,
        hacivat_scripts::TextEncoding::UTF_8,
    )
    .unwrap();
    assert!(s.next_chunk().unwrap().is_ok());
    {
        use std::io::Write;
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(d.path().join("f.sql"))
            .unwrap();
        f.write_all(b"DROP TABLE t;").unwrap();
    }
    assert_eq!(
        s.next_chunk(),
        Some(Err(hacivat_scripts::ChunkError::Changed))
    );
    assert_eq!(s.next_chunk(), None);
    // Rename-replace: another file renamed over the path.
    let d = lib_with_bytes("g.sql", &bytes);
    let mut s = open_stream(
        &root(&d),
        &rel("g.sql"),
        None,
        hacivat_scripts::TextEncoding::UTF_8,
    )
    .unwrap();
    assert!(s.next_chunk().unwrap().is_ok());
    fs::write(d.path().join("other"), vec![b'b'; bytes.len()]).unwrap();
    fs::rename(d.path().join("other"), d.path().join("g.sql")).unwrap();
    assert_eq!(
        s.next_chunk(),
        Some(Err(hacivat_scripts::ChunkError::Changed))
    );
}

// ## m4b-t05-fileio fix round 1

/// I4: a preview holds only the start of the file and can never be saved over it.
#[test]
fn preview_cannot_be_saved() {
    let line = format!("SELECT {};\n", "1".repeat(90));
    let bytes = line.repeat(230_000).into_bytes();
    assert!(bytes.len() as u64 > 20 << 20);
    let d = lib_with_bytes("dump.sql", &bytes);
    let r = read_default(&d, "dump.sql");
    assert_eq!(r.tier, SizeTier::Preview);
    assert!(r.save_text().is_none());
    // A whole read of the same file can be.
    let whole = read(
        &root(&d),
        &rel("dump.sql"),
        None,
        ReadLimits { whole: true },
    )
    .unwrap();
    assert!(whole.save_text().is_some());
    assert_eq!(
        fs::read(d.path().join("dump.sql")).unwrap().len(),
        bytes.len()
    );
}

/// I1 (R-T5-1) on the rename path: a write that stops part-way (the ENOSPC stand-in) leaves the
/// original and no temp.
#[test]
fn partial_write_on_the_rename_path_leaves_original() {
    let d = lib_with_bytes("f.sql", b"SELECT 1;\n");
    let r = read_default(&d, "f.sql");
    faults::inject(faults::Fault::WriteAfter(5));
    let got = save(
        &root(&d),
        &rel("f.sql"),
        None,
        text("DELETE FROM t WHERE id = 1;\n"),
        Some(r.stamp),
    );
    assert!(matches!(got, Err(SaveError::Io(_))), "{got:?}");
    assert_eq!(fs::read(d.path().join("f.sql")).unwrap(), b"SELECT 1;\n");
    assert_eq!(names_under(d.path()), ["f.sql"]);
}

/// R-T5-1: a new file whose write stops part-way leaves nothing behind.
#[test]
fn partial_write_of_a_new_file_leaves_nothing() {
    let d = tempfile::tempdir().unwrap();
    faults::inject(faults::Fault::WriteAfter(3));
    assert!(matches!(
        save(&root(&d), &rel("new.sql"), None, text("SELECT 1;"), None),
        Err(SaveError::Io(_))
    ));
    assert!(names_under(d.path()).is_empty());
}

/// m2: our own temps a crash left, older than a minute, are swept on the next save in that
/// folder; a younger one, and names that only look similar, stay.
#[test]
fn stale_temps_are_swept_on_save() {
    let d = lib_with_bytes("f.sql", b"a");
    let old = format!(".f.sql.hv-{}.tmp", "00112233445566778899aabbccddeeff");
    let young = format!(".f.sql.hv-{}.tmp", "fedcba9876543210".repeat(2));
    let upper = format!(".f.sql.hv-{}.tmp", "0123456789ABCDEF".repeat(2));
    let short = ".f.sql.hv-12.tmp".to_owned();
    for n in [&old, &young, &upper, &short] {
        fs::write(d.path().join(n), b"x").unwrap();
    }
    let past = std::time::SystemTime::now() - std::time::Duration::from_secs(120);
    for n in [&old, &upper, &short] {
        fs::File::options()
            .write(true)
            .open(d.path().join(n))
            .unwrap()
            .set_modified(past)
            .unwrap();
    }
    let r = read_default(&d, "f.sql");
    save(&root(&d), &rel("f.sql"), None, text("b"), Some(r.stamp)).unwrap();
    let left = names_under(d.path());
    assert!(!left.contains(&old), "{left:?}");
    for kept in [&young, &upper, &short] {
        assert!(left.contains(kept), "{kept}: {left:?}");
    }
    assert_eq!(left.len(), 4);
}

/// alg/28 `temp_file_is_never_listed`: the scanner never lists a temp a save makes.
#[test]
fn temp_file_is_never_listed() {
    let d = lib_with_bytes("a.sql", b"SELECT 1;");
    fs::write(d.path().join(fileio::temp_name_for("a.sql")), b"x").unwrap();
    fs::write(d.path().join(fileio::temp_name_for("b.md")), b"x").unwrap();
    let listed = hacivat_scripts::scan::scan(
        &root(&d),
        &hacivat_scripts::scan::Limits::default(),
        &hacivat_scripts::scan::RealFs,
    );
    let rels: Vec<&str> = listed.entries.iter().map(|e| e.rel.as_str()).collect();
    assert_eq!(rels, ["a.sql"]);
}

/// fr2 I-fr1-1 (R-T5-6): the environment is a required argument of every public function. A
/// `@production` file is refused under staging and under `None` (an untagged connection or an
/// unbound tab) by each of them, and nothing changes on disk; under production it opens.
#[test]
fn every_public_function_refuses_what_the_environment_does_not_show() {
    use hacivat_proto::EnvTag;
    let d = lib_with_bytes("@production/purge.sql", b"DELETE FROM t;");
    let r = root(&d);
    let p = rel("@production/purge.sql");
    let ok = read(&r, &p, Some(EnvTag::Production), ReadLimits::default()).unwrap();
    let enc = hacivat_scripts::TextEncoding::UTF_8;
    for env in [Some(EnvTag::Staging), Some(EnvTag::Dev), None] {
        assert_eq!(
            read(&r, &p, env, ReadLimits::default()).unwrap_err(),
            ReadError::OutOfScope,
            "{env:?}"
        );
        assert_eq!(
            read_with(&r, &p, env, "windows-1254").unwrap_err(),
            ReadError::OutOfScope,
            "{env:?}"
        );
        assert_eq!(
            save(&r, &p, env, text("x"), Some(ok.stamp)),
            Err(SaveError::OutOfScope),
            "{env:?}"
        );
        assert_eq!(
            save(&r, &p, env, text("x"), None),
            Err(SaveError::OutOfScope),
            "{env:?}"
        );
        assert_eq!(
            save_with_echo(&r, &p, env, text("x"), None, &mut |_| panic!("no echo")),
            Err(SaveError::OutOfScope),
            "{env:?}"
        );
        assert!(
            matches!(open_stream(&r, &p, env, enc), Err(ReadError::OutOfScope)),
            "{env:?}"
        );
        // A new file there is refused too.
        assert_eq!(
            save(&r, &rel("@production/new.sql"), env, text("x"), None),
            Err(SaveError::OutOfScope),
            "{env:?}"
        );
    }
    assert_eq!(
        names_under(d.path()),
        ["@production", "@production/purge.sql"]
    );
    assert_eq!(
        fs::read(d.path().join("@production/purge.sql")).unwrap(),
        b"DELETE FROM t;"
    );
    assert!(open_stream(&r, &p, Some(EnvTag::Production), enc).is_ok());
    save(&r, &p, Some(EnvTag::Production), text("x"), Some(ok.stamp)).unwrap();
    // An unscoped file opens under every environment, `None` included.
    let d = lib_with_bytes("plain.sql", b"SELECT 1;");
    for env in [Some(EnvTag::Production), Some(EnvTag::Staging), None] {
        assert!(read(&root(&d), &rel("plain.sql"), env, ReadLimits::default()).is_ok());
    }
}

/// fr2 I-fr1-2: the reviewer's route. An editor Save builds `SaveText` from its buffer and the
/// tab's meta, with the tab's stamp; for a preview tab that meta is `partial`, and the save is
/// refused whatever the text, with or without `expect`.
#[test]
fn a_preview_tabs_meta_cannot_be_saved() {
    let line = format!("SELECT {};\n", "1".repeat(90));
    let bytes = line.repeat(230_000).into_bytes();
    assert!(bytes.len() > 22_000_000);
    let d = lib_with_bytes("dump.sql", &bytes);
    let r = read_default(&d, "dump.sql");
    assert_eq!(r.tier, SizeTier::Preview);
    assert!(r.meta.partial);
    for expect in [Some(r.stamp), None] {
        for body in [r.text.clone(), "SELECT 1;\n".to_owned()] {
            let tab = SaveText {
                text: body,
                meta: r.meta,
            };
            assert_eq!(
                save(&root(&d), &rel("dump.sql"), None, tab, expect),
                Err(SaveError::PreviewOnly)
            );
        }
    }
    let after = fs::read(d.path().join("dump.sql")).unwrap();
    assert_eq!(after.len(), bytes.len());
    assert_eq!(xxh3(&after), xxh3(&bytes));
    assert_eq!(names_under(d.path()), ["dump.sql"]);
    // A whole read's meta is not partial.
    let whole = read(
        &root(&d),
        &rel("dump.sql"),
        None,
        ReadLimits { whole: true },
    )
    .unwrap();
    assert!(!whole.meta.partial);
}

/// m8: a preview read in a chosen encoding gets the same loss check as a whole read.
#[test]
fn preview_in_a_chosen_encoding_is_checked_for_loss() {
    let mut bytes = vec![b'a', 0x87, 0x90, b'\n'];
    bytes.extend(b"SELECT 1;\n".repeat(2_200_000));
    assert!(bytes.len() as u64 > 20 << 20);
    let d = lib_with_bytes("sj.sql", &bytes);
    assert_eq!(
        read_with(&root(&d), &rel("sj.sql"), None, "Shift_JIS").unwrap_err(),
        ReadError::Lossy
    );
    // Without the NEC row, the preview reads.
    let d = lib_with_bytes("ok.sql", &bytes[4..]);
    let r = read_with(&root(&d), &rel("ok.sql"), None, "Shift_JIS").unwrap();
    assert_eq!(r.tier, SizeTier::Preview);
}

/// m4: a new-file save never replaces a file that appeared after its check.
#[test]
fn new_file_save_never_replaces_one_that_appeared() {
    let d = tempfile::tempdir().unwrap();
    let theirs = d.path().join("new.sql");
    faults::before_publish(move || fs::write(theirs, b"theirs").unwrap());
    assert!(matches!(
        save(&root(&d), &rel("new.sql"), None, text("mine"), None),
        Err(SaveError::ChangedOnDisk(now)) if now.hash == xxh3(b"theirs")
    ));
    assert_eq!(fs::read(d.path().join("new.sql")).unwrap(), b"theirs");
    assert_eq!(names_under(d.path()), ["new.sql"]);
}

/// m1: Linux without procfs refuses every write (T3's rule), and still reads.
#[test]
fn without_procfs_saves_refuse_and_reads_work() {
    let d = lib_with_bytes("f.sql", b"SELECT 1;");
    let r = read_default(&d, "f.sql");
    let got = hacivat_scripts::manifest::with_procfs_absent(|| {
        (
            save(&root(&d), &rel("f.sql"), None, text("x"), Some(r.stamp)),
            save(&root(&d), &rel("new.sql"), None, text("x"), None),
            read(&root(&d), &rel("f.sql"), None, ReadLimits::default()).map(|r| r.text),
        )
    });
    assert_eq!(got.0, Err(SaveError::Io(std::io::ErrorKind::Unsupported)));
    assert_eq!(got.1, Err(SaveError::Io(std::io::ErrorKind::Unsupported)));
    assert_eq!(got.2.unwrap(), "SELECT 1;");
    assert_eq!(names_under(d.path()), ["f.sql"]);
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    use super::*;

    fn mode(p: &Path) -> u32 {
        fs::symlink_metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn save_keeps_mode_and_link_target() {
        let d = lib_with_bytes("real.sql", b"SELECT 1;");
        fs::set_permissions(d.path().join("real.sql"), fs::Permissions::from_mode(0o755)).unwrap();
        let r = read_default(&d, "real.sql");
        save(
            &root(&d),
            &rel("real.sql"),
            None,
            text("SELECT 2;"),
            Some(r.stamp),
        )
        .unwrap();
        assert_eq!(mode(&d.path().join("real.sql")), 0o755);
        // An InsideLink 'l.sql' -> 'real.sql': the save writes real.sql, l.sql stays a link.
        symlink("real.sql", d.path().join("l.sql")).unwrap();
        let r = read_default(&d, "l.sql");
        assert_eq!(r.text, "SELECT 2;");
        save(
            &root(&d),
            &rel("l.sql"),
            None,
            text("SELECT 3;"),
            Some(r.stamp),
        )
        .unwrap();
        assert!(
            fs::symlink_metadata(d.path().join("l.sql"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(d.path().join("real.sql")).unwrap(), b"SELECT 3;");
        assert_eq!(mode(&d.path().join("real.sql")), 0o755);
        // A link into a sub-folder, through `..` in its text, still lands inside.
        fs::create_dir(d.path().join("sub")).unwrap();
        symlink("../real.sql", d.path().join("sub/up.sql")).unwrap();
        let r = read_default(&d, "sub/up.sql");
        save(
            &root(&d),
            &rel("sub/up.sql"),
            None,
            text("SELECT 4;"),
            Some(r.stamp),
        )
        .unwrap();
        assert_eq!(fs::read(d.path().join("real.sql")).unwrap(), b"SELECT 4;");
        // A new file takes the umask's mode, never the 0600 of the temp (fr1 m9: compared
        // with a file the same umask made, so the test holds under any umask).
        save(&root(&d), &rel("new.sql"), None, text("x"), None).unwrap();
        fs::write(d.path().join("probe.txt"), b"").unwrap();
        assert_eq!(
            mode(&d.path().join("new.sql")),
            mode(&d.path().join("probe.txt"))
        );
    }

    #[test]
    fn save_keeps_hard_link() {
        let d = lib_with_bytes("a.sql", b"SELECT 1;");
        let other = tempfile::tempdir().unwrap();
        fs::hard_link(d.path().join("a.sql"), d.path().join("b.sql")).unwrap();
        let r = read_default(&d, "a.sql");
        let ino = fs::metadata(d.path().join("a.sql")).unwrap().ino();
        save(
            &root(&d),
            &rel("a.sql"),
            None,
            text("SELECT 22;"),
            Some(r.stamp),
        )
        .unwrap();
        // Written in place: both names still show the same inode and the new bytes.
        assert_eq!(fs::metadata(d.path().join("a.sql")).unwrap().ino(), ino);
        assert_eq!(fs::read(d.path().join("b.sql")).unwrap(), b"SELECT 22;");
        assert_eq!(fs::metadata(d.path().join("a.sql")).unwrap().nlink(), 2);
        // The stamp check runs first there too.
        fs::write(d.path().join("b.sql"), b"theirs").unwrap();
        assert!(matches!(
            save(&root(&d), &rel("a.sql"), None, text("mine"), Some(r.stamp)),
            Err(SaveError::ChangedOnDisk(_))
        ));
        assert_eq!(fs::read(d.path().join("a.sql")).unwrap(), b"theirs");
        // A shorter text is not padded with the old tail.
        let r = read_default(&d, "a.sql");
        save(&root(&d), &rel("a.sql"), None, text("x"), Some(r.stamp)).unwrap();
        assert_eq!(fs::read(d.path().join("b.sql")).unwrap(), b"x");
        assert!(names_under(other.path()).is_empty());
        assert_eq!(names_under(d.path()), ["a.sql", "b.sql"]);
    }

    #[test]
    fn save_refuses_a_path_that_became_a_link() {
        let outside = lib_with_bytes("target.sql", b"outside");
        let d = lib_with_bytes("f.sql", b"SELECT 1;");
        let r = read_default(&d, "f.sql");
        fs::remove_file(d.path().join("f.sql")).unwrap();
        symlink(outside.path().join("target.sql"), d.path().join("f.sql")).unwrap();
        assert_eq!(
            save(&root(&d), &rel("f.sql"), None, text("pwned"), Some(r.stamp)),
            Err(SaveError::PathChanged)
        );
        assert_eq!(
            save(&root(&d), &rel("f.sql"), None, text("pwned"), None),
            Err(SaveError::PathChanged)
        );
        assert_eq!(
            fs::read(outside.path().join("target.sql")).unwrap(),
            b"outside"
        );
        assert_eq!(
            read(&root(&d), &rel("f.sql"), None, ReadLimits::default()).unwrap_err(),
            ReadError::PathRefused
        );
        assert!(
            open_stream(
                &root(&d),
                &rel("f.sql"),
                None,
                hacivat_scripts::TextEncoding::UTF_8
            )
            .is_err()
        );
        // A folder on the way that became a link to outside.
        let d = lib_with_bytes("sub/a.sql", b"SELECT 1;");
        let r = read_default(&d, "sub/a.sql");
        fs::rename(d.path().join("sub"), d.path().join("moved")).unwrap();
        fs::create_dir(outside.path().join("sub")).unwrap();
        fs::write(outside.path().join("sub/a.sql"), b"SELECT 1;").unwrap();
        symlink(outside.path().join("sub"), d.path().join("sub")).unwrap();
        assert_eq!(
            save(
                &root(&d),
                &rel("sub/a.sql"),
                None,
                text("pwned"),
                Some(r.stamp)
            ),
            Err(SaveError::PathChanged)
        );
        assert_eq!(
            save(&root(&d), &rel("sub/new.sql"), None, text("pwned"), None),
            Err(SaveError::PathChanged)
        );
        assert_eq!(names_under(&outside.path().join("sub")), ["a.sql"]);
        assert_eq!(
            fs::read(outside.path().join("sub/a.sql")).unwrap(),
            b"SELECT 1;"
        );
        // A dangling link, and a link into a hidden folder of the library.
        let d = lib_with_bytes(".hacivat/workspace.toml", b"format = 1\n");
        symlink("missing.sql", d.path().join("dangling.sql")).unwrap();
        symlink(".hacivat/workspace.toml", d.path().join("m.sql")).unwrap();
        for name in ["dangling.sql", "m.sql"] {
            assert_eq!(
                save(&root(&d), &rel(name), None, text("x"), None),
                Err(SaveError::PathChanged),
                "{name}"
            );
        }
        assert!(!d.path().join("missing.sql").exists());
        assert_eq!(
            fs::read(d.path().join(".hacivat/workspace.toml")).unwrap(),
            b"format = 1\n"
        );
    }

    #[test]
    fn read_follows_an_inside_link_only() {
        let outside = lib_with_bytes("secret.sql", b"outside");
        let d = lib_with_bytes("real.sql", b"inside");
        symlink(outside.path().join("secret.sql"), d.path().join("out.sql")).unwrap();
        symlink("real.sql", d.path().join("in.sql")).unwrap();
        symlink("loop.sql", d.path().join("loop.sql")).unwrap();
        assert_eq!(read_default(&d, "in.sql").text, "inside");
        for name in ["out.sql", "loop.sql"] {
            assert_eq!(
                read(&root(&d), &rel(name), None, ReadLimits::default()).unwrap_err(),
                ReadError::PathRefused,
                "{name}"
            );
        }
    }

    #[test]
    fn fifo_is_never_opened() {
        let d = tempfile::tempdir().unwrap();
        let ok = std::process::Command::new("mkfifo")
            .arg(d.path().join("p.sql"))
            .status()
            .unwrap();
        assert!(ok.success());
        assert_eq!(
            read(&root(&d), &rel("p.sql"), None, ReadLimits::default()).unwrap_err(),
            ReadError::PathRefused
        );
        assert_eq!(
            save(&root(&d), &rel("p.sql"), None, text("x"), None),
            Err(SaveError::PathChanged)
        );
    }

    /// A folder the user cannot write: the temp file cannot be made, the original stays.
    #[test]
    fn failed_save_in_a_read_only_folder_leaves_original() {
        let d = lib_with_bytes("sub/f.sql", b"SELECT 1;");
        let sub = d.path().join("sub");
        let r = read_default(&d, "sub/f.sql");
        fs::set_permissions(&sub, fs::Permissions::from_mode(0o555)).unwrap();
        if fs::write(sub.join("probe"), b"").is_ok() {
            // Running as root: permissions do not apply; nothing to test.
            fs::remove_file(sub.join("probe")).unwrap();
            fs::set_permissions(&sub, fs::Permissions::from_mode(0o755)).unwrap();
            return;
        }
        let got = save(&root(&d), &rel("sub/f.sql"), None, text("x"), Some(r.stamp));
        fs::set_permissions(&sub, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            got,
            Err(SaveError::Io(std::io::ErrorKind::PermissionDenied))
        );
        assert_eq!(fs::read(sub.join("f.sql")).unwrap(), b"SELECT 1;");
        assert_eq!(names_under(&sub), ["f.sql"]);
    }

    /// alg/28 edge case: a read-only file is not replaced (the tab stays dirty).
    #[test]
    fn read_only_file_is_not_replaced() {
        let d = lib_with_bytes("f.sql", b"SELECT 1;");
        let p = d.path().join("f.sql");
        fs::set_permissions(&p, fs::Permissions::from_mode(0o444)).unwrap();
        if fs::OpenOptions::new().write(true).open(&p).is_ok() {
            return; // root
        }
        let r = read_default(&d, "f.sql");
        assert_eq!(
            save(&root(&d), &rel("f.sql"), None, text("x"), Some(r.stamp)),
            Err(SaveError::Io(std::io::ErrorKind::PermissionDenied))
        );
        assert_eq!(fs::read(&p).unwrap(), b"SELECT 1;");
        assert_eq!(mode(&p), 0o444);
        assert_eq!(names_under(d.path()), ["f.sql"]);
    }

    /// The library root itself replaced by a link after it was registered.
    #[test]
    fn root_that_became_a_link_is_refused() {
        let base = tempfile::tempdir().unwrap();
        let outside = lib_with_bytes("a.sql", b"outside");
        let lib = base.path().join("lib");
        fs::create_dir(&lib).unwrap();
        fs::write(lib.join("a.sql"), b"inside").unwrap();
        let r = LibraryRoot {
            id: LibraryId::new(),
            path: lib.clone(),
            canonical: lib.canonicalize().unwrap(),
        };
        fs::rename(&lib, base.path().join("old")).unwrap();
        symlink(outside.path(), &lib).unwrap();
        assert_eq!(
            read(&r, &rel("a.sql"), None, ReadLimits::default()).unwrap_err(),
            ReadError::PathRefused
        );
        assert_eq!(
            save(&r, &rel("a.sql"), None, text("pwned"), None),
            Err(SaveError::PathChanged)
        );
        assert_eq!(fs::read(outside.path().join("a.sql")).unwrap(), b"outside");
    }

    #[test]
    fn nfd_name_is_kept_on_save() {
        let nfd = "mu\u{0308}s.sql";
        let d = lib_with_bytes(nfd, b"SELECT 1;");
        let r = read_default(&d, nfd);
        save(&root(&d), &rel(nfd), None, text("SELECT 2;"), Some(r.stamp)).unwrap();
        let names: Vec<String> = fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        // The save reused the name's bytes: still NFD, not composed (fr1 m9).
        assert_eq!(names, [nfd.to_owned()]);
        assert_eq!(fs::read(d.path().join(nfd)).unwrap(), b"SELECT 2;");
    }

    #[test]
    fn hard_link_save_keeps_owner_mode() {
        let d = lib_with_bytes("a.sql", b"x");
        fs::hard_link(d.path().join("a.sql"), d.path().join("b.sql")).unwrap();
        fs::set_permissions(d.path().join("a.sql"), fs::Permissions::from_mode(0o640)).unwrap();
        let r = read_default(&d, "a.sql");
        let before = fs::metadata(d.path().join("a.sql")).unwrap();
        save(&root(&d), &rel("a.sql"), None, text("y"), Some(r.stamp)).unwrap();
        let after = fs::metadata(d.path().join("b.sql")).unwrap();
        assert_eq!(mode(&d.path().join("b.sql")), 0o640);
        // fr1 m9: the owner too (the same inode, written in place).
        assert_eq!((after.uid(), after.gid()), (before.uid(), before.gid()));
        assert_eq!(after.ino(), before.ino());
    }

    /// I1 (R-T5-1): a hard-linked file whose in-place write stops part-way is written back:
    /// every name holds the original bytes, and the stamp still matches.
    #[test]
    fn partial_in_place_write_restores_the_original() {
        let original = b"DELETE FROM t WHERE id = 1;\n".to_vec();
        let d = lib_with_bytes("a.sql", &original);
        fs::hard_link(d.path().join("a.sql"), d.path().join("b.sql")).unwrap();
        let r = read_default(&d, "a.sql");
        faults::inject(faults::Fault::WriteAfter(8));
        let got = save(
            &root(&d),
            &rel("a.sql"),
            None,
            text("SELECT 'a much longer replacement text';\n"),
            Some(r.stamp),
        );
        assert!(matches!(got, Err(SaveError::Io(_))), "{got:?}");
        for n in ["a.sql", "b.sql"] {
            assert_eq!(fs::read(d.path().join(n)).unwrap(), original, "{n}");
        }
        // The editor's stamp still matches, so the next save goes through.
        assert_eq!(read_default(&d, "a.sql").stamp, r.stamp);
        save(
            &root(&d),
            &rel("a.sql"),
            None,
            text("SELECT 2;"),
            Some(r.stamp),
        )
        .unwrap();
        assert_eq!(fs::read(d.path().join("b.sql")).unwrap(), b"SELECT 2;");
        assert_eq!(names_under(d.path()), ["a.sql", "b.sql"]);
    }

    /// R-T5-1: when the write-back fails too, the answer is `Damaged`, never a plain error.
    #[test]
    fn failed_restore_reports_damage() {
        let d = lib_with_bytes("a.sql", b"SELECT 1;\n");
        fs::hard_link(d.path().join("a.sql"), d.path().join("b.sql")).unwrap();
        let r = read_default(&d, "a.sql");
        faults::inject_both(faults::Fault::WriteAfter(4), faults::Fault::Restore);
        assert_eq!(
            save(
                &root(&d),
                &rel("a.sql"),
                None,
                text("DROP TABLE t;\n"),
                Some(r.stamp)
            ),
            Err(SaveError::Damaged)
        );
    }

    /// m3: the in-place path re-checks the file right before it writes.
    #[test]
    fn in_place_save_refuses_a_change_since_the_check() {
        let d = lib_with_bytes("a.sql", b"SELECT 1;");
        fs::hard_link(d.path().join("a.sql"), d.path().join("b.sql")).unwrap();
        let r = read_default(&d, "a.sql");
        // Another writer changes the file after the stamp check, before the write (same
        // length, so only the hash and the times tell).
        let other = d.path().join("b.sql");
        faults::before_publish(move || fs::write(other, b"SELECT 9;").unwrap());
        assert!(matches!(
            save(&root(&d), &rel("a.sql"), None, text("mine"), Some(r.stamp)),
            Err(SaveError::ChangedOnDisk(now)) if now.hash == xxh3(b"SELECT 9;")
        ));
        assert_eq!(fs::read(d.path().join("a.sql")).unwrap(), b"SELECT 9;");
    }

    /// I2 (R-T5-2): the reviewer's bounce link leaves the root and comes back; it is refused
    /// and nothing outside is touched.
    #[test]
    fn link_that_leaves_the_root_and_returns_is_refused() {
        let base = tempfile::tempdir().unwrap();
        let lib = base.path().join("lib");
        let out = base.path().join("out");
        fs::create_dir(&lib).unwrap();
        fs::create_dir(&out).unwrap();
        fs::write(lib.join("real.sql"), b"inside").unwrap();
        let canonical = lib.canonicalize().unwrap();
        symlink(canonical.join("real.sql"), out.join("bounce.sql")).unwrap();
        symlink("../out/bounce.sql", lib.join("l.sql")).unwrap();
        let r = LibraryRoot {
            id: LibraryId::new(),
            path: lib.clone(),
            canonical,
        };
        assert_eq!(
            read(&r, &rel("l.sql"), None, ReadLimits::default()).unwrap_err(),
            ReadError::PathRefused
        );
        assert_eq!(
            save(&r, &rel("l.sql"), None, text("pwned"), None),
            Err(SaveError::PathChanged)
        );
        assert_eq!(fs::read(lib.join("real.sql")).unwrap(), b"inside");
        assert_eq!(names_under(&out), ["bounce.sql"]);
    }

    /// fr2 mf1: a write-back whose last sync fails is not a restore: `Damaged`.
    #[test]
    fn failed_restore_sync_reports_damage() {
        let d = lib_with_bytes("a.sql", b"SELECT 1;\n");
        fs::hard_link(d.path().join("a.sql"), d.path().join("b.sql")).unwrap();
        let r = read_default(&d, "a.sql");
        faults::inject_both(faults::Fault::WriteAfter(4), faults::Fault::RestoreSync);
        assert_eq!(
            save(
                &root(&d),
                &rel("a.sql"),
                None,
                text("DROP TABLE t;\n"),
                Some(r.stamp)
            ),
            Err(SaveError::Damaged)
        );
    }

    /// The folder `sub` moves to `moved` (and a new, empty `sub` takes its name) right after a
    /// save published its bytes.
    fn move_folder_after_publish(d: &tempfile::TempDir) {
        let base = d.path().to_path_buf();
        faults::after_publish(move || {
            fs::rename(base.join("sub"), base.join("moved")).unwrap();
            fs::create_dir(base.join("sub")).unwrap();
        });
    }

    /// fr2 mf2: once the new bytes are on disk, a folder check that fails after it is
    /// `SavedButPathChanged` with the new stamp, never a plain failure: on the rename path, the
    /// in-place path and the new-file path.
    #[test]
    fn a_folder_moved_after_the_write_is_a_completed_save() {
        // Rename path.
        let d = lib_with_bytes("sub/f.sql", b"old");
        let r = read_default(&d, "sub/f.sql");
        move_folder_after_publish(&d);
        match save(
            &root(&d),
            &rel("sub/f.sql"),
            None,
            text("new"),
            Some(r.stamp),
        ) {
            Err(SaveError::SavedButPathChanged(stamp)) => assert_eq!(stamp.hash, xxh3(b"new")),
            other => panic!("{other:?}"),
        }
        assert_eq!(fs::read(d.path().join("moved/f.sql")).unwrap(), b"new");
        assert_eq!(names_under(&d.path().join("moved")), ["f.sql"]);
        // In place (hard-linked).
        let d = lib_with_bytes("sub/a.sql", b"old");
        fs::hard_link(d.path().join("sub/a.sql"), d.path().join("sub/b.sql")).unwrap();
        let r = read_default(&d, "sub/a.sql");
        move_folder_after_publish(&d);
        assert!(matches!(
            save(
                &root(&d),
                &rel("sub/a.sql"),
                None,
                text("new"),
                Some(r.stamp)
            ),
            Err(SaveError::SavedButPathChanged(_))
        ));
        assert_eq!(fs::read(d.path().join("moved/b.sql")).unwrap(), b"new");
        // A new file.
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("sub")).unwrap();
        move_folder_after_publish(&d);
        assert!(matches!(
            save(&root(&d), &rel("sub/n.sql"), None, text("new"), None),
            Err(SaveError::SavedButPathChanged(_))
        ));
        assert_eq!(fs::read(d.path().join("moved/n.sql")).unwrap(), b"new");
        assert_eq!(names_under(&d.path().join("moved")), ["n.sql"]);
    }
}

#[cfg(windows)]
mod windows {
    use super::*;

    /// A directory junction made with `mklink /J` (no privilege needed).
    fn junction(link: &Path, target: &Path) {
        let ok = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(link)
            .arg(target)
            .status()
            .unwrap();
        assert!(ok.success());
    }

    #[test]
    fn save_refuses_a_junctioned_folder() {
        let outside = lib_with_bytes("a.sql", b"outside");
        let d = tempfile::tempdir().unwrap();
        junction(&d.path().join("sub"), outside.path());
        assert_eq!(
            save(&root(&d), &rel("sub/a.sql"), None, text("pwned"), None),
            Err(SaveError::PathChanged)
        );
        assert_eq!(
            read(&root(&d), &rel("sub/a.sql"), None, ReadLimits::default()).unwrap_err(),
            ReadError::PathRefused
        );
        assert_eq!(fs::read(outside.path().join("a.sql")).unwrap(), b"outside");
        assert_eq!(names_under(outside.path()), ["a.sql"]);
    }

    #[test]
    fn device_and_stream_names_are_refused() {
        let d = tempfile::tempdir().unwrap();
        for bad in ["con.sql", "NUL", "a.sql:stream", "trail.sql.", "trail.sql "] {
            assert_eq!(
                save(&root(&d), &rel(bad), None, text("x"), None),
                Err(SaveError::PathRefused),
                "{bad:?}"
            );
        }
        assert!(names_under(d.path()).is_empty());
    }

    /// I2: an absolute UNC link target is refused as text, without a network call.
    #[test]
    fn unc_link_target_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let target = r"\\hv-unreachable.invalid\share\x.sql";
        if std::os::windows::fs::symlink_file(target, d.path().join("l.sql")).is_err() {
            return; // file links need Developer Mode or the privilege on this runner
        }
        assert_eq!(
            read(&root(&d), &rel("l.sql"), None, ReadLimits::default()).unwrap_err(),
            ReadError::PathRefused
        );
    }
}
