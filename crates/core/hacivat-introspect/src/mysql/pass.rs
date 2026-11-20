// ## m4-t17-mysql
//! The MySQL and MariaDB passes over one namespace (HV-INTRO-01, -07, -11, -13), written
//! against [`SectionReader`] so the decision logic runs without a server in the unit tests.
//!
//! - **Full** (every pass on MariaDB, R-M4-10; MySQL when no usable watermark): `Tables`,
//!   `Views`, `Triggers`, `Routines`, `Events` at every level; `Columns`, `Statistics`,
//!   `TableConstraints`, `KeyColumnUsage`, `ReferentialConstraints`, `CheckConstraints` and
//!   `Parameters` at `Structure` and above; with the watermark, 13 bulk queries. One row per
//!   column or key part, grouped here; existence sets `Ok` for every kind the level reads.
//! - **Incremental** (MySQL 8.0+, R-M4-39): `W = UTC_TIMESTAMP(6)` is read inside the
//!   snapshot; every pass reads `(name, kind, marker)` of every relation, trigger, routine and
//!   event (the existence sets); an object is re-read when it is new, its marker differs from
//!   the stored one, or its marker is `>= W_prev − 1 s` (compared here, never in SQL). Views
//!   over a changed or dropped relation (`VIEW_TABLE_USAGE`, transitively; every view before
//!   8.0.13) and tables whose foreign keys reference one are re-read too. The re-read runs the
//!   full sections with `… IN :[names]` (chunks of 1,000, R-M4-3); child kinds report sets
//!   scoped to the re-read parents. A changed server identity, privilege hash, level, session
//!   time zone or more than [`INCREMENTAL_LIMIT`] changed objects make the pass `Full`.
//! - **Fragment**: the named objects (a child's key names its relation) re-read as above, with
//!   scoped existence sets; it carries no watermark (a fragment never covers the namespace).
//! - **System** (R-M4-44): relations, columns and routines of a system schema, with no state
//!   marks, times or definers.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use futures::future::BoxFuture;
use hacivat_meta::merge::{ChangedObject, ExistSet, Identity, NamespaceResult};
use hacivat_meta::props::Props;
use hacivat_meta::{
    Level, Name, NamespaceSnapshot, NsKey, NsKind, ObjectKey, ObjectKind, StateMark, Watermark,
};
use hacivat_proto::DbmsKind;
use hacivat_session::meta::Rows;
use hacivat_values::{BindValue, LogicalType};
use smallvec::smallvec;

use super::build::{
    RelationParts, Tree, event_tree, micros, raw_kind, relation_kind, relation_mark, relation_tree,
    routine_kind, routine_tree, trigger_stats, view_mark, view_marks_with_columns,
};
use super::rows::{
    self, ColumnRow, EventRow, ParamRow, RoutineRow, TableRow, TriggerRow, ViewRow, cell, name,
    number32, opt_name,
};
use super::server::{ServerCtx, parse_version};
use crate::assemble::mysql::{
    CheckClauseRow, CheckRow, FkColumnRow, FkRow, Flavor as AsmFlavor, KeyRows, StatRow,
    assemble_keys,
};
use crate::assemble::{ColumnIndex, KeyObj, TableId};
use crate::engine::{Phase, bounded_pushdown};
use crate::filter::CompiledFilter;
use crate::introspector::{IntroError, report_progress};
use crate::pack::{Mode, PackCtx, Params};

/// More changed objects than this make an incremental pass `Full` (one bulk query per kind is
/// then cheaper than chunked `IN` lists).
pub const INCREMENTAL_LIMIT: usize = 2_000;

/// The marker window of R-M4-39: a marker at or after `W_prev − 1 s` is re-read (MySQL times
/// have one-second granularity).
pub const WINDOW_MICROS: i64 = 1_000_000;

/// The system schemas (R-M4-44): never introspected live by default (R-M4-14).
pub const SYSTEM_SCHEMAS: [&str; 4] = ["information_schema", "mysql", "performance_schema", "sys"];

/// The relation kinds a `TABLES` row of a user schema holds, for `pushdown_kinds` of a
/// `TableNamesLike` filter (T12 review I2): tables and views, and MariaDB sequences. The other
/// `TABLE_TYPE` values (`SYSTEM VIEW` is a view; MariaDB `SYSTEM VERSIONED` a table;
/// `TEMPORARY` is listed only to the session that made it) never reach a metadata pass, so
/// no [`ObjectKind::Unknown`] relation is cut by a pushed pattern.
pub fn relation_kinds(srv: &ServerCtx) -> &'static [ObjectKind] {
    if srv.is_mariadb() {
        &[ObjectKind::Table, ObjectKind::View, ObjectKind::Sequence]
    } else {
        &[ObjectKind::Table, ObjectKind::View]
    }
}

/// The kinds a table's subtree holds (scoped to re-read relations).
const TABLE_CHILD_KINDS: [ObjectKind; 7] = [
    ObjectKind::Column,
    ObjectKind::Index,
    ObjectKind::PrimaryKey,
    ObjectKind::UniqueKey,
    ObjectKind::ForeignKey,
    ObjectKind::Check,
    ObjectKind::Trigger,
];

/// Reads one pack section: every chunk's rows, in order. The production reader renders the
/// built-in pack on a `SnapshotTxn` (`mysql::TxnReader`); tests answer from fixtures.
pub trait SectionReader: Send {
    fn read<'a>(
        &'a mut self,
        section: &'static str,
        ctx: PackCtx,
    ) -> BoxFuture<'a, Result<Rows, IntroError>>;
}

/// A text bind.
pub fn text(s: &str) -> BindValue {
    BindValue::Typed {
        hint: LogicalType::Text {
            max_chars: None,
            fixed: false,
        },
        text: Arc::from(s),
    }
}

/// `:schema`.
pub fn schema_params(schema: &str) -> Params {
    Params {
        scalars: vec![(Name::from("schema"), text(schema))],
        lists: Vec::new(),
    }
}

/// `:schema` and the list `:[names]`.
pub fn list_params<'a>(schema: &str, names: impl IntoIterator<Item = &'a Name>) -> Params {
    Params {
        scalars: vec![(Name::from("schema"), text(schema))],
        lists: vec![(
            Name::from("names"),
            names.into_iter().map(|n| text(n)).collect(),
        )],
    }
}

