//! `SqlDocument`, `DocSnapshot` and `DocDelta` (Task 8).

use std::sync::Arc;

use sql_dialect::{ClientFlavor, DialectCtx, DialectId, ServerVersion};
use sql_lexer::LexState;
use sql_split::{Confidence, Item, SplitMode, SqlDocument, Stmt};
use sql_text::{TextEdit, TextRange, TextSize};

fn pg18() -> DialectCtx {
    DialectCtx::new(
        DialectId::Postgres,
        Some(ServerVersion {
            major: 18,
            minor: 0,
            patch: 0,
        }),
    )
}

fn is_heuristic(i: &Item) -> bool {
    matches!(i, Item::Statement(s) if s.confidence == Confidence::Heuristic)
}

fn texts(d: &SqlDocument) -> Vec<&str> {
    d.statements()
        .iter()
        .filter_map(Item::statement)
        .map(|s| &d.text()[s.strict])
        .collect()
}

fn at(n: u32) -> TextSize {
    TextSize::from(n)
}

fn edit(start: u32, end: u32, insert: &str) -> TextEdit {
    TextEdit {
        delete: TextRange::new(at(start), at(end)),
        insert: insert.to_string(),
    }
}

#[test]
fn unterminated_quote_keeps_later_boundaries_provisional() {
    let d = SqlDocument::new(
        format!("SELECT 'x;\n{}", "SELECT 1;\n".repeat(50)),
        pg18(),
        ClientFlavor::None,
    );
    assert_eq!(
        d.statements().len(),
        51,
        "every later statement keeps its boundary"
    );
    assert!(d.statements().iter().skip(1).all(is_heuristic));
    assert_eq!(texts(&d)[0], "SELECT 'x;");
    assert!(texts(&d).iter().skip(1).all(|t| *t == "SELECT 1"));
    // Execution mode never does that: one statement, and it is uncertain.
    let snap = d.snapshot();
    let exec = snap.execution_split();
    assert_eq!(exec.statements().count(), 1);
    assert!(!exec.ambiguous.is_empty());
}

#[test]
fn the_stability_rule_follows_an_open_block_and_a_comment() {
    let d = SqlDocument::new(
        "SELECT 1;\n/* typing\nSELECT 2;\nSELECT 3;\n".to_string(),
        pg18(),
        ClientFlavor::None,
    );
    assert_eq!(texts(&d), ["SELECT 1", "SELECT 2", "SELECT 3"]);
    assert!(!is_heuristic(&d.statements()[0]));
    assert!(d.statements().iter().skip(1).all(is_heuristic));
    // A dollar quote opened inside the tail of a tail is followed again (bounded).
    let d = SqlDocument::new(
        "SELECT $$\nSELECT 'a\nSELECT 2;\n".to_string(),
        pg18(),
        ClientFlavor::None,
    );
    assert_eq!(texts(&d), ["SELECT $$", "SELECT 'a", "SELECT 2"]);
}

#[test]
fn snapshots_share_one_execution_split_per_revision() {
    let d = SqlDocument::new("SELECT 1; SELECT 2".into(), pg18(), ClientFlavor::None);
    let (a, b) = (d.snapshot(), d.snapshot());
    let pa: *const _ = a.execution_split();
    let pb: *const _ = b.execution_split();
    assert_eq!(pa, pb, "the execution split is computed once per revision");
    assert_eq!(a.execution_split().statements().count(), 2);
    assert_eq!(a.editor_split(), b.editor_split());
    assert_eq!(a.revision(), 0);
    assert!(std::ptr::eq(a.text(), b.text()));
    fn send_sync<T: Send + Sync>(_: &T) {}
    send_sync(&a);
    let _: Arc<str> = Arc::from(a.text());
}

