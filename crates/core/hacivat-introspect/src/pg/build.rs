// ## m4-t16-pg
//! From the PostgreSQL rows ([`super::model`]) to the model: objects, subtrees and existence
//! identities (HV-MODEL-01). Keys, indexes, foreign keys, checks and exclusions are assembled
//! by Task 13 ([`crate::assemble::pg`]); every object carries its oid as `ServerId::Oid`.
//!
//! Paths: a `Ref` names an object by `[database, schema, name]`; an existence identity by its
//! namespace-relative path (`[table]`, `[table, column]`).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use hacivat_meta::merge::Identity as ExistId;
use hacivat_meta::props::{
    ArgMode, ColumnProps, ColumnRare, FnArg, FnKind, Generated as MGenerated, GeneratedKind,
    GenericProps, Identity as MIdentity, Persistence as MPersistence, PgTableExt, Props,
    RoutineProps, SchemaProps, SeqProps, TableExt, TableProps, TriggerEvents, TriggerProps,
    TriggerTiming as MTiming, TypeAttr, TypeBody, TypeProps, TypeRef, ViewProps,
    Volatility as MVolatility,
};
use hacivat_meta::{
    Name, NsKey, Obj, ObjFlags, ObjectKey, ObjectKind, Ref, ServerId, SourceRef, StateMark, Subtree,
};
use smallvec::SmallVec;

use super::decode::check_body;
use super::model::*;
use crate::assemble::pg::{ConstraintRow, FkTarget, IndexPart, IndexRow, KeyRows, assemble_keys};
use crate::assemble::{ColRef, ColumnIndex, KeyObj, TableId};

/// The raw kind of a PostgreSQL base type (`typtype b`): the model has no kind for it.
pub const BASE_TYPE_RAW_KIND: &str = "pg-base-type";

/// The top-level kinds a PostgreSQL namespace holds (every one gets an existence set).
pub const TOP_KINDS: [ObjectKind; 13] = [
    ObjectKind::Table,
    ObjectKind::View,
    ObjectKind::MaterializedView,
    ObjectKind::ForeignTable,
    ObjectKind::Sequence,
    ObjectKind::Routine,
    ObjectKind::Procedure,
    ObjectKind::Aggregate,
    ObjectKind::Domain,
    ObjectKind::EnumType,
    ObjectKind::CompositeType,
    ObjectKind::RangeType,
    ObjectKind::Unknown,
];

/// The child kinds of a relation that a `Structure` pass reads.
pub const CHILD_KINDS: [ObjectKind; 8] = [
    ObjectKind::Column,
    ObjectKind::Index,
    ObjectKind::PrimaryKey,
    ObjectKind::UniqueKey,
    ObjectKind::ForeignKey,
    ObjectKind::Check,
    ObjectKind::Exclusion,
    ObjectKind::Trigger,
];

/// The namespace being built: `[database, schema]`.
#[derive(Debug, Clone)]
pub struct Ns {
    pub key: NsKey,
    pub db: Name,
    pub schema: Name,
}

impl Ns {
    pub fn new(key: &NsKey) -> Option<Ns> {
        if key.kind != hacivat_meta::NsKind::Schema || key.path.len() != 2 {
            return None;
        }
        Some(Ns {
            key: key.clone(),
            db: key.path[0].clone(),
            schema: key.path[1].clone(),
        })
    }

    /// The key of the namespace root (the parent of every top-level object).
    pub fn root_key(&self) -> ObjectKey {
        ObjectKey {
            kind: ObjectKind::Schema,
            path: self.key.path.iter().cloned().collect(),
            server_id: None,
            overload_sig: None,
            raw_kind: None,
        }
    }

    /// The key of a top-level object `name` of `kind` with `oid`.
    pub fn key_of(&self, kind: ObjectKind, name: &str, oid: Oid) -> ObjectKey {
        let mut path: SmallVec<[Name; 4]> = self.key.path.iter().cloned().collect();
        path.push(Name::from(name));
        ObjectKey {
            kind,
            path,
            server_id: Some(ServerId::Oid(oid)),
            overload_sig: None,
            raw_kind: (kind == ObjectKind::Unknown).then(|| Name::from(BASE_TYPE_RAW_KIND)),
        }
    }

