//! Caret and selection to execution units (Task 13; SQL-SPLIT-04 aligned to D-011, R-M3-8,
//! R-M3-24): the D-011 defaults, the chooser's candidates and dedupe rule, the caret-at-boundary
//! rules, strict-range sending, and every refusal.

use sql_dialect::{ClientFlavor, DialectCtx, DialectId};
use sql_split::exec_range::{
    Candidate, CandidateKind, CaretOrSelection, ExecPolicy, InsidePolicy, LABEL_MAX_CHARS,
    OutsidePolicy, RangeResolution, SelectionPolicy, SubqueryOracle, UnitKind, UnitSpan, resolve,
};
use sql_split::{AmbiguityReason, DocSnapshot, SplitMode, SqlDocument, Stmt};
use sql_testkit::split::fixture;
use sql_text::{TextRange, TextSize};

const PROC_WITH_DELIMITER: &str = "DELIMITER //\nCREATE PROCEDURE p()\nBEGIN\n  SELECT 1;\n  SELECT 2;\nEND//\nDELIMITER ;\nCALL p();\n";
const PROC_BODY: &str = "CREATE PROCEDURE p()\nBEGIN\n  SELECT 1;\n  SELECT 2;\nEND";

fn doc_in(sql: &str, id: DialectId, flavor: ClientFlavor) -> DocSnapshot {
    SqlDocument::new(sql.to_owned(), DialectCtx::new(id, None), flavor).snapshot()
}

fn pg(sql: &str) -> DocSnapshot {
    doc_in(sql, DialectId::Postgres, ClientFlavor::Psql)
}

fn mysql(sql: &str) -> DocSnapshot {
    doc_in(sql, DialectId::MySql, ClientFlavor::MySqlCli)
}

fn at(n: usize) -> TextSize {
    TextSize::try_from(n).unwrap()
}

fn caret(n: usize) -> [CaretOrSelection; 1] {
    [CaretOrSelection::Caret(at(n))]
}

fn sel(r: std::ops::Range<usize>) -> [CaretOrSelection; 1] {
    [CaretOrSelection::Selection(TextRange::new(
        at(r.start),
        at(r.end),
    ))]
}

/// The offset right after the first `needle` in `sql`.
fn after(sql: &str, needle: &str) -> usize {
    sql.find(needle)
        .unwrap_or_else(|| panic!("{needle:?} is not in the text"))
        + needle.len()
}

/// The range of the first `needle` in `sql`.
fn span(sql: &str, needle: &str) -> std::ops::Range<usize> {
    let end = after(sql, needle);
    end - needle.len()..end
}

fn exec(doc: &DocSnapshot, at: &[CaretOrSelection], policy: &ExecPolicy) -> RangeResolution {
    resolve(doc, at, policy, SplitMode::Execution, None, None)
}

fn editor(doc: &DocSnapshot, at: &[CaretOrSelection], policy: &ExecPolicy) -> RangeResolution {
    resolve(doc, at, policy, SplitMode::Editor, None, None)
}

/// Run (D-011's defaults) in execution mode with the caret right after `needle`.
fn resolve_at(sql: &str, needle: &str) -> RangeResolution {
    exec(&pg(sql), &caret(after(sql, needle)), &ExecPolicy::default())
}

fn resolve_caret(sql: &str, offset: usize) -> RangeResolution {
    exec(&pg(sql), &caret(offset), &ExecPolicy::default())
}

fn resolve_sel(sql: &str, r: std::ops::Range<usize>) -> RangeResolution {
    exec(&pg(sql), &sel(r), &ExecPolicy::default())
}

fn resolve_mysql_at(sql: &str, needle: &str) -> RangeResolution {
    exec(
        &mysql(sql),
        &caret(after(sql, needle)),
        &ExecPolicy::default(),
    )
}

fn resolve_mysql_exec(sql: &str, offset: usize) -> RangeResolution {
    exec(&mysql(sql), &caret(offset), &ExecPolicy::default())
}

fn inside(p: InsidePolicy) -> ExecPolicy {
    ExecPolicy {
        inside: p,
        ..ExecPolicy::default()
    }
}

fn outside(p: OutsidePolicy) -> ExecPolicy {
    ExecPolicy {
        outside: p,
        ..ExecPolicy::default()
    }
}

fn selecting(p: SelectionPolicy) -> ExecPolicy {
    ExecPolicy {
        selection: p,
        ..ExecPolicy::default()
    }
}

fn units_of(r: &RangeResolution) -> &[UnitSpan] {
    match r {
        RangeResolution::Units(u) => u,
        other => panic!("expected units, got {other:?}"),
    }
}

fn texts<'a>(sql: &'a str, r: &RangeResolution) -> Vec<&'a str> {
    units_of(r).iter().map(|u| &sql[u.range]).collect()
}

fn candidates_of(r: &RangeResolution) -> (&[Candidate], usize) {
    match r {
        RangeResolution::Ask {
            candidates,
            preselect,
        } => (candidates, *preselect),
        other => panic!("expected a chooser, got {other:?}"),
    }
}

fn kinds(r: &RangeResolution) -> Vec<CandidateKind> {
    candidates_of(r).0.iter().map(|c| c.kind).collect()
}

fn candidate_texts<'a>(sql: &'a str, c: &Candidate) -> Vec<&'a str> {
    c.units.iter().map(|u| &sql[u.range]).collect()
}

fn reasons(r: &RangeResolution) -> Vec<AmbiguityReason> {
    match r {
        RangeResolution::Ambiguous(b) => {
            assert!(!b.is_empty(), "an ambiguous answer names its boundaries");
            b.iter().map(|a| a.reason).collect()
        }
        other => panic!("expected ambiguous, got {other:?}"),
    }
}

// --- the plan's cases ----------------------------------------------------------------------

#[test]
fn choose_01_offers_statement_and_whole_script() {
    let sql = fixture("FX-CARET-01");
    let r = resolve_at(&sql, "SELECT id FROM recent r");
    assert_eq!(
        kinds(&r),
        [CandidateKind::Statement, CandidateKind::WholeScript]
    );
    let (candidates, preselect) = candidates_of(&r);
    assert_eq!(candidates[preselect].kind, CandidateKind::Statement);
    assert_eq!(candidates[0].units.len(), 1);
    assert!(sql[candidates[0].units[0].range].starts_with("WITH recent AS ("));
    assert!(sql[candidates[0].units[0].range].ends_with("FROM customers c"));
    assert_eq!(candidate_texts(&sql, &candidates[1])[1], "SELECT 2");
}

#[test]
fn one_candidate_runs_directly() {
    assert!(matches!(
        resolve_at("SELECT 1", "SELECT"),
        RangeResolution::Units(u) if u.len() == 1
    ));
    // With a terminator and trailing white space too: still one statement, one candidate.
    let sql = "SELECT 1;\n";
    assert_eq!(texts(sql, &resolve_at(sql, "SELECT")), ["SELECT 1"]);
}

