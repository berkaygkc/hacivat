// ## m4-t18-sqlite
//! Reading one SQLite schema through the pack (`packs/sqlite/*.hvq.sql`) on one snapshot: the
//! `sqlite_schema` rows, the `pragma_table_list` flags where the connection allows them, and
//! the bulk pragma joins (spike origin `f690f45`, `sqlite.rs` `read_schema` and
//! `columns_one_by_one`), with their per-relation fallback (F10). Rows are kept raw here;
//! `build` turns them into model objects.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use hacivat_meta::snapshot::IntroErrorCode;
use hacivat_meta::{Level, Name, NsKey};
use hacivat_proto::DbmsKind;
use hacivat_session::meta::{MetaError, SnapshotTxn};
use tokio_util::sync::CancellationToken;

use super::{ATTACH_ALIAS, MAX_FALLBACK_FAILURES, ProgressSink, SectionTrace};
use crate::engine::Phase;
use crate::introspector::IntroError;
use crate::pack::{BindValue, DialectId, LogicalType, Mode, Pack, PackCtx, PackFlags, Params};

/// Which copy of the `sqlite_schema` sections reads a schema (the pack's header).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Variant {
    Main,
    Temp,
    /// An attached file, read under [`ATTACH_ALIAS`].
    Attached,
}

impl Variant {
    fn suffix(self) -> &'static str {
        match self {
            Variant::Main => "Main",
            Variant::Temp => "Temp",
            Variant::Attached => "Attached",
        }
    }

    /// The schema name the pragmas take.
    pub(crate) fn schema(self) -> &'static str {
        match self {
            Variant::Main => "main",
            Variant::Temp => "temp",
            Variant::Attached => ATTACH_ALIAS,
        }
    }
}

/// What every read of one pass shares.
pub(crate) struct Cx<'a> {
    pub pack: &'a Pack,
    pub ctx: PackCtx,
    pub trace: Option<&'a SectionTrace>,
    pub cancel: &'a CancellationToken,
    /// Where the pass's counts go (`SqliteIntrospector::with_progress`), with its namespace.
    pub progress: Option<(&'a NsKey, &'a ProgressSink)>,
}

impl<'a> Cx<'a> {
    pub(crate) fn new(
        pack: &'a Pack,
        joined: bool,
        level: Level,
        trace: Option<&'a SectionTrace>,
        cancel: &'a CancellationToken,
    ) -> Self {
        Cx {
            pack,
            ctx: PackCtx {
                dialect: DialectId::Sqlite,
                version: 0,
                flavor: DbmsKind::Sqlite,
                mode: Mode::Full,
                level,
                flags: if joined {
                    PackFlags::JOINED
                } else {
                    PackFlags::empty()
                },
                params: Params::default(),
            },
            trace,
            cancel,
            progress: None,
        }
    }

    /// Reports `done` of `total` in `phase` to the progress sink, if any.
    pub(crate) fn report(&self, phase: Phase, done: u64, total: Option<u64>) {
        if let Some((ns, sink)) = self.progress {
            sink(ns, phase, done, total);
        }
    }
}

/// A text bind.
fn text(s: &str) -> BindValue {
    BindValue::Typed {
        hint: LogicalType::Text {
            max_chars: None,
            fixed: false,
        },
        text: Arc::from(s),
    }
}

/// One row: text cells, `None` for NULL.
pub(crate) type Row = Vec<Option<Arc<str>>>;

/// Runs pack section `section` with text binds `binds` on `snap`. The answer must have the
/// section's declared columns, by name and count (alg/02 HV-INTRO-04 step 4), else
/// [`IntroError::Protocol`].
pub(crate) async fn rows(
    cx: &Cx<'_>,
    snap: &mut SnapshotTxn<'_>,
    section: &str,
    binds: &[(&str, &str)],
) -> Result<Vec<Row>, IntroError> {
    if cx.cancel.is_cancelled() {
        return Err(IntroError::Cancelled);
    }
    if let Some(t) = cx.trace {
        t.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(Name::from(section));
    }
    let protocol = || IntroError::Protocol(Name::from(section));
    let mut ctx = cx.ctx.clone();
    ctx.params.scalars = binds
        .iter()
        .map(|(n, v)| (Name::from(*n), text(v)))
        .collect();
    let queries = cx.pack.render(section, &ctx).map_err(|_| protocol())?;
    let [q] = queries.as_slice() else {
        return Err(protocol());
    };
    let declared = cx.pack.section(section).ok_or_else(protocol)?.returns();
    let got = snap.rows(q.as_meta_query()).await?;
    let names_ok = got.columns.is_empty()
        || (got.columns.len() == declared.len()
            && got
                .columns
                .iter()
                .zip(declared)
                .all(|(c, d)| **c == *d.name));
    if !names_ok || got.rows.iter().any(|r| r.len() != declared.len()) {
        return Err(protocol());
    }
    Ok(got.rows)
}

