//! M3c Task 32: folding (UI-EDIT-10) over the chunked height map (UI-EDIT-08).
//!
//! The folded set maps through every change (an edit inside a fold or at its edges keeps it;
//! an edit that deletes its header unfolds it); its hidden lines feed `HeightMap::chunked`,
//! whose `y_of` and `line_at_y` are inverse over the visible rows; a folded header composes
//! into one visual line ending in a placeholder.

#![allow(
    clippy::single_range_in_vec_init,
    clippy::cloned_ref_to_slice_refs,
    reason = "ranges as data"
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_text::display::{
    Fragment, HeightMap, PlaceholderId, ScrollAnchor, compose_folded, compose_line,
};
use hacivat_text::fold::MIN_FOLD_LINES;
use hacivat_text::{Bias, ChangeSet, FoldSet, SelectionRange, SelectionSet, TextBuffer};

const LH: f64 = 18.0;

/// `text` with `r` replaced by `ins`: the change's description and the new buffer.
fn edit(
    buf: &TextBuffer,
    r: std::ops::Range<usize>,
    ins: &str,
) -> (hacivat_text::ChangeDesc, TextBuffer) {
    let cs = ChangeSet::from_edits(buf.len_bytes(), [(r, Arc::from(ins))]).unwrap();
    let mut out = buf.clone();
    cs.apply(&mut out);
    (cs.desc(), out)
}

fn at(text: &str, needle: &str) -> usize {
    text.find(needle)
        .unwrap_or_else(|| panic!("{needle:?} in {text:?}"))
}

const THREE: &str = "SELECT a,\n  b\nFROM t;\nSELECT 2;\n";

/// UI-EDIT-10 test cases 1–5 (case 5 at the M3c minimum of three lines, ruling M3c-43; case 3
/// is the core primitive the editor calls when a caret lands inside a fold).
#[test]
fn ui_edit_10_cases_1_to_5() {
    // 1. A 3-line statement folds into one visual line: its header and the placeholder; the
    //    two lines under it are hidden.
    let buf = TextBuffer::from_str(THREE);
    let stmt = 0..at(THREE, "t;") + 2;
    let mut folds = FoldSet::default();
    assert!(folds.fold(stmt.clone(), &buf));
    assert_eq!(folds.folded(), &[stmt.clone()]);
    assert_eq!(folds.hidden_lines(&buf), vec![1..3]);
    let vl = compose_folded(&buf, 0, 2, 4, &[], PlaceholderId(0));
    assert_eq!((vl.first_line, vl.last_line), (0, 2));
    assert_eq!(
        vl.fragments.to_vec(),
        vec![
            Fragment::Text { bytes: 0..9 },
            Fragment::Placeholder {
                id: PlaceholderId(0)
            }
        ]
    );
    assert_eq!(compose_line(&buf, 0, 4, &[]).fragments.len(), 1);

    // 2. Fold a subquery, then type in the outer statement above it: the fold stays.
    let sub = "SELECT *\nFROM (\n  SELECT 1\n  FROM u\n) s;\n";
    let buf = TextBuffer::from_str(sub);
    let paren = at(sub, "(")..at(sub, ") s");
    let mut folds = FoldSet::default();
    assert!(folds.fold(paren.clone(), &buf));
    // The closing line stays visible: only the lines between the parentheses hide.
    assert_eq!(folds.hidden_lines(&buf), vec![2..4]);
    let (d, buf2) = edit(&buf, 7..7, ", x");
    folds.map(&d);
    folds.prune(&buf2);
    assert_eq!(folds.folded(), &[paren.start + 3..paren.end + 3]);
    assert_eq!(folds.hidden_lines(&buf2), vec![2..4]);

    // 3. Landing inside a folded region opens that fold (and only the folds hiding it).
    let buf = TextBuffer::from_str(THREE);
    let mut folds = FoldSet::default();
    assert!(folds.fold(stmt.clone(), &buf));
    assert!(
        !folds.reveal(at(THREE, "SELECT 2"), &buf),
        "outside: nothing opens"
    );
    assert!(!folds.reveal(3, &buf), "the header line is visible");
    assert!(folds.reveal(at(THREE, "FROM"), &buf));
    assert!(folds.folded().is_empty());

    // A selection holding the fold whole (select all) keeps it; one ending inside opens it.
    let mut folds = FoldSet::default();
    assert!(folds.fold(stmt.clone(), &buf));
    let all = SelectionSet::single(SelectionRange {
        anchor: 0,
        head: buf.len_bytes(),
        goal_x: None,
        assoc: Bias::After,
    });
    assert!(!folds.reveal_selection(&all, &buf));
    assert!(!folds.reveal_selection(&SelectionSet::cursor(3), &buf));
    assert!(folds.reveal_selection(&SelectionSet::cursor(at(THREE, "b")), &buf));
    assert!(folds.folded().is_empty());

    // 4. Fold, then delete the opening `(`: the fold's start is gone, the fold with it.
    let buf = TextBuffer::from_str(sub);
    let mut folds = FoldSet::default();
    assert!(folds.fold(paren.clone(), &buf));
    let (d, buf2) = edit(&buf, paren.start..paren.start + 1, "");
    folds.map(&d);
    folds.prune(&buf2);
    assert!(folds.folded().is_empty());

    // 5. A range of two lines never folds; three lines do.
    let two = "-- a\n-- b\nSELECT 1;\n";
    let buf = TextBuffer::from_str(two);
    let mut folds = FoldSet::default();
    assert_eq!(MIN_FOLD_LINES, 3);
    assert!(!folds.fold(0..at(two, "\nSELECT"), &buf));
    let three = "-- a\n-- b\n-- c\nSELECT 1;\n";
    let buf = TextBuffer::from_str(three);
    assert!(folds.fold(0..at(three, "\nSELECT"), &buf));
    assert_eq!(folds.hidden_lines(&buf), vec![1..3]);
}

