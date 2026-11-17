//! The SQLite renderer (the bundled library).
//!
//! SQLite specifics (alg/03 HV-DDL-07, HV-DDL-09): constraints live inside the table;
//! `AUTOINCREMENT` exists only as `INTEGER PRIMARY KEY AUTOINCREMENT` in its column; comments
//! do not exist and are never written; a schema qualifies the *created* name of an index or
//! trigger (`CREATE INDEX aux.i ON t`), never the table after `ON`, and a foreign key names
//! its parent table bare (both must be in the same schema).

use super::{
    Delim, RenderCtx, Renderer, Stmt, TxnClass, deferrable_tail, fk_tail, index_part, stmt,
    trim_server_text, unsupported,
};
use crate::error::RenderError;
use crate::ir::*;

pub struct SqliteRenderer;

const TX: TxnClass = TxnClass::Transactional;

impl Renderer for SqliteRenderer {
    fn render(&self, op: &DdlOp, ctx: &RenderCtx<'_>) -> Result<Vec<Stmt>, RenderError> {
        match op {
            DdlOp::CreateTable(t) => {
                let mut out = vec![stmt(table(ctx, t)?, TX)];
                for i in &t.indexes {
                    out.push(stmt(index(ctx, i)?, TX));
                }
                Ok(out)
            }
            DdlOp::CreateIndex(i) => Ok(vec![stmt(index(ctx, i)?, TX)]),
            DdlOp::CreateView(v) => {
                let text = match &v.body {
                    ViewBody::ServerText(t) => trim_server_text(t).to_string(),
                    ViewBody::Query(q) => {
                        format!(
                            "CREATE VIEW {} AS {}",
                            ctx.qname(&v.name),
                            trim_server_text(q)
                        )
                    }
                };
                if v.kind == ViewKind::Materialized {
                    return Err(unsupported(op, ctx.dialect()));
                }
                Ok(vec![stmt(text, TX)])
            }
            DdlOp::CreateTrigger(t) => Ok(vec![server_text(&t.text)]),
            DdlOp::Raw {
                sql,
                origin: RawOrigin::ServerText,
            } => Ok(vec![server_text(sql)]),
            _ => Err(unsupported(op, ctx.dialect())),
        }
    }
}

fn server_text(text: &str) -> Stmt {
    let t = trim_server_text(text).to_string();
    let delimiter = if t.contains(';') {
        Delim::Block
    } else {
        Delim::Semicolon
    };
    Stmt {
        text: t,
        txn: TX,
        delimiter,
    }
}

fn constraint(
    ctx: &RenderCtx<'_>,
    k: &ConstraintDef,
    inline: bool,
    autoincrement: bool,
) -> Result<String, RenderError> {
    let mut s = String::new();
    if let Some(n) = &k.name {
        s.push_str("CONSTRAINT ");
        s.push_str(&ctx.ident(n));
        s.push(' ');
    }
    match &k.body {
        ConstraintBody::PrimaryKey { columns } => {
            s.push_str("PRIMARY KEY");
            if inline {
                if autoincrement {
                    s.push_str(" AUTOINCREMENT");
                }
            } else {
                s.push(' ');
                s.push_str(&ctx.ident_list(columns));
            }
        }
        ConstraintBody::Unique { columns } => {
            s.push_str("UNIQUE");
            if !inline {
                s.push(' ');
                s.push_str(&ctx.ident_list(columns));
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
                s.push_str(&ctx.ident_list(columns));
                s.push(' ');
            }
            // The parent table is always in the child's schema: never qualified.
            s.push_str("REFERENCES ");
            s.push_str(&ctx.ident(&target.name));
            if !target_columns.is_empty() {
                s.push(' ');
                s.push_str(&ctx.ident_list(target_columns));
            }
            s.push_str(&fk_tail(*match_kind, *on_update, *on_delete));
            s.push_str(deferrable_tail(k.deferrable));
        }
        ConstraintBody::Check { expr } => s.push_str(&format!("CHECK ({expr})")),
        ConstraintBody::Exclusion { .. } => return Err(RenderError::Invalid("EXCLUDE on SQLite")),
    }
    Ok(s)
}

fn table(ctx: &RenderCtx<'_>, t: &TableDef) -> Result<String, RenderError> {
    let ext = match &t.ext {
        TableDefExt::Sqlite(x) => x.clone(),
        TableDefExt::None => SqliteTableDef::default(),
        _ => return Err(RenderError::Invalid("non-SQLite table clauses")),
    };
    let mut items = Vec::new();
    for c in &t.columns {
        let mut s = ctx.ident(&c.name);
        if !c.type_text.is_empty() {
            s.push(' ');
            s.push_str(&c.type_text);
        }
        let auto = c.identity == Some(IdentityDef::AutoIncrement);
        if matches!(
            c.identity,
            Some(IdentityDef::Always | IdentityDef::ByDefault)
        ) {
            return Err(RenderError::Invalid("identity clause on SQLite"));
        }
        let inline: Vec<&ConstraintDef> = t
            .constraints
            .iter()
            .filter(|k| matches!(&k.position, Position::InColumn(n) if *n == c.name))
            .collect();
        if auto
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
            s.push_str(&ctx.ident(coll));
        }
        if let Some(d) = &c.default {
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
            s.push_str(&constraint(ctx, k, true, auto)?);
        }
        items.push(s);
    }
    for k in &t.constraints {
        if k.position == Position::InTable {
            items.push(constraint(ctx, k, false, false)?);
        }
    }
    let ine = if t.if_not_exists {
        "IF NOT EXISTS "
    } else {
        ""
    };
    let mut s = format!("CREATE TABLE {ine}{} (\n", ctx.qname(&t.name));
    for (n, e) in items.iter().enumerate() {
        s.push_str("    ");
        s.push_str(e);
        if n + 1 < items.len() {
            s.push(',');
        }
        s.push('\n');
    }
    s.push(')');
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

fn index(ctx: &RenderCtx<'_>, i: &IndexDef) -> Result<String, RenderError> {
    let mut s = String::from("CREATE ");
    if i.unique {
        s.push_str("UNIQUE ");
    }
    s.push_str("INDEX ");
    if i.if_not_exists {
        s.push_str("IF NOT EXISTS ");
    }
    // The schema qualifies the index name; the table after `ON` is always bare.
    let name = QName {
        schema: i.table.schema.clone(),
        name: i.name.clone(),
    };
    s.push_str(&ctx.qname(&name));
    s.push_str(" ON ");
    s.push_str(&ctx.ident(&i.table.name));
    let parts: Result<Vec<String>, _> = i.parts.iter().map(|p| index_part(ctx, p)).collect();
    s.push_str(&format!(" ({})", parts?.join(", ")));
    if let Some(p) = &i.predicate {
        s.push_str(" WHERE ");
        s.push_str(p);
    }
    Ok(s)
}
