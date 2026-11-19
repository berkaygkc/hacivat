//! The MySQL (8.0, 8.4) and MariaDB (10.6, 10.11, 11.4) renderer: one renderer, flavor-aware
//! (MariaDB-only clauses; F9 texts are emitted as each flavor reports them). M4 Task 7 body
//! (spike origin `da358e4`).
//!
//! MySQL specifics (alg/03 HV-DDL-07 matrix): comments inline (`COMMENT '…'`, escaped per
//! `NO_BACKSLASH_ESCAPES`); foreign keys always table elements (InnoDB parses an inline
//! `REFERENCES` and ignores it); column defaults and `ON UPDATE` texts verbatim, never
//! re-quoted (F9: MySQL `CURRENT_TIMESTAMP`, MariaDB `current_timestamp()`); every DDL
//! statement commits implicitly; routine and trigger bodies holding `;` get a client delimiter
//! in file output. Server text (`SHOW CREATE …`) is emitted verbatim except for the two head
//! edits an option asks for ([`edit_head`]).

use hacivat_meta::kind::ObjectKind;
use hacivat_meta::name::Name;
use hacivat_meta::props::Identity;

use super::{
    Delim, RenderCtx, RenderError, Renderer, Stmt, TxnClass, check_option, create_flags,
    element_block, fk_action, fk_match, index_part, privilege, stmt, trim_server_text, unsupported,
};
use crate::ir::*;
use crate::options::or_replace_supported;
use crate::produce::placement::Position;

/// Renders the IR as MySQL or MariaDB text.
#[derive(Debug, Clone, Copy)]
pub struct MySqlRenderer {
    pub mariadb: bool,
}

const IC: TxnClass = TxnClass::ImplicitCommit;

impl Renderer for MySqlRenderer {
    fn render(&self, op: &DdlOp, ctx: &RenderCtx) -> Result<Vec<Stmt>, RenderError> {
        match op {
            DdlOp::CreateSchema(s) => {
                let (_, ine) = create_flags(ctx, ObjectKind::Schema, false, s.if_not_exists);
                let ine = if ine { "IF NOT EXISTS " } else { "" };
                let mut text = format!("CREATE DATABASE {ine}{}", ctx.ident(&s.name)?);
                if let Some(c) = &s.comment {
                    // MariaDB has database comments (10.5+); MySQL does not.
                    if !self.mariadb {
                        return Err(unsupported(op, ctx));
                    }
                    text.push_str(&format!(" COMMENT {}", ctx.literal(c)?));
                }
                Ok(vec![stmt(text, IC)])
            }
            DdlOp::CreateTable(t) => table(ctx, t),
            DdlOp::AlterTable { table, actions } => {
                let mut out = Vec::new();
                for a in actions {
                    match a {
                        AlterAction::AddConstraint(k) => out.push(add_constraint(ctx, table, k)?),
                        _ => return Err(unsupported(op, ctx)),
                    }
                }
                Ok(out)
            }
            DdlOp::CreateIndex(i) => Ok(vec![stmt(create_index(ctx, i)?, IC)]),
            DdlOp::CreateView(v) => {
                if v.kind == ViewKind::Materialized {
                    return Err(unsupported(op, ctx));
                }
                let (or_replace, _) = create_flags(ctx, ObjectKind::View, v.or_replace, false);
                let mut text = format!(
                    "CREATE {}VIEW {} AS {}",
                    if or_replace { "OR REPLACE " } else { "" },
                    ctx.qname(&v.name)?,
                    trim_server_text(&v.body)
                );
                if let Some(opt) = &v.check_option {
                    text.push_str(&format!(" WITH {} CHECK OPTION", check_option(opt)?));
                }
                Ok(vec![stmt(text, IC)])
            }
            DdlOp::CreateRoutine(r) => Ok(vec![server_text(ctx, &r.text)]),
            DdlOp::CreateTrigger(t) => Ok(vec![server_text(ctx, &t.text)]),
            DdlOp::CreateSequence(q) if self.mariadb => {
                Ok(vec![stmt(mariadb_sequence(ctx, q)?, IC)])
            }
            DdlOp::Grant(g) => Ok(vec![stmt(grant(ctx, op, g)?, IC)]),
            DdlOp::Raw {
                sql,
                origin: RawOrigin::ServerText,
            } => Ok(vec![server_text(ctx, sql)]),
            _ => Err(unsupported(op, ctx)),
        }
    }
}

