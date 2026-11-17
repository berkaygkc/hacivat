//! Golden-file hygiene (the CRLF and byte-stability half of plan Task 31 / G5): every golden
//! is LF-only UTF-8 ending in exactly one newline, so a Windows checkout (the files are
//! `-text` in `.gitattributes`) compares byte for byte.

mod fixtures;

use fixtures::golden_path;

fn all_goldens() -> Vec<std::path::PathBuf> {
    let root = golden_path("x").parent().unwrap().to_path_buf();
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == "sql") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn golden_files_are_lf_only_and_end_with_one_newline() {
    let files = all_goldens();
    assert!(files.len() >= 30, "{} goldens", files.len());
    for f in files {
        let bytes = std::fs::read(&f).unwrap();
        let text =
            std::str::from_utf8(&bytes).unwrap_or_else(|_| panic!("{} is not UTF-8", f.display()));
        assert!(!text.contains('\r'), "{} has CR", f.display());
        assert!(
            text.ends_with(";\n"),
            "{} must end with one statement terminator and LF",
            f.display()
        );
        assert!(
            !text.ends_with("\n\n"),
            "{} ends with a blank line",
            f.display()
        );
        assert!(!text.contains('\t'), "{} has a tab", f.display());
    }
}

#[test]
fn gitattributes_keeps_goldens_byte_exact() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let attrs = std::fs::read_to_string(root.join(".gitattributes")).unwrap();
    assert!(
        attrs
            .lines()
            .any(|l| l.trim() == "crates/core/hacivat-ddl/tests/golden/** -text"),
        ".gitattributes must mark the DDL goldens -text"
    );
}
