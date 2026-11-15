//! The Run preview tells the truth (UI-EDIT-21; R-M3-8, R-M3-24; Task 24): it answers only at
//! exactly the editor's version, shows a range as runnable only when Run sends exactly that
//! range, and draws everything Run refuses as uncertain.

mod support;

use gpui::TestAppContext;
use hacivat_sql_editor::{preview_of, statement_spans};
use hacivat_ui_editor::{
    Confidence, DocContext, ExecAction, ExecAt, LanguageService, PreviewState, Tier,
};
use proptest::prelude::*;
use smallvec::smallvec;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId};
use sql_split::exec_range::{
    CandidateKind, CaretOrSelection, ExecPolicy, InsidePolicy, RangeResolution, TextRange,
    TextSize, UnitKind, UnitSpan, resolve,
};
use sql_split::{SplitMode, SqlDocument};
use support::{DOC, Doc, caret, mysql, pg, selection};

#[gpui::test]
fn stale_snapshot_never_yields_a_preview(cx: &mut TestAppContext) {
    let mut d = Doc::open(cx, "SELECT 1;\nSELECT 2;", Some(pg()));
    assert!(
        d.preview(&caret(3), ExecAction::Execute1).is_none(),
        "no analysis yet"
    );
    cx.run_until_parked();
    assert!(d.preview(&caret(3), ExecAction::Execute1).is_some());
    d.edit(cx, 0..0, "\n");
    assert!(
        d.preview(&caret(3), ExecAction::Execute1).is_none(),
        "the analysis is one version behind"
    );
    let v = d.version();
    for wrong in [v.wrapping_sub(1), v + 1, 7] {
        assert!(
            d.svc
                .exec_preview(DOC, wrong, &caret(3), ExecAction::Execute1)
                .is_none()
        );
    }
    cx.run_until_parked();
    let p = d.preview(&caret(3), ExecAction::Execute1).expect("current");
    assert_eq!(p.state, PreviewState::AskPreselected);
    assert_eq!(d.texts(&p), ["SELECT 1"]);
}

/// D-011: one candidate runs directly, so the preview is `Units`, not a chooser.
#[gpui::test]
fn single_candidate_preview_is_units(cx: &mut TestAppContext) {
    let d = Doc::analysed(cx, "-- title\nSELECT 1;", pg());
    let p = d.preview(&caret(12), ExecAction::Execute1).unwrap();
    assert_eq!(p.state, PreviewState::Units);
    assert_eq!(d.texts(&p), ["SELECT 1"], "no terminator, no title comment");
}

#[gpui::test]
fn ask_previews_the_preselected_candidate(cx: &mut TestAppContext) {
    let d = Doc::analysed(cx, "SELECT 1;\nSELECT 2;", pg());
    let p = d.preview(&caret(13), ExecAction::Execute1).unwrap();
    assert_eq!(p.state, PreviewState::AskPreselected);
    assert_eq!(d.texts(&p), ["SELECT 2"]);
    // The console's remembered kind is where the chooser opens, so the preview follows it.
    d.svc
        .set_remembered_kind(DOC, Some(CandidateKind::WholeScript));
    let p = d.preview(&caret(13), ExecAction::Execute1).unwrap();
    assert_eq!(p.state, PreviewState::AskPreselected);
    assert_eq!(d.texts(&p), ["SELECT 1", "SELECT 2"]);
}

#[gpui::test]
fn run_2_and_run_3_use_their_policies(cx: &mut TestAppContext) {
    let d = Doc::analysed(cx, "SELECT 1;\nSELECT 2;", pg());
    let p = d.preview(&caret(13), ExecAction::Execute2).unwrap();
    assert_eq!(
        (p.state, d.texts(&p)),
        (PreviewState::Units, vec!["SELECT 2".into()])
    );
    let p = d.preview(&caret(13), ExecAction::Execute3).unwrap();
    assert_eq!(p.state, PreviewState::Units);
    assert_eq!(d.texts(&p), ["SELECT 1", "SELECT 2"]);
    let p = d.preview(&caret(9), ExecAction::Execute2).unwrap();
    assert_eq!(
        d.texts(&p),
        ["SELECT 1"],
        "a caret after `;` means that statement"
    );
}

