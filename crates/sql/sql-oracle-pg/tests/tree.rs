//! The parse tree, scanner tokens and fingerprint for the PG classifier (M3b Task 9), and the
//! libpg_query hygiene rulings: the guard on every call, the decode-depth failure as an `Err`,
//! process stderr during parses (ledger 151, 164), and the unused `pg_query` functions that
//! share the null-slice hazard staying unreachable.

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use sql_oracle_pg::tree::{ScanKind, fingerprint, parse_tree, pb, scan_tokens};
use sql_oracle_pg::{MAX_ORACLE_NESTING, on_parser_stack};
use sql_text::{TextRange, TextSize};

fn first_stmt(tree: &pb::ParseResult) -> &pb::node::Node {
    tree.stmts[0]
        .stmt
        .as_ref()
        .and_then(|n| n.node.as_ref())
        .unwrap()
}

#[test]
fn parse_tree_of_statements() {
    let tree = parse_tree("SELECT 1").unwrap();
    assert_eq!(tree.stmts.len(), 1);
    assert!(matches!(first_stmt(&tree), pb::node::Node::SelectStmt(_)));
    assert!(tree.version >= 180_000, "{}", tree.version);

    let text = "SELECT 1; DELETE FROM t WHERE a = 1;\n-- trailing\nUPDATE t SET a = 2";
    let tree = parse_tree(text).unwrap();
    assert_eq!(tree.stmts.len(), 3);
    assert!(matches!(
        tree.stmts[1].stmt.as_ref().and_then(|n| n.node.as_ref()),
        Some(pb::node::Node::DeleteStmt(_))
    ));
    // The raw statements' spans cover the statements in text order.
    let mut last_end = 0;
    for s in &tree.stmts {
        let start = usize::try_from(s.stmt_location).unwrap();
        assert!(start >= last_end, "{s:?}");
        let len = usize::try_from(s.stmt_len).unwrap();
        last_end = if len == 0 { text.len() } else { start + len };
    }
    assert_eq!(last_end, text.len());
}

#[test]
fn parse_tree_errors_name_the_server_text() {
    let e = parse_tree("SELEC 1").unwrap_err();
    assert!(e.message.contains("syntax error"), "{e}");
    assert_eq!(e.cursor, Some(TextSize::from(0)));
    // Placeholders are not normalised for the classifier: it sees what Run sends.
    assert!(parse_tree("SELECT :a").is_err());
    assert!(parse_tree("SELECT $1").is_ok());
}

#[test]
fn every_call_passes_the_guard() {
    let nul = "SELECT 1\0; DROP TABLE t";
    assert!(parse_tree(nul).is_err());
    assert!(scan_tokens(nul).is_err());
    assert!(fingerprint(nul).is_err());

    let deep = format!(
        "SELECT {}1{}",
        "(".repeat(usize::from(MAX_ORACLE_NESTING) + 1),
        ")".repeat(usize::from(MAX_ORACLE_NESTING) + 1)
    );
    assert!(parse_tree(&deep).unwrap_err().message.contains("nest"));
    assert!(scan_tokens(&deep).is_err());
    assert!(fingerprint(&deep).is_err());

    let long = format!("SELECT 1{}", " ".repeat(8 << 20));
    assert!(parse_tree(&long).is_err());
    assert!(scan_tokens(&long).is_err());
    assert!(fingerprint(&long).is_err());
}

#[test]
fn a_decode_depth_failure_is_an_err_not_a_panic() {
    // prost stops decoding at 100 nested messages; 47 nested ARRAY[ reach it (ledger 151).
    let nested = |n: usize| format!("SELECT {}1{}", "ARRAY[".repeat(n), "]".repeat(n));
    assert!(parse_tree(&nested(10)).is_ok());
    let e = parse_tree(&nested(47)).unwrap_err();
    assert!(!e.message.is_empty());
    // A limit, told apart from a syntax error (fix round 1, M4).
    assert!(sql_oracle_pg::tree::over_a_limit(&e));
    assert!(!sql_oracle_pg::tree::over_a_limit(
        &parse_tree("SELEC 1").unwrap_err()
    ));
    let deep = format!("SELECT {}1{}", "(".repeat(300), ")".repeat(300));
    assert!(sql_oracle_pg::tree::over_a_limit(
        &parse_tree(&deep).unwrap_err()
    ));
    let long = format!("SELECT 1{}", " ".repeat(9 << 20));
    assert!(sql_oracle_pg::tree::over_a_limit(
        &parse_tree(&long).unwrap_err()
    ));
    let unions: Vec<String> = (0..100).map(|i| format!("SELECT {i}")).collect();
    assert!(parse_tree(&unions.join(" UNION ALL ")).is_err());
    // The thread is still there.
    assert_eq!(on_parser_stack(|| 3), Some(3));
    assert!(parse_tree("SELECT 1").is_ok());
}

