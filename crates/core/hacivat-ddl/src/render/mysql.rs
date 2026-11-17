//! The MySQL (8.0, 8.4) and MariaDB (10.6, 10.11, 11.4) renderer.
//!
//! MySQL specifics (alg/03 HV-DDL-07 matrix): comments inline (`COMMENT '…'`, escaped per
//! `NO_BACKSLASH_ESCAPES`); foreign keys always table elements (InnoDB parses an inline
//! `REFERENCES` and ignores it); secondary indexes are table elements (`KEY …`), as
//! `SHOW CREATE TABLE` prints them, so a foreign key finds its index when it is created and
//! MySQL does not add one of its own; every DDL statement commits implicitly.

use hacivat_meta::ObjectKind;
use sql_dialect::DialectId;

use super::{
    Delim, RenderCtx, Renderer, Stmt, TxnClass, fk_tail, index_part, stmt, trim_server_text,
    unsupported,
};
use crate::error::RenderError;
use crate::ir::*;

pub struct MySqlRenderer;

const IC: TxnClass = TxnClass::ImplicitCommit;

impl Renderer for MySqlRenderer {
    fn render(&self, op: &DdlOp, ctx: &RenderCtx<'_>) -> Result<Vec<Stmt>, RenderError> {
        match op {
            DdlOp::CreateSchema(s) => {
                let ine = if s.if_not_exists {
                    "IF NOT EXISTS "
                } else {
                    ""
                };
                let mut text = format!("CREATE DATABASE {ine}{}", ctx.ident(&s.name));
                if let Some(c) = &s.comment {
                    // MariaDB has database comments (10.5+); MySQL does not.
                    if ctx.dialect() != DialectId::MariaDb {
                        return Err(RenderError::Unsupported {
                            op: "database comment",
                            dialect: ctx.dialect(),
                        });
                    }
                    text.push_str(&format!(" COMMENT {}", ctx.literal(c, "comment")?));
                }
                Ok(vec![stmt(text, IC)])
            }
            DdlOp::CreateTable(t) => Ok(vec![stmt(table(ctx, t)?, IC)]),
            DdlOp::AlterTable { table, actions } => {
                let mut out = Vec::new();
                for a in actions {
                    match a {
                        AlterAction::AddConstraint(k) => out.push(stmt(
                            format!(
                                "ALTER TABLE {} ADD {}",
                                ctx.qname(table),
                                constraint(ctx, k)?
                            ),
                            IC,
                        )),
                        _ => return Err(unsupported(op, ctx.dialect())),
                    }
                }
                Ok(out)
            }
            DdlOp::CreateIndex(i) => Ok(vec![stmt(create_index(ctx, i)?, IC)]),
            DdlOp::CreateView(v) => {
                let text = match &v.body {
                    ViewBody::ServerText(t) => trim_server_text(t).to_string(),
                    ViewBody::Query(q) => format!(
                        "CREATE {}VIEW {} AS {}",
                        if v.or_replace { "OR REPLACE " } else { "" },
                        ctx.qname(&v.name),
                        trim_server_text(q)
                    ),
                };
                if v.kind == ViewKind::Materialized {
                    return Err(unsupported(op, ctx.dialect()));
                }
                Ok(vec![stmt(text, IC)])
            }
            DdlOp::CreateRoutine(r) => Ok(vec![block(&r.text)]),
            DdlOp::CreateTrigger(t) => Ok(vec![block(&t.text)]),
            DdlOp::CreateSequence(q) if ctx.dialect() == DialectId::MariaDb => {
                Ok(vec![stmt(mariadb_sequence(ctx, q), IC)])
            }
            DdlOp::Grant(g) => Ok(vec![stmt(grant(ctx, g)?, IC)]),
            DdlOp::Raw {
                sql,
                origin: RawOrigin::ServerText,
            } => Ok(vec![block(sql)]),
            _ => Err(unsupported(op, ctx.dialect())),
        }
    }
}

/// Server text as one statement; a `DELIMITER` is needed in file output when it holds `;`.
fn block(text: &str) -> Stmt {
    let t = trim_server_text(text).to_string();
    let delimiter = if t.contains(';') {
        Delim::Block
    } else {
        Delim::Semicolon
    };
    Stmt {
        text: t,
        txn: IC,
        delimiter,
    }
}

