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
    let r = exec(&d, &sel(span(sql, "SELECT 22")), &p);
    assert_eq!(texts(sql, &r), ["SELECT 22"]);
    assert_eq!(units_of(&r)[0].kind, UnitKind::Verbatim);
    assert_eq!(units_of(&r)[0].key, None);
    // But never with a token cut: `SELECT 2` of `SELECT 22` is not what the document shows.
    let r = exec(&d, &sel(span(sql, "SELECT 2")), &p);
    assert_eq!(reasons(&r), [AmbiguityReason::UnterminatedConstruct]);
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
        RangeResolution::UnsupportedClientCommand { at } if &copy[at] == "1\t2\n"
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
    // The preview shows the same: one statement, no chooser over an immediate run.
    assert_eq!(editor(&d, &caret(1), &ExecPolicy::default()), first);
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

// --- review round 1 -----------------------------------------------------------------------

/// `marked` without its `«…»`, and the selection they mark.
fn marked_sel(marked: &str) -> (String, std::ops::Range<usize>) {
    let a = marked.find('«').expect("a selection start");
    let rest = marked.replacen('«', "", 1);
    let b = rest.find('»').expect("a selection end");
    (rest.replacen('»', "", 1), a..b)
}

/// The reasons Run refuses the selection of `marked` with, the same in both modes and under
/// every selection policy that runs the selection itself.
fn refused(marked: &str, id: DialectId, flavor: ClientFlavor) -> Vec<AmbiguityReason> {
    let (sql, r) = marked_sel(marked);
    let d = doc_in(&sql, id, flavor);
    let x = exec(&d, &sel(r.clone()), &ExecPolicy::default());
    assert_eq!(
        editor(&d, &sel(r.clone()), &ExecPolicy::default()),
        x,
        "{marked:?}"
    );
    let as_one = exec(&d, &sel(r), &selecting(SelectionPolicy::AsOne));
    assert!(
        !matches!(as_one, RangeResolution::Units(_)),
        "{marked:?} as one: {as_one:?}"
    );
    match x {
        RangeResolution::Ambiguous(b) => {
            assert!(!b.is_empty());
            b.iter().map(|a| a.reason).collect()
        }
        other => panic!("{marked:?}: expected a refusal, got {other:?}"),
    }
}

// I1: COPY rows are never in a sent range.
#[test]
fn a_selection_reaching_copy_rows_from_inside_their_statement_is_refused() {
    for marked in [
        "COPY t FROM STDIN;«\nDROP TABLE x;\nDELETE FROM y;\n»\\.\nSELECT 9;",
        "COPY t FROM STDIN«;\nDROP TABLE x;\n»\\.\nSELECT 9;",
        "COPY t FROM STD«IN;\nDROP TABLE x;\n»\\.\nSELECT 9;",
        "COPY t FROM STDIN;\n«DROP TABLE x;\n»\\.\nSELECT 9;",
        "COPY t FROM STDIN;\nDROP «TABLE x;\n»\\.\nSELECT 9;",
    ] {
        let r = refused(marked, DialectId::Postgres, ClientFlavor::Psql);
        assert!(
            r.contains(&AmbiguityReason::UnterminatedConstruct),
            "{marked:?}: {r:?}"
        );
    }
    // The statement after the data, selected alone, runs.
    let (sql, r) = marked_sel("COPY t FROM STDIN;\nDROP TABLE x;\n\\.\n«SELECT 9;»");
    assert_eq!(
        texts(&sql, &exec(&pg(&sql), &sel(r), &ExecPolicy::default())),
        ["SELECT 9"]
    );
}

