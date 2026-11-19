// ## m4-t01c-core (seam), filled by m4-t13-assemble
//! Composite object assembly (HV-INTRO-13; M4 Task 13): primary and unique keys, indexes,
//! foreign keys, checks and PostgreSQL exclusion constraints, built from per-dialect catalog
//! rows ([`pg`], [`mysql`], [`sqlite`]; each has `assemble_keys(rows, &ColumnIndex) ->
//! Vec<KeyObj>`).
//!
//! The rules every adapter follows:
//!
//! - **One object per catalog object.** PostgreSQL returns one row per index or constraint with
//!   ordered arrays; MySQL/MariaDB key parts arrive **one row each** and are grouped here by
//!   `(table, index)` in `SEQ_IN_INDEX` order (no `GROUP_CONCAT`, so no length limit and no
//!   warning 1260); SQLite parts come from `pragma_index_xinfo`.
//! - **Column references** are `(ordinal, name)` pairs ([`ColRef`]) resolved against the
//!   owning table's columns ([`ColumnIndex`]) **by ordinal, then by name**. Key order is the
//!   row order (`conkey`, `SEQ_IN_INDEX`, `seqno`), never ordinal or name order. A reference
//!   that does not resolve keeps what the row said and flags the object
//!   [`ObjFlags::BROKEN`]: the object is **kept**, never dropped, so DDL tooling can warn.
//! - **Expression parts** are slots `KeyColumn { ordinal: 0, name: None }` in
//!   `KeyProps::columns`; their raw server text is in `KeyProps::expressions`, in slot order.
//! - **Implicit indexes.** An index that exists for a constraint (a PostgreSQL PK/UNIQUE/EXCLUDE
//!   backing index through `conindid`, MySQL `PRIMARY` and unique indexes, SQLite
//!   `sqlite_autoindex_*` of `origin` `pk`/`u`, a MySQL FK-created index under
//!   [`mysql::FkIndexRule`]) gets [`ObjFlags::IMPLICIT`] and `KeyProps::implicit_for` pointing
//!   at the constraint; the constraint's `backing_index` names the index.
//! - **Names.** Server names are kept verbatim. A name the server does not report (SQLite's
//!   unnamed primary and foreign keys) is a display name flagged
//!   [`ObjFlags::SERVER_GENERATED_NAME`], which DDL never emits as a real name.
//! - **Header rows decide existence.** An object exists when its own row does (`pg_index`,
//!   `pg_constraint`, `STATISTICS`, `REFERENTIAL_CONSTRAINTS`, a `CHECK` row of
//!   `TABLE_CONSTRAINTS`, `index_list`, `foreign_key_list`); a part row whose object row is
//!   missing (a read without a consistent snapshot, MariaDB) is ignored, and the next pass
//!   reads both.
//! - **Deterministic output:** sorted by table, then by name, kind and server id, whatever
//!   order the rows arrived in.

use std::collections::HashMap;
use std::sync::Arc;

use smallvec::SmallVec;

use hacivat_meta::name::{DialectId, FoldRule, IdentCtx, NameClass, match_key};
use hacivat_meta::props::{
    CheckProps, FkAction, FkMatch, FkProps, IndexProps, KeyColumn, KeyProps, Props,
};
use hacivat_meta::{Name, NsKey, Obj, ObjFlags, ObjectKind, Ref, ServerId};

pub mod mysql;
pub mod pg;
pub mod sqlite;

/// The table a row belongs to (or an FK references): its server id when the engine has one
/// (PostgreSQL OID), and its name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TableId {
    pub server_id: Option<ServerId>,
    pub name: Name,
}

impl TableId {
    /// A table known by name only (MySQL, MariaDB, SQLite).
    pub fn named(name: impl Into<Name>) -> Self {
        TableId {
            server_id: None,
            name: name.into(),
        }
    }