/// `SHOW CREATE …` text as one statement: the head edits the options ask for (where the head
/// has the expected shape, else unchanged), and a client delimiter when the text holds `;`.
fn server_text(ctx: &RenderCtx, text: &str) -> Stmt {
    let t = trim_server_text(text);
    let strip_definer = ctx.opts.skip_definer;
    let head = if strip_definer || ctx.opts.or_replace {
        parse_head(t)
    } else {
        None
    };
    let text = match head {
        Some(head) => {
            let or_replace = ctx.opts.or_replace
                && head
                    .kind
                    .is_some_and(|k| or_replace_supported(&ctx.target, k));
            apply_head(t, &head, strip_definer, or_replace)
        }
        None => t.to_string(),
    };
    let delimiter = if text.contains(';') {
        Delim::Custom(Name::from(delimiter_for(&text)))
    } else {
        Delim::Semicolon
    };
    Stmt {
        text,
        txn: IC,
        delimiter,
    }
}

/// The first client delimiter of a fixed list that `text` does not contain.
fn delimiter_for(text: &str) -> String {
    for d in [";;", "$$", "//", "@@", "%%"] {
        if !text.contains(d) {
            return d.to_string();
        }
    }
    let mut d = String::from("$$$");
    while text.contains(&d) {
        d.push('$');
    }
    d
}

fn comment_clause(ctx: &RenderCtx, c: Option<&Name>) -> Result<String, RenderError> {
    Ok(match c {
        Some(c) => format!(" COMMENT {}", ctx.literal(c)?),
        None => String::new(),
    })
}

fn is_timestamp(ty: &str) -> bool {
    ty.len() >= 9 && ty.as_bytes()[..9].eq_ignore_ascii_case(b"timestamp")
}

fn column(
    ctx: &RenderCtx,
    c: &ColumnDef,
    inline: &[&ConstraintDef],
) -> Result<String, RenderError> {
    let mut s = format!("{} {}", ctx.ident(&c.name)?, c.raw_type);
    if let Some(coll) = &c.collation {
        s.push_str(" COLLATE ");
        s.push_str(&ctx.ident(coll)?);
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
    } else if is_timestamp(&c.raw_type) {
        // Under `explicit_defaults_for_timestamp = OFF` a bare TIMESTAMP is NOT NULL: say NULL.
        s.push_str(" NULL");
    }
    if c.generated.is_none()
        && let Some(d) = &c.default_raw
    {
        s.push_str(" DEFAULT ");
        s.push_str(d);
    }
    if let Some(u) = &c.on_update {
        s.push_str(" ON UPDATE ");
        s.push_str(u);
    }
    match c.identity {
        Some(Identity::AutoIncrement) => s.push_str(" AUTO_INCREMENT"),
        Some(_) => return Err(RenderError::Invalid("an identity clause on MySQL")),
        None => {}
    }
    // ## m4-t19-produce: a column attribute, after the type and any generated clause (MySQL
    // 8.0.23+, MariaDB 10.3+).
    if c.invisible {
        s.push_str(" INVISIBLE");
    }
    // ## end m4-t19-produce
    for k in inline {
        match &k.body {
            ConstraintBody::PrimaryKey { .. } => s.push_str(" PRIMARY KEY"),
            ConstraintBody::Unique { .. } => s.push_str(" UNIQUE"),
            // Placement never puts these in a MySQL column.
            _ => return Err(RenderError::Invalid("a constraint inside a MySQL column")),
        }
    }
    s.push_str(&comment_clause(ctx, c.comment.as_ref())?);
    Ok(s)
}

