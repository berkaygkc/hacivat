//! The patched one-parse oracle is byte-identical to upstream's separate calls on the
//! PostgreSQL regression corpus (Task 20 fix round 1; `third_party/pg_query` PATCHES §3).
//!
//! `sql_oracle_pg::tree::analyse` (one raw parse for the tree and the fingerprint, no summary
//! walk) must answer what `parse_tree`, `scan_tokens` and `fingerprint` (upstream's second
//! parse) answer: the same tree, tokens, errors and fingerprint, on every REL_17_2 execution
//! statement (its strict range, and with its terminator) and every whole file. The one designed
//! divergence: when libpg_query cannot encode the tree (nested too deeply to serialize),
//! `analyse` gives the same tree `Err` and no fingerprint where `fingerprint` still gives one;
//! `classify` then reports `ParseError` and keeps `HvNorm1`, as before the patch.
//!
//! It lives here, not in `sql-testkit`'s differential, because only the oracle and this crate
//! may name `sql_oracle_pg::tree` (`cargo xtask check-deps`). Needs the corpus (`cargo xtask
//! fetch-pg-regress --tag REL_17_2`); `#[ignore = "network"]`; the nightly `sql-differential`
//! job runs it:
//!
//! ```text
//! cargo test -p sql-safety --test oracle_corpus -- --ignored
//! ```

use std::path::PathBuf;

use sql_dialect::{ClientFlavor, DialectCtx, DialectId, ServerVersion};
use sql_oracle_pg::tree::{analyse, fingerprint, parse_tree, scan_tokens};
use sql_split::{SplitMode, SplitStart, split_from};
use sql_text::TextRange;

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target-pg-regress/REL_17_2/sql")
}

/// Every statement of a psql script as Run cuts it (strict, then with its terminator), then
/// the whole file.
fn texts_of(text: &str) -> Vec<String> {
    let ctx = DialectCtx::new(DialectId::Postgres, Some(ServerVersion::new(17, 2, 0)));
    let split = split_from(
        text,
        &ctx,
        ClientFlavor::Psql,
        SplitMode::Execution,
        &SplitStart::default(),
    );
    let mut out = Vec::new();
    for s in split.statements() {
        out.push(text[s.strict].to_string());
        let end = s.terminator.map_or(s.strict.end(), |t| t.end());
        out.push(text[TextRange::new(s.strict.start(), end)].to_string());
    }
    out.push(text.to_string());
    out
}

fn short(t: &str) -> String {
    t.chars().take(120).collect()
}

#[test]
#[ignore = "network"]
fn the_one_parse_oracle_equals_upstreams_separate_calls_on_the_corpus() {
    let dir = corpus_dir();
    let mut names: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| {
            panic!(
                "no corpus at {} ({e}): run `cargo xtask fetch-pg-regress --tag REL_17_2`",
                dir.display()
            )
        })
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "sql"))
        .collect();
    names.sort();
    let mut texts = Vec::new();
    for p in names {
        if let Ok(text) = String::from_utf8(std::fs::read(&p).unwrap()) {
            texts.extend(texts_of(&text));
        }
    }
    let h = sql_safety::spawn_classifier_thread("oracle-corpus".to_string(), move || {
        let (mut fingerprinted, mut divergent, mut failures) = (0usize, 0usize, Vec::new());
        for t in &texts {
            let tree = parse_tree(t);
            let tokens = scan_tokens(t);
            let fp = fingerprint(t);
            match analyse(t) {
                Ok(a) => {
                    if a.tree != tree {
                        failures.push(format!("tree differs: {:?}", short(t)));
                    }
                    if a.tokens != tokens {
                        failures.push(format!("tokens differ: {:?}", short(t)));
                    }
                    match (&a.fingerprint, &fp) {
                        (Some(x), Ok(y)) if x == y => fingerprinted += 1,
                        (None, Err(_)) => {}
                        (None, Ok(_)) if a.tree.is_err() => divergent += 1,
                        _ => failures.push(format!(
                            "fingerprint {:?} vs {:?}: {:?}",
                            a.fingerprint,
                            fp,
                            short(t)
                        )),
                    }
                }
                Err(e) => {
                    let same = tree.as_ref().err() == Some(&e)
                        && tokens.as_ref().err() == Some(&e)
                        && fp.as_ref().err() == Some(&e);
                    if !same {
                        failures.push(format!("guard error {e:?} differs: {:?}", short(t)));
                    }
                }
            }
        }
        eprintln!(
            "one-parse oracle: {} texts, {fingerprinted} fingerprinted alike, {divergent} \
             encode-failure divergences (designed), {} failures",
            texts.len(),
            failures.len()
        );
        assert!(texts.len() >= 80_000, "{} texts", texts.len());
        assert!(fingerprinted >= 80_000, "{fingerprinted} fingerprinted");
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    })
    .unwrap();
    if let Err(p) = h.join() {
        std::panic::resume_unwind(p);
    }
}
