//! Caret and selection markers in golden fixtures (Task 13).
//!
//! An inline `input` marks a caret with `┃` and a selection with `«…»`; the markers are
//! removed and their byte offsets in the clean text are returned. A `fixture` is copied
//! verbatim and so cannot hold markers: its caret is an [`Anchor`] (`after`/`before` the n-th
//! occurrence of a text), its selection a [`Select`].

use std::ops::Range;

use serde::Deserialize;

/// The caret marker.
pub const CARET: char = '┃';
/// The selection markers.
pub const OPEN: char = '«';
pub const CLOSE: char = '»';

/// An input without its markers, and where they were (byte offsets into `text`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Marked {
    pub text: String,
    pub carets: Vec<usize>,
    pub selections: Vec<Range<usize>>,
}

/// Removes the markers of `input`.
///
/// # Errors
/// A `»` without a `«`, a `«` inside a selection, or a `«` that is never closed.
pub fn extract(input: &str) -> Result<Marked, String> {
    let mut out = Marked::default();
    let mut open: Option<usize> = None;
    for c in input.chars() {
        match c {
            CARET => out.carets.push(out.text.len()),
            OPEN => {
                if open.replace(out.text.len()).is_some() {
                    return Err("a selection opens inside a selection".into());
                }
            }
            CLOSE => {
                let Some(start) = open.take() else {
                    return Err("a selection closes without opening".into());
                };
                out.selections.push(start..out.text.len());
            }
            other => out.text.push(other),
        }
    }
    if open.is_some() {
        return Err("a selection is never closed".into());
    }
    Ok(out)
}

/// A caret in a fixture: right `after` (or right `before`) the `occurrence`-th (1-based,
/// default 1) occurrence of a text.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub occurrence: Option<usize>,
}

/// A selection in a fixture: the `occurrence`-th occurrence of `text`, or `all` of it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Select {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub occurrence: Option<usize>,
    #[serde(default)]
    pub all: bool,
}

/// Where the `occurrence`-th `needle` is in `text`.
fn nth(text: &str, needle: &str, occurrence: Option<usize>) -> Result<Range<usize>, String> {
    let n = occurrence.unwrap_or(1);
    if n == 0 || needle.is_empty() {
        return Err(format!(
            "anchor {needle:?}: occurrences count from 1 and the text is not empty"
        ));
    }
    text.match_indices(needle)
        .nth(n - 1)
        .map(|(at, m)| at..at + m.len())
        .ok_or_else(|| format!("anchor {needle:?} has no occurrence {n}"))
}

impl Anchor {
    /// The caret's byte offset in `text`.
    ///
    /// # Errors
    /// When the anchor names neither or both of `after` and `before`, or its text is missing.
    pub fn offset(&self, text: &str) -> Result<usize, String> {
        match (&self.after, &self.before) {
            (Some(a), None) => Ok(nth(text, a, self.occurrence)?.end),
            (None, Some(b)) => Ok(nth(text, b, self.occurrence)?.start),
            _ => Err("a caret anchor names exactly one of `after` and `before`".into()),
        }
    }
}

impl Select {
    /// The selection's byte range in `text`.
    ///
    /// # Errors
    /// When it names neither or both of `text` and `all`, or its text is missing.
    pub fn range(&self, text: &str) -> Result<Range<usize>, String> {
        match (&self.text, self.all) {
            (Some(t), false) => nth(text, t, self.occurrence),
            (None, true) => Ok(0..text.len()),
            _ => Err("a selection names exactly one of `text` and `all = true`".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_are_removed_and_located_in_the_clean_text() {
        let m = extract("SELECT ┃1; «SELECT 2»;┃").unwrap();
        assert_eq!(m.text, "SELECT 1; SELECT 2;");
        assert_eq!(m.carets, [7, 19]);
        assert_eq!(m.selections.len(), 1);
        assert_eq!((m.selections[0].start, m.selections[0].end), (10, 18));
        assert_eq!(&m.text[m.selections[0].clone()], "SELECT 2");
    }

    #[test]
    fn unbalanced_selections_are_errors() {
        assert!(extract("a » b").is_err());
        assert!(extract("« a « b » »").is_err());
        assert!(extract("« a").is_err());
    }

    #[test]
    fn anchors_count_occurrences_from_one() {
        let text = "SELECT 1; SELECT 2;";
        let second = Anchor {
            after: Some("SELECT".into()),
            before: None,
            occurrence: Some(2),
        };
        assert_eq!(second.offset(text).unwrap(), 16);
        let before = Anchor {
            after: None,
            before: Some("SELECT 2".into()),
            occurrence: None,
        };
        assert_eq!(before.offset(text).unwrap(), 10);
        let missing = Anchor {
            after: Some("SELECT".into()),
            before: None,
            occurrence: Some(3),
        };
        assert!(missing.offset(text).is_err());
        let both = Anchor {
            after: Some("a".into()),
            before: Some("b".into()),
            occurrence: None,
        };
        assert!(both.offset(text).is_err());
        let all = Select {
            text: None,
            occurrence: None,
            all: true,
        };
        assert_eq!(all.range(text).unwrap(), 0..19);
    }
}