/// Whether `e` is the engine's `SQLITE_BUSY` or `SQLITE_LOCKED` (any extended code): another
/// connection holds a lock; retried once (alg/02 HV-INTRO-08 edge 11).
pub(crate) fn is_busy(e: &IntroError) -> bool {
    match e {
        IntroError::Meta(MetaError::Server { message, .. }) => message
            .vendor_code
            .is_some_and(|c| matches!(c & 0xff, 5 | 6)),
        _ => false,
    }
}

/// The engine refused one statement (not a lost connection, a cancel or a busy file): what a
/// fallback may absorb.
fn refused(e: &IntroError) -> bool {
    matches!(e, IntroError::Meta(MetaError::Server { .. })) && !is_busy(e)
}

/// The text of a refused statement's message (the engine's own words).
fn message(e: &IntroError) -> String {
    match e {
        IntroError::Meta(MetaError::Server { message, .. }) => message.text.clone(),
        _ => String::new(),
    }
}

pub(crate) fn cell(r: &Row, i: usize) -> Option<&Arc<str>> {
    r.get(i).and_then(Option::as_ref)
}

pub(crate) fn name(r: &Row, i: usize) -> Name {
    cell(r, i).cloned().unwrap_or_else(|| Name::from(""))
}

pub(crate) fn int(r: &Row, i: usize) -> Option<i64> {
    cell(r, i).and_then(|c| c.trim().parse().ok())
}

fn flag(r: &Row, i: usize) -> bool {
    int(r, i).is_some_and(|v| v != 0)
}

/// One `pragma_database_list` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DbEntry {
    pub seq: i64,
    pub name: Name,
    /// The file's path; empty for `temp` and an in-memory database.
    pub file: Name,
}

pub(crate) async fn database_list(
    cx: &Cx<'_>,
    snap: &mut SnapshotTxn<'_>,
) -> Result<Vec<DbEntry>, IntroError> {
    let mut out: Vec<DbEntry> = rows(cx, snap, "DatabaseList", &[])
        .await?
        .iter()
        .map(|r| DbEntry {
            seq: int(r, 0).unwrap_or(i64::MAX),
            name: name(r, 1),
            file: name(r, 2),
        })
        .collect();
    out.sort_by_key(|e| e.seq);
    Ok(out)
}

/// `main`'s `schema_version` (the table-valued form takes no schema: main only).
pub(crate) async fn schema_version(
    cx: &Cx<'_>,
    snap: &mut SnapshotTxn<'_>,
) -> Result<u32, IntroError> {
    let r = rows(cx, snap, "SchemaVersionMain", &[]).await?;
    r.first()
        .and_then(|r| int(r, 0))
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| IntroError::Protocol(Name::from("SchemaVersionMain")))
}

/// One `sqlite_schema` row.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ObjRow {
    pub ty: Name,
    pub name: Name,
    pub tbl: Name,
    pub sql: Option<Name>,
}

pub(crate) async fn objects(
    cx: &Cx<'_>,
    snap: &mut SnapshotTxn<'_>,
    v: Variant,
) -> Result<Vec<ObjRow>, IntroError> {
    let section = format!("Objects{}", v.suffix());
    Ok(rows(cx, snap, &section, &[])
        .await?
        .iter()
        .map(|r| ObjRow {
            ty: name(r, 0),
            name: name(r, 1),
            tbl: name(r, 2),
            sql: cell(r, 3).cloned(),
        })
        .collect())
}

/// One `pragma_table_list` row's flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TableFlags {
    /// `table`, `view`, `virtual` or `shadow`.
    pub ty: Name,
    pub without_rowid: bool,
    pub strict: bool,
}

