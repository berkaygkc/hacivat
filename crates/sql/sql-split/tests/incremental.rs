//! Incremental resplit (SQL-SPLIT-05, R-M3-65; Task 5): `SqlDocument::apply` relexes from the
//! nearest exact state and resplits only the statements an edit touched, and every answer the
//! document gives afterwards equals a document built from the new text: its lexing, both splits
//! (items, uncertain boundaries, client segments, diagnostics), and the snapshot's selection and
//! split-start answers. `StmtKey` is content-addressed (`key.rs`), so the fixtures that count
//! keys hold distinct statements.

#[path = "../../sql-lexer/tests/support/edits.rs"]
mod edits;
#[path = "../../sql-lexer/tests/support/gen_sql.rs"]
mod gen_sql;

use std::time::Instant;

use proptest::prelude::*;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, lex_profile};
use sql_split::{DocDelta, Item, SqlDocument};
use sql_text::{TextEdit, TextRange, TextSize};

// ---- generators --------------------------------------------------------------------------------

/// Lines and pieces of scripts: statements, `$$` bodies, `COPY` data, psql meta-commands, MySQL
/// executable comments and hints, client delimiters, blocks, comments, open quotes.
const PIECES: &[&str] = &[
    "SELECT 1;\n",
    "SELECT a, b FROM t WHERE x = 'y';\n",
    "select 'it''s';\n",
    "INSERT INTO t VALUES (1, 'a;b');\n",
    "UPDATE t SET a = 1\nWHERE b = 2;\n",
    "SELECT 1\nSELECT 2\n",
    "WITH x AS (SELECT 1) SELECT * FROM x;\n",
    "-- a note\n",
    "/* block\n   comment */\n",
    "\n",
    "\n\n",
    "\r\n",
    "  ",
    "CREATE FUNCTION f() RETURNS int AS $$ SELECT 1; $$ LANGUAGE sql;\n",
    "CREATE FUNCTION g() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1; SELECT 2; END;\n",
    "DO $body$\nBEGIN\n  PERFORM 1;\nEND\n$body$;\n",
    "$$",
    "COPY t FROM STDIN;\n1\tdon't\n\\.\n",
    "COPY t FROM STDIN; SELECT 1;\n",
    "\\.\n",
    "\\set x 'a\n",
    "\\echo don't\n",
    "\\if :x\n",
    "\\endif\n",
    "SELECT 1 \\g\n",
    "SELECT 1 \\gset p_\n",
    "\\copy t from stdin\nrow\n\\.\n",
    "DELIMITER //\n",
    "DELIMITER ;\n",
    "CREATE PROCEDURE p() BEGIN SELECT 1; END//\n",
    "CREATE PROCEDURE q() BEGIN IF x THEN SELECT 1; END IF; END;\n",
    "CREATE TRIGGER tr AFTER INSERT ON a BEGIN UPDATE b SET c = 1; END;\n",
    "/*!50000 SELECT x'\\' */;\n",
    "/*!40101 SET @x = 1 */;\n",
    "/*! , \"*/ x; -- \" */\n",
    "/*+ SET_VAR(a=1) */",
    "/*M! SELECT 1 */;\n",
    "SELECT E'a\\'b';\n",
    "SELECT \"q;\" FROM `t;`;\n",
    "# hash comment\n",
    "source f.sql\n",
    ".mode csv\n",
    "BEGIN;\n",
    "END;\n",
    "END\n",
    "'",
    "\"",
    "/*",
    "*/",
    "--",
    "\u{feff}",
    "İ",
    ";",
    "\\",
];

fn sql_doc() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(PIECES), 0..32).prop_map(|v| v.concat())
}

const INSERTS: &[&str] = &[
    "",
    "x",
    " ",
    "\n",
    "\r",
    "\r\n",
    "'",
    "\"",
    "`",
    "$",
    "$$",
    ";",
    "-",
    "--",
    "/*",
    "*/",
    "\\",
    "\\.",
    "!",
    "#",
    "\u{feff}",
    "İ",
    "SELECT 2;\n",
    "END",
    "BEGIN",
    "IF",
    "THEN",
    "DELIMITER $$\n",
    "DELIMITER ;\n",
    "\\set y 1\n",
    "\\if 1\n",
    "COPY t FROM STDIN;\n",
    "/*!",
    "/*+",
    "-- c\n",
    "\n\n",
    "(",
    ")",
    "CREATE PROCEDURE r() ",
];

