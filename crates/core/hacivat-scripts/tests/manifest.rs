//! M4b Task 3: `.hacivat/workspace.toml`, untrusted input with a fixed schema, and its typed
//! writer (HV-SCRIPT-01 steps 3–4; R-M4b-8, -21, -42; carries-w1 "T3").

use std::fs;
use std::path::Path;

use hacivat_scripts::LogicalDb;
use hacivat_scripts::manifest::{
    self, Diag, Field, MANIFEST_MAX_BYTES, ManifestRead, ManifestV1, WriteError, add_alias, read,
    read_bytes, write_new,
};
use hacivat_scripts::names::NameError;

fn db(name: &str) -> LogicalDb {
    LogicalDb::parse(name).unwrap()
}

fn m(database: &str, aliases: &[&str]) -> ManifestV1 {
    ManifestV1::new(db(database), aliases.iter().map(|a| db(a)).collect())
}

/// An empty library folder.
fn lib_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

/// A library folder whose manifest holds `text`.
fn lib_with(text: &str) -> tempfile::TempDir {
    let d = lib_dir();
    fs::create_dir(d.path().join(".hacivat")).unwrap();
    fs::write(d.path().join(".hacivat/workspace.toml"), text).unwrap();
    d
}

/// The manifest text as read from a folder named `lib`.
fn read_text(text: &str) -> ManifestRead {
    read_bytes(text.as_bytes(), "lib")
}

/// The manifest `text` read from disk in a library folder named `folder`.
fn read_in(folder: &str, text: &str) -> ManifestRead {
    let parent = tempfile::tempdir().unwrap();
    let dir = parent.path().join(folder);
    fs::create_dir_all(dir.join(".hacivat")).unwrap();
    fs::write(dir.join(".hacivat/workspace.toml"), text).unwrap();
    let r = read(&dir);
    // The folder-name fallback the binding uses when the manifest names no database.
    if r.db.is_none() {
        assert_eq!(r.logical(folder).unwrap().as_str(), folder);
    }
    r
}

fn manifest_path(root: &Path) -> std::path::PathBuf {
    root.join(".hacivat/workspace.toml")
}

/// Every name under `dir` (the "nothing else was written" check).
fn names_under(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

#[test]
fn manifest_v1_round_trips() {
    let d = lib_dir();
    write_new(d.path(), &m("orders", &["orders_db"])).unwrap();
    let r = read(d.path());
    assert_eq!(r.db.unwrap().as_str(), "orders");
    assert_eq!(r.aliases.len(), 1);
    assert!(r.diags.is_empty());
}

#[test]
fn manifest_type_has_only_three_fields() {
    let t = toml::to_string(&m("orders", &[])).unwrap();
    let keys: Vec<_> = t
        .lines()
        .filter_map(|l| l.split('=').next())
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .collect();
    assert_eq!(keys, ["format", "database", "aliases"]);
}

/// alg/28 HV-SCRIPT-01 `writer_type_has_exactly_three_fields`: what the writer puts on disk,
/// read back as a table, has exactly the three keys, and is the text the attach dialog shows.
#[test]
fn writer_type_has_exactly_three_fields() {
    let d = lib_dir();
    let man = m("orders", &["orders_p", "orders_s"]);
    write_new(d.path(), &man).unwrap();
    let text = fs::read_to_string(manifest_path(d.path())).unwrap();
    assert_eq!(text, man.to_toml());
    assert_eq!(
        text,
        "format = 1\ndatabase = \"orders\"\naliases = [\"orders_p\", \"orders_s\"]\n"
    );
    let table: toml::Table = text.parse().unwrap();
    let mut keys: Vec<_> = table.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["aliases", "database", "format"]);
}

#[test]
fn write_new_refuses_an_existing_file() {
    let d = lib_with("# a teammate's file\nformat = 1\ndatabase = \"orders\"\n");
    let before = fs::read(manifest_path(d.path())).unwrap();
    assert_eq!(
        write_new(d.path(), &m("other", &[])),
        Err(WriteError::Exists)
    );
    assert_eq!(fs::read(manifest_path(d.path())).unwrap(), before);
    // No temp file is left behind either.
    assert_eq!(names_under(&d.path().join(".hacivat")), ["workspace.toml"]);
}

/// alg/28 `attach_writes_manifest_only_when_absent`.
#[test]
fn attach_writes_manifest_only_when_absent() {
    let d = lib_dir();
    write_new(d.path(), &m("orders", &[])).unwrap();
    let first = fs::read(manifest_path(d.path())).unwrap();
    assert_eq!(first, b"format = 1\ndatabase = \"orders\"\naliases = []\n");
    assert_eq!(
        write_new(d.path(), &m("orders", &["x"])),
        Err(WriteError::Exists)
    );
    assert_eq!(fs::read(manifest_path(d.path())).unwrap(), first);
    assert_eq!(names_under(&d.path().join(".hacivat")), ["workspace.toml"]);
    assert_eq!(names_under(d.path()), [".hacivat"]);
}

