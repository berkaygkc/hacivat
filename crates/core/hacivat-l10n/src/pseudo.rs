//! Pseudo-locales `en-XA` and `tr-XT` (I18N-16, ALG-I18N-03), built from the `en` source.
//!
//! The literal transform replaces ASCII letters with accented look-alikes (`tr-XT` first injects
//! `ı İ ş ğ ç ö ü`), lengthens the text to the length-budget table by repeating vowels (or
//! padding with `·`), and leaves alone what must stay byte-identical: markup tags, entities,
//! URLs, `<code>` elements and backtick spans, single-grapheme literals (access keys),
//! whitespace-only literals, digits and punctuation. Placeables are separate pattern elements
//! and are never touched. Messages are bracketed as a whole by `wrap_message` at render time, so
//! truncation shows as a missing `]` and concatenation as `][`.
//!
//! Pseudo rendering needs the `pseudo` feature (dev and nightly builds): without it
//! [`PseudoMode::for_tag`], the only switch, never answers.

use std::borrow::Cow;
use std::ops::Range;

use fluent_syntax::ast;
use unic_langid::LanguageIdentifier;
use unicode_segmentation::UnicodeSegmentation;

/// Which pseudo transform applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PseudoMode {
    /// `en-XA`: accented, lengthened English.
    EnXa,
    /// `tr-XT`: Turkish letters injected, lengthened.
    TrXt,
}

impl PseudoMode {
    /// The pseudo mode for `tag` (`en-XA`, `tr-XT`); the only switch. `None` for every other tag,
    /// and for every tag in builds without the `pseudo` feature.
    pub fn for_tag(tag: &LanguageIdentifier) -> Option<PseudoMode> {
        if !cfg!(feature = "pseudo") || tag.script.is_some() || tag.variants().next().is_some() {
            return None;
        }
        match (
            tag.language.as_str(),
            tag.region.as_ref().map(|r| r.as_str()),
        ) {
            ("en", Some("XA")) => Some(PseudoMode::EnXa),
            ("tr", Some("XT")) => Some(PseudoMode::TrXt),
            _ => None,
        }
    }
}

const ACCENTED_UPPER: [char; 26] = [
    'Ȧ', 'Ɓ', 'Ƈ', 'Ḓ', 'Ḗ', 'Ƒ', 'Ɠ', 'Ħ', 'Ī', 'Ĵ', 'Ķ', 'Ŀ', 'Ḿ', 'Ƞ', 'Ǿ', 'Ƥ', 'Ɋ', 'Ř', 'Ş',
    'Ŧ', 'Ŭ', 'Ṽ', 'Ẇ', 'Ẋ', 'Ẏ', 'Ẑ',
];
const ACCENTED_LOWER: [char; 26] = [
    'ȧ', 'ƀ', 'ƈ', 'ḓ', 'ḗ', 'ƒ', 'ɠ', 'ħ', 'ī', 'ĵ', 'ķ', 'ŀ', 'ḿ', 'ƞ', 'ǿ', 'ƥ', 'ɋ', 'ř', 'ş',
    'ŧ', 'ŭ', 'ṽ', 'ẇ', 'ẋ', 'ẏ', 'ẑ',
];

