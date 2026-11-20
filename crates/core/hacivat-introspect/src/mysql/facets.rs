// ## m4-t01c-core (seam), filled by m4-t17b-mysql-facets
//! MySQL and MariaDB facets and the server-level namespace (R-M4-30; Task 17b): grants,
//! view dependencies and server text (`SHOW CREATE`) of the objects of one schema, and the
//! roles of the `NsKind::Server` namespace. Every read is one snapshot of the metadata
//! connection (`Identity` first, then the sections of `packs/mysql/20-facets.hvq.sql`, or
//! Task 17's `show-create` sections), written against [`SectionReader`] so every decision
//! runs without a server in the unit tests.
//!
//! **What a read answers** (rulings R-W4-3, R-W4-4). [`read`] answers [`FacetRows`] for the
//! keys it could read completely; a key it leaves out is not loaded (the engine keeps the facet
//! `NotLoaded`, so a DDL script says "grants not loaded", never a script that silently lacks
//! them). A read the login cannot make complete is an error for the whole read
//! ([`IntroError::Permission`], published as `Failed`), never an empty or a partial answer
//! presented as complete.
//!
//! **Grants** are read from the grant tables (`mysql.db`, `mysql.tables_priv`,
//! `mysql.columns_priv`), never from the `INFORMATION_SCHEMA` privilege views: those show
//! another account's rows only to a login with `SELECT` on the `mysql` schema (so a login
//! without it sees a partial list it cannot tell apart from the whole one), and print the
//! grantee as one quoted string the server does not escape (a user name may hold a quote).
//! A grant-table read answers every row or is refused (`1142`): complete or `Permission`.
//! `mysql.user` and `mysql.global_priv` are never read. The grantee is built here from the
//! separate `User` and `Host` columns as the account literal `'user'@'host'` (a quote
//! doubled), the form Task 19's renderer accepts. Not loaded ([`ObjGrants::opaque`]): a user
//! or host holding a backslash or NUL (no portable literal), a MariaDB row with an empty host
//! (a role or an account: the grant tables cannot tell), a privilege outside the server's
//! documented set, a grant option with no privilege, and a schema whose grant row is stored in
//! its escaped form (`hvm4\_db`: re-granting `ON hvm4_db.*` would widen it to a pattern).
//! Database-level grants are read on MySQL 8 only (MariaDB's `mysql.db` has columns MySQL's
//! lacks; a fixed column list could miss one). Routine grants are not read (no key answered).
//! Table names compare exactly only under `lower_case_table_names = 0`; on another setting no
//! key is answered.
//!
//! **Dependencies** of a view are the rows of `VIEW_TABLE_USAGE` and `VIEW_ROUTINE_USAGE`
//! (MySQL 8.0.13+; MariaDB has neither: `NotImplemented`), [`DepsKind::Usage`]. The manual: "You
//! can see information only for views for which you have some privilege, and only for tables
//! [functions] for which you have some privilege." A row the login may not see cannot be told
//! from no row, so the read is complete only when the login holds `SELECT` and `EXECUTE` at the
//! global level, as `USER_PRIVILEGES` lists them for its own account, and `partial_revokes` is
//! off ([`coverage`]); otherwise the read is `Permission { View }`. A view the login cannot see
//! (no `VIEWS` row) and a used relation without a `TABLES` row are not answered.
//!
//! **Sources** are the `SHOW CREATE` statement of tables, views, routines, triggers and
//! events (R-M4-24, R-M4-37; Task 17's `show_create`). A missing object, or a form the server
//! will not prepare, is not answered; a refused one fails the read.
//!
//! **Roles** ([`server_namespace`]): `APPLICABLE_ROLES` (MySQL 8.0.19+, MariaDB) and the
//! current account. That is not every account of the server, so the result's level is `Names`
//! and a name it does not list is `Unknown`, never missing; a refused read is
//! `Permission { Role }`. A server without the view reports the role set as failed.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

use futures::future::BoxFuture;
use hacivat_meta::merge::{ChangedObject, ExistSet, Identity, NamespaceResult};
use hacivat_meta::props::{DepsFacet, DepsKind, FacetKind, Grant, GrantsFacet};
use hacivat_meta::snapshot::IntroErrorCode;
use hacivat_meta::{
    Level, Name, NsKey, NsKind, Obj, ObjectKey, ObjectKind, Ref, Subtree, Watermark,
};
use hacivat_proto::DbmsKind;
use hacivat_session::meta::{MetaConn, MetaError, Rows};
use smallvec::SmallVec;

use super::TxnReader;
use super::build::relation_kind;
use super::pass::{SectionReader, list_params, read_server, schema_of};
use super::rows::{self, Row, cell};
use super::server::ServerCtx;
use super::show_create::{is_not_preparable, section_of, show_create};
use crate::introspector::{FacetRows, IntroError};
use crate::pack::{Mode, Params};