#[test]
fn write_new_refuses_a_format_it_does_not_write() {
    let d = lib_dir();
    let mut man = m("orders", &[]);
    man.format = 2;
    assert_eq!(write_new(d.path(), &man), Err(WriteError::Format));
    assert!(!d.path().join(".hacivat").exists());
}

#[test]
fn unknown_keys_are_ignored_with_a_diagnostic() {
    let r = read_text("format = 1\ndatabase = \"orders\"\nhost = \"db.prod\"\npassword = \"x\"\n");
    assert_eq!(r.db.unwrap().as_str(), "orders");
    assert_eq!(
        r.diags
            .iter()
            .filter(|d| matches!(d, Diag::UnknownKey(_)))
            .count(),
        2
    );
    // The diagnostic names the key, never its value.
    let shown = format!("{:?}", r.diags);
    assert!(
        shown.contains("host") && shown.contains("password"),
        "{shown}"
    );
    assert!(!shown.contains("db.prod"), "{shown}");
}

#[test]
fn newer_format_binds_by_folder_name() {
    let r = read_in("orders_lib", "format = 2\ndatabase = \"x\"\n");
    assert!(r.db.is_none());
    assert!(r.diags.iter().any(|d| matches!(d, Diag::NewerFormat(2))));
}

#[test]
fn oversized_manifest_is_not_parsed() {
    let r = read_text(&"#".repeat(65 * 1024));
    assert!(r.diags.iter().any(|d| matches!(d, Diag::TooLarge)));
}

#[test]
fn oversized_manifest_on_disk_is_not_read() {
    let mut text = String::from("format = 1\ndatabase = \"orders\"\n");
    text.push_str(&"#".repeat(MANIFEST_MAX_BYTES));
    let r = read(lib_with(&text).path());
    assert!(r.db.is_none());
    assert_eq!(r.diags, [Diag::TooLarge]);
    // Exactly at the limit is read.
    let mut text = String::from("format = 1\ndatabase = \"orders\"\n#");
    text.push_str(&"#".repeat(MANIFEST_MAX_BYTES - text.len()));
    assert_eq!(text.len(), MANIFEST_MAX_BYTES);
    assert_eq!(read(lib_with(&text).path()).db.unwrap().as_str(), "orders");
}

#[test]
fn control_characters_in_names_are_refused() {
    for n in ["ord\u{0}ers", "ord\u{202E}ers", "a\nb", ""] {
        assert!(LogicalDb::parse(n).is_err(), "{n:?}");
    }
    // C1 controls, DEL and every bidi control too.
    for n in [
        "a\u{7F}",
        "a\u{85}",
        "a\u{9F}",
        "a\u{61C}",
        "a\u{200E}",
        "a\u{200F}",
        "a\u{202A}",
        "a\u{2066}",
        "a\u{2069}",
    ] {
        assert!(LogicalDb::parse(n).is_err(), "{n:?}");
    }
    assert_eq!(LogicalDb::parse("").unwrap_err(), NameError::Empty);
    assert_eq!(LogicalDb::parse("a\tb").unwrap_err(), NameError::Control);
    assert_eq!(LogicalDb::parse("a\u{202E}b").unwrap_err(), NameError::Bidi);
    // Ordinary names, Turkish letters and spaces pass.
    for n in ["orders", "müşteri", "İSTANBUL", "orders prod", "a-b_c.d"] {
        assert_eq!(LogicalDb::parse(n).unwrap().as_str(), n);
    }
}

#[test]
fn name_over_128_bytes_is_refused() {
    assert!(LogicalDb::parse(&"a".repeat(128)).is_ok());
    assert_eq!(
        LogicalDb::parse(&"a".repeat(129)).unwrap_err(),
        NameError::TooLong
    );
    // Bytes, not characters: 64 × `ü` is 128 bytes, 65 is 130.
    assert!(LogicalDb::parse(&"ü".repeat(64)).is_ok());
    assert_eq!(
        LogicalDb::parse(&"ü".repeat(65)).unwrap_err(),
        NameError::TooLong
    );
    // The limit applies to the stored NFC text: 64 × NFD `u` + U+0308 (192 bytes) is 128 NFC.
    let nfd = "u\u{0308}".repeat(64);
    assert_eq!(nfd.len(), 192);
    assert_eq!(LogicalDb::parse(&nfd).unwrap().as_str(), "ü".repeat(64));
    let r = read_text(&format!("format = 1\ndatabase = \"{}\"\n", "a".repeat(129)));
    assert!(r.db.is_none());
    assert_eq!(
        r.diags,
        [Diag::BadName(Field::Database, NameError::TooLong)]
    );
}

