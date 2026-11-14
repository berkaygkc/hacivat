//! The SQL lexer (SQL-LEX-02, -04 use, -05; Task 4): total and lossless over any input,
//! table-driven by a `LexDialect` (PostgreSQL 14/15/16+, MySQL, MariaDB, SQLite, generic) and
//! the session's `LexModes`, with no recursion; its output is a struct of arrays with O(log n)
//! position queries and a chunked interface (R-M3-9).
//!
//! - [`lex`] is one linear pass (`memchr` for terminators); every byte belongs to exactly one
//!   token, and malformed input becomes flagged tokens and [`LexError`]s, never a panic.
//! - Words carry their keyword (`sql_dialect::keyword_lookup`, ASCII-only: `lımıt` and `lİmİt`
//!   are never `LIMIT`).
//! - [`LexState`] resumes lexing inside an unterminated construct; `relex` is a full relex in
//!   M3a (incremental relex is M3b).
//! - [`exec_comment_gate`] reads an executable comment's version gate; [`token_class`] is the
//!   highlight class.

pub mod class;
pub mod comments;
pub mod cursor;
pub mod dollar;
pub mod exec_comment;
pub mod numbers;
pub mod ops;
pub mod params;
pub mod relex;
pub mod state;
pub mod strings;
pub mod token;
pub mod words;

pub use class::{TokenClass, token_class};
#[doc(hidden)]
pub use cursor::lex_counting_steps;
pub use cursor::{lex, lex_chunks};
pub use exec_comment::{ExecComment, ExecMarker, GateStatus, exec_comment, exec_comment_gate};
pub use relex::{RelexDelta, relex};
pub use state::{LexState, StringRule};
pub use token::{LexError, LexErrorCode, Lexed, TokenFlags, TokenKind};