/// A constraint as a table element (also the text after `ALTER TABLE … ADD`).
fn constraint(ctx: &RenderCtx, k: &ConstraintDef) -> Result<String, RenderError> {
    let mut s = String::new();
    let named = |s: &mut String| -> Result<(), RenderError> {
        if let Some(n) = &k.name {
            s.push_str("CONSTRAINT ");
            s.push_str(&ctx.ident(n)?);
            s.push(' ');
        }
        Ok(())
    };
    match &k.body {
        // The primary key's name is always `PRIMARY`: never written.
        ConstraintBody::PrimaryKey { columns } => {
            s.push_str("PRIMARY KEY ");
            s.push_str(&ctx.ident_list(columns)?);
        }
        ConstraintBody::Unique {
            columns,
            nulls_not_distinct,
        } => {
            if *nulls_not_distinct {
                return Err(RenderError::Invalid("NULLS NOT DISTINCT on MySQL"));
            }
            s.push_str("UNIQUE KEY ");
            if let Some(n) = &k.name {
                s.push_str(&ctx.ident(n)?);
                s.push(' ');
            }
            s.push_str(&ctx.ident_list(columns)?);
        }
        ConstraintBody::ForeignKey {
            columns,
            target,
            target_columns,
            on_update,
            on_delete,
            match_kind,
        } => {
            named(&mut s)?;
            s.push_str("FOREIGN KEY ");
            s.push_str(&ctx.ident_list(columns)?);
            s.push_str(" REFERENCES ");
            s.push_str(&ctx.qname(target)?);
            s.push(' ');
            s.push_str(&ctx.ident_list(target_columns)?);
            // MySQL's grammar (and `SHOW CREATE TABLE`) order: MATCH, ON DELETE, ON UPDATE.
            s.push_str(fk_match(*match_kind));
            if let Some(a) = fk_action(*on_delete) {
                s.push_str(" ON DELETE ");
                s.push_str(a);
            }
            if let Some(a) = fk_action(*on_update) {
                s.push_str(" ON UPDATE ");
                s.push_str(a);
            }
        }
        ConstraintBody::Check { expr } => {
            named(&mut s)?;
            s.push_str(&format!("CHECK ({expr})"));
        }
        ConstraintBody::Exclusion { .. } => {
            return Err(RenderError::Invalid("EXCLUDE on MySQL"));
        }
    }
    Ok(s)
}

fn add_constraint(
    ctx: &RenderCtx,
    table: &QualifiedName,
    k: &ConstraintDef,
) -> Result<Stmt, RenderError> {
    Ok(stmt(
        format!(
            "ALTER TABLE {} ADD {}",
            ctx.qname(table)?,
            constraint(ctx, k)?
        ),
        IC,
    ))
}

