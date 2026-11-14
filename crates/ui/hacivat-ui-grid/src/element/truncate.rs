//! Width-capped truncation at a grapheme boundary and visible controls (HV-GRID-03; Task 20).
//!
//! - **The cap.** Before shaping, a cell's text is cut to `ceil(col_px / min_advance_px) + 2`
//!   grapheme clusters ([`grapheme_cap`]): no glyph is narrower than the font's narrowest
//!   advance, so the rest could never show. Shaping cost grows with length, and real data holds
//!   kilobyte strings (R2).
//! - **The ellipsis.** A shape wider than its cell is cut at the last cluster boundary whose
//!   prefix and `…` fit ([`fitting_boundary`], a binary search over the boundaries), and
//!   `prefix + "…"` is shaped.
//! - **Controls** become visible ([`segments`]): `\n` → `↵`, `\t` → `→`, the other C0 controls
//!   and DEL → their control pictures (`␀` … `␟`, `␡`), C1 controls → `⟨U+0085⟩`, and the bidi
//!   controls (U+202A–E, U+2066–9, U+200E/F, U+061C) → a named chip such as `⟨RLO⟩` (the Trojan
//!   Source defence). Each is painted as a chip; the cell's text itself is unchanged, so a copy
//!   keeps the real characters.
//! - **Fast-scroll** paints an ASCII-safe prefix ([`fast_prefix`]): at most
//!   [`FAST_PREFIX_CHARS`] characters, stopping before the first one that is not printable
//!   ASCII, so the run shapes in the primary font with no fallback and never shows a wrong
//!   character.

use std::fmt::Write as _;
use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation as _;

/// The ellipsis a truncated shape ends with.
pub const ELLIPSIS: &str = "…";

/// The longest fast-scroll prefix (R-M3-12).
pub const FAST_PREFIX_CHARS: usize = 24;

/// The most grapheme clusters a cell `col_px` wide can show: `ceil(col_px / min_advance) + 2`
/// (`usize::MAX` without a usable advance).
pub fn grapheme_cap(col_px: f32, min_advance_px: f32) -> usize {
    if !(min_advance_px.is_finite() && min_advance_px > 0.0) || !col_px.is_finite() {
        return usize::MAX;
    }
    ((col_px.max(0.0) / min_advance_px).ceil() as usize).saturating_add(2)
}

/// `text` cut to at most `cap` grapheme clusters, and whether anything was cut.
pub fn cap_graphemes(text: &str, cap: usize) -> (&str, bool) {
    // Every cluster is at least one byte: a text no longer than the cap in bytes is whole.
    if text.len() <= cap {
        return (text, false);
    }
    match text.grapheme_indices(true).nth(cap) {
        Some((at, _)) => (&text[..at], true),
        None => (text, false),
    }
}

/// Pushes the cluster boundaries of `text` (`0`, each cluster's end) into `out`.
pub fn cluster_boundaries(text: &str, out: &mut Vec<usize>) {
    out.clear();
    out.push(0);
    out.extend(text.grapheme_indices(true).map(|(at, g)| at + g.len()));
}

/// The largest boundary in `bounds` (ascending, starting at 0) whose prefix, `ellipsis_w` px
/// of ellipsis after it, fits `avail`; `x_for(b)` is the prefix's width and grows with `b`.
/// Binary search: O(log n) widths.
pub fn fitting_boundary(
    bounds: &[usize],
    avail: f32,
    ellipsis_w: f32,
    x_for: impl Fn(usize) -> f32,
) -> usize {
    let fits = |b: usize| x_for(b) + ellipsis_w <= avail;
    // The first boundary that does not fit; everything before it fits.
    let first_over = bounds.partition_point(|b| fits(*b));
    match first_over {
        0 => 0,
        n => bounds[n - 1],
    }
}

/// The fast-scroll prefix of `text`: at most `max_chars` characters, stopping before the first
/// that is not printable ASCII (or a space).
pub fn fast_prefix(text: &str, max_chars: usize) -> &str {
    let end = text
        .bytes()
        .take(max_chars)
        .position(|b| !(b == b' ' || b.is_ascii_graphic()))
        .unwrap_or_else(|| text.len().min(max_chars));
    &text[..end]
}

/// A control made visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chip {
    /// `\n`: `↵`.
    Newline,
    /// `\t`: `→`.
    Tab,
    /// A C0 control or DEL: its control picture.
    Picture(char),
    /// A C1 control: `⟨U+0085⟩`.
    C1(char),
    /// A bidi control: `⟨RLO⟩`.
    Bidi(&'static str),
}

impl Chip {
    /// Appends the chip's text to `out`.
    pub fn write(&self, out: &mut String) {
        match self {
            Chip::Newline => out.push('↵'),
            Chip::Tab => out.push('→'),
            Chip::Picture(c) => out.push(*c),
            Chip::C1(c) => {
                let _ = write!(out, "⟨U+{:04X}⟩", u32::from(*c));
            }
            Chip::Bidi(name) => {
                out.push('⟨');
                out.push_str(name);
                out.push('⟩');
            }
        }
    }
}

/// The chip `c` is shown as; `None` for a character drawn as itself.
pub fn control_chip(c: char) -> Option<Chip> {
    Some(match c {
        '\n' => Chip::Newline,
        '\t' => Chip::Tab,
        '\u{0}'..='\u{1f}' => Chip::Picture(char::from_u32(u32::from('\u{2400}') + u32::from(c))?),
        '\u{7f}' => Chip::Picture('\u{2421}'),
        '\u{80}'..='\u{9f}' => Chip::C1(c),
        '\u{202a}' => Chip::Bidi("LRE"),
        '\u{202b}' => Chip::Bidi("RLE"),
        '\u{202c}' => Chip::Bidi("PDF"),
        '\u{202d}' => Chip::Bidi("LRO"),
        '\u{202e}' => Chip::Bidi("RLO"),
        '\u{2066}' => Chip::Bidi("LRI"),
        '\u{2067}' => Chip::Bidi("RLI"),
        '\u{2068}' => Chip::Bidi("FSI"),
        '\u{2069}' => Chip::Bidi("PDI"),
        '\u{200e}' => Chip::Bidi("LRM"),
        '\u{200f}' => Chip::Bidi("RLM"),
        '\u{061c}' => Chip::Bidi("ALM"),
        _ => return None,
    })
}