#[test]
fn from_caret_equal_to_the_whole_script_is_labelled_whole_script() {
    let sql = "SELECT 1;\nSELECT 2;\nSELECT 3;";
    // In the first statement "from the caret" is the whole script: one entry, WholeScript.
    let first = resolve_at(sql, "SELECT 1");
    assert_eq!(
        kinds(&first),
        [CandidateKind::Statement, CandidateKind::WholeScript]
    );
    assert_eq!(
        candidate_texts(sql, &candidates_of(&first).0[1]),
        ["SELECT 1", "SELECT 2", "SELECT 3"]
    );
    // In the middle it is a strict subset, and offered.
    let middle = resolve_at(sql, "SELECT 2");
    assert_eq!(
        kinds(&middle),
        [
            CandidateKind::Statement,
            CandidateKind::FromCaret,
            CandidateKind::WholeScript
        ]
    );
    let (c, _) = candidates_of(&middle);
    assert_eq!(candidate_texts(sql, &c[0]), ["SELECT 2"]);
    assert_eq!(candidate_texts(sql, &c[1]), ["SELECT 2", "SELECT 3"]);
    assert_eq!(c[2].units.len(), 3);
    // In the last statement it is the statement: the earlier kind stays.
    let last = resolve_at(sql, "SELECT 3");
    assert_eq!(
        kinds(&last),
        [CandidateKind::Statement, CandidateKind::WholeScript]
    );
    // No list ever names the same ranges twice.
    for r in [&first, &middle, &last] {
        let (c, _) = candidates_of(r);
        for (i, a) in c.iter().enumerate() {
            for b in &c[i + 1..] {
                let ranges = |x: &Candidate| x.units.iter().map(|u| u.range).collect::<Vec<_>>();
                assert_ne!(ranges(a), ranges(b));
            }
        }
    }
}

#[test]
fn caret_outside_statements_runs_nothing() {
    assert!(matches!(
        resolve_caret("SELECT 1;\n\n\nSELECT 2;", 11),
        RangeResolution::NothingToRun
    ));
}

#[test]
fn selection_runs_as_separate_statements() {
    let sql = "SELECT 1; SELECT 2;";
    assert_eq!(
        texts(sql, &resolve_sel(sql, 0..19)),
        ["SELECT 1", "SELECT 2"]
    );
}

#[test]
fn selection_starting_inside_a_dollar_quote_carries_the_state() {
    let sql = "DO $$ BEGIN PERFORM 1; PERFORM 2; END $$; SELECT 3; SELECT 4;";
    // From inside the body to the end: the body's `;` are not boundaries (the state at the
    // selection's start is "inside `$$`"), so no unit is `PERFORM 2`. The body's tail is no
    // statement the server would read the same way: nothing runs.
    let from_body = after(sql, "PERFORM 1;")..sql.len();
    let r = resolve_sel(sql, from_body.clone());
    assert_eq!(reasons(&r), [AmbiguityReason::UnterminatedConstruct]);
    let RangeResolution::Ambiguous(b) = &r else {
        unreachable!()
    };
    assert_eq!(sql[b[0].at].trim(), "PERFORM 2; END $$");
    // The preview says the same.
    assert_eq!(editor(&pg(sql), &sel(from_body), &ExecPolicy::default()), r);

    // A selection wholly inside the body is an open dollar quote: refused, never split at `;`.
    let inner = resolve_sel(sql, span(sql, "PERFORM 1; PERFORM 2;"));
    assert!(matches!(inner, RangeResolution::Ambiguous(_)), "{inner:?}");

    // Without the carried state the same bytes would split into `PERFORM 2`, `END $$`, …:
    // a selection starting at a clean place does split at its `;`.
    let clean = "PERFORM 1; PERFORM 2;";
    assert_eq!(
        texts(clean, &resolve_sel(clean, 0..clean.len())),
        ["PERFORM 1", "PERFORM 2"]
    );

    // Starting inside a string: the tail of the literal never runs as code.
    let s = "SELECT 'a; DROP TABLE t; --'; SELECT 2;";
    let tail = resolve_sel(s, after(s, "'a;")..s.len());
    assert!(matches!(tail, RangeResolution::Ambiguous(_)), "{tail:?}");
}

#[test]
fn a_selection_starting_inside_a_token_never_sends_its_tail() {
    // (text, where the selection starts, dialect): each starts inside a token whose rest
    // the server, reading from a clean state, would take as code.
    let cases: [(&str, &str, bool); 9] = [
        ("SELECT 'a; DROP TABLE t; '; SELECT 2;", "'a;", true),
        ("SELECT 'a; DROP TABLE t; '; SELECT 2;", "'a;", false),
        ("SELECT 'x DELETE FROM t'; SELECT 2;", "'x ", true),
        ("SELECT `a; DROP TABLE t; `; SELECT 2;", "`a;", true),
        ("SELECT \"a; DROP TABLE t; \"; SELECT 2;", "\"a;", false),
        (
            "SELECT $q$ a; DROP TABLE t; $q$; SELECT 2;",
            "$q$ a;",
            false,
        ),
        (
            "SELECT 1 /*! x; DROP TABLE t; */; SELECT 2;",
            "/*! x;",
            true,
        ),
        ("SELECT /*+ x */ 1; DROP TABLE t; SELECT 2;", "/*+", true),
        (
            "COPY t FROM STDIN;\nDROP TABLE x;\n\\.\nSELECT 2;",
            "STDIN;\n",
            false,
        ),
    ];
    for (sql, from, my) in cases {
        let d = if my { mysql(sql) } else { pg(sql) };
        // To the end of the text, and to just inside the token.
        let start = after(sql, from);
        for end in [sql.len(), start + 6] {
            for mode in [SplitMode::Execution, SplitMode::Editor] {
                let r = resolve(
                    &d,
                    &sel(start..end),
                    &ExecPolicy::default(),
                    mode,
                    None,
                    None,
                );
                assert!(
                    matches!(r, RangeResolution::Ambiguous(ref b) if !b.is_empty()),
                    "{sql:?} from {from:?} in {mode:?}: {r:?}"
                );
            }
        }
    }
    // COPY data without its end line, a row selected alone: data, never a statement.
    let copy = "COPY t FROM STDIN;\nDROP TABLE x;\n";
    let r = resolve_sel(copy, after(copy, "STDIN;\n")..copy.len());
    assert!(matches!(r, RangeResolution::Ambiguous(_)), "{r:?}");

    // Inside a plain comment the rest is trivia: what follows runs, the comment's text never.
    let c = "SELECT 1 /* x; DROP TABLE t; */; SELECT 2;";
    assert_eq!(
        texts(c, &resolve_sel(c, after(c, "/* x;")..c.len())),
        ["SELECT 2"]
    );

    // Inside a client command's line the text is the command's argument.
    let echo = "\\echo hi; DROP TABLE x;\nSELECT 2;";
    let r = resolve_sel(echo, after(echo, "hi;")..echo.len());
    assert!(
        matches!(r, RangeResolution::UnsupportedClientCommand { at } if echo[at].starts_with("\\echo")),
        "{r:?}"
    );
    let delim = "DELIMITER //\nSELECT 1//\n";
    let r = exec(&mysql(delim), &sel(5..delim.len()), &ExecPolicy::default());
    assert!(
        matches!(r, RangeResolution::UnsupportedClientCommand { .. }),
        "{r:?}"
    );
}

