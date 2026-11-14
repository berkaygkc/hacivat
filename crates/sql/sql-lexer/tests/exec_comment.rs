//! Executable comments and their version gates (SQL-LEX-05; Task 4).

use sql_dialect::DialectCtx;
use sql_lexer::{ExecMarker, GateStatus, Lexed, TokenKind, exec_comment, exec_comment_gate};
use sql_testkit::lex::{dialect_ctx, lex_in};
use sql_text::{TextRange, TextSize};

fn mysql84() -> DialectCtx {
    dialect_ctx("mysql", Some("8.4.0"))
}

fn lex_mysql84(text: &str) -> Lexed {
    lex_in(&mysql84(), text)
}

fn range(start: u32, end: u32) -> TextRange {
    TextRange::new(TextSize::from(start), TextSize::from(end))
}

#[test]
fn skipped_versioned_comment_holds_one_nested_comment() {
    let l = lex_mysql84("/*!99999 a /* b */ c */ SELECT 1");
    assert_eq!(l.kind(0), TokenKind::ExecutableComment);
    assert_eq!(l.range(0).end(), TextSize::from(23)); // the outer comment closes at byte 23 (P18)
}

/// The gate of the first executable comment in `text`, lexed and judged in `ctx`.
fn gate(ctx: &DialectCtx, text: &str) -> GateStatus {
    let l = lex_in(ctx, text);
    let i = (0..l.len())
        .find(|i| l.kind(*i) == TokenKind::ExecutableComment)
        .unwrap_or_else(|| panic!("no executable comment in {text:?}"));
    exec_comment_gate(text, &l, i, ctx)
}

#[test]
fn version_gates_follow_the_server_version() {
    let m84 = mysql84();
    let m80 = dialect_ctx("mysql", Some("8.0.36"));
    assert_eq!(gate(&m84, "SELECT 1 /*!50000 , 2 */"), GateStatus::Active);
    assert_eq!(
        gate(&m84, "SELECT 1 /*!99999 , SLEEP(100) */"),
        GateStatus::Inactive
    );
    assert_eq!(gate(&m84, "/*!80400 x */"), GateStatus::Active);
    assert_eq!(gate(&m84, "/*!80401 x */"), GateStatus::Inactive);
    assert_eq!(gate(&m80, "/*!80036 x */"), GateStatus::Active);
    assert_eq!(gate(&m80, "/*!80400 x */"), GateStatus::Inactive);
    assert_eq!(gate(&m84, "/*!12345SELECT*/"), GateStatus::Active);
}

#[test]
fn an_unversioned_comment_always_runs() {
    let unknown_version = dialect_ctx("mysql", None);
    assert_eq!(
        gate(&unknown_version, "/*! STRAIGHT_JOIN */"),
        GateStatus::Active
    );
    assert_eq!(gate(&mysql84(), "/*! x */"), GateStatus::Active);
    // Fewer than five digits are body text, not a version.
    assert_eq!(gate(&mysql84(), "/*!1 x */"), GateStatus::Active);
}

#[test]
fn a_versioned_comment_without_a_server_version_is_unknown() {
    let ctx = dialect_ctx("mysql", None);
    assert_eq!(gate(&ctx, "/*!50000 x */"), GateStatus::Unknown);
    let ctx = dialect_ctx("mariadb", None);
    assert_eq!(gate(&ctx, "/*M!100100 x */"), GateStatus::Unknown);
}

#[test]
fn mariadb_runs_both_markers_by_its_own_version() {
    let ctx = dialect_ctx("mariadb", Some("10.6.12"));
    assert_eq!(gate(&ctx, "/*M!100100 x */"), GateStatus::Active);
    assert_eq!(gate(&ctx, "/*M!100700 x */"), GateStatus::Inactive);
    assert_eq!(gate(&ctx, "/*!50000 x */"), GateStatus::Active);
    assert_eq!(gate(&ctx, "/*!100612 x */"), GateStatus::Active);
    assert_eq!(gate(&ctx, "/*!100613 x */"), GateStatus::Inactive);
    assert_eq!(gate(&ctx, "/*M! x */"), GateStatus::Active);
}

