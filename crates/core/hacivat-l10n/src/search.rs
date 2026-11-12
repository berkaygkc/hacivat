//! The search key (I18N-09, ALG-I18N-06): Contains / WordPrefix / Fuzzy with highlight ranges and
//! optional Turkish-tolerant folding (R9, D-074).
//!
//! Folding works per grapheme cluster of the original text, so every folded character maps back
//! to a whole cluster and highlights never split `İ`, `ß` or an emoji:
//! 1. NFKC (width-insensitive: `ＩＤ` → `ID`, `ﬁ` → `fi`); whitespace runs collapse to one space;
//!    Default_Ignorable code points (bidi controls, zero-width characters) are dropped.
//! 2. Turkish-tolerant: `I`, `i`, `İ`, `ı` (and `I`/`i` followed by U+0307) fold to `i`.
//! 3. Everything else: ICU4X full default case folding (not Turkic: `ß` → `ss`, `ς` → `σ`).
//! 4. Turkish-tolerant also folds accents: NFD, drop nonspacing marks, then `ø đ ł ħ ŧ æ œ` →
//!    `o d l h t ae oe`.
//!
//! The key is not locale-dependent: the same text and options give the same key in every UI
//! language. Only `turkish_tolerant` changes it (on for Turkish users, `turkish_tolerant_for`).

use std::ops::Range;

use icu_casemap::{CaseMapper, CaseMapperBorrowed};
use icu_normalizer::{ComposingNormalizerBorrowed, DecomposingNormalizerBorrowed};
use icu_properties::props::{DefaultIgnorableCodePoint, GeneralCategory};
use icu_properties::{CodePointMapData, CodePointSetData};
use icu_segmenter::GraphemeClusterSegmenter;

/// Search options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SearchOpts {
    /// Fold `{I, i, İ, ı}` together and fold accents.
    pub turkish_tolerant: bool,
    pub mode: MatchMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatchMode {
    /// The query anywhere in the text.
    Contains,
    /// The query at the start of a word, or as an acronym of word starts (`ui` → `user_id`).
    WordPrefix,
    /// Contains, else the query's characters in order with gaps (`e10` → `events_2026_10`).
    Fuzzy,
}

/// A pre-folded search target that maps folded positions back to the original text.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SearchKey {
    text: String,
    folded: String,
    /// One entry per source cluster that produced folded text, in order.
    map: Vec<Seg>,
    /// Folded byte offsets where a word starts (start, after `_ . -` and spaces, at digit↔letter
    /// and lower→upper transitions of the original), ascending.
    word_starts: Vec<u32>,
}

/// A source grapheme cluster and where its folded text starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Seg {
    folded: u32,
    src_start: u32,
    src_end: u32,
}

/// How a query matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatchKind {
    /// A whole word.
    ExactWord,
    /// The start of a word.
    Prefix,
    /// Inside a word.
    Substring,
    /// A subsequence or an acronym.
    Fuzzy,
}

/// A match: byte ranges in the original text (extended to grapheme boundaries) and a score. A
/// higher score is a better match; `ExactWord` > `Prefix` > `Substring` > `Fuzzy` always.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Match {
    pub ranges: Vec<Range<usize>>,
    pub score: u32,
    pub kind: MatchKind,
}

/// At most this many highlight ranges per item (ALG-I18N-06 edge 15).
const MAX_RANGES: usize = 100;
/// Fuzzy matching is skipped beyond these sizes (it is quadratic).
const FUZZY_MAX_TEXT: usize = 4096;
const FUZZY_MAX_QUERY: usize = 64;

/// The character class of a cluster for word boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Separator,
    Digit,
    Lower,
    Upper,
    Other,
}

fn class_of(c: char) -> Class {
    if c.is_whitespace() || matches!(c, '_' | '.' | '-') || c.is_ascii_punctuation() {
        Class::Separator
    } else if c.is_numeric() {
        Class::Digit
    } else if c.is_uppercase() {
        Class::Upper
    } else if c.is_lowercase() {
        Class::Lower
    } else {
        Class::Other
    }
}

fn starts_word(prev: Option<Class>, cur: Class) -> bool {
    match (prev, cur) {
        (_, Class::Separator) => false,
        (None | Some(Class::Separator), _) => true,
        (Some(Class::Digit), c) => c != Class::Digit,
        (Some(_), Class::Digit) => true,
        (Some(Class::Lower), Class::Upper) => true,
        _ => false,
    }
}