#[test]
fn apply_rebuilds_and_reports_the_delta() {
    let mut d = SqlDocument::new(
        "SELECT 1;\nSELECT 2;\nSELECT 3;".into(),
        pg18(),
        ClientFlavor::None,
    );
    let before: Vec<_> = d
        .statements()
        .iter()
        .filter_map(Item::statement)
        .map(|s| s.key)
        .collect();
    // Type "0" after "SELECT 2": the second statement changes, the third moves.
    let delta = d.apply(&[edit(18, 18, "0")]).unwrap();
    assert_eq!(d.revision(), 1);
    assert_eq!(texts(&d), ["SELECT 1", "SELECT 20", "SELECT 3"]);
    let after: Vec<_> = d
        .statements()
        .iter()
        .filter_map(Item::statement)
        .map(|s| s.key)
        .collect();
    assert_eq!(delta.changed, [after[1]]);
    assert_eq!(delta.removed, [before[1]]);
    assert_eq!(delta.moved, [after[2]]);
    assert!(delta.dirty.contains_range(TextRange::new(at(10), at(19))));
    // Deleting a `;` (and the line break, so no heuristic keeps them apart) merges two
    // statements into one.
    let delta = d.apply(&[edit(8, 10, " ")]).unwrap();
    assert_eq!(texts(&d), ["SELECT 1 SELECT 20", "SELECT 3"]);
    assert_eq!(delta.removed.len(), 2);
    assert_eq!(delta.changed.len(), 1);
    assert_eq!(delta.moved.len(), 1);
    // Deleting only the `;` leaves a heuristic boundary where the line breaks.
    let mut e = SqlDocument::new("SELECT 1;\nSELECT 2;".into(), pg18(), ClientFlavor::None);
    e.apply(&[edit(8, 9, "")]).unwrap();
    assert_eq!(texts(&e), ["SELECT 1", "SELECT 2"]);
    assert!(e.statements().iter().all(is_heuristic));
    // An edit that does not fit is refused: the document, its revision and its analysis are
    // unchanged, even when an earlier edit of the same batch fitted.
    let (text, revision) = (d.text().to_string(), d.revision());
    let before = d.statements().to_vec();
    assert!(d.apply(&[edit(0, 9_999, "x")]).is_err());
    assert!(
        d.apply(&[edit(0, 0, "-- ok\n"), edit(500, 501, "x")])
            .is_err()
    );
    assert_eq!((d.text(), d.revision()), (text.as_str(), revision));
    assert_eq!(d.statements(), before.as_slice());
    // A character is never split.
    let mut c = SqlDocument::new("SELECT 'ç';".into(), pg18(), ClientFlavor::None);
    assert!(c.apply(&[edit(9, 9, "x")]).is_err());
    assert_eq!((c.text(), c.revision()), ("SELECT 'ç';", 0));
}

#[test]
fn set_context_relexes_with_the_new_modes() {
    let mut d = SqlDocument::new(
        "SELECT 'a\\'; SELECT 1;".into(),
        DialectCtx::new(DialectId::MySql, None),
        ClientFlavor::None,
    );
    // With backslash escapes the first string runs on: one statement.
    assert_eq!(d.snapshot().execution_split().statements().count(), 1);
    let mut ctx = DialectCtx::new(DialectId::MySql, None);
    ctx.modes.no_backslash_escapes = true;
    let delta = d.set_context(ctx, ClientFlavor::None);
    assert_eq!(texts(&d), ["SELECT 'a\\'", "SELECT 1"]);
    assert_eq!(
        delta.dirty,
        TextRange::up_to(at(u32::try_from(d.text().len()).unwrap()))
    );
}

#[test]
fn statement_at_finds_the_smart_range() {
    let d = SqlDocument::new(
        "-- t\nSELECT 1;\n\nSELECT 2; -- c\n".into(),
        pg18(),
        ClientFlavor::None,
    );
    assert_eq!(
        d.statement_at(at(0)),
        Some(0),
        "the leading comment belongs to it"
    );
    assert_eq!(d.statement_at(at(8)), Some(0));
    assert_eq!(d.statement_at(at(15)), None, "a blank line is outside");
    assert_eq!(d.statement_at(at(17)), Some(1));
    assert_eq!(
        d.statement_at(at(28)),
        Some(1),
        "the same-line trailing comment"
    );
    assert_eq!(d.snapshot().statement_at(at(17)), Some(1));
}

#[test]
fn split_start_reads_the_state_inside_a_construct() {
    let text = "SELECT 1;\nSELECT $$ a; b $$;\nSELECT 'x;y';";
    let d = SqlDocument::new(text.into(), pg18(), ClientFlavor::None);
    let snap = d.snapshot();
    let in_dollar = u32::try_from(text.find("a;").unwrap()).unwrap();
    assert!(matches!(
        snap.split_start_at(at(in_dollar)).lex,
        LexState::InDollar { .. }
    ));
    assert_eq!(snap.split_start_at(at(10)).lex, LexState::Initial);
    // A selection that starts inside the dollar quote splits from that state.
    let s = snap.split_selection(
        TextRange::new(at(in_dollar), at(u32::try_from(text.len()).unwrap())),
        SplitMode::Execution,
    );
    let got: Vec<&str> = s.statements().map(|st| &text[st.strict]).collect();
    assert_eq!(got, ["a; b $$", "SELECT 'x;y'"]);
    // The body's tail is no statement the server would read the same way: never certain.
    let certain: Vec<&str> = s.runnable().map(|st| &text[st.strict]).collect();
    assert_eq!(certain, ["SELECT 'x;y'"]);
}

fn doc_in(text: &str, id: DialectId, flavor: ClientFlavor) -> sql_split::DocSnapshot {
    SqlDocument::new(text.into(), DialectCtx::new(id, None), flavor).snapshot()
}

/// The selection from right after the first `from` to the text's end, in both modes: every
/// statement's text, and the certain ones.
fn selected(
    snap: &sql_split::DocSnapshot,
    from: &str,
) -> [(Vec<String>, Vec<String>, sql_split::Split); 2] {
    let text = snap.text();
    let start = text.find(from).unwrap() + from.len();
    let r = TextRange::new(
        at(u32::try_from(start).unwrap()),
        at(u32::try_from(text.len()).unwrap()),
    );
    [SplitMode::Execution, SplitMode::Editor].map(|mode| {
        let s = snap.split_selection(r, mode);
        let all = s
            .statements()
            .map(|st| text[st.strict].to_string())
            .collect();
        let certain = s.runnable().map(|st| text[st.strict].to_string()).collect();
        (all, certain, s)
    })
}