/// The section names of `packs/mysql/20-facets.hvq.sql`.
pub mod name {
    pub const GRANTS_TABLE: &str = "GrantsTable";
    pub const GRANTS_COLUMN: &str = "GrantsColumn";
    pub const GRANTS_SCHEMA: &str = "GrantsSchema";
    pub const DEPS_VIEWS: &str = "DepsViews";
    pub const DEPS_VIEW_TABLES: &str = "DepsViewTables";
    pub const DEPS_VIEW_ROUTINES: &str = "DepsViewRoutines";
    pub const DEPS_COVERAGE: &str = "DepsCoverage";
    pub const PARTIAL_REVOKES: &str = "PartialRevokes";
    pub const LIST_ROLES: &str = "ListRoles";
    /// Every section of the file.
    pub const ALL: [&str; 9] = [
        GRANTS_TABLE,
        GRANTS_COLUMN,
        GRANTS_SCHEMA,
        DEPS_VIEWS,
        DEPS_VIEW_TABLES,
        DEPS_VIEW_ROUTINES,
        DEPS_COVERAGE,
        PARTIAL_REVOKES,
        LIST_ROLES,
    ];
    /// The sections that read the grant tables (a login needs `SELECT` on them).
    pub const GRANT_TABLES: [&str; 3] = [GRANTS_TABLE, GRANTS_COLUMN, GRANTS_SCHEMA];
}

/// The facets [`read`] answers on `flavor` (`IntroCaps.facets`): MariaDB has no usage views,
/// so no `Dependencies`.
pub fn facets_for(flavor: DbmsKind) -> Vec<FacetKind> {
    if flavor == DbmsKind::MariaDb {
        vec![FacetKind::Sources, FacetKind::Grants]
    } else {
        vec![
            FacetKind::Sources,
            FacetKind::Grants,
            FacetKind::Dependencies,
        ]
    }
}

/// The server's refusals that mean "this login may not read it" (MySQL and MariaDB error
/// numbers): 1044 database access denied, 1142 table, 1143 column, 1227 a privilege the
/// statement needs, 1370 routine.
pub const PERMISSION_CODES: [i64; 5] = [1044, 1142, 1143, 1227, 1370];

/// The server's "no such object" errors a `SHOW CREATE` of a dropped object answers: 1049
/// database, 1146 table, 1305 routine, 1360 trigger, 1539 event.
pub const GONE_CODES: [i64; 5] = [1049, 1146, 1305, 1360, 1539];

fn vendor_code(e: &IntroError) -> Option<i64> {
    match e {
        IntroError::Meta(MetaError::Server { message, .. }) => message.vendor_code,
        _ => None,
    }
}

/// `e` as [`IntroError::Permission`] of `kind` when the server refused the read for lack of a
/// privilege; any other error unchanged.
pub fn permission(e: IntroError, kind: ObjectKind) -> IntroError {
    match vendor_code(&e) {
        Some(c) if PERMISSION_CODES.contains(&c) => IntroError::Permission { kind },
        _ => match e {
            IntroError::Permission { .. } => IntroError::Permission { kind },
            e => e,
        },
    }
}

fn read_at_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// The rows of `section` (`width` wide); an empty answer when the section renders no
/// statement on this server (a version or flavor gate).
async fn read_rows(
    r: &mut dyn SectionReader,
    srv: &ServerCtx,
    section: &'static str,
    params: Params,
    width: usize,
) -> Result<Option<Rows>, IntroError> {
    if !renders(srv, section) {
        return Ok(None);
    }
    let rows = r
        .read(section, srv.pack_ctx(Mode::Full, Level::Structure, params))
        .await?;
    rows::checked(section, &rows, width)?;
    Ok(Some(rows))
}

/// Whether `section` holds a statement on `srv` (the pack's gates, mirrored: a gated section
/// renders nothing, which the reader would answer as zero rows).
pub fn renders(srv: &ServerCtx, section: &str) -> bool {
    match section {
        name::GRANTS_SCHEMA => !srv.is_mariadb() && srv.version_num < 90_000,
        name::DEPS_VIEW_TABLES | name::DEPS_VIEW_ROUTINES => srv.has_view_usage(),
        name::PARTIAL_REVOKES => srv.mysql_at_least(80_016),
        name::LIST_ROLES => srv.is_mariadb() || srv.mysql_at_least(80_019),
        _ => true,
    }
}

// ---- accounts ----------------------------------------------------------------------------------

