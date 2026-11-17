//! The MySQL 8.0/8.4 and MariaDB 10.6/10.11/11.4 introspector (HV-INTRO-01, -04 shape, -07,
//! -11, -13, -14, -15) over `INFORMATION_SCHEMA`.
//!
//! - Every per-kind query is bounded by the namespace (`… WHERE TABLE_SCHEMA = ?`) and returns
//!   one row per column / key part, grouped client-side (HV-INTRO-13: no `GROUP_CONCAT`, so no
//!   `group_concat_max_len` truncation; no N+1 queries).
//! - Names are bound, never spliced or folded (HV-INTRO-14 rule 1); catalog names are spelled
//!   canonically, upper case (rule 2).
//! - Incremental (MySQL only, HV-INTRO-07): one marker query per kind, then the full rows of
//!   the changed objects only (`… AND TABLE_NAME IN (?, …)`). MariaDB always runs a full pass
//!   (design spec §4: the MariaDB marker is v1.x); its `CREATE_TIME` behaviour is recorded by
//!   the Docker matrix test for that decision.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::conn::{CatalogConn, IntroError, Row, placeholders};
use crate::model::*;

/// MySQL or MariaDB (HV-INTRO-04 `flavor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Flavor {
    MySql,
    MariaDb,
}

/// What a pass knows of the server (HV-INTRO-07 step 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCtx {
    pub flavor: Flavor,
    pub version: (u32, u32, u32),
    pub version_text: String,
    pub lower_case_table_names: u8,
    /// MySQL `@@server_uuid`; MariaDB has none (`None`): identity falls back to the version.
    pub server_uuid: Option<String>,
}

impl ServerCtx {
    pub fn at_least(&self, v: (u32, u32, u32)) -> bool {
        self.version >= v
    }

    fn mysql_at_least(&self, v: (u32, u32, u32)) -> bool {
        self.flavor == Flavor::MySql && self.at_least(v)
    }

    fn mariadb_at_least(&self, v: (u32, u32, u32)) -> bool {
        self.flavor == Flavor::MariaDb && self.at_least(v)
    }

    /// Whether this server gets the `CREATE_TIME` incremental path (HV-INTRO-07 matrix).
    pub fn incremental_supported(&self) -> bool {
        self.mysql_at_least((8, 0, 0))
    }
}

/// `8.4.3`, `11.4.5-MariaDB-ubu2404` → `(maj, min, patch)` and the flavor.
pub fn parse_version(text: &str) -> ((u32, u32, u32), Flavor) {
    let flavor = if text.contains("MariaDB") {
        Flavor::MariaDb
    } else {
        Flavor::MySql
    };
    // MariaDB once prefixed `5.5.5-` for old clients; the server's own VERSION() does not.
    let core = text.strip_prefix("5.5.5-").unwrap_or(text);
    let mut parts = core
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<u32>().unwrap_or(0));
    let v = (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    );
    (v, flavor)
}

/// Reads the server context and prepares the metadata session: `time_zone = '+00:00'` so every
/// `TIMESTAMP` marker reads as UTC (HV-INTRO-07 step 1, edge 8).
pub async fn server_ctx(c: &mut dyn CatalogConn) -> Result<ServerCtx, IntroError> {
    c.control("SET SESSION time_zone = '+00:00'").await?;
    let rows = c
        .rows("SELECT VERSION(), @@lower_case_table_names", &[])
        .await?;
    let r = rows.first().ok_or_else(|| IntroError::Shape {
        sql: "VERSION()".into(),
        detail: "no row".into(),
    })?;
    let version_text = r.str(0).to_owned();
    let (version, flavor) = parse_version(&version_text);
    let lower_case_table_names = r.u32(1).unwrap_or(0) as u8;
    let server_uuid = if flavor == Flavor::MySql {
        c.rows("SELECT @@server_uuid", &[])
            .await?
            .first()
            .and_then(|r| r.text(0))
    } else {
        None
    };
    Ok(ServerCtx {
        flavor,
        version,
        version_text,
        lower_case_table_names,
        server_uuid,
    })
}

const SYSTEM_SCHEMAS: [&str; 4] = ["mysql", "information_schema", "performance_schema", "sys"];

