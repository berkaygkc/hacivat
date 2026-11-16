//! The read corpus (Task 10): at least 300 Hacivat-authored honest reads per engine over the
//! `datagrip/13` §3.1 seed schema, and the bar of **zero false denials** on the interim
//! backend (MySQL, MariaDB, SQLite). The PostgreSQL file is Task 9's and Task 14's input; here
//! it is only checked to parse (libpg_query) and to be large enough.

use std::path::PathBuf;
use std::time::Instant;

use sql_dialect::{ClientFlavor, DialectCtx, DialectId, ServerVersion};
use sql_safety::{
    Actor, ClassifyCtx, CoarseClass, Expect, GateClass, Profile, classify_interim, decide,
    gate_class,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Engine {
    Pg,
    MySql,
    MariaDb,
    Sqlite,
}

impl Engine {
    fn file(self) -> &'static str {
        match self {
            Engine::Pg => "pg.sql",
            Engine::MySql => "mysql.sql",
            Engine::MariaDb => "mariadb.sql",
            Engine::Sqlite => "sqlite.sql",
        }
    }

    fn dialect(self) -> DialectId {
        match self {
            Engine::Pg => DialectId::Postgres,
            Engine::MySql => DialectId::MySql,
            Engine::MariaDb => DialectId::MariaDb,
            Engine::Sqlite => DialectId::Sqlite,
        }
    }
}

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sql-testkit/corpus/read")
}

/// The entries of one corpus file: leading `--` lines are comments; an entry ends with a line
/// whose last character is `;` (the `;` is not part of the entry).
fn entries(e: Engine) -> Vec<String> {
    let text = std::fs::read_to_string(corpus_dir().join(e.file())).unwrap();
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in text.lines() {
        if cur.is_empty() && (line.starts_with("--") || line.trim().is_empty()) {
            continue;
        }
        if !cur.is_empty() {
            cur.push('\n');
        }
        cur.push_str(line);
        if let Some(stmt) = cur.strip_suffix(';') {
            out.push(stmt.to_string());
            cur.clear();
        }
    }
    assert!(cur.is_empty(), "{e:?}: unterminated entry {cur:?}");
    out
}

fn read_corpus() -> Vec<(Engine, String)> {
    [Engine::MySql, Engine::MariaDb, Engine::Sqlite]
        .into_iter()
        .flat_map(|e| entries(e).into_iter().map(move |q| (e, q)))
        .collect()
}

fn ctx(e: Engine) -> ClassifyCtx {
    ClassifyCtx::new(
        DialectCtx::new(e.dialect(), None),
        ClientFlavor::None,
        Expect::SingleStatement,
    )
}

#[test]
fn every_corpus_has_at_least_300_distinct_reads() {
    for e in [Engine::Pg, Engine::MySql, Engine::MariaDb, Engine::Sqlite] {
        let all = entries(e);
        let mut uniq = all.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), all.len(), "{e:?}: duplicates");
        assert!(all.len() >= 300, "{e:?}: {} entries", all.len());
    }
}