/// The server context: `Identity`, and `ServerUuid` on MySQL. `hint` (the profile's flavor)
/// only renders `Identity`, which holds no flavor gate; the flavor is `VERSION()`'s.
pub async fn read_server(
    r: &mut dyn SectionReader,
    hint: DbmsKind,
) -> Result<ServerCtx, IntroError> {
    let probe = ServerCtx::assumed(hint, "");
    let rows = r
        .read(
            "Identity",
            probe.pack_ctx(Mode::Full, Level::Structure, Params::default()),
        )
        .await?;
    let row = rows::checked("Identity", &rows, 10)?
        .first()
        .ok_or_else(|| IntroError::Protocol(Name::from("Identity")))?;
    let version_text = name(row, 0);
    let (version_num, flavor) = parse_version(&version_text);
    let utf8mb4 = (4..=6).all(|i| cell(row, i) == Some("utf8mb4"));
    let mut srv = ServerCtx {
        flavor,
        version_text,
        version_num,
        // Unknown reads as an out-of-range value: no fold rule is pinned for it (R-M4-41).
        lower_case_table_names: number32(row, 1)
            .and_then(|n| u8::try_from(n).ok())
            .unwrap_or(u8::MAX),
        sql_mode: name(row, 2),
        character_set_system: opt_name(row, 3),
        utf8mb4,
        utc_session: matches!(cell(row, 7), Some("+00:00" | "UTC")),
        server_uuid: None,
        login: opt_name(row, 8),
        role: opt_name(row, 9),
    };
    if !srv.is_mariadb() {
        let rows = r
            .read(
                "ServerUuid",
                srv.pack_ctx(Mode::Full, Level::Structure, Params::default()),
            )
            .await?;
        srv.server_uuid = rows::checked("ServerUuid", &rows, 1)?
            .first()
            .and_then(|r| opt_name(r, 0));
    }
    Ok(srv)
}

/// `@@server_id`, `@@hostname` and `@@port` (MariaDB's part of the server fingerprint).
pub async fn read_host_facts(
    r: &mut dyn SectionReader,
    srv: &ServerCtx,
) -> Result<super::server::HostFacts, IntroError> {
    let rows = r
        .read(
            "ServerIdentity",
            srv.pack_ctx(Mode::Full, Level::Structure, Params::default()),
        )
        .await?;
    let row = rows::checked("ServerIdentity", &rows, 3)?.first();
    Ok(super::server::HostFacts {
        server_id: row.and_then(|r| opt_name(r, 0)),
        host_name: row.and_then(|r| opt_name(r, 1)),
        port: row.and_then(|r| opt_name(r, 2)),
    })
}

/// `W`: `UTC_TIMESTAMP(6)` read inside the pass's snapshot (R-M4-39), microseconds.
pub async fn read_watermark(r: &mut dyn SectionReader, srv: &ServerCtx) -> Result<i64, IntroError> {
    let rows = r
        .read(
            "Watermark",
            srv.pack_ctx(Mode::Full, Level::Structure, Params::default()),
        )
        .await?;
    rows::checked("Watermark", &rows, 1)?
        .first()
        .and_then(|r| cell(r, 0))
        .and_then(rows::parse_time_micros)
        .ok_or_else(|| IntroError::Protocol(Name::from("Watermark")))
}

/// The privilege hash of the login on `schema` (alg/02 HV-INTRO-07 step 8): XXH3-64 of the
/// sorted `Privileges` rows with `CURRENT_USER()` and `CURRENT_ROLE()`. A grant or revoke
/// changes which objects and columns `INFORMATION_SCHEMA` shows, which no marker sees.
pub async fn privilege_hash(
    r: &mut dyn SectionReader,
    srv: &ServerCtx,
    schema: &str,
) -> Result<u64, IntroError> {
    let rows = r
        .read(
            "Privileges",
            srv.pack_ctx(Mode::Full, Level::Structure, schema_params(schema)),
        )
        .await?;
    let mut lines: Vec<String> = rows::checked("Privileges", &rows, 6)?
        .iter()
        .map(|r| {
            (0..6)
                .map(|i| cell(r, i).unwrap_or(""))
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect();
    lines.sort();
    lines.push(format!(
        "{}\t{}",
        srv.login.as_deref().unwrap_or(""),
        srv.role.as_deref().unwrap_or("")
    ));
    Ok(twox_hash::XxHash3_64::oneshot(lines.join("\n").as_bytes()))
}

/// The databases the login can see, and the default collation of each (`ServerFacts`).
pub async fn list_namespaces(
    r: &mut dyn SectionReader,
    srv: &ServerCtx,
) -> Result<Vec<(Name, Option<Name>)>, IntroError> {
    let rows = r
        .read(
            "ListNamespaces",
            srv.pack_ctx(Mode::Full, Level::Names, Params::default()),
        )
        .await?;
    let mut out: Vec<(Name, Option<Name>)> = rows::checked("ListNamespaces", &rows, 3)?
        .iter()
        .map(|r| (name(r, 0), opt_name(r, 2)))
        .filter(|(n, _)| !n.is_empty())
        .collect();
    out.sort();
    Ok(out)
}

/// What a pass did (the tests' query log; HV-INTRO-07 test 4).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassStats {
    /// Why the pass is `Full` (`None`: incremental or a fragment).
    pub full_reason: Option<&'static str>,
    /// The relations, routines (specific names) and events read with their full rows.
    pub relations: Vec<Name>,
    pub routines: Vec<Name>,
    pub events: Vec<Name>,
}

/// The objects to re-read, by top-level name.
#[derive(Debug, Clone, Default)]
struct Names {
    relations: BTreeSet<Name>,
    routines: BTreeSet<Name>,
    events: BTreeSet<Name>,
    /// Names asked by an `ObjectKind::Unknown` key: both families are read for them, so their
    /// `Unknown` objects are decided too (review m6).
    unknown: BTreeSet<Name>,
}

impl Names {
    fn len(&self) -> usize {
        self.relations.len() + self.routines.len() + self.events.len()
    }
}

/// What [`read_set`] restricts per family (`None`: every object of the family).
#[derive(Clone, Copy, Default)]
struct Lists<'a> {
    rel: Option<&'a BTreeSet<Name>>,
    rtn: Option<&'a BTreeSet<Name>>,
    evt: Option<&'a BTreeSet<Name>>,
}

impl<'a> Lists<'a> {
    fn of(n: &'a Names) -> Self {
        Lists {
            rel: Some(&n.relations),
            rtn: Some(&n.routines),
            evt: Some(&n.events),
        }
    }
}

