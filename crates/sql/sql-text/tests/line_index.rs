//! `LineIndex` (Task 3): byte offset ↔ line and byte column, PostgreSQL's character error
//! cursors, and `TextEdit` alignment. CRLF fixtures are built in code (P15).

use sql_text::{LineCol, LineIndex, TextEdit, TextRange, TextSize};

fn ts(n: u32) -> TextSize {
    TextSize::from(n)
}

#[test]
fn line_index_round_trips_crlf_and_multibyte() {
    let t = "a\r\nçğ\nx";
    let ix = LineIndex::new(t);
    // Byte 7 is the `\n` after `çğ` (P18).
    assert_eq!(ix.line_col(ts(7)), LineCol { line: 1, col: 4 });
    assert_eq!(ix.line_col(ts(8)), LineCol { line: 2, col: 0 });
    assert_eq!(ix.char_to_byte(t, 3), Some(ts(3)));
}

#[test]
fn every_break_kind_ends_a_line() {
    // LF, CRLF and a lone CR each end one line; CRLF is one break, not two.
    let t = "a\nb\r\nc\rd";
    let ix = LineIndex::new(t);
    assert_eq!(ix.len_lines(), 4);
    let starts: Vec<_> = (0..4)
        .map(|line| ix.offset(LineCol { line, col: 0 }))
        .collect();
    assert_eq!(starts, [Some(ts(0)), Some(ts(2)), Some(ts(5)), Some(ts(7))]);
    // `\r\r\n` is a lone CR then a CRLF: two breaks.
    assert_eq!(LineIndex::new("x\r\r\ny").len_lines(), 3);
}

#[test]
fn empty_and_trailing_break_texts_count_their_last_line() {
    assert_eq!(LineIndex::new("").len_lines(), 1);
    assert_eq!(
        LineIndex::new("").line_col(ts(0)),
        LineCol { line: 0, col: 0 }
    );
    let ix = LineIndex::new("a\n");
    assert_eq!(ix.len_lines(), 2);
    assert_eq!(ix.line_col(ts(2)), LineCol { line: 1, col: 0 });
    assert_eq!(ix.offset(LineCol { line: 1, col: 0 }), Some(ts(2)));
}

#[test]
fn offset_inverts_line_col_at_every_offset_outside_a_break() {
    let crlf = "SELECT 1;\r\n-- çğışöü\r\n\r\nSELECT 'İ';\rx\n".to_string();
    for t in [crlf.as_str(), "a\nb", "\n\n", "çok\r\nsatır\r\n", "x"] {
        let ix = LineIndex::new(t);
        for (at, _) in t.char_indices().chain(std::iter::once((t.len(), ' '))) {
            let inside_crlf = at > 0 && t.as_bytes()[at - 1] == b'\r' && t[at..].starts_with('\n');
            let lc = ix.line_col(ts(at as u32));
            if inside_crlf {
                // Between `\r` and `\n`: the position names the line the break ends, and
                // offset refuses a column inside the break.
                assert_eq!(ix.offset(lc), None, "{t:?} at {at}");
            } else {
                assert_eq!(ix.offset(lc), Some(ts(at as u32)), "{t:?} at {at}");
            }
        }
    }
}

#[test]
fn offset_refuses_missing_lines_and_columns_past_the_line() {
    let ix = LineIndex::new("ab\r\ncd");
    assert_eq!(ix.offset(LineCol { line: 0, col: 2 }), Some(ts(2)));
    assert_eq!(ix.offset(LineCol { line: 0, col: 3 }), None);
    assert_eq!(ix.offset(LineCol { line: 0, col: 4 }), None);
    assert_eq!(ix.offset(LineCol { line: 1, col: 2 }), Some(ts(6)));
    assert_eq!(ix.offset(LineCol { line: 1, col: 3 }), None);
    assert_eq!(ix.offset(LineCol { line: 2, col: 0 }), None);
}

#[test]
fn offsets_past_the_end_clamp_to_the_end() {
    let ix = LineIndex::new("ab\ncd");
    assert_eq!(ix.line_col(ts(99)), LineCol { line: 1, col: 2 });
}

#[test]
fn char_to_byte_counts_characters_not_bytes() {
    // PostgreSQL reports error cursors as 1-based characters; the caller subtracts 1.
    let t = "SELECT 'ğ', x FROM şema";
    let ix = LineIndex::new(t);
    assert_eq!(ix.char_to_byte(t, 0), Some(ts(0)));
    // Character 12 is `x`: the `ğ` before it is two bytes.
    assert_eq!(ix.char_to_byte(t, 12), Some(ts(13)));
    assert_eq!(&t[13..14], "x");
    // One past the last character is the end of the text (an error at end of input).
    let chars = t.chars().count();
    assert_eq!(ix.char_to_byte(t, chars), Some(ts(t.len() as u32)));
    assert_eq!(ix.char_to_byte(t, chars + 1), None);
}

#[test]
fn text_edits_apply_only_on_char_boundaries() {
    let mut t = String::from("SELECT 'çğ'");
    // Byte 9 is inside `ç`: misaligned edits change nothing.
    let inside = TextEdit {
        delete: TextRange::new(ts(9), ts(10)),
        insert: "x".into(),
    };
    assert!(!inside.fits(&t));
    assert!(inside.apply(&mut t).is_err());
    assert_eq!(t, "SELECT 'çğ'");
    let past_end = TextEdit {
        delete: TextRange::new(ts(12), ts(14)),
        insert: String::new(),
    };
    assert!(past_end.apply(&mut t).is_err());
    let replace = TextEdit {
        delete: TextRange::new(ts(8), ts(12)),
        insert: "İı".into(),
    };
    assert!(replace.fits(&t));
    assert!(replace.apply(&mut t).is_ok());
    assert_eq!(t, "SELECT 'İı'");
}
