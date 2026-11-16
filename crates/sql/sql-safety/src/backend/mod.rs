//! Classifier backends: libpg_query for PostgreSQL ([`pg`], Task 9), the interim `sqlparser`
//! path for MySQL, MariaDB and SQLite ([`interim`], Task 10) with the lexer pre-scan
//! ([`prescan`], Task 10), and the per-dialect plan the orchestrator follows ([`plan`],
//! Task 14). Crate-private: no backend type is public (R-M3-54).

use sql_text::TextRange;

use crate::classify::ClassifyCtx;
use crate::report::StmtReport;

pub(crate) mod interim;
pub(crate) mod pg;
pub(crate) mod plan;
pub(crate) mod prescan;

/// One backend: a report per statement range of `sql`.
#[allow(
    dead_code,
    reason = "implemented by Tasks 9 and 10, called by Task 14's orchestrator"
)]
pub(crate) trait Backend {
    fn analyse(&self, sql: &str, items: &[TextRange], ctx: &ClassifyCtx) -> Vec<StmtReport>;
}