    /// A reference to `[db, schema, name]`.
    pub fn ref_to(&self, kind: ObjectKind, schema: &str, name: &str, oid: Oid) -> Ref {
        Ref {
            kind,
            path: [self.db.clone(), Name::from(schema), Name::from(name)]
                .into_iter()
                .collect(),
            server_id: Some(ServerId::Oid(oid)),
        }
    }
}

fn n(s: &str) -> Name {
    Name::from(s)
}

fn on(s: &Option<String>) -> Option<Name> {
    s.as_deref().map(Name::from)
}

/// The schema root object.
pub fn schema_obj(s: &PgSchema) -> Obj {
    Obj::new(ObjectKind::Schema, n(&s.name))
        .with_server_id(ServerId::Oid(s.oid))
        .with_props(Props::Schema(Arc::new(SchemaProps {
            owner: Some(n(&s.owner)),
            comment: on(&s.comment),
        })))
}

/// The model kind of a relation.
pub fn rel_kind(k: RelKind) -> ObjectKind {
    match k {
        RelKind::Table | RelKind::PartitionedTable => ObjectKind::Table,
        RelKind::View => ObjectKind::View,
        RelKind::MaterializedView => ObjectKind::MaterializedView,
        RelKind::ForeignTable => ObjectKind::ForeignTable,
    }
}

/// A relation's own object (no children).
pub fn relation_obj(ns: &Ns, r: &PgRelation) -> Obj {
    let kind = rel_kind(r.kind);
    let props = match r.kind {
        RelKind::View | RelKind::MaterializedView => Props::View(Arc::new(ViewProps {
            owner: Some(n(&r.owner)),
            comment: on(&r.comment),
            source: r.view_definition.as_deref().map(SourceRef::of),
            check_option: r
                .options
                .iter()
                .find_map(|o| o.strip_prefix("check_option="))
                .map(n),
            ..ViewProps::default()
        })),
        _ => Props::Table(Arc::new(TableProps {
            comment: on(&r.comment),
            owner: Some(n(&r.owner)),
            persistence: match r.persistence {
                Persistence::Permanent => MPersistence::Permanent,
                Persistence::Unlogged => MPersistence::Unlogged,
                Persistence::Temporary => MPersistence::Temporary,
            },
            partitioned: r.kind == RelKind::PartitionedTable,
            partition_of: r
                .partition_of
                .as_ref()
                .map(|(oid, s, t)| ns.ref_to(ObjectKind::Table, s, t, *oid)),
            partition_key: on(&r.partition_key),
            partition_bound: on(&r.partition_bound),
            row_security: r.row_security,
            ext: TableExt::Pg(PgTableExt {
                relkind: match r.kind {
                    RelKind::PartitionedTable => b'p',
                    RelKind::ForeignTable => b'f',
                    _ => b'r',
                },
                tablespace: on(&r.tablespace),
                access_method: on(&r.access_method),
                options: r.options.iter().map(|o| n(o)).collect(),
            }),
        })),
    };
    let mut o = Obj::new(kind, n(&r.name))
        .with_server_id(ServerId::Oid(r.oid))
        .with_props(props);
    if r.persistence == Persistence::Temporary {
        o = o.add_flags(ObjFlags::TEMP);
    }
    o
}

