//! Relexing after an edit (R-M3-9, R-M3-65; SQL-LEX-03): incremental since M3b (Task 5;
//! `incremental`), and still off the UI thread (R-M3-22).

use std::ops::Range;

use sql_dialect::{LexDialect, LexModes};
use sql_text::{TextChunks, TextEdit};

use crate::incremental::relex_text;
use crate::token::Lexed;

/// The tokens a relex changed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RelexDelta {
    /// The new tokens that may differ from the old ones (indices into the new lexing); the
    /// tokens before are unchanged, those after are the old ones moved by the edit.
    pub changed: Range<usize>,
    /// The relex ran to the end of the text: nothing after the edit re-synchronised (an opened
    /// string, comment or dollar quote), or the edit touched a client command, a `DELIMITER`
    /// line or a lexical-mode setting, which relex the tail on purpose.
    pub full: bool,
}

/// Relexes `lexed` (a lexing of the text before `edit`) for `new_text`, the text after it, in
/// place: from the last exact state before the edit until the new tokens re-synchronise with
/// the old ones. The result always equals a full lex of `new_text` from the lexing's start
/// state, with its client flavor (`lex_with_client`). An edit that does not describe the change
/// from the old text's length to `new_text` relexes everything.
pub fn relex(
    lexed: &mut Lexed,
    new_text: &dyn TextChunks,
    edit: &TextEdit,
    profile: &LexDialect,
    modes: LexModes,
) -> RelexDelta {
    let mut chunks = new_text.chunks();
    match (chunks.next(), chunks.next()) {
        (None, _) => relex_text(lexed, "", edit, profile, modes),
        (Some(only), None) => relex_text(lexed, only, edit, profile, modes),
        _ => {
            let whole: String = new_text.chunks().collect();
            relex_text(lexed, &whole, edit, profile, modes)
        }
    }
}