#[test]
fn sent_text_has_no_terminator_and_no_leading_comment() {
    let sql = "-- title\nSELECT 1;";
    let r = resolve_at(sql, "SELECT");
    let [u] = units_of(&r) else {
        panic!("one unit, got {r:?}")
    };
    assert_eq!(&sql[u.range], "SELECT 1");
    assert_eq!(u.title_comment.map(|t| &sql[t]), Some("-- title"));
    assert_eq!(u.kind, UnitKind::Statement);
    assert!(u.key.is_some());

    // The same through every policy that returns statements, and through a selection.
    let script = "/* a */\nSELECT 1 ; -- trailing\n\n-- b\nUPDATE t SET a = 1\n;\n";
    let d = pg(script);
    for policy in [
        ExecPolicy::whole_script(),
        inside(InsidePolicy::EverythingFromCaret),
    ] {
        let r = exec(&d, &caret(after(script, "SELECT")), &policy);
        assert_eq!(texts(script, &r), ["SELECT 1", "UPDATE t SET a = 1"]);
    }
    let r = exec(&d, &sel(0..script.len()), &ExecPolicy::default());
    assert_eq!(texts(script, &r), ["SELECT 1", "UPDATE t SET a = 1"]);
    let titles: Vec<Option<&str>> = units_of(&r)
        .iter()
        .map(|u| u.title_comment.map(|t| &script[t]))
        .collect();
    assert_eq!(titles, [Some("/* a */"), Some("-- b")]);
}

#[test]
fn caret_in_dollar_body_runs_the_whole_create_function() {
    let sql = "CREATE FUNCTION f() RETURNS int AS $$ SELECT 1; SELECT 2; $$ LANGUAGE sql;";
    assert_eq!(
        texts(sql, &resolve_at(sql, "SELECT 2")),
        [&sql[..sql.len() - 1]]
    );
}

#[test]
fn mysql_delimiter_block_is_one_unit() {
    let r = resolve_mysql_at(PROC_WITH_DELIMITER, "SELECT 1");
    // Two statements in the script, so Run asks; the statement is the whole procedure.
    let (c, preselect) = candidates_of(&r);
    assert_eq!(c[preselect].kind, CandidateKind::Statement);
    assert_eq!(
        candidate_texts(PROC_WITH_DELIMITER, &c[preselect]),
        [PROC_BODY]
    );
    // Run 2 (the largest statement) runs it directly.
    let d = mysql(PROC_WITH_DELIMITER);
    let run2 = exec(
        &d,
        &caret(after(PROC_WITH_DELIMITER, "SELECT 1")),
        &ExecPolicy::largest(),
    );
    assert_eq!(texts(PROC_WITH_DELIMITER, &run2), [PROC_BODY]);
    // The whole script: the procedure and the call; `DELIMITER` is applied, never sent.
    let all = exec(&d, &caret(0), &ExecPolicy::whole_script());
    assert_eq!(texts(PROC_WITH_DELIMITER, &all), [PROC_BODY, "CALL p()"]);
}

#[test]
fn ambiguous_boundary_is_never_a_unit() {
    assert!(matches!(
        resolve_mysql_exec("SELECT 1 /*! ; */ ;", 0),
        RangeResolution::Ambiguous(_)
    ));
    assert_eq!(
        reasons(&resolve_mysql_exec("SELECT 1 /*! ; */ ;", 0)),
        [AmbiguityReason::SemicolonInExecutableComment]
    );
}

// --- D-011 defaults and policies -------------------------------------------------------------

#[test]
fn the_default_policy_is_d011() {
    assert_eq!(
        ExecPolicy::default(),
        ExecPolicy {
            inside: InsidePolicy::Ask,
            outside: OutsidePolicy::Nothing,
            selection: SelectionPolicy::AsSeparate,
        }
    );
}

#[test]
fn nothing_asked_runs_nothing() {
    let d = pg("SELECT 1;");
    assert_eq!(
        exec(&d, &[], &ExecPolicy::whole_script()),
        RangeResolution::NothingToRun
    );
    assert_eq!(
        exec(&pg(""), &caret(0), &ExecPolicy::whole_script()),
        RangeResolution::NothingToRun
    );
    assert_eq!(
        exec(
            &pg("  \n-- only a comment\n"),
            &caret(3),
            &ExecPolicy::whole_script()
        ),
        RangeResolution::NothingToRun
    );
}

#[test]
fn inside_policies_pick_their_statements() {
    let sql = "SELECT 1;\nSELECT 2;\nSELECT 3;";
    let d = pg(sql);
    let c = caret(after(sql, "SELECT 2"));
    for p in [
        InsidePolicy::SmallestSubqueryOrStatement,
        InsidePolicy::SmallestStatement,
        InsidePolicy::LargestStatement,
        InsidePolicy::LargestStatementOrBatch,
    ] {
        assert_eq!(texts(sql, &exec(&d, &c, &inside(p))), ["SELECT 2"], "{p:?}");
    }
    assert_eq!(
        texts(sql, &exec(&d, &c, &inside(InsidePolicy::WholeScript))),
        ["SELECT 1", "SELECT 2", "SELECT 3"]
    );
    assert_eq!(
        texts(
            sql,
            &exec(&d, &c, &inside(InsidePolicy::EverythingFromCaret))
        ),
        ["SELECT 2", "SELECT 3"]
    );
}

#[test]
fn outside_policies_pick_their_statements() {
    let sql = "SELECT 1;\n\n\nSELECT 2;\nSELECT 3;";
    let d = pg(sql);
    let blank = caret(11);
    assert_eq!(
        exec(&d, &blank, &outside(OutsidePolicy::Nothing)),
        RangeResolution::NothingToRun
    );
    assert_eq!(
        texts(sql, &exec(&d, &blank, &outside(OutsidePolicy::WholeScript))),
        ["SELECT 1", "SELECT 2", "SELECT 3"]
    );
    assert_eq!(
        texts(
            sql,
            &exec(&d, &blank, &outside(OutsidePolicy::EverythingBelow))
        ),
        ["SELECT 2", "SELECT 3"]
    );
    // Below the last statement there is nothing.
    let end = "SELECT 1;\n\n";
    assert_eq!(
        exec(
            &pg(end),
            &caret(end.len()),
            &outside(OutsidePolicy::EverythingBelow)
        ),
        RangeResolution::NothingToRun
    );
}