fn table(ctx: &RenderCtx, t: &TableDef) -> Result<Vec<Stmt>, RenderError> {
    let ext = match &t.ext {
        TableDefExt::MySql(m) => m.clone(),
        TableDefExt::None => MySqlTableDef::default(),
        _ => return Err(RenderError::Invalid("another dialect's table clauses")),
    };
    let auto = t
        .columns
        .iter()
        .find(|c| c.identity == Some(Identity::AutoIncrement))
        .map(|c| &c.name);
    let placed = ctx.place(&t.constraints, auto);
    let mut items = Vec::new();
    for c in &t.columns {
        let inline: Vec<&ConstraintDef> = t
            .constraints
            .iter()
            .zip(&placed)
            .filter(|(_, p)| matches!(&p.position, Position::InColumn(n) if *n == c.name))
            .map(|(k, _)| k)
            .collect();
        items.push(column(ctx, c, &inline)?);
    }
    // `SHOW CREATE TABLE` order: primary key, unique keys, foreign keys, checks.
    let rank = |k: &ConstraintDef| match k.body {
        ConstraintBody::PrimaryKey { .. } => 0,
        ConstraintBody::Unique { .. } => 1,
        ConstraintBody::ForeignKey { .. } => 2,
        _ => 3,
    };
    let mut in_table: Vec<&ConstraintDef> = t
        .constraints
        .iter()
        .zip(&placed)
        .filter(|(_, p)| p.position == Position::InTable)
        .map(|(k, _)| k)
        .collect();
    in_table.sort_by_key(|k| rank(k));
    for k in in_table {
        items.push(constraint(ctx, k)?);
    }
    let (_, ine) = create_flags(ctx, ObjectKind::Table, false, t.if_not_exists);
    let mut s = format!(
        "CREATE TABLE {}{} {}",
        if ine { "IF NOT EXISTS " } else { "" },
        ctx.qname(&t.name)?,
        element_block(&items)
    );
    // Engine, charset and collation names are identifiers to MySQL (`ident_or_text`).
    if let Some(e) = &ext.engine {
        s.push_str(" ENGINE=");
        s.push_str(&ctx.ident(e)?);
    }
    if let Some(c) = &ext.charset {
        s.push_str(" DEFAULT CHARSET=");
        s.push_str(&ctx.ident(c)?);
    }
    if let Some(c) = &ext.collation {
        s.push_str(" COLLATE=");
        s.push_str(&ctx.ident(c)?);
    }
    if let Some(r) = &ext.row_format {
        s.push_str(" ROW_FORMAT=");
        s.push_str(row_format(r)?);
    }
    if let Some(c) = &t.comment {
        s.push_str(&format!(" COMMENT={}", ctx.literal(c)?));
    }
    let mut out = vec![stmt(s, IC)];
    for (k, p) in t.constraints.iter().zip(&placed) {
        if p.position == Position::After {
            out.push(add_constraint(ctx, &t.name, k)?);
        }
    }
    Ok(out)
}

/// `ROW_FORMAT` takes one of the server's keywords, never free text.
fn row_format(r: &str) -> Result<&'static str, RenderError> {
    const FORMATS: [&str; 7] = [
        "DEFAULT",
        "DYNAMIC",
        "FIXED",
        "COMPRESSED",
        "REDUNDANT",
        "COMPACT",
        "PAGE",
    ];
    FORMATS
        .into_iter()
        .find(|f| r.eq_ignore_ascii_case(f))
        .ok_or(RenderError::Invalid("an unknown ROW_FORMAT"))
}

/// `FULLTEXT` / `SPATIAL` (index kinds) or `USING HASH` (the one method besides the default
/// `BTREE`, which is not written). Methods are keywords: never quoted, never free text.
fn index_kind(method: Option<&str>) -> Result<(&'static str, Option<&'static str>), RenderError> {
    Ok(match method {
        Some(m) if m.eq_ignore_ascii_case("fulltext") => ("FULLTEXT ", None),
        Some(m) if m.eq_ignore_ascii_case("spatial") => ("SPATIAL ", None),
        Some(m) if m.eq_ignore_ascii_case("btree") => ("", None),
        Some(m) if m.eq_ignore_ascii_case("hash") => ("", Some("HASH")),
        Some(_) => return Err(RenderError::Invalid("an unknown MySQL index method")),
        None => ("", None),
    })
}

fn create_index(ctx: &RenderCtx, i: &IndexDef) -> Result<String, RenderError> {
    if i.predicate.is_some() || !i.include.is_empty() || i.nulls_not_distinct || i.concurrently {
        return Err(RenderError::Invalid(
            "a partial, covering, NULLS NOT DISTINCT or concurrent index on MySQL",
        ));
    }
    let (kind, method) = index_kind(i.method.as_deref())?;
    let unique = if i.unique { "UNIQUE " } else { "" };
    let (_, ine) = create_flags(ctx, ObjectKind::Index, false, i.if_not_exists);
    let parts: Result<Vec<String>, _> = i.parts.iter().map(|p| index_part(ctx, p)).collect();
    let mut s = format!(
        "CREATE {unique}{kind}INDEX {}{} ON {} ({})",
        if ine { "IF NOT EXISTS " } else { "" },
        ctx.ident(&i.name)?,
        ctx.qname(&i.table)?,
        parts?.join(", ")
    );
    if let Some(m) = method {
        s.push_str(" USING ");
        s.push_str(m);
    }
    s.push_str(&comment_clause(ctx, i.comment.as_ref())?);
    Ok(s)
}

