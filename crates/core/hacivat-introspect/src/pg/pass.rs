// ## m4-t16-pg (spike origin 5a5ab08, `src/pg/introspect.rs`)
//! The passes (HV-INTRO-05): [`introspect`] reads one namespace in full, incrementally since
//! the cached watermark, at `Names` level (`Brief`), or the system schemas (`System`);
//! [`fragment`] re-reads a few objects; [`stamps`] reads state marks; [`list_namespaces`] and
//! [`check_namespace`] are the cheap reads.
//!
//! Every pass is one `SnapshotTxn` of Task 5's `MetaConn` (`REPEATABLE READ READ ONLY`, the
//! pinned `search_path`, R-M4-46): `Identity` first, then the sections, then the `Canary` (no
//! XID assigned, else the pass fails), then `COMMIT`. A failed section rolls the snapshot back
//! and publishes nothing.
//!
//! **Incremental** (R-M4-8, R-M4-48): `ListExistent` decides deletions (namespace-wide sets);
//! `ChangedParents` and `ChangedDependents` (young rows of the namespace, and one dependency
//! hop from young rows of the whole database) and `ListExistentComments` (removed comments,
//! finding 3) decide the re-reads; the same-relation cascade widens them (finding 5); then only
//! the changed facets are read, by oid. A relation's object is always re-emitted with the
//! children read (its cached properties when its own row is not young), so a re-read child
//! replaces its twin and an unread child stays. Full instead: no watermark, identity,
//! restart, version, standby, wraparound ([`super::watermark::plan`]), a level above the
//! cached one, a schema rename, or a mass change ([`super::watermark::is_mass_change`]).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use futures::future::BoxFuture;
use hacivat_meta::merge::{ChangedObject, ExistSet, Identity as ExistId, NamespaceResult};
use hacivat_meta::props::{Props, StatsFacet};
use hacivat_meta::{
    Level, Name, NamespaceSnapshot, NsKey, Obj, ObjectKey, ObjectKind, ServerId, StateMark,
    Watermark,
};
use hacivat_session::meta::MetaConn;
use tokio_util::sync::CancellationToken;

use super::build::{self, CHILD_KINDS, Ns, RelRead, TOP_KINDS};
use super::changes::Changes;
use super::decode::{self, Existent, IdentityRow};
use super::model::*;
use super::sections::{Binds, QueryLog, Reader, error, name, pack};
use super::watermark::{FullReason, Now, Plan, is_mass_change, plan};
use crate::filter::CompiledFilter;
use crate::introspector::IntroError;
use crate::pack::Mode;

/// A test-support pause point (the Docker test of finding 6 runs DDL and burns XIDs in another
/// session while the pass waits after `Identity`). Product code passes none.
pub type Hook = Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>;

/// How a pass reads.
#[derive(Clone, Default)]
pub struct PassOpts<'a> {
    pub mode: Mode,
    pub level: Level,
    pub prev: Option<&'a NamespaceSnapshot>,
    /// The scope filter (HV-INTRO-03): objects it excludes are not read. Applied on the client
    /// for now; server pushdown arrives with the trait's filter parameter (ruling R-W3-2).
    pub filter: Option<&'a CompiledFilter>,
    /// Runs after `Identity`, inside the snapshot.
    pub after_identity: Option<Hook>,
}

/// What a pass did, for the Docker tests and the timings.
#[derive(Debug, Clone, Default)]
pub struct PassReport {
    pub log: QueryLog,
    /// `Some` when the pass read the namespace whole.
    pub full_reason: Option<FullReason>,
    pub version: u32,
    pub snap_xmin: u64,
    pub snap_xmax: u64,
    /// The server facts `Identity` read (`ServerFacts`: version, encoding,
    /// `standard_conforming_strings`, recovery, the default collation).
    pub facts: Option<hacivat_meta::snapshot::ServerFacts>,
}

/// One pass's result, its report, and the row estimates read with the relations
/// (`StatsFacet`, R-M4-30; `reltuples` `-1` is `None`).
#[derive(Debug, Clone)]
pub struct PgPass {
    pub result: NamespaceResult,
    pub report: PassReport,
    pub stats: Vec<(ObjectKey, StatsFacet)>,
}

/// The scope filter's verdict on a top-level object (no filter keeps everything).
fn keep(filter: Option<&CompiledFilter>, kind: ObjectKind, name: &str) -> bool {
    filter.is_none_or(|f| f.matches(kind, &Name::from(name)))
}

pub(super) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Runs `body` inside one snapshot of `c`: `Identity` first (with `schema` as its one bind),
/// `body`, the canary, `COMMIT`.
pub(super) async fn in_snapshot<T>(
    c: &mut MetaConn,
    schema: &Name,
    cancel: &CancellationToken,
    report: &mut PassReport,
    after_identity: Option<Hook>,
    body: impl AsyncFnOnce(&mut Reader<'_, '_>, &IdentityRow) -> Result<T, IntroError>,
) -> Result<T, IntroError> {
    let pack = pack()?;
    let mut txn = c
        .begin_snapshot()
        .await
        .map_err(|e| error(e, name::IDENTITY))?;
    let out = {
        let mut r = Reader {
            txn: &mut txn,
            pack,
            version: 0,
            cancel,
            log: &mut report.log,
        };
        let binds = Binds {
            schema: Some(schema.clone()),
            ..Binds::default()
        };
        let id = r
            .read(
                name::IDENTITY,
                Mode::Full,
                Level::Structure,
                &binds,
                decode::identity,
            )
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| IntroError::Protocol(Name::from("pg/Identity: no row")))?;
        r.version = id.version;
        report.version = id.version;
        report.snap_xmin = id.snap_xmin;
        report.snap_xmax = id.snap_xmax;
        report.facts = Some(id.server_facts());
        if let Some(h) = after_identity {
            h().await;
        }
        let v = body(&mut r, &id).await?;
        let canary = r
            .read(
                name::CANARY,
                Mode::Full,
                Level::Structure,
                &Binds::default(),
                |row| row.bool(0),
            )
            .await?;
        if canary.first() != Some(&true) {
            return Err(IntroError::Protocol(Name::from(
                "pg/Canary: the pass's transaction has an XID",
            )));
        }
        v
    };
    txn.commit().await.map_err(|e| error(e, name::CANARY))?;
    Ok(out)
}