#[test]
fn remembered_kind_is_preselected() {
    let sql = "SELECT 1;\nSELECT 2;\nSELECT 3;";
    let d = pg(sql);
    let c = caret(after(sql, "SELECT 2"));
    let with = |remembered| {
        resolve(
            &d,
            &c,
            &ExecPolicy::default(),
            SplitMode::Execution,
            None,
            remembered,
        )
    };
    let chosen = |r: &RangeResolution| {
        let (c, p) = candidates_of(r);
        c[p].kind
    };
    assert_eq!(chosen(&with(None)), CandidateKind::Statement);
    assert_eq!(
        chosen(&with(Some(CandidateKind::WholeScript))),
        CandidateKind::WholeScript
    );
    assert_eq!(
        chosen(&with(Some(CandidateKind::FromCaret))),
        CandidateKind::FromCaret
    );
    // A remembered kind that is not offered here falls back to the statement.
    assert_eq!(
        chosen(&with(Some(CandidateKind::Subquery))),
        CandidateKind::Statement
    );
    assert_eq!(
        chosen(&with(Some(CandidateKind::LargestOrBatch))),
        CandidateKind::Statement
    );
    // The remembered kind never changes what is offered.
    assert_eq!(
        kinds(&with(Some(CandidateKind::WholeScript))),
        kinds(&with(None))
    );
    // One candidate still runs directly, whatever is remembered.
    let one = pg("SELECT 1;");
    let r = resolve(
        &one,
        &caret(3),
        &ExecPolicy::default(),
        SplitMode::Execution,
        None,
        Some(CandidateKind::WholeScript),
    );
    assert_eq!(texts("SELECT 1;", &r), ["SELECT 1"]);
}

// --- caret-at-boundary rules (alg/10 SQL-SPLIT-04 step 2) ----------------------------------

/// The statement a caret at `┃` means under "smallest statement", or `None` outside.
fn statement_at_marker(marked: &str) -> Option<String> {
    let offset = marked.find('┃').expect("a caret marker");
    let sql = marked.replace('┃', "");
    let r = exec(
        &pg(&sql),
        &caret(offset),
        &inside(InsidePolicy::SmallestStatement),
    );
    match r {
        RangeResolution::NothingToRun => None,
        other => Some(texts(&sql, &other).join(" | ")),
    }
}

#[test]
fn caret_at_boundary_rules() {
    let cases: &[(&str, Option<&str>)] = &[
        // At the statement's first character, inside it, at its end, right after its `;`.
        ("┃SELECT 1;", Some("SELECT 1")),
        ("SELECT┃ 1;", Some("SELECT 1")),
        ("SELECT 1┃;", Some("SELECT 1")),
        ("SELECT 1;┃", Some("SELECT 1")),
        ("SELECT 1;┃\nSELECT 2;", Some("SELECT 1")),
        // In white space after it on its line: the statement before.
        ("SELECT 1;  ┃", Some("SELECT 1")),
        ("SELECT 1; \t┃  \nSELECT 2;", Some("SELECT 1")),
        ("SELECT 1  ┃", Some("SELECT 1")),
        // But not when another statement follows on the line: the caret is between two.
        ("SELECT 1;  ┃  SELECT 2;", None),
        // On the next line: the statement that starts there, or outside on a blank line.
        ("SELECT 1;\n┃SELECT 2;", Some("SELECT 2")),
        ("SELECT 1;\n┃\nSELECT 2;", None),
        ("SELECT 1;\n  ┃\nSELECT 2;", None),
        ("SELECT 1;\n\n┃", None),
        // Before the first statement on its line.
        ("  ┃  SELECT 1;", None),
        // Where two statements touch, the earlier one.
        ("SELECT 1;┃SELECT 2;", Some("SELECT 1")),
        ("SELECT 1; ┃SELECT 2;", Some("SELECT 2")),
        // An empty statement is no statement.
        ("SELECT 1;;┃", None),
        ("SELECT 1; ;┃", None),
        (";┃", None),
        ("SELECT 1;┃;", Some("SELECT 1")),
        // Inside a string, a quoted identifier, a dollar-quoted body, a block comment in it.
        ("SELECT 'a;┃b';\nSELECT 2;", Some("SELECT 'a;b'")),
        ("SELECT \"a;┃b\";\nSELECT 2;", Some("SELECT \"a;b\"")),
        (
            "DO $$ BEGIN ┃NULL; END $$;\nSELECT 2;",
            Some("DO $$ BEGIN NULL; END $$"),
        ),
        ("SELECT /* ; ┃ */ 1;\nSELECT 2;", Some("SELECT /* ; ┃ */ 1")),
        // In an attached leading comment: inside. Set apart by a blank line: outside.
        ("-- ti┃tle\nSELECT 1;", Some("SELECT 1")),
        ("/* a */ /* ┃b */\nSELECT 1;", Some("SELECT 1")),
        ("-- ti┃tle\n\nSELECT 1;", None),
        ("SELECT 1;\n\n/* no┃te */\n\nSELECT 2;", None),
        // A comment after the `;` on the same line belongs to that statement (SPLIT-02).
        ("SELECT 1; -- do┃ne\nSELECT 2;", Some("SELECT 1")),
        (
            "select 10 ; -- Comm┃ents\nselect 20 ; /* Comments */",
            Some("select 10"),
        ),
        (
            "select 10 ; -- Comments\nselect 20 ; /* Comm┃ents */",
            Some("select 20"),
        ),
        // The unterminated last statement runs.
        ("SELECT 1;\nSELECT ┃2", Some("SELECT 2")),
        ("SELECT 1;\nSELECT 2┃", Some("SELECT 2")),
    ];
    for (marked, want) in cases {
        let want = want.map(|w| w.replace('┃', ""));
        assert_eq!(statement_at_marker(marked), want, "{marked:?}");
    }
}

#[test]
fn a_caret_past_the_end_or_inside_a_character_is_clamped() {
    let sql = "SELECT 'é'";
    let d = pg(sql);
    let p = inside(InsidePolicy::SmallestStatement);
    assert_eq!(texts(sql, &exec(&d, &caret(999), &p)), [sql]);
    // Byte 9 is inside `é` (bytes 8..10).
    assert_eq!(texts(sql, &exec(&d, &caret(9), &p)), [sql]);
    // A selection that splits a character is no text.
    assert_eq!(
        exec(&d, &sel(0..9), &ExecPolicy::default()),
        RangeResolution::NothingToRun
    );
    // A selection past the end is cut at the end.
    assert_eq!(
        texts(sql, &exec(&d, &sel(0..999), &ExecPolicy::default())),
        [sql]
    );
}