/// MariaDB `CREATE SEQUENCE` with the clauses that differ from MariaDB's defaults (increment
/// 1; ascending min 1 and max 9223372036854775806, descending min -9223372036854775807 and
/// max -1; start at min or max; cache 1000; no cycle). MariaDB has no `OWNED BY`, and its
/// sequences up to 11.4 are always `bigint` (`AS type` is 11.5+).
fn mariadb_sequence(ctx: &RenderCtx, q: &SeqDef) -> Result<String, RenderError> {
    if q.owned_by.is_some() {
        return Err(RenderError::Invalid("OWNED BY on MariaDB"));
    }
    if q.data_type
        .as_deref()
        .is_some_and(|t| !t.eq_ignore_ascii_case("bigint"))
    {
        return Err(RenderError::Invalid("a non-bigint MariaDB sequence"));
    }
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
    let (_, ine) = create_flags(ctx, ObjectKind::Sequence, false, q.if_not_exists);
    let mut s = format!(
        "CREATE SEQUENCE {}{}",
        if ine { "IF NOT EXISTS " } else { "" },
        ctx.qname(&q.name)?
    );
    let mut clauses = Vec::new();
    if inc != 1 {
        clauses.push(format!("INCREMENT BY {inc}"));
    }
    if min != dmin {
        clauses.push(format!("MINVALUE {min}"));
    }
    if max != dmax {
        clauses.push(format!("MAXVALUE {max}"));
    }
    if let Some(st) = q.start
        && st != dstart
    {
        clauses.push(format!("START WITH {st}"));
    }
    if let Some(c) = q.cache
        && c != 1000
    {
        clauses.push(format!("CACHE {c}"));
    }
    if q.cycle {
        clauses.push("CYCLE".to_string());
    }
    for c in clauses {
        s.push_str("\n    ");
        s.push_str(&c);
    }
    s.push_str(&comment_clause(ctx, q.comment.as_ref())?);
    Ok(s)
}

/// Whether `g` is a MySQL account as the server prints it: `'user'@'host'`, each part a
/// single-quoted literal with `''` for a quote and no backslash.
fn is_account(g: &str) -> bool {
    fn literal(s: &str) -> Option<&str> {
        let rest = s.strip_prefix('\'')?;
        let mut it = rest.char_indices().peekable();
        while let Some((i, c)) = it.next() {
            match c {
                '\\' => return None,
                '\'' if it.peek().map(|&(_, n)| n) == Some('\'') => {
                    it.next();
                }
                '\'' => return Some(&rest[i + 1..]),
                _ => {}
            }
        }
        None
    }
    literal(g)
        .and_then(|rest| rest.strip_prefix('@'))
        .and_then(literal)
        .is_some_and(str::is_empty)
}