// Found by Task 13: a selection that starts inside a token returned the token's tail as a
// certain statement, which a server reading it from a clean state runs as code.
#[test]
fn a_selection_inside_a_token_never_returns_a_certain_tail() {
    use sql_split::AmbiguityReason::UnterminatedConstruct;
    let (my, pg) = (DialectId::MySql, DialectId::Postgres);
    let (cli, psql) = (ClientFlavor::MySqlCli, ClientFlavor::Psql);
    // (text, the selection starts after this, dialect, flavor, the tail's statement)
    let cases = [
        (
            "SELECT 'a; DROP TABLE t; '; SELECT 2;",
            "'a;",
            my,
            cli,
            " DROP TABLE t; '",
        ),
        (
            "SELECT 'a; DROP TABLE t; '; SELECT 2;",
            "'a;",
            pg,
            psql,
            " DROP TABLE t; '",
        ),
        (
            "SELECT 'a; DROP TABLE t; --'; SELECT 2;",
            "'a;",
            pg,
            psql,
            " DROP TABLE t; --'",
        ),
        (
            "SELECT `a; DROP TABLE t; `; SELECT 2;",
            "`a;",
            my,
            cli,
            " DROP TABLE t; `",
        ),
        (
            "SELECT \"a; DROP TABLE t; \"; SELECT 2;",
            "\"a;",
            pg,
            psql,
            " DROP TABLE t; \"",
        ),
        (
            "SELECT $q$ a; DROP TABLE t; $q$; SELECT 2;",
            "$q$ a;",
            pg,
            psql,
            " DROP TABLE t; $q$",
        ),
        (
            "SELECT E'a\\'; DROP TABLE t; '; SELECT 2;",
            "E'a",
            pg,
            psql,
            "\\'; DROP TABLE t; '",
        ),
    ];
    for (text, from, id, flavor, tail) in cases {
        let snap = doc_in(text, id, flavor);
        for (all, certain, s) in selected(&snap, from) {
            assert_eq!(all, [tail.trim(), "SELECT 2"], "{text}");
            assert_eq!(certain, ["SELECT 2"], "{text}");
            assert!(
                s.ambiguous
                    .iter()
                    .any(|a| a.reason == UnterminatedConstruct && text[a.at].trim() == tail.trim()),
                "{text}: {:?}",
                s.ambiguous
            );
        }
    }
}

#[test]
fn a_selection_inside_copy_data_never_returns_a_row_as_certain() {
    for text in [
        "COPY t FROM STDIN;\nDROP TABLE x;\n",
        "COPY t FROM STDIN;\nDROP TABLE x;\n\\.\nSELECT 2;",
        "COPY t FROM STDIN;\nDROP TABLE x;\nDROP TABLE y;\n\\.\nSELECT 2;",
    ] {
        let snap = doc_in(text, DialectId::Postgres, ClientFlavor::Psql);
        for from in ["STDIN;\n", "STDIN;\nDROP"] {
            for (all, certain, s) in selected(&snap, from) {
                assert!(
                    !certain.iter().any(|c| c.contains("TABLE")),
                    "{text:?} from {from:?}: {all:?} / {certain:?}"
                );
                assert!(!s.ambiguous.is_empty(), "{text:?} from {from:?}");
            }
        }
    }
}

#[test]
fn a_selection_inside_an_executable_comment_or_hint_is_uncertain_to_the_end() {
    use sql_split::AmbiguityReason::AmbiguousCommentEnd;
    let cases = [
        // The tail starts with `/`, a space and a three-byte character: no `/*!` opener to
        // skip (this panicked inside a character).
        (
            "SELECT 1 /*! ; */ \u{202E} /*!50000 x */ ; SELECT 2;",
            "/*! ; *",
        ),
        ("SELECT 1 /*! x; DROP TABLE t; */; SELECT 2;", "/*! x;"),
        ("SELECT 1 /*!50000 x */ FROM t; SELECT 2;", "/*!5"),
        ("SELECT 1 /*M! ; é */; SELECT 2;", "/*M! ;"),
        ("SELECT /*+ x */ 1; SELECT 2;", "/*+"),
    ];
    for (text, from) in cases {
        let snap = doc_in(text, DialectId::MariaDb, ClientFlavor::MySqlCli);
        for (all, certain, s) in selected(&snap, from) {
            // What follows is read, and none of it is certain.
            assert!(
                all.iter().any(|a| a.contains("SELECT 2")),
                "{text}: {all:?}"
            );
            assert!(certain.is_empty(), "{text}: {certain:?}");
            assert!(
                s.ambiguous.iter().any(|a| a.reason == AmbiguousCommentEnd),
                "{text}: {:?}",
                s.ambiguous
            );
        }
    }
}