fn edit_in(doc: &str) -> impl Strategy<Value = TextEdit> + use<> {
    let doc = doc.to_owned();
    (
        any::<prop::sample::Index>(),
        prop_oneof![4 => 0usize..3, 2 => 0usize..24, 1 => 0usize..2048],
        prop::collection::vec(prop::sample::select(INSERTS), 0..3),
    )
        .prop_map(move |(at, del, ins)| {
            let a = floor(&doc, at.index(doc.len() + 1));
            let b = floor(&doc, (a + del).min(doc.len()));
            edit(a, b, &ins.concat())
        })
}

fn setups() -> impl Strategy<Value = (DialectId, ClientFlavor)> {
    prop::sample::select(vec![
        (DialectId::Postgres, ClientFlavor::Psql),
        (DialectId::Postgres, ClientFlavor::None),
        (DialectId::MySql, ClientFlavor::MySqlCli),
        (DialectId::MySql, ClientFlavor::None),
        (DialectId::MariaDb, ClientFlavor::MySqlCli),
        (DialectId::Sqlite, ClientFlavor::Sqlite3),
        (DialectId::Sqlite, ClientFlavor::None),
        (DialectId::Generic, ClientFlavor::None),
    ])
}

fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000)
}

// ---- helpers -----------------------------------------------------------------------------------

fn floor(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn at(n: usize) -> TextSize {
    TextSize::from(u32::try_from(n).unwrap())
}

fn edit(a: usize, b: usize, insert: &str) -> TextEdit {
    TextEdit {
        delete: TextRange::new(at(a), at(b)),
        insert: insert.to_owned(),
    }
}

/// Everything `d` answers equals what `fresh` (built from the same text) answers.
fn same_as_fresh(d: &SqlDocument, probes: &[usize]) -> Result<(), String> {
    let fresh = SqlDocument::new(d.text().to_owned(), ctx_of(d), flavor_of(d));
    if d.lexed() != fresh.lexed() {
        return Err("the lexing differs".into());
    }
    // Equal lexings may keep different checkpoints; every one must still hold (fix round 1).
    let ctx = ctx_of(d);
    let lp = lex_profile(&ctx)
        .or_else(|_| lex_profile(&DialectCtx::new(DialectId::Generic, None)))
        .map_err(|e| format!("{e:?}"))?;
    edits::checkpoints_hold(d.text(), d.lexed(), lp, ctx.modes)?;
    if d.statements() != fresh.statements() {
        return Err(format!(
            "the editor items differ:\n{:#?}\n{:#?}",
            d.statements(),
            fresh.statements()
        ));
    }
    let (a, b) = (d.snapshot(), fresh.snapshot());
    if a.editor_split() != b.editor_split() {
        return Err(format!(
            "the editor split differs:\n{:#?}\n{:#?}",
            a.editor_split(),
            b.editor_split()
        ));
    }
    if a.execution_split() != b.execution_split() {
        return Err(format!(
            "the execution split differs:\n{:#?}\n{:#?}",
            a.execution_split(),
            b.execution_split()
        ));
    }
    let n = d.text().len();
    for (i, p) in probes.iter().enumerate() {
        let x = floor(d.text(), p % (n + 1));
        if a.split_start_at(at(x)) != b.split_start_at(at(x)) {
            return Err(format!("split_start_at({x}) differs"));
        }
        let y = floor(d.text(), probes[(i + 1) % probes.len()] % (n + 1));
        let r = TextRange::new(at(x.min(y)), at(x.max(y)));
        if a.selection_cuts(r) != b.selection_cuts(r) {
            return Err(format!("selection_cuts({r:?}) differs"));
        }
    }
    Ok(())
}

fn ctx_of(d: &SqlDocument) -> DialectCtx {
    *d.snapshot().ctx()
}

fn flavor_of(d: &SqlDocument) -> ClientFlavor {
    d.snapshot().flavor()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), ..ProptestConfig::default() })]

    #[test]
    fn incremental_resplit_equals_full_split(
        (doc, edits) in sql_doc().prop_flat_map(|d| {
            // A run of up to four edits, each against the text the one before left.
            let first = edit_in(&d);
            (Just(d), first, prop::collection::vec(
                (any::<prop::sample::Index>(), 0usize..6, prop::sample::select(INSERTS)), 0..4))
        }).prop_map(|(d, first, rest)| (d, (first, rest))),
        (id, flavor) in setups(),
        warm in any::<bool>(),
        probes in prop::collection::vec(any::<usize>(), 4),
    ) {
        let (first, rest) = edits;
        let mut d = SqlDocument::new(doc, DialectCtx::new(id, None), flavor);
        if warm {
            // The worker computes the execution split before the next edit arrives.
            let _ = d.snapshot().execution_split();
        }
        d.apply(std::slice::from_ref(&first)).map_err(|e| TestCaseError::fail(format!("{e:?}")))?;
        same_as_fresh(&d, &probes).map_err(TestCaseError::fail)?;
        for (pos, del, ins) in rest {
            let t = d.text();
            let a = floor(t, pos.index(t.len() + 1));
            let b = floor(t, (a + del).min(t.len()));
            if warm {
                let _ = d.snapshot().execution_split();
            }
            d.apply(&[edit(a, b, ins)]).map_err(|e| TestCaseError::fail(format!("{e:?}")))?;
            same_as_fresh(&d, &probes).map_err(TestCaseError::fail)?;
        }
    }
}