#[test]
fn nested_folds_keep_their_state_and_partial_overlaps_are_refused() {
    let text = "SELECT *\nFROM (\n  SELECT 1\n  FROM u\n) s\nWHERE x;\nSELECT 2;\n";
    let buf = TextBuffer::from_str(text);
    let stmt = 0..at(text, "x;") + 2;
    let paren = at(text, "(")..at(text, ") s");
    let mut folds = FoldSet::default();
    assert!(folds.fold(paren.clone(), &buf));
    assert!(folds.fold(stmt.clone(), &buf));
    assert!(!folds.fold(stmt.clone(), &buf), "already folded");
    assert_eq!(folds.folded(), &[stmt.clone(), paren.clone()]);
    // The outer hides everything under its header.
    assert_eq!(folds.hidden_lines(&buf), vec![1..6]);
    // A range that crosses a folded one's edge is refused.
    assert!(!folds.fold(at(text, "  SELECT 1")..at(text, "SELECT 2") + 8, &buf));
    // Unfolding the outer one restores the inner one's state.
    assert!(folds.unfold_at(0));
    assert_eq!(folds.folded(), &[paren.clone()]);
    assert_eq!(folds.hidden_lines(&buf), vec![2..4]);
    // The innermost fold holding the position unfolds first.
    assert!(folds.fold(stmt.clone(), &buf));
    assert!(folds.unfold_at(at(text, "SELECT 1")));
    assert_eq!(folds.folded(), &[stmt.clone()]);
    assert!(folds.unfold_all());
    assert!(!folds.unfold_all());
    // The outermost fold on a header line is the line's.
    assert!(folds.fold(stmt.clone(), &buf));
    assert_eq!(folds.fold_on_line(0, &buf), Some(stmt.clone()));
    assert_eq!(folds.fold_on_line(1, &buf), None);
}

#[test]
fn chunked_height_map_matches_uniform_without_folds() {
    for lines in [0u32, 1, 2, 1_023, 1_024, 1_025, 50_000] {
        let u = HeightMap::uniform(lines, LH);
        let c = HeightMap::chunked(lines, LH, &[]);
        assert_eq!(c.lines(), lines);
        assert_eq!(c.total_height(), u.total_height(), "{lines}");
        assert_eq!(c.visible_rows(), lines);
        for l in (0..lines.saturating_add(2)).step_by(97) {
            assert_eq!(c.y_of(l), u.y_of(l), "y_of({l}) of {lines}");
        }
        let mut y = -10.0;
        while y < u.total_height() + 40.0 {
            assert_eq!(c.line_at_y(y), u.line_at_y(y), "line_at_y({y}) of {lines}");
            y += 7.25 * 97.0;
        }
        assert_eq!(c.line_at_y(f64::NAN), 0);
    }
}