    /// A PostgreSQL relation: its OID and name.
    pub fn with_oid(oid: u32, name: impl Into<Name>) -> Self {
        TableId {
            server_id: Some(ServerId::Oid(oid)),
            name: name.into(),
        }
    }
}

/// A column reference as a catalog row gives it: the column's ordinal (PostgreSQL `attnum`,
/// SQLite `cid + 1`) and/or its name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ColRef {
    pub ordinal: Option<u32>,
    pub name: Option<Name>,
}

impl ColRef {
    /// A reference by ordinal, with the name the row joined.
    pub fn at(ordinal: u32, name: impl Into<Name>) -> Self {
        ColRef {
            ordinal: Some(ordinal),
            name: Some(name.into()),
        }
    }

    /// A reference by name only (MySQL `COLUMN_NAME`, SQLite FK columns).
    pub fn named(name: impl Into<Name>) -> Self {
        ColRef {
            ordinal: None,
            name: Some(name.into()),
        }
    }
}

/// The columns of one table.
#[derive(Debug, Clone)]
struct TableCols {
    id: TableId,
    /// `(ordinal, name)` in insertion order.
    cols: Vec<(u32, Name)>,
    by_ordinal: HashMap<u32, usize>,
    /// Exact name → index; [`AMBIGUOUS`] when two columns share it.
    by_name: HashMap<Name, usize>,
    /// Certain folded key → index, on engines whose column names fold (ASCII only).
    by_folded: HashMap<Name, usize>,
}

/// Marks a key two entries share: such a key resolves nothing.
const AMBIGUOUS: usize = usize::MAX;

fn insert_unique<K: std::hash::Hash + Eq>(map: &mut HashMap<K, usize>, k: K, v: usize) {
    map.entry(k).and_modify(|e| *e = AMBIGUOUS).or_insert(v);
}

fn unique<K, Q>(map: &HashMap<K, usize>, k: &Q) -> Option<usize>
where
    K: std::borrow::Borrow<Q> + std::hash::Hash + Eq,
    Q: std::hash::Hash + Eq + ?Sized,
{
    map.get(k).copied().filter(|&i| i != AMBIGUOUS)
}

/// The columns of the tables of one namespace, for resolving the column references of key,
/// index and FK rows (alg/02 HV-INTRO-13 step 3). The caller adds every table it read columns
/// for; a table not added is unknown, and its references do not resolve.
#[derive(Debug, Clone)]
pub struct ColumnIndex {
    ns: NsKey,
    dialect: DialectId,
    ctx: IdentCtx,
    tables: Vec<TableCols>,
    by_sid: HashMap<ServerId, usize>,
    by_name: HashMap<Name, usize>,
    by_folded: HashMap<Name, usize>,
}

impl ColumnIndex {
    /// An empty index of namespace `ns` (PostgreSQL `[db, schema]`, MySQL `[schema]`, SQLite
    /// `[main]`) under `dialect` and `ctx` (MySQL's `lower_case_table_names`).
    pub fn new(ns: NsKey, dialect: DialectId, ctx: IdentCtx) -> Self {
        ColumnIndex {
            ns,
            dialect,
            ctx,
            tables: Vec::new(),
            by_sid: HashMap::new(),
            by_name: HashMap::new(),
            by_folded: HashMap::new(),
        }
    }

    /// The namespace the rows belong to.
    pub fn ns(&self) -> &NsKey {
        &self.ns
    }