fn comment_clause(
    ctx: &RenderCtx<'_>,
    c: Option<&hacivat_meta::Name>,
) -> Result<String, RenderError> {
    Ok(match c {
        Some(c) => format!(" COMMENT {}", ctx.literal(c, "comment")?),
        None => String::new(),
    })
}

fn is_timestamp(ty: &str) -> bool {
    ty.len() >= 9 && ty.as_bytes()[..9].eq_ignore_ascii_case(b"timestamp")
}

fn column(
    ctx: &RenderCtx<'_>,
    c: &ColumnDef,
    inline: &[&ConstraintDef],
) -> Result<String, RenderError> {
    let mut s = format!("{} {}", ctx.ident(&c.name), c.type_text);
    if let Some(coll) = &c.collation {
        s.push_str(" COLLATE ");
        s.push_str(&ctx.ident(coll));
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
    } else if is_timestamp(&c.type_text) {
        // `explicit_defaults_for_timestamp = OFF` makes a bare TIMESTAMP NOT NULL; say NULL.
        s.push_str(" NULL");
    }
    if c.generated.is_none()
        && let Some(d) = &c.default
    {
        s.push_str(" DEFAULT ");
        s.push_str(d);
    }
    if let Some(u) = &c.on_update {
        s.push_str(" ON UPDATE ");
        s.push_str(u);
    }
    match c.identity {
        Some(IdentityDef::AutoIncrement) => s.push_str(" AUTO_INCREMENT"),
        Some(_) => return Err(RenderError::Invalid("identity clause on MySQL")),
        None => {}
    }
    for k in inline {
        match &k.body {
            ConstraintBody::PrimaryKey { .. } => s.push_str(" PRIMARY KEY"),
            ConstraintBody::Unique { .. } => s.push_str(" UNIQUE"),
            // Placement never puts these in a column on MySQL; a hand-built op is refused.
            _ => return Err(RenderError::Invalid("constraint inside a MySQL column")),
        }
    }
    s.push_str(&comment_clause(ctx, c.comment.as_ref())?);
    Ok(s)
}

/// A constraint as a table element (also the text after `ALTER TABLE … ADD`).
fn constraint(ctx: &RenderCtx<'_>, k: &ConstraintDef) -> Result<String, RenderError> {
    let named = |s: &mut String| {
        if let Some(n) = &k.name {
            s.push_str("CONSTRAINT ");
            s.push_str(&ctx.ident(n));
            s.push(' ');
        }
    };
    let mut s = String::new();
    match &k.body {
        // The primary key's name is always `PRIMARY`: never written.
        ConstraintBody::PrimaryKey { columns } => {
            s.push_str("PRIMARY KEY ");
            s.push_str(&ctx.ident_list(columns));
        }
        ConstraintBody::Unique { columns } => {
            s.push_str("UNIQUE KEY ");
            if let Some(n) = &k.name {
                s.push_str(&ctx.ident(n));
                s.push(' ');
            }
            s.push_str(&ctx.ident_list(columns));
        }
        ConstraintBody::ForeignKey {
            columns,
            target,
            target_columns,
            on_update,
            on_delete,
            match_kind,
        } => {
            named(&mut s);
            s.push_str("FOREIGN KEY ");
            s.push_str(&ctx.ident_list(columns));
            s.push_str(" REFERENCES ");
            s.push_str(&ctx.qname(target));
            s.push(' ');
            s.push_str(&ctx.ident_list(target_columns));
            s.push_str(&fk_tail(*match_kind, *on_update, *on_delete));
        }
        ConstraintBody::Check { expr } => {
            named(&mut s);
            s.push_str(&format!("CHECK ({expr})"));
        }
        ConstraintBody::Exclusion { .. } => {
            return Err(RenderError::Invalid("EXCLUDE on MySQL"));
        }
    }
    Ok(s)
}

