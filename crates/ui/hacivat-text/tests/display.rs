//! Task 11: the display model's pure parts (UI-EDIT-07, UI-EDIT-08 uniform).

use std::sync::Arc;

use hacivat_text::display::{
    Decoration, Fragment, HeightMap, PlaceholderId, RangeSet, ScrollAnchor, StyleId, compose_line,
    control_name,
};
use hacivat_text::{ChangeSet, TextBuffer};

fn frag(buf: &TextBuffer) -> Vec<Fragment> {
    compose_line(buf, 0, 4, &[]).fragments.to_vec()
}

fn columns(text: &str, tab: u8) -> Vec<u32> {
    let b = TextBuffer::from_str(text);
    compose_line(&b, 0, tab, &[]).cell_widths(&b)
}

#[test]
fn rlo_is_a_visible_control_fragment() {
    let b = TextBuffer::from_str("a\u{202E}b");
    let f = frag(&b);
    assert_eq!(f.len(), 3);
    assert!(matches!(f[1], Fragment::Control { byte: 1, .. }));
}

#[test]
fn every_bidi_and_invisible_control_is_visible() {
    let cs = [
        '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}',
        '\u{2068}', '\u{2069}', '\u{200E}', '\u{200F}', '\u{061C}', '\u{FEFF}', '\u{85}', '\u{7F}',
        '\u{1}', '\u{1F}', '\u{0B}',
    ];
    for c in cs {
        let b = TextBuffer::from_str(&format!("a{c}b"));
        let f = frag(&b);
        assert!(
            matches!(f[1], Fragment::Control { .. }),
            "U+{:04X} must be a Control fragment",
            c as u32
        );
    }
}

#[test]
fn nul_is_a_visible_control_fragment() {
    let b = TextBuffer::from_str("a\0b");
    assert!(matches!(
        frag(&b)[1],
        Fragment::Control {
            byte: 1,
            glyph: '\u{2400}'
        }
    ));
}

#[test]
fn ordinary_text_and_zwj_stay_plain() {
    let b = TextBuffer::from_str("héllo \u{200D}x");
    assert_eq!(
        frag(&b),
        vec![Fragment::Text {
            bytes: 0..b.len_bytes()
        }]
    );
}

#[test]
fn tabs_expand_to_the_next_stop_with_wide_chars() {
    assert_eq!(columns("日\tx", 4), [2, 2, 1]);
    assert_eq!(columns("ab\tc", 4), [2, 2, 1]);
    assert_eq!(columns("abcd\tc", 4), [4, 4, 1]);
    assert_eq!(columns("e\u{301}\tx", 4), [1, 3, 1]); // a combining mark is zero-width
    assert_eq!(columns("\tx", 0), [1, 1]); // tab size 0 acts as 1
}

#[test]
fn compose_reads_one_line_without_its_break() {
    let b = TextBuffer::from_str("ab\r\ncd\n");
    let l = compose_line(&b, 1, 4, &[]);
    assert_eq!((l.first_line, l.last_line), (1, 1));
    assert_eq!(l.fragments.to_vec(), vec![Fragment::Text { bytes: 4..6 }]);
}

#[test]
fn replace_decorations_become_placeholders() {
    let b = TextBuffer::from_str("0123456789");
    let rs = RangeSet::from_sorted(vec![
        (
            2..5,
            Decoration::Replace {
                placeholder: PlaceholderId(7),
            },
        ),
        (6..8, Decoration::Mark { style: StyleId(1) }),
    ]);
    let f = compose_line(&b, 0, 4, &[&rs]).fragments.to_vec();
    assert_eq!(
        f,
        vec![
            Fragment::Text { bytes: 0..2 },
            Fragment::Placeholder {
                id: PlaceholderId(7)
            },
            Fragment::Text { bytes: 5..10 },
        ]
    );
}

#[test]
fn ui_edit_08_cases_1_2_4_5_without_wrap() {
    // 1: y of a line is line x height; 2: the line under a y; 4: past the end clamps; 5: an
    // empty document has one addressable row 0.
    let h = HeightMap::uniform(100, 18.0);
    assert_eq!(h.y_of(3), 54.0);
    assert_eq!(h.line_at_y(54.0), 3);
    assert_eq!(h.line_at_y(71.9), 3);
    assert_eq!(h.line_at_y(72.0), 4);
    assert_eq!(h.line_at_y(-5.0), 0);
    assert_eq!(h.line_at_y(1e12), 99);
    assert_eq!(h.y_of(500), 1800.0);
    assert_eq!(h.total_height(), 1800.0);
    let empty = HeightMap::uniform(0, 18.0);
    assert_eq!((empty.line_at_y(10.0), empty.total_height()), (0, 0.0));
}

#[test]
fn million_lines_have_exact_y() {
    assert_eq!(
        HeightMap::uniform(1_000_000, 18.0).y_of(999_999),
        17_999_982.0_f64
    );
}

fn numbered(n: usize) -> TextBuffer {
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!("line {i}\n"));
    }
    TextBuffer::from_str(&s)
}

fn line_text(b: &TextBuffer, l: u32) -> String {
    b.slice(b.line_range(l as usize, false)).into_owned()
}

fn insert_at(b: &mut TextBuffer, anchor: &mut ScrollAnchor, at: usize, text: &str) {
    let cs = ChangeSet::from_edits(b.len_bytes(), [(at..at, Arc::from(text))]).unwrap();
    let old = b.clone();
    cs.apply(b);
    anchor.rebase(&old, b, &cs.desc());
}