// I2: the selection path answers what the caret path answers at the same offset.
#[test]
fn a_selection_inside_an_opener_or_a_prefixed_string_answers_as_the_caret_does() {
    use AmbiguityReason::{AmbiguousCommentEnd, UnterminatedConstruct};
    let (my, maria) = (DialectId::MySql, DialectId::MariaDb);
    let cli = ClientFlavor::MySqlCli;
    for (marked, id) in [
        (
            "SELECT 1 /*«! , \"*/ DROP TABLE t;» -- \" */ ; SELECT 2;",
            my,
        ),
        ("SELECT /*«+ \"*/ DROP TABLE u;» -- \" */ 2;", my),
        (
            "SELECT 1 /*M«! , \"*/ DROP TABLE t;» -- \" */ ; SELECT 2;",
            maria,
        ),
        ("SELECT 1 /*«! SELECT 2; */ ; DROP TABLE t;»", my),
    ] {
        assert_eq!(
            refused(marked, id, cli),
            [AmbiguousCommentEnd],
            "{marked:?}"
        );
        // The caret at the selection's start: the same refusal.
        let (sql, r) = marked_sel(marked);
        let at_caret = exec(
            &doc_in(&sql, id, cli),
            &caret(r.start),
            &ExecPolicy::largest(),
        );
        assert!(
            matches!(at_caret, RangeResolution::Ambiguous(_)),
            "{marked:?}: {at_caret:?}"
        );
    }
    for (marked, statement) in [
        (
            "SELECT E«'a\\'; DROP TABLE t; --'»;\nSELECT 2;",
            "SELECT E'a\\'; DROP TABLE t; --'",
        ),
        // In a `U&''` string a backslash escapes no quote: the string ends at `\'`.
        (
            "SELECT U&«'a\\'; DROP TABLE t; --'»;\nSELECT 2;",
            "SELECT U&'a\\'",
        ),
    ] {
        assert_eq!(
            refused(marked, DialectId::Postgres, ClientFlavor::Psql),
            [UnterminatedConstruct],
            "{marked:?}"
        );
        // The caret there is inside the string: it runs the whole statement, never a part.
        let (sql, r) = marked_sel(marked);
        let at_caret = exec(&pg(&sql), &caret(r.start), &ExecPolicy::largest());
        assert_eq!(texts(&sql, &at_caret), [statement], "{marked:?}");
    }
}

// I3: a start between a backslash and the quote it escapes.
#[test]
fn a_selection_that_cuts_a_backslash_escape_is_refused() {
    for (marked, id, flavor) in [
        (
            "SELECT 'a\\«'; DROP TABLE t; -- ';»\nSELECT 2;",
            DialectId::MySql,
            ClientFlavor::MySqlCli,
        ),
        (
            "SELECT E'a\\«'; DROP TABLE t; -- ';»\nSELECT 2;",
            DialectId::Postgres,
            ClientFlavor::Psql,
        ),
        (
            "SELECT \"a\\«\"; DROP TABLE t; -- \";»\nSELECT 2;",
            DialectId::MySql,
            ClientFlavor::MySqlCli,
        ),
    ] {
        assert_eq!(
            refused(marked, id, flavor),
            [AmbiguityReason::UnterminatedConstruct],
            "{marked:?}"
        );
    }
}

#[test]
fn a_selection_that_cuts_a_word_is_refused_and_a_comments_text_runs_as_selected() {
    for marked in [
        "«DROP TABLE cust»omers_backup;",
        "DELETE FROM t WHERE id = 1«2; SELECT 2;»",
        "SEL«ECT 1»;",
    ] {
        assert_eq!(
            refused(marked, DialectId::Postgres, ClientFlavor::Psql),
            [AmbiguityReason::UnterminatedConstruct],
            "{marked:?}"
        );
    }
    // A statement kept in a `--` comment, selected to be run: plain comments are not tokens
    // a selection cuts.
    let (sql, r) = marked_sel("-- old: «SELECT 1;» kept\nSELECT 2;");
    assert_eq!(
        texts(&sql, &exec(&pg(&sql), &sel(r), &ExecPolicy::default())),
        ["SELECT 1"]
    );
}

