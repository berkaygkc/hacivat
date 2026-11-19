// ## m4-t06-ddl-effects
//! The catalog effects of one MySQL, MariaDB or SQLite statement, from `sqlparser`'s tree of
//! one reading of it (M4 Task 6; `crate::ddl_effects`'s docs). `sqlparser` keeps every
//! identifier as written (its text and its quote), so names are taken from the tree.
//!
//! - A MySQL/MariaDB database is the model's schema (`NsKind::Schema`): `CREATE/DROP
//!   DATABASE|SCHEMA` are [`DdlKind::Schema`] effects there.
//! - SQLite's `temp` schema (any ASCII case) makes an object temporary.
//! - MySQL `RENAME TABLE a TO db2.b` (and `ALTER TABLE … RENAME TO db2.b`) moves the table to
//!   another database: a rename of `a`, and a create of `db2.b` so the target schema is
//!   refreshed too. `RENAME TABLE db1.a TO b` moves it to the current database: a create of
//!   the unqualified `b` (fix round 1, I2).
//!
//! A statement this module does not read answers none (`crate::ddl_effects::finish` adds the
//! opaque effect when the statement is in scope); a part of one that cannot be read exactly
//! adds [`DdlEffect::opaque`]. No `sqlparser` type leaves this module.

use std::sync::Arc;

use smallvec::SmallVec;
use sql_dialect::DialectId;
use sql_ident::{Ident, QualifiedName};
use sqlparser::ast::{
    self, AlterTableOperation, ColumnDef, ColumnOption, CommentObject, DropBehavior, GrantObjects,
    ObjectName, ObjectNamePart, ObjectType, RenameTableNameKind, SchemaName, Statement,
    TableConstraint,
};

use crate::report::{DdlAction, DdlEffect, DdlKind};

