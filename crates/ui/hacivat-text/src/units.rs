//! Text units for movement (UI-EDIT-06; Task 10).
//!
//! - **Characters** are extended grapheme clusters (UAX #29, `unicode-segmentation`), found
//!   with a cursor over a small window of the rope that grows only as far as a cluster needs
//!   (so a move is O(cluster), never O(line)). CRLF is one cluster, so it is one unit; so are
//!   `I` + U+0307, a ZWJ emoji sequence and a flag.
//! - **Words** come from the language service's tokens when it has them for the line
//!   ([`TokenOracle`]): a quoted identifier, a string, an operator such as `::` or `->>`, a
//!   parameter marker are each one word. Without them, from character classes
//!   ([`char_class`]): `Ident` (Unicode XID_Continue plus `$ # @`), `Space`, `Punct`,
//!   `LineBreak`, each grapheme classed by its first character. A word move skips a run of
//!   spaces, then one word (a token, or a run of one class); a line break is a stop of its own,
//!   so a move never crosses one together with anything else.
//! - **Sub-words** split an identifier word at `_` runs and at ASCII case transitions only
//!   (`user|ID`, `HTTP|Server`, `utf8|Decode`, `müşteri|Adı`): no Unicode casing (INV-3), so
//!   `ü` or `Ş` never make a transition.
//! - **Smart Home** goes to the first non-space character of the line, and from there to the
//!   line's start; End to the end of the line's text (before its break).
//! - **Up and Down** move one buffer line at the same grapheme column (the display-free
//!   fallback; the view moves by its layout with the goal column,
//!   [`SelectionRange::moved_vertically`](crate::SelectionRange::moved_vertically)).
//!
//! **Bounds.** No move scans more than [`SCAN_LIMIT`] bytes: a word move on a megabyte-long
//! identifier stops at the limit, a grapheme cluster longer than the limit (only adversarial
//! text has one) is walked one character at a time, and grapheme columns count exactly within
//! the limit and in UTF-16 units beyond it. Every move lands on a grapheme boundary, never
//! between the CR and LF of a CRLF.

use std::borrow::Cow;
use std::ops::Range;

use smallvec::SmallVec;
use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation as _};

use crate::buffer::{Bias, TextBuffer};

/// How far one move looks, in bytes: the longest cluster, word run or counted column.
pub const SCAN_LIMIT: usize = 64 * 1024;

/// How far a grapheme lookup first looks on each side of the position.
const FIRST_REACH: usize = 64;

/// How far a move goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unit {
    Grapheme,
    Word,
    SubWord,
    /// Back: smart Home; forward: End.
    Line,
    Document,
}

/// Which way a move goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dir {
    Back,
    Forward,
    Up,
    Down,
}

/// Words from the language service.
pub trait TokenOracle {
    /// The words on `line` (the line's bytes without its break): one range per token, or per
    /// UAX #29 word inside a comment. Ranges that start with whitespace or a line break (the
    /// lexer's `Whitespace` and `Newline` tokens) are ignored, so they may be listed or not.
    /// Ranges may run past the line (a block comment); they are clipped to it. `None` when the
    /// service has no analysis at the current version: the character classes apply.
    fn words_on_line(&self, line: Range<usize>) -> Option<SmallVec<[Range<usize>; 16]>>;
}

/// The class of a character for word movement without tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CharClass {
    /// XID_Continue (letters of every script, digits, `_`, combining marks) and `$ # @`.
    Ident,
    /// Whitespace other than a line break (U+2028 and NEL are not breaks here).
    Space,
    /// Everything else: operators, punctuation, symbols, emoji, controls.
    Punct,
    /// CR or LF.
    LineBreak,
}

/// The word class of `c` (no casing, no locale).
pub fn char_class(c: char) -> CharClass {
    match c {
        '\r' | '\n' => CharClass::LineBreak,
        '$' | '#' | '@' => CharClass::Ident,
        c if unicode_ident::is_xid_continue(c) => CharClass::Ident,
        c if c.is_whitespace() => CharClass::Space,
        _ => CharClass::Punct,
    }
}

