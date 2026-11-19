// ## m4-t17-mysql
//! State stamps and the cheap namespace check (HV-INTRO-15, MySQL part): the raw marks of
//! named objects from the `StateOf<Kind>` sections (30-state), and [`NsCheck`] computed the
//! same way from the server's marker rows and from a cached namespace, so Task 22 compares like
//! with like.
//!
//! - A relation's mark is R-M4-39's table stamp (`CREATE_TIME | max(TRIGGERS.CREATED) |
//!   trigger count`), a view's its definition hash; a routine's and an event's
//!   `LAST_ALTERED`; a trigger's `CREATED`.
//! - A child key (a column, index, key, check; an argument) answers its top-level parent's
//!   mark: the parent's stamp moves whenever the child's definition does (the re-read unit is
//!   the parent), and `None` when the parent is gone.
//! - A key of a kind a MySQL schema cannot hold answers `None` (it does not exist there).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use hacivat_meta::{Name, NamespaceSnapshot, ObjectKey, ObjectKind, StateMark};

use super::build::{micros, raw_kind, relation_kind, relation_mark, routine_type_of};
use super::pass::{SectionReader, list_params};
use super::rows::{self, cell, name, opt_name};
use super::server::ServerCtx;
use crate::freshness::NsCheck;
use crate::introspector::IntroError;
use crate::pack::Mode;
use hacivat_meta::Level;

/// One relation's `StateOfRelation` rows folded: its type, `CREATE_TIME`, view hash, and the
/// newest trigger `CREATED` with the trigger count.
type RelationState = (Name, Option<Name>, Option<u64>, (i64, u32));

/// What a key's stamp is read from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Probe {
    /// A relation (or a child of one): `(kind, raw kind)` of the key when it is the relation
    /// itself, `None` for a child.
    Relation(Name, Option<(ObjectKind, Option<Name>)>),
    /// A routine (or one of its arguments): `(ROUTINE_TYPE, specific name)`.
    Routine(&'static str, Name),
    Trigger(Name, Name),
    Event(Name),
    /// No such object can exist in a MySQL schema.
    Absent,
}

fn probe_of(k: &ObjectKey) -> Probe {
    let p = &k.path;
    match (k.kind, p.len()) {
        (ObjectKind::Table | ObjectKind::View | ObjectKind::Sequence, 2) => {
            Probe::Relation(p[1].clone(), Some((k.kind, None)))
        }
        (ObjectKind::Unknown, 2) => Probe::Relation(
            p[1].clone(),
            Some((ObjectKind::Unknown, k.raw_kind.clone())),
        ),
        (ObjectKind::Routine | ObjectKind::Procedure, 2) => {
            Probe::Routine(routine_type_of(k.kind).unwrap_or(""), p[1].clone())
        }
        (ObjectKind::Argument, 3) => Probe::Routine("", p[1].clone()),
        (ObjectKind::Event, 2) => Probe::Event(p[1].clone()),
        (ObjectKind::Trigger, 3) => Probe::Trigger(p[1].clone(), p[2].clone()),
        (
            ObjectKind::Column
            | ObjectKind::Index
            | ObjectKind::PrimaryKey
            | ObjectKind::UniqueKey
            | ObjectKind::ForeignKey
            | ObjectKind::Check
            | ObjectKind::Partition
            | ObjectKind::Subpartition,
            3,
        ) => Probe::Relation(p[1].clone(), None),
        _ => Probe::Absent,
    }
}