/// The account literal `'user'@'host'` of a grant-table row (a quote doubled, as the server
/// prints accounts and Task 19's renderer accepts). `None` when no portable literal exists: a
/// backslash (an escape under the default `sql_mode`) or NUL in either part, and on MariaDB an
/// empty host (a role's rows have one; the grant tables cannot tell a role from a user).
pub fn account(user: &str, host: &str, mariadb: bool) -> Option<Name> {
    let bad = |s: &str| s.contains(['\\', '\0']);
    if bad(user) || bad(host) || (mariadb && host.is_empty()) {
        return None;
    }
    Some(Name::from(format!(
        "'{}'@'{}'",
        user.replace('\'', "''"),
        host.replace('\'', "''")
    )))
}

/// The login's account as `(user, host)` from `CURRENT_USER()` (`user@host`). `None` unless the
/// text holds exactly one `@`: a user or host holding one could be split two ways.
pub fn login_parts(current_user: &str) -> Option<(&str, &str)> {
    let mut it = current_user.split('@');
    let (u, h) = (it.next()?, it.next()?);
    it.next().is_none().then_some((u, h))
}

// ---- grants -----------------------------------------------------------------------------------

/// The keyword of a `Table_priv` set element (MySQL 8.4 manual, Grant Tables, "Set-Type
/// Privilege Column Values"; MariaDB adds `Delete versioning rows`, granted as `DELETE
/// HISTORY`). `Grant` is the grant option, handled apart. `None`: not a documented element.
pub fn table_privilege(element: &str) -> Option<&'static str> {
    Some(match element {
        "Select" => "SELECT",
        "Insert" => "INSERT",
        "Update" => "UPDATE",
        "Delete" => "DELETE",
        "Create" => "CREATE",
        "Drop" => "DROP",
        "References" => "REFERENCES",
        "Index" => "INDEX",
        "Alter" => "ALTER",
        "Create View" => "CREATE VIEW",
        "Show view" => "SHOW VIEW",
        "Trigger" => "TRIGGER",
        "Delete versioning rows" => "DELETE HISTORY",
        _ => return None,
    })
}

/// The keyword of a `Column_priv` set element (same source). `None`: not documented.
pub fn column_privilege(element: &str) -> Option<&'static str> {
    Some(match element {
        "Select" => "SELECT",
        "Insert" => "INSERT",
        "Update" => "UPDATE",
        "References" => "REFERENCES",
        _ => return None,
    })
}

/// The privilege columns of `mysql.db` (MySQL 8.4 manual, Grant Tables, table "db Table
/// Columns"), in `GrantsSchema`'s order after the three scope columns, with their keywords;
/// `None` is `Grant_priv` (the grant option).
pub const SCHEMA_PRIVILEGES: [Option<&str>; 19] = [
    Some("SELECT"),
    Some("INSERT"),
    Some("UPDATE"),
    Some("DELETE"),
    Some("CREATE"),
    Some("DROP"),
    None,
    Some("REFERENCES"),
    Some("INDEX"),
    Some("ALTER"),
    Some("CREATE TEMPORARY TABLES"),
    Some("LOCK TABLES"),
    Some("CREATE VIEW"),
    Some("SHOW VIEW"),
    Some("CREATE ROUTINE"),
    Some("ALTER ROUTINE"),
    Some("EXECUTE"),
    Some("EVENT"),
    Some("TRIGGER"),
];

/// Why an object's grants are not loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opaque {
    /// A grantee with no portable account literal ([`account`]).
    Account { user: String, host: String },
    /// A privilege element outside the documented set, or a value that is not `Y`/`N`.
    Privilege(String),
    /// The grant option with no privilege it applies to.
    GrantOptionOnly,
    /// A column grant whose account has no `tables_priv` row (its grant option is unknown).
    NoTableRow,
    /// A `mysql.db` row stored in the schema name's escaped (`\_`, `\%`) form.
    EscapedSchemaRow,
    /// `partial_revokes` is not off (fix round 1, I-1): a database-level `REVOKE` of a global
    /// privilege is then stored in `mysql.user.User_attributes`, which is never read.
    PartialRevokes,
}

/// One object's grants as read: the facet, and why it is incomplete (empty: complete).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjGrants {
    pub key: ObjectKey,
    pub facet: GrantsFacet,
    pub opaque: Vec<Opaque>,
}

impl ObjGrants {
    pub fn complete(&self) -> bool {
        self.opaque.is_empty()
    }
}

/// What a grants read selects `key` by: a table-level object (its name), or the namespace's
/// own schema. `None`: not an object of `ns` whose grants this read covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantTarget {
    Relation,
    Schema,
}

/// The grant target of `key` in `ns` (module docs).
pub fn grant_target(ns: &NsKey, key: &ObjectKey) -> Option<GrantTarget> {
    let [schema] = ns.path.as_slice() else {
        return None;
    };
    if ns.kind != NsKind::Schema {
        return None;
    }
    match (key.kind, key.path.as_slice()) {
        (ObjectKind::Schema, [s]) if s == schema => Some(GrantTarget::Schema),
        (ObjectKind::Table | ObjectKind::View | ObjectKind::Sequence, [s, _]) if s == schema => {
            Some(GrantTarget::Relation)
        }
        _ => None,
    }
}