// --- several carets --------------------------------------------------------------------------

#[test]
fn several_carets_unite_in_text_order_without_duplicates() {
    let sql = "SELECT 1;\nSELECT 2;\nSELECT 3;\n\nSELECT 4;";
    let d = pg(sql);
    let carets = [
        CaretOrSelection::Caret(at(after(sql, "SELECT 3"))),
        CaretOrSelection::Caret(at(after(sql, "SELECT 1"))),
        // A second caret in the same statement, and one outside every statement.
        CaretOrSelection::Caret(at(1)),
        CaretOrSelection::Caret(at(after(sql, "SELECT 3;\n"))),
    ];
    // D-011's Ask runs each caret's statement: no chooser per caret.
    let r = exec(&d, &carets, &ExecPolicy::default());
    assert_eq!(texts(sql, &r), ["SELECT 1", "SELECT 3"]);
    // Carets that are all outside run nothing.
    let outside_only = [
        CaretOrSelection::Caret(at(30)),
        CaretOrSelection::Caret(at(30)),
    ];
    assert_eq!(
        exec(&d, &outside_only, &ExecPolicy::default()),
        RangeResolution::NothingToRun
    );
    // A caret and a selection.
    let mixed = [
        CaretOrSelection::Caret(at(1)),
        CaretOrSelection::Selection(TextRange::new(at(10), at(sql.len()))),
    ];
    assert_eq!(
        texts(sql, &exec(&d, &mixed, &ExecPolicy::default())),
        ["SELECT 1", "SELECT 2", "SELECT 3", "SELECT 4"]
    );
}

#[test]
fn several_carets_with_one_uncertain_statement_run_nothing() {
    let sql = "SELECT 1;\nSELECT 2 /*! ; */;\nSELECT 3;";
    let d = mysql(sql);
    let carets = [
        CaretOrSelection::Caret(at(1)),
        CaretOrSelection::Caret(at(after(sql, "SELECT 2"))),
    ];
    assert_eq!(
        reasons(&exec(&d, &carets, &ExecPolicy::default())),
        [AmbiguityReason::SemicolonInExecutableComment]
    );
}

// --- selections ------------------------------------------------------------------------------

#[test]
fn a_selection_of_white_space_or_comments_runs_nothing() {
    let sql = "SELECT 1;   \n\t\n  -- note\n /* more */ \nSELECT 2;";
    let d = pg(sql);
    let ws = after(sql, "SELECT 1;")..after(sql, "\t\n");
    assert!(sql[ws.clone()].trim().is_empty());
    for p in [
        SelectionPolicy::AsSeparate,
        SelectionPolicy::AsOne,
        SelectionPolicy::SmartExpand,
    ] {
        assert_eq!(
            exec(&d, &sel(ws.clone()), &selecting(p)),
            RangeResolution::NothingToRun,
            "{p:?}"
        );
    }
    let comments = after(sql, "SELECT 1;")..after(sql, "/* more */ ");
    assert_eq!(
        exec(&d, &sel(comments.clone()), &ExecPolicy::default()),
        RangeResolution::NothingToRun
    );
    assert_eq!(
        exec(&d, &sel(comments), &selecting(SelectionPolicy::AsOne)),
        RangeResolution::NothingToRun
    );
    // An empty selection is a caret.
    let empty = [CaretOrSelection::Selection(TextRange::empty(at(3)))];
    assert_eq!(
        texts(
            sql,
            &exec(&d, &empty, &inside(InsidePolicy::SmallestStatement))
        ),
        ["SELECT 1"]
    );
}

#[test]
fn a_selection_runs_exactly_what_is_selected() {
    // Part of a statement runs as selected (D-011), without the terminator.
    let sql = "SELECT 1 + 2;\nDELETE FROM t WHERE id = 1;";
    assert_eq!(texts(sql, &resolve_sel(sql, 0..8)), ["SELECT 1"]);
    assert_eq!(
        texts(sql, &resolve_sel(sql, span(sql, "DELETE FROM t"))),
        ["DELETE FROM t"]
    );
    // Statements the selection only touches at its edge are not run.
    assert_eq!(
        texts(sql, &resolve_sel(sql, 0..after(sql, "2;\n"))),
        ["SELECT 1 + 2"]
    );
    // The preview agrees.
    let d = pg(sql);
    assert_eq!(
        editor(&d, &sel(0..8), &ExecPolicy::default()),
        exec(&d, &sel(0..8), &ExecPolicy::default())
    );
}

#[test]
fn a_selection_ending_inside_a_string_is_refused() {
    let sql = "SELECT 'abc'; SELECT 2;";
    let r = resolve_sel(sql, 0..after(sql, "'ab"));
    assert_eq!(reasons(&r), [AmbiguityReason::UnterminatedConstruct]);
    // One uncertain statement refuses the whole selection: the certain one does not run.
    let both = "SELECT 2; SELECT 'abc';";
    assert!(matches!(
        resolve_sel(both, 0..after(both, "'ab")),
        RangeResolution::Ambiguous(_)
    ));
}

#[test]
fn a_selection_after_an_active_delimiter_carries_it() {
    let sql = PROC_WITH_DELIMITER;
    let d = mysql(sql);
    // From the CREATE to the `//`: `;` is not the delimiter there.
    let body = after(sql, "DELIMITER //\n")..after(sql, "END//");
    assert_eq!(
        texts(sql, &exec(&d, &sel(body), &ExecPolicy::default())),
        [PROC_BODY]
    );
    // The whole script, DELIMITER lines included.
    assert_eq!(
        texts(sql, &exec(&d, &sel(0..sql.len()), &ExecPolicy::default())),
        [PROC_BODY, "CALL p()"]
    );
    // Under `//`, a selection holding a top-level `;` is two statements the server would
    // run as one: refused.
    let two = "DELIMITER //\nSELECT 1; SELECT 2//\n";
    let r = exec(
        &mysql(two),
        &sel(after(two, "//\n")..two.len()),
        &ExecPolicy::default(),
    );
    assert_eq!(reasons(&r), [AmbiguityReason::DelimiterChange]);
}

#[test]
fn smart_expand_takes_the_statements_a_boundary_joins() {
    let sql = "SELECT 1; SELECT 22; SELECT 3;";
    let d = pg(sql);
    let p = selecting(SelectionPolicy::SmartExpand);
    // `1; SELECT 2` crosses a boundary: both statements, whole.
    let r = exec(&d, &sel(7..18), &p);
    assert_eq!(texts(sql, &r), ["SELECT 1", "SELECT 22"]);
    assert!(units_of(&r).iter().all(|u| u.kind == UnitKind::Statement));
    // Inside one statement: the selection as written.
    let r = exec(&d, &sel(span(sql, "SELECT 2")), &p);
    assert_eq!(texts(sql, &r), ["SELECT 2"]);
    assert_eq!(units_of(&r)[0].kind, UnitKind::Verbatim);
    assert_eq!(units_of(&r)[0].key, None);
}

