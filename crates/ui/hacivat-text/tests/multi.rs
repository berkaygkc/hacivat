//! UI-EDIT-05 multi (M3b Task 31): carets added at a point, above or below at the goal column,
//! the next or every occurrence (case- and Turkish-exact, whole words from a caret), column
//! selection in display columns, a selection split into lines, typing at every caret as one
//! undo group, the IME composition mirrored to every caret, and the selection-only history
//! (`undo_selection`).

use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_text::{
    Annotations, ChangeSet, EditorModel, ImeCore, MAX_OCCURRENCES, ModelConfig, Origin,
    SelectionRange, SelectionSet, TextBuffer, TokenOracle, Transaction, UserEvent, add_caret,
    add_caret_vertical, carets_in_plain_text, column_selection, select_all_occurrences,
    select_next_occurrence, split_into_lines,
};
use smallvec::SmallVec;

fn ends(s: &SelectionSet) -> Vec<(usize, usize)> {
    s.ranges().iter().map(|r| (r.anchor, r.head)).collect()
}

fn carets(at: &[usize], main: usize) -> SelectionSet {
    SelectionSet::normalized(at.iter().map(|&a| SelectionRange::caret(a)), main)
}

fn select(m: &EditorModel, sel: SelectionSet) -> Transaction {
    Transaction {
        changes: ChangeSet::identity(m.buffer().len_bytes()),
        selection: Some(sel),
        ann: Annotations {
            user_event: UserEvent::Select,
            add_to_history: true,
            time: Instant::now(),
            origin: Origin::User,
        },
    }
}

#[test]
fn ui_edit_05_cases_3_and_5() {
    // 3. `SELECT id FROM t WHERE id = 1`, a caret in the first `id`, add-next twice: the word,
    //    then both `id`s, the new one the main range.
    let buf = TextBuffer::from_str("SELECT id FROM t WHERE id = 1");
    let one = select_next_occurrence(&buf, &SelectionSet::cursor(8)).expect("the word");
    assert_eq!(ends(&one), [(7, 9)]);
    let two = select_next_occurrence(&buf, &one).expect("the second id");
    assert_eq!(ends(&two), [(7, 9), (23, 25)]);
    assert_eq!(two.main_index(), 1);
    // No third one: the search wraps and finds only what is selected.
    assert_eq!(select_next_occurrence(&buf, &two), None);
    // Whole words from a caret: never the `id` inside `idx`, `id_x` or `xid`.
    let buf = TextBuffer::from_str("SELECT id, idx, id_x, xid FROM t WHERE id = 1");
    let word = select_next_occurrence(&buf, &SelectionSet::cursor(7)).unwrap();
    let next = select_next_occurrence(&buf, &word).unwrap();
    assert_eq!(ends(&next), [(7, 9), (39, 41)]);
    assert_eq!(
        ends(&select_all_occurrences(&buf, &word, MAX_OCCURRENCES)),
        [(7, 9), (39, 41)]
    );

    // 5. A box over display columns [4, 8) of `a\tb` (tab width 4) and `abcdefgh`: the first
    //    line from the tab's end to `b`, the second `efgh`; the main range on the head's line.
    let buf = TextBuffer::from_str("a\tb\nabcdefgh");
    let boxed = column_selection(&buf, (0, 4), (1, 8), 4);
    assert_eq!(ends(&boxed), [(2, 3), (8, 12)]);
    assert_eq!(boxed.main_index(), 1);
}