/// The escaped form of a schema name in a database-level grant (`_`, `%` and `\` behind a
/// backslash), the second spelling `mysql.db` may hold for it.
pub fn escaped_schema(schema: &str) -> String {
    let mut s = String::with_capacity(schema.len() + 4);
    for c in schema.chars() {
        if matches!(c, '_' | '%' | '\\') {
            s.push('\\');
        }
        s.push(c);
    }
    s
}

fn add(facet: &mut GrantsFacet, grantee: &Name, privilege: &str, columns: Vec<Name>, g: bool) {
    facet.entries.push(Grant {
        grantee: grantee.clone(),
        privilege: Name::from(privilege),
        columns,
        grantable: g,
    });
}

fn sort(facet: &mut GrantsFacet) {
    facet.entries.sort_by(|a, b| {
        (&a.grantee, &a.privilege, &a.columns, a.grantable).cmp(&(
            &b.grantee,
            &b.privilege,
            &b.columns,
            b.grantable,
        ))
    });
    facet.entries.dedup();
}

/// A set column's elements (`Select,Insert`; the documented elements hold no comma).
fn elements(text: Option<&str>) -> impl Iterator<Item = &str> {
    text.unwrap_or("").split(',').filter(|e| !e.is_empty())
}

/// The grants of the relations `keys` from the `GrantsTable` and `GrantsColumn` rows (rows of
/// other tables, or whose table name differs in any byte, are ignored).
pub fn decide_relation_grants(
    keys: &[ObjectKey],
    table_rows: &[Row],
    column_rows: &[Row],
    mariadb: bool,
) -> Vec<ObjGrants> {
    keys.iter()
        .map(|key| {
            let table = key.name();
            let mut out = ObjGrants {
                key: key.clone(),
                facet: GrantsFacet::default(),
                opaque: Vec::new(),
            };
            // (user, host) → grant option of its table row.
            let mut grant_option: HashMap<(&str, &str), bool> = HashMap::new();
            // ## m4-t17b-mysql-facets (fix round 1, m-5): the grant-table scope columns are
            // NOT NULL; a NULL one (a broken or hostile server) is opaque, never `''`.
            let null_scope = |r: &Row, n: usize| (0..n).any(|i| cell(r, i).is_none());
            for r in table_rows.iter().chain(column_rows) {
                if null_scope(r, 3) || (r.len() == 5 && cell(r, 3).is_none()) {
                    out.opaque.push(Opaque::Account {
                        user: cell(r, 0).unwrap_or("").to_string(),
                        host: cell(r, 1).unwrap_or("").to_string(),
                    });
                }
            }
            for r in table_rows.iter().filter(|r| cell(r, 2) == Some(table)) {
                let (Some(user), Some(host)) = (cell(r, 0), cell(r, 1)) else {
                    continue;
                };
                let mut privs = Vec::new();
                let mut g = false;
                for e in elements(cell(r, 3)) {
                    match (e, table_privilege(e)) {
                        ("Grant", _) => g = true,
                        (_, Some(p)) => privs.push(p),
                        (e, None) => out.opaque.push(Opaque::Privilege(e.to_string())),
                    }
                }
                grant_option.insert((user, host), g);
                let Some(grantee) = account(user, host, mariadb) else {
                    out.opaque.push(Opaque::Account {
                        user: user.to_string(),
                        host: host.to_string(),
                    });
                    continue;
                };
                let has_cols = column_rows.iter().any(|c| {
                    cell(c, 2) == Some(table)
                        && cell(c, 0) == Some(user)
                        && cell(c, 1) == Some(host)
                });
                if g && privs.is_empty() && !has_cols {
                    out.opaque.push(Opaque::GrantOptionOnly);
                }
                for p in privs {
                    add(&mut out.facet, &grantee, p, Vec::new(), g);
                }
            }
            // (user, host, privilege) → columns.
            let mut cols: BTreeMap<(&str, &str, &'static str), BTreeSet<Name>> = BTreeMap::new();
            for r in column_rows.iter().filter(|r| cell(r, 2) == Some(table)) {
                let (Some(user), Some(host), Some(column)) = (cell(r, 0), cell(r, 1), cell(r, 3))
                else {
                    continue;
                };
                let column = Name::from(column);
                for e in elements(cell(r, 4)) {
                    match column_privilege(e) {
                        Some(p) => {
                            cols.entry((user, host, p))
                                .or_default()
                                .insert(column.clone());
                        }
                        None => out.opaque.push(Opaque::Privilege(e.to_string())),
                    }
                }
            }
            for ((user, host, p), columns) in cols {
                let Some(&g) = grant_option.get(&(user, host)) else {
                    out.opaque.push(Opaque::NoTableRow);
                    continue;
                };
                match account(user, host, mariadb) {
                    Some(grantee) => add(
                        &mut out.facet,
                        &grantee,
                        p,
                        columns.into_iter().collect(),
                        g,
                    ),
                    None => out.opaque.push(Opaque::Account {
                        user: user.to_string(),
                        host: host.to_string(),
                    }),
                }
            }
            sort(&mut out.facet);
            out
        })
        .collect()
}

/// The database-level grants of schema `key` (named `schema`) from the `GrantsSchema` rows.
pub fn decide_schema_grants(key: &ObjectKey, schema: &str, rows: &[Row]) -> ObjGrants {
    let mut out = ObjGrants {
        key: key.clone(),
        facet: GrantsFacet::default(),
        opaque: Vec::new(),
    };
    for r in rows {
        // Fix round 1, m-5: a NULL scope column is opaque.
        let (Some(user), Some(host), Some(db)) = (cell(r, 0), cell(r, 1), cell(r, 2)) else {
            out.opaque.push(Opaque::Account {
                user: cell(r, 0).unwrap_or("").to_string(),
                host: cell(r, 1).unwrap_or("").to_string(),
            });
            continue;
        };
        if db != schema {
            if db == escaped_schema(schema) {
                out.opaque.push(Opaque::EscapedSchemaRow);
            }
            continue;
        }
        let mut privs = Vec::new();
        let mut g = false;
        for (i, kw) in SCHEMA_PRIVILEGES.iter().enumerate() {
            match (cell(r, 3 + i), kw) {
                (Some("N"), _) => {}
                (Some("Y"), Some(p)) => privs.push(*p),
                (Some("Y"), None) => g = true,
                (other, _) => out
                    .opaque
                    .push(Opaque::Privilege(other.unwrap_or("NULL").to_string())),
            }
        }
        if g && privs.is_empty() {
            out.opaque.push(Opaque::GrantOptionOnly);
        }
        match account(user, host, false) {
            Some(grantee) => {
                for p in privs {
                    add(&mut out.facet, &grantee, p, Vec::new(), g);
                }
            }
            None => out.opaque.push(Opaque::Account {
                user: user.to_string(),
                host: host.to_string(),
            }),
        }
    }
    sort(&mut out.facet);
    out
}

/// The grants of `keys` in `ns` (every key the read covers, complete or not; module docs). A
/// refused grant-table read is `Permission { Table }` for the whole read.
pub async fn grants(
    r: &mut dyn SectionReader,
    srv: &ServerCtx,
    ns: &NsKey,
    keys: &[ObjectKey],
) -> Result<Vec<ObjGrants>, IntroError> {
    if srv.lower_case_table_names != 0 {
        return Ok(Vec::new());
    }
    let schema = schema_of(ns)?;
    let mut rels: Vec<ObjectKey> = Vec::new();
    let mut own: Option<ObjectKey> = None;
    for k in keys {
        match grant_target(ns, k) {
            Some(GrantTarget::Relation) if !rels.contains(k) => rels.push(k.clone()),
            Some(GrantTarget::Schema) => own = Some(k.clone()),
            _ => {}
        }
    }
    let refused = |e| permission(e, ObjectKind::Table);
    let mut out = Vec::new();
    if !rels.is_empty() {
        let names: BTreeSet<Name> = rels.iter().map(|k| Name::from(k.name())).collect();
        let t = read_rows(r, srv, name::GRANTS_TABLE, list_params(&schema, &names), 4)
            .await
            .map_err(refused)?
            .unwrap_or_default();
        let c = read_rows(r, srv, name::GRANTS_COLUMN, list_params(&schema, &names), 5)
            .await
            .map_err(refused)?
            .unwrap_or_default();
        out.extend(decide_relation_grants(
            &rels,
            &t.rows,
            &c.rows,
            srv.is_mariadb(),
        ));
    }
    if let Some(key) = own
        && renders(srv, name::GRANTS_SCHEMA)
    {
        // ## m4-t17b-mysql-facets (fix round 1, I-1): read in the same snapshot. Only "off"
        // proves no restriction exists (MySQL refuses to turn it off while one does); a
        // server without the variable (before 8.0.16) has no partial revokes.
        if renders(srv, name::PARTIAL_REVOKES) {
            let pr = read_rows(r, srv, name::PARTIAL_REVOKES, Params::default(), 1)
                .await
                .map_err(refused)?
                .unwrap_or_default();
            if !matches!(pr.rows.first().and_then(|r| cell(r, 0)), Some("0" | "OFF")) {
                out.push(ObjGrants {
                    key,
                    facet: GrantsFacet::default(),
                    opaque: vec![Opaque::PartialRevokes],
                });
                return Ok(out);
            }
        }
        let spellings = [schema.clone(), Name::from(escaped_schema(&schema))];
        let rows = read_rows(
            r,
            srv,
            name::GRANTS_SCHEMA,
            list_params(&schema, &spellings),
            22,
        )
        .await
        .map_err(refused)?
        .unwrap_or_default();
        out.push(decide_schema_grants(&key, &schema, &rows.rows));
    }
    Ok(out)
}

// ---- dependencies -----------------------------------------------------------------------------

/// Whether the usage views can show this login every row (module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Coverage {
    Complete,
    /// The reason, for the report and the tests.
    Incomplete(&'static str),
}

/// [`Coverage`] from the login (`CURRENT_USER()`), its `USER_PRIVILEGES` rows (grantee,
/// privilege) and `partial_revokes` (`None`: the server has no such variable). Global `SELECT`
/// covers every table and view, global `EXECUTE` every function; partial revokes can take
/// either away below the global level, which no `USER_PRIVILEGES` row shows.
pub fn coverage(
    login: Option<&str>,
    user_privileges: &[Row],
    partial_revokes: Option<&str>,
) -> Coverage {
    if !matches!(partial_revokes, None | Some("0" | "OFF")) {
        return Coverage::Incomplete("partial_revokes");
    }
    let Some((u, h)) = login.and_then(login_parts) else {
        return Coverage::Incomplete("login");
    };
    // `USER_PRIVILEGES.GRANTEE` is printed `'user'@'host'`, unescaped; with one `@` in the
    // login, the text is that account's alone.
    let grantee = format!("'{u}'@'{h}'");
    let has = |p: &str| {
        user_privileges
            .iter()
            .any(|r| cell(r, 0) == Some(&grantee) && cell(r, 1) == Some(p))
    };
    if has("SELECT") && has("EXECUTE") {
        Coverage::Complete
    } else {
        Coverage::Incomplete("global privileges")
    }
}

/// One view's dependencies as read: the facet, and whether every used object was named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjDeps {
    pub key: ObjectKey,
    pub facet: DepsFacet,
    /// A used relation without a `TABLES` row (or of a kind not modelled): not answered.
    pub unnamed: bool,
}

