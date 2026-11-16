//! The classifier's offline differential and fuzz properties (Task 20; SQL-T13, R-M3-80;
//! `alg/13` :399). Each fuzz target of `fuzz/` exists here, or in its own crate, as a seeded
//! property (no proptest strategy: seeded generators, the M3a ruling); `PROPTEST_CASES` raises
//! the case count (the nightly jobs), as for every other generated property.
//!
//! - **`classify` never panics** (`fuzz/classify_no_panic`): on generated text, every dialect,
//!   both lexing-mode paths and both expectations, and its report is well formed.
//! - **The seeds never get weaker:** the adversarial inputs of the Task 9, 10 and 14 security
//!   reviews (`sql-testkit/corpus/fuzz/classifier_seeds.toml`) keep at least their accepted
//!   verdicts.
//! - **Meaning-preserving mutations never weaken a verdict** (letter case, whitespace, a
//!   comment for a space).
//! - **The PostgreSQL oracle:** mutants of the read corpus and the seeds that the read-only
//!   profile allows parse, with libpg_query, to no write: no `INSERT`/`UPDATE`/`DELETE`/
//!   `MERGE` anywhere in the tree, no `INTO`, no row lock, no other statement than the
//!   read-only ones. (Functions with side effects are the server's to show: the Docker leg,
//!   `hacivat-exec/tests/docker_mutation_fuzz.rs`.)
//! - **Escaped literals** (T14's corpus) and their meaning-preserving mutants: no false denial
//!   on the known-modes and the unknown-modes paths.
//! - **The T10 gap:** unqualified `threads` / `session` with `performance_schema` / `sys` as
//!   the current database.
//! - **Readings × length** (T10 review): a statement whose executable comments would hand
//!   `sqlparser` more than 256 KiB of readings is `TooComplex`, quickly.
//!
//! Every finding is a failure that names the input.

#[path = "../../sql-lexer/tests/support/edits.rs"]
mod edits;
#[path = "support/mutate.rs"]
mod mutate;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use edits::{Rng, cases};
use mutate::Family;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, LexModes, PgWireFlavor, ServerVersion};
use sql_safety::{
    Actor, ClassifyCtx, CoarseClass, Expect, Profile, ReasonCode, SafetyReport, Verdict, classify,
    decide, pg_backend_hooks,
};

// ---------------------------------------------------------------------------------------------
// Helpers

/// Runs `f` on a classifier thread (16 MiB, no scoped-thread hop per call), re-raising its
/// panic.
fn on_classifier_thread(f: impl FnOnce() + Send + 'static) {
    let h = sql_safety::spawn_classifier_thread("t20-props".to_string(), f).unwrap();
    if let Err(p) = h.join() {
        std::panic::resume_unwind(p);
    }
}

fn ctx(d: DialectId, expect: Expect) -> ClassifyCtx {
    ClassifyCtx::new(DialectCtx::new(d, None), ClientFlavor::None, expect)
}

fn known(d: DialectId, expect: Expect) -> ClassifyCtx {
    ctx(d, expect).with_known_modes(LexModes::default_for(d))
}

/// `pr` cases, scaled by `PROPTEST_CASES` / 1,500 when it is set (the nightly jobs): one
/// knob for every loop, nested ones included.
fn scaled(pr: u32) -> u32 {
    let n = u64::from(pr) * u64::from(cases(1_500)) / 1_500;
    u32::try_from(n.max(1)).unwrap_or(u32::MAX)
}

/// 0 allow, 1 confirm, 2 deny.
fn strictness(v: &Verdict) -> u8 {
    match v {
        Verdict::Allow => 0,
        Verdict::Confirm(_) => 1,
        Verdict::Deny(_) => 2,
    }
}

fn ro(r: &SafetyReport) -> Verdict {
    decide(r, Profile::ReadOnlyPlusExplain, Actor::Human).verdict
}

