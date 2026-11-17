//! View-only DDL producers over the model (alg/03 HV-DDL-07 producers; plan-m4 Task 19).
//!
//! [`create_script`] turns a selection of objects into a [`Script`]: it lowers each object
//! from the snapshot into IR, orders the ops, places constraints (HV-DDL-09), decides
//! qualification (HV-DDL-10) and renders. It is a pure function of the snapshot, the source
//! texts, the target and the options: no I/O, no clock, no locale, no hash order.
//!
//! **Synthesized** from the model: schemas, tables (columns, defaults, generated and identity
//! columns, constraints, table clauses), indexes, sequences, PostgreSQL types, comments,
//! owners and grants. **Server text** (R-M4-24): views and materialized views, functions,
//! procedures, triggers. What the source store must hold per dialect (the introspectors'
//! contract, T16–T18) is in [`Sources`].
//!
//! Ordering (deterministic; every bucket sorted by namespace, then name bytes, then overload
//! signature): schemas, types (enum, range, domain, composite), sequences, routines, tables
//! (foreign-key topological order, cycles broken by name), postponed constraints, views,
//! materialized views, then standalone indexes, triggers and constraints. Each table carries
//! its block: the table, its indexes, `OWNED BY` of its serial sequences, its triggers, its
//! owner and its grants.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use hacivat_meta::props::*;
use hacivat_meta::{
    CatalogSnapshot, KindGroup, Level, Name, NamespaceSnapshot, NsKey, NsKind, Obj, ObjFlags,
    ObjectKey, ObjectKind, Props, Ref, SourceRef,
};
use sql_dialect::DialectId;

use crate::error::{DdlError, Notice, NoticeCode};
use crate::ir::*;
use crate::options::{DdlTarget, GenOptions, if_not_exists_supported, or_replace_supported};
use crate::render::{RenderCtx, renderer};
use crate::script::Script;

pub mod mysql;
pub mod placement;
pub mod qualify;

use placement::{PlaceInput, Placed, place};
use qualify::QualifyPlan;

/// One privilege of one grantee on one object (A-3 `Grant`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    /// PostgreSQL role name (`PUBLIC` for the public pseudo-role); MySQL account
    /// `'user'@'host'` as `information_schema` prints it.
    pub grantee: Name,
    /// Upper-case privilege keyword (`SELECT`).
    pub privilege: Name,
    /// Column privileges name their columns; empty for the whole object.
    pub columns: Vec<Name>,
    pub grantable: bool,
}

/// The grants facet of one object (A-3, R-M4-30).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum GrantsFacet {
    #[default]
    NotLoaded,
    /// The engine has no grants (SQLite).
    NotApplicable,
    /// The login could not read every grant.
    Partial(Vec<Grant>),
    Loaded(Vec<Grant>),
}

/// Source texts and facets the model keeps outside the arenas.
///
/// What [`Sources::source`] must return for an object's `SourceRef` (the introspectors'
/// contract):
///
/// | Dialect | View, materialized view | Function, procedure | Trigger |
/// |---|---|---|---|
/// | PostgreSQL | the query only: `pg_get_viewdef(oid, true)` | the whole statement: `pg_get_functiondef(oid)` | the whole statement: `pg_get_triggerdef(oid, true)` |
/// | MySQL, MariaDB | `SHOW CREATE VIEW` statement | `SHOW CREATE FUNCTION/PROCEDURE` statement | `SHOW CREATE TRIGGER` statement |
/// | SQLite | `sqlite_schema.sql` | — | `sqlite_schema.sql` |
///
/// Texts are emitted verbatim; only trailing whitespace and one trailing `;` are dropped.
pub trait Sources {
    fn source(&self, r: &SourceRef) -> Option<Name>;
    fn grants(&self, _key: &ObjectKey) -> GrantsFacet {
        GrantsFacet::NotLoaded
    }
}

/// No sources loaded, no grants loaded.
pub struct NoSources;

impl Sources for NoSources {
    fn source(&self, _: &SourceRef) -> Option<Name> {
        None
    }
}

/// The CREATE script of `objects` (alg/03 HV-DDL-07 `create_script`).
///
/// A schema in `objects` expands to every object in it. An `ObjectKind::Unknown` object is
/// refused; any other kind without a producer is left out with `ddl-unsupported-kind`.
pub fn create_script(
    snap: &CatalogSnapshot,
    sources: &dyn Sources,
    objects: &[ObjectKey],
    target: &DdlTarget,
    opts: &GenOptions,
) -> Result<Script, DdlError> {
    let mut g = Gen {
        snap,
        sources,
        target,
        opts,
        notices: Vec::new(),
        ops: Vec::new(),
        used_names: BTreeSet::new(),
    };
    let sel = g.select(objects)?;
    g.emit(&sel)?;
    let plan = QualifyPlan::new(opts.qualify, target.dialect, target.current_schema.clone())
        .with_ambiguous(g.ambiguous_names());
    let r = renderer(target.dialect).ok_or(DdlError::Unsupported {
        kind: ObjectKind::Unknown,
        dialect: target.dialect,
    })?;
    let ctx = RenderCtx::new(target, &plan);
    let mut stmts = Vec::new();
    for op in &g.ops {
        stmts.extend(r.render(op, &ctx)?);
    }
    let mut notices: Vec<Notice> = Vec::new();
    for n in g.notices {
        if !notices.contains(&n) {
            notices.push(n);
        }
    }
    Ok(Script { stmts, notices })
}

/// A sequence's `OWNED BY` op and the table that owns it (when that table is in the snapshot).
type OwnedBy = (Option<(NsKey, u32)>, DdlOp);

/// One object of the selection.
#[derive(Clone)]
struct Item {
    ns: Arc<NamespaceSnapshot>,
    idx: u32,
}

impl Item {
    fn obj(&self) -> &Obj {
        &self.ns.objs()[self.idx as usize]
    }

