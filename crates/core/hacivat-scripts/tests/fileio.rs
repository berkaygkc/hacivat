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
    read(&root(d), &rel(name), ReadLimits::default()).unwrap()
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
fn names_under(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            out.push(p.strip_prefix(dir).unwrap().to_string_lossy().into_owned());
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
        let r = read(&root(&d), &rel("f.sql"), ReadLimits::default())
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
        save(&root(&d), &rel("f.sql"), SaveText::from(&r), Some(r.stamp))
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
        read(&root(&d), &rel("tr.sql"), ReadLimits::default()),
        Err(ReadError::NeedsEncoding { .. })
    ));
    let r = read_with(&root(&d), &rel("tr.sql"), "windows-1254").unwrap();
    assert_eq!(r.text, "SELECT 'İĞŞ ğüşıöçİ';\r\n");
    assert_eq!(r.meta.encoding.name(), "windows-1254");
    assert!(!r.meta.bom);
    // ISO-8859-9 is the same encoding to WHATWG.
    let iso = read_with(&root(&d), &rel("tr.sql"), "ISO-8859-9").unwrap();
    assert_eq!(iso.meta.encoding.name(), "windows-1254");
    save(&root(&d), &rel("tr.sql"), SaveText::from(&r), Some(r.stamp)).unwrap();
    assert_eq!(fs::read(d.path().join("tr.sql")).unwrap(), bytes);
    // An edit is encoded back in the file's encoding, never as UTF-8.
    let r = read_with(&root(&d), &rel("tr.sql"), "windows-1254").unwrap();
    let mut edited = SaveText::from(&r);
    edited.text.push_str("-- Ş\r\n");
    save(&root(&d), &rel("tr.sql"), edited, Some(r.stamp)).unwrap();
    assert_eq!(
        fs::read(d.path().join("tr.sql")).unwrap(),
        cp1254("SELECT 'İĞŞ ğüşıöçİ';\r\n-- Ş\r\n")
    );
}

#[test]
fn read_with_refuses_an_unknown_encoding_and_a_lossy_one() {
    let d = lib_with_bytes("a.sql", b"SELECT 1;");
    assert_eq!(
        read_with(&root(&d), &rel("a.sql"), "no-such-encoding").unwrap_err(),
        ReadError::UnknownEncoding
    );
    // `replacement` would decode every byte to U+FFFD.
    assert_eq!(
        read_with(&root(&d), &rel("a.sql"), "csiso2022kr").unwrap_err(),
        ReadError::UnknownEncoding
    );
    // Shift_JIS 0x87 0x90 (NEC row 13) decodes to U+2252, which encodes back as 0x81 0xE0: a
    // save of the unchanged text would change the file, so the read refuses it.
    let d = lib_with_bytes("sj.sql", &[b'a', 0x87, 0x90, b'b']);
    assert_eq!(
        read_with(&root(&d), &rel("sj.sql"), "Shift_JIS").unwrap_err(),
        ReadError::Lossy
    );
    // Bytes the encoding cannot decode at all.
    let d = lib_with_bytes("bad.sql", &[0xFF, 0xFF, 0xFF]);
    assert_eq!(
        read_with(&root(&d), &rel("bad.sql"), "UTF-8").unwrap_err(),
        ReadError::Undecodable
    );
}