fn rel_ref(kind: ObjectKind, schema: &str, name: &str) -> Ref {
    Ref {
        kind,
        path: [Name::from(schema), Name::from(name)].into_iter().collect(),
        server_id: None,
    }
}

/// The dependencies of the views `keys` that the `DepsViews` rows list, from the
/// `DepsViewTables` and `DepsViewRoutines` rows.
pub fn decide_deps(
    keys: &[ObjectKey],
    views: &[Row],
    tables: &[Row],
    routines: &[Row],
) -> Vec<ObjDeps> {
    keys.iter()
        .filter(|k| views.iter().any(|r| cell(r, 0) == Some(k.name())))
        .map(|key| {
            let view = key.name();
            let mut unnamed = false;
            let mut refs: Vec<Ref> = Vec::new();
            for r in tables.iter().filter(|r| cell(r, 0) == Some(view)) {
                let (Some(s), Some(t)) = (cell(r, 1), cell(r, 2)) else {
                    unnamed = true;
                    continue;
                };
                let kind = cell(r, 3).map_or(ObjectKind::Unknown, |ty| relation_kind(ty, false));
                if kind == ObjectKind::Unknown {
                    unnamed = true;
                    continue;
                }
                refs.push(rel_ref(kind, s, t));
            }
            for r in routines.iter().filter(|r| cell(r, 0) == Some(view)) {
                let (Some(s), Some(f)) = (cell(r, 1), cell(r, 2)) else {
                    unnamed = true;
                    continue;
                };
                refs.push(rel_ref(ObjectKind::Routine, s, f));
            }
            refs.sort_by(|a, b| (a.kind as u16, &a.path).cmp(&(b.kind as u16, &b.path)));
            refs.dedup();
            ObjDeps {
                key: key.clone(),
                facet: DepsFacet {
                    depends_on: refs,
                    kind: DepsKind::Usage,
                },
                unnamed,
            }
        })
        .collect()
}