// Found by the properties at 1M cases: sent as written from inside a block comment, the
// comment's quote paired with a quote in the COPY rows and the whole selection read as one
// certain statement.
#[test]
fn a_verbatim_selection_never_holds_the_documents_copy_rows() {
    let (sql, r) = marked_sel("/*«' */ SELECT 1;\nCOPY t FROM STDIN;\nit's»\n\\.\n");
    let d = pg(&sql);
    let x = exec(&d, &sel(r.clone()), &selecting(SelectionPolicy::AsOne));
    assert!(
        matches!(
            x,
            RangeResolution::Ambiguous(_) | RangeResolution::UnsupportedClientCommand { .. }
        ),
        "{x:?}"
    );
    // With no quote in the rows the refusal names them.
    let (plain, pr) = marked_sel("/*«x */ SELECT 1;\nCOPY t FROM STDIN;\nrow\n»\\.\n");
    let x = exec(&pg(&plain), &sel(pr), &selecting(SelectionPolicy::AsOne));
    assert!(
        matches!(x, RangeResolution::UnsupportedClientCommand { at } if &plain[at] == "row\n"),
        "{x:?}"
    );
    // Expanded, it is the document's two statements, the COPY with its data apart.
    let x = exec(&d, &sel(r), &selecting(SelectionPolicy::SmartExpand));
    assert_eq!(texts(&sql, &x), ["SELECT 1", "COPY t FROM STDIN"]);
    assert!(matches!(units_of(&x)[1].kind, UnitKind::CopyIn { data } if &sql[data] == "it's\n"));
    // The part of a comment at the selection's start is not sent.
    let (sql, r) = marked_sel("/* a «b */ SELECT 1 /* c» d */;");
    let x = exec(&pg(&sql), &sel(r), &selecting(SelectionPolicy::AsOne));
    assert_eq!(texts(&sql, &x), ["SELECT 1"]);
}

// M5: units that overlap without being the same range.
#[test]
fn a_selection_and_a_caret_in_the_same_statement_are_refused() {
    let sql = "DELETE FROM t WHERE id = 1;\nSELECT 2;";
    let d = pg(sql);
    let both = [
        CaretOrSelection::Selection(TextRange::new(at(0), at(13))),
        CaretOrSelection::Caret(at(after(sql, "WHERE "))),
    ];
    for mode in [SplitMode::Execution, SplitMode::Editor] {
        let r = resolve(&d, &both, &ExecPolicy::default(), mode, None, None);
        let RangeResolution::Ambiguous(b) = &r else {
            panic!("{mode:?}: {r:?}")
        };
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].reason, AmbiguityReason::OverlappingUnits);
        assert_eq!(&sql[b[0].at], "DELETE FROM t");
    }
    // The same range twice is one unit; disjoint ranges both run.
    let twice = [
        CaretOrSelection::Selection(TextRange::new(at(0), at(26))),
        CaretOrSelection::Caret(at(3)),
    ];
    assert_eq!(
        texts(sql, &exec(&d, &twice, &ExecPolicy::default())),
        ["DELETE FROM t WHERE id = 1"]
    );
    let apart = [
        CaretOrSelection::Selection(TextRange::new(at(0), at(13))),
        CaretOrSelection::Caret(at(after(sql, "SELECT"))),
    ];
    assert_eq!(
        texts(sql, &exec(&d, &apart, &ExecPolicy::default())),
        ["DELETE FROM t", "SELECT 2"]
    );
}

// M6: a selection is its own script. Inside a psql conditional branch the caret is refused
// (psql may skip the branch); a selection of the same statement runs as selected.
#[test]
fn a_selection_inside_a_psql_conditional_branch_runs_as_selected() {
    let (sql, r) = marked_sel("\\if false\n«DROP TABLE t;»\n\\endif\nSELECT 2;");
    let d = pg(&sql);
    let at_caret = exec(&d, &caret(r.start + 2), &ExecPolicy::largest());
    assert_eq!(reasons(&at_caret), [AmbiguityReason::ClientCommand]);
    for mode in [SplitMode::Execution, SplitMode::Editor] {
        let r = resolve(
            &d,
            &sel(r.clone()),
            &ExecPolicy::default(),
            mode,
            None,
            None,
        );
        assert_eq!(texts(&sql, &r), ["DROP TABLE t"], "{mode:?}");
    }
}