#[test]
fn unencodable_character_refuses_the_save() {
    let bytes = cp1254("SELECT 'İ';\n");
    let d = lib_with_bytes("tr.sql", &bytes);
    let r = read_with(&root(&d), &rel("tr.sql"), "windows-1254").unwrap();
    let mut edited = SaveText::from(&r);
    edited.text = "SELECT 'İ';\n-- 漢\n".into();
    assert_eq!(
        save(&root(&d), &rel("tr.sql"), edited, Some(r.stamp)),
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
        read(&root(&d), &rel("a.sql"), ReadLimits::default()).unwrap_err(),
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
                read(&root(&d), &rel("a.sql"), ReadLimits::default()),
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
    match save(&root(&d), &rel("f.sql"), text("c"), Some(r.stamp)) {
        Err(SaveError::ChangedOnDisk(now)) => {
            assert_eq!(now.len, 1);
            assert_eq!(now.hash, xxh3(b"b"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(fs::read(d.path().join("f.sql")).unwrap(), b"b");
    // Without `expect` (the user chose Overwrite), the save goes through.
    let stamp = save(&root(&d), &rel("f.sql"), text("c"), None).unwrap();
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
        save(&root(&d), &rel("f.sql"), text("x"), Some(forged)),
        Err(SaveError::ChangedOnDisk(now)) if now == r.stamp
    ));
    assert_eq!(fs::read(d.path().join("f.sql")).unwrap(), b"SELECT 1;");
    // The stamp the save returns is the one a read sees next.
    let saved = save(&root(&d), &rel("f.sql"), text("SELECT 2;"), Some(r.stamp)).unwrap();
    assert_eq!(read_default(&d, "f.sql").stamp, saved);
}

#[test]
fn save_creates_a_new_file_and_refuses_one_deleted_since_open() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("sub")).unwrap();
    let s = save(&root(&d), &rel("sub/new.sql"), text("SELECT 1;\n"), None).unwrap();
    assert_eq!(
        fs::read(d.path().join("sub/new.sql")).unwrap(),
        b"SELECT 1;\n"
    );
    assert_eq!(s.len, 10);
    let r = read_default(&d, "sub/new.sql");
    fs::remove_file(d.path().join("sub/new.sql")).unwrap();
    assert_eq!(
        save(&root(&d), &rel("sub/new.sql"), text("x"), Some(r.stamp)),
        Err(SaveError::DeletedOnDisk)
    );
    assert!(!d.path().join("sub/new.sql").exists());
    // "Save to recreate": no expectation.
    save(&root(&d), &rel("sub/new.sql"), text("x"), None).unwrap();
    assert_eq!(fs::read(d.path().join("sub/new.sql")).unwrap(), b"x");
    // A missing folder is not created.
    assert!(matches!(
        save(&root(&d), &rel("nope/a.sql"), text("x"), None),
        Err(SaveError::Io(std::io::ErrorKind::NotFound))
    ));
    assert!(!d.path().join("nope").exists());
}

#[test]
fn save_refuses_a_folder_and_hidden_paths() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("x.sql")).unwrap();
    fs::create_dir(d.path().join(".hacivat")).unwrap();
    let before = names_under(d.path());
    assert_eq!(
        save(&root(&d), &rel("x.sql"), text("x"), None),
        Err(SaveError::PathChanged)
    );
    // Never listed, so never written (C11): the manifest folder, a dot file.
    for hidden in [".hacivat/workspace.toml", ".a.sql", "sub/.git/config"] {
        assert_eq!(
            save(&root(&d), &rel(hidden), text("x"), None),
            Err(SaveError::PathRefused),
            "{hidden}"
        );
        assert_eq!(
            read(&root(&d), &rel(hidden), ReadLimits::default()).unwrap_err(),
            ReadError::PathRefused,
            "{hidden}"
        );
    }
    assert_eq!(names_under(d.path()), before);
    assert_eq!(
        read(&root(&d), &rel("x.sql"), ReadLimits::default()).unwrap_err(),
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
        save(&root(&d), &rel("f.sql"), text("SELECT 2;"), Some(r.stamp)).unwrap();
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
    save(&root(&d), &rel(&long), text("SELECT 2;"), Some(r.stamp)).unwrap();
    assert_eq!(names_under(d.path()), std::slice::from_ref(&long));
    // The temp file a save really writes has that shape.
    faults::inject(faults::Fault::Rename);
    let _ = save(&root(&d), &rel(&long), text("x"), None);
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
    let r = read(&root(&d), &rel("u16.sql"), ReadLimits { whole: true }).unwrap();
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
    let mut s = open_stream(&root(&d), &rel("big.sql"), whole.meta.encoding).unwrap();
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
    let mut s = open_stream(&root(&d), &rel("u8.sql"), r.meta.encoding).unwrap();
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
            text("SELECT 2;"),
            Some(r.stamp),
        )
        .unwrap();
        assert_eq!(mode(&d.path().join("real.sql")), 0o755);
        // An InsideLink 'l.sql' -> 'real.sql': the save writes real.sql, l.sql stays a link.
        symlink("real.sql", d.path().join("l.sql")).unwrap();
        let r = read_default(&d, "l.sql");
        assert_eq!(r.text, "SELECT 2;");
        save(&root(&d), &rel("l.sql"), text("SELECT 3;"), Some(r.stamp)).unwrap();
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
            text("SELECT 4;"),
            Some(r.stamp),
        )
        .unwrap();
        assert_eq!(fs::read(d.path().join("real.sql")).unwrap(), b"SELECT 4;");
        // A new file takes the umask's mode, never the 0600 of the temp.
        save(&root(&d), &rel("new.sql"), text("x"), None).unwrap();
        assert_ne!(mode(&d.path().join("new.sql")), 0o600);
    }

    #[test]
    fn save_keeps_hard_link() {
        let d = lib_with_bytes("a.sql", b"SELECT 1;");
        let other = tempfile::tempdir().unwrap();
        fs::hard_link(d.path().join("a.sql"), d.path().join("b.sql")).unwrap();
        let r = read_default(&d, "a.sql");
        let ino = fs::metadata(d.path().join("a.sql")).unwrap().ino();
        save(&root(&d), &rel("a.sql"), text("SELECT 22;"), Some(r.stamp)).unwrap();
        // Written in place: both names still show the same inode and the new bytes.
        assert_eq!(fs::metadata(d.path().join("a.sql")).unwrap().ino(), ino);
        assert_eq!(fs::read(d.path().join("b.sql")).unwrap(), b"SELECT 22;");
        assert_eq!(fs::metadata(d.path().join("a.sql")).unwrap().nlink(), 2);
        // The stamp check runs first there too.
        fs::write(d.path().join("b.sql"), b"theirs").unwrap();
        assert!(matches!(
            save(&root(&d), &rel("a.sql"), text("mine"), Some(r.stamp)),
            Err(SaveError::ChangedOnDisk(_))
        ));
        assert_eq!(fs::read(d.path().join("a.sql")).unwrap(), b"theirs");
        // A shorter text is not padded with the old tail.
        let r = read_default(&d, "a.sql");
        save(&root(&d), &rel("a.sql"), text("x"), Some(r.stamp)).unwrap();
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
            save(&root(&d), &rel("f.sql"), text("pwned"), Some(r.stamp)),
            Err(SaveError::PathChanged)
        );
        assert_eq!(
            save(&root(&d), &rel("f.sql"), text("pwned"), None),
            Err(SaveError::PathChanged)
        );
        assert_eq!(
            fs::read(outside.path().join("target.sql")).unwrap(),
            b"outside"
        );
        assert_eq!(
            read(&root(&d), &rel("f.sql"), ReadLimits::default()).unwrap_err(),
            ReadError::PathRefused
        );
        assert!(
            open_stream(
                &root(&d),
                &rel("f.sql"),
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
            save(&root(&d), &rel("sub/a.sql"), text("pwned"), Some(r.stamp)),
            Err(SaveError::PathChanged)
        );
        assert_eq!(
            save(&root(&d), &rel("sub/new.sql"), text("pwned"), None),
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
                save(&root(&d), &rel(name), text("x"), None),
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
                read(&root(&d), &rel(name), ReadLimits::default()).unwrap_err(),
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
            read(&root(&d), &rel("p.sql"), ReadLimits::default()).unwrap_err(),
            ReadError::PathRefused
        );
        assert_eq!(
            save(&root(&d), &rel("p.sql"), text("x"), None),
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
        let got = save(&root(&d), &rel("sub/f.sql"), text("x"), Some(r.stamp));
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
            save(&root(&d), &rel("f.sql"), text("x"), Some(r.stamp)),
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
            read(&r, &rel("a.sql"), ReadLimits::default()).unwrap_err(),
            ReadError::PathRefused
        );
        assert_eq!(
            save(&r, &rel("a.sql"), text("pwned"), None),
            Err(SaveError::PathChanged)
        );
        assert_eq!(fs::read(outside.path().join("a.sql")).unwrap(), b"outside");
    }

    #[test]
    fn nfd_name_is_kept_on_save() {
        let nfd = "mu\u{0308}s.sql";
        let d = lib_with_bytes(nfd, b"SELECT 1;");
        let r = read_default(&d, nfd);
        save(&root(&d), &rel(nfd), text("SELECT 2;"), Some(r.stamp)).unwrap();
        let names: Vec<String> = fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        // macOS APFS keeps the bytes as given; the save reused them.
        assert_eq!(names.len(), 1);
        assert_eq!(fs::read(d.path().join(nfd)).unwrap(), b"SELECT 2;");
    }

    #[test]
    fn hard_link_save_keeps_owner_mode() {
        let d = lib_with_bytes("a.sql", b"x");
        fs::hard_link(d.path().join("a.sql"), d.path().join("b.sql")).unwrap();
        fs::set_permissions(d.path().join("a.sql"), fs::Permissions::from_mode(0o640)).unwrap();
        let r = read_default(&d, "a.sql");
        save(&root(&d), &rel("a.sql"), text("y"), Some(r.stamp)).unwrap();
        assert_eq!(mode(&d.path().join("b.sql")), 0o640);
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
            save(&root(&d), &rel("sub/a.sql"), text("pwned"), None),
            Err(SaveError::PathChanged)
        );
        assert_eq!(
            read(&root(&d), &rel("sub/a.sql"), ReadLimits::default()).unwrap_err(),
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
                save(&root(&d), &rel(bad), text("x"), None),
                Err(SaveError::PathRefused),
                "{bad:?}"
            );
        }
        assert!(names_under(d.path()).is_empty());
    }
}
