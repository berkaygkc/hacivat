//! The splitter (Task 8; SQL-SPLIT-01..03, R-M3-5): the `alg/10` corpus, one fixture per
//! splitter-table row, and the cases the plan names.

use sql_dialect::split_profile::client_profile;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, ServerVersion, split_profile};
use sql_split::{
    AmbiguityReason, ClientKind, Confidence, DiagCode, Item, Split, SplitMode, SplitStart, Stmt,
    StmtKey, split, split_from,
};
use sql_testkit::split::{SplitCase, corpus_file, load, profile_fixture_file};

fn ctx(id: DialectId) -> DialectCtx {
    DialectCtx::new(id, None)
}

fn pg() -> DialectCtx {
    DialectCtx::new(
        DialectId::Postgres,
        Some(ServerVersion {
            major: 18,
            minor: 0,
            patch: 0,
        }),
    )
}

fn run(text: &str, c: &DialectCtx, flavor: ClientFlavor, mode: SplitMode) -> Split {
    split_from(text, c, flavor, mode, &SplitStart::default())
}

fn texts(text: &str, s: &Split) -> Vec<String> {
    s.statements()
        .map(|st| text[st.strict].to_string())
        .collect()
}

fn strict_texts_pg(text: &str) -> Vec<String> {
    texts(
        text,
        &run(text, &pg(), ClientFlavor::Psql, SplitMode::Execution),
    )
}

fn strict_texts_mysql(text: &str) -> Vec<String> {
    texts(
        text,
        &run(
            text,
            &ctx(DialectId::MySql),
            ClientFlavor::MySqlCli,
            SplitMode::Execution,
        ),
    )
}

fn strict_texts_sqlite(text: &str) -> Vec<String> {
    texts(
        text,
        &run(
            text,
            &ctx(DialectId::Sqlite),
            ClientFlavor::Sqlite3,
            SplitMode::Execution,
        ),
    )
}

fn count_mariadb(text: &str) -> usize {
    run(
        text,
        &ctx(DialectId::MariaDb),
        ClientFlavor::MySqlCli,
        SplitMode::Execution,
    )
    .statements()
    .count()
}

fn split_mysql(text: &str) -> (String, Split) {
    (
        text.to_string(),
        run(
            text,
            &ctx(DialectId::MySql),
            ClientFlavor::MySqlCli,
            SplitMode::Execution,
        ),
    )
}

fn statements(s: &(String, Split)) -> Vec<String> {
    texts(&s.0, &s.1)
}

fn split_exec_mysql(text: &str) -> Split {
    run(
        text,
        &ctx(DialectId::MySql),
        ClientFlavor::MySqlCli,
        SplitMode::Execution,
    )
}

fn key_of(text: &str) -> StmtKey {
    StmtKey::of(&pg(), None, text)
}

fn split_cases() -> Vec<SplitCase> {
    ["pg", "mysql", "sqlite"]
        .iter()
        .flat_map(|name| load(&corpus_file(name)))
        .collect()
}

