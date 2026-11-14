//! UI-EDIT-05 model (M3a Task 10): a `SelectionSet` holds many ranges, sorted and merged after
//! every change with the main one followed; ranges map through edits (carets by `assoc`,
//! selections never growing); vertical moves keep the goal column; Escape collapses.

use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use hacivat_text::units::{grapheme_column, offset_at_column};
use hacivat_text::{
    Annotations, Bias, ChangeSet, EditorModel, ModelConfig, Origin, SelectionRange, SelectionSet,
    TextBuffer, Transaction, UserEvent,
};
use proptest::prelude::*;

fn sel(anchor: usize, head: usize) -> SelectionRange {
    SelectionRange::new(anchor, head)
}

fn ends(s: &SelectionSet) -> Vec<(usize, usize)> {
    s.ranges().iter().map(|r| (r.anchor, r.head)).collect()
}

fn edit(len: usize, edits: &[(Range<usize>, &str)]) -> ChangeSet {
    ChangeSet::from_edits(len, edits.iter().map(|(r, t)| (r.clone(), Arc::from(*t)))).unwrap()
}

/// x = 8 px per grapheme column (the test's layout).
fn x_of(b: &TextBuffer) -> impl Fn(usize) -> f32 + Copy + '_ {
    move |at| grapheme_column(b, at) as f32 * 8.0
}

fn at_x(b: &TextBuffer) -> impl Fn(usize, f32) -> usize + Copy + '_ {
    move |line, x| offset_at_column(b, line, (x / 8.0).round() as usize)
}

#[test]
fn ui_edit_05_cases_1_2_4_6() {
    // 1. [5,8) and [7,10) normalise to [5,10).
    let s = SelectionSet::normalized([sel(5, 8), sel(7, 10)], 0);
    assert_eq!(ends(&s), [(5, 10)]);

    // 2. Two cursors at 3 collapse to one (the main one survives).
    let s = SelectionSet::normalized([SelectionRange::caret(3), SelectionRange::caret(3)], 1);
    assert_eq!(ends(&s), [(3, 3)]);
    assert_eq!(s.main_index(), 0);

    // 4. The goal column: line 1 col 20, down onto a 5-column line, then a 30-column line →
    //    col 5, then col 20 again (x comes from the test's 8 px columns).
    let b = TextBuffer::from_str(&format!(
        "{}\n{}\n{}",
        "a".repeat(25),
        "b".repeat(5),
        "c".repeat(30)
    ));
    let line2 = b.line_to_byte(1);
    let line3 = b.line_to_byte(2);
    let down1 = SelectionRange::caret(20).moved_vertically(&b, 1, false, x_of(&b), at_x(&b));
    assert_eq!(down1.head, line2 + 5);
    assert_eq!(down1.goal_x, Some(160.0));
    let down2 = down1.moved_vertically(&b, 1, false, x_of(&b), at_x(&b));
    assert_eq!(down2.head, line3 + 20);
    assert_eq!(down2.goal_x, Some(160.0));

    // 6. Three cursors, Escape → the main cursor only; one caret is not Escape's to consume.
    let s = SelectionSet::normalized([2, 9, 5].map(SelectionRange::caret), 2);
    assert_eq!(s.main(), &SelectionRange::caret(5));
    let once = s.collapse().unwrap();
    assert_eq!(ends(&once), [(5, 5)]);
    assert_eq!(once.collapse(), None);
}

#[test]
fn normalisation_sorts_and_follows_the_main_range() {
    let s = SelectionSet::normalized([sel(9, 9), sel(1, 2), sel(5, 6)], 0);
    assert_eq!(ends(&s), [(1, 2), (5, 6), (9, 9)]);
    assert_eq!(s.main_index(), 2);
    assert_eq!(s.main(), &sel(9, 9));
}

#[test]
fn touching_selections_and_a_caret_at_a_selection_edge_merge() {
    // Both non-empty and touching.
    assert_eq!(
        ends(&SelectionSet::normalized([sel(1, 3), sel(3, 5)], 0)),
        [(1, 5)]
    );
    // A caret at either edge of a selection, or inside it.
    for at in [1, 2, 3] {
        let s = SelectionSet::normalized([sel(1, 3), SelectionRange::caret(at)], 1);
        assert_eq!(ends(&s), [(1, 3)], "caret at {at}");
        assert_eq!(s.main_index(), 0, "main follows the merged range");
    }
    // Disjoint ranges stay apart.
    assert_eq!(
        ends(&SelectionSet::normalized([sel(1, 3), sel(4, 6)], 0)),
        [(1, 3), (4, 6)]
    );
}

