//! The search key (I18N-09): Contains / WordPrefix / Fuzzy with highlight ranges and optional
//! Turkish-tolerant folding (R9). Bodies in Task 9.

use std::ops::Range;

/// Search options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SearchOpts {
    /// Fold `{I, i, İ, ı}` together and fold accents.
    pub turkish_tolerant: bool,
    pub mode: MatchMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatchMode {
    Contains,
    WordPrefix,
    Fuzzy,
}

/// A pre-folded search target that maps folded positions back to the original text.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SearchKey {
    text: String,
}

/// How a query matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatchKind {
    ExactWord,
    Prefix,
    Substring,
    Fuzzy,
}

/// A match: byte ranges in the original text (extended to grapheme boundaries) and a score.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Match {
    pub ranges: Vec<Range<usize>>,
    pub score: u32,
    pub kind: MatchKind,
}

/// Builds the search key of `text`. Stub: keeps the text unfolded.
pub fn build_key(text: &str, o: &SearchOpts) -> SearchKey {
    let _ = o;
    SearchKey {
        text: text.to_string(),
    }
}

/// Finds `query` in `text`. Stub: `None`.
pub fn find(text: &SearchKey, query: &str, o: &SearchOpts) -> Option<Match> {
    let _ = (&text.text, query, o);
    None
}

/// Whether Turkish-tolerant folding is on: the UI language or the regional format is Turkish
/// (D-074). Stub: `false`.
pub fn turkish_tolerant_for(ui_lang: &str, format_locale: &str) -> bool {
    let _ = (ui_lang, format_locale);
    false
}
