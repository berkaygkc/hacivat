//! Classifier backends: libpg_query for PostgreSQL ([`pg`], Task 9), the interim `sqlparser`
//! path for MySQL, MariaDB and SQLite ([`interim`], Task 10) with the lexer pre-scan
//! ([`prescan`], Task 10), and the per-dialect plan the orchestrator follows ([`plan`],
//! Task 14). Crate-private: no backend type is public (R-M3-54).

use sql_lexer::Lexed;
use sql_text::TextRange;

use crate::classify::ClassifyCtx;
use crate::report::StmtReport;

pub(crate) mod interim;
pub(crate) mod pg;
pub(crate) mod plan;
pub(crate) mod prescan;

/// One backend: a report per statement range of `sql`.
// ## m3b-t14-orchestrator (named exception, T10 C1 carry): the orchestrator hands its lexing
// in, so a backend does not lex the same text again.
pub(crate) trait Backend {
    /// `lexed`, when given, is `sql` lexed from `LexState::Initial` with `ctx.dialect`'s
    /// profile and modes and no client flavor (exactly what `sql_lexer::lex` answers); a
    /// backend that needs that lexing may use it instead of lexing again.
    fn analyse(
        &self,
        sql: &str,
        items: &[TextRange],
        ctx: &ClassifyCtx,
        lexed: Option<&Lexed>,
    ) -> Vec<StmtReport>;
}