#[test]
fn a_merged_range_keeps_the_direction_of_the_range_holding_main() {
    // Main is the reversed one: the merged range is reversed.
    let s = SelectionSet::normalized([sel(8, 5), sel(4, 6)], 0);
    assert_eq!(ends(&s), [(8, 4)]);
    // Main is the forward one: the merged range is forward.
    let s = SelectionSet::normalized([sel(8, 5), sel(4, 6)], 1);
    assert_eq!(ends(&s), [(4, 8)]);
    // Main is a caret inside a reversed selection: a caret has no direction, so the
    // selection's stays.
    let s = SelectionSet::normalized([sel(8, 2), SelectionRange::caret(5)], 1);
    assert_eq!(ends(&s), [(8, 2)]);
}

#[test]
fn no_ranges_is_a_caret_at_zero_and_main_is_clamped() {
    assert_eq!(ends(&SelectionSet::normalized([], 0)), [(0, 0)]);
    let s = SelectionSet::normalized([sel(1, 2), sel(4, 5)], 7);
    assert_eq!(s.main(), &sel(4, 5));
}

#[test]
fn mapping_moves_carets_by_assoc_and_never_grows_a_selection() {
    // "abcdef": a selection [2,4); insertions at both of its edges stay outside it.
    let c = edit(6, &[(2..2, "X"), (4..4, "Y")]).desc();
    let s = SelectionSet::single(sel(2, 4)).map(&c);
    assert_eq!(ends(&s), [(3, 5)]);
    let s = SelectionSet::single(sel(4, 2)).map(&c);
    assert_eq!(ends(&s), [(5, 3)], "direction kept");

    // A caret keeps to its `assoc` side of an insertion at it.
    let c = edit(6, &[(2..2, "XY")]).desc();
    let after = SelectionSet::cursor(2).map(&c);
    assert_eq!(after.main().head, 4);
    let before = SelectionSet::single(SelectionRange {
        assoc: Bias::Before,
        ..SelectionRange::caret(2)
    })
    .map(&c);
    assert_eq!(before.main().head, 2);
}

#[test]
fn a_selection_whose_text_was_replaced_collapses_after_the_new_text() {
    // [2,4) inside a replacement of [1,5) by "ab" → a caret after "ab".
    let c = edit(6, &[(1..5, "ab")]).desc();
    let s = SelectionSet::single(sel(2, 4)).map(&c);
    assert_eq!(ends(&s), [(3, 3)]);
}

#[test]
fn mapping_merges_ranges_an_edit_brought_together() {
    let s = SelectionSet::normalized([2, 5].map(SelectionRange::caret), 1);
    let s = s.map(&edit(8, &[(2..5, "")]).desc());
    assert_eq!(ends(&s), [(2, 2)]);
    assert_eq!(s.main_index(), 0);
}

#[test]
fn an_edit_resets_the_goal_column_and_a_selection_only_change_keeps_it() {
    let r = SelectionRange {
        goal_x: Some(40.0),
        ..SelectionRange::caret(3)
    };
    let s = SelectionSet::single(r);
    assert_eq!(
        s.map(&ChangeSet::identity(9).desc()).main().goal_x,
        Some(40.0)
    );
    assert_eq!(s.map(&edit(9, &[(0..0, "x")]).desc()).main().goal_x, None);
    // A horizontal move resets it too.
    assert_eq!(r.moved_to(4, false).goal_x, None);
}

#[test]
fn vertical_moves_stop_at_the_document_edges_and_keep_the_goal() {
    let b = TextBuffer::from_str("abcdef\nxy\nabcdef");
    let r = SelectionRange::caret(4);
    let up = r.moved_vertically(&b, -1, false, x_of(&b), at_x(&b));
    assert_eq!((up.head, up.goal_x), (0, Some(32.0)));
    // Down onto the 2-column line clamps; the next line gets column 4 back.
    let short = up.moved_vertically(&b, 1, false, x_of(&b), at_x(&b));
    assert_eq!(short.head, b.line_to_byte(1) + 2);
    let again = short.moved_vertically(&b, 1, false, x_of(&b), at_x(&b));
    assert_eq!(
        again.head,
        b.line_to_byte(2) + 4,
        "the goal survives the edge"
    );
    let last = SelectionRange::caret(b.line_to_byte(2) + 4);
    let down = last.moved_vertically(&b, 1, false, x_of(&b), at_x(&b));
    assert_eq!(down.head, b.len_bytes());
    // A page down of 10 lines from line 0 ends at the end.
    let page = r.moved_vertically(&b, 10, false, x_of(&b), at_x(&b));
    assert_eq!(page.head, b.len_bytes());
}

