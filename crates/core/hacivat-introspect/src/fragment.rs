// ## m4-t01c-core
//! Fragment introspection on a caller's session (R-M4-29; Task 15b): a joined `MetaConn` reads
//! `keys` of `ns` inside a transaction Hacivat owns (M7's drift check inside the apply
//! transaction). Only `joinable` pack sections run there; the connection never begins, commits
//! or rolls back.
//!
//! ## m4-t15b-facets (M4 Task 15b; ADR 0008 `## m4-t15b — facets and joined fragments`)
//!
//! [`on_session`] joins the caller's session ([`MetaConn::join`]: only a `Job(DdlApply |
//! DriftCheck)` session inside a transaction; never a console or an editor), reads the
//! relations `keys` name (a column's key names its relation) and, at `Structure`, their
//! columns, and answers a **standalone** namespace snapshot: generation 0, built here from the
//! read, never handed to a publisher and never offered to a cache (it takes no catalog and no
//! cache, so it cannot reach either).
//!
//! - **What it sends.** Only `joinable` sections, rendered with `PackFlags::JOINED` (the pack
//!   renderer refuses any other section), each a single `SELECT` that the session's own guard
//!   checks again; no `BEGIN`, `COMMIT`, `ROLLBACK`, `SAVEPOINT`, `RELEASE` or `SET` exists in
//!   them, and `SnapshotTxn::commit` on a joined connection sends nothing.
//! - **Engine.** The caller passes the catalog's `ServerFacts`: `MetaConn::join_as` makes the
//!   session's actor refuse every read of a session of another engine before anything is sent
//!   (review m-1); the fold of the snapshot's names follows the facts' identifier rule (m-6).
//! - **PostgreSQL** (`packs/pg/50-joined.hvq.sql`): first `JoinedContext`, which answers the
//!   session's database, `current_schemas(true)` and the session's own temp schema
//!   (`pg_my_temp_schema()`); a session connected to another database than `ns`'s, or whose
//!   path does not start with `pg_catalog` once **its own** temp schema is skipped (another
//!   backend's `pg_temp_N` is refused: PostgreSQL searches it for operators, R-W5-5), is
//!   refused (`MetaError::NotJoinable`) before anything else is sent (R-W3-11: the one
//!   unqualified operator, the list bind's `IN`, then resolves in `pg_catalog`). Then
//!   `JoinedNamespaces` and `JoinedRelations` list the schema oids and the schema's relations
//!   (no name reaches the server: the schema and the caller's relations are picked by exact
//!   name here; every bind is an oid list), and `JoinedColumns` reads theirs by oid. A
//!   column's
//!   type is its schema and name (`TypeRef`, raw type = the type's name), never
//!   `format_type` text (R-M4-46); defaults and generation expressions are not read.
//! - **MySQL and MariaDB** (`packs/mysql/50-joined.hvq.sql`): `JoinedTables` and
//!   `JoinedColumns` from `INFORMATION_SCHEMA` by schema and names. MySQL commits a
//!   transaction implicitly before DDL: a job joins the transaction it holds when it asks.
//! - **SQLite**: the introspector's own fragment on the joined connection (every SQLite
//!   section is joinable; Task 18). A SQLite job session is the profile's one session; M7
//!   adds its writer seam, until then `MetaConn::join` refuses it.
//! - **Levels.** `Names` reads relations, `Structure` and above relations and columns (a
//!   joined read has no source text: `pg_get_*def` is not joinable).
//! - **Facets** are not read here: PostgreSQL's facet sections are not joinable, and
//!   `Introspector::facet` on a joined connection answers `Meta(NotJoinable)` (T16b m-7).