fn admin(r: &SafetyReport) -> Verdict {
    decide(r, Profile::Admin, Actor::Human).verdict
}

fn codes(r: &SafetyReport) -> Vec<ReasonCode> {
    r.pre_decision
        .iter()
        .flatten()
        .chain(r.items.iter().flat_map(|i| &i.reasons))
        .map(|x| x.code)
        .collect()
}

fn corpus(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../sql-testkit/corpus")
        .join(rel)
}

/// The entries of a read corpus file (`read/*.sql`: leading `--` lines are comments, an entry
/// ends with a line whose last character is `;`).
fn read_entries(file: &str) -> Vec<String> {
    let text = std::fs::read_to_string(corpus("read").join(file)).unwrap();
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
    out
}

#[derive(serde::Deserialize)]
struct Seeds {
    row: Vec<Seed>,
}

#[derive(serde::Deserialize, Clone)]
struct Seed {
    set: String,
    tag: String,
    ro: String,
    admin: String,
    sql: String,
}

fn seeds() -> Vec<Seed> {
    let text = std::fs::read_to_string(corpus("fuzz/classifier_seeds.toml")).unwrap();
    toml::from_str::<Seeds>(&text).unwrap().row
}

/// A seed's context, from its tag (the corpus file's header).
fn seed_ctx(tag: &str) -> ClassifyCtx {
    let mut parts = tag.split(',');
    let d = match parts.next().unwrap() {
        "pg" => DialectId::Postgres,
        "cr" => DialectId::PgWire(PgWireFlavor::Cockroach),
        "rs" => DialectId::PgWire(PgWireFlavor::Redshift),
        "my" => DialectId::MySql,
        "ma" => DialectId::MariaDb,
        "sl" => DialectId::Sqlite,
        "ms" => DialectId::SqlServer,
        "gen" => DialectId::Generic,
        x => panic!("seed dialect {x}"),
    };
    let (mut version, mut modes, mut flavor) = (None, None, ClientFlavor::None);
    let (mut expect, mut sp, mut is_known) = (Expect::SingleStatement, Vec::new(), false);
    for p in parts {
        match p {
            "ansi" => modes = Some(LexModes::from_mysql_sql_mode("ANSI_QUOTES")),
            "nbe" => modes = Some(LexModes::from_mysql_sql_mode("NO_BACKSLASH_ESCAPES")),
            "scsoff" => {
                let mut m = LexModes::default_for(d);
                m.standard_conforming_strings = false;
                modes = Some(m);
            }
            "cli" => flavor = ClientFlavor::MySqlCli,
            "psql" => flavor = ClientFlavor::Psql,
            "script" => expect = Expect::Script,
            "known" => is_known = true,
            s if s.starts_with("sp=") => {
                sp = s[3..].split(';').map(Arc::<str>::from).collect();
            }
            v if v.starts_with('v') => {
                let n: Vec<u16> = v[1..].split('.').map(|x| x.parse().unwrap()).collect();
                version = Some(ServerVersion::new(n[0], n[1], n[2]));
            }
            x => panic!("seed tag {x}"),
        }
    }
    let mut dialect = DialectCtx::new(d, version);
    if let Some(m) = modes {
        dialect.modes = m;
    }
    let mut c = ClassifyCtx::new(dialect, flavor, expect);
    if is_known {
        let m = c.dialect.modes;
        c = c.with_known_modes(m);
    }
    c.search_path = Arc::from(sp);
    c
}