fn check_all(cases: &[SplitCase]) {
    let failures: Vec<String> = cases.iter().filter_map(|c| c.check().err()).collect();
    assert!(
        failures.is_empty(),
        "{} failing:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn split_cases_pg_mysql_sqlite() {
    let cases = split_cases();
    assert!(cases.len() >= 99, "the corpus lost cases: {}", cases.len());
    check_all(&cases);
}

const PROFILES: [(&str, DialectId, ClientFlavor); 5] = [
    ("pg", DialectId::Postgres, ClientFlavor::Psql),
    ("mysql", DialectId::MySql, ClientFlavor::MySqlCli),
    ("mariadb", DialectId::MariaDb, ClientFlavor::MySqlCli),
    ("sqlite", DialectId::Sqlite, ClientFlavor::Sqlite3),
    ("generic", DialectId::Generic, ClientFlavor::None),
];

fn profile_cases() -> Vec<SplitCase> {
    ["pg", "mysql", "mariadb", "sqlite", "generic"]
        .iter()
        .flat_map(|name| load(&profile_fixture_file(name)))
        .collect()
}

#[test]
fn every_splitter_table_row_has_a_passing_fixture() {
    let cases = profile_cases();
    check_all(&cases);
    let mut missing = Vec::new();
    for (name, id, flavor) in PROFILES {
        let dialect = match id {
            DialectId::Postgres => "postgres",
            DialectId::MySql => "mysql",
            DialectId::MariaDb => "mariadb",
            DialectId::Sqlite => "sqlite",
            _ => "generic",
        };
        let mut rows = split_profile(&ctx(id)).rows();
        rows.extend(client_profile(flavor).rows());
        for row in rows {
            let covered = cases.iter().any(|c| {
                c.covers.contains(&row)
                    && c.dialects
                        .as_deref()
                        .unwrap_or_default()
                        .iter()
                        .any(|d| d == dialect)
            });
            if !covered {
                missing.push(format!("{name}: {row}"));
            }
        }
    }
    assert!(missing.is_empty(), "rows without a fixture: {missing:?}");
    // A fixture names only rows that exist.
    let all: Vec<String> = PROFILES
        .iter()
        .flat_map(|(_, id, flavor)| {
            let mut r = split_profile(&ctx(*id)).rows();
            r.extend(client_profile(*flavor).rows());
            r
        })
        .collect();
    for c in &cases {
        for row in &c.covers {
            assert!(all.contains(row), "{}: no such row {row}", c.id);
        }
    }
}

#[test]
fn end_at_statement_start_is_commit() {
    assert_eq!(
        strict_texts_pg("BEGIN; SELECT 1; END; SELECT 2"),
        ["BEGIN", "SELECT 1", "END", "SELECT 2"]
    );
    assert_eq!(
        strict_texts_sqlite("BEGIN; SELECT 1; END TRANSACTION; SELECT 2"),
        ["BEGIN", "SELECT 1", "END TRANSACTION", "SELECT 2"]
    );
}

#[test]
fn repeat_function_is_not_a_block() {
    assert_eq!(
        strict_texts_mysql("SELECT REPEAT('a', 3); SELECT 2"),
        ["SELECT REPEAT('a', 3)", "SELECT 2"]
    );
}

#[test]
fn mariadb_begin_not_atomic_is_a_block() {
    assert_eq!(
        count_mariadb("BEGIN NOT ATOMIC SELECT 1; SELECT 2; END; SELECT 3"),
        2
    );
}

#[test]
fn sqlite_begin_immediate_is_a_transaction() {
    assert_eq!(
        strict_texts_sqlite("BEGIN IMMEDIATE; SELECT 1"),
        ["BEGIN IMMEDIATE", "SELECT 1"]
    );
}

#[test]
fn mysql_delimiter_matches_inside_end() {
    let s = split_mysql(
        "DELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT 1; END$$\nDELIMITER ;\nSELECT 2;",
    );
    assert_eq!(
        statements(&s),
        ["CREATE PROCEDURE p() BEGIN SELECT 1; END", "SELECT 2"]
    );
    // The delimiter is the statement's terminator and never part of the sent text.
    let first = s.1.statements().next().unwrap();
    assert_eq!(&s.0[first.terminator.unwrap()], "$$");
    assert!(
        s.1.client
            .iter()
            .all(|c| c.kind == ClientKind::Delimiter && c.supported)
    );
}

#[test]
fn semicolon_in_executable_comment_is_ambiguous() {
    let s = split_exec_mysql("SELECT 1 /*! ; DROP TABLE t */");
    assert!(matches!(
        s.ambiguous[0].reason,
        AmbiguityReason::SemicolonInExecutableComment
    ));
    // The boundary overlaps the one statement, so execution never sends it.
    let stmt = s.statements().next().unwrap();
    assert_eq!(s.ambiguities_in(stmt.strict).count(), 1);
    assert!(
        s.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::SemicolonInExecutableComment)
    );
}

#[test]
fn copy_from_stdin_data_is_one_segment() {
    let text = "COPY t (a, b) FROM STDIN;\n1\tx;y\n2\t'\n\\.\nSELECT 1;\n";
    for mode in [SplitMode::Editor, SplitMode::Execution] {
        let s = run(text, &pg(), ClientFlavor::Psql, mode);
        let stmts: Vec<&Stmt> = s.statements().collect();
        assert_eq!(texts(text, &s), ["COPY t (a, b) FROM STDIN", "SELECT 1"]);
        let data = stmts[0]
            .copy_data
            .expect("the COPY statement carries its data");
        assert_eq!(&text[data.rows], "1\tx;y\n2\t'\n");
        assert_eq!(&text[data.end.unwrap()], "\\.");
        assert!(stmts[1].copy_data.is_none());
        assert!(s.ambiguous.is_empty(), "{:?}", s.ambiguous);
        assert!(stmts.iter().all(|st| st.confidence == Confidence::Exact));
    }
}

