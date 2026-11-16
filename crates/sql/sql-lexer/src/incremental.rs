//! Incremental relexing (SQL-LEX-03; Task 5): public checkpoints, so a relex restarts at the
//! last exact state before an edit instead of the start of the text.
//!
//! STUB until Task 5: [`Lexed::checkpoint_before`] answers the start of the text in the
//! initial state, which is exact for every text `lex` lexed from `LexState::Initial` (a full
//! relex: correct, slow).

use sql_text::TextSize;

use crate::state::LexState;
use crate::token::Lexed;

impl Lexed {
    /// The last offset at or before `at` where lexing can restart, with the state there.
    /// STUB: `(0, LexState::Initial)`.
    pub fn checkpoint_before(&self, at: TextSize) -> (TextSize, LexState) {
        let _ = at;
        (TextSize::from(0), LexState::Initial)
    }
}

#[cfg(test)]
mod tests {
    use sql_dialect::{DialectCtx, DialectId, lex_profile};

    use super::*;

    #[test]
    fn stub_checkpoint_is_the_start() {
        let ctx = DialectCtx::new(DialectId::Postgres, None);
        let profile = lex_profile(&ctx).unwrap();
        let l = crate::lex(
            "SELECT 1;\nSELECT 2;\n",
            profile,
            ctx.modes,
            &LexState::Initial,
        );
        assert_eq!(
            l.checkpoint_before(TextSize::from(12)),
            (TextSize::from(0), LexState::Initial)
        );
    }
}