/// A report is well formed: for `sql`, in `d`, pre-decided with no item or decided per item
/// inside the text; a read-only allow is a read or a harmless session change.
fn check_report(sql: &str, d: DialectId, r: &SafetyReport) -> Result<(), String> {
    if r.dialect != Some(d) {
        return Err(format!("dialect {:?}", r.dialect));
    }
    if !r.matches_text(sql) {
        return Err("the report is of other bytes".into());
    }
    if r.pre_decision.is_some() {
        if !r.items.is_empty() || r.request_class != CoarseClass::Unknown {
            return Err("a pre-decided report with items or a class".into());
        }
    } else if r.items.is_empty() {
        return Err("no pre-decision and no item".into());
    }
    for i in &r.items {
        if usize::from(i.range.end()) > sql.len() {
            return Err(format!("item {:?} outside the text", i.range));
        }
    }
    // A harmless `SET` (`SET SCHEMA 'x'`: P-M3b-T3-1) is a `Session` read-only allows.
    if ro(r) == Verdict::Allow && r.request_class > CoarseClass::Session {
        return Err(format!("read-only allows a {:?}", r.request_class));
    }
    Ok(())
}

/// Generated text of at most `max` bytes: script pieces, seed slices, random characters.
fn any_text(r: &mut Rng, seeds: &[Seed], max: usize) -> String {
    let mut s = String::new();
    while s.len() < max {
        match r.below(6) {
            0 if s.len() > 8 => break,
            0 | 1 => s.push_str(r.pick(edits::PIECES)),
            2 => s.push_str(r.pick(edits::INSERTS)),
            3 => {
                let seed = &seeds[r.below(seeds.len())].sql;
                let a = r.below(seed.len() + 1);
                let b = (a + r.below(64)).min(seed.len());
                if let Some(x) = seed.get(a..b) {
                    s.push_str(x);
                }
            }
            4 => {
                const CHARS: &[char] = &[
                    '\0', '\u{1}', '\t', '\u{a0}', '\u{feff}', 'é', 'İ', 'ı', '😀', '；', '\\',
                    '$', '\'', '"', '`', '/', '*', '-', '#', ';', '(', ')', '[', ']', '!',
                ];
                s.push(CHARS[r.below(CHARS.len())]);
            }
            _ => {
                let len = s.len();
                if len > 0 {
                    let a = r.below(len);
                    if let Some(x) = s.get(a..) {
                        let x = x.chars().take(16).collect::<String>();
                        s.push_str(&x);
                    }
                }
            }
        }
    }
    let mut cut = s.len().min(max);
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    s.truncate(cut);
    s
}

const DIALECTS: [DialectId; 5] = [
    DialectId::Postgres,
    DialectId::MySql,
    DialectId::MariaDb,
    DialectId::Sqlite,
    DialectId::PgWire(PgWireFlavor::Redshift),
];

// ---------------------------------------------------------------------------------------------
// classify never panics (fuzz/classify_no_panic)

