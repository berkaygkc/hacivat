//! UI-EDIT-06 (M3a Task 10): characters are extended grapheme clusters (CRLF one unit); words
//! come from the language service's tokens, else from character classes (`Ident` =
//! XID_Continue plus `$ # @`, `Space`, `Punct`, `LineBreak`); sub-words split at `_` and ASCII
//! case transitions only; smart Home; every move is bounded on huge lines.

use std::ops::Range;

use hacivat_text::units::{
    self, CharClass, SCAN_LIMIT, char_class, grapheme_column, offset_at_column, snap_to_grapheme,
};
use hacivat_text::{Bias, Dir, TextBuffer, TokenOracle, Unit, move_offset};
use proptest::prelude::*;
use smallvec::SmallVec;
use unicode_segmentation::UnicodeSegmentation as _;

/// No analysis: the class fallback applies.
struct NoTokens;

impl TokenOracle for NoTokens {
    fn words_on_line(&self, _line: Range<usize>) -> Option<SmallVec<[Range<usize>; 16]>> {
        None
    }
}

/// A fake language service over a fixed text: identifier runs, `"…"` quoted identifiers,
/// `::`, and any other non-space character alone (test-only; the real one is the SQL lexer).
struct FakeSql(Vec<Range<usize>>);

impl FakeSql {
    fn new(text: &str) -> Self {
        let mut out = Vec::new();
        let mut i = 0;
        let word = |c: char| c.is_alphanumeric() || c == '_';
        while let Some(c) = text[i..].chars().next() {
            let start = i;
            i += c.len_utf8();
            if c.is_whitespace() {
                continue;
            }
            if c == '"' {
                while let Some(d) = text[i..].chars().next() {
                    i += d.len_utf8();
                    if d == '"' {
                        break;
                    }
                }
            } else if word(c) {
                while let Some(d) = text[i..].chars().next().filter(|&d| word(d)) {
                    i += d.len_utf8();
                }
            } else if c == ':' && text[i..].starts_with(':') {
                i += 1;
            }
            out.push(start..i);
        }
        FakeSql(out)
    }
}

impl TokenOracle for FakeSql {
    fn words_on_line(&self, line: Range<usize>) -> Option<SmallVec<[Range<usize>; 16]>> {
        Some(
            self.0
                .iter()
                .filter(|t| t.start < line.end && t.end > line.start)
                .cloned()
                .collect(),
        )
    }
}

/// Tokens given by hand (they may span lines).
struct Given(Vec<Range<usize>>);

impl TokenOracle for Given {
    fn words_on_line(&self, line: Range<usize>) -> Option<SmallVec<[Range<usize>; 16]>> {
        Some(
            self.0
                .iter()
                .filter(|t| t.start < line.end && t.end > line.start)
                .cloned()
                .collect(),
        )
    }
}

fn buf(text: &str) -> TextBuffer {
    TextBuffer::from_str(text)
}

/// Every stop of repeated moves from `at` until the move no longer moves.
fn stops(b: &TextBuffer, mut at: usize, dir: Dir, unit: Unit, tok: &dyn TokenOracle) -> Vec<usize> {
    let mut out = Vec::new();
    loop {
        let next = move_offset(b, at, dir, unit, tok);
        if next == at {
            return out;
        }
        out.push(next);
        at = next;
    }
}

/// The double-click word by character classes.
fn word_at(text: &str, at: usize) -> Range<usize> {
    units::word_at(&buf(text), at, &NoTokens)
}

/// The double-click word by the fake language service's tokens.
fn token_word(text: &str, at: usize) -> Range<usize> {
    units::word_at(&buf(text), at, &FakeSql::new(text))
}