/// `FULLTEXT` / `SPATIAL` (index kinds) or `USING HASH|BTREE` (methods); BTREE is the
/// default and not written.
fn index_kind(method: Option<&str>) -> (&'static str, Option<&str>) {
    match method {
        Some(m) if m.eq_ignore_ascii_case("fulltext") => ("FULLTEXT ", None),
        Some(m) if m.eq_ignore_ascii_case("spatial") => ("SPATIAL ", None),
        Some(m) if m.eq_ignore_ascii_case("btree") => ("", None),
        Some(m) => ("", Some(m)),
        None => ("", None),
    }
}

fn index_tail(ctx: &RenderCtx<'_>, i: &IndexDef) -> Result<String, RenderError> {
    let parts: Result<Vec<String>, _> = i.parts.iter().map(|p| index_part(ctx, p)).collect();
    let mut s = format!("({})", parts?.join(", "));
    if let (_, Some(m)) = index_kind(i.method.as_deref()) {
        s.push_str(" USING ");
        s.push_str(m);
    }
    s.push_str(&comment_clause(ctx, i.comment.as_ref())?);
    Ok(s)
}

/// An index as a table element: `[UNIQUE|FULLTEXT|SPATIAL] KEY name (…)`.
fn key_element(ctx: &RenderCtx<'_>, i: &IndexDef) -> Result<String, RenderError> {
    let (kind, _) = index_kind(i.method.as_deref());
    let unique = if i.unique { "UNIQUE " } else { "" };
    Ok(format!(
        "{unique}{kind}KEY {} {}",
        ctx.ident(&i.name),
        index_tail(ctx, i)?
    ))
}

fn create_index(ctx: &RenderCtx<'_>, i: &IndexDef) -> Result<String, RenderError> {
    let (kind, _) = index_kind(i.method.as_deref());
    let unique = if i.unique { "UNIQUE " } else { "" };
    let ine = if i.if_not_exists && ctx.dialect() == DialectId::MariaDb {
        "IF NOT EXISTS "
    } else {
        ""
    };
    Ok(format!(
        "CREATE {unique}{kind}INDEX {ine}{} ON {} {}",
        ctx.ident(&i.name),
        ctx.qname(&i.table),
        index_tail(ctx, i)?
    ))
}