#![allow(
    clippy::result_large_err,
    reason = "IntroError carries the server's message by value (MetaError::Server), as the seam does"
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use futures::future::BoxFuture;
use hacivat_meta::merge::{self, ChangedObject, MergePolicy, NamespaceResult, Trigger};
use hacivat_meta::name::{FoldRule, NameClass};
use hacivat_meta::props::{ColumnProps, ColumnRare, Identity, Props, TableProps, TypeRef};
use hacivat_meta::{
    Level, Name, NamespaceSnapshot, NsKey, NsKind, Obj, ObjectKey, ObjectKind, ServerFacts,
    ServerId, Subtree, Watermark,
};
use hacivat_proto::DbmsKind;
use hacivat_session::meta::{MetaConn, MetaError, SnapshotTxn};
use hacivat_session::session::SessionHandle;
use hacivat_values::{BindValue, LogicalType};
use sql_dialect::DialectId;
use tokio_util::sync::CancellationToken;

use crate::engine::Filters;
use crate::introspector::{IntroError, Introspector};
use crate::pack::render::RenderError;
use crate::pack::{Mode, Pack, PackCtx, PackFlags, Params};

/// The PostgreSQL sections of `packs/pg/50-joined.hvq.sql`.
pub mod pg_name {
    pub const CONTEXT: &str = "JoinedContext";
    pub const NAMESPACES: &str = "JoinedNamespaces";
    pub const RELATIONS: &str = "JoinedRelations";
    pub const COLUMNS: &str = "JoinedColumns";
    pub const ALL: [&str; 4] = [CONTEXT, NAMESPACES, RELATIONS, COLUMNS];
}

/// The MySQL and MariaDB sections of `packs/mysql/50-joined.hvq.sql`.
pub mod mysql_name {
    pub const TABLES: &str = "JoinedTables";
    pub const COLUMNS: &str = "JoinedColumns";
    pub const ALL: [&str; 2] = [TABLES, COLUMNS];
}

/// Reads `keys` of `ns` at `level` on `handle`'s open, Hacivat-owned transaction (module docs).
/// `facts` are the catalog's server facts: `facts.dbms` is the session's engine (a joined
/// connection cannot ask the server: it knows no server of its own; a session of another
/// engine is refused before anything is sent), `facts.ident` the identifier rule its names
/// fold by.
///
/// The answer is never published and never cached: M7 compares it with the catalog.
pub fn on_session<'a>(
    handle: &'a SessionHandle,
    facts: &'a ServerFacts,
    ns: &'a NsKey,
    keys: &'a [ObjectKey],
    level: Level,
    cancel: &'a CancellationToken,
) -> BoxFuture<'a, Result<NamespaceSnapshot, IntroError>> {
    Box::pin(async move {
        let dbms = facts.dbms;
        // ## m4-t15b-facets (review m-1): the session's actor refuses every read when the
        // session is not of engine `dbms`, before anything is sent.
        let mut conn = MetaConn::join_as(handle, dbms)?;
        let flavor = flavor_of(facts);
        let r = match dbms {
            DbmsKind::Postgres => {
                pg(&mut conn, ns, keys, level.min(Level::Structure), cancel).await?
            }
            DbmsKind::MySql | DbmsKind::MariaDb => {
                mysql(
                    &mut conn,
                    flavor,
                    ns,
                    keys,
                    level.min(Level::Structure),
                    cancel,
                )
                .await?
            }
            DbmsKind::Sqlite => {
                let filter = Filters::default().for_pass(ns, false).clone();
                crate::sqlite::SqliteIntrospector::default()
                    .fragment(&mut conn, ns, keys, level, &filter, cancel)
                    .await?
            }
            _ => return Err(IntroError::NotImplemented),
        };
        drop(conn);
        standalone(ns, dbms, fold_of(facts, flavor), r).await
    })
}

/// The pack flavor of `facts`: the server's own (MariaDB behind a MySQL profile) within the
/// profile's family, else the profile's engine.
fn flavor_of(facts: &ServerFacts) -> DbmsKind {
    match (facts.dbms, facts.flavor) {
        (DbmsKind::MySql | DbmsKind::MariaDb, f @ (DbmsKind::MySql | DbmsKind::MariaDb)) => f,
        (d, _) => d,
    }
}

