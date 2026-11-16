//! The SQL benchmark behind `cargo xtask bench-sql` (Task 20; SQL-T13, L2): lexing, the worst
//! relex, relex and resplit of a typing edit, `resolve`, and `classify`, on the gen-sql 1 MB and
//! 50k-line fixtures (`sql-lexer/tests/support/gen_sql.rs`, byte for byte the B3 fixture), plus
//! the classifier's per-10 KB unit and the PostgreSQL oracle's one-call against three-call
//! parse (the T14 perf carry). xtask runs it as a subprocess (no SQL crate in xtask, T1a I3) and
//! collects every `bench-sql: {json}` line into its JSON report.
//!
//! Figures, not assertions: CI reads them as a relative signal (L2); no number here is a
//! budget. It runs only when asked (`#[ignore]`, and `HACIVAT_BENCH_SQL=1`), in release.

#[path = "../../sql-lexer/tests/support/gen_sql.rs"]
mod gen_sql;

use std::time::{Duration, Instant};

use sql_dialect::{ClientFlavor, DialectCtx, DialectId, LexModes, lex_profile};
use sql_lexer::{LexState, lex_with_client, relex};
use sql_safety::{ClassifyCtx, Expect, classify};
use sql_split::exec_range::{CaretOrSelection, ExecPolicy, resolve};
use sql_split::{SplitMode, SqlDocument};
use sql_text::{TextEdit, TextRange, TextSize};

fn enabled() -> bool {
    std::env::var("HACIVAT_BENCH_SQL").as_deref() == Ok("1")
}

fn size(n: usize) -> TextSize {
    TextSize::from(u32::try_from(n).unwrap())
}