/// The catalog effects of the top-level statement `s` read in dialect `d`.
pub(crate) fn effects(s: &Statement, d: DialectId) -> Vec<DdlEffect> {
    let mut out = Vec::new();
    match s {
        Statement::CreateTable(ct) => {
            let mut referenced = Vec::new();
            for c in &ct.constraints {
                constraint_target(c, &mut referenced, &mut out);
            }
            for col in &ct.columns {
                column_targets(col, &mut referenced, &mut out);
            }
            for parent in ct.inherits.iter().flatten().chain(&ct.partition_of) {
                push_name(parent, &mut referenced, &mut out);
            }
            created(
                &ct.name,
                DdlKind::Table,
                ct.temporary,
                referenced,
                d,
                &mut out,
            );
        }
        Statement::CreateVirtualTable { name, .. } => {
            created(name, DdlKind::Table, false, Vec::new(), d, &mut out);
        }
        Statement::CreateView(v) => {
            let kind = if v.materialized {
                DdlKind::MaterializedView
            } else {
                DdlKind::View
            };
            created(&v.name, kind, v.temporary, Vec::new(), d, &mut out);
        }
        Statement::CreateIndex(ci) => match qualified(&ci.table_name) {
            Some(table) => {
                let name = match &ci.name {
                    Some(n) => match qualified(n) {
                        Some(q) => Some(q),
                        None => {
                            out.push(DdlEffect::opaque());
                            None
                        }
                    },
                    None => None,
                };
                out.push(DdlEffect {
                    name,
                    on: Some(table),
                    concurrently: ci.concurrently,
                    ..effect(DdlAction::Create, DdlKind::Index)
                });
            }
            None => out.push(DdlEffect::opaque()),
        },
        Statement::Drop {
            object_type,
            names,
            cascade,
            temporary,
            table,
            ..
        } => {
            let kind = match object_type {
                ObjectType::Table | ObjectType::ForeignTable => Some(DdlKind::Table),
                ObjectType::View => Some(DdlKind::View),
                ObjectType::MaterializedView => Some(DdlKind::MaterializedView),
                ObjectType::Index => Some(DdlKind::Index),
                ObjectType::Sequence => Some(DdlKind::Sequence),
                ObjectType::Type => Some(DdlKind::Type),
                ObjectType::Schema | ObjectType::Database => Some(database_kind(d)),
                _ => None,
            };
            let on = table.as_ref().map(qualified);
            match (kind, on) {
                (Some(kind), on @ (None | Some(Some(_)))) if !names.is_empty() => {
                    let on = on.flatten();
                    for n in names {
                        match qualified(n) {
                            Some(name) => out.push(DdlEffect {
                                name: Some(name.clone()),
                                on: on.clone(),
                                temp: *temporary || in_temp_schema(&name, d),
                                ..effect(DdlAction::Drop { cascade: *cascade }, kind)
                            }),
                            None => out.push(DdlEffect::opaque()),
                        }
                    }
                }
                _ => out.push(DdlEffect::opaque()),
            }
        }
        Statement::DropFunction(df) => {
            dropped_routines(
                df.func_desc.iter().map(|f| &f.name),
                df.drop_behavior.as_ref(),
                &mut out,
            );
        }
        Statement::DropProcedure {
            proc_desc,
            drop_behavior,
            ..
        } => dropped_routines(
            proc_desc.iter().map(|f| &f.name),
            drop_behavior.as_ref(),
            &mut out,
        ),
        Statement::DropTrigger(dt) => match qualified(&dt.trigger_name) {
            Some(name) => {
                let on = dt.table_name.as_ref().map(qualified);
                match on {
                    None | Some(Some(_)) => out.push(DdlEffect {
                        name: Some(name),
                        on: on.flatten(),
                        ..effect(DdlAction::Drop { cascade: false }, DdlKind::Trigger)
                    }),
                    Some(None) => out.push(DdlEffect::opaque()),
                }
            }
            None => out.push(DdlEffect::opaque()),
        },
        Statement::RenameTable(pairs) => {
            if pairs.is_empty() {
                out.push(DdlEffect::opaque());
            }
            for p in pairs {
                renamed(&p.old_name, &p.new_name, DdlKind::Table, d, &mut out);
            }
        }
        Statement::AlterTable(at) => alter_table(at, d, &mut out),
        Statement::AlterView(av) => {
            let kind = if av.materialized {
                DdlKind::MaterializedView
            } else {
                DdlKind::View
            };
            named_or_opaque(DdlAction::Alter, kind, &av.name, &mut out);
        }
        Statement::AlterIndex { name, operation } => match operation {
            ast::AlterIndexOperation::RenameIndex { index_name } => {
                renamed(name, index_name, DdlKind::Index, d, &mut out);
            }
        },
        Statement::AlterSchema(a) => {
            named_or_opaque(DdlAction::Alter, database_kind(d), &a.name, &mut out);
        }
        Statement::CreateSchema { schema_name, .. } => match schema_name {
            SchemaName::Simple(n) | SchemaName::NamedAuthorization(n, _) => {
                named_or_opaque(DdlAction::Create, database_kind(d), n, &mut out);
            }
            SchemaName::UnnamedAuthorization(_) => out.push(DdlEffect::opaque()),
        },
        Statement::CreateDatabase { db_name, .. } => {
            named_or_opaque(DdlAction::Create, database_kind(d), db_name, &mut out);
        }
        Statement::CreateTrigger(ct) => match (qualified(&ct.name), qualified(&ct.table_name)) {
            (Some(name), Some(table)) => {
                let mut referenced = Vec::new();
                if let Some(r) = &ct.referenced_table_name {
                    push_name(r, &mut referenced, &mut out);
                }
                out.push(DdlEffect {
                    name: Some(name),
                    on: Some(table),
                    referenced,
                    ..effect(DdlAction::Create, DdlKind::Trigger)
                });
            }
            _ => out.push(DdlEffect::opaque()),
        },
        Statement::CreateFunction(cf) => {
            named_or_opaque(DdlAction::Create, DdlKind::Routine, &cf.name, &mut out);
        }
        Statement::CreateProcedure { name, .. } => {
            named_or_opaque(DdlAction::Create, DdlKind::Routine, name, &mut out);
        }
        Statement::Comment {
            object_type,
            object_name,
            ..
        } => comment(object_type, object_name, d, &mut out),
        Statement::Grant(g) => granted(DdlAction::Grant, g.objects.as_ref(), d, &mut out),
        Statement::Revoke(r) => granted(DdlAction::Revoke, r.objects.as_ref(), d, &mut out),
        _ => {}
    }
    out
}

/// An effect with no names and no flags.
fn effect(action: DdlAction, kind: DdlKind) -> DdlEffect {
    DdlEffect {
        action,
        kind,
        name: None,
        on: None,
        referenced: Vec::new(),
        temp: false,
        concurrently: false,
        opaque: false,
    }
}

/// A database of `d` as the model holds it: MySQL's and MariaDB's are schemas.
fn database_kind(d: DialectId) -> DdlKind {
    match d {
        DialectId::MySql | DialectId::MariaDb => DdlKind::Schema,
        _ => DdlKind::Database,
    }
}

/// One identifier as written: the text the server reads (a doubled closing quote is one; the
/// tokenizer runs with unescaping off) and whether it was quoted.
fn ident(i: &ast::Ident) -> Ident {
    Ident {
        text: match i.quote_style {
            Some(q @ ('"' | '`')) => Arc::from(i.value.replace(&format!("{q}{q}"), &q.to_string())),
            _ => Arc::from(i.value.as_str()),
        },
        quoted: i.quote_style.is_some(),
    }
}

