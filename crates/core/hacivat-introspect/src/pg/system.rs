// ## m4-t16-pg
//! System mode (R-M4-44, `Mode::System`): the built-in objects (OID < 16384) of `pg_catalog`
//! or `information_schema`, for Task 23's snapshots and for the live system introspection of a
//! version without a snapshot (R-M4-6). [`read_rows`] returns the raw rows (the generator's
//! input); [`read`] the same objects as a full `NamespaceResult`: relations with their
//! columns, routines, operators (`Operator`, generic props) and types (`Unknown` with the raw
//! kind `pg-type`), every object flagged `SYSTEM`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use hacivat_meta::merge::{ChangedObject, ExistSet, NamespaceResult};
use hacivat_meta::props::{
    ColumnProps, FnKind, GenericProps, Props, RoutineProps, StatsFacet, TableProps, ViewProps,
    Volatility,
};
use hacivat_meta::{Level, Name, Obj, ObjFlags, ObjectKey, ObjectKind, ServerId};
use hacivat_session::meta::MetaConn;
use tokio_util::sync::CancellationToken;

use super::build::{self, Ns};
use super::decode::Row;
use super::model::Oid;
use super::sections::{Binds, Reader, name};
use super::watermark::Now;
use crate::introspector::IntroError;
use crate::pack::Mode;

/// The raw kind of a system type in the model.
pub const SYSTEM_TYPE_RAW_KIND: &str = "pg-type";

/// One built-in relation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SysRelation {
    pub oid: Oid,
    pub name: String,
    pub relkind: char,
    pub comment: Option<String>,
    /// `(attnum, name, type text, not null)`.
    pub columns: Vec<(i16, String, String, bool)>,
}

/// One built-in routine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SysRoutine {
    pub oid: Oid,
    pub name: String,
    pub prokind: char,
    pub args: String,
    /// Only when it differs from `args`.
    pub identity_args: Option<String>,
    pub result: Option<String>,
    pub volatility: char,
    pub returns_set: bool,
    pub comment: Option<String>,
}

/// One built-in operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SysOperator {
    pub oid: Oid,
    pub name: String,
    pub kind: char,
    pub left: Option<String>,
    pub right: Option<String>,
    pub result: String,
    pub function: String,
    pub comment: Option<String>,
}

/// One built-in type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SysType {
    pub oid: Oid,
    pub name: String,
    pub sql_name: String,
    pub typtype: char,
    pub category: char,
    pub element: Option<String>,
    pub comment: Option<String>,
}

/// One setting (GUC) name with its type and context, never its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SysSetting {
    pub name: String,
    pub vartype: String,
    pub context: String,
}

/// The built-in objects of one system schema.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SystemRows {
    pub relations: Vec<SysRelation>,
    pub routines: Vec<SysRoutine>,
    pub operators: Vec<SysOperator>,
    pub types: Vec<SysType>,
}