/// A column object; `not_null_constraint` is PostgreSQL 18's catalogued constraint name.
pub fn column_obj(c: &PgColumn, not_null_constraint: Option<&str>) -> Obj {
    let generated = c.generated.map(|g| MGenerated {
        kind: match g {
            Generated::Stored => GeneratedKind::Stored,
            Generated::Virtual => GeneratedKind::Virtual,
        },
        expr: n(c.default.as_deref().unwrap_or_default()),
    });
    let identity = c.identity.map(|i| match i {
        Identity::Always => MIdentity::Always,
        Identity::ByDefault => MIdentity::ByDefault,
    });
    let rare = ColumnRare {
        not_null_constraint: not_null_constraint.map(n),
        identity,
        generated: generated.clone(),
        collation: on(&c.collation),
        enum_labels: None,
    };
    let has_rare = rare.not_null_constraint.is_some()
        || rare.identity.is_some()
        || rare.generated.is_some()
        || rare.collation.is_some();
    let props = ColumnProps {
        ordinal: u32::try_from(c.num).unwrap_or(0),
        raw_type: n(&c.ty.display),
        type_ref: Some(TypeRef {
            schema: Some(n(&c.ty.schema)),
            name: n(&c.ty.name),
        }),
        nullable: !c.not_null,
        // A generated column's expression is in `pg_attrdef` too: it is the generation.
        default_raw: if generated.is_some() {
            None
        } else {
            on(&c.default)
        },
        comment: on(&c.comment),
        hidden: false,
        rare: has_rare.then(|| Arc::new(rare)),
        ext: None,
    };
    let mut o = Obj::new(ObjectKind::Column, n(&c.name)).with_props(Props::Column(Arc::new(props)));
    if c.inherited {
        o = o.add_flags(ObjFlags::INHERITED);
    }
    o
}

/// A trigger object.
pub fn trigger_obj(ns: &Ns, t: &PgTrigger) -> Obj {
    let mut events = TriggerEvents::empty();
    for e in &t.events {
        events |= match e {
            TriggerEvent::Insert => TriggerEvents::INSERT,
            TriggerEvent::Update => TriggerEvents::UPDATE,
            TriggerEvent::Delete => TriggerEvents::DELETE,
            TriggerEvent::Truncate => TriggerEvents::TRUNCATE,
        };
    }
    let (foid, fschema, fname) = &t.function;
    let props = TriggerProps {
        timing: match t.timing {
            TriggerTiming::Before => MTiming::Before,
            TriggerTiming::After => MTiming::After,
            TriggerTiming::InsteadOf => MTiming::InsteadOf,
        },
        events,
        for_each_row: t.row_level,
        function: Some(ns.ref_to(ObjectKind::Routine, fschema, fname, *foid)),
        when_expr: None,
        source: t.definition.as_deref().map(SourceRef::of),
        // `tgenabled` `D` is disabled; `O`, `R` and `A` fire (in some replication role).
        enabled: t.enabled != 'D',
        definer: None,
    };
    Obj::new(ObjectKind::Trigger, n(&t.name))
        .with_server_id(ServerId::Oid(t.oid))
        .with_props(Props::Trigger(Arc::new(props)))
}

/// A sequence object.
pub fn sequence_obj(ns: &Ns, s: &PgSequence) -> Obj {
    let props = SeqProps {
        owner: Some(n(&s.owner)),
        data_type: Some(n(&s.data_type)),
        start: Some(s.start),
        increment: Some(s.increment),
        min: Some(s.min),
        max: Some(s.max),
        cache: Some(s.cache),
        cycle: s.cycle,
        owned_by: s.owned_by.as_ref().map(|(oid, schema, table, col)| {
            (ns.ref_to(ObjectKind::Table, schema, table, *oid), n(col))
        }),
        comment: on(&s.comment),
    };
    let mut o = Obj::new(ObjectKind::Sequence, n(&s.name))
        .with_server_id(ServerId::Oid(s.oid))
        .with_props(Props::Sequence(Arc::new(props)));
    if s.identity {
        o = o.add_flags(ObjFlags::IMPLICIT);
    }
    o
}

/// The model kind of a routine.
pub fn routine_kind(k: RoutineKind) -> ObjectKind {
    match k {
        RoutineKind::Function | RoutineKind::Window => ObjectKind::Routine,
        RoutineKind::Procedure => ObjectKind::Procedure,
        RoutineKind::Aggregate => ObjectKind::Aggregate,
    }
}