// ---- the plan's cases --------------------------------------------------------------------------

fn pg() -> DialectCtx {
    DialectCtx::new(DialectId::Postgres, None)
}

/// `n` distinct statements, one per line. Each starts with `(`, so deleting a `;` leaves no
/// statement-start word at a line start to keep the two apart heuristically.
fn statements(n: usize) -> (String, Vec<usize>) {
    let mut text = String::new();
    let mut semicolons = Vec::with_capacity(n);
    for i in 0..n {
        text.push_str(&format!("(SELECT c{i} FROM t{i})"));
        semicolons.push(text.len());
        text.push_str(";\n");
    }
    (text, semicolons)
}

/// Types `x` into statement `k` of `n`.
fn apply_typing(n: usize, k: usize) -> (SqlDocument, DocDelta) {
    let (text, semis) = statements(n);
    let mut d = SqlDocument::new(text, pg(), ClientFlavor::Psql);
    let a = semis[k] - 1; // before its `)`
    let delta = d.apply(&[edit(a, a, "x")]).unwrap();
    same_as_fresh(&d, &[0, a, a + 7, semis[n - 1]]).unwrap();
    (d, delta)
}

/// Deletes the `;` of statement `k` of `n`.
fn delete_semicolon(n: usize, k: usize) -> DocDelta {
    let (text, semis) = statements(n);
    let mut d = SqlDocument::new(text, pg(), ClientFlavor::Psql);
    let delta = d.apply(&[edit(semis[k], semis[k] + 1, "")]).unwrap();
    same_as_fresh(&d, &[0, semis[k], semis[k] + 3]).unwrap();
    assert_eq!(
        d.statements().iter().filter_map(Item::statement).count(),
        n - 1
    );
    delta
}

#[test]
fn typing_in_one_of_1000_statements_changes_one_key() {
    let (d, delta) = apply_typing(1000, 500);
    assert_eq!(delta.changed.len(), 1);
    assert_eq!(delta.moved.len(), 499);
    assert_eq!(delta.removed.len(), 1);
    assert!(d.last_apply_was_incremental());
}

#[test]
fn deleting_a_semicolon_removes_two_and_changes_one() {
    let d = delete_semicolon(1000, 500);
    assert_eq!(d.removed.len(), 2);
    assert_eq!(d.changed.len(), 1);
}