/// How many distinct relations `tables` name (a progress count).
fn covered<'a>(tables: impl Iterator<Item = &'a Name>) -> u64 {
    u64::try_from(tables.collect::<HashSet<_>>().len()).unwrap_or(u64::MAX)
}

/// The rows one read returns.
#[derive(Debug, Default)]
struct ReadSet {
    tables: Vec<TableRow>,
    views: Vec<ViewRow>,
    /// The views' `COLUMNS` rows (`ViewColumnMarks`), for their marks (review I-1).
    view_cols: Vec<rows::ViewCol>,
    columns: Vec<ColumnRow>,
    keys: Option<KeyRows>,
    triggers: Vec<TriggerRow>,
    routines: Vec<RoutineRow>,
    params: Vec<ParamRow>,
    events: Vec<EventRow>,
}

/// Reads every object (`names = None`, `Mode::Full`) or the named ones (`Mode::Incremental`
/// with lists) at `level`.
async fn read_set(
    r: &mut dyn SectionReader,
    srv: &ServerCtx,
    schema: &str,
    level: Level,
    names: Lists<'_>,
) -> Result<ReadSet, IntroError> {
    // A family with a list reads its named objects (`mode == incremental` adds `… IN
    // :[names]`); one without reads all of them.
    let ctx = |which: Option<&BTreeSet<Name>>| {
        let (mode, params) = match which {
            Some(list) => (Mode::Incremental, list_params(schema, list)),
            None => (Mode::Full, schema_params(schema)),
        };
        srv.pack_ctx(mode, level, params)
    };
    let Lists { rel, rtn, evt } = names;
    let structure = level >= Level::Structure;
    let tables = rows::tables(&r.read("Tables", ctx(rel)).await?)?;
    let count = u64::try_from(tables.len()).unwrap_or(u64::MAX);
    let total = Some(count);
    report_progress(Phase::Tables, count, total);
    let mut set = ReadSet {
        tables,
        views: rows::views(&r.read("Views", ctx(rel)).await?)?,
        view_cols: rows::view_cols(&r.read("ViewColumnMarks", ctx(rel)).await?)?,
        triggers: rows::triggers(&r.read("Triggers", ctx(rel)).await?)?,
        ..ReadSet::default()
    };
    report_progress(Phase::Routines, 0, None);
    set.routines = rows::routines(&r.read("Routines", ctx(rtn)).await?)?;
    set.events = rows::events(&r.read("Events", ctx(evt)).await?)?;
    if structure {
        report_progress(Phase::Columns, 0, total);
        set.columns = rows::columns(&r.read("Columns", ctx(rel)).await?)?;
        report_progress(
            Phase::Columns,
            covered(set.columns.iter().map(|c| &c.table)),
            total,
        );
        set.params = rows::params(&r.read("Parameters", ctx(rtn)).await?)?;
        let mut keys = KeyRows::new(if srv.is_mariadb() {
            AsmFlavor::MariaDb
        } else {
            AsmFlavor::MySql
        });
        // ## m4-t17b-mysql-facets (named exception; T17b carry from CI 44649745390): on every
        // server of the matrix the index InnoDB creates for a named FK takes the constraint's
        // name (`fk_index table=c1 index=fk_named`), so such an index is the FK's.
        keys.fk_index = crate::assemble::mysql::FkIndexRule::NamedAfterConstraint;
        report_progress(Phase::Indexes, 0, total);
        keys.indexes = stat_rows(&r.read("Statistics", ctx(rel)).await?)?;
        report_progress(Phase::Constraints, 0, total);
        let tc = r.read("TableConstraints", ctx(rel)).await?;
        keys.checks = rows::checked("TableConstraints", &tc, 4)?
            .iter()
            .filter(|r| cell(r, 2) == Some("CHECK"))
            .map(|r| CheckRow {
                table: name(r, 0),
                name: name(r, 1),
                enforced: cell(r, 3) != Some("NO"),
            })
            .collect();
        keys.fk_columns = fk_column_rows(&r.read("KeyColumnUsage", ctx(rel)).await?)?;
        keys.foreign_keys = fk_rows(&r.read("ReferentialConstraints", ctx(rel)).await?)?;
        if srv.has_checks() {
            let cc = r.read("CheckConstraints", ctx(rel)).await?;
            keys.check_clauses = rows::checked("CheckConstraints", &cc, 3)?
                .iter()
                .map(|r| CheckClauseRow {
                    table: opt_name(r, 0),
                    name: name(r, 1),
                    clause: name(r, 2),
                })
                .collect();
        }
        set.keys = Some(keys);
    }
    Ok(set)
}

fn stat_rows(rows: &Rows) -> Result<Vec<StatRow>, IntroError> {
    Ok(rows::checked("Statistics", rows, 11)?
        .iter()
        .map(|r| StatRow {
            table: name(r, 0),
            index: name(r, 1),
            non_unique: number32(r, 2).unwrap_or(1) != 0,
            seq_in_index: number32(r, 3).unwrap_or(0),
            column: opt_name(r, 4),
            collation: opt_name(r, 5),
            sub_part: number32(r, 6),
            index_type: opt_name(r, 7),
            comment: opt_name(r, 8),
            visible: cell(r, 9) != Some("NO"),
            expression: opt_name(r, 10),
        })
        .collect())
}

fn fk_column_rows(rows: &Rows) -> Result<Vec<FkColumnRow>, IntroError> {
    Ok(rows::checked("KeyColumnUsage", rows, 7)?
        .iter()
        .filter(|r| cell(r, 5).is_some())
        .map(|r| FkColumnRow {
            table: name(r, 0),
            constraint: name(r, 1),
            column: name(r, 2),
            ordinal_position: number32(r, 3).unwrap_or(0),
            referenced_column: opt_name(r, 6),
        })
        .collect())
}

fn fk_rows(rows: &Rows) -> Result<Vec<FkRow>, IntroError> {
    Ok(rows::checked("ReferentialConstraints", rows, 8)?
        .iter()
        .map(|r| FkRow {
            table: name(r, 0),
            name: name(r, 1),
            referenced_schema: name(r, 2),
            referenced_table: name(r, 3),
            unique_constraint_name: opt_name(r, 4),
            match_option: name(r, 5),
            update_rule: name(r, 6),
            delete_rule: name(r, 7),
        })
        .collect())
}

/// The relations, routines and events built from one read.
#[derive(Debug, Default)]
struct Built {
    relations: Vec<Tree>,
    routines: Vec<Tree>,
    events: Vec<Tree>,
}

