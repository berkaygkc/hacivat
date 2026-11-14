//! Relexing after an edit (R-M3-9). M3a: a full relex, `full: true` (incremental relex is
//! M3b; R-M3-22 keeps it off the UI thread).

use std::ops::Range;

use sql_dialect::{LexDialect, LexModes};
use sql_text::{TextChunks, TextEdit};

use crate::cursor::lex_chunks;
use crate::state::LexState;
use crate::token::Lexed;

/// The tokens a relex changed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RelexDelta {
    pub changed: Range<usize>,
    /// Every token was relexed.
    pub full: bool,
}

/// Relexes `lexed` for `new_text` after `edit`: in M3a a full relex.
pub fn relex(
    lexed: &mut Lexed,
    new_text: &dyn TextChunks,
    edit: &TextEdit,
    profile: &LexDialect,
    modes: LexModes,
) -> RelexDelta {
    let _ = edit;
    *lexed = lex_chunks(new_text, profile, modes, &LexState::Initial);
    RelexDelta {
        changed: 0..lexed.len(),
        full: true,
    }
}