#[test]
fn database_with_path_text_is_just_a_name() {
    let r = read_text("format = 1\ndatabase = \"../../etc\"\n");
    assert_eq!(r.db.unwrap().as_str(), "../../etc");
} // a name, never joined to a path

#[test]
fn add_alias_keeps_comments() {
    let d = lib_with("# team note\nformat = 1\ndatabase = \"orders\"\n");
    add_alias(d.path(), &db("orders_p")).unwrap();
    let t = fs::read_to_string(d.path().join(".hacivat/workspace.toml")).unwrap();
    assert!(t.starts_with("# team note\n"));
    assert!(t.contains("orders_p"));
}

/// alg/28 `alias_append_keeps_comments`: an existing array keeps its comments and layout, an
/// alias already there (after NFC, ASCII case-insensitive) is not added twice, and no temp file
/// is left.
#[test]
fn alias_append_keeps_comments() {
    let text = "# team note\nformat = 1 # the schema\ndatabase = \"orders\"\n\n# who uses which name\naliases = [\"orders_db\"] # keep\n";
    let d = lib_with(text);
    add_alias(d.path(), &db("orders_p")).unwrap();
    let t = fs::read_to_string(manifest_path(d.path())).unwrap();
    assert_eq!(
        t,
        "# team note\nformat = 1 # the schema\ndatabase = \"orders\"\n\n# who uses which name\naliases = [\"orders_db\", \"orders_p\"] # keep\n"
    );
    // Already there, in another case, or the database itself: nothing changes.
    for again in ["orders_p", "ORDERS_P", "Orders", "orders_db"] {
        add_alias(d.path(), &db(again)).unwrap();
        assert_eq!(fs::read_to_string(manifest_path(d.path())).unwrap(), t);
    }
    let r = read(d.path());
    assert!(r.diags.is_empty(), "{:?}", r.diags);
    assert_eq!(
        r.aliases.iter().map(LogicalDb::as_str).collect::<Vec<_>>(),
        ["orders_db", "orders_p"]
    );
    assert_eq!(names_under(&d.path().join(".hacivat")), ["workspace.toml"]);
}

#[test]
fn add_alias_refuses_what_it_cannot_read() {
    // No manifest: the user never saw one, so none is written behind their back (R-M4b-8).
    let d = lib_dir();
    assert_eq!(add_alias(d.path(), &db("x")), Err(WriteError::NoManifest));
    assert!(!d.path().join(".hacivat").exists());
    // A newer format or a broken file is never rewritten.
    for text in [
        "format = 2\ndatabase = \"orders\"\n",
        "format = 1\ndatabase = [\n",
        "database = \"orders\"\n",
    ] {
        let d = lib_with(text);
        assert!(add_alias(d.path(), &db("x")).is_err(), "{text:?}");
        assert_eq!(fs::read_to_string(manifest_path(d.path())).unwrap(), text);
    }
    // At most 32 aliases.
    let many: Vec<String> = (0..32).map(|i| format!("\"a{i}\"")).collect();
    let text = format!(
        "format = 1\ndatabase = \"orders\"\naliases = [{}]\n",
        many.join(", ")
    );
    let d = lib_with(&text);
    assert_eq!(
        add_alias(d.path(), &db("one_more")),
        Err(WriteError::TooManyAliases)
    );
    assert_eq!(fs::read_to_string(manifest_path(d.path())).unwrap(), text);
}