/// The schema oid of a probe for namespace `ns`, or why there is none.
fn ns_oid(ns: &Ns, id: &IdentityRow) -> Result<Oid, IntroError> {
    if *id.db_name != *ns.db {
        return Err(IntroError::Protocol(Name::from("pg/wrong-database")));
    }
    id.ns_oid
        .ok_or_else(|| IntroError::Protocol(Name::from("pg/namespace-gone")))
}

/// One pass over `ns` (module docs).
pub async fn introspect(
    c: &mut MetaConn,
    ns: &NsKey,
    opts: PassOpts<'_>,
    cancel: &CancellationToken,
) -> Result<PgPass, IntroError> {
    let nsx = Ns::new(ns).ok_or_else(|| IntroError::Protocol(Name::from("pg/namespace-key")))?;
    let mut report = PassReport::default();
    let schema = nsx.schema.clone();
    let hook = opts.after_identity.clone();
    let (result, stats, reason) = in_snapshot(
        c,
        &schema,
        cancel,
        &mut report,
        hook,
        async |r: &mut Reader<'_, '_>, id: &IdentityRow| {
            let oid = ns_oid(&nsx, id)?;
            let now = Now::of(id);
            match opts.mode {
                Mode::System => {
                    let (res, stats) = super::system::read(r, &nsx, oid, &now).await?;
                    return Ok((res, stats, Some(FullReason::Requested)));
                }
                Mode::Brief => {
                    let (res, stats) =
                        read_full(r, &nsx, oid, Level::Names, opts.filter, &now).await?;
                    return Ok((res, stats, Some(FullReason::Requested)));
                }
                Mode::Full => {
                    let (res, stats) =
                        read_full(r, &nsx, oid, opts.level, opts.filter, &now).await?;
                    return Ok((res, stats, Some(FullReason::Requested)));
                }
                Mode::Incremental => {}
            }
            let decision = match opts.prev {
                None => Plan::Full(FullReason::NoWatermark),
                Some(prev) if opts.level > prev.meta().level => Plan::Full(FullReason::Level),
                Some(prev) => plan(&prev.meta().watermark, &now),
            };
            let reason = match (decision, opts.prev) {
                (Plan::Incremental { since }, Some(prev)) => {
                    let level = opts.level.max(prev.meta().level);
                    match read_incremental(r, &nsx, oid, prev, since, level, opts.filter, &now)
                        .await?
                    {
                        Ok((res, stats)) => return Ok((res, stats, None)),
                        Err(reason) => reason,
                    }
                }
                (Plan::Full(reason), _) => reason,
                (Plan::Incremental { .. }, None) => FullReason::NoWatermark,
            };
            let level = opts
                .prev
                .map_or(opts.level, |p| opts.level.max(p.meta().level));
            let (res, stats) = read_full(r, &nsx, oid, level, opts.filter, &now).await?;
            Ok((res, stats, Some(reason)))
        },
    )
    .await?;
    report.full_reason = reason;
    Ok(PgPass {
        result,
        report,
        stats,
    })
}

/// Groups `(oid, T)` rows by oid.
fn group<T>(rows: Vec<(Oid, T)>) -> BTreeMap<Oid, Vec<T>> {
    let mut m: BTreeMap<Oid, Vec<T>> = BTreeMap::new();
    for (k, v) in rows {
        m.entry(k).or_default().push(v);
    }
    m
}

/// The relations' children read at `level` for `binds` (namespace or oid list).
struct RelChildrenRead {
    columns: BTreeMap<Oid, Vec<PgColumn>>,
    indexes: BTreeMap<Oid, Vec<PgIndex>>,
    constraints: BTreeMap<Oid, Vec<PgConstraint>>,
    triggers: BTreeMap<Oid, Vec<PgTrigger>>,
}

/// The types of `binds` with their attributes and domain constraints.
async fn read_types(
    r: &mut Reader<'_, '_>,
    mode: Mode,
    level: Level,
    binds: &Binds,
) -> Result<Vec<PgType>, IntroError> {
    let mut types = r
        .read(name::RETRIEVE_TYPES, mode, level, binds, decode::user_type)
        .await?;
    if level == Level::Names || types.is_empty() {
        return Ok(types);
    }
    let mut attrs = group(
        r.read(
            name::RETRIEVE_TYPE_ATTRIBUTES,
            mode,
            level,
            binds,
            decode::type_attribute,
        )
        .await?,
    );
    let mut dcons = group(
        r.read(
            name::RETRIEVE_DOMAIN_CONSTRAINTS,
            mode,
            level,
            binds,
            decode::domain_constraint,
        )
        .await?,
    );
    for t in &mut types {
        match &mut t.kind {
            UserTypeKind::Domain { constraints, .. } => {
                *constraints = dcons.remove(&t.oid).unwrap_or_default();
            }
            UserTypeKind::Composite { attrs: a, .. } => {
                *a = attrs.remove(&t.oid).unwrap_or_default();
            }
            _ => {}
        }
    }
    Ok(types)
}

/// The row estimate of a relation, for the stats list.
fn stat_of(nsx: &Ns, r: &PgRelation, at: SystemTime) -> (ObjectKey, StatsFacet) {
    (
        nsx.key_of(build::rel_kind(r.kind), &r.name, r.oid),
        StatsFacet {
            row_estimate: r.row_estimate,
            at,
        },
    )
}

type Read = (NamespaceResult, Vec<(ObjectKey, StatsFacet)>);