// M4: under Ask the preview and Run give the same answer wherever Run runs or asks.
#[test]
fn the_preview_never_shows_a_chooser_over_an_immediate_run() {
    // "Whole script" would cross the uncertain statement: Run runs `SELECT 1` at once, and
    // the preview shows that one statement, not a chooser.
    let sql = "SELECT 1; SELECT 2 /*! ; */ ;";
    let d = mysql(sql);
    let run = exec(&d, &caret(7), &ExecPolicy::default());
    assert_eq!(texts(sql, &run), ["SELECT 1"]);
    assert_eq!(editor(&d, &caret(7), &ExecPolicy::default()), run);

    // With more than one runnable candidate both show the same chooser, without the entry
    // that cannot run.
    let three = "SELECT 0 /*! ; */;\nSELECT 1;\nSELECT 2;";
    let d3 = mysql(three);
    let c = caret(after(three, "SELECT 1"));
    let run = exec(&d3, &c, &ExecPolicy::default());
    assert_eq!(
        kinds(&run),
        [CandidateKind::Statement, CandidateKind::FromCaret]
    );
    assert_eq!(editor(&d3, &c, &ExecPolicy::default()), run);

    // Where Run refuses, the preview keeps the uncertain range to draw: in the uncertain
    // statement itself, and when the remembered kind is the one that cannot run.
    let c3 = caret(after(sql, "SELECT 2"));
    assert!(matches!(
        exec(&d, &c3, &ExecPolicy::default()),
        RangeResolution::Ambiguous(_)
    ));
    assert!(matches!(
        editor(&d, &c3, &ExecPolicy::default()),
        RangeResolution::Ask { .. }
    ));
    let remembered = Some(CandidateKind::WholeScript);
    let run = resolve(
        &d,
        &caret(7),
        &ExecPolicy::default(),
        SplitMode::Execution,
        None,
        remembered,
    );
    let preview = resolve(
        &d,
        &caret(7),
        &ExecPolicy::default(),
        SplitMode::Editor,
        None,
        remembered,
    );
    assert!(matches!(run, RangeResolution::Ambiguous(_)));
    let (c, p) = candidates_of(&preview);
    assert_eq!(c[p].kind, CandidateKind::WholeScript);
    assert_eq!(c[p].units.len(), 2);
}

// M7: no invisible character in a label.
#[test]
fn labels_show_zero_width_and_tag_characters() {
    let sql = "SELECT 'a\u{200B}b\u{FEFF}c\u{00AD}d\u{E0001}e\u{200D}f';\nSELECT 2;";
    let r = resolve_at(sql, "SELECT");
    let (c, _) = candidates_of(&r);
    assert_eq!(
        c[0].label,
        "SELECT 'a<U+200B>b<U+FEFF>c<U+00AD>d<U+E0001>e<U+200D>f'"
    );
}

// --- review round 2 -----------------------------------------------------------------------

/// What Run answers for the selection of `marked` under `policy`, the same in both modes.
fn answer(
    marked: &str,
    id: DialectId,
    flavor: ClientFlavor,
    policy: SelectionPolicy,
) -> RangeResolution {
    let (sql, r) = marked_sel(marked);
    let d = doc_in(&sql, id, flavor);
    let x = exec(&d, &sel(r.clone()), &selecting(policy));
    assert_eq!(
        editor(&d, &sel(r), &selecting(policy)),
        x,
        "{marked:?} {policy:?}"
    );
    x
}

// Re-review I1: COPY rows are data wherever their COPY statement is on its line.
#[test]
fn copy_rows_are_never_sent_when_another_item_shares_the_copy_line() {
    for marked in [
        "COPY t FROM STDIN; SELECT 1;«\nDROP TABLE x;»\nDELETE FROM y;\n\\.\n",
        "COPY t FROM STDIN; SELECT «1;\nDROP TABLE x;»\nDELETE FROM y;\n\\.\n",
        "COPY t FROM STDIN; /* c */ ;«\nDROP TABLE x;»\nDELETE FROM y;\n\\.\n",
        "COPY t FROM STDIN; SELECT 1 «-- c\nDROP TABLE x»;\n\\.\n",
        "COPY t FROM STDIN; ;«\nDROP TABLE x;\n»",
    ] {
        for policy in [
            SelectionPolicy::AsSeparate,
            SelectionPolicy::AsOne,
            SelectionPolicy::SmartExpand,
        ] {
            let x = answer(marked, DialectId::Postgres, ClientFlavor::Psql, policy);
            let (sql, _) = marked_sel(marked);
            let sent: Vec<&str> = match &x {
                RangeResolution::Units(u) => u.iter().map(|u| &sql[u.range]).collect(),
                RangeResolution::Ask { .. } => panic!("{marked:?} {policy:?}: {x:?}"),
                _ => Vec::new(),
            };
            assert!(
                !sent
                    .iter()
                    .any(|t| t.contains("DROP") || t.contains("DELETE")),
                "{marked:?} {policy:?}: {sent:?}"
            );
            if policy == SelectionPolicy::AsSeparate {
                assert!(
                    matches!(x, RangeResolution::Ambiguous(_)),
                    "{marked:?}: {x:?}"
                );
            }
        }
    }
}