#[test]
fn a_verbatim_selection_is_trimmed_and_checked() {
    let p = selecting(SelectionPolicy::AsOne);
    let sql = "  SELECT 1; SELECT 2;  \n";
    let r = exec(&pg(sql), &sel(0..sql.len()), &p);
    assert_eq!(texts(sql, &r), ["SELECT 1; SELECT 2;"]);
    assert_eq!(units_of(&r)[0].kind, UnitKind::Verbatim);
    // A client command or COPY data would reach the server as SQL: refused.
    let cmd = "SELECT 1;\n\\echo hi\nSELECT 2;";
    assert!(matches!(
        exec(&pg(cmd), &sel(0..cmd.len()), &p),
        RangeResolution::UnsupportedClientCommand { at } if &cmd[at] == "\\echo hi"
    ));
    let delim = "DELIMITER //\nSELECT 1//\n";
    assert!(matches!(
        exec(&mysql(delim), &sel(0..delim.len()), &p),
        RangeResolution::UnsupportedClientCommand { .. }
    ));
    let copy = "COPY t FROM STDIN;\n1\t2\n\\.\nSELECT 1;";
    assert!(matches!(
        exec(&pg(copy), &sel(0..copy.len()), &p),
        RangeResolution::UnsupportedClientCommand { at } if &copy[at] == "COPY t FROM STDIN;\n1\t2\n"
    ));
    // An uncertain boundary inside it: nothing is sent (R-M3-24).
    let amb = "SELECT 1 /*! ; */ ;";
    assert_eq!(
        reasons(&exec(&mysql(amb), &sel(0..amb.len()), &p)),
        [AmbiguityReason::SemicolonInExecutableComment]
    );
}

#[test]
fn a_selection_starting_inside_an_executable_comment_is_refused_unsplit() {
    // From the `/` that closes the first comment: the carried state is "inside `/*!`", and the
    // comment's tail (`/`, a space, then a three-byte character) is no `/*!` opener to skip.
    let sql = "SELECT 1 /*! ; */ \u{202E} /*!50000 x */ ; SELECT 2;";
    let from = after(sql, "/*! ; *");
    for mode in [SplitMode::Execution, SplitMode::Editor] {
        let r = resolve(
            &mysql(sql),
            &sel(from..sql.len()),
            &ExecPolicy::default(),
            mode,
            None,
            None,
        );
        assert_eq!(
            reasons(&r),
            [AmbiguityReason::AmbiguousCommentEnd],
            "{mode:?}"
        );
    }
}

// --- refusals --------------------------------------------------------------------------------

#[test]
fn execution_refuses_what_the_editor_previews_as_uncertain() {
    // No `;`: the editor guesses two statements, execution knows one uncertain text.
    let sql = "select * from t\n\nselect 1";
    let d = pg(sql);
    let c = caret(after(sql, "select 1"));
    let p = inside(InsidePolicy::SmallestStatement);
    assert_eq!(texts(sql, &editor(&d, &c, &p)), ["select 1"]);
    assert_eq!(
        reasons(&exec(&d, &c, &p)),
        [AmbiguityReason::MissingSemicolon]
    );
    assert_eq!(
        reasons(&exec(&d, &c, &ExecPolicy::whole_script())),
        [AmbiguityReason::MissingSemicolon]
    );
    assert_eq!(
        reasons(&exec(&d, &c, &ExecPolicy::default())),
        [AmbiguityReason::MissingSemicolon]
    );
    assert_eq!(
        reasons(&exec(&d, &sel(0..sql.len()), &ExecPolicy::default())),
        [AmbiguityReason::MissingSemicolon]
    );
}

#[test]
fn an_unterminated_construct_refuses_only_what_it_touches() {
    let sql = "SELECT 1;\nSELECT 2;\nSELECT 'open;\nSELECT 4;";
    let d = pg(sql);
    // The clean statement above still runs at the caret.
    let p = inside(InsidePolicy::SmallestStatement);
    assert_eq!(texts(sql, &exec(&d, &caret(1), &p)), ["SELECT 1"]);
    // The open string runs nothing.
    let open = caret(after(sql, "SELECT 'op"));
    assert_eq!(
        reasons(&exec(&d, &open, &p)),
        [AmbiguityReason::UnterminatedConstruct]
    );
    assert_eq!(
        reasons(&exec(&d, &open, &ExecPolicy::default())),
        [AmbiguityReason::UnterminatedConstruct]
    );
    // Nor does any run that would include it.
    for policy in [
        ExecPolicy::whole_script(),
        inside(InsidePolicy::EverythingFromCaret),
    ] {
        assert!(matches!(
            exec(&d, &caret(1), &policy),
            RangeResolution::Ambiguous(_)
        ));
    }
}

#[test]
fn ask_in_execution_mode_leaves_out_what_it_would_refuse() {
    let sql = "SELECT 1;\nSELECT 2;\nSELECT 3 /*! ; */;";
    let d = mysql(sql);
    // In `SELECT 1`: "whole script" would cross the uncertain statement, so it is not
    // offered; one candidate is left and runs directly.
    let first = exec(&d, &caret(1), &ExecPolicy::default());
    assert_eq!(texts(sql, &first), ["SELECT 1"]);
    // The preview offers the same statement first.
    let preview = editor(&d, &caret(1), &ExecPolicy::default());
    let (c, preselect) = candidates_of(&preview);
    assert_eq!(c[preselect].units, units_of(&first));
    assert_eq!(
        kinds(&preview),
        [CandidateKind::Statement, CandidateKind::WholeScript]
    );
    // No candidate of an execution-mode chooser ever holds the uncertain statement.
    let four = "SELECT 0;\nSELECT 1;\nSELECT 2;\nSELECT 3 /*! ; */;\nSELECT 4;";
    let d4 = mysql(four);
    let r = exec(&d4, &caret(after(four, "SELECT 1")), &ExecPolicy::default());
    assert_eq!(texts(four, &r), ["SELECT 1"]);
    // In the uncertain statement itself: nothing.
    assert!(matches!(
        exec(&d, &caret(after(sql, "SELECT 3")), &ExecPolicy::default()),
        RangeResolution::Ambiguous(_)
    ));

    // When the console remembers "whole script" (or "from the caret"), the chooser would
    // open on a range that cannot run: Run says so, and never runs the statement instead.
    for (at_needle, remembered) in [
        ("SELECT 1", CandidateKind::WholeScript),
        ("SELECT 2", CandidateKind::WholeScript),
        ("SELECT 2", CandidateKind::FromCaret),
    ] {
        let r = resolve(
            &d,
            &caret(after(sql, at_needle)),
            &ExecPolicy::default(),
            SplitMode::Execution,
            None,
            Some(remembered),
        );
        assert_eq!(
            reasons(&r),
            [AmbiguityReason::SemicolonInExecutableComment],
            "{at_needle} {remembered:?}"
        );
    }
    // The same for a script with a client command Hacivat does not run, in both modes.
    let meta = "SELECT 1;\n\\echo x\nSELECT 2;";
    for mode in [SplitMode::Execution, SplitMode::Editor] {
        let r = resolve(
            &pg(meta),
            &caret(1),
            &ExecPolicy::default(),
            mode,
            None,
            Some(CandidateKind::WholeScript),
        );
        assert!(
            matches!(r, RangeResolution::UnsupportedClientCommand { at } if &meta[at] == "\\echo x"),
            "{mode:?}: {r:?}"
        );
    }
    // A remembered kind that can run is preselected among what can run.
    let r = resolve(
        &d4,
        &caret(after(four, "SELECT 4")),
        &ExecPolicy::default(),
        SplitMode::Execution,
        None,
        Some(CandidateKind::WholeScript),
    );
    assert!(matches!(r, RangeResolution::Ambiguous(_)), "{r:?}");
}