/// The raw marks of `keys` (see the module docs), reading each schema's objects with the
/// `StateOf<Kind>` sections.
pub async fn stamps(
    r: &mut dyn SectionReader,
    srv: &ServerCtx,
    keys: &[ObjectKey],
) -> Result<Vec<Option<StateMark>>, IntroError> {
    let mut out: Vec<Option<StateMark>> = vec![None; keys.len()];
    let mut by_schema: BTreeMap<Name, Vec<(usize, Probe)>> = BTreeMap::new();
    for (i, k) in keys.iter().enumerate() {
        let Some(schema) = k.path.first() else {
            continue;
        };
        by_schema
            .entry(schema.clone())
            .or_default()
            .push((i, probe_of(k)));
    }
    for (schema, probes) in by_schema {
        let mut rels = BTreeSet::new();
        let mut rtns = BTreeSet::new();
        let mut trgs = BTreeSet::new();
        let mut evts = BTreeSet::new();
        for (_, p) in &probes {
            match p {
                Probe::Relation(n, _) => {
                    rels.insert(n.clone());
                }
                Probe::Routine(_, n) => {
                    rtns.insert(n.clone());
                }
                Probe::Trigger(_, n) => {
                    trgs.insert(n.clone());
                }
                Probe::Event(n) => {
                    evts.insert(n.clone());
                }
                Probe::Absent => {}
            }
        }
        let ctx = |names: &BTreeSet<Name>| {
            srv.pack_ctx(Mode::Full, Level::Structure, list_params(&schema, names))
        };
        // Relations: one row per trigger (or one with NULL trigger columns).
        let mut relations: HashMap<Name, RelationState> = HashMap::new();
        if !rels.is_empty() {
            let rows = r.read("StateOfRelation", ctx(&rels)).await?;
            for row in rows::checked("StateOfRelation", &rows, 6)? {
                let e = relations.entry(name(row, 0)).or_insert_with(|| {
                    (
                        name(row, 1),
                        opt_name(row, 2),
                        cell(row, 3).and_then(rows::hash_mark),
                        (0, 0),
                    )
                });
                if cell(row, 4).is_some() {
                    e.3.0 = e.3.0.max(micros(opt_name(row, 5).as_ref()).unwrap_or(0));
                    e.3.1 = e.3.1.saturating_add(1);
                }
            }
        }
        let mut routines: HashMap<(Name, Name), StateMark> = HashMap::new();
        if !rtns.is_empty() {
            let rows = r.read("StateOfRoutine", ctx(&rtns)).await?;
            for row in rows::checked("StateOfRoutine", &rows, 3)? {
                let mark =
                    micros(opt_name(row, 2).as_ref()).map_or(StateMark::None, StateMark::Modified);
                routines.insert((name(row, 1), name(row, 0)), mark);
            }
        }
        let mut triggers: HashMap<(Name, Name), StateMark> = HashMap::new();
        if !trgs.is_empty() {
            let rows = r.read("StateOfTrigger", ctx(&trgs)).await?;
            for row in rows::checked("StateOfTrigger", &rows, 3)? {
                let mark =
                    micros(opt_name(row, 2).as_ref()).map_or(StateMark::None, StateMark::Created);
                triggers.insert((name(row, 1), name(row, 0)), mark);
            }
        }
        let mut events: HashMap<Name, StateMark> = HashMap::new();
        if !evts.is_empty() {
            let rows = r.read("StateOfEvent", ctx(&evts)).await?;
            for row in rows::checked("StateOfEvent", &rows, 2)? {
                let mark =
                    micros(opt_name(row, 1).as_ref()).map_or(StateMark::None, StateMark::Modified);
                events.insert(name(row, 0), mark);
            }
        }
        for (i, p) in probes {
            out[i] = match p {
                Probe::Relation(n, want) => relations.get(&n).and_then(|(ty, ct, hash, trg)| {
                    let kind = relation_kind(ty, srv.is_mariadb());
                    if let Some((wk, wraw)) = &want
                        && (*wk != kind
                            || (kind == ObjectKind::Unknown && *wraw != raw_kind(kind, ty)))
                    {
                        // The name now holds another kind of relation: this object is gone.
                        return None;
                    }
                    Some(relation_mark(kind, ct.as_ref(), *hash, Some(trg)))
                }),
                Probe::Routine("", n) => routines
                    .iter()
                    .find(|((_, spec), _)| *spec == n)
                    .map(|(_, m)| *m),
                Probe::Routine(ty, n) => routines.get(&(Name::from(ty), n)).copied(),
                Probe::Trigger(table, n) => triggers.get(&(table, n)).copied(),
                Probe::Event(n) => events.get(&n).copied(),
                Probe::Absent => None,
            };
        }
    }
    Ok(out)
}