    fn id(&self) -> (NsKey, u32) {
        (self.ns.key().clone(), self.idx)
    }

    fn key(&self) -> ObjectKey {
        self.ns.object_key(self.idx).unwrap_or_else(|| ObjectKey {
            kind: self.obj().kind,
            path: Default::default(),
            server_id: None,
            overload_sig: None,
        })
    }

    fn sort_key(&self) -> (NsKey, Name, Option<Name>, u32) {
        let o = self.obj();
        (
            self.ns.key().clone(),
            o.name.clone(),
            o.props.overload_sig().cloned(),
            self.idx,
        )
    }
}

#[derive(Default)]
struct Selection {
    schemas: Vec<Item>,
    types: Vec<Item>,
    sequences: Vec<Item>,
    routines: Vec<Item>,
    tables: Vec<Item>,
    views: Vec<Item>,
    matviews: Vec<Item>,
    indexes: Vec<Item>,
    triggers: Vec<Item>,
    constraints: Vec<Item>,
    /// Sequences selected directly (kept even when an identity column owns them).
    direct: BTreeSet<(NsKey, u32)>,
}

struct Gen<'a> {
    snap: &'a CatalogSnapshot,
    sources: &'a dyn Sources,
    target: &'a DdlTarget,
    opts: &'a GenOptions,
    notices: Vec<Notice>,
    ops: Vec<DdlOp>,
    /// `(schema, name)` of every name the script writes (for HV-DDL-10 ambiguity).
    used_names: BTreeSet<(Option<Name>, Name)>,
}

fn is_pg(d: DialectId) -> bool {
    matches!(d, DialectId::Postgres | DialectId::PgWire(_))
}

fn is_mysql(d: DialectId) -> bool {
    matches!(d, DialectId::MySql | DialectId::MariaDb)
}

fn is_constraint(k: ObjectKind) -> bool {
    matches!(
        k,
        ObjectKind::PrimaryKey
            | ObjectKind::UniqueKey
            | ObjectKind::ForeignKey
            | ObjectKind::Check
            | ObjectKind::Exclusion
    )
}

impl<'a> Gen<'a> {
    fn d(&self) -> DialectId {
        self.target.dialect
    }

    fn notice(&mut self, code: NoticeCode, item: Option<&Item>) {
        self.notices.push(Notice {
            code,
            kind: item.map(|i| i.obj().kind),
            object: item.map(Item::key),
        });
    }

    // ---- selection ----------------------------------------------------------------------

    fn resolve(&self, key: &ObjectKey) -> Result<Item, DdlError> {
        let id = self
            .snap
            .find(key)
            .ok_or_else(|| DdlError::not_found(key.clone()))?;
        let ns = self
            .snap
            .namespace(&id.ns)
            .ok_or_else(|| DdlError::not_found(key.clone()))?
            .clone();
        Ok(Item { ns, idx: id.idx })
    }

    fn select(&mut self, objects: &[ObjectKey]) -> Result<Selection, DdlError> {
        let mut items: BTreeMap<(NsKey, u32), Item> = BTreeMap::new();
        let mut direct = BTreeSet::new();
        for key in objects {
            let it = self.resolve(key)?;
            if it.obj().kind == ObjectKind::Unknown {
                return Err(DdlError::Unsupported {
                    kind: ObjectKind::Unknown,
                    dialect: self.d(),
                });
            }
            direct.insert(it.id());
            if it.obj().kind == ObjectKind::Schema && it.idx == 0 {
                for (i, _) in it.ns.children(0) {
                    let child = Item {
                        ns: it.ns.clone(),
                        idx: i,
                    };
                    items.insert(child.id(), child);
                }
            }
            items.insert(it.id(), it);
        }
        let tables: BTreeSet<(NsKey, u32)> = items
            .values()
            .filter(|i| {
                matches!(
                    i.obj().kind,
                    ObjectKind::Table | ObjectKind::View | ObjectKind::MaterializedView
                )
            })
            .map(Item::id)
            .collect();
        let mut sel = Selection {
            direct,
            ..Selection::default()
        };
        for it in items.into_values() {
            let o = it.obj();
            // A child of a selected table is part of that table's block.
            if let Some(p) = o.parent()
                && p != 0
                && tables.contains(&(it.ns.key().clone(), p))
            {
                continue;
            }
            use ObjectKind::*;
            match o.kind {
                Schema if it.idx == 0 => sel.schemas.push(it),
                EnumType | RangeType | Domain | CompositeType if is_pg(self.d()) => {
                    sel.types.push(it)
                }
                Sequence if is_pg(self.d()) || self.d() == DialectId::MariaDb => {
                    sel.sequences.push(it)
                }
                Routine | Procedure if self.d() != DialectId::Sqlite => sel.routines.push(it),
                Table => sel.tables.push(it),
                View => sel.views.push(it),
                MaterializedView if is_pg(self.d()) => sel.matviews.push(it),
                Index => sel.indexes.push(it),
                Trigger => sel.triggers.push(it),
                k if is_constraint(k) => sel.constraints.push(it),
                _ => self.notice(NoticeCode::UnsupportedKind, Some(&it)),
            }
        }
        for v in [
            &mut sel.schemas,
            &mut sel.sequences,
            &mut sel.routines,
            &mut sel.tables,
            &mut sel.views,
            &mut sel.matviews,
            &mut sel.indexes,
            &mut sel.triggers,
            &mut sel.constraints,
        ] {
            v.sort_by_key(Item::sort_key);
        }
        let type_rank = |i: &Item| match i.obj().kind {
            ObjectKind::EnumType => 0,
            ObjectKind::RangeType => 1,
            ObjectKind::Domain => 2,
            _ => 3,
        };
        sel.types.sort_by_key(|i| (type_rank(i), i.sort_key()));
        Ok(sel)
    }

    // ---- names --------------------------------------------------------------------------

