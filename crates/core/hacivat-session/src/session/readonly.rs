//! Guarded read-only execution and the role probe (HV-SESS-04, SQL-SAFE-05, MCP-09 step 1,
//! B21; Task 18): every statement of a read-only session runs inside the guard plan of
//! `hacivat_guard::plan_for`, with the post checks, and the connection is labelled by the role
//! probe.
//!
//! Task 1 stubs ("No silent weakening"): `run_guarded` runs nothing and fails with `Internal`;
//! `access_for` keeps M2a's label (R10) and probes nothing.

use futures::future::BoxFuture;
use hacivat_driver::{ExecOpts, ExecSummary, RawConn};
use hacivat_guard::RoleWarning;
use hacivat_proto::{AccessLabel, DbmsKind};

use crate::session::{ExecError, ExecErrorKind};

/// Runs `sql` on `conn` inside `kind`'s guard plan; `replay` is the session's
/// `EnvPlan::statements()` (without PostgreSQL's client identity), run after the reset (Q15).
/// Task 1 stub: runs nothing, fails with `Internal`.
pub fn run_guarded<'a>(
    _conn: &'a mut dyn RawConn,
    _kind: DbmsKind,
    _sql: &'a str,
    _opts: ExecOpts,
    _replay: &'a [String],
) -> BoxFuture<'a, Result<ExecSummary, ExecError>> {
    Box::pin(async { Err(ExecError::of(ExecErrorKind::Internal)) })
}

/// The access label of a fresh connection and the role probe's warnings. Task 1 stub: M2a's
/// label (read-only is `ReadOnlyGuarded`, never `ReadOnlyEnforced`), no probe, no warnings.
pub fn access_for<'a>(
    _conn: &'a mut dyn RawConn,
    _kind: DbmsKind,
    read_only: bool,
) -> BoxFuture<'a, (AccessLabel, Vec<RoleWarning>)> {
    let label = if read_only {
        AccessLabel::ReadOnlyGuarded
    } else {
        AccessLabel::ReadWrite
    };
    Box::pin(async move { (label, Vec::new()) })
}