    /// Adds `table` with its `(ordinal, name)` columns (PostgreSQL `attnum`, MySQL
    /// `ORDINAL_POSITION`, SQLite `cid + 1`).
    pub fn add_table<I>(&mut self, table: TableId, columns: I)
    where
        I: IntoIterator<Item = (u32, Name)>,
    {
        let mut t = TableCols {
            id: table,
            cols: Vec::new(),
            by_ordinal: HashMap::new(),
            by_name: HashMap::new(),
            by_folded: HashMap::new(),
        };
        for (ordinal, name) in columns {
            let i = t.cols.len();
            insert_unique(&mut t.by_ordinal, ordinal, i);
            insert_unique(&mut t.by_name, name.clone(), i);
            if let Some(k) = self.folded(NameClass::Column, &name) {
                insert_unique(&mut t.by_folded, k, i);
            }
            t.cols.push((ordinal, name));
        }
        let at = self.tables.len();
        if let Some(sid) = t.id.server_id {
            insert_unique(&mut self.by_sid, sid, at);
        }
        insert_unique(&mut self.by_name, t.id.name.clone(), at);
        if let Some(k) = self.folded(NameClass::Table, &t.id.name) {
            insert_unique(&mut self.by_folded, k, at);
        }
        self.tables.push(t);
    }

    /// Whether `table` was added.
    pub fn contains(&self, table: &TableId) -> bool {
        self.table(table).is_some()
    }

    /// The column `col` of `table` refers to, by ordinal first, then by name (exact, then by a
    /// certain ASCII-caseless key where the engine folds column names). `None` when the table
    /// is unknown, nothing matches, two columns match, or the ordinal's column has another name
    /// than the row says (an inconsistent read).
    pub fn resolve(&self, table: &TableId, col: &ColRef) -> Option<KeyColumn> {
        let t = self.table(table)?;
        let at = match col.ordinal.and_then(|o| unique(&t.by_ordinal, &o)) {
            Some(i) => {
                if let Some(name) = &col.name
                    && !self.same_column(&t.cols[i].1, name)
                {
                    return None;
                }
                i
            }
            None => {
                let name = col.name.as_ref()?;
                match unique(&t.by_name, &**name) {
                    Some(i) => i,
                    None if t.by_name.contains_key(&**name) => return None,
                    None => {
                        let k = self.folded(NameClass::Column, name)?;
                        unique(&t.by_folded, &*k)?
                    }
                }
            }
        };
        let (ordinal, name) = &t.cols[at];
        Some(KeyColumn {
            ordinal: *ordinal,
            name: Some(name.clone()),
        })
    }

    fn table(&self, table: &TableId) -> Option<&TableCols> {
        let at = match table.server_id {
            Some(sid) if self.by_sid.contains_key(&sid) => unique(&self.by_sid, &sid)?,
            _ => match unique(&self.by_name, &*table.name) {
                Some(i) => i,
                None if self.by_name.contains_key(&*table.name) => return None,
                None => {
                    let k = self.folded(NameClass::Table, &table.name)?;
                    unique(&self.by_folded, &*k)?
                }
            },
        };
        let t = &self.tables[at];
        // A server id names one table: a name match must not stand in for another id.
        match (table.server_id, t.id.server_id) {
            (Some(a), Some(b)) if a != b => None,
            _ => Some(t),
        }
    }

    /// The certain caseless key of `name` of `class`, where the engine folds that class:
    /// `None` for an exact class or a text whose server rule is not pinned (R-M4-41).
    fn folded(&self, class: NameClass, name: &str) -> Option<Name> {
        if FoldRule::for_names(self.dialect, &self.ctx, class) != Some(FoldRule::AsciiCaseless) {
            return None;
        }
        let key = match_key(self.dialect, &self.ctx, name, true, class).ok()?;
        Some(Name::from(key.as_str()))
    }

    fn same_column(&self, stored: &Name, given: &str) -> bool {
        **stored == *given
            || self
                .folded(NameClass::Column, given)
                .zip(self.folded(NameClass::Column, stored))
                .is_some_and(|(a, b)| a == b)
    }

    /// Whether the table `name` of schema `schema` (`None`: this namespace) is one this index
    /// can resolve columns of.
    fn knows(&self, schema: Option<&str>, table: &TableId) -> bool {
        schema.is_none_or(|s| s == self.ns.root_name()) && self.contains(table)
    }

