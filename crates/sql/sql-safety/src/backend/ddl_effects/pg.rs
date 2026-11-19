// ## m4-t06-ddl-effects
//! The catalog effects of one PostgreSQL statement, from libpg_query's raw tree (M4 Task 6;
//! `crate::ddl_effects`'s docs).
//!
//! The raw tree holds names as the server reads them (unquoted parts folded to lower case,
//! every part cut to 63 bytes). [`Namer`] finds how each was written: at the name's own
//! position when the tree keeps one (`RangeVar`, `TypeName`, `RoleSpec`), else the first name
//! in the statement's text that the server reads the same way (the last one for a `RENAME TO`
//! or `SET SCHEMA` target). A spelling found either way is
//! that name exactly (`sql_ident::fold` of it is the tree's text), so either reading is the
//! same object; when none is found (`U&"…"`, a comment between the parts) the tree's text is
//! the name, every part quoted.
//!
//! Each statement `crate::ddl_effects`'s scope names maps to its effects here; anything else
//! answers none (`crate::ddl_effects::finish` adds the opaque effect when the statement is in
//! scope), and a part of a statement that cannot be read exactly adds [`DdlEffect::opaque`].

use std::cell::OnceCell;
use std::collections::HashMap;
use std::sync::Arc;

use smallvec::SmallVec;
use sql_dialect::DialectId;
use sql_ident::{Ident, QualifiedName, fold, parse_qualified};
use sql_oracle_pg::tree::{ScanKind, pb};
use sql_text::{TextRange, TextSize};

use crate::report::{DdlAction, DdlEffect, DdlKind};

use pb::node::Node as N;

/// Names by their server reading (folded parts): the first and the last spelling written.
type Spellings = HashMap<Vec<Arc<str>>, (QualifiedName, QualifiedName)>;

/// How the names of one statement are written (the module docs).
pub(crate) struct Namer<'s> {
    sql: &'s str,
    range: TextRange,
    toks: &'s [(TextRange, ScanKind)],
    /// Every name the statement's text writes, by its server reading: the first and the last
    /// spelling.
    written: OnceCell<Spellings>,
}

impl<'s> Namer<'s> {
    pub(crate) fn new(sql: &'s str, range: TextRange, toks: &'s [(TextRange, ScanKind)]) -> Self {
        Namer {
            sql,
            range,
            toks,
            written: OnceCell::new(),
        }
    }

    /// Token `i`: its text, kind and range.
    fn tok(&self, i: usize) -> Option<(&'s str, ScanKind, TextRange)> {
        let (r, k) = *self.toks.get(i)?;
        let t = self.sql.get(usize::from(r.start())..usize::from(r.end()))?;
        Some((t, k, r))
    }

    fn skip_comments(&self, mut i: usize) -> usize {
        while matches!(self.tok(i), Some((_, ScanKind::Comment, _))) {
            i += 1;
        }
        i
    }

    /// The span of `part ( . part )*` from token `i`, inside the statement.
    fn chain_from(&self, i: usize) -> Option<TextRange> {
        let (_, k, first) = self.tok(i)?;
        if !matches!(k, ScanKind::Ident | ScanKind::Keyword) || !self.range.contains_range(first) {
            return None;
        }
        let (mut end, mut j) = (first.end(), i);
        loop {
            let dot = self.skip_comments(j + 1);
            if !matches!(self.tok(dot), Some((".", ScanKind::Other, _))) {
                break;
            }
            let next = self.skip_comments(dot + 1);
            match self.tok(next) {
                Some((_, ScanKind::Ident | ScanKind::Keyword, r))
                    if self.range.contains_range(r) =>
                {
                    end = r.end();
                    j = next;
                }
                _ => break,
            }
        }
        Some(TextRange::new(first.start(), end))
    }