#[test]
fn statement_keys_are_content_addressed() {
    assert_eq!(key_of("SELECT 1"), key_of("SELECT 1"));
    assert_ne!(key_of("SELECT 1"), key_of("SELECT 2"));
    // Dialect, modes and delimiter are part of the key; the position is not.
    let my = ctx(DialectId::MySql);
    assert_ne!(StmtKey::of(&my, None, "SELECT 1"), key_of("SELECT 1"));
    assert_ne!(
        StmtKey::of(&my, Some("$$"), "SELECT 1"),
        StmtKey::of(&my, None, "SELECT 1")
    );
    let mut no_bs = my;
    no_bs.modes.no_backslash_escapes = true;
    assert_ne!(
        StmtKey::of(&no_bs, None, "SELECT 1"),
        StmtKey::of(&my, None, "SELECT 1")
    );
    let text = "SELECT 1;\nSELECT 2;\nSELECT 1;";
    let s = run(text, &pg(), ClientFlavor::None, SplitMode::Editor);
    let keys: Vec<StmtKey> = s.statements().map(|st| st.key).collect();
    assert_eq!(keys[0], keys[2]);
    assert_ne!(keys[0], keys[1]);
    assert_eq!(keys[0], key_of("SELECT 1"));
}

#[test]
fn the_seam_split_reads_the_given_lexing() {
    let text = "SELECT 1; SELECT 2";
    let c = pg();
    let lexed = sql_lexer::lex(
        text,
        sql_dialect::lex_profile(&c).unwrap(),
        c.modes,
        &sql_lexer::LexState::Initial,
    );
    let s = split(text, &lexed, &c, ClientFlavor::None, SplitMode::Execution);
    assert_eq!(texts(text, &s), ["SELECT 1", "SELECT 2"]);
    // A lexing of another text is not trusted: the splitter lexes the text itself.
    let other = sql_lexer::lex(
        "x",
        sql_dialect::lex_profile(&c).unwrap(),
        c.modes,
        &sql_lexer::LexState::Initial,
    );
    let s = split(text, &other, &c, ClientFlavor::None, SplitMode::Execution);
    assert_eq!(texts(text, &s), ["SELECT 1", "SELECT 2"]);
}

#[test]
fn smart_ranges_take_leading_and_trailing_comments() {
    let text = "-- one\n-- two\nSELECT 1; -- after one\n\n-- apart\n\n/* lead */ SELECT 2; /* same line */\nSELECT 3";
    let s = run(text, &pg(), ClientFlavor::None, SplitMode::Execution);
    let st: Vec<&Stmt> = s.statements().collect();
    assert_eq!(&text[st[0].leading_comments.unwrap()], "-- one\n-- two");
    assert_eq!(&text[st[0].smart], "-- one\n-- two\nSELECT 1; -- after one");
    // A comment with a blank line on both sides belongs to nobody.
    assert_eq!(&text[st[1].leading_comments.unwrap()], "/* lead */");
    assert_eq!(&text[st[1].smart], "/* lead */ SELECT 2; /* same line */");
    assert_eq!(st[2].leading_comments, None);
    assert_eq!(&text[st[2].smart], "SELECT 3");
    // A trailing comment after the last statement, on its line, trails it.
    let t2 = "SELECT 1 -- end";
    let s2 = run(t2, &pg(), ClientFlavor::None, SplitMode::Execution);
    assert_eq!(&t2[s2.statements().next().unwrap().smart], t2);
}

#[test]
fn heuristic_parts_keep_their_comments() {
    let text = "SELECT 1 -- first\n-- about two\nSELECT 2";
    let s = run(text, &pg(), ClientFlavor::None, SplitMode::Editor);
    let st: Vec<&Stmt> = s.statements().collect();
    assert_eq!(texts(text, &s), ["SELECT 1", "SELECT 2"]);
    assert_eq!(&text[st[0].smart], "SELECT 1 -- first");
    assert_eq!(&text[st[1].smart], "-- about two\nSELECT 2");
    assert!(st.iter().all(|x| x.confidence == Confidence::Heuristic));
    assert_eq!(st[1].head.first_token.map(|r| &text[r]), Some("SELECT"));
}

