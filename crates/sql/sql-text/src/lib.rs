//! SQL text positions (Task 3): UTF-8 byte offsets as [`TextSize`] (u32) everywhere in
//! `crates/sql`, the line index, char-boundary edits and the lexer's chunked input (R-M3-9).
//!
//! Types complete since M3a Task 1b; [`TextChunks`] for `str` works since Task 1b, [`LineIndex`]
//! and [`TextEdit::apply`] since Task 3.

pub mod chunks;
pub mod edit;
pub mod line_index;

pub use chunks::TextChunks;
pub use edit::TextEdit;
pub use line_index::{LineCol, LineIndex};
pub use text_size::{TextRange, TextSize};