// Re-review M3.
#[test]
fn a_selection_inside_an_ambiguous_comment_end_is_refused() {
    for marked in [
        "SELECT 1 /*! , \"*/« DROP TABLE t;» -- \" */ ;",
        "SELECT /*+ \"*/« DROP TABLE u;» -- \" */ 2;",
    ] {
        for policy in [
            SelectionPolicy::AsSeparate,
            SelectionPolicy::AsOne,
            SelectionPolicy::SmartExpand,
        ] {
            let x = answer(marked, DialectId::MySql, ClientFlavor::MySqlCli, policy);
            assert_eq!(
                reasons(&x),
                [AmbiguityReason::AmbiguousCommentEnd],
                "{marked:?} {policy:?}"
            );
        }
    }
}

// --- performance (HV-EXEC-01: ≤ 2 ms p95 on a 50k-line file, warm) --------------------------

#[test]
#[ignore = "perf"]
fn resolve_on_50k_lines_within_2_ms_release() {
    if !std::env::var_os("HACIVAT_PERF").is_some_and(|v| v == "1") {
        return;
    }
    // 50,000 lines, 12,500 statements, each with a title comment; and the same script after
    // a psql command on line 1, after which the splitter lexes on its own.
    let body = "-- a note\nSELECT id, name\nFROM customers\nWHERE id = 1;\n".repeat(12_500);
    let with_command = format!("\\set x 1\n{body}");
    let p95 = |mut times: Vec<std::time::Duration>| {
        times.sort();
        times[times.len() * 95 / 100 - 1]
    };
    for (name, sql) in [("plain", &body), ("after a client command", &with_command)] {
        let d = pg(sql);
        let shift = sql.len() - body.len();
        // Warm: both splits exist before Run asks.
        assert_eq!(d.execution_split().statements().count(), 12_500);
        let carets = |policy: ExecPolicy, mode: SplitMode| {
            p95((0..200)
                .map(|n| {
                    let at = caret(shift + (body.len() / 200) * n + 15);
                    let start = std::time::Instant::now();
                    let r = resolve(&d, &at, &policy, mode, None, None);
                    let took = start.elapsed();
                    assert!(
                        matches!(r, RangeResolution::Units(_) | RangeResolution::Ask { .. }),
                        "{name}: {r:?}"
                    );
                    took
                })
                .collect())
        };
        // One statement selected, and three with their comments, all over the script.
        let selections = |lines: usize, mode: SplitMode, policy: ExecPolicy| {
            p95((0..200)
                .map(|n| {
                    let from = shift + 55 * (n * 62 + 1) + 10;
                    let r = from..from + 55 * lines - 10;
                    let start = std::time::Instant::now();
                    let x = resolve(&d, &sel(r), &policy, mode, None, None);
                    let took = start.elapsed();
                    assert_eq!(units_of(&x).len(), lines, "{name}");
                    took
                })
                .collect())
        };
        let statement = carets(ExecPolicy::largest(), SplitMode::Execution);
        let ask = carets(ExecPolicy::default(), SplitMode::Execution);
        let preview = carets(ExecPolicy::default(), SplitMode::Editor);
        let one = selections(1, SplitMode::Execution, ExecPolicy::default());
        let one_preview = selections(1, SplitMode::Editor, ExecPolicy::default());
        let three = selections(3, SplitMode::Execution, ExecPolicy::default());
        let as_one = selections(1, SplitMode::Execution, selecting(SelectionPolicy::AsOne));
        let mut all = vec![
            ("statement", statement),
            ("ask", ask),
            ("preview", preview),
            ("selection of one statement", one),
            ("selection preview", one_preview),
            ("selection of three statements", three),
            ("selection as one", as_one),
        ];
        // A whole-script run spans the client command and is refused there.
        if shift == 0 {
            all.push((
                "whole script",
                carets(ExecPolicy::whole_script(), SplitMode::Execution),
            ));
        }
        println!("resolve p95, {name}: {all:?}");
        for (what, t) in all {
            assert!(t.as_micros() <= 2_000, "{name}, {what}: {t:?}");
        }
    }
}

// ## m3a-f1-followups
// Follow-up F1-a: which unit is a streaming `COPY` is the splitter's answer
// (`StmtHead::copy_stream`, `UnitKind::CopyStream`), the one the Run preview and Run both read.

