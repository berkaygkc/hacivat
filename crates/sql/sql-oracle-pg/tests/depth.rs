//! The nesting check before every parser call (Task 9; R-M3-3, R-M3-35).

use sql_oracle_pg::{MAX_ORACLE_NESTING, split_with_parser, statement_count};
use sql_text::TextSize;

fn nested(open: &str, close: &str, depth: usize) -> String {
    format!("SELECT {}1{}", open.repeat(depth), close.repeat(depth))
}

/// `SELECT c0 + c1 + … ` with `n` operands.
fn chain(n: usize) -> String {
    let terms: Vec<String> = (0..n).map(|i| format!("c{i}")).collect();
    format!("SELECT {} FROM t", terms.join(" + "))
}

/// `n` `SELECT`s joined by `UNION ALL`.
fn unions(n: usize) -> String {
    let parts: Vec<String> = (0..n).map(|i| format!("SELECT {i}")).collect();
    parts.join(" UNION ALL ")
}

#[test]
fn ten_thousand_parens_on_a_small_stack_is_an_error() {
    let sql = format!("SELECT {}1{}", "(".repeat(10_000), ")".repeat(10_000));
    let r = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || statement_count(&sql))
        .unwrap()
        .join()
        .unwrap();
    assert!(r.is_err());
}

#[test]
fn the_limit_parses_and_one_more_level_is_refused() {
    let limit = usize::from(MAX_ORACLE_NESTING);
    assert_eq!(statement_count(&nested("(", ")", limit)), Ok(1));
    assert_eq!(
        split_with_parser(&nested("(", ")", limit)).unwrap().len(),
        1
    );
    for e in [
        statement_count(&nested("(", ")", limit + 1)).unwrap_err(),
        split_with_parser(&nested("(", ")", limit + 1)).unwrap_err(),
    ] {
        // The cursor is on the first bracket past the limit: `SELECT ` and 256 `(` before it.
        assert_eq!(e.cursor, Some(TextSize::from(7 + 256)));
        assert_eq!(e.message, "brackets nest too deeply for the parser");
    }
}

#[test]
fn parentheses_and_square_brackets_count_together() {
    let half = usize::from(MAX_ORACLE_NESTING) / 2;
    let mixed = |n: usize| format!("SELECT {}1{}", "(ARRAY[".repeat(n), "])".repeat(n));
    assert_eq!(statement_count(&mixed(half)), Ok(1));
    assert!(statement_count(&mixed(half + 1)).is_err());
}

#[test]
fn brackets_in_strings_identifiers_and_comments_do_not_count() {
    let sql = format!(
        "SELECT '{p}', \"{p}\", $${p}$$ /* {b} */ -- {b}\n",
        p = "(".repeat(1_000),
        b = "[".repeat(1_000)
    );
    assert_eq!(statement_count(&sql), Ok(1));
    assert_eq!(split_with_parser(&sql).unwrap().len(), 1);
}

#[test]
fn long_chains_and_unions_parse() {
    assert!(statement_count(&chain(50)).is_ok());
    assert!(statement_count(&unions(100)).is_ok());
    assert_eq!(
        split_with_parser(&format!("{}; {}", chain(50), unions(100)))
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn deep_trees_without_brackets_parse_or_fail_cleanly() {
    // Trees the bracket count cannot see. Left-deep ones (2 MB of `+`, 50,000 `UNION`s) are
    // built iteratively and parse; right-nested ones fill bison's stack (10,000 entries) and
    // end with its "memory exhausted" error at the token where it filled. Cursors are
    // libpg_query's own; the `NOT` one is None (a recoded keyword with many occurrences).
    for sql in [chain(200_000), unions(50_000)] {
        assert_eq!(statement_count(&sql), Ok(1));
        assert_eq!(split_with_parser(&sql).map(|r| r.len()), Ok(1));
    }
    let not_chain = format!("SELECT {}true", "NOT ".repeat(100_000));
    let case_chain = format!(
        "SELECT {}1{}",
        "CASE WHEN true THEN ".repeat(20_000),
        " END".repeat(20_000)
    );
    let minus_chain = format!("SELECT {}1", "- ".repeat(30_000));
    for (sql, token, cursor) in [
        (not_chain, "NOT", None),
        (case_chain, "WHEN", Some(39_992)),
        (minus_chain, "-", Some(19_999)),
    ] {
        let want = format!("memory exhausted at or near \"{token}\"");
        for e in [
            statement_count(&sql).unwrap_err(),
            split_with_parser(&sql).unwrap_err(),
        ] {
            assert_eq!(e.message, want);
            assert_eq!(e.cursor, cursor.map(TextSize::from), "{token}");
        }
    }
}

#[test]
fn a_lexical_error_does_not_hide_deep_nesting() {
    // The unterminated string is placed, so the tokens before it are counted: too deep.
    let sql = format!("SELECT {}'abc", "(".repeat(300));
    let e = statement_count(&sql).unwrap_err();
    assert_eq!(e.message, "brackets nest too deeply for the parser");
    assert_eq!(e.cursor, Some(TextSize::from(7 + 256)));
    // A lexical error whose text occurs twice cannot be placed: the bound is every bracket
    // byte in the text, and above the limit the scanner's own error comes back, unparsed.
    let sql = format!("SELECT {}1e, 1e", "(".repeat(300));
    let e = statement_count(&sql).unwrap_err();
    assert!(e.message.starts_with("trailing junk"), "{e:?}");
    // Below the limit the parser runs and reports its own error.
    let sql = format!("SELECT {}1e, 1e", "(".repeat(3));
    assert!(
        statement_count(&sql)
            .unwrap_err()
            .message
            .starts_with("trailing junk")
    );
}