/// A full pass at `level` (all objects; children from `Structure`).
async fn read_full(
    r: &mut Reader<'_, '_>,
    nsx: &Ns,
    ns: Oid,
    level: Level,
    filter: Option<&CompiledFilter>,
    now: &Now,
) -> Result<Read, IntroError> {
    let b = Binds::ns(ns);
    let m = Mode::Full;
    let schema = r
        .read(name::RETRIEVE_NAMESPACE, m, level, &b, decode::schema)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| IntroError::Protocol(Name::from("pg/namespace-gone")))?;
    let mut rels = r
        .read(name::RETRIEVE_RELATIONS, m, level, &b, decode::relation)
        .await?;
    rels.retain(|x| keep(filter, build::rel_kind(x.kind), &x.name));
    let structure = level >= Level::Structure;
    let ch = if structure {
        RelChildrenRead {
            columns: group(
                r.read(name::RETRIEVE_COLUMNS, m, level, &b, decode::column)
                    .await?,
            ),
            indexes: group(
                r.read(name::RETRIEVE_INDEXES, m, level, &b, decode::index)
                    .await?,
            ),
            constraints: group(
                r.read(name::RETRIEVE_CONSTRAINTS, m, level, &b, decode::constraint)
                    .await?,
            ),
            triggers: group(
                r.read(name::RETRIEVE_TRIGGERS, m, level, &b, decode::trigger)
                    .await?,
            ),
        }
    } else {
        RelChildrenRead {
            columns: BTreeMap::new(),
            indexes: BTreeMap::new(),
            constraints: BTreeMap::new(),
            triggers: BTreeMap::new(),
        }
    };
    let mut seqs = r
        .read(name::RETRIEVE_SEQUENCES, m, level, &b, decode::sequence)
        .await?;
    seqs.retain(|s| keep(filter, ObjectKind::Sequence, &s.name));
    let mut routines = r
        .read(name::RETRIEVE_ROUTINES, m, level, &b, decode::routine)
        .await?;
    routines.retain(|p| keep(filter, build::routine_kind(p.kind), &p.name));
    let mut types = read_types(r, m, level, &b).await?;
    types.retain(|t| keep(filter, build::type_kind(&t.kind), &t.name));
    let mut ch = ch;
    let reads: Vec<RelRead> = rels
        .iter()
        .map(|x| RelRead {
            oid: x.oid,
            name: x.name.clone(),
            own: Some(build::relation_obj(nsx, x)),
            columns: structure.then(|| ch.columns.remove(&x.oid).unwrap_or_default()),
            keys: structure.then(|| {
                (
                    ch.indexes.remove(&x.oid).unwrap_or_default(),
                    ch.constraints.remove(&x.oid).unwrap_or_default(),
                )
            }),
            triggers: structure.then(|| ch.triggers.remove(&x.oid).unwrap_or_default()),
        })
        .collect();
    let kids = build::children(nsx, &reads, &|_| None, true);
    let root = nsx.root_key();
    let mut changed = vec![ChangedObject {
        parent: root.clone(),
        subtree: build::subtree(build::schema_obj(&schema), []),
    }];
    let mut exist: BTreeMap<ObjectKind, Vec<ExistId>> = BTreeMap::new();
    let mut child_exist: BTreeMap<ObjectKind, Vec<ExistId>> = BTreeMap::new();
    let mut emit = |o: Obj, children: Vec<Obj>, changed: &mut Vec<ChangedObject>| {
        exist
            .entry(o.kind())
            .or_default()
            .push(build::top_identity(&o));
        for k in &children {
            child_exist
                .entry(k.kind())
                .or_default()
                .push(build::child_identity(o.name(), k));
        }
        changed.push(ChangedObject {
            parent: root.clone(),
            subtree: build::subtree(o, children),
        });
    };
    let mut kids = kids;
    for x in reads {
        let mut children = kids.columns.remove(&x.oid).unwrap_or_default();
        children.extend(kids.keys.remove(&x.oid).unwrap_or_default());
        children.extend(kids.triggers.remove(&x.oid).unwrap_or_default());
        if let Some(own) = x.own {
            emit(own, children, &mut changed);
        }
    }
    for s in &seqs {
        emit(build::sequence_obj(nsx, s), Vec::new(), &mut changed);
    }
    for p in &routines {
        emit(build::routine_obj(p), Vec::new(), &mut changed);
    }
    for t in &types {
        emit(build::type_obj(t), Vec::new(), &mut changed);
    }
    let mut existent = BTreeMap::new();
    for k in TOP_KINDS {
        existent.insert(k, ExistSet::Ok(exist.remove(&k).unwrap_or_default()));
    }
    if structure {
        for k in CHILD_KINDS {
            existent.insert(k, ExistSet::Ok(child_exist.remove(&k).unwrap_or_default()));
        }
    }
    let at = SystemTime::now();
    let stats = if structure {
        rels.iter().map(|x| stat_of(nsx, x, at)).collect()
    } else {
        Vec::new()
    };
    Ok((
        NamespaceResult {
            existent,
            changed,
            categories_to_reload: BTreeSet::new(),
            new_watermark: now.watermark(),
            full: true,
            level,
            read_at_ms: now_ms(),
        },
        stats,
    ))
}

/// Where a comment of the cached namespace lives (for `ListExistentComments`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommentOwner {
    /// A relation's own or column comment.
    RelComments(Oid),
    /// An index comment: its relation's keys.
    RelKeys(Oid),
    Sequence(Oid),
    Routine(Oid),
    Type(Oid),
    Schema,
}

fn oid_of(o: &Obj) -> Option<Oid> {
    match o.server_id() {
        Some(ServerId::Oid(x)) => Some(x),
        _ => None,
    }
}

/// Every comment of the cached namespace by its `pg_description` key.
fn comment_owners(prev: &NamespaceSnapshot) -> BTreeMap<(char, Oid, i32), CommentOwner> {
    let mut m = BTreeMap::new();
    for (i, o) in prev.objs().iter().enumerate() {
        if o.props().comment().is_none() {
            continue;
        }
        let parent_oid = o.parent().and_then(|p| prev.obj(p)).and_then(oid_of);
        let own = oid_of(o);
        use ObjectKind as K;
        let entry = match o.kind() {
            K::Schema if i == 0 => own.map(|x| (('n', x, 0), CommentOwner::Schema)),
            K::Table | K::View | K::MaterializedView | K::ForeignTable => {
                own.map(|x| (('c', x, 0), CommentOwner::RelComments(x)))
            }
            K::Column => match (parent_oid, o.props()) {
                (Some(t), Props::Column(c)) => Some((
                    ('c', t, i32::try_from(c.ordinal).unwrap_or(0)),
                    CommentOwner::RelComments(t),
                )),
                _ => None,
            },
            K::Index => match (own, parent_oid) {
                (Some(x), Some(t)) => Some((('c', x, 0), CommentOwner::RelKeys(t))),
                _ => None,
            },
            K::Sequence => own.map(|x| (('c', x, 0), CommentOwner::Sequence(x))),
            K::Routine | K::Procedure | K::Aggregate => {
                own.map(|x| (('p', x, 0), CommentOwner::Routine(x)))
            }
            K::Domain | K::EnumType | K::CompositeType | K::RangeType | K::Unknown => {
                own.map(|x| (('y', x, 0), CommentOwner::Type(x)))
            }
            _ => None,
        };
        if let Some((k, v)) = entry {
            m.insert(k, v);
        }
    }
    m
}

