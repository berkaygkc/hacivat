//! The SQL lexer (SQL-LEX-02, -04 use, -05; Task 4): total and lossless over any input,
//! table-driven by a `LexDialect`, with no recursion; its output is a struct of arrays with
//! O(log n) position queries and a chunked interface (R-M3-9).
//!
//! Types complete since M3a Task 1b. Task 1b stub (fail closed): [`lex`] returns one `Unknown`
//! token over the whole input (lossless), `relex` is a full relex, `exec_comment_gate` answers
//! `Unknown`; the `Lexed` accessors work over whatever was lexed.

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
pub use cursor::{lex, lex_chunks};
pub use exec_comment::{GateStatus, exec_comment_gate};
pub use relex::{RelexDelta, relex};
pub use state::{LexState, StringRule};
pub use token::{LexError, LexErrorCode, Lexed, TokenFlags, TokenKind};