fn floor(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Best, median and 95th percentile of `times`, in milliseconds.
fn stats(times: &mut [Duration]) -> (f64, f64, f64) {
    times.sort_unstable();
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    let n = times.len();
    (
        ms(times[0]),
        ms(times[n / 2]),
        ms(times[(n * 95 / 100).min(n - 1)]),
    )
}

/// One result line: `bench-sql: {json}` (no JSON crate needed; names are plain ASCII).
fn emit(name: &str, fixture: &str, bytes: usize, times: &mut [Duration]) {
    let (best, p50, p95) = stats(times);
    println!(
        "bench-sql: {{\"name\":\"{name}\",\"fixture\":\"{fixture}\",\"bytes\":{bytes},\"runs\":{},\"best_ms\":{best:.4},\"p50_ms\":{p50:.4},\"p95_ms\":{p95:.4}}}",
        times.len()
    );
}

fn time(runs: usize, mut f: impl FnMut()) -> Vec<Duration> {
    (0..runs)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .collect()
}

fn pg() -> DialectCtx {
    DialectCtx::new(DialectId::Postgres, None)
}

/// One dense ~10 KB read unit (as `tests/classify.rs`'s budget test).
fn ten_kb_unit() -> String {
    let mut cols = Vec::new();
    let mut k = 0;
    while cols.iter().map(|c: &String| c.len() + 2).sum::<usize>() < 10_000 {
        cols.push(format!(
            "CASE WHEN o.id > {k} THEN UPPER(c.full_name) ELSE LOWER(c.email) END AS c{k}"
        ));
        k += 1;
    }
    format!(
        "SELECT {} FROM customers c JOIN orders o ON o.customer_id = c.id WHERE c.city = 'İstanbul'",
        cols.join(", ")
    )
}

#[test]
#[ignore = "bench"]
fn bench_sql() {
    if !enabled() {
        return;
    }
    let h = sql_safety::spawn_classifier_thread("bench-sql".to_string(), run).unwrap();
    if let Err(p) = h.join() {
        std::panic::resume_unwind(p);
    }
}

fn run() {
    let ctx = pg();
    let p = lex_profile(&ctx).unwrap();
    let fifty_k = gen_sql::gen_sql(50_000);
    let mut one_mb = fifty_k.clone();
    one_mb.truncate(floor(&one_mb, 1 << 20));
    // Cut at the last statement end, so the 1 MB fixture is whole statements.
    if let Some(end) = one_mb.rfind(";\n") {
        one_mb.truncate(end + 2);
    }
    let fixtures = [
        ("gen-sql-1mb", one_mb.as_str()),
        ("gen-sql-50k-lines", fifty_k.as_str()),
    ];

    // Lexing, whole.
    for (name, text) in fixtures {
        let mut t = time(10, || {
            std::hint::black_box(lex_with_client(
                text,
                p,
                ctx.modes,
                &LexState::Initial,
                ClientFlavor::Psql,
            ));
        });
        emit("lex", name, text.len(), &mut t);
    }

    // The worst relex: an opening quote at offset 0 of 1 MB.
    let base = lex_with_client(
        &one_mb,
        p,
        ctx.modes,
        &LexState::Initial,
        ClientFlavor::Psql,
    );
    let quote = TextEdit {
        delete: TextRange::empty(size(0)),
        insert: "'".into(),
    };
    let mut quoted = one_mb.clone();
    quote.apply(&mut quoted).unwrap();
    let mut t: Vec<Duration> = (0..10)
        .map(|_| {
            let mut l = base.clone();
            let start = Instant::now();
            relex(&mut l, &quoted.as_str(), &quote, p, ctx.modes);
            start.elapsed()
        })
        .collect();
    emit("relex-worst", "gen-sql-1mb", one_mb.len(), &mut t);

    // A typing edit (a character typed at a random place, then undone): relex alone, and relex
    // with resplit (`SqlDocument::apply`, the editor's path).
    let mut l = lex_with_client(
        &fifty_k,
        p,
        ctx.modes,
        &LexState::Initial,
        ClientFlavor::Psql,
    );
    let mut text = fifty_k.clone();
    let mut rng = gen_sql::Positions::new(20);
    let mut relex_t = Vec::new();
    for _ in 0..200 {
        let a = floor(&fifty_k, rng.below(fifty_k.len()));
        let e = TextEdit {
            delete: TextRange::empty(size(a)),
            insert: "x".into(),
        };
        let undo = TextEdit {
            delete: TextRange::new(size(a), size(a + 1)),
            insert: String::new(),
        };
        e.apply(&mut text).unwrap();
        let start = Instant::now();
        relex(&mut l, &text.as_str(), &e, p, ctx.modes);
        relex_t.push(start.elapsed());
        undo.apply(&mut text).unwrap();
        relex(&mut l, &text.as_str(), &undo, p, ctx.modes);
    }
    emit(
        "relex-typing",
        "gen-sql-50k-lines",
        fifty_k.len(),
        &mut relex_t,
    );

    let mut doc = SqlDocument::new(fifty_k.clone(), ctx, ClientFlavor::Psql);
    let mut resplit_t = Vec::new();
    for _ in 0..200 {
        let a = floor(&fifty_k, rng.below(fifty_k.len()));
        let held = doc.snapshot();
        let _ = held.execution_split();
        let start = Instant::now();
        doc.apply(&[TextEdit {
            delete: TextRange::empty(size(a)),
            insert: "x".into(),
        }])
        .unwrap();
        resplit_t.push(start.elapsed());
        drop(held);
        doc.apply(&[TextEdit {
            delete: TextRange::new(size(a), size(a + 1)),
            insert: String::new(),
        }])
        .unwrap();
    }
    emit(
        "resplit-typing",
        "gen-sql-50k-lines",
        fifty_k.len(),
        &mut resplit_t,
    );

    // `resolve`: the caret's unit, and the whole script, on a fresh snapshot each time (its
    // execution split computed inside the timing).
    for (name, text) in fixtures {
        let d = SqlDocument::new(text.to_owned(), ctx, ClientFlavor::Psql);
        let mid = CaretOrSelection::Caret(size(floor(text, text.len() / 2)));
        let mut t = time(10, || {
            let snap = d.snapshot();
            std::hint::black_box(resolve(
                &snap,
                &[mid],
                &ExecPolicy::default(),
                SplitMode::Execution,
                None,
                None,
            ));
        });
        emit("resolve-caret", name, text.len(), &mut t);
        let mut t = time(10, || {
            let snap = d.snapshot();
            std::hint::black_box(resolve(
                &snap,
                &[CaretOrSelection::Caret(size(0))],
                &ExecPolicy::whole_script(),
                SplitMode::Execution,
                None,
                None,
            ));
        });
        emit("resolve-whole-script", name, text.len(), &mut t);
    }

    // `classify`: the 1 MB fixture as one script (the classifier's size limit is 1 MiB), and
    // its statements one at a time (how Run classifies units).
    let script = ClassifyCtx::new(ctx, ClientFlavor::None, Expect::Script);
    let mut t = time(5, || {
        std::hint::black_box(classify(&one_mb, &script));
    });
    // What the script's report is (a long script is often decided before any backend runs).
    let r = classify(&one_mb, &script);
    let outcome: Vec<String> = r
        .pre_decision
        .iter()
        .flatten()
        .map(|x| format!("{:?}", x.code))
        .collect();
    println!(
        "bench-sql: {{\"name\":\"classify-script-outcome\",\"fixture\":\"gen-sql-1mb\",\"items\":{},\"pre_decision\":\"{}\"}}",
        r.items.len(),
        outcome.join(",")
    );
    emit("classify-script", "gen-sql-1mb", one_mb.len(), &mut t);
    let d = SqlDocument::new(one_mb.clone(), ctx, ClientFlavor::Psql);
    let units: Vec<String> = d
        .snapshot()
        .execution_split()
        .statements()
        .map(|s| one_mb[s.strict].to_owned())
        .collect();
    let single = ClassifyCtx::new(ctx, ClientFlavor::None, Expect::SingleStatement)
        .with_known_modes(LexModes::default_for(DialectId::Postgres));
    let mut t = time(3, || {
        for u in &units {
            std::hint::black_box(classify(u, &single));
        }
    });
    emit("classify-units", "gen-sql-1mb", one_mb.len(), &mut t);

    // The classifier per 10 KB unit, per dialect, modes known.
    let unit = ten_kb_unit();
    for (name, dialect) in [
        ("classify-10kb-pg", DialectId::Postgres),
        ("classify-10kb-mysql", DialectId::MySql),
        ("classify-10kb-mariadb", DialectId::MariaDb),
        ("classify-10kb-sqlite", DialectId::Sqlite),
    ] {
        let c = ClassifyCtx::new(
            DialectCtx::new(dialect, None),
            ClientFlavor::None,
            Expect::SingleStatement,
        )
        .with_known_modes(LexModes::default_for(dialect));
        let mut t = time(200, || {
            std::hint::black_box(classify(&unit, &c));
        });
        emit(name, "10kb-unit", unit.len(), &mut t);
    }

    // The T14 perf carry, measured: libpg_query's three calls (tree with `pg_query`'s summary
    // walk, scan, fingerprint with its own parse) against the one call the PG backend makes.
    let mut t = time(200, || {
        std::hint::black_box(sql_oracle_pg::tree::parse_tree(&unit).unwrap());
        std::hint::black_box(sql_oracle_pg::tree::scan_tokens(&unit).unwrap());
        std::hint::black_box(sql_oracle_pg::tree::fingerprint(&unit).unwrap());
    });
    emit("pg-oracle-three-calls", "10kb-unit", unit.len(), &mut t);
    let mut t = time(200, || {
        std::hint::black_box(sql_oracle_pg::tree::analyse(&unit).unwrap());
    });
    emit("pg-oracle-one-call", "10kb-unit", unit.len(), &mut t);

    // Executable comments: 16 readings of a 10 KB MySQL unit (inside the readings × length
    // budget), and 16 readings of 20 KB (past it: `TooComplex` before any reading).
    let my = ClassifyCtx::new(
        DialectCtx::new(DialectId::MySql, None),
        ClientFlavor::None,
        Expect::SingleStatement,
    )
    .with_known_modes(LexModes::default_for(DialectId::MySql));
    for (name, bytes) in [
        ("classify-16-readings-10kb", 10_000),
        ("classify-16-readings-20kb", 20_000),
    ] {
        let mut s = String::from("SELECT 1");
        for g in 0..15 {
            s.push_str(&format!(" /*!{} , {g} */", 50001 + g));
        }
        let mut k = 0;
        while s.len() < bytes {
            s.push_str(&format!(
                ", CASE WHEN a > {k} THEN UPPER(b) ELSE LOWER(c) END AS c{k}"
            ));
            k += 1;
        }
        s.push_str(" FROM t");
        let mut t = time(20, || {
            std::hint::black_box(classify(&s, &my));
        });
        emit(name, "mysql-exec-comments", s.len(), &mut t);
    }
}
