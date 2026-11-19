// ## m4-t17-mysql
//! Model objects from typed MySQL rows: one subtree per relation (columns, keys, indexes,
//! foreign keys, checks, triggers), routine (arguments) and event, with their R-M4-39 state
//! marks, and the existence identities a pass reports (HV-INTRO-11).
//!
//! Engine differences are kept verbatim (F9): `COLUMN_TYPE` (MariaDB display widths), `EXTRA`
//! (`on update` text per flavor), check clauses, FK rules as reported. Defaults arrive
//! pre-quoted ([`rows::default_sql`]). A child row whose relation is not among the listed
//! relations is skipped (MariaDB reads without a consistent snapshot, F2; HV-INTRO-01 edge 4):
//! the next pass reads both.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use hacivat_meta::merge::Identity;
use hacivat_meta::props::{
    ColumnExt, ColumnProps, ColumnRare, Generated, GenericProps, Identity as ColIdentity,
    Persistence, Props, RoutineProps, SeqProps, TableExt, TableProps, TriggerEvents, TriggerProps,
    TriggerTiming, ViewProps,
};
use hacivat_meta::{Name, Obj, ObjFlags, ObjectKind, StateMark, Subtree};

use super::rows::{
    self, ColumnRow, EventRow, ParamRow, RoutineRow, TableRow, TriggerRow, ViewRow, parse_extra,
    parse_time_micros,
};
use super::server::ServerCtx;
use crate::assemble::KeyObj;

/// The relation kind of a `TABLES.TABLE_TYPE` (MariaDB sequences are tables with type
/// `SEQUENCE`; a type this build does not model is kept as [`ObjectKind::Unknown`] with its raw
/// text).
pub fn relation_kind(table_type: &str, mariadb: bool) -> ObjectKind {
    match table_type {
        "BASE TABLE" | "SYSTEM VERSIONED" => ObjectKind::Table,
        "VIEW" | "SYSTEM VIEW" => ObjectKind::View,
        "SEQUENCE" if mariadb => ObjectKind::Sequence,
        _ => ObjectKind::Unknown,
    }
}

/// The raw kind kept for an [`ObjectKind::Unknown`] relation or routine.
pub fn raw_kind(kind: ObjectKind, text: &Name) -> Option<Name> {
    (kind == ObjectKind::Unknown).then(|| text.clone())
}

/// The routine kind of a `ROUTINES.ROUTINE_TYPE` (MariaDB packages are kept as unknown).
pub fn routine_kind(routine_type: &str) -> ObjectKind {
    match routine_type {
        "FUNCTION" => ObjectKind::Routine,
        "PROCEDURE" => ObjectKind::Procedure,
        _ => ObjectKind::Unknown,
    }
}

/// The `ROUTINE_TYPE` text of a routine kind (`None`: not a MySQL routine kind).
pub fn routine_type_of(kind: ObjectKind) -> Option<&'static str> {
    match kind {
        ObjectKind::Routine => Some("FUNCTION"),
        ObjectKind::Procedure => Some("PROCEDURE"),
        _ => None,
    }
}

/// A time marker: `Some(micros)` for a parseable time, `None` otherwise.
pub fn micros(text: Option<&Name>) -> Option<i64> {
    text.and_then(|t| parse_time_micros(t))
}

/// The newest trigger `CREATED` and the trigger count per table, from trigger rows.
pub fn trigger_stats<'a>(
    triggers: impl IntoIterator<Item = &'a TriggerRow>,
) -> HashMap<Name, (i64, u32)> {
    let mut out: HashMap<Name, (i64, u32)> = HashMap::new();
    for t in triggers {
        let e = out.entry(t.table.clone()).or_insert((0, 0));
        e.0 = e.0.max(micros(t.created.as_ref()).unwrap_or(0));
        e.1 = e.1.saturating_add(1);
    }
    out
}