#[test]
fn select_next_occurrence_is_case_and_turkish_exact() {
    // "İ" (U+0130) and "ı" (U+0131) are two bytes each: "id"0..2 "İd"3..6 "ID"7..9 "Id"10..12
    // "id"13..15 "ıd"16..19.
    let buf = TextBuffer::from_str("id İd ID Id id ıd");
    let first = SelectionSet::single(SelectionRange::new(0, 2));
    let next = select_next_occurrence(&buf, &first).expect("the other id");
    assert_eq!(ends(&next), [(0, 2), (13, 15)]);
    assert_eq!(select_next_occurrence(&buf, &next), None);
    assert_eq!(
        ends(&select_all_occurrences(&buf, &first, MAX_OCCURRENCES)),
        [(0, 2), (13, 15)]
    );
    // "İd" has no other occurrence: "id", "Id" and "ıd" are other words.
    let dotted = SelectionSet::single(SelectionRange::new(3, 6));
    assert_eq!(select_next_occurrence(&buf, &dotted), None);
    // A part of a word is matched as a substring, still exactly: "i" never matches "İ" or "ı".
    let i = SelectionSet::single(SelectionRange::new(0, 1));
    assert_eq!(
        ends(&select_next_occurrence(&buf, &i).unwrap()),
        [(0, 1), (13, 14)]
    );
    // Select all occurrences stops at the cap; the main range stays the one asked from.
    let buf = TextBuffer::from_str(&"ab ".repeat(50));
    let ab = SelectionSet::single(SelectionRange::new(9, 11));
    let capped = select_all_occurrences(&buf, &ab, 10);
    assert_eq!(capped.ranges().len(), 10);
    assert_eq!(capped.main().range(), 9..11);
}

#[test]
fn column_selection_uses_display_columns_with_tabs_and_wide_chars() {
    // Line 0 `\tab` (the tab fills cells 0–3), line 1 `日本語x` (each ideograph 2 cells), line 2
    // `ab` (shorter than the box), line 3 `abcdefghij`.
    let buf = TextBuffer::from_str("\tab\n日本語x\nab\nabcdefghij");
    // Cells [3, 5): a tab or a wide character the box cuts is taken whole; a line shorter than
    // the box gets a caret at its end (no virtual space).
    let boxed = column_selection(&buf, (0, 3), (3, 5), 4);
    assert_eq!(ends(&boxed), [(0, 2), (7, 13), (17, 17), (21, 23)]);
    assert_eq!(boxed.main_index(), 3);
    // Dragged up and left: each range keeps the drag's direction; the main one is on line 0.
    let back = column_selection(&buf, (3, 5), (0, 3), 4);
    assert_eq!(ends(&back), [(2, 0), (13, 7), (17, 17), (23, 21)]);
    assert_eq!(back.main_index(), 0);
    // A zero-width box is a caret per line, inside a wide character at its start.
    let thin = column_selection(&buf, (0, 5), (1, 5), 4);
    assert_eq!(ends(&thin), [(2, 2), (10, 10)]);
}

#[test]
fn add_caret_and_add_caret_vertical_keep_the_goal_column() {
    let buf = TextBuffer::from_str(&format!(
        "{}\n{}\n{}",
        "a".repeat(25),
        "b".repeat(5),
        "c".repeat(30)
    ));
    let line2 = buf.line_to_byte(1);
    let line3 = buf.line_to_byte(2);
    // UI-EDIT-05 case 4 for an added caret: col 20 onto a 5-column line, then a 30-column one.
    let one = add_caret_vertical(&buf, &SelectionSet::cursor(20), true, 4);
    assert_eq!(
        one.ranges().iter().map(|r| r.head).collect::<Vec<_>>(),
        [20, line2 + 5]
    );
    assert_eq!(one.main_index(), 1);
    let two = add_caret_vertical(&buf, &one, true, 4);
    assert_eq!(
        two.ranges().iter().map(|r| r.head).collect::<Vec<_>>(),
        [20, line2 + 5, line3 + 20]
    );
    // Nothing above the first line: the set is unchanged.
    assert_eq!(
        add_caret_vertical(&buf, &SelectionSet::cursor(3), false, 4),
        SelectionSet::cursor(3)
    );
    // A caret added at a point becomes the main one; one on an existing caret merges.
    let added = add_caret(&SelectionSet::cursor(3), line2);
    assert_eq!(ends(&added), [(3, 3), (line2, line2)]);
    assert_eq!(added.main_index(), 1);
    assert_eq!(add_caret(&added, 3).ranges().len(), 2);
}