#[gpui::test]
fn a_selection_previews_its_units_or_is_uncertain(cx: &mut TestAppContext) {
    let text = "SELECT 1; SELECT 'a;b'; SELECT 3;";
    let d = Doc::analysed(cx, text, pg());
    let p = d
        .preview(&selection(0..text.len()), ExecAction::Execute1)
        .unwrap();
    assert_eq!(p.state, PreviewState::Units);
    assert_eq!(d.texts(&p), ["SELECT 1", "SELECT 'a;b'", "SELECT 3"]);
    // Starting inside the string: the server would read its rest as code.
    let inside = text.find("a;b").unwrap() + 2;
    let p = d
        .preview(&selection(inside..text.len()), ExecAction::Execute1)
        .unwrap();
    assert_eq!(p.state, PreviewState::Uncertain);
    let nothing = d.preview(&selection(9..10), ExecAction::Execute1).unwrap();
    assert_eq!(nothing.state, PreviewState::Nothing);
    assert!(nothing.ranges.is_empty());
}

/// R-M3-24: a heuristic boundary is drawn uncertain, never as a range Run sends.
#[gpui::test]
fn heuristic_boundary_previews_uncertain(cx: &mut TestAppContext) {
    // A `;` inside an executable comment.
    let d = Doc::analysed(cx, "SELECT /*! ; */ 1;", mysql());
    let p = d.preview(&caret(2), ExecAction::Execute1).unwrap();
    assert_eq!(p.state, PreviewState::Uncertain);
    assert!(!p.ranges.is_empty(), "the uncertain range is drawn");

    // The editor's missing-semicolon guess.
    let d = Doc::analysed(cx, "SELECT 1\nSELECT 2;", pg());
    let p = d.preview(&caret(2), ExecAction::Execute1).unwrap();
    assert_eq!(p.state, PreviewState::Uncertain);

    // An unterminated string after a good statement: the statement still runs; the tail and
    // the whole script do not.
    let d = Doc::analysed(cx, "SELECT 1;\nSELECT 'x", pg());
    assert_eq!(
        d.preview(&caret(2), ExecAction::Execute1).unwrap().state,
        PreviewState::Units
    );
    assert_eq!(
        d.preview(&caret(14), ExecAction::Execute1).unwrap().state,
        PreviewState::Uncertain
    );
    assert_eq!(
        d.preview(&caret(2), ExecAction::Execute3).unwrap().state,
        PreviewState::Uncertain
    );
}

/// What M3a does not send is never drawn as runnable: a client command Hacivat does not run,
/// and `COPY … FROM STDIN`.
#[gpui::test]
fn unsupported_commands_and_copy_never_preview_as_runnable(cx: &mut TestAppContext) {
    let d = Doc::analysed(cx, "\\echo hi\nSELECT 1;", pg());
    let p = d.preview(&caret(2), ExecAction::Execute1).unwrap();
    assert_eq!(p.state, PreviewState::Uncertain);
    assert_eq!(d.texts(&p), ["\\echo hi"]);
    let p = d.preview(&caret(12), ExecAction::Execute3).unwrap();
    assert_eq!(
        p.state,
        PreviewState::Uncertain,
        "the script spans the command"
    );

    let d = Doc::analysed(cx, "COPY t FROM STDIN;\n1\n\\.\n", pg());
    let p = d.preview(&caret(2), ExecAction::Execute1).unwrap();
    assert_eq!(p.state, PreviewState::Uncertain);
    assert_eq!(d.texts(&p), ["COPY t FROM STDIN"]);
}

#[gpui::test]
fn a_disconnected_console_previews_no_connection(cx: &mut TestAppContext) {
    let d = Doc::analysed(cx, "SELECT 1;", pg());
    let offline = DocContext {
        connected: false,
        tier: Tier::Full,
    };
    cx.update(|cx| d.svc.set_context(DOC, offline, cx));
    let p = d.preview(&caret(2), ExecAction::Execute1).unwrap();
    assert_eq!(p.state, PreviewState::NoConnection);
    assert_eq!(d.texts(&p), ["SELECT 1"], "the same geometry, neutral");

    // Before the console's SQL context is known: no preview while connected (the dialect
    // decides the split), `NoConnection` while disconnected.
    let d = Doc::open(cx, "SELECT 1;", None);
    cx.run_until_parked();
    assert!(d.preview(&caret(2), ExecAction::Execute1).is_none());
    cx.update(|cx| d.svc.set_context(DOC, offline, cx));
    let p = d.preview(&caret(2), ExecAction::Execute1).unwrap();
    assert_eq!(p.state, PreviewState::NoConnection);
}