fn grant(ctx: &RenderCtx, op: &DdlOp, g: &GrantDef) -> Result<String, RenderError> {
    if g.privileges.is_empty() {
        return Err(RenderError::Invalid("a grant without privileges"));
    }
    if g.grantee.contains('\0') {
        return Err(RenderError::Quote(sql_ident::quote::QuoteError::Nul));
    }
    if !is_account(&g.grantee) {
        return Err(RenderError::Invalid(
            "a MySQL grantee that is not 'user'@'host'",
        ));
    }
    let on = match g.kind {
        ObjectKind::Schema | ObjectKind::Database => format!("{}.*", ctx.last(&g.object)?),
        ObjectKind::Routine => format!("FUNCTION {}", ctx.qname(&g.object)?),
        ObjectKind::Procedure => format!("PROCEDURE {}", ctx.qname(&g.object)?),
        ObjectKind::Table | ObjectKind::View => ctx.qname(&g.object)?,
        _ => return Err(unsupported(op, ctx)),
    };
    let cols = if g.columns.is_empty() {
        String::new()
    } else {
        format!(" {}", ctx.ident_list(&g.columns)?)
    };
    let privs: Result<Vec<String>, RenderError> = g
        .privileges
        .iter()
        .map(|p| Ok(format!("{}{cols}", privilege(p)?)))
        .collect();
    let mut s = format!("GRANT {} ON {on} TO {}", privs?.join(", "), g.grantee);
    if g.grantable {
        s.push_str(" WITH GRANT OPTION");
    }
    Ok(s)
}

// ---- server-text head edits (HV-DDL-11 `skip_definer`, `or_replace`) -------------------------

/// A token of the head: its byte range and kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    Word,
    Quoted,
    Punct(u8),
}

struct Scanner<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> Scanner<'a> {
    fn skip_ws(&mut self) {
        let b = self.text.as_bytes();
        while self.pos < b.len() && b[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    /// The next token `(start, end, kind)`; `None` at the end or on anything the head may not
    /// contain (a comment start, a non-ASCII byte outside quotes).
    fn next(&mut self) -> Option<(usize, usize, Tok)> {
        self.skip_ws();
        let b = self.text.as_bytes();
        let start = self.pos;
        let c = *b.get(start)?;
        if c == b'/' || c == b'#' || (c == b'-' && b.get(start + 1) == Some(&b'-')) {
            return None;
        }
        if c.is_ascii_alphanumeric() || c == b'_' || c == b'$' {
            let mut end = start;
            while end < b.len()
                && (b[end].is_ascii_alphanumeric() || b[end] == b'_' || b[end] == b'$')
            {
                end += 1;
            }
            self.pos = end;
            return Some((start, end, Tok::Word));
        }
        if c == b'`' || c == b'\'' || c == b'"' {
            let mut end = start + 1;
            loop {
                let x = *b.get(end)?;
                end += 1;
                if x == b'\\' && c != b'`' {
                    // A backslash escape inside a quoted account part: not a head we edit.
                    return None;
                }
                if x == c {
                    if b.get(end) == Some(&c) {
                        end += 1;
                        continue;
                    }
                    break;
                }
            }
            self.pos = end;
            return Some((start, end, Tok::Quoted));
        }
        if matches!(c, b'=' | b'@' | b'(' | b')') {
            self.pos = start + 1;
            return Some((start, start + 1, Tok::Punct(c)));
        }
        None
    }

    fn word(&self, t: (usize, usize, Tok)) -> &'a str {
        &self.text[t.0..t.1]
    }
}

fn is_word(s: &Scanner<'_>, t: Option<(usize, usize, Tok)>, w: &str) -> bool {
    matches!(t, Some(tok @ (_, _, Tok::Word)) if s.word(tok).eq_ignore_ascii_case(w))
}

const OBJECT_WORDS: [(&str, ObjectKind); 6] = [
    ("VIEW", ObjectKind::View),
    ("FUNCTION", ObjectKind::Routine),
    ("PROCEDURE", ObjectKind::Procedure),
    ("TRIGGER", ObjectKind::Trigger),
    ("EVENT", ObjectKind::Event),
    ("AGGREGATE", ObjectKind::Aggregate),
];

/// The parsed head of a `SHOW CREATE` statement.
struct Head {
    /// Byte offset just after `CREATE`.
    after_create: usize,
    or_replace: bool,
    /// `DEFINER = user` from `DEFINER` to the start of the next token.
    definer: Option<(usize, usize)>,
    /// The object the statement creates.
    kind: Option<ObjectKind>,
}