#[test]
fn crlf_split_by_an_edit_stays_one_break() {
    // An edit that joins `\r` and `\n` makes one line break, as a fresh lexing reads it.
    let mut d = SqlDocument::new(
        "SELECT 1;\rX\nSELECT 2;\r\n".into(),
        pg(),
        ClientFlavor::None,
    );
    d.apply(&[edit(10, 11, "")]).unwrap();
    assert_eq!(d.text(), "SELECT 1;\r\nSELECT 2;\r\n");
    same_as_fresh(&d, &[0, 9, 10, 11, 12]).unwrap();
    let l = d.lexed();
    let nl = l.token_at(at(9)).unwrap();
    assert_eq!(l.range(nl), TextRange::new(at(9), at(11)));
    // And one typed between them splits it in two, again as a fresh lexing reads it.
    d.apply(&[edit(10, 10, "\n")]).unwrap();
    same_as_fresh(&d, &[0, 9, 10, 11, 12]).unwrap();
    assert_eq!(d.statements().iter().filter_map(Item::statement).count(), 2);
}

#[test]
fn multi_range_edit_batches_apply_in_order() {
    let (text, semis) = statements(50);
    let mut d = SqlDocument::new(text.clone(), pg(), ClientFlavor::Psql);
    // Each edit is against the text the one before left: the second offset is after the first
    // insertion, the third deletes across the second.
    let batch = [
        edit(semis[3], semis[3], " AND 1"),
        edit(semis[40] + 6, semis[40] + 6, "-- moved\n"),
        edit(semis[10], semis[12] + 6, ""),
    ];
    let mut want = text;
    for e in &batch {
        e.apply(&mut want).unwrap();
    }
    d.apply(&batch).unwrap();
    assert_eq!(d.text(), want);
    same_as_fresh(&d, &[0, semis[3], semis[10], semis[40]]).unwrap();
}

// ---- fix round 1 (C1, I1) ----------------------------------------------------------------------

/// The setups of the seeded runs, by index.
fn setup_at(i: usize) -> (DialectId, ClientFlavor) {
    let all = [
        (DialectId::Postgres, ClientFlavor::Psql),
        (DialectId::Postgres, ClientFlavor::None),
        (DialectId::MySql, ClientFlavor::MySqlCli),
        (DialectId::MySql, ClientFlavor::None),
        (DialectId::MariaDb, ClientFlavor::MySqlCli),
        (DialectId::Sqlite, ClientFlavor::Sqlite3),
        (DialectId::Sqlite, ClientFlavor::None),
        (DialectId::Generic, ClientFlavor::None),
    ];
    all[i % all.len()]
}

/// One seeded run: a script (up to 16 KiB, or with more statements on one line than
/// `ANCHOR_SCAN`), then 1–8 edits biased to start or end at checkpoints, line starts and
/// statement starts (anchors); after each, the document answers as a fresh one and every
/// checkpoint holds. Returns the paths the edits took.
fn resplit_run(seed: u64) -> Result<Vec<sql_split::incremental::ApplyPath>, String> {
    let mut r = edits::Rng::new(seed);
    let (id, flavor) = setup_at(r.below(8));
    let text = edits::doc(&mut r);
    let mut d = SqlDocument::new(text, DialectCtx::new(id, None), flavor);
    let mut paths = Vec::new();
    for step in 0..1 + r.below(8) {
        if r.below(2) == 0 {
            // The worker computes the execution split before the next edit arrives.
            let _ = d.snapshot().execution_split();
        }
        let mut marks: Vec<usize> = d
            .lexed()
            .checkpoints()
            .iter()
            .map(|c| usize::from(*c))
            .collect();
        marks.extend(
            d.statements()
                .iter()
                .filter_map(Item::statement)
                .map(|s| usize::from(s.smart.start())),
        );
        let e = edits::edit(&mut r, d.text(), &marks);
        d.apply(std::slice::from_ref(&e))
            .map_err(|x| format!("seed {seed}: {x:?}"))?;
        paths.push(d.last_apply_path());
        let probes = [r.next() as usize, r.next() as usize, r.next() as usize];
        same_as_fresh(&d, &probes).map_err(|x| format!("seed {seed}, edit {step} ({e:?}): {x}"))?;
    }
    Ok(paths)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: edits::cases(10_000), ..ProptestConfig::default() })]

    /// Runs of edits over scripts of every size (fix round 1, I1); 10,000 cases in PR (each is up
    /// to eight edits on up to 16 KiB, each compared with a fresh build), `PROPTEST_CASES`
    /// otherwise.
    #[test]
    fn incremental_resplit_over_edit_runs(seed in any::<u64>()) {
        resplit_run(seed).map_err(TestCaseError::fail)?;
    }
}