    /// The name written at `span`, with its server reading.
    fn read(&self, span: TextRange) -> Option<(Vec<Arc<str>>, QualifiedName)> {
        let text = self
            .sql
            .get(usize::from(span.start())..usize::from(span.end()))?;
        let q = parse_qualified(DialectId::Postgres, text)?;
        let key = q
            .parts
            .iter()
            .map(|p| fold(DialectId::Postgres, &p.text, p.quoted))
            .collect();
        Some((key, q))
    }

    fn written(&self) -> &Spellings {
        self.written.get_or_init(|| {
            let mut map = HashMap::new();
            let first = self
                .toks
                .partition_point(|(r, _)| r.start() < self.range.start());
            let mut prev_dot = false;
            for i in first..self.toks.len() {
                let Some((t, k, r)) = self.tok(i) else { break };
                if !self.range.contains_range(r) {
                    break;
                }
                if k == ScanKind::Comment {
                    continue;
                }
                if !prev_dot && let Some((key, q)) = self.chain_from(i).and_then(|s| self.read(s)) {
                    map.entry(key)
                        .and_modify(|(_, last)| *last = q.clone())
                        .or_insert_with(|| (q.clone(), q));
                }
                prev_dot = t == "." && k == ScanKind::Other;
            }
            map
        })
    }

    /// The name whose server reading is `tree` (outermost part first), as written: at byte
    /// `at` when given and it reads so there, else the first such name of the statement, else
    /// `tree` itself, quoted.
    fn name(&self, tree: &[&str], at: Option<i32>) -> QualifiedName {
        self.find(tree, at).unwrap_or_else(|| quoted(tree))
    }

    /// The name whose server reading is `tree`, as written at byte `at` or anywhere in the
    /// statement; `None` when the text writes it nowhere as a plain name.
    fn find(&self, tree: &[&str], at: Option<i32>) -> Option<QualifiedName> {
        let key: Vec<Arc<str>> = tree.iter().map(|p| Arc::from(*p)).collect();
        at.and_then(|l| usize::try_from(l).ok())
            .and_then(|l| TextSize::try_from(l).ok())
            .and_then(|l| self.toks.binary_search_by(|(r, _)| r.start().cmp(&l)).ok())
            .and_then(|i| self.chain_from(i))
            .and_then(|s| self.read(s))
            .filter(|(k, _)| *k == key)
            .map(|(_, q)| q)
            .or_else(|| self.written().get(&key).map(|(first, _)| first.clone()))
    }

    /// One identifier whose server reading is `tree`, as written: the last such name of the
    /// statement (a `RENAME TO` or `SET SCHEMA` target is written last), else `tree`, quoted.
    fn ident(&self, tree: &str) -> Ident {
        let key: Vec<Arc<str>> = vec![Arc::from(tree)];
        self.written()
            .get(&key)
            .and_then(|(_, last)| last.parts.first().cloned())
            .unwrap_or(Ident {
                text: Arc::from(tree),
                quoted: true,
            })
    }

    fn rv(&self, rv: &pb::RangeVar) -> QualifiedName {
        let parts: Vec<&str> = [&rv.catalogname, &rv.schemaname, &rv.relname]
            .into_iter()
            .filter(|p| !p.is_empty())
            .map(String::as_str)
            .collect();
        self.name(&parts, Some(rv.location))
    }

    /// The name of an object node: a `List` of `String`s, a `String`, an `ObjectWithArgs`, a
    /// `TypeName` or a `RangeVar`.
    fn object(&self, n: &pb::Node) -> Option<QualifiedName> {
        Some(match n.node.as_ref()? {
            N::RangeVar(rv) => self.rv(rv),
            N::TypeName(t) => self.name(&strings(&t.names)?, Some(t.location)),
            N::List(l) => self.name(&strings(&l.items)?, None),
            N::String(s) if !s.sval.is_empty() => self.name(&[s.sval.as_str()], None),
            N::ObjectWithArgs(o) => self.name(&strings(&o.objname)?, None),
            _ => return None,
        })
    }