/// `CREATE [OR REPLACE] [ALGORITHM = …] [DEFINER = …] [SQL SECURITY …] <object word>`, token
/// by token; anything else (a comment, an executable comment, another word) is `None`. The
/// body after the object word is never read.
fn parse_head(text: &str) -> Option<Head> {
    let mut s = Scanner { text, pos: 0 };
    let create = s.next();
    if !is_word(&s, create, "CREATE") {
        return None;
    }
    let after_create = create?.1;
    let mut or_replace = false;
    let mut definer = None;
    let mut tok = s.next();
    if is_word(&s, tok, "OR") {
        let t = s.next();
        if !is_word(&s, t, "REPLACE") {
            return None;
        }
        or_replace = true;
        tok = s.next();
    }
    loop {
        let t = tok?;
        if is_word(&s, tok, "ALGORITHM") {
            if s.next()?.2 != Tok::Punct(b'=') || s.next()?.2 != Tok::Word {
                return None;
            }
        } else if is_word(&s, tok, "DEFINER") {
            if s.next()?.2 != Tok::Punct(b'=') {
                return None;
            }
            let user = s.next()?;
            match user.2 {
                Tok::Word if s.word(user).eq_ignore_ascii_case("CURRENT_USER") => {
                    let save = s.pos;
                    match s.next() {
                        Some((_, _, Tok::Punct(b'('))) => {
                            if s.next()?.2 != Tok::Punct(b')') {
                                return None;
                            }
                        }
                        _ => s.pos = save,
                    }
                }
                Tok::Word | Tok::Quoted => {
                    let save = s.pos;
                    match s.next() {
                        Some((_, _, Tok::Punct(b'@'))) => {
                            if !matches!(s.next()?.2, Tok::Word | Tok::Quoted) {
                                return None;
                            }
                        }
                        _ => s.pos = save,
                    }
                }
                Tok::Punct(_) => return None,
            }
            s.skip_ws();
            definer = Some((t.0, s.pos));
        } else if is_word(&s, tok, "SQL") {
            let t = s.next();
            if !is_word(&s, t, "SECURITY") || s.next()?.2 != Tok::Word {
                return None;
            }
        } else if let Some((_, kind)) = OBJECT_WORDS.iter().find(|(w, _)| is_word(&s, tok, w)) {
            return Some(Head {
                after_create,
                or_replace,
                definer,
                kind: Some(*kind),
            });
        } else {
            return None;
        }
        tok = s.next();
    }
}

fn apply_head(text: &str, head: &Head, strip_definer: bool, or_replace: bool) -> String {
    let mut out = String::with_capacity(text.len() + 11);
    out.push_str(&text[..head.after_create]);
    if or_replace && !head.or_replace {
        out.push_str(" OR REPLACE");
    }
    match (strip_definer, head.definer) {
        (true, Some((a, b))) => {
            out.push_str(&text[head.after_create..a]);
            out.push_str(&text[b..]);
        }
        _ => out.push_str(&text[head.after_create..]),
    }
    out
}

/// A requested head edit could not be made: the head is not the expected shape, so the text
/// stays as the server printed it (producers show `ddl-head-not-edited`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeadNotEdited;

/// `text` with the requested head edits: drop the `DEFINER = user` clause, insert `OR
/// REPLACE` (the caller checks that the flavor has it for the object). Without a request the
/// text is returned as is, never parsed. The renderer applies the same edits under
/// `GenOptions::skip_definer` / `or_replace`; producers call this to know whether to notice.
pub fn edit_head(
    text: &str,
    strip_definer: bool,
    or_replace: bool,
) -> Result<String, HeadNotEdited> {
    if !strip_definer && !or_replace {
        return Ok(text.to_string());
    }
    let head = parse_head(text).ok_or(HeadNotEdited)?;
    Ok(apply_head(text, &head, strip_definer, or_replace))
}

