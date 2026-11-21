//! M3b Task 1a: the classifier seams fail closed until their tasks land (F31), and no backend
//! type leaks into the public API (`BackendOutsideOwner`, R-M3-54). Tasks 4 and 14 replaced the
//! stub assertions here when they fill `decide` and `classify`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use sql_dialect::{ClientFlavor, DialectCtx, DialectId};
use sql_safety::{
    Actor, ClassifyCtx, CoarseClass, EffectSet, Expect, Profile, ReasonCode, Verdict, classify,
    decide,
};

fn ctx_pg() -> ClassifyCtx {
    ClassifyCtx::new(
        DialectCtx::new(DialectId::Postgres, None),
        ClientFlavor::Psql,
        Expect::SingleStatement,
    )
}

// ## m3b-t04-safety (named exception): Task 4 adds eleven policy reasons (48 codes).
fn all_reason_codes() -> [ReasonCode; 48] {
    ReasonCode::ALL
}

// ## m3b-t14-orchestrator (named exception): `classify` is real since Task 14; the stub's
// assertions become the real classifier's (Task 14's own tests: `tests/classify.rs`).
#[test]
fn classify_reads_a_select_and_fails_closed_on_nothing() {
    let r = classify("SELECT 1", &ctx_pg());
    assert_eq!(r.request_class, CoarseClass::Read);
    assert!(r.pre_decision.is_none());
    assert_eq!(r.classified_len, 8);
    assert_eq!(r.items.len(), 1);
    assert_eq!(r.dialect, Some(DialectId::Postgres));
    assert!(!r.request_effects.contains(EffectSet::UNKNOWN));
    // A failure keeps the stub's fail-closed shape in every field a caller might read first.
    let e = classify("", &ctx_pg());
    assert_eq!(e.request_class, CoarseClass::Unknown);
    assert!(e.request_effects.contains(EffectSet::UNKNOWN));
    assert!(e.items.is_empty());
    let reasons = e.pre_decision.as_deref().unwrap();
    assert_eq!(reasons.len(), 1);
    assert_eq!(reasons[0].code, ReasonCode::NoStatement);
}

#[test]
fn classified_hash_matches_only_the_exact_bytes() {
    let r = classify("SELECT $1", &ctx_pg());
    assert!(r.matches_text("SELECT $1"));
    assert!(!r.matches_text("SELECT :a"));
    assert!(!r.matches_text("SELECT $1 "));
    assert_eq!(r.classified_hash, sql_safety::classified_hash("SELECT $1"));
}

// ## m3b-t04-safety (named exception): `decide` is real since Task 4; a pre-decided report
// denies in every profile, `Admin` included.
// ## m3b-t14-orchestrator (named exception): the classify stub's pre-decided `SELECT 1` is a
// read now (allowed on read-only); an unterminated string is the pre-decided report.
#[test]
fn decide_denies_a_pre_decided_report_everywhere() {
    assert!(
        decide(
            &classify("SELECT 1", &ctx_pg()),
            Profile::ReadOnly,
            Actor::Human
        )
        .allows()
    );
    for p in [
        Profile::ReadOnly,
        Profile::ReadOnlyPlusExplain,
        Profile::ReadWriteDml,
        Profile::Ddl,
        Profile::Admin,
    ] {
        let d = decide(&classify("SELECT 'open", &ctx_pg()), p, Actor::Human);
        assert!(matches!(d.verdict, Verdict::Deny(_)), "{p:?}");
        assert_eq!(d.class, CoarseClass::Unknown, "{p:?}");
    }
}

#[test]
fn every_reason_code_has_a_literal_fluent_id() {
    let mut seen = BTreeSet::new();
    for c in all_reason_codes() {
        let id = c.fluent_id();
        assert!(id.starts_with("safety-reason-"), "{c:?}");
        assert!(
            id.bytes().all(|b| b.is_ascii_lowercase() || b == b'-'),
            "{c:?}: {id}"
        );
        assert!(seen.insert(id), "duplicate id {id}");
    }
}

/// Every `.rs` file under `dir`.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Every non-comment line of every module outside the private `backend/` and `visit/` trees,
/// with its file (fix round 1, M1: a `pub`-line scan missed trait impls, aliases and wrapped
/// signatures). Only the backends may name a backend crate or the oracle's tree.
fn lines_outside_backends() -> Vec<(PathBuf, String)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    files.sort();
    let mut out = Vec::new();
    for f in files {
        let rel = f.strip_prefix(&src).unwrap();
        if rel.starts_with("backend") || rel.starts_with("visit") {
            continue;
        }
        for line in std::fs::read_to_string(&f).unwrap().lines() {
            let t = line.trim_start();
            if !t.starts_with("//") {
                out.push((rel.to_path_buf(), t.to_string()));
            }
        }
    }
    out
}

#[test]
fn no_sqlparser_type_in_the_public_api() {
    for (file, line) in lines_outside_backends() {
        for needle in ["sqlparser", "pg_query", "tree::pb", "pb::"] {
            assert!(!line.contains(needle), "{}: {line}", file.display());
        }
    }
    // The backends and visitors stay private modules.
    let lib =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs")).unwrap();
    for private in ["backend", "visit"] {
        assert!(lib.contains(&format!("\nmod {private};")), "{private}");
        assert!(!lib.contains(&format!("pub mod {private};")), "{private}");
    }
}