#[test]
fn y_of_and_line_at_y_are_inverse_with_folds() {
    let hidden = [2..5, 5..6, 9..10, 1_020..1_030, 4_000..4_999];
    // Touching runs merge; the map is built from sorted runs.
    let map = HeightMap::chunked(5_000, LH, &hidden);
    assert_eq!(map.hidden(), &[2..6, 9..10, 1_020..1_030, 4_000..4_999]);
    let n_hidden = 4 + 1 + 10 + 999;
    assert_eq!(map.visible_rows(), 5_000 - n_hidden);
    assert_eq!(map.total_height(), f64::from(5_000 - n_hidden) * LH);
    for line in 0..5_000 {
        let y = map.y_of(line);
        if map.is_hidden(line) {
            // A hidden line takes no height: it sits where the next visible line starts.
            assert_eq!(y, map.y_of(map.next_visible(line)), "{line}");
            let header = map.visible_line(line);
            assert!(header < line && !map.is_hidden(header), "{line}");
            assert_eq!(map.y_of(header) + LH, y, "{line}");
            continue;
        }
        assert_eq!(map.visible_line(line), line);
        assert_eq!(map.next_visible(line), line);
        assert_eq!(map.line_at_y(y), line, "top of {line}");
        assert_eq!(map.line_at_y(y + LH - 0.5), line, "bottom of {line}");
        assert_eq!(map.offset_in_row(y + 3.0), 3.0);
    }
    assert_eq!(map.visible_line(4_998), 3_999);
    assert_eq!(map.next_visible(4_998), 4_999);
    // Past the end: the last visible line.
    assert_eq!(map.line_at_y(1e12), 4_999);
    // A fold at the very end: the next visible line is the end.
    let tail = HeightMap::chunked(10, LH, &[7..10]);
    assert_eq!(tail.line_at_y(1e9), 6);
    assert_eq!(tail.next_visible(8), 10);
    assert_eq!(tail.y_of(10), 7.0 * LH);
}

#[test]
fn scroll_anchor_rebases_onto_the_header_of_a_new_fold() {
    let map = HeightMap::chunked(100, LH, &[10..20]);
    let mut a = ScrollAnchor {
        line: 15,
        wrap_index: 0,
        offset_in_row: 4.0,
    };
    a.snap_visible(&map);
    assert_eq!((a.line, a.offset_in_row), (9, 4.0));
    let mut b = ScrollAnchor {
        line: 25,
        wrap_index: 0,
        offset_in_row: 2.0,
    };
    b.snap_visible(&map);
    assert_eq!(b.line, 25, "a visible line stays");
    // The absolute y follows the folds.
    assert_eq!(b.y(&map), (25.0 - 10.0) * LH + 2.0);
}

#[test]
fn edit_inside_a_fold_keeps_it_edit_on_header_unfolds() {
    let text = "SELECT a,\n  b,\n  c\nFROM t;\nSELECT 2;\n";
    let buf = TextBuffer::from_str(text);
    let stmt = 0..at(text, "t;") + 2;
    let fold = |buf: &TextBuffer| {
        let mut f = FoldSet::default();
        assert!(f.fold(stmt.clone(), buf));
        f
    };
    // Inside: a programmatic edit of a hidden line keeps the fold.
    let mut f = fold(&buf);
    let r = at(text, "b,");
    let (d, b2) = edit(&buf, r..r + 1, "bb");
    f.map(&d);
    f.prune(&b2);
    assert_eq!(f.folded(), &[stmt.start..stmt.end + 1]);
    // At its end: text typed there stays outside it (fix round 1, I2).
    let mut f = fold(&buf);
    let (d, b2) = edit(&buf, stmt.end..stmt.end, " -- x");
    f.map(&d);
    f.prune(&b2);
    assert_eq!(f.folded(), &[stmt.clone()]);
    // Typing at its start: the text goes above the fold, whose header stays.
    let mut f = fold(&buf);
    let (d, b2) = edit(&buf, 0..0, "-- c\n");
    f.map(&d);
    f.prune(&b2);
    assert_eq!(f.folded(), &[stmt.start + 5..stmt.end + 5]);
    assert_eq!(f.hidden_lines(&b2), vec![2..5]);
    // Typing on the header line, before its break: kept.
    let mut f = fold(&buf);
    let (d, b2) = edit(&buf, 8..8, ", z");
    f.map(&d);
    f.prune(&b2);
    assert_eq!(f.folded().len(), 1);
    assert_eq!(f.hidden_lines(&b2), vec![1..4]);
    // Joining the header with the next line (its break deleted): unfolded.
    let mut f = fold(&buf);
    let (d, b2) = edit(&buf, 9..10, "");
    f.map(&d);
    f.prune(&b2);
    assert!(f.folded().is_empty());
    // Deleting the header line whole (the line command): unfolded.
    let mut f = fold(&buf);
    let (d, b2) = edit(&buf, 0..10, "");
    f.map(&d);
    f.prune(&b2);
    assert!(f.folded().is_empty());
    // Deleting the header's first character: unfolded.
    let mut f = fold(&buf);
    let (d, b2) = edit(&buf, 0..1, "");
    f.map(&d);
    f.prune(&b2);
    assert!(f.folded().is_empty());
    // Deleting the whole text: gone.
    let mut f = fold(&buf);
    let (d, b2) = edit(&buf, 0..buf.len_bytes(), "");
    f.map(&d);
    f.prune(&b2);
    assert!(f.folded().is_empty());
    // A fold that an edit left on fewer than three lines is dropped.
    let mut f = fold(&buf);
    let (d, b2) = edit(&buf, at(text, "  b,")..at(text, "FROM"), "");
    f.map(&d);
    f.prune(&b2);
    assert!(f.folded().is_empty(), "{:?}", f.folded());
    // Every change bumps the generation (the editor caches the hidden lines by it).
    let mut f = FoldSet::default();
    let g0 = f.generation();
    assert!(f.fold(stmt.clone(), &buf));
    assert_ne!(f.generation(), g0);
}

