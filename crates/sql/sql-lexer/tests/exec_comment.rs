//! Executable comments and their version gates (SQL-LEX-05; Task 4).

use sql_dialect::DialectCtx;
use sql_lexer::{
    ExecMarker, GateStatus, LexErrorCode, LexState, Lexed, TokenFlags, TokenKind, exec_comment,
    exec_comment_gate, lex,
};
use sql_testkit::lex::{dialect_ctx, lex_in, profile};
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

/// The first comment token of `l` (executable or hint) and whether it is flagged as having
/// an ambiguous end.
fn ambiguous_comment(l: &Lexed) -> (usize, bool) {
    let i = (0..l.len())
        .find(|i| {
            matches!(
                l.kind(*i),
                TokenKind::ExecutableComment | TokenKind::HintComment
            )
        })
        .expect("a comment token");
    let flagged = l.flags(i).contains(TokenFlags::ERROR);
    let coded = l
        .errors()
        .iter()
        .any(|e| e.range == l.range(i) && e.code == LexErrorCode::AmbiguousCommentEnd);
    assert_eq!(flagged, coded, "the flag and the error go together");
    (i, flagged)
}

/// Review I1: a body that quotes `*/` (or hides it in a line comment) ends elsewhere for a
/// server that runs it. The lexer cannot tell which reading the server takes, so the token is
/// flagged and no `;` the running reading sees is a boundary the lexer trusts.
#[test]
fn a_hidden_close_makes_the_comment_end_ambiguous() {
    let mariadb = dialect_ctx("mariadb", Some("11.4.0"));
    for (ctx, text) in [
        // The review's probe: MySQL 8.4 runs `DROP TABLE t` as a second statement.
        (
            mysql84(),
            "SELECT 1 /*!50000 , '*/' */ ; DROP TABLE t; -- '",
        ),
        // An unversioned comment always runs.
        (mysql84(), "SELECT 1 /*! , '*/' */ ; DROP TABLE t; -- '"),
        // `"…"` is a string without ANSI_QUOTES.
        (
            mysql84(),
            "SELECT 1 /*!50000 , \"*/\" */ ; DROP TABLE t; # \"",
        ),
        // A `#` and a `-- ` comment hide `*/` to the line feed.
        (mysql84(), "SELECT 1 /*! # */\n */ ; DROP TABLE t"),
        (mysql84(), "SELECT 1 /*! -- */\n */ ; DROP TABLE t"),
        // A backtick identifier.
        (mysql84(), "SELECT /*! `*/` */ 1"),
        // MariaDB's marker.
        (mariadb, "/*M! '*/' */ ; DROP TABLE t; -- '"),
        // An optimizer hint fails closed the same way (controller ruling).
        (
            mysql84(),
            "SELECT /*+ SET_VAR(sql_mode='*/') */ 1; DROP TABLE t; -- '",
        ),
        // The running reading closes first: a `/*` inside quotes nests only when skipped.
        (mysql84(), "/*! '/*' */ ; DROP t; */ x"),
    ] {
        let l = lex_in(&ctx, text);
        let (_, ambiguous) = ambiguous_comment(&l);
        assert!(ambiguous, "{text:?}");
    }
}

#[test]
fn bodies_both_readings_agree_on_are_not_flagged() {
    for text in [
        "/*!80000 SELECT 'versioned;' */;",
        "/*!50000 a /* b */ c */",
        "/*!40101 SET @OLD_CHARACTER_SET_CLIENT=@@CHARACTER_SET_CLIENT */;",
        "/*!50017 DEFINER=`root`@`localhost`*/",
        "SELECT /*+ SET_VAR(sql_mode='ANSI_QUOTES') */ 1",
        "SELECT /*+ MAX_EXECUTION_TIME(1000) BKA(t1) */ 1",
        "/*! # a line comment\n x */",
        "/*!99999 /* a /* b */ SELECT 1 */ BEGIN",
    ] {
        let l = lex_mysql84(text);
        let (_, ambiguous) = ambiguous_comment(&l);
        assert!(!ambiguous, "{text:?}");
        assert!(l.errors().is_empty(), "{text:?}");
    }
}

#[test]
fn modes_change_the_running_reading() {
    // With backslash escapes `'\' */ '` is one string, so the body runs past the first `*/`;
    // with NO_BACKSLASH_ESCAPES the quote closes and both readings agree.
    let text = "/*! '\\' */ ' */";
    assert!(ambiguous_comment(&lex_mysql84(text)).1);
    let mut nbe = mysql84();
    nbe.modes.no_backslash_escapes = true;
    assert!(!ambiguous_comment(&lex_in(&nbe, text)).1);
    // Under ANSI_QUOTES `"…"` is an identifier, which hides `*/` just as well.
    let mut ansi = mysql84();
    ansi.modes.ansi_quotes = true;
    assert!(ambiguous_comment(&lex_in(&ansi, "/*! \"*/\" */ x\"")).1);
}

#[test]
fn resuming_inside_a_comment_keeps_its_reading() {
    // Lex up to a line start inside the comment, resume from the end state: the resumed piece
    // carries the same flag as the full lex's token.
    let ctx = mysql84();
    let p = profile(&ctx);
    for (head, tail) in [
        ("SELECT /*!50000 'a\n", "*/' */ ; DROP TABLE t; -- '"),
        ("SELECT /*! # x\n", "*/ y"),
        ("SELECT /*! 'a'\n", "*/ ; SELECT 2"),
        ("SELECT /*+ x\n", "'*/' */"),
        ("/*! '/*' */\n", "; DROP t; */"),
    ] {
        let full = lex(&format!("{head}{tail}"), p, ctx.modes, &LexState::Initial);
        let left = lex(head, p, ctx.modes, &LexState::Initial);
        let right = lex(tail, p, ctx.modes, left.end_state());
        let (i, full_flag) = ambiguous_comment(&full);
        let resumed_flag = right.flags(0).contains(TokenFlags::ERROR);
        assert_eq!(resumed_flag, full_flag, "{head:?} + {tail:?}");
        assert_eq!(
            usize::from(full.range(i).end()) - head.len(),
            usize::from(right.range(0).end()),
            "{head:?} + {tail:?}"
        );
    }
}

#[test]
fn executable_comments_are_not_trivia() {
    let l = lex_mysql84("/*!50000 SELECT 1 */ /* c */ /*+ h */");
    assert!(!l.is_trivia(0));
    assert!(l.is_trivia(2));
    assert!(l.is_trivia(4));
}