#[test]
fn execution_never_sends_an_uncertain_boundary() {
    // Every kind of uncertainty: a missing `;`, an unterminated string, a `;` in /*! */, an
    // ambiguous comment end, a client command inside, a mismatched block.
    let cases: [(&str, DialectId, ClientFlavor, AmbiguityReason); 6] = [
        (
            "SELECT 1\nSELECT 2",
            DialectId::Postgres,
            ClientFlavor::None,
            AmbiguityReason::MissingSemicolon,
        ),
        (
            "SELECT 'x",
            DialectId::Postgres,
            ClientFlavor::None,
            AmbiguityReason::UnterminatedConstruct,
        ),
        (
            "SELECT /*! 1; */ 2",
            DialectId::MySql,
            ClientFlavor::None,
            AmbiguityReason::SemicolonInExecutableComment,
        ),
        (
            "SELECT 1 /*!50000 '*/' */; DROP TABLE t",
            DialectId::MySql,
            ClientFlavor::None,
            AmbiguityReason::AmbiguousCommentEnd,
        ),
        (
            "SELECT 1\n\\echo x\nFROM t;",
            DialectId::Postgres,
            ClientFlavor::Psql,
            AmbiguityReason::ClientCommand,
        ),
        (
            "CREATE PROCEDURE p() BEGIN WHILE a DO SELECT 1; END LOOP; END",
            DialectId::MySql,
            ClientFlavor::None,
            AmbiguityReason::UnknownBlock,
        ),
    ];
    for (text, id, flavor, reason) in cases {
        let exec = run(text, &ctx(id), flavor, SplitMode::Execution);
        let editor = run(text, &ctx(id), flavor, SplitMode::Editor);
        // A refused execution statement never reads as runnable: not `Exact`, not certain.
        assert!(
            exec.statements()
                .all(|s| s.confidence == Confidence::Heuristic),
            "{text}"
        );
        assert_eq!(exec.runnable().count(), 0, "{text}");
        assert!(
            exec.ambiguous.iter().any(|a| a.reason == reason),
            "{text}: {:?}",
            exec.ambiguous
        );
        for s in exec.statements() {
            assert!(!exec.is_certain(s), "{text}");
            assert!(
                exec.ambiguities_in(s.strict).next().is_some(),
                "{text}: {:?} is not covered",
                &text[s.strict]
            );
        }
        assert!(
            editor
                .statements()
                .all(|s| s.confidence == Confidence::Heuristic),
            "{text}"
        );
    }
}

#[test]
fn a_command_before_the_terminator_changes_no_text() {
    // psql sends `SELECT 1` either way: the command does not sit between two of its tokens.
    let text = "SELECT 1\n\\echo x\n;";
    let s = run(text, &pg(), ClientFlavor::Psql, SplitMode::Execution);
    assert_eq!(texts(text, &s), ["SELECT 1"]);
    assert!(s.ambiguous.is_empty(), "{:?}", s.ambiguous);
    assert_eq!(s.client.len(), 1);
}

#[test]
fn a_dml_keyword_in_an_ambiguous_comment_never_splits_silently() {
    // MySQL runs the comment and then DELETE as a second statement; the lexer sees one.
    let text = "SELECT 1 /*!50000 , '*/' */ ; DELETE FROM t; -- '";
    let s = split_exec_mysql(text);
    assert_eq!(statements(&(text.to_string(), s.clone())), [text]);
    let a = s
        .ambiguous
        .iter()
        .find(|a| a.reason == AmbiguityReason::AmbiguousCommentEnd)
        .unwrap();
    // The ambiguity runs to the server's end of the comment.
    assert_eq!(&text[a.at], "/*!50000 , '*/' */");
}

#[test]
fn long_tokens_across_chunks_after_a_command_stay_whole() {
    // After a psql command the splitter lexes in chunks; strings and dollar quotes longer than
    // a chunk, and COPY data longer than one, must come back whole.
    let long = "x;".repeat(40_000);
    let text = format!(
        "\\set a 1\nSELECT '{long}';\nDO $$ {long} $$;\nCOPY t FROM STDIN;\n{}\\.\nSELECT 2;\n",
        "row\t;\n".repeat(30_000)
    );
    let s = run(&text, &pg(), ClientFlavor::Psql, SplitMode::Execution);
    let got = texts(&text, &s);
    assert_eq!(
        got.len(),
        4,
        "{:?}",
        got.iter().map(|g| g.len()).collect::<Vec<_>>()
    );
    assert_eq!(got[0], format!("SELECT '{long}'"));
    assert_eq!(got[1], format!("DO $$ {long} $$"));
    assert_eq!(got[2], "COPY t FROM STDIN");
    assert_eq!(got[3], "SELECT 2");
    let data = s.statements().nth(2).unwrap().copy_data.unwrap();
    assert_eq!(
        data.rows.len(),
        u32::try_from("row\t;\n".len() * 30_000).unwrap().into()
    );
    assert!(s.ambiguous.is_empty());
}