// -------------------------------------------------------------------------------------------
// Statements (the gutter's spans)

/// `(strict text, confidence)` of the statements in view of `r` at the editor's version.
fn statements(d: &Doc, r: std::ops::Range<usize>) -> Vec<(String, Confidence)> {
    let text = d.text();
    d.svc
        .statements(DOC, d.version(), r)
        .iter()
        .map(|s| (text[s.strict.clone()].to_owned(), s.confidence))
        .collect()
}

/// A statement is `Exact` only where Run would send it as it is.
#[gpui::test]
fn statements_are_exact_only_where_run_sends_them(cx: &mut TestAppContext) {
    let text = "SELECT 1;\nSELECT 2\nSELECT 3;";
    let d = Doc::analysed(cx, text, pg());
    let all = statements(&d, 0..text.len());
    assert_eq!(all[0], ("SELECT 1".into(), Confidence::Exact));
    assert!(
        all[1..].iter().all(|(_, c)| *c == Confidence::Heuristic),
        "{all:?}"
    );
    // Only the statements in view.
    assert_eq!(
        statements(&d, 0..4),
        [("SELECT 1".into(), Confidence::Exact)]
    );
    let s = &d.svc.statements(DOC, d.version(), 0..4)[0];
    assert_eq!((s.smart.clone(), s.header), (0..9, 0));

    let d = Doc::analysed(cx, "SELECT /*! ; */ 1;\nSELECT 2;", mysql());
    let all = statements(&d, 0..30);
    assert_eq!(all[0].1, Confidence::Heuristic, "{all:?}");
    assert_eq!(all.last().unwrap(), &("SELECT 2".into(), Confidence::Exact));
}

/// When the editor is ahead, statements are mapped through the changes as containers, and one
/// an edit touched is `Heuristic` until the next analysis.
#[gpui::test]
fn statements_are_rebased_and_touched_ones_turn_heuristic(cx: &mut TestAppContext) {
    let mut d = Doc::analysed(cx, "SELECT 1;\n\nSELECT 2;", pg());
    d.edit(cx, 0..0, "  ");
    let all = statements(&d, 0..d.text().len());
    assert_eq!(
        all,
        [
            ("  SELECT 1".into(), Confidence::Heuristic),
            ("SELECT 2".into(), Confidence::Exact)
        ],
        "an insertion at the edge grows the first statement and marks it"
    );
    d.edit(cx, 16..16, "X");
    assert_eq!(d.text(), "  SELECT 1;\n\nSELXECT 2;");
    let all = statements(&d, 0..d.text().len());
    assert_eq!(all[1], ("SELXECT 2".into(), Confidence::Heuristic));
    cx.run_until_parked();
    let all = statements(&d, 0..d.text().len());
    assert_eq!(
        all,
        [
            ("SELECT 1".into(), Confidence::Exact),
            ("SELXECT 2".into(), Confidence::Exact)
        ]
    );

    // A new SQL context: nothing is certain until it is analysed.
    let (ctx, flavor) = mysql();
    d.svc.set_sql_context(DOC, ctx, flavor);
    let all = statements(&d, 0..d.text().len());
    assert!(
        all.iter().all(|(_, c)| *c == Confidence::Heuristic),
        "{all:?}"
    );
    cx.run_until_parked();
    let all = statements(&d, 0..d.text().len());
    assert!(all.iter().all(|(_, c)| *c == Confidence::Exact), "{all:?}");

    // Older than the changes known: nothing rather than a wrong range.
    for _ in 0..300 {
        d.edit(cx, 0..0, " ");
    }
    assert!(statements(&d, 0..d.text().len()).is_empty());
}

// -------------------------------------------------------------------------------------------
// The truth property (R-M3-8)