/// The views of `keys` in `ns` (a view of the namespace's schema).
fn view_keys(ns: &NsKey, keys: &[ObjectKey]) -> Vec<ObjectKey> {
    let mut out: Vec<ObjectKey> = Vec::new();
    for k in keys {
        if k.kind == ObjectKind::View
            && grant_target(ns, k) == Some(GrantTarget::Relation)
            && !out.contains(k)
        {
            out.push(k.clone());
        }
    }
    out
}

/// The dependencies of the views among `keys` and the login's [`Coverage`]. MariaDB and MySQL
/// before 8.0.13 have no usage views: `NotImplemented`.
pub async fn dependencies(
    r: &mut dyn SectionReader,
    srv: &ServerCtx,
    ns: &NsKey,
    keys: &[ObjectKey],
) -> Result<(Vec<ObjDeps>, Coverage), IntroError> {
    if !srv.has_view_usage() {
        return Err(IntroError::NotImplemented);
    }
    let schema = schema_of(ns)?;
    let refused = |e| permission(e, ObjectKind::View);
    let cov_rows = read_rows(r, srv, name::DEPS_COVERAGE, Params::default(), 2)
        .await
        .map_err(refused)?
        .unwrap_or_default();
    let partial = read_rows(r, srv, name::PARTIAL_REVOKES, Params::default(), 1)
        .await
        .map_err(refused)?;
    let partial = match &partial {
        None => None,
        // A server that has the variable answers one row; anything else is not "off".
        Some(rows) => Some(rows.rows.first().and_then(|r| cell(r, 0)).unwrap_or("?")),
    };
    let cov = coverage(srv.login.as_deref(), &cov_rows.rows, partial);
    let views = view_keys(ns, keys);
    if views.is_empty() {
        return Ok((Vec::new(), cov));
    }
    let names: BTreeSet<Name> = views.iter().map(|k| Name::from(k.name())).collect();
    let p = || list_params(&schema, &names);
    let v = read_rows(r, srv, name::DEPS_VIEWS, p(), 1)
        .await
        .map_err(refused)?;
    let t = read_rows(r, srv, name::DEPS_VIEW_TABLES, p(), 4)
        .await
        .map_err(refused)?;
    let f = read_rows(r, srv, name::DEPS_VIEW_ROUTINES, p(), 3)
        .await
        .map_err(refused)?;
    let rows_of = |x: &Option<Rows>| x.as_ref().map(|r| r.rows.clone()).unwrap_or_default();
    Ok((
        decide_deps(&views, &rows_of(&v), &rows_of(&t), &rows_of(&f)),
        cov,
    ))
}