/// Whether `kind` (a `TABLES` row's) contributes columns to the FK column index.
fn indexed(kind: ObjectKind) -> bool {
    matches!(kind, ObjectKind::Table | ObjectKind::Unknown)
}

/// Columns per relation, of the listed relations only (edge 4: a row of an unlisted relation
/// is skipped).
fn columns_by_table(set: &ReadSet) -> HashMap<Name, Vec<&ColumnRow>> {
    let listed: HashSet<&Name> = set.tables.iter().map(|t| &t.name).collect();
    let mut out: HashMap<Name, Vec<&ColumnRow>> = HashMap::new();
    for c in &set.columns {
        if listed.contains(&c.table) {
            out.entry(c.table.clone()).or_default().push(c);
        }
    }
    out
}

/// Adds the read relations' columns to `index`.
fn index_read_tables(index: &mut ColumnIndex, set: &ReadSet, srv: &ServerCtx) {
    let by_table = columns_by_table(set);
    for t in &set.tables {
        if !indexed(relation_kind(&t.table_type, srv.is_mariadb())) {
            continue;
        }
        let mut cols: Vec<(u32, Name)> = by_table
            .get(&t.name)
            .map(|v| v.iter().map(|c| (c.ordinal, c.name.clone())).collect())
            .unwrap_or_default();
        cols.sort();
        index.add_table(TableId::named(t.name.clone()), cols);
    }
}

/// Builds the trees of one read. `index` holds the columns FK targets resolve against.
fn build(set: ReadSet, srv: &ServerCtx, level: Level, index: &ColumnIndex) -> Built {
    let structure = level >= Level::Structure;
    let views: HashMap<&Name, &ViewRow> = set.views.iter().map(|v| (&v.name, v)).collect();
    let mut view_cols: HashMap<&Name, Vec<&rows::ViewCol>> = HashMap::new();
    for c in &set.view_cols {
        view_cols.entry(&c.view).or_default().push(c);
    }
    let cols = columns_by_table(&set);
    let stats = trigger_stats(&set.triggers);
    let mut trigs: HashMap<&Name, Vec<&TriggerRow>> = HashMap::new();
    for t in &set.triggers {
        trigs.entry(&t.table).or_default().push(t);
    }
    let mut keys: HashMap<Name, Vec<KeyObj>> = HashMap::new();
    if let Some(k) = set.keys.clone() {
        for o in assemble_keys(k, index) {
            keys.entry(o.table.name.clone()).or_default().push(o);
        }
    }
    let mut out = Built::default();
    let mut tables: Vec<&TableRow> = set.tables.iter().collect();
    tables.sort_by(|a, b| a.name.cmp(&b.name));
    for t in tables {
        let view = views.get(&t.name).copied();
        out.relations.push(relation_tree(
            RelationParts {
                table: t,
                view,
                view_hash: view.map(|v| {
                    view_mark(
                        v.hash,
                        v.def_visible,
                        view_cols.get(&t.name).map(Vec::as_slice).unwrap_or(&[]),
                    )
                }),
                columns: cols.get(&t.name).map(Vec::as_slice).unwrap_or(&[]),
                keys: keys.remove(&t.name).unwrap_or_default(),
                triggers: trigs.get(&t.name).map(Vec::as_slice).unwrap_or(&[]),
                trigger_stats: stats.get(&t.name),
                children: structure,
                system: false,
            },
            srv,
        ));
    }
    let mut params: HashMap<(&Name, &Name), Vec<&ParamRow>> = HashMap::new();
    for p in &set.params {
        params
            .entry((&p.routine_type, &p.specific_name))
            .or_default()
            .push(p);
    }
    let mut routines: Vec<&RoutineRow> = set.routines.iter().collect();
    routines.sort_by(|a, b| (&a.name, &a.routine_type).cmp(&(&b.name, &b.routine_type)));
    for r in routines {
        let ps = params
            .get(&(&r.routine_type, &r.specific_name))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        out.routines.push(routine_tree(r, ps, structure, false));
    }
    let mut events: Vec<&EventRow> = set.events.iter().collect();
    events.sort_by(|a, b| a.name.cmp(&b.name));
    out.events = events.into_iter().map(event_tree).collect();
    out
}

/// The key of the namespace root (the parent of every top-level changed object).
fn root_key(schema: &Name) -> ObjectKey {
    ObjectKey {
        kind: ObjectKind::Schema,
        path: smallvec![schema.clone()],
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    }
}

/// The top-level kinds a pass of `srv` decides at any level.
fn top_kinds(srv: &ServerCtx) -> Vec<ObjectKind> {
    let mut k = vec![
        ObjectKind::Table,
        ObjectKind::View,
        ObjectKind::Unknown,
        ObjectKind::Routine,
        ObjectKind::Procedure,
        ObjectKind::Event,
    ];
    if srv.is_mariadb() {
        k.push(ObjectKind::Sequence);
    }
    k
}

/// A full pass's result: every tree changed, every read kind's set `Ok`.
fn full_result(
    built: Built,
    schema: &Name,
    srv: &ServerCtx,
    level: Level,
    watermark: Watermark,
    w: i64,
) -> NamespaceResult {
    let mut ids: BTreeMap<ObjectKind, Vec<Identity>> = BTreeMap::new();
    let mut kinds = top_kinds(srv);
    if level >= Level::Structure {
        kinds.extend(TABLE_CHILD_KINDS);
        kinds.push(ObjectKind::Argument);
    }
    for k in &kinds {
        ids.insert(*k, Vec::new());
    }
    let mut changed = Vec::new();
    let parent = root_key(schema);
    for t in built
        .relations
        .into_iter()
        .chain(built.routines)
        .chain(built.events)
    {
        t.identities(&mut ids);
        changed.push(ChangedObject {
            parent: parent.clone(),
            subtree: t.subtree(),
        });
    }
    ids.retain(|k, _| kinds.contains(k));
    NamespaceResult {
        existent: ids.into_iter().map(|(k, v)| (k, ExistSet::Ok(v))).collect(),
        changed,
        categories_to_reload: BTreeSet::new(),
        new_watermark: watermark,
        full: true,
        level,
        read_at_ms: w / 1_000,
    }
}

fn my_watermark(srv: &ServerCtx, w: i64, priv_hash: u64) -> Watermark {
    if srv.is_mariadb() {
        // R-M4-10: no MariaDB marker in v1; every pass is full.
        return Watermark::None;
    }
    Watermark::MyTime {
        at_utc_micros: w,
        server_uuid: srv.server_uuid.clone().unwrap_or_else(|| Name::from("")),
        version: srv.version_text.clone(),
        priv_hash,
    }
}