/// Fix round 1 (I2): Enter or a key before a folded block's visible closing line never joins
/// the fold: the fold and the lines it hides stay as they were.
#[test]
fn typing_at_a_blocks_visible_closing_line_keeps_the_fold() {
    for (text, close) in [
        ("SELECT *\nFROM (\n  SELECT 1\n  FROM u\n) s;\n", ") s"),
        ("BEGIN\n  SELECT 1;\n  SELECT 2;\nEND;\n", "END;"),
    ] {
        let buf = TextBuffer::from_str(text);
        let open = if text.starts_with("BEGIN") {
            0
        } else {
            at(text, "(")
        };
        let r = open..at(text, close);
        for typed in ["\n", "x", "\n\n"] {
            let mut f = FoldSet::default();
            assert!(f.fold(r.clone(), &buf));
            let hidden = f.hidden_lines(&buf);
            let (d, b2) = edit(&buf, r.end..r.end, typed);
            f.map(&d);
            f.prune(&b2);
            assert_eq!(f.folded(), &[r.clone()], "{typed:?} before {close:?}");
            assert_eq!(f.hidden_lines(&b2), hidden, "{typed:?} before {close:?}");
            // A caret after what was typed is on a visible line: nothing opens.
            let caret = SelectionSet::cursor(r.end + typed.len());
            assert!(
                !f.reveal_selection(&caret, &b2),
                "{typed:?} before {close:?}"
            );
        }
    }
}

/// Fix round 1 (M3): a range off a char boundary folds nothing and never panics.
#[test]
fn a_range_off_a_char_boundary_is_refused() {
    let text = "SELECT 'ğ',\n  b\nFROM t;\n";
    let buf = TextBuffer::from_str(text);
    let g = at(text, "ğ");
    let mut f = FoldSet::default();
    assert!(!f.fold(g + 1..text.len() - 1, &buf));
    assert!(!f.fold(0..g + 1, &buf));
    assert!(!hacivat_text::fold::can_fold(&buf, &(g + 1..text.len())));
    assert_eq!(
        hacivat_text::fold::fold_lines(&buf, &(g + 1..text.len())).0,
        0
    );
    assert!(f.fold(0..text.len() - 1, &buf));
}