/// A fresh copy of a cached object (same kind, name, id, flags, state and props).
fn copy_obj(o: &Obj) -> Obj {
    let mut c = Obj::new(o.kind(), o.name().clone())
        .add_flags(o.flags())
        .with_state(o.state())
        .with_props(o.props().clone());
    if let Some(s) = o.server_id() {
        c = c.with_server_id(s);
    }
    c
}

/// `o` with its comment replaced (relations and columns).
fn with_comment(o: Obj, comment: Option<&str>) -> Obj {
    let c = comment.map(Name::from);
    let props = match o.props() {
        Props::Table(t) => {
            let mut t = (**t).clone();
            t.comment = c;
            Props::Table(Arc::new(t))
        }
        Props::View(v) => {
            let mut v = (**v).clone();
            v.comment = c;
            Props::View(Arc::new(v))
        }
        Props::Column(k) => {
            let mut k = (**k).clone();
            k.comment = c;
            Props::Column(Arc::new(k))
        }
        other => other.clone(),
    };
    o.with_props(props)
}

/// The kind and name of every existing object by oid, and the namespace-wide existence sets.
struct Alive {
    by_oid: HashMap<Oid, Existent>,
    relations: BTreeSet<Oid>,
    sets: BTreeMap<ObjectKind, Vec<ExistId>>,
    relation_count: usize,
}

fn alive_of(rows: Vec<Existent>, filter: Option<&CompiledFilter>) -> Alive {
    let mut a = Alive {
        by_oid: HashMap::new(),
        relations: BTreeSet::new(),
        sets: BTreeMap::new(),
        relation_count: 0,
    };
    // Relations the filter keeps, by name, so their children are kept too.
    let mut kept_rel: BTreeSet<String> = BTreeSet::new();
    for e in &rows {
        if e.tag == "rel"
            && let Some(k) = build::existent_kind(&e.tag, e.sub.as_deref())
            && keep(filter, k, &e.name)
        {
            kept_rel.insert(e.name.clone());
        }
    }
    for e in rows {
        let Some(kind) = build::existent_kind(&e.tag, e.sub.as_deref()) else {
            continue;
        };
        let id = match &e.parent {
            Some(p) => {
                if !kept_rel.contains(p) {
                    continue;
                }
                ExistId::of(
                    vec![Name::from(p.as_str()), Name::from(e.name.as_str())],
                    Some(ServerId::Oid(e.oid)),
                )
            }
            None => {
                if !keep(filter, kind, &e.name) {
                    continue;
                }
                let mut id = ExistId::of(
                    vec![Name::from(e.name.as_str())],
                    Some(ServerId::Oid(e.oid)),
                );
                if kind == ObjectKind::Unknown {
                    id.raw_kind = Some(Name::from(build::BASE_TYPE_RAW_KIND));
                }
                id
            }
        };
        if e.tag == "rel" {
            a.relations.insert(e.oid);
            a.relation_count += 1;
        }
        a.sets.entry(kind).or_default().push(id);
        a.by_oid.insert(e.oid, e);
    }
    a
}