/// Why `prev` cannot be the base of an incremental pass (`None`: it can).
fn incremental_blocker(
    srv: &ServerCtx,
    level: Level,
    prev: Option<&NamespaceSnapshot>,
    priv_hash: u64,
) -> Option<&'static str> {
    if !srv.incremental_supported() {
        return Some("flavor");
    }
    if !srv.utc_session {
        return Some("time zone");
    }
    let Some(prev) = prev else {
        return Some("no previous snapshot");
    };
    if level < Level::Structure || prev.meta().level != level {
        return Some("level");
    }
    match &prev.meta().watermark {
        Watermark::MyTime {
            server_uuid,
            version,
            priv_hash: ph,
            ..
        } => {
            if Some(server_uuid) != srv.server_uuid.as_ref() || *version != srv.version_text {
                Some("server identity")
            } else if *ph != priv_hash {
                Some("privileges")
            } else {
                None
            }
        }
        _ => Some("no watermark"),
    }
}

/// One pass over `ns` (see the module docs). `ns` is a MySQL schema namespace `[schema]`.
pub async fn run_pass(
    r: &mut dyn SectionReader,
    srv: &ServerCtx,
    ns: &NsKey,
    mode: Mode,
    level: Level,
    filter: &CompiledFilter,
    prev: Option<&NamespaceSnapshot>,
) -> Result<(NamespaceResult, PassStats), IntroError> {
    let schema = schema_of(ns)?;
    if mode == Mode::System {
        return system_pass(r, srv, &schema, level).await;
    }
    let level = if mode == Mode::Brief {
        Level::Names
    } else {
        level
    };
    let w = read_watermark(r, srv).await?;
    let priv_hash = if srv.incremental_supported() {
        privilege_hash(r, srv, &schema).await?
    } else {
        0
    };
    let watermark = my_watermark(srv, w, priv_hash);
    // R-W3-2: the relations the scope filter may keep, pushed to the server (≤ 16 runs).
    let pushed = pushed_relations(r, srv, &schema, filter).await?;
    let at = At {
        w,
        watermark,
        pushed: pushed.as_ref(),
    };
    let blocker = if mode == Mode::Incremental {
        incremental_blocker(srv, level, prev, priv_hash)
    } else {
        Some("requested")
    };
    match (blocker, prev) {
        (None, Some(prev)) => incremental_pass(r, srv, &schema, level, prev, at).await,
        (reason, _) => full_pass(r, srv, &schema, level, at, reason.unwrap_or("requested")).await,
    }
}

/// The schema of a MySQL schema namespace.
pub fn schema_of(ns: &NsKey) -> Result<Name, IntroError> {
    match (ns.kind, ns.path.as_slice()) {
        (NsKind::Schema, [s]) => Ok(s.clone()),
        _ => Err(IntroError::Protocol(Name::from("namespace"))),
    }
}

/// When and under what filter a pass reads.
#[derive(Clone)]
struct At<'a> {
    /// `W`, microseconds.
    w: i64,
    /// The watermark the pass will store.
    watermark: Watermark,
    /// The relations a pushed filter keeps (`None`: no predicate was pushed).
    pushed: Option<&'a BTreeSet<Name>>,
}

async fn full_pass(
    r: &mut dyn SectionReader,
    srv: &ServerCtx,
    schema: &Name,
    level: Level,
    at: At<'_>,
    reason: &'static str,
) -> Result<(NamespaceResult, PassStats), IntroError> {
    let At {
        w,
        watermark,
        pushed,
    } = at;
    let lists = Lists {
        rel: pushed,
        ..Lists::default()
    };
    let set = read_set(r, srv, schema, level, lists).await?;
    let mut index = ColumnIndex::new(NsKey::schema([schema.clone()]), srv.dialect(), srv.ident());
    index_read_tables(&mut index, &set, srv);
    index.set_complete(true);
    let stats = PassStats {
        full_reason: Some(reason),
        relations: set.tables.iter().map(|t| t.name.clone()).collect(),
        routines: set
            .routines
            .iter()
            .map(|t| t.specific_name.clone())
            .collect(),
        events: set.events.iter().map(|t| t.name.clone()).collect(),
    };
    let built = build(set, srv, level, &index);
    Ok((full_result(built, schema, srv, level, watermark, w), stats))
}

/// The top-level objects of `prev`, by `(kind, raw kind, name)`.
type TopKey = (ObjectKind, Option<Name>, Name);

fn prev_top(prev: &NamespaceSnapshot) -> HashMap<TopKey, (u32, StateMark)> {
    prev.children(0)
        .map(|(i, o)| {
            (
                (o.kind(), o.raw_kind().cloned(), o.name().clone()),
                (i, o.state()),
            )
        })
        .collect()
}

/// R-M4-39's window: a time component of `mark` at or after `w_prev − 1 s`.
pub fn in_window(mark: &StateMark, w_prev: i64) -> bool {
    let from = w_prev.saturating_sub(WINDOW_MICROS);
    match *mark {
        StateMark::MyTable {
            created,
            trigger_max,
            ..
        } => created >= from || (trigger_max > 0 && trigger_max >= from),
        StateMark::Modified(t) | StateMark::Created(t) => t >= from,
        _ => false,
    }
}

/// Whether an object whose fresh mark is `fresh` is re-read against its stored mark (R-M4-39:
/// new, a different marker, or a marker in the window; a missing time is always re-read).
pub fn reread(stored: Option<&StateMark>, fresh: &StateMark, has_time: bool, w_prev: i64) -> bool {
    match stored {
        None => true,
        Some(s) => s != fresh || !has_time || in_window(fresh, w_prev),
    }
}