/// The kind of the one unit Run 2 (the statement at the caret) sends after `needle`.
fn unit_kind_at(doc: &DocSnapshot, sql: &str, needle: &str) -> UnitKind {
    let r = exec(
        doc,
        &caret(after(sql, needle)),
        &inside(InsidePolicy::SmallestStatement),
    );
    let u = units_of(&r);
    assert_eq!(u.len(), 1, "{r:?}");
    u[0].kind
}

/// The splitter's head is the server's: `)COPY` is no `COPY` (PostgreSQL rejects it as a
/// syntax error at `)`, so its "rows" are never read as data). They are SQL to the splitter,
/// and no caret on them runs anything: the `\.` line is a client command.
#[test]
fn a_copy_is_recognised_by_the_statement_head() {
    let sql = "SELECT 1;)COPY t FROM STDIN;\na\tb\n\\.\nSELECT 2;";
    let doc = pg(sql);
    assert_eq!(unit_kind_at(&doc, sql, ")CO"), UnitKind::Statement);
    for i in span(sql, "a\tb").start..=after(sql, "\\.") {
        let r = exec(&doc, &caret(i), &ExecPolicy::default());
        assert!(
            !matches!(r, RangeResolution::Units(_) | RangeResolution::Ask { .. }),
            "{i}: {r:?}"
        );
    }
    assert!(!matches!(
        exec(&doc, &caret(0), &ExecPolicy::whole_script()),
        RangeResolution::Units(_) | RangeResolution::Ask { .. }
    ));

    // A `COPY` that streams, whatever follows it, is the splitter's `CopyStream`.
    for (sql, needle) in [
        ("COPY (SELECT 1) TO STDOUT;", "COPY"),
        ("COPY t TO stdout;\nSELECT 1;", "COPY"),
        ("/* c */ COPY t FROM STDIN", "COPY"),
        ("copy t (a) from stdin with (format csv)", "copy"),
        ("SELECT 1;\n  COPY t FROM STDIN", "COPY"),
    ] {
        assert_eq!(
            unit_kind_at(&pg(sql), sql, needle),
            UnitKind::CopyStream,
            "{sql}"
        );
    }
    // Under a flavor without psql's inline rows, `COPY … FROM STDIN;` still streams.
    let sql = "COPY t FROM STDIN;";
    let doc = doc_in(sql, DialectId::Postgres, ClientFlavor::None);
    assert_eq!(unit_kind_at(&doc, sql, "COPY"), UnitKind::CopyStream);
    // With psql's rows it stays `CopyIn`.
    let sql = "COPY t FROM STDIN;\n1\n\\.\n";
    assert!(matches!(
        unit_kind_at(&pg(sql), sql, "COPY"),
        UnitKind::CopyIn { .. }
    ));

    // Not a streaming `COPY`: the head is another word, or `STDIN` is in a string or comment.
    for (sql, needle) in [
        ("SELECT copy, stdin FROM t;", "SEL"),
        ("COPY t FROM '/srv/stdin';", "COPY"),
        ("COPY t TO '/srv/t.csv' /* STDOUT */;", "COPY"),
        ("(COPY t FROM STDIN);", "(CO"),
        ("EXPLAIN COPY t TO STDOUT;", "EXP"),
    ] {
        assert_eq!(
            unit_kind_at(&pg(sql), sql, needle),
            UnitKind::Statement,
            "{sql}"
        );
    }
}

/// A selection of a `COPY … FROM STDIN` without its rows never runs as a plain statement.
#[test]
fn a_copy_selected_without_its_rows_is_never_a_plain_statement() {
    let sql = "SELECT 1;\nCOPY t FROM STDIN;\n1\n\\.\nSELECT 2;";
    for r in [
        span(sql, "COPY t FROM STDIN"),
        span(sql, "COPY t FROM STDIN;"),
        span(sql, "SELECT 1;\nCOPY t FROM STDIN;"),
    ] {
        match resolve_sel(sql, r.clone()) {
            RangeResolution::Units(u) => assert!(
                u.iter()
                    .any(|u| matches!(u.kind, UnitKind::CopyStream | UnitKind::CopyIn { .. })),
                "{r:?}: {u:?}"
            ),
            RangeResolution::Ask { .. } => panic!("{r:?}: a selection never asks"),
            _ => {}
        }
    }
}