#[test]
fn split_into_lines_gives_one_range_per_line() {
    let buf = TextBuffer::from_str("abc\ndef\nghi\n");
    // 1..9 covers "bc", "def" and "g"; a selection that ends at a line's start leaves it out.
    let split = split_into_lines(&buf, &SelectionSet::single(SelectionRange::new(1, 9)));
    assert_eq!(ends(&split), [(1, 3), (4, 7), (8, 9)]);
    assert_eq!(split.main_index(), 2);
    let to_start = split_into_lines(&buf, &SelectionSet::single(SelectionRange::new(4, 8)));
    assert_eq!(ends(&to_start), [(4, 7)]);
}

#[test]
fn undo_selection_restores_carets_without_text_change() {
    let mut m = EditorModel::new("a\nb\nc\nd\n", ModelConfig::default());
    m.dispatch(select(&m, carets(&[0, 2], 1))).unwrap();
    m.dispatch(select(&m, carets(&[0, 2, 4], 2))).unwrap();
    let (version, depth) = (m.version(), m.undo_depth());
    let c = m.undo_selection().expect("a selection to take back");
    assert!(!c.buffer_changed);
    assert_eq!(ends(m.selection()), [(0, 0), (2, 2)]);
    assert_eq!(m.selection().main_index(), 1);
    assert_eq!((m.version(), m.undo_depth()), (version, depth));
    assert_eq!(m.buffer().to_string(), "a\nb\nc\nd\n");
    m.undo_selection().unwrap();
    assert_eq!(ends(m.selection()), [(0, 0)]);
    // Redo walks forward again; a new selection change clears what could be redone.
    m.redo_selection().unwrap();
    assert_eq!(ends(m.selection()), [(0, 0), (2, 2)]);
    m.dispatch(select(&m, carets(&[6], 0))).unwrap();
    assert_eq!(m.redo_selection(), None);
    // A stored selection follows the text: typing at three carets, then undo selection gives
    // the selection before the last selection change, mapped through the typing.
    m.dispatch(select(&m, carets(&[0, 2, 4], 2))).unwrap();
    let mut ime = ImeCore::default();
    ime.commit(&mut m, None, "x", Instant::now()).unwrap();
    assert_eq!(m.buffer().to_string(), "xa\nxb\nxc\nd\n");
    m.undo_selection().unwrap();
    assert_eq!(ends(m.selection()), [(9, 9)]);
    assert_eq!(m.buffer().to_string(), "xa\nxb\nxc\nd\n");
}

#[test]
fn typing_at_many_carets_is_one_undo_group() {
    let text = "SELECT a;\n".repeat(100);
    let mut m = EditorModel::new(&text, ModelConfig::default());
    let at: Vec<usize> = (0..100).map(|l| m.buffer().line_to_byte(l)).collect();
    m.dispatch(select(&m, carets(&at, 99))).unwrap();
    let mut ime = ImeCore::default();
    let t0 = Instant::now();
    for (i, c) in ["-", "-", " "].into_iter().enumerate() {
        ime.commit(&mut m, None, c, t0 + Duration::from_millis(10 * i as u64))
            .unwrap();
    }
    assert_eq!(m.buffer().to_string(), "-- SELECT a;\n".repeat(100));
    assert_eq!(m.selection().ranges().len(), 100);
    assert_eq!(m.selection().main_index(), 99);
    assert!(m.selection().ranges().iter().all(|r| r.is_empty()));
    assert_eq!(m.undo_depth(), 1, "one group");
    m.undo();
    assert_eq!(m.buffer().to_string(), text);
    assert_eq!(
        ends(m.selection()),
        at.iter().map(|&a| (a, a)).collect::<Vec<_>>()
    );
}