/// Whether `text` holds a character [`control_chip`] makes visible.
pub fn has_controls(text: &str) -> bool {
    text.chars().any(|c| control_chip(c).is_some())
}

/// A piece of a cell's display: a run of its own text (a byte range) or a chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seg {
    Text(Range<usize>),
    Chip(Chip),
}

/// Splits `text` into its runs and chips, in order, into `out`.
pub fn segments(text: &str, out: &mut Vec<Seg>) {
    out.clear();
    let mut run = 0;
    for (at, c) in text.char_indices() {
        if let Some(chip) = control_chip(c) {
            if at > run {
                out.push(Seg::Text(run..at));
            }
            out.push(Seg::Chip(chip));
            run = at + c.len_utf8();
        }
    }
    if run < text.len() {
        out.push(Seg::Text(run..text.len()));
    }
}

/// The display text of `text`: its runs with every control replaced by its chip's text.
pub fn display_text(text: &str) -> String {
    let mut segs = Vec::new();
    segments(text, &mut segs);
    let mut out = String::with_capacity(text.len() + 8);
    for seg in &segs {
        match seg {
            Seg::Text(r) => out.push_str(&text[r.clone()]),
            Seg::Chip(chip) => chip.write(&mut out),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cap_is_columns_over_the_narrowest_advance_plus_two() {
        assert_eq!(grapheme_cap(100.0, 3.0), 36);
        assert_eq!(grapheme_cap(99.0, 3.0), 35);
        assert_eq!(grapheme_cap(0.0, 3.0), 2);
        assert_eq!(grapheme_cap(100.0, 0.0), usize::MAX);
        assert_eq!(grapheme_cap(f32::INFINITY, 3.0), usize::MAX);
    }

    #[test]
    fn capping_counts_clusters_not_bytes() {
        assert_eq!(cap_graphemes("abcdef", 3), ("abc", true));
        assert_eq!(cap_graphemes("abc", 3), ("abc", false));
        // "İ" is two bytes and one cluster; "e\u{301}" (e + combining acute) one cluster.
        assert_eq!(cap_graphemes("İİİİ", 3), ("İİİ", true));
        assert_eq!(
            cap_graphemes("e\u{301}e\u{301}x", 2),
            ("e\u{301}e\u{301}", true)
        );
        assert_eq!(cap_graphemes("e\u{301}", 1), ("e\u{301}", false));
    }

    #[test]
    fn the_fitting_boundary_is_the_last_prefix_that_fits_with_the_ellipsis() {
        let mut b = Vec::new();
        cluster_boundaries("abcdefgh", &mut b);
        assert_eq!(b, [0, 1, 2, 3, 4, 5, 6, 7, 8]);
        // 10 px per byte, 8 px of ellipsis, 50 px available: "abcd" (40) + 8 fits, "abcde" not.
        let x = |i: usize| i as f32 * 10.0;
        assert_eq!(fitting_boundary(&b, 50.0, 8.0, x), 4);
        assert_eq!(fitting_boundary(&b, 7.0, 8.0, x), 0);
        assert_eq!(fitting_boundary(&b, 1_000.0, 8.0, x), 8);
        // Boundaries are clusters: never inside "e\u{301}".
        cluster_boundaries("xe\u{301}y", &mut b);
        assert_eq!(b, [0, 1, 4, 5]);
    }

    #[test]
    fn controls_become_chips_and_the_text_stays() {
        assert_eq!(display_text("a\tb\nc"), "a→b↵c");
        assert_eq!(display_text("x\u{202E}y"), "x⟨RLO⟩y");
        assert_eq!(display_text("\u{0}\r\u{1b}\u{7f}"), "␀␍␛␡");
        assert_eq!(display_text("\u{85}"), "⟨U+0085⟩");
        assert_eq!(display_text("plain İstanbul"), "plain İstanbul");
        let mut segs = Vec::new();
        segments("a\tb", &mut segs);
        assert_eq!(
            segs,
            [Seg::Text(0..1), Seg::Chip(Chip::Tab), Seg::Text(2..3)]
        );
        assert!(has_controls("x\u{2067}y"));
        assert!(!has_controls("NULL"));
    }

    #[test]
    fn every_bidi_control_has_a_named_chip() {
        for c in [
            '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{202e}', '\u{2066}', '\u{2067}',
            '\u{2068}', '\u{2069}', '\u{200e}', '\u{200f}', '\u{061c}',
        ] {
            assert!(
                matches!(control_chip(c), Some(Chip::Bidi(_))),
                "U+{:04X}",
                u32::from(c)
            );
        }
    }

    #[test]
    fn fast_prefix_is_ascii_safe_and_short() {
        assert_eq!(fast_prefix("hello world", 24), "hello world");
        assert_eq!(fast_prefix("abcdefghijklmnopqrstuvwxyz", 24).len(), 24);
        assert_eq!(fast_prefix("Kadıköy", 24), "Kad");
        assert_eq!(fast_prefix("İstanbul", 24), "");
        assert_eq!(fast_prefix("a\tb", 24), "a");
        assert_eq!(fast_prefix("abc", 2), "ab");
    }
}