#[test]
fn extending_keeps_the_anchor_and_a_plain_move_makes_a_caret() {
    let b = TextBuffer::from_str("abc\nabc");
    let r = sel(1, 2);
    let ext = r.moved_vertically(&b, 1, true, x_of(&b), at_x(&b));
    assert_eq!((ext.anchor, ext.head), (1, 6));
    let plain = r.moved_vertically(&b, 1, false, x_of(&b), at_x(&b));
    assert_eq!((plain.anchor, plain.head), (6, 6));
    assert_eq!(r.moved_to(3, true), sel(1, 3));
    assert_eq!(r.moved_to(3, false), SelectionRange::caret(3));
}

#[test]
fn a_vertical_move_never_lands_inside_a_character_or_a_crlf() {
    // Line 2 is "çx": the layout's answer (a byte inside `ç`) is snapped to a boundary.
    let b = TextBuffer::from_str("abc\r\nçx");
    let line2 = b.line_to_byte(1);
    let r = SelectionRange::caret(1).moved_vertically(&b, 1, false, x_of(&b), |_, _| line2 + 1);
    assert!(b.is_char_boundary(r.head));
    // An answer past the line's end clamps to the end of its text (before the break).
    let r = SelectionRange::caret(5).moved_vertically(&b, -1, false, x_of(&b), |_, _| 4);
    assert_eq!(r.head, 3);
}

#[test]
fn escape_collapses_a_selection_to_its_head() {
    let s = SelectionSet::single(sel(7, 2));
    assert_eq!(s.collapse(), Some(SelectionSet::cursor(2)));
}

#[test]
fn the_model_maps_its_selection_through_an_edit_without_one() {
    let mut m = EditorModel::new("abcdef", ModelConfig::default());
    let ann = |user_event| Annotations {
        user_event,
        add_to_history: true,
        time: Instant::now(),
        origin: Origin::User,
    };
    m.dispatch(Transaction {
        changes: ChangeSet::identity(6),
        selection: Some(SelectionSet::single(sel(2, 4))),
        ann: ann(UserEvent::Select),
    })
    .unwrap();
    m.dispatch(Transaction {
        changes: edit(6, &[(2..2, "X"), (4..4, "Y")]),
        selection: None,
        ann: ann(UserEvent::InputType),
    })
    .unwrap();
    assert_eq!(ends(m.selection()), [(3, 5)]);
}

fn arb_range() -> impl Strategy<Value = SelectionRange> {
    (0usize..40, 0usize..40, any::<bool>()).prop_map(|(a, h, before)| SelectionRange {
        assoc: if before { Bias::Before } else { Bias::After },
        ..SelectionRange::new(a, h)
    })
}

/// Sorted, merged (no overlap, no touching selections, no caret on a selection), main valid.
fn assert_normalized(s: &SelectionSet) {
    let r = s.ranges();
    assert!(!r.is_empty());
    assert!(s.main_index() < r.len());
    for w in r.windows(2) {
        let (a, b) = (w[0].range(), w[1].range());
        assert!(a.end < b.start, "{a:?} and {b:?} should have merged");
    }
}

proptest! {
    #[test]
    fn normalized_sets_are_sorted_merged_and_stable(
        ranges in proptest::collection::vec(arb_range(), 1..12),
        main in 0usize..12,
    ) {
        let s = SelectionSet::normalized(ranges.clone(), main);
        assert_normalized(&s);
        // Idempotent.
        let again = SelectionSet::normalized(s.ranges().iter().copied(), s.main_index());
        prop_assert_eq!(&again, &s);
        // Every input position is covered by some output range.
        for r in &ranges {
            let r = r.range();
            prop_assert!(s.ranges().iter().any(|o| o.range().start <= r.start && r.end <= o.range().end));
        }
        // The main range covers the input's main range.
        let m = ranges[main.min(ranges.len() - 1)].range();
        let om = s.main().range();
        prop_assert!(om.start <= m.start && m.end <= om.end);
    }

    #[test]
    fn mapped_sets_stay_normalized_and_inside_the_text(
        ranges in proptest::collection::vec(arb_range(), 1..8),
        main in 0usize..8,
        at in 0usize..40,
        del in 0usize..10,
        ins in "[a-c]{0,4}",
    ) {
        let s = SelectionSet::normalized(ranges, main);
        let len = 40;
        let at = at.min(len);
        let del = del.min(len - at);
        let c = edit(len, &[(at..at + del, ins.as_str())]).desc();
        let mapped = s.map(&c);
        assert_normalized(&mapped);
        let new_len = len - del + ins.len();
        prop_assert!(mapped.ranges().iter().all(|r| r.anchor <= new_len && r.head <= new_len));
    }
}
