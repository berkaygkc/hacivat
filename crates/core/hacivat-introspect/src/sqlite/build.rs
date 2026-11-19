// ## m4-t18-sqlite
//! Model objects from one schema's rows: relations (tables, views, virtual tables), their
//! columns, keys and indexes (assembled by Task 13's `assemble::sqlite`), foreign keys and
//! triggers, plus the existence sets that say what the read decided (R-W2-5).
//!
//! - **Kinds.** A `sqlite_schema` row of type `view` is a view; `table` is a virtual table when
//!   its text is `CREATE VIRTUAL TABLE` (the module from `USING`), else a table. Names starting
//!   with `sqlite_` are the engine's own (`ObjFlags::SYSTEM`); `pragma_table_list`'s `shadow`
//!   tables (FTS5 `docs_data`, …) exist for their virtual table (`ObjFlags::IMPLICIT`).
//! - **Columns** from `pragma_table_xinfo` by `cid`: `hidden` 1 is a hidden virtual-table
//!   column, 2 a virtual and 3 a stored generated column (its expression from the table's
//!   text); the declared collation from the text; `INTEGER PRIMARY KEY` on a rowid table
//!   without a backing index is the rowid alias (`Identity::RowidAlias`, or `AutoIncrement`
//!   with `AUTOINCREMENT`).
//! - **Index part collations** are kept only where they differ from the column's own (`BINARY`
//!   when it declares none, and for an expression or the rowid), compared ASCII-caselessly
//!   (T13's contract). **FK deferral** comes from the table's text, clause by clause:
//!   `foreign_key_list` numbers the FKs from the last declared (id 0), so clause `k` of `n` in
//!   text order is id `n − 1 − k`; when the counts disagree the deferral stays unknown
//!   (SQLite's default, not deferrable). CHECK constraints are not modelled (R-M4-24, F10).
//! - **Triggers** sit under their table or view (`tbl_name`); timing, events and `WHEN` come
//!   from the text, `FOR EACH ROW` always (SQLite has no statement triggers). A trigger whose
//!   text does not have that shape is kept `BROKEN`. A temp trigger on a `main` table has no
//!   parent in `temp` and is left out.
//! - **Existence** (alg/02 HV-INTRO-11, R-W2-5): the relation kinds are complete for the read
//!   (`Ok` on a full read, `Scoped` to the asked relations on a fragment); every child kind is
//!   `Scoped` to the relations whose children were actually read, so a broken relation's cached
//!   columns are never decided gone; a failed index or FK pragma is `Failed`.
//! - **Marks** are schema-level (HV-INTRO-15: SQLite has no per-object marker): every object
//!   carries the schema's mark, `schema_version` plus digest.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use hacivat_meta::merge::{ExistSet, Identity as ExistId};
use hacivat_meta::name::{DialectId, IdentCtx};
use hacivat_meta::props::{
    ColumnProps, ColumnRare, Generated, GeneratedKind, Identity, Persistence, Props, TableExt,
    TableProps, TriggerEvents, TriggerProps, TriggerTiming, ViewProps,
};
use hacivat_meta::{Level, Name, NsKey, Obj, ObjFlags, ObjectKind, SourceRef, StateMark, Subtree};

use super::read::{ObjRow, SchemaRows, TableFlags};
use super::text::{self, Event, Timing};
use crate::assemble::sqlite::{
    FkRow as AFkRow, IndexPartRow, IndexRow, KeyRows, PkColumnRow, assemble_keys,
};
use crate::assemble::{ColumnIndex, TableId};

/// The relation kinds, in the order their existence sets are reported.
pub(crate) const RELATION_KINDS: [ObjectKind; 3] = [
    ObjectKind::Table,
    ObjectKind::View,
    ObjectKind::VirtualTable,
];

/// The kind of a `sqlite_schema` row that is a relation; `None` for indexes and triggers.
pub(crate) fn relation_kind(o: &ObjRow) -> Option<ObjectKind> {
    match &*o.ty {
        "view" => Some(ObjectKind::View),
        "table" if o.sql.as_deref().and_then(text::virtual_module).is_some() => {
            Some(ObjectKind::VirtualTable)
        }
        "table" => Some(ObjectKind::Table),
        _ => None,
    }
}

/// What a read of one schema becomes.
pub(crate) struct Built {
    /// One subtree per relation, by name (byte order).
    pub relations: Vec<Subtree>,
    pub existent: BTreeMap<ObjectKind, ExistSet>,
    /// The `(kind, namespace-relative path)` of every object built.
    pub paths: BTreeSet<(ObjectKind, Vec<Name>)>,
}