/// R-M4-39's table stamp: `CREATE_TIME | max(TRIGGERS.CREATED) | trigger count` (a NULL or
/// unreadable `CREATE_TIME` reads 0, and such a table is re-read by every incremental pass).
pub fn table_stamp(create_time: Option<&Name>, triggers: Option<&(i64, u32)>) -> StateMark {
    let (trigger_max, trigger_count) = triggers.copied().unwrap_or((0, 0));
    StateMark::MyTable {
        created: micros(create_time).unwrap_or(0),
        trigger_max,
        trigger_count,
    }
}

/// The state mark of a relation: a view's definition hash, anything else's table stamp.
pub fn relation_mark(
    kind: ObjectKind,
    create_time: Option<&Name>,
    view_hash: Option<u64>,
    triggers: Option<&(i64, u32)>,
) -> StateMark {
    if kind == ObjectKind::View {
        view_hash.map_or(StateMark::None, StateMark::Hash)
    } else {
        table_stamp(create_time, triggers)
    }
}

/// The relation object (no children).
fn relation_obj(
    t: &TableRow,
    kind: ObjectKind,
    view: Option<&ViewRow>,
    state: StateMark,
    system: bool,
) -> Obj {
    let props = match kind {
        ObjectKind::Table => Props::Table(Arc::new(TableProps {
            comment: t.comment.clone(),
            persistence: Persistence::Permanent,
            partitioned: t.options.as_deref().is_some_and(|o| {
                o.split_ascii_whitespace()
                    .any(|w| w.eq_ignore_ascii_case("partitioned"))
            }),
            ext: TableExt::MySql {
                engine: t.engine.clone(),
                collation: t.collation.clone(),
                options: t.options.clone(),
            },
            ..TableProps::default()
        })),
        ObjectKind::View => Props::View(Arc::new(ViewProps {
            check_option: view.and_then(|v| v.check_option.clone()),
            updatable: view.and_then(|v| v.updatable),
            algorithm: view.and_then(|v| v.algorithm.clone()),
            definer: view.and_then(|v| v.definer.clone()),
            sql_security: view.and_then(|v| v.security_type.clone()),
            ..ViewProps::default()
        })),
        ObjectKind::Sequence => Props::Sequence(Arc::new(SeqProps {
            comment: t.comment.clone(),
            ..SeqProps::default()
        })),
        _ => Props::Generic(Arc::new(GenericProps {
            comment: t.comment.clone(),
            raw_kind: Some(t.table_type.clone()),
            ..GenericProps::default()
        })),
    };
    let obj = Obj::new(kind, t.name.clone())
        .with_props(props)
        .with_state(state);
    if system {
        obj.add_flags(ObjFlags::SYSTEM)
    } else {
        obj
    }
}

/// One column object.
pub fn column_obj(c: &ColumnRow, table_collation: Option<&Name>, srv: &ServerCtx) -> Obj {
    let extra = c.extra.as_deref().map(parse_extra).unwrap_or_default();
    let default_raw = rows::default_sql(
        srv.is_mariadb(),
        c.default.as_deref(),
        &c.data_type,
        &extra,
        &srv.quote_ctx(),
    );
    let generated = extra.generated.map(|kind| Generated {
        kind,
        expr: c
            .generation_expression
            .clone()
            .unwrap_or_else(|| Name::from("")),
    });
    // A collation is "declared" when it is not the table's own (what `SHOW CREATE TABLE`
    // writes); the table's collation is on the table.
    let collation = c
        .collation
        .clone()
        .filter(|col| table_collation != Some(col));
    let enum_labels =
        rows::enum_labels(&c.column_type, !srv.no_backslash_escapes()).map(Arc::<[Name]>::from);
    let rare = ColumnRare {
        not_null_constraint: None,
        identity: extra.auto_increment.then_some(ColIdentity::AutoIncrement),
        generated,
        collation,
        enum_labels,
    };
    let has_rare = rare.identity.is_some()
        || rare.generated.is_some()
        || rare.collation.is_some()
        || rare.enum_labels.is_some();
    let props = ColumnProps {
        ordinal: c.ordinal,
        raw_type: c.column_type.clone(),
        type_ref: None,
        nullable: c.nullable,
        default_raw,
        comment: c.comment.clone(),
        hidden: extra.invisible,
        rare: has_rare.then(|| Arc::new(rare)),
        ext: Some(Arc::new(ColumnExt::MySql {
            column_type: Some(c.column_type.clone()),
            extra: c.extra.clone(),
            charset: c.charset.clone(),
        })),
    };
    Obj::new(ObjectKind::Column, c.name.clone()).with_props(Props::Column(Arc::new(props)))
}