/// One `pragma_table_xinfo` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ColRow {
    pub rel: Name,
    pub cid: i64,
    pub name: Name,
    pub ty: Name,
    pub notnull: bool,
    pub dflt: Option<Name>,
    pub pk: u32,
    pub hidden: i64,
}

/// One `pragma_index_list` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IdxRow {
    pub rel: Name,
    pub name: Name,
    pub unique: bool,
    pub origin: Name,
    pub partial: bool,
}

/// One `pragma_index_xinfo` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PartRow {
    pub index: Name,
    pub seqno: u32,
    pub cid: i64,
    pub name: Option<Name>,
    pub desc: bool,
    pub coll: Option<Name>,
    pub key: bool,
}

/// One `pragma_foreign_key_list` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FkRow {
    pub rel: Name,
    pub id: u32,
    pub seq: u32,
    pub ref_table: Name,
    pub from: Name,
    pub to: Option<Name>,
    pub on_update: Name,
    pub on_delete: Name,
    pub match_kind: Name,
}

/// Everything one read of a schema returned.
#[derive(Debug, Clone, Default)]
pub(crate) struct SchemaRows {
    pub objects: Vec<ObjRow>,
    /// `pragma_table_list` flags by name; `None` when not read (the user's connection, whose
    /// authorizer refuses it, R-M4-38) or refused: the text scan answers instead.
    pub flags: Option<HashMap<Name, TableFlags>>,
    /// The pragmas below were read (`Structure` and above).
    pub structure: bool,
    pub columns: Vec<ColRow>,
    /// Relations whose columns could not be read, with the engine's message (F10).
    pub broken: BTreeMap<Name, String>,
    pub indexes: Vec<IdxRow>,
    pub parts: Vec<PartRow>,
    /// The index pragmas failed: no index or unique key is decided.
    pub index_failed: Option<IntroErrorCode>,
    pub fks: Vec<FkRow>,
    /// The foreign-key pragma failed: no foreign key is decided.
    pub fk_failed: Option<IntroErrorCode>,
}

fn col_row(rel: Name, r: &Row, at: usize) -> ColRow {
    ColRow {
        rel,
        cid: int(r, at).unwrap_or(-1),
        name: name(r, at + 1),
        ty: name(r, at + 2),
        notnull: flag(r, at + 3),
        dflt: cell(r, at + 4).cloned(),
        pk: int(r, at + 5)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0),
        hidden: int(r, at + 6).unwrap_or(0),
    }
}