#[test]
fn an_unsupported_client_command_is_never_run_past() {
    let sql = fixture("FX-SPLIT-PG-07");
    let d = pg(&sql);
    let p = ExecPolicy::default();
    // On the command itself.
    for needle in ["\\set ON", "\\echo"] {
        let r = exec(&d, &caret(after(&sql, needle)), &p);
        assert!(
            matches!(r, RangeResolution::UnsupportedClientCommand { at } if sql[at].starts_with('\\')),
            "{needle}: {r:?}"
        );
    }
    // A statement beside it runs alone: "whole script" and "from the caret" are not offered.
    assert_eq!(
        texts(&sql, &exec(&d, &caret(after(&sql, "SELECT 1")), &p)),
        ["SELECT 1"]
    );
    assert_eq!(
        texts(&sql, &exec(&d, &caret(after(&sql, "SELECT 2")), &p)),
        ["SELECT 2"]
    );
    // A run of several statements across it is refused, in both modes.
    for mode in [SplitMode::Execution, SplitMode::Editor] {
        for policy in [
            ExecPolicy::whole_script(),
            inside(InsidePolicy::EverythingFromCaret),
        ] {
            let r = resolve(
                &d,
                &caret(after(&sql, "SELECT 1")),
                &policy,
                mode,
                None,
                None,
            );
            assert!(
                matches!(r, RangeResolution::UnsupportedClientCommand { .. }),
                "{mode:?} {policy:?}: {r:?}"
            );
        }
        let r = resolve(&d, &sel(0..sql.len()), &p, mode, None, None);
        assert!(matches!(
            r,
            RangeResolution::UnsupportedClientCommand { at } if &sql[at] == "\\set ON_ERROR_STOP on"
        ));
    }
    // Two carets on either side of `\echo`.
    let carets = [
        CaretOrSelection::Caret(at(after(&sql, "SELECT 1"))),
        CaretOrSelection::Caret(at(after(&sql, "SELECT 2"))),
    ];
    assert!(matches!(
        exec(&d, &carets, &p),
        RangeResolution::UnsupportedClientCommand { at } if &sql[at] == "\\echo done"
    ));
    // From the last statement on, nothing unsupported is spanned.
    assert_eq!(
        texts(
            &sql,
            &exec(
                &d,
                &caret(after(&sql, "SELECT 2")),
                &inside(InsidePolicy::EverythingFromCaret)
            )
        ),
        ["SELECT 2"]
    );
}

#[test]
fn psql_conditional_branches_are_refused() {
    let sql = "SELECT 1;\n\\if :x\nDROP TABLE t;\n\\endif\nSELECT 2;";
    let d = pg(sql);
    let branch = exec(
        &d,
        &caret(after(sql, "DROP")),
        &inside(InsidePolicy::SmallestStatement),
    );
    assert!(
        !matches!(
            branch,
            RangeResolution::Units(_) | RangeResolution::Ask { .. }
        ),
        "{branch:?}"
    );
    let all = exec(&d, &caret(1), &ExecPolicy::whole_script());
    assert!(
        !matches!(all, RangeResolution::Units(_) | RangeResolution::Ask { .. }),
        "{all:?}"
    );
    let selected = exec(&d, &sel(0..sql.len()), &ExecPolicy::default());
    assert!(
        !matches!(
            selected,
            RangeResolution::Units(_) | RangeResolution::Ask { .. }
        ),
        "{selected:?}"
    );
}

#[test]
fn a_supported_client_command_line_is_outside_every_statement() {
    let d = mysql(PROC_WITH_DELIMITER);
    assert_eq!(
        exec(&d, &caret(4), &ExecPolicy::default()),
        RangeResolution::NothingToRun
    );
    // psql `\g` ends its statement and is part of it.
    let sql = "SELECT 1 \\g\nSELECT 2;";
    assert_eq!(
        texts(
            sql,
            &exec(
                &pg(sql),
                &caret(after(sql, "\\")),
                &inside(InsidePolicy::SmallestStatement)
            )
        ),
        ["SELECT 1"]
    );
}

#[test]
fn copy_from_stdin_is_one_unit_with_its_data() {
    let sql = fixture("FX-SPLIT-PG-05");
    let d = pg(&sql);
    let r = exec(
        &d,
        &caret(after(&sql, "COPY audit")),
        &inside(InsidePolicy::SmallestStatement),
    );
    let [u] = units_of(&r) else {
        panic!("one unit, got {r:?}")
    };
    assert_eq!(&sql[u.range], "COPY audit_log(actor, message) FROM STDIN");
    let UnitKind::CopyIn { data } = u.kind else {
        panic!("a COPY unit, got {:?}", u.kind)
    };
    assert_eq!(&sql[data], "sys\thello; world\nsys\tsecond\n");
    // A caret in the data rows is not in a statement the rows could run as.
    let rows = exec(
        &d,
        &caret(after(&sql, "sys\thello")),
        &inside(InsidePolicy::SmallestStatement),
    );
    assert!(
        match &rows {
            RangeResolution::NothingToRun => true,
            RangeResolution::Units(u) =>
                u.iter().all(|u| matches!(u.kind, UnitKind::CopyIn { .. })),
            _ => false,
        },
        "{rows:?}"
    );
}

// --- the subquery oracle (M5) ----------------------------------------------------------------

struct Fixed(TextRange);