/// One top-level object as the namespace check sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopMark {
    pub kind: ObjectKind,
    pub raw_kind: Option<Name>,
    pub name: Name,
    pub mark: StateMark,
}

/// The latest time a mark holds (`None` for a hash or no mark).
fn latest(m: &StateMark) -> Option<i64> {
    match *m {
        StateMark::MyTable {
            created,
            trigger_max,
            ..
        } => Some(created.max(trigger_max)),
        StateMark::Modified(t) | StateMark::Created(t) => Some(t),
        _ => None,
    }
}

/// [`NsCheck`] of top-level objects: their count, the latest time any mark holds
/// (`StateMark::Modified`), and the XXH3-64 of the sorted `kind id, raw kind, name, mark`
/// lines (a dropped trigger moves its table's count, a replaced view its hash).
pub fn ns_check(objs: &[TopMark]) -> NsCheck {
    let mut lines: Vec<String> = objs
        .iter()
        .map(|o| {
            format!(
                "{}\t{}\t{}\t{:?}",
                o.kind.id(),
                o.raw_kind.as_deref().unwrap_or(""),
                o.name,
                o.mark
            )
        })
        .collect();
    lines.sort();
    let max = objs.iter().filter_map(|o| latest(&o.mark)).max();
    NsCheck {
        objects: u64::try_from(objs.len()).unwrap_or(u64::MAX),
        max_mark: max.map_or(StateMark::None, StateMark::Modified),
        existence_digest: twox_hash::XxHash3_64::oneshot(lines.join("\n").as_bytes()),
    }
}

/// The top-level objects of a cached MySQL namespace, as [`ns_check`] reads them.
pub fn top_marks_of(snap: &NamespaceSnapshot) -> Vec<TopMark> {
    snap.children(0)
        .map(|(_, o)| TopMark {
            kind: o.kind(),
            raw_kind: o.raw_kind().cloned(),
            name: o.name().clone(),
            mark: o.state(),
        })
        .collect()
}

/// The server's top-level objects with their marks (the incremental pass's marker sections).
pub async fn server_top_marks(
    r: &mut dyn SectionReader,
    srv: &ServerCtx,
    schema: &Name,
) -> Result<Vec<TopMark>, IntroError> {
    let ctx = || {
        srv.pack_ctx(
            Mode::Full,
            Level::Structure,
            super::pass::schema_params(schema),
        )
    };
    let tables = rows::table_marks(&r.read("TableMarks", ctx()).await?)?;
    let views: HashMap<Name, Option<u64>> = rows::view_marks(&r.read("ViewMarks", ctx()).await?)?
        .into_iter()
        .collect();
    let tstats =
        super::build::trigger_stats(&rows::trigger_marks(&r.read("TriggerMarks", ctx()).await?)?);
    let routines = rows::routine_marks(&r.read("RoutineMarks", ctx()).await?)?;
    let events = rows::event_marks(&r.read("EventMarks", ctx()).await?)?;
    let mut out = Vec::new();
    for t in tables {
        let kind = relation_kind(&t.table_type, srv.is_mariadb());
        out.push(TopMark {
            kind,
            raw_kind: raw_kind(kind, &t.table_type),
            mark: relation_mark(
                kind,
                t.create_time.as_ref(),
                views.get(&t.name).copied().flatten(),
                tstats.get(&t.name),
            ),
            name: t.name,
        });
    }
    for (spec, ty, last) in routines {
        let kind = super::build::routine_kind(&ty);
        out.push(TopMark {
            kind,
            raw_kind: raw_kind(kind, &ty),
            name: spec,
            mark: micros(last.as_ref()).map_or(StateMark::None, StateMark::Modified),
        });
    }
    for e in events {
        out.push(TopMark {
            kind: ObjectKind::Event,
            raw_kind: None,
            mark: micros(e.last_altered.as_ref()).map_or(StateMark::None, StateMark::Modified),
            name: e.name,
        });
    }
    Ok(out)
}
