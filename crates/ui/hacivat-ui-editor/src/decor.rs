//! The editor's highlight vocabulary (R-M3-42: the SQL adapter maps `sql_lexer::TokenClass` to
//! it; Task 19 paints each class with a `syn_*` token).
//!
//! The class ↔ [`StyleId`] mapping (fix round 1): the adapter's decoration source emits
//! `Decoration::Mark { style: class.style() }` (Task 24), the element reads the class back with
//! [`HighlightClass::from_style`] to pick its `syn_*` token (Task 19). **Task 24 owns
//! [`STYLES`]** (a named exception in this Task 1b file); keep it a bijection.

use hacivat_text::display::StyleId;

/// A highlight class.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HighlightClass {
    Plain,
    Keyword,
    Ident,
    QuotedIdent,
    String,
    Number,
    /// Painted italic.
    Comment,
    Operator,
    Punct,
    Param,
    DollarTag,
    /// MySQL `/*! … */`, its own tint.
    ExecComment,
    Hint,
    MetaCommand,
    ErrorToken,
    /// A word directly followed by `(`.
    Function,
}

impl HighlightClass {
    /// Every class, in declaration order (the index into [`STYLES`]).
    pub const ALL: [HighlightClass; 16] = [
        HighlightClass::Plain,
        HighlightClass::Keyword,
        HighlightClass::Ident,
        HighlightClass::QuotedIdent,
        HighlightClass::String,
        HighlightClass::Number,
        HighlightClass::Comment,
        HighlightClass::Operator,
        HighlightClass::Punct,
        HighlightClass::Param,
        HighlightClass::DollarTag,
        HighlightClass::ExecComment,
        HighlightClass::Hint,
        HighlightClass::MetaCommand,
        HighlightClass::ErrorToken,
        HighlightClass::Function,
    ];

    /// The decoration style that carries this class.
    pub fn style(self) -> StyleId {
        STYLES[self as usize]
    }

    /// The class a decoration style carries; `None` for a style that is not a highlight class
    /// (another source's).
    pub fn from_style(style: StyleId) -> Option<HighlightClass> {
        STYLES
            .iter()
            .position(|s| *s == style)
            .map(|i| HighlightClass::ALL[i])
    }
}

/// The style of each class, indexed by `HighlightClass as usize` (Task 24 owns the values;
/// Task 1b: the class's own index).
pub const STYLES: [StyleId; 16] = [
    StyleId(0),
    StyleId(1),
    StyleId(2),
    StyleId(3),
    StyleId(4),
    StyleId(5),
    StyleId(6),
    StyleId(7),
    StyleId(8),
    StyleId(9),
    StyleId(10),
    StyleId(11),
    StyleId(12),
    StyleId(13),
    StyleId(14),
    StyleId(15),
];
