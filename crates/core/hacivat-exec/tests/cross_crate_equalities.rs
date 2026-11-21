//! M3b Task 18: the cross-crate equalities of digest 01 R18 (P-M3b-13: they reach the guard,
//! which lives with the sessions, so they live here and not in `crates/sql`).
//!
//! - Every function the read-only guard's text layer refuses on PostgreSQL (1b,
//!   `hacivat_guard::PG_NON_TRANSACTIONAL_FUNCTIONS`) is on the classifier's PostgreSQL deny
//!   list (`sql_safety::denied_function_names`): the classifier never allows what the guard
//!   refuses.
//! - The SQLite read pragmas of the guard and of the classifier are the same list.
//! - For random statements, every first-keyword reading the guard follows (`guard_keywords`,
//!   M2b's hand scanner: steps 1a and 7) is a reading the lexer gives too (`sql_lexer::lex`,
//!   with every version-gated executable comment read both ways): the classifier's view of
//!   where a statement starts covers the guard's.

use std::collections::BTreeSet;

use hacivat_exec::sqlctx::dialect_id;
use hacivat_guard::PG_NON_TRANSACTIONAL_FUNCTIONS;
use hacivat_guard::sqlite::SQLITE_READ_PRAGMAS;
use hacivat_proto::DbmsKind;
use hacivat_session::session::readonly::guard_keyword_readings;
use proptest::prelude::*;
use sql_dialect::{DialectCtx, DialectId, LexModes};
use sql_lexer::{LexState, TokenFlags, TokenKind, exec_comment, lex};

#[test]
fn guard_function_list_is_a_subset_of_the_denylist() {
    let denied: BTreeSet<&str> = sql_safety::denied_function_names(DialectId::Postgres).collect();
    let missing: Vec<&str> = PG_NON_TRANSACTIONAL_FUNCTIONS
        .iter()
        .copied()
        .filter(|f| !denied.contains(f))
        .collect();
    assert!(missing.is_empty(), "not on the PG deny list: {missing:?}");
}

#[test]
fn sqlite_pragma_lists_are_equal() {
    let guard: BTreeSet<&str> = SQLITE_READ_PRAGMAS.iter().copied().collect();
    let classifier: BTreeSet<&str> = sql_safety::sqlite_read_pragmas().collect();
    assert_eq!(guard, classifier);
    assert_eq!(guard.len(), SQLITE_READ_PRAGMAS.len(), "no duplicates");
}

// ---------------------------------------------------------------------------------------------
// First keywords

/// How deep the lexer readings follow executable comments (the guard forks at most 16 times).
const MAX_DEPTH: usize = 24;

/// The first keyword of every reading of `text` on `kind` by the lexer: whitespace, comments
/// and `;` skipped; an executable comment read inside (its body, then the rest) and, when
/// version-gated, skipped too; the first other token's text ASCII upper-cased when it is a
/// word, else `""`; `None` when a reading holds no statement (only comments, or an unterminated
/// one).
fn lexer_readings(kind: DbmsKind, text: &str, depth: usize, out: &mut BTreeSet<Option<String>>) {
    if depth > MAX_DEPTH {
        return;
    }
    let ctx = DialectCtx::new(dialect_id(kind), None);
    let profile = sql_dialect::lex_profile(&ctx).expect("a lexing profile");
    let lexed = lex(
        text,
        profile,
        LexModes::default_for(ctx.id),
        &LexState::Initial,
    );
    for i in 0..lexed.len() {
        let range = lexed.range(i);
        let (start, end) = (usize::from(range.start()), usize::from(range.end()));
        match lexed.kind(i) {
            TokenKind::BlockComment | TokenKind::HintComment
                if lexed.flags(i).contains(TokenFlags::UNTERMINATED) =>
            {
                out.insert(None);
                return;
            }
            _ if lexed.is_trivia(i) => {}
            TokenKind::Semicolon => {}
            TokenKind::ExecutableComment => {
                let Some(ec) = exec_comment(text, &lexed, i) else {
                    out.insert(Some(String::new()));
                    return;
                };
                let rest = &text[end..];
                let body = &text[usize::from(ec.body.start())..usize::from(ec.body.end())];
                lexer_readings(kind, &format!("{body} {rest}"), depth + 1, out);
                if ec.version.is_some() {
                    lexer_readings(kind, rest, depth + 1, out);
                }
                return;
            }
            TokenKind::Word => {
                out.insert(Some(text[start..end].to_ascii_uppercase()));
                return;
            }
            _ => {
                out.insert(Some(String::new()));
                return;
            }
        }
    }
    out.insert(None);
}

