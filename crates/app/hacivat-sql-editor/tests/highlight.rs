//! Lexical highlighting through the language service (UI-EDIT-11 L1; Task 24): token classes,
//! dollar tags, executable comments, client and `COPY` regions, rebasing through the editor's
//! change log, and the token words for movement.

mod support;

use gpui::TestAppContext;
use hacivat_ui_editor::HighlightClass as H;
use support::{Doc, mysql, mysql_at, pg};

/// UI-EDIT-11 case 1: `SELECT "select" FROM t` → the first `SELECT` a keyword, `"select"` a
/// quoted identifier.
#[gpui::test]
fn ui_edit_11_cases_1_and_3(cx: &mut TestAppContext) {
    let d = Doc::analysed(cx, "SELECT \"select\" FROM t", pg());
    assert_eq!(d.classes_of("SELECT", 0), [H::Keyword]);
    assert_eq!(d.classes_of("\"select\"", 0), [H::QuotedIdent]);
    assert_eq!(d.classes_of("FROM", 0), [H::Keyword]);
    assert_eq!(d.classes_of("t", 1), [H::Ident]);

    // Case 3: MySQL `SELECT 1 /*!80000 , 2 */` → `, 2` is code on the executable-comment
    // well: the whole comment carries `ExecComment` (the well), its tokens their own classes,
    // and its delimiters no token class (they take the well's comment ink, not italic).
    let d = Doc::analysed(cx, "SELECT 1 /*!80000 , 2 */", mysql());
    assert_eq!(d.classes_of("/*!80000 , 2 */", 0), [H::ExecComment]);
    assert_eq!(d.classes_of(",", 0), [H::Punct]);
    assert_eq!(d.classes_of("2", 0), [H::Number]);
    assert!(d.classes_of("/*!80000", 0).is_empty());
    assert!(d.classes_of("*/", 0).is_empty());
    assert_eq!(d.classes_of("1", 0), [H::Number]);
}

#[gpui::test]
fn dollar_strings_split_their_tags_and_e_strings_are_one_string(cx: &mut TestAppContext) {
    let d = Doc::analysed(cx, "SELECT $fn$ it's; $x$ $fn$, E'it\\'s', 'a''b';", pg());
    assert_eq!(d.classes_of("$fn$", 0), [H::DollarTag]);
    assert_eq!(d.classes_of(" it's; $x$ ", 0), [H::String]);
    assert_eq!(d.classes_of("$fn$", 1), [H::DollarTag]);
    assert_eq!(d.classes_of("E'it\\'s'", 0), [H::String]);
    assert_eq!(d.classes_of("'a''b'", 0), [H::String]);
    // Nothing inside the dollar body is lexed as code.
    let body = d.text().find(" it's;").unwrap()..d.text().find(" $fn$,").unwrap();
    let inside: Vec<_> = d
        .all_marks()
        .into_iter()
        .filter(|(r, _)| body.start < r.start && r.start < body.end)
        .collect();
    assert!(inside.is_empty(), "{inside:?}");

    // An empty tag and an unterminated dollar string (its colour shows how far it runs).
    let d = Doc::analysed(cx, "SELECT $$x$$; SELECT $t$ open", pg());
    assert_eq!(d.classes_of("$$", 0), [H::DollarTag]);
    assert_eq!(d.classes_of("x", 0), [H::String]);
    assert_eq!(d.classes_of("$$", 1), [H::DollarTag]);
    assert_eq!(d.classes_of("$t$", 0), [H::DollarTag]);
    assert_eq!(d.classes_of(" open", 0), [H::String]);
}

#[gpui::test]
fn functions_params_hints_and_errors_get_their_classes(cx: &mut TestAppContext) {
    let d = Doc::analysed(
        cx,
        "SELECT count(*), coalesce(a, $1) FROM t WHERE b IN(1) AND c = 1abc",
        pg(),
    );
    assert_eq!(d.classes_of("count", 0), [H::Function]);
    // A non-reserved keyword directly before `(` is a function name; a reserved one is not.
    assert_eq!(d.classes_of("coalesce", 0), [H::Function]);
    assert_eq!(d.classes_of("IN", 0), [H::Keyword]);
    assert_eq!(d.classes_of("$1", 0), [H::Param]);
    assert_eq!(d.classes_of("(", 0), [H::Punct]);
    assert_eq!(d.classes_of("=", 0), [H::Operator]);
    // `1abc`: trailing junk the server rejects.
    assert_eq!(d.classes_of("1", 2), [H::ErrorToken]);

    let d = Doc::analysed(cx, "SELECT /*+ BKA(t) */ a FROM t -- note\n", mysql());
    assert_eq!(d.classes_of("/*+ BKA(t) */", 0), [H::Hint]);
    assert_eq!(d.classes_of("-- note", 0), [H::Comment]);
    assert_eq!(d.classes_of("a", 0), [H::Ident]);
}

