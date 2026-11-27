//! The SQLite renderer (the bundled library). M4 Task 7 body (spike origin `da358e4`).
//!
//! SQLite specifics (alg/03 HV-DDL-07, HV-DDL-09): constraints live inside the table (there
//! is no `ADD CONSTRAINT`); `AUTOINCREMENT` exists only as `INTEGER PRIMARY KEY AUTOINCREMENT`
//! in its column; comments do not exist and are never written (they stay Hacivat
//! annotations); a schema qualifies the *created* name of an index (`CREATE INDEX aux.i ON t`),
//! never the table after `ON`, and a foreign key names its parent table bare (both live in the
//! same schema); there are no schemas to create, no sequences, types, grants or owners.

use hacivat_meta::kind::ObjectKind;
use hacivat_meta::props::Identity;

use super::{
    RenderCtx, RenderError, Renderer, Stmt, TxnClass, create_flags, deferrable_tail, element_block,
    fk_action, fk_match, index_part, stmt, trim_server_text, unsupported,
};
use crate::ir::*;
use crate::produce::placement::Position;

/// Renders the IR as SQLite text.
#[derive(Debug, Clone, Copy)]
pub struct SqliteRenderer;

const TX: TxnClass = TxnClass::Transactional;

impl Renderer for SqliteRenderer {
    fn render(&self, op: &DdlOp, ctx: &RenderCtx) -> Result<Vec<Stmt>, RenderError> {
        match op {
            DdlOp::CreateTable(t) => Ok(vec![stmt(table(ctx, t)?, TX)]),
            DdlOp::CreateIndex(i) => Ok(vec![stmt(index(ctx, i)?, TX)]),
            DdlOp::CreateView(v) => {
                if v.kind == ViewKind::Materialized || v.check_option.is_some() {
                    return Err(unsupported(op, ctx));
                }
                Ok(vec![stmt(
                    format!(
                        "CREATE VIEW {} AS {}",
                        ctx.qname(&v.name)?,
                        trim_server_text(&v.body)
                    ),
                    TX,
                )])
            }
            DdlOp::CreateTrigger(t) => Ok(vec![stmt(trim_server_text(&t.text).to_string(), TX)]),
            DdlOp::Raw {
                sql,
                origin: RawOrigin::ServerText,
            } => Ok(vec![stmt(trim_server_text(sql).to_string(), TX)]),
            _ => Err(unsupported(op, ctx)),
        }
    }
}

fn constraint(
    ctx: &RenderCtx,
    k: &ConstraintDef,
    inline: bool,
    autoincrement: bool,
) -> Result<String, RenderError> {
    let mut s = String::new();
    if let Some(n) = &k.name {
        s.push_str("CONSTRAINT ");
        s.push_str(&ctx.ident(n)?);
        s.push(' ');
    }
    match &k.body {
        ConstraintBody::PrimaryKey { columns } => {
            s.push_str("PRIMARY KEY");
            if !inline {
                s.push(' ');
                s.push_str(&ctx.ident_list(columns)?);
            } else if autoincrement {
                s.push_str(" AUTOINCREMENT");
            }
        }
        ConstraintBody::Unique {
            columns,
            nulls_not_distinct,
        } => {
            if *nulls_not_distinct {
                return Err(RenderError::Invalid("NULLS NOT DISTINCT on SQLite"));
            }
            s.push_str("UNIQUE");
            if !inline {
                s.push(' ');
                s.push_str(&ctx.ident_list(columns)?);
            }
        }
        ConstraintBody::ForeignKey {
            columns,
            target,
            target_columns,
            on_update,
            on_delete,
            match_kind,
        } => {
            if !inline {
                s.push_str("FOREIGN KEY ");
                s.push_str(&ctx.ident_list(columns)?);
                s.push(' ');
            }
            // The parent table is always in the child's schema: never qualified.
            s.push_str("REFERENCES ");
            s.push_str(&ctx.last(target)?);
            if !target_columns.is_empty() {
                s.push(' ');
                s.push_str(&ctx.ident_list(target_columns)?);
            }
            if let Some(a) = fk_action(*on_delete) {
                s.push_str(" ON DELETE ");
                s.push_str(a);
            }
            if let Some(a) = fk_action(*on_update) {
                s.push_str(" ON UPDATE ");
                s.push_str(a);
            }
            s.push_str(fk_match(*match_kind));
            s.push_str(deferrable_tail(k.deferrable));
        }
        // ## m4-t13-assemble (R-W2-8)
        ConstraintBody::Check {
            no_inherit: true, ..
        } => return Err(RenderError::Invalid("NO INHERIT on SQLite")),
        ConstraintBody::Check { expr, .. } => s.push_str(&format!("CHECK ({expr})")),
        ConstraintBody::Exclusion { .. } => return Err(RenderError::Invalid("EXCLUDE on SQLite")),
    }
    Ok(s)
}