#[test]
#[ignore = "perf"]
fn typing_at_10k_carets_under_50ms() {
    if std::env::var_os("HACIVAT_PERF").is_none_or(|v| v != "1") {
        return;
    }
    let text = "SELECT a FROM t;\n".repeat(10_000);
    let mut m = EditorModel::new(&text, ModelConfig::default());
    let at: Vec<usize> = (0..10_000).map(|l| m.buffer().line_to_byte(l)).collect();
    m.dispatch(select(&m, carets(&at, 0))).unwrap();
    let mut ime = ImeCore::default();
    let mut worst = Duration::ZERO;
    let t0 = Instant::now();
    for (i, c) in "SELECT 1;".chars().enumerate() {
        let start = Instant::now();
        ime.commit(
            &mut m,
            None,
            &c.to_string(),
            t0 + Duration::from_millis(i as u64),
        )
        .unwrap();
        worst = worst.max(start.elapsed());
    }
    assert_eq!(m.selection().ranges().len(), 10_000);
    assert!(m.buffer().to_string().starts_with("SELECT 1;SELECT a"));
    eprintln!("perf typing_at_10k_carets: worst keystroke {worst:?} (budget 50 ms)");
    assert!(worst < Duration::from_millis(50), "{worst:?}");
}

/// Strings are `'…'`: a position strictly inside one, or at the end of an unterminated one.
struct Quotes(String);

impl TokenOracle for Quotes {
    fn words_on_line(&self, _: Range<usize>) -> Option<SmallVec<[Range<usize>; 16]>> {
        None
    }

    fn in_string_or_comment(&self, at: usize) -> Option<bool> {
        let quotes = self.0[..at].matches('\'').count();
        Some(quotes % 2 == 1)
    }
}

#[test]
fn the_ime_composition_mirrors_to_every_caret() {
    // UI-EDIT-12 test 5 on the model: three carets, a composition, a commit, one undo.
    let mut m = EditorModel::new("a\nb\nc\n", ModelConfig::default());
    m.dispatch(select(&m, carets(&[1, 3, 5], 1))).unwrap();
    let mut ime = ImeCore::default();
    let now = Instant::now();
    ime.mark(&mut m, None, "か", None, now).unwrap();
    ime.mark(&mut m, None, "かな", Some(0..1), now).unwrap();
    assert_eq!(m.buffer().to_string(), "aかな\nbかな\ncかな\n");
    // The platform sees the main caret's composition only; the underline is at every caret.
    assert_eq!(ime.marked(&m), Some(9..15));
    assert_eq!(ime.marked_all(&m).to_vec(), [1..7, 9..15, 17..23]);
    assert_eq!(ends(m.selection()), [(1, 4), (9, 12), (17, 20)]);
    assert_eq!(m.selection().main_index(), 1);
    ime.commit(&mut m, None, "仮名", now).unwrap();
    assert_eq!(m.buffer().to_string(), "a仮名\nb仮名\nc仮名\n");
    assert_eq!(ends(m.selection()), [(7, 7), (15, 15), (23, 23)]);
    assert!(!ime.is_composing());
    m.undo();
    assert_eq!(m.buffer().to_string(), "a\nb\nc\n");
    assert_eq!(m.undo_depth(), 0, "one undo group");
}

#[test]
fn carets_in_plain_text_asks_the_token_oracle() {
    let text = "SELECT 'ab', x";
    let tok = Quotes(text.to_string());
    assert!(carets_in_plain_text(&carets(&[0, 13], 0), &tok));
    assert!(!carets_in_plain_text(&carets(&[0, 9], 0), &tok));
    // An oracle that does not know answers plain text.
    struct Unknown;
    impl TokenOracle for Unknown {
        fn words_on_line(&self, _: Range<usize>) -> Option<SmallVec<[Range<usize>; 16]>> {
            None
        }
    }
    assert!(carets_in_plain_text(&carets(&[0, 9], 0), &Unknown));
    let _ = Arc::<str>::from("");
}
