//! The MySQL and MariaDB renderer (one renderer, flavor-aware: MariaDB-only clauses and texts
//! differ, F9). M4 Task 1a stub: every op answers `Unsupported` until Task 7.

use super::{RenderCtx, RenderError, Renderer, Stmt};
use crate::ir::DdlOp;

/// Renders the IR as MySQL or MariaDB text.
#[derive(Debug, Clone, Copy)]
pub struct MySqlRenderer {
    pub mariadb: bool,
}

impl Renderer for MySqlRenderer {
    fn render(&self, op: &DdlOp, ctx: &RenderCtx) -> Result<Vec<Stmt>, RenderError> {
        let _ = self.mariadb;
        Err(RenderError::Unsupported {
            dialect: ctx.target.dialect,
            op: op.variant(),
        })
    }
}
