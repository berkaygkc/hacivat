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

    // ## m3b-t20-differential (named exception, T14 perf carry: no second parse for the
    // fingerprint)
    /// [`Backend::analyse`], and what the backend's own parse of `sql` already says about the
    /// request's fingerprint ([`ParsedFingerprint`]). The default parses nothing for it.
    fn analyse_fingerprinted(
        &self,
        sql: &str,
        items: &[TextRange],
        ctx: &ClassifyCtx,
        lexed: Option<&Lexed>,
    ) -> (Vec<StmtReport>, ParsedFingerprint) {
        (
            self.analyse(sql, items, ctx, lexed),
            ParsedFingerprint::NotParsed,
        )
    }
}

// ## m3b-t20-differential
/// libpg_query's fingerprint of a request, as the PG backend's one parse of it left it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParsedFingerprint {
    /// The backend did not send `sql` itself to libpg_query: the orchestrator asks for the
    /// fingerprint on its own.
    NotParsed,
    /// libpg_query parsed exactly `sql`: its fingerprint, or `None` where
    /// `sql_oracle_pg::tree::fingerprint(sql)` is an `Err`, and also where libpg_query cannot
    /// encode the tree (nested too deeply to serialize) though `fingerprint` would answer: the
    /// backend then reports `ParseError`, so the request keeps `HvNorm1`, as it did before
    /// the one-parse call (fix round 1, m1).
    Parsed(Option<u64>),
}
