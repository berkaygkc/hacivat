//! Split items and boundaries (Task 8).

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

/// One statement: its strict range (no terminator, no leading comments), its smart range
/// (with them), the terminator, the leading comments, how certain its boundaries are, its head
/// and its content key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Stmt {
    pub strict: TextRange,
    pub smart: TextRange,
    pub terminator: Option<TextRange>,
    pub leading_comments: Option<TextRange>,
    pub confidence: Confidence,
    pub head: StmtHead,
    pub key: StmtKey,
}

/// A statement, or an empty item (a lone `;`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Item {
    Statement(Stmt),
    Empty { range: TextRange },
}

/// Why a boundary is uncertain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AmbiguityReason {
    SemicolonInExecutableComment,
    UnterminatedConstruct,
    UnknownBlock,
    DelimiterChange,
}

/// A boundary execution mode will not cross.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AmbiguousBoundary {
    pub at: TextRange,
    pub reason: AmbiguityReason,
}

/// A client command (psql `\cmd`, mysql `DELIMITER`, sqlite3 `.cmd`) and whether Hacivat
/// supports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientSegment {
    pub range: TextRange,
    pub supported: bool,
}

/// A split text.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Split {
    pub items: Vec<Item>,
    pub ambiguous: Vec<AmbiguousBoundary>,
    pub client: Vec<ClientSegment>,
}