/// A routine object (`overload_sig` = the identity arguments).
pub fn routine_obj(p: &PgRoutine) -> Obj {
    let args = p
        .args
        .iter()
        .map(|a| FnArg {
            name: on(&a.name),
            type_name: n(&a.type_text),
            mode: match a.mode {
                'o' => ArgMode::Out,
                'b' => ArgMode::InOut,
                'v' => ArgMode::Variadic,
                't' => ArgMode::Table,
                _ => ArgMode::In,
            },
            has_default: a.has_default,
        })
        .collect();
    let props = RoutineProps {
        args: Some(args),
        returns: on(&p.result),
        returns_set: p.returns_set,
        kind: match p.kind {
            RoutineKind::Function => FnKind::Function,
            RoutineKind::Procedure => FnKind::Procedure,
            RoutineKind::Aggregate => FnKind::Aggregate,
            RoutineKind::Window => FnKind::Window,
        },
        volatility: match p.volatility {
            Volatility::Immutable => MVolatility::Immutable,
            Volatility::Stable => MVolatility::Stable,
            Volatility::Volatile => MVolatility::Volatile,
        },
        language: Some(n(&p.language)),
        owner: Some(n(&p.owner)),
        definer: None,
        specific_name: None,
        security_definer: p.security_definer,
        overload_sig: Some(n(&p.identity_args)),
        source: p.definition.as_deref().map(SourceRef::of),
        comment: on(&p.comment),
    };
    Obj::new(routine_kind(p.kind), n(&p.name))
        .with_server_id(ServerId::Oid(p.oid))
        .with_props(Props::Routine(Arc::new(props)))
}

/// The model kind of a type.
pub fn type_kind(t: &UserTypeKind) -> ObjectKind {
    match t {
        UserTypeKind::Enum { .. } => ObjectKind::EnumType,
        UserTypeKind::Domain { .. } => ObjectKind::Domain,
        UserTypeKind::Composite { .. } => ObjectKind::CompositeType,
        UserTypeKind::Range { .. } => ObjectKind::RangeType,
        UserTypeKind::Base => ObjectKind::Unknown,
    }
}

/// A type object (a base type is `Unknown` with [`BASE_TYPE_RAW_KIND`]).
pub fn type_obj(t: &PgType) -> Obj {
    let kind = type_kind(&t.kind);
    let props = match &t.kind {
        UserTypeKind::Base => Props::Generic(Arc::new(GenericProps {
            owner: Some(n(&t.owner)),
            version: None,
            comment: on(&t.comment),
            raw_kind: Some(n(BASE_TYPE_RAW_KIND)),
        })),
        other => {
            let body = match other {
                UserTypeKind::Enum { labels } => TypeBody::Enum {
                    labels: labels.iter().map(|l| n(l)).collect(),
                },
                UserTypeKind::Domain {
                    base_display,
                    not_null,
                    default,
                    constraints,
                } => TypeBody::Domain {
                    base: n(base_display),
                    not_null: *not_null,
                    not_null_constraint: constraints
                        .iter()
                        .find(|k| k.kind == 'n')
                        .map(|k| n(&k.name)),
                    default_raw: on(default),
                    checks: constraints
                        .iter()
                        .filter(|k| k.kind == 'c')
                        .map(|k| n(check_body(&k.definition).unwrap_or(&k.definition)))
                        .collect(),
                },
                UserTypeKind::Composite { attrs, .. } => TypeBody::Composite {
                    attrs: attrs
                        .iter()
                        .map(|(_, name, ty)| TypeAttr {
                            name: n(name),
                            raw_type: n(ty),
                        })
                        .collect(),
                },
                UserTypeKind::Range { subtype } => TypeBody::Range {
                    subtype: n(subtype),
                },
                UserTypeKind::Base => TypeBody::Base,
            };
            Props::Type(Arc::new(TypeProps {
                owner: Some(n(&t.owner)),
                comment: on(&t.comment),
                body,
            }))
        }
    };
    let o = Obj::new(kind, n(&t.name)).with_server_id(ServerId::Oid(t.oid));
    o.with_props(props)
}

/// The number of `pg_constraint` rows a cached domain stood for (its checks and, from
/// PostgreSQL 17, its catalogued `NOT NULL`), for the `ListExistent` comparison.
pub fn domain_constraint_count(props: &Props) -> Option<usize> {
    match props {
        Props::Type(t) => match &t.body {
            TypeBody::Domain {
                not_null_constraint,
                checks,
                ..
            } => Some(checks.len() + usize::from(not_null_constraint.is_some())),
            _ => Some(0),
        },
        _ => None,
    }
}