#[test]
fn many_client_commands_split_in_one_pass() {
    // A psql script with a command on every other line (and quotes in the arguments).
    let mut text = String::new();
    for i in 0..2_000 {
        text.push_str(&format!("\\echo it's {i}\nSELECT {i};\n"));
    }
    let s = run(&text, &pg(), ClientFlavor::Psql, SplitMode::Execution);
    assert_eq!(s.statements().count(), 2_000);
    assert_eq!(s.client.len(), 2_000);
    assert!(s.ambiguous.is_empty());
    assert!(
        s.client
            .iter()
            .all(|c| !c.supported && c.kind == ClientKind::Command)
    );
}

#[test]
fn turkish_text_never_becomes_a_keyword() {
    // `İNSERT` / `ınsert` are not INSERT: no heuristic boundary, no block.
    let text = "SELECT 1\nİNSERT INTO t VALUES (1)";
    let s = run(text, &pg(), ClientFlavor::None, SplitMode::Execution);
    assert!(s.ambiguous.is_empty());
    let s = run(
        "CREATE PROCEDURE p() BEGİN SELECT 1; END; SELECT 2",
        &ctx(DialectId::MySql),
        ClientFlavor::None,
        SplitMode::Execution,
    );
    assert_eq!(s.statements().count(), 3);
}

/// About `bytes` of mixed PostgreSQL: one-line and multi-line statements, strings with `;`,
/// comments, a CTE, a dollar-quoted function, statements a `;` is missing between.
fn gen_script(bytes: usize) -> String {
    let parts = [
        "SELECT id, full_name FROM customers WHERE city = 'Ankara;' ORDER BY id;\n",
        "-- the next one spans lines\nINSERT INTO orders (customer_id, total)\nSELECT id, 12.5 FROM customers\nWHERE is_active;\n",
        "WITH recent AS (\n  SELECT id FROM orders WHERE ordered_at > now() - interval '30 days'\n)\nSELECT count(*) FROM recent;\n",
        "CREATE OR REPLACE FUNCTION f() RETURNS int LANGUAGE plpgsql AS $$ BEGIN PERFORM 1; RETURN 2; END $$;\n",
        "UPDATE products SET price = price * 1.1 /* ; */ WHERE sku IN ('a', 'b');\n",
        "SELECT 1\nSELECT 2;\n",
    ];
    let mut s = String::with_capacity(bytes + 256);
    let mut i = 0;
    while s.len() < bytes {
        s.push_str(parts[i % parts.len()]);
        i += 1;
    }
    s
}

#[test]
#[ignore = "perf"]
fn one_megabyte_splits_within_30_ms_release() {
    if !std::env::var_os("HACIVAT_PERF").is_some_and(|v| v == "1") {
        return;
    }
    let text = gen_script(1 << 20);
    let mut times: Vec<std::time::Duration> = (0..10)
        .map(|_| {
            let start = std::time::Instant::now();
            // What the editor's background worker does per snapshot (R-M3-22), and what Run
            // asks of the same snapshot.
            let d = sql_split::SqlDocument::new(text.clone(), pg(), ClientFlavor::Psql);
            let snap = d.snapshot();
            let n = snap.execution_split().statements().count();
            let took = start.elapsed();
            assert!(n > 10_000);
            took
        })
        .collect();
    times.sort();
    let median = times[5];
    // The parts, for the record: lexing alone, and the document without the execution view.
    let c = pg();
    let lexer = sql_dialect::lex_profile(&c).unwrap();
    let mut lex_t: Vec<_> = (0..10)
        .map(|_| {
            let s = std::time::Instant::now();
            let l = sql_lexer::lex(&text, lexer, c.modes, &sql_lexer::LexState::Initial);
            assert!(!l.is_empty());
            s.elapsed()
        })
        .collect();
    lex_t.sort();
    let mut doc_t: Vec<_> = (0..10)
        .map(|_| {
            let s = std::time::Instant::now();
            let d = sql_split::SqlDocument::new(text.clone(), pg(), ClientFlavor::Psql);
            assert!(!d.statements().is_empty());
            s.elapsed()
        })
        .collect();
    doc_t.sort();
    println!(
        "1 MiB: lex + editor split + execution split, median {median:?} (lex {:?}, document with the editor split {:?})",
        lex_t[5], doc_t[5]
    );
    assert!(median <= std::time::Duration::from_millis(30), "{median:?}");
}

