//! The passes: [`full`] reads a whole namespace; [`refresh`] reads only what changed since a
//! watermark (HV-INTRO-05), or falls back to full. Each pass is one `REPEATABLE READ READ
//! ONLY` transaction (consistent snapshot; one transaction also suits PgBouncer transaction
//! pooling) that ends with the no-write canary.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use super::IntroError;
use super::conn::CatalogConn;
use super::decode::{self, Row};
use super::merge::normalize;
use super::model::*;
use super::sql::{self, Filter, PgVersion};
use super::watermark::{FullReason, PgWatermark, Plan, Probe, plan};

/// How much a pass reads (HV-INTRO-01 levels, reduced to the switch the spike measures).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct IntrospectOpts {
    /// The `Sources` level: view, index and routine definitions.
    pub sources: bool,
}

/// One section's cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionStat {
    pub name: &'static str,
    pub rows: usize,
    /// The round trip, including the transfer of the rows as text.
    pub query: Duration,
    pub decode: Duration,
}

/// What a pass cost and whether it wrote.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PassStats {
    pub sections: Vec<SectionStat>,
    /// The canary: whether the pass's transaction was assigned an XID (it must not be).
    pub xid_assigned: bool,
    pub total: Duration,
}

impl PassStats {
    /// Whether a section of this name ran.
    pub fn ran(&self, name: &str) -> bool {
        self.sections.iter().any(|s| s.name == name)
    }
}

/// A pass's result, the watermark to store with it, and its cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pass<T> {
    pub result: T,
    pub watermark: PgWatermark,
    pub stats: PassStats,
}

/// What [`refresh`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refresh {
    /// The namespace was read whole, for `reason`.
    Full { ns: PgNamespace, reason: FullReason },
    /// Only what changed; apply it with [`super::apply`].
    Delta(PgNamespaceDelta),
    /// The namespace no longer exists.
    Gone,
}

struct Reader<'c, C: ?Sized> {
    conn: &'c mut C,
    stats: Vec<SectionStat>,
}

impl<C: CatalogConn + ?Sized> Reader<'_, C> {
    async fn read<T>(
        &mut self,
        name: &'static str,
        sql: &str,
        f: impl Fn(&Row<'_>) -> Result<T, IntroError>,
    ) -> Result<Vec<T>, IntroError> {
        let t0 = Instant::now();
        let rows = self.conn.rows(sql).await.map_err(IntroError::Db)?;
        let query = t0.elapsed();
        let t1 = Instant::now();
        let out = rows
            .iter()
            .map(|cells| {
                f(&Row {
                    section: name,
                    cells,
                })
            })
            .collect::<Result<Vec<T>, _>>()?;
        self.stats.push(SectionStat {
            name,
            rows: rows.len(),
            query,
            decode: t1.elapsed(),
        });
        Ok(out)
    }

    async fn probe(&mut self, ns: Oid) -> Result<Probe, IntroError> {
        self.read("pg/Identity", &sql::probe(ns), Probe::decode)
            .await?
            .into_iter()
            .next()
            .ok_or(IntroError::Decode {
                section: "pg/Identity",
                detail: "no row".into(),
            })
    }
}