#[test]
fn mysql_reads_five_version_digits() {
    // `/*!100100` on MySQL is version 10010 (1.0.10) and body text `0 x`.
    assert_eq!(gate(&mysql84(), "/*!100100 x */"), GateStatus::Active);
    assert_eq!(
        gate(&dialect_ctx("mysql", Some("1.0.9")), "/*!100100 x */"),
        GateStatus::Inactive
    );
}

#[test]
fn a_mariadb_comment_judged_for_mysql_is_inactive() {
    // Lexed as MariaDB, it is an executable comment; MySQL does not run `/*M!`.
    let text = "/*M! x */";
    let l = lex_in(&dialect_ctx("mariadb", None), text);
    assert_eq!(l.kind(0), TokenKind::ExecutableComment);
    assert_eq!(
        exec_comment_gate(text, &l, 0, &mysql84()),
        GateStatus::Inactive
    );
    assert_eq!(
        exec_comment_gate(text, &l, 0, &dialect_ctx("postgres", Some("18.0.0"))),
        GateStatus::Inactive
    );
    // On MySQL it lexes as an ordinary comment, which is never an executable body.
    let l = lex_mysql84(text);
    assert_eq!(l.kind(0), TokenKind::BlockComment);
    assert_eq!(
        exec_comment_gate(text, &l, 0, &mysql84()),
        GateStatus::Inactive
    );
    assert_eq!(exec_comment(text, &l, 0), None);
}

#[test]
fn exec_comment_names_its_parts() {
    let text = "SELECT 1 /*!50110 KEY_BLOCK_SIZE=1024 */;";
    let l = lex_mysql84(text);
    let i = 4;
    assert_eq!(l.kind(i), TokenKind::ExecutableComment);
    let c = exec_comment(text, &l, i).unwrap();
    assert_eq!(c.marker, ExecMarker::Bang);
    assert_eq!(c.version, Some(50110));
    assert_eq!(
        &text[usize::from(c.opener.start())..usize::from(c.opener.end())],
        "/*!50110"
    );
    assert_eq!(
        &text[usize::from(c.body.start())..usize::from(c.body.end())],
        " KEY_BLOCK_SIZE=1024 "
    );
    let text = "/*M!100100x*/";
    let l = lex_in(&dialect_ctx("mariadb", None), text);
    let c = exec_comment(text, &l, 0).unwrap();
    assert_eq!((c.marker, c.version), (ExecMarker::MariaDb, Some(100_100)));
    assert_eq!(c.body, range(10, 11));
    // Unterminated: the body runs to the end.
    let text = "/*! SELECT 1";
    let c = exec_comment(text, &lex_mysql84(text), 0).unwrap();
    assert_eq!((c.version, c.body), (None, range(3, 12)));
    // `/*!*/` has an empty body.
    let text = "/*!*/";
    let c = exec_comment(text, &lex_mysql84(text), 0).unwrap();
    assert_eq!(c.body, range(3, 3));
}

#[test]
fn a_body_semicolon_stays_inside_the_comment() {
    // Task 8 flags `;` inside an executable comment as ambiguous; the lexer keeps it inside.
    let l = lex_mysql84("SELECT 1 /*! ; DROP TABLE t */");
    assert_eq!(l.len(), 5);
    assert_eq!(l.kind(4), TokenKind::ExecutableComment);
    assert!(!(0..l.len()).any(|i| l.kind(i) == TokenKind::Semicolon));
}

#[test]
fn executable_comments_are_not_trivia() {
    let l = lex_mysql84("/*!50000 SELECT 1 */ /* c */ /*+ h */");
    assert!(!l.is_trivia(0));
    assert!(l.is_trivia(2));
    assert!(l.is_trivia(4));
}