/// Reads schema `v` at `level`, starting from its `objects` (already read for the digest).
/// `with_flags`: the connection may run `pragma_table_list` (the read-only reader and a joined
/// writer; never the user's connection).
pub(crate) async fn read_schema(
    cx: &Cx<'_>,
    snap: &mut SnapshotTxn<'_>,
    v: Variant,
    level: Level,
    objects: Vec<ObjRow>,
    with_flags: bool,
) -> Result<SchemaRows, IntroError> {
    let schema = v.schema();
    let mut out = SchemaRows {
        objects,
        ..SchemaRows::default()
    };
    if with_flags {
        match rows(cx, snap, "TableFlags", &[("schema", schema)]).await {
            Ok(rs) => {
                out.flags = Some(
                    rs.iter()
                        .map(|r| {
                            (
                                name(r, 0),
                                TableFlags {
                                    ty: name(r, 1),
                                    without_rowid: flag(r, 2),
                                    strict: flag(r, 3),
                                },
                            )
                        })
                        .collect(),
                );
            }
            Err(e) if refused(&e) => {}
            Err(e) => return Err(e),
        }
    }
    if level < Level::Structure {
        return Ok(out);
    }
    out.structure = true;
    let relations = out
        .objects
        .iter()
        .filter(|o| matches!(&*o.ty, "table" | "view"))
        .count() as u64;
    cx.report(Phase::Columns, 0, Some(relations));

    // Columns: one query for tables and one for views. When one fails (a view over a dropped
    // table, a virtual table whose module is missing), only that kind goes per relation, and a
    // relation that still fails is kept broken with the engine's message (F10).
    for (ty, section) in [("table", "TableColumns"), ("view", "ViewColumns")] {
        let section = format!("{section}{}", v.suffix());
        match rows(cx, snap, &section, &[("schema", schema)]).await {
            Ok(rs) => out
                .columns
                .extend(rs.iter().map(|r| col_row(name(r, 0), r, 1))),
            Err(e) if refused(&e) => {
                let rels: Vec<Name> = out
                    .objects
                    .iter()
                    .filter(|o| &*o.ty == ty)
                    .map(|o| o.name.clone())
                    .collect();
                let n = rels.len() as u64;
                let mut failures = 0usize;
                for (k, rel) in rels.into_iter().enumerate() {
                    // The fallback counts its own relations, one query each.
                    cx.report(Phase::Columns, k as u64, Some(n));
                    if failures >= MAX_FALLBACK_FAILURES {
                        // Bounded work against a crafted file (review m3): the rest stays
                        // broken, unread, its children undecided.
                        out.broken.insert(rel, String::new());
                        continue;
                    }
                    match rows(
                        cx,
                        snap,
                        "RelationColumns",
                        &[("rel", &rel), ("schema", schema)],
                    )
                    .await
                    {
                        Ok(rs) => out
                            .columns
                            .extend(rs.iter().map(|r| col_row(rel.clone(), r, 0))),
                        Err(e) if refused(&e) => {
                            failures += 1;
                            out.broken.insert(rel, message(&e));
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
            Err(e) => return Err(e),
        }
    }

    cx.report(Phase::Columns, relations, Some(relations));
    cx.report(Phase::Indexes, 0, None);
    let indexes = rows(
        cx,
        snap,
        &format!("Indexes{}", v.suffix()),
        &[("schema", schema)],
    )
    .await;
    let parts = match &indexes {
        Ok(_) => Some(
            rows(
                cx,
                snap,
                &format!("IndexParts{}", v.suffix()),
                &[("schema", schema)],
            )
            .await,
        ),
        Err(_) => None,
    };
    match (indexes, parts) {
        (Ok(ix), Some(Ok(ps))) => {
            out.indexes = ix
                .iter()
                .map(|r| IdxRow {
                    rel: name(r, 0),
                    name: name(r, 1),
                    unique: flag(r, 2),
                    origin: name(r, 3),
                    partial: flag(r, 4),
                })
                .collect();
            out.parts = ps
                .iter()
                .map(|r| PartRow {
                    index: name(r, 0),
                    seqno: int(r, 1)
                        .and_then(|v| u32::try_from(v).ok())
                        .unwrap_or(u32::MAX),
                    cid: int(r, 2).unwrap_or(-1),
                    name: cell(r, 3).cloned(),
                    desc: flag(r, 4),
                    coll: cell(r, 5).cloned(),
                    key: flag(r, 6),
                })
                .collect();
        }
        (Err(e), _) | (_, Some(Err(e))) => {
            if !refused(&e) {
                return Err(e);
            }
            out.index_failed = Some(e.code());
        }
        (Ok(_), None) => unreachable!("the parts are read whenever the list is"),
    }

    cx.report(
        Phase::Indexes,
        out.indexes.len() as u64,
        Some(out.indexes.len() as u64),
    );
    cx.report(Phase::Constraints, 0, None);
    match rows(
        cx,
        snap,
        &format!("ForeignKeys{}", v.suffix()),
        &[("schema", schema)],
    )
    .await
    {
        Ok(rs) => {
            out.fks = rs
                .iter()
                .map(|r| FkRow {
                    rel: name(r, 0),
                    id: int(r, 1).and_then(|v| u32::try_from(v).ok()).unwrap_or(0),
                    seq: int(r, 2).and_then(|v| u32::try_from(v).ok()).unwrap_or(0),
                    ref_table: name(r, 3),
                    from: name(r, 4),
                    to: cell(r, 5).cloned(),
                    on_update: name(r, 6),
                    on_delete: name(r, 7),
                    match_kind: name(r, 8),
                })
                .collect();
        }
        Err(e) if refused(&e) => out.fk_failed = Some(e.code()),
        Err(e) => return Err(e),
    }
    cx.report(
        Phase::Constraints,
        out.fks.len() as u64,
        Some(out.fks.len() as u64),
    );
    Ok(out)
}