/// alg/28 `manifest_table`.
#[test]
fn manifest_table() {
    struct Row {
        text: &'static str,
        db: Option<&'static str>,
        diags: Vec<Diag>,
    }
    let rows = [
        Row {
            text: "format = 1\ndatabase = \"orders\"\n",
            db: Some("orders"),
            diags: vec![],
        },
        Row {
            text: "format = 2\ndatabase = \"orders\"\nrun = \"x\"\n",
            db: None,
            diags: vec![Diag::NewerFormat(2)],
        },
        Row {
            text: "format = 1\ndatabase = [\n",
            db: None,
            diags: vec![Diag::InvalidToml],
        },
        Row {
            text: "format = 1\ndatabase = \"ord\u{202E}ers\"\n",
            db: None,
            diags: vec![Diag::BadName(Field::Database, NameError::Bidi)],
        },
        Row {
            text: "format = 1\ndatabase = \"orders\"\nrun = \"x\"\n",
            db: Some("orders"),
            diags: vec![Diag::UnknownKey("run".into())],
        },
        Row {
            text: "format = 1\n",
            db: None,
            diags: vec![],
        },
        Row {
            text: "database = \"orders\"\n",
            db: None,
            diags: vec![Diag::MissingFormat],
        },
        Row {
            text: "format = \"1\"\ndatabase = \"orders\"\n",
            db: None,
            diags: vec![Diag::BadFormat],
        },
        Row {
            text: "format = 0\ndatabase = \"orders\"\n",
            db: None,
            diags: vec![Diag::BadFormat],
        },
        Row {
            text: "format = 1\ndatabase = 7\n",
            db: None,
            diags: vec![Diag::WrongType(Field::Database)],
        },
        Row {
            text: "format = 1\ndatabase = \"\"\n",
            db: None,
            diags: vec![Diag::BadName(Field::Database, NameError::Empty)],
        },
        Row {
            text: "\u{FEFF}format = 1\ndatabase = \"orders\"\n",
            db: Some("orders"),
            diags: vec![],
        },
        Row {
            text: "format = 1\ndatabase = \"orders\"\naliases = \"orders_p\"\n",
            db: Some("orders"),
            diags: vec![Diag::WrongType(Field::Aliases)],
        },
        Row {
            text: "format = 1\ndatabase = \"orders\"\naliases = [\"ok\", 3, \"a\\u0000b\"]\n",
            db: Some("orders"),
            diags: vec![
                Diag::WrongType(Field::Alias(1)),
                Diag::BadName(Field::Alias(2), NameError::Control),
            ],
        },
        // Nested tables are unknown keys too, never read.
        Row {
            text: "format = 1\ndatabase = \"orders\"\n[connection]\nhost = \"h\"\n",
            db: Some("orders"),
            diags: vec![Diag::UnknownKey("connection".into())],
        },
    ];
    for row in rows {
        let r = read_text(row.text);
        assert_eq!(
            r.db.as_ref().map(LogicalDb::as_str),
            row.db,
            "{:?}",
            row.text
        );
        assert_eq!(r.diags, row.diags, "{:?}", row.text);
    }
    assert_eq!(
        read_bytes(b"format = 1\ndatabase = \"\xFF\"\n", "lib").diags,
        [Diag::NotUtf8]
    );
}

#[test]
fn aliases_are_validated_and_deduplicated() {
    let r = read_text(
        "format = 1\ndatabase = \"orders\"\naliases = [\"orders_p\", \"ORDERS_P\", \"Orders\", \"caf\\u0065\\u0301\", \"café\"]\n",
    );
    assert!(r.diags.is_empty(), "{:?}", r.diags);
    assert_eq!(
        r.aliases.iter().map(LogicalDb::as_str).collect::<Vec<_>>(),
        ["orders_p", "café"]
    );
    // More than 32: the first 32 are kept, with a diagnostic.
    let many: Vec<String> = (0..40).map(|i| format!("\"a{i}\"")).collect();
    let r = read_text(&format!(
        "format = 1\ndatabase = \"orders\"\naliases = [{}]\n",
        many.join(", ")
    ));
    assert_eq!(r.aliases.len(), 32);
    assert_eq!(r.aliases[31].as_str(), "a31");
    assert_eq!(r.diags, [Diag::TooManyAliases]);
    // Without a `database`, an alias equal to the folder name is the folder name itself.
    let r = read_bytes(b"format = 1\naliases = [\"LIB\", \"other\"]\n", "lib");
    assert_eq!(
        r.aliases.iter().map(LogicalDb::as_str).collect::<Vec<_>>(),
        ["other"]
    );
}