/// Transforms one literal (ALG-I18N-03 steps 1–5), skipping access keys, markup and URLs.
pub fn transform_literal(s: &str, m: PseudoMode) -> Cow<'_, str> {
    if s.trim().is_empty() || s.graphemes(true).nth(1).is_none() {
        return Cow::Borrowed(s);
    }
    let spans = spans(s);
    let translatable = || spans.iter().filter(|(_, protected)| !protected);
    if !translatable().any(|(r, _)| s[r.clone()].bytes().any(|b| b.is_ascii_alphabetic())) {
        return Cow::Borrowed(s);
    }

    let len: usize = translatable()
        .map(|(r, _)| s[r.clone()].graphemes(true).count())
        .sum();
    let extra = target_len(len).saturating_sub(len);
    let vowels: usize = translatable()
        .map(|(r, _)| s[r.clone()].chars().filter(|c| is_vowel(*c)).count())
        .sum();
    let (per_vowel, first_with_one_more, padding) = match vowels {
        0 => (0, 0, extra),
        v => (extra / v, extra % v, 0),
    };

    let mut out = String::with_capacity(s.len() * 3);
    let mut letters = Letters::default();
    let mut vowel_index = 0;
    let mut after_last_letter = 0;
    for (range, protected) in &spans {
        let text = &s[range.clone()];
        if *protected {
            out.push_str(text);
            continue;
        }
        for c in text.chars() {
            if !c.is_ascii_alphabetic() {
                out.push(c);
                continue;
            }
            let mapped = letters.map(c, m);
            out.push(mapped);
            if is_vowel(c) {
                let copies = per_vowel + usize::from(vowel_index < first_with_one_more);
                out.extend(std::iter::repeat_n(mapped, copies));
                vowel_index += 1;
            }
            after_last_letter = out.len();
        }
    }
    if padding > 0 {
        let pad: String = std::iter::repeat_n('·', padding).collect();
        out.insert_str(after_last_letter, &pad);
    }
    Cow::Owned(out)
}

/// The total length a literal of `len` graphemes must reach: ≤ 10 → +100 %, 11–20 → +80 %,
/// 21–30 → +60 %, 31–50 → +40 %, 51–70 → +30 %, > 70 → +20 % (rounded up).
fn target_len(len: usize) -> usize {
    let pct = match len {
        0..=10 => 100,
        11..=20 => 80,
        21..=30 => 60,
        31..=50 => 40,
        51..=70 => 30,
        _ => 20,
    };
    (len * (100 + pct)).div_ceil(100)
}

fn is_vowel(c: char) -> bool {
    matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'A' | 'E' | 'I' | 'O' | 'U')
}

/// Maps ASCII letters; `tr-XT` alternates dotless/dotted for `i` and dotted/plain for `I`.
#[derive(Default)]
struct Letters {
    lower_i: usize,
    upper_i: usize,
}

impl Letters {
    fn map(&mut self, c: char, m: PseudoMode) -> char {
        if m == PseudoMode::TrXt {
            let turkish = match c {
                'i' => {
                    self.lower_i += 1;
                    Some(if self.lower_i % 2 == 1 { 'ı' } else { 'i' })
                }
                'I' => {
                    self.upper_i += 1;
                    Some(if self.upper_i % 2 == 1 { 'İ' } else { 'I' })
                }
                's' => Some('ş'),
                'S' => Some('Ş'),
                'g' => Some('ğ'),
                'G' => Some('Ğ'),
                'c' => Some('ç'),
                'C' => Some('Ç'),
                'o' => Some('ö'),
                'O' => Some('Ö'),
                'u' => Some('ü'),
                'U' => Some('Ü'),
                _ => None,
            };
            if let Some(t) = turkish {
                return t;
            }
        }
        let table = if c.is_ascii_uppercase() {
            &ACCENTED_UPPER
        } else {
            &ACCENTED_LOWER
        };
        let index = usize::from(c.to_ascii_lowercase() as u8 - b'a');
        table.get(index).copied().unwrap_or(c)
    }
}