#[test]
fn ui_edit_06_cases_1_to_6() {
    // 1. `SELECT a.b::int`, Ctrl+Right from 0: 6, 8, 9, 10, 12, 15 — by tokens, and the same
    //    by character classes; and back.
    let text = "SELECT a.b::int";
    let (b, toks) = (buf(text), FakeSql::new(text));
    assert_eq!(
        stops(&b, 0, Dir::Forward, Unit::Word, &toks),
        [6, 8, 9, 10, 12, 15]
    );
    assert_eq!(
        stops(&b, 0, Dir::Forward, Unit::Word, &NoTokens),
        [6, 8, 9, 10, 12, 15]
    );
    assert_eq!(
        stops(&b, 15, Dir::Back, Unit::Word, &toks),
        [12, 10, 9, 8, 7, 0]
    );

    // 2. A double-click inside `"Order Lines"` selects it with its quotes.
    assert_eq!(token_word(r#"SELECT "Order Lines" FROM t"#, 10), 7..20);

    // 3. `👩‍💻x`: Right from 0 lands after the ZWJ sequence (byte 11).
    assert_eq!(
        move_offset(&buf("👩‍💻x"), 0, Dir::Forward, Unit::Grapheme, &NoTokens),
        11
    );

    // 4. `I` + U+0307: Backspace's unit at the end is both code points.
    assert_eq!(
        move_offset(&buf("I\u{307}"), 3, Dir::Back, Unit::Grapheme, &NoTokens),
        0
    );

    // 5. Home on `    SELECT` at column 8 → 4; Home again → 0; again → 4.
    let b = buf("    SELECT");
    assert_eq!(move_offset(&b, 8, Dir::Back, Unit::Line, &NoTokens), 4);
    assert_eq!(move_offset(&b, 4, Dir::Back, Unit::Line, &NoTokens), 0);
    assert_eq!(move_offset(&b, 0, Dir::Back, Unit::Line, &NoTokens), 4);

    // 6. Sub-words of `customer_order_id`: customer|_, order|_, id|; and back.
    let b = buf("customer_order_id");
    assert_eq!(
        stops(&b, 0, Dir::Forward, Unit::SubWord, &NoTokens),
        [8, 14, 17]
    );
    assert_eq!(
        stops(&b, 17, Dir::Back, Unit::SubWord, &NoTokens),
        [15, 9, 0]
    );
    // A whole word does not stop at `_`.
    assert_eq!(stops(&b, 0, Dir::Forward, Unit::Word, &NoTokens), [17]);
}

#[test]
fn dotted_capital_i_with_combining_dot_is_one_grapheme() {
    let b = TextBuffer::from_str("I\u{307}x");
    assert_eq!(
        move_offset(&b, 0, Dir::Forward, Unit::Grapheme, &NoTokens),
        3
    );
    assert_eq!(move_offset(&b, 3, Dir::Back, Unit::Grapheme, &NoTokens), 0);
    // One identifier word too.
    assert_eq!(word_at("I\u{307}x y", 0), 0..4);
}

#[test]
fn dotless_i_is_an_ordinary_letter() {
    assert_eq!(word_at("sık_isim", 1), 0..9); // bytes: `ı` is two (P18)
    assert_eq!(char_class('ı'), CharClass::Ident);
    assert_eq!(char_class('İ'), CharClass::Ident);
}

#[test]
fn quoted_identifier_is_one_token_word() {
    assert_eq!(token_word(r#"SELECT "Müşteri Adı" FROM t"#, 9), 7..23); // bytes, quotes included (P18)
    // A word move from inside it, either way, leaves it whole.
    let text = r#"SELECT "Müşteri Adı" FROM t"#;
    let (b, toks) = (buf(text), FakeSql::new(text));
    assert_eq!(move_offset(&b, 16, Dir::Forward, Unit::Word, &toks), 23);
    assert_eq!(move_offset(&b, 16, Dir::Back, Unit::Word, &toks), 7);
    assert_eq!(move_offset(&b, 23, Dir::Back, Unit::Word, &toks), 7);
}

#[test]
fn character_classes() {
    for c in ['a', 'Z', '_', '9', 'ş', 'ü', '\u{307}', '$', '#', '@', '日'] {
        assert_eq!(char_class(c), CharClass::Ident, "{c:?}");
    }
    for c in [' ', '\t', '\u{a0}', '\u{2028}', '\u{85}', '\u{c}'] {
        assert_eq!(char_class(c), CharClass::Space, "{c:?}");
    }
    for c in ['\r', '\n'] {
        assert_eq!(char_class(c), CharClass::LineBreak, "{c:?}");
    }
    for c in ['.', ':', '(', '"', '\'', '\0', '😀', '\u{202e}'] {
        assert_eq!(char_class(c), CharClass::Punct, "{c:?}");
    }
}

#[test]
fn dollar_hash_and_at_are_identifier_characters() {
    let b = buf("a$b #t @p");
    assert_eq!(stops(&b, 0, Dir::Forward, Unit::Word, &NoTokens), [3, 6, 9]);
}

#[test]
fn crlf_is_one_grapheme_unit() {
    let b = buf("a\r\nb");
    assert_eq!(
        move_offset(&b, 1, Dir::Forward, Unit::Grapheme, &NoTokens),
        3
    );
    assert_eq!(move_offset(&b, 3, Dir::Back, Unit::Grapheme, &NoTokens), 1);
    // From between the CR and the LF, either way, to the break's edges.
    assert_eq!(
        move_offset(&b, 2, Dir::Forward, Unit::Grapheme, &NoTokens),
        3
    );
    assert_eq!(move_offset(&b, 2, Dir::Back, Unit::Grapheme, &NoTokens), 1);
    assert_eq!(snap_to_grapheme(&b, 2, Bias::Before), 1);
    assert_eq!(snap_to_grapheme(&b, 2, Bias::After), 3);
}

#[test]
fn regional_indicator_flags_pair_up() {
    let b = buf("🇹🇷🇩🇪x");
    assert_eq!(
        stops(&b, 0, Dir::Forward, Unit::Grapheme, &NoTokens),
        [8, 16, 17]
    );
    assert_eq!(
        stops(&b, 17, Dir::Back, Unit::Grapheme, &NoTokens),
        [16, 8, 0]
    );
    assert_eq!(snap_to_grapheme(&b, 12, Bias::Before), 8);
    assert_eq!(snap_to_grapheme(&b, 4, Bias::After), 8);
}

#[test]
fn words_stop_at_each_line_break() {
    let b = buf("ab  \r\ncd");
    assert_eq!(
        stops(&b, 0, Dir::Forward, Unit::Word, &NoTokens),
        [2, 4, 6, 8]
    );
    assert_eq!(stops(&b, 8, Dir::Back, Unit::Word, &NoTokens), [6, 4, 0]);
}

#[test]
fn tokens_win_over_classes_and_are_clipped_to_the_line() {
    // A qualified name as one token.
    let b = buf("a.b c");
    assert_eq!(
        move_offset(&b, 0, Dir::Forward, Unit::Word, &Given(vec![0..3, 4..5])),
        3
    );
    // A block comment token spanning two lines: moves stay on their line.
    let b = buf("x /* a\nb */ y");
    let toks = Given(vec![0..1, 2..11, 12..13]);
    assert_eq!(move_offset(&b, 1, Dir::Forward, Unit::Word, &toks), 6);
    assert_eq!(move_offset(&b, 7, Dir::Forward, Unit::Word, &toks), 11);
    assert_eq!(move_offset(&b, 11, Dir::Back, Unit::Word, &toks), 7);
}

#[test]
fn sub_words_split_at_ascii_case_transitions_only() {
    let fwd = |text: &str| stops(&buf(text), 0, Dir::Forward, Unit::SubWord, &NoTokens);
    assert_eq!(fwd("HTTPServer"), [4, 10]);
    assert_eq!(fwd("userID"), [4, 6]);
    assert_eq!(fwd("utf8Decode"), [4, 10]);
    // `müşteriAdı` splits at the ASCII `A`, never at `ü` (no Unicode casing, INV-3).
    assert_eq!(fwd("müşteriAdı"), [9, 13]);
    assert_eq!(fwd("ŞehirAdı"), [6, 10]);
    // Back from inside `Bc` (`AB|c`): the boundary before `B` is seen through the `c` after.
    assert_eq!(
        move_offset(&buf("ABc"), 2, Dir::Back, Unit::SubWord, &NoTokens),
        1
    );
    assert_eq!(
        stops(&buf("HTTPServer"), 10, Dir::Back, Unit::SubWord, &NoTokens),
        [4, 0]
    );
    // Token words are refined only when the token is an identifier.
    let text = r#"fooBar "Müşteri Adı""#;
    let (b, toks) = (buf(text), FakeSql::new(text));
    assert_eq!(stops(&b, 0, Dir::Forward, Unit::SubWord, &toks), [3, 6, 23]);
}

#[test]
fn a_sub_word_stop_inside_a_cluster_is_snapped_out_of_it() {
    // `_` + U+0301 is one cluster: the sub-word after the `_` starts inside it, so the stop
    // backward is the cluster's start (1), never the accent (2).
    let b = buf("a_\u{301}b");
    assert_eq!(move_offset(&b, 5, Dir::Back, Unit::SubWord, &NoTokens), 1);
    assert_eq!(stops(&b, 5, Dir::Back, Unit::SubWord, &NoTokens), [1, 0]);
    assert_eq!(stops(&b, 0, Dir::Forward, Unit::SubWord, &NoTokens), [1, 5]);
    // U+0D4E (a Prepend letter) + `_` is one cluster: the `_` stop forward (4) is inside it,
    // so it lands after the cluster (5).
    let b = buf("a\u{d4e}_b");
    assert_eq!(
        move_offset(&b, 0, Dir::Forward, Unit::SubWord, &NoTokens),
        5
    );
    assert_eq!(stops(&b, 6, Dir::Back, Unit::SubWord, &NoTokens), [5, 0]);
}

#[test]
fn whitespace_tokens_from_the_oracle_are_not_words() {
    // The SQL lexer emits `Whitespace` tokens: a move from inside the spaces still skips them
    // and takes the next word.
    let text = "SELECT \t a";
    let b = buf(text);
    let toks = Given(vec![0..6, 6..9, 9..10]);
    assert_eq!(move_offset(&b, 7, Dir::Forward, Unit::Word, &toks), 10);
    assert_eq!(stops(&b, 0, Dir::Forward, Unit::Word, &toks), [6, 10]);
    assert_eq!(move_offset(&b, 8, Dir::Back, Unit::Word, &toks), 0);
    assert_eq!(stops(&b, 10, Dir::Back, Unit::Word, &toks), [9, 0]);
    // A double-click inside them still selects the whole space run.
    assert_eq!(units::word_at(&b, 7, &toks), 6..9);
    // A newline token (past the line's text) never counts either.
    let b = buf("ab\r\ncd");
    let toks = Given(vec![0..2, 2..4, 4..6]);
    assert_eq!(stops(&b, 0, Dir::Forward, Unit::Word, &toks), [2, 4, 6]);
}

#[test]
fn line_end_document_and_vertical_moves() {
    let b = buf("  çay\r\nab\r\nxyz");
    assert_eq!(move_offset(&b, 0, Dir::Forward, Unit::Line, &NoTokens), 6);
    assert_eq!(move_offset(&b, 3, Dir::Back, Unit::Document, &NoTokens), 0);
    assert_eq!(
        move_offset(&b, 3, Dir::Forward, Unit::Document, &NoTokens),
        b.len_bytes()
    );
    // Down from after `ç` (grapheme column 3) onto "ab" clamps to its end, then "xyz" col 2.
    let after_c = 4;
    assert_eq!(grapheme_column(&b, after_c), 3);
    let down = move_offset(&b, after_c, Dir::Down, Unit::Line, &NoTokens);
    assert_eq!(down, 10);
    assert_eq!(move_offset(&b, 9, Dir::Down, Unit::Line, &NoTokens), 13);
    assert_eq!(move_offset(&b, 9, Dir::Up, Unit::Line, &NoTokens), 1);
    assert_eq!(move_offset(&b, 1, Dir::Up, Unit::Line, &NoTokens), 0);
    assert_eq!(offset_at_column(&b, 2, 99), b.len_bytes());
}

#[test]
fn word_at_prefers_a_word_beside_a_space_and_selects_a_space_run() {
    let text = "ab  cd";
    assert_eq!(word_at(text, 2), 0..2, "the word before the spaces");
    assert_eq!(word_at(text, 3), 2..4, "inside the spaces");
    assert_eq!(word_at(text, 6), 4..6, "at the end");
    assert_eq!(word_at("", 0), 0..0);
    assert_eq!(word_at("a\n\nb", 2), 2..2, "an empty line");
}

#[test]
fn a_word_move_on_a_huge_line_is_bounded() {
    // One 1 MiB identifier: a move stops at the scan limit instead of walking it all.
    let b = buf(&"a".repeat(1 << 20));
    assert_eq!(
        move_offset(&b, 0, Dir::Forward, Unit::Word, &NoTokens),
        SCAN_LIMIT
    );
    let end = b.len_bytes();
    assert_eq!(
        move_offset(&b, end, Dir::Back, Unit::Word, &NoTokens),
        end - SCAN_LIMIT
    );
}

#[test]
fn columns_round_trip_on_lines_past_the_scan_limit() {
    // 100,000 `ş` (2 bytes each) per line: exact columns in the first 64 KiB, UTF-16 beyond,
    // and the two functions stay inverses, so Down then Up comes back.
    let line = "ş".repeat(100_000);
    let b = buf(&format!("{line}\n{line}"));
    for at in [
        0,
        10,
        SCAN_LIMIT - 2,
        SCAN_LIMIT,
        SCAN_LIMIT + 2,
        150_000,
        200_000,
    ] {
        let col = grapheme_column(&b, at);
        assert_eq!(col, at / 2, "at {at}");
        assert_eq!(offset_at_column(&b, 0, col), at, "at {at}");
        let down = move_offset(&b, at, Dir::Down, Unit::Line, &NoTokens);
        assert_eq!(down, b.line_to_byte(1) + at);
        assert_eq!(move_offset(&b, down, Dir::Up, Unit::Line, &NoTokens), at);
    }
}

#[test]
fn a_huge_cluster_is_walked_by_characters() {
    // `a` with 100,000 combining marks: past the scan limit the move goes one character.
    let text = format!("a{}", "\u{301}".repeat(100_000));
    let b = buf(&text);
    assert_eq!(
        move_offset(&b, 0, Dir::Forward, Unit::Grapheme, &NoTokens),
        1
    );
    assert_eq!(
        move_offset(&b, b.len_bytes(), Dir::Back, Unit::Grapheme, &NoTokens),
        b.len_bytes() - 2
    );
    // A short one is still one unit.
    let b = buf(&format!("a{}b", "\u{301}".repeat(50)));
    assert_eq!(
        move_offset(&b, 0, Dir::Forward, Unit::Grapheme, &NoTokens),
        101
    );
}

/// Text from interesting pieces (clusters, CRLF, Turkish, emoji), long enough to cross the
/// rope's chunks.
fn arb_text() -> impl Strategy<Value = String> {
    let pieces = prop::sample::select(vec![
        "a",
        "İ",
        "I\u{307}",
        "ı",
        "ş",
        " ",
        "\t",
        "\r\n",
        "\n",
        "\r",
        "👩‍💻",
        "🇹🇷",
        "🇩🇪",
        "e\u{301}",
        "_",
        "$",
        "::",
        "日本",
        "\u{a0}",
        "x",
        "Ab",
        "\u{200d}",
        "\u{600}",
    ]);
    proptest::collection::vec(pieces, 0..600).prop_map(|v| v.concat())
}

/// `PROPTEST_CASES` when set (a stress run), else `default`.
fn cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(64), ..ProptestConfig::default() })]

    #[test]
    fn grapheme_moves_match_the_reference_segmentation(text in arb_text()) {
        let b = buf(&text);
        let mut bounds: Vec<usize> = text.grapheme_indices(true).map(|(i, _)| i).collect();
        bounds.push(text.len());
        bounds.dedup();
        let fwd = stops(&b, 0, Dir::Forward, Unit::Grapheme, &NoTokens);
        prop_assert_eq!(&fwd[..], &bounds[1..]);
        let mut back = stops(&b, text.len(), Dir::Back, Unit::Grapheme, &NoTokens);
        back.reverse();
        prop_assert_eq!(&back[..], &bounds[..bounds.len() - 1]);
        for at in (0..=text.len()).filter(|&i| text.is_char_boundary(i)) {
            let before = snap_to_grapheme(&b, at, Bias::Before);
            let after = snap_to_grapheme(&b, at, Bias::After);
            prop_assert!(bounds.contains(&before) && bounds.contains(&after));
            prop_assert!(before <= at && at <= after);
        }
    }

    #[test]
    fn word_moves_progress_and_land_on_cluster_boundaries(text in arb_text(), sub in any::<bool>()) {
        let b = buf(&text);
        let mut bounds: Vec<usize> = text.grapheme_indices(true).map(|(i, _)| i).collect();
        bounds.push(text.len());
        let unit = if sub { Unit::SubWord } else { Unit::Word };
        let mut prev = 0;
        for at in stops(&b, 0, Dir::Forward, unit, &NoTokens) {
            prop_assert!(at > prev && bounds.contains(&at), "{} after {}", at, prev);
            prev = at;
        }
        prop_assert_eq!(prev, text.len());
        let mut prev = text.len();
        for at in stops(&b, text.len(), Dir::Back, unit, &NoTokens) {
            prop_assert!(at < prev && bounds.contains(&at), "{} before {}", at, prev);
            prev = at;
        }
        prop_assert_eq!(prev, 0);
    }
}
