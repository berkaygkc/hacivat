// ## m4-t16-pg
//! `StateOf<Kind>` stamps (HV-INTRO-15; R-M4-48; `packs/pg/30-state.hvq.sql`): the raw state
//! marks of objects for `check_state`. Each section returns, per object oid, the xmins of its
//! own row, its comment rows, its children's rows and the rows one `pg_depend` hop away; the
//! mark is an XXH3-64 of the sorted rows, so a comment removal, a second trigger or a renamed
//! dependency moves it. Task 22 compares marks with the snapshot's by equality (its fenced
//! `// ## m4-t22-state` blocks go in this file).

use std::collections::{BTreeMap, BTreeSet};

use hacivat_meta::{Level, Name, ObjectKey, ObjectKind, ServerId, StateMark};
use hacivat_session::meta::MetaConn;
use tokio_util::sync::CancellationToken;

use super::decode::IdentityRow;
use super::model::Oid;
use super::pass::{PassHooks, PassReport, in_snapshot};
use super::sections::{Binds, Reader, name};
use crate::introspector::IntroError;
use crate::pack::Mode;

/// The state section of a key's kind (`None`: the kind has no PostgreSQL oid of its own).
pub fn state_section(kind: ObjectKind) -> Option<&'static str> {
    use ObjectKind as K;
    Some(match kind {
        K::Table | K::View | K::MaterializedView | K::ForeignTable | K::Sequence | K::Index => {
            name::STATE_OF_RELATION
        }
        K::Routine | K::Procedure | K::Aggregate => name::STATE_OF_ROUTINE,
        K::Domain | K::EnumType | K::CompositeType | K::RangeType | K::Unknown => {
            name::STATE_OF_TYPE
        }
        K::PrimaryKey | K::UniqueKey | K::ForeignKey | K::Check | K::Exclusion => {
            name::STATE_OF_CONSTRAINT
        }
        K::Trigger => name::STATE_OF_TRIGGER,
        _ => return None,
    })
}

