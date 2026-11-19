//! Renderers (HV-DDL-07 "two layers"): one per dialect turns the IR into statement text.
//! Unsupported `(dialect, op)` combinations answer `RenderError::Unsupported` explicitly,
//! never a "cannot generate" comment.
//!
//! M4 Task 1a seam: every renderer answers `Unsupported` until Task 7.

pub mod mysql;
pub mod pg;
pub mod sqlite;

use std::fmt;

use hacivat_meta::name::Name;
use sql_dialect::DialectId;

use crate::DdlTarget;
use crate::ir::DdlOp;
use crate::options::GenOptions;

/// How a statement relates to transactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TxnClass {
    /// Runs inside a transaction and rolls back with it.
    Transactional,
    /// Commits the open transaction (MySQL DDL).
    ImplicitCommit,
    /// Cannot run inside a transaction (`CREATE INDEX CONCURRENTLY`).
    NonTransactional,
}

/// What ends a statement in file output.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Delim {
    Semicolon,
    /// A client delimiter around a routine body in file output (MySQL `DELIMITER`).
    Custom(Name),
}

/// One rendered statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stmt {
    pub text: String,
    pub txn: TxnClass,
    pub delimiter: Delim,
}

/// What a renderer needs besides the op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderCtx {
    pub target: DdlTarget,
    pub opts: GenOptions,
}

impl RenderCtx {
    pub fn new(target: DdlTarget, opts: GenOptions) -> Self {
        RenderCtx { target, opts }
    }
}

/// Why an op cannot be rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    /// The dialect cannot express this op (or it is not written yet).
    Unsupported {
        dialect: DialectId,
        op: &'static str,
    },
    /// A name that cannot be quoted (it holds a NUL).
    Quote(sql_ident::quote::QuoteError),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::Unsupported { dialect, op } => {
                write!(f, "{op} is not supported for {dialect:?}")
            }
            RenderError::Quote(e) => write!(f, "cannot quote a name: {e:?}"),
        }
    }
}

impl std::error::Error for RenderError {}

/// A dialect's renderer.
pub trait Renderer: Send + Sync {
    fn render(&self, op: &DdlOp, ctx: &RenderCtx) -> Result<Vec<Stmt>, RenderError>;
}

/// The renderer of a dialect M4 has no renderer for (SQL Server, ClickHouse, Oracle, generic).
#[derive(Debug, Clone, Copy)]
struct NoRenderer;

impl Renderer for NoRenderer {
    fn render(&self, op: &DdlOp, ctx: &RenderCtx) -> Result<Vec<Stmt>, RenderError> {
        Err(RenderError::Unsupported {
            dialect: ctx.target.dialect,
            op: op.variant(),
        })
    }
}

/// The renderer of `d`.
pub fn renderer(d: DialectId) -> &'static dyn Renderer {
    match d {
        DialectId::Postgres => &pg::PgRenderer,
        DialectId::MySql => &mysql::MySqlRenderer { mariadb: false },
        DialectId::MariaDb => &mysql::MySqlRenderer { mariadb: true },
        DialectId::Sqlite => &sqlite::SqliteRenderer,
        _ => &NoRenderer,
    }
}
