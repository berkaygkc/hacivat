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

/// One relation's `StateOfRelation` rows folded: its type, `CREATE_TIME`, view definition hash
/// and whether the definition is readable, and the newest trigger `CREATED` with the trigger
/// count.
type RelationState = (Name, Option<Name>, Option<u64>, bool, (i64, u32));

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
            for row in rows::checked("StateOfRelation", &rows, 7)? {
                let e = relations.entry(name(row, 0)).or_insert_with(|| {
                    (
                        name(row, 1),
                        opt_name(row, 2),
                        cell(row, 3).and_then(rows::hash_mark),
                        cell(row, 4) == Some("YES"),
                        (0, 0),
                    )
                });
                if cell(row, 5).is_some() {
                    e.4.0 = e.4.0.max(micros(opt_name(row, 6).as_ref()).unwrap_or(0));
                    e.4.1 = e.4.1.saturating_add(1);
                }
            }
        }
        // A view's stamp folds in its columns, as the passes' marks do (review I-1).
        let mut view_cols: Vec<rows::ViewCol> = Vec::new();
        let views: BTreeSet<Name> = relations
            .iter()
            .filter(|(_, s)| relation_kind(&s.0, srv.is_mariadb()) == ObjectKind::View)
            .map(|(n, _)| n.clone())
            .collect();
        if !views.is_empty() {
            let ctx = srv.pack_ctx(
                Mode::Incremental,
                Level::Structure,
                list_params(&schema, &views),
            );
            view_cols = rows::view_cols(&r.read("ViewColumnMarks", ctx).await?)?;
        }
        let view_marks = super::build::view_marks_with_columns(
            relations
                .iter()
                .filter(|(n, _)| views.contains(*n))
                .map(|(n, s)| (n.clone(), s.2, s.3))
                .collect(),
            &view_cols,
        );
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
                Probe::Relation(n, want) => relations.get(&n).and_then(|(ty, ct, _, _, trg)| {
                    let kind = relation_kind(ty, srv.is_mariadb());
                    if let Some((wk, wraw)) = &want
                        && (*wk != kind
                            || (kind == ObjectKind::Unknown && *wraw != raw_kind(kind, ty)))
                    {
                        // The name now holds another kind of relation: this object is gone.
                        return None;
                    }
                    let view = view_marks.get(&n).map(|(m, _)| *m);
                    Some(relation_mark(kind, ct.as_ref(), view, Some(trg)))
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
    let views = super::build::view_marks_with_columns(
        rows::view_marks(&r.read("ViewMarks", ctx()).await?)?,
        &rows::view_cols(&r.read("ViewColumnMarks", ctx()).await?)?,
    );
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
                views.get(&t.name).map(|(m, _)| *m),
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

// ## m4-t22-state
// Task 22 (fix round 1 I-2, I-3, I-5): the cached side of the namespace check vouches only for
// a MySQL schema read by an incremental-capable pass (`Watermark::MyTime`: MySQL 8.0+, R-M4-10
// keeps MariaDB `Full`), folds that watermark's server identity, version and privilege hash
// into its digest (the server side folds the same facts read live: another server, another
// version or other privileges never compare equal, R-W3-3), and never vouches while any top
// mark is inside R-M4-39's one-second window of that watermark (two DDLs in one second leave
// one `CREATE_TIME`: the incremental pass re-reads it, so validation must too).

/// The `max_mark` of a cached side that cannot vouch because a mark is in the window: a
/// `Hash`, which the server side (`Modified` or `None`) never answers, so the verdict is
/// `Changed`.
pub const IN_WINDOW: StateMark = StateMark::Hash(0x4d59_5749_4e44_4f57);

/// `c` with the server's identity, version and privilege hash folded into its digest.
pub fn fold_facts(mut c: NsCheck, server_uuid: &str, version: &str, priv_hash: u64) -> NsCheck {
    let mut b = Vec::with_capacity(32 + server_uuid.len() + version.len());
    b.extend_from_slice(&c.existence_digest.to_le_bytes());
    for s in [server_uuid, version] {
        b.extend_from_slice(&(s.len() as u64).to_le_bytes());
        b.extend_from_slice(s.as_bytes());
    }
    b.extend_from_slice(&priv_hash.to_le_bytes());
    c.existence_digest = twox_hash::XxHash3_64::oneshot(&b);
    c
}

/// The cached side (see the block docs); `None`: validation reads the namespace again.
pub fn cached_check(snap: &NamespaceSnapshot, flavor: hacivat_proto::DbmsKind) -> Option<NsCheck> {
    if flavor != hacivat_proto::DbmsKind::MySql || snap.key().kind != hacivat_meta::NsKind::Schema {
        return None;
    }
    let hacivat_meta::Watermark::MyTime {
        at_utc_micros,
        server_uuid,
        version,
        priv_hash,
    } = &snap.meta().watermark
    else {
        return None;
    };
    let marks = top_marks_of(snap);
    let mut c = ns_check(&marks);
    if marks
        .iter()
        .any(|m| super::pass::in_window(&m.mark, *at_utc_micros))
    {
        c.max_mark = IN_WINDOW;
    }
    Some(fold_facts(c, server_uuid, version, *priv_hash))
}
// ## m4-t22-state