/// Task 13's rows of one relation's indexes and constraints.
pub fn key_rows(
    table: &TableId,
    indexes: &[PgIndex],
    constraints: &[PgConstraint],
    into: &mut KeyRows,
) {
    let col = |(attnum, name): &(i16, Option<String>)| -> ColRef {
        ColRef {
            ordinal: u32::try_from(*attnum).ok(),
            name: on(name),
        }
    };
    for ix in indexes {
        into.indexes.push(IndexRow {
            table: table.clone(),
            oid: ix.oid,
            name: n(&ix.name),
            method: Some(n(&ix.method)),
            unique: ix.unique,
            valid: ix.valid,
            nulls_not_distinct: ix.nulls_not_distinct,
            parts: ix
                .keys
                .iter()
                .map(|k| IndexPart {
                    attnum: k.attnum,
                    name: on(&k.name),
                    expression: on(&k.expression),
                    desc: k.descending,
                    nulls_first: k.nulls_first,
                    opclass: on(&k.opclass),
                    collation: on(&k.collation),
                })
                .collect(),
            include: ix.include.iter().map(col).collect(),
            predicate: on(&ix.predicate),
            comment: on(&ix.comment),
        });
    }
    for k in constraints {
        let foreign = k.foreign.as_ref().map(|f| FkTarget {
            table: TableId::with_oid(f.table, n(&f.name)),
            schema: n(&f.schema),
            columns: f.columns.iter().map(col).collect(),
            on_update: ascii(f.on_update),
            on_delete: ascii(f.on_delete),
            match_type: ascii(f.match_type),
            index: k.index.as_ref().map(|(oid, name)| (*oid, n(name))),
        });
        into.constraints.push(ConstraintRow {
            table: table.clone(),
            oid: k.oid,
            name: n(&k.name),
            contype: k.kind.contype(),
            columns: k.columns.iter().map(col).collect(),
            definition: n(&k.definition),
            check_expr: on(&k.check_expr),
            deferrable: k.deferrable,
            deferred: k.deferred,
            validated: k.validated,
            enforced: k.enforced,
            no_inherit: k.no_inherit,
            local: k.local,
            parent: k.parent,
            index: if k.kind == ConstraintKind::ForeignKey {
                None
            } else {
                k.index.as_ref().map(|(oid, _)| *oid)
            },
            foreign,
            operators: k.exclusion_ops.iter().map(|o| n(o)).collect(),
        });
    }
}

fn ascii(c: char) -> u8 {
    u8::try_from(c).unwrap_or(b'?')
}

/// The column name of PostgreSQL 18's `NOT NULL` constraint per attnum.
pub fn not_null_names(constraints: &[PgConstraint]) -> HashMap<i16, String> {
    constraints
        .iter()
        .filter(|k| k.kind == ConstraintKind::NotNull)
        .filter_map(|k| k.columns.first().map(|(a, _)| (*a, k.name.clone())))
        .collect()
}

/// One relation and the children this pass read for it.
#[derive(Debug, Clone)]
pub struct RelRead {
    pub oid: Oid,
    pub name: String,
    /// The re-read own row (`None`: the cached object stays, only children are replaced).
    pub own: Option<Obj>,
    pub columns: Option<Vec<PgColumn>>,
    pub keys: Option<(Vec<PgIndex>, Vec<PgConstraint>)>,
    pub triggers: Option<Vec<PgTrigger>>,
}

/// The assembled children of the relations of one pass, by relation oid.
#[derive(Debug, Default)]
pub struct Children {
    pub columns: BTreeMap<Oid, Vec<Obj>>,
    pub keys: BTreeMap<Oid, Vec<Obj>>,
    pub triggers: BTreeMap<Oid, Vec<Obj>>,
}