/// One trigger object (MySQL triggers are row-level and enabled).
pub fn trigger_obj(t: &TriggerRow) -> Obj {
    let timing = match t.timing.as_deref() {
        Some("AFTER") => TriggerTiming::After,
        _ => TriggerTiming::Before,
    };
    let events = match t.event.as_deref() {
        Some("INSERT") => TriggerEvents::INSERT,
        Some("UPDATE") => TriggerEvents::UPDATE,
        Some("DELETE") => TriggerEvents::DELETE,
        _ => TriggerEvents::empty(),
    };
    let props = TriggerProps {
        timing,
        events,
        for_each_row: true,
        definer: t.definer.clone(),
        enabled: true,
        ..TriggerProps::default()
    };
    Obj::new(ObjectKind::Trigger, t.name.clone())
        .with_props(Props::Trigger(Arc::new(props)))
        .with_state(micros(t.created.as_ref()).map_or(StateMark::None, StateMark::Created))
}

/// A top-level object and its children (every MySQL subtree is at most two levels deep).
#[derive(Debug, Clone)]
pub struct Tree {
    pub root: Obj,
    pub children: Vec<Obj>,
}

impl Tree {
    fn new(root: Obj) -> Self {
        Tree {
            root,
            children: Vec::new(),
        }
    }

    pub fn name(&self) -> &Name {
        self.root.name()
    }

    pub fn kind(&self) -> ObjectKind {
        self.root.kind()
    }

    /// The model subtree.
    pub fn subtree(self) -> Subtree {
        let mut t = Subtree::with_capacity(self.root, self.children.len() + 1);
        let root = t.root();
        for c in self.children {
            t.add(root, c);
        }
        t
    }

    /// The existence identities of the tree, by kind: the root at `[name]`, each child at
    /// `[root, child]` (namespace-relative paths).
    pub fn identities(&self, out: &mut BTreeMap<ObjectKind, Vec<Identity>>) {
        let root = &self.root;
        out.entry(root.kind())
            .or_default()
            .push(identity(vec![root.name().clone()], root));
        for o in &self.children {
            out.entry(o.kind())
                .or_default()
                .push(identity(vec![root.name().clone(), o.name().clone()], o));
        }
    }
}

/// What one relation subtree is built from.
pub struct RelationParts<'a> {
    pub table: &'a TableRow,
    pub view: Option<&'a ViewRow>,
    pub view_hash: Option<u64>,
    pub columns: &'a [&'a ColumnRow],
    pub keys: Vec<KeyObj>,
    pub triggers: &'a [&'a TriggerRow],
    pub trigger_stats: Option<&'a (i64, u32)>,
    /// Add columns, keys and triggers (`Structure` and above).
    pub children: bool,
    /// A system schema's relation (`Mode::System`): flagged, no state mark.
    pub system: bool,
}