async fn rows_of(r: &mut Reader<'_, '_>, ns: Oid) -> Result<SystemRows, IntroError> {
    let b = Binds::ns(ns);
    let (m, l) = (Mode::System, Level::Structure);
    let mut cols: BTreeMap<Oid, Vec<(i16, String, String, bool)>> = BTreeMap::new();
    for (rel, c) in r
        .read(name::SYSTEM_COLUMNS, m, l, &b, |x: &Row<'_>| {
            Ok((
                x.num::<Oid>(0)?,
                (x.num(1)?, x.string(2)?, x.string(3)?, x.bool(4)?),
            ))
        })
        .await?
    {
        cols.entry(rel).or_default().push(c);
    }
    let relations = r
        .read(name::SYSTEM_RELATIONS, m, l, &b, |x: &Row<'_>| {
            Ok(SysRelation {
                oid: x.num(0)?,
                name: x.string(1)?,
                relkind: x.char(2)?,
                comment: x.opt_string(3),
                columns: Vec::new(),
            })
        })
        .await?
        .into_iter()
        .map(|mut s| {
            s.columns = cols.remove(&s.oid).unwrap_or_default();
            s
        })
        .collect();
    let routines = r
        .read(name::SYSTEM_ROUTINES, m, l, &b, |x: &Row<'_>| {
            Ok(SysRoutine {
                oid: x.num(0)?,
                name: x.string(1)?,
                prokind: x.char(2)?,
                args: x.string(3)?,
                identity_args: x.opt_string(4),
                result: x.opt_string(5),
                volatility: x.char(6)?,
                returns_set: x.bool(7)?,
                comment: x.opt_string(8),
            })
        })
        .await?;
    let operators = r
        .read(name::SYSTEM_OPERATORS, m, l, &b, |x: &Row<'_>| {
            Ok(SysOperator {
                oid: x.num(0)?,
                name: x.string(1)?,
                kind: x.char(2)?,
                left: x.opt_string(3),
                right: x.opt_string(4),
                result: x.string(5)?,
                function: x.string(6)?,
                comment: x.opt_string(7),
            })
        })
        .await?;
    let types = r
        .read(name::SYSTEM_TYPES, m, l, &b, |x: &Row<'_>| {
            Ok(SysType {
                oid: x.num(0)?,
                name: x.string(1)?,
                sql_name: x.string(2)?,
                typtype: x.char(3)?,
                category: x.char(4)?,
                element: x.opt_string(5),
                comment: x.opt_string(6),
            })
        })
        .await?;
    Ok(SystemRows {
        relations,
        routines,
        operators,
        types,
    })
}

/// The built-in objects of `schema` (`pg_catalog` or `information_schema`) on `c`'s database
/// (Task 23 reads database `postgres`).
pub async fn read_rows(
    c: &mut MetaConn,
    schema: &str,
    cancel: &CancellationToken,
) -> Result<SystemRows, IntroError> {
    let mut report = super::pass::PassReport::default();
    let schema = Name::from(schema);
    super::pass::in_snapshot(c, &schema, cancel, &mut report, None, async |r, id| {
        let ns = id
            .ns_oid
            .ok_or_else(|| IntroError::Protocol(Name::from("pg/namespace-gone")))?;
        rows_of(r, ns).await
    })
    .await
}

/// The setting names with their type and context (`ListSettings`).
pub async fn list_settings(
    c: &mut MetaConn,
    cancel: &CancellationToken,
) -> Result<Vec<SysSetting>, IntroError> {
    let mut report = super::pass::PassReport::default();
    super::pass::in_snapshot(
        c,
        &Name::from("pg_catalog"),
        cancel,
        &mut report,
        None,
        async |r, _| {
            r.read(
                name::LIST_SETTINGS,
                Mode::System,
                Level::Names,
                &Binds::default(),
                |x: &Row<'_>| {
                    Ok(SysSetting {
                        name: x.string(0)?,
                        vartype: x.string(1)?,
                        context: x.string(2)?,
                    })
                },
            )
            .await
        },
    )
    .await
}

fn sys(o: Obj) -> Obj {
    o.add_flags(ObjFlags::SYSTEM)
}

fn on(s: &Option<String>) -> Option<Name> {
    s.as_deref().map(Name::from)
}

