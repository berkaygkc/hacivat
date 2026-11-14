//! The byte cursor and the lexing entry points (Task 4).

use sql_dialect::{LexDialect, LexModes};
use sql_text::{TextChunks, TextSize};

use crate::state::LexState;
use crate::token::Lexed;

/// Lexes `text` with `profile` and `modes`, starting in `start` (Task 4). Total and lossless:
/// the tokens tile the text.
///
/// Task 1b stub (fail closed): one `Unknown` token over the whole input.
pub fn lex(text: &str, profile: &LexDialect, modes: LexModes, start: &LexState) -> Lexed {
    let _ = (profile, modes, start);
    Lexed::unknown(text_len(text.len()))
}

/// [`lex`] over chunked text (M3a: Task 4 materialises the chunks and calls `lex`).
pub fn lex_chunks(
    text: &dyn TextChunks,
    profile: &LexDialect,
    modes: LexModes,
    start: &LexState,
) -> Lexed {
    let _ = (profile, modes, start);
    Lexed::unknown(text_len(text.len()))
}

/// A byte length as a `TextSize`.
///
/// # Panics
/// Above 4 GiB, which no editor document reaches (R-M3-49 caps it at 20 MB).
fn text_len(len: usize) -> TextSize {
    TextSize::from(u32::try_from(len).expect("a SQL text is under 4 GiB"))
}