/// The fold rule of table-class names under `facts` (the server's `lower_case_table_names`),
/// as the engine stores the catalog's (review m-6): the joined snapshot's keys fold the same.
fn fold_of(facts: &ServerFacts, flavor: DbmsKind) -> FoldRule {
    let d = dialect_of(flavor);
    let mut ident = facts.ident.clone();
    ident.dialect = d;
    FoldRule::for_names(d, &ident, NameClass::Table).unwrap_or(FoldRule::Exact)
}

/// The dialect of `dbms`'s packs.
fn dialect_of(dbms: DbmsKind) -> DialectId {
    match dbms {
        DbmsKind::MySql => DialectId::MySql,
        DbmsKind::MariaDb => DialectId::MariaDb,
        DbmsKind::Sqlite => DialectId::Sqlite,
        _ => DialectId::Postgres,
    }
}

/// `r` merged into an empty namespace `ns` (on the blocking pool): a snapshot no publisher
/// stamped (generation 0).
async fn standalone(
    ns: &NsKey,
    dbms: DbmsKind,
    fold: FoldRule,
    r: NamespaceResult,
) -> Result<NamespaceSnapshot, IntroError> {
    let ns = ns.clone();
    let built = tokio::task::spawn_blocking(move || {
        let base = merge::base(ns, dbms, fold)
            .map_err(|_| IntroError::Protocol(Name::from("joined/base")))?;
        let policy = MergePolicy {
            trigger: Trigger::User,
            ..MergePolicy::default()
        };
        let (next, _, _) = merge::merge(Some(&base), r, &policy)
            .map_err(|_| IntroError::Protocol(Name::from("joined/merge")))?
            .into_parts();
        Ok(next)
    })
    .await;
    built.unwrap_or_else(|_| Err(IntroError::Protocol(Name::from("joined/merge"))))
}

// ---- reading ------------------------------------------------------------------------------------

type Row = Vec<Option<Arc<str>>>;

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

fn render_error(e: RenderError) -> IntroError {
    match e {
        RenderError::NotJoinable => IntroError::Meta(MetaError::NotJoinable),
        e => IntroError::Protocol(Name::from(format!("joined/render: {e:?}"))),
    }
}

/// Runs joined section `section` of `pack` with `params` (every chunk of a list bind) and
/// answers its rows; the answer must have the section's declared columns, by name and order.
async fn rows(
    snap: &mut SnapshotTxn<'_>,
    pack: &Pack,
    dbms: DbmsKind,
    section: &str,
    params: Params,
    cancel: &CancellationToken,
) -> Result<Vec<Row>, IntroError> {
    let sec = pack
        .section(section)
        .ok_or_else(|| IntroError::Protocol(Name::from(section)))?;
    let declared: Vec<&str> = sec.returns().iter().map(|c| &*c.name).collect();
    let mut flags = PackFlags::JOINED;
    flags.set(PackFlags::MARIADB, dbms == DbmsKind::MariaDb);
    let ctx = PackCtx {
        dialect: dialect_of(dbms),
        version: 0,
        flavor: dbms,
        mode: Mode::Full,
        level: Level::Structure,
        flags,
        params,
    };
    let queries = pack.render(section, &ctx).map_err(render_error)?;
    let mut out = Vec::new();
    for q in &queries {
        if cancel.is_cancelled() {
            return Err(IntroError::Cancelled);
        }
        let got = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(IntroError::Cancelled),
            got = snap.rows(q.as_meta_query()) => got?,
        };
        let names: Vec<&str> = got.columns.iter().map(|c| &**c).collect();
        if names != declared {
            return Err(IntroError::Protocol(Name::from(section)));
        }
        out.extend(got.rows);
    }
    Ok(out)
}

fn cell<'r>(row: &'r Row, i: usize, section: &str) -> Result<&'r str, IntroError> {
    row.get(i)
        .and_then(|c| c.as_deref())
        .ok_or_else(|| IntroError::Protocol(Name::from(section)))
}

