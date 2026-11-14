//! Cursor eligibility (R-M3-23; Task 8): only a unit whose head reads, with no `INTO`, no
//! locking clause and no executable comment, may stay suspended.

use sql_dialect::{ClientFlavor, DialectCtx, DialectId};
use sql_split::head::cursor_eligible;
use sql_split::{DocSnapshot, SqlDocument};
use sql_text::{TextRange, TextSize};

fn doc(id: DialectId, text: &str) -> DocSnapshot {
    SqlDocument::new(
        text.to_string(),
        DialectCtx::new(id, None),
        ClientFlavor::None,
    )
    .snapshot()
}

fn eligible(id: DialectId, sql: &str) -> bool {
    let d = doc(id, sql);
    cursor_eligible(
        &d,
        TextRange::up_to(TextSize::from(u32::try_from(sql.len()).unwrap())),
    )
}

fn eligible_pg(sql: &str) -> bool {
    eligible(DialectId::Postgres, sql)
}

fn eligible_mysql(sql: &str) -> bool {
    eligible(DialectId::MySql, sql)
}

#[test]
fn cursor_eligibility_reads_the_head() {
    for (sql, want) in [
        ("SELECT 1", true),
        ("  -- c\n select 1", true),
        ("VALUES (1)", true),
        ("TABLE t", true),
        ("WITH x AS (SELECT 1) SELECT * FROM x", true),
        (
            "WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d",
            false,
        ),
        ("SELECT * FROM t FOR UPDATE", false),
        ("SELECT 1 INTO t2", false),
        ("INSERT INTO t VALUES (1) RETURNING *", false),
        ("CALL p()", false),
    ] {
        assert_eq!(eligible_pg(sql), want, "{sql}");
    }
    assert!(eligible_mysql("SHOW TABLES"));
    assert!(!eligible_mysql("/*!50000 DELETE FROM t*/ SELECT 1"));
}

#[test]
fn every_locking_clause_streams_to_completion() {
    for sql in [
        "SELECT * FROM t FOR SHARE",
        "SELECT * FROM t FOR NO KEY UPDATE",
        "SELECT * FROM t FOR KEY SHARE",
        "SELECT * FROM t WHERE a = 1 FOR UPDATE OF t NOWAIT",
        "select * from t for update skip locked",
    ] {
        assert!(!eligible_pg(sql), "{sql}");
    }
    assert!(!eligible_mysql("SELECT * FROM t LOCK IN SHARE MODE"));
    assert!(!eligible_mysql("SELECT * FROM t FOR UPDATE"));
    assert!(!eligible_mysql("SELECT 1 INTO @x"));
    assert!(!eligible_mysql("SELECT * FROM t INTO OUTFILE '/tmp/x'"));
    // FOR in other positions is no lock.
    assert!(eligible_pg("SELECT substring('abc' FROM 1 FOR 2)"));
}

#[test]
fn a_keyword_in_a_comment_does_not_count_one_in_an_executable_comment_does() {
    assert!(eligible_pg("SELECT 1 -- FOR UPDATE\n"));
    assert!(eligible_pg(
        "WITH x AS (SELECT 1 /* DELETE */) SELECT * FROM x"
    ));
    assert!(eligible_pg("SELECT 'INTO', \"for\" FROM t"));
    assert!(!eligible_mysql("SELECT /*! FOR UPDATE */ 1"));
    assert!(!eligible_mysql("SELECT 1 /*!80000 INTO @x */"));
    assert!(!eligible_mysql("/*!50000 SELECT 1 */"));
    // An optimizer hint is a comment.
    assert!(eligible_mysql(
        "SELECT /*+ MAX_EXECUTION_TIME(1000) */ * FROM t"
    ));
}

#[test]
fn what_it_cannot_read_is_never_eligible() {
    for sql in [
        "SELECT 1; DELETE FROM t",
        "SELECT 'unterminated",
        "",
        "-- only a comment",
        "EXPLAIN SELECT 1",
        "SHOW search_path",
    ] {
        assert!(!eligible_pg(sql), "{sql:?}");
    }
    // A comment whose end MySQL reads differently.
    assert!(!eligible_mysql("SELECT 1 /*!50000 '*/' */"));
    // A range that is not the document's text (past the end, or inside a character).
    let d = doc(DialectId::Postgres, "SELECT 'ç'");
    assert!(!cursor_eligible(
        &d,
        TextRange::new(TextSize::from(0), TextSize::from(9))
    ));
    assert!(!cursor_eligible(
        &d,
        TextRange::new(TextSize::from(0), TextSize::from(99))
    ));
}

#[test]
fn the_range_is_read_as_sent_not_as_the_document_lexes_it() {
    // The unit is the second statement only; the document around it does not matter.
    let text = "DELETE FROM t; SELECT 1 FROM u";
    let d = doc(DialectId::Postgres, text);
    let start = u32::try_from(text.find("SELECT").unwrap()).unwrap();
    let r = TextRange::new(
        TextSize::from(start),
        TextSize::from(u32::try_from(text.len()).unwrap()),
    );
    assert!(cursor_eligible(&d, r));
    assert!(!cursor_eligible(
        &d,
        TextRange::new(TextSize::from(0), TextSize::from(13))
    ));
}

#[test]
fn ascii_keywords_only() {
    // Turkish dotless `ı` and dotted `İ` make no keyword, so nothing reads.
    assert!(!eligible_pg("sel\u{0131}ct 1"));
    assert!(!eligible_pg("WİTH x AS (SELECT 1) SELECT * FROM x"));
    assert!(eligible_pg("sElEcT 1"));
}