fn table(ctx: &RenderCtx, t: &TableDef) -> Result<String, RenderError> {
    let ext = match &t.ext {
        TableDefExt::Sqlite(x) => x.clone(),
        TableDefExt::None => SqliteTableDef::default(),
        _ => return Err(RenderError::Invalid("another dialect's table clauses")),
    };
    let auto = t
        .columns
        .iter()
        .find(|c| {
            matches!(
                c.identity,
                Some(Identity::AutoIncrement | Identity::RowidAlias)
            )
        })
        .map(|c| &c.name);
    let placed = ctx.place(&t.constraints, auto);
    let mut items = Vec::new();
    for c in &t.columns {
        // ## m4-t19-produce
        if c.invisible {
            return Err(RenderError::Invalid("an invisible column on SQLite"));
        }
        // ## end m4-t19-produce
        let mut s = ctx.ident(&c.name)?;
        if !c.raw_type.is_empty() {
            s.push(' ');
            s.push_str(&c.raw_type);
        }
        if matches!(c.identity, Some(Identity::Always | Identity::ByDefault)) {
            return Err(RenderError::Invalid("an identity clause on SQLite"));
        }
        let autoincrement = c.identity == Some(Identity::AutoIncrement);
        let inline: Vec<&ConstraintDef> = t
            .constraints
            .iter()
            .zip(&placed)
            .filter(|(_, p)| matches!(&p.position, Position::InColumn(n) if *n == c.name))
            .map(|(k, _)| k)
            .collect();
        if autoincrement
            && !inline
                .iter()
                .any(|k| matches!(k.body, ConstraintBody::PrimaryKey { .. }))
        {
            return Err(RenderError::Invalid(
                "AUTOINCREMENT without its INTEGER PRIMARY KEY",
            ));
        }
        if let Some(coll) = &c.collation {
            s.push_str(" COLLATE ");
            s.push_str(&ctx.ident(coll)?);
        }
        if let Some(d) = &c.default_raw {
            s.push_str(" DEFAULT ");
            s.push_str(d);
        }
        if let Some(g) = &c.generated {
            s.push_str(&format!(
                " GENERATED ALWAYS AS ({}) {}",
                g.expr,
                if g.stored { "STORED" } else { "VIRTUAL" }
            ));
        }
        if !c.nullable {
            s.push_str(" NOT NULL");
        }
        for k in inline {
            s.push(' ');
            s.push_str(&constraint(ctx, k, true, autoincrement)?);
        }
        items.push(s);
    }
    for (k, p) in t.constraints.iter().zip(&placed) {
        match p.position {
            Position::InTable => items.push(constraint(ctx, k, false, false)?),
            Position::InColumn(_) => {}
            // `place` never answers `After` on SQLite; refuse rather than drop a constraint.
            Position::After => {
                return Err(RenderError::Invalid("a constraint after a SQLite table"));
            }
        }
    }
    let (_, ine) = create_flags(ctx, ObjectKind::Table, false, t.if_not_exists);
    let mut s = format!(
        "CREATE TABLE {}{} {}",
        if ine { "IF NOT EXISTS " } else { "" },
        ctx.qname(&t.name)?,
        element_block(&items)
    );
    let mut opts = Vec::new();
    if ext.strict {
        opts.push("STRICT");
    }
    if ext.without_rowid {
        opts.push("WITHOUT ROWID");
    }
    if !opts.is_empty() {
        s.push(' ');
        s.push_str(&opts.join(", "));
    }
    Ok(s)
}

fn index(ctx: &RenderCtx, i: &IndexDef) -> Result<String, RenderError> {
    if !i.include.is_empty() || i.nulls_not_distinct || i.concurrently || i.method.is_some() {
        return Err(RenderError::Invalid(
            "a covering, NULLS NOT DISTINCT, concurrent or method index on SQLite",
        ));
    }
    let mut s = String::from("CREATE ");
    if i.unique {
        s.push_str("UNIQUE ");
    }
    s.push_str("INDEX ");
    let (_, ine) = create_flags(ctx, ObjectKind::Index, false, i.if_not_exists);
    if ine {
        s.push_str("IF NOT EXISTS ");
    }
    // The schema qualifies the index name; the table after `ON` is always bare.
    let mut name = i.table.clone();
    let last = name
        .parts
        .last_mut()
        .ok_or(RenderError::Invalid("a name without parts"))?;
    last.text = i.name.clone();
    s.push_str(&ctx.qname(&name)?);
    s.push_str(" ON ");
    s.push_str(&ctx.last(&i.table)?);
    let parts: Result<Vec<String>, _> = i.parts.iter().map(|p| index_part(ctx, p)).collect();
    s.push_str(&format!(" ({})", parts?.join(", ")));
    if let Some(p) = &i.predicate {
        s.push_str(" WHERE ");
        s.push_str(p);
    }
    Ok(s)
}