/// Builds the objects of `rows` read at `level`. `only`: a fragment's relations (by exact
/// name); `None`: every relation (a full read).
pub(crate) fn build(
    ns: &NsKey,
    rows: &SchemaRows,
    level: Level,
    temp: bool,
    mark: StateMark,
    only: Option<&BTreeSet<Name>>,
) -> Built {
    let extra = if temp {
        ObjFlags::TEMP
    } else {
        ObjFlags::empty()
    };
    let wanted = |n: &Name| only.is_none_or(|o| o.contains(n));
    let mut rels: Vec<(&ObjRow, ObjectKind)> = rows
        .objects
        .iter()
        .filter_map(|o| relation_kind(o).map(|k| (o, k)))
        .filter(|(o, _)| wanted(&o.name))
        .collect();
    rels.sort_by(|a, b| a.0.name.cmp(&b.0.name));

    let defs: HashMap<&str, text::TableDef> = rels
        .iter()
        .filter(|(_, k)| *k == ObjectKind::Table)
        .filter_map(|(o, _)| Some((&*o.name, text::table_def(o.sql.as_deref()?))))
        .collect();

    // Columns per relation, by cid.
    let mut cols: HashMap<&str, Vec<&super::read::ColRow>> = HashMap::new();
    for c in &rows.columns {
        cols.entry(&*c.rel).or_default().push(c);
    }
    for v in cols.values_mut() {
        v.sort_by_key(|c| c.cid);
    }
    // Which tables have a primary-key autoindex (an `INTEGER PRIMARY KEY` that has one is not
    // the rowid alias: `… PRIMARY KEY DESC`).
    let pk_index: BTreeSet<&str> = rows
        .indexes
        .iter()
        .filter(|i| &*i.origin == "pk")
        .map(|i| &*i.rel)
        .collect();
    let read_ok = |n: &str| rows.structure && !rows.broken.contains_key(n);

    let mut keys_of: HashMap<Name, Vec<Obj>> = HashMap::new();
    if rows.structure {
        for k in assemble(ns, rows, &rels, &defs, &cols) {
            if read_ok(&k.table.name) {
                keys_of
                    .entry(k.table.name.clone())
                    .or_default()
                    .push(k.obj.with_state(mark).add_flags(extra));
            }
        }
    }
    let mut triggers_of: HashMap<String, Vec<&ObjRow>> = HashMap::new();
    if rows.structure {
        for o in rows.objects.iter().filter(|o| &*o.ty == "trigger") {
            triggers_of
                .entry(o.tbl.to_ascii_lowercase())
                .or_default()
                .push(o);
        }
        for v in triggers_of.values_mut() {
            v.sort_by(|a, b| a.name.cmp(&b.name));
        }
    }

    let mut relations = Vec::with_capacity(rels.len());
    let mut ids: BTreeMap<ObjectKind, Vec<ExistId>> = BTreeMap::new();
    let mut child_scope: Vec<Vec<Name>> = Vec::new();
    let mut key_scope: Vec<Vec<Name>> = Vec::new();
    let mut paths = BTreeSet::new();
    for (o, kind) in &rels {
        let flags =
            relation_flags(o, rows.flags.as_ref(), rows.broken.contains_key(&o.name)) | extra;
        let props = relation_props(o, *kind, rows.flags.as_ref(), temp, level);
        let mut t = Subtree::new(
            Obj::new(*kind, o.name.clone())
                .with_props(props)
                .with_state(mark)
                .add_flags(flags),
        );
        ids.entry(*kind)
            .or_default()
            .push(ExistId::of(vec![o.name.clone()], None));
        paths.insert((*kind, vec![o.name.clone()]));
        if rows.structure {
            let root = t.root();
            let rel_path = vec![o.name.clone()];
            if read_ok(&o.name) {
                child_scope.push(rel_path.clone());
                if *kind != ObjectKind::View {
                    key_scope.push(rel_path.clone());
                }
                let def = defs.get(&*o.name);
                let alias = rowid_alias(o, *kind, rows, &cols, &pk_index, def);
                for c in cols.get(&*o.name).map_or(&[][..], |v| v.as_slice()) {
                    let p = column_props(c, def, alias.filter(|(cid, _)| *cid == c.cid));
                    t.add(
                        root,
                        Obj::new(ObjectKind::Column, c.name.clone())
                            .with_props(Props::Column(Arc::new(p)))
                            .with_state(mark)
                            .add_flags(extra),
                    );
                    ids.entry(ObjectKind::Column)
                        .or_default()
                        .push(ExistId::of(vec![o.name.clone(), c.name.clone()], None));
                    paths.insert((ObjectKind::Column, vec![o.name.clone(), c.name.clone()]));
                }
                for k in keys_of.remove(&o.name).unwrap_or_default() {
                    ids.entry(k.kind())
                        .or_default()
                        .push(ExistId::of(vec![o.name.clone(), k.name().clone()], None));
                    paths.insert((k.kind(), vec![o.name.clone(), k.name().clone()]));
                    t.add(root, k);
                }
            }
            for tr in triggers_of
                .get(&o.name.to_ascii_lowercase())
                .map_or(&[][..], |v| v.as_slice())
            {
                let (props, ok) = trigger_props(tr, level);
                let broken = if ok {
                    ObjFlags::empty()
                } else {
                    ObjFlags::BROKEN
                };
                t.add(
                    root,
                    Obj::new(ObjectKind::Trigger, tr.name.clone())
                        .with_props(Props::Trigger(Arc::new(props)))
                        .with_state(mark)
                        .add_flags(extra | broken),
                );
                ids.entry(ObjectKind::Trigger)
                    .or_default()
                    .push(ExistId::of(vec![o.name.clone(), tr.name.clone()], None));
                paths.insert((ObjectKind::Trigger, vec![o.name.clone(), tr.name.clone()]));
            }
        }
        relations.push(t);
    }

    // Existence (module docs).
    let mut existent = BTreeMap::new();
    for kind in RELATION_KINDS {
        let found = ids.remove(&kind).unwrap_or_default();
        let set = match only {
            None => ExistSet::Ok(found),
            Some(o) => ExistSet::Scoped {
                scope: o.iter().map(|n| vec![n.clone()]).collect(),
                ids: found,
            },
        };
        existent.insert(kind, set);
    }
    if rows.structure {
        let all: Vec<Vec<Name>> = rels.iter().map(|(o, _)| vec![o.name.clone()]).collect();
        let mut scoped = |kind: ObjectKind, scope: &Vec<Vec<Name>>| ExistSet::Scoped {
            scope: scope.clone(),
            ids: ids.remove(&kind).unwrap_or_default(),
        };
        let failed = |code| ExistSet::Failed(code);
        let sets = [
            (ObjectKind::Column, scoped(ObjectKind::Column, &child_scope)),
            (
                ObjectKind::PrimaryKey,
                scoped(ObjectKind::PrimaryKey, &key_scope),
            ),
            (ObjectKind::Trigger, scoped(ObjectKind::Trigger, &all)),
            (
                ObjectKind::Index,
                rows.index_failed
                    .map_or_else(|| scoped(ObjectKind::Index, &key_scope), failed),
            ),
            (
                ObjectKind::UniqueKey,
                rows.index_failed
                    .map_or_else(|| scoped(ObjectKind::UniqueKey, &key_scope), failed),
            ),
            (
                ObjectKind::ForeignKey,
                rows.fk_failed
                    .map_or_else(|| scoped(ObjectKind::ForeignKey, &key_scope), failed),
            ),
        ];
        existent.extend(sets);
    }
    Built {
        relations,
        existent,
        paths,
    }
}