#[test]
fn a_selection_inside_a_plain_comment_or_at_a_clean_place_stays_certain() {
    // The rest of a plain comment is trivia in both readings: what follows is certain.
    let text = "SELECT 1 /* x; DROP TABLE t; */; SELECT 2;";
    let snap = doc_in(text, DialectId::MySql, ClientFlavor::MySqlCli);
    for (all, certain, s) in selected(&snap, "/* x;") {
        assert_eq!(all, ["SELECT 2"]);
        assert_eq!(certain, ["SELECT 2"]);
        assert!(s.ambiguous.is_empty());
    }
    // Between two tokens nothing is carried: a string literal that starts the selection is a
    // whole token, and the server reads it as one.
    let text = "SELECT 'x' 'y; DROP TABLE t; '; SELECT 2;";
    let snap = doc_in(text, DialectId::MySql, ClientFlavor::MySqlCli);
    for (all, certain, _) in selected(&snap, "'x' ") {
        assert_eq!(all, ["'y; DROP TABLE t; '", "SELECT 2"]);
        assert_eq!(certain, all);
    }
}

#[test]
fn a_selection_inside_a_client_command_line_is_the_command_not_sql() {
    let text = "\\echo hi; DROP TABLE x;\nSELECT 2;";
    let snap = doc_in(text, DialectId::Postgres, ClientFlavor::Psql);
    for (all, certain, s) in selected(&snap, "hi;") {
        assert_eq!(all, ["SELECT 2"]);
        assert_eq!(certain, ["SELECT 2"]);
        let [c] = s.client.as_slice() else {
            panic!("one client segment, got {:?}", s.client)
        };
        assert_eq!(&text[c.range], " DROP TABLE x;");
        assert!(!c.supported);
    }
    // A selection that ends inside the same line holds only the command's rest.
    let r = TextRange::new(at(9), at(15));
    let s = snap.split_selection(r, SplitMode::Execution);
    assert_eq!(s.statements().count(), 0);
    assert_eq!(s.client.len(), 1);
    assert_eq!(s.client[0].range, r);

    // mysql `DELIMITER`: the rest of its line is the delimiter's text, and the delimiter it
    // sets is in force after it.
    let text = "DELIMITER //\nSELECT 1; SELECT 2//\nSELECT 3//\n";
    let snap = doc_in(text, DialectId::MySql, ClientFlavor::MySqlCli);
    for (all, certain, s) in selected(&snap, "DELIM") {
        assert_eq!(all, ["SELECT 1; SELECT 2", "SELECT 3"]);
        assert_eq!(certain, ["SELECT 3"]);
        assert_eq!(&text[s.client[0].range], "ITER //");
        assert!(!s.client[0].supported);
    }
}

#[test]
fn split_start_carries_the_client_delimiter() {
    let text = "DELIMITER //\nSELECT 1; SELECT 2//\nSELECT 3//\nDELIMITER ;\nSELECT 4;";
    let d = SqlDocument::new(
        text.into(),
        DialectCtx::new(DialectId::MySql, None),
        ClientFlavor::MySqlCli,
    );
    let snap = d.snapshot();
    let s3 = u32::try_from(text.find("SELECT 3").unwrap()).unwrap();
    let start = snap.split_start_at(at(s3));
    assert_eq!(start.delimiter.as_deref(), Some("//"));
    let end = u32::try_from(text.find("\nDELIMITER ;").unwrap()).unwrap();
    let s = snap.split_selection(TextRange::new(at(s3), at(end)), SplitMode::Execution);
    let got: Vec<&str> = s.statements().map(|st| &text[st.strict]).collect();
    assert_eq!(got, ["SELECT 3"]);
    assert_eq!(
        snap.split_start_at(at(u32::try_from(text.find("SELECT 4").unwrap()).unwrap()))
            .delimiter,
        None
    );
}

#[test]
fn split_start_after_copy_data_and_commands() {
    let text = "\\set x 1\nCOPY t FROM STDIN;\na'b\n\\.\nSELECT 'q;r';\n";
    let d = SqlDocument::new(text.into(), pg18(), ClientFlavor::Psql);
    let snap = d.snapshot();
    let rows = u32::try_from(text.find("a'b").unwrap()).unwrap();
    assert_eq!(snap.split_start_at(at(rows + 1)).lex, LexState::InCopyData);
    let q = u32::try_from(text.find("q;r").unwrap()).unwrap();
    assert!(matches!(
        snap.split_start_at(at(q)).lex,
        LexState::InString { .. }
    ));
    let stmts: Vec<&Stmt> = snap.execution_split().statements().collect();
    assert_eq!(stmts.len(), 2);
}

#[test]
fn from_chunks_equals_new() {
    struct Two<'a>(&'a str, &'a str);
    impl sql_text::TextChunks for Two<'_> {
        fn chunks(&self) -> Box<dyn Iterator<Item = &str> + '_> {
            Box::new([self.0, self.1].into_iter())
        }
        fn len(&self) -> usize {
            self.0.len() + self.1.len()
        }
        fn slice(&self, r: std::ops::Range<usize>) -> std::borrow::Cow<'_, str> {
            std::borrow::Cow::Owned(format!("{}{}", self.0, self.1)[r].to_string())
        }
    }
    let a = SqlDocument::from_chunks(&Two("SELECT 1; SEL", "ECT 2"), pg18(), ClientFlavor::None);
    let b = SqlDocument::new("SELECT 1; SELECT 2".into(), pg18(), ClientFlavor::None);
    assert_eq!(a.statements(), b.statements());
    assert_eq!(a.lexed(), b.lexed());
}

