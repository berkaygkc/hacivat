//! `split_with_parser` and the error cursors (Task 9).
//!
//! Every expected cursor below is libpg_query's own `cursorpos` for the same text (1-based
//! characters, turned into a 0-based byte offset), read from the C library by a scratch
//! harness while this file was written.

use sql_oracle_pg::{PgError, split_with_parser, statement_count};
use sql_text::{TextRange, TextSize};

fn texts(sql: &str, ranges: Vec<TextRange>) -> Vec<&str> {
    ranges.into_iter().map(|r| &sql[r]).collect()
}

fn split_texts(sql: &str) -> Vec<&str> {
    texts(
        sql,
        split_with_parser(sql).unwrap_or_else(|e| panic!("{sql:?}: {e:?}")),
    )
}

fn at(n: u32) -> Option<TextSize> {
    Some(TextSize::from(n))
}

fn count_err(sql: &str) -> PgError {
    statement_count(sql).unwrap_err()
}

fn split_err(sql: &str) -> PgError {
    split_with_parser(sql).unwrap_err()
}

#[test]
fn split_returns_statement_ranges() {
    assert_eq!(
        texts(
            "SELECT 1; SELECT 2",
            split_with_parser("SELECT 1; SELECT 2").unwrap()
        ),
        ["SELECT 1", "SELECT 2"]
    );
}

#[test]
fn named_params_are_normalised_before_the_oracle() {
    assert_eq!(split_with_parser("SELECT :a; SELECT 2").unwrap().len(), 2);
    // The ranges are the caller's offsets: the placeholder is inside, unchanged.
    assert_eq!(
        split_texts("SELECT :abc;  SELECT :d"),
        ["SELECT :abc", "SELECT :d"]
    );
    assert_eq!(
        split_texts("SELECT :'v'; SELECT :\"w\""),
        ["SELECT :'v'", "SELECT :\"w\""]
    );
    assert_eq!(
        split_texts("SELECT 1 FROM t WHERE a = :x$; SELECT 2"),
        ["SELECT 1 FROM t WHERE a = :x$", "SELECT 2"]
    );
    // Without the normalisation the parser stops at the `:` (libpg_query, cursor 7).
    assert_eq!(count_err("SELECT :abc;  SELECT :d").cursor, at(7));
}

#[test]
fn casts_and_slices_are_not_placeholders() {
    assert_eq!(
        split_texts("SELECT x::int; SELECT 2"),
        ["SELECT x::int", "SELECT 2"]
    );
    assert_eq!(
        split_texts("SELECT a[1:n], a[lo:hi] FROM t; SELECT 2"),
        ["SELECT a[1:n], a[lo:hi] FROM t", "SELECT 2"]
    );
}

