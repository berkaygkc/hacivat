//! M3c Task 32: the foldable ranges of a script from its split (UI-EDIT-10 candidates; NOTES
//! surface 15): statements, blocks, block comments and `-- region` markers, each spanning at
//! least three lines (ruling M3c-43).

mod support;

use gpui::TestAppContext;
use hacivat_sql_editor::folding_ranges;
use hacivat_text::{FoldKind, FoldRange};
use hacivat_ui_editor::LanguageService as _;
use sql_split::SqlDocument;
use support::{DOC, Doc, mysql, pg};

fn ranges(
    text: &str,
    (ctx, flavor): (sql_dialect::DialectCtx, sql_dialect::ClientFlavor),
) -> Vec<FoldRange> {
    let doc = SqlDocument::new(text.to_string(), ctx, flavor).snapshot();
    folding_ranges(&doc)
}

/// The kinds and texts of the ranges.
fn texts(text: &str, r: &[FoldRange]) -> Vec<(FoldKind, String)> {
    r.iter()
        .map(|f| (f.kind, text[f.range.clone()].to_string()))
        .collect()
}

fn at(text: &str, needle: &str) -> usize {
    text.find(needle).unwrap_or_else(|| panic!("{needle:?}"))
}

#[test]
fn sql_folding_ranges_from_the_split() {
    let text = "\
-- region totals
SELECT id,
  total
FROM orders;
SELECT 1;
SELECT a
FROM b;
-- endregion
/* one
   two
   three */
SELECT *
FROM (
  SELECT 1
  FROM u
) s
WHERE x
  AND y;
/* a
   b */
";
    let r = ranges(text, pg());
    let got = texts(text, &r);
    // Sorted by start, the outer one first on a shared start.
    let starts: Vec<usize> = r.iter().map(|f| f.range.start).collect();
    let mut sorted = starts.clone();
    sorted.sort_unstable();
    assert_eq!(starts, sorted);
    assert!(
        got.contains(&(
            FoldKind::Region,
            text[at(text, "-- region")..at(text, "-- endregion") + "-- endregion".len()]
                .to_string()
        )),
        "{got:#?}"
    );
    // The 3-line statement folds from its first token to its terminator.
    assert!(
        got.contains(&(
            FoldKind::Statement,
            "SELECT id,\n  total\nFROM orders;".to_string()
        )),
        "{got:#?}"
    );
    // Statements of one and two lines do not fold.
    assert!(!got.iter().any(|(_, t)| t.starts_with("SELECT 1;")));
    assert!(!got.iter().any(|(_, t)| t.starts_with("SELECT a")));
    // A block comment of three lines folds; one of two lines does not.
    assert!(got.contains(&(FoldKind::Comment, "/* one\n   two\n   three */".to_string())));
    assert!(!got.iter().any(|(_, t)| t.starts_with("/* a")));
    // The subquery's block: from its `(` to before its `)` (the closing line stays visible).
    let open = at(text, "FROM (") + 5;
    let close = at(text, ") s");
    assert!(
        r.contains(&FoldRange {
            range: open..close,
            kind: FoldKind::Block
        }),
        "{got:#?}"
    );
    // The statement around it folds too.
    assert!(got.contains(&(
        FoldKind::Statement,
        text[at(text, "SELECT *")..at(text, "AND y;") + 6].to_string()
    )));
    // Every range spans at least three lines.
    for f in &r {
        let lines = text[f.range.clone()].matches('\n').count() + 1;
        assert!(lines >= 3, "{f:?}");
    }
}

#[test]
fn begin_end_case_and_dollar_bodies_are_blocks() {
    let text = "\
CREATE FUNCTION f() RETURNS int AS $$
BEGIN
  RETURN 1;
END
$$ LANGUAGE plpgsql;
SELECT CASE
  WHEN a THEN 1
  ELSE 2
END
FROM t;
";
    let got = texts(text, &ranges(text, pg()));
    // The dollar-quoted body.
    assert!(
        got.iter()
            .any(|(k, t)| *k == FoldKind::Block && t.starts_with("$$\nBEGIN")),
        "{got:#?}"
    );
    // CASE … END: from CASE to before the END line.
    assert!(
        got.iter()
            .any(|(k, t)| *k == FoldKind::Block && t.starts_with("CASE\n  WHEN")),
        "{got:#?}"
    );

    let my = "\
CREATE PROCEDURE p()
BEGIN
  SELECT 1;
  SELECT 2;
END;
";
    let got = texts(my, &ranges(my, mysql()));
    assert!(
        got.iter()
            .any(|(k, t)| *k == FoldKind::Block && t.starts_with("BEGIN\n")),
        "{got:#?}"
    );
}