/// Builds the children of `rels`. `cached_columns` gives a relation's `(attnum, name)`
/// columns when this pass did not read them (an incremental key re-read), so key columns
/// resolve; `complete` says whether `rels` is every relation of the namespace.
pub fn children(
    ns: &Ns,
    rels: &[RelRead],
    cached_columns: &dyn Fn(Oid) -> Option<Vec<(u32, Name)>>,
    complete: bool,
) -> Children {
    let mut out = Children::default();
    let mut index = ColumnIndex::new(
        ns.key.clone(),
        hacivat_meta::name::DialectId::Postgres,
        hacivat_meta::name::IdentCtx::new(hacivat_meta::name::DialectId::Postgres),
    );
    index.set_complete(complete);
    let mut rows = KeyRows::default();
    for r in rels {
        let table = TableId::with_oid(r.oid, n(&r.name));
        let cols: Vec<(u32, Name)> = match &r.columns {
            Some(cs) => cs
                .iter()
                .map(|c| (u32::try_from(c.num).unwrap_or(0), n(&c.name)))
                .collect(),
            None => cached_columns(r.oid).unwrap_or_default(),
        };
        index.add_table(table.clone(), cols);
        if let Some(cs) = &r.columns {
            let nn = r
                .keys
                .as_ref()
                .map(|(_, k)| not_null_names(k))
                .unwrap_or_default();
            out.columns.insert(
                r.oid,
                cs.iter()
                    .map(|c| column_obj(c, nn.get(&c.num).map(String::as_str)))
                    .collect(),
            );
        }
        if let Some((ix, k)) = &r.keys {
            key_rows(&table, ix, k, &mut rows);
            out.keys.entry(r.oid).or_default();
        }
        if let Some(ts) = &r.triggers {
            out.triggers
                .insert(r.oid, ts.iter().map(|t| trigger_obj(ns, t)).collect());
        }
    }
    for KeyObj { table, obj } in assemble_keys(rows, &index) {
        if let Some(ServerId::Oid(oid)) = table.server_id {
            out.keys.entry(oid).or_default().push(obj);
        }
    }
    out
}

/// A subtree: `root` with `kids` under it.
pub fn subtree(root: Obj, kids: impl IntoIterator<Item = Obj>) -> Subtree {
    let mut t = Subtree::new(root);
    let r = t.root();
    for k in kids {
        t.add(r, k);
    }
    t
}

/// The existence identity of a top-level object.
pub fn top_identity(o: &Obj) -> ExistId {
    ExistId {
        path: vec![o.name().clone()],
        server_id: o.server_id(),
        overload_sig: o.props().overload_sig().cloned(),
        raw_kind: o.raw_kind().cloned(),
        state: StateMark::None,
    }
}

/// The existence identity of a child `o` of relation `table`.
pub fn child_identity(table: &str, o: &Obj) -> ExistId {
    ExistId {
        path: vec![n(table), o.name().clone()],
        server_id: o.server_id(),
        overload_sig: None,
        raw_kind: None,
        state: StateMark::None,
    }
}

/// The model kind of a `ListExistent` row.
pub fn existent_kind(tag: &str, sub: Option<&str>) -> Option<ObjectKind> {
    let first = sub.and_then(|s| s.chars().next());
    Some(match (tag, first) {
        ("rel", Some(c)) => rel_kind(RelKind::from_relkind(c)?),
        ("seq", _) => ObjectKind::Sequence,
        ("idx", _) => ObjectKind::Index,
        ("con", Some('p')) => ObjectKind::PrimaryKey,
        ("con", Some('u')) => ObjectKind::UniqueKey,
        ("con", Some('f')) => ObjectKind::ForeignKey,
        ("con", Some('c')) => ObjectKind::Check,
        ("con", Some('x')) => ObjectKind::Exclusion,
        ("trg", _) => ObjectKind::Trigger,
        ("fn", Some('p')) => ObjectKind::Procedure,
        ("fn", Some('a')) => ObjectKind::Aggregate,
        ("fn", Some('f' | 'w')) => ObjectKind::Routine,
        ("typ", Some('e')) => ObjectKind::EnumType,
        ("typ", Some('d')) => ObjectKind::Domain,
        ("typ", Some('c')) => ObjectKind::CompositeType,
        ("typ", Some('r')) => ObjectKind::RangeType,
        ("typ", Some('b')) => ObjectKind::Unknown,
        _ => return None,
    })
}