#[test]
fn the_perf_script_splits_as_written() {
    let text = gen_script(4_000);
    let s = run(&text, &pg(), ClientFlavor::Psql, SplitMode::Execution);
    // Every part but the last ends with `;`; the last has one missing `;` (one ambiguity).
    assert!(s.statements().count() >= 6);
    assert_eq!(
        s.ambiguous.len(),
        s.ambiguous
            .iter()
            .filter(|a| a.reason == AmbiguityReason::MissingSemicolon)
            .count()
    );
    assert!(!s.ambiguous.is_empty());
}

#[test]
fn hostile_client_scripts_fail_closed() {
    let pg_exec = |t: &str| run(t, &pg(), ClientFlavor::Psql, SplitMode::Execution);
    let my_exec = |t: &str| {
        run(
            t,
            &ctx(DialectId::MySql),
            ClientFlavor::MySqlCli,
            SplitMode::Execution,
        )
    };
    // psql: `\g file` writes to a file named by the rest of the line: no DROP runs.
    let t = "SELECT 1 \\g DROP TABLE t\n";
    let s = pg_exec(t);
    assert_eq!(texts(t, &s), ["SELECT 1"]);
    assert_eq!(&t[s.client[0].range], "\\g DROP TABLE t");
    assert!(
        !s.ambiguous.is_empty(),
        "Hacivat cannot write to the file: Run asks"
    );
    // The mysql client discards a statement before `\c`: running it is refused.
    let t = "DELETE FROM t \\c SELECT 1;";
    let s = my_exec(t);
    assert_eq!(texts(t, &s), ["DELETE FROM t", "SELECT 1"]);
    let del = s.statements().next().unwrap();
    assert!(s.ambiguities_in(del.strict).next().is_some());
    assert!(
        s.ambiguities_in(s.statements().nth(1).unwrap().strict)
            .next()
            .is_none()
    );
    // `\. ` (a trailing space) does not end COPY data: the data runs on and is refused.
    let t = "COPY t FROM STDIN;\n1\n\\. \nDELETE FROM t;\n";
    let s = pg_exec(t);
    assert_eq!(texts(t, &s), ["COPY t FROM STDIN"]);
    assert!(!s.ambiguous.is_empty());
    // COPY text inside a dollar quote is no COPY.
    let t = "DO $$ BEGIN EXECUTE 'COPY t FROM STDIN;'; END $$;\nSELECT 1;";
    let s = pg_exec(t);
    assert_eq!(texts(t, &s).len(), 2);
    assert!(s.statements().all(|st| st.copy_data.is_none()));
    // A nested executable comment: nothing after it is certain.
    let t = "SELECT 1 /*!50000 /*!40101 x */ ; DELETE FROM t */; SELECT 2;";
    let s = run(
        t,
        &ctx(DialectId::MySql),
        ClientFlavor::None,
        SplitMode::Execution,
    );
    for st in s.statements() {
        assert!(
            s.ambiguities_in(st.strict).next().is_some(),
            "{:?}",
            &t[st.strict]
        );
    }
}

#[test]
fn delimiters_are_raw_text() {
    let my = |t: &str| {
        let s = run(
            t,
            &ctx(DialectId::MySql),
            ClientFlavor::MySqlCli,
            SplitMode::Execution,
        );
        texts(t, &s)
    };
    // A non-ASCII delimiter, and one that is a word: matched as raw text, even inside a word.
    assert_eq!(
        my("DELIMITER ☃\nSELECT 'ç'☃SELECT 2☃\n"),
        ["SELECT 'ç'", "SELECT 2"]
    );
    assert_eq!(
        my("DELIMITER GO\nSELECT 1 GO\nSELECT AGOB GO\n"),
        ["SELECT 1", "SELECT A", "B"]
    );
    // A backslash in a delimiter is refused (the delimiter stays `;`).
    let t = "DELIMITER \\\\\nSELECT 1;";
    let s = run(
        t,
        &ctx(DialectId::MySql),
        ClientFlavor::MySqlCli,
        SplitMode::Execution,
    );
    assert_eq!(texts(t, &s), ["SELECT 1"]);
    assert!(
        s.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::DelimiterInvalid)
    );
    assert!(!s.client[0].supported);
    // DELIMITER inside a comment or a string is no command.
    let t = "/* DELIMITER $$ */ SELECT 1;\nSELECT '\nDELIMITER $$\n';\n-- DELIMITER $$\nSELECT 3;";
    assert_eq!(my(t).len(), 3);
}

