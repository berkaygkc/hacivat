//! Split items and boundaries (Task 8).
//!
//! A [`Split`] is what `split` returns for one text in one [`SplitMode`]:
//! - **Editor mode** (highlighting, the Run preview) is total and stable under typing: it adds
//!   `Heuristic` items where a `;` seems to be missing (SQL-SPLIT-03) and, after an
//!   unterminated construct, keeps the later statements as provisional `Heuristic` items (the
//!   stability rule).
//! - **Execution mode** is exact or refuses: every item is `Exact`, and every place where a
//!   boundary is uncertain is an [`AmbiguousBoundary`] in `ambiguous`, whose `at` overlaps the
//!   strict range of each statement it makes uncertain. Execution never sends such a statement
//!   (R-M3-24): Run asks.
//!
//! Both modes list the same `ambiguous` boundaries, so an editor item is `Exact` exactly when an
//! execution item has the same strict range and no ambiguity overlaps it (R-M3-8).

use sql_dialect::KeywordId;
use sql_text::TextRange;

use crate::key::StmtKey;

/// Editor mode adds heuristic items (missing-semicolon and blank-line recovery); execution
/// mode never returns a heuristic runnable item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SplitMode {
    Editor,
    Execution,
}

/// Whether a statement's boundaries are certain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Confidence {
    Exact,
    Heuristic,
}

/// A statement's first significant token and its keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct StmtHead {
    pub first_keyword: Option<KeywordId>,
    pub first_token: Option<TextRange>,
}

/// psql `COPY … FROM STDIN` data: the data lines, and the `\.` line that ends them (`None`
/// when the text ends first, which also makes the statement ambiguous).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CopyData {
    /// The data lines, from the line after the `COPY` statement's terminator to the `\.` line
    /// (line breaks included; empty when there are no rows).
    pub rows: TextRange,
    /// The `\.` that ends the data.
    pub end: Option<TextRange>,
}

/// One statement: its strict range (first to last significant token: no terminator, no
/// leading comments; the text sent), its smart range (with the attached leading comments, the
/// terminator and a same-line trailing comment), the terminator (`;`, the client delimiter, or
/// a client command such as `\g`), the leading comments, how certain its boundaries are, its
/// head, its content key, and its `COPY … FROM STDIN` data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Stmt {
    pub strict: TextRange,
    pub smart: TextRange,
    pub terminator: Option<TextRange>,
    pub leading_comments: Option<TextRange>,
    pub confidence: Confidence,
    pub head: StmtHead,
    pub key: StmtKey,
    pub copy_data: Option<CopyData>,
}

/// A statement, or an empty item (a lone `;` or delimiter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Item {
    Statement(Stmt),
    Empty { range: TextRange },
}

impl Item {
    /// The statement, when this is one.
    pub fn statement(&self) -> Option<&Stmt> {
        match self {
            Item::Statement(s) => Some(s),
            Item::Empty { .. } => None,
        }
    }

    /// The item's smart range (an empty item's range).
    pub fn smart(&self) -> TextRange {
        match self {
            Item::Statement(s) => s.smart,
            Item::Empty { range } => *range,
        }
    }
}

/// Why a boundary is uncertain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AmbiguityReason {
    /// A `;` (or the client delimiter) inside an executable comment `/*! … */`: a server that
    /// runs the comment ends the statement there (`split.semicolon-in-executable-comment`).
    SemicolonInExecutableComment,
    /// A string, quoted identifier, dollar quote, executable comment, block or `COPY` data
    /// still open where the statement ends.
    UnterminatedConstruct,
    /// The block stack does not match (`END IF` closing a `LOOP`, an `END` with nothing open
    /// in a stored program, a `)` that closes an open `CASE`).
    UnknownBlock,
    /// Under a client delimiter (`DELIMITER $$`), a top-level `;` inside one statement: the
    /// server would run two statements as one unit.
    DelimiterChange,
    /// Where the editor's heuristic puts a missing `;` (SQL-SPLIT-03).
    MissingSemicolon,
    /// An executable comment or optimizer hint whose end depends on how the server reads its
    /// body (`LexErrorCode::AmbiguousCommentEnd`).
    AmbiguousCommentEnd,
    /// A client command inside the statement or ending it that Hacivat cannot run as written
    /// (psql `\echo` between its lines, `\;`, `\gset`, mysql `\c`).
    ClientCommand,
}