/// A grapheme cluster's class: its first character's.
fn class_of(cluster: &str) -> CharClass {
    cluster.chars().next().map_or(CharClass::Space, char_class)
}

/// Where a move of `unit` in `dir` from `at` lands (module docs). `at` is clamped to the text
/// and snapped back to a character boundary first.
pub fn move_offset(
    buf: &TextBuffer,
    at: usize,
    dir: Dir,
    unit: Unit,
    tok: &dyn TokenOracle,
) -> usize {
    let len = buf.len_bytes();
    let at = buf.snap_to_char(at.min(len), Bias::Before);
    match (unit, dir) {
        (Unit::Document, Dir::Back | Dir::Up) => 0,
        (Unit::Document, Dir::Forward | Dir::Down) => len,
        (_, Dir::Up) => vertical(buf, at, -1),
        (_, Dir::Down) => vertical(buf, at, 1),
        (Unit::Grapheme, Dir::Forward) => grapheme_step(buf, at, true),
        (Unit::Grapheme, Dir::Back) => grapheme_step(buf, at, false),
        (Unit::Word, dir) => word_move(buf, at, dir == Dir::Forward, tok, false),
        (Unit::SubWord, dir) => word_move(buf, at, dir == Dir::Forward, tok, true),
        (Unit::Line, Dir::Back) => smart_home(buf, at),
        (Unit::Line, Dir::Forward) => {
            let line = buf.byte_to_line(at);
            buf.line_range(line, false).end
        }
    }
}

/// `at` moved to a grapheme cluster boundary on `bias`'s side (itself when it is one).
pub fn snap_to_grapheme(buf: &TextBuffer, at: usize, bias: Bias) -> usize {
    let len = buf.len_bytes();
    let at = buf.snap_to_char(at.min(len), Bias::Before);
    if at == 0 || at == len {
        return at;
    }
    let start = grapheme_step(buf, at, false);
    let end = grapheme_step(buf, start, true);
    if end <= at {
        // `at` ends the cluster that starts at `start`: a boundary.
        return at;
    }
    match bias {
        Bias::Before => start,
        Bias::After => end,
    }
}

/// The next (`forward`) or previous grapheme boundary from `at` (a char boundary).
fn grapheme_step(buf: &TextBuffer, at: usize, forward: bool) -> usize {
    let len = buf.len_bytes();
    if (forward && at >= len) || (!forward && at == 0) {
        return at.min(len);
    }
    let mut reach = FIRST_REACH;
    loop {
        let lo = buf.snap_to_char(at.saturating_sub(reach), Bias::Before);
        let hi = buf.snap_to_char(at.saturating_add(reach).min(len), Bias::After);
        let window = buf.slice(lo..hi);
        let mut cursor = GraphemeCursor::new(at, len, true);
        let step = if forward {
            cursor.next_boundary(&window, lo)
        } else {
            cursor.prev_boundary(&window, lo)
        };
        match step {
            Ok(Some(b)) => return b,
            Ok(None) => return if forward { len } else { 0 },
            // More context or text is needed: look further, up to the limit.
            Err(_) if reach < SCAN_LIMIT => reach *= 4,
            // A cluster longer than the limit: walk it a character at a time.
            Err(_) if forward => return buf.snap_to_char(at + 1, Bias::After),
            Err(_) => return buf.snap_to_char(at - 1, Bias::Before),
        }
    }
}

/// The word range for a double-click at `at` (UI-EDIT-05 step 9, UI-EDIT-06 rule 3): the
/// token or class run holding the character at `at`; when that is a run of spaces (or `at`
/// ends the line), the word just before `at` if there is one; an empty line gives `at..at`.
pub fn word_at(buf: &TextBuffer, at: usize, tok: &dyn TokenOracle) -> Range<usize> {
    let len = buf.len_bytes();
    let at = snap_to_grapheme(buf, at.min(len), Bias::Before);
    let body = buf.line_range(buf.byte_to_line(at), false);
    let at = at.clamp(body.start, body.end);
    if body.is_empty() {
        return at..at;
    }
    let words = words_on(buf, &body, tok);
    let unit_at = |p: usize| {
        words
            .iter()
            .find(|w| w.start <= p && p < w.end)
            .cloned()
            .unwrap_or_else(|| class_run_around(buf, p, &body))
    };
    let is_space = |r: &Range<usize>| class_at(buf, r.start) == CharClass::Space;
    let right = (at < body.end).then(|| unit_at(at));
    let left = (at > body.start).then(|| unit_at(grapheme_step(buf, at, false)));
    match (right, left) {
        (Some(r), _) if !is_space(&r) => r,
        (_, Some(l)) if !is_space(&l) => l,
        (Some(r), _) => r,
        (None, Some(l)) => l,
        (None, None) => at..at,
    }
}