async fn incremental_pass(
    r: &mut dyn SectionReader,
    srv: &ServerCtx,
    schema: &Name,
    level: Level,
    prev: &NamespaceSnapshot,
    at: At<'_>,
) -> Result<(NamespaceResult, PassStats), IntroError> {
    let Watermark::MyTime {
        at_utc_micros: w_prev,
        ..
    } = prev.meta().watermark
    else {
        return full_pass(r, srv, schema, level, at, "no watermark").await;
    };
    let ctx = || srv.pack_ctx(Mode::Full, level, schema_params(schema));
    let mut tables = rows::table_marks(&r.read("TableMarks", ctx()).await?)?;
    if let Some(keep) = at.pushed {
        // The pushed predicate decides which relations this pass reads at all (R-W3-2).
        tables.retain(|t| keep.contains(&t.name));
    }
    // A view's mark folds in its columns, so a change under it moves it even when its base is
    // outside the pushed filter or another schema, or its definition is hidden (I-1, I-4).
    let views = view_marks_with_columns(
        rows::view_marks(&r.read("ViewMarks", ctx()).await?)?,
        &rows::view_cols(&r.read("ViewColumnMarks", ctx()).await?)?,
    );
    let trigger_marks = rows::trigger_marks(&r.read("TriggerMarks", ctx()).await?)?;
    let routines = rows::routine_marks(&r.read("RoutineMarks", ctx()).await?)?;
    let events = rows::event_marks(&r.read("EventMarks", ctx()).await?)?;
    let tstats = trigger_stats(&trigger_marks);
    let top = prev_top(prev);

    // Existence of every top-level object, with its fresh mark; what changed.
    let mut ids: BTreeMap<ObjectKind, Vec<Identity>> = BTreeMap::new();
    for k in top_kinds(srv) {
        ids.insert(k, Vec::new());
    }
    let mut want = Names::default();
    let mut definition_changed: BTreeSet<Name> = BTreeSet::new();
    let mut listed: HashSet<Name> = HashSet::new();
    for t in &tables {
        let kind = relation_kind(&t.table_type, srv.is_mariadb());
        let raw = raw_kind(kind, &t.table_type);
        let mark = relation_mark(
            kind,
            t.create_time.as_ref(),
            views.get(&t.name).map(|(m, _)| *m),
            tstats.get(&t.name),
        );
        // A view whose definition this login cannot read (no SHOW VIEW) may change without a
        // column moving: it is re-read by every pass (review I-1).
        let has_time = if kind == ObjectKind::View {
            views.get(&t.name).is_some_and(|(_, visible)| *visible)
        } else {
            micros(t.create_time.as_ref()).is_some()
        };
        let stored = top
            .get(&(kind, raw.clone(), t.name.clone()))
            .map(|(_, m)| m);
        if reread(stored, &mark, has_time, w_prev) {
            want.relations.insert(t.name.clone());
            definition_changed.insert(t.name.clone());
        }
        listed.insert(t.name.clone());
        ids.entry(kind).or_default().push(Identity {
            path: vec![t.name.clone()],
            server_id: None,
            overload_sig: None,
            raw_kind: raw,
            state: mark,
        });
    }
    // Relations the cache holds that the server no longer lists.
    for (kind, _, n) in top.keys() {
        if matches!(
            kind,
            ObjectKind::Table | ObjectKind::View | ObjectKind::Sequence
        ) && !listed.contains(n)
        {
            definition_changed.insert(n.clone());
        }
    }
    // Views over a changed or dropped relation, transitively (their columns follow the base,
    // HV-INTRO-07 step 7); every view when the server has no `VIEW_TABLE_USAGE`.
    if !definition_changed.is_empty() && !views.is_empty() {
        if srv.has_view_usage() {
            let usage = rows::view_usage(&r.read("ViewUsage", ctx()).await?)?;
            let mut dirty = definition_changed.clone();
            loop {
                let before = dirty.len();
                for (v, ts, tn) in &usage {
                    if **ts == **schema && dirty.contains(tn) {
                        dirty.insert(v.clone());
                    }
                }
                if dirty.len() == before {
                    break;
                }
            }
            for v in dirty {
                if views.contains_key(&v) {
                    want.relations.insert(v);
                }
            }
        } else {
            want.relations.extend(views.keys().cloned());
        }
    }
    // Tables whose foreign keys reference a changed or dropped table (the FK's referenced
    // columns and index follow the target).
    for ((kind, _, n), (idx, _)) in &top {
        if *kind != ObjectKind::Table || !listed.contains(n) {
            continue;
        }
        let refers = prev.children(*idx).any(|(_, o)| match o.props() {
            Props::ForeignKey(f) => {
                f.target.path.len() == 2
                    && *f.target.path[0] == **schema
                    && definition_changed.contains(&f.target.path[1])
            }
            _ => false,
        });
        if refers {
            want.relations.insert(n.clone());
        }
    }
    for (spec, ty, last) in &routines {
        let kind = routine_kind(ty);
        let raw = raw_kind(kind, ty);
        let mark = micros(last.as_ref()).map_or(StateMark::None, StateMark::Modified);
        let stored = top.get(&(kind, raw.clone(), spec.clone())).map(|(_, m)| m);
        if reread(stored, &mark, micros(last.as_ref()).is_some(), w_prev) {
            want.routines.insert(spec.clone());
        }
        ids.entry(kind).or_default().push(Identity {
            path: vec![spec.clone()],
            server_id: None,
            overload_sig: None,
            raw_kind: raw,
            state: mark,
        });
    }
    for e in &events {
        let mark = micros(e.last_altered.as_ref()).map_or(StateMark::None, StateMark::Modified);
        let stored = top
            .get(&(ObjectKind::Event, None, e.name.clone()))
            .map(|(_, m)| m);
        if reread(
            stored,
            &mark,
            micros(e.last_altered.as_ref()).is_some(),
            w_prev,
        ) {
            want.events.insert(e.name.clone());
        }
        ids.entry(ObjectKind::Event).or_default().push(Identity {
            path: vec![e.name.clone()],
            server_id: None,
            overload_sig: None,
            raw_kind: None,
            state: mark,
        });
    }
    if want.len() > INCREMENTAL_LIMIT {
        return full_pass(r, srv, schema, level, at, "too many changes").await;
    }

    // The re-read, with FK targets resolving against the fresh rows of re-read tables and the
    // cached columns of every other table the server still lists (as a full pass would).
    let set = read_set(r, srv, schema, level, Lists::of(&want)).await?;
    let mut index = ColumnIndex::new(NsKey::schema([schema.clone()]), srv.dialect(), srv.ident());
    index_read_tables(&mut index, &set, srv);
    let reread_names: HashSet<&Name> = set.tables.iter().map(|t| &t.name).collect();
    for ((kind, _, n), (idx, _)) in &top {
        if !indexed(*kind) || reread_names.contains(n) || !listed.contains(n) {
            continue;
        }
        let mut cols: Vec<(u32, Name)> = prev
            .children(*idx)
            .filter_map(|(_, o)| match o.props() {
                Props::Column(c) if o.kind() == ObjectKind::Column => {
                    Some((c.ordinal, o.name().clone()))
                }
                _ => None,
            })
            .collect();
        cols.sort();
        index.add_table(TableId::named(n.clone()), cols);
    }
    index.set_complete(true);
    let stats = PassStats {
        full_reason: None,
        relations: set.tables.iter().map(|t| t.name.clone()).collect(),
        routines: set
            .routines
            .iter()
            .map(|t| t.specific_name.clone())
            .collect(),
        events: set.events.iter().map(|t| t.name.clone()).collect(),
    };
    let built = build(set, srv, level, &index);
    let result = changed_result(built, schema, &want, ids, level, at.watermark, at.w, false);
    Ok((result, stats))
}

