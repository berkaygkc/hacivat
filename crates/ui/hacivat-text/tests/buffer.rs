//! UI-EDIT-02 (M3a Task 5): the rope buffer keeps line endings and the BOM exactly; its
//! line-ending statistics follow every edit; U+2028 and NEL are not line breaks.

use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_text::{
    AnchorSet, Annotations, Bias, ChangeSet, Committed, EditorModel, LineEnding, LineEndingStats,
    MapMode, ModelConfig, Origin, TextBuffer, Transaction, UserEvent,
};

fn ann() -> Annotations {
    Annotations {
        user_event: UserEvent::InputType,
        add_to_history: true,
        time: Instant::now(),
        origin: Origin::User,
    }
}

/// Replaces `r` of `m`'s text with `text`, the selection mapped.
fn edit(m: &mut EditorModel, r: Range<usize>, text: &str) -> Committed {
    let len = m.buffer().len_bytes();
    m.dispatch(Transaction {
        changes: ChangeSet::from_edits(len, [(r, Arc::from(text))]).unwrap(),
        selection: None,
        ann: ann(),
    })
    .unwrap()
}

fn stats(lf: u32, crlf: u32, cr: u32) -> LineEndingStats {
    LineEndingStats { lf, crlf, cr }
}

#[test]
fn ui_edit_02_cases_1_to_7() {
    // 1. Mixed endings are counted, not normalised; a tie goes to LF.
    let b = TextBuffer::from_str("SELECT 1;\r\nSELECT 2;\n");
    assert_eq!(b.len_lines(), 3);
    assert_eq!(b.line_to_byte(1), 11);
    assert_eq!(b.meta().line_endings, stats(1, 1, 0));
    assert_eq!(b.dominant_line_ending(), LineEnding::Lf);

    // 2. The BOM is not text; saving writes it back.
    let b = TextBuffer::from_str("\u{FEFF}SELECT 1");
    assert!(b.meta().bom);
    assert_eq!(b.len_bytes(), 8);
    assert_eq!(b.to_string(), "SELECT 1");
    assert_eq!(b.to_string_with_bom(), "\u{FEFF}SELECT 1");

    // 3. `\n` inserted after a lone CR makes a CRLF: still two lines; an anchor at the
    //    insertion point with bias After moves past it.
    let mut m = EditorModel::new("a\rb", ModelConfig::default());
    let mut anchors = AnchorSet::default();
    anchors.insert(2, Bias::After, MapMode::Simple, 'b');
    assert_eq!(m.buffer().len_lines(), 2);
    let c = edit(&mut m, 2..2, "\n");
    anchors.map(&c.desc);
    assert_eq!(m.buffer().to_string(), "a\r\nb");
    assert_eq!(m.buffer().len_lines(), 2);
    assert_eq!(anchors.iter().collect::<Vec<_>>(), [(3, &'b')]);
    assert_eq!(m.buffer().meta().line_endings, stats(0, 1, 0));

    // 4. UTF-16: an astral character is two code units.
    let b = TextBuffer::from_str("😀a");
    assert_eq!(b.len_utf16(), 3);
    assert_eq!(b.byte_to_utf16(4), 2);
    assert_eq!(b.utf16_to_byte(2), 4);

    // 5. is the perf test `byte_to_line_at_the_end_of_20_mb_under_5_us` below.

    // 6. U+2028 is not a line break.
    assert_eq!(TextBuffer::from_str("x\u{2028}y").len_lines(), 1);

    // 7. A newline typed into a CRLF-dominant document is a CRLF.
    let mut m = EditorModel::new("SELECT 1;\r\nSELECT 2;\r\n", ModelConfig::default());
    let nl = m.buffer().dominant_line_ending();
    assert_eq!(nl, LineEnding::CrLf);
    assert_eq!(nl.as_str(), "\r\n");
    edit(&mut m, 9..9, nl.as_str());
    assert_eq!(m.buffer().to_string(), "SELECT 1;\r\n\r\nSELECT 2;\r\n");
    assert_eq!(m.buffer().len_lines(), 4);
    assert_eq!(m.buffer().meta().line_endings, stats(0, 3, 0));
    assert_eq!(LineEnding::Lf.as_str(), "\n");
    assert_eq!(LineEnding::Cr.as_str(), "\r");
}

#[test]
fn crlf_cr_lf_mix_round_trips_byte_for_byte() {
    let t = "a\r\nb\rc\nd";
    let b = TextBuffer::from_str(t);
    assert_eq!(b.to_string(), t);
    assert_eq!(b.meta().line_endings, stats(1, 1, 1));
    assert_eq!(b.len_lines(), 4);
    // Lines end where their break ends; the break is never split.
    assert_eq!(b.line_range(0, true), 0..3);
    assert_eq!(b.line_range(1, true), 3..5);
    assert_eq!(b.line_range(1, false), 3..4);
}

#[test]
fn u2028_is_not_a_line_break() {
    assert_eq!(TextBuffer::from_str("x\u{2028}y\u{85}z").len_lines(), 1);
}

#[test]
fn bom_round_trips() {
    let b = TextBuffer::from_str("\u{FEFF}SELECT 1");
    assert!(b.meta().bom);
    assert_eq!(b.to_string(), "SELECT 1");
    assert_eq!(b.to_string_with_bom(), "\u{FEFF}SELECT 1");
    // Without a BOM nothing is added, and a later U+FEFF is text.
    let plain = TextBuffer::from_str("SELECT 1\u{FEFF}");
    assert!(!plain.meta().bom);
    assert_eq!(plain.to_string_with_bom(), "SELECT 1\u{FEFF}");
    // Edits keep the BOM flag.
    let mut m = EditorModel::new("\u{FEFF}x", ModelConfig::default());
    edit(&mut m, 1..1, "y");
    assert!(m.buffer().meta().bom);
    assert_eq!(m.buffer().to_string_with_bom(), "\u{FEFF}xy");
}