/// The folding steps shared by keys and queries.
struct Folder {
    tolerant: bool,
    nfkc: ComposingNormalizerBorrowed<'static>,
    nfd: DecomposingNormalizerBorrowed<'static>,
    case: CaseMapperBorrowed<'static>,
}

impl Folder {
    fn new(tolerant: bool) -> Folder {
        Folder {
            tolerant,
            nfkc: ComposingNormalizerBorrowed::new_nfkc(),
            nfd: DecomposingNormalizerBorrowed::new_nfd(),
            case: CaseMapper::new(),
        }
    }

    /// Appends the folded form of one cluster; `space` tracks whether `out` ends in a collapsed
    /// whitespace run.
    fn fold(&self, cluster: &str, out: &mut String, space: &mut bool) {
        let ignorable = CodePointSetData::new::<DefaultIgnorableCodePoint>();
        let gc = CodePointMapData::<GeneralCategory>::new();
        let normalized = self.nfkc.normalize(cluster);
        let mut chars = normalized.chars().peekable();
        while let Some(c) = chars.next() {
            if c.is_whitespace() {
                if !*space {
                    out.push(' ');
                    *space = true;
                }
                continue;
            }
            if ignorable.contains(c) {
                continue;
            }
            *space = false;
            if self.tolerant && matches!(c, 'I' | 'i' | 'İ' | 'ı') {
                if matches!(c, 'I' | 'i') && chars.peek() == Some(&'\u{307}') {
                    chars.next();
                }
                out.push('i');
                continue;
            }
            let mut buf = [0u8; 4];
            let folded = self.case.fold_string(c.encode_utf8(&mut buf));
            if !self.tolerant {
                out.push_str(&folded);
                continue;
            }
            for d in self.nfd.normalize(&folded).chars() {
                if gc.get(d) == GeneralCategory::NonspacingMark {
                    continue;
                }
                match d {
                    'ø' => out.push('o'),
                    'đ' => out.push('d'),
                    'ł' => out.push('l'),
                    'ħ' => out.push('h'),
                    'ŧ' => out.push('t'),
                    'æ' => out.push_str("ae"),
                    'œ' => out.push_str("oe"),
                    'ı' => out.push('i'),
                    _ => out.push(d),
                }
            }
        }
    }

    /// A folded query: trimmed, inner whitespace runs collapsed to one space.
    fn query(&self, q: &str) -> String {
        let mut out = String::with_capacity(q.len());
        let mut space = true; // drops leading whitespace
        for (a, b) in clusters(q) {
            self.fold(&q[a..b], &mut out, &mut space);
        }
        let trimmed = out.trim_end_matches(' ').len();
        out.truncate(trimmed);
        out
    }
}

