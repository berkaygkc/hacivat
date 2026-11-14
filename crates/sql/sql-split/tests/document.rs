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
            assert_eq!(all, [tail, "SELECT 2"], "{text}");
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