impl SubqueryOracle for Fixed {
    fn innermost_runnable_subquery(
        &self,
        _doc: &DocSnapshot,
        _stmt: &Stmt,
        _caret: TextSize,
    ) -> Option<TextRange> {
        Some(self.0)
    }
}

#[test]
fn a_subquery_oracle_adds_the_first_candidate() {
    let sql = "SELECT x.id FROM (SELECT id FROM orders) x;\nSELECT 2;";
    let d = pg(sql);
    let inner = span(sql, "SELECT id FROM orders");
    let oracle = Fixed(TextRange::new(at(inner.start), at(inner.end)));
    let c = caret(after(sql, "SELECT id"));
    let ask = resolve(
        &d,
        &c,
        &ExecPolicy::default(),
        SplitMode::Execution,
        Some(&oracle),
        None,
    );
    assert_eq!(
        kinds(&ask),
        [
            CandidateKind::Subquery,
            CandidateKind::Statement,
            CandidateKind::WholeScript
        ]
    );
    let (cands, preselect) = candidates_of(&ask);
    assert_eq!(cands[preselect].kind, CandidateKind::Statement);
    assert_eq!(candidate_texts(sql, &cands[0]), ["SELECT id FROM orders"]);
    assert_eq!(cands[0].units[0].kind, UnitKind::Subquery);
    assert_eq!(cands[0].label, "SELECT id FROM orders");

    let smallest = resolve(
        &d,
        &c,
        &inside(InsidePolicy::SmallestSubqueryOrStatement),
        SplitMode::Execution,
        Some(&oracle),
        None,
    );
    assert_eq!(texts(sql, &smallest), ["SELECT id FROM orders"]);

    // A range outside the statement at the caret is ignored.
    let stray = Fixed(TextRange::new(at(after(sql, ";\n")), at(sql.len() - 1)));
    let r = resolve(
        &d,
        &c,
        &inside(InsidePolicy::SmallestSubqueryOrStatement),
        SplitMode::Execution,
        Some(&stray),
        None,
    );
    assert_eq!(
        texts(sql, &r),
        ["SELECT x.id FROM (SELECT id FROM orders) x"]
    );

    // A subquery that is the whole statement is one entry, not two.
    let one = "(SELECT 1);\nSELECT 2;";
    let whole = Fixed(TextRange::new(at(0), at(10)));
    let r = resolve(
        &pg(one),
        &caret(3),
        &ExecPolicy::default(),
        SplitMode::Execution,
        Some(&whole),
        None,
    );
    assert_eq!(
        kinds(&r),
        [CandidateKind::Subquery, CandidateKind::WholeScript]
    );

    // In an uncertain statement the oracle changes nothing: nothing runs.
    let amb = "SELECT (SELECT 1) /*! ; */;";
    let r = resolve(
        &mysql(amb),
        &caret(10),
        &inside(InsidePolicy::SmallestSubqueryOrStatement),
        SplitMode::Execution,
        Some(&Fixed(TextRange::new(at(8), at(16)))),
        None,
    );
    assert!(matches!(r, RangeResolution::Ambiguous(_)), "{r:?}");
}

// --- labels ----------------------------------------------------------------------------------

#[test]
fn labels_collapse_white_space_and_keep_leading_comments() {
    let sql = "-- top\n-- customers\nSELECT\tid,\n       name\r\nFROM   t;\n\nSELECT 2;";
    let r = resolve_at(sql, "SELECT\tid");
    let (c, _) = candidates_of(&r);
    assert_eq!(c[0].label, "-- top -- customers SELECT id, name FROM t");
    assert_eq!(
        c[1].label,
        "-- top -- customers SELECT id, name FROM t; SELECT 2"
    );
    // The label is not the sent text: the unit has no comment.
    assert_eq!(
        &sql[c[0].units[0].range],
        "SELECT\tid,\n       name\r\nFROM   t"
    );
}

#[test]
fn labels_neutralise_controls_and_bidi_overrides() {
    let sql = "SELECT '\u{202E}evil\u{202C}', '\u{7}bell';\nSELECT 2;";
    let r = resolve_at(sql, "SELECT");
    let (c, _) = candidates_of(&r);
    assert_eq!(c[0].label, "SELECT '<U+202E>evil<U+202C>', ' bell'");
    assert!(!c[1].label.contains('\u{202E}'));
    assert!(c.iter().all(|c| !c.label.chars().any(char::is_control)));
}

#[test]
fn a_whole_script_label_is_bounded() {
    let sql = "SELECT 1;\n".repeat(2_000);
    let r = resolve_at(&sql, "SELECT");
    let (c, _) = candidates_of(&r);
    assert_eq!(c[1].kind, CandidateKind::WholeScript);
    assert_eq!(c[1].units.len(), 2_000);
    assert!(c[1].label.chars().count() <= LABEL_MAX_CHARS);
    assert!(c[1].label.starts_with("SELECT 1; SELECT 1;"));
}

// --- performance (HV-EXEC-01: ≤ 2 ms p95 on a 50k-line file, warm) --------------------------

#[test]
#[ignore = "perf"]
fn resolve_on_50k_lines_within_2_ms_release() {
    if !std::env::var_os("HACIVAT_PERF").is_some_and(|v| v == "1") {
        return;
    }
    // 50,000 lines, 12,500 statements, each with a title comment.
    let sql = "-- a note\nSELECT id, name\nFROM customers\nWHERE id = 1;\n".repeat(12_500);
    let d = pg(&sql);
    // Warm: both splits exist before Run asks.
    assert_eq!(d.execution_split().statements().count(), 12_500);
    let p95 = |policy: ExecPolicy, mode: SplitMode| {
        let mut times: Vec<std::time::Duration> = (0..200)
            .map(|n| {
                let at = caret((sql.len() / 200) * n + 15);
                let start = std::time::Instant::now();
                let r = resolve(&d, &at, &policy, mode, None, None);
                let took = start.elapsed();
                assert!(matches!(
                    r,
                    RangeResolution::Units(_) | RangeResolution::Ask { .. }
                ));
                took
            })
            .collect();
        times.sort();
        times[189]
    };
    let statement = p95(ExecPolicy::largest(), SplitMode::Execution);
    let ask = p95(ExecPolicy::default(), SplitMode::Execution);
    let preview = p95(ExecPolicy::default(), SplitMode::Editor);
    let whole = p95(ExecPolicy::whole_script(), SplitMode::Execution);
    println!(
        "resolve p95: statement {statement:?}, ask {ask:?}, preview {preview:?}, whole script {whole:?}"
    );
    for (name, t) in [
        ("statement", statement),
        ("ask", ask),
        ("preview", preview),
        ("whole script", whole),
    ] {
        assert!(t.as_micros() <= 2_000, "{name}: {t:?}");
    }
}