/// Runs `body` in a `REPEATABLE READ READ ONLY` transaction, then the canary; commits on
/// success, rolls back on error.
async fn in_snapshot<C, T>(
    conn: &mut C,
    body: impl AsyncFnOnce(&mut Reader<'_, C>) -> Result<T, IntroError>,
) -> Result<(T, PassStats), IntroError>
where
    C: CatalogConn + ?Sized,
{
    let t0 = Instant::now();
    conn.control("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .map_err(IntroError::Db)?;
    let mut r = Reader {
        conn: &mut *conn,
        stats: Vec::new(),
    };
    let out = async {
        let v = body(&mut r).await?;
        let canary = r.read("pg/Canary", sql::CANARY, |row| row.bool(0)).await?;
        Ok::<_, IntroError>((v, !canary.first().copied().unwrap_or(false)))
    }
    .await;
    let stats = std::mem::take(&mut r.stats);
    match out {
        Ok((v, xid_assigned)) => {
            conn.control("COMMIT").await.map_err(IntroError::Db)?;
            Ok((
                v,
                PassStats {
                    sections: stats,
                    xid_assigned,
                    total: t0.elapsed(),
                },
            ))
        }
        Err(e) => {
            // The error is what the caller needs; a failed rollback leaves a broken
            // connection the session discards anyway.
            let _ = conn.control("ROLLBACK").await;
            Err(e)
        }
    }
}

/// Every schema of the connected database (`pg_toast*` and other sessions' temporary schemas
/// left out).
pub async fn list_schemas<C: CatalogConn + ?Sized>(
    conn: &mut C,
) -> Result<Vec<PgSchema>, IntroError> {
    let mut r = Reader {
        conn,
        stats: Vec::new(),
    };
    let mut v = r
        .read("pg/ListNamespaces", sql::SCHEMAS, decode::schema)
        .await?;
    v.retain(|s| {
        !(s.name.starts_with("pg_toast")
            || s.name.starts_with("pg_temp_")
            || s.name.starts_with("pg_toast_temp_"))
    });
    v.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(v)
}

/// Reads the namespace `ns` whole.
pub async fn full<C: CatalogConn + ?Sized>(
    conn: &mut C,
    ns: Oid,
    opts: IntrospectOpts,
) -> Result<Pass<PgNamespace>, IntroError> {
    let ((ns, probe), stats) = in_snapshot(conn, async |r: &mut Reader<'_, C>| {
        let probe = r.probe(ns).await?;
        if !probe.namespace_exists {
            return Err(IntroError::NamespaceGone(ns));
        }
        let read = read_namespace(r, ns, probe.version, opts).await?;
        Ok((read, probe))
    })
    .await?;
    Ok(Pass {
        result: ns,
        watermark: probe.watermark(),
        stats,
    })
}

/// Re-reads what changed in `prev`'s namespace since `since` (HV-INTRO-05), or the whole
/// namespace when [`plan`] says so.
pub async fn refresh<C: CatalogConn + ?Sized>(
    conn: &mut C,
    prev: &PgNamespace,
    since: Option<&PgWatermark>,
    opts: IntrospectOpts,
) -> Result<Pass<Refresh>, IntroError> {
    let ns = prev.schema.oid;
    let ((result, probe), stats) = in_snapshot(conn, async |r: &mut Reader<'_, C>| {
        let probe = r.probe(ns).await?;
        if !probe.namespace_exists {
            return Ok((Refresh::Gone, probe));
        }
        let result = match plan(since, &probe) {
            Plan::Full(reason) => Refresh::Full {
                ns: read_namespace(r, ns, probe.version, opts).await?,
                reason,
            },
            Plan::Incremental { since } => {
                Refresh::Delta(read_delta(r, prev, probe.version, since, opts).await?)
            }
        };
        Ok((result, probe))
    })
    .await?;
    Ok(Pass {
        result,
        watermark: probe.watermark(),
        stats,
    })
}

/// The relation children read by one filter.
struct Children {
    columns: BTreeMap<Oid, Vec<PgColumn>>,
    indexes: BTreeMap<Oid, Vec<PgIndex>>,
    constraints: BTreeMap<Oid, Vec<PgConstraint>>,
    triggers: BTreeMap<Oid, Vec<PgTrigger>>,
}

/// `m`'s list for `o` (empty when it has none) when `on`.
fn take<T: Clone>(m: &BTreeMap<Oid, Vec<T>>, o: Oid, on: bool) -> Option<Vec<T>> {
    on.then(|| m.get(&o).cloned().unwrap_or_default())
}

fn group<T>(rows: Vec<(Oid, T)>) -> BTreeMap<Oid, Vec<T>> {
    let mut m: BTreeMap<Oid, Vec<T>> = BTreeMap::new();
    for (k, v) in rows {
        m.entry(k).or_default().push(v);
    }
    m
}

async fn read_types<C: CatalogConn + ?Sized>(
    r: &mut Reader<'_, C>,
    f: &Filter<'_>,
) -> Result<Vec<PgType>, IntroError> {
    let mut types = r
        .read("pg/RetrieveTypes", &sql::types(f), decode::user_type)
        .await?;
    let mut dcons = group(
        r.read(
            "pg/RetrieveDomainConstraints",
            &sql::domain_constraints(f),
            decode::domain_constraint,
        )
        .await?,
    );
    for t in &mut types {
        if let UserTypeKind::Domain { constraints, .. } = &mut t.kind {
            *constraints = dcons.remove(&t.oid).unwrap_or_default();
        }
    }
    Ok(types)
}