// ---- sources ----------------------------------------------------------------------------------

/// What a `SHOW CREATE` names `key` by in `ns`: `(schema, name)`; a trigger is a table's child
/// (`[schema, table, trigger]`) but named in its schema. `None`: no server text for it.
pub fn source_target<'k>(ns: &NsKey, key: &'k ObjectKey) -> Option<(&'k str, &'k str)> {
    let [schema] = ns.path.as_slice() else {
        return None;
    };
    section_of(key.kind)?;
    match (key.kind, key.path.as_slice()) {
        (ObjectKind::Trigger, [s, _, t]) if s == schema => Some((s, t)),
        (ObjectKind::Trigger, _) => None,
        (_, [s, n]) if s == schema => Some((s, n)),
        _ => None,
    }
}

/// The `SHOW CREATE` text of each of `keys` the server answers (module docs).
pub async fn sources(
    r: &mut dyn SectionReader,
    srv: &ServerCtx,
    ns: &NsKey,
    keys: &[ObjectKey],
) -> Result<Vec<(ObjectKey, Name)>, IntroError> {
    let mut out = Vec::new();
    for key in keys {
        let Some((schema, name)) = source_target(ns, key) else {
            continue;
        };
        match show_create(r, srv, key.kind, schema, name).await {
            Ok(Some(text)) => out.push((key.clone(), text)),
            Ok(None) => {}
            Err(e) if is_not_preparable(&e) => {}
            Err(e) if vendor_code(&e).is_some_and(|c| GONE_CODES.contains(&c)) => {}
            Err(e) => return Err(permission(e, key.kind)),
        }
    }
    Ok(out)
}

// ---- roles ------------------------------------------------------------------------------------