/// Pieces of scripts per dialect, chosen to make every kind of boundary: certain statements,
/// quotes holding `;`, unterminated constructs, executable comments, client commands, `COPY`
/// data, blocks and missing semicolons.
fn pieces(id: DialectId) -> &'static [&'static str] {
    match id {
        DialectId::Postgres => &[
            "SELECT 1;",
            "SELECT 'a;b';",
            "SELECT $$ x; y $$;",
            "\nSELECT 2\n",
            "-- c ;\n",
            "/* x; */",
            "SELECT \"q;\" FROM t;",
            "COPY t FROM STDIN;\na\tb\n\\.\n",
            "\\set x 1\n",
            "\\echo hi\n",
            "BEGIN;",
            "END;",
            "SELECT E'\\';';",
            "'",
            "$$",
            "(",
            ")",
            ";",
            " ",
            "\n\n",
            "CREATE FUNCTION f() RETURNS int AS $f$ SELECT 1; $f$ LANGUAGE sql;",
            "WITH a AS (SELECT 1) SELECT * FROM a;",
            "müşteri",
            "/*",
            "BEGIN ",
            "CASE WHEN x THEN 1 END",
            "END ",
        ],
        DialectId::MySql => &[
            "SELECT 1;",
            "/*!50000 SELECT 2 */;",
            "SELECT /*! ; */ 1;",
            "DELIMITER $$\n",
            "DELIMITER ;\n",
            "CREATE PROCEDURE p() BEGIN SELECT 1; END$$\n",
            "SELECT `a;b`;",
            "# c\n",
            "SELECT 'x\\';y';",
            "\nSELECT 3\n",
            "/*",
            "BEGIN ",
            "END ",
            "'",
            ";",
            " ",
            "\n",
        ],
        _ => &[
            "SELECT 1;",
            ".mode csv\n",
            "CREATE TRIGGER t AFTER INSERT ON x BEGIN SELECT 1; END;",
            "SELECT [a;b];",
            "SELECT 'a;b';",
            "\nSELECT 2\n",
            "'",
            ";",
            " ",
        ],
    }
}

fn sql_docs() -> impl Strategy<Value = (DialectId, ClientFlavor, String)> {
    prop_oneof![
        Just((DialectId::Postgres, ClientFlavor::Psql)),
        Just((DialectId::MySql, ClientFlavor::MySqlCli)),
        Just((DialectId::Sqlite, ClientFlavor::Sqlite3)),
    ]
    .prop_flat_map(|(id, flavor)| {
        let n = pieces(id).len();
        proptest::collection::vec(0..n, 0..12)
            .prop_map(move |ix| (id, flavor, ix.iter().map(|&i| pieces(id)[i]).collect()))
    })
}

/// Where Run is asked, as fractions of the text: a caret, a selection, or several carets.
#[derive(Debug, Clone)]
enum At {
    Caret(u16),
    Selection(u16, u16),
    Carets(u16, u16),
}

fn carets() -> impl Strategy<Value = At> {
    prop_oneof![
        any::<u16>().prop_map(At::Caret),
        (any::<u16>(), any::<u16>()).prop_map(|(a, b)| At::Selection(a, b)),
        (any::<u16>(), any::<u16>()).prop_map(|(a, b)| At::Carets(a, b)),
    ]
}

