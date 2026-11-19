//! The SQLite renderer. M4 Task 1a stub: every op answers `Unsupported` until Task 7.

use super::{RenderCtx, RenderError, Renderer, Stmt};
use crate::ir::DdlOp;

/// Renders the IR as SQLite text.
#[derive(Debug, Clone, Copy)]
pub struct SqliteRenderer;

impl Renderer for SqliteRenderer {
    fn render(&self, op: &DdlOp, ctx: &RenderCtx) -> Result<Vec<Stmt>, RenderError> {
        Err(RenderError::Unsupported {
            dialect: ctx.target.dialect,
            op: op.variant(),
        })
    }
}