/// The grapheme clusters of `s` as byte ranges.
fn clusters(s: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    let breaks: Vec<usize> = GraphemeClusterSegmenter::new().segment_str(s).collect();
    (0..breaks.len().saturating_sub(1)).map(move |i| (breaks[i], breaks[i + 1]))
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Builds the search key of `text`.
pub fn build_key(text: &str, o: &SearchOpts) -> SearchKey {
    let folder = Folder::new(o.turkish_tolerant);
    let mut folded = String::with_capacity(text.len());
    let mut map = Vec::new();
    let mut word_starts = Vec::new();
    let mut space = false;
    let mut prev: Option<Class> = None;
    for (a, b) in clusters(text) {
        let start = folded.len();
        folder.fold(&text[a..b], &mut folded, &mut space);
        if folded.len() == start {
            continue; // a dropped control or a collapsed space: no class change either
        }
        map.push(Seg {
            folded: to_u32(start),
            src_start: to_u32(a),
            src_end: to_u32(b),
        });
        let class = text[a..b].chars().next().map_or(Class::Other, class_of);
        if starts_word(prev, class) {
            word_starts.push(to_u32(start));
        }
        prev = Some(class);
    }
    SearchKey {
        text: text.to_string(),
        folded,
        map,
        word_starts,
    }
}

impl SearchKey {
    /// The original text; `Match::ranges` index into it.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The original byte range of the folded range `r` (whole clusters).
    fn source(&self, r: Range<usize>) -> Range<usize> {
        let seg_of = |pos: usize| {
            let i = self.map.partition_point(|s| s.folded as usize <= pos);
            self.map[i.saturating_sub(1)]
        };
        let first = seg_of(r.start);
        let last = seg_of(r.end.saturating_sub(1).max(r.start));
        first.src_start as usize..last.src_end as usize
    }

    fn is_word_start(&self, pos: usize) -> bool {
        self.word_starts.binary_search(&to_u32(pos)).is_ok()
    }

    fn is_word_end(&self, pos: usize) -> bool {
        pos >= self.folded.len()
            || self.is_word_start(pos)
            || self.folded[pos..]
                .chars()
                .next()
                .is_some_and(|c| class_of(c) == Class::Separator)
    }

    fn kind_at(&self, r: &Range<usize>) -> MatchKind {
        match (self.is_word_start(r.start), self.is_word_end(r.end)) {
            (true, true) => MatchKind::ExactWord,
            (true, false) => MatchKind::Prefix,
            _ => MatchKind::Substring,
        }
    }

    /// Source ranges of folded ranges, merged where they touch, at most `MAX_RANGES`.
    fn ranges(&self, folded: impl IntoIterator<Item = Range<usize>>) -> Vec<Range<usize>> {
        let mut out: Vec<Range<usize>> = Vec::new();
        for r in folded {
            let s = self.source(r);
            if let Some(last) = out.last_mut()
                && s.start <= last.end
            {
                last.end = last.end.max(s.end);
                continue;
            }
            if out.len() == MAX_RANGES {
                break;
            }
            out.push(s);
        }
        out
    }
}

fn score(kind: MatchKind, pos: usize) -> u32 {
    let base = match kind {
        MatchKind::ExactWord => 3000,
        MatchKind::Prefix => 2000,
        MatchKind::Substring => 1000,
        MatchKind::Fuzzy => return 0,
    };
    let at_start = if pos == 0 { 500 } else { 0 };
    base + at_start - to_u32(pos.min(499))
}

/// Finds `query` in `text`. An empty (or all-whitespace) query matches everything without a
/// highlight.
pub fn find(text: &SearchKey, query: &str, o: &SearchOpts) -> Option<Match> {
    let q = Folder::new(o.turkish_tolerant).query(query);
    if q.is_empty() {
        return Some(Match {
            ranges: Vec::new(),
            score: 0,
            kind: MatchKind::Substring,
        });
    }
    if q.len() > text.folded.len() && o.mode != MatchMode::Fuzzy {
        return None;
    }
    match o.mode {
        MatchMode::Contains => contains(text, &q),
        MatchMode::WordPrefix => word_prefix(text, &q),
        MatchMode::Fuzzy => contains(text, &q).or_else(|| fuzzy(text, &q)),
    }
}

fn contains(key: &SearchKey, q: &str) -> Option<Match> {
    let hits: Vec<Range<usize>> = key
        .folded
        .match_indices(q)
        .map(|(i, m)| i..i + m.len())
        .collect();
    let best = hits
        .iter()
        .map(|r| (score(key.kind_at(r), r.start), key.kind_at(r)))
        .max_by_key(|(s, _)| *s)?;
    Some(Match {
        ranges: key.ranges(hits),
        score: best.0,
        kind: best.1,
    })
}

fn word_prefix(key: &SearchKey, q: &str) -> Option<Match> {
    let hits: Vec<Range<usize>> = key
        .word_starts
        .iter()
        .map(|&s| s as usize)
        .filter(|&s| key.folded[s..].starts_with(q))
        .map(|s| s..s + q.len())
        .collect();
    if let Some(best) = hits
        .iter()
        .map(|r| (score(key.kind_at(r), r.start), key.kind_at(r)))
        .max_by_key(|(s, _)| *s)
    {
        return Some(Match {
            ranges: key.ranges(hits),
            score: best.0,
            kind: best.1,
        });
    }
    // An acronym: each query character starts a later word.
    let mut starts = key.word_starts.iter().map(|&s| s as usize);
    let mut picked = Vec::new();
    for qc in q.chars().filter(|c| *c != ' ') {
        let s = starts.find(|&s| key.folded[s..].starts_with(qc))?;
        picked.push(s..s + qc.len_utf8());
    }
    let first = picked.first()?.start;
    Some(Match {
        ranges: key.ranges(picked),
        score: 900 - to_u32(first.min(99)),
        kind: MatchKind::Fuzzy,
    })
}

/// A subsequence match maximising: +16 per character, +16 at a word start, +8 at the very start,
/// +32 when consecutive, -1 per skipped character between matches. Consecutive runs outweigh
/// scattered word starts (`e10` prefers `events_2026_10` to `e_x_y_1_z_0`).
fn fuzzy(key: &SearchKey, q: &str) -> Option<Match> {
    let t: Vec<(usize, char)> = key.folded.char_indices().collect();
    let qc: Vec<char> = q.chars().collect();
    let (n, m) = (t.len(), qc.len());
    if m > n || n > FUZZY_MAX_TEXT || m > FUZZY_MAX_QUERY {
        return None;
    }
    const NONE: i64 = i64::MIN / 4;
    let gain = |j: usize| -> i64 {
        16 + if key.is_word_start(t[j].0) { 16 } else { 0 } + if j == 0 { 8 } else { 0 }
    };
    // score[i][j]: best with query[..=i] matched and query[i] at t[j]; from[i][j]: where
    // query[i-1] was matched.
    let mut score = vec![vec![NONE; n]; m];
    let mut from = vec![vec![usize::MAX; n]; m];
    for j in 0..n {
        if t[j].1 == qc[0] {
            score[0][j] = gain(j);
        }
    }
    for i in 1..m {
        // best[k] over earlier positions, decayed by the gap: (value, position).
        let mut run: (i64, usize) = (NONE, usize::MAX);
        for j in 1..n {
            if j >= 2 {
                let cand = score[i - 1][j - 2];
                run.0 -= 1;
                if cand > NONE && cand - 1 > run.0 {
                    run = (cand - 1, j - 2);
                }
            }
            if t[j].1 != qc[i] {
                continue;
            }
            let consecutive = score[i - 1][j - 1];
            let (prev, k) = if consecutive > NONE && consecutive + 32 >= run.0 {
                (consecutive + 32, j - 1)
            } else {
                run
            };
            if prev > NONE {
                score[i][j] = prev + gain(j);
                from[i][j] = k;
            }
        }
    }
    let (mut j, best) = score[m - 1]
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, s)| *s > NONE)
        .max_by_key(|(_, s)| *s)?;
    let mut picked = vec![0; m];
    for i in (0..m).rev() {
        picked[i] = j;
        j = from[i][j];
    }
    let ranges = picked
        .iter()
        .map(|&j| t[j].0..t[j].0 + t[j].1.len_utf8())
        .collect::<Vec<_>>();
    Some(Match {
        ranges: key.ranges(ranges),
        score: u32::try_from(best.clamp(1, 999)).unwrap_or(1),
        kind: MatchKind::Fuzzy,
    })
}

