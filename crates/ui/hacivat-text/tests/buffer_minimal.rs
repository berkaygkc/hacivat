//! M3a Task 1b: the working `TextBuffer` and the minimal `EditorModel::dispatch` (Q8) that
//! Tasks 5, 10 and 19 build on.

use std::sync::Arc;
use std::time::Instant;

use hacivat_text::{
    Annotations, Bias, ChangeSet, EditorModel, LineEnding, ModelConfig, Origin, SelectionRange,
    SelectionSet, TextBuffer, Transaction, UserEvent,
};

fn typed(changes: ChangeSet, selection: Option<SelectionSet>) -> Transaction {
    Transaction {
        changes,
        selection,
        ann: Annotations {
            user_event: UserEvent::InputType,
            add_to_history: true,
            time: Instant::now(),
            origin: Origin::User,
        },
    }
}

/// Inserts `text` at byte `at` of `m`'s document, the selection mapped.
fn insert_at_in(m: &EditorModel, at: usize, text: &str) -> Transaction {
    let len = m.snapshot().buffer.len_bytes();
    typed(
        ChangeSet::from_edits(len, [(at..at, Arc::from(text))]).unwrap(),
        None,
    )
}

#[test]
fn utf16_round_trips_turkish_and_emoji() {
    let b = TextBuffer::from_str("İı😀x");
    assert_eq!(b.len_utf16(), 5);
    assert_eq!(b.utf16_to_byte(b.byte_to_utf16(8)), 8);
    assert_eq!(b.byte_to_utf16(4), 2); // after `İı`
    assert_eq!(b.utf16_to_byte(3), 4); // inside the surrogate pair: the pair's start
}

#[test]
fn crlf_is_one_line_break() {
    assert_eq!(TextBuffer::from_str("a\r\nb").len_lines(), 2);
    let b = TextBuffer::from_str("a\r\nb\rc\nd");
    assert_eq!(b.len_lines(), 4);
    assert_eq!(b.to_string(), "a\r\nb\rc\nd");
    assert_eq!(b.line_range(0, false), 0..1);
    assert_eq!(b.line_range(0, true), 0..3);
    // Never between the CR and LF of a CRLF.
    assert_eq!(b.snap(2, Bias::Before), 1);
    assert_eq!(b.snap(2, Bias::After), 3);
}

#[test]
fn u2028_is_not_a_line_break() {
    assert_eq!(TextBuffer::from_str("a\u{2028}b").len_lines(), 1);
    assert_eq!(TextBuffer::from_str("x\u{85}y").len_lines(), 1);
}

#[test]
fn bom_moves_into_meta() {
    let b = TextBuffer::from_str("\u{FEFF}x");
    assert!(b.meta().bom);
    assert_eq!(b.to_string(), "x");
    assert!(!TextBuffer::from_str("x\u{FEFF}").meta().bom);
}

#[test]
fn snapping_keeps_to_char_boundaries() {
    let b = TextBuffer::from_str("çğ");
    assert_eq!(b.snap(1, Bias::Before), 0);
    assert_eq!(b.snap(1, Bias::After), 2);
    assert_eq!(b.snap(99, Bias::After), 4);
}

#[test]
fn dominant_line_ending_and_content_hash() {
    let crlf = TextBuffer::from_str("a\r\nb\r\nc\n");
    assert_eq!(crlf.dominant_line_ending(), LineEnding::CrLf);
    assert_eq!(
        TextBuffer::from_str("a").dominant_line_ending(),
        LineEnding::Lf
    );
    assert_eq!(
        TextBuffer::from_str("SELECT 1").content_hash(),
        TextBuffer::from_str("SELECT 1").content_hash()
    );
    assert_ne!(
        TextBuffer::from_str("SELECT 1").content_hash(),
        TextBuffer::from_str("SELECT 2").content_hash()
    );
}

#[test]
fn dispatch_inserts_and_maps_the_selection() {
    let mut m = EditorModel::new("ab", ModelConfig::default());
    let before = m.snapshot();
    let c = m.dispatch(insert_at_in(&m, 1, "X")).unwrap();
    assert_eq!(m.snapshot().buffer.to_string(), "aXb");
    assert_eq!(c.version, 1);
    assert!(c.buffer_changed);
    // The caret at 0 stays before the insertion; a caret at the insertion point moves past it.
    assert_eq!(m.snapshot().selection.main().head, 0);
    let at_end = typed(
        ChangeSet::from_edits(3, [(3..3, Arc::from("ç"))]).unwrap(),
        Some(SelectionSet::cursor(3)),
    );
    m.dispatch(at_end).unwrap();
    let mapped = m.dispatch(insert_at_in(&m, 3, "Z")).unwrap();
    assert_eq!(m.snapshot().buffer.to_string(), "aXbZç");
    assert_eq!(m.snapshot().selection.main().head, 4);
    assert_eq!(mapped.version, 3);
    // A snapshot is unchanged by later edits.
    assert_eq!(before.buffer.to_string(), "ab");
    assert_eq!(before.version, 0);
    assert_eq!(m.changelog().version(), 3);
}

#[test]
fn dispatch_replaces_deletes_and_sets_a_given_selection() {
    let mut m = EditorModel::new("SELECT 1;\r\nSELECT 2;", ModelConfig::default());
    let len = m.snapshot().buffer.len_bytes();
    let changes = ChangeSet::from_edits(
        len,
        [
            (7..8, Arc::from("42")),
            (9..11, Arc::from("")),
            (0..0, Arc::from("-- x\n")),
        ],
    )
    .unwrap();
    let sel = SelectionSet::single(SelectionRange {
        anchor: 0,
        head: 4,
        goal_x: None,
        assoc: Bias::After,
    });
    m.dispatch(typed(changes, Some(sel.clone()))).unwrap();
    assert_eq!(m.snapshot().buffer.to_string(), "-- x\nSELECT 42;SELECT 2;");
    assert_eq!(m.snapshot().selection, sel);
}

#[test]
fn dispatch_vetoes_a_change_set_of_another_length() {
    let mut m = EditorModel::new("abc", ModelConfig::default());
    let wrong = typed(
        ChangeSet::from_edits(2, [(0..0, Arc::from("x"))]).unwrap(),
        None,
    );
    assert!(m.dispatch(wrong).is_err());
    assert_eq!(m.version(), 0);
    assert_eq!(m.snapshot().buffer.to_string(), "abc");
}

#[test]
fn overlapping_edits_are_refused() {
    assert!(ChangeSet::from_edits(5, [(0..3, Arc::from("")), (2..4, Arc::from(""))]).is_err());
    assert!(ChangeSet::from_edits(5, [(4..6, Arc::from(""))]).is_err());
}