/// The result of a re-read (incremental or fragment): the re-read trees changed, `top_ids` as
/// given, child kinds scoped to the re-read parents.
#[allow(clippy::too_many_arguments, reason = "one call site per pass kind")]
fn changed_result(
    built: Built,
    schema: &Name,
    want: &Names,
    mut top_ids: BTreeMap<ObjectKind, Vec<Identity>>,
    level: Level,
    watermark: Watermark,
    w: i64,
    fragment: bool,
) -> NamespaceResult {
    let mut child_ids: BTreeMap<ObjectKind, Vec<Identity>> = BTreeMap::new();
    let mut rel_scope: Vec<Vec<Name>> = Vec::new();
    let mut rtn_scope: Vec<Vec<Name>> = Vec::new();
    let mut changed = Vec::new();
    let parent = root_key(schema);
    let mut fresh_top: BTreeMap<ObjectKind, Vec<Identity>> = BTreeMap::new();
    for (t, routine) in built
        .relations
        .into_iter()
        .map(|t| (t, false))
        .chain(built.routines.into_iter().map(|t| (t, true)))
        .chain(built.events.into_iter().map(|t| (t, false)))
    {
        let mut all = BTreeMap::new();
        t.identities(&mut all);
        for (k, v) in all {
            let into = if v.first().is_some_and(|i| i.path.len() == 1) {
                &mut fresh_top
            } else {
                &mut child_ids
            };
            into.entry(k).or_default().extend(v);
        }
        if routine {
            rtn_scope.push(vec![t.name().clone()]);
        } else if t.kind() != ObjectKind::Event {
            rel_scope.push(vec![t.name().clone()]);
        }
        changed.push(ChangedObject {
            parent: parent.clone(),
            subtree: t.subtree(),
        });
    }
    let mut existent: BTreeMap<ObjectKind, ExistSet> = BTreeMap::new();
    if fragment {
        // Scoped to what was asked: an asked object the read did not return is gone.
        let scope = |set: &BTreeSet<Name>| -> Vec<Vec<Name>> {
            set.iter().map(|n| vec![n.clone()]).collect()
        };
        for (kinds, asked) in [
            (
                &[ObjectKind::Table, ObjectKind::View, ObjectKind::Sequence][..],
                &want.relations,
            ),
            (
                &[ObjectKind::Routine, ObjectKind::Procedure][..],
                &want.routines,
            ),
            (&[ObjectKind::Event][..], &want.events),
            (&[ObjectKind::Unknown][..], &want.unknown),
        ] {
            if asked.is_empty() {
                continue;
            }
            for k in kinds {
                existent.insert(
                    *k,
                    ExistSet::Scoped {
                        scope: scope(asked),
                        ids: fresh_top.remove(k).unwrap_or_default(),
                    },
                );
            }
        }
    } else {
        for (k, v) in std::mem::take(&mut top_ids) {
            existent.insert(k, ExistSet::Ok(v));
        }
    }
    if level >= Level::Structure {
        for k in TABLE_CHILD_KINDS {
            existent.insert(
                k,
                ExistSet::Scoped {
                    scope: rel_scope.clone(),
                    ids: child_ids.remove(&k).unwrap_or_default(),
                },
            );
        }
        existent.insert(
            ObjectKind::Argument,
            ExistSet::Scoped {
                scope: rtn_scope,
                ids: child_ids.remove(&ObjectKind::Argument).unwrap_or_default(),
            },
        );
    }
    NamespaceResult {
        existent,
        changed,
        categories_to_reload: BTreeSet::new(),
        new_watermark: watermark,
        full: false,
        level,
        read_at_ms: w / 1_000,
    }
}

/// The top-level names `keys` of `schema` ask for (a child's key names its relation or
/// routine; keys of other namespaces are ignored).
fn names_of(schema: &Name, keys: &[ObjectKey]) -> Names {
    let mut out = Names::default();
    for k in keys {
        if k.path.first() != Some(schema) || k.path.len() < 2 {
            continue;
        }
        let top = k.path[1].clone();
        match (k.kind, k.path.len()) {
            (ObjectKind::Routine | ObjectKind::Procedure, 2) | (ObjectKind::Argument, 3) => {
                out.routines.insert(top);
            }
            (ObjectKind::Event, 2) => {
                out.events.insert(top);
            }
            (ObjectKind::Unknown, 2) => {
                // A relation or a routine of a kind this build does not model: read both.
                out.relations.insert(top.clone());
                out.routines.insert(top.clone());
                out.unknown.insert(top);
            }
            _ => {
                out.relations.insert(top);
            }
        }
    }
    out
}

/// A fragment: the objects `keys` name, re-read at `level` (module docs).
pub async fn run_fragment(
    r: &mut dyn SectionReader,
    srv: &ServerCtx,
    ns: &NsKey,
    keys: &[ObjectKey],
    level: Level,
) -> Result<(NamespaceResult, PassStats), IntroError> {
    let schema = schema_of(ns)?;
    let w = read_watermark(r, srv).await?;
    let want = names_of(&schema, keys);
    let set = read_set(r, srv, &schema, level, Lists::of(&want)).await?;
    let mut index = ColumnIndex::new(NsKey::schema([schema.clone()]), srv.dialect(), srv.ident());
    index_read_tables(&mut index, &set, srv);
    let stats = PassStats {
        full_reason: None,
        relations: set.tables.iter().map(|t| t.name.clone()).collect(),
        routines: set
            .routines
            .iter()
            .map(|t| t.specific_name.clone())
            .collect(),
        events: set.events.iter().map(|t| t.name.clone()).collect(),
    };
    let built = build(set, srv, level, &index);
    // No watermark: a fragment never covers the namespace (the engine keeps the stored one,
    // or the next pass is `Full`).
    let result = changed_result(
        built,
        &schema,
        &want,
        BTreeMap::new(),
        level,
        Watermark::None,
        w,
        true,
    );
    Ok((result, stats))
}