/// The subtree of one relation.
pub fn relation_tree(p: RelationParts<'_>, srv: &ServerCtx) -> Tree {
    let kind = relation_kind(&p.table.table_type, srv.is_mariadb());
    let state = if p.system {
        StateMark::None
    } else {
        relation_mark(
            kind,
            p.table.create_time.as_ref(),
            p.view_hash,
            p.trigger_stats,
        )
    };
    let mut t = Tree::new(relation_obj(p.table, kind, p.view, state, p.system));
    // A MariaDB sequence holds no children in the model (its `COLUMNS` rows are the
    // sequence's own state, not a definition).
    if !p.children || kind == ObjectKind::Sequence {
        return t;
    }
    let mut cols: Vec<&&ColumnRow> = p.columns.iter().collect();
    cols.sort_by_key(|c| c.ordinal);
    for c in cols {
        t.children
            .push(column_obj(c, p.table.collation.as_ref(), srv));
    }
    if kind == ObjectKind::View {
        return t;
    }
    t.children.extend(p.keys.into_iter().map(|k| k.obj));
    let mut trigs: Vec<&&TriggerRow> = p.triggers.iter().collect();
    trigs.sort_by(|a, b| (a.order, &a.name).cmp(&(b.order, &b.name)));
    t.children.extend(trigs.into_iter().map(|g| trigger_obj(g)));
    t
}

/// The subtree of one routine with its arguments (`PARAMETERS`, `ORDINAL_POSITION > 0`;
/// identity `(ROUTINE_TYPE, SPECIFIC_NAME)`, F9). An argument is a column-shaped object: its
/// position, its `DTD_IDENTIFIER` as the raw type and its `PARAMETER_MODE` in the MySQL
/// column extension's `extra` (the model has no argument props; ADR 0008 `m4-t17`).
pub fn routine_tree(r: &RoutineRow, params: &[&ParamRow], children: bool, system: bool) -> Tree {
    let kind = routine_kind(&r.routine_type);
    let props = if kind == ObjectKind::Unknown {
        Props::Generic(Arc::new(GenericProps {
            owner: r.definer.clone(),
            comment: r.comment.clone(),
            raw_kind: Some(r.routine_type.clone()),
            ..GenericProps::default()
        }))
    } else {
        Props::Routine(Arc::new(RoutineProps {
            returns: if kind == ObjectKind::Routine {
                r.returns.clone()
            } else {
                None
            },
            definer: r.definer.clone(),
            specific_name: Some(r.specific_name.clone()),
            security_definer: r.security_type.as_deref() == Some("DEFINER"),
            comment: r.comment.clone(),
            ..RoutineProps::default()
        }))
    };
    let state = if system {
        StateMark::None
    } else {
        micros(r.last_altered.as_ref()).map_or(StateMark::None, StateMark::Modified)
    };
    let mut obj = Obj::new(kind, r.name.clone())
        .with_props(props)
        .with_state(state);
    if system {
        obj = obj.add_flags(ObjFlags::SYSTEM);
    }
    let mut t = Tree::new(obj);
    if !children || kind == ObjectKind::Unknown {
        return t;
    }
    let mut ps: Vec<&&ParamRow> = params.iter().collect();
    ps.sort_by_key(|p| p.ordinal);
    for p in ps {
        let Some(name) = &p.name else { continue };
        let props = ColumnProps {
            ordinal: p.ordinal,
            raw_type: p.data_type.clone(),
            nullable: true,
            ext: Some(Arc::new(ColumnExt::MySql {
                column_type: None,
                extra: p.mode.clone(),
                charset: None,
            })),
            ..ColumnProps::default()
        };
        t.children.push(
            Obj::new(ObjectKind::Argument, name.clone()).with_props(Props::Column(Arc::new(props))),
        );
    }
    t
}

/// The subtree of one event.
pub fn event_tree(e: &EventRow) -> Tree {
    let props = GenericProps {
        owner: e.definer.clone(),
        comment: e.comment.clone(),
        ..GenericProps::default()
    };
    Tree::new(
        Obj::new(ObjectKind::Event, e.name.clone())
            .with_props(Props::Generic(Arc::new(props)))
            .with_state(
                micros(e.last_altered.as_ref()).map_or(StateMark::None, StateMark::Modified),
            ),
    )
}

fn identity(path: Vec<Name>, o: &Obj) -> Identity {
    Identity {
        path,
        server_id: None,
        overload_sig: None,
        raw_kind: o.raw_kind().cloned(),
        state: o.state(),
    }
}