// --- Task 13 review round 1: the cut is decided from the document's tokens and COPY ranges ---

/// `marked` without its `«…»`, and the selection they mark.
fn marked(marked: &str) -> (String, TextRange) {
    let a = marked.find('«').expect("a selection start");
    let rest = marked.replacen('«', "", 1);
    let b = rest.find('»').expect("a selection end");
    let sql = rest.replacen('»', "", 1);
    (
        sql,
        TextRange::new(at(u32::try_from(a).unwrap()), at(u32::try_from(b).unwrap())),
    )
}

/// Every statement and every runnable statement `split_selection` gives for `marked`, in both
/// modes, with the reasons of its boundaries.
fn cut(
    text: &str,
    id: DialectId,
    flavor: ClientFlavor,
) -> Vec<(Vec<String>, Vec<String>, Vec<sql_split::AmbiguityReason>)> {
    let (sql, r) = marked(text);
    let snap = doc_in(&sql, id, flavor);
    [SplitMode::Execution, SplitMode::Editor]
        .into_iter()
        .map(|mode| {
            let s = snap.split_selection(r, mode);
            (
                s.statements()
                    .map(|st| sql[st.strict].to_string())
                    .collect(),
                s.runnable().map(|st| sql[st.strict].to_string()).collect(),
                s.ambiguous.iter().map(|a| a.reason).collect(),
            )
        })
        .collect()
}

// Review I1: the rows of `COPY … FROM STDIN` are data. A selection that starts inside the
// COPY statement (after its first byte) and reaches the rows never returns a row as certain.
#[test]
fn a_selection_from_inside_a_copy_statement_never_returns_its_rows_as_certain() {
    use sql_split::AmbiguityReason::UnterminatedConstruct;
    for text in [
        "COPY t FROM STDIN;«\nDROP TABLE x;\nDELETE FROM y;\n»\\.\nSELECT 9;",
        "COPY t FROM STDIN«;\nDROP TABLE x;\n»\\.\nSELECT 9;",
        "COPY t FROM STD«IN;\nDROP TABLE x;\n»\\.\nSELECT 9;",
        "COPY t FROM STDIN;\n«DROP TABLE x;\n»\\.\nSELECT 9;",
        "COPY t FROM STDIN«;\nDROP TABLE x;\n\\.\nSELECT 9;»",
        "C«OPY t FROM STDIN;\nDROP TABLE x;\n\\.\n»SELECT 9;",
        "COPY t FROM STDIN;\nDROP «TABLE x;\nDELETE FROM y;\n\\.\nSELECT 9;»",
    ] {
        for (all, runnable, reasons) in cut(text, DialectId::Postgres, ClientFlavor::Psql) {
            assert!(
                !runnable
                    .iter()
                    .any(|u| u.contains("TABLE") || u.contains("DELETE") || u == "IN"),
                "{text:?}: {runnable:?} of {all:?}"
            );
            assert!(
                reasons.contains(&UnterminatedConstruct),
                "{text:?}: {reasons:?}"
            );
        }
    }
    // The statement after the data is read from the document's state there.
    let (_, runnable, _) = &cut(
        "COPY t FROM STDIN«;\nDROP TABLE x;\n\\.\nSELECT 9;»",
        DialectId::Postgres,
        ClientFlavor::Psql,
    )[0];
    assert_eq!(runnable, &["SELECT 9"]);
    // A selection that holds the COPY statement from its first byte is the COPY with its data.
    let whole = cut(
        "«COPY t FROM STDIN;\nDROP TABLE x;\n\\.\nSELECT 9;»",
        DialectId::Postgres,
        ClientFlavor::Psql,
    );
    assert_eq!(whole[0].1, ["COPY t FROM STDIN", "SELECT 9"]);
    assert!(whole[0].2.is_empty());
}