fn opt_cell(row: &Row, i: usize) -> Option<&str> {
    row.get(i).and_then(|c| c.as_deref())
}

fn number<T: std::str::FromStr>(row: &Row, i: usize, section: &str) -> Result<T, IntroError> {
    cell(row, i, section)?
        .parse()
        .map_err(|_| IntroError::Protocol(Name::from(section)))
}

fn boolean(row: &Row, i: usize, section: &str) -> Result<bool, IntroError> {
    match cell(row, i, section)? {
        "t" | "true" | "1" => Ok(true),
        "f" | "false" | "0" => Ok(false),
        _ => Err(IntroError::Protocol(Name::from(section))),
    }
}

/// The top-level names `keys` of `ns` stand for (a column's key names its relation).
fn relation_names(ns: &NsKey, keys: &[ObjectKey]) -> Vec<Name> {
    let mut out: Vec<Name> = Vec::new();
    for k in keys {
        if let Some(rest) = k.path.strip_prefix(ns.path.as_slice())
            && let Some(first) = rest.first()
            && !out.contains(first)
        {
            out.push(first.clone());
        }
    }
    out
}

/// One relation read, with its columns.
struct Relation {
    obj: Obj,
    columns: Vec<Obj>,
}

/// The result of a joined read of `names` that found `found` (scoped existence: a requested
/// name not found is gone from the transaction's view).
fn result_of(ns: &NsKey, found: Vec<Relation>, level: Level) -> NamespaceResult {
    let root = ObjectKey {
        kind: ns.kind.root_kind(),
        path: ns.path.iter().cloned().collect(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    };
    let changed = found
        .into_iter()
        .map(|r| {
            let mut tree = Subtree::new(r.obj);
            let top = tree.root();
            for c in r.columns {
                tree.add(top, c);
            }
            ChangedObject {
                parent: root.clone(),
                subtree: tree,
            }
        })
        .collect();
    NamespaceResult {
        existent: BTreeMap::new(),
        changed,
        categories_to_reload: Default::default(),
        new_watermark: Watermark::None,
        full: false,
        level,
        read_at_ms: 0,
    }
}

/// A column object.
fn column(
    name: &str,
    ordinal: u32,
    raw_type: Name,
    type_ref: Option<TypeRef>,
    nullable: bool,
    identity: Option<Identity>,
) -> Obj {
    let props = ColumnProps {
        ordinal,
        raw_type,
        type_ref,
        nullable,
        default_raw: None,
        comment: None,
        hidden: false,
        rare: identity.map(|i| {
            Arc::new(ColumnRare {
                identity: Some(i),
                ..ColumnRare::default()
            })
        }),
        ext: None,
    };
    Obj::new(ObjectKind::Column, name).with_props(Props::Column(Arc::new(props)))
}

/// The props of a relation of `kind` read with no detail.
fn relation_props(kind: ObjectKind) -> Props {
    match kind {
        ObjectKind::View | ObjectKind::MaterializedView => Props::View(Arc::default()),
        ObjectKind::Table | ObjectKind::ForeignTable => {
            Props::Table(Arc::new(TableProps::default()))
        }
        _ => Props::None,
    }
}

/// Whether a relation of `kind` holds columns in the model.
fn has_columns(kind: ObjectKind) -> bool {
    matches!(
        kind,
        ObjectKind::Table
            | ObjectKind::View
            | ObjectKind::MaterializedView
            | ObjectKind::ForeignTable
    )
}

// ---- PostgreSQL ---------------------------------------------------------------------------------

/// Whether `current_schemas(true)` (in order) starts with `pg_catalog` once the session's
/// **own** temporary schema `own_temp` (`pg_my_temp_schema()`, `None` when the session has
/// none) is skipped (R-W3-11 as corrected by R-W5-5): PostgreSQL never resolves an operator or
/// a function in the session's own temporary schema, but it does in any other backend's
/// `pg_temp_N` named in the path. Anything else before `pg_catalog` (another backend's temp
/// schema, a `pg_toast_temp_N`, a user schema) refuses the path.
pub fn pg_path_is_safe(path: &[&str], own_temp: Option<&str>) -> bool {
    let first = path.iter().find(|s| Some(**s) != own_temp);
    first == Some(&"pg_catalog")
}

fn pg_kind(relkind: &str) -> Option<ObjectKind> {
    Some(match relkind {
        "r" | "p" => ObjectKind::Table,
        "v" => ObjectKind::View,
        "m" => ObjectKind::MaterializedView,
        "f" => ObjectKind::ForeignTable,
        _ => return None,
    })
}

async fn pg(
    c: &mut MetaConn,
    ns: &NsKey,
    keys: &[ObjectKey],
    level: Level,
    cancel: &CancellationToken,
) -> Result<NamespaceResult, IntroError> {
    if ns.kind != NsKind::Schema || ns.path.len() != 2 {
        return Err(IntroError::Protocol(Name::from("joined/pg namespace")));
    }
    let (db, schema) = (&ns.path[0], &ns.path[1]);
    let pack = Pack::builtin(DialectId::Postgres)
        .map_err(|_| IntroError::Protocol(Name::from("pg/pack")))?;
    let names = relation_names(ns, keys);
    let mut snap = c.begin_snapshot().await?;
    let d = DbmsKind::Postgres;
    let context = rows(
        &mut snap,
        &pack,
        d,
        pg_name::CONTEXT,
        Params::default(),
        cancel,
    )
    .await?;
    let on_db = context.iter().all(|r| opt_cell(r, 0) == Some(&**db));
    let path: Vec<&str> = context.iter().filter_map(|r| opt_cell(r, 1)).collect();
    // Every row carries the same own temp schema (a scalar subquery of the same snapshot).
    let own: Vec<Option<&str>> = context.iter().map(|r| opt_cell(r, 2)).collect();
    let own_temp = own.first().copied().flatten();
    let consistent = own.iter().all(|o| *o == own_temp);
    if context.is_empty()
        || !on_db
        || !consistent
        || path.len() != context.len()
        || !pg_path_is_safe(&path, own_temp)
    {
        return Err(IntroError::Meta(MetaError::NotJoinable));
    }
    let mut found: Vec<Relation> = Vec::new();
    // The schema's oid, picked by exact name (no name reaches the server); a schema the
    // transaction does not see holds nothing.
    let mut ns_oid = None;
    if !names.is_empty() {
        for r in rows(
            &mut snap,
            &pack,
            d,
            pg_name::NAMESPACES,
            Params::default(),
            cancel,
        )
        .await?
        {
            let s = pg_name::NAMESPACES;
            if *cell(&r, 1, s)? == **schema {
                ns_oid = Some(number::<u32>(&r, 0, s)?);
            }
        }
    }
    if let Some(ns_oid) = ns_oid {
        let params = Params {
            scalars: Vec::new(),
            lists: vec![(Name::from("oids"), vec![text(&ns_oid.to_string())])],
        };
        for r in rows(&mut snap, &pack, d, pg_name::RELATIONS, params, cancel).await? {
            let s = pg_name::RELATIONS;
            // The schema's relations are listed; the caller's are picked here, by exact name.
            let relname = cell(&r, 1, s)?;
            if !names.iter().any(|n| **n == *relname) {
                continue;
            }
            let Some(kind) = pg_kind(cell(&r, 2, s)?) else {
                continue;
            };
            let oid: u32 = number(&r, 0, s)?;
            found.push(Relation {
                obj: Obj::new(kind, cell(&r, 1, s)?)
                    .with_server_id(ServerId::Oid(oid))
                    .with_props(relation_props(kind)),
                columns: Vec::new(),
            });
        }
    }
    if level >= Level::Structure && !found.is_empty() {
        let oids: Vec<BindValue> = found
            .iter()
            .filter_map(|r| match r.obj.server_id() {
                Some(ServerId::Oid(o)) => Some(text(&o.to_string())),
                _ => None,
            })
            .collect();
        let params = Params {
            scalars: Vec::new(),
            lists: vec![(Name::from("oids"), oids)],
        };
        for r in rows(&mut snap, &pack, d, pg_name::COLUMNS, params, cancel).await? {
            let s = pg_name::COLUMNS;
            let relid: u32 = number(&r, 0, s)?;
            let attnum: i16 = number(&r, 1, s)?;
            let type_name = Name::from(cell(&r, 4, s)?);
            let identity = match opt_cell(&r, 6) {
                Some("a") => Some(Identity::Always),
                Some("d") => Some(Identity::ByDefault),
                _ => None,
            };
            let col = column(
                cell(&r, 2, s)?,
                u32::try_from(attnum).map_err(|_| IntroError::Protocol(Name::from(s)))?,
                type_name.clone(),
                Some(TypeRef {
                    schema: Some(Name::from(cell(&r, 3, s)?)),
                    name: type_name,
                }),
                !boolean(&r, 5, s)?,
                identity,
            );
            if let Some(rel) = found
                .iter_mut()
                .find(|x| x.obj.server_id() == Some(ServerId::Oid(relid)))
            {
                rel.columns.push(col);
            }
        }
    }
    snap.commit().await?;
    Ok(result_of(ns, found, level))
}

// ---- MySQL and MariaDB ---------------------------------------------------------------------------

async fn mysql(
    c: &mut MetaConn,
    dbms: DbmsKind,
    ns: &NsKey,
    keys: &[ObjectKey],
    level: Level,
    cancel: &CancellationToken,
) -> Result<NamespaceResult, IntroError> {
    if ns.kind != NsKind::Schema || ns.path.len() != 1 {
        return Err(IntroError::Protocol(Name::from("joined/mysql namespace")));
    }
    let schema = &ns.path[0];
    let pack = Pack::builtin(dialect_of(dbms))
        .map_err(|_| IntroError::Protocol(Name::from("mysql/pack")))?;
    let names = relation_names(ns, keys);
    let mut found: Vec<Relation> = Vec::new();
    if names.is_empty() {
        return Ok(result_of(ns, found, level));
    }
    let params = || Params {
        scalars: vec![(Name::from("schema"), text(schema))],
        lists: vec![(Name::from("names"), names.iter().map(|n| text(n)).collect())],
    };
    let mut snap = c.begin_snapshot().await?;
    for r in rows(&mut snap, &pack, dbms, mysql_name::TABLES, params(), cancel).await? {
        let s = mysql_name::TABLES;
        // T17's mapping (review m-6): MariaDB sequences are their own kind; a type this build
        // does not model is left out (the joined read describes only what it can).
        let kind = crate::mysql::build::relation_kind(cell(&r, 1, s)?, dbms == DbmsKind::MariaDb);
        if kind == ObjectKind::Unknown {
            continue;
        }
        found.push(Relation {
            obj: Obj::new(kind, cell(&r, 0, s)?).with_props(relation_props(kind)),
            columns: Vec::new(),
        });
    }
    if level >= Level::Structure && !found.is_empty() {
        for r in rows(
            &mut snap,
            &pack,
            dbms,
            mysql_name::COLUMNS,
            params(),
            cancel,
        )
        .await?
        {
            let s = mysql_name::COLUMNS;
            let table = cell(&r, 0, s)?;
            let col = column(
                cell(&r, 1, s)?,
                number(&r, 2, s)?,
                Name::from(cell(&r, 3, s)?),
                None,
                cell(&r, 4, s)? == "YES",
                None,
            );
            // INFORMATION_SCHEMA answers the server's own spelling of each table.
            if let Some(rel) = found
                .iter_mut()
                .find(|x| &**x.obj.name() == table && has_columns(x.obj.kind()))
            {
                rel.columns.push(col);
            }
        }
    }
    snap.commit().await?;
    Ok(result_of(ns, found, level))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_path_must_start_with_pg_catalog_after_the_sessions_temp_schema() {
        let own = Some("pg_temp_3");
        assert!(pg_path_is_safe(&["pg_catalog", "public"], None));
        assert!(pg_path_is_safe(&["pg_catalog", "public"], own));
        assert!(pg_path_is_safe(&["pg_temp_3", "pg_catalog", "public"], own));
        assert!(!pg_path_is_safe(&["public", "pg_catalog"], own));
        assert!(!pg_path_is_safe(&["pg_temp_3", "evil", "pg_catalog"], own));
        // Only the session's own temp schema is skipped (R-W5-5): another backend's is
        // searched for operators, and so is refused.
        assert!(!pg_path_is_safe(&["pg_temp_7", "pg_catalog"], own));
        assert!(!pg_path_is_safe(
            &["pg_temp_3", "pg_temp_7", "pg_catalog"],
            own
        ));
        assert!(!pg_path_is_safe(&["pg_temp_3", "pg_catalog"], None));
        assert!(!pg_path_is_safe(&["pg_toast_temp_3", "pg_catalog"], own));
        assert!(!pg_path_is_safe(&["pg_temp_x", "pg_catalog"], own));
        assert!(!pg_path_is_safe(&["PG_CATALOG"], own));
        assert!(!pg_path_is_safe(&[], own));
    }

    #[tokio::test]
    async fn a_joined_read_is_a_standalone_snapshot_no_publisher_stamped() {
        let ns = NsKey::schema([Name::from("db"), Name::from("s")]);
        let int4 = TypeRef {
            schema: Some(Name::from("pg_catalog")),
            name: Name::from("int4"),
        };
        let t = Relation {
            obj: Obj::new(ObjectKind::Table, "t")
                .with_server_id(ServerId::Oid(7))
                .with_props(relation_props(ObjectKind::Table)),
            columns: vec![column(
                "c",
                1,
                Name::from("int4"),
                Some(int4.clone()),
                true,
                Some(Identity::Always),
            )],
        };
        let snap = standalone(
            &ns,
            DbmsKind::Postgres,
            FoldRule::Exact,
            result_of(&ns, vec![t], Level::Structure),
        )
        .await
        .expect("built");
        assert_eq!(snap.generation(), 0);
        assert_eq!(snap.key(), &ns);
        let key = |kind, path: &[&str], oid| ObjectKey {
            kind,
            path: path.iter().map(|p| Name::from(*p)).collect(),
            server_id: oid,
            overload_sig: None,
            raw_kind: None,
        };
        assert!(
            snap.find(&key(
                ObjectKind::Table,
                &["db", "s", "t"],
                Some(ServerId::Oid(7))
            ))
            .is_some()
        );
        let c = snap
            .find(&key(ObjectKind::Column, &["db", "s", "t", "c"], None))
            .and_then(|i| snap.obj(i))
            .expect("the column");
        let Props::Column(p) = c.props() else {
            panic!("column props");
        };
        assert_eq!(p.type_ref, Some(int4));
        assert_eq!(&*p.raw_type, "int4");
        assert_eq!(p.ordinal, 1);
    }

    #[test]
    fn a_columns_key_stands_for_its_relation() {
        let ns = NsKey::schema([Name::from("db"), Name::from("s")]);
        let key = |path: &[&str], kind| ObjectKey {
            kind,
            path: path.iter().map(|p| Name::from(*p)).collect(),
            server_id: None,
            overload_sig: None,
            raw_kind: None,
        };
        assert_eq!(
            relation_names(
                &ns,
                &[
                    key(&["db", "s", "t", "c"], ObjectKind::Column),
                    key(&["db", "s", "t"], ObjectKind::Table),
                    key(&["db", "s", "u"], ObjectKind::View),
                    key(&["db", "other", "x"], ObjectKind::Table),
                ]
            ),
            vec![Name::from("t"), Name::from("u")]
        );
    }
}
