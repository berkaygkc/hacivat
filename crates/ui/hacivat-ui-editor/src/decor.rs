//! The editor's highlight vocabulary (R-M3-42: the SQL adapter maps `sql_lexer::TokenClass` to
//! it; Task 19 paints each class with a `syn_*` token).

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