#[test]
fn the_edit_runs_reach_every_path() {
    // The window, a later window, the window to the end, the whole pass, and a window start
    // that fell back to the text's start after `ANCHOR_SCAN` items without an anchor.
    let mut seen = std::collections::BTreeMap::new();
    for seed in 0..400 {
        for p in resplit_run(seed).unwrap() {
            *seen.entry(p.how).or_insert(0) += 1;
            if p.scan_exhausted {
                *seen.entry("scan-exhausted").or_insert(0) += 1;
            }
        }
    }
    for path in ["window", "retried", "to-end", "whole", "scan-exhausted"] {
        assert!(seen.contains_key(path), "{path} never taken: {seen:?}");
    }
}

#[test]
fn a_checkpoint_after_a_joined_line_is_dropped() {
    // The reviewer's repros (C1): the second line starts at 2048, a lexer checkpoint. Join it to
    // the line before (delete the line feed), then type what makes the token before the old
    // checkpoint look past it. The MySQL case with a stray `END` takes the whole pass, which
    // used to run over the wrong relexed tokens: a split with statements the server reads as a
    // comment.
    let cases = [
        (
            DialectId::Postgres,
            ClientFlavor::Psql,
            "SELECT U",
            "&x FROM t;",
            1,
            "'",
        ),
        (
            DialectId::Postgres,
            ClientFlavor::Psql,
            "SELECT $",
            "fn x; SELECT 4;",
            3,
            "$",
        ),
        (
            DialectId::MySql,
            ClientFlavor::MySqlCli,
            "SELECT 1 -",
            "-x; SELECT 5;",
            1,
            " ",
        ),
        (
            DialectId::MySql,
            ClientFlavor::None,
            "SELECT 1 -",
            "-x; SELECT 5; END;",
            1,
            " ",
        ),
    ];
    for (id, flavor, prev, line, offset, typed) in cases {
        let mut text = String::new();
        while text.len() < 2048 - 40 {
            text.push_str("SELECT 1;\n");
        }
        text.push_str(&" ".repeat(2048 - text.len() - prev.len() - 1));
        text.push_str(prev);
        text.push('\n');
        let c = text.len();
        text.push_str(line);
        text.push_str("\nSELECT 2;\nSELECT 3;\n");
        let mut d = SqlDocument::new(text, DialectCtx::new(id, None), flavor);
        assert!(d.lexed().checkpoints().contains(&at(c)), "{line}");
        for e in [
            edit(c - 1, c, ""),
            edit(c - 1 + offset, c - 1 + offset, typed),
        ] {
            let _ = d.snapshot().execution_split();
            d.apply(std::slice::from_ref(&e)).unwrap();
            same_as_fresh(&d, &[0, c, c + 4, d.text().len()])
                .unwrap_or_else(|x| panic!("{line}: {x}"));
        }
    }
}

