//! Result tab titles: `Result N` in M3a (Task 23); comment titles in M3b (Task 16, R-M3-66).
//!
//! The title is the run service's own label for a tab (logs, tests, the tab list); the app's
//! tab strip shows its localized `hv-results-result-n` when the unit has no comment title.
//!
//! ## m3b-t16-run-service
//! - A unit's title is its directly preceding comment (`UnitSpan::title_comment`): the last
//!   comment before the statement, without its markers (`--`, `#`, `/* … */`) and the spaces
//!   around them; its first line when it spans several. User text, kept as written: never
//!   translated, case-mapped or normalized. A unit without one is `Result N` (N counts the
//!   run's units that opened a tab).
//! - Fix round 1 (review I-1): the title never carries what could re-order or restyle the
//!   chrome: C0/C1 controls (a lone `\r`, ESC, NUL, tab), U+2028/2029, the bidi marks,
//!   embeddings, overrides and isolates (U+200E/200F, U+061C, U+202A–202E, U+2066–2069) and
//!   U+FEFF become white space, runs of white space one space, and the stored title is cut
//!   after [`TITLE_MAX_GRAPHEMES`] graphemes with `…`. An optimizer hint or executable comment
//!   (`/*+ … */`, `/*! … */`, `/*M! … */`) is code, never a title.
//! - Further result sets of the same unit add ` (k)` (k from 2): `müşteriler (2)`.
//! - A title over [`TITLE_GRAPHEMES`] graphemes is shown cut with `…`
//!   ([`short_title`]); the full title stays on the tab for its tooltip.

use std::sync::Arc;

/// The title of a run's `n`-th result tab, from 1.
pub(crate) fn result_title(n: usize) -> Arc<str> {
    format!("Result {n}").into()
}

// ## m3b-t16-run-service

/// The most graphemes a tab shows of its title.
pub const TITLE_GRAPHEMES: usize = 40;

/// The most graphemes of a title kept at all (the tooltip's text; review I-1).
pub const TITLE_MAX_GRAPHEMES: usize = 256;

/// The title a comment gives its statement (the module docs), or `None` when it holds no
/// text. `comment` is the source text of the unit's leading comments.
pub fn comment_title(comment: &str) -> Option<Arc<str>> {
    let last = last_comment(comment.trim());
    let body = strip_markers(last)?;
    let line = body.lines().map(sanitized).find(|l| !l.is_empty())?;
    Some(match grapheme_cut(&line, TITLE_MAX_GRAPHEMES) {
        Some(at) => format!("{}…", line[..at].trim_end()).into(),
        None => line.into(),
    })
}

/// Whether `c` could re-order or restyle the text around it: a control character, a line or
/// paragraph separator, a bidi mark, embedding, override or isolate, or a byte-order mark.
fn is_hostile(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{2028}'
                | '\u{2029}'
                | '\u{200E}'
                | '\u{200F}'
                | '\u{061C}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2066}'..='\u{2069}'
                | '\u{FEFF}'
        )
}

/// One line of a comment as a title: every hostile character and every run of white space
/// one space, a block comment's leading `*` gone, trimmed.
fn sanitized(line: &str) -> String {
    let mut out = String::with_capacity(line.len().min(4 << 10));
    let mut space = true;
    for c in line.chars() {
        if is_hostile(c) || c.is_whitespace() {
            if !space {
                out.push(' ');
                space = true;
            }
        } else {
            out.push(c);
            space = false;
        }
    }
    let t = out.trim().trim_start_matches('*').trim();
    t.to_string()
}

/// The last comment of `text` (a run of comments and white space).
fn last_comment(text: &str) -> &str {
    // A block comment ends the text: its start is the last `/*` that opens it (nested blocks
    // are rare in titles; the outermost unmatched start is good enough).
    if text.ends_with("*/") {
        let mut depth = 0usize;
        let bytes = text.as_bytes();
        let mut i = bytes.len();
        while i >= 2 {
            if &bytes[i - 2..i] == b"*/" {
                depth += 1;
                i -= 2;
                continue;
            }
            if &bytes[i - 2..i] == b"/*" {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return &text[i - 2..];
                }
                i -= 2;
                continue;
            }
            i -= 1;
        }
        return text;
    }
    // Else the last line comment: on the last line that holds text, from its first `--` or
    // `#` outside a block comment.
    let line = text
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let b = line.as_bytes();
    let (mut i, mut depth) = (0usize, 0usize);
    while i < b.len() {
        if b[i..].starts_with(b"/*") {
            depth += 1;
            i += 2;
        } else if depth > 0 && b[i..].starts_with(b"*/") {
            depth -= 1;
            i += 2;
        } else if depth == 0 && (b[i..].starts_with(b"--") || b[i] == b'#') {
            // ASCII bytes: `i` is a character boundary.
            return &line[i..];
        } else {
            i += 1;
        }
    }
    line
}