    /// A trigger's, policy's, rule's, column's or constraint's name and its table, from the
    /// `List` `[schema?, table, name]`: written as one name (`COMMENT ON COLUMN s.t.c`) or as
    /// two (`DROP TRIGGER trg ON s.t`).
    fn on_table(&self, n: &pb::Node) -> Option<(QualifiedName, QualifiedName)> {
        let N::List(l) = n.node.as_ref()? else {
            return None;
        };
        let parts = strings(&l.items)?;
        let (last, table) = parts.split_last()?;
        if table.is_empty() {
            return None;
        }
        if let Some(mut whole) = self.find(&parts, None) {
            let name = whole.parts.pop()?;
            return Some((
                QualifiedName {
                    parts: SmallVec::from_elem(name, 1),
                },
                whole,
            ));
        }
        Some((self.name(&[*last], None), self.name(table, None)))
    }

    fn role(&self, rs: &pb::RoleSpec) -> Option<QualifiedName> {
        (rs.roletype == pb::RoleSpecType::RolespecCstring as i32 && !rs.rolename.is_empty())
            .then(|| self.name(&[rs.rolename.as_str()], Some(rs.location)))
    }
}

/// The catalog effects of the statement `root` (the module docs).
pub(crate) fn effects(root: &pb::Node, n: &Namer<'_>) -> Vec<DdlEffect> {
    let Some(node) = root.node.as_ref() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match node {
        N::CreateStmt(s) => create_stmt(s, n, &mut out),
        N::CreateForeignTableStmt(s) => match &s.base_stmt {
            Some(b) => create_stmt(b, n, &mut out),
            None => out.push(DdlEffect::opaque()),
        },
        N::CreateTableAsStmt(s) => {
            let kind = if s.objtype == pb::ObjectType::ObjectMatview as i32 {
                DdlKind::MaterializedView
            } else {
                DdlKind::Table
            };
            into(s.into.as_deref(), kind, n, &mut out);
        }
        N::SelectStmt(s) => {
            if s.into_clause.is_some() {
                into(s.into_clause.as_deref(), DdlKind::Table, n, &mut out);
            }
        }
        N::ViewStmt(s) => created_rel(s.view.as_ref(), DdlKind::View, n, &mut out),
        N::CreateSeqStmt(s) => created_rel(s.sequence.as_ref(), DdlKind::Sequence, n, &mut out),
        N::AlterSeqStmt(s) => match &s.sequence {
            Some(rv) => out.push(named(DdlAction::Alter, DdlKind::Sequence, n.rv(rv))),
            None => out.push(DdlEffect::opaque()),
        },
        N::IndexStmt(s) => match &s.relation {
            Some(rv) => out.push(DdlEffect {
                name: (!s.idxname.is_empty()).then(|| n.name(&[s.idxname.as_str()], None)),
                on: Some(n.rv(rv)),
                concurrently: s.concurrent,
                ..effect(DdlAction::Create, DdlKind::Index)
            }),
            None => out.push(DdlEffect::opaque()),
        },
        N::DropStmt(s) => drop_stmt(s, n, &mut out),
        N::DropdbStmt(s) => out.push(named(
            DdlAction::Drop { cascade: false },
            DdlKind::Database,
            n.name(&[s.dbname.as_str()], None),
        )),
        N::DropRoleStmt(s) => {
            for r in &s.roles {
                match &r.node {
                    Some(N::RoleSpec(rs)) if let Some(name) = n.role(rs) => {
                        out.push(named(
                            DdlAction::Drop { cascade: false },
                            DdlKind::Role,
                            name,
                        ));
                    }
                    _ => out.push(DdlEffect::opaque()),
                }
            }
        }
        N::RenameStmt(s) => rename_stmt(s, n, &mut out),
        N::AlterTableStmt(s) => alter_table(s, n, &mut out),
        N::AlterObjectSchemaStmt(s) => {
            let to = n.ident(&s.newschema);
            let name = match (&s.relation, &s.object) {
                (Some(rv), _) => Some(n.rv(rv)),
                (None, Some(o)) => n.object(o),
                (None, None) => None,
            };
            match (kind_of(s.object_type), name) {
                (Some(kind), Some(name)) if !s.newschema.is_empty() => {
                    out.push(named(DdlAction::SetSchema { to }, kind, name));
                }
                _ => out.push(DdlEffect::opaque()),
            }
        }
        N::AlterOwnerStmt(s) => {
            let name = match (&s.relation, &s.object) {
                (Some(rv), _) => Some(n.rv(rv)),
                (None, Some(o)) => n.object(o),
                (None, None) => None,
            };
            match (kind_of(s.object_type), name) {
                (Some(kind), Some(name)) => out.push(named(DdlAction::Alter, kind, name)),
                _ => out.push(DdlEffect::opaque()),
            }
        }
        N::CommentStmt(s) => comment(s, n, &mut out),
        N::GrantStmt(s) => grant(s, n, &mut out),
        N::GrantRoleStmt(s) => {
            let action = if s.is_grant {
                DdlAction::Grant
            } else {
                DdlAction::Revoke
            };
            for r in &s.granted_roles {
                match &r.node {
                    Some(N::AccessPriv(p)) if !p.priv_name.is_empty() => out.push(named(
                        action.clone(),
                        DdlKind::Role,
                        n.name(&[p.priv_name.as_str()], None),
                    )),
                    _ => out.push(DdlEffect::opaque()),
                }
            }
        }
        N::CreateSchemaStmt(s) => {
            if s.schemaname.is_empty() {
                // `CREATE SCHEMA AUTHORIZATION r`: named after a role the text may not spell
                // as a schema would be read.
                out.push(DdlEffect::opaque());
            } else {
                out.push(named(
                    DdlAction::Create,
                    DdlKind::Schema,
                    n.name(&[s.schemaname.as_str()], None),
                ));
            }
        }
        N::CreatedbStmt(s) => out.push(named(
            DdlAction::Create,
            DdlKind::Database,
            n.name(&[s.dbname.as_str()], None),
        )),
        N::AlterDatabaseStmt(s) => out.push(named(
            DdlAction::Alter,
            DdlKind::Database,
            n.name(&[s.dbname.as_str()], None),
        )),
        N::AlterDatabaseSetStmt(s) => out.push(named(
            DdlAction::Alter,
            DdlKind::Database,
            n.name(&[s.dbname.as_str()], None),
        )),
        N::CompositeTypeStmt(s) => match &s.typevar {
            Some(rv) => out.push(named(DdlAction::Create, DdlKind::Type, n.rv(rv))),
            None => out.push(DdlEffect::opaque()),
        },
        N::CreateEnumStmt(s) => listed(DdlAction::Create, DdlKind::Type, &s.type_name, n, &mut out),
        N::CreateRangeStmt(s) => {
            listed(DdlAction::Create, DdlKind::Type, &s.type_name, n, &mut out);
        }
        N::AlterEnumStmt(s) => listed(DdlAction::Alter, DdlKind::Type, &s.type_name, n, &mut out),
        N::AlterTypeStmt(s) => listed(DdlAction::Alter, DdlKind::Type, &s.type_name, n, &mut out),
        N::CreateDomainStmt(s) => {
            listed(
                DdlAction::Create,
                DdlKind::Domain,
                &s.domainname,
                n,
                &mut out,
            );
        }
        N::AlterDomainStmt(s) => {
            listed(DdlAction::Alter, DdlKind::Domain, &s.type_name, n, &mut out);
        }
        N::CreateFunctionStmt(s) => {
            listed(
                DdlAction::Create,
                DdlKind::Routine,
                &s.funcname,
                n,
                &mut out,
            );
        }
        N::AlterFunctionStmt(s) => match &s.func {
            Some(f) => listed(DdlAction::Alter, DdlKind::Routine, &f.objname, n, &mut out),
            None => out.push(DdlEffect::opaque()),
        },
        N::DefineStmt(s) => {
            use pb::ObjectType as O;
            match O::try_from(s.kind) {
                Ok(O::ObjectType) => {
                    listed(DdlAction::Create, DdlKind::Type, &s.defnames, n, &mut out);
                }
                Ok(O::ObjectAggregate) => {
                    listed(
                        DdlAction::Create,
                        DdlKind::Routine,
                        &s.defnames,
                        n,
                        &mut out,
                    );
                }
                _ => out.push(DdlEffect::opaque()),
            }
        }
        N::CreateTrigStmt(s) => match &s.relation {
            Some(rv) if !s.trigname.is_empty() => out.push(DdlEffect {
                name: Some(n.name(&[s.trigname.as_str()], None)),
                on: Some(n.rv(rv)),
                referenced: s.constrrel.iter().map(|c| n.rv(c)).collect(),
                ..effect(DdlAction::Create, DdlKind::Trigger)
            }),
            _ => out.push(DdlEffect::opaque()),
        },
        N::CreatePolicyStmt(s) => {
            attached(
                DdlAction::Create,
                DdlKind::Policy,
                &s.policy_name,
                s.table.as_ref(),
                n,
                &mut out,
            );
        }
        N::AlterPolicyStmt(s) => {
            attached(
                DdlAction::Alter,
                DdlKind::Policy,
                &s.policy_name,
                s.table.as_ref(),
                n,
                &mut out,
            );
        }
        N::RuleStmt(s) => {
            attached(
                DdlAction::Create,
                DdlKind::Rule,
                &s.rulename,
                s.relation.as_ref(),
                n,
                &mut out,
            );
        }
        // An extension creates (or drops) its member objects in a schema the statement may not
        // name: the extension, and the current namespace.
        N::CreateExtensionStmt(s) => extension(DdlAction::Create, &s.extname, n, &mut out),
        N::AlterExtensionStmt(s) => extension(DdlAction::Alter, &s.extname, n, &mut out),
        N::AlterExtensionContentsStmt(s) => extension(DdlAction::Alter, &s.extname, n, &mut out),
        N::CreateRoleStmt(s) if !s.role.is_empty() => out.push(named(
            DdlAction::Create,
            DdlKind::Role,
            n.name(&[s.role.as_str()], None),
        )),
        N::AlterRoleStmt(s) => match s.role.as_ref().and_then(|rs| n.role(rs)) {
            Some(name) => out.push(named(DdlAction::Alter, DdlKind::Role, name)),
            None => out.push(DdlEffect::opaque()),
        },
        N::ExplainStmt(s) => {
            // `EXPLAIN ANALYZE` runs its statement (`finish` adds the opaque effect).
            if crate::backend::pg::explain_analyze(&s.options)
                && let Some(q) = s.query.as_deref()
            {
                out.extend(effects(q, n));
            }
        }
        _ => {}
    }
    out
}