/// An incremental pass; `Err(reason)` asks the caller for a full pass in the same snapshot.
#[allow(clippy::too_many_arguments, reason = "one pass's inputs")]
async fn read_incremental(
    r: &mut Reader<'_, '_>,
    nsx: &Ns,
    ns: Oid,
    prev: &NamespaceSnapshot,
    since: u32,
    level: Level,
    filter: Option<&CompiledFilter>,
    now: &Now,
) -> Result<Result<Read, FullReason>, IntroError> {
    let b = Binds::ns(ns);
    let full = Mode::Full;
    // 4a: existence.
    let alive = alive_of(
        r.read(name::LIST_EXISTENT, full, level, &b, decode::existent)
            .await?,
        filter,
    );
    // 4b: young rows of the namespace, then one dependency hop from the database's.
    let bw = Binds::ns(ns).with_w(since);
    let mut changes = Changes::default();
    for (f, oid) in r
        .read(name::CHANGED_PARENTS, full, level, &bw, decode::facet)
        .await?
    {
        changes.add(&f, oid)?;
    }
    for (f, oid) in r
        .read(name::CHANGED_DEPENDENTS, full, level, &bw, decode::facet)
        .await?
    {
        changes.add(&f, oid)?;
    }
    if changes.schema_rename {
        return Ok(Err(FullReason::SchemaRename));
    }
    // Removed comments leave no young row (finding 3).
    let now_keys: BTreeSet<(char, Oid, i32)> = r
        .read(
            name::LIST_EXISTENT_COMMENTS,
            full,
            level,
            &b,
            decode::comment_key,
        )
        .await?
        .into_iter()
        .collect();
    for (key, owner) in comment_owners(prev) {
        if now_keys.contains(&key) {
            continue;
        }
        let _ = match owner {
            CommentOwner::RelComments(t) => changes.add("comments", t),
            CommentOwner::RelKeys(t) => changes.add("indexes", t),
            CommentOwner::Sequence(o) => changes.add("sequence", o),
            CommentOwner::Routine(o) => changes.add("routine", o),
            CommentOwner::Type(o) => changes.add("type", o),
            CommentOwner::Schema => changes.add("schema", ns),
        };
    }
    // Objects the cache lacks are read whole; a domain whose constraint count moved (a
    // dropped constraint leaves no young row) is re-read.
    let prev_index = |e: &Existent| -> Option<u32> {
        let kind = build::existent_kind(&e.tag, e.sub.as_deref())?;
        prev.find(&nsx.key_of(kind, &e.name, e.oid))
    };
    for e in alive.by_oid.values() {
        if e.parent.is_some() {
            continue;
        }
        let cached = prev_index(e);
        match (e.tag.as_str(), cached) {
            ("rel", None) => changes.add("rel", e.oid)?,
            ("seq", None) => changes.add("sequence", e.oid)?,
            ("fn", None) => changes.add("routine", e.oid)?,
            ("typ", None) => changes.add("type", e.oid)?,
            ("typ", Some(i)) => {
                let count = e
                    .sub
                    .as_deref()
                    .and_then(|s| s.split_once(':'))
                    .and_then(|(_, n)| n.parse::<usize>().ok());
                let cached = prev
                    .obj(i)
                    .and_then(|o| build::domain_constraint_count(o.props()));
                if e.sub.as_deref().is_some_and(|s| s.starts_with('d')) && count != cached {
                    changes.add("type", e.oid)?;
                }
            }
            _ => {}
        }
    }
    changes.settle(&alive.relations);
    changes.sequences.retain(|o| alive.by_oid.contains_key(o));
    changes.routines.retain(|o| alive.by_oid.contains_key(o));
    changes.types.retain(|o| alive.by_oid.contains_key(o));
    let changed_rels = changes.relations.values().filter(|f| f.any()).count();
    if is_mass_change(changed_rels, alive.relation_count) {
        return Ok(Err(FullReason::MassChange));
    }
    // 4c: re-read the changed facets, by oid.
    let inc = Mode::Incremental;
    let list = |oids: Vec<Oid>| Binds::ns(ns).with_oids(oids);
    let l_rel = changes.relations_with(|f| f.rel);
    let structure = level >= Level::Structure;
    let l_col = if structure {
        changes.relations_with(|f| f.columns)
    } else {
        Vec::new()
    };
    let l_keys = if structure {
        changes.relations_with(|f| f.keys())
    } else {
        Vec::new()
    };
    let l_trg = if structure {
        changes.relations_with(|f| f.triggers)
    } else {
        Vec::new()
    };
    let l_cmt = changes.relations_with(|f| f.comments && !f.columns);
    let mut own: BTreeMap<Oid, PgRelation> = r
        .read(
            name::RETRIEVE_RELATIONS,
            inc,
            level,
            &list(l_rel),
            decode::relation,
        )
        .await?
        .into_iter()
        .map(|x| (x.oid, x))
        .collect();
    let mut cols = group(
        r.read(
            name::RETRIEVE_COLUMNS,
            inc,
            level,
            &list(l_col.clone()),
            decode::column,
        )
        .await?,
    );
    let (mut idx, mut cons) = if l_keys.is_empty() {
        (BTreeMap::new(), BTreeMap::new())
    } else {
        (
            group(
                r.read(
                    name::RETRIEVE_INDEXES,
                    inc,
                    level,
                    &list(l_keys.clone()),
                    decode::index,
                )
                .await?,
            ),
            group(
                r.read(
                    name::RETRIEVE_CONSTRAINTS,
                    inc,
                    level,
                    &list(l_keys.clone()),
                    decode::constraint,
                )
                .await?,
            ),
        )
    };
    let mut trgs = group(
        r.read(
            name::RETRIEVE_TRIGGERS,
            inc,
            level,
            &list(l_trg.clone()),
            decode::trigger,
        )
        .await?,
    );
    let mut comments: BTreeMap<Oid, Vec<(i32, String)>> = BTreeMap::new();
    for (o, sub, text) in r
        .read(
            name::COMMENTS,
            inc,
            level,
            &list(l_cmt.clone()),
            decode::comment,
        )
        .await?
    {
        comments.entry(o).or_default().push((sub, text));
    }
    let cached_obj = |oid: Oid| -> Option<u32> {
        let e = alive.by_oid.get(&oid)?;
        prev_index(e)
    };
    let cached_columns = |oid: Oid| -> Option<Vec<(u32, Name)>> {
        let i = cached_obj(oid)?;
        Some(
            prev.children_of_kind(i, ObjectKind::Column)
                .filter_map(|(_, c)| c.props().as_column().map(|p| (p.ordinal, c.name().clone())))
                .collect(),
        )
    };
    let mut reads = Vec::new();
    let mut stats = Vec::new();
    let at = SystemTime::now();
    for (&oid, f) in &changes.relations {
        if !f.any() {
            continue;
        }
        let Some(e) = alive.by_oid.get(&oid) else {
            continue;
        };
        let own_row = own.remove(&oid);
        if let Some(x) = &own_row
            && structure
        {
            stats.push(stat_of(nsx, x, at));
        }
        let own_obj = match &own_row {
            Some(x) => Some(build::relation_obj(nsx, x)),
            None => cached_obj(oid).and_then(|i| prev.obj(i)).map(copy_obj),
        };
        let Some(mut own_obj) = own_obj else {
            continue;
        };
        if f.comments && !f.rel {
            let text = comments
                .get(&oid)
                .and_then(|v| v.iter().find(|(s, _)| *s == 0))
                .map(|(_, t)| t.as_str());
            own_obj = with_comment(own_obj, text);
        }
        reads.push(RelRead {
            oid,
            name: e.name.clone(),
            own: Some(own_obj),
            columns: (structure && f.columns).then(|| cols.remove(&oid).unwrap_or_default()),
            keys: (structure && f.keys()).then(|| {
                (
                    idx.remove(&oid).unwrap_or_default(),
                    cons.remove(&oid).unwrap_or_default(),
                )
            }),
            triggers: (structure && f.triggers).then(|| trgs.remove(&oid).unwrap_or_default()),
        });
    }
    let mut kids = build::children(nsx, &reads, &cached_columns, false);
    let root = nsx.root_key();
    let mut changed = Vec::new();
    let mut column_scope: Vec<Vec<Name>> = Vec::new();
    let mut column_ids: Vec<ExistId> = Vec::new();
    for x in reads {
        let Some(own_obj) = x.own else {
            continue;
        };
        let f = changes.relations.get(&x.oid).copied().unwrap_or_default();
        let mut children = Vec::new();
        if let Some(cs) = kids.columns.remove(&x.oid) {
            column_scope.push(vec![Name::from(x.name.as_str())]);
            column_ids.extend(cs.iter().map(|c| build::child_identity(&x.name, c)));
            children.extend(cs);
        } else if f.comments
            && structure
            && let Some(i) = cached_obj(x.oid)
        {
            // Column comments without a column re-read: patch the cached columns.
            let rows = comments.get(&x.oid);
            for (_, c) in prev.children_of_kind(i, ObjectKind::Column) {
                let ordinal = c.props().as_column().map_or(0, |p| p.ordinal);
                let text = rows.and_then(|v| {
                    v.iter()
                        .find(|(s, _)| u32::try_from(*s).ok() == Some(ordinal))
                        .map(|(_, t)| t.as_str())
                });
                if text.map(Name::from) != c.props().comment().cloned() {
                    children.push(with_comment(copy_obj(c), text));
                }
            }
        }
        children.extend(kids.keys.remove(&x.oid).unwrap_or_default());
        children.extend(kids.triggers.remove(&x.oid).unwrap_or_default());
        changed.push(ChangedObject {
            parent: root.clone(),
            subtree: build::subtree(own_obj, children),
        });
    }
    // Sequences, routines, types and the schema, re-read whole.
    let l_seq: Vec<Oid> = changes.sequences.iter().copied().collect();
    for s in r
        .read(
            name::RETRIEVE_SEQUENCES,
            inc,
            level,
            &list(l_seq),
            decode::sequence,
        )
        .await?
    {
        changed.push(ChangedObject {
            parent: root.clone(),
            subtree: build::subtree(build::sequence_obj(nsx, &s), []),
        });
    }
    let l_rtn: Vec<Oid> = changes.routines.iter().copied().collect();
    for p in r
        .read(
            name::RETRIEVE_ROUTINES,
            inc,
            level,
            &list(l_rtn),
            decode::routine,
        )
        .await?
    {
        changed.push(ChangedObject {
            parent: root.clone(),
            subtree: build::subtree(build::routine_obj(&p), []),
        });
    }
    let l_typ: Vec<Oid> = changes.types.iter().copied().collect();
    if !l_typ.is_empty() {
        for t in read_types(r, inc, level, &list(l_typ)).await? {
            changed.push(ChangedObject {
                parent: root.clone(),
                subtree: build::subtree(build::type_obj(&t), []),
            });
        }
    }
    if changes.schema
        && let Some(s) = r
            .read(name::RETRIEVE_NAMESPACE, full, level, &b, decode::schema)
            .await?
            .into_iter()
            .next()
    {
        changed.push(ChangedObject {
            parent: root.clone(),
            subtree: build::subtree(build::schema_obj(&s), []),
        });
    }
    let mut sets = alive.sets;
    let mut existent = BTreeMap::new();
    for k in TOP_KINDS {
        existent.insert(k, ExistSet::Ok(sets.remove(&k).unwrap_or_default()));
    }
    if prev.meta().level >= Level::Structure {
        for k in CHILD_KINDS {
            if k == ObjectKind::Column {
                if !column_scope.is_empty() {
                    existent.insert(
                        k,
                        ExistSet::Scoped {
                            scope: column_scope.clone(),
                            ids: std::mem::take(&mut column_ids),
                        },
                    );
                }
            } else {
                existent.insert(k, ExistSet::Ok(sets.remove(&k).unwrap_or_default()));
            }
        }
    }
    Ok(Ok((
        NamespaceResult {
            existent,
            changed,
            categories_to_reload: BTreeSet::new(),
            new_watermark: now.watermark(),
            full: false,
            level,
            read_at_ms: now_ms(),
        },
        stats,
    )))
}