/// The grapheme column of `at` on its line (what the status bar shows, 0-based here): exact
/// within the line's first [`SCAN_LIMIT`] bytes, counted in UTF-16 units beyond them (the
/// inverse of [`offset_at_column`]).
pub fn grapheme_column(buf: &TextBuffer, at: usize) -> usize {
    let at = buf.snap(at.min(buf.len_bytes()), Bias::Before);
    let start = buf.line_to_byte(buf.byte_to_line(at));
    let exact = buf.snap_to_char(at.min(start + SCAN_LIMIT), Bias::Before);
    let beyond = buf.byte_to_utf16(at) - buf.byte_to_utf16(exact);
    buf.slice(start..exact).graphemes(true).count() + beyond
}

/// The offset `col` grapheme columns into `line` (clamped to the end of the line's text):
/// exact within the line's first [`SCAN_LIMIT`] bytes, in UTF-16 units beyond them (the
/// inverse of [`grapheme_column`]).
pub fn offset_at_column(buf: &TextBuffer, line: usize, col: usize) -> usize {
    if line >= buf.len_lines() {
        return buf.len_bytes();
    }
    let body = buf.line_range(line, false);
    let hi = buf.snap_to_char((body.start + SCAN_LIMIT).min(body.end), Bias::Before);
    let window = buf.slice(body.start..hi);
    let mut seen = 0;
    for (i, _) in window.grapheme_indices(true) {
        if seen == col {
            return body.start + i;
        }
        seen += 1;
    }
    if hi == body.end || seen == col {
        return hi;
    }
    let u = buf.byte_to_utf16(hi) + (col - seen);
    buf.snap(buf.utf16_to_byte(u).min(body.end), Bias::Before)
}

/// One buffer line up (`delta` −1) or down (+1) at the same grapheme column.
fn vertical(buf: &TextBuffer, at: usize, delta: isize) -> usize {
    let line = buf.byte_to_line(at);
    match line.checked_add_signed(delta) {
        None => 0,
        Some(target) if target >= buf.len_lines() => buf.len_bytes(),
        Some(target) => offset_at_column(buf, target, grapheme_column(buf, at)),
    }
}

/// Smart Home: to the first non-space character of the line, from there to its start.
fn smart_home(buf: &TextBuffer, at: usize) -> usize {
    let body = buf.line_range(buf.byte_to_line(at), false);
    let text_start = skip_class(buf, body.start, body.end, true, CharClass::Space);
    if at == text_start {
        body.start
    } else {
        text_start
    }
}

/// The class of the cluster starting at `at`.
fn class_at(buf: &TextBuffer, at: usize) -> CharClass {
    let end = grapheme_step(buf, at, true);
    class_of(&buf.slice(at..end))
}

/// The words the oracle knows on `body`, clipped to it; empty ones, and whitespace or line-break
/// tokens (a move skips spaces by class, then takes one word), dropped.
fn words_on(
    buf: &TextBuffer,
    body: &Range<usize>,
    tok: &dyn TokenOracle,
) -> SmallVec<[Range<usize>; 16]> {
    let Some(words) = tok.words_on_line(body.clone()) else {
        return SmallVec::new();
    };
    words
        .into_iter()
        .filter_map(|w| {
            let start = w.start.max(body.start);
            let end = w.end.min(body.end);
            (start < end && buf.is_char_boundary(start) && buf.is_char_boundary(end))
                .then_some(start..end)
        })
        .filter(|w| {
            // A cluster's class is its first character's: no segmentation per token.
            let first = buf.char_before(buf.snap_to_char(w.start + 1, Bias::After));
            !matches!(
                first.map(char_class),
                Some(CharClass::Space | CharClass::LineBreak)
            )
        })
        .collect()
}