fn relation_flags(o: &ObjRow, flags: Option<&HashMap<Name, TableFlags>>, broken: bool) -> ObjFlags {
    let mut f = ObjFlags::empty();
    if o.name
        .get(..7)
        .is_some_and(|p| p.eq_ignore_ascii_case("sqlite_"))
    {
        f |= ObjFlags::SYSTEM;
    }
    if flags
        .and_then(|m| m.get(&o.name))
        .is_some_and(|t| &*t.ty == "shadow")
    {
        f |= ObjFlags::IMPLICIT;
    }
    if broken {
        f |= ObjFlags::BROKEN;
    }
    f
}

fn relation_props(
    o: &ObjRow,
    kind: ObjectKind,
    flags: Option<&HashMap<Name, TableFlags>>,
    temp: bool,
    level: Level,
) -> Props {
    let sql = o.sql.as_deref();
    match kind {
        ObjectKind::View => Props::View(Arc::new(ViewProps {
            source: sql.filter(|_| level >= Level::Sources).map(SourceRef::of),
            ..ViewProps::default()
        })),
        _ => {
            let module = sql
                .and_then(text::virtual_module)
                .map(|m| Name::from(m.as_str()));
            let (without_rowid, strict) = if kind == ObjectKind::VirtualTable {
                (false, false)
            } else if let Some(t) = flags.and_then(|m| m.get(&o.name)) {
                (t.without_rowid, t.strict)
            } else {
                sql.map_or((false, false), text::table_options)
            };
            Props::Table(Arc::new(TableProps {
                persistence: if temp {
                    Persistence::Temporary
                } else {
                    Persistence::Permanent
                },
                ext: TableExt::Sqlite {
                    without_rowid,
                    strict,
                    module,
                },
                ..TableProps::default()
            }))
        }
    }
}

