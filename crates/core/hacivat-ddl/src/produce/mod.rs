//! Producers (HV-DDL-07): model objects to IR to a script, for view-only use in M4 (the DDL
//! view, copy, open in console). M4 Task 19 body (spike origin `da358e4`, adapted to the
//! merged model, `ModelView` and the T7 renderers).
//!
//! [`create_script_with`] is a pure function of the view, the [`DdlSources`], the target and the
//! options: no I/O, no clock, no locale, no hash order (every bucket is sorted by namespace,
//! name bytes and overload signature; maps are `BTreeMap`s).
//!
//! - **Synthesized** from the model (R-M4-24): schemas, tables (columns in model order without
//!   inherited ones; constraints per placement; table clauses), indexes, sequences, PostgreSQL
//!   types, comments, owners (`Props.owner`) and grants (the `Grants` facet).
//! - **Server text**, verbatim: views and materialized views, functions, procedures and
//!   triggers; PostgreSQL indexes with an expression, a predicate or `INCLUDE` when the
//!   `Sources` facet holds `pg_get_indexdef`; MySQL/MariaDB tables under
//!   `server_definitions` (`SHOW CREATE TABLE`); **every SQLite table, index, view and
//!   trigger** (`sqlite_schema.sql`: CHECK and generated expressions exist only there, F10;
//!   SQLite ignores `server_definitions`, it is always on, carries M8 of the T7 review).
//! - **Fail closed** (F31): an `ObjectKind::Unknown` object is `DdlError::UnsupportedKind`; an
//!   object the view does not hold `Found` in a complete namespace, a definition text that is
//!   not loaded, a lossy name or props of the wrong shape are `DdlError::NotLoaded`; a grants
//!   facet that is not loaded gives `ddl-grants-not-loaded`, never a script that silently lacks
//!   grants; a kind with no producer is left out with `ddl-unsupported-kind`; a NUL anywhere in
//!   the output is refused.
//! - **Virtual objects are never emitted**: virtual foreign keys live in the view's overlays,
//!   never in the arenas this module reads.
//!
//! Ordering: schemas, types (enum, range, domain, composite), sequences, routines, tables
//! (foreign-key and partition-parent topological order; a cycle is broken at its smallest
//! remaining table), constraints postponed past later tables, views, materialized views, then
//! standalone indexes, triggers and constraints. Each table carries its block: the table (with
//! its comments), `ENABLE ROW LEVEL SECURITY`, its indexes, the `OWNED BY` of its serial
//! sequences, its triggers, its owner and its grants.

pub mod mysql;
pub mod pg;
pub mod placement;
pub mod qualify;
pub mod sqlite;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use hacivat_meta::ids::{NsKey, NsKind, ObjectKey, Ref, SourceRef};
use hacivat_meta::kind::{KindGroup, ObjectKind};
use hacivat_meta::name::Name;
use hacivat_meta::namespace::{NamespaceSnapshot, Obj, ObjFlags};
use hacivat_meta::props::{
    CheckProps, ColumnExt, ColumnProps, Facet, FkProps, GrantsFacet, Identity, IndexProps,
    KeyProps, Persistence, Props, RoutineProps, SeqProps, TableExt, TableProps,
};
use hacivat_meta::snapshot::{LoadState, ObjHandle};
use hacivat_meta::view::{Lookup, ModelView};
use hacivat_proto::DbmsKind;
use smallvec::SmallVec;
use sql_catalog::types::{ArgMode, FnKind};
use sql_dialect::DialectId;
use sql_ident::QualifiedName;
use sql_ident::ident::Ident;
use sql_ident::quote::QuoteError;

use crate::ir::*;
use crate::options::{GenOptions, Placement, Qualify};
use crate::render::{RenderCtx, RenderError, Stmt, renderer};
use crate::script::{Notice, Script};
use crate::{DdlError, DdlTarget};

use qualify::{QualifyCtx, qualify};

/// `ddl-unsupported-kind` (args `dbms`, `kind`): a kind with no producer, left out.
pub const UNSUPPORTED_KIND: &str = "ddl-unsupported-kind";
/// `ddl-grants-not-loaded`: the grants facet is not loaded and grants are not skipped.
pub const GRANTS_NOT_LOADED: &str = "ddl-grants-not-loaded";
/// `ddl-grants-partial`: the login could not read every grant.
pub const GRANTS_PARTIAL: &str = "ddl-grants-partial";
/// `ddl-policies-not-loaded`: a PostgreSQL table with row security; policies are not
/// introspected in M4.
pub const POLICIES_NOT_LOADED: &str = "ddl-policies-not-loaded";
/// `ddl-warn-executable-comment`: MySQL/MariaDB text holding `/*!` (it runs as SQL).
pub const WARN_EXECUTABLE_COMMENT: &str = "ddl-warn-executable-comment";
pub use placement::SQLITE_PLACEMENT_FORCED;

/// What the model keeps outside the arenas: source texts and facets (R-M4-30). The caller
/// (the DDL view, Task 25) backs it with the cache's content-addressed source store and the
/// facets it loaded; every method defaults to "not loaded".
///
/// The texts are the introspectors' contract (T16–T18), emitted verbatim (only trailing
/// whitespace and one trailing `;` are dropped):
///
/// | Dialect | View, materialized view | Function, procedure | Trigger | Table, index |
/// |---|---|---|---|---|
/// | PostgreSQL | `pg_get_viewdef(oid, true)` (the query only) | `pg_get_functiondef(oid)` | `pg_get_triggerdef(oid, true)` | index: `pg_get_indexdef(oid)` (`server_text`) |
/// | MySQL, MariaDB | `SHOW CREATE VIEW` statement | `SHOW CREATE FUNCTION/PROCEDURE` statement | `SHOW CREATE TRIGGER` statement | table: `SHOW CREATE TABLE` (`server_text`) |
/// | SQLite | `sqlite_schema.sql` | — | `sqlite_schema.sql` | `sqlite_schema.sql` (`server_text`) |
pub trait DdlSources {
    /// The text a `SourceRef` of the model's props names (views, routines, triggers).
    fn text(&self, r: &SourceRef) -> Option<Name> {
        let _ = r;
        None
    }

    /// The `Sources` facet of `object`: server text whose props hold no `SourceRef` (the last
    /// column of the table above). Also consulted for a view, routine or trigger whose
    /// `SourceRef` text is not at hand.
    fn server_text(&self, object: &ObjectKey) -> Option<Name> {
        let _ = object;
        None
    }

    /// The `Grants` facet of `object`.
    fn grants(&self, object: &ObjectKey) -> Facet<GrantsFacet> {
        let _ = object;
        Facet::not_loaded()
    }
}

/// No source text and no facet loaded.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSources;

impl DdlSources for NoSources {}

/// The CREATE script of `objects` as `view` holds them, with no source text and no facet
/// loaded ([`NoSources`]): tables, indexes, sequences, types and schemas only, with the
/// notice `ddl-grants-not-loaded`; a view, routine or trigger is `DdlError::NotLoaded`. The
/// DDL view calls [`create_script_with`].
pub fn create_script(
    view: &dyn ModelView<Def = ObjHandle>,
    objects: &[ObjectKey],
    target: &DdlTarget,
    opts: &GenOptions,
) -> Result<Script, DdlError> {
    create_script_with(view, &NoSources, objects, target, opts)
}