fn offset(text: &str, f: u16) -> usize {
    let mut at = text.len() * usize::from(f) / usize::from(u16::MAX);
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The editor's `ExecAt` and the same place as `exec_range` points.
fn place(text: &str, at: &At) -> (ExecAt, Vec<CaretOrSelection>) {
    let size = |n: usize| TextSize::from(u32::try_from(n).unwrap());
    match *at {
        At::Caret(f) => {
            let c = offset(text, f);
            (
                ExecAt::Carets(smallvec![c]),
                vec![CaretOrSelection::Caret(size(c))],
            )
        }
        At::Carets(f, g) => {
            let (a, b) = (offset(text, f), offset(text, g));
            (
                ExecAt::Carets(smallvec![a, b]),
                vec![
                    CaretOrSelection::Caret(size(a)),
                    CaretOrSelection::Caret(size(b)),
                ],
            )
        }
        At::Selection(f, g) => {
            let (a, b) = (offset(text, f), offset(text, g));
            let (a, b) = (a.min(b), a.max(b));
            (
                ExecAt::Selections(smallvec![a..b]),
                vec![CaretOrSelection::Selection(TextRange::new(
                    size(a),
                    size(b),
                ))],
            )
        }
    }
}

/// Run's policy for each action (`hacivat_exec::policy_for`).
fn policy(action: ExecAction) -> ExecPolicy {
    match action {
        ExecAction::Execute1 => ExecPolicy::default(),
        ExecAction::Execute2 => ExecPolicy::largest(),
        ExecAction::Execute3 => ExecPolicy::whole_script(),
    }
}

/// Units M3a sends as they are (not `COPY … FROM STDIN`, not a verbatim selection).
fn sent(units: &[UnitSpan]) -> bool {
    units
        .iter()
        .all(|u| !matches!(u.kind, UnitKind::CopyIn { .. } | UnitKind::Verbatim))
}

fn ranges(units: &[UnitSpan]) -> Vec<std::ops::Range<usize>> {
    units
        .iter()
        .map(|u| usize::from(u.range.start())..usize::from(u.range.end()))
        .collect()
}

/// One case: the preview against what execution resolves for the same place.
fn preview_matches_resolve(
    (id, flavor, text): &(DialectId, ClientFlavor, String),
    at: &At,
    remembered: Option<CandidateKind>,
) -> Result<(), TestCaseError> {
    let doc = SqlDocument::new(text.clone(), DialectCtx::new(*id, None), *flavor).snapshot();
    let (exec_at, points) = place(text, at);
    for action in [
        ExecAction::Execute1,
        ExecAction::Execute2,
        ExecAction::Execute3,
    ] {
        let p = preview_of(&doc, &exec_at, action, true, remembered);
        let run = resolve(
            &doc,
            &points,
            &policy(action),
            SplitMode::Execution,
            None,
            remembered,
        );
        match &run {
            RangeResolution::Units(u) if sent(u) => {
                prop_assert_eq!(p.state, PreviewState::Units, "{:?}", run);
                prop_assert_eq!(p.ranges.to_vec(), ranges(u));
            }
            RangeResolution::Ask {
                candidates,
                preselect,
            } if sent(&candidates[*preselect].units) => {
                prop_assert_eq!(p.state, PreviewState::AskPreselected, "{:?}", run);
                prop_assert_eq!(p.ranges.to_vec(), ranges(&candidates[*preselect].units));
            }
            RangeResolution::NothingToRun => {
                prop_assert_eq!(p.state, PreviewState::Nothing);
                prop_assert!(p.ranges.is_empty());
            }
            // Run sends nothing: the preview never says it does.
            _ => prop_assert_eq!(p.state, PreviewState::Uncertain, "{:?}", run),
        }
        // Exact items: the editor-mode answer is the same (Task 13's agreement, through the
        // adapter's mapping).
        if matches!(p.state, PreviewState::Units | PreviewState::AskPreselected) {
            let editor = resolve(
                &doc,
                &points,
                &policy(action),
                SplitMode::Editor,
                None,
                remembered,
            );
            prop_assert_eq!(&editor, &run);
        }
        // Offline: the same geometry, never a runnable state.
        let offline = preview_of(&doc, &exec_at, action, false, remembered);
        prop_assert_eq!(offline.state, PreviewState::NoConnection);
        prop_assert_eq!(offline.ranges, p.ranges);
    }
    Ok(())
}

/// 512 cases by default; `PROPTEST_CASES` raises it for a long run.
fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(512)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases()))]

    #[test]
    fn preview_equals_execution_for_exact_items(doc in sql_docs(), c in carets()) {
        preview_matches_resolve(&doc, &c, None)?;
    }

    /// The gutter's `Exact` is a promise: Run at that statement sends exactly its strict range.
    #[test]
    fn exact_statement_spans_are_what_execution_runs((id, flavor, text) in sql_docs()) {
        let doc = SqlDocument::new(text.clone(), DialectCtx::new(id, None), flavor).snapshot();
        let smallest = ExecPolicy {
            inside: InsidePolicy::SmallestStatement,
            ..ExecPolicy::default()
        };
        for s in statement_spans(&doc) {
            if s.confidence != Confidence::Exact {
                continue;
            }
            // A caret inside the statement (its start may touch the one before).
            let inside = text[s.strict.clone()]
                .chars()
                .next()
                .map_or(s.strict.start, |c| s.strict.start + c.len_utf8())
                .min(s.strict.end);
            let at = [CaretOrSelection::Caret(TextSize::from(u32::try_from(inside).unwrap()))];
            let run = resolve(&doc, &at, &smallest, SplitMode::Execution, None, None);
            let RangeResolution::Units(u) = &run else {
                return Err(TestCaseError::fail(format!("{s:?} is Exact but Run answers {run:?}")));
            };
            prop_assert_eq!(ranges(u), vec![s.strict.clone()], "{:?}", s);
        }
    }

    #[test]
    fn preview_follows_the_remembered_kind(
        doc in sql_docs(),
        c in carets(),
        kind in prop_oneof![
            Just(CandidateKind::Statement),
            Just(CandidateKind::FromCaret),
            Just(CandidateKind::WholeScript),
        ],
    ) {
        preview_matches_resolve(&doc, &c, Some(kind))?;
    }
}