#[test]
fn an_unclosed_region_or_paren_folds_nothing() {
    let text = "-- region open\nSELECT 1;\nSELECT 2;\nSELECT 3;\nSELECT (\n1\n,\n2;\n";
    let got = texts(text, &ranges(text, pg()));
    assert!(!got.iter().any(|(k, _)| *k == FoldKind::Region), "{got:#?}");
    assert!(!got.iter().any(|(_, t)| t.starts_with("(")), "{got:#?}");
}

/// Through the service: the analysis's ranges at its version, mapped through the edits since
/// while the editor is ahead of it.
#[gpui::test]
fn the_service_answers_the_ranges_at_the_editors_version(cx: &mut TestAppContext) {
    let text = "SELECT a,\n  b\nFROM t;\n";
    let mut d = Doc::analysed(cx, text, pg());
    let now = d.svc.folding_ranges(DOC, d.version());
    assert_eq!(
        now.iter()
            .map(|f| (f.range.clone(), f.kind))
            .collect::<Vec<_>>(),
        vec![(0..text.len() - 1, FoldKind::Statement)]
    );
    // Typed at its start, before the next analysis lands: mapped as the editor maps a fold,
    // the new line above it (fix round 1, M10).
    d.edit(cx, 0..0, "-- x\n");
    let mapped = d.svc.folding_ranges(DOC, d.version());
    assert_eq!(mapped[0].range, 5..text.len() + 4);
    // The next analysis has the statement from its first token.
    cx.run_until_parked();
    let fresh = d.svc.folding_ranges(DOC, d.version());
    assert_eq!(fresh[0].range, 5..text.len() + 4);
}

/// Fix round 1 (I4): a blank line ends a run of `--` comment lines, whatever the lexer folds
/// into one break token.
#[test]
fn a_blank_line_ends_a_run_of_comment_lines() {
    for nl in ["\n", "\r\n"] {
        let text = ["-- a", "-- b", "", "-- c", "-- d", "-- e", "SELECT 1;", ""].join(nl);
        let got = texts(&text, &ranges(&text, pg()));
        let comments: Vec<&String> = got
            .iter()
            .filter(|(k, _)| *k == FoldKind::Comment)
            .map(|(_, t)| t)
            .collect();
        assert_eq!(
            comments,
            vec![&["-- c", "-- d", "-- e"].join(nl)],
            "{nl:?}: {got:#?}"
        );
    }
}

/// Fix round 1 (M12): a `CASE` left open at a `;` never pairs with a later plain `END`.
#[test]
fn an_unclosed_case_never_pairs_with_a_later_end() {
    let text = "SELECT CASE\n  WHEN a THEN 1\n;\nSELECT 2;\nEND;\n";
    let got = texts(text, &ranges(text, pg()));
    assert!(
        !got.iter()
            .any(|(k, t)| *k == FoldKind::Block && t.starts_with("CASE")),
        "{got:#?}"
    );
}

/// Fix round 1 (M6): the blocks inside a dollar-quoted body fold too, and `IF … END IF`
/// and `LOOP … END LOOP` are blocks; `IF EXISTS` and `IF(` are not.
#[test]
fn blocks_inside_dollar_bodies_and_if_and_loop_blocks() {
    let text = "\
CREATE FUNCTION f() RETURNS int AS $$
BEGIN
  IF x THEN
    RETURN 1;
    RETURN 2;
  END IF;
  LOOP
    EXIT;
    EXIT;
  END LOOP;
  RETURN 3;
END
$$ LANGUAGE plpgsql;
DROP TABLE IF EXISTS t;
SELECT IF(a,
  1,
  2);
";
    let got = texts(text, &ranges(text, pg()));
    let blocks: Vec<&str> = got
        .iter()
        .filter(|(k, _)| *k == FoldKind::Block)
        .map(|(_, t)| t.as_str())
        .collect();
    assert!(
        blocks.iter().any(|t| t.starts_with("BEGIN\n  IF")),
        "{blocks:#?}"
    );
    assert!(
        blocks.iter().any(|t| t.starts_with("IF x THEN")),
        "{blocks:#?}"
    );
    assert!(
        blocks.iter().any(|t| t.starts_with("LOOP\n")),
        "{blocks:#?}"
    );
    assert!(
        !blocks.iter().any(|t| t.starts_with("IF EXISTS")),
        "{blocks:#?}"
    );
    // The `IF(` call's parentheses fold as a group, never as an IF block.
    assert!(!blocks.iter().any(|t| t.starts_with("IF(")), "{blocks:#?}");
    let all = ranges(text, pg());
    for f in &all {
        assert!(f.range.end <= text.len());
        assert!(text.is_char_boundary(f.range.start) && text.is_char_boundary(f.range.end));
    }
}