/// The cases the equivalence property found while the fast path was built: each must equal a
/// fresh document (the fast path declines them or handles them).
#[test]
fn cases_the_property_found() {
    let check = |doc: &str, id: DialectId, flavor: ClientFlavor, edits: &[TextEdit]| {
        let mut d = SqlDocument::new(doc.into(), DialectCtx::new(id, None), flavor);
        let _ = d.snapshot().execution_split();
        for e in edits {
            d.apply(std::slice::from_ref(e)).unwrap();
            same_as_fresh(&d, &[0, 9, 27, 360]).unwrap();
        }
    };
    // A byte-order mark typed at the window's start reads as white space there, as text in the
    // document.
    check(
        "INSERT INTO t VALUES (1, 'a;b');\nsource f.sql\n\\.\n\\echo don't\n",
        DialectId::Postgres,
        ClientFlavor::Psql,
        &[edit(33, 33, "\u{feff}")],
    );
    // A `\copy … from stdin` whose `\.` line an edit breaks runs past the window's end.
    check(
        "\\copy t from stdin\nrow\n\\.\n",
        DialectId::Postgres,
        ClientFlavor::Psql,
        &[edit(26, 26, "'"), edit(23, 23, "'")],
    );
    // A statement that starts with a byte-order mark (not at offset 0) and runs into an
    // unterminated hint: the stability rule ended it inside the mark (M3a: a fresh build
    // panicked too; fix round 1).
    check(
        "\u{feff}/*+ SET_VAR(a=1) */",
        DialectId::MariaDb,
        ClientFlavor::MySqlCli,
        &[
            edit(6, 6, "DELIMITER $$\n"),
            edit(0, 0, " "),
            edit(30, 35, "/*!"),
        ],
    );
    // A byte-order mark right after a mysql `\g`: the pass restarts there.
    check(
        "SELECT 1 \\gset p_\nSELECT 1 \\gset p_\nEND;\nINSERT INTO t VALUES (1);\n",
        DialectId::MariaDb,
        ClientFlavor::MySqlCli,
        &[edit(29, 29, "\u{feff}")],
    );
}

#[test]
fn typing_through_the_gen_sql_fixture_takes_the_incremental_path() {
    // Ordinary typing in a real script: every result equals a fresh build, and nearly every
    // edit is resplit locally.
    let text = gen_sql::gen_sql(2_000);
    let mut d = SqlDocument::new(text, pg(), ClientFlavor::Psql);
    let mut rng = gen_sql::Positions::new(11);
    let (mut fast, total) = (0, 300);
    for i in 0..total {
        let t = d.text();
        let a = floor(t, rng.below(t.len()));
        let e = match i % 3 {
            0 => edit(a, a, "x"),
            1 => edit(a, a, " "),
            _ => edit(a, floor(t, a + 1).max(a), ""),
        };
        let _ = d.snapshot().execution_split();
        d.apply(&[e]).unwrap();
        fast += usize::from(d.last_apply_was_incremental());
        if i % 25 == 0 {
            same_as_fresh(&d, &[a, a + 100, 0]).unwrap();
        }
    }
    same_as_fresh(&d, &[0, 1, 2]).unwrap();
    assert!(
        fast * 10 >= total * 9,
        "{fast} of {total} edits resplit locally"
    );
}

// ---- performance (R-M3-9; P9: `HACIVAT_PERF=1`, release) ------------------------------------

#[test]
#[ignore = "perf"]
fn relex_typing_edit_p95_under_2ms() {
    if !gen_sql::perf_enabled() {
        return;
    }
    // R-M3-9: relex + resplit (`SqlDocument::apply`) on the gen-sql 50k-line fixture: 1,000
    // single-character edits at random lines (a character typed, or one deleted), each on the
    // fixture (it is undone, unmeasured, before the next: a random walk of deletions would
    // leave a quote open for hundreds of edits, which typing does not), with the previous
    // revision's snapshot held (as the worker's published analysis holds it) and its execution
    // split computed.
    let text = gen_sql::gen_sql(50_000);
    let mut d = SqlDocument::new(text.clone(), pg(), ClientFlavor::Psql);
    let mut rng = gen_sql::Positions::new(7);
    let mut times = Vec::with_capacity(1_000);
    let mut fast = 0;
    for i in 0..1_000 {
        let held = d.snapshot();
        let _ = held.execution_split();
        let a = floor(&text, rng.below(text.len()));
        let (e, undo) = if i % 2 == 0 {
            (edit(a, a, "x"), edit(a, a + 1, ""))
        } else {
            let b = (a + 1..=text.len())
                .find(|i| text.is_char_boundary(*i))
                .unwrap_or(a);
            (edit(a, b, ""), edit(a, a, &text[a..b]))
        };
        let start = Instant::now();
        d.apply(&[e]).unwrap();
        times.push(start.elapsed());
        fast += usize::from(d.last_apply_was_incremental());
        drop(held);
        d.apply(&[undo]).unwrap();
    }
    assert_eq!(d.text(), text);
    same_as_fresh(&d, &[0, 1_000_000]).unwrap();
    let p95 = gen_sql::p95(&mut times);
    println!("relex + resplit p95 over 1,000 typing edits on 50k lines: {p95:?} ({fast} local)");
    assert!(p95.as_micros() <= 2_000, "{p95:?}");
}