/// A name as written; `None` for a part that is not an identifier, or `*`.
fn qualified(n: &ObjectName) -> Option<QualifiedName> {
    let mut parts: SmallVec<[Ident; 3]> = SmallVec::new();
    for p in &n.0 {
        match p {
            ObjectNamePart::Identifier(i) if !(i.quote_style.is_none() && i.value == "*") => {
                parts.push(ident(i));
            }
            _ => return None,
        }
    }
    (!parts.is_empty()).then_some(QualifiedName { parts })
}

/// Whether `name` is written in SQLite's `temp` schema.
fn in_temp_schema(name: &QualifiedName, d: DialectId) -> bool {
    d == DialectId::Sqlite
        && name.parts.len() == 2
        && name.parts[0].text.eq_ignore_ascii_case("temp")
}

fn push_name(n: &ObjectName, into: &mut Vec<QualifiedName>, out: &mut Vec<DdlEffect>) {
    match qualified(n) {
        Some(q) if !into.contains(&q) => into.push(q),
        Some(_) => {}
        None => out.push(DdlEffect::opaque()),
    }
}

fn constraint_target(c: &TableConstraint, into: &mut Vec<QualifiedName>, out: &mut Vec<DdlEffect>) {
    if let TableConstraint::ForeignKey(fk) = c {
        push_name(&fk.foreign_table, into, out);
    }
}

fn column_targets(col: &ColumnDef, into: &mut Vec<QualifiedName>, out: &mut Vec<DdlEffect>) {
    for o in &col.options {
        if let ColumnOption::ForeignKey(fk) = &o.option {
            push_name(&fk.foreign_table, into, out);
        }
    }
}

fn named_or_opaque(action: DdlAction, kind: DdlKind, n: &ObjectName, out: &mut Vec<DdlEffect>) {
    match qualified(n) {
        Some(name) => out.push(DdlEffect {
            name: Some(name),
            ..effect(action, kind)
        }),
        None => out.push(DdlEffect::opaque()),
    }
}

fn created(
    n: &ObjectName,
    kind: DdlKind,
    temporary: bool,
    referenced: Vec<QualifiedName>,
    d: DialectId,
    out: &mut Vec<DdlEffect>,
) {
    match qualified(n) {
        Some(name) => out.push(DdlEffect {
            temp: temporary || in_temp_schema(&name, d),
            name: Some(name),
            referenced,
            ..effect(DdlAction::Create, kind)
        }),
        None => out.push(DdlEffect::opaque()),
    }
}

fn dropped_routines<'n>(
    names: impl Iterator<Item = &'n ObjectName>,
    behavior: Option<&DropBehavior>,
    out: &mut Vec<DdlEffect>,
) {
    let cascade = matches!(behavior, Some(DropBehavior::Cascade));
    let mut any = false;
    for n in names {
        any = true;
        named_or_opaque(DdlAction::Drop { cascade }, DdlKind::Routine, n, out);
    }
    if !any {
        out.push(DdlEffect::opaque());
    }
}

/// `old` renamed to `new`: the rename, and a create of `new` when it names another schema
/// (the module docs). MySQL and MariaDB resolve an unqualified `new` against the session's
/// current database, so `RENAME TABLE db1.a TO b` moves `a` there: a create of the unqualified
/// `b` too, which the consumer resolves as every other unqualified name (fix round 1, I2).
/// SQLite keeps a renamed table in its schema.
fn renamed(
    old: &ObjectName,
    new: &ObjectName,
    kind: DdlKind,
    d: DialectId,
    out: &mut Vec<DdlEffect>,
) {
    let (Some(old), Some(new)) = (qualified(old), qualified(new)) else {
        out.push(DdlEffect::opaque());
        return;
    };
    let Some((to, new_schema)) = new.parts.split_last() else {
        out.push(DdlEffect::opaque());
        return;
    };
    let old_schema = &old.parts[..old.parts.len() - 1];
    let mysql = matches!(d, DialectId::MySql | DialectId::MariaDb);
    let moved = if new_schema.is_empty() {
        mysql && !old_schema.is_empty()
    } else {
        new_schema != old_schema
    };
    out.push(DdlEffect {
        name: Some(old),
        ..effect(DdlAction::Rename { to: to.clone() }, kind)
    });
    if moved {
        out.push(DdlEffect {
            name: Some(new.clone()),
            ..effect(DdlAction::Create, kind)
        });
    }
}

