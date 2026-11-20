//! Gate G5 (M4 Task 31; spec §6 M4 and §7.1 "Golden tests … generated DDL (byte-stable)"):
//! every generated-DDL golden renders to the same bytes twice in one process, in child
//! processes whose hash seeds differ, and under the `tr_TR` and `C` locales; every golden file
//! is LF-only and ends with exactly one newline (a Windows checkout that converted line
//! endings fails here, `.gitattributes` keeps the directory `-text`). The goldens are never
//! blessed here: only `UPDATE_GOLDEN=1 cargo test -p hacivat-ddl --test golden` writes them, and
//! CI's `golden-guard` fails a run that changed one.
//!
//! The cases are `fixtures::CASES` (every file under `tests/golden/{pg,mysql,mariadb,sqlite}`;
//! `golden.rs` proves none is orphaned). The IR goldens under `tests/golden/ir` are rendered by
//! `render_ir.rs` (`ir_goldens_byte_stable`, `output_is_identical_under_en_and_tr`); their files
//! are checked for line endings here too.

mod fixtures;

use std::collections::BTreeSet;
use std::hash::BuildHasher;
use std::path::{Path, PathBuf};

use fixtures::{CASES, golden_path};

const BEGIN: &str = "<<<hv-t31-golden-begin>>>";
const END: &str = "<<<hv-t31-golden-end>>>";
const SEED: &str = "hv-t31-seed ";

fn blessing() -> bool {
    std::env::var_os("UPDATE_GOLDEN").is_some_and(|v| v == "1")
}

/// Every case's text, each preceded by a line naming its golden.
fn all_cases_text() -> String {
    let mut out = String::new();
    for c in CASES {
        out.push_str("-- ");
        out.push_str(c.rel);
        out.push('\n');
        out.push_str(&(c.make)());
    }
    out
}

/// The same text built from the blessed files.
fn all_goldens_text() -> String {
    let mut out = String::new();
    for c in CASES {
        out.push_str("-- ");
        out.push_str(c.rel);
        out.push('\n');
        let bytes = std::fs::read(golden_path(c.rel)).expect("blessed golden");
        out.push_str(&String::from_utf8(bytes).expect("a golden is UTF-8"));
    }
    out
}

/// Prints this process's hash-seed fingerprint and [`all_cases_text`]; run in a child by the
/// tests below.
#[test]
#[ignore = "helper: run in a child process by the G5 tests"]
fn golden_stability_probe() {
    // `RandomState` keys come from the OS once per process: a fixed value hashed with a new
    // state differs between processes whose seeds differ.
    let fingerprint = std::collections::hash_map::RandomState::new().hash_one("hv-t31");
    println!("{SEED}{fingerprint:016x}");
    println!("{BEGIN}");
    print!("{}", all_cases_text());
    println!("{END}");
}

/// What a child probe printed: its seed fingerprint and its golden text.
fn run_probe(env: &[(&str, &str)]) -> (String, String) {
    let exe = std::env::current_exe().expect("test binary");
    let mut cmd = std::process::Command::new(exe);
    cmd.args([
        "--exact",
        "golden_stability_probe",
        "--ignored",
        "--nocapture",
        "--test-threads=1",
    ]);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run the probe");
    assert!(out.status.success(), "probe failed ({env:?}): {out:?}");
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    // libtest prints `test golden_stability_probe ... ` on the same line before the output.
    let at = stdout.find(SEED).expect("seed line") + SEED.len();
    let seed = stdout[at..].lines().next().expect("seed").to_string();
    let start = stdout.find(BEGIN).expect("begin marker") + BEGIN.len() + 1;
    let end = stdout.find(END).expect("end marker");
    (seed, stdout[start..end].to_string())
}

#[test]
fn every_golden_renders_byte_identical_twice_in_one_process() {
    for c in CASES {
        let a = (c.make)();
        let b = (c.make)();
        assert!(
            a.as_bytes() == b.as_bytes(),
            "{} differs between two renders",
            c.rel
        );
        if !blessing() {
            let file = std::fs::read(golden_path(c.rel)).expect("blessed golden");
            assert!(file == a.as_bytes(), "{} differs from its golden", c.rel);
        }
    }
    // Interleaved: rendering the other cases in between changes nothing.
    assert_eq!(all_cases_text(), all_cases_text());
    println!("HV-FACT g5 in-process cases={} identical=yes", CASES.len());
}

#[test]
fn every_golden_renders_identically_with_shuffled_hash_seeds() {
    let here = all_cases_text();
    let mut seeds = BTreeSet::new();
    for i in 0..3 {
        // The label only tells the children apart in a failure; std's hash keys are drawn
        // from the OS per process (no environment variable sets them).
        let label = i.to_string();
        let (seed, text) = run_probe(&[("HV_T31_PROBE", &label)]);
        seeds.insert(seed);
        assert!(
            text == here,
            "child {i}: golden text differs from this process"
        );
    }
    assert_eq!(seeds.len(), 3, "the children ran with distinct hash seeds");
    println!(
        "HV-FACT g5 hash-seeds children=3 distinct_seeds={} identical=yes",
        seeds.len()
    );
}

/// Every file under `dir`, recursively, sorted.
fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).expect("read dir") {
        let p = e.expect("entry").path();
        if p.is_dir() {
            files_under(&p, out);
        } else {
            out.push(p);
        }
    }
}

#[test]
fn golden_files_are_lf_only_and_end_with_one_newline() {
    let root = golden_path("x");
    let root = root.parent().expect("golden dir");
    let mut files = Vec::new();
    files_under(root, &mut files);
    files.sort();
    assert!(
        files.len() > CASES.len(),
        "every case file plus the IR goldens"
    );
    for f in &files {
        let shown = f.strip_prefix(root).unwrap_or(f).display().to_string();
        assert_eq!(
            f.extension().and_then(|e| e.to_str()),
            Some("sql"),
            "{shown}: only .sql goldens"
        );
        let bytes = std::fs::read(f).expect("read golden");
        assert!(!bytes.is_empty(), "{shown}: empty");
        assert!(!bytes.contains(&b'\r'), "{shown}: carriage return (CRLF)");
        assert!(bytes.ends_with(b"\n"), "{shown}: no final newline");
        assert!(
            !bytes.ends_with(b"\n\n"),
            "{shown}: more than one final newline"
        );
        assert!(std::str::from_utf8(&bytes).is_ok(), "{shown}: not UTF-8");
        assert!(
            !bytes.starts_with(b"\xEF\xBB\xBF"),
            "{shown}: a byte-order mark"
        );
    }
    println!(
        "HV-FACT g5 files={} lf_only=yes one_final_newline=yes",
        files.len()
    );
}

/// Run in CI's `turkey-test` too (the whole job runs under `LC_ALL=tr_TR.UTF-8`), where this
/// process itself is Turkish.
#[test]
fn goldens_identical_under_lang_tr_and_c() {
    if blessing() {
        return;
    }
    let blessed = all_goldens_text();
    assert!(
        all_cases_text() == blessed,
        "this process differs from the goldens"
    );
    for locale in ["tr_TR.UTF-8", "C"] {
        let (_, text) = run_probe(&[("LC_ALL", locale), ("LANG", locale)]);
        assert!(text == blessed, "{locale}: differs from the goldens");
    }
    println!("HV-FACT g5 locales=tr_TR.UTF-8,C identical=yes");
}