/// Statements built from the pieces both readers meet before a first keyword: whitespace,
/// `;`, line and block comments in every engine's spelling, hints, executable comments (MySQL's
/// and MariaDB's, with and without a version, empty or holding a keyword), then a keyword or
/// another token, optionally inside an executable comment that closes after it.
///
/// A `*/` appears only as the end of an executable comment. A stray one at the top level is
/// skipped by the guard's scanner (`*/COMMIT` reads `COMMIT`), where the server and the lexer
/// read an operator: the guard then reads a keyword the server does not, which only ever
/// refuses more (step 1a) or judges more (step 7), so it is no gap and not generated here.
fn sql_text() -> impl Strategy<Value = String> {
    let lead = prop::sample::select(vec![
        " ",
        "\n",
        "\t",
        "\r\n",
        ";",
        "-- c\n",
        "--\n",
        "# c\n",
        "/* c */",
        "/* a /* nested */ b */",
        "/*+ hint */",
        "/*!*/",
        "/*! */",
        "/*!50000 */",
        "/*!50000 BEGIN */",
        "/*!99999 COMMIT */",
        "/*! ROLLBACK */",
        "/*M! START TRANSACTION */",
        "/*M!100100 SAVEPOINT s */",
        "--x ",
    ]);
    let head = prop::sample::select(vec![
        "SELECT 1",
        "select a from t",
        "BEGIN",
        "commit",
        "Rollback",
        "INSERT INTO t VALUES (1)",
        "DROP TABLE t",
        "START TRANSACTION",
        "SET x = 1",
        "WITH d AS (SELECT 1) SELECT 1",
        "(SELECT 1)",
        "\"q\"",
        "'s'",
        "",
        "LOCK TABLES t WRITE",
        "end",
    ]);
    let wrap = prop::sample::select(vec![
        "",
        "/*!",
        "/*!50000 ",
        "/*!99999 ",
        "/*M!",
        "/*M!100100 ",
    ]);
    (
        prop::collection::vec(lead.clone(), 0..6),
        wrap,
        head,
        prop::collection::vec(lead, 0..3),
    )
        .prop_map(|(a, w, h, b)| {
            let close = if w.is_empty() { "" } else { " */" };
            format!("{}{w}{h}{close}{}", a.concat(), b.concat())
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]
    #[test]
    fn guard_keywords_are_a_subset_of_lexer_readings(sql in sql_text()) {
        for kind in [DbmsKind::Postgres, DbmsKind::MySql, DbmsKind::MariaDb, DbmsKind::Sqlite] {
            let (guard, complete) = guard_keyword_readings(kind, &sql);
            // Readings cut short already fail closed in the guard (step 1a refuses them).
            if !complete {
                continue;
            }
            let mut lexer = BTreeSet::new();
            lexer_readings(kind, &sql, 0, &mut lexer);
            for reading in guard {
                prop_assert!(
                    lexer.contains(&reading),
                    "{kind:?} {sql:?}: the guard reads {reading:?}, the lexer {lexer:?}"
                );
            }
        }
    }
}

#[test]
fn lexer_readings_follow_version_gates_both_ways() {
    let mut out = BTreeSet::new();
    lexer_readings(DbmsKind::MySql, "/*!50000 BEGIN */ SELECT 1", 0, &mut out);
    assert!(out.contains(&Some("BEGIN".into())), "{out:?}");
    assert!(out.contains(&Some("SELECT".into())), "{out:?}");
    let mut out = BTreeSet::new();
    lexer_readings(DbmsKind::Postgres, "/* a /* b */ c */ commit", 0, &mut out);
    assert_eq!(out, BTreeSet::from([Some("COMMIT".to_string())]));
}