#[test]
fn deep_nesting_is_linear_and_never_recurses() {
    let depth = 200_000;
    let text = format!(
        "SELECT {}1{}; SELECT 2",
        "(".repeat(depth),
        ")".repeat(depth)
    );
    let s = run(&text, &pg(), ClientFlavor::None, SplitMode::Execution);
    assert_eq!(s.statements().count(), 2);
    let open = format!("CREATE PROCEDURE p() {}", "BEGIN ".repeat(depth));
    let s = run(
        &open,
        &ctx(DialectId::MySql),
        ClientFlavor::None,
        SplitMode::Editor,
    );
    assert_eq!(s.statements().count(), 1);
}

#[test]
fn certainty_is_a_lookup_and_every_boundary_sits_in_one_statement() {
    // The hint's server-side end swallows `DELETE FROM t`: one ambiguity per statement it
    // makes uncertain, each inside that statement; `SELECT 5`/`SELECT 6` miss a `;`.
    let text = "SELECT /*+ BKA('*/ 1; DELETE FROM t; -- ') */ 3;\nSELECT 4;\nSELECT 5\nSELECT 6;";
    let s = run(
        text,
        &ctx(DialectId::MySql),
        ClientFlavor::None,
        SplitMode::Execution,
    );
    let stmts: Vec<&Stmt> = s.statements().collect();
    for a in &s.ambiguous {
        assert_eq!(
            stmts
                .iter()
                .filter(|st| st.strict.contains_range(a.at))
                .count(),
            1,
            "{a:?}"
        );
    }
    let runnable: Vec<&str> = s.runnable().map(|st| &text[st.strict]).collect();
    assert_eq!(runnable, ["SELECT 4"]);
    for st in &stmts {
        assert_eq!(
            s.is_certain(st),
            st.confidence == Confidence::Exact,
            "{:?}",
            &text[st.strict]
        );
    }
    assert_eq!(
        s.ambiguous
            .iter()
            .filter(|a| a.reason == AmbiguityReason::AmbiguousCommentEnd)
            .count(),
        2
    );
}

#[test]
fn an_end_that_follows_an_operator_or_a_branch_is_a_name() {
    // `x > end` and `CASE … THEN end END`: names, not block ends (erratum (e)).
    let text = "CREATE TRIGGER g AFTER INSERT ON u BEGIN DELETE FROM t WHERE x > end; SELECT CASE a WHEN 1 THEN end END FROM t; END; SELECT 2";
    let s = run(
        text,
        &ctx(DialectId::Sqlite),
        ClientFlavor::None,
        SplitMode::Execution,
    );
    assert_eq!(texts(text, &s).len(), 2);
    assert_eq!(s.runnable().count(), 2, "{:?}", s.ambiguous);
}

#[test]
fn a_stray_end_with_no_block_before_it_refuses_back_to_the_start() {
    // MariaDB `sql_mode=ORACLE` anonymous blocks are not tracked (digest R3); their `END`
    // starts a statement, which the backstop takes for an opener gone unseen. With no
    // block-tracked statement before it, everything back to the text's start is uncertain
    // (T8 round-1 re-review R1-2).
    for text in [
        "BEGIN SELECT 1; DELETE FROM t; END;",
        "DECLARE x INT; BEGIN DELETE FROM t; END;",
    ] {
        for oracle in [false, true] {
            let mut c = ctx(DialectId::MariaDb);
            c.modes.mariadb_oracle_mode = oracle;
            let s = run(text, &c, ClientFlavor::None, SplitMode::Execution);
            assert_eq!(
                s.runnable().count(),
                0,
                "{text:?} oracle={oracle}: {:?}",
                s.ambiguous
            );
        }
    }
}

