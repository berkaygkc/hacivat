//! Renderers (HV-DDL-07 "two layers"): one per dialect turns the IR into statement text.
//! Unsupported `(dialect, op)` combinations answer `RenderError::Unsupported` explicitly,
//! never a "cannot generate" comment. M4 Task 7 bodies (spike origin `da358e4`).
//!
//! Output rules (plan-m4 Task 7 step 3): never from Fluent and never from the process locale;
//! `\n` line endings only; four-space indentation; statement text without its terminator
//! ([`render_text`] and the script add `;` and one `\n`). Identifiers go through
//! `sql_ident::quote` only, literals through `sql_ident::quote::string_literal`; expressions,
//! types, defaults and server text are emitted verbatim.
//!
//! M4 renders the view-only variants: `CreateSchema`, `CreateTable`, `CreateIndex`,
//! `CreateView`, `CreateRoutine` and `CreateTrigger` (server text), `CreateSequence`,
//! `CreateType`, `Comment`, `Grant`, `Owner`, `RowSecurity`, `Raw { origin: ServerText }`, and
//! `AlterTable` whose actions are all `AddConstraint` (a constraint postponed past later
//! tables, HV-DDL-09). Every other variant, and every variant a dialect cannot express, is
//! `Unsupported` (M7 fills them).

pub mod mysql;
pub mod pg;
pub mod sqlite;

use std::fmt;

use hacivat_meta::name::Name;
use hacivat_meta::props::{FkAction, FkMatch};
use sql_dialect::DialectId;
use sql_ident::QualifiedName;
use sql_ident::quote::{quote, string_literal};

use crate::DdlTarget;
use crate::ir::{ConstraintDef, DdlOp, Deferrable, IndexPartDef};
use crate::options::GenOptions;
use crate::produce::placement::{PlacePlan, Placed, place};

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

    pub fn dialect(&self) -> DialectId {
        self.target.dialect
    }

    /// An identifier, quoted where the dialect needs it.
    pub(crate) fn ident(&self, name: &str) -> Result<String, RenderError> {
        quote(self.target.dialect, name, &self.target.quote).map_err(RenderError::Quote)
    }

    /// Every part of `q`, quoted, joined by `.`: the producer chose the parts (HV-DDL-10).
    pub(crate) fn qname(&self, q: &QualifiedName) -> Result<String, RenderError> {
        if q.parts.is_empty() {
            return Err(RenderError::Invalid("a name without parts"));
        }
        let mut out = String::new();
        for (i, p) in q.parts.iter().enumerate() {
            if i > 0 {
                out.push('.');
            }
            out.push_str(&self.ident(&p.text)?);
        }
        Ok(out)
    }

    /// The last part of `q` (the object's own name), quoted.
    pub(crate) fn last(&self, q: &QualifiedName) -> Result<String, RenderError> {
        let p = q
            .parts
            .last()
            .ok_or(RenderError::Invalid("a name without parts"))?;
        self.ident(&p.text)
    }

    /// `(a, b, c)` of identifiers.
    pub(crate) fn ident_list(&self, names: &[Name]) -> Result<String, RenderError> {
        let parts: Result<Vec<String>, _> = names.iter().map(|n| self.ident(n)).collect();
        Ok(format!("({})", parts?.join(", ")))
    }

    /// A string literal under the target's settings; a NUL is refused.
    pub(crate) fn literal(&self, text: &str) -> Result<String, RenderError> {
        string_literal(self.target.dialect, text, &self.target.quote).map_err(RenderError::Quote)
    }

    /// The table's constraints placed under the requested mode (HV-DDL-09). Constraints that
    /// must follow later tables are the producer's `AlterTable` ops, not the table's.
    pub(crate) fn place(
        &self,
        constraints: &[ConstraintDef],
        auto_column: Option<&Name>,
    ) -> Vec<Placed> {
        place(
            constraints,
            self.opts.placement,
            self.target.dialect,
            &PlacePlan {
                later: &[],
                auto_column,
            },
        )
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
    /// A name or literal that cannot be quoted (it holds a NUL).
    Quote(sql_ident::quote::QuoteError),
    /// The op is malformed for this dialect (an index part without a column or expression,
    /// `AUTOINCREMENT` without its primary key, a routine named without its argument types).
    Invalid(&'static str),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::Unsupported { dialect, op } => {
                write!(f, "{op} is not supported for {dialect:?}")
            }
            RenderError::Quote(e) => write!(f, "cannot quote a name: {e:?}"),
            RenderError::Invalid(why) => write!(f, "invalid op: {why}"),
        }
    }
}

impl std::error::Error for RenderError {}

/// A dialect's renderer.
pub trait Renderer: Send + Sync {
    fn render(&self, op: &DdlOp, ctx: &RenderCtx) -> Result<Vec<Stmt>, RenderError>;
}

/// The renderer of a dialect M4 has no renderer for (SQL Server, ClickHouse, Oracle, generic,
/// the PG-wire forks).
#[derive(Debug, Clone, Copy)]
struct NoRenderer;

