//! The orchestrator (`alg/13` SQL-SAFE-02 steps; Task 14): limits, lexing, splitting, the
//! backend for the dialect, the union with the lexer pre-scan, and the request's report.
//!
//! STUB until Task 14 (fail closed): [`classify`] answers `Unknown` with the pre-decision
//! `[NotImplemented]`; `classified_len` and `classified_hash` describe the input.

use std::fmt;
use std::sync::Arc;

use sql_dialect::{ClientFlavor, DialectCtx};

use crate::functions::FnCatalog;
use crate::limits::Limits;
use crate::reason::{Reason, ReasonCode};
use crate::report::SafetyReport;

/// What the caller expects the text to hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Expect {
    /// One execution unit (Run); more than one statement is `MultiStatement`.
    SingleStatement,
    /// A script of statements.
    Script,
}

/// Everything classification depends on besides the text.
#[derive(Clone)]
pub struct ClassifyCtx {
    pub dialect: DialectCtx,
    pub flavor: ClientFlavor,
    pub expect: Expect,
    pub catalog: Option<Arc<dyn FnCatalog>>,
    /// The profile's read-only function allowlist (P-M3b-10).
    pub fn_allowlist: Arc<[Arc<str>]>,
    // ## m3b-t09-pg-backend (named exception, fix round 1, I2b): the field's contract.
    /// The session's search path, one schema per element, each its name exactly as the server
    /// stores it (folded: what `current_schemas(true)` returns, or `SHOW search_path` split and
    /// folded), `$user` kept literally. Empty means unknown (`pg_catalog` assumed first, as the
    /// server searches it first unless it is listed). An element that cannot be such a name
    /// (a raw `SHOW` string, quotes, a comma, edge spaces, `pg_catalog` in another case) is
    /// read fail-closed: unqualified builtins are then unknown (PG backend, P-M3b-T3-1).
    pub search_path: Arc<[Arc<str>]>,
    pub limits: Limits,
}

impl ClassifyCtx {
    /// A context with no catalog, empty allowlist and search path, and the default limits.
    pub fn new(dialect: DialectCtx, flavor: ClientFlavor, expect: Expect) -> Self {
        ClassifyCtx {
            dialect,
            flavor,
            expect,
            catalog: None,
            fn_allowlist: Arc::from(Vec::new()),
            search_path: Arc::from(Vec::new()),
            limits: Limits::default(),
        }
    }
}

impl fmt::Debug for ClassifyCtx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClassifyCtx")
            .field("dialect", &self.dialect)
            .field("flavor", &self.flavor)
            .field("expect", &self.expect)
            .field("catalog", &self.catalog.is_some())
            .field("fn_allowlist", &self.fn_allowlist)
            .field("search_path", &self.search_path)
            .field("limits", &self.limits)
            .finish()
    }
}

/// Classifies `sql` in `ctx`: pure, synchronous, never panics.
///
/// STUB: `Unknown` with the pre-decision `[NotImplemented]`.
pub fn classify(sql: &str, ctx: &ClassifyCtx) -> SafetyReport {
    SafetyReport::pre_decided(
        sql,
        sql_split::hv_norm1(sql, &ctx.dialect),
        vec![Reason::new(ReasonCode::NotImplemented)],
    )
}