/// The rowid alias of table `o`: its one primary-key column when that is declared exactly
/// `INTEGER` on a rowid table with no primary-key index, with whether it is `AUTOINCREMENT`.
fn rowid_alias(
    o: &ObjRow,
    kind: ObjectKind,
    rows: &SchemaRows,
    cols: &HashMap<&str, Vec<&super::read::ColRow>>,
    pk_index: &BTreeSet<&str>,
    def: Option<&text::TableDef>,
) -> Option<(i64, bool)> {
    if kind != ObjectKind::Table || pk_index.contains(&*o.name) {
        return None;
    }
    let without_rowid = rows
        .flags
        .as_ref()
        .and_then(|m| m.get(&o.name))
        .map_or_else(
            || o.sql.as_deref().is_some_and(|s| text::table_options(s).0),
            |t| t.without_rowid,
        );
    if without_rowid {
        return None;
    }
    let pk: Vec<_> = cols.get(&*o.name)?.iter().filter(|c| c.pk > 0).collect();
    let [c] = pk.as_slice() else {
        return None;
    };
    if !c.ty.eq_ignore_ascii_case("INTEGER") {
        return None;
    }
    let auto = def
        .and_then(|d| d.column(&c.name))
        .is_some_and(|d| d.autoincrement);
    Some((c.cid, auto))
}

fn column_props(
    c: &super::read::ColRow,
    def: Option<&text::TableDef>,
    alias: Option<(i64, bool)>,
) -> ColumnProps {
    let d = def.and_then(|d| d.column(&c.name));
    let generated = match c.hidden {
        2 | 3 => Some(Generated {
            kind: if c.hidden == 3 {
                GeneratedKind::Stored
            } else {
                GeneratedKind::Virtual
            },
            expr: Name::from(d.and_then(|d| d.generated.as_deref()).unwrap_or("")),
        }),
        _ => None,
    };
    let identity = alias.map(|(_, auto)| {
        if auto {
            Identity::AutoIncrement
        } else {
            Identity::RowidAlias
        }
    });
    let collation = d.and_then(|d| d.collation.as_deref()).map(Name::from);
    let rare = (generated.is_some() || identity.is_some() || collation.is_some()).then(|| {
        Arc::new(ColumnRare {
            identity,
            generated,
            collation,
            ..ColumnRare::default()
        })
    });
    ColumnProps {
        ordinal: u32::try_from(c.cid.saturating_add(1)).unwrap_or(0),
        raw_type: c.ty.clone(),
        type_ref: None,
        nullable: !c.notnull,
        default_raw: c.dflt.clone(),
        comment: None,
        hidden: c.hidden == 1,
        rare,
        ext: None,
    }
}

fn trigger_props(o: &ObjRow, level: Level) -> (TriggerProps, bool) {
    let source = o
        .sql
        .as_deref()
        .filter(|_| level >= Level::Sources)
        .map(SourceRef::of);
    let Some(d) = o.sql.as_deref().and_then(text::trigger_def) else {
        return (
            TriggerProps {
                for_each_row: true,
                source,
                ..TriggerProps::default()
            },
            false,
        );
    };
    let props = TriggerProps {
        timing: match d.timing {
            Timing::Before => TriggerTiming::Before,
            Timing::After => TriggerTiming::After,
            Timing::InsteadOf => TriggerTiming::InsteadOf,
        },
        events: match d.event {
            Event::Insert => TriggerEvents::INSERT,
            Event::Update => TriggerEvents::UPDATE,
            Event::Delete => TriggerEvents::DELETE,
        },
        for_each_row: true,
        when_expr: d.when.map(|w| Name::from(w.as_str())),
        source,
        ..TriggerProps::default()
    };
    (props, true)
}