/// The system schema as a full `NamespaceResult` (module docs).
pub(super) async fn read(
    r: &mut Reader<'_, '_>,
    nsx: &Ns,
    ns: Oid,
    now: &Now,
) -> Result<(NamespaceResult, Vec<(ObjectKey, StatsFacet)>), IntroError> {
    let rows = rows_of(r, ns).await?;
    let root = nsx.root_key();
    let mut changed = Vec::new();
    let mut ids: BTreeMap<ObjectKind, Vec<hacivat_meta::merge::Identity>> = BTreeMap::new();
    let mut col_ids = Vec::new();
    let mut emit = |o: Obj, kids: Vec<Obj>| {
        ids.entry(o.kind())
            .or_default()
            .push(build::top_identity(&o));
        for k in &kids {
            col_ids.push(build::child_identity(o.name(), k));
        }
        changed.push(ChangedObject {
            parent: root.clone(),
            subtree: build::subtree(o, kids),
        });
    };
    for rel in &rows.relations {
        let (kind, props) = match rel.relkind {
            'v' | 'm' => (
                if rel.relkind == 'v' {
                    ObjectKind::View
                } else {
                    ObjectKind::MaterializedView
                },
                Props::View(Arc::new(ViewProps {
                    comment: on(&rel.comment),
                    ..ViewProps::default()
                })),
            ),
            _ => (
                ObjectKind::Table,
                Props::Table(Arc::new(TableProps {
                    comment: on(&rel.comment),
                    ..TableProps::default()
                })),
            ),
        };
        let o = sys(Obj::new(kind, Name::from(rel.name.as_str()))
            .with_server_id(ServerId::Oid(rel.oid))
            .with_props(props));
        let kids = rel
            .columns
            .iter()
            .map(|(num, name, ty, not_null)| {
                sys(
                    Obj::new(ObjectKind::Column, Name::from(name.as_str())).with_props(
                        Props::Column(Arc::new(ColumnProps {
                            ordinal: u32::try_from(*num).unwrap_or(0),
                            raw_type: Name::from(ty.as_str()),
                            nullable: !*not_null,
                            ..ColumnProps::default()
                        })),
                    ),
                )
            })
            .collect();
        emit(o, kids);
    }
    for p in &rows.routines {
        let kind = match p.prokind {
            'p' => ObjectKind::Procedure,
            'a' => ObjectKind::Aggregate,
            _ => ObjectKind::Routine,
        };
        let props = RoutineProps {
            args: None,
            returns: on(&p.result),
            returns_set: p.returns_set,
            kind: match p.prokind {
                'p' => FnKind::Procedure,
                'a' => FnKind::Aggregate,
                'w' => FnKind::Window,
                _ => FnKind::Function,
            },
            volatility: match p.volatility {
                'i' => Volatility::Immutable,
                's' => Volatility::Stable,
                'v' => Volatility::Volatile,
                _ => Volatility::Unknown,
            },
            overload_sig: Some(Name::from(
                p.identity_args.as_deref().unwrap_or(p.args.as_str()),
            )),
            comment: on(&p.comment),
            ..RoutineProps::default()
        };
        emit(
            sys(Obj::new(kind, Name::from(p.name.as_str()))
                .with_server_id(ServerId::Oid(p.oid))
                .with_props(Props::Routine(Arc::new(props)))),
            Vec::new(),
        );
    }
    for o in &rows.operators {
        emit(
            sys(Obj::new(ObjectKind::Operator, Name::from(o.name.as_str()))
                .with_server_id(ServerId::Oid(o.oid))
                .with_props(Props::Generic(Arc::new(GenericProps {
                    comment: on(&o.comment),
                    ..GenericProps::default()
                })))),
            Vec::new(),
        );
    }
    for t in &rows.types {
        emit(
            sys(Obj::new(ObjectKind::Unknown, Name::from(t.name.as_str()))
                .with_server_id(ServerId::Oid(t.oid))
                .with_props(Props::Generic(Arc::new(GenericProps {
                    comment: on(&t.comment),
                    raw_kind: Some(Name::from(SYSTEM_TYPE_RAW_KIND)),
                    ..GenericProps::default()
                })))),
            Vec::new(),
        );
    }
    let mut existent = BTreeMap::new();
    for k in [
        ObjectKind::Table,
        ObjectKind::View,
        ObjectKind::MaterializedView,
        ObjectKind::Routine,
        ObjectKind::Procedure,
        ObjectKind::Aggregate,
        ObjectKind::Operator,
        ObjectKind::Unknown,
    ] {
        existent.insert(k, ExistSet::Ok(ids.remove(&k).unwrap_or_default()));
    }
    existent.insert(ObjectKind::Column, ExistSet::Ok(col_ids));
    Ok((
        NamespaceResult {
            existent,
            changed,
            categories_to_reload: BTreeSet::new(),
            new_watermark: now.watermark(),
            full: true,
            level: Level::Structure,
            read_at_ms: super::pass::now_ms(),
        },
        Vec::new(),
    ))
}