// Review I2: "inside a token" is the document's token at the selection's start, openers and
// string prefixes included, not what lexing the token's first bytes alone leaves open.
#[test]
fn a_selection_from_inside_an_opener_or_after_a_string_prefix_is_inside_the_token() {
    use sql_split::AmbiguityReason::{AmbiguousCommentEnd, UnterminatedConstruct};
    let (my, maria, pg) = (DialectId::MySql, DialectId::MariaDb, DialectId::Postgres);
    let (cli, psql) = (ClientFlavor::MySqlCli, ClientFlavor::Psql);
    let cases = [
        (
            "SELECT 1 /*«! , \"*/ DROP TABLE t;» -- \" */ ; SELECT 2;",
            my,
            cli,
            AmbiguousCommentEnd,
        ),
        (
            "SELECT 1 /«*! , \"*/ DROP TABLE t;» -- \" */ ; SELECT 2;",
            my,
            cli,
            AmbiguousCommentEnd,
        ),
        (
            "SELECT /*«+ \"*/ DROP TABLE u;» -- \" */ 2;",
            my,
            cli,
            AmbiguousCommentEnd,
        ),
        (
            "SELECT 1 /*M«! , \"*/ DROP TABLE t;» -- \" */ ; SELECT 2;",
            maria,
            cli,
            AmbiguousCommentEnd,
        ),
        (
            "SELECT 1 /*«M! , \"*/ DROP TABLE t;» -- \" */ ; SELECT 2;",
            maria,
            cli,
            AmbiguousCommentEnd,
        ),
        (
            "SELECT 1 /*«! SELECT 2; */ ; DROP TABLE t;»",
            my,
            cli,
            AmbiguousCommentEnd,
        ),
        (
            "SELECT E«'a\\'; DROP TABLE t; --'»;\nSELECT 2;",
            pg,
            psql,
            UnterminatedConstruct,
        ),
        (
            "SELECT E«'a\\'; DROP TABLE t; --' FROM u; SELECT 3»;\nSELECT 2;",
            pg,
            psql,
            UnterminatedConstruct,
        ),
        (
            "SELECT $ta«g$ a; DROP TABLE t; $tag$; SELECT 2;»",
            pg,
            psql,
            UnterminatedConstruct,
        ),
    ];
    for (text, id, flavor, reason) in cases {
        for (all, runnable, reasons) in cut(text, id, flavor) {
            assert!(
                !runnable.iter().any(|u| u.contains("DROP")),
                "{text:?}: {runnable:?} of {all:?}"
            );
            assert!(reasons.contains(&reason), "{text:?}: {reasons:?}");
        }
    }
}

// Review I3: the whole rest of the document's token is marked, and splitting goes on from the
// document's state after it, so the split never runs out of step with the document.
#[test]
fn a_selection_that_cuts_a_backslash_escape_never_reads_the_strings_rest_as_code() {
    let (my, pg) = (DialectId::MySql, DialectId::Postgres);
    let (cli, psql) = (ClientFlavor::MySqlCli, ClientFlavor::Psql);
    for (text, id, flavor, tail) in [
        (
            "SELECT 'a\\«'; DROP TABLE t; -- ';»\nSELECT 2;",
            my,
            cli,
            "'; DROP TABLE t; -- '",
        ),
        (
            "SELECT E'a\\«'; DROP TABLE t; -- ';»\nSELECT 2;",
            pg,
            psql,
            "'; DROP TABLE t; -- '",
        ),
        (
            "SELECT \"a\\«\"; DROP TABLE t; -- \";»\nSELECT 2;",
            my,
            cli,
            "\"; DROP TABLE t; -- \"",
        ),
    ] {
        for (all, runnable, _) in cut(text, id, flavor) {
            assert_eq!(all, [tail], "{text:?}");
            assert!(runnable.is_empty(), "{text:?}: {runnable:?}");
        }
    }
    // What follows the token in its statement is part of the same uncertain statement.
    let text = "SELECT 'a\\«'b' FROM t; SELECT 3;»";
    for (all, runnable, _) in cut(text, my, cli) {
        assert_eq!(all, ["'b' FROM t", "SELECT 3"]);
        assert_eq!(runnable, ["SELECT 3"]);
    }
    // In a `U&''` string a backslash escapes no quote: the document's string ends at the
    // second quote and `DROP TABLE t` is code there. The token's rest is still not certain.
    let text = "SELECT U&«'a\\'; DROP TABLE t; --'»;\nSELECT 2;";
    for (all, runnable, _) in cut(text, pg, psql) {
        assert_eq!(all, ["'a\\'", "DROP TABLE t"]);
        assert_eq!(runnable, ["DROP TABLE t"]);
    }
    // Out of step: read alone from inside the first string, the quotes pair up the other way
    // and the second string's content reads as code. The document's tokens decide instead.
    let text = "SELECT 'a\\«'b'; SELECT 'c; DROP TABLE t; d'; SELECT 3;»";
    for (all, runnable, _) in cut(text, my, cli) {
        assert_eq!(all, ["'b'", "SELECT 'c; DROP TABLE t; d'", "SELECT 3"]);
        assert_eq!(runnable, ["SELECT 'c; DROP TABLE t; d'", "SELECT 3"]);
    }
}