/// Whether Turkish-tolerant folding is on: the UI language or the regional format is Turkish
/// (D-074; the search box toggle overrides it).
pub fn turkish_tolerant_for(ui_lang: &str, format_locale: &str) -> bool {
    let is_tr = |tag: &str| {
        tag.split(['-', '_', '.', '@'])
            .next()
            .is_some_and(|lang| lang.eq_ignore_ascii_case("tr"))
    };
    is_tr(ui_lang) || is_tr(format_locale)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOL: SearchOpts = SearchOpts {
        turkish_tolerant: true,
        mode: MatchMode::Contains,
    };

    #[test]
    fn keys_fold_and_map_clusters() {
        let k = build_key("İSTANBUL_Straße", &TOL);
        assert_eq!(k.folded, "istanbul_strasse");
        assert_eq!(k.text, "İSTANBUL_Straße");
        // `ß` (bytes 14..16; `İ` is two bytes) produced `ss` at 13..15: both map to it.
        assert_eq!(k.source(13..14), 14..16);
        assert_eq!(k.source(14..15), 14..16);
        assert_eq!(k.source(0..1), 0..2);
    }

    #[test]
    fn word_starts_come_from_the_original_text() {
        let k = build_key("userId_2026x.y", &TOL);
        let starts: Vec<&str> = k
            .word_starts
            .iter()
            .map(|&s| &k.folded[s as usize..])
            .collect();
        assert_eq!(
            starts,
            vec!["userid_2026x.y", "id_2026x.y", "2026x.y", "x.y", "y"]
        );
    }

    #[test]
    fn controls_are_dropped_and_whitespace_collapses() {
        let k = build_key("a\u{200B}b \u{2066}\t c", &TOL);
        assert_eq!(k.folded, "ab c");
    }
}