#[test]
fn classify_never_panics_and_reports_are_well_formed() {
    on_classifier_thread(|| {
        let seeds = seeds();
        let mut failures = Vec::new();
        for case in 0..scaled(48) {
            let mut r = Rng::new(0x7432_3000 + u64::from(case));
            let max = 1 + r.below(4096);
            let text = any_text(&mut r, &seeds, max);
            for d in DIALECTS {
                for expect in [Expect::SingleStatement, Expect::Script] {
                    for c in [ctx(d, expect), known(d, expect)] {
                        let rep = classify(&text, &c);
                        if let Err(e) = check_report(&text, d, &rep) {
                            failures.push(format!("case {case} {d:?} {expect:?}: {e}: {text:?}"));
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    });
}

// ---------------------------------------------------------------------------------------------
// Seeds

#[test]
fn seeds_never_get_weaker() {
    on_classifier_thread(|| {
        let seeds = seeds();
        assert!(seeds.len() >= 1_000, "{} seeds", seeds.len());
        let mut failures = Vec::new();
        for s in &seeds {
            let c = seed_ctx(&s.tag);
            let r = classify(&s.sql, &c);
            let was = |v: &str| match v {
                "allow" => 0,
                "confirm" => 1,
                "deny" => 2,
                x => panic!("verdict {x}"),
            };
            for (profile, now, then) in [
                ("read-only", strictness(&ro(&r)), was(&s.ro)),
                ("admin", strictness(&admin(&r)), was(&s.admin)),
            ] {
                if now < then {
                    failures.push(format!(
                        "{} [{}] {profile} got weaker ({then} → {now}): {:?} {:?}",
                        s.set,
                        s.tag,
                        s.sql,
                        codes(&r)
                    ));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    });
}

// ---------------------------------------------------------------------------------------------
// Meaning-preserving mutations

#[test]
fn meaning_preserving_mutations_never_weaken_a_verdict() {
    on_classifier_thread(|| {
        let seeds: Vec<Seed> = seeds()
            .into_iter()
            // Client flavours read meta-commands whose arguments are not tokens.
            .filter(|s| !s.tag.contains("psql") && !s.tag.contains("cli"))
            .collect();
        let mut failures = Vec::new();
        let per_seed = scaled(3);
        for (k, s) in seeds.iter().enumerate() {
            let c = seed_ctx(&s.tag);
            let f = Family::of(c.dialect.id);
            let base = classify(&s.sql, &c);
            let (base_ro, base_admin) = (strictness(&ro(&base)), strictness(&admin(&base)));
            let mut r = Rng::new(0x5052_4553 ^ (k as u64));
            let mut text = s.sql.clone();
            for _ in 0..per_seed {
                let Some(m) = mutate::preserve(&mut r, &text, f) else {
                    break;
                };
                let rep = classify(&m, &c);
                if strictness(&ro(&rep)) < base_ro || strictness(&admin(&rep)) < base_admin {
                    failures.push(format!(
                        "[{}] {:?} → {:?}: {:?} → {:?}",
                        s.tag,
                        s.sql,
                        m,
                        codes(&base),
                        codes(&rep)
                    ));
                }
                text = m;
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    });
}

// ---------------------------------------------------------------------------------------------
// The PostgreSQL oracle

/// The parse-tree node variants that are or hold a write, a row lock or a statement other than
/// a read-only one.
fn pg_writes(sql: &str) -> Result<Vec<&'static str>, String> {
    const READ_STATEMENTS: &[&str] = &[
        "SelectStmt",
        "ExplainStmt",
        "VariableShowStmt",
        "VariableSetStmt",
        "TransactionStmt",
        "DeclareCursorStmt",
        "FetchStmt",
        "ClosePortalStmt",
        "DeallocateStmt",
    ];
    // A leading byte-order mark: the server strips it or rejects the text; libpg_query reads
    // neither, so it parses the mark as spaces, as the classifier does.
    let blanked = sql
        .strip_prefix('\u{feff}')
        .map(|rest| format!("   {rest}"));
    let counts = pg_backend_hooks::visit_counts(blanked.as_deref().unwrap_or(sql))?;
    Ok(counts
        .keys()
        .copied()
        .filter(|v| {
            (v.ends_with("Stmt") && !READ_STATEMENTS.contains(v))
                || matches!(*v, "IntoClause" | "LockingClause")
        })
        .collect())
}

#[test]
fn pg_mutants_the_read_only_profile_allows_parse_to_no_write() {
    on_classifier_thread(|| {
        let mut donors = read_entries("pg.sql");
        donors.extend(
            seeds()
                .into_iter()
                .filter(|s| s.tag.starts_with("pg") && !s.tag.contains("psql"))
                .map(|s| s.sql),
        );
        let refs: Vec<&str> = donors.iter().map(String::as_str).collect();
        let mut r = Rng::new(0x5047_4f52);
        let (mut allowed, mut findings) = (0usize, Vec::new());
        let n = cases(1_500);
        for case in 0..n {
            let base = refs[r.below(refs.len())];
            let m = mutate::mutate(&mut r, base, Family::Pg, &refs);
            for c in [
                ctx(DialectId::Postgres, Expect::Script),
                known(DialectId::Postgres, Expect::SingleStatement),
            ] {
                let rep = classify(&m, &c);
                if ro(&rep) != Verdict::Allow {
                    continue;
                }
                allowed += 1;
                match pg_writes(&m) {
                    Ok(w) if w.is_empty() => {}
                    Ok(w) => findings.push(format!("case {case}: allowed, but {w:?}: {m:?}")),
                    Err(e) => findings.push(format!("case {case}: allowed, but {e}: {m:?}")),
                }
            }
        }
        eprintln!("pg oracle: {n} mutants, {allowed} read-only allows");
        assert!(allowed > 0, "no mutant was allowed: the oracle saw nothing");
        assert!(findings.is_empty(), "{}", findings.join("\n"));
    });
}

// ---------------------------------------------------------------------------------------------
// Escaped literals (T14's corpus)

#[derive(serde::Deserialize)]
struct EscapedRows {
    row: Vec<EscapedRow>,
}

#[derive(serde::Deserialize)]
struct EscapedRow {
    dialect: String,
    kind: String,
    sql: String,
}

fn escaped() -> Vec<(DialectId, String, String)> {
    let text = std::fs::read_to_string(corpus("read/escaped_literals.toml")).unwrap();
    toml::from_str::<EscapedRows>(&text)
        .unwrap()
        .row
        .into_iter()
        .map(|row| {
            let d = match row.dialect.as_str() {
                "pg" => DialectId::Postgres,
                "my" => DialectId::MySql,
                "ma" => DialectId::MariaDb,
                "sl" => DialectId::Sqlite,
                x => panic!("{x}"),
            };
            (d, row.kind, row.sql)
        })
        .collect()
}

#[test]
fn escaped_literals_and_their_meaning_preserving_mutants_are_never_falsely_denied() {
    on_classifier_thread(|| {
        let rows = escaped();
        assert!(rows.len() >= 20);
        let mut failures = Vec::new();
        for (k, (d, kind, sql)) in rows.iter().enumerate() {
            let mut texts = vec![sql.clone()];
            let mut r = Rng::new(0x4553_4300 + k as u64);
            for _ in 0..scaled(8) {
                let last = texts.last().unwrap().clone();
                match mutate::preserve(&mut r, &last, Family::of(*d)) {
                    Some(m) => texts.push(m),
                    None => break,
                }
            }
            for t in &texts {
                for (path, c) in [
                    ("unknown", ctx(*d, Expect::SingleStatement)),
                    ("known", known(*d, Expect::SingleStatement)),
                ] {
                    let rep = classify(t, &c);
                    if matches!(admin(&rep), Verdict::Deny(_)) {
                        failures.push(format!(
                            "{path} {d:?} Admin denies {t:?}: {:?}",
                            codes(&rep)
                        ));
                    }
                    if kind == "read" && ro(&rep) != Verdict::Allow {
                        failures.push(format!("{path} {d:?} read denied {t:?}: {:?}", codes(&rep)));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    });
}

// ---------------------------------------------------------------------------------------------
// The T10 gap: unqualified information-disclosure tables

fn in_db(d: DialectId, db: Option<&str>) -> ClassifyCtx {
    let mut c = ctx(d, Expect::SingleStatement);
    c.search_path = Arc::from(db.map(Arc::<str>::from).into_iter().collect::<Vec<_>>());
    c
}

#[test]
fn unqualified_threads_and_session_are_disclosure_in_their_schema() {
    on_classifier_thread(|| {
        for d in [DialectId::MySql, DialectId::MariaDb] {
            let denied = [
                ("performance_schema", "SELECT * FROM threads"),
                (
                    "PERFORMANCE_SCHEMA",
                    "SELECT processlist_info FROM `threads`",
                ),
                (
                    "performance_schema",
                    "SELECT 1 FROM t WHERE EXISTS (SELECT 1 FROM THREADS)",
                ),
                ("sys", "SELECT * FROM session"),
                ("sys", "SELECT * FROM `Session` s JOIN t ON s.thd_id = t.id"),
                ("Sys", "WITH x AS (SELECT * FROM session) SELECT * FROM x"),
                ("sys", "SELECT * FROM x$session"),
                (
                    "performance_schema",
                    "SELECT * FROM events_statements_current",
                ),
            ];
            for (db, sql) in denied {
                let r = classify(sql, &in_db(d, Some(db)));
                assert_ne!(ro(&r), Verdict::Allow, "{d:?} in {db}: {sql}");
                assert_eq!(
                    r.request_class,
                    CoarseClass::Unknown,
                    "{d:?} in {db}: {sql}"
                );
            }
            // Another current database, or none known: user tables of those names.
            for db in [Some("shop"), Some("performance_schemas"), None] {
                for sql in ["SELECT * FROM threads", "SELECT * FROM session"] {
                    let r = classify(sql, &in_db(d, db));
                    assert_eq!(ro(&r), Verdict::Allow, "{d:?} in {db:?}: {sql}");
                }
            }
            // The other schema's name is a user table there.
            assert_eq!(
                ro(&classify(
                    "SELECT * FROM session",
                    &in_db(d, Some("performance_schema"))
                )),
                Verdict::Allow
            );
            assert_eq!(
                ro(&classify("SELECT * FROM threads", &in_db(d, Some("sys")))),
                Verdict::Allow
            );
            // Qualified, a call, a column: as before.
            let any = in_db(d, None);
            assert_ne!(
                ro(&classify("SELECT * FROM sys.session", &any)),
                Verdict::Allow
            );
            assert_eq!(
                ro(&classify("SELECT t.session FROM t", &any)),
                Verdict::Allow
            );
        }
    });
}

// ---------------------------------------------------------------------------------------------
// Readings × length (T10 review)

/// A dense MySQL select of about `bytes` bytes with `gates` distinct version gates.
fn gated_select(bytes: usize, gates: usize) -> String {
    let mut s = String::from("SELECT 1");
    for g in 0..gates {
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
    s
}

#[test]
fn readings_times_length_is_bounded() {
    on_classifier_thread(|| {
        let has = |r: &SafetyReport, c: ReasonCode| codes(r).contains(&c);
        let c = known(DialectId::MySql, Expect::SingleStatement);
        // 16 readings of 10 KB (160 KB) fit.
        let fits = gated_select(10_000, 15);
        let r = classify(&fits, &c);
        assert!(!has(&r, ReasonCode::TooComplex), "{:?}", codes(&r));
        // 16 readings of 20 KB (320 KB) do not, and the answer comes before any reading.
        let over = gated_select(20_000, 15);
        let start = Instant::now();
        let r = classify(&over, &c);
        let took = start.elapsed();
        assert!(has(&r, ReasonCode::TooComplex), "{:?}", codes(&r));
        assert_ne!(ro(&r), Verdict::Allow);
        assert!(took.as_secs() < 2, "{took:?}");
        // Two readings of 200 KB do not either.
        let r = classify(&gated_select(200_000, 1), &c);
        assert!(has(&r, ReasonCode::TooComplex), "{:?}", codes(&r));
        // The budget is the request's: two 10 KB statements of 16 readings each (320 KB) pass
        // 256 KiB, though each alone fits.
        let script = known(DialectId::MySql, Expect::Script);
        let r = classify(&format!("SELECT 1;\n{fits}"), &script);
        assert!(!has(&r, ReasonCode::TooComplex), "{:?}", codes(&r));
        let r = classify(&format!("{fits};\n{fits}"), &script);
        assert!(has(&r, ReasonCode::TooComplex), "{:?}", codes(&r));
        // A statement without executable comments costs nothing from the budget.
        let plain = gated_select(200_000, 0);
        let r = classify(&format!("{plain};\n{fits}"), &script);
        assert!(!has(&r, ReasonCode::TooComplex), "{:?}", codes(&r));
    });
}