/// A system schema's relations, columns and routines (R-M4-44): no marks, no definers.
async fn system_pass(
    r: &mut dyn SectionReader,
    srv: &ServerCtx,
    schema: &Name,
    level: Level,
) -> Result<(NamespaceResult, PassStats), IntroError> {
    let ctx = || srv.pack_ctx(Mode::System, level, schema_params(schema));
    let rel = r.read("SystemRelations", ctx()).await?;
    let tables: Vec<TableRow> = rows::checked("SystemRelations", &rel, 2)?
        .iter()
        .map(|r| TableRow {
            name: name(r, 0),
            table_type: name(r, 1),
            engine: None,
            collation: None,
            options: None,
            comment: None,
            create_time: None,
            rows: None,
        })
        .collect();
    let col = r.read("SystemColumns", ctx()).await?;
    let columns: Vec<ColumnRow> = rows::checked("SystemColumns", &col, 5)?
        .iter()
        .map(|r| ColumnRow {
            table: name(r, 0),
            name: name(r, 1),
            ordinal: number32(r, 2).unwrap_or(0),
            default: None,
            nullable: rows::yes(r, 4),
            data_type: Name::from(""),
            column_type: name(r, 3),
            charset: None,
            collation: None,
            extra: None,
            comment: None,
            generation_expression: None,
        })
        .collect();
    let rtn = r.read("SystemRoutines", ctx()).await?;
    let routines: Vec<RoutineRow> = rows::checked("SystemRoutines", &rtn, 4)?
        .iter()
        .map(|r| RoutineRow {
            specific_name: name(r, 0),
            name: name(r, 1),
            routine_type: name(r, 2),
            returns: opt_name(r, 3),
            deterministic: false,
            data_access: None,
            security_type: None,
            definer: None,
            comment: None,
            last_altered: None,
        })
        .collect();
    let params = rows::params(&r.read("SystemParameters", ctx()).await?)?;
    let set = ReadSet {
        tables,
        columns,
        routines,
        params,
        ..ReadSet::default()
    };
    let cols = columns_by_table(&set);
    let mut built = Built::default();
    let mut tables: Vec<&TableRow> = set.tables.iter().collect();
    tables.sort_by(|a, b| a.name.cmp(&b.name));
    for t in tables {
        built.relations.push(relation_tree(
            RelationParts {
                table: t,
                view: None,
                view_hash: None,
                columns: cols.get(&t.name).map(Vec::as_slice).unwrap_or(&[]),
                keys: Vec::new(),
                triggers: &[],
                trigger_stats: None,
                children: true,
                system: true,
            },
            srv,
        ));
    }
    let mut params: HashMap<(&Name, &Name), Vec<&ParamRow>> = HashMap::new();
    for p in &set.params {
        params
            .entry((&p.routine_type, &p.specific_name))
            .or_default()
            .push(p);
    }
    let mut routines: Vec<&RoutineRow> = set.routines.iter().collect();
    routines.sort_by(|a, b| (&a.name, &a.routine_type).cmp(&(&b.name, &b.routine_type)));
    for r in routines {
        let ps = params
            .get(&(&r.routine_type, &r.specific_name))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        built.routines.push(routine_tree(r, ps, true, true));
    }
    let stats = PassStats {
        full_reason: Some("system"),
        relations: set.tables.iter().map(|t| t.name.clone()).collect(),
        routines: set
            .routines
            .iter()
            .map(|t| t.specific_name.clone())
            .collect(),
        events: Vec::new(),
    };
    let mut result = full_result(built, schema, srv, Level::Structure, Watermark::None, 0);
    // The system kinds have no events, keys or triggers to decide.
    result.existent.retain(|k, _| {
        matches!(
            k,
            ObjectKind::Table
                | ObjectKind::View
                | ObjectKind::Unknown
                | ObjectKind::Routine
                | ObjectKind::Procedure
                | ObjectKind::Column
                | ObjectKind::Argument
        )
    });
    Ok((result, stats))
}

/// Scope and filter pushdown (T12, ruling R-W3-2): the relation names of `schema` the
/// introspect filter's patterns select on the server, through `TableNamesLike` (`LIKE ?
/// ESCAPE ?`, one query per pattern, unioned; R-W2-10). `TABLES` holds every relation kind, so
/// the predicate is `engine::bounded_pushdown` (≤ 16 runs) over [`relation_kinds`] (T12 review
/// I2): `None` when it would take more runs, or when the
/// filter cannot be pushed for every kind, and the client then filters everything. Under
/// `lower_case_table_names = 1` the pattern is already lower-cased by T12 (given the ident).
/// The pass then reads the selected relations with `… IN :[names]`.
pub async fn pushed_relations(
    r: &mut dyn SectionReader,
    srv: &ServerCtx,
    schema: &str,
    filter: &crate::filter::CompiledFilter,
) -> Result<Option<BTreeSet<Name>>, IntroError> {
    let Some(pred) = bounded_pushdown(filter, relation_kinds(srv), srv.dialect(), &srv.ident())
    else {
        return Ok(None);
    };
    let mut out = BTreeSet::new();
    for run in pred.runs() {
        let mut params = schema_params(schema);
        params.scalars.extend(run);
        let rows = r
            .read(
                "TableNamesLike",
                srv.pack_ctx(Mode::Full, Level::Names, params),
            )
            .await?;
        for row in rows::checked("TableNamesLike", &rows, 2)? {
            out.insert(name(row, 0));
        }
    }
    Ok(Some(out))
}

/// One variable name of the server (`ListSettings`): MariaDB adds its scope and type; values
/// and comments are never read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setting {
    pub name: Name,
    pub scope: Option<Name>,
    pub ty: Option<Name>,
}

/// The server's variable names (R-M4-44, the minicat `ListSettings`), sorted.
pub async fn list_settings(
    r: &mut dyn SectionReader,
    srv: &ServerCtx,
) -> Result<Vec<Setting>, IntroError> {
    let rows = r
        .read(
            "ListSettings",
            srv.pack_ctx(Mode::System, Level::Names, Params::default()),
        )
        .await?;
    let mut out: Vec<Setting> = rows::checked("ListSettings", &rows, 3)?
        .iter()
        .map(|r| Setting {
            name: name(r, 0),
            scope: opt_name(r, 1),
            ty: opt_name(r, 2),
        })
        .filter(|s| !s.name.is_empty())
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}