#[test]
fn a_selection_edge_inside_a_word_or_a_number_cuts_it() {
    use sql_split::AmbiguityReason::UnterminatedConstruct;
    let pg = DialectId::Postgres;
    for text in [
        "«DROP TABLE cust»omers_backup; SELECT 2;",
        "DELETE FROM t WHERE id = 1«2; SELECT 2;»",
        "SEL«ECT 1»; SELECT 2;",
        "SEL«ECT 1 FROM t; SELECT 2;»",
        "SELECT a «>»= 1;",
        "SELECT 1«0»0;",
    ] {
        for (all, runnable, reasons) in cut(text, pg, ClientFlavor::Psql) {
            assert!(
                !runnable.iter().any(|u| u != "SELECT 2"),
                "{text:?}: {runnable:?} of {all:?}"
            );
            assert!(
                reasons.contains(&UnterminatedConstruct),
                "{text:?}: {reasons:?}"
            );
        }
    }
    let (sql, r) = marked("«DROP TABLE cust»omers_backup; SELECT 2;");
    let cuts = doc_in(&sql, pg, ClientFlavor::Psql).selection_cuts(r);
    assert_eq!(cuts.len(), 1);
    assert_eq!(&sql[cuts[0].at], "cust");
    // Between tokens, in white space and inside plain comments nothing is cut.
    for text in [
        "«SELECT 1»; SELECT 2;",
        "SELECT «1; SELECT 2»;",
        "SELECT 1; « SELECT 2; »",
        "-- old: «SELECT 1;» kept\nSELECT 2;",
        "/* «x» */ SELECT 2;",
    ] {
        let (sql, r) = marked(text);
        assert!(
            doc_in(&sql, pg, ClientFlavor::Psql)
                .selection_cuts(r)
                .is_empty(),
            "{text:?}"
        );
    }
}

#[test]
fn after_copy_data_both_lexings_are_asked() {
    // The document's one lexing reads the quote in the data as a string's start, so to it
    // `SELECT 'after'` starts inside a string; the splitter lexes again after the data and
    // reads a statement. Either one saying "inside" refuses (fail closed).
    let text = "COPY t FROM stdin;\nO'Brien\t1\n\\.\n«SELECT 'after';»\n";
    for (_, runnable, reasons) in cut(text, DialectId::Postgres, ClientFlavor::Psql) {
        assert!(runnable.is_empty(), "{runnable:?}");
        assert!(!reasons.is_empty());
    }
    // Without such a quote the two agree, and the statement after the data is certain.
    let text = "COPY t FROM stdin;\nBrien\t1\n\\.\n«SELECT 'after';»\n";
    for (_, runnable, reasons) in cut(text, DialectId::Postgres, ClientFlavor::Psql) {
        assert_eq!(runnable, ["SELECT 'after'"]);
        assert!(reasons.is_empty());
    }
    // Where only the second lexing sees the token: inside a word after the data.
    let text = "COPY t FROM stdin;\nBrien\t1\n\\.\nSEL«ECT 'after';»\n";
    for (_, runnable, _) in cut(text, DialectId::Postgres, ClientFlavor::Psql) {
        assert!(runnable.is_empty(), "{runnable:?}");
    }
}

// --- Task 13 review round 2 ---------------------------------------------------------------

// Re-review I1: the COPY statement is not always the item just before its rows.
#[test]
fn copy_rows_are_data_when_another_item_shares_the_copy_line() {
    for text in [
        "COPY t FROM STDIN; SELECT 1;«\nDROP TABLE x;»\nDELETE FROM y;\n\\.\n",
        "COPY t FROM STDIN; SELECT «1;\nDROP TABLE x;»\nDELETE FROM y;\n\\.\n",
        "COPY t FROM STDIN; /* c */ ;«\nDROP TABLE x;»\nDELETE FROM y;\n\\.\n",
        "COPY t FROM STDIN; SELECT 1 «-- c\nDROP TABLE x»;\n\\.\n",
        "COPY t FROM STDIN; ;«\nDROP TABLE x;\n»",
        "COPY t FROM STDIN; SELECT 1;\n«DROP TABLE x;\n»\\.\n",
        "COPY t FROM STDIN; «SELECT 1;\nDROP TABLE x;\n\\.\nSELECT 9;»",
    ] {
        let (sql, r) = marked(text);
        let snap = doc_in(&sql, DialectId::Postgres, ClientFlavor::Psql);
        assert!(!snap.selection_cuts(r).is_empty(), "{text:?}");
        for (all, runnable, reasons) in cut(text, DialectId::Postgres, ClientFlavor::Psql) {
            assert!(
                !runnable
                    .iter()
                    .any(|u| u.contains("DROP") || u.contains("DELETE") || u == "1"),
                "{text:?}: {runnable:?} of {all:?}"
            );
            assert!(!reasons.is_empty(), "{text:?}");
        }
    }
    // After the data the split goes on from the document's state.
    let (_, runnable, _) = &cut(
        "COPY t FROM STDIN; «SELECT 1;\nDROP TABLE x;\n\\.\nSELECT 9;»",
        DialectId::Postgres,
        ClientFlavor::Psql,
    )[0];
    assert_eq!(runnable, &["SELECT 9"]);
    // A selection that holds the COPY from its first byte reads the same data.
    let whole = cut(
        "«COPY t FROM STDIN; SELECT 1;\nDROP TABLE x;\n\\.\nSELECT 9;»",
        DialectId::Postgres,
        ClientFlavor::Psql,
    );
    assert!(
        !whole[0].1.iter().any(|u| u.contains("DROP")),
        "{:?}",
        whole[0]
    );
    assert!(whole[0].1.contains(&"SELECT 9".to_string()));
}

