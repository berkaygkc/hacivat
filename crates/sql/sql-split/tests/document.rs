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
    let delta = d.apply(&[edit(18, 18, "0")]);
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
    let delta = d.apply(&[edit(8, 10, " ")]);
    assert_eq!(texts(&d), ["SELECT 1 SELECT 20", "SELECT 3"]);
    assert_eq!(delta.removed.len(), 2);
    assert_eq!(delta.changed.len(), 1);
    assert_eq!(delta.moved.len(), 1);
    // Deleting only the `;` leaves a heuristic boundary where the line breaks.
    let mut e = SqlDocument::new("SELECT 1;\nSELECT 2;".into(), pg18(), ClientFlavor::None);
    let _ = e.apply(&[edit(8, 9, "")]);
    assert_eq!(texts(&e), ["SELECT 1", "SELECT 2"]);
    assert!(e.statements().iter().all(is_heuristic));
    // An edit that does not fit is skipped, never a panic.
    let text = d.text().to_string();
    let _ = d.apply(&[edit(0, 9_999, "x")]);
    assert_eq!(d.text(), text);
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