#[test]
fn client_command_splitting_lexes_linearly() {
    // The structural pass's own lexing (`lexed_bytes`, the complexity counter) stays within a
    // small multiple of the text whatever the number of commands, COPY blocks or delimiters:
    // each restart lexes a short first chunk, and quotes in commands or data never send it to
    // the end. A pass that re-lexed the rest after each command would lex O(n²).
    use sql_split::splitter::lexed_bytes;
    let mut psql = String::new();
    let mut copy = String::new();
    let mut delim = String::from("DELIMITER $$\n");
    let mut dot = String::new();
    for i in 0..4_000 {
        psql.push_str(&format!("\\echo it's {i}\nSELECT {i};\n"));
        copy.push_str(&format!("COPY t{i} FROM STDIN;\nO'Brien\t{i}\n\\.\n"));
        delim.push_str(&format!("SELECT {i}$$\n"));
        dot.push_str(&format!(".print it's {i}\nSELECT {i};\n"));
    }
    for (text, id, flavor) in [
        (&psql, DialectId::Postgres, ClientFlavor::Psql),
        (&copy, DialectId::Postgres, ClientFlavor::Psql),
        (&delim, DialectId::MySql, ClientFlavor::MySqlCli),
        (&dot, DialectId::Sqlite, ClientFlavor::Sqlite3),
    ] {
        let n = text.len() as u64;
        let lexed = lexed_bytes(text, &ctx(id), flavor);
        assert!(
            lexed <= 4 * n + 4_096,
            "{id:?}: {lexed} bytes lexed for {n}"
        );
        // Doubling the script at most doubles the work.
        let twice = format!("{text}{text}");
        let lexed2 = lexed_bytes(&twice, &ctx(id), flavor);
        assert!(lexed2 <= 2 * lexed + 4_096, "{id:?}: {lexed2} vs {lexed}");
    }
}

#[test]
fn one_line_scripts_and_comment_ends_lex_linearly() {
    // With no line start to cut at, chunks are cut inside the line; each executable comment's
    // server-side end is read in prefixes, not to the end of the text. Before either, a
    // one-line script cost O(n) per restart or per comment: O(n²) in all.
    use sql_split::splitter::lexed_bytes;
    for (unit, id, flavor) in [
        (r"SELECT 1 \; ", DialectId::Postgres, ClientFlavor::Psql),
        (r"SELECT 1 \g ", DialectId::MySql, ClientFlavor::MySqlCli),
        (
            "/*!40101 SET @a = 1 */; ",
            DialectId::MySql,
            ClientFlavor::MySqlCli,
        ),
        ("/*!*/1/*!'  */", DialectId::MySql, ClientFlavor::None),
        (
            r"/*!b'\'' */ SELECT 1; ",
            DialectId::MariaDb,
            ClientFlavor::None,
        ),
        (
            r#"1'.print "*/"`$$\;\\g\;SELECT \echo "#,
            DialectId::Postgres,
            ClientFlavor::Psql,
        ),
        // A restart before a comment or a word that runs to the cut, again and again.
        (
            r"\echo '*/'\-- `;'*/''\",
            DialectId::Postgres,
            ClientFlavor::Psql,
        ),
        (r"/* */x\\;\/*M!", DialectId::Postgres, ClientFlavor::Psql),
        (
            r"\echo x $a_long_dollar_tag_name ",
            DialectId::Postgres,
            ClientFlavor::Psql,
        ),
    ] {
        let text = unit.repeat(2_000);
        let n = text.len() as u64;
        let lexed = lexed_bytes(&text, &ctx(id), flavor);
        assert!(
            lexed <= 32 * n + 4_096,
            "{unit:?}: {lexed} bytes lexed for {n}"
        );
        let twice = format!("{text}{text}");
        let lexed2 = lexed_bytes(&twice, &ctx(id), flavor);
        assert!(lexed2 <= 2 * lexed + 4_096, "{unit:?}: {lexed2} vs {lexed}");
    }
}

#[test]
fn empty_and_blank_texts_have_no_items() {
    for text in ["", "   \n\t", "-- c\n/* d */"] {
        for mode in [SplitMode::Editor, SplitMode::Execution] {
            let s = run(text, &pg(), ClientFlavor::Psql, mode);
            assert!(s.items.is_empty(), "{text:?}");
        }
    }
    let s = run(";", &pg(), ClientFlavor::None, SplitMode::Execution);
    assert!(matches!(s.items.as_slice(), [Item::Empty { .. }]));
}