/// Splits `s` into (range, protected) spans. Protected: `<code>…</code>` elements, backtick
/// spans, markup tags, entities and `http(s)://` URLs.
fn spans(s: &str) -> Vec<(Range<usize>, bool)> {
    let mut out: Vec<(Range<usize>, bool)> = Vec::new();
    let mut push = |r: Range<usize>, protected: bool| match out.last_mut() {
        Some((last, p)) if *p == protected && last.end == r.start => last.end = r.end,
        _ => out.push((r, protected)),
    };
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        let protected_len = if rest.starts_with("<code>") || rest.starts_with("<code ") {
            Some(
                rest.find("</code>")
                    .map_or(rest.len(), |e| e + "</code>".len()),
            )
        } else if let Some(code) = rest.strip_prefix('`') {
            code.find('`').map(|e| e + 2)
        } else if rest.starts_with('<')
            && bytes
                .get(i + 1)
                .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'/' || *b == b'!')
        {
            rest.find('>').map(|e| e + 1)
        } else if rest.starts_with('&') {
            entity_len(rest)
        } else if rest.starts_with("https://") || rest.starts_with("http://") {
            Some(rest.find(char::is_whitespace).unwrap_or(rest.len()))
        } else {
            None
        };
        match protected_len {
            Some(n) => {
                push(i..i + n, true);
                i += n;
            }
            None => {
                let n = rest.chars().next().map_or(1, char::len_utf8);
                push(i..i + n, false);
                i += n;
            }
        }
    }
    out
}

/// The length of an entity (`&nbsp;`, `&#160;`) at the start of `s`.
fn entity_len(s: &str) -> Option<usize> {
    let body = s[1..]
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'#')
        .count();
    (body > 0 && s.as_bytes().get(1 + body) == Some(&b';')).then_some(body + 2)
}

/// Transforms every text element of every message (value, attributes, select variants) in
/// `res`. Term values are transformed too, except brand terms (`-brand-*`, F6: the brand is never
/// pseudo-transformed); term attributes are selector data and are never transformed.
pub fn transform_resource(res: &mut ast::Resource<String>, m: PseudoMode) {
    for entry in &mut res.body {
        match entry {
            ast::Entry::Message(msg) => {
                if let Some(value) = &mut msg.value {
                    transform_pattern(value, m);
                }
                for attr in &mut msg.attributes {
                    transform_pattern(&mut attr.value, m);
                }
            }
            ast::Entry::Term(term) if !term.id.name.starts_with("brand-") => {
                transform_pattern(&mut term.value, m);
            }
            _ => {}
        }
    }
}

fn transform_pattern(p: &mut ast::Pattern<String>, m: PseudoMode) {
    for el in &mut p.elements {
        match el {
            ast::PatternElement::TextElement { value } => {
                if let Cow::Owned(t) = transform_literal(value, m) {
                    *value = t;
                }
            }
            ast::PatternElement::Placeable { expression } => transform_expression(expression, m),
        }
    }
}

fn transform_expression(e: &mut ast::Expression<String>, m: PseudoMode) {
    match e {
        ast::Expression::Select { variants, .. } => {
            for v in variants {
                transform_pattern(&mut v.value, m);
            }
        }
        ast::Expression::Inline(ast::InlineExpression::Placeable { expression }) => {
            transform_expression(expression, m);
        }
        ast::Expression::Inline(_) => {}
    }
}

/// Brackets a rendered message: `[` … `]`.
pub fn wrap_message(s: String) -> String {
    format!("[{s}]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_protect_markup_entities_urls_and_code() {
        let s = "a <b>x</b> &amp; `y` <code>z</code> https://q.r s";
        let protected: Vec<&str> = spans(s)
            .into_iter()
            .filter(|(_, p)| *p)
            .map(|(r, _)| &s[r])
            .collect();
        assert_eq!(
            protected,
            vec![
                "<b>",
                "</b>",
                "&amp;",
                "`y`",
                "<code>z</code>",
                "https://q.r"
            ]
        );
    }

    #[test]
    fn lone_markers_are_text() {
        let s = "a < b & c ` d";
        assert!(spans(s).iter().all(|(_, p)| !p));
    }

    #[test]
    fn budget_table() {
        assert_eq!(
            [1, 10, 11, 20, 21, 30, 31, 50, 51, 70, 71, 100].map(target_len),
            [2, 20, 20, 36, 34, 48, 44, 70, 67, 91, 86, 120]
        );
    }
}
