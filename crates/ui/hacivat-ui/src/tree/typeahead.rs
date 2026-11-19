//! Type-ahead (pure): letters typed within [`TYPEAHEAD_RESET`] of each other build a prefix,
//! and the next selectable row whose text starts with it becomes the cursor.
//!
//! Matching never calls a casing function (INV-3, `clippy.toml`): letters fold through an
//! explicit table. [`Fold::Locale`] (the widget's default, R-M4-50) is the access-key fold
//! ([`typed_matches`]): `İ↔i`, `I↔ı` when Turkic, `I↔i` otherwise. [`Fold::Tolerant`] folds
//! ASCII case, the Turkish pairs (`Ç/ç`, `Ş/ş`, `Ö/ö`, `Ü/ü`, `Ğ/ğ`) and all four of `I`,
//! `i`, `İ`, `ı` together, so a user who types `i` finds `İSTANBUL`, `ılık` and `Index`; the
//! explorer sets it for a Turkish UI language or region and ties it to the filter's
//! Turkish-tolerant toggle (D-074, Task 14). A decomposed `I` + U+0307 reads as `İ`.
//!
//! Repeating one letter (`s`, `s`, `s`) cycles through the rows that start with it.

use std::time::{Duration, Instant};

use crate::accesskey::typed_matches;

/// Typing pauses longer than this start a new prefix (measured on the GPUI executor's clock).
pub const TYPEAHEAD_RESET: Duration = Duration::from_millis(1_000);

/// How typed letters match shown ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fold {
    /// Case and dotted/dotless `i` folded together (`I i İ ı` one letter, plus the Turkish
    /// pairs). The explorer sets it for a Turkish UI language or region and ties it to the
    /// filter's Turkish-tolerant toggle (D-074, R-M4-50).
    Tolerant,
    /// The access-key fold for the UI language.
    Locale { turkic: bool },
}

impl Default for Fold {
    /// `Locale { turkic: false }` (R-M4-50: the spike's Tolerant default is overruled by D-074).
    fn default() -> Self {
        Fold::Locale { turkic: false }
    }
}

/// The tolerant fold: `I i İ ı` are one letter; every other letter folds as the access-key
/// fold does without the Turkic rule (exact, the Turkish pairs, ASCII case).
fn tolerant_matches(shown: char, typed: char) -> bool {
    let dotted = |c: char| matches!(c, 'I' | 'i' | 'İ' | 'ı');
    if dotted(shown) || dotted(typed) {
        return dotted(shown) && dotted(typed);
    }
    typed_matches(shown, typed, false)
}

impl Fold {
    /// Whether a typed `typed` selects a shown `shown`.
    pub fn matches(self, shown: char, typed: char) -> bool {
        match self {
            Fold::Tolerant => tolerant_matches(shown, typed),
            Fold::Locale { turkic } => typed_matches(shown, typed, turkic),
        }
    }
}

/// The letters of `text` as typed letters see them: a decomposed `I` + U+0307 is `İ`.
fn letters(text: &str) -> impl Iterator<Item = char> + '_ {
    let mut chars = text.chars().peekable();
    std::iter::from_fn(move || {
        let c = chars.next()?;
        if c == 'I' && chars.peek() == Some(&'\u{307}') {
            chars.next();
            return Some('İ');
        }
        Some(c)
    })
}

/// Whether `text` starts with `typed` under `fold`.
pub fn prefix_matches(text: &str, typed: &str, fold: Fold) -> bool {
    let mut shown = letters(text);
    typed
        .chars()
        .all(|t| shown.next().is_some_and(|s| fold.matches(s, t)))
}

/// The typed prefix and when it was last extended.
#[derive(Debug, Clone, Default)]
pub struct TypeAhead {
    query: String,
    last: Option<Instant>,
}

impl TypeAhead {
    /// Whether a letter typed at `now` extends the prefix.
    pub fn is_active(&self, now: Instant) -> bool {
        !self.query.is_empty()
            && self
                .last
                .is_some_and(|t| now.saturating_duration_since(t) < TYPEAHEAD_RESET)
    }

    /// Adds `typed` at `now` (starting over after a pause) and answers the prefix.
    pub fn push(&mut self, typed: &str, now: Instant) -> &str {
        if !self.is_active(now) {
            self.query.clear();
        }
        self.query.push_str(typed);
        self.last = Some(now);
        &self.query
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn clear(&mut self) {
        self.query.clear();
        self.last = None;
    }
}

/// The row type-ahead moves to for `query`, among `len` rows whose texts `text(ix)` gives
/// (`None`: the row cannot be selected), starting at the cursor `from`. A query of one
/// repeated letter cycles: it starts after the cursor and matches that letter alone. Wraps.
pub fn find<T: AsRef<str>>(
    len: usize,
    from: Option<usize>,
    query: &str,
    fold: Fold,
    mut text: impl FnMut(usize) -> Option<T>,
) -> Option<usize> {
    let mut chars = query.chars();
    let first = chars.next()?;
    let repeated = chars.all(|c| c == first);
    let mut one = [0u8; 4];
    let (needle, skip) = if repeated {
        (&*first.encode_utf8(&mut one), 1)
    } else {
        (query, 0)
    };
    let start = from.map_or(0, |f| f + skip);
    (0..len)
        .map(|i| (start + i) % len)
        .find(|&ix| text(ix).is_some_and(|t| prefix_matches(t.as_ref(), needle, fold)))
}