/// A boundary execution mode will not cross: `at` overlaps the strict range of every statement
/// it makes uncertain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AmbiguousBoundary {
    pub at: TextRange,
    pub reason: AmbiguityReason,
}

/// What a client command is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientKind {
    /// mysql `DELIMITER x` / `\d x`: the splitter applies it; it is never sent.
    Delimiter,
    /// Ends the statement (psql `\g`, mysql `\G`); the segment is the statement's terminator.
    Terminator,
    /// Discards the statement typed so far (psql `\r`, mysql `\c`).
    ClearBuffer,
    /// psql `\;`: a `;` that does not end the statement.
    InlineSemicolon,
    /// Any other client command (psql `\set`, `\echo`; mysql `source`; sqlite3 `.mode`).
    Command,
}

/// A client command (psql `\cmd`, mysql `DELIMITER`, sqlite3 `.cmd`): its range (to the end of
/// its line, line break excluded, for line commands; psql `\copy … from stdin` with its data
/// rows through `\.`), its kind, and whether Hacivat runs the
/// script as the client would (`DELIMITER` and `\g` yes; `\set`, `source`, `.mode` no — Run on
/// them is `UnsupportedClientCommand`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientSegment {
    pub range: TextRange,
    pub kind: ClientKind,
    pub supported: bool,
}

/// A splitter diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Diagnostic {
    pub range: TextRange,
    pub code: DiagCode,
}

/// What a diagnostic says (`DiagCode::id` is its stable id).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagCode {
    /// `DELIMITER $$;`: the delimiter is the whole run, `;` included.
    DelimiterEndsWithSemicolon,
    /// `DELIMITER` with nothing after it: the delimiter is unchanged.
    DelimiterWithoutArgument,
    /// A delimiter holding a backslash, which the client refuses: the delimiter is unchanged.
    DelimiterInvalid,
    /// A `;` inside an executable comment.
    SemicolonInExecutableComment,
    /// A block still open where the statement ends.
    UnterminatedBlock,
    /// A `)` or `]` with no open parenthesis.
    UnbalancedClose,
    /// An `END` (or `END IF`, …) that closes nothing or the wrong block.
    EndWithoutBlock,
    /// `COPY … FROM STDIN` data without its `\.` line.
    UnterminatedCopyData,
    /// Where a `;` seems to be missing (editor heuristic).
    MissingSemicolonSuspected,
}

impl DiagCode {
    /// The diagnostic's stable id (`split.…`), as the golden corpora name it.
    pub fn id(self) -> &'static str {
        match self {
            DiagCode::DelimiterEndsWithSemicolon => "split.delimiter-ends-with-semicolon",
            DiagCode::DelimiterWithoutArgument => "split.delimiter-without-argument",
            DiagCode::DelimiterInvalid => "split.delimiter-invalid",
            DiagCode::SemicolonInExecutableComment => "split.semicolon-in-executable-comment",
            DiagCode::UnterminatedBlock => "split.unterminated-block",
            DiagCode::UnbalancedClose => "split.unbalanced-close",
            DiagCode::EndWithoutBlock => "split.end-without-block",
            DiagCode::UnterminatedCopyData => "split.unterminated-copy-data",
            DiagCode::MissingSemicolonSuspected => "split.missing-semicolon-suspected",
        }
    }
}

/// A split text: its items in text order, the uncertain boundaries, the client commands and
/// the diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Split {
    pub items: Vec<Item>,
    pub ambiguous: Vec<AmbiguousBoundary>,
    pub client: Vec<ClientSegment>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Split {
    /// The statements, in text order.
    pub fn statements(&self) -> impl Iterator<Item = &Stmt> {
        self.items.iter().filter_map(Item::statement)
    }

    /// The uncertain boundaries that overlap `range` (a statement's strict range).
    pub fn ambiguities_in(&self, range: TextRange) -> impl Iterator<Item = &AmbiguousBoundary> {
        self.ambiguous.iter().filter(move |a| overlaps(a.at, range))
    }
}

/// Whether two ranges share a byte, or an empty one sits inside the other.
pub(crate) fn overlaps(a: TextRange, b: TextRange) -> bool {
    if a.is_empty() || b.is_empty() {
        return b.contains_range(a) || a.contains_range(b);
    }
    a.start() < b.end() && b.start() < a.end()
}