/// The text of `a..b` (char boundaries) for a forward scan from `a`, at most [`SCAN_LIMIT`]
/// bytes of it.
fn forward_window(buf: &TextBuffer, a: usize, b: usize) -> (Cow<'_, str>, usize) {
    let hi = buf.snap_to_char(b.min(a + SCAN_LIMIT), Bias::Before);
    (buf.slice(a..hi), hi)
}

/// The text of `a..b` for a backward scan from `b`, at most [`SCAN_LIMIT`] bytes of it.
fn backward_window(buf: &TextBuffer, a: usize, b: usize) -> (Cow<'_, str>, usize) {
    let lo = buf.snap_to_char(a.max(b.saturating_sub(SCAN_LIMIT)), Bias::After);
    (buf.slice(lo..b), lo)
}

/// From `at`, past every cluster of `class` toward `limit` (forward when `forward`).
fn skip_class(buf: &TextBuffer, at: usize, limit: usize, forward: bool, class: CharClass) -> usize {
    if forward {
        let (text, hi) = forward_window(buf, at, limit);
        text.grapheme_indices(true)
            .find(|(_, g)| class_of(g) != class)
            .map_or(hi, |(i, _)| at + i)
    } else {
        let (text, lo) = backward_window(buf, limit, at);
        text.grapheme_indices(true)
            .rev()
            .find(|(_, g)| class_of(g) != class)
            .map_or(lo, |(i, g)| lo + i + g.len())
    }
}

/// The run of one class from `p` on (forward), or the whole run holding the cluster before `p`
/// (backward: a sub-word move back needs the character after `p` too), within `body`.
fn class_run(buf: &TextBuffer, p: usize, body: &Range<usize>, forward: bool) -> Range<usize> {
    if forward {
        let class = class_at(buf, p);
        p..skip_class(buf, p, body.end, true, class)
    } else {
        let class = class_at(buf, grapheme_step(buf, p, false));
        skip_class(buf, p, body.start, false, class)..skip_class(buf, p, body.end, true, class)
    }
}

/// The run of one class holding the cluster at `p`.
fn class_run_around(buf: &TextBuffer, p: usize, body: &Range<usize>) -> Range<usize> {
    let class = class_at(buf, p);
    skip_class(buf, p, body.start, false, class)..skip_class(buf, p, body.end, true, class)
}

/// A word move (module docs), `sub` for sub-words.
fn word_move(
    buf: &TextBuffer,
    at: usize,
    forward: bool,
    tok: &dyn TokenOracle,
    sub: bool,
) -> usize {
    let at = snap_to_grapheme(buf, at, if forward { Bias::Before } else { Bias::After });
    let line = buf.byte_to_line(at);
    let body = buf.line_range(line, false);
    // At the line's edge, the break is the move.
    if forward && at >= body.end {
        return buf.line_range(line, true).end;
    }
    if !forward && at <= body.start {
        return match line.checked_sub(1) {
            Some(prev) => buf.line_range(prev, false).end,
            None => 0,
        };
    }
    let words = words_on(buf, &body, tok);
    let target = if forward {
        word_forward(buf, at, &body, &words, sub)
    } else {
        word_back(buf, at, &body, &words, sub)
    };
    // Land on a cluster boundary, and always move.
    let snapped = snap_to_grapheme(
        buf,
        target,
        if forward { Bias::After } else { Bias::Before },
    );
    match (forward, snapped) {
        (true, s) if s > at => s,
        (false, s) if s < at => s,
        _ => grapheme_step(buf, at, forward),
    }
}