fn table(ctx: &RenderCtx<'_>, t: &TableDef) -> Result<String, RenderError> {
    let ext = match &t.ext {
        TableDefExt::MySql(m) => m.clone(),
        TableDefExt::None => MySqlTableDef::default(),
        _ => return Err(RenderError::Invalid("non-MySQL table clauses")),
    };
    let mut items = Vec::new();
    for c in &t.columns {
        let inline: Vec<&ConstraintDef> = t
            .constraints
            .iter()
            .filter(|k| matches!(&k.position, Position::InColumn(n) if *n == c.name))
            .filter(|k| {
                matches!(
                    k.body,
                    ConstraintBody::PrimaryKey { .. } | ConstraintBody::Unique { .. }
                )
            })
            .collect();
        items.push(column(ctx, c, &inline)?);
    }
    // `SHOW CREATE TABLE` order: primary key, unique keys, keys, foreign keys, checks. A
    // foreign key or check a hand-built op put in a column is written here instead.
    let in_table = |k: &&ConstraintDef| {
        k.position == Position::InTable
            || matches!(
                k.body,
                ConstraintBody::ForeignKey { .. } | ConstraintBody::Check { .. }
            )
    };
    let rank = |k: &ConstraintDef| match k.body {
        ConstraintBody::PrimaryKey { .. } => 0,
        ConstraintBody::Unique { .. } => 1,
        ConstraintBody::ForeignKey { .. } => 3,
        _ => 4,
    };
    let mut keys: Vec<&ConstraintDef> = t.constraints.iter().filter(in_table).collect();
    keys.sort_by_key(|k| rank(k));
    for k in keys.iter().filter(|k| rank(k) < 3) {
        items.push(constraint(ctx, k)?);
    }
    for i in &t.indexes {
        items.push(key_element(ctx, i)?);
    }
    for k in keys.iter().filter(|k| rank(k) >= 3) {
        items.push(constraint(ctx, k)?);
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
    if let Some(e) = &ext.engine {
        s.push_str(" ENGINE=");
        s.push_str(e);
    }
    if let Some(c) = &ext.collation {
        s.push_str(" COLLATE=");
        s.push_str(c);
    }
    if let Some(r) = &ext.row_format {
        s.push_str(" ROW_FORMAT=");
        s.push_str(r);
    }
    if let Some(c) = &t.comment {
        s.push_str(&format!(" COMMENT={}", ctx.literal(c, "comment")?));
    }
    Ok(s)
}

/// MariaDB `CREATE SEQUENCE` with the clauses that differ from MariaDB's defaults
/// (increment 1, min 1 and max 9223372036854775806 ascending, start at min, cache 1000,
/// no cycle).
fn mariadb_sequence(ctx: &RenderCtx<'_>, q: &SeqDef) -> String {
    let inc = q.increment.unwrap_or(1);
    let asc = inc > 0;
    let (dmin, dmax) = if asc {
        (1, i64::MAX - 1)
    } else {
        (i64::MIN + 1, -1)
    };
    let min = q.min.unwrap_or(dmin);
    let max = q.max.unwrap_or(dmax);
    let dstart = if asc { min } else { max };
    let mut s = format!(
        "CREATE SEQUENCE {}{}",
        if q.if_not_exists {
            "IF NOT EXISTS "
        } else {
            ""
        },
        ctx.qname(&q.name)
    );
    let mut push = |c: String| {
        s.push_str("\n    ");
        s.push_str(&c);
    };
    if inc != 1 {
        push(format!("INCREMENT BY {inc}"));
    }
    if min != dmin {
        push(format!("MINVALUE {min}"));
    }
    if max != dmax {
        push(format!("MAXVALUE {max}"));
    }
    if let Some(st) = q.start
        && st != dstart
    {
        push(format!("START WITH {st}"));
    }
    if let Some(c) = q.cache
        && c != 1000
    {
        push(format!("CACHE {c}"));
    }
    if q.cycle {
        push("CYCLE".to_string());
    }
    s
}

/// Whether `g` is a MySQL account as the server prints it: `'user'@'host'`, each part a
/// single-quoted literal with `''` for a quote.
fn is_account(g: &str) -> bool {
    fn literal(s: &str) -> Option<&str> {
        let mut it = s.strip_prefix('\'')?.char_indices().peekable();
        while let Some((i, c)) = it.next() {
            if c == '\'' {
                if it.peek().map(|&(_, n)| n) == Some('\'') {
                    it.next();
                    continue;
                }
                return Some(&s[i + 2..]);
            }
        }
        None
    }
    literal(g)
        .and_then(|rest| rest.strip_prefix('@'))
        .and_then(literal)
        .is_some_and(str::is_empty)
}

fn grant(ctx: &RenderCtx<'_>, g: &GrantDef) -> Result<String, RenderError> {
    if g.privileges.is_empty() {
        return Err(RenderError::Invalid("grant without privileges"));
    }
    if !is_account(&g.grantee) {
        return Err(RenderError::Invalid("MySQL grantee is not 'user'@'host'"));
    }
    let on = match g.kind {
        ObjectKind::Schema => format!("{}.*", ctx.ident(&g.object.name)),
        ObjectKind::Routine => format!("FUNCTION {}", ctx.qname(&g.object)),
        ObjectKind::Procedure => format!("PROCEDURE {}", ctx.qname(&g.object)),
        ObjectKind::Table | ObjectKind::View => ctx.qname(&g.object),
        _ => return Err(RenderError::Invalid("MySQL grant on this kind")),
    };
    let cols = if g.columns.is_empty() {
        String::new()
    } else {
        format!(" {}", ctx.ident_list(&g.columns))
    };
    let privs: Vec<String> = g.privileges.iter().map(|p| format!("{p}{cols}")).collect();
    let mut s = format!("GRANT {} ON {on} TO {}", privs.join(", "), g.grantee);
    if g.with_grant_option {
        s.push_str(" WITH GRANT OPTION");
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::is_account;

    #[test]
    fn accounts_are_two_quoted_literals() {
        assert!(is_account("'app'@'%'"));
        assert!(is_account("'o''brien'@'localhost'"));
        assert!(!is_account("app@%"));
        assert!(!is_account("'app'@'%' ; DROP TABLE t"));
        assert!(!is_account("'app'@"));
    }
}