async fn read_namespace<C: CatalogConn + ?Sized>(
    r: &mut Reader<'_, C>,
    ns: Oid,
    v: PgVersion,
    opts: IntrospectOpts,
) -> Result<PgNamespace, IntroError> {
    let f = Filter::Namespace(ns);
    let schema = r
        .read("pg/RetrieveNamespace", &sql::schema(ns), decode::schema)
        .await?
        .into_iter()
        .next()
        .ok_or(IntroError::NamespaceGone(ns))?;
    let mut relations = r
        .read(
            "pg/RetrieveRelations",
            &sql::relations(&f, opts.sources),
            decode::relation,
        )
        .await?;
    let mut ch = Children {
        columns: group(
            r.read("pg/RetrieveColumns", &sql::columns(&f), decode::column)
                .await?,
        ),
        indexes: group(
            r.read(
                "pg/RetrieveIndexes",
                &sql::indexes(&f, v, opts.sources),
                decode::index,
            )
            .await?,
        ),
        constraints: group(
            r.read(
                "pg/RetrieveConstraints",
                &sql::constraints(&f, v),
                decode::constraint,
            )
            .await?,
        ),
        triggers: group(
            r.read("pg/RetrieveTriggers", &sql::triggers(&f), decode::trigger)
                .await?,
        ),
    };
    for rel in &mut relations {
        rel.columns = ch.columns.remove(&rel.oid).unwrap_or_default();
        rel.indexes = ch.indexes.remove(&rel.oid).unwrap_or_default();
        rel.constraints = ch.constraints.remove(&rel.oid).unwrap_or_default();
        rel.triggers = ch.triggers.remove(&rel.oid).unwrap_or_default();
    }
    let sequences = r
        .read(
            "pg/RetrieveSequences",
            &sql::sequences(&f),
            decode::sequence,
        )
        .await?;
    let routines = r
        .read(
            "pg/RetrieveRoutines",
            &sql::routines(&f, opts.sources),
            decode::routine,
        )
        .await?;
    let types = read_types(r, &f).await?;
    let mut out = PgNamespace {
        schema,
        relations,
        sequences,
        routines,
        types,
    };
    normalize(&mut out);
    Ok(out)
}

/// The facets a relation needs re-read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Facets {
    rel: bool,
    columns: bool,
    indexes: bool,
    constraints: bool,
    triggers: bool,
    comments: bool,
}

/// Where a comment of the previous snapshot lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommentOwner {
    Rel(Oid, Facet),
    Sequence(Oid),
    Routine(Oid),
    Type(Oid),
    Schema,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Facet {
    Comments,
    Indexes,
    Constraints,
    Triggers,
}

type CommentKey = (char, Oid, i32);

/// Every comment of `ns` by its `pg_description` key, and where it lives.
fn comment_owners(ns: &PgNamespace) -> BTreeMap<CommentKey, CommentOwner> {
    let mut m = BTreeMap::new();
    if ns.schema.comment.is_some() {
        m.insert(('n', ns.schema.oid, 0), CommentOwner::Schema);
    }
    for r in &ns.relations {
        let own = CommentOwner::Rel(r.oid, Facet::Comments);
        if r.comment.is_some() {
            m.insert(('c', r.oid, 0), own);
        }
        for c in r.columns.iter().filter(|c| c.comment.is_some()) {
            m.insert(('c', r.oid, i32::from(c.num)), own);
        }
        for i in r.indexes.iter().filter(|i| i.comment.is_some()) {
            m.insert(('c', i.oid, 0), CommentOwner::Rel(r.oid, Facet::Indexes));
        }
        for k in r.constraints.iter().filter(|k| k.comment.is_some()) {
            m.insert(
                ('k', k.oid, 0),
                CommentOwner::Rel(r.oid, Facet::Constraints),
            );
        }
        for t in r.triggers.iter().filter(|t| t.comment.is_some()) {
            m.insert(('t', t.oid, 0), CommentOwner::Rel(r.oid, Facet::Triggers));
        }
    }
    for s in ns.sequences.iter().filter(|s| s.comment.is_some()) {
        m.insert(('c', s.oid, 0), CommentOwner::Sequence(s.oid));
    }
    for p in ns.routines.iter().filter(|p| p.comment.is_some()) {
        m.insert(('p', p.oid, 0), CommentOwner::Routine(p.oid));
    }
    for t in ns.types.iter().filter(|t| t.comment.is_some()) {
        m.insert(('y', t.oid, 0), CommentOwner::Type(t.oid));
    }
    m
}