#[test]
fn scan_tokens_ranges_and_kinds() {
    let text = "SELECT \"Ad\", 'a''b', 1.5e3, x::int /* c */ FROM şema.t WHERE $1 >= 2";
    let toks = scan_tokens(text).unwrap();
    let pieces: Vec<(&str, ScanKind)> = toks
        .iter()
        .map(|(r, k)| (&text[usize::from(r.start())..usize::from(r.end())], *k))
        .collect();
    assert_eq!(
        pieces,
        [
            ("SELECT", ScanKind::Keyword),
            ("\"Ad\"", ScanKind::Ident),
            (",", ScanKind::Other),
            ("'a''b'", ScanKind::Literal),
            (",", ScanKind::Other),
            ("1.5e3", ScanKind::Literal),
            (",", ScanKind::Other),
            ("x", ScanKind::Ident),
            ("::", ScanKind::Operator),
            ("int", ScanKind::Keyword),
            ("/* c */", ScanKind::Comment),
            ("FROM", ScanKind::Keyword),
            ("şema", ScanKind::Ident),
            (".", ScanKind::Other),
            ("t", ScanKind::Ident),
            ("WHERE", ScanKind::Keyword),
            ("$1", ScanKind::Param),
            (">=", ScanKind::Operator),
            ("2", ScanKind::Literal),
        ]
    );
    // Whitespace-only texts never reach the scanner (its empty-array hazard).
    for blank in ["", " ", "\n\t \r\n", "\u{b}"] {
        assert_eq!(scan_tokens(blank).unwrap(), Vec::new(), "{blank:?}");
    }
    assert!(scan_tokens("SELECT 'unterminated").is_err());
    assert_eq!(
        scan_tokens("-- only").unwrap(),
        vec![(
            TextRange::new(TextSize::from(0), TextSize::from(7)),
            ScanKind::Comment
        )]
    );
}

#[test]
fn fingerprints_ignore_constants_and_see_structure() {
    let a = fingerprint("SELECT * FROM t WHERE a = 1").unwrap();
    assert_eq!(
        a,
        fingerprint("select *  from t where a = 42 -- x").unwrap()
    );
    assert_ne!(a, fingerprint("SELECT * FROM t WHERE b = 1").unwrap());
    assert_ne!(a, fingerprint("DELETE FROM t WHERE a = 1").unwrap());
    assert!(fingerprint("SELEC 1").is_err());
}

const CHILD: &str = "HACIVAT_M3B_T09_STDERR_CHILD";
const MARK: &str = "hv-t09-stderr-line ";
const LINES: usize = 10_000;

/// The child half of [`concurrent_stderr_is_not_lost_during_parses`]: one thread writes
/// [`LINES`] numbered lines straight to fd 2 while this thread keeps libpg_query busy (parse,
/// scan and fingerprint, on the parser thread) until the writer is done and at least
/// [`LINES`] parses ran.
fn stderr_child() {
    let done = Arc::new(AtomicBool::new(false));
    let writer = {
        let done = Arc::clone(&done);
        std::thread::spawn(move || {
            let mut err = std::io::stderr();
            for i in 0..LINES {
                err.write_all(format!("{MARK}{i}\n").as_bytes()).unwrap();
                std::thread::yield_now();
            }
            done.store(true, Ordering::SeqCst);
        })
    };
    let mut parses = 0usize;
    while parses < LINES || !done.load(Ordering::SeqCst) {
        let sql = format!("SELECT a, {parses} FROM t WHERE b = 'x{parses}'");
        assert!(parse_tree(&sql).is_ok());
        if parses.is_multiple_of(10) {
            assert!(scan_tokens(&sql).is_ok());
            assert!(fingerprint(&sql).is_ok());
            assert!(parse_tree("SELEC 1").is_err());
        }
        parses += 1;
    }
    writer.join().unwrap();
}

/// Ledger 151/164 ruling, measured: no line another thread writes to stderr during
/// libpg_query calls is lost. (`pg_query_internal.h` defines `DEBUG`, which compiles out
/// libpg_query's per-call stderr redirect; this test fails if a future version brings it
/// back.) The lines are captured by running this test again as a child process.
#[test]
fn concurrent_stderr_is_not_lost_during_parses() {
    if std::env::var_os(CHILD).is_some() {
        stderr_child();
        return;
    }
    let out = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "concurrent_stderr_is_not_lost_during_parses",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "child failed: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    let seen: BTreeSet<usize> = stderr
        .lines()
        .filter_map(|l| l.strip_prefix(MARK))
        .map(|n| n.parse().unwrap())
        .collect();
    assert_eq!(seen.len(), LINES, "lines lost from stderr");
    assert_eq!(seen.last(), Some(&(LINES - 1)));
}

/// Every `.rs` file under `dir`.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Two unused `pg_query` functions (named in `forbidden`) turn a null C array into a slice, like
/// `scan_tokens` on an empty text (ledger 164): no Hacivat crate names them.
#[test]
fn the_null_slice_functions_stay_unreachable() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    rust_files(&crates, &mut files);
    assert!(files.len() > 50, "{}", crates.display());
    let forbidden = [
        concat!("deparse_comments", "_for_query"),
        concat!("is_utility", "_stmt"),
    ];
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        for name in forbidden {
            assert!(!text.contains(name), "{} names {name}", f.display());
        }
    }
}