/// The CREATE script of `objects` (alg/03 HV-DDL-07 `create_script`). A schema in `objects`
/// expands to every object in it; an object's children (columns, keys, indexes, triggers)
/// belong to its block.
pub fn create_script_with(
    view: &dyn ModelView<Def = ObjHandle>,
    sources: &dyn DdlSources,
    objects: &[ObjectKey],
    target: &DdlTarget,
    opts: &GenOptions,
) -> Result<Script, DdlError> {
    let d = target.dialect;
    if !matches!(
        d,
        DialectId::Postgres | DialectId::MySql | DialectId::MariaDb | DialectId::Sqlite
    ) {
        // No renderer for this dialect in M4 (SQL Server, ClickHouse, Oracle, the PG-wire
        // forks, generic).
        return Err(DdlError::UnsupportedKind);
    }
    let mut g = Gen {
        view,
        sources,
        target,
        opts,
        dbms: dbms_of(d),
        qctx: QualifyCtx::for_target(target, opts.qualify),
        ambiguous: BTreeMap::new(),
        schema_lists: BTreeMap::new(),
        notices: Vec::new(),
        steps: Vec::new(),
    };
    let sel = g.select(objects)?;
    g.emit(&sel)?;
    let ctx = RenderCtx::new(target.clone(), *opts);
    let r = renderer(d);
    let mut stmts: Vec<Stmt> = Vec::new();
    for step in std::mem::take(&mut g.steps) {
        match step {
            Step::Op(op) => stmts.extend(r.render(&op, &ctx).map_err(DdlError::Render)?),
            Step::OwnedBy { seq, table, column } => {
                stmts.push(pg::owned_by(&ctx, &seq, &table, &column).map_err(DdlError::Render)?);
            }
            Step::IndexOption { op, word } => {
                let mut v = r.render(&op, &ctx).map_err(DdlError::Render)?;
                if let Some(first) = v.first_mut() {
                    first.text.push(' ');
                    first.text.push_str(word);
                }
                stmts.extend(v);
            }
        }
    }
    if stmts.iter().any(|s| s.text.contains('\0')) {
        return Err(DdlError::Quote(QuoteError::Nul));
    }
    if is_mysql(d) && stmts.iter().any(|s| mysql::has_executable_comment(&s.text)) {
        g.notices.push(Notice::new(WARN_EXECUTABLE_COMMENT));
    }
    let mut notices: Vec<Notice> = Vec::new();
    for n in g.notices {
        if !notices.contains(&n) {
            notices.push(n);
        }
    }
    Ok(Script { stmts, notices })
}

/// One step of a script: an IR op, or a statement the IR has no variant for yet (T7 carry:
/// `ALTER SEQUENCE … OWNED BY`; a MySQL index visibility option).
#[derive(Debug, Clone)]
enum Step {
    Op(DdlOp),
    /// PostgreSQL `ALTER SEQUENCE seq OWNED BY table.column`.
    OwnedBy {
        seq: QualifiedName,
        table: QualifiedName,
        column: Name,
    },
    /// A MySQL `CREATE INDEX` followed by an index option keyword (`INVISIBLE`, MariaDB
    /// `IGNORED`).
    IndexOption {
        op: DdlOp,
        word: &'static str,
    },
}

/// One object of one generation: its namespace and arena index.
#[derive(Clone)]
struct Item {
    ns: Arc<NamespaceSnapshot>,
    idx: u32,
}

impl Item {
    fn from_handle(h: &ObjHandle) -> Self {
        Item {
            ns: h.ns.clone(),
            idx: h.id.idx,
        }
    }

    fn at(&self, idx: u32) -> Item {
        Item {
            ns: self.ns.clone(),
            idx,
        }
    }

    fn obj(&self) -> &Obj {
        &self.ns.objs()[self.idx as usize]
    }

    fn kind(&self) -> ObjectKind {
        self.obj().kind()
    }

    fn name(&self) -> Name {
        self.obj().name().clone()
    }

    fn props(&self) -> Props {
        self.obj().props().clone()
    }

    fn flags(&self) -> ObjFlags {
        self.obj().flags()
    }

    fn key(&self) -> ObjectKey {
        self.ns.object_key(self.idx).unwrap_or_else(|| ObjectKey {
            kind: self.kind(),
            path: SmallVec::new(),
            server_id: None,
            overload_sig: None,
            raw_kind: None,
        })
    }

    fn id(&self) -> (NsKey, u32) {
        (self.ns.key().clone(), self.idx)
    }

    /// Namespace, name bytes, overload signature, arena index: the bucket order.
    fn sort_key(&self) -> (NsKey, Name, Option<Name>, u32) {
        (
            self.ns.key().clone(),
            self.name(),
            self.obj().props().overload_sig().cloned(),
            self.idx,
        )
    }

    /// The parent object (a table, a view), when the object is not at the namespace root.
    fn parent(&self) -> Option<Item> {
        match self.obj().parent() {
            Some(p) if p != 0 => Some(self.at(p)),
            _ => None,
        }
    }

    fn children(&self) -> Vec<Item> {
        self.ns
            .children(self.idx)
            .map(|(i, _)| self.at(i))
            .collect()
    }

    /// The schema this object's namespace is (`None` outside a schema namespace).
    fn schema(&self) -> Option<Name> {
        let k = self.ns.key();
        (k.kind == NsKind::Schema)
            .then(|| k.path.last().cloned())
            .flatten()
    }

    /// The database of a PostgreSQL schema namespace `[db, schema]`.
    fn database(&self) -> Option<Name> {
        let k = self.ns.key();
        (k.kind == NsKind::Schema && k.path.len() == 2).then(|| k.path[0].clone())
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
    /// Objects named in `objects` themselves (an implicit sequence is kept when selected).
    direct: BTreeSet<(NsKey, u32)>,
}

struct Gen<'a> {
    view: &'a dyn ModelView<Def = ObjHandle>,
    sources: &'a dyn DdlSources,
    target: &'a DdlTarget,
    opts: &'a GenOptions,
    dbms: DbmsKind,
    qctx: QualifyCtx,
    /// Whether a name exists in more than one schema of its database (HV-DDL-10 `Auto`).
    ambiguous: BTreeMap<(Option<Name>, Name), bool>,
    /// The schema arenas of a database (`None`: the list is not loaded).
    schema_lists: BTreeMap<Option<Name>, Option<Vec<Arc<NamespaceSnapshot>>>>,
    notices: Vec<Notice>,
    steps: Vec<Step>,
}

fn is_pg(d: DialectId) -> bool {
    matches!(d, DialectId::Postgres | DialectId::PgWire(_))
}

fn is_mysql(d: DialectId) -> bool {
    matches!(d, DialectId::MySql | DialectId::MariaDb)
}