#[test]
fn read_corpus_has_no_false_denial() {
    let mut failures = Vec::new();
    for (engine, q) in read_corpus() {
        let r = classify_interim(&q, &ctx(engine));
        if r.request_class != CoarseClass::Read {
            failures.push(format!(
                "{engine:?}: {q}\n    {:?} {:?} {:?}",
                r.request_class,
                r.request_effects,
                r.items
                    .iter()
                    .flat_map(|i| i.reasons.iter().map(|x| (x.code, x.arg.clone())))
                    .collect::<Vec<_>>()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} false denials:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The corpus is also allowed end to end on the read-only profiles and is `Read` at the gate,
/// except where the policy says otherwise for every read (none in this corpus).
#[test]
fn read_corpus_is_allowed_on_read_only_and_read_at_the_gate() {
    for (engine, q) in read_corpus() {
        let r = classify_interim(&q, &ctx(engine));
        for p in [Profile::ReadOnly, Profile::ReadOnlyPlusExplain] {
            assert!(
                decide(&r, p, Actor::Human).allows(),
                "{engine:?} {p:?}: {q}"
            );
        }
        for item in &r.items {
            assert_eq!(gate_class(item), GateClass::Read, "{engine:?}: {q}");
        }
    }
}

/// Fix round 1 (M6): the corpus again with explicit server versions, where the probe floors
/// and version gates matter: MySQL 8.0.12 and 8.4.0, MariaDB 10.6.0.
#[test]
fn read_corpus_has_no_false_denial_on_explicit_server_versions() {
    let mut failures = Vec::new();
    for (engine, version) in [
        (Engine::MySql, ServerVersion::new(8, 0, 12)),
        (Engine::MySql, ServerVersion::new(8, 4, 0)),
        (Engine::MariaDb, ServerVersion::new(10, 6, 0)),
    ] {
        let ctx = ClassifyCtx::new(
            DialectCtx::new(engine.dialect(), Some(version)),
            ClientFlavor::None,
            Expect::SingleStatement,
        );
        for q in entries(engine) {
            let r = classify_interim(&q, &ctx);
            if r.request_class != CoarseClass::Read {
                failures.push(format!(
                    "{engine:?} {version:?}: {q}\n    {:?}",
                    r.items
                        .iter()
                        .flat_map(|i| i.reasons.iter().map(|x| (x.code, x.arg.clone())))
                        .collect::<Vec<_>>()
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} false denials:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Every PostgreSQL entry is one statement libpg_query accepts (Task 9 classifies it).
#[test]
fn pg_corpus_parses_as_one_statement_each() {
    for q in entries(Engine::Pg) {
        assert_eq!(sql_oracle_pg::statement_count(&q).ok(), Some(1), "{q}");
    }
}

/// Classification time on the interim path (release, `HACIVAT_PERF=1`):
///
/// - a dense ~10 KB read unit (a wide `CASE`/call select list over a join) of each interim
///   dialect within **2 ms per 10 KB** for the whole `classify_interim` (lex, split, `HvNorm1`,
///   the backend). The spec's ≤ 1 ms per 10 KB (SQL-SAFE-02) is `classify`'s budget (Task 14,
///   `--test classify`); on this path `sqlparser`'s own tokenize and parse take ~0.85 ms of the
///   ~1.5 ms measured at Task 10 (report; Task 20 owns performance);
/// - the corpus's short statements within 50 µs each (the fixed costs).
///
/// Each figure is the fastest of several runs, so a busy machine does not fail it.
#[test]
#[ignore = "perf"]
fn read_corpus_classifies_within_budget() {
    if std::env::var("HACIVAT_PERF").as_deref() != Ok("1") {
        return;
    }
    // ## m3b-t14-orchestrator (named exception, fix round 1, I2): measured on a classifier
    // thread (16 MiB, marked), as the pool runs it; elsewhere each call first moves to a
    // scoped thread of its own, which this budget is not about.
    sql_safety::spawn_classifier_thread("read-corpus-budget".to_string(), within_budget)
        .unwrap()
        .join()
        .unwrap();
}

fn within_budget() {
    const RUNS: u32 = 20;
    for e in [Engine::MySql, Engine::MariaDb, Engine::Sqlite] {
        // One ~10 KB read: a wide select list over a join, the shape of a reporting query.
        let mut cols = Vec::new();
        let mut k = 0;
        while cols.iter().map(|c: &String| c.len() + 2).sum::<usize>() < 10_000 {
            cols.push(format!(
                "CASE WHEN o.id > {k} THEN UPPER(c.full_name) ELSE LOWER(c.email) END AS c{k}"
            ));
            k += 1;
        }
        let unit = format!(
            "SELECT {} FROM customers c JOIN orders o ON o.customer_id = c.id WHERE c.city = 'İstanbul'",
            cols.join(", ")
        );
        let r = classify_interim(&unit, &ctx(e));
        assert_eq!(r.request_class, CoarseClass::Read, "{e:?}");
        // The fastest of the runs: the machine may be busy (other builds); the budget is the
        // classifier's own cost.
        let best = (0..RUNS)
            .map(|_| {
                let start = Instant::now();
                std::hint::black_box(classify_interim(&unit, &ctx(e)));
                start.elapsed().as_secs_f64()
            })
            .fold(f64::INFINITY, f64::min);
        let per_10k = best / (unit.len() as f64 / 10_240.0);
        eprintln!("{e:?}: {:.3} ms per 10 KB", per_10k * 1e3);
        assert!(per_10k < 2e-3, "{e:?}: {:.3} ms per 10 KB", per_10k * 1e3);
    }
    let corpus = read_corpus();
    let best = (0..RUNS / 4)
        .map(|_| {
            let start = Instant::now();
            for (e, q) in &corpus {
                std::hint::black_box(classify_interim(q, &ctx(*e)));
            }
            start.elapsed().as_secs_f64()
        })
        .fold(f64::INFINITY, f64::min);
    let per_stmt = best / corpus.len() as f64;
    eprintln!("corpus: {:.1} µs per statement", per_stmt * 1e6);
    assert!(per_stmt < 50e-6, "{:.1} µs per statement", per_stmt * 1e6);
}