#[test]
fn anchor_survives_a_thousand_lines_inserted_above() {
    let mut b = numbered(50_000);
    let map = HeightMap::uniform(b.len_lines() as u32, 18.0);
    let mut a = ScrollAnchor::from_y(&map, 40_000.0 * 18.0 + 5.0);
    assert_eq!((a.line, a.offset_in_row), (40_000, 5.0));
    let before = line_text(&b, a.line);
    let at = b.line_to_byte(10);
    let block = "inserted\n".repeat(1_000);
    insert_at(&mut b, &mut a, at, &block);
    assert_eq!(a.line, 41_000);
    assert_eq!(line_text(&b, a.line), before);
    assert_eq!(a.offset_in_row, 5.0);
}

#[test]
fn anchor_follows_removal_above_and_ignores_edits_below_or_inside() {
    let mut b = numbered(100);
    let mut a = ScrollAnchor {
        line: 50,
        wrap_index: 0,
        offset_in_row: 0.0,
    };
    let before = line_text(&b, 50);
    // Delete lines 0..10.
    let end = b.line_to_byte(10);
    let cs = ChangeSet::from_edits(b.len_bytes(), [(0..end, Arc::from(""))]).unwrap();
    let old = b.clone();
    cs.apply(&mut b);
    a.rebase(&old, &b, &cs.desc());
    assert_eq!((a.line, line_text(&b, a.line)), (40, before.clone()));
    // Type inside the anchor line and insert below it.
    let inside = b.line_to_byte(40) + 2;
    insert_at(&mut b, &mut a, inside, "ZZ");
    assert_eq!(a.line, 40);
    let below = b.line_to_byte(80);
    insert_at(&mut b, &mut a, below, "x\ny\n");
    assert_eq!(a.line, 40);
    // Inserting lines exactly at the start of the anchor line keeps the content still.
    let at = b.line_to_byte(40);
    insert_at(&mut b, &mut a, at, "new\n");
    assert_eq!(a.line, 41);
    assert_eq!(line_text(&b, a.line), "liZZne 50");
}

#[test]
fn anchor_rebase_survives_a_deletion_spanning_it() {
    let mut b = numbered(20);
    let mut a = ScrollAnchor {
        line: 10,
        wrap_index: 0,
        offset_in_row: 0.0,
    };
    let (s, e) = (b.line_to_byte(5), b.line_to_byte(15));
    let cs = ChangeSet::from_edits(b.len_bytes(), [(s..e, Arc::from(""))]).unwrap();
    let old = b.clone();
    cs.apply(&mut b);
    a.rebase(&old, &b, &cs.desc());
    assert_eq!(a.line, 5);
    assert!((a.line as usize) < b.len_lines());
}

#[test]
fn absolute_y_is_f64_scrollbar_only() {
    let map = HeightMap::uniform(1_000_000, 18.0);
    let a = ScrollAnchor::from_y(&map, 17_999_982.0);
    assert_eq!(a.line, 999_999);
    assert_eq!(a.y(&map), 17_999_982.0);
}

fn rangeset_of(n: usize) -> RangeSet<Decoration> {
    RangeSet::from_sorted(
        (0..n)
            .map(|i| {
                (
                    i * 10..i * 10 + 8,
                    Decoration::Mark {
                        style: StyleId((i % 7) as u16),
                    },
                )
            })
            .collect(),
    )
}

#[test]
fn rangeset_query_is_logarithmic() {
    let rs = rangeset_of(100_000);
    let (hits, stats) = rs.query_with_stats(500_000..500_100);
    assert_eq!(hits.len(), 10);
    assert_eq!(stats.hits, 10);
    assert!(stats.probes <= 40, "probes {}", stats.probes);
    // A query past the end and before the start touch nothing.
    assert!(rs.query(2_000_000..2_000_010).next().is_none());
    assert!(rs.query_with_stats(0..0).1.probes <= 40);
}

#[test]
fn rangeset_handles_overlaps_and_empty_entries() {
    let rs = RangeSet::from_sorted(vec![
        (0..100, 'a'),
        (5..5, 'e'),
        (10..12, 'b'),
        (50..60, 'c'),
    ]);
    let hit = |r: std::ops::Range<usize>| rs.query(r).map(|(_, v)| *v).collect::<String>();
    assert_eq!(hit(11..13), "ab"); // the wide early range still reaches
    assert_eq!(hit(5..5), "ae");
    assert_eq!(hit(90..91), "a");
    assert_eq!(hit(100..110), "");
    assert_eq!(hit(55..58), "ac");
}

#[test]
fn rangeset_matches_a_full_scan() {
    let items: Vec<_> = (0..300usize)
        .map(|i| {
            let s = (i * 37) % 500;
            (s..s + (i * 13) % 40, i)
        })
        .collect::<Vec<_>>();
    let mut items = items;
    items.sort_by_key(|(r, _)| r.start);
    let rs = RangeSet::from_sorted(items.clone());
    for q in (0..560).step_by(7) {
        for w in [0usize, 1, 9, 80] {
            let r = q..q + w;
            let end = r.end.max(r.start + 1);
            let want: Vec<usize> = items
                .iter()
                .filter(|(x, _)| x.start < end && r.start < x.end.max(x.start + 1))
                .map(|(_, v)| *v)
                .collect();
            let got: Vec<usize> = rs.query(r.clone()).map(|(_, v)| *v).collect();
            assert_eq!(got, want, "{r:?}");
        }
    }
}

#[test]
fn controls_have_names() {
    assert_eq!(control_name('\u{202E}'), "RLO");
    assert_eq!(control_name('\u{85}'), "U+0085");
}