/// The server's reading of a name, every part quoted (the same identity, never folded again).
fn quoted(tree: &[&str]) -> QualifiedName {
    QualifiedName {
        parts: tree
            .iter()
            .map(|p| Ident {
                text: Arc::from(*p),
                quoted: true,
            })
            .collect(),
    }
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

fn named(action: DdlAction, kind: DdlKind, name: QualifiedName) -> DdlEffect {
    DdlEffect {
        name: Some(name),
        ..effect(action, kind)
    }
}

/// The texts of a list of `String` nodes; `None` if any is something else or it is empty.
fn strings(list: &[pb::Node]) -> Option<Vec<&str>> {
    list.iter()
        .map(|n| match &n.node {
            Some(N::String(s)) if !s.sval.is_empty() => Some(s.sval.as_str()),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
        .filter(|v| !v.is_empty())
}

/// Whether a relation is temporary: `TEMP`/`TEMPORARY`, or written in `pg_temp`.
fn is_temp(rv: &pb::RangeVar) -> bool {
    rv.relpersistence == "t" || rv.schemaname == "pg_temp" || rv.schemaname.starts_with("pg_temp_")
}

/// The kind of an `ObjectType`; `None` for the kinds the explorer model does not hold.
fn kind_of(t: i32) -> Option<DdlKind> {
    use pb::ObjectType as O;
    Some(match O::try_from(t).ok()? {
        O::ObjectTable | O::ObjectForeignTable => DdlKind::Table,
        O::ObjectView => DdlKind::View,
        O::ObjectMatview => DdlKind::MaterializedView,
        O::ObjectIndex => DdlKind::Index,
        O::ObjectSequence => DdlKind::Sequence,
        O::ObjectFunction | O::ObjectProcedure | O::ObjectRoutine | O::ObjectAggregate => {
            DdlKind::Routine
        }
        O::ObjectTrigger => DdlKind::Trigger,
        O::ObjectType => DdlKind::Type,
        O::ObjectDomain => DdlKind::Domain,
        O::ObjectExtension => DdlKind::Extension,
        O::ObjectPolicy => DdlKind::Policy,
        O::ObjectRule => DdlKind::Rule,
        O::ObjectSchema => DdlKind::Schema,
        O::ObjectDatabase => DdlKind::Database,
        O::ObjectRole => DdlKind::Role,
        _ => return None,
    })
}

/// Kinds named `name ON table`.
fn attached_to_a_table(kind: DdlKind) -> bool {
    matches!(kind, DdlKind::Trigger | DdlKind::Policy | DdlKind::Rule)
}

/// The tables a table definition's elements and constraints reference (foreign keys).
fn fk_targets(elements: &[pb::Node], n: &Namer<'_>, into: &mut Vec<QualifiedName>) {
    for e in elements {
        match e.node.as_ref() {
            Some(N::Constraint(c)) => fk_target(c, n, into),
            Some(N::ColumnDef(col)) => fk_targets(&col.constraints, n, into),
            _ => {}
        }
    }
}

fn fk_target(c: &pb::Constraint, n: &Namer<'_>, into: &mut Vec<QualifiedName>) {
    if c.contype == pb::ConstrType::ConstrForeign as i32
        && let Some(rv) = &c.pktable
    {
        push_unique(into, n.rv(rv));
    }
}

fn push_unique(into: &mut Vec<QualifiedName>, name: QualifiedName) {
    if !into.contains(&name) {
        into.push(name);
    }
}

fn create_stmt(s: &pb::CreateStmt, n: &Namer<'_>, out: &mut Vec<DdlEffect>) {
    let Some(rv) = &s.relation else {
        out.push(DdlEffect::opaque());
        return;
    };
    let mut referenced = Vec::new();
    // A partition or an inheritance child changes its parent's catalog rows.
    for p in &s.inh_relations {
        match p.node.as_ref() {
            Some(N::RangeVar(parent)) => push_unique(&mut referenced, n.rv(parent)),
            _ => out.push(DdlEffect::opaque()),
        }
    }
    fk_targets(&s.table_elts, n, &mut referenced);
    fk_targets(&s.constraints, n, &mut referenced);
    out.push(DdlEffect {
        name: Some(n.rv(rv)),
        referenced,
        temp: is_temp(rv),
        ..effect(DdlAction::Create, DdlKind::Table)
    });
}

/// `CREATE TABLE … AS`, `CREATE MATERIALIZED VIEW`, `SELECT … INTO`.
fn into(clause: Option<&pb::IntoClause>, kind: DdlKind, n: &Namer<'_>, out: &mut Vec<DdlEffect>) {
    match clause.and_then(|c| c.rel.as_ref()) {
        Some(rv) => out.push(DdlEffect {
            name: Some(n.rv(rv)),
            temp: is_temp(rv),
            ..effect(DdlAction::Create, kind)
        }),
        None => out.push(DdlEffect::opaque()),
    }
}

fn created_rel(rv: Option<&pb::RangeVar>, kind: DdlKind, n: &Namer<'_>, out: &mut Vec<DdlEffect>) {
    match rv {
        Some(rv) => out.push(DdlEffect {
            name: Some(n.rv(rv)),
            temp: is_temp(rv),
            ..effect(DdlAction::Create, kind)
        }),
        None => out.push(DdlEffect::opaque()),
    }
}

/// An effect on the object a list of `String`s names.
fn listed(
    action: DdlAction,
    kind: DdlKind,
    list: &[pb::Node],
    n: &Namer<'_>,
    out: &mut Vec<DdlEffect>,
) {
    match strings(list) {
        Some(parts) => out.push(named(action, kind, n.name(&parts, None))),
        None => out.push(DdlEffect::opaque()),
    }
}

/// An effect on `name ON table` (a policy, a rule).
fn attached(
    action: DdlAction,
    kind: DdlKind,
    name: &str,
    table: Option<&pb::RangeVar>,
    n: &Namer<'_>,
    out: &mut Vec<DdlEffect>,
) {
    match table {
        Some(rv) if !name.is_empty() => out.push(DdlEffect {
            name: Some(n.name(&[name], None)),
            on: Some(n.rv(rv)),
            ..effect(action, kind)
        }),
        _ => out.push(DdlEffect::opaque()),
    }
}

fn extension(action: DdlAction, name: &str, n: &Namer<'_>, out: &mut Vec<DdlEffect>) {
    out.push(DdlEffect::opaque());
    if !name.is_empty() {
        out.push(named(action, DdlKind::Extension, n.name(&[name], None)));
    }
}

fn drop_stmt(s: &pb::DropStmt, n: &Namer<'_>, out: &mut Vec<DdlEffect>) {
    let Some(kind) = kind_of(s.remove_type) else {
        out.push(DdlEffect::opaque());
        return;
    };
    if s.objects.is_empty() || kind == DdlKind::Extension {
        out.push(DdlEffect::opaque());
    }
    let action = DdlAction::Drop {
        cascade: s.behavior == pb::DropBehavior::DropCascade as i32,
    };
    for o in &s.objects {
        let e = if attached_to_a_table(kind) {
            n.on_table(o).map(|(name, table)| DdlEffect {
                name: Some(name),
                on: Some(table),
                ..effect(action.clone(), kind)
            })
        } else {
            n.object(o).map(|name| DdlEffect {
                name: Some(name),
                concurrently: s.concurrent,
                ..effect(action.clone(), kind)
            })
        };
        out.push(e.unwrap_or_else(DdlEffect::opaque));
    }
}

fn rename_stmt(s: &pb::RenameStmt, n: &Namer<'_>, out: &mut Vec<DdlEffect>) {
    use pb::ObjectType as O;
    let to = n.ident(&s.newname);
    let relation = s.relation.as_ref().map(|rv| n.rv(rv));
    let e = match O::try_from(s.rename_type) {
        // A column, a constraint or a composite type's attribute: its table (or type) changes.
        // `RENAME CONSTRAINT` leaves `relation_type` unset: it is a table's.
        Ok(O::ObjectColumn | O::ObjectTabconstraint | O::ObjectAttribute) => {
            let kind = kind_of(s.relation_type)
                .or((s.rename_type == O::ObjectTabconstraint as i32).then_some(DdlKind::Table));
            match (kind, relation) {
                (Some(kind), Some(rel)) => Some(named(DdlAction::Alter, kind, rel)),
                _ => None,
            }
        }
        Ok(O::ObjectTrigger | O::ObjectPolicy | O::ObjectRule) => {
            match (kind_of(s.rename_type), relation) {
                (Some(kind), Some(rel)) if !s.subname.is_empty() => Some(DdlEffect {
                    name: Some(n.name(&[s.subname.as_str()], None)),
                    on: Some(rel),
                    ..effect(DdlAction::Rename { to }, kind)
                }),
                _ => None,
            }
        }
        Ok(O::ObjectSchema | O::ObjectDatabase | O::ObjectRole) if !s.subname.is_empty() => {
            kind_of(s.rename_type).map(|kind| {
                named(
                    DdlAction::Rename { to },
                    kind,
                    n.name(&[s.subname.as_str()], None),
                )
            })
        }
        _ => {
            let name = relation.or_else(|| s.object.as_deref().and_then(|o| n.object(o)));
            match (kind_of(s.rename_type), name) {
                (Some(kind), Some(name)) => Some(named(DdlAction::Rename { to }, kind, name)),
                _ => None,
            }
        }
    };
    out.push(e.unwrap_or_else(DdlEffect::opaque));
}

fn alter_table(s: &pb::AlterTableStmt, n: &Namer<'_>, out: &mut Vec<DdlEffect>) {
    use pb::AlterTableType as T;
    let (Some(kind), Some(rv)) = (kind_of(s.objtype), &s.relation) else {
        out.push(DdlEffect::opaque());
        return;
    };
    let mut referenced = Vec::new();
    for c in &s.cmds {
        let Some(N::AlterTableCmd(cmd)) = c.node.as_ref() else {
            out.push(DdlEffect::opaque());
            continue;
        };
        let def = cmd.def.as_deref().and_then(|d| d.node.as_ref());
        match (T::try_from(cmd.subtype), def) {
            (Ok(T::AtAddConstraint), Some(N::Constraint(c))) => fk_target(c, n, &mut referenced),
            (Ok(T::AtAddColumn), Some(N::ColumnDef(col))) => {
                fk_targets(&col.constraints, n, &mut referenced);
            }
            (
                Ok(T::AtAttachPartition | T::AtDetachPartition | T::AtDetachPartitionFinalize),
                Some(N::PartitionCmd(p)),
            ) => match &p.name {
                Some(part) => push_unique(&mut referenced, n.rv(part)),
                None => out.push(DdlEffect::opaque()),
            },
            (Ok(T::AtAddInherit | T::AtDropInherit), Some(N::RangeVar(parent))) => {
                push_unique(&mut referenced, n.rv(parent));
            }
            _ => {}
        }
    }
    out.push(DdlEffect {
        name: Some(n.rv(rv)),
        referenced,
        ..effect(DdlAction::Alter, kind)
    });
}

fn comment(s: &pb::CommentStmt, n: &Namer<'_>, out: &mut Vec<DdlEffect>) {
    use pb::ObjectType as O;
    let Some(object) = s.object.as_deref() else {
        out.push(DdlEffect::opaque());
        return;
    };
    let e = match O::try_from(s.objtype) {
        // A column or a table constraint: its table's catalog rows.
        Ok(O::ObjectColumn | O::ObjectTabconstraint) => {
            n.on_table(object).map(|(name, table)| DdlEffect {
                name: Some(name),
                on: Some(table),
                ..effect(DdlAction::Comment, DdlKind::Other)
            })
        }
        _ => match kind_of(s.objtype) {
            Some(kind) if attached_to_a_table(kind) => {
                n.on_table(object).map(|(name, table)| DdlEffect {
                    name: Some(name),
                    on: Some(table),
                    ..effect(DdlAction::Comment, kind)
                })
            }
            Some(kind) => n
                .object(object)
                .map(|name| named(DdlAction::Comment, kind, name)),
            None => None,
        },
    };
    out.push(e.unwrap_or_else(DdlEffect::opaque));
}

fn grant(s: &pb::GrantStmt, n: &Namer<'_>, out: &mut Vec<DdlEffect>) {
    use pb::GrantTargetType as G;
    let action = if s.is_grant {
        DdlAction::Grant
    } else {
        DdlAction::Revoke
    };
    let kind = match G::try_from(s.targtype) {
        // `ON ALL TABLES IN SCHEMA s`: the objects are the schemas.
        Ok(G::AclTargetAllInSchema) => Some(DdlKind::Schema),
        Ok(G::AclTargetObject) => kind_of(s.objtype),
        _ => None,
    };
    let Some(kind) = kind else {
        out.push(DdlEffect::opaque());
        return;
    };
    if s.objects.is_empty() {
        out.push(DdlEffect::opaque());
    }
    for o in &s.objects {
        match n.object(o) {
            Some(name) => out.push(named(action.clone(), kind, name)),
            None => out.push(DdlEffect::opaque()),
        }
    }
}