/// The gate decides: a version the server is below makes the body a plain comment; an
/// unversioned or unknown-gate comment runs, so it is code on the well.
#[gpui::test]
fn an_inactive_executable_comment_is_a_comment(cx: &mut TestAppContext) {
    let d = Doc::analysed(cx, "SELECT 1 /*!80000 , 2 */", mysql_at(5, 7, 44));
    assert_eq!(d.classes_of("/*!80000 , 2 */", 0), [H::Comment]);
    assert!(d.classes_of("2", 0).is_empty());

    let d = Doc::analysed(
        cx,
        "SELECT 1 /*!80000 , 2 */ /*! , 3 */",
        mysql_at(8, 0, 36),
    );
    assert_eq!(d.classes_of("/*!80000 , 2 */", 0), [H::ExecComment]);
    assert_eq!(d.classes_of("/*! , 3 */", 0), [H::ExecComment]);
    assert_eq!(d.classes_of("3", 0), [H::Number]);
}

/// Client commands paint as commands and `COPY … FROM STDIN` rows as data (the ledger's T24
/// rule): a quote in the rows colours nothing inside them as SQL.
#[gpui::test]
fn client_and_copy_regions_paint_as_command_and_data(cx: &mut TestAppContext) {
    let text = "\\set x 1\nCOPY t FROM STDIN;\na'b\tSELECT\n\\.\n";
    let d = Doc::analysed(cx, text, pg());
    assert_eq!(d.classes_of("\\set x 1", 0), [H::MetaCommand]);
    assert_eq!(d.classes_of("a'b\tSELECT\n", 0), [H::String]);
    assert_eq!(d.classes_of("\\.", 0), [H::MetaCommand]);
    assert_eq!(d.classes_of("COPY", 0), [H::Keyword]);
    let rows = text.find("a'b").unwrap()..text.find("\\.").unwrap();
    let marks = d.all_marks();
    let inside: Vec<_> = marks
        .iter()
        .filter(|(r, _)| r.start < rows.end && rows.start < r.end && *r != rows)
        .collect();
    assert!(
        inside.is_empty(),
        "only the data class over the rows: {inside:?}"
    );
    // The marks never overlap one another outside the executable-comment well.
    for w in marks.windows(2) {
        assert!(w[0].0.end <= w[1].0.start, "{marks:?}");
    }
}

/// R-M3-22: marks of an older analysis are rebased through the editor's change log; a token
/// an edit touched paints plain until the next analysis lands.
#[gpui::test]
fn touched_range_paints_plain_until_the_next_analysis(cx: &mut TestAppContext) {
    let mut d = Doc::analysed(cx, "SELECT 1;", pg());
    let before = d.svc.analyses();
    d.edit(cx, 3..3, "X");
    assert_eq!(d.text(), "SELXECT 1;");
    // Nothing was analysed for the edit yet.
    assert_eq!(d.svc.analyses(), before);
    let marks = d.all_marks();
    assert!(
        marks.iter().all(|(r, _)| r.start >= 7),
        "the touched word is plain: {marks:?}"
    );
    assert_eq!(d.classes_of("1", 0), [H::Number]);
    assert_eq!(d.classes_of(";", 0), [H::Punct]);

    // An insertion above shifts the untouched tokens.
    d.edit(cx, 0..0, "-- c\n");
    assert_eq!(d.classes_of("-- c", 0), Vec::<H>::new());
    assert_eq!(d.classes_of("1", 0), [H::Number]);

    cx.run_until_parked();
    assert_eq!(d.classes_of("SELXECT", 0), [H::Ident]);
    assert_eq!(d.classes_of("-- c", 0), [H::Comment]);
}

/// An analysis older than the editor's change log cannot be rebased: nothing is painted rather
/// than a wrong colour.
#[gpui::test]
fn an_analysis_older_than_the_log_paints_nothing(cx: &mut TestAppContext) {
    let mut d = Doc::analysed(cx, "SELECT 1;", pg());
    for _ in 0..300 {
        d.edit(cx, 0..0, " ");
    }
    assert!(d.all_marks().is_empty());
    cx.run_until_parked();
    assert_eq!(d.classes_of("SELECT", 0), [H::Keyword]);
}

/// The token oracle (UI-EDIT-06 words): one word per token at the current version, a string or
/// dollar-quoted body whole (Task 25's double click reads its quotes), comments left to the
/// character classes; `None` while the analysis is stale.
#[gpui::test]
fn token_words_come_from_the_current_analysis(cx: &mut TestAppContext) {
    let mut d = Doc::analysed(
        cx,
        "SELECT 'a b', \"x y\", $q$ z $q$::text -- c d\nFROM t",
        pg(),
    );
    assert_eq!(
        d.words(0).unwrap(),
        [
            "SELECT",
            "'a b'",
            ",",
            "\"x y\"",
            ",",
            "$q$ z $q$",
            "::",
            "text"
        ]
    );
    assert_eq!(d.words(1).unwrap(), ["FROM", "t"]);
    d.edit(cx, 0..0, "\n");
    assert_eq!(d.words(1), None, "stale: the character classes apply");
    cx.run_until_parked();
    assert_eq!(d.words(2).unwrap(), ["FROM", "t"]);

    let d = Doc::analysed(cx, "SELECT /*!80000 1, 2 */ x", mysql());
    assert_eq!(
        d.words(0).unwrap(),
        ["SELECT", "/*!80000", "1", ",", "2", "*/", "x"]
    );
}