impl Renderer for NoRenderer {
    fn render(&self, op: &DdlOp, ctx: &RenderCtx) -> Result<Vec<Stmt>, RenderError> {
        Err(unsupported(op, ctx))
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

/// Renders `ops` with `ctx`'s dialect and joins the statements: each ends with `;` and `\n`,
/// statements are separated by one empty line. The display form of IR-level output.
pub fn render_text(ops: &[DdlOp], ctx: &RenderCtx) -> Result<String, RenderError> {
    let r = renderer(ctx.dialect());
    let mut stmts = Vec::new();
    for op in ops {
        stmts.extend(r.render(op, ctx)?);
    }
    Ok(join_semicolon(&stmts))
}

/// Statements as text, each ended by `;` and `\n`, separated by one empty line. Byte-stable:
/// no locale, no clock, no hash order.
pub fn join_semicolon(stmts: &[Stmt]) -> String {
    let mut out = String::new();
    for (i, s) in stmts.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&s.text);
        out.push_str(terminator(&s.text));
    }
    out
}

/// The `;` that ends `text`: on a line of its own when the last line may end in a line
/// comment (`--`, or MySQL `#`), which would otherwise swallow it. The check is conservative
/// (a `--` or `#` inside a quoted string also moves it); that costs only a line break.
pub fn terminator(text: &str) -> &'static str {
    let last = text.rsplit('\n').next().unwrap_or("");
    if last.contains("--") || last.contains('#') {
        "\n;\n"
    } else {
        ";\n"
    }
}

pub(crate) fn unsupported(op: &DdlOp, ctx: &RenderCtx) -> RenderError {
    RenderError::Unsupported {
        dialect: ctx.dialect(),
        op: op.variant(),
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

/// `(a, b)` of table elements, one per line and indented; `()` for none.
pub(crate) fn element_block(items: &[String]) -> String {
    if items.is_empty() {
        return "()".to_string();
    }
    let mut s = String::from("(\n");
    for (i, e) in items.iter().enumerate() {
        s.push_str("    ");
        s.push_str(e);
        if i + 1 < items.len() {
            s.push(',');
        }
        s.push('\n');
    }
    s.push(')');
    s
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

pub(crate) fn fk_match(m: FkMatch) -> &'static str {
    match m {
        FkMatch::Simple => "",
        FkMatch::Full => " MATCH FULL",
        FkMatch::Partial => " MATCH PARTIAL",
    }
}

pub(crate) fn deferrable_tail(d: Deferrable) -> &'static str {
    match d {
        Deferrable::No => "",
        Deferrable::InitiallyImmediate => " DEFERRABLE",
        Deferrable::InitiallyDeferred => " DEFERRABLE INITIALLY DEFERRED",
    }
}

/// One index key part: a column (with a MySQL prefix length) or a parenthesized expression,
/// then its direction and (PostgreSQL) null ordering.
pub(crate) fn index_part(ctx: &RenderCtx, p: &IndexPartDef) -> Result<String, RenderError> {
    let d = ctx.dialect();
    let mysql = matches!(d, DialectId::MySql | DialectId::MariaDb);
    let mut s = match (&p.column, &p.expr) {
        (Some(c), None) => ctx.ident(c)?,
        (None, Some(e)) => format!("({e})"),
        _ => {
            return Err(RenderError::Invalid(
                "an index part needs exactly one of a column and an expression",
            ));
        }
    };
    match p.prefix_len {
        Some(n) if mysql && p.column.is_some() => s.push_str(&format!("({n})")),
        Some(_) => return Err(RenderError::Invalid("a prefix length outside MySQL")),
        None => {}
    }
    if p.desc {
        s.push_str(" DESC");
    }
    match p.nulls_first {
        Some(_) if d != DialectId::Postgres => {
            return Err(RenderError::Invalid("NULLS FIRST/LAST outside PostgreSQL"));
        }
        // Only the ordering that differs from the default for the direction is written
        // (ascending: NULLS LAST, descending: NULLS FIRST).
        Some(true) if !p.desc => s.push_str(" NULLS FIRST"),
        Some(false) if p.desc => s.push_str(" NULLS LAST"),
        _ => {}
    }
    Ok(s)
}

/// `CREATE`'s optional `IF NOT EXISTS` and `OR REPLACE` per the op and the options, each only
/// where the dialect has it for `kind` (`OR REPLACE` wins where both apply, HV-DDL-11 step 3).
pub(crate) fn create_flags(
    ctx: &RenderCtx,
    kind: hacivat_meta::kind::ObjectKind,
    op_or_replace: bool,
    op_if_not_exists: bool,
) -> (bool, bool) {
    use crate::options::{if_not_exists_supported, or_replace_supported};
    let or_replace =
        (op_or_replace || ctx.opts.or_replace) && or_replace_supported(&ctx.target, kind);
    let if_not_exists = !or_replace
        && (op_if_not_exists || ctx.opts.if_not_exists)
        && if_not_exists_supported(&ctx.target, kind);
    (or_replace, if_not_exists)
}

/// A privilege as a grant writes it: upper-case ASCII words (`SELECT`, `SHOW VIEW`), never
/// free text.
pub(crate) fn privilege(p: &str) -> Result<&str, RenderError> {
    let ok = !p.is_empty()
        && p.bytes()
            .all(|b| b.is_ascii_uppercase() || b == b' ' || b == b'_')
        && !p.starts_with(' ')
        && !p.ends_with(' ');
    if ok {
        Ok(p)
    } else {
        Err(RenderError::Invalid("a privilege that is not a keyword"))
    }
}

/// `WITH LOCAL|CASCADED CHECK OPTION` takes one of two keywords, never free text.
pub(crate) fn check_option(opt: &str) -> Result<&'static str, RenderError> {
    ["LOCAL", "CASCADED"]
        .into_iter()
        .find(|o| opt.eq_ignore_ascii_case(o))
        .ok_or(RenderError::Invalid(
            "a check option that is not LOCAL or CASCADED",
        ))
}