// Re-review M3: between an executable comment's lexer end and the end the server may read.
#[test]
fn an_edge_inside_an_ambiguous_comment_end_is_a_cut() {
    use sql_split::AmbiguityReason::AmbiguousCommentEnd;
    for text in [
        "SELECT 1 /*! , \"*/« DROP TABLE t;» -- \" */ ;",
        "SELECT /*+ \"*/« DROP TABLE u;» -- \" */ 2;",
        "«SELECT 1 /*! , \"*/ DROP» TABLE t; -- \" */ ;",
    ] {
        let (sql, r) = marked(text);
        let snap = doc_in(&sql, DialectId::MySql, ClientFlavor::MySqlCli);
        let cuts = snap.selection_cuts(r);
        assert!(
            cuts.iter().any(|c| c.reason == AmbiguousCommentEnd),
            "{text:?}: {cuts:?}"
        );
        for (all, runnable, _) in cut(text, DialectId::MySql, ClientFlavor::MySqlCli) {
            assert!(runnable.is_empty(), "{text:?}: {runnable:?} of {all:?}");
        }
    }
}

// Re-review M4: the public entry points take any offsets.
#[test]
fn selection_offsets_inside_a_character_are_moved_back_not_panicked_on() {
    let sql = "SELECT 1; -- é x\nSELECT 'é'; /* é */ SELECT 2;";
    let snap = doc_in(sql, DialectId::Postgres, ClientFlavor::Psql);
    let len = u32::try_from(sql.len()).unwrap();
    for a in 0..=len + 2 {
        let state = snap.split_start_at(at(a));
        if !sql.is_char_boundary(a.min(len) as usize) {
            assert_eq!(state, snap.split_start_at(at(a - 1)), "{a}");
        }
        for b in a..=len + 2 {
            let r = TextRange::new(at(a), at(b));
            let cuts = snap.selection_cuts(r);
            let trimmed = snap.without_comment_parts(r);
            assert!(
                sql.get(usize::from(trimmed.start())..usize::from(trimmed.end()))
                    .is_some()
            );
            for c in cuts {
                assert!(
                    sql.get(usize::from(c.at.start())..usize::from(c.at.end()))
                        .is_some()
                );
            }
        }
    }
    // The reviewer's case: a start inside `é` in a `--` comment.
    let mid = u32::try_from(sql.find('é').unwrap()).unwrap() + 1;
    assert_eq!(
        snap.selection_cuts(TextRange::new(at(mid), at(mid + 3))),
        snap.selection_cuts(TextRange::new(at(mid - 1), at(mid + 3)))
    );
    // The same after a client command, where the second lexing is asked.
    let sql = "\\set x 1\nSELECT 1; -- é x\nSELECT 'é';";
    let snap = doc_in(sql, DialectId::Postgres, ClientFlavor::Psql);
    let len = u32::try_from(sql.len()).unwrap();
    for a in 0..=len {
        let _ = snap.split_start_at(at(a));
        let _ = snap.selection_cuts(TextRange::new(at(a), at(len)));
        let _ = snap.selection_cuts(TextRange::new(at(0), at(a)));
    }
}

// Re-review I2: after a restart the splitter's lexing is picked up at a checkpoint, and the
// answer is the one a lex from the restart gives.
#[test]
fn a_split_start_far_after_a_client_command_reads_the_same_state() {
    let body = "SELECT 'a;b', $q$ x; y $q$, 1 /* c */ FROM t -- d\n;\n".repeat(400);
    let sql = format!("\\set x 1\n{body}");
    let plain = doc_in(&body, DialectId::Postgres, ClientFlavor::Psql);
    let snap = doc_in(&sql, DialectId::Postgres, ClientFlavor::Psql);
    let shift = u32::try_from(sql.len() - body.len()).unwrap();
    let len = u32::try_from(body.len()).unwrap();
    // Every offset near the text's end, and a spread over the whole text.
    let offsets = (len - 400..=len).chain((0..len).step_by(97));
    for o in offsets {
        if !body.is_char_boundary(o as usize) {
            continue;
        }
        assert_eq!(
            snap.split_start_at(at(o + shift)).lex,
            plain.split_start_at(at(o)).lex,
            "offset {o}"
        );
        let r = TextRange::new(at(o), at(len));
        let moved = TextRange::new(at(o + shift), at(len + shift));
        let cuts: Vec<_> = snap
            .selection_cuts(moved)
            .into_iter()
            .map(|c| (c.at - at(shift), c.reason))
            .collect();
        let want: Vec<_> = plain
            .selection_cuts(r)
            .into_iter()
            .map(|c| (c.at, c.reason))
            .collect();
        // What is cut without the command is cut after it; and between two tokens nothing
        // is. (Inside a plain comment the two differ on purpose: after a restart a comment
        // of the document's one lexing is not trusted, so its edge is a cut.)
        if !want.is_empty() {
            assert!(!cuts.is_empty(), "offset {o}: {want:?}");
        }
        let l = plain.lexed();
        let between = l
            .token_at(at(o))
            .is_none_or(|i| l.range(i).start() == at(o));
        if between {
            assert!(cuts.is_empty(), "offset {o}: {cuts:?}");
        }
    }
}