/// The databases the login can see (HV-INTRO-01 `ScopeRefresh`).
pub async fn list_namespaces(c: &mut dyn CatalogConn) -> Result<Vec<NamespaceInfo>, IntroError> {
    let rows = c
        .rows(
            "SELECT SCHEMA_NAME, DEFAULT_CHARACTER_SET_NAME, DEFAULT_COLLATION_NAME \
             FROM INFORMATION_SCHEMA.SCHEMATA",
            &[],
        )
        .await?;
    let mut out: Vec<NamespaceInfo> = rows
        .iter()
        .map(|r| NamespaceInfo {
            name: r.str(0).to_owned(),
            detail: r.text(1),
            collation: r.text(2),
            // Exact comparison: these four are always stored lower case.
            system: SYSTEM_SCHEMAS.contains(&r.str(0)),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// The incremental watermark (HV-INTRO-07 `Watermark::MyTime`): server UTC time read at the
/// start of the pass, and the server's identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MyWatermark {
    pub at_utc: String,
    pub server_uuid: Option<String>,
    pub version_text: String,
}

/// What an incremental pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IncStats {
    /// Why the pass fell back to a full pass, when it did.
    pub full: Option<&'static str>,
    pub relations_reread: Vec<String>,
    pub routines_reread: Vec<String>,
    pub triggers_reread: Vec<String>,
    pub events_reread: Vec<String>,
}

/// More changed objects than this re-reads the whole namespace (one bulk query per kind is then
/// cheaper than chunks of `IN` lists).
const INCREMENTAL_LIMIT: usize = 2_000;
/// Names per `IN (…)` chunk.
const IN_CHUNK: usize = 500;

async fn now_utc(c: &mut dyn CatalogConn) -> Result<String, IntroError> {
    Ok(c.rows("SELECT CAST(UTC_TIMESTAMP(6) AS CHAR)", &[])
        .await?
        .first()
        .map(|r| r.str(0).to_owned())
        .unwrap_or_default())
}

fn watermark(ctx: &ServerCtx, at_utc: String) -> MyWatermark {
    MyWatermark {
        at_utc,
        server_uuid: ctx.server_uuid.clone(),
        version_text: ctx.version_text.clone(),
    }
}

/// A full pass over `schema` at `level` (HV-INTRO-01 `Full`).
pub async fn introspect(
    c: &mut dyn CatalogConn,
    ctx: &ServerCtx,
    schema: &str,
    level: Level,
) -> Result<(SchemaSnapshot, MyWatermark), IntroError> {
    let at = now_utc(c).await?;
    let mut snap = SchemaSnapshot {
        name: schema.to_owned(),
        level: Some(level),
        relations: load_relations(c, ctx, schema, None, level).await?,
        routines: load_routines(c, schema, None, level).await?,
        triggers: if level >= Level::Structure {
            load_triggers(c, schema, None, level).await?
        } else {
            Vec::new()
        },
        events: if level >= Level::Structure {
            load_events(c, schema, None, level).await?
        } else {
            Vec::new()
        },
    };
    snap.normalize();
    Ok((snap, watermark(ctx, at)))
}

/// An incremental pass (HV-INTRO-07): markers per kind, full rows of what changed, merged onto
/// `prev` (HV-INTRO-11: existence by name per kind, upsert the changed, keep the rest).
///
/// An object is re-read when it is new, its marker differs from the stored one, or its marker
/// is within one second before the previous pass started (`>= W − 1 s`, the 1-second
/// granularity overlap). Equality catches a long `ALTER` whose stamp predates `W`.
pub async fn incremental(
    c: &mut dyn CatalogConn,
    ctx: &ServerCtx,
    prev: &SchemaSnapshot,
    prev_w: &MyWatermark,
    level: Level,
) -> Result<(SchemaSnapshot, MyWatermark, IncStats), IntroError> {
    let schema = prev.name.as_str();
    let full_because = if !ctx.incremental_supported() {
        Some("flavor")
    } else if prev.level != Some(level) || level < Level::Structure {
        Some("level")
    } else if prev_w.server_uuid != ctx.server_uuid || prev_w.version_text != ctx.version_text {
        Some("server identity")
    } else {
        None
    };
    if let Some(why) = full_because {
        let (snap, w) = introspect(c, ctx, schema, level).await?;
        return Ok((
            snap,
            w,
            IncStats {
                full: Some(why),
                ..IncStats::default()
            },
        ));
    }
    let at = now_utc(c).await?;
    let w = prev_w.at_utc.as_str();
    let mut stats = IncStats::default();

    // Relations: existence + markers.
    let tables = c
        .rows(
            "SELECT TABLE_NAME, TABLE_TYPE, CAST(CREATE_TIME AS CHAR), \
             CREATE_TIME >= CAST(? AS DATETIME(6)) - INTERVAL 1 SECOND \
             FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA = ?",
            &[w, schema],
        )
        .await?;
    let views = c
        .rows(
            &format!(
                "SELECT TABLE_NAME, {VIEW_HASH} FROM INFORMATION_SCHEMA.VIEWS \
                 WHERE TABLE_SCHEMA = ?"
            ),
            &[schema],
        )
        .await?;
    let view_hash: HashMap<&str, Option<&str>> =
        views.iter().map(|r| (r.str(0), r.opt(1))).collect();
    let prev_rel: HashMap<&str, &Relation> = prev
        .relations
        .iter()
        .map(|r| (r.name.as_str(), r))
        .collect();
    let mut exist: Vec<&str> = Vec::with_capacity(tables.len());
    let mut changed: BTreeSet<String> = BTreeSet::new();
    for r in &tables {
        let name = r.str(0);
        exist.push(name);
        let kind = relation_kind(r.str(1));
        let marker = if kind == RelationKind::View {
            view_hash.get(name).copied().flatten()
        } else {
            r.opt(2)
        };
        let is_changed = match prev_rel.get(name) {
            None => true,
            Some(p) => p.kind.as_ref() != Some(&kind) || p.marker.as_deref() != marker || r.flag(3),
        };
        if is_changed {
            changed.insert(name.to_owned());
        }
    }
    // Views over a changed or dropped relation, transitively (their columns follow the base:
    // HV-INTRO-07 step 7). `VIEW_TABLE_USAGE` is MySQL 8.0.13+.
    let exist_set: BTreeSet<&str> = exist.iter().copied().collect();
    let dropped: Vec<&str> = prev
        .relations
        .iter()
        .map(|r| r.name.as_str())
        .filter(|n| !exist_set.contains(n))
        .collect();
    if !views.is_empty() && (!changed.is_empty() || !dropped.is_empty()) {
        let usage = c
            .rows(
                "SELECT VIEW_NAME, TABLE_SCHEMA, TABLE_NAME FROM INFORMATION_SCHEMA.VIEW_TABLE_USAGE \
                 WHERE VIEW_SCHEMA = ?",
                &[schema],
            )
            .await?;
        let mut dirty: BTreeSet<String> = changed.iter().cloned().collect();
        dirty.extend(dropped.iter().map(|s| (*s).to_owned()));
        loop {
            let before = dirty.len();
            for u in &usage {
                if u.str(1) == schema && dirty.contains(u.str(2)) {
                    dirty.insert(u.str(0).to_owned());
                }
            }
            if dirty.len() == before {
                break;
            }
        }
        for v in dirty {
            if view_hash.contains_key(v.as_str()) {
                changed.insert(v);
            }
        }
    }

    // Routines (`LAST_ALTERED`), triggers (`CREATED` + table: a renamed table moves its
    // triggers without a new `CREATED`), events (`LAST_ALTERED`).
    let routines = c
        .rows(
            "SELECT SPECIFIC_NAME, ROUTINE_TYPE, CAST(LAST_ALTERED AS CHAR), \
             LAST_ALTERED >= CAST(? AS DATETIME(6)) - INTERVAL 1 SECOND \
             FROM INFORMATION_SCHEMA.ROUTINES WHERE ROUTINE_SCHEMA = ?",
            &[w, schema],
        )
        .await?;
    let triggers = c
        .rows(
            "SELECT TRIGGER_NAME, EVENT_OBJECT_TABLE, CAST(CREATED AS CHAR), \
             CREATED >= CAST(? AS DATETIME(6)) - INTERVAL 1 SECOND \
             FROM INFORMATION_SCHEMA.TRIGGERS WHERE TRIGGER_SCHEMA = ?",
            &[w, schema],
        )
        .await?;
    let events = c
        .rows(
            "SELECT EVENT_NAME, CAST(LAST_ALTERED AS CHAR), \
             LAST_ALTERED >= CAST(? AS DATETIME(6)) - INTERVAL 1 SECOND \
             FROM INFORMATION_SCHEMA.EVENTS WHERE EVENT_SCHEMA = ?",
            &[w, schema],
        )
        .await?;

    let prev_rt: HashMap<(Option<RoutineKind>, &str), &Routine> = prev
        .routines
        .iter()
        .map(|r| ((r.kind, r.specific_name.as_str()), r))
        .collect();
    let mut rt_changed: BTreeSet<String> = BTreeSet::new();
    for r in &routines {
        let key = (routine_kind(r.str(1)), r.str(0));
        let ch = match prev_rt.get(&key) {
            None => true,
            Some(p) => p.marker.as_deref() != r.opt(2) || r.flag(3),
        };
        if ch {
            rt_changed.insert(r.str(0).to_owned());
        }
    }
    let prev_tg: HashMap<&str, &Trigger> =
        prev.triggers.iter().map(|t| (t.name.as_str(), t)).collect();
    let mut tg_changed: BTreeSet<String> = BTreeSet::new();
    for r in &triggers {
        let ch = match prev_tg.get(r.str(0)) {
            None => true,
            Some(p) => p.table != r.str(1) || p.marker.as_deref() != r.opt(2) || r.flag(3),
        };
        if ch {
            tg_changed.insert(r.str(0).to_owned());
        }
    }
    let prev_ev: HashMap<&str, &Event> = prev.events.iter().map(|e| (e.name.as_str(), e)).collect();
    let mut ev_changed: BTreeSet<String> = BTreeSet::new();
    for r in &events {
        let ch = match prev_ev.get(r.str(0)) {
            None => true,
            Some(p) => p.marker.as_deref() != r.opt(1) || r.flag(2),
        };
        if ch {
            ev_changed.insert(r.str(0).to_owned());
        }
    }

    if changed.len() + rt_changed.len() + tg_changed.len() + ev_changed.len() > INCREMENTAL_LIMIT {
        let (snap, w) = introspect(c, ctx, schema, level).await?;
        return Ok((
            snap,
            w,
            IncStats {
                full: Some("too many changes"),
                ..IncStats::default()
            },
        ));
    }

    // Full rows of the changed objects.
    let rel_names: Vec<String> = changed.iter().cloned().collect();
    let mut fresh: HashMap<String, Relation> =
        load_relations(c, ctx, schema, Some(&rel_names), level)
            .await?
            .into_iter()
            .map(|r| (r.name.clone(), r))
            .collect();
    let rt_names: Vec<String> = rt_changed.iter().cloned().collect();
    let fresh_rt = load_routines(c, schema, Some(&rt_names), level).await?;
    let tg_names: Vec<String> = tg_changed.iter().cloned().collect();
    let mut fresh_tg: HashMap<String, Trigger> = load_triggers(c, schema, Some(&tg_names), level)
        .await?
        .into_iter()
        .map(|t| (t.name.clone(), t))
        .collect();
    let ev_names: Vec<String> = ev_changed.iter().cloned().collect();
    let mut fresh_ev: HashMap<String, Event> = load_events(c, schema, Some(&ev_names), level)
        .await?
        .into_iter()
        .map(|e| (e.name.clone(), e))
        .collect();

    // Merge (HV-INTRO-11): existence from the marker queries; changed objects from `fresh`,
    // the rest from `prev`. An object listed by the marker query but gone by the time its rows
    // were read is dropped from this pass (the next one sees it consistently).
    let mut next = SchemaSnapshot {
        name: schema.to_owned(),
        level: Some(level),
        ..SchemaSnapshot::default()
    };
    for name in &exist {
        if let Some(r) = fresh.remove(*name) {
            next.relations.push(r);
        } else if !changed.contains(*name)
            && let Some(p) = prev_rel.get(name)
        {
            next.relations.push((*p).clone());
        }
    }
    let mut fresh_rt_map: HashMap<(Option<RoutineKind>, String), Routine> = fresh_rt
        .into_iter()
        .map(|r| ((r.kind, r.specific_name.clone()), r))
        .collect();
    for r in &routines {
        let kind = routine_kind(r.str(1));
        let name = r.str(0);
        if let Some(f) = fresh_rt_map.remove(&(kind, name.to_owned())) {
            next.routines.push(f);
        } else if !rt_changed.contains(name)
            && let Some(p) = prev_rt.get(&(kind, name))
        {
            next.routines.push((*p).clone());
        }
    }
    for r in &triggers {
        let name = r.str(0);
        if let Some(f) = fresh_tg.remove(name) {
            next.triggers.push(f);
        } else if !tg_changed.contains(name)
            && let Some(p) = prev_tg.get(name)
        {
            next.triggers.push((*p).clone());
        }
    }
    for r in &events {
        let name = r.str(0);
        if let Some(f) = fresh_ev.remove(name) {
            next.events.push(f);
        } else if !ev_changed.contains(name)
            && let Some(p) = prev_ev.get(name)
        {
            next.events.push((*p).clone());
        }
    }
    next.normalize();
    stats.relations_reread = rel_names;
    stats.routines_reread = rt_names;
    stats.triggers_reread = tg_names;
    stats.events_reread = ev_names;
    Ok((next, watermark(ctx, at), stats))
}

/// The view-definition hash (HV-INTRO-07 step 7): views have no timestamp. `CONCAT_WS` with
/// the unit separator; computed server-side so a marker pass never ships definitions.
const VIEW_HASH: &str = "SHA2(CONCAT_WS(0x1f, VIEW_DEFINITION, CHECK_OPTION, IS_UPDATABLE, \
                         SECURITY_TYPE, DEFINER), 256)";

/// Runs `sql` (ending in `WHERE <schema column> = ?`) for `schema`, restricted to `filter`'s
/// names in chunks when given. An empty filter runs nothing.
async fn fetch(
    c: &mut dyn CatalogConn,
    sql: &str,
    schema: &str,
    filter: Option<(&str, &[String])>,
) -> Result<Vec<Row>, IntroError> {
    match filter {
        None => c.rows(sql, &[schema]).await,
        Some((_, [])) => Ok(Vec::new()),
        Some((col, names)) => {
            let mut out = Vec::new();
            for chunk in names.chunks(IN_CHUNK) {
                let q = format!("{sql} AND {col} IN ({})", placeholders(chunk.len()));
                let mut binds: Vec<&str> = Vec::with_capacity(chunk.len() + 1);
                binds.push(schema);
                binds.extend(chunk.iter().map(String::as_str));
                out.extend(c.rows(&q, &binds).await?);
            }
            Ok(out)
        }
    }
}

pub fn relation_kind(table_type: &str) -> RelationKind {
    match table_type {
        "BASE TABLE" => RelationKind::Table,
        "VIEW" | "SYSTEM VIEW" => RelationKind::View,
        "SEQUENCE" => RelationKind::Sequence,
        "SYSTEM VERSIONED" => RelationKind::SystemVersioned,
        other => RelationKind::Other(other.to_owned()),
    }
}

fn routine_kind(t: &str) -> Option<RoutineKind> {
    match t {
        "FUNCTION" => Some(RoutineKind::Function),
        "PROCEDURE" => Some(RoutineKind::Procedure),
        "PACKAGE" => Some(RoutineKind::Package),
        "PACKAGE BODY" => Some(RoutineKind::PackageBody),
        _ => None,
    }
}

fn non_empty(v: Option<String>) -> Option<String> {
    v.filter(|s| !s.is_empty())
}

/// `EXTRA` → (generated kind, invisible, auto_increment, on update). ASCII-case-insensitive
/// token tests (INV-3): MySQL writes `VIRTUAL GENERATED`, `DEFAULT_GENERATED on update
/// CURRENT_TIMESTAMP`, `INVISIBLE`; MariaDB `on update current_timestamp()`.
pub fn parse_extra(extra: &str) -> (Option<GeneratedKind>, bool, bool, Option<String>) {
    let lower = extra.to_ascii_lowercase();
    let generated = if lower.contains("stored generated") || lower.contains("persistent generated")
    {
        Some(GeneratedKind::Stored)
    } else if lower.contains("virtual generated") {
        Some(GeneratedKind::Virtual)
    } else {
        None
    };
    let invisible = lower.split_whitespace().any(|t| t == "invisible");
    let auto_increment = lower.contains("auto_increment");
    let on_update = lower
        .find("on update ")
        .map(|i| extra[i + "on update ".len()..].trim().to_owned());
    (generated, invisible, auto_increment, on_update)
}

/// Relations of `schema` (all, or `names`) at `level`.
async fn load_relations(
    c: &mut dyn CatalogConn,
    ctx: &ServerCtx,
    schema: &str,
    names: Option<&[String]>,
    level: Level,
) -> Result<Vec<Relation>, IntroError> {
    let f = |col: &'static str| names.map(|n| (col, n));
    let tables = fetch(
        c,
        "SELECT TABLE_NAME, TABLE_TYPE, ENGINE, TABLE_COLLATION, CREATE_OPTIONS, TABLE_COMMENT, \
         CAST(CREATE_TIME AS CHAR) FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA = ?",
        schema,
        f("TABLE_NAME"),
    )
    .await?;
    let mut rels: BTreeMap<String, Relation> = BTreeMap::new();
    for r in &tables {
        let kind = relation_kind(r.str(1));
        let is_view = kind == RelationKind::View;
        rels.insert(
            r.str(0).to_owned(),
            Relation {
                name: r.str(0).to_owned(),
                kind: Some(kind),
                engine: r.text(2),
                collation: r.text(3),
                options: non_empty(r.text(4)),
                // Views report `VIEW` as their comment.
                comment: if is_view { None } else { non_empty(r.text(5)) },
                marker: if is_view { None } else { r.text(6) },
                ..Relation::default()
            },
        );
    }
    if level < Level::Structure {
        return Ok(rels.into_values().collect());
    }

    // Views: attributes, the marker hash, the definition at `Sources`.
    let def = if level >= Level::Sources {
        ", VIEW_DEFINITION"
    } else {
        ""
    };
    let views = fetch(
        c,
        &format!(
            "SELECT TABLE_NAME, CHECK_OPTION, IS_UPDATABLE, SECURITY_TYPE, DEFINER, {VIEW_HASH}{def} \
             FROM INFORMATION_SCHEMA.VIEWS WHERE TABLE_SCHEMA = ?"
        ),
        schema,
        f("TABLE_NAME"),
    )
    .await?;
    for r in &views {
        if let Some(rel) = rels.get_mut(r.str(0)) {
            rel.marker = r.text(5);
            rel.view = Some(ViewDef {
                definition: if level >= Level::Sources {
                    non_empty(r.text(6))
                } else {
                    None
                },
                check_option: r.text(1),
                updatable: Some(r.flag(2)),
                security_type: r.text(3),
                definer: r.text(4),
            });
        }
    }

    // Columns.
    let cols = fetch(
        c,
        "SELECT TABLE_NAME, COLUMN_NAME, ORDINAL_POSITION, COLUMN_DEFAULT, IS_NULLABLE, \
         COLUMN_TYPE, CHARACTER_SET_NAME, COLLATION_NAME, EXTRA, COLUMN_COMMENT, \
         GENERATION_EXPRESSION FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA = ?",
        schema,
        f("TABLE_NAME"),
    )
    .await?;
    for r in &cols {
        let Some(rel) = rels.get_mut(r.str(0)) else {
            continue; // created after the relation list was read (HV-INTRO-01 edge 4)
        };
        let (gen_kind, invisible, auto_increment, on_update) = parse_extra(r.str(8));
        rel.columns.push(Column {
            name: r.str(1).to_owned(),
            ordinal: r.u32(2).unwrap_or(0),
            default: r.text(3),
            nullable: r.flag(4),
            data_type: r.str(5).to_owned(),
            charset: r.text(6),
            collation: r.text(7),
            comment: non_empty(r.text(9)),
            generated: gen_kind.map(|k| (k, non_empty(r.text(10)))),
            hidden: invisible,
            auto_increment,
            on_update,
            pk_position: 0,
        });
    }

    // Indexes, one row per key part (HV-INTRO-13).
    let (vis, expr) = if ctx.flavor == Flavor::MySql {
        (
            "IS_VISIBLE",
            if ctx.mysql_at_least((8, 0, 13)) {
                "EXPRESSION"
            } else {
                "NULL"
            },
        )
    } else if ctx.mariadb_at_least((10, 6, 0)) {
        ("CASE IGNORED WHEN 'YES' THEN 'NO' ELSE 'YES' END", "NULL")
    } else {
        ("'YES'", "NULL")
    };
    let stats = fetch(
        c,
        &format!(
            "SELECT TABLE_NAME, INDEX_NAME, NON_UNIQUE, SEQ_IN_INDEX, COLUMN_NAME, COLLATION, \
             SUB_PART, INDEX_TYPE, INDEX_COMMENT, {vis}, {expr} \
             FROM INFORMATION_SCHEMA.STATISTICS WHERE TABLE_SCHEMA = ?"
        ),
        schema,
        f("TABLE_NAME"),
    )
    .await?;
    let mut parts: BTreeMap<(String, String), Vec<(u32, &Row)>> = BTreeMap::new();
    for r in &stats {
        parts
            .entry((r.str(0).to_owned(), r.str(1).to_owned()))
            .or_default()
            .push((r.u32(3).unwrap_or(0), r));
    }
    for ((table, index), mut ps) in parts {
        let Some(rel) = rels.get_mut(&table) else {
            continue;
        };
        ps.sort_by_key(|(seq, _)| *seq);
        let first = ps[0].1;
        rel.indexes.push(Index {
            primary: index == "PRIMARY",
            name: index,
            unique: first.u32(2) == Some(0),
            index_type: first.text(7),
            comment: non_empty(first.text(8)),
            visible: first.flag(9),
            columns: ps
                .iter()
                .map(|(_, r)| IndexColumn {
                    name: r.text(4),
                    expression: r.text(10),
                    descending: r.opt(5) == Some("D"),
                    sub_part: r.u32(6),
                    collation: None,
                })
                .collect(),
            ..Index::default()
        });
    }

    // Constraints: PK/UK/FK from TABLE_CONSTRAINTS + KEY_COLUMN_USAGE (+ REFERENTIAL_CONSTRAINTS);
    // CHECK from CHECK_CONSTRAINTS.
    let enforced = if ctx.mysql_at_least((8, 0, 16)) {
        "ENFORCED"
    } else {
        "'YES'"
    };
    let tcs = fetch(
        c,
        &format!(
            "SELECT TABLE_NAME, CONSTRAINT_NAME, CONSTRAINT_TYPE, {enforced} \
             FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS WHERE TABLE_SCHEMA = ?"
        ),
        schema,
        f("TABLE_NAME"),
    )
    .await?;
    let kcu = fetch(
        c,
        "SELECT TABLE_NAME, CONSTRAINT_NAME, COLUMN_NAME, ORDINAL_POSITION, \
         REFERENCED_TABLE_SCHEMA, REFERENCED_TABLE_NAME, REFERENCED_COLUMN_NAME \
         FROM INFORMATION_SCHEMA.KEY_COLUMN_USAGE WHERE TABLE_SCHEMA = ?",
        schema,
        f("TABLE_NAME"),
    )
    .await?;
    let refs = fetch(
        c,
        "SELECT TABLE_NAME, CONSTRAINT_NAME, UNIQUE_CONSTRAINT_NAME, MATCH_OPTION, \
         UPDATE_RULE, DELETE_RULE FROM INFORMATION_SCHEMA.REFERENTIAL_CONSTRAINTS \
         WHERE CONSTRAINT_SCHEMA = ?",
        schema,
        f("TABLE_NAME"),
    )
    .await?;
    let mut key_cols: HashMap<(&str, &str), Vec<(u32, &Row)>> = HashMap::new();
    for r in &kcu {
        key_cols
            .entry((r.str(0), r.str(1)))
            .or_default()
            .push((r.u32(3).unwrap_or(0), r));
    }
    for v in key_cols.values_mut() {
        v.sort_by_key(|(o, _)| *o);
    }
    let ref_of: HashMap<(&str, &str), &Row> =
        refs.iter().map(|r| ((r.str(0), r.str(1)), r)).collect();
    for r in &tcs {
        let (table, name, ty) = (r.str(0), r.str(1), r.str(2));
        let Some(rel) = rels.get_mut(table) else {
            continue;
        };
        let cols = key_cols.get(&(table, name));
        let columns: Vec<String> = cols
            .map(|v| v.iter().map(|(_, k)| k.str(2).to_owned()).collect())
            .unwrap_or_default();
        let kind = match ty {
            "PRIMARY KEY" => ConstraintKind::PrimaryKey,
            "UNIQUE" => ConstraintKind::Unique,
            "FOREIGN KEY" => {
                let first = cols.and_then(|v| v.first()).map(|(_, k)| *k);
                let rc = ref_of.get(&(table, name));
                ConstraintKind::ForeignKey(ForeignKey {
                    ref_schema: first.and_then(|k| k.text(4)),
                    ref_table: first.map(|k| k.str(5).to_owned()).unwrap_or_default(),
                    ref_columns: cols
                        .map(|v| v.iter().map(|(_, k)| k.text(6)).collect())
                        .unwrap_or_default(),
                    ref_constraint: rc.and_then(|r| r.text(2)),
                    match_option: rc.and_then(|r| r.text(3)),
                    on_update: rc.and_then(|r| r.text(4)),
                    on_delete: rc.and_then(|r| r.text(5)),
                })
            }
            // CHECK rows come from CHECK_CONSTRAINTS below (their clause lives there).
            _ => continue,
        };
        rel.constraints.push(Constraint {
            name: Some(name.to_owned()),
            kind,
            columns,
            enforced: r.flag(3),
        });
    }
    let checks = if ctx.mysql_at_least((8, 0, 16)) {
        // MySQL: check names are unique per schema; the table comes from TABLE_CONSTRAINTS.
        fetch(
            c,
            "SELECT tc.TABLE_NAME, cc.CONSTRAINT_NAME, cc.CHECK_CLAUSE, tc.ENFORCED \
             FROM INFORMATION_SCHEMA.CHECK_CONSTRAINTS cc \
             JOIN INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc \
               ON tc.CONSTRAINT_SCHEMA = cc.CONSTRAINT_SCHEMA \
              AND tc.CONSTRAINT_NAME = cc.CONSTRAINT_NAME AND tc.CONSTRAINT_TYPE = 'CHECK' \
             WHERE cc.CONSTRAINT_SCHEMA = ?",
            schema,
            f("tc.TABLE_NAME"),
        )
        .await?
    } else if ctx.flavor == Flavor::MariaDb {
        // MariaDB: names are unique per table, and CHECK_CONSTRAINTS has TABLE_NAME.
        fetch(
            c,
            "SELECT TABLE_NAME, CONSTRAINT_NAME, CHECK_CLAUSE, 'YES' \
             FROM INFORMATION_SCHEMA.CHECK_CONSTRAINTS WHERE CONSTRAINT_SCHEMA = ?",
            schema,
            f("TABLE_NAME"),
        )
        .await?
    } else {
        Vec::new()
    };
    for r in &checks {
        if let Some(rel) = rels.get_mut(r.str(0)) {
            rel.constraints.push(Constraint {
                name: Some(r.str(1).to_owned()),
                kind: ConstraintKind::Check {
                    clause: r.str(2).to_owned(),
                },
                columns: Vec::new(),
                enforced: r.flag(3),
            });
        }
    }
    let mut out: Vec<Relation> = rels.into_values().collect();
    for r in &mut out {
        r.normalize();
    }
    Ok(out)
}

async fn load_routines(
    c: &mut dyn CatalogConn,
    schema: &str,
    names: Option<&[String]>,
    level: Level,
) -> Result<Vec<Routine>, IntroError> {
    let def = if level >= Level::Sources {
        ", ROUTINE_DEFINITION"
    } else {
        ""
    };
    let f = names.map(|n| ("SPECIFIC_NAME", n));
    let rows = fetch(
        c,
        &format!(
            "SELECT SPECIFIC_NAME, ROUTINE_NAME, ROUTINE_TYPE, DTD_IDENTIFIER, IS_DETERMINISTIC, \
             SQL_DATA_ACCESS, SECURITY_TYPE, DEFINER, ROUTINE_COMMENT, \
             CAST(LAST_ALTERED AS CHAR){def} FROM INFORMATION_SCHEMA.ROUTINES \
             WHERE ROUTINE_SCHEMA = ?"
        ),
        schema,
        f,
    )
    .await?;
    let mut out: Vec<Routine> = rows
        .iter()
        .map(|r| Routine {
            specific_name: r.str(0).to_owned(),
            name: r.str(1).to_owned(),
            kind: routine_kind(r.str(2)),
            returns: non_empty(r.text(3)),
            deterministic: Some(r.flag(4)),
            data_access: r.text(5),
            security_type: r.text(6),
            definer: r.text(7),
            comment: non_empty(r.text(8)),
            marker: r.text(9),
            definition: if level >= Level::Sources {
                r.text(10)
            } else {
                None
            },
            params: Vec::new(),
        })
        .collect();
    if level >= Level::Structure && !out.is_empty() {
        // `ORDINAL_POSITION = 0` is a function's result, not a parameter.
        let params = fetch(
            c,
            "SELECT SPECIFIC_NAME, ROUTINE_TYPE, ORDINAL_POSITION, PARAMETER_MODE, PARAMETER_NAME, \
             DTD_IDENTIFIER FROM INFORMATION_SCHEMA.PARAMETERS \
             WHERE ORDINAL_POSITION > 0 AND SPECIFIC_SCHEMA = ?",
            schema,
            f,
        )
        .await?;
        let mut by: HashMap<(Option<RoutineKind>, &str), Vec<Param>> = HashMap::new();
        for p in &params {
            by.entry((routine_kind(p.str(1)), p.str(0)))
                .or_default()
                .push(Param {
                    ordinal: p.u32(2).unwrap_or(0),
                    mode: p.text(3),
                    name: p.text(4),
                    data_type: p.str(5).to_owned(),
                });
        }
        for r in &mut out {
            if let Some(ps) = by.remove(&(r.kind, r.specific_name.as_str())) {
                r.params = ps;
                r.params.sort_by_key(|p| p.ordinal);
            }
        }
    }
    Ok(out)
}

async fn load_triggers(
    c: &mut dyn CatalogConn,
    schema: &str,
    names: Option<&[String]>,
    level: Level,
) -> Result<Vec<Trigger>, IntroError> {
    let body = if level >= Level::Sources {
        ", ACTION_STATEMENT"
    } else {
        ""
    };
    let rows = fetch(
        c,
        &format!(
            "SELECT TRIGGER_NAME, EVENT_OBJECT_TABLE, ACTION_TIMING, EVENT_MANIPULATION, \
             ACTION_ORDER, CAST(CREATED AS CHAR){body} FROM INFORMATION_SCHEMA.TRIGGERS \
             WHERE TRIGGER_SCHEMA = ?"
        ),
        schema,
        names.map(|n| ("TRIGGER_NAME", n)),
    )
    .await?;
    Ok(rows
        .iter()
        .map(|r| Trigger {
            name: r.str(0).to_owned(),
            table: r.str(1).to_owned(),
            timing: r.text(2),
            event: r.text(3),
            order: r.u32(4),
            marker: r.text(5),
            body: if level >= Level::Sources {
                r.text(6)
            } else {
                None
            },
        })
        .collect())
}

async fn load_events(
    c: &mut dyn CatalogConn,
    schema: &str,
    names: Option<&[String]>,
    level: Level,
) -> Result<Vec<Event>, IntroError> {
    let body = if level >= Level::Sources {
        ", EVENT_DEFINITION"
    } else {
        ""
    };
    let rows = fetch(
        c,
        &format!(
            "SELECT EVENT_NAME, EVENT_TYPE, CAST(EXECUTE_AT AS CHAR), INTERVAL_VALUE, \
             INTERVAL_FIELD, STATUS, ON_COMPLETION, EVENT_COMMENT, DEFINER, \
             CAST(LAST_ALTERED AS CHAR){body} FROM INFORMATION_SCHEMA.EVENTS \
             WHERE EVENT_SCHEMA = ?"
        ),
        schema,
        names.map(|n| ("EVENT_NAME", n)),
    )
    .await?;
    Ok(rows
        .iter()
        .map(|r| Event {
            name: r.str(0).to_owned(),
            event_type: r.text(1),
            execute_at: r.text(2),
            interval_value: r.text(3),
            interval_field: r.text(4),
            status: r.text(5),
            on_completion: r.text(6),
            comment: non_empty(r.text(7)),
            definer: r.text(8),
            marker: r.text(9),
            body: if level >= Level::Sources {
                r.text(10)
            } else {
                None
            },
        })
        .collect())
}

/// `check_state` (HV-INTRO-15) for tables: the current `CREATE_TIME` marker per name, plus the
/// newest trigger `CREATED` on it (trigger creation does not move `CREATE_TIME`, ver/01).
/// `None` for a name that no longer exists.
pub async fn check_tables(
    c: &mut dyn CatalogConn,
    schema: &str,
    names: &[String],
) -> Result<BTreeMap<String, Option<String>>, IntroError> {
    let mut out: BTreeMap<String, Option<String>> =
        names.iter().map(|n| (n.clone(), None)).collect();
    for chunk in names.chunks(IN_CHUNK) {
        let ph = placeholders(chunk.len());
        let sql = format!(
            "SELECT t.TABLE_NAME, CONCAT_WS('|', CAST(t.CREATE_TIME AS CHAR), \
               (SELECT CAST(MAX(g.CREATED) AS CHAR) FROM INFORMATION_SCHEMA.TRIGGERS g \
                 WHERE g.EVENT_OBJECT_SCHEMA = t.TABLE_SCHEMA AND g.EVENT_OBJECT_TABLE = t.TABLE_NAME), \
               (SELECT COUNT(*) FROM INFORMATION_SCHEMA.TRIGGERS g \
                 WHERE g.EVENT_OBJECT_SCHEMA = t.TABLE_SCHEMA AND g.EVENT_OBJECT_TABLE = t.TABLE_NAME)) \
             FROM INFORMATION_SCHEMA.TABLES t WHERE t.TABLE_SCHEMA = ? AND t.TABLE_NAME IN ({ph})"
        );
        let mut binds: Vec<&str> = vec![schema];
        binds.extend(chunk.iter().map(String::as_str));
        for r in c.rows(&sql, &binds).await? {
            out.insert(r.str(0).to_owned(), Some(r.str(1).to_owned()));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_and_flavors_parse() {
        assert_eq!(parse_version("8.4.3"), ((8, 4, 3), Flavor::MySql));
        assert_eq!(parse_version("8.0.46"), ((8, 0, 46), Flavor::MySql));
        assert_eq!(
            parse_version("11.4.5-MariaDB-ubu2404"),
            ((11, 4, 5), Flavor::MariaDb)
        );
        assert_eq!(
            parse_version("5.5.5-10.6.21-MariaDB-log"),
            ((10, 6, 21), Flavor::MariaDb)
        );
    }

    #[test]
    fn extra_parses_generated_invisible_and_on_update() {
        assert_eq!(
            parse_extra("VIRTUAL GENERATED"),
            (Some(GeneratedKind::Virtual), false, false, None)
        );
        assert_eq!(
            parse_extra("STORED GENERATED INVISIBLE"),
            (Some(GeneratedKind::Stored), true, false, None)
        );
        assert_eq!(parse_extra("auto_increment"), (None, false, true, None));
        assert_eq!(
            parse_extra("DEFAULT_GENERATED on update CURRENT_TIMESTAMP"),
            (None, false, false, Some("CURRENT_TIMESTAMP".into()))
        );
        assert_eq!(
            parse_extra("on update current_timestamp()"),
            (None, false, false, Some("current_timestamp()".into()))
        );
        assert_eq!(
            parse_extra("PERSISTENT GENERATED").0,
            Some(GeneratedKind::Stored)
        );
    }

    #[test]
    fn table_types_map_and_unknown_is_kept() {
        assert_eq!(relation_kind("BASE TABLE"), RelationKind::Table);
        assert_eq!(relation_kind("SEQUENCE"), RelationKind::Sequence);
        assert_eq!(
            relation_kind("TEMPORARY"),
            RelationKind::Other("TEMPORARY".into())
        );
    }

    #[test]
    fn only_mysql_8_is_incremental() {
        let mut ctx = ServerCtx {
            flavor: Flavor::MySql,
            version: (8, 0, 46),
            version_text: "8.0.46".into(),
            lower_case_table_names: 0,
            server_uuid: None,
        };
        assert!(ctx.incremental_supported());
        ctx.flavor = Flavor::MariaDb;
        ctx.version = (11, 4, 5);
        assert!(!ctx.incremental_supported());
    }
}