    /// The schema name of a namespace (`None` for server and database namespaces).
    fn schema_of(ns: &NamespaceSnapshot) -> Option<Name> {
        (ns.key().kind == NsKind::Schema).then(|| Name::from(ns.key().root_name()))
    }

    fn check_name(&self, it: &Item, o: &Obj) -> Result<(), DdlError> {
        if o.flags.contains(ObjFlags::LOSSY_NAME) {
            return Err(DdlError::lossy_name(
                it.ns.object_key(it.idx).unwrap_or_else(|| it.key()),
            ));
        }
        Ok(())
    }

    fn qname(&mut self, ns: &NamespaceSnapshot, name: &Name) -> QName {
        let schema = Self::schema_of(ns);
        self.used_names.insert((schema.clone(), name.clone()));
        QName {
            schema,
            name: name.clone(),
        }
    }

    fn qname_of(&mut self, it: &Item) -> Result<QName, DdlError> {
        let o = it.obj();
        self.check_name(it, o)?;
        Ok(self.qname(&it.ns, &o.name.clone()))
    }

    /// The qualified name a reference points to: its last two path elements.
    fn qname_of_ref(&mut self, r: &Ref) -> QName {
        let n = r.path.len();
        let name = r.path.last().cloned().unwrap_or_else(|| Name::from(""));
        let schema = (n >= 2).then(|| r.path[n - 2].clone());
        self.used_names.insert((schema.clone(), name.clone()));
        QName { schema, name }
    }

    /// Names written by the script that also name an object in another schema namespace of
    /// the snapshot.
    fn ambiguous_names(&self) -> Vec<Name> {
        let mut out = Vec::new();
        let groups = [
            KindGroup::Relations,
            KindGroup::Routines,
            KindGroup::Procedures,
            KindGroup::Types,
            KindGroup::Other,
        ];
        let names: BTreeSet<&Name> = self.used_names.iter().map(|(_, n)| n).collect();
        for name in names {
            let schemas = self
                .snap
                .namespaces()
                .filter(|ns| ns.key().kind == NsKind::Schema)
                .filter(|ns| groups.iter().any(|g| !ns.lookup(*g, name).is_empty()))
                .count();
            if schemas > 1 {
                out.push(name.clone());
            }
        }
        out
    }

    // ---- helpers ------------------------------------------------------------------------

    fn source(&self, it: &Item, r: Option<&SourceRef>) -> Result<Name, DdlError> {
        r.and_then(|r| self.sources.source(r))
            .ok_or_else(|| DdlError::source_not_loaded(it.key()))
    }

    fn require_structure(&self, it: &Item) -> Result<(), DdlError> {
        if it.ns.meta().level < Level::Structure {
            return Err(DdlError::source_not_loaded(it.key()));
        }
        Ok(())
    }

    fn comment(&self, c: Option<&Name>) -> Option<Name> {
        if self.d() == DialectId::Sqlite {
            None
        } else {
            c.cloned()
        }
    }

    fn flag_executable(&mut self, text: &str, it: &Item) {
        if is_mysql(self.d()) && mysql::has_executable_comment(text) {
            self.notice(NoticeCode::WarnExecutableComment, Some(it));
        }
    }

    /// MySQL / MariaDB server text with the requested head edits.
    fn mysql_text(&mut self, text: Name, it: &Item, or_replace: bool) -> Name {
        self.flag_executable(&text, it);
        let strip = self.opts.skip_definer;
        let orr =
            or_replace && self.opts.or_replace && or_replace_supported(self.target, it.obj().kind);
        match mysql::edit_head(&text, strip, orr) {
            Ok(t) => Name::from(t),
            Err(_) => {
                self.notice(NoticeCode::HeadNotEdited, Some(it));
                text
            }
        }
    }

    fn owner(&mut self, kind: ObjectKind, name: &QName, sig: Option<Name>, owner: Option<&Name>) {
        if !is_pg(self.d()) || self.opts.skip_owner {
            return;
        }
        if let Some(o) = owner {
            self.ops.push(DdlOp::Owner {
                kind,
                name: name.clone(),
                sig,
                owner: o.clone(),
            });
        }
    }

    fn grants(&mut self, it: &Item, kind: ObjectKind, name: &QName, sig: Option<Name>) {
        if self.opts.skip_grants || self.d() == DialectId::Sqlite {
            return;
        }
        let entries = match self.sources.grants(&it.key()) {
            GrantsFacet::NotApplicable => return,
            GrantsFacet::NotLoaded => {
                self.notice(NoticeCode::GrantsNotLoaded, Some(it));
                return;
            }
            GrantsFacet::Partial(e) => {
                self.notice(NoticeCode::GrantsPartial, Some(it));
                e
            }
            GrantsFacet::Loaded(e) => e,
        };
        // One statement per (grantee, grant option, columns), privileges sorted.
        let mut groups: BTreeMap<(Name, bool, Vec<Name>), BTreeSet<Name>> = BTreeMap::new();
        for e in entries {
            let mut cols = e.columns.clone();
            cols.sort();
            groups
                .entry((e.grantee.clone(), e.grantable, cols))
                .or_default()
                .insert(e.privilege.clone());
        }
        for ((grantee, grantable, columns), privs) in groups {
            self.ops.push(DdlOp::Grant(GrantDef {
                kind,
                object: name.clone(),
                sig: sig.clone(),
                privileges: privs.into_iter().collect(),
                columns,
                grantee,
                with_grant_option: grantable,
            }));
        }
    }

    // ---- emission -----------------------------------------------------------------------