/// Keys, indexes and foreign keys of the tables in `rels` (Task 13's assembler).
fn assemble(
    ns: &NsKey,
    rows: &SchemaRows,
    rels: &[(&ObjRow, ObjectKind)],
    defs: &HashMap<&str, text::TableDef>,
    cols: &HashMap<&str, Vec<&super::read::ColRow>>,
) -> Vec<crate::assemble::KeyObj> {
    let mut index = ColumnIndex::new(
        ns.clone(),
        DialectId::Sqlite,
        IdentCtx::new(DialectId::Sqlite),
    );
    // Every relation of the schema (not only a fragment's) resolves FK targets; the index is
    // complete only when every relation's columns were read.
    let mut all_cols: HashMap<&str, Vec<&super::read::ColRow>> = HashMap::new();
    for c in &rows.columns {
        all_cols.entry(&*c.rel).or_default().push(c);
    }
    for o in rows.objects.iter().filter(|o| relation_kind(o).is_some()) {
        if let Some(cs) = all_cols.get(&*o.name) {
            index.add_table(
                TableId::named(o.name.clone()),
                cs.iter().map(|c| {
                    (
                        u32::try_from(c.cid.saturating_add(1)).unwrap_or(0),
                        c.name.clone(),
                    )
                }),
            );
        }
    }
    index.set_complete(rows.broken.is_empty());

    let tables: BTreeSet<&str> = rels
        .iter()
        .filter(|(_, k)| *k != ObjectKind::View)
        .map(|(o, _)| &*o.name)
        .collect();
    let sql_of: HashMap<&str, &Name> = rows
        .objects
        .iter()
        .filter(|o| &*o.ty == "index")
        .filter_map(|o| Some((&*o.name, o.sql.as_ref()?)))
        .collect();
    let mut k = KeyRows::default();
    for t in &tables {
        for c in cols.get(t).map_or(&[][..], |v| v.as_slice()) {
            if c.pk > 0 {
                k.pk_columns.push(PkColumnRow {
                    table: c.rel.clone(),
                    cid: c.cid,
                    name: c.name.clone(),
                    pk: c.pk,
                });
            }
        }
    }
    let mut table_of_index: HashMap<&str, &str> = HashMap::new();
    for i in rows.indexes.iter().filter(|i| tables.contains(&*i.rel)) {
        table_of_index.insert(&i.name, &i.rel);
        k.indexes.push(IndexRow {
            table: i.rel.clone(),
            name: i.name.clone(),
            unique: i.unique,
            origin: i.origin.clone(),
            partial: i.partial,
            sql: sql_of.get(&*i.name).map(|s| (*s).clone()),
        });
    }
    for p in &rows.parts {
        let Some(table) = table_of_index.get(&*p.index) else {
            continue;
        };
        // The collation a part would have without its own `COLLATE` (T13's contract).
        let base = if p.cid >= 0 {
            p.name
                .as_deref()
                .and_then(|n| defs.get(table)?.column(n)?.collation.clone())
                .unwrap_or_else(|| "BINARY".to_string())
        } else {
            "BINARY".to_string()
        };
        let coll = p.coll.clone().filter(|c| !c.eq_ignore_ascii_case(&base));
        k.index_parts.push(IndexPartRow {
            index: p.index.clone(),
            seqno: p.seqno,
            cid: p.cid,
            name: p.name.clone(),
            desc: p.desc,
            coll,
            key: p.key,
        });
    }
    // FK deferral from the text (module docs).
    let mut ids_of: HashMap<&str, BTreeSet<u32>> = HashMap::new();
    for f in rows.fks.iter().filter(|f| tables.contains(&*f.rel)) {
        ids_of.entry(&f.rel).or_default().insert(f.id);
    }
    for f in rows.fks.iter().filter(|f| tables.contains(&*f.rel)) {
        let clauses = defs.get(&*f.rel).map_or(&[][..], |d| d.fks.as_slice());
        let n = ids_of.get(&*f.rel).map_or(0, BTreeSet::len);
        let clause = (clauses.len() == n)
            .then(|| usize::try_from(f.id).ok())
            .flatten()
            .and_then(|id| n.checked_sub(1)?.checked_sub(id))
            .and_then(|k| clauses.get(k))
            .copied()
            .unwrap_or_default();
        k.foreign_keys.push(AFkRow {
            table: f.rel.clone(),
            id: f.id,
            seq: f.seq,
            ref_table: f.ref_table.clone(),
            from: f.from.clone(),
            to: f.to.clone(),
            on_update: f.on_update.clone(),
            on_delete: f.on_delete.clone(),
            match_kind: f.match_kind.clone(),
            deferrable: clause.deferrable,
            initially_deferred: clause.initially_deferred,
        });
    }
    assemble_keys(k, &index)
}