/// The raw state marks of `keys`: `Some(StateMark::Hash)` per object, `None` when its own row
/// is gone. A key without a PostgreSQL oid (a column) has no mark of its own:
/// `Some(StateMark::None)`.
pub async fn stamps(
    c: &mut MetaConn,
    keys: &[ObjectKey],
    cancel: &CancellationToken,
) -> Result<(Vec<Option<StateMark>>, PassReport), IntroError> {
    let mut report = PassReport::default();
    let mut by_section: BTreeMap<&'static str, BTreeSet<Oid>> = BTreeMap::new();
    for k in keys {
        if let (Some(s), Some(ServerId::Oid(o))) = (state_section(k.kind), k.server_id) {
            by_section.entry(s).or_default().insert(o);
        }
    }
    let rows = in_snapshot(
        c,
        &Name::from("pg_catalog"),
        cancel,
        &mut report,
        PassHooks::default(),
        async |r: &mut Reader<'_, '_>, _id: &IdentityRow| {
            let mut out: BTreeMap<(&'static str, Oid), Vec<(String, String)>> = BTreeMap::new();
            for (&section, oids) in &by_section {
                let binds = Binds::default().with_oids(oids.iter().copied().collect());
                for (o, src, stamp) in r
                    .read(section, Mode::Full, Level::Structure, &binds, |row| {
                        Ok((row.num::<Oid>(0)?, row.string(1)?, row.string(2)?))
                    })
                    .await?
                {
                    out.entry((section, o)).or_default().push((src, stamp));
                }
            }
            Ok(out)
        },
    )
    .await?;
    let marks = keys
        .iter()
        .map(|k| match (state_section(k.kind), k.server_id) {
            (Some(s), Some(ServerId::Oid(o))) => {
                let rs = rows.get(&(s, o))?;
                if !rs.iter().any(|(src, _)| src == "self") {
                    return None;
                }
                Some(StateMark::Hash(stamp_hash(rs)))
            }
            _ => Some(StateMark::None),
        })
        .collect();
    Ok((marks, report))
}

/// XXH3-64 of the sorted `(src, stamp)` rows.
pub fn stamp_hash(rows: &[(String, String)]) -> u64 {
    let mut v: Vec<&(String, String)> = rows.iter().collect();
    v.sort();
    let mut text = String::new();
    for (s, x) in v {
        text.push_str(s);
        text.push('\u{1f}');
        text.push_str(x);
        text.push('\u{1e}');
    }
    twox_hash::XxHash3_64::oneshot(text.as_bytes())
}

// ## m4-t22-state
// The cheap validation of a cached namespace (HV-INTRO-15; Task 22). Both sides answer one
// `NsCheck` that `freshness::check_namespace` compares by equality:
//
// - **Existence.** The cached side lists every top-level object of an existence kind and every
//   index, key, check, exclusion and trigger of a top-level relation (each with its oid; a
//   domain with its constraint count), and every comment key of the cache (`comment_owners`).
//   The server side reads `ListExistent` and `ListExistentComments` and keeps what the cache
//   holds (by kind and oid, comment key). So a drop (an object or a comment the server lost), a
//   kind change and a dropped domain constraint differ; an object the cache never held (a scope
//   filter's, a level's, a new one) does not: a new object leaves a young row, below.
// - **Change.** The cached side's mark is the watermark's low 32 bits. The server side answers
//   the same when the watermark's plan is incremental and nothing is young since it
//   (`ChangedParents`, `ChangedDependents`), a [`CHANGED`] hash when **any** row is young
//   (never the cached side's value, whatever the snapshot's xmin: fix round 1 C-1, an older
//   open transaction holds the xmin back), and `StateMark::None` (a full pass) when the plan
//   is full or a schema the namespace depends on was renamed (`ChangedDependents`'
//   `schema-rename` branch, which since fix round 1 I-1 also follows referenced collations,
//   operators and operator classes, so every incremental pass catches those too).

/// The server side's mark when a change query saw young rows: a `Hash`, a variant the cached
/// side (`Xmin`) never produces, so the verdict is `Changed` (fix round 1 C-1).
pub const CHANGED: StateMark = StateMark::Hash(0x4356_4847_4544_5f31);

/// The server side's mark (block docs above): `full` the watermark's plan is full; `w` the
/// cached watermark; the change queries' answers. Pure, so the verdict logic is testable
/// without a server.
pub fn validation_mark(
    full: bool,
    w: Option<u64>,
    parents: &[(String, Oid)],
    dependents: &[(String, Oid)],
) -> StateMark {
    let Some(w) = w.filter(|_| !full) else {
        return StateMark::None;
    };
    if dependents.iter().any(|(f, _)| f == "schema-rename") {
        StateMark::None
    } else if !parents.is_empty() || !dependents.is_empty() {
        CHANGED
    } else {
        StateMark::Xmin((w & 0xFFFF_FFFF) as u32)
    }
}

/// An object the existence comparison covers: its kind and oid, and a domain's constraint
/// count.
type ExistLine = (ObjectKind, Oid, Option<usize>);

/// The kinds of `ListExistent`'s top-level rows, and of its rows under a relation.
fn top_kind(k: ObjectKind) -> bool {
    use ObjectKind as K;
    matches!(
        k,
        K::Table
            | K::View
            | K::MaterializedView
            | K::ForeignTable
            | K::Sequence
            | K::Routine
            | K::Procedure
            | K::Aggregate
            | K::EnumType
            | K::Domain
            | K::CompositeType
            | K::RangeType
            | K::Unknown
    )
}

fn child_kind(k: ObjectKind) -> bool {
    use ObjectKind as K;
    matches!(
        k,
        K::Index
            | K::PrimaryKey
            | K::UniqueKey
            | K::ForeignKey
            | K::Check
            | K::Exclusion
            | K::Trigger
    )
}

fn is_relation(k: ObjectKind) -> bool {
    matches!(
        k,
        ObjectKind::Table
            | ObjectKind::View
            | ObjectKind::MaterializedView
            | ObjectKind::ForeignTable
    )
}

/// What the existence comparison covers in a cached namespace.
fn cached_existence(
    snap: &hacivat_meta::NamespaceSnapshot,
) -> (BTreeSet<ExistLine>, BTreeSet<(char, Oid, i32)>) {
    let oid = |o: &hacivat_meta::Obj| match o.server_id() {
        Some(ServerId::Oid(x)) => Some(x),
        _ => None,
    };
    let mut lines = BTreeSet::new();
    for (i, o) in snap.children(0) {
        if !top_kind(o.kind()) {
            continue;
        }
        if let Some(x) = oid(o) {
            let count = (o.kind() == ObjectKind::Domain)
                .then(|| super::build::domain_constraint_count(o.props()))
                .flatten();
            lines.insert((o.kind(), x, count));
        }
        if is_relation(o.kind()) {
            for (_, c) in snap.children(i) {
                if child_kind(c.kind())
                    && let Some(x) = oid(c)
                {
                    lines.insert((c.kind(), x, None));
                }
            }
        }
    }
    let comments = super::pass::comment_owners(snap).into_keys().collect();
    (lines, comments)
}

/// One side's [`NsCheck`] of `lines` and `comments` with `mark`.
fn ns_check_of(
    lines: &BTreeSet<ExistLine>,
    comments: &BTreeSet<(char, Oid, i32)>,
    mark: StateMark,
) -> crate::freshness::NsCheck {
    let mut text = String::new();
    for (k, o, n) in lines {
        text.push_str(&format!(
            "{}:{o}:{}\n",
            k.code(),
            n.map_or(-1, |n| n as i64)
        ));
    }
    for (t, o, s) in comments {
        text.push_str(&format!("comment:{t}:{o}:{s}\n"));
    }
    crate::freshness::NsCheck {
        objects: lines.len() as u64,
        max_mark: mark,
        existence_digest: twox_hash::XxHash3_64::oneshot(text.as_bytes()),
    }
}

/// The cached side (`Introspector::cached_check`): `None` for a namespace with no PostgreSQL
/// watermark (nothing to compare a change with; validation reads it again).
pub fn cached_check(snap: &hacivat_meta::NamespaceSnapshot) -> Option<crate::freshness::NsCheck> {
    let hacivat_meta::Watermark::PgXid { w, .. } = &snap.meta().watermark else {
        return None;
    };
    let (lines, comments) = cached_existence(snap);
    Some(ns_check_of(
        &lines,
        &comments,
        StateMark::Xmin((*w & 0xFFFF_FFFF) as u32),
    ))
}

/// The server side (`Introspector::check_namespace`; block docs above): one snapshot running
/// `Identity`, `ListExistent`, `ListExistentComments`, and when the plan is incremental the two
/// change queries. No object row is read.
pub async fn check_namespace(
    c: &mut MetaConn,
    ns: &hacivat_meta::NsKey,
    snap: &hacivat_meta::NamespaceSnapshot,
    cancel: &CancellationToken,
) -> Result<(crate::freshness::NsCheck, PassReport), IntroError> {
    use super::decode;
    use super::watermark::{Now, Plan, plan};
    let nsx = super::build::Ns::new(ns)
        .ok_or_else(|| IntroError::Protocol(Name::from("pg/namespace-key")))?;
    let (cached, cached_comments) = cached_existence(snap);
    let held: BTreeMap<(ObjectKind, Oid), Option<usize>> =
        cached.iter().map(|(k, o, n)| ((*k, *o), *n)).collect();
    let mut report = PassReport::default();
    let schema = nsx.schema.clone();
    let check = in_snapshot(
        c,
        &schema,
        cancel,
        &mut report,
        PassHooks::default(),
        async |r: &mut Reader<'_, '_>, id: &IdentityRow| {
            let oid = super::pass::ns_oid(&nsx, id)?;
            let b = Binds::ns(oid);
            let rows = r
                .read(
                    name::LIST_EXISTENT,
                    Mode::Full,
                    Level::Structure,
                    &b,
                    decode::existent,
                )
                .await?;
            let comments: BTreeSet<(char, Oid, i32)> = r
                .read(
                    name::LIST_EXISTENT_COMMENTS,
                    Mode::Full,
                    Level::Structure,
                    &b,
                    decode::comment_key,
                )
                .await?
                .into_iter()
                .filter(|k| cached_comments.contains(k))
                .collect();
            let mut lines = BTreeSet::new();
            for e in &rows {
                let Some(kind) = super::build::existent_kind(&e.tag, e.sub.as_deref()) else {
                    continue;
                };
                let Some(held_count) = held.get(&(kind, e.oid)) else {
                    continue;
                };
                // A domain the cache holds without its constraints (names only) compares by
                // existence alone.
                let count = (kind == ObjectKind::Domain && held_count.is_some())
                    .then(|| {
                        e.sub
                            .as_deref()
                            .and_then(|s| s.split_once(':'))
                            .and_then(|(_, n)| n.parse::<usize>().ok())
                    })
                    .flatten();
                lines.insert((kind, e.oid, count));
            }
            let now = Now::of(id);
            let mark = match (&snap.meta().watermark, plan(&snap.meta().watermark, &now)) {
                (_, Plan::Full(_)) => StateMark::None,
                (hacivat_meta::Watermark::PgXid { w, .. }, Plan::Incremental { since }) => {
                    let bw = Binds::ns(oid).with_w(since);
                    let parents = r
                        .read(
                            name::CHANGED_PARENTS,
                            Mode::Full,
                            Level::Structure,
                            &bw,
                            decode::facet,
                        )
                        .await?;
                    let dependents = r
                        .read(
                            name::CHANGED_DEPENDENTS,
                            Mode::Full,
                            Level::Structure,
                            &bw,
                            decode::facet,
                        )
                        .await?;
                    validation_mark(false, Some(*w), &parents, &dependents)
                }
                (_, Plan::Incremental { .. }) => StateMark::None,
            };
            Ok(ns_check_of(&lines, &comments, mark))
        },
    )
    .await?;
    Ok((check, report))
}
// ## m4-t22-state

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_hash_is_order_free_and_moves_with_any_row() {
        let a = vec![
            ("self".to_string(), "10".to_string()),
            ("desc".into(), "0:11".into()),
        ];
        let mut b = a.clone();
        b.reverse();
        assert_eq!(stamp_hash(&a), stamp_hash(&b));
        assert_ne!(stamp_hash(&a), stamp_hash(&a[..1]));
    }

    // ## m4-t22-state
    fn arena(with_index: bool, w: Option<u64>) -> hacivat_meta::NamespaceSnapshot {
        use hacivat_meta::{NsKey, Obj, Subtree, Watermark};
        let mut t = Subtree::new(Obj::new(ObjectKind::Schema, Name::from("s")));
        let root = t.root();
        let tab = t.add(
            root,
            Obj::new(ObjectKind::Table, Name::from("t")).with_server_id(ServerId::Oid(10)),
        );
        t.add(
            tab,
            Obj::new(ObjectKind::Column, Name::from("a")).with_props(
                hacivat_meta::props::Props::Column(std::sync::Arc::new(
                    hacivat_meta::props::ColumnProps {
                        ordinal: 1,
                        ..Default::default()
                    },
                )),
            ),
        );
        if with_index {
            t.add(
                tab,
                Obj::new(ObjectKind::Index, Name::from("ti")).with_server_id(ServerId::Oid(11)),
            );
        }
        t.add(
            root,
            Obj::new(ObjectKind::Sequence, Name::from("q")).with_server_id(ServerId::Oid(12)),
        );
        hacivat_meta::NamespaceSnapshot::build(
            NsKey::schema(["db", "s"]),
            hacivat_proto::DbmsKind::Postgres,
            hacivat_meta::name::FoldRule::Exact,
            hacivat_meta::namespace::NsMeta {
                level: Level::Structure,
                watermark: w.map_or(Watermark::None, |w| Watermark::PgXid {
                    w,
                    identity: Default::default(),
                }),
                loaded_at_ms: 1,
            },
            t,
        )
        .unwrap()
    }

    #[test]
    fn the_cached_side_needs_a_watermark_and_moves_with_a_drop() {
        assert_eq!(cached_check(&arena(true, None)), None);
        let w = (7u64 << 32) | 5;
        let with = cached_check(&arena(true, Some(w))).unwrap();
        // The table, its index and the sequence; a column has no row of its own there.
        assert_eq!(with.objects, 3);
        // The watermark's low 32 bits: an xid is compared by equality, never ordered.
        assert_eq!(with.max_mark, StateMark::Xmin(5));
        let without = cached_check(&arena(false, Some(w))).unwrap();
        assert_eq!(without.objects, 2);
        assert_ne!(with.existence_digest, without.existence_digest);
        assert_eq!(cached_check(&arena(true, Some(w))), Some(with));
    }

    /// Fix round 1 C-1: whatever the snapshot's xmin, a young row is never `Current`; a schema
    /// rename or a full plan asks for a full pass; nothing young is the cached mark.
    #[test]
    fn a_young_row_never_validates_current() {
        let w = (3u64 << 32) | 77;
        let cached = StateMark::Xmin(77);
        let row = |f: &str| vec![(f.to_string(), 1u32)];
        assert_eq!(validation_mark(false, Some(w), &[], &[]), cached);
        for (p, d) in [
            (row("columns"), vec![]),
            (vec![], row("rel")),
            (row("rel"), row("type")),
        ] {
            let m = validation_mark(false, Some(w), &p, &d);
            assert_ne!(m, cached, "{p:?} {d:?}");
            assert_ne!(m, StateMark::None, "an incremental pass suffices");
        }
        assert_eq!(
            validation_mark(false, Some(w), &row("columns"), &row("schema-rename")),
            StateMark::None
        );
        assert_eq!(validation_mark(true, Some(w), &[], &[]), StateMark::None);
        assert_eq!(validation_mark(false, None, &[], &[]), StateMark::None);
        // The verdict: never Current with a young row, whatever the held-back xmin.
        let c = |m| crate::freshness::NsCheck {
            objects: 1,
            max_mark: m,
            existence_digest: 9,
        };
        assert_eq!(
            crate::freshness::verdict(
                &c(cached),
                &c(validation_mark(false, Some(w), &row("columns"), &[]))
            ),
            crate::freshness::NsVerdict::Changed { full: false }
        );
    }
    // ## m4-t22-state
}