#[test]
fn unknown_key_diagnostics_are_bounded_and_escaped() {
    let mut text = String::from("format = 1\ndatabase = \"orders\"\n");
    for i in 0..100 {
        text.push_str(&format!("k{i} = 1\n"));
    }
    text.push_str("\"\u{202E}evil\" = 1\n");
    text.push_str(&format!("{} = 1\n", "x".repeat(500)));
    let r = read_text(&text);
    let keys: Vec<&str> = r
        .diags
        .iter()
        .filter_map(|d| match d {
            Diag::UnknownKey(k) => Some(k.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(keys.len(), manifest::MAX_KEY_DIAGS);
    assert!(
        r.diags
            .contains(&Diag::MoreUnknownKeys(102 - manifest::MAX_KEY_DIAGS))
    );
    // No diagnostic carries a bidi control or a key longer than the cap.
    for d in &r.diags {
        let shown = format!("{d:?}");
        assert!(!shown.contains('\u{202E}'), "{shown}");
    }
    let r = read_text("format = 1\n\"\u{202E}evil\" = 1\n");
    assert_eq!(r.diags, [Diag::UnknownKey("<U+202E>evil".into())]);
    let r = read_text(&format!("format = 1\n{} = 1\n", "x".repeat(500)));
    assert_eq!(r.diags, [Diag::UnknownKey(format!("{}…", "x".repeat(64)))]);
}

#[test]
fn missing_manifest_reads_as_folder_name() {
    let d = lib_dir();
    let r = read(d.path());
    assert!(r.db.is_none() && r.aliases.is_empty() && r.diags.is_empty());
    assert_eq!(r.logical("orders").unwrap().as_str(), "orders");
    // A folder name that is not a valid logical name binds nothing.
    assert!(r.logical("a\u{202E}b").is_none());
    // `.hacivat` without the file.
    fs::create_dir(d.path().join(".hacivat")).unwrap();
    let r = read(d.path());
    assert!(r.db.is_none() && r.diags.is_empty());
}

#[test]
fn manifest_that_is_a_folder_or_a_file_hacivat_is_ignored() {
    let d = lib_dir();
    fs::create_dir_all(d.path().join(".hacivat/workspace.toml")).unwrap();
    assert_eq!(read(d.path()).diags, [Diag::NotAFile]);
    let d = lib_dir();
    fs::write(d.path().join(".hacivat"), "format = 1\n").unwrap();
    assert_eq!(read(d.path()).diags, [Diag::NotAFolder]);
}

#[test]
fn manifest_write_refuses_a_hacivat_file() {
    let d = lib_dir();
    fs::write(d.path().join(".hacivat"), "not a folder").unwrap();
    assert_eq!(
        write_new(d.path(), &m("orders", &[])),
        Err(WriteError::NotAFolder)
    );
    assert_eq!(
        fs::read_to_string(d.path().join(".hacivat")).unwrap(),
        "not a folder"
    );
}

/// `read_text` on a 256 KiB thread: a nesting-guard regression aborts the test process loudly
/// (a stack overflow) instead of passing on a large stack.
fn read_text_small_stack(text: String) -> ManifestRead {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || read_text(&text))
        .unwrap()
        .join()
        .unwrap()
}

/// A hostile manifest that nests arrays or inline tables up to the size cap is refused, never
/// a stack overflow (the parser runs on a small core-runtime thread). The guard's lexer follows
/// the pinned `toml_parser` lexer, so no string, comment or atom form hides brackets from it
/// that the parser then sees (fix round 1, m1).
#[test]
fn deeply_nested_manifest_is_refused_without_overflow() {
    let room = MANIFEST_MAX_BYTES - 64;
    let open = |n: usize| "[".repeat(n);
    let close = |n: usize| "]".repeat(n);
    let hostile = [
        format!("format = 1\nx = {}", open(room)),
        format!("format = 1\nx = {}", "{a=".repeat(room / 3)),
        format!(
            "format = 1\nx = {}{}\n",
            open(room / 2 - 8),
            close(room / 2 - 8)
        ),
        // A one-line basic string ends at the newline, even after a backslash (review m1).
        format!("format = 1\na = \"\\\nb = {}", open(room)),
        format!("format = 1\na = \"x\\\\\\\nb = {}", open(room)),
        // A one-line literal string ends at the newline.
        format!("format = 1\na = 'x\nb = {}", open(room)),
        // A comment ends at a carriage return too.
        format!("format = 1 # note\rb = {}", open(room)),
        // A quote inside an atom starts no string.
        format!("format = 1\na = x\"y\nb = {}", open(room)),
        format!("format = 1\na = x\"{}", open(room)),
        format!("format = 1\na = x'{}", open(room)),
        // An unclosed multi-line string runs to the end; the brackets before it count.
        format!("format = 1\nb = {}\"\"\"", open(room)),
    ];
    for text in hostile {
        let shown = text[..40].escape_debug().to_string();
        let r = read_text_small_stack(text);
        assert!(r.db.is_none(), "{shown}");
        assert_eq!(r.diags, [Diag::TooDeep], "{shown}");
    }
    // The same forms just past the limit are refused by the guard, not by the parser.
    for text in [
        format!("format = 1\na = \"\\\nb = {}{}\n", open(17), close(17)),
        format!("format = 1 # note\rb = {}{}\n", open(17), close(17)),
        format!("format = 1\na = x\"y\nb = {}{}\n", open(17), close(17)),
        format!("format = 1\nw = {}{}\n", open(17), close(17)),
    ] {
        assert_eq!(read_text_small_stack(text).diags, [Diag::TooDeep]);
    }
    // Brackets in strings (every form) and comments do not count; 16 levels still parse.
    let deep = open(40);
    let text = format!(
        "format = 1 # {deep}\ndatabase = \"{}\"\nx = '{deep}'\ny = \"\"\"\n{deep}\\\"\"\"\"\nz = '''{deep}'''\nv = \"\"\"a\\\n  {deep}\"\"\"\nw = {}{}\n",
        open(20),
        open(16),
        close(16)
    );
    let r = read_text_small_stack(text);
    assert_eq!(r.db.unwrap().as_str(), open(20));
    assert!(!r.diags.contains(&Diag::TooDeep), "{:?}", r.diags);
    // Re-review n1: a second BOM (the reader strips one, the lexer one more) is skipped like
    // `toml_parser` skips it, so the first token is still a string: its brackets do not count,
    // and a deep file behind two BOMs is still refused.
    let two_boms = format!(
        "\u{FEFF}\u{FEFF}\"{}\" = 1\nformat = 1\ndatabase = \"orders\"\n",
        open(20)
    );
    let r = read_text_small_stack(two_boms);
    assert_eq!(r.db.unwrap().as_str(), "orders");
    assert!(!r.diags.contains(&Diag::TooDeep), "{:?}", r.diags);
    let deep_two_boms = format!("\u{FEFF}\u{FEFF}format = 1\nx = {}", open(room));
    assert_eq!(read_text_small_stack(deep_two_boms).diags, [Diag::TooDeep]);
}

/// carries-w1 T3: when the volume has no hard links (exFAT, FAT, SMB), the fallback still never
/// replaces an existing file.
#[test]
fn write_new_falls_back_without_hard_links_and_never_replaces() {
    let d = lib_dir();
    manifest::write_new_without_hard_links(d.path(), &m("orders", &["orders_db"])).unwrap();
    let r = read(d.path());
    assert_eq!(r.db.unwrap().as_str(), "orders");
    assert!(r.diags.is_empty());
    assert_eq!(names_under(&d.path().join(".hacivat")), ["workspace.toml"]);
    let before = fs::read(manifest_path(d.path())).unwrap();
    assert_eq!(
        manifest::write_new_without_hard_links(d.path(), &m("other", &[])),
        Err(WriteError::Exists)
    );
    assert_eq!(fs::read(manifest_path(d.path())).unwrap(), before);
    assert_eq!(names_under(&d.path().join(".hacivat")), ["workspace.toml"]);
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    /// alg/28 `manifest_symlink_is_ignored`: a link at `.hacivat` or at the file is never read.
    #[test]
    fn manifest_symlink_is_ignored() {
        let outside = lib_with("format = 1\ndatabase = \"secret_db\"\n");
        let d = lib_dir();
        symlink(outside.path().join(".hacivat"), d.path().join(".hacivat")).unwrap();
        let r = read(d.path());
        assert!(r.db.is_none());
        assert_eq!(r.diags, [Diag::LinkIgnored]);

        let d = lib_dir();
        fs::create_dir(d.path().join(".hacivat")).unwrap();
        symlink(
            manifest_path(outside.path()),
            d.path().join(".hacivat/workspace.toml"),
        )
        .unwrap();
        let r = read(d.path());
        assert!(r.db.is_none());
        assert_eq!(r.diags, [Diag::LinkIgnored]);
    }

    /// alg/28 `manifest_write_refuses_symlinked_hacivat_dir` (I4/n5): `<outside dir>` stays
    /// empty.
    #[test]
    fn manifest_write_refuses_symlinked_hacivat_dir() {
        let outside = lib_dir();
        let d = lib_dir();
        symlink(outside.path(), d.path().join(".hacivat")).unwrap();
        assert_eq!(
            write_new(d.path(), &m("orders", &[])),
            Err(WriteError::NotAFolder)
        );
        assert!(names_under(outside.path()).is_empty());
        assert_eq!(add_alias(d.path(), &db("x")), Err(WriteError::NotAFolder));
        assert!(names_under(outside.path()).is_empty());
        // A dangling link is refused too, and nothing is created where it points.
        let d = lib_dir();
        let target = outside.path().join("made");
        symlink(&target, d.path().join(".hacivat")).unwrap();
        assert_eq!(
            write_new(d.path(), &m("orders", &[])),
            Err(WriteError::NotAFolder)
        );
        assert!(!target.exists());
    }

    /// A link at `workspace.toml` (dangling or not) is never written through.
    #[test]
    fn manifest_write_never_follows_a_linked_file() {
        let outside = lib_dir();
        let target = outside.path().join("victim.toml");
        let d = lib_dir();
        fs::create_dir(d.path().join(".hacivat")).unwrap();
        symlink(&target, manifest_path(d.path())).unwrap();
        assert_eq!(
            write_new(d.path(), &m("orders", &[])),
            Err(WriteError::Exists)
        );
        assert!(!target.exists());
        fs::write(&target, "mine").unwrap();
        assert!(add_alias(d.path(), &db("x")).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "mine");
    }

    /// A hard link to a file outside the library: the alias edit replaces the library's entry
    /// and never writes into the other file.
    #[test]
    fn add_alias_never_writes_through_a_hard_link() {
        let outside = lib_dir();
        let other = outside.path().join("other.toml");
        let text = "format = 1\ndatabase = \"orders\"\n";
        fs::write(&other, text).unwrap();
        let d = lib_dir();
        fs::create_dir(d.path().join(".hacivat")).unwrap();
        fs::hard_link(&other, manifest_path(d.path())).unwrap();
        add_alias(d.path(), &db("orders_p")).unwrap();
        assert_eq!(fs::read_to_string(&other).unwrap(), text);
        assert!(
            fs::read_to_string(manifest_path(d.path()))
                .unwrap()
                .contains("orders_p")
        );
    }

    /// A FIFO named `workspace.toml` is never opened for reading (it would block).
    #[test]
    fn manifest_fifo_is_ignored() {
        let d = lib_dir();
        fs::create_dir(d.path().join(".hacivat")).unwrap();
        let fifo = manifest_path(d.path());
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        assert_eq!(read(d.path()).diags, [Diag::NotAFile]);
    }

    /// R-M4b-42: `workspace.toml` and `.hacivat/` get the mode of a reference file and folder
    /// made beside them with `std::fs` (the process umask), never 0o600/0o700; `libraries.toml`
    /// is 0o600.
    #[test]
    fn manifest_is_world_readable_and_registry_is_private() {
        let d = lib_dir();
        write_new(d.path(), &m("orders", &[])).unwrap();
        fs::write(d.path().join("reference.txt"), "x").unwrap();
        fs::create_dir(d.path().join("reference_dir")).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let file_mode = mode(&manifest_path(d.path()));
        let dir_mode = mode(&d.path().join(".hacivat"));
        assert_eq!(file_mode, mode(&d.path().join("reference.txt")));
        assert_eq!(dir_mode, mode(&d.path().join("reference_dir")));
        assert_ne!(file_mode, 0o600);
        assert_ne!(dir_mode, 0o700);
        // The alias edit keeps the file's mode.
        add_alias(d.path(), &db("orders_p")).unwrap();
        assert_eq!(mode(&manifest_path(d.path())), file_mode);

        let app = tempfile::tempdir().unwrap();
        let paths = hacivat_settings::LocalPaths::under(app.path());
        let (mut reg, diags) =
            hacivat_scripts::registry::LibraryRegistry::load(&paths.libraries_toml());
        assert!(diags.is_empty());
        let guard = hacivat_scripts::registry::AttachGuard::new(&paths, None);
        reg.add(d.path(), &guard, std::time::SystemTime::now())
            .unwrap();
        reg.save().unwrap();
        assert_eq!(mode(&paths.libraries_toml()), 0o600);
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

    /// carries-w1 T3: the symlinked-`.hacivat` case for a Windows junction.
    #[test]
    fn manifest_write_refuses_junctioned_hacivat_dir() {
        let outside = lib_dir();
        let d = lib_dir();
        junction(&d.path().join(".hacivat"), outside.path());
        assert_eq!(
            write_new(d.path(), &m("orders", &[])),
            Err(WriteError::NotAFolder)
        );
        assert!(names_under(outside.path()).is_empty());
        assert_eq!(add_alias(d.path(), &db("x")), Err(WriteError::NotAFolder));
        assert!(names_under(outside.path()).is_empty());
    }

    #[test]
    fn manifest_behind_a_junction_is_ignored() {
        let outside = lib_with("format = 1\ndatabase = \"secret_db\"\n");
        let d = lib_dir();
        junction(&d.path().join(".hacivat"), &outside.path().join(".hacivat"));
        let r = read(d.path());
        assert!(r.db.is_none());
        assert_eq!(r.diags, [Diag::LinkIgnored]);
    }
}

// ## m4b-t03-registry fix round 1

/// Review I1 (R-M4b-8): the alias edit keeps the file's line endings (CRLF stays CRLF; a mixed
/// file takes its dominant style, ties LF) and its BOM.
#[test]
fn add_alias_keeps_line_endings_and_bom() {
    let bom = "\u{FEFF}";
    let rows = [
        (
            "format = 1\r\ndatabase = \"orders\"\r\n".to_string(),
            "format = 1\r\ndatabase = \"orders\"\r\naliases = [\"orders_p\"]\r\n".to_string(),
        ),
        (
            "# note\r\nformat = 1\r\ndatabase = \"orders\"\r\naliases = [\"a\"] # keep\r\n"
                .to_string(),
            "# note\r\nformat = 1\r\ndatabase = \"orders\"\r\naliases = [\"a\", \"orders_p\"] # keep\r\n"
                .to_string(),
        ),
        (
            format!("{bom}format = 1\r\ndatabase = \"orders\"\r\n"),
            format!("{bom}format = 1\r\ndatabase = \"orders\"\r\naliases = [\"orders_p\"]\r\n"),
        ),
        (
            format!("{bom}format = 1\ndatabase = \"orders\"\n"),
            format!("{bom}format = 1\ndatabase = \"orders\"\naliases = [\"orders_p\"]\n"),
        ),
        // Mixed: two CRLF lines, one LF line → CRLF.
        (
            "# a\r\nformat = 1\r\ndatabase = \"orders\"\n".to_string(),
            "# a\r\nformat = 1\r\ndatabase = \"orders\"\r\naliases = [\"orders_p\"]\r\n".to_string(),
        ),
        // Mixed: one CRLF line, two LF lines → LF.
        (
            "# a\r\nformat = 1\ndatabase = \"orders\"\n".to_string(),
            "# a\nformat = 1\ndatabase = \"orders\"\naliases = [\"orders_p\"]\n".to_string(),
        ),
    ];
    for (before, after) in rows {
        let d = lib_with(&before);
        add_alias(d.path(), &db("orders_p")).unwrap();
        let got = fs::read_to_string(manifest_path(d.path())).unwrap();
        assert_eq!(got, after, "{before:?}");
        let r = read(d.path());
        assert!(r.diags.is_empty(), "{:?}", r.diags);
        assert!(r.aliases.iter().any(|a| a.as_str() == "orders_p"));
    }
}

/// Review m2: where operations cannot be relative to the folder handle (Linux without procfs),
/// every write refuses (`Unsupported`) and nothing is created or changed; reading still works.
#[test]
fn writes_refuse_without_handle_relative_paths() {
    let unsupported = Err(WriteError::Io(std::io::ErrorKind::Unsupported));
    let d = lib_dir();
    let r = manifest::with_procfs_absent(|| write_new(d.path(), &m("orders", &[])));
    assert_eq!(r, unsupported);
    assert!(names_under(d.path()).is_empty());
    let text = "format = 1\ndatabase = \"orders\"\n";
    let d = lib_with(text);
    let r = manifest::with_procfs_absent(|| add_alias(d.path(), &db("x")));
    assert_eq!(r, unsupported);
    assert_eq!(fs::read_to_string(manifest_path(d.path())).unwrap(), text);
    assert_eq!(names_under(&d.path().join(".hacivat")), ["workspace.toml"]);
    let r = manifest::with_procfs_absent(|| read(d.path()));
    assert_eq!(r.db.unwrap().as_str(), "orders");
    // The flag is the closure's only: the same thread writes normally again.
    add_alias(d.path(), &db("x")).unwrap();
}

/// Review m4: a temp file left by a crash (`.workspace.toml.hv-<uuid>.tmp`, older than a
/// minute) is swept by the next write; a fresh one (another writer's) and other names stay.
#[test]
fn stale_temp_files_are_swept() {
    use std::time::{Duration, SystemTime};
    let age = |p: &Path, t: SystemTime| {
        fs::File::options()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(t)
            .unwrap();
    };
    let d = lib_dir();
    let h = d.path().join(".hacivat");
    fs::create_dir(&h).unwrap();
    let stale = h.join(".workspace.toml.hv-0123456789abcdef0123456789abcdef.tmp");
    let fresh = h.join(format!(".workspace.toml.hv-{}.tmp", "f".repeat(32)));
    let other = h.join(".workspace.toml.hv-notours.tmp");
    let note = h.join("notes.txt");
    for p in [&stale, &fresh, &other, &note] {
        fs::write(p, "x").unwrap();
    }
    let old = SystemTime::now() - Duration::from_secs(300);
    for p in [&stale, &other, &note] {
        age(p, old);
    }
    write_new(d.path(), &m("orders", &[])).unwrap();
    assert!(!stale.exists());
    assert!(fresh.exists() && other.exists() && note.exists());
    // add_alias sweeps too.
    fs::write(&stale, "x").unwrap();
    age(&stale, old);
    add_alias(d.path(), &db("orders_p")).unwrap();
    assert!(!stale.exists());
    assert!(fresh.exists());
}

/// Review m8: names may hold invisible format characters (per spec), so their display is
/// escaped.
#[test]
fn display_escaped_shows_invisible_characters() {
    for (name, shown) in [
        ("or\u{200B}ders", "or<U+200B>ders"),
        ("a\u{2028}b", "a<U+2028>b"),
        ("\u{FEFF}x", "<U+FEFF>x"),
        ("so\u{00AD}ft", "so<U+00AD>ft"),
        ("müşteri", "müşteri"),
    ] {
        assert_eq!(db(name).display_escaped(), shown);
    }
    assert_eq!(
        hacivat_scripts::names::escape_untrusted("a\u{202E}b", 64),
        "a<U+202E>b"
    );
}

/// Review m5: duplicate keys (plain and quoted), a NUL byte, and CRLF input.
#[test]
fn manifest_edge_rows() {
    for text in [
        "format = 1\ndatabase = \"a\"\ndatabase = \"b\"\n",
        "format = 1\ndatabase = \"a\"\n\"database\" = \"b\"\n",
        "format = 1\ndatabase = \"a\u{0}\"\n",
        "format = 1\n\u{0}\n",
    ] {
        let r = read_text(text);
        assert!(r.db.is_none(), "{text:?}");
        assert_eq!(r.diags, [Diag::InvalidToml], "{text:?}");
    }
    let r = read_text("format = 1\r\ndatabase = \"orders\"\r\naliases = [\"o\"]\r\n");
    assert_eq!(r.db.unwrap().as_str(), "orders");
    assert_eq!(r.aliases.len(), 1);
    assert!(r.diags.is_empty());
    let r = read_text("\u{FEFF}format = 1\r\ndatabase = \"orders\"\r\n");
    assert_eq!(r.db.unwrap().as_str(), "orders");
}
// ## m4b-t03-registry fix round 1
