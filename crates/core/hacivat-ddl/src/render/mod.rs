//! Renderers: one per dialect, IR in, statements out (alg/03 HV-DDL-07 "two layers").
//!
//! Output rules (plan-m4 Task 7 step 3): never from Fluent, `\n` line endings only, four-space
//! indentation, statement text without its terminator (the [`crate::Script`] adds `;` or the
//! MySQL file delimiter). Identifiers go through [`crate::quote`] only; expressions, types
//! and server text are emitted verbatim.

use hacivat_meta::Name;
use hacivat_meta::props::{Deferrable, FkAction, FkMatch};
use sql_dialect::DialectId;

use crate::error::RenderError;
use crate::ir::{DdlOp, IndexPartDef, QName};
use crate::options::DdlTarget;
use crate::produce::qualify::QualifyPlan;
use crate::quote::{quote, quote_always, string_literal};

pub mod mysql;
pub mod pg;
pub mod sqlite;

/// Whether a statement may run inside a transaction (alg/03 HV-DDL-06 segments).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxnClass {
    /// Transactional DDL (PostgreSQL, SQLite).
    Transactional,
    /// Commits implicitly (MySQL / MariaDB DDL).
    ImplicitCommit,
    /// Must run outside a transaction (`CREATE INDEX CONCURRENTLY`).
    NoTransaction,
}

/// How a statement ends in file output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delim {
    /// A plain `;`.
    Semicolon,
    /// The text holds `;` of its own (a MySQL routine or trigger body): a MySQL file needs a
    /// `DELIMITER` around it.
    Block,
}

/// One rendered statement, without its terminator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stmt {
    pub text: String,
    pub txn: TxnClass,
    pub delimiter: Delim,
}

/// What rendering depends on besides the op.
#[derive(Debug, Clone, Copy)]
pub struct RenderCtx<'a> {
    pub target: &'a DdlTarget,
    pub qualify: &'a QualifyPlan,
}

impl<'a> RenderCtx<'a> {
    pub fn new(target: &'a DdlTarget, qualify: &'a QualifyPlan) -> Self {
        RenderCtx { target, qualify }
    }

    pub fn dialect(&self) -> DialectId {
        self.target.dialect
    }

    /// An identifier, quoted where needed.
    pub fn ident(&self, name: &str) -> String {
        quote(self.target.quote_ctx(), name).into_owned()
    }

    /// An identifier, always quoted.
    pub fn ident_quoted(&self, name: &str) -> String {
        quote_always(self.target.quote_ctx(), name)
    }

    /// A schema-qualified name as the qualification plan wants it.
    pub fn qname(&self, q: &QName) -> String {
        match &q.schema {
            Some(s) if self.qualify.show_schema(q) => {
                format!("{}.{}", self.ident(s), self.ident(&q.name))
            }
            _ => self.ident(&q.name),
        }
    }

    /// `(a, b, c)` of identifiers.
    pub fn ident_list(&self, names: &[Name]) -> String {
        let parts: Vec<String> = names.iter().map(|n| self.ident(n)).collect();
        format!("({})", parts.join(", "))
    }

    /// A string literal; a NUL is refused.
    pub fn literal(&self, text: &str, what: &'static str) -> Result<String, RenderError> {
        string_literal(self.target.literal_ctx(), text).map_err(|_| RenderError::Nul { what })
    }
}

/// A renderer of one dialect.
pub trait Renderer {
    fn render(&self, op: &DdlOp, ctx: &RenderCtx<'_>) -> Result<Vec<Stmt>, RenderError>;
}

/// The renderer of `d`; `None` for dialects without one in v1 (SQL Server, ClickHouse,
/// Oracle, generic).
pub fn renderer(d: DialectId) -> Option<&'static dyn Renderer> {
    match d {
        DialectId::Postgres | DialectId::PgWire(_) => Some(&pg::PgRenderer),
        DialectId::MySql | DialectId::MariaDb => Some(&mysql::MySqlRenderer),
        DialectId::Sqlite => Some(&sqlite::SqliteRenderer),
        _ => None,
    }
}