/// Re-reads the objects `keys` of `ns` (and, for a relation, its children) at `level`. The
/// result is scoped to the keys (R-W2-5): their existence sets are `Scoped`, resolved by oid
/// (RR-1), and the namespace's watermark is not advanced (`Watermark::None`: the engine keeps
/// the cached one; a fragment read nothing else).
pub async fn fragment(
    c: &mut MetaConn,
    ns: &NsKey,
    keys: &[ObjectKey],
    level: Level,
    filter: Option<&CompiledFilter>,
    cancel: &CancellationToken,
) -> Result<PgPass, IntroError> {
    let nsx = Ns::new(ns).ok_or_else(|| IntroError::Protocol(Name::from("pg/namespace-key")))?;
    let mut report = PassReport::default();
    let schema = nsx.schema.clone();
    let (result, stats) = in_snapshot(
        c,
        &schema,
        cancel,
        &mut report,
        None,
        async |r: &mut Reader<'_, '_>, id: &IdentityRow| {
            let oid = ns_oid(&nsx, id)?;
            read_fragment(r, &nsx, oid, keys, level, filter).await
        },
    )
    .await?;
    Ok(PgPass {
        result,
        report,
        stats,
    })
}

async fn read_fragment(
    r: &mut Reader<'_, '_>,
    nsx: &Ns,
    ns: Oid,
    keys: &[ObjectKey],
    level: Level,
    filter: Option<&CompiledFilter>,
) -> Result<Read, IntroError> {
    let b = Binds::ns(ns);
    let alive = alive_of(
        r.read(name::LIST_EXISTENT, Mode::Full, level, &b, decode::existent)
            .await?,
        filter,
    );
    let by_name: HashMap<(&str, &str), Oid> = alive
        .by_oid
        .values()
        .filter(|e| e.parent.is_none())
        .map(|e| ((e.tag.as_str(), e.name.as_str()), e.oid))
        .collect();
    // Each key resolves to a top-level object (a child key to its relation): by oid when the
    // key has one and the server still has it, else by its path's name.
    let base = nsx.key.path.len();
    let mut rels = BTreeMap::new();
    let mut seqs = BTreeSet::new();
    let mut routines = BTreeSet::new();
    let mut types = BTreeSet::new();
    let mut scopes: BTreeMap<ObjectKind, Vec<Vec<Name>>> = BTreeMap::new();
    for k in keys {
        let Some(top) = k.path.get(base) else {
            continue;
        };
        let child = k.path.len() > base + 1;
        let tag = match k.kind {
            _ if child => "rel",
            ObjectKind::Table
            | ObjectKind::View
            | ObjectKind::MaterializedView
            | ObjectKind::ForeignTable => "rel",
            ObjectKind::Sequence => "seq",
            ObjectKind::Routine | ObjectKind::Procedure | ObjectKind::Aggregate => "fn",
            ObjectKind::Domain
            | ObjectKind::EnumType
            | ObjectKind::CompositeType
            | ObjectKind::RangeType
            | ObjectKind::Unknown => "typ",
            _ => continue,
        };
        let by_id = match (child, k.server_id) {
            (false, Some(ServerId::Oid(o))) if alive.by_oid.contains_key(&o) => Some(o),
            _ => None,
        };
        let found = by_id.or_else(|| by_name.get(&(tag, &**top)).copied());
        if !child {
            scopes.entry(k.kind).or_default().push(vec![top.clone()]);
        }
        let Some(oid) = found else {
            continue;
        };
        match tag {
            "rel" => {
                rels.insert(oid, ());
            }
            "seq" => {
                seqs.insert(oid);
            }
            "fn" => {
                routines.insert(oid);
            }
            _ => {
                types.insert(oid);
            }
        }
    }
    let inc = Mode::Incremental;
    let list = |s: Vec<Oid>| Binds::ns(ns).with_oids(s);
    let l_rel: Vec<Oid> = rels.keys().copied().collect();
    let structure = level >= Level::Structure;
    let rel_rows = r
        .read(
            name::RETRIEVE_RELATIONS,
            inc,
            level,
            &list(l_rel.clone()),
            decode::relation,
        )
        .await?;
    let mut ch = if structure && !l_rel.is_empty() {
        RelChildrenRead {
            columns: group(
                r.read(
                    name::RETRIEVE_COLUMNS,
                    inc,
                    level,
                    &list(l_rel.clone()),
                    decode::column,
                )
                .await?,
            ),
            indexes: group(
                r.read(
                    name::RETRIEVE_INDEXES,
                    inc,
                    level,
                    &list(l_rel.clone()),
                    decode::index,
                )
                .await?,
            ),
            constraints: group(
                r.read(
                    name::RETRIEVE_CONSTRAINTS,
                    inc,
                    level,
                    &list(l_rel.clone()),
                    decode::constraint,
                )
                .await?,
            ),
            triggers: group(
                r.read(
                    name::RETRIEVE_TRIGGERS,
                    inc,
                    level,
                    &list(l_rel.clone()),
                    decode::trigger,
                )
                .await?,
            ),
        }
    } else {
        RelChildrenRead {
            columns: BTreeMap::new(),
            indexes: BTreeMap::new(),
            constraints: BTreeMap::new(),
            triggers: BTreeMap::new(),
        }
    };
    let reads: Vec<RelRead> = rel_rows
        .iter()
        .map(|x| RelRead {
            oid: x.oid,
            name: x.name.clone(),
            own: Some(build::relation_obj(nsx, x)),
            columns: structure.then(|| ch.columns.remove(&x.oid).unwrap_or_default()),
            keys: structure.then(|| {
                (
                    ch.indexes.remove(&x.oid).unwrap_or_default(),
                    ch.constraints.remove(&x.oid).unwrap_or_default(),
                )
            }),
            triggers: structure.then(|| ch.triggers.remove(&x.oid).unwrap_or_default()),
        })
        .collect();
    let mut kids = build::children(nsx, &reads, &|_| None, false);
    let root = nsx.root_key();
    let mut changed = Vec::new();
    let mut ids: BTreeMap<ObjectKind, Vec<ExistId>> = BTreeMap::new();
    let mut child_scope: Vec<Vec<Name>> = Vec::new();
    let mut child_ids: BTreeMap<ObjectKind, Vec<ExistId>> = BTreeMap::new();
    let at = SystemTime::now();
    let mut stats = Vec::new();
    for (x, row) in reads.into_iter().zip(&rel_rows) {
        let Some(own) = x.own else { continue };
        if structure {
            stats.push(stat_of(nsx, row, at));
        }
        ids.entry(own.kind())
            .or_default()
            .push(build::top_identity(&own));
        let mut children = kids.columns.remove(&x.oid).unwrap_or_default();
        children.extend(kids.keys.remove(&x.oid).unwrap_or_default());
        children.extend(kids.triggers.remove(&x.oid).unwrap_or_default());
        if structure {
            child_scope.push(vec![Name::from(x.name.as_str())]);
            for c in &children {
                child_ids
                    .entry(c.kind())
                    .or_default()
                    .push(build::child_identity(&x.name, c));
            }
        }
        changed.push(ChangedObject {
            parent: root.clone(),
            subtree: build::subtree(own, children),
        });
    }
    let mut push = |o: Obj, changed: &mut Vec<ChangedObject>| {
        ids.entry(o.kind())
            .or_default()
            .push(build::top_identity(&o));
        changed.push(ChangedObject {
            parent: root.clone(),
            subtree: build::subtree(o, []),
        });
    };
    for s in r
        .read(
            name::RETRIEVE_SEQUENCES,
            inc,
            level,
            &list(seqs.into_iter().collect()),
            decode::sequence,
        )
        .await?
    {
        push(build::sequence_obj(nsx, &s), &mut changed);
    }
    for p in r
        .read(
            name::RETRIEVE_ROUTINES,
            inc,
            level,
            &list(routines.into_iter().collect()),
            decode::routine,
        )
        .await?
    {
        push(build::routine_obj(&p), &mut changed);
    }
    let l_typ: Vec<Oid> = types.into_iter().collect();
    if !l_typ.is_empty() {
        for t in read_types(r, inc, level, &list(l_typ)).await? {
            push(build::type_obj(&t), &mut changed);
        }
    }
    let mut existent = BTreeMap::new();
    for (kind, scope) in scopes {
        existent.insert(
            kind,
            ExistSet::Scoped {
                scope,
                ids: ids.remove(&kind).unwrap_or_default(),
            },
        );
    }
    if structure && !child_scope.is_empty() {
        for k in CHILD_KINDS {
            existent.insert(
                k,
                ExistSet::Scoped {
                    scope: child_scope.clone(),
                    ids: child_ids.remove(&k).unwrap_or_default(),
                },
            );
        }
    }
    Ok((
        NamespaceResult {
            existent,
            changed,
            categories_to_reload: BTreeSet::new(),
            new_watermark: Watermark::None,
            full: false,
            level,
            read_at_ms: now_ms(),
        },
        stats,
    ))
}