fn alter_table(at: &ast::AlterTable, d: DialectId, out: &mut Vec<DdlEffect>) {
    let Some(name) = qualified(&at.name) else {
        out.push(DdlEffect::opaque());
        return;
    };
    let mut referenced = Vec::new();
    let mut renames = Vec::new();
    let mut altered = false;
    for op in &at.operations {
        match op {
            AlterTableOperation::RenameTable { table_name } => {
                let (RenameTableNameKind::As(to) | RenameTableNameKind::To(to)) = table_name;
                renames.push(to);
            }
            AlterTableOperation::AddConstraint { constraint, .. } => {
                altered = true;
                constraint_target(constraint, &mut referenced, out);
            }
            AlterTableOperation::AddColumn { column_def, .. } => {
                altered = true;
                column_targets(column_def, &mut referenced, out);
            }
            _ => altered = true,
        }
    }
    if altered || renames.is_empty() {
        out.push(DdlEffect {
            name: Some(name),
            referenced,
            ..effect(DdlAction::Alter, DdlKind::Table)
        });
    }
    for to in renames {
        renamed(&at.name, to, DdlKind::Table, d, out);
    }
}

fn comment(object_type: &CommentObject, n: &ObjectName, d: DialectId, out: &mut Vec<DdlEffect>) {
    let Some(name) = qualified(n) else {
        out.push(DdlEffect::opaque());
        return;
    };
    let kind = match object_type {
        CommentObject::Column => {
            let Some((column, table)) = name.parts.split_last() else {
                out.push(DdlEffect::opaque());
                return;
            };
            if table.is_empty() {
                out.push(DdlEffect::opaque());
                return;
            }
            out.push(DdlEffect {
                name: Some(QualifiedName {
                    parts: SmallVec::from_elem(column.clone(), 1),
                }),
                on: Some(QualifiedName {
                    parts: table.iter().cloned().collect(),
                }),
                ..effect(DdlAction::Comment, DdlKind::Other)
            });
            return;
        }
        CommentObject::Table | CommentObject::ForeignTable => DdlKind::Table,
        CommentObject::View => DdlKind::View,
        CommentObject::MaterializedView => DdlKind::MaterializedView,
        CommentObject::Index => DdlKind::Index,
        CommentObject::Sequence => DdlKind::Sequence,
        CommentObject::Function | CommentObject::Procedure => DdlKind::Routine,
        CommentObject::Type => DdlKind::Type,
        CommentObject::Domain => DdlKind::Domain,
        CommentObject::Extension => DdlKind::Extension,
        CommentObject::Schema => database_kind(d),
        CommentObject::Database => database_kind(d),
        CommentObject::Role | CommentObject::User => DdlKind::Role,
        CommentObject::Collation => {
            out.push(DdlEffect::opaque());
            return;
        }
    };
    out.push(DdlEffect {
        name: Some(name),
        ..effect(DdlAction::Comment, kind)
    });
}

fn granted(
    action: DdlAction,
    objects: Option<&GrantObjects>,
    d: DialectId,
    out: &mut Vec<DdlEffect>,
) {
    let (kind, names): (DdlKind, Vec<&ObjectName>) = match objects {
        Some(GrantObjects::Tables(v)) => {
            // MySQL `ON db.*`: every table of `db` (the schema); `ON *.*` and `ON *`: no name.
            if let [one] = v.as_slice()
                && let [
                    ObjectNamePart::Identifier(db),
                    ObjectNamePart::Identifier(star),
                ] = one.0.as_slice()
                && star.quote_style.is_none()
                && star.value == "*"
            {
                if db.quote_style.is_none() && db.value == "*" {
                    out.push(DdlEffect::opaque());
                } else {
                    out.push(DdlEffect {
                        name: Some(QualifiedName {
                            parts: SmallVec::from_elem(ident(db), 1),
                        }),
                        ..effect(action, database_kind(d))
                    });
                }
                return;
            }
            (DdlKind::Table, v.iter().collect())
        }
        Some(GrantObjects::Views(v)) => (DdlKind::View, v.iter().collect()),
        Some(GrantObjects::Sequences(v)) => (DdlKind::Sequence, v.iter().collect()),
        Some(GrantObjects::Schemas(v) | GrantObjects::Databases(v)) => {
            (database_kind(d), v.iter().collect())
        }
        Some(
            GrantObjects::AllTablesInSchema { schemas }
            | GrantObjects::AllSequencesInSchema { schemas }
            | GrantObjects::AllViewsInSchema { schemas }
            | GrantObjects::AllFunctionsInSchema { schemas }
            | GrantObjects::AllProceduresInSchema { schemas }
            | GrantObjects::AllRoutinesInSchema { schemas },
        ) => (database_kind(d), schemas.iter().collect()),
        Some(
            GrantObjects::Functions(v) | GrantObjects::Procedures(v) | GrantObjects::Routines(v),
        ) => (DdlKind::Routine, v.iter().map(|f| &f.name).collect()),
        _ => {
            out.push(DdlEffect::opaque());
            return;
        }
    };
    if names.is_empty() {
        out.push(DdlEffect::opaque());
    }
    for n in names {
        named_or_opaque(action.clone(), kind, n, out);
    }
}