/// Renders `ops` and joins them the way [`crate::Script::text`] does with semicolons: the
/// test and display helper for IR-level goldens.
pub fn render_text(ops: &[DdlOp], ctx: &RenderCtx<'_>) -> Result<String, RenderError> {
    let r = renderer(ctx.dialect()).ok_or(RenderError::Unsupported {
        op: "any",
        dialect: ctx.dialect(),
    })?;
    let mut stmts = Vec::new();
    for op in ops {
        stmts.extend(r.render(op, ctx)?);
    }
    Ok(crate::script::join_semicolon(&stmts))
}

pub(crate) fn unsupported(op: &DdlOp, d: DialectId) -> RenderError {
    RenderError::Unsupported {
        op: op.variant(),
        dialect: d,
    }
}

pub(crate) fn stmt(text: String, txn: TxnClass) -> Stmt {
    Stmt {
        text,
        txn,
        delimiter: Delim::Semicolon,
    }
}

/// Server text without trailing whitespace and one trailing `;` (the script adds its own
/// terminator). Nothing else in it changes.
pub(crate) fn trim_server_text(text: &str) -> &str {
    let t = text.trim_end();
    t.strip_suffix(';').map_or(t, str::trim_end)
}

pub(crate) fn fk_action(a: FkAction) -> Option<&'static str> {
    match a {
        FkAction::NoAction => None,
        FkAction::Restrict => Some("RESTRICT"),
        FkAction::Cascade => Some("CASCADE"),
        FkAction::SetNull => Some("SET NULL"),
        FkAction::SetDefault => Some("SET DEFAULT"),
    }
}

/// ` MATCH FULL` / ` ON DELETE …` / ` ON UPDATE …` in the server's order.
pub(crate) fn fk_tail(match_kind: FkMatch, on_update: FkAction, on_delete: FkAction) -> String {
    let mut s = String::new();
    match match_kind {
        FkMatch::Simple => {}
        FkMatch::Full => s.push_str(" MATCH FULL"),
        FkMatch::Partial => s.push_str(" MATCH PARTIAL"),
    }
    if let Some(a) = fk_action(on_update) {
        s.push_str(" ON UPDATE ");
        s.push_str(a);
    }
    if let Some(a) = fk_action(on_delete) {
        s.push_str(" ON DELETE ");
        s.push_str(a);
    }
    s
}

pub(crate) fn deferrable_tail(d: Deferrable) -> &'static str {
    match d {
        Deferrable::No => "",
        Deferrable::InitiallyImmediate => " DEFERRABLE",
        Deferrable::InitiallyDeferred => " DEFERRABLE INITIALLY DEFERRED",
    }
}

/// One index key part: a column (with a MySQL prefix length) or a parenthesized expression,
/// then direction and null ordering.
pub(crate) fn index_part(ctx: &RenderCtx<'_>, p: &IndexPartDef) -> Result<String, RenderError> {
    let mut s = match (&p.column, &p.expr) {
        (Some(c), _) => {
            let mut s = ctx.ident(c);
            if let Some(n) = p.prefix_len {
                s.push_str(&format!("({n})"));
            }
            s
        }
        (None, Some(e)) => format!("({e})"),
        (None, None) => return Err(RenderError::Invalid("index part without column or expr")),
    };
    if p.desc {
        s.push_str(" DESC");
    }
    match p.nulls_first {
        Some(true) => s.push_str(" NULLS FIRST"),
        Some(false) => s.push_str(" NULLS LAST"),
        None => {}
    }
    Ok(s)
}

/// `name(sig)` of a routine.
pub(crate) fn routine_ref(ctx: &RenderCtx<'_>, name: &QName, sig: Option<&Name>) -> String {
    format!("{}({})", ctx.qname(name), sig.map_or("", |s| s))
}
