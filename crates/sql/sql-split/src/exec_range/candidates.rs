//! Execution units, chooser candidates and their labels (Task 13).

use sql_text::{TextRange, TextSize};

use crate::document::DocSnapshot;
use crate::item::{AmbiguousBoundary, Stmt};
use crate::key::StmtKey;

/// Where Run was asked: a caret, or a selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaretOrSelection {
    Caret(TextSize),
    Selection(TextRange),
}

/// What a unit is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnitKind {
    Statement,
    Subquery,
    Batch,
    /// Sent as written (a selection run as one; running it is M3b).
    Verbatim,
    /// psql `COPY … FROM STDIN` with its data range.
    CopyIn {
        data: TextRange,
    },
}

/// One unit to run: its strict range (no terminator, no leading comments; the sent text), its
/// kind, its statement key and its title comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnitSpan {
    pub range: TextRange,
    pub kind: UnitKind,
    pub key: Option<StmtKey>,
    pub title_comment: Option<TextRange>,
}

impl UnitSpan {
    /// The unit that runs `stmt`: its strict range, with its leading comments as the title.
    pub(crate) fn of(stmt: &Stmt) -> UnitSpan {
        UnitSpan {
            range: stmt.strict,
            kind: match stmt.copy_data {
                Some(data) => UnitKind::CopyIn { data: data.rows },
                None => UnitKind::Statement,
            },
            key: Some(stmt.key),
            title_comment: stmt.leading_comments,
        }
    }
}

/// A chooser candidate's kind, in resolve order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CandidateKind {
    Subquery,
    Statement,
    LargestOrBatch,
    FromCaret,
    WholeScript,
}

/// One chooser entry: its units and its label. The label is the candidate's text from its
/// first leading comment, with runs of white space and control characters as one space and
/// every invisible character (Unicode's default-ignorable code points: bidirectional
/// controls, zero-width characters, fillers, variation selectors, tags) shown as `<U+XXXX>`; it holds at most [`LABEL_MAX_CHARS`] characters, and the
/// UI truncates it further.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Candidate {
    pub kind: CandidateKind,
    pub units: Vec<UnitSpan>,
    pub label: String,
}

/// What Run at a caret or selection runs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RangeResolution {
    Units(Vec<UnitSpan>),
    Ask {
        candidates: Vec<Candidate>,
        preselect: usize,
    },
    NothingToRun,
    /// An uncertain boundary is involved: nothing runs (R-M3-24).
    Ambiguous(Vec<AmbiguousBoundary>),
    UnsupportedClientCommand {
        at: TextRange,
    },
}

/// The innermost runnable subquery at a caret (needs the parser: `None` in M3a).
pub trait SubqueryOracle: Send + Sync {
    fn innermost_runnable_subquery(
        &self,
        doc: &DocSnapshot,
        stmt: &Stmt,
        caret: TextSize,
    ) -> Option<TextRange>;
}

/// The most characters a label holds. A popup shows far fewer and truncates with its own
/// ellipsis; the bound keeps a whole-script label from copying a large document.
pub const LABEL_MAX_CHARS: usize = 1024;

/// The characters a label shows as text, because drawn as they are they would change or hide
/// what a person reads of the SQL they are about to run: Unicode's
/// `Default_Ignorable_Code_Point` set (its reserved ranges included), and the interlinear
/// annotation controls. That covers
/// - the bidirectional controls (ALM, LRM, RLM, the embeddings, overrides and isolates),
///   which reorder the text;
/// - the zero-width and invisible characters (soft hyphen, combining grapheme joiner, ZWSP,
///   ZWNJ, ZWJ, word joiner and the invisible operators, Mongolian vowel separator and free
///   variation selectors, Khmer inherent vowels, the Hangul fillers, the BOM), which make
///   two different texts look the same;
/// - the variation selectors, the shorthand and musical format controls, and the tag
///   characters, which hide text.
fn is_invisible_format(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFFB}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}

/// The label of `text`: runs of whitespace and control characters become one space (none at
/// either end), and an invisible formatting character becomes the visible text `<U+XXXX>`,
/// so the label reads in the order the server reads the SQL and shows every character of it. At most [`LABEL_MAX_CHARS`]
/// characters.
pub(crate) fn label_of(text: &str) -> String {
    let mut out = String::new();
    let (mut chars, mut gap) = (0, false);
    for c in text.chars() {
        if c.is_whitespace() || c.is_control() {
            gap = !out.is_empty();
            continue;
        }
        let shown = if is_invisible_format(c) {
            format!("<U+{:04X}>", u32::from(c))
        } else {
            String::from(c)
        };
        let need = shown.chars().count() + usize::from(gap);
        if chars + need > LABEL_MAX_CHARS {
            break;
        }
        if gap {
            out.push(' ');
            gap = false;
        }
        out.push_str(&shown);
        chars += need;
    }
    out
}

/// The label of a candidate that runs `units`: the text from the first unit's leading
/// comments to the last unit's end.
pub(crate) fn label_for(text: &str, units: &[UnitSpan]) -> String {
    let (Some(first), Some(last)) = (units.first(), units.last()) else {
        return String::new();
    };
    let start = first
        .title_comment
        .map_or(first.range.start(), |t| t.start().min(first.range.start()));
    text.get(usize::from(start)..usize::from(last.range.end()))
        .map(label_of)
        .unwrap_or_default()
}