/// `comment` without its comment markers.
/// `comment` without its comment markers; `None` for an optimizer hint or an executable
/// comment (`/*+`, `/*!`, `/*M!`): code, never a title.
fn strip_markers(comment: &str) -> Option<&str> {
    let c = comment.trim();
    if let Some(inner) = c.strip_prefix("/*") {
        if inner.starts_with(['+', '!']) || inner.starts_with("M!") {
            return None;
        }
        return Some(inner.strip_suffix("*/").unwrap_or(inner));
    }
    if let Some(rest) = c.strip_prefix("--") {
        return Some(rest.trim_start_matches('-'));
    }
    if let Some(rest) = c.strip_prefix('#') {
        return Some(rest);
    }
    if let Some(rest) = c.strip_prefix("//") {
        return Some(rest);
    }
    Some(c)
}

/// The title of a unit's `set`-th result set (from 0): the unit's own title, else
/// `Result {n}`, with ` (k)` for the second set on.
pub(crate) fn tab_title(unit_title: Option<&str>, n: usize, set: u32) -> Arc<str> {
    let base: Arc<str> = match unit_title {
        Some(t) => Arc::from(t),
        None => result_title(n),
    };
    if set == 0 {
        base
    } else {
        format!("{base} ({})", u64::from(set) + 1).into()
    }
}

/// `title` as a tab shows it: its first [`TITLE_GRAPHEMES`] graphemes and `…` when it is
/// longer, else the title itself.
pub fn short_title(title: &str) -> Arc<str> {
    match grapheme_cut(title, TITLE_GRAPHEMES) {
        Some(at) => format!("{}…", &title[..at]).into(),
        None => Arc::from(title),
    }
}

/// The byte offset after the first `n` graphemes of `s`, or `None` when `s` has at most `n`.
///
/// Extended grapheme clusters approximated without a segmentation table (no new dependency
/// in this crate): a cluster is a base character with the combining marks, variation
/// selectors, emoji modifiers and tag characters after it, a ZWJ joining the next character
/// to it, and a pair of regional indicators. Every Turkish letter (precomposed or with a
/// combining dot or cedilla) is one cluster, as are flags and ZWJ emoji sequences.
pub(crate) fn grapheme_cut(s: &str, n: usize) -> Option<usize> {
    let mut count = 0usize;
    let mut chars = s.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if count == n {
            return Some(at);
        }
        count += 1;
        let mut ri = is_regional_indicator(c);
        let mut joined = false;
        while let Some(&(_, next)) = chars.peek() {
            if joined {
                joined = false;
                chars.next();
                continue;
            }
            if next == '\u{200D}' {
                joined = true;
                chars.next();
                continue;
            }
            if ri && is_regional_indicator(next) {
                ri = false;
                chars.next();
                continue;
            }
            if is_extend(next) {
                chars.next();
                continue;
            }
            break;
        }
    }
    None
}

fn is_regional_indicator(c: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&c)
}

/// Characters that extend the cluster before them.
fn is_extend(c: char) -> bool {
    matches!(c,
        '\u{0300}'..='\u{036F}'
        | '\u{0483}'..='\u{0489}'
        | '\u{0591}'..='\u{05BD}'
        | '\u{0610}'..='\u{061A}'
        | '\u{064B}'..='\u{065F}'
        | '\u{0E31}' | '\u{0E34}'..='\u{0E3A}' | '\u{0E47}'..='\u{0E4E}'
        | '\u{1AB0}'..='\u{1AFF}'
        | '\u{1DC0}'..='\u{1DFF}'
        | '\u{200C}'
        | '\u{20D0}'..='\u{20FF}'
        | '\u{FE00}'..='\u{FE0F}'
        | '\u{FE20}'..='\u{FE2F}'
        | '\u{1F3FB}'..='\u{1F3FF}'
        | '\u{E0020}'..='\u{E007F}'
        | '\u{E0100}'..='\u{E01EF}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clusters_count_as_one() {
        // `i` + combining dot above, a flag, a ZWJ family, a thumbs-up with a skin tone.
        let s = "i\u{307}🇹🇷👨\u{200D}👩\u{200D}👧👍🏽x";
        assert_eq!(grapheme_cut(s, 5), None);
        let at = grapheme_cut(s, 4).expect("five clusters");
        assert_eq!(&s[at..], "x");
        assert_eq!(&s[..grapheme_cut(s, 1).unwrap()], "i\u{307}");
    }

    #[test]
    fn markers_are_stripped() {
        for (c, t) in [
            ("-- müşteriler", Some("müşteriler")),
            ("---  sipariş  ", Some("sipariş")),
            ("# MySQL title", Some("MySQL title")),
            ("/* blok */", Some("blok")),
            ("/*\n * first\n * second\n */", Some("first")),
            ("-- a\n-- b", Some("b")),
            ("/* a */ -- b", Some("b")),
            ("-- a\n/* b */", Some("b")),
            ("--", None),
            ("/* */", None),
        ] {
            assert_eq!(comment_title(c).as_deref(), t, "{c:?}");
        }
    }
}