/// The namespaces the login can see: every database it may connect to (`NsKind::Database`;
/// a scope naming another database loads it on that database's connection, T15 review I-2),
/// then the schemas of the connected database (`pg_toast*` and other sessions' `pg_temp_*`
/// left out; `pg_catalog`, `information_schema` and other `pg_*` schemas flagged system).
pub async fn list_namespaces(
    c: &mut MetaConn,
    cancel: &CancellationToken,
) -> Result<Vec<crate::scope::NsInfo>, IntroError> {
    let mut report = PassReport::default();
    let (dbs, rows) = in_snapshot(
        c,
        &Name::from("pg_catalog"),
        cancel,
        &mut report,
        None,
        async |r: &mut Reader<'_, '_>, _id: &IdentityRow| {
            let dbs = r
                .read(
                    name::LIST_DATABASES,
                    Mode::Full,
                    Level::Names,
                    &Binds::default(),
                    |row| row.string(0),
                )
                .await?;
            let rows = r
                .read(
                    name::LIST_NAMESPACES,
                    Mode::Full,
                    Level::Names,
                    &Binds::default(),
                    decode::namespace_listed,
                )
                .await?;
            Ok((dbs, rows))
        },
    )
    .await?;
    let mut out: Vec<crate::scope::NsInfo> = dbs
        .into_iter()
        .map(|db| crate::scope::NsInfo {
            key: NsKey::database(db),
            system: false,
        })
        .collect();
    out.extend(
        rows.into_iter()
            .filter(|(s, _)| {
                !(s.name.starts_with("pg_toast")
                    || s.name.starts_with("pg_temp_")
                    || s.name.starts_with("pg_toast_temp_"))
            })
            .map(|(s, db)| crate::scope::NsInfo {
                system: s.name.starts_with("pg_") || s.name == "information_schema",
                key: NsKey::schema([db, s.name]),
            }),
    );
    Ok(out)
}