/// Whether server text holds a MySQL / MariaDB executable comment (`/*!…*/`, MariaDB
/// `/*M!…*/`), which the server executes (verification/01 `#mysql-comments`).
pub fn has_executable_comment(text: &str) -> bool {
    text.contains("/*!") || text.contains("/*M!")
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: &str = "CREATE ALGORITHM=UNDEFINED DEFINER=`root`@`%` SQL SECURITY DEFINER VIEW `v` AS select 1 AS `a`";

    #[test]
    fn strips_the_definer_and_nothing_else() {
        assert_eq!(
            edit_head(VIEW, true, false).unwrap(),
            "CREATE ALGORITHM=UNDEFINED SQL SECURITY DEFINER VIEW `v` AS select 1 AS `a`"
        );
    }

    #[test]
    fn inserts_or_replace_once() {
        assert_eq!(
            edit_head(VIEW, false, true).unwrap(),
            "CREATE OR REPLACE ALGORITHM=UNDEFINED DEFINER=`root`@`%` SQL SECURITY DEFINER VIEW `v` AS select 1 AS `a`"
        );
        let already = "CREATE OR REPLACE VIEW v AS select 1";
        assert_eq!(edit_head(already, false, true).unwrap(), already);
    }

    #[test]
    fn quoted_definers_and_current_user() {
        let p = "CREATE DEFINER='app'@'10.0.%' PROCEDURE `p`() BEGIN SELECT 1; END";
        assert_eq!(
            edit_head(p, true, false).unwrap(),
            "CREATE PROCEDURE `p`() BEGIN SELECT 1; END"
        );
        let c =
            "CREATE DEFINER=CURRENT_USER() TRIGGER t BEFORE INSERT ON x FOR EACH ROW SET @a = 1";
        assert_eq!(
            edit_head(c, true, false).unwrap(),
            "CREATE TRIGGER t BEFORE INSERT ON x FOR EACH ROW SET @a = 1"
        );
        let odd = "CREATE DEFINER=`we``ird`@`h` FUNCTION f() RETURNS int RETURN 1";
        assert_eq!(
            edit_head(odd, true, false).unwrap(),
            "CREATE FUNCTION f() RETURNS int RETURN 1"
        );
    }

    #[test]
    fn unexpected_heads_are_left_alone() {
        for t in [
            "CREATE /*!50017 DEFINER=`root`@`%`*/ TRIGGER t BEFORE INSERT ON x FOR EACH ROW SET @a = 1",
            "CREATE TABLE t (a int)",
            "create definer = root@localhost  something",
            "CREATE DEFINER='a\\'b'@'%' VIEW v AS select 1",
            "SELECT 1",
        ] {
            assert_eq!(edit_head(t, true, true), Err(HeadNotEdited), "{t}");
        }
        // The body is never parsed: a DEFINER word after the object word stays.
        let body = "CREATE VIEW v AS select 'DEFINER=`x`@`y`' AS a";
        assert_eq!(edit_head(body, true, false).unwrap(), body);
    }

    #[test]
    fn executable_comments_are_detected() {
        assert!(has_executable_comment("select /*!80000 1 */"));
        assert!(has_executable_comment("select /*M!100000 1 */"));
        assert!(!has_executable_comment("select /* 1 */"));
    }

    #[test]
    fn accounts_are_two_quoted_literals() {
        assert!(is_account("'app'@'%'"));
        assert!(is_account("'o''brien'@'localhost'"));
        assert!(!is_account("app@%"));
        assert!(!is_account("'app'@'%' ; DROP TABLE t"));
        assert!(!is_account("'app'@"));
        assert!(!is_account("'a\\'@'%'"));
    }

    #[test]
    fn delimiters_avoid_the_text() {
        assert_eq!(delimiter_for("BEGIN SELECT 1; END"), ";;");
        assert_eq!(delimiter_for("a ;; b $$ c // d @@ e %% f"), "$$$");
        assert_eq!(delimiter_for(";; $$ // @@ %% $$$"), "$$$$");
    }
}