    /// The path of `table` in `schema` (`None`: this namespace).
    fn table_path(&self, schema: Option<&Name>, table: &Name) -> SmallVec<[Name; 4]> {
        let mut p: SmallVec<[Name; 4]> = match schema {
            Some(s) => {
                let parent = self.ns.path.len().saturating_sub(1);
                let mut p: SmallVec<[Name; 4]> = self.ns.path[..parent].iter().cloned().collect();
                p.push(s.clone());
                p
            }
            None => self.ns.path.iter().cloned().collect(),
        };
        p.push(table.clone());
        p
    }

    /// A reference to the child `name` of kind `kind` of `table` in this namespace.
    fn child_ref(
        &self,
        table: &TableId,
        kind: ObjectKind,
        name: &Name,
        server_id: Option<ServerId>,
    ) -> Ref {
        let mut path = self.table_path(None, &table.name);
        path.push(name.clone());
        Ref {
            kind,
            path,
            server_id,
        }
    }
}

/// One assembled object and the facts the model's props have no field for.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyObj {
    /// The owning table (the introspector adds [`KeyObj::obj`] under it).
    pub table: TableId,
    /// The object: kind, name, server id, flags and props.
    pub obj: Obj,
    /// A foreign key's parent key on the referenced table: the index it relies on (PostgreSQL
    /// `conindid`, a unique index or a key's backing index; MySQL `UNIQUE_CONSTRAINT_NAME`,
    /// which names an index). `None` elsewhere or when the server does not say.
    pub ref_constraint: Option<Ref>,
    /// A PostgreSQL exclusion constraint's operator per key part, in key order (the
    /// definition in `CheckProps::expr` holds them too). Empty elsewhere.
    pub operators: Vec<Name>,
}

impl KeyObj {
    fn new(table: TableId, obj: Obj) -> Self {
        KeyObj {
            table,
            obj,
            ref_constraint: None,
            operators: Vec::new(),
        }
    }

    /// The key fields of a primary or unique key, or of an index.
    pub fn key(&self) -> Option<&KeyProps> {
        match self.obj.props() {
            Props::Key(k) => Some(k),
            Props::Index(i) => Some(&i.key),
            _ => None,
        }
    }

    /// The props of an index.
    pub fn index(&self) -> Option<&IndexProps> {
        match self.obj.props() {
            Props::Index(i) => Some(i),
            _ => None,
        }
    }

    /// The props of a foreign key.
    pub fn fk(&self) -> Option<&FkProps> {
        match self.obj.props() {
            Props::ForeignKey(f) => Some(f),
            _ => None,
        }
    }

    /// The props of a check or exclusion constraint.
    pub fn check(&self) -> Option<&CheckProps> {
        match self.obj.props() {
            Props::Check(c) => Some(c),
            _ => None,
        }
    }

    /// Whether the object is flagged [`ObjFlags::BROKEN`].
    pub fn is_broken(&self) -> bool {
        self.obj.flags().contains(ObjFlags::BROKEN)
    }
}

/// Key parts in key order, and whether one did not resolve.
#[derive(Debug, Default)]
struct Parts {
    columns: Vec<KeyColumn>,
    expressions: Vec<Name>,
    desc: Vec<bool>,
    broken: bool,
}

impl Parts {
    /// A column part: resolved, or kept as the row gave it and marked broken.
    fn column(&mut self, cols: &ColumnIndex, table: &TableId, col: &ColRef, desc: bool) {
        let kc = match cols.resolve(table, col) {
            Some(kc) => kc,
            None => {
                self.broken = true;
                KeyColumn {
                    ordinal: col.ordinal.unwrap_or(0),
                    name: col.name.clone(),
                }
            }
        };
        self.columns.push(kc);
        self.desc.push(desc);
    }

    /// An expression part: its slot and its text (a missing text is broken).
    fn expression(&mut self, text: Option<&Name>, desc: bool) {
        self.columns.push(KeyColumn {
            ordinal: 0,
            name: None,
        });
        match text {
            Some(t) => self.expressions.push(t.clone()),
            None => {
                self.broken = true;
                self.expressions.push(Name::from(""));
            }
        }
        self.desc.push(desc);
    }
}

