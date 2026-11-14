//! MySQL/MariaDB executable comments and their version gates (Task 4).

use sql_dialect::DialectCtx;

use crate::token::Lexed;

/// Whether an executable comment's text runs on the connected server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GateStatus {
    Active,
    Inactive,
    /// No server version is known.
    Unknown,
}

/// The gate of token `i` of `lexed` (an `ExecutableComment`) under `ctx.version` (Task 4).
/// Task 1b stub (fail closed): `Unknown`.
pub fn exec_comment_gate(text: &str, lexed: &Lexed, i: usize, ctx: &DialectCtx) -> GateStatus {
    let _ = (text, lexed, i, ctx);
    GateStatus::Unknown
}