fn word_forward(
    buf: &TextBuffer,
    at: usize,
    body: &Range<usize>,
    words: &[Range<usize>],
    sub: bool,
) -> usize {
    // Inside a token: to its end, spaces in it included (a quoted identifier).
    let (p, word) = match words.iter().find(|w| w.start < at && at < w.end) {
        Some(w) => (at, w.clone()),
        None => {
            let p = skip_class(buf, at, body.end, true, CharClass::Space);
            if p >= body.end {
                return body.end;
            }
            let word = words
                .iter()
                .find(|w| w.start <= p && p < w.end)
                .cloned()
                .unwrap_or_else(|| class_run(buf, p, body, true));
            (p, word)
        }
    };
    if sub && is_ident_run(buf, &word) {
        sub_word_end(buf, p, word.end)
    } else {
        word.end
    }
}

fn word_back(
    buf: &TextBuffer,
    at: usize,
    body: &Range<usize>,
    words: &[Range<usize>],
    sub: bool,
) -> usize {
    let (p, word) = match words.iter().find(|w| w.start < at && at < w.end) {
        Some(w) => (at, w.clone()),
        None => {
            let p = skip_class(buf, at, body.start, false, CharClass::Space);
            if p <= body.start {
                return body.start;
            }
            let word = words
                .iter()
                .find(|w| w.start < p && p <= w.end)
                .cloned()
                .unwrap_or_else(|| class_run(buf, p, body, false));
            (p, word)
        }
    };
    if sub && is_ident_run(buf, &word) {
        sub_word_start(buf, &word, p)
    } else {
        word.start
    }
}

/// Every cluster of `r` is `Ident` (a bare identifier, keyword or number token).
fn is_ident_run(buf: &TextBuffer, r: &Range<usize>) -> bool {
    r.len() <= SCAN_LIMIT
        && buf
            .slice(r.clone())
            .graphemes(true)
            .all(|g| class_of(g) == CharClass::Ident)
}

/// A sub-word boundary sits before `cur`: an ASCII lowercase letter or digit, then an ASCII
/// capital (`user|ID`, `utf8|Decode`); or capitals, then a capital before a lowercase letter
/// (`HTTP|Server`). ASCII only (INV-3): any other character makes no transition.
fn camel_boundary(prev: char, cur: char, next: Option<char>) -> bool {
    let lower_or_digit = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    cur.is_ascii_uppercase()
        && (lower_or_digit(prev)
            || (prev.is_ascii_uppercase() && next.is_some_and(|n| n.is_ascii_lowercase())))
}

/// The end of the sub-word at or after `p` in the identifier run ending at `end`: past any
/// `_` run, then to the next `_` or case transition.
fn sub_word_end(buf: &TextBuffer, p: usize, end: usize) -> usize {
    let (text, hi) = forward_window(buf, p, end);
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = chars.iter().take_while(|(_, c)| *c == '_').count();
    if i == chars.len() {
        return hi;
    }
    i += 1;
    while let Some(&(at, c)) = chars.get(i) {
        let next = chars.get(i + 1).map(|&(_, n)| n);
        if c == '_' || camel_boundary(chars[i - 1].1, c, next) {
            return p + at;
        }
        i += 1;
    }
    hi
}

/// The start of the sub-word at or before `p` in the identifier run `word`: back past any `_`
/// run, then to the previous `_` or case transition.
fn sub_word_start(buf: &TextBuffer, word: &Range<usize>, p: usize) -> usize {
    let (text, lo) = backward_window(buf, word.start, p);
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let trailing = chars.iter().rev().take_while(|(_, c)| *c == '_').count();
    if trailing == chars.len() {
        return lo;
    }
    // The character at `p`, when the word goes on: `AB|c` still splits as `A|Bc`.
    let after = (p < word.end)
        .then(|| buf.slice(p..grapheme_step(buf, p, true)).chars().next())
        .flatten();
    // The last character of the sub-word; walk back while no boundary sits before it.
    let mut i = chars.len() - trailing - 1;
    while i > 0 {
        let (prev, cur) = (chars[i - 1].1, chars[i].1);
        let next = chars.get(i + 1).map(|&(_, n)| n).or(after);
        if prev == '_' || camel_boundary(prev, cur, next) {
            break;
        }
        i -= 1;
    }
    lo + chars[i].0
}