#[test]
fn cr_then_lf_insert_merges_into_crlf_without_splitting_anchors() {
    let mut m = EditorModel::new("ab", ModelConfig::default());
    let mut anchors = AnchorSet::default();
    anchors.insert(1, Bias::Before, MapMode::Simple, "end of a");
    anchors.insert(1, Bias::After, MapMode::Simple, "start of b");

    let c = edit(&mut m, 1..1, "\r");
    anchors.map(&c.desc);
    assert_eq!(m.buffer().len_lines(), 2);
    assert_eq!(m.buffer().meta().line_endings, stats(0, 0, 1));
    assert_eq!(
        anchors.iter().collect::<Vec<_>>(),
        [(1, &"end of a"), (2, &"start of b")]
    );

    let c = edit(&mut m, 2..2, "\n");
    anchors.map(&c.desc);
    assert_eq!(m.buffer().to_string(), "a\r\nb");
    // One line break, not two: the CR and the LF merged.
    assert_eq!(m.buffer().len_lines(), 2);
    assert_eq!(m.buffer().meta().line_endings, stats(0, 1, 0));
    // Each anchor moved once, by what was inserted before it.
    assert_eq!(
        anchors.iter().collect::<Vec<_>>(),
        [(1, &"end of a"), (3, &"start of b")]
    );
    // Line ranges and snapping keep the CRLF whole.
    assert_eq!(m.buffer().line_range(0, false), 0..1);
    assert_eq!(m.buffer().line_range(0, true), 0..3);
    assert_eq!(m.buffer().snap(2, Bias::Before), 1);
    assert_eq!(m.buffer().snap(2, Bias::After), 3);
}

#[test]
fn line_ending_stats_follow_edits_that_split_and_join_breaks() {
    let mut m = EditorModel::new("a\r\nb", ModelConfig::default());
    // An explicit byte edit between CR and LF splits the CRLF into CR + LF.
    edit(&mut m, 2..2, "x");
    assert_eq!(m.buffer().to_string(), "a\rx\nb");
    assert_eq!(m.buffer().len_lines(), 3);
    assert_eq!(m.buffer().meta().line_endings, stats(1, 0, 1));
    // Deleting it joins them again.
    edit(&mut m, 2..3, "");
    assert_eq!(m.buffer().meta().line_endings, stats(0, 1, 0));
    // Deleting the CR leaves a lone LF.
    edit(&mut m, 1..2, "");
    assert_eq!(m.buffer().to_string(), "a\nb");
    assert_eq!(m.buffer().meta().line_endings, stats(1, 0, 0));
    // Replacing the whole text (a restored console) recounts it: CRLF now dominates.
    let len = m.buffer().len_bytes();
    edit(&mut m, 0..len, "x\r\ny\r\nz\n");
    assert_eq!(m.buffer().meta().line_endings, stats(1, 2, 0));
    assert_eq!(m.buffer().dominant_line_ending(), LineEnding::CrLf);
    // A CR at the end of the text, then a LF typed after it.
    let len = m.buffer().len_bytes();
    edit(&mut m, 0..len, "q\r");
    assert_eq!(m.buffer().meta().line_endings, stats(0, 0, 1));
    edit(&mut m, 2..2, "\n");
    assert_eq!(m.buffer().meta().line_endings, stats(0, 1, 0));
    // Several edits in one change set, each at a break's edge.
    let mut m = EditorModel::new("1\r2\n3\r\n4", ModelConfig::default());
    let len = m.buffer().len_bytes();
    m.dispatch(Transaction {
        changes: ChangeSet::from_edits(
            len,
            [
                (2..2, Arc::from("\n")),   // after the lone CR: CRLF
                (3..4, Arc::from("\r")),   // the LF becomes a CR, before "3"
                (6..7, Arc::from("")),     // the LF of "3\r\n": a lone CR
                (8..8, Arc::from("\r\n")), // at the end
            ],
        )
        .unwrap(),
        selection: None,
        ann: ann(),
    })
    .unwrap();
    assert_eq!(m.buffer().to_string(), "1\r\n2\r3\r4\r\n");
    assert_eq!(m.buffer().meta().line_endings, stats(0, 2, 2));
    assert_eq!(
        TextBuffer::from_str(&m.buffer().to_string())
            .meta()
            .line_endings,
        m.buffer().meta().line_endings
    );
}

#[test]
fn char_boundaries_are_reported() {
    let b = TextBuffer::from_str("aç😀");
    let on: Vec<usize> = (0..=b.len_bytes())
        .filter(|&i| b.is_char_boundary(i))
        .collect();
    assert_eq!(on, [0, 1, 3, 7]);
    assert!(!b.is_char_boundary(99));
}

#[test]
#[ignore = "perf"]
fn byte_to_line_at_the_end_of_20_mb_under_5_us() {
    if std::env::var_os("HACIVAT_PERF").is_none_or(|v| v != "1") {
        return;
    }
    let line = "SELECT id, name FROM customers WHERE id = 1;\n";
    let text = line.repeat((20 << 20) / line.len());
    let b = TextBuffer::from_str(&text);
    let last = b.len_bytes() - 1;
    const N: u32 = 20_000;
    let start = Instant::now();
    let mut lines = 0;
    for _ in 0..N {
        lines = b.byte_to_line(std::hint::black_box(last));
    }
    let each = start.elapsed() / N;
    // The text ends with a break, so its last byte is on the second-to-last line.
    assert_eq!(lines, b.len_lines() - 2);
    assert!(
        each < Duration::from_micros(5),
        "byte_to_line took {each:?}"
    );
}