fn dbms_of(d: DialectId) -> DbmsKind {
    match d {
        DialectId::MySql => DbmsKind::MySql,
        DialectId::MariaDb => DbmsKind::MariaDb,
        DialectId::Sqlite => DbmsKind::Sqlite,
        _ => DbmsKind::Postgres,
    }
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

fn ident(text: &Name) -> Ident {
    Ident {
        text: text.clone(),
        quoted: false,
    }
}

fn bare(name: &Name) -> QualifiedName {
    QualifiedName {
        parts: std::iter::once(ident(name)).collect(),
    }
}

/// The key a reference denotes (for matching against selected objects).
fn key_of_ref(r: &Ref) -> ObjectKey {
    ObjectKey {
        kind: r.kind,
        path: r.path.clone(),
        server_id: r.server_id,
        overload_sig: None,
        raw_kind: None,
    }
}

/// `Deferrable` from the model's two flags.
fn deferrable(deferrable: bool, initially_deferred: bool) -> Deferrable {
    match (deferrable, initially_deferred) {
        (false, _) => Deferrable::No,
        (true, false) => Deferrable::InitiallyImmediate,
        (true, true) => Deferrable::InitiallyDeferred,
    }
}

/// The argument type list naming a routine (PostgreSQL `COMMENT ON`, owner, grants): the
/// stored overload signature, else the input argument types.
fn routine_sig(rp: &RoutineProps) -> Option<Name> {
    if let Some(s) = &rp.overload_sig {
        return Some(s.clone());
    }
    let args = rp.args.as_ref()?;
    let types: Vec<&str> = args
        .iter()
        .filter(|a| matches!(a.mode, ArgMode::In | ArgMode::InOut | ArgMode::Variadic))
        .map(|a| &*a.type_name)
        .collect();
    Some(Name::from(types.join(", ")))
}

impl<'a> Gen<'a> {
    fn d(&self) -> DialectId {
        self.target.dialect
    }

    fn pg(&self) -> bool {
        is_pg(self.d())
    }

    fn mysql(&self) -> bool {
        is_mysql(self.d())
    }

    fn sqlite(&self) -> bool {
        self.d() == DialectId::Sqlite
    }

    fn unsupported(&mut self, kind: ObjectKind) {
        let dbms = match self.d() {
            DialectId::MySql => "MySQL",
            DialectId::MariaDb => "MariaDB",
            DialectId::Sqlite => "SQLite",
            _ => "PostgreSQL",
        };
        self.notices.push(Notice {
            id: UNSUPPORTED_KIND,
            args: vec![("dbms", dbms.to_string()), ("kind", kind.id().to_string())],
        });
    }

    fn op(&mut self, op: DdlOp) {
        self.steps.push(Step::Op(op));
    }

    // ---- finding objects --------------------------------------------------------------------

    /// The parent key of the schema list a schema key `[…, schema]` sits in.
    fn schema_parent(&self, db: Option<&Name>) -> ObjectKey {
        let (kind, path): (ObjectKind, SmallVec<[Name; 4]>) = match db {
            Some(db) if self.pg() => (ObjectKind::Database, std::iter::once(db.clone()).collect()),
            _ => (ObjectKind::Root, SmallVec::new()),
        };
        ObjectKey {
            kind,
            path,
            server_id: None,
            overload_sig: None,
            raw_kind: None,
        }
    }

    /// `key`'s object, from a namespace the view holds complete (`Found`); anything else is
    /// `NotLoaded` (an object the view cannot vouch for is never generated).
    fn locate(&self, key: &ObjectKey) -> Result<Item, DdlError> {
        let n = key.path.len();
        if n == 0 {
            return Err(DdlError::NotLoaded);
        }
        if key.kind == ObjectKind::Schema {
            let db = (self.pg() && n == 2).then(|| &key.path[0]);
            let parent = self.schema_parent(db);
            if let Lookup::Found(hs) = self.view.children(&parent, ObjectKind::Schema) {
                for h in hs {
                    if h.ns.key().kind == NsKind::Schema
                        && h.ns.key().path.as_slice() == key.path.as_slice()
                    {
                        return Ok(Item::from_handle(&h));
                    }
                }
            }
            return Err(DdlError::NotLoaded);
        }
        let parent_path: SmallVec<[Name; 4]> = key.path[..n - 1].iter().cloned().collect();
        for &pk in ObjectKind::ALL {
            if !pk.allows_child(key.kind, self.dbms) {
                continue;
            }
            let parent = ObjectKey {
                kind: pk,
                path: parent_path.clone(),
                server_id: None,
                overload_sig: None,
                raw_kind: None,
            };
            if let Lookup::Found(hs) = self.view.children(&parent, key.kind) {
                for h in hs {
                    if h.key().is_some_and(|k| k.same_object(key)) {
                        return Ok(Item::from_handle(&h));
                    }
                }
            }
        }
        Err(DdlError::NotLoaded)
    }

    fn select(&mut self, objects: &[ObjectKey]) -> Result<Selection, DdlError> {
        let mut items: BTreeMap<(NsKey, u32), Item> = BTreeMap::new();
        let mut direct = BTreeSet::new();
        for key in objects {
            if key.kind == ObjectKind::Unknown {
                return Err(DdlError::UnsupportedKind);
            }
            let it = self.locate(key)?;
            if it.kind() == ObjectKind::Unknown {
                return Err(DdlError::UnsupportedKind);
            }
            direct.insert(it.id());
            if it.idx == 0 && it.kind() == ObjectKind::Schema {
                for c in it.children() {
                    // System objects (FTS5 shadow tables, catalog objects) are the server's.
                    // An unknown kind inside the schema is left out with a notice below.
                    if !c.flags().contains(ObjFlags::SYSTEM) {
                        items.insert(c.id(), c);
                    }
                }
            }
            items.insert(it.id(), it);
        }
        let blocks: BTreeSet<(NsKey, u32)> = items
            .values()
            .filter(|i| {
                matches!(
                    i.kind(),
                    ObjectKind::Table | ObjectKind::View | ObjectKind::MaterializedView
                )
            })
            .map(Item::id)
            .collect();
        let mut sel = Selection {
            direct,
            ..Selection::default()
        };
        let d = self.d();
        for it in items.into_values() {
            // A child of a selected table or view is part of that object's block.
            if let Some(p) = it.parent()
                && blocks.contains(&p.id())
            {
                continue;
            }
            use ObjectKind::*;
            match it.kind() {
                Schema if it.idx == 0 => sel.schemas.push(it),
                EnumType | RangeType | Domain | CompositeType if is_pg(d) => sel.types.push(it),
                Sequence if is_pg(d) || d == DialectId::MariaDb => sel.sequences.push(it),
                Routine | Procedure | Aggregate if d != DialectId::Sqlite => {
                    sel.routines.push(it);
                }
                Table => sel.tables.push(it),
                // `CREATE VIRTUAL TABLE … USING module(…)`: its server text.
                VirtualTable if d == DialectId::Sqlite => sel.tables.push(it),
                View => sel.views.push(it),
                MaterializedView if is_pg(d) => sel.matviews.push(it),
                Index => sel.indexes.push(it),
                Trigger => sel.triggers.push(it),
                k if is_constraint(k) => sel.constraints.push(it),
                k => self.unsupported(k),
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
        let rank = |i: &Item| match i.kind() {
            ObjectKind::EnumType => 0,
            ObjectKind::RangeType => 1,
            ObjectKind::Domain => 2,
            _ => 3,
        };
        sel.types.sort_by_key(|i| (rank(i), i.sort_key()));
        Ok(sel)
    }

    // ---- names (HV-DDL-10) --------------------------------------------------------------------

    /// The schema arenas of `db`'s schema list (`None` when the view cannot list them).
    fn schema_arenas(&mut self, db: Option<&Name>) -> Option<Vec<Arc<NamespaceSnapshot>>> {
        let k = db.cloned();
        if let Some(v) = self.schema_lists.get(&k) {
            return v.clone();
        }
        let parent = self.schema_parent(db);
        let list = match self.view.children(&parent, ObjectKind::Schema) {
            Lookup::Found(hs) => Some(hs.iter().map(|h| h.ns.clone()).collect()),
            _ => None,
        };
        self.schema_lists.insert(k, list.clone());
        list
    }

    /// Whether `name` names objects in more than one schema of `db`; `true` when the schema
    /// list is not known (qualifying more is the safe side).
    fn is_ambiguous(&mut self, db: Option<&Name>, name: &Name) -> bool {
        let k = (db.cloned(), name.clone());
        if let Some(&a) = self.ambiguous.get(&k) {
            return a;
        }
        const GROUPS: [KindGroup; 5] = [
            KindGroup::Relations,
            KindGroup::Routines,
            KindGroup::Procedures,
            KindGroup::Types,
            KindGroup::Other,
        ];
        let a = match self.schema_arenas(db) {
            Some(arenas) => {
                arenas
                    .iter()
                    .filter(|ns| GROUPS.iter().any(|g| !ns.lookup(*g, name).is_empty()))
                    .count()
                    > 1
            }
            None => true,
        };
        self.ambiguous.insert(k, a);
        a
    }

    /// `schema.name` as the qualification mode wants it written.
    fn qname(&mut self, db: Option<&Name>, schema: Option<&Name>, name: &Name) -> QualifiedName {
        let full = QualifiedName {
            parts: schema.into_iter().chain([name]).map(ident).collect(),
        };
        if self.opts.qualify == Qualify::Auto
            && schema.is_some()
            && !self.qctx.ambiguous.contains(name)
            && self.is_ambiguous(db, name)
        {
            self.qctx.ambiguous.insert(name.clone());
        }
        qualify(&full, &self.qctx).to_qualified_name()
    }

    /// An object's own name, refused when the model stored it lossily.
    fn checked_name(it: &Item) -> Result<Name, DdlError> {
        if it.flags().contains(ObjFlags::LOSSY_NAME) {
            return Err(DdlError::NotLoaded);
        }
        Ok(it.name())
    }

    fn qname_of(&mut self, it: &Item) -> Result<QualifiedName, DdlError> {
        let name = Self::checked_name(it)?;
        let (db, schema) = (it.database(), it.schema());
        Ok(self.qname(db.as_ref(), schema.as_ref(), &name))
    }

    /// A table's name; a PostgreSQL temporary table is never schema-qualified.
    fn table_qname(&mut self, it: &Item) -> Result<QualifiedName, DdlError> {
        let temp =
            matches!(it.obj().props(), Props::Table(t) if t.persistence == Persistence::Temporary);
        if temp && self.pg() {
            return Ok(bare(&Self::checked_name(it)?));
        }
        self.qname_of(it)
    }

    /// The name a reference points to: its last two path elements (`schema.name`).
    fn qname_of_ref(&mut self, r: &Ref) -> QualifiedName {
        let n = r.path.len();
        let name = r.path.last().cloned().unwrap_or_else(|| Name::from(""));
        let schema = (n >= 2).then(|| r.path[n - 2].clone());
        let db = (self.pg() && n >= 3).then(|| r.path[0].clone());
        self.qname(db.as_ref(), schema.as_ref(), &name)
    }

    /// The name of an object's parent table or view.
    fn parent_qname(&mut self, it: &Item) -> Result<Option<QualifiedName>, DdlError> {
        match it.parent() {
            Some(p) => Ok(Some(self.table_qname(&p)?)),
            None => Ok(None),
        }
    }

    // ---- shared pieces ----------------------------------------------------------------------

    /// The definition text of `it`: its `SourceRef`'s text, else its `Sources` facet.
    fn source(&self, it: &Item, r: Option<&SourceRef>) -> Result<Name, DdlError> {
        r.and_then(|r| self.sources.text(r))
            .or_else(|| self.sources.server_text(&it.key()))
            .ok_or(DdlError::NotLoaded)
    }

    /// A comment as the dialect keeps it (SQLite has none).
    fn comment(&self, c: Option<&Name>) -> Option<Name> {
        if self.sqlite() { None } else { c.cloned() }
    }

    fn owner(
        &mut self,
        kind: ObjectKind,
        name: &QualifiedName,
        sig: Option<Name>,
        owner: Option<&Name>,
    ) {
        if !self.pg() || self.opts.skip_owner {
            return;
        }
        if let Some(o) = owner {
            self.op(DdlOp::Owner {
                kind,
                name: name.clone(),
                sig,
                owner: o.clone(),
            });
        }
    }

    /// The `GRANT`s of `it` from its grants facet: one statement per grantee, grant option
    /// and column list, privileges sorted. Not loaded (and not skipped) → the notice, never
    /// silence; partial → the grants read plus the notice.
    fn grants(&mut self, it: &Item, kind: ObjectKind, name: &QualifiedName, sig: Option<Name>) {
        if self.opts.skip_grants || self.sqlite() {
            return;
        }
        let facet = self.sources.grants(&it.key());
        let entries = match facet.state {
            LoadState::Loaded { .. } => facet.value,
            LoadState::Partial { .. } => {
                self.notices.push(Notice::new(GRANTS_PARTIAL));
                facet.value
            }
            _ => {
                self.notices.push(Notice::new(GRANTS_NOT_LOADED));
                return;
            }
        };
        let Some(entries) = entries else { return };
        // MariaDB sequences are tables to the privilege system.
        let kind = if self.mysql() && kind == ObjectKind::Sequence {
            ObjectKind::Table
        } else {
            kind
        };
        let mut groups: BTreeMap<(Name, bool, Vec<Name>), BTreeSet<Name>> = BTreeMap::new();
        for e in &entries.entries {
            let mut cols = e.columns.clone();
            cols.sort();
            groups
                .entry((e.grantee.clone(), e.grantable, cols))
                .or_default()
                .insert(e.privilege.clone());
        }
        for ((grantee, grantable, columns), privs) in groups {
            self.op(DdlOp::Grant(GrantDef {
                kind,
                object: name.clone(),
                sig: sig.clone(),
                privileges: privs.into_iter().collect(),
                columns,
                grantee,
                grantable,
            }));
        }
    }

    // ---- emission ---------------------------------------------------------------------------

    fn emit(&mut self, sel: &Selection) -> Result<(), DdlError> {
        for it in &sel.schemas {
            self.emit_schema(it)?;
        }
        for it in &sel.types {
            self.emit_type(it)?;
        }
        let table_ids: BTreeSet<(NsKey, u32)> = sel.tables.iter().map(Item::id).collect();
        // Sequences: the selected ones plus the serial sequences of selected tables.
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
        let mut owned_by: BTreeMap<(NsKey, u32), Vec<Step>> = BTreeMap::new();
        for it in &seqs {
            if let Some((owner, step)) = self.emit_sequence(it, &sel.direct)? {
                match owner {
                    Some(t) if table_ids.contains(&t) => owned_by.entry(t).or_default().push(step),
                    _ => self.steps.push(step),
                }
            }
        }
        for it in &sel.routines {
            self.emit_routine(it)?;
        }
        let order = self.table_order(&sel.tables);
        let mut created: BTreeSet<(NsKey, u32)> = BTreeSet::new();
        let mut after: Vec<Step> = Vec::new();
        if self.sqlite() && !sel.tables.is_empty() && self.opts.placement != Placement::InsideTable
        {
            // SQLite tables are their server text: constraints stay where the server keeps
            // them, inside the table.
            self.notices.push(Notice::new(SQLITE_PLACEMENT_FORCED));
        }
        for it in &order {
            created.insert(it.id());
            let steps = owned_by.remove(&it.id()).unwrap_or_default();
            self.emit_table(it, &sel.tables, &created, &mut after, steps)?;
        }
        self.steps.append(&mut after);
        for it in sel.views.iter().chain(&sel.matviews) {
            self.emit_view(it)?;
        }
        for it in &sel.indexes {
            if let Some(table) = self.parent_qname(it)?
                && let Some(step) = self.lower_index(it, &table)?
            {
                self.steps.push(step);
            }
        }
        for it in &sel.triggers {
            self.emit_trigger(it)?;
        }
        for it in &sel.constraints {
            if self.sqlite() {
                // Only the table's server text holds a SQLite constraint.
                self.unsupported(it.kind());
                continue;
            }
            if let Some(table) = self.parent_qname(it)? {
                let def = self.lower_constraint(it)?;
                self.op(DdlOp::AlterTable {
                    table,
                    actions: vec![AlterAction::AddConstraint(def)],
                });
            }
        }
        Ok(())
    }

    fn emit_schema(&mut self, it: &Item) -> Result<(), DdlError> {
        if self.sqlite() {
            // `main`, `temp` and attached files are not created by DDL.
            return Ok(());
        }
        let name = Self::checked_name(it)?;
        let (owner, comment) = match it.props() {
            Props::Schema(p) => (p.owner.clone(), p.comment.clone()),
            Props::Database(p) => (p.owner.clone(), p.comment.clone()),
            _ => (None, None),
        };
        // MySQL has no database comment (MariaDB has).
        let comment = if self.d() == DialectId::MySql {
            None
        } else {
            comment
        };
        self.op(DdlOp::CreateSchema(SchemaDef {
            name: name.clone(),
            if_not_exists: self.opts.if_not_exists,
            comment,
        }));
        let q = bare(&name);
        self.owner(ObjectKind::Schema, &q, None, owner.as_ref());
        self.grants(it, ObjectKind::Schema, &q, None);
        Ok(())
    }

    fn emit_type(&mut self, it: &Item) -> Result<(), DdlError> {
        let Props::Type(tp) = it.props() else {
            return Err(DdlError::NotLoaded);
        };
        if matches!(tp.body, hacivat_meta::props::TypeBody::Base) {
            // A base type needs C functions: no CREATE script.
            self.unsupported(it.kind());
            return Ok(());
        }
        let name = self.qname_of(it)?;
        self.op(DdlOp::CreateType(TypeDef {
            name: name.clone(),
            body: tp.body.clone(),
            comment: tp.comment.clone(),
        }));
        self.owner(it.kind(), &name, None, tp.owner.as_ref());
        self.grants(it, it.kind(), &name, None);
        Ok(())
    }

    /// The column a sequence's `OWNED BY` names, when it is in `ns` and an identity column
    /// (its sequence is implicit).
    fn owner_is_identity(ns: &NamespaceSnapshot, sp: &SeqProps) -> bool {
        let Some((r, col)) = &sp.owned_by else {
            return false;
        };
        let Some(t) = ns.find(&key_of_ref(r)) else {
            return false;
        };
        ns.child_by_name(t, ObjectKind::Column, col)
            .and_then(|c| ns.obj(c))
            .and_then(|c| c.props().as_column())
            .is_some_and(|c| matches!(c.identity(), Some(Identity::Always | Identity::ByDefault)))
    }

    /// The serial sequences of a PostgreSQL table: sequences of its namespace owned by one of
    /// its non-identity columns.
    fn owned_sequences(&self, table: &Item) -> Vec<Item> {
        if !self.pg() {
            return Vec::new();
        }
        let tkey = table.key();
        let mut out = Vec::new();
        for (i, o) in table.ns.children_of_kind(0, ObjectKind::Sequence) {
            let Props::Sequence(sp) = o.props() else {
                continue;
            };
            if o.flags().contains(ObjFlags::IMPLICIT) || Self::owner_is_identity(&table.ns, sp) {
                continue;
            }
            if sp
                .owned_by
                .as_ref()
                .is_some_and(|(r, _)| key_of_ref(r).same_object(&tkey))
            {
                out.push(table.at(i));
            }
        }
        out
    }

    /// Emits a sequence; returns its `OWNED BY` step and the owning table (when that table
    /// is in the same namespace), which the caller places.
    #[allow(clippy::type_complexity)]
    fn emit_sequence(
        &mut self,
        it: &Item,
        direct: &BTreeSet<(NsKey, u32)>,
    ) -> Result<Option<(Option<(NsKey, u32)>, Step)>, DdlError> {
        let Props::Sequence(sp) = it.props() else {
            return Err(DdlError::NotLoaded);
        };
        let implicit =
            it.flags().contains(ObjFlags::IMPLICIT) || Self::owner_is_identity(&it.ns, &sp);
        if implicit && !direct.contains(&it.id()) {
            // An identity column's sequence is created by the column.
            return Ok(None);
        }
        let name = self.qname_of(it)?;
        self.op(DdlOp::CreateSequence(SeqDef {
            name: name.clone(),
            if_not_exists: self.opts.if_not_exists,
            data_type: sp.data_type.clone(),
            start: sp.start,
            increment: sp.increment,
            min: sp.min,
            max: sp.max,
            cache: sp.cache,
            cycle: sp.cycle,
            owned_by: None,
            comment: self.comment(sp.comment.as_ref()),
        }));
        self.owner(ObjectKind::Sequence, &name, None, sp.owner.as_ref());
        self.grants(it, ObjectKind::Sequence, &name, None);
        let Some((r, col)) = &sp.owned_by else {
            return Ok(None);
        };
        if !self.pg() || implicit {
            return Ok(None);
        }
        let owner = it.ns.find(&key_of_ref(r)).map(|i| (it.ns.key().clone(), i));
        let table = self.qname_of_ref(r);
        Ok(Some((
            owner,
            Step::OwnedBy {
                seq: name,
                table,
                column: col.clone(),
            },
        )))
    }

    fn emit_routine(&mut self, it: &Item) -> Result<(), DdlError> {
        let Props::Routine(rp) = it.props() else {
            return Err(DdlError::NotLoaded);
        };
        if it.kind() == ObjectKind::Aggregate || rp.kind == FnKind::Aggregate {
            // `pg_get_functiondef` has no aggregate form.
            self.unsupported(ObjectKind::Aggregate);
            return Ok(());
        }
        let name = self.qname_of(it)?;
        let text = self.source(it, rp.source.as_ref())?;
        let kind = if it.kind() == ObjectKind::Procedure || rp.kind == FnKind::Procedure {
            ObjectKind::Procedure
        } else {
            ObjectKind::Routine
        };
        let sig = if self.pg() { routine_sig(&rp) } else { None };
        // MySQL's comment is inside its `SHOW CREATE` text.
        let comment = if self.pg() { rp.comment.clone() } else { None };
        self.op(DdlOp::CreateRoutine(RoutineDef {
            name: name.clone(),
            kind,
            sig: sig.clone(),
            text,
            comment,
        }));
        self.owner(kind, &name, sig.clone(), rp.owner.as_ref());
        self.grants(it, kind, &name, sig);
        Ok(())
    }

    /// Whether `r` points at `it` (by server id, else by path).
    fn ref_is(r: &Ref, it: &Item) -> bool {
        let k = key_of_ref(r);
        let ik = it.key();
        match (k.server_id, ik.server_id) {
            (Some(a), Some(b)) => a == b,
            _ => k.path == ik.path,
        }
    }

    /// Selected tables in creation order: a table after the tables its foreign keys and its
    /// partition parent reference; a cycle is broken at its smallest remaining table.
    fn table_order(&self, tables: &[Item]) -> Vec<Item> {
        let mut deps: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); tables.len()];
        for (i, t) in tables.iter().enumerate() {
            let mut refs: Vec<Ref> = Vec::new();
            for c in t.children() {
                if let Props::ForeignKey(fk) = c.obj().props() {
                    refs.push(fk.target.clone());
                }
            }
            if let Some(tp) = t.obj().props().as_table()
                && let Some(p) = &tp.partition_of
            {
                refs.push(p.clone());
            }
            for r in &refs {
                for (j, u) in tables.iter().enumerate() {
                    if j != i && Self::ref_is(r, u) {
                        deps[i].insert(j);
                    }
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

    /// Key columns by name (a key part without a column name has no constraint form).
    fn key_columns(k: &KeyProps) -> Result<Vec<Name>, DdlError> {
        k.columns
            .iter()
            .map(|c| c.name.clone().ok_or(DdlError::NotLoaded))
            .collect()
    }

    fn lower_constraint(&mut self, it: &Item) -> Result<ConstraintDef, DdlError> {
        let name = Self::checked_name(it)?;
        let server_named = it.flags().contains(ObjFlags::SERVER_GENERATED_NAME);
        let mut name = (!server_named || self.opts.surrogate_names).then_some(name);
        if name.as_deref() == Some("") {
            name = None;
        }
        let pg = self.pg();
        let invalid = pg && it.flags().contains(ObjFlags::INVALID);
        let (body, deferrable, not_valid) = match (it.props(), it.kind()) {
            (Props::Key(k), ObjectKind::PrimaryKey) => {
                if self.mysql() {
                    // A MySQL primary key is always named `PRIMARY`.
                    name = None;
                }
                (
                    ConstraintBody::PrimaryKey {
                        columns: Self::key_columns(&k)?,
                    },
                    deferrable(k.deferrable, k.initially_deferred),
                    pg && k.not_valid,
                )
            }
            (Props::Key(k), ObjectKind::UniqueKey) => (
                ConstraintBody::Unique {
                    columns: Self::key_columns(&k)?,
                    nulls_not_distinct: k.nulls_not_distinct,
                },
                deferrable(k.deferrable, k.initially_deferred),
                pg && k.not_valid,
            ),
            (Props::ForeignKey(fk), ObjectKind::ForeignKey) => {
                let FkProps {
                    columns,
                    target,
                    target_columns,
                    on_update,
                    on_delete,
                    match_kind,
                    ..
                } = &*fk;
                (
                    ConstraintBody::ForeignKey {
                        columns: columns.clone(),
                        target: self.qname_of_ref(target),
                        target_columns: target_columns.clone(),
                        on_update: *on_update,
                        on_delete: *on_delete,
                        match_kind: *match_kind,
                    },
                    deferrable(fk.deferrable, fk.initially_deferred),
                    pg && fk.not_valid,
                )
            }
            (Props::Check(c), ObjectKind::Check) => {
                let CheckProps {
                    expr, not_valid, ..
                } = &*c;
                (
                    ConstraintBody::Check { expr: expr.clone() },
                    Deferrable::No,
                    pg && *not_valid,
                )
            }
            (Props::Check(c), ObjectKind::Exclusion) if pg => (
                ConstraintBody::Exclusion {
                    raw: c.expr.clone(),
                },
                // Exclusion deferral is a T13 field (R-W2-8, `CheckRare`).
                Deferrable::No,
                false,
            ),
            _ => return Err(DdlError::NotLoaded),
        };
        Ok(ConstraintDef {
            name,
            body,
            deferrable,
            not_valid: not_valid || invalid,
        })
    }

    /// Whether `ix` is created by something else: an index backing a key (it carries
    /// `IMPLICIT` / `implicit_for`, or a key of its table names it), an inherited partition
    /// index, or a SQLite autoindex.
    fn index_is_implicit(ix: &Item, ip: &IndexProps) -> bool {
        if ix
            .flags()
            .intersects(ObjFlags::IMPLICIT | ObjFlags::INHERITED)
            || ip.key.implicit_for.is_some()
            || sqlite::is_autoindex(&ix.name())
        {
            return true;
        }
        let name = ix.name();
        ix.parent().is_some_and(|t| {
            t.children().iter().any(|k| {
                matches!(
                    k.kind(),
                    ObjectKind::PrimaryKey | ObjectKind::UniqueKey | ObjectKind::Exclusion
                ) && match k.obj().props() {
                    Props::Key(kp) => *kp.backing_index.as_ref().unwrap_or(k.obj().name()) == name,
                    _ => *k.obj().name() == name,
                }
            })
        })
    }

    /// The index parts: key columns in key order, an expression slot (no column name) taking
    /// the next expression text.
    fn index_parts(&self, ip: &IndexProps) -> Result<Vec<IndexPartDef>, DdlError> {
        let mut exprs = ip.key.expressions.iter();
        let mysql = self.mysql();
        ip.key
            .columns
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let (column, expr) = match &c.name {
                    Some(n) => (Some(n.clone()), None),
                    None => (
                        None,
                        Some(exprs.next().cloned().ok_or(DdlError::NotLoaded)?),
                    ),
                };
                Ok(IndexPartDef {
                    column,
                    expr,
                    desc: ip.key.desc.get(i).copied().unwrap_or(false),
                    // `NULLS FIRST/LAST` per part is a T13 field (R-W2-8, `KeyRare`).
                    nulls_first: None,
                    prefix_len: if mysql {
                        ip.prefix_len.get(i).copied().flatten()
                    } else {
                        None
                    },
                })
            })
            .collect()
    }

    /// An index of `table` as a step, or `None` when something else creates it.
    fn lower_index(&mut self, ix: &Item, table: &QualifiedName) -> Result<Option<Step>, DdlError> {
        let Props::Index(ip) = ix.props() else {
            return Err(DdlError::NotLoaded);
        };
        if Self::index_is_implicit(ix, &ip) {
            return Ok(None);
        }
        let name = Self::checked_name(ix)?;
        if self.sqlite() {
            let text = self.source(ix, None)?;
            return Ok(Some(Step::Op(raw(text))));
        }
        let pg = self.pg();
        if pg
            && (!ip.key.expressions.is_empty()
                || ip.key.predicate.is_some()
                || !ip.key.include.is_empty())
            && let Some(text) = self.sources.server_text(&ix.key())
        {
            // `pg_get_indexdef`: per-part operator classes and collations included.
            if let Some(c) = &ip.comment {
                let mut target = table.clone();
                if let Some(last) = target.parts.last_mut() {
                    last.text = name.clone();
                }
                self.op(raw(text));
                return Ok(Some(Step::Op(DdlOp::Comment {
                    target: CommentTarget {
                        kind: ObjectKind::Index,
                        name: target,
                        column: None,
                        sig: None,
                    },
                    text: Some(c.clone()),
                })));
            }
            return Ok(Some(Step::Op(raw(text))));
        }
        let op = self.index_op(&ip, name, table, ip.key.unique)?;
        if self.mysql() && ip.key.invisible {
            return Ok(Some(Step::IndexOption {
                op,
                word: mysql::invisible_word(self.d()),
            }));
        }
        Ok(Some(Step::Op(op)))
    }

    fn index_op(
        &self,
        ip: &IndexProps,
        name: Name,
        table: &QualifiedName,
        unique: bool,
    ) -> Result<DdlOp, DdlError> {
        Ok(DdlOp::CreateIndex(IndexDef {
            name,
            table: table.clone(),
            unique,
            if_not_exists: self.opts.if_not_exists,
            concurrently: false,
            method: ip.method.clone(),
            parts: self.index_parts(ip)?,
            include: if self.pg() {
                ip.key.include.clone()
            } else {
                Vec::new()
            },
            predicate: ip.key.predicate.clone(),
            nulls_not_distinct: ip.key.nulls_not_distinct && self.pg(),
            comment: self.comment(ip.comment.as_ref()),
        }))
    }

    fn lower_column(&self, c: &Item) -> Result<ColumnDef, DdlError> {
        let name = Self::checked_name(c)?;
        let Props::Column(cp) = c.props() else {
            return Err(DdlError::NotLoaded);
        };
        let ColumnProps {
            raw_type,
            nullable,
            default_raw,
            comment,
            hidden,
            ..
        } = &*cp;
        let (raw_type, on_update) = match cp.ext() {
            ColumnExt::MySql {
                column_type, extra, ..
            } if self.mysql() => {
                let mut t = column_type.clone().unwrap_or_else(|| raw_type.clone());
                if *hidden {
                    // `INVISIBLE` is a column attribute (MySQL 8.0.23+, MariaDB 10.3+).
                    t = Name::from(format!("{t} INVISIBLE"));
                }
                (t, extra.as_deref().and_then(mysql::on_update_of))
            }
            _ => (raw_type.clone(), None),
        };
        Ok(ColumnDef {
            name,
            raw_type,
            collation: cp.collation().cloned(),
            nullable: *nullable,
            default_raw: default_raw.clone(),
            generated: cp.generated().map(|g| GeneratedDef {
                expr: g.expr.clone(),
                stored: g.kind == hacivat_meta::props::GeneratedKind::Stored,
            }),
            identity: cp.identity(),
            on_update,
            comment: self.comment(comment.as_ref()),
        })
    }

    fn emit_table(
        &mut self,
        it: &Item,
        tables: &[Item],
        created: &BTreeSet<(NsKey, u32)>,
        after: &mut Vec<Step>,
        owned_by: Vec<Step>,
    ) -> Result<(), DdlError> {
        let tp: Arc<TableProps> = match it.props() {
            Props::Table(p) => p,
            Props::None => Arc::new(TableProps::default()),
            _ => return Err(DdlError::NotLoaded),
        };
        let name = self.table_qname(it)?;
        let children = it.children();
        if self.sqlite() {
            // `sqlite_schema.sql`: CHECK constraints and generated expressions live only there.
            let text = self.source(it, None)?;
            self.op(raw(text));
            for c in &children {
                match c.kind() {
                    ObjectKind::Index => {
                        if let Some(step) = self.lower_index(c, &name)? {
                            self.steps.push(step);
                        }
                    }
                    ObjectKind::Trigger => self.emit_trigger(c)?,
                    _ => {}
                }
            }
            return Ok(());
        }
        if self.mysql() && self.opts.server_definitions {
            // `SHOW CREATE TABLE`, verbatim; its triggers and grants follow.
            let text = self.source(it, None)?;
            self.op(raw(text));
            for c in &children {
                if c.kind() == ObjectKind::Trigger {
                    self.emit_trigger(c)?;
                }
            }
            self.grants(it, ObjectKind::Table, &name, None);
            return Ok(());
        }
        let pg = self.pg();
        let mysql = self.mysql();
        let mut columns = Vec::new();
        let mut keys = Vec::new();
        let mut indexes = Vec::new();
        let mut triggers = Vec::new();
        for c in &children {
            match c.kind() {
                ObjectKind::Column => {
                    if c.flags().contains(ObjFlags::INHERITED) {
                        if tp.partition_of.is_none() {
                            // An `INHERITS` child: the model holds no parent list yet, and a
                            // table without its inherited columns or its parents would be
                            // another table.
                            return Err(DdlError::NotLoaded);
                        }
                        continue;
                    }
                    columns.push(self.lower_column(c)?);
                }
                k if is_constraint(k) => {
                    if !c.flags().contains(ObjFlags::INHERITED) {
                        keys.push(c.clone());
                    }
                }
                ObjectKind::Index => indexes.push(c.clone()),
                ObjectKind::Trigger => triggers.push(c.clone()),
                k => self.unsupported(k),
            }
        }
        // Constraints: inside the table (the renderer places them), postponed past later
        // tables, or (MySQL) after an index of the same name.
        let mut constraints = Vec::new();
        let mut postponed = Vec::new();
        let mut after_indexes = Vec::new();
        let mut index_steps = Vec::new();
        let separate_index_names: BTreeSet<Name> = indexes
            .iter()
            .filter(|ix| match ix.props() {
                Props::Index(ip) => !Self::index_is_implicit(ix, &ip),
                _ => false,
            })
            .map(Item::name)
            .collect();
        for k in &keys {
            if mysql
                && k.kind() == ObjectKind::UniqueKey
                && let Some(step) = self.mysql_prefix_unique(k, &indexes, &name)?
            {
                // A unique key on column prefixes: the prefixes live on its index.
                index_steps.push(step);
                continue;
            }
            let def = self.lower_constraint(k)?;
            let target_later = match k.obj().props() {
                Props::ForeignKey(fk) => tables.iter().any(|u| {
                    u.id() != it.id() && Self::ref_is(&fk.target, u) && !created.contains(&u.id())
                }),
                _ => false,
            };
            if target_later {
                postponed.push(def);
            } else if mysql
                && k.kind() == ObjectKind::ForeignKey
                && def
                    .name
                    .as_ref()
                    .is_some_and(|n| separate_index_names.contains(n))
            {
                // MySQL would create the FK's own index under the index's name.
                after_indexes.push(def);
            } else {
                constraints.push(def);
            }
        }
        if mysql && keys.iter().any(|k| k.kind() == ObjectKind::PrimaryKey) {
            self.check_mysql_pk_prefix(&keys, &indexes)?;
        }
        let partition_of = match &tp.partition_of {
            Some(r) if pg => Some((
                self.qname_of_ref(r),
                tp.partition_bound.clone().ok_or(DdlError::NotLoaded)?,
            )),
            _ => None,
        };
        let ext = if pg {
            TableDefExt::Pg(pg::table_ext(&tp, partition_of))
        } else {
            if let TableExt::MySql { options, .. } = &tp.ext
                && options.as_deref().is_some_and(mysql::is_partitioned)
            {
                // MySQL partitioning is not introspected in M4.
                self.unsupported(ObjectKind::Partition);
            }
            TableDefExt::MySql(mysql::table_ext(&tp))
        };
        self.op(DdlOp::CreateTable(TableDef {
            name: name.clone(),
            if_not_exists: self.opts.if_not_exists,
            columns,
            constraints,
            comment: self.comment(tp.comment.as_ref()),
            ext,
        }));
        if pg && tp.row_security {
            self.op(DdlOp::RowSecurity {
                table: name.clone(),
                enable: true,
            });
            self.notices.push(Notice::new(POLICIES_NOT_LOADED));
        }
        self.steps.append(&mut index_steps);
        for ix in &indexes {
            if let Some(step) = self.lower_index(ix, &name)? {
                self.steps.push(step);
            }
        }
        if !after_indexes.is_empty() {
            self.op(DdlOp::AlterTable {
                table: name.clone(),
                actions: after_indexes
                    .into_iter()
                    .map(AlterAction::AddConstraint)
                    .collect(),
            });
        }
        self.steps.extend(owned_by);
        for t in &triggers {
            self.emit_trigger(t)?;
        }
        self.owner(ObjectKind::Table, &name, None, tp.owner.as_ref());
        self.grants(it, ObjectKind::Table, &name, None);
        if !postponed.is_empty() {
            after.push(Step::Op(DdlOp::AlterTable {
                table: name,
                actions: postponed
                    .into_iter()
                    .map(AlterAction::AddConstraint)
                    .collect(),
            }));
        }
        Ok(())
    }

    /// A MySQL unique key whose backing index has column prefixes, as `CREATE UNIQUE INDEX`
    /// from that index (the constraint form has no prefix lengths; T13 carry m4).
    fn mysql_prefix_unique(
        &mut self,
        k: &Item,
        indexes: &[Item],
        table: &QualifiedName,
    ) -> Result<Option<Step>, DdlError> {
        let Some((ix, ip)) = Self::backing_index(k, indexes) else {
            return Ok(None);
        };
        if ip.prefix_len.iter().all(Option::is_none) {
            return Ok(None);
        }
        let name = Self::checked_name(&ix)?;
        let op = self.index_op(&ip, name, table, true)?;
        Ok(Some(if ip.key.invisible {
            Step::IndexOption {
                op,
                word: mysql::invisible_word(self.d()),
            }
        } else {
            Step::Op(op)
        }))
    }

    /// The index backing key `k`: named by its `backing_index`, else by the key's own name.
    fn backing_index(k: &Item, indexes: &[Item]) -> Option<(Item, Arc<IndexProps>)> {
        let want = match k.obj().props() {
            Props::Key(kp) => kp.backing_index.clone().unwrap_or_else(|| k.name()),
            _ => k.name(),
        };
        indexes.iter().find_map(|ix| match ix.props() {
            Props::Index(ip) if ix.name() == want => Some((ix.clone(), ip)),
            _ => None,
        })
    }

    /// A MySQL primary key on column prefixes cannot be written as a constraint.
    fn check_mysql_pk_prefix(&self, keys: &[Item], indexes: &[Item]) -> Result<(), DdlError> {
        for k in keys.iter().filter(|k| k.kind() == ObjectKind::PrimaryKey) {
            if let Some((_, ip)) = Self::backing_index(k, indexes)
                && ip.prefix_len.iter().any(Option::is_some)
            {
                return Err(DdlError::Render(RenderError::Invalid(
                    "a MySQL primary key on column prefixes",
                )));
            }
        }
        Ok(())
    }

    fn emit_view(&mut self, it: &Item) -> Result<(), DdlError> {
        let Props::View(vp) = it.props() else {
            return Err(DdlError::NotLoaded);
        };
        let name = self.qname_of(it)?;
        let text = self.source(it, vp.source.as_ref())?;
        let materialized = it.kind() == ObjectKind::MaterializedView;
        let kind = if materialized {
            ObjectKind::MaterializedView
        } else {
            ObjectKind::View
        };
        if !self.pg() {
            // MySQL `SHOW CREATE VIEW` and SQLite `sqlite_schema.sql`: complete statements.
            self.op(raw(text));
        } else {
            let mut column_comments = Vec::new();
            for c in it.children() {
                if c.kind() == ObjectKind::Column
                    && let Some(cm) = c.obj().props().comment()
                {
                    column_comments.push((Self::checked_name(&c)?, cm.clone()));
                }
            }
            self.op(DdlOp::CreateView(ViewDef {
                name: name.clone(),
                kind: if materialized {
                    ViewKind::Materialized
                } else {
                    ViewKind::View
                },
                or_replace: false,
                body: text,
                check_option: vp.check_option.clone(),
                comment: vp.comment.clone(),
                column_comments,
            }));
        }
        for c in it.children() {
            match c.kind() {
                ObjectKind::Column => {}
                ObjectKind::Index => {
                    if let Some(step) = self.lower_index(&c, &name)? {
                        self.steps.push(step);
                    }
                }
                ObjectKind::Trigger => self.emit_trigger(&c)?,
                k => self.unsupported(k),
            }
        }
        self.owner(kind, &name, None, vp.owner.as_ref());
        self.grants(it, kind, &name, None);
        Ok(())
    }

    fn emit_trigger(&mut self, it: &Item) -> Result<(), DdlError> {
        let Props::Trigger(tp) = it.props() else {
            return Err(DdlError::NotLoaded);
        };
        let name = Self::checked_name(it)?;
        let text = self.source(it, tp.source.as_ref())?;
        match self.parent_qname(it)? {
            Some(table) if !self.sqlite() => self.op(DdlOp::CreateTrigger(TriggerDef {
                name,
                table,
                text,
                comment: None,
            })),
            _ => self.op(raw(text)),
        }
        Ok(())
    }
}

/// Server text as an op: emitted verbatim (the MySQL renderer makes the requested head
/// edits).
fn raw(text: Name) -> DdlOp {
    DdlOp::Raw {
        sql: text.to_string(),
        origin: RawOrigin::ServerText,
    }
}
