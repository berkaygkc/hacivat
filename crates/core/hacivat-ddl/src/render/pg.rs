//! The PostgreSQL renderer. M4 Task 1a stub: every op answers `Unsupported` until Task 7.

use super::{RenderCtx, RenderError, Renderer, Stmt};
use crate::ir::DdlOp;

/// Renders the IR as PostgreSQL text.
#[derive(Debug, Clone, Copy)]
pub struct PgRenderer;

impl Renderer for PgRenderer {
    fn render(&self, op: &DdlOp, ctx: &RenderCtx) -> Result<Vec<Stmt>, RenderError> {
        Err(RenderError::Unsupported {
            dialect: ctx.target.dialect,
            op: op.variant(),
        })
    }
}