/// Whether two candidates run the same ranges.
fn same_ranges(a: &[UnitSpan], b: &[UnitSpan]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.range == y.range)
}

/// Removes duplicate candidates from `tried` (in resolve order): a `FromCaret` that runs what
/// `WholeScript` runs is dropped, so it only ever appears as a strict subset; any other
/// candidate that runs what an earlier one runs is dropped, so the earliest (most specific)
/// kind stays.
pub(crate) fn dedupe(
    tried: Vec<(CandidateKind, Vec<UnitSpan>)>,
) -> Vec<(CandidateKind, Vec<UnitSpan>)> {
    let whole = tried
        .iter()
        .find(|(k, _)| *k == CandidateKind::WholeScript)
        .map(|(_, u)| u.clone());
    let mut kept: Vec<(CandidateKind, Vec<UnitSpan>)> = Vec::with_capacity(tried.len());
    for (kind, units) in tried {
        if units.is_empty() {
            continue;
        }
        if kind == CandidateKind::FromCaret
            && whole.as_deref().is_some_and(|w| same_ranges(w, &units))
        {
            continue;
        }
        if kept.iter().any(|(_, u)| same_ranges(u, &units)) {
            continue;
        }
        kept.push((kind, units));
    }
    kept
}

/// Which of `kinds` the chooser preselects: the remembered kind when it is offered, else the
/// statement, else the first.
pub(crate) fn preselect(kinds: &[CandidateKind], remembered: Option<CandidateKind>) -> usize {
    let position = |kind| kinds.iter().position(|k| *k == kind);
    remembered
        .and_then(position)
        .or_else(|| position(CandidateKind::Statement))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_collapse_whitespace_and_controls() {
        assert_eq!(
            label_of("  SELECT\t1,\r\n\n  2\u{0}FROM\u{7}\u{85} t  "),
            "SELECT 1, 2 FROM t"
        );
        assert_eq!(label_of(" \n\t "), "");
    }

    #[test]
    fn labels_show_bidi_controls_as_text() {
        assert_eq!(
            label_of("SELECT '\u{202E}x\u{2066}' \u{200F}"),
            "SELECT '<U+202E>x<U+2066>' <U+200F>"
        );
    }

    #[test]
    fn labels_show_every_invisible_format_character_as_text() {
        for (c, shown) in [
            ('\u{00AD}', "<U+00AD>"),
            ('\u{200B}', "<U+200B>"),
            ('\u{200C}', "<U+200C>"),
            ('\u{200D}', "<U+200D>"),
            ('\u{200E}', "<U+200E>"),
            ('\u{200F}', "<U+200F>"),
            ('\u{2060}', "<U+2060>"),
            ('\u{2064}', "<U+2064>"),
            ('\u{FEFF}', "<U+FEFF>"),
            ('\u{E0001}', "<U+E0001>"),
            ('\u{E0041}', "<U+E0041>"),
            ('\u{E007F}', "<U+E007F>"),
            // The rest of Default_Ignorable_Code_Point, one of each range.
            ('\u{034F}', "<U+034F>"),
            ('\u{115F}', "<U+115F>"),
            ('\u{1160}', "<U+1160>"),
            ('\u{17B4}', "<U+17B4>"),
            ('\u{180B}', "<U+180B>"),
            ('\u{180E}', "<U+180E>"),
            ('\u{2065}', "<U+2065>"),
            ('\u{206F}', "<U+206F>"),
            ('\u{3164}', "<U+3164>"),
            ('\u{FE00}', "<U+FE00>"),
            ('\u{FE0F}', "<U+FE0F>"),
            ('\u{FFA0}', "<U+FFA0>"),
            ('\u{FFF0}', "<U+FFF0>"),
            ('\u{FFF9}', "<U+FFF9>"),
            ('\u{FFFB}', "<U+FFFB>"),
            ('\u{1BCA0}', "<U+1BCA0>"),
            ('\u{1D173}', "<U+1D173>"),
            ('\u{1D17A}', "<U+1D17A>"),
            ('\u{E0100}', "<U+E0100>"),
            ('\u{E01EF}', "<U+E01EF>"),
            ('\u{E0FFF}', "<U+E0FFF>"),
            ('\u{061C}', "<U+061C>"),
        ] {
            assert_eq!(
                label_of(&format!("DROP TABLE a{c}b")),
                format!("DROP TABLE a{shown}b")
            );
        }
        // Visible characters next to the ranges are left alone.
        let visible = "a\u{2010}b\u{00AE}c\u{FFFC}d\u{FFFD}e\u{1D172}f\u{3163}g\u{FE10}";
        assert_eq!(label_of(visible), visible);
    }

    #[test]
    fn labels_are_bounded() {
        let long = "ab ".repeat(LABEL_MAX_CHARS);
        let label = label_of(&long);
        let n = label.chars().count();
        assert!(n <= LABEL_MAX_CHARS && n > LABEL_MAX_CHARS - 3, "{n}");
        assert!(long.starts_with(&label));
        // A bidi token is never cut in the middle.
        let bidi = format!("{}\u{202E}z", "a".repeat(LABEL_MAX_CHARS - 3));
        assert_eq!(label_of(&bidi), "a".repeat(LABEL_MAX_CHARS - 3));
    }
}