    fn emit(&mut self, sel: &Selection) -> Result<(), DdlError> {
        for it in &sel.schemas {
            self.emit_schema(it)?;
        }
        for it in &sel.types {
            self.emit_type(it)?;
        }
        let table_ids: BTreeSet<(NsKey, u32)> = sel.tables.iter().map(Item::id).collect();
        // Sequences: the selected ones, plus the serial sequences of selected tables.
        let mut seqs: BTreeMap<(NsKey, u32), Item> =
            sel.sequences.iter().map(|i| (i.id(), i.clone())).collect();
        for t in &sel.tables {
            for s in self.owned_sequences(t) {
                seqs.entry(s.id()).or_insert(s);
            }
        }
        let mut seqs: Vec<Item> = seqs.into_values().collect();
        seqs.sort_by_key(Item::sort_key);
        // `OWNED BY` waits for its table when the table is in the script.
        let mut owned_by: BTreeMap<(NsKey, u32), Vec<DdlOp>> = BTreeMap::new();
        for it in &seqs {
            if let Some((owner, op)) = self.emit_sequence(it, &sel.direct)? {
                match owner {
                    Some(t) if table_ids.contains(&t) => owned_by.entry(t).or_default().push(op),
                    _ => self.ops.push(op),
                }
            }
        }
        for it in &sel.routines {
            self.emit_routine(it)?;
        }
        let order = self.table_order(&sel.tables);
        let mut created: BTreeSet<(NsKey, u32)> = BTreeSet::new();
        let mut after: Vec<DdlOp> = Vec::new();
        for it in &order {
            created.insert(it.id());
            let ops = owned_by.remove(&it.id()).unwrap_or_default();
            self.emit_table(it, &table_ids, &created, &mut after, ops)?;
        }
        self.ops.append(&mut after);
        for it in sel.views.iter().chain(&sel.matviews) {
            self.emit_view(it)?;
        }
        for it in &sel.indexes {
            if let Some(parent) = self.parent_qname(it)? {
                let def = self.lower_index(it, parent)?;
                if let Some(def) = def {
                    self.ops.push(DdlOp::CreateIndex(def));
                }
            }
        }
        for it in &sel.triggers {
            if let Some(parent) = self.parent_qname(it)? {
                let def = self.lower_trigger(it, parent)?;
                self.ops.push(DdlOp::CreateTrigger(def));
            }
        }
        for it in &sel.constraints {
            if self.d() == DialectId::Sqlite {
                self.notice(NoticeCode::UnsupportedKind, Some(it));
                continue;
            }
            if let Some(table) = self.parent_qname(it)? {
                let mut def = self.lower_constraint(it)?;
                def.position = Position::InTable;
                self.ops.push(DdlOp::AlterTable {
                    table,
                    actions: vec![AlterAction::AddConstraint(def)],
                });
            }
        }
        Ok(())
    }

    /// The qualified name of an object's parent table or view.
    fn parent_qname(&mut self, it: &Item) -> Result<Option<QName>, DdlError> {
        match it.obj().parent() {
            Some(p) if p != 0 => {
                let parent = Item {
                    ns: it.ns.clone(),
                    idx: p,
                };
                Ok(Some(self.table_qname(&parent)?))
            }
            _ => Ok(None),
        }
    }

    /// A table's name; a PostgreSQL temporary table is never schema-qualified.
    fn table_qname(&mut self, it: &Item) -> Result<QName, DdlError> {
        let mut q = self.qname_of(it)?;
        if let Some(TableExt::Pg(p)) = it.obj().props.as_table().map(|t| &t.ext)
            && p.persistence == b't'
        {
            q.schema = None;
        }
        Ok(q)
    }

    fn emit_schema(&mut self, it: &Item) -> Result<(), DdlError> {
        if self.d() == DialectId::Sqlite {
            self.notice(NoticeCode::UnsupportedKind, Some(it));
            return Ok(());
        }
        let o = it.obj().clone();
        self.check_name(it, &o)?;
        let (owner, comment) = match &o.props {
            Props::Schema(p) => (p.owner.clone(), p.comment.clone()),
            Props::Database(p) => (p.owner.clone(), p.comment.clone()),
            _ => (None, None),
        };
        let comment = if self.d() == DialectId::MySql {
            None
        } else {
            comment
        };
        self.ops.push(DdlOp::CreateSchema(SchemaDef {
            name: o.name.clone(),
            if_not_exists: self.opts.if_not_exists
                && if_not_exists_supported(self.target, ObjectKind::Schema),
            comment,
        }));
        let q = QName {
            schema: None,
            name: o.name.clone(),
        };
        self.owner(ObjectKind::Schema, &q, None, owner.as_ref());
        Ok(())
    }

    fn emit_type(&mut self, it: &Item) -> Result<(), DdlError> {
        self.require_structure(it)?;
        let name = self.qname_of(it)?;
        let o = it.obj().clone();
        let tp = match &o.props {
            Props::Type(p) => p.clone(),
            _ => return Err(DdlError::bad_props(it.key())),
        };
        let body = match o.kind {
            ObjectKind::EnumType => TypeBody::Enum {
                labels: tp.labels.clone(),
            },
            ObjectKind::Domain => TypeBody::Domain {
                base: tp
                    .base
                    .clone()
                    .ok_or_else(|| DdlError::bad_props(it.key()))?,
                collation: None,
                default: tp.default_expr.clone(),
                not_null: tp.not_null,
                checks: Vec::new(),
            },
            ObjectKind::RangeType => TypeBody::Range {
                subtype: tp
                    .base
                    .clone()
                    .ok_or_else(|| DdlError::bad_props(it.key()))?,
            },
            _ => {
                let mut attrs = Vec::new();
                for (i, c) in it.ns.children_of_kind(it.idx, ObjectKind::Column) {
                    let ci = Item {
                        ns: it.ns.clone(),
                        idx: i,
                    };
                    self.check_name(&ci, c)?;
                    let cp = c
                        .props
                        .as_column()
                        .ok_or_else(|| DdlError::bad_props(ci.key()))?;
                    attrs.push((c.name.clone(), cp.type_text.clone(), cp.collation.clone()));
                }
                TypeBody::Composite { attrs }
            }
        };
        // `TypeProps` has no owner yet (plan-m4 A-2 adds one): no `ALTER TYPE … OWNER TO`.
        self.ops.push(DdlOp::CreateType(TypeDef {
            name,
            body,
            comment: tp.comment.clone(),
        }));
        Ok(())
    }