async fn read_delta<C: CatalogConn + ?Sized>(
    r: &mut Reader<'_, C>,
    prev: &PgNamespace,
    v: PgVersion,
    since: u32,
    opts: IntrospectOpts,
) -> Result<PgNamespaceDelta, IntroError> {
    let ns = prev.schema.oid;
    // 4a: existence.
    let mut alive = Alive::default();
    for (tag, oid) in r
        .read("pg/ListExistent", &sql::alive(ns), |row| {
            Ok((row.char(0)?, row.num::<Oid>(1)?))
        })
        .await?
    {
        let set = match tag {
            'r' => &mut alive.relations,
            'S' => &mut alive.sequences,
            'x' => &mut alive.children,
            'p' => &mut alive.routines,
            _ => &mut alive.types,
        };
        set.insert(oid);
    }
    // 4b: young rows, then one level of dependents of the database's young rows.
    let mut young: Vec<(String, Oid)> = r
        .read("pg/ChangedParents", &sql::changed(ns, since), |row| {
            Ok((row.string(0)?, row.num(1)?))
        })
        .await?;
    young.extend(
        r.read(
            "pg/ChangedDependents",
            &sql::changed_deps(ns, since),
            |row| Ok((row.string(0)?, row.num(1)?)),
        )
        .await?,
    );
    let mut rels: BTreeMap<Oid, Facets> = BTreeMap::new();
    let mut seqs = BTreeSet::new();
    let mut routines = BTreeSet::new();
    let mut types = BTreeSet::new();
    let mut schema = false;
    for (facet, oid) in &young {
        match facet.as_str() {
            "rel" => rels.entry(*oid).or_default().rel = true,
            "columns" => rels.entry(*oid).or_default().columns = true,
            "indexes" => rels.entry(*oid).or_default().indexes = true,
            "constraints" => rels.entry(*oid).or_default().constraints = true,
            "triggers" => rels.entry(*oid).or_default().triggers = true,
            "comments" => rels.entry(*oid).or_default().comments = true,
            "sequence" => {
                seqs.insert(*oid);
            }
            "routine" => {
                routines.insert(*oid);
            }
            "type" => {
                types.insert(*oid);
            }
            "schema" => schema = true,
            other => {
                return Err(IntroError::Decode {
                    section: "pg/ChangedParents",
                    detail: format!("unknown facet {other:?}"),
                });
            }
        }
    }
    // Comments that disappeared leave no young row (`COMMENT … IS NULL` deletes it).
    let now: BTreeSet<CommentKey> = r
        .read("pg/ListExistentComments", &sql::alive_comments(ns), |row| {
            Ok((row.char(0)?, row.num(1)?, row.num(2)?))
        })
        .await?
        .into_iter()
        .collect();
    for (key, owner) in comment_owners(prev) {
        if now.contains(&key) {
            continue;
        }
        match owner {
            CommentOwner::Rel(oid, f) => {
                let e = rels.entry(oid).or_default();
                match f {
                    Facet::Comments => e.comments = true,
                    Facet::Indexes => e.indexes = true,
                    Facet::Constraints => e.constraints = true,
                    Facet::Triggers => e.triggers = true,
                }
            }
            CommentOwner::Sequence(o) => {
                seqs.insert(o);
            }
            CommentOwner::Routine(o) => {
                routines.insert(o);
            }
            CommentOwner::Type(o) => {
                types.insert(o);
            }
            CommentOwner::Schema => schema = true,
        }
    }
    // A column change re-deparses every text of the same relation that names columns
    // (partition key, index expressions, constraint and trigger definitions).
    rels.retain(|oid, _| alive.relations.contains(oid));
    for f in rels.values_mut() {
        if f.columns {
            f.rel = true;
            f.indexes = true;
            f.constraints = true;
            f.triggers = true;
        }
    }
    let list = |pick: fn(&Facets) -> bool| -> Vec<Oid> {
        rels.iter()
            .filter(|(_, f)| pick(f))
            .map(|(o, _)| *o)
            .collect()
    };
    let (l_rel, l_col, l_idx, l_con, l_trg, l_cmt) = (
        list(|f| f.rel),
        list(|f| f.columns),
        list(|f| f.indexes),
        list(|f| f.constraints),
        list(|f| f.triggers),
        list(|f| f.comments && !f.columns),
    );
    // 4c: re-read the changed facets.
    let mut headers: BTreeMap<Oid, PgRelation> = BTreeMap::new();
    if !l_rel.is_empty() {
        for h in r
            .read(
                "pg/RetrieveRelations",
                &sql::relations(&Filter::Oids(&l_rel), opts.sources),
                decode::relation,
            )
            .await?
        {
            headers.insert(h.oid, h);
        }
    }
    let columns = if l_col.is_empty() {
        BTreeMap::new()
    } else {
        group(
            r.read(
                "pg/RetrieveColumns",
                &sql::columns(&Filter::Oids(&l_col)),
                decode::column,
            )
            .await?,
        )
    };
    let indexes = if l_idx.is_empty() {
        BTreeMap::new()
    } else {
        group(
            r.read(
                "pg/RetrieveIndexes",
                &sql::indexes(&Filter::Oids(&l_idx), v, opts.sources),
                decode::index,
            )
            .await?,
        )
    };
    let constraints = if l_con.is_empty() {
        BTreeMap::new()
    } else {
        group(
            r.read(
                "pg/RetrieveConstraints",
                &sql::constraints(&Filter::Oids(&l_con), v),
                decode::constraint,
            )
            .await?,
        )
    };
    let triggers = if l_trg.is_empty() {
        BTreeMap::new()
    } else {
        group(
            r.read(
                "pg/RetrieveTriggers",
                &sql::triggers(&Filter::Oids(&l_trg)),
                decode::trigger,
            )
            .await?,
        )
    };
    let comments = if l_cmt.is_empty() {
        BTreeMap::new()
    } else {
        group(
            r.read("pg/Comments", &sql::rel_comments(&l_cmt), |row| {
                Ok((row.num::<Oid>(0)?, (row.num::<i16>(1)?, row.string(2)?)))
            })
            .await?,
        )
    };
    let mut patches = Vec::new();
    for (&oid, f) in &rels {
        let comments = (f.comments && !f.columns).then(|| {
            let mut c = RelComments::default();
            for (num, text) in comments.get(&oid).cloned().unwrap_or_default() {
                if num == 0 {
                    c.relation = Some(text);
                } else {
                    c.columns.push((num, text));
                }
            }
            c
        });
        let header = headers.remove(&oid);
        let cols = take(&columns, oid, f.columns);
        patches.push(RelationPatch {
            oid,
            header,
            columns: cols,
            indexes: take(&indexes, oid, f.indexes),
            constraints: take(&constraints, oid, f.constraints),
            triggers: take(&triggers, oid, f.triggers),
            comments,
        });
    }
    let alive_list = |s: &BTreeSet<Oid>, alive: &BTreeSet<Oid>| -> Vec<Oid> {
        s.iter().filter(|o| alive.contains(o)).copied().collect()
    };
    let l_seq = alive_list(&seqs, &alive.sequences);
    let sequences = if l_seq.is_empty() {
        Vec::new()
    } else {
        r.read(
            "pg/RetrieveSequences",
            &sql::sequences(&Filter::Oids(&l_seq)),
            decode::sequence,
        )
        .await?
    };
    let l_rtn = alive_list(&routines, &alive.routines);
    let routines = if l_rtn.is_empty() {
        Vec::new()
    } else {
        r.read(
            "pg/RetrieveRoutines",
            &sql::routines(&Filter::Oids(&l_rtn), opts.sources),
            decode::routine,
        )
        .await?
    };
    let l_typ = alive_list(&types, &alive.types);
    let types = if l_typ.is_empty() {
        Vec::new()
    } else {
        read_types(r, &Filter::Oids(&l_typ)).await?
    };
    let schema = if schema {
        r.read("pg/RetrieveNamespace", &sql::schema(ns), decode::schema)
            .await?
            .into_iter()
            .next()
    } else {
        None
    };
    Ok(PgNamespaceDelta {
        schema,
        alive,
        relations: patches,
        sequences,
        routines,
        types,
    })
}