/// Types `phrase` character by character at `at`, then erases it, timing each `apply` with the
/// previous snapshot held (the worker's case); the document is then the text it started as.
fn type_and_erase(d: &mut SqlDocument, at: usize, phrase: &str) -> Vec<std::time::Duration> {
    let mut times = Vec::new();
    let mut pos = at;
    for ch in phrase.chars() {
        let held = d.snapshot();
        let _ = held.execution_split();
        let start = Instant::now();
        d.apply(&[edit(pos, pos, ch.encode_utf8(&mut [0; 4]))])
            .unwrap();
        times.push(start.elapsed());
        pos += ch.len_utf8();
    }
    for ch in phrase.chars().rev() {
        let held = d.snapshot();
        let start = Instant::now();
        d.apply(&[edit(pos - ch.len_utf8(), pos, "")]).unwrap();
        times.push(start.elapsed());
        pos -= ch.len_utf8();
        drop(held);
    }
    times
}

fn figure(times: &mut [std::time::Duration]) -> String {
    times.sort_unstable();
    format!(
        "p50 {:?} p95 {:?} max {:?}",
        times[times.len() / 2],
        times[(times.len() * 95 / 100).min(times.len() - 1)],
        times[times.len() - 1]
    )
}

#[test]
#[ignore = "perf"]
fn typing_figures_inside_an_open_string_and_on_a_long_line() {
    if !gen_sql::perf_enabled() {
        return;
    }
    // Recorded for T30/T44 next to the R-M3-9 figure (fix round 1, M3), not budgeted by it:
    // typing B3's phrase, whose `'İzmir'` is a string opened mid-file (every keystroke until
    // the closing quote leaves the rest of the file inside a string: the whole pass, O(tail)),
    // and typing on a long line (no line starts there: the relex and the window cover the line).
    const PHRASE: &str = " AND c.city = 'İzmir' OR o.total > 4200";
    let fixture = gen_sql::gen_sql(50_000);
    let line = fixture
        .match_indices('\n')
        .nth(24_999)
        .map_or(fixture.len(), |(i, _)| i);
    let mut d = SqlDocument::new(fixture.clone(), pg(), ClientFlavor::Psql);
    let mut burst = Vec::new();
    for _ in 0..3 {
        burst.extend(type_and_erase(&mut d, line, PHRASE));
    }
    assert_eq!(d.text(), fixture);
    println!(
        "typing B3's phrase at the end of line 25,000 of 50k lines (string opened mid-file): {}",
        figure(&mut burst)
    );
    // The T1 fixture: 50k lines and a 100,000-character minified line, typed into its middle.
    let long = format!("{fixture}{}\n", gen_sql::minified_line(100_000));
    let mid = fixture.len() + 50_000;
    let mid = (0..=mid).rev().find(|i| long.is_char_boundary(*i)).unwrap();
    let mut d = SqlDocument::new(long.clone(), pg(), ClientFlavor::Psql);
    let mut times = type_and_erase(&mut d, mid, "x AND y");
    assert_eq!(d.text(), long);
    println!(
        "typing in the middle of a 100,000-character line after 50k lines: {}",
        figure(&mut times)
    );
    // A 1 MB script on one line (a mysqldump extended INSERT is like this).
    let one = gen_sql::minified_line(1 << 20);
    let mut d = SqlDocument::new(one.clone(), pg(), ClientFlavor::Psql);
    let mut times = type_and_erase(&mut d, one.len() / 2 - 7, "x AND y");
    println!(
        "typing in the middle of a 1 MiB one-line script: {}",
        figure(&mut times)
    );
    same_as_fresh(&d, &[0, one.len() / 2]).unwrap();
}