/// 1,000 ten-line statements, each with two nested subqueries: 3,000 folds.
fn three_thousand_folds() -> (TextBuffer, FoldSet, Vec<usize>) {
    let stmt = "SELECT a,\n  (SELECT x\n   FROM (\n     SELECT 1\n     FROM u\n   ) q\n  ) b\nFROM t\nWHERE c;\n\n";
    let text = stmt.repeat(1_000);
    let buf = TextBuffer::from_str(&text);
    let mut f = FoldSet::default();
    let mut headers = Vec::new();
    for i in 0..1_000 {
        let base = i * stmt.len();
        let s = |n: &str| base + stmt.find(n).unwrap();
        let ranges = [
            base..s("c;") + 2,
            s("(SELECT")..s(") b"),
            s("(\n     SELECT")..s(") q"),
        ];
        for r in ranges {
            assert!(f.fold(r, &buf));
        }
        headers.push(base + 3);
    }
    assert_eq!(f.folded().len(), 3_000);
    (buf, f, headers)
}

/// Fix round 1 (I3): a keystroke costs well under a millisecond with 3,000 folds, and a
/// selection of 1,000 carets reveals in a few.
#[test]
#[ignore = "perf"]
fn fold_upkeep_with_3000_folds_stays_cheap() {
    if std::env::var_os("HACIVAT_PERF").is_none_or(|v| v != "1") {
        return;
    }
    let (mut buf, mut f, headers) = three_thousand_folds();
    let mut worst = Duration::ZERO;
    for k in 0..200 {
        // From the last statement back, so no insertion moves a header still to come.
        let at = headers[headers.len() - 1 - k * 5] + 1;
        let cs = ChangeSet::from_edits(buf.len_bytes(), [(at..at, Arc::from("x"))]).unwrap();
        cs.apply(&mut buf);
        let caret = SelectionSet::cursor(at + 1);
        let start = Instant::now();
        f.map(&cs.desc());
        f.prune(&buf);
        assert!(!f.reveal_selection(&caret, &buf));
        worst = worst.max(start.elapsed());
    }
    assert_eq!(f.folded().len(), 3_000);
    // A caret on every statement's header line (the statements are every third fold).
    let ranges: Vec<SelectionRange> = f
        .folded()
        .iter()
        .step_by(3)
        .map(|r| SelectionRange::caret(r.start + 3))
        .collect();
    assert_eq!(ranges.len(), 1_000);
    let carets = SelectionSet::normalized(ranges, 0);
    let start = Instant::now();
    assert!(!f.reveal_selection(&carets, &buf));
    let reveal = start.elapsed();
    // One keystroke at the 1,000 carets.
    let edits: Vec<_> = carets
        .ranges()
        .iter()
        .map(|r| (r.head..r.head, Arc::from("x")))
        .collect();
    let cs = ChangeSet::from_edits(buf.len_bytes(), edits).unwrap();
    cs.apply(&mut buf);
    let start = Instant::now();
    f.map(&cs.desc());
    f.prune(&buf);
    let moved = carets.map(&cs.desc());
    assert!(!f.reveal_selection(&moved, &buf));
    let multi = start.elapsed();
    assert_eq!(f.folded().len(), 3_000);
    eprintln!(
        "3,000 folds: worst 1-caret keystroke {worst:?} (map + prune + reveal); 1,000-caret reveal {reveal:?}; 1,000-caret keystroke {multi:?}"
    );
    assert!(
        multi < Duration::from_millis(10),
        "1,000-caret keystroke took {multi:?}"
    );
    assert!(worst < Duration::from_millis(1), "keystroke took {worst:?}");
    assert!(reveal < Duration::from_millis(5), "reveal took {reveal:?}");
}

#[test]
#[ignore = "perf"]
fn line_at_y_on_50k_lines_with_1000_folds_under_5us() {
    if std::env::var_os("HACIVAT_PERF").is_none_or(|v| v != "1") {
        return;
    }
    // 1,000 folds of 5 lines each, one every 50 lines.
    let hidden: Vec<std::ops::Range<u32>> = (0..1_000u32).map(|i| i * 50 + 1..i * 50 + 6).collect();
    let map = HeightMap::chunked(50_000, LH, &hidden);
    let total = map.total_height();
    const N: u32 = 200_000;
    let start = Instant::now();
    let mut acc = 0u64;
    for i in 0..N {
        let y = std::hint::black_box(f64::from(i) * 7.31 % total);
        acc += u64::from(map.line_at_y(y));
        acc += map.y_of(std::hint::black_box(i % 50_000)) as u64;
    }
    let each = start.elapsed() / N;
    assert!(acc > 0);
    eprintln!("line_at_y + y_of on 50k lines with 1,000 folds: {each:?} per pair");
    assert!(
        each < Duration::from_micros(5),
        "line_at_y + y_of took {each:?}"
    );
}