/// The server's identity as one probe reads it (the `Introspector::identity` seam of ruling
/// R-W3-3 adopts it): the `Identity` row, whose `system_identifier` and `timeline` are `None`
/// where the role may not call `pg_control_system()` / `pg_control_checkpoint()`.
pub async fn server_identity(
    c: &mut MetaConn,
    cancel: &CancellationToken,
) -> Result<IdentityRow, IntroError> {
    let mut report = PassReport::default();
    in_snapshot(
        c,
        &Name::from("pg_catalog"),
        cancel,
        &mut report,
        None,
        async |_r: &mut Reader<'_, '_>, id: &IdentityRow| Ok(id.clone()),
    )
    .await
}

/// The cheap validation of a cached namespace (HV-INTRO-15): the existing objects (count and
/// an XXH3-64 of the sorted `tag:oid:sub` keys and the comment keys) and `max_mark` =
/// `StateMark::Xmin` of the cached watermark's low 32 bits when nothing changed since it (no
/// young row, no dependency hop, an incremental plan), else of the server's current snapshot
/// xmin. Only `Identity`, `ListExistent`, `ListExistentComments`, the two change queries and
/// the canary run.
pub async fn check_namespace(
    c: &mut MetaConn,
    ns: &NsKey,
    snap: &NamespaceSnapshot,
    cancel: &CancellationToken,
) -> Result<(crate::freshness::NsCheck, PassReport), IntroError> {
    let nsx = Ns::new(ns).ok_or_else(|| IntroError::Protocol(Name::from("pg/namespace-key")))?;
    let mut report = PassReport::default();
    let schema = nsx.schema.clone();
    let check = in_snapshot(
        c,
        &schema,
        cancel,
        &mut report,
        None,
        async |r: &mut Reader<'_, '_>, id: &IdentityRow| {
            let oid = ns_oid(&nsx, id)?;
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
            // The digest also covers what leaves no young row behind (review I-3): each
            // object's `sub` (a domain's constraint count moves on `ALTER DOMAIN … DROP
            // CONSTRAINT`), and every comment key (`COMMENT … IS NULL` deletes its row).
            let comments = r
                .read(
                    name::LIST_EXISTENT_COMMENTS,
                    Mode::Full,
                    Level::Structure,
                    &b,
                    decode::comment_key,
                )
                .await?;
            let mut keys: Vec<String> = rows
                .iter()
                .map(|e| format!("{}:{}:{}", e.tag, e.oid, e.sub.as_deref().unwrap_or("")))
                .chain(
                    comments
                        .iter()
                        .map(|(t, o, s)| format!("comment:{t}:{o}:{s}")),
                )
                .collect();
            keys.sort();
            let digest = twox_hash::XxHash3_64::oneshot(keys.join("\n").as_bytes());
            let now = Now::of(id);
            let changed = match plan(&snap.meta().watermark, &now) {
                Plan::Full(_) => true,
                Plan::Incremental { since } => {
                    let bw = Binds::ns(oid).with_w(since);
                    let a = r
                        .read(
                            name::CHANGED_PARENTS,
                            Mode::Full,
                            Level::Structure,
                            &bw,
                            decode::facet,
                        )
                        .await?;
                    let d = r
                        .read(
                            name::CHANGED_DEPENDENTS,
                            Mode::Full,
                            Level::Structure,
                            &bw,
                            decode::facet,
                        )
                        .await?;
                    !(a.is_empty() && d.is_empty())
                }
            };
            let mark = match (&snap.meta().watermark, changed) {
                (Watermark::PgXid { w, .. }, false) => (*w & 0xFFFF_FFFF) as u32,
                _ => (id.snap_xmin & 0xFFFF_FFFF) as u32,
            };
            Ok(crate::freshness::NsCheck {
                objects: rows.len() as u64,
                max_mark: StateMark::Xmin(mark),
                existence_digest: digest,
            })
        },
    )
    .await?;
    Ok((check, report))
}