#[test]
fn a_replacement_never_opens_a_dollar_quote() {
    // `$t:x` → `$t$1` would open a dollar quote reaching `$t$` and swallow the `DROP`. The
    // `:` follows a `$`, so it stays, and the text fails where libpg_query fails on it.
    let e = split_err("SELECT $t:x; DROP TABLE t; SELECT $t$");
    assert_eq!(e.message, r#"syntax error at or near "$""#);
    assert_eq!(e.cursor, at(7));
}

#[test]
fn ranges_leave_out_comments_whitespace_and_terminators() {
    assert_eq!(
        split_texts("  -- lead\n SELECT 1;  /* c */ SELECT 2 -- tail\n"),
        ["SELECT 1", "SELECT 2"]
    );
    assert_eq!(
        split_texts("SELECT 1 ; SELECT 2 ;  "),
        ["SELECT 1", "SELECT 2"]
    );
    assert_eq!(split_texts("SELECT 1 /* x */ ;"), ["SELECT 1"]);
    assert_eq!(split_texts("SELECT 'çğ' AS ı ;"), ["SELECT 'çğ' AS ı"]);
}

#[test]
fn semicolons_inside_a_statement_stay_inside() {
    let atomic = "CREATE FUNCTION f() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1; SELECT 2; END; SELECT 3";
    assert_eq!(
        split_texts(atomic),
        [
            "CREATE FUNCTION f() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1; SELECT 2; END",
            "SELECT 3"
        ]
    );
    let dollar = "CREATE FUNCTION g() RETURNS int AS $body$ SELECT 1; SELECT 2 $body$ LANGUAGE sql; SELECT 3";
    assert_eq!(split_texts(dollar).len(), 2);
    let rule = "CREATE RULE r AS ON INSERT TO t DO ALSO (INSERT INTO x VALUES (1); INSERT INTO y VALUES (2)); SELECT 4";
    assert_eq!(split_texts(rule)[1], "SELECT 4");
    assert_eq!(
        split_texts("SELECT ';'; SELECT \";\""),
        ["SELECT ';'", "SELECT \";\""]
    );
}

#[test]
fn empty_statements_and_comments_give_no_range() {
    for sql in ["", "   ", "  -- c\n/* d */", ";;", " ; -- x\n ;"] {
        assert_eq!(split_with_parser(sql), Ok(Vec::new()), "{sql:?}");
    }
    assert_eq!(split_texts("SELECT 1;;SELECT 2;"), ["SELECT 1", "SELECT 2"]);
}

#[test]
fn error_cursor_maps_after_multibyte_text() {
    // `FRM` is an alias; the error is at `t` (P18).
    let e = statement_count("SELECT 'çğ' FRM t").unwrap_err();
    assert_eq!(e.cursor, Some(TextSize::from(18)));
    assert_eq!(e.message, r#"syntax error at or near "t""#);
    assert_eq!(split_err("SELECT 'çğ' FRM t; SELECT 1").cursor, at(18));
    assert_eq!(count_err("SELECT 'ı' ı ı").cursor, at(15));
    assert_eq!(count_err("SELECT 1\r\nFRM t").cursor, at(14));
}

#[test]
fn split_errors_carry_byte_cursors() {
    assert_eq!(split_err("SELECT 1; SELEC 2").cursor, at(10));
    let e = split_err("SELECT 1; this is not sql; SELECT 2");
    assert_eq!(e.message, r#"syntax error at or near "this""#);
    assert_eq!(e.cursor, at(10));
}

#[test]
fn end_of_input_and_unterminated_constructs() {
    for (sql, want) in [
        ("SELECT 1 +", 10),
        ("SELECT (((1)", 12),
        ("SELECT a, b FROM t WHERE a = 1 AND b = 2 AND c = 3 AND", 54),
        ("SELECT 'abc', 'abc", 14),
        ("SELECT /* abc", 7),
        ("SELECT $$abc", 7),
    ] {
        assert_eq!(count_err(sql).cursor, at(want), "{sql:?}");
        assert_eq!(split_err(sql).cursor, at(want), "{sql:?}");
    }
}

#[test]
fn lexical_errors_point_at_their_token() {
    for (sql, want) in [
        ("SELECT 1a", 7),
        ("SELECT \"\"", 7),
        ("SELECT 0x", 7),
        ("SELECT 1_000_", 7),
    ] {
        assert_eq!(count_err(sql).cursor, at(want), "{sql:?}");
    }
}

#[test]
fn repeated_token_text_is_resolved_by_the_parser() {
    for (sql, want) in [
        ("SELECT 1 t t", 11),
        ("SELECT x FROM t t t", 18),
        ("SELECT 1 t t; SELECT 'çğ' t t", 11),
        ("SELECT 1, 2, 3,, 4", 15),
        ("SELECT f(a, b, c, , d)", 18),
        ("SELECT 'x', 'x' 'x'", 16),
        ("INSERT INTO t VALUES (1, 2), (3, 4) (5, 6)", 36),
        ("SELECT ARRAY[1, 2][1:2] [", 18),
        ("UPDATE t SET a = 1, b = 2 WHERE WHERE", 32),
        ("SELECT a FROM b GROUP BY a HAVING HAVING", 34),
        ("WITH a AS (SELECT 1) SELECT * FROM a a a", 39),
        // Keywords the lookahead filter recodes by the next token: decided with it.
        (
            "SELECT x FROM t WHERE x NOT ONININ (1, 2) AND y NOT LIKE 'a'",
            24,
        ),
        ("SELECT a FROM t ORDER BY a NULLS FIRST, b NULLS ", 42),
        ("COPY t (a,WITH b) FROM STDIN WITH (FORMAT csv)", 10),
    ] {
        assert_eq!(count_err(sql).cursor, at(want), "{sql:?}");
        assert_eq!(split_err(sql).cursor, at(want), "{sql:?}");
    }
}

#[test]
fn cursor_is_none_rather_than_unproven() {
    // A grammar action's error carries no token text (libpg_query: 9).
    let e = count_err("SELECT 1 LIMIT 1, 2");
    assert_eq!(e.message, "LIMIT #,# syntax is not supported");
    assert_eq!(e.cursor, None);
    // Recoded keywords next to each other (libpg_query: 27, 27, 16), a `U&` identifier in
    // the text (27), and lexical errors whose text occurs twice (12, 15).
    for sql in [
        "SELECT a NOT LIKE b, c NOT NOT",
        "SELECT a FROM t ORDER BY a NULLS NULLS",
        "SELECT 1 FROM t WITH WITH",
        "SELECT U&\"a\" UESCAPE '!' x x",
        "SELECT 1e5, 1e",
        "SELECT E'\\uD800'",
    ] {
        assert_eq!(count_err(sql).cursor, None, "{sql:?}");
    }
    // A recoded keyword whose text occurs once is still placed (libpg_query: 9).
    assert_eq!(count_err("SELECT 1 WITH ORDINALITY").cursor, at(9));
}

#[test]
fn nul_is_an_error_not_a_truncated_parse() {
    // Cut at the NUL, the first text would be one valid statement.
    let e = statement_count("SELECT 1\0garbage").unwrap_err();
    assert_eq!(e.cursor, at(8));
    assert!(e.message.contains("NUL"), "{e:?}");
    let e = split_err("SELECT 1;\0 DROP TABLE t");
    assert_eq!(e.cursor, at(9));
}