/// The server namespace from the `ListRoles` rows (`None`: the server has no
/// `APPLICABLE_ROLES`) and the login (`CURRENT_USER()`): each role as an [`ObjectKind::Role`]
/// (MySQL `'role'@'host'`, MariaDB the bare role name, as `GRANT … TO` names them) and the
/// login as an [`ObjectKind::User`]. Read at `Names`, so a name it does not list stays
/// `Unknown`; the role set is `Failed` when the view is missing, never empty.
pub fn roles_result(
    roles: Option<&[Row]>,
    login: Option<&str>,
    mariadb: bool,
    read_at_ms: i64,
) -> NamespaceResult {
    let root = ObjectKey {
        kind: ObjectKind::Root,
        path: SmallVec::new(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    };
    let mut changed = Vec::new();
    let mut existent = BTreeMap::new();
    let mut push = |kind: ObjectKind, n: Name, ids: &mut Vec<Identity>| {
        ids.push(Identity::of(vec![n.clone()], None));
        changed.push(ChangedObject {
            parent: root.clone(),
            subtree: Subtree::new(Obj::new(kind, n)),
        });
    };
    match roles {
        Some(rows) => {
            let mut seen = BTreeSet::new();
            let mut ids = Vec::new();
            for r in rows {
                let Some(role) = cell(r, 0) else { continue };
                let n = if mariadb {
                    (!role.contains('\0')).then(|| Name::from(role))
                } else {
                    account(role, cell(r, 1).unwrap_or(""), false)
                };
                if let Some(n) = n
                    && seen.insert(n.clone())
                {
                    push(ObjectKind::Role, n, &mut ids);
                }
            }
            existent.insert(ObjectKind::Role, ExistSet::Ok(ids));
        }
        None => {
            existent.insert(
                ObjectKind::Role,
                ExistSet::Failed(IntroErrorCode::NotImplemented),
            );
        }
    }
    if let Some((u, h)) = login.and_then(login_parts)
        && let Some(n) = account(u, h, false)
    {
        let mut ids = Vec::new();
        push(ObjectKind::User, n, &mut ids);
        existent.insert(ObjectKind::User, ExistSet::Ok(ids));
    }
    NamespaceResult {
        existent,
        changed,
        categories_to_reload: BTreeSet::new(),
        new_watermark: Watermark::None,
        full: true,
        level: Level::Names,
        read_at_ms,
    }
}

/// The roles of the server ([`roles_result`]); a refused read is `Permission { Role }`.
pub async fn roles(
    r: &mut dyn SectionReader,
    srv: &ServerCtx,
) -> Result<NamespaceResult, IntroError> {
    let rows = read_rows(r, srv, name::LIST_ROLES, Params::default(), 2)
        .await
        .map_err(|e| permission(e, ObjectKind::Role))?;
    Ok(roles_result(
        rows.as_ref().map(|r| r.rows.as_slice()),
        srv.login.as_deref(),
        srv.is_mariadb(),
        read_at_ms(),
    ))
}

// ---- the seam ---------------------------------------------------------------------------------

/// One facet of `keys` in `ns` on `r` (module docs: the keys answered are the ones read
/// completely). `Statistics` is not a MySQL facet read: `NotImplemented`.
pub async fn read_on(
    r: &mut dyn SectionReader,
    srv: &ServerCtx,
    ns: &NsKey,
    keys: &[ObjectKey],
    kind: FacetKind,
) -> Result<FacetRows, IntroError> {
    match kind {
        FacetKind::Grants => Ok(FacetRows::Grants(
            grants(r, srv, ns, keys)
                .await?
                .into_iter()
                .filter(ObjGrants::complete)
                .map(|g| (g.key, g.facet))
                .collect(),
        )),
        FacetKind::Dependencies => {
            let (deps, cov) = dependencies(r, srv, ns, keys).await?;
            if cov != Coverage::Complete {
                return Err(IntroError::Permission {
                    kind: ObjectKind::View,
                });
            }
            Ok(FacetRows::Dependencies(
                deps.into_iter()
                    .filter(|d| !d.unnamed)
                    .map(|d| (d.key, d.facet))
                    .collect(),
            ))
        }
        FacetKind::Sources => Ok(FacetRows::Sources(sources(r, srv, ns, keys).await?)),
        FacetKind::Statistics => Err(IntroError::NotImplemented),
    }
}

/// One facet of `keys` in `ns`: [`read_on`] on one snapshot of `c`.
pub fn read<'a>(
    c: &'a mut MetaConn,
    ns: &'a NsKey,
    keys: &'a [ObjectKey],
    kind: FacetKind,
) -> BoxFuture<'a, Result<FacetRows, IntroError>> {
    Box::pin(async move {
        if kind == FacetKind::Statistics {
            return Err(IntroError::NotImplemented);
        }
        let mut txn = c.begin_snapshot().await?;
        let out = {
            let mut reader = TxnReader::new(&mut txn, None)?;
            let srv = read_server(&mut reader, DbmsKind::MySql).await?;
            read_on(&mut reader, &srv, ns, keys, kind).await?
        };
        txn.commit().await?;
        Ok(out)
    })
}

/// The server-level namespace: the roles ([`roles`]).
pub fn server_namespace(c: &mut MetaConn) -> BoxFuture<'_, Result<NamespaceResult, IntroError>> {
    Box::pin(async move {
        let mut txn = c.begin_snapshot().await?;
        let out = {
            let mut reader = TxnReader::new(&mut txn, None)?;
            let srv = read_server(&mut reader, DbmsKind::MySql).await?;
            roles(&mut reader, &srv).await?
        };
        txn.commit().await?;
        Ok(out)
    })
}