/// Resolves `refs` of `table` to names; unresolved ones keep the row's name (or an empty one).
fn resolve_names(cols: &ColumnIndex, table: &TableId, refs: &[ColRef]) -> (Vec<Name>, bool) {
    let mut broken = false;
    let names = refs
        .iter()
        .map(|r| match cols.resolve(table, r).and_then(|k| k.name) {
            Some(n) => n,
            None => {
                broken = true;
                r.name.clone().unwrap_or_else(|| Name::from(""))
            }
        })
        .collect();
    (names, broken)
}

/// Default key props: enforced, everything else off.
fn key_props() -> KeyProps {
    KeyProps {
        enforced: true,
        ..KeyProps::default()
    }
}

fn index_obj(name: &Name, props: IndexProps) -> Obj {
    Obj::new(ObjectKind::Index, name.clone()).with_props(Props::Index(Arc::new(props)))
}

fn key_obj(kind: ObjectKind, name: &Name, props: KeyProps) -> Obj {
    Obj::new(kind, name.clone()).with_props(Props::Key(Arc::new(props)))
}

/// `flags` when `on`, else none.
fn flag_if(on: bool, flags: ObjFlags) -> ObjFlags {
    if on { flags } else { ObjFlags::empty() }
}

/// Marks the index `idx` implicit for `owner` (a key, exclusion or FK of the same table).
fn mark_implicit(cols: &ColumnIndex, idx: &mut KeyObj, owner: &KeyObj) {
    let Props::Index(p) = idx.obj.props() else {
        return;
    };
    let mut p = (**p).clone();
    p.key.implicit_for = Some(cols.child_ref(
        &owner.table,
        owner.obj.kind(),
        owner.obj.name(),
        owner.obj.server_id(),
    ));
    let obj = std::mem::replace(&mut idx.obj, Obj::new(ObjectKind::Index, ""));
    idx.obj = obj
        .add_flags(ObjFlags::IMPLICIT)
        .with_props(Props::Index(Arc::new(p)));
}

/// The SQL standard's referential action words (MySQL `UPDATE_RULE`, SQLite `on_update`).
fn fk_action_word(text: &str) -> Option<FkAction> {
    const WORDS: [(&str, FkAction); 5] = [
        ("NO ACTION", FkAction::NoAction),
        ("RESTRICT", FkAction::Restrict),
        ("CASCADE", FkAction::Cascade),
        ("SET NULL", FkAction::SetNull),
        ("SET DEFAULT", FkAction::SetDefault),
    ];
    let t = text.trim();
    WORDS
        .iter()
        .find(|(w, _)| w.eq_ignore_ascii_case(t))
        .map(|&(_, a)| a)
}

/// The match words (MySQL `MATCH_OPTION`, SQLite `match`): `NONE` and `SIMPLE` are simple.
fn fk_match_word(text: &str) -> Option<FkMatch> {
    const WORDS: [(&str, FkMatch); 4] = [
        ("NONE", FkMatch::Simple),
        ("SIMPLE", FkMatch::Simple),
        ("FULL", FkMatch::Full),
        ("PARTIAL", FkMatch::Partial),
    ];
    let t = text.trim();
    WORDS
        .iter()
        .find(|(w, _)| w.eq_ignore_ascii_case(t))
        .map(|&(_, m)| m)
}

/// Sorts objects by table, then name, kind and server id (alg/02 HV-INTRO-13 step 6).
fn finish(mut out: Vec<KeyObj>) -> Vec<KeyObj> {
    out.sort_by(|a, b| {
        (
            &a.table.name,
            a.table.server_id,
            a.obj.name(),
            a.obj.kind(),
            a.obj.server_id(),
        )
            .cmp(&(
                &b.table.name,
                b.table.server_id,
                b.obj.name(),
                b.obj.kind(),
                b.obj.server_id(),
            ))
    });
    out
}