    /// The sequences a table's non-identity columns own (PostgreSQL serials).
    fn owned_sequences(&self, table: &Item) -> Vec<Item> {
        if !is_pg(self.d()) {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (i, o) in table.ns.children_of_kind(0, ObjectKind::Sequence) {
            let Props::Sequence(sp) = &o.props else {
                continue;
            };
            let Some((r, _)) = &sp.owned_by else {
                continue;
            };
            if self
                .snap
                .resolve(r)
                .is_some_and(|id| id.ns == *table.ns.key() && id.idx == table.idx)
            {
                out.push(Item {
                    ns: table.ns.clone(),
                    idx: i,
                });
            }
        }
        out
    }

    /// The identity of the column a sequence's `OWNED BY` names, when it is an identity
    /// column (its sequence is implicit).
    fn owner_is_identity(&self, sp: &SequenceProps) -> bool {
        let Some((r, col)) = &sp.owned_by else {
            return false;
        };
        let Some(id) = self.snap.resolve(r) else {
            return false;
        };
        let Some(ns) = self.snap.namespace(&id.ns) else {
            return false;
        };
        ns.child_by_name(id.idx, ObjectKind::Column, col)
            .and_then(|c| ns.obj(c))
            .and_then(|c| c.props.as_column())
            .is_some_and(|c| matches!(c.identity, Identity::Always | Identity::ByDefault))
    }

    /// Emits a sequence; returns its `OWNED BY` op and the owning table, which the caller
    /// places.
    fn emit_sequence(
        &mut self,
        it: &Item,
        direct: &BTreeSet<(NsKey, u32)>,
    ) -> Result<Option<OwnedBy>, DdlError> {
        self.require_structure(it)?;
        let sp = match &it.obj().props {
            Props::Sequence(p) => p.clone(),
            _ => return Err(DdlError::bad_props(it.key())),
        };
        if self.owner_is_identity(&sp) && !direct.contains(&it.id()) {
            return Ok(None);
        }
        let name = self.qname_of(it)?;
        self.ops.push(DdlOp::CreateSequence(SeqDef {
            name: name.clone(),
            if_not_exists: self.opts.if_not_exists
                && if_not_exists_supported(self.target, ObjectKind::Sequence),
            data_type: sp.data_type.clone(),
            start: sp.start,
            increment: sp.increment,
            min: sp.min,
            max: sp.max,
            cache: sp.cache,
            cycle: sp.cycle,
            comment: self.comment(sp.comment.as_ref()),
        }));
        self.grants(it, ObjectKind::Sequence, &name, None);
        let Some((r, col)) = &sp.owned_by else {
            return Ok(None);
        };
        if !is_pg(self.d()) {
            return Ok(None);
        }
        let owner = self.snap.resolve(r).map(|id| (id.ns, id.idx));
        let table = self.qname_of_ref(r);
        Ok(Some((
            owner,
            DdlOp::AlterSequence {
                name,
                actions: vec![SeqAction::OwnedBy(Some((table, col.clone())))],
            },
        )))
    }

    fn routine_sig(rp: &RoutineProps) -> Name {
        if let Some(s) = &rp.overload_sig {
            return s.clone();
        }
        let args: Vec<&str> = rp
            .args
            .iter()
            .filter(|a| matches!(a.mode, ArgMode::In | ArgMode::InOut | ArgMode::Variadic))
            .map(|a| &*a.type_text)
            .collect();
        Name::from(args.join(", "))
    }

    fn emit_routine(&mut self, it: &Item) -> Result<(), DdlError> {
        let o = it.obj().clone();
        let rp = match &o.props {
            Props::Routine(p) => p.clone(),
            _ => return Err(DdlError::bad_props(it.key())),
        };
        if rp.kind == RoutineKind::Aggregate || o.kind == ObjectKind::Aggregate {
            // `pg_get_functiondef` has no aggregate form; an aggregate needs its own producer.
            self.notice(NoticeCode::UnsupportedKind, Some(it));
            return Ok(());
        }
        let name = self.qname_of(it)?;
        let text = self.source(it, rp.source.as_ref())?;
        let (text, comment) = if is_mysql(self.d()) {
            (self.mysql_text(text, it, true), None)
        } else {
            (text, rp.comment.clone())
        };
        let sig = Self::routine_sig(&rp);
        let kind = if o.kind == ObjectKind::Procedure || rp.kind == RoutineKind::Procedure {
            ObjectKind::Procedure
        } else {
            ObjectKind::Routine
        };
        self.ops.push(DdlOp::CreateRoutine(RoutineDef {
            name: name.clone(),
            kind,
            sig: Some(sig.clone()),
            text,
            comment,
        }));
        // `RoutineProps` has no owner yet (plan-m4 A-2 adds one): no `ALTER FUNCTION … OWNER`.
        self.grants(it, kind, &name, Some(sig));
        Ok(())
    }

    /// Selected tables in creation order: a table after the tables its foreign keys (and its
    /// partition parent) reference; a cycle is broken at its smallest remaining table.
    fn table_order(&self, tables: &[Item]) -> Vec<Item> {
        let ids: BTreeMap<(NsKey, u32), usize> = tables
            .iter()
            .enumerate()
            .map(|(i, t)| (t.id(), i))
            .collect();
        let mut deps: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); tables.len()];
        for (i, t) in tables.iter().enumerate() {
            let mut refs: Vec<&Ref> = Vec::new();
            for (_, c) in t.ns.children_of_kind(t.idx, ObjectKind::ForeignKey) {
                if let Props::ForeignKey(fk) = &c.props {
                    refs.push(&fk.target);
                }
            }
            if let Some(tp) = t.obj().props.as_table()
                && let Some(p) = &tp.partition_of
            {
                refs.push(p);
            }
            for r in refs {
                if let Some(id) = self.snap.resolve(r)
                    && let Some(&j) = ids.get(&(id.ns, id.idx))
                    && j != i
                {
                    deps[i].insert(j);
                }
            }
        }
        let mut done = vec![false; tables.len()];
        let mut out = Vec::with_capacity(tables.len());
        while out.len() < tables.len() {
            let next = (0..tables.len())
                .find(|&i| !done[i] && deps[i].iter().all(|&j| done[j]))
                .or_else(|| (0..tables.len()).find(|&i| !done[i]));
            let Some(i) = next else { break };
            done[i] = true;
            out.push(tables[i].clone());
        }
        out
    }

    fn lower_constraint(&mut self, it: &Item) -> Result<ConstraintDef, DdlError> {
        let o = it.obj().clone();
        self.check_name(it, &o)?;
        let server_named = o.flags.contains(ObjFlags::SERVER_GENERATED_NAME);
        let mut name = (!server_named || self.opts.surrogate_names).then(|| o.name.clone());
        if name.as_deref() == Some("") {
            name = None;
        }
        let (body, deferrable) = match (&o.props, o.kind) {
            (Props::Key(k), ObjectKind::PrimaryKey) => {
                if is_mysql(self.d()) {
                    name = None;
                }
                (
                    ConstraintBody::PrimaryKey {
                        columns: k.columns.clone(),
                    },
                    k.deferrable,
                )
            }
            (Props::Key(k), ObjectKind::UniqueKey) => (
                ConstraintBody::Unique {
                    columns: k.columns.clone(),
                },
                k.deferrable,
            ),
            (Props::ForeignKey(fk), _) => (
                ConstraintBody::ForeignKey {
                    columns: fk.columns.clone(),
                    target: self.qname_of_ref(&fk.target),
                    target_columns: fk.target_columns.clone(),
                    on_update: fk.on_update,
                    on_delete: fk.on_delete,
                    match_kind: fk.match_kind,
                },
                fk.deferrable,
            ),
            (Props::Check(c), ObjectKind::Check) => {
                self.flag_executable(&c.expr.clone(), it);
                (
                    ConstraintBody::Check {
                        expr: c.expr.clone(),
                    },
                    Deferrable::No,
                )
            }
            (Props::Check(c), ObjectKind::Exclusion) => (
                ConstraintBody::Exclusion {
                    def: c.expr.clone(),
                },
                Deferrable::No,
            ),
            _ => return Err(DdlError::bad_props(it.key())),
        };
        Ok(ConstraintDef {
            name,
            body,
            deferrable,
            not_valid: is_pg(self.d()) && o.flags.contains(ObjFlags::INVALID),
            position: Position::InTable,
        })
    }

    fn lower_column(&mut self, it: &Item) -> Result<Option<ColumnDef>, DdlError> {
        let o = it.obj().clone();
        if o.flags.contains(ObjFlags::INHERITED) {
            return Ok(None);
        }
        let cp = o
            .props
            .as_column()
            .ok_or_else(|| DdlError::bad_props(it.key()))?
            .clone();
        match &cp.ext {
            ColumnExt::Sqlite(s) if s.hidden == 1 => return Ok(None),
            ColumnExt::Pg(p) if p.attnum < 0 => return Ok(None),
            _ => {}
        }
        self.check_name(it, &o)?;
        let (type_text, on_update) = match &cp.ext {
            ColumnExt::MySql(m) => (
                m.column_type
                    .clone()
                    .unwrap_or_else(|| cp.type_text.clone()),
                m.extra.as_deref().and_then(on_update_of),
            ),
            _ => (cp.type_text.clone(), None),
        };
        let generated = match cp.generated {
            Generated::No => None,
            g => Some(GeneratedDef {
                expr: cp
                    .generation_expr
                    .clone()
                    .ok_or_else(|| DdlError::bad_props(it.key()))?,
                stored: g == Generated::Stored,
            }),
        };
        let identity = match cp.identity {
            Identity::No => None,
            Identity::Always => Some(IdentityDef::Always),
            Identity::ByDefault => Some(IdentityDef::ByDefault),
            Identity::AutoIncrement => Some(IdentityDef::AutoIncrement),
        };
        for e in [&cp.default_expr, &cp.generation_expr]
            .into_iter()
            .flatten()
        {
            self.flag_executable(&e.clone(), it);
        }
        Ok(Some(ColumnDef {
            name: o.name.clone(),
            type_text,
            collation: cp.collation.clone(),
            nullable: cp.nullable,
            default: cp.default_expr.clone(),
            generated,
            identity,
            on_update,
            comment: self.comment(cp.comment.as_ref()),
        }))
    }

    /// An index of a table, or `None` when it backs a key (the key creates it) or is the
    /// server's own (`sqlite_autoindex_*`).
    fn lower_index(&mut self, it: &Item, table: QName) -> Result<Option<IndexDef>, DdlError> {
        let o = it.obj().clone();
        if o.flags.contains(ObjFlags::INHERITED) {
            return Ok(None);
        }
        let ip = match &o.props {
            Props::Index(p) => p.clone(),
            _ => return Err(DdlError::bad_props(it.key())),
        };
        if ip.primary || o.name.starts_with("sqlite_autoindex_") {
            return Ok(None);
        }
        if let Some(p) = o.parent() {
            let backs_key = it.ns.children(p).any(|(_, k)| {
                matches!(
                    k.kind,
                    ObjectKind::PrimaryKey | ObjectKind::UniqueKey | ObjectKind::Exclusion
                ) && match &k.props {
                    Props::Key(kp) => kp.index_name.as_ref().unwrap_or(&k.name) == &o.name,
                    _ => k.name == o.name,
                }
            });
            if backs_key {
                return Ok(None);
            }
        }
        self.check_name(it, &o)?;
        let pg = is_pg(self.d());
        let parts = ip
            .parts
            .iter()
            .map(|p| IndexPartDef {
                column: p.column.clone(),
                expr: if p.column.is_some() {
                    None
                } else {
                    p.expr.clone()
                },
                desc: p.desc,
                // Written only where it differs from the direction's default (NULLS LAST
                // ascending, NULLS FIRST descending); PostgreSQL only.
                nulls_first: p.nulls_first.filter(|&nf| pg && nf != p.desc),
                prefix_len: if is_mysql(self.d()) {
                    p.prefix_len
                } else {
                    None
                },
            })
            .collect();
        Ok(Some(IndexDef {
            name: o.name.clone(),
            table,
            unique: ip.unique,
            if_not_exists: self.opts.if_not_exists
                && if_not_exists_supported(self.target, ObjectKind::Index),
            method: if self.d() == DialectId::Sqlite {
                None
            } else {
                ip.method.clone()
            },
            parts,
            include: if pg { ip.include.clone() } else { Vec::new() },
            predicate: ip.predicate.clone(),
            comment: self.comment(ip.comment.as_ref()),
        }))
    }

    fn lower_trigger(&mut self, it: &Item, table: QName) -> Result<TriggerDef, DdlError> {
        let o = it.obj().clone();
        self.check_name(it, &o)?;
        let tp = match &o.props {
            Props::Trigger(p) => p.clone(),
            _ => return Err(DdlError::bad_props(it.key())),
        };
        let mut text = self.source(it, tp.source.as_ref())?;
        if is_mysql(self.d()) {
            text = self.mysql_text(text, it, true);
        } else if is_pg(self.d())
            && self.opts.or_replace
            && or_replace_supported(self.target, ObjectKind::Trigger)
            && let Some(rest) = text.strip_prefix("CREATE TRIGGER ")
        {
            // `pg_get_triggerdef` prints exactly this head; a constraint trigger has no
            // `OR REPLACE` form and is left as it is.
            text = Name::from(format!("CREATE OR REPLACE TRIGGER {rest}"));
        }
        Ok(TriggerDef {
            name: o.name.clone(),
            table,
            text,
            comment: None,
        })
    }

    fn emit_table(
        &mut self,
        it: &Item,
        selected: &BTreeSet<(NsKey, u32)>,
        created: &BTreeSet<(NsKey, u32)>,
        after: &mut Vec<DdlOp>,
        owned_by: Vec<DdlOp>,
    ) -> Result<(), DdlError> {
        self.require_structure(it)?;
        let o = it.obj().clone();
        let tp = match &o.props {
            Props::Table(p) => p.clone(),
            Props::None => Arc::new(TableProps::default()),
            _ => return Err(DdlError::bad_props(it.key())),
        };
        if let TableExt::Sqlite(s) = &tp.ext
            && s.virtual_module.is_some()
        {
            // A virtual table's arguments are only in its server text, which the model does
            // not keep for tables.
            self.notice(NoticeCode::UnsupportedKind, Some(it));
            return Ok(());
        }
        if self.opts.server_definitions {
            self.notice(NoticeCode::ServerDefinitionUnavailable, Some(it));
        }
        let name = self.table_qname(it)?;
        let d = self.d();

        let mut columns = Vec::new();
        let mut children_unsupported = Vec::new();
        let mut keys = Vec::new();
        let mut indexes = Vec::new();
        let mut triggers = Vec::new();
        for (i, c) in it.ns.children(it.idx) {
            let ci = Item {
                ns: it.ns.clone(),
                idx: i,
            };
            match c.kind {
                ObjectKind::Column => {
                    if let Some(col) = self.lower_column(&ci)? {
                        columns.push(col);
                    }
                }
                k if is_constraint(k) => {
                    if !c.flags.contains(ObjFlags::INHERITED) {
                        keys.push(ci);
                    }
                }
                ObjectKind::Index => indexes.push(ci),
                ObjectKind::Trigger => triggers.push(ci),
                _ => children_unsupported.push(ci),
            }
        }
        let auto_column = columns
            .iter()
            .find(|c| c.identity == Some(IdentityDef::AutoIncrement))
            .map(|c| c.name.clone());
        let mut constraints = Vec::new();
        let mut postponed = Vec::new();
        for k in &keys {
            let def = self.lower_constraint(k)?;
            let target_later = match &k.obj().props {
                Props::ForeignKey(fk) => self.snap.resolve(&fk.target).is_some_and(|id| {
                    let id = (id.ns, id.idx);
                    id != it.id() && selected.contains(&id) && !created.contains(&id)
                }),
                _ => false,
            };
            let (placed, notice) = place(
                &PlaceInput {
                    body: &def.body,
                    name: def.name.as_ref(),
                    not_valid: def.not_valid,
                    target_later,
                    auto_column: auto_column.as_ref(),
                },
                self.opts.placement,
                d,
            );
            if let Some(n) = notice {
                self.notice(n, Some(k));
            }
            match placed {
                Placed::Inside(pos) => constraints.push(ConstraintDef {
                    position: pos,
                    ..def
                }),
                Placed::After => postponed.push(ConstraintDef {
                    position: Position::InTable,
                    ..def
                }),
            }
        }
        let mut index_defs = Vec::new();
        for ix in &indexes {
            if let Some(def) = self.lower_index(ix, name.clone())? {
                index_defs.push(def);
            }
        }
        let partition_of = match &tp.partition_of {
            Some(r) if is_pg(d) => Some((
                self.qname_of_ref(r),
                tp.partition_bound
                    .clone()
                    .ok_or_else(|| DdlError::bad_props(it.key()))?,
            )),
            _ => None,
        };
        let ext = match (&tp.ext, d) {
            (TableExt::Pg(p), _) if is_pg(d) => TableDefExt::Pg(PgTableDef {
                unlogged: p.persistence == b'u',
                temporary: p.persistence == b't',
                partition_of,
                partition_by: tp.partition_key.clone(),
                inherits: Vec::new(),
                access_method: p.access_method.clone(),
                with: p.options.clone(),
                tablespace: p.tablespace.clone(),
            }),
            (TableExt::MySql(m), _) if is_mysql(d) => TableDefExt::MySql(MySqlTableDef {
                engine: m.engine.clone(),
                row_format: m.row_format.clone(),
                collation: m.collation.clone(),
            }),
            (TableExt::Sqlite(s), DialectId::Sqlite) => TableDefExt::Sqlite(SqliteTableDef {
                strict: s.strict,
                without_rowid: s.without_rowid,
            }),
            _ if is_pg(d) && (partition_of.is_some() || tp.partition_key.is_some()) => {
                TableDefExt::Pg(PgTableDef {
                    partition_of,
                    partition_by: tp.partition_key.clone(),
                    ..PgTableDef::default()
                })
            }
            _ => TableDefExt::None,
        };
        // MySQL keeps secondary indexes inside the table (`KEY …`); the others create them
        // after it.
        let (inline_indexes, separate_indexes) = if is_mysql(d) {
            (index_defs, Vec::new())
        } else {
            (Vec::new(), index_defs)
        };
        self.ops.push(DdlOp::CreateTable(TableDef {
            name: name.clone(),
            if_not_exists: self.opts.if_not_exists
                && if_not_exists_supported(self.target, ObjectKind::Table),
            columns,
            constraints,
            indexes: inline_indexes,
            comment: self.comment(tp.comment.as_ref()),
            ext,
        }));
        for i in separate_indexes {
            self.ops.push(DdlOp::CreateIndex(i));
        }
        self.ops.extend(owned_by);
        for t in &triggers {
            let def = self.lower_trigger(t, name.clone())?;
            self.ops.push(DdlOp::CreateTrigger(def));
        }
        for c in &children_unsupported {
            self.notice(NoticeCode::UnsupportedKind, Some(c));
        }
        self.owner(ObjectKind::Table, &name, None, tp.owner.as_ref());
        self.grants(it, ObjectKind::Table, &name, None);
        if !postponed.is_empty() {
            after.push(DdlOp::AlterTable {
                table: name,
                actions: postponed
                    .into_iter()
                    .map(AlterAction::AddConstraint)
                    .collect(),
            });
        }
        Ok(())
    }

    fn emit_view(&mut self, it: &Item) -> Result<(), DdlError> {
        let o = it.obj().clone();
        let vp = match &o.props {
            Props::View(p) => p.clone(),
            _ => return Err(DdlError::bad_props(it.key())),
        };
        let name = self.qname_of(it)?;
        let text = self.source(it, vp.source.as_ref())?;
        let materialized = o.kind == ObjectKind::MaterializedView;
        let d = self.d();
        let body = if is_pg(d) {
            ViewBody::Query(text)
        } else if is_mysql(d) {
            ViewBody::ServerText(self.mysql_text(text, it, true))
        } else {
            ViewBody::ServerText(text)
        };
        let mut column_comments = Vec::new();
        if is_pg(d) {
            for (i, c) in it.ns.children_of_kind(it.idx, ObjectKind::Column) {
                let ci = Item {
                    ns: it.ns.clone(),
                    idx: i,
                };
                self.check_name(&ci, c)?;
                if let Some(cm) = c.props.comment() {
                    column_comments.push((c.name.clone(), cm.clone()));
                }
            }
        }
        let kind = if materialized {
            ObjectKind::MaterializedView
        } else {
            ObjectKind::View
        };
        self.ops.push(DdlOp::CreateView(ViewDef {
            name: name.clone(),
            kind: if materialized {
                ViewKind::Materialized
            } else {
                ViewKind::View
            },
            or_replace: is_pg(d) && self.opts.or_replace && or_replace_supported(self.target, kind),
            body,
            check_option: if is_pg(d) {
                vp.check_option.clone()
            } else {
                None
            },
            comment: if is_pg(d) { vp.comment.clone() } else { None },
            column_comments,
        }));
        for (i, c) in it.ns.children(it.idx) {
            let ci = Item {
                ns: it.ns.clone(),
                idx: i,
            };
            match c.kind {
                ObjectKind::Index => {
                    if let Some(def) = self.lower_index(&ci, name.clone())? {
                        self.ops.push(DdlOp::CreateIndex(def));
                    }
                }
                ObjectKind::Trigger => {
                    let def = self.lower_trigger(&ci, name.clone())?;
                    self.ops.push(DdlOp::CreateTrigger(def));
                }
                ObjectKind::Column => {}
                _ => self.notice(NoticeCode::UnsupportedKind, Some(&ci)),
            }
        }
        self.owner(kind, &name, None, vp.owner.as_ref());
        self.grants(it, kind, &name, None);
        Ok(())
    }
}

/// The expression of MySQL's `EXTRA` `on update …` clause (`DEFAULT_GENERATED on update
/// CURRENT_TIMESTAMP(3)` → `CURRENT_TIMESTAMP(3)`).
fn on_update_of(extra: &str) -> Option<Name> {
    const KEY: &[u8] = b"on update ";
    let b = extra.as_bytes();
    (0..b.len().saturating_sub(KEY.len() - 1))
        .find(|&i| b[i..].len() >= KEY.len() && b[i..i + KEY.len()].eq_ignore_ascii_case(KEY))
        .map(|i| Name::from(extra[i + KEY.len()..].trim()))
        .filter(|e| !e.is_empty())
}

#[cfg(test)]
mod tests {
    use super::on_update_of;

    #[test]
    fn on_update_is_read_from_extra() {
        assert_eq!(
            on_update_of("DEFAULT_GENERATED on update CURRENT_TIMESTAMP(3)").as_deref(),
            Some("CURRENT_TIMESTAMP(3)")
        );
        assert_eq!(on_update_of("auto_increment"), None);
        assert_eq!(on_update_of(""), None);
    }
}
