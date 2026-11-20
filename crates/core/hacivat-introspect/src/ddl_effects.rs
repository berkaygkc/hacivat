// ## m4-t01c-core (seam) / ## m4-t21-refresh (bodies)
//! Refresh planning after DDL (HV-INTRO-12; Task 21): from the catalog effects of the
//! statements a run executed (`sql_safety::StmtReport.ddl`), the session's state when they ran
//! and its transaction state, the namespaces or objects to refresh now and those pending until
//! `COMMIT` (R-M4-42).
//!
//! ## m4-t21-refresh (ADR 0008 `## m4-t21 — refresh after DDL`)
//!
//! [`plan_refresh`] is alg/02 HV-INTRO-12 steps 1–4; [`plan_refresh_in`] is the same with the
//! catalog the session's profile has loaded, which resolves names (a relation an `ALTER TABLE`
//! names may be a view; a PostgreSQL `DROP INDEX` names no table; an unqualified name follows
//! the search path) and finds dependents (`ALTER TYPE`, a dropped table's partitions and owned
//! sequences). The rule everywhere is **refresh more, never less**:
//!
//! - **Only successful statements** reach the planner (the caller filters); a statement whose
//!   `ddl` is empty and whose kind is not a catalog change (DML, `SELECT`, `TRUNCATE`, U-3)
//!   has no target.
//! - **Names** are folded with the dialect's rule (`sql_ident::fold`), qualified with the
//!   session state **captured at execution time** ([`SessionStateAtExec`]): a qualified name
//!   is its schema's (PostgreSQL: of the session's database unless the name says another);
//!   an unqualified `CREATE` is the current namespace's; any other unqualified name is looked
//!   up along the search path, and every search-path namespace the catalog cannot rule out
//!   before the first hit is a target too.
//! - **Targets per action** (alg/02 step 2): `CREATE`/`DROP` → the object and its namespace's
//!   name list ([`Target::NamespaceNames`]); `RENAME` → the old and the new object and the
//!   name list; `SET SCHEMA` → both namespaces; `ALTER`, `COMMENT`, `GRANT`, `REVOKE` → the
//!   object; a foreign key's referenced table → that table; an index, trigger, policy or rule →
//!   its table (a PostgreSQL `DROP INDEX` finds it in the catalog, else the namespace); a
//!   routine or a type → its namespace (overloads and dependents), plus every table in the
//!   catalog with a column of that type; a schema created, dropped or renamed →
//!   [`Target::ScopeRefresh`]; a database → `ScopeRefresh`; a role → the server namespace; an
//!   extension → the database namespace and the current one. A PostgreSQL relation named by
//!   `ALTER TABLE` (or a rename, a move) that the catalog cannot match in the relation group
//!   widens to its namespace (T6 review carry).
//! - **Cascades**: `DROP … CASCADE` refreshes every loaded namespace of the database
//!   ([`Target::Database`]); a plain `DROP TABLE` adds the partitions and the owned sequences
//!   the catalog knows.
//! - **Opaque** (`DO`, `CALL`, `EXECUTE`, dynamic SQL, an executable comment, a statement the
//!   classifier could not read) → [`Target::Namespace`] of the current namespace plus every
//!   namespace the statement's text names (`StmtReport.touched`); an opaque DDL-class
//!   statement (`DROP OWNED`, `REASSIGN OWNED`, `ALTER DEFAULT PRIVILEGES`, `CREATE CAST`,
//!   `DROP COLUMN … CASCADE`, `CREATE EXTENSION`) → every loaded namespace of the database and
//!   the namespace list (U-2); an opaque `Other` statement (SQLite `ATTACH`, MySQL `CREATE
//!   EVENT`) also lists the namespaces again. A DDL-class, `Other` or
//!   `EXPLAIN ANALYZE` statement with no effect at all is treated as opaque (m-B3). A run the
//!   classifier did not analyse (a script over 1 MiB, an ambiguous boundary) is opaque as a
//!   whole: every loaded namespace of the database and the scope (T6 review carry).
//! - **Temporary objects** (`temp`, PostgreSQL `pg_temp*`, SQLite `temp.`) → nothing (M5's
//!   session overlay).
//! - **Widening** (step 3): more than [`WIDEN_OVER`] object targets in one namespace become one
//!   [`Target::Namespace`]; a namespace target subsumes the object and name-list targets of the
//!   same namespace.
//! - **Timing** (step 4): `now` in autocommit, on an engine whose DDL commits on its own
//!   (MySQL/MariaDB implicit commit, ver/01 `#mysql-ro`), and for a statement that cannot run
//!   inside a transaction (PostgreSQL `CREATE INDEX CONCURRENTLY`, `CREATE`/`DROP DATABASE`);
//!   otherwise `pending` until the transaction ends (PostgreSQL and SQLite, R-M4-42).
//!
//! Targets name namespaces that may not be loaded; [`lower`] (the coordinator, at the time it
//! refreshes) keeps only those the catalog holds or is loading, so a refresh never loads a
//! namespace nobody opened.

use std::collections::{BTreeMap, BTreeSet};

use hacivat_meta::props::Props;
use hacivat_meta::snapshot::{CatalogSnapshot, ContainerKey, LoadState};
use hacivat_meta::{
    KindGroup, Lookup, Name, NamespaceSnapshot, NsKey, NsKind, ObjectKey, ObjectKind,
};
use hacivat_proto::DbmsKind;
use smallvec::SmallVec;
use sql_dialect::DialectId;
use sql_ident::{Ident, QualifiedName};
use sql_safety::{DdlAction, DdlEffect, DdlKind, StmtKind, StmtReport};

use crate::introspector::IntroCaps;

/// More object targets than this in one namespace become one namespace target (alg/02 step 3).
pub const WIDEN_OVER: usize = 50;

/// One statement a run executed successfully.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutedStmt {
    pub kind: StmtKind,
    /// Its catalog effects (`StmtReport.ddl`).
    pub ddl: Vec<DdlEffect>,
    // ## m4-t21-refresh
    /// The names the statement's text shows (`StmtReport.touched`): the namespaces an opaque
    /// statement names literally are refreshed too.
    pub names: Vec<QualifiedName>,
    /// `false` when the classifier gave no report for the statement (a script over 1 MiB, an
    /// ambiguous boundary): the run is opaque as a whole (T6 review carry).
    pub analysed: bool,
}

// ## m4-t21-refresh
impl ExecutedStmt {
    /// The statement `report` describes.
    pub fn from_report(report: &StmtReport) -> Self {
        let t = &report.touched;
        let names = t
            .reads
            .iter()
            .chain(&t.writes)
            .chain(&t.calls)
            .map(|o| o.name.clone())
            .collect();
        ExecutedStmt {
            kind: report.kind,
            ddl: report.ddl.clone(),
            names,
            analysed: true,
        }
    }

    /// A statement the classifier did not analyse.
    pub fn unanalysed() -> Self {
        ExecutedStmt {
            kind: StmtKind::Other,
            ddl: Vec::new(),
            names: Vec::new(),
            analysed: false,
        }
    }
}

/// The session's state when the statements ran (captured at execution time, never later).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStateAtExec {
    /// The namespace the session was in (PostgreSQL `[db, current_schema]`; MySQL/MariaDB
    /// `[database]`; SQLite `[main]`).
    pub current: NsKey,
    /// The search path, for resolving unqualified names (Task 21).
    pub search_path: Vec<NsKey>,
}

// ## m4-t21-refresh
impl SessionStateAtExec {
    /// The state a session of `dbms` reports in `schema` (its `SessionSnapshot::schema`):
    /// PostgreSQL `[database, schema]` and the search path's schemas of that database;
    /// MySQL/MariaDB `[database]`; SQLite `[main]` (unqualified names search `main`; `temp`
    /// objects are never refreshed). `None` when the session does not know its namespace.
    pub fn from_schema(
        dbms: DbmsKind,
        database: Option<&str>,
        schema: Option<&str>,
        search_path: &[String],
    ) -> Option<Self> {
        match dbms {
            DbmsKind::Postgres => {
                let db = database?;
                let current = NsKey::schema([db, schema?]);
                let mut path: Vec<NsKey> = search_path
                    .iter()
                    .filter(|s| !s.is_empty())
                    .map(|s| NsKey::schema([db, s.as_str()]))
                    .collect();
                if path.is_empty() {
                    path.push(current.clone());
                }
                Some(SessionStateAtExec {
                    current,
                    search_path: path,
                })
            }
            DbmsKind::MySql | DbmsKind::MariaDb => {
                let db = database.or(schema)?;
                let current = NsKey::schema([db]);
                Some(SessionStateAtExec {
                    search_path: vec![current.clone()],
                    current,
                })
            }
            DbmsKind::Sqlite => {
                let current = NsKey::schema(["main"]);
                Some(SessionStateAtExec {
                    search_path: vec![current.clone()],
                    current,
                })
            }
            _ => None,
        }
    }
}

/// The session's transaction state when the statements ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TxnState {
    AutoCommit,
    InTransaction,
}

/// What to refresh.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Target {
    /// A whole namespace.
    Namespace(NsKey),
    /// Some objects of one namespace.
    Objects { ns: NsKey, keys: Vec<ObjectKey> },
    /// The list of namespaces (a schema created, dropped or renamed).
    ScopeRefresh,
    // ## m4-t21-refresh: the planner's targets (alg/02 HV-INTRO-12); [`lower`] turns them into
    // the three above, which are what the engine runs.
    /// One top-level object of its namespace (its namespace is its key's path without the last
    /// name): created, dropped or changed.
    Object(ObjectKey),
    /// The names of a namespace (an object created, dropped or renamed there).
    NamespaceNames(NsKey),
    /// Every loaded namespace of database `db` (PostgreSQL: its schemas and its database-level
    /// namespace; MySQL/MariaDB and SQLite: every loaded namespace of the profile).
    Database(Name),
}

/// The plan for one run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RefreshPlan {
    /// Refresh at once.
    pub now: Vec<Target>,
    /// Refresh on `COMMIT`, drop on `ROLLBACK` (R-M4-42; the coordinator keeps them per
    /// session, R-M4-11).
    pub pending: Vec<Target>,
}

/// Plans the refresh after `stmts` (alg/02 HV-INTRO-12 steps 1–4; the module docs) without a
/// catalog: names are not resolved against loaded objects.
pub fn plan_refresh(
    stmts: &[ExecutedStmt],
    session: &SessionStateAtExec,
    txn: TxnState,
    caps: &IntroCaps,
) -> RefreshPlan {
    plan_refresh_in(stmts, session, txn, caps, None)
}

// ## m4-t21-refresh
/// [`plan_refresh`] with the profile's loaded catalog `snap` (the module docs).
pub fn plan_refresh_in(
    stmts: &[ExecutedStmt],
    session: &SessionStateAtExec,
    txn: TxnState,
    caps: &IntroCaps,
    snap: Option<&CatalogSnapshot>,
) -> RefreshPlan {
    let mut plan = RefreshPlan::default();
    let mut p = Planner {
        session,
        dbms: caps.dbms,
        d: dialect_of(caps.dbms),
        snap,
        out: Vec::new(),
    };
    for stmt in stmts {
        p.out.clear();
        let immediate = p.statement(stmt);
        let targets = std::mem::take(&mut p.out);
        let waits = txn == TxnState::InTransaction && caps.transactional_ddl && !immediate;
        if waits {
            plan.pending.extend(targets);
        } else {
            plan.now.extend(targets);
        }
    }
    plan.now = normalize(plan.now);
    plan.pending = normalize(plan.pending);
    plan
}

/// The dialect whose name rules `dbms` follows.
fn dialect_of(dbms: DbmsKind) -> DialectId {
    match dbms {
        DbmsKind::MySql => DialectId::MySql,
        DbmsKind::MariaDb => DialectId::MariaDb,
        DbmsKind::Sqlite => DialectId::Sqlite,
        _ => DialectId::Postgres,
    }
}

/// The DDL-class kinds: a statement of one of them that names nothing is opaque (m-B3).
fn ddl_class(kind: StmtKind) -> bool {
    matches!(
        kind,
        StmtKind::CreateTable
            | StmtKind::CreateOther
            | StmtKind::Alter
            | StmtKind::Drop
            | StmtKind::DropOwned
            | StmtKind::Grant
            | StmtKind::Revoke
    )
}

/// The namespace an object target's key belongs to (its path without its own name).
pub fn ns_of_key(key: &ObjectKey) -> NsKey {
    let n = key.path.len().saturating_sub(1);
    NsKey {
        kind: NsKind::Schema,
        path: key.path[..n].iter().cloned().collect(),
    }
}

/// Step 3: each target once, in order; a namespace target subsumes the object and name-list
/// targets of its namespace; more than [`WIDEN_OVER`] objects of one namespace become one
/// namespace target, in the place of the first.
fn normalize(list: Vec<Target>) -> Vec<Target> {
    let mut seen = BTreeSet::new();
    let mut list: Vec<Target> = list
        .into_iter()
        .filter(|t| seen.insert(t.clone()))
        .collect();
    let mut per_ns: BTreeMap<NsKey, usize> = BTreeMap::new();
    for t in &list {
        if let Target::Object(k) = t {
            *per_ns.entry(ns_of_key(k)).or_default() += 1;
        }
    }
    let mut whole: BTreeSet<NsKey> = list
        .iter()
        .filter_map(|t| match t {
            Target::Namespace(ns) => Some(ns.clone()),
            _ => None,
        })
        .collect();
    let widened: BTreeSet<NsKey> = per_ns
        .into_iter()
        .filter(|(ns, n)| *n > WIDEN_OVER && !whole.contains(ns))
        .map(|(ns, _)| ns)
        .collect();
    let mut placed = BTreeSet::new();
    list = list
        .into_iter()
        .filter_map(|t| match &t {
            Target::Object(k) => {
                let ns = ns_of_key(k);
                if widened.contains(&ns) {
                    placed.insert(ns.clone()).then_some(Target::Namespace(ns))
                } else if whole.contains(&ns) {
                    None
                } else {
                    Some(t)
                }
            }
            Target::NamespaceNames(ns) | Target::Objects { ns, .. }
                if whole.contains(ns) || widened.contains(ns) =>
            {
                None
            }
            _ => Some(t),
        })
        .collect();
    whole.extend(widened);
    let mut seen = BTreeSet::new();
    list.retain(|t| seen.insert(t.clone()));
    list
}

/// The planner of one statement at a time.
struct Planner<'a> {
    session: &'a SessionStateAtExec,
    dbms: DbmsKind,
    d: DialectId,
    snap: Option<&'a CatalogSnapshot>,
    out: Vec<Target>,
}

/// Where a name was found.
enum Located {
    /// The catalog holds these objects (in their namespace's arena).
    Found(Vec<ObjectKey>),
    /// Not found for certain, or unknown: the name in this namespace.
    Named(NsKey, Name),
}

/// How sure a statement is of the kind of relation it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Naming {
    /// It creates it (in the current namespace when unqualified).
    Create,
    /// It names its kind (`DROP TABLE`, an index's table, a foreign key's target).
    Definite,
    /// PostgreSQL's `ALTER TABLE` family accepts any relation kind.
    Ambiguous,
}

impl Planner<'_> {
    fn push(&mut self, t: Target) {
        self.out.push(t);
    }

    fn db(&self) -> Name {
        self.session
            .current
            .path
            .first()
            .cloned()
            .unwrap_or_else(|| Name::from(""))
    }

    /// The targets of one statement into `self.out`; whether it ran outside any transaction
    /// whatever the session's state (it cannot run inside one).
    fn statement(&mut self, s: &ExecutedStmt) -> bool {
        if !s.analysed {
            self.push(Target::Database(self.db()));
            self.push(Target::ScopeRefresh);
            return false;
        }
        let opaque = s.ddl.iter().any(|e| e.opaque)
            || (s.ddl.is_empty()
                && (ddl_class(s.kind)
                    || s.kind == StmtKind::Other
                    || s.kind == StmtKind::Explain { analyze: true }));
        if opaque {
            self.opaque(s);
        }
        let mut immediate = false;
        for e in s.ddl.iter().filter(|e| !e.opaque) {
            immediate |= e.concurrently || (e.kind == DdlKind::Database && self.pg());
            self.effect(e);
        }
        immediate
    }

    fn pg(&self) -> bool {
        self.dbms == DbmsKind::Postgres
    }

    /// An opaque statement (the module docs).
    fn opaque(&mut self, s: &ExecutedStmt) {
        // A DDL statement that may change what it does not name (`DROP OWNED`, `REASSIGN
        // OWNED`, `ALTER DEFAULT PRIVILEGES`, `DROP COLUMN … CASCADE`, `CREATE SCHEMA
        // AUTHORIZATION r`, `CREATE EXTENSION`): every loaded namespace of the database and the
        // namespace list (U-2).
        if ddl_class(s.kind) {
            self.push(Target::Database(self.db()));
            self.push(Target::ScopeRefresh);
            return;
        }
        self.push(Target::Namespace(self.session.current.clone()));
        if s.kind == StmtKind::Other {
            self.push(Target::ScopeRefresh);
        }
        let named: Vec<NsKey> = s
            .names
            .iter()
            .filter(|n| n.parts.len() > 1)
            .filter_map(|n| self.qualifier_ns(n))
            .collect();
        for ns in named {
            self.push(Target::Namespace(ns));
        }
    }

    /// The namespace of a qualified name's qualifier, unless it is a temporary one.
    fn qualifier_ns(&self, n: &QualifiedName) -> Option<NsKey> {
        let (_, qual) = n.parts.split_last()?;
        self.ns_for(qual)
    }

    /// The namespace `qual` (the parts before an object's name) names; `None` for a temporary
    /// schema or a qualifier this dialect does not have.
    fn ns_for(&self, qual: &[Ident]) -> Option<NsKey> {
        match self.dbms {
            DbmsKind::Postgres => {
                let (db, schema) = match qual {
                    [s] => (self.db(), self.fold(s)),
                    [db, s] => (self.fold(db), self.fold(s)),
                    _ => return None,
                };
                if is_pg_temp(&schema) {
                    return None;
                }
                Some(NsKey::schema([db, schema]))
            }
            DbmsKind::MySql | DbmsKind::MariaDb => match qual {
                [db] => Some(NsKey::schema([self.fold(db)])),
                _ => None,
            },
            DbmsKind::Sqlite => match qual {
                [s] if s.text.eq_ignore_ascii_case("temp") => None,
                [s] if s.text.eq_ignore_ascii_case("main") => Some(NsKey::schema(["main"])),
                [s] => Some(NsKey::schema([self.fold(s)])),
                _ => None,
            },
            _ => None,
        }
    }

    fn fold(&self, i: &Ident) -> Name {
        sql_ident::fold(self.d, &i.text, i.quoted)
    }

    /// The candidate namespaces of `name` (with its last part, folded) for `action`: the named
    /// schema, the current namespace for an unqualified `CREATE`, else the search path.
    fn candidates(&self, name: &QualifiedName, create: bool) -> Option<(Vec<NsKey>, Ident)> {
        let (last, qual) = name.parts.split_last()?;
        if !qual.is_empty() {
            return Some((vec![self.ns_for(qual)?], last.clone()));
        }
        let mut list = if create || self.session.search_path.is_empty() {
            vec![self.session.current.clone()]
        } else {
            self.session.search_path.clone()
        };
        // SQLite resolves an unqualified name in `main`, then in each attached schema (`temp`
        // objects are never refreshed): every other loaded schema follows `main`.
        if !create
            && self.dbms == DbmsKind::Sqlite
            && let Some(snap) = self.snap
        {
            for ns in snap.namespaces.keys() {
                let temp = ns.path.first().is_some_and(|s| &**s == "temp");
                if ns.kind == NsKind::Schema && !temp && !list.contains(ns) {
                    list.push(ns.clone());
                }
            }
        }
        Some((list, last.clone()))
    }

    /// Where `name` (of the objects `kinds` in `group`) is: the catalog's hits in the first
    /// search-path namespace that holds it, after every earlier namespace the catalog cannot
    /// rule out; else the name in every candidate (a `CREATE`: the first).
    fn locate(
        &self,
        name: &QualifiedName,
        group: Option<KindGroup>,
        accept: &dyn Fn(ObjectKind) -> bool,
        create: bool,
    ) -> Vec<Located> {
        let Some((cands, last)) = self.candidates(name, create) else {
            return Vec::new();
        };
        let folded = self.fold(&last);
        let mut out = Vec::new();
        for ns in &cands {
            match self.lookup(ns, group, &last, accept) {
                Lookup::Found(keys) => {
                    out.push(Located::Found(keys));
                    return out;
                }
                Lookup::Missing(_) => {}
                Lookup::Unknown(_) => out.push(Located::Named(ns.clone(), folded.clone())),
            }
        }
        if out.is_empty() {
            // Not there for certain: a create, or a harmless target (`DROP … IF EXISTS`).
            out.push(Located::Named(cands[0].clone(), folded));
        }
        out
    }

    /// The catalog's answer for `name` in `ns`: `Unknown` when the namespace is not loaded.
    fn lookup(
        &self,
        ns: &NsKey,
        group: Option<KindGroup>,
        name: &Ident,
        accept: &dyn Fn(ObjectKind) -> bool,
    ) -> Lookup<Vec<ObjectKey>> {
        let (Some(snap), Some(group)) = (self.snap, group) else {
            return Lookup::Unknown(hacivat_meta::UnknownReason::NotLoaded);
        };
        let Some(arena) = snap.namespaces.get(ns) else {
            return Lookup::Unknown(hacivat_meta::UnknownReason::NotLoaded);
        };
        match arena.match_named(group, &name.text, name.quoted, &snap.server.ident) {
            Lookup::Found(hits) => {
                let keys: Vec<ObjectKey> = hits
                    .iter()
                    .filter(|&&i| arena.obj(i).is_some_and(|o| accept(o.kind())))
                    .filter_map(|&i| top_level_key(arena, i))
                    .collect();
                if keys.is_empty() {
                    Lookup::Missing(hacivat_meta::MissingReason::NotFound)
                } else {
                    Lookup::Found(keys)
                }
            }
            Lookup::Missing(m) => Lookup::Missing(m),
            Lookup::Unknown(u) => Lookup::Unknown(u),
        }
    }

    /// The targets of one named effect.
    fn effect(&mut self, e: &DdlEffect) {
        if e.temp {
            return;
        }
        for r in &e.referenced {
            self.relation(r, ObjectKind::Table, Naming::Definite);
        }
        if matches!(e.action, DdlAction::Drop { cascade: true }) {
            self.push(Target::Database(self.db()));
        }
        // An index, a trigger, a policy, a rule, or a column or constraint (`COMMENT ON COLUMN
        // s.t.c`, kind `Other` with its table in `on`): its table.
        let attached = matches!(
            e.kind,
            DdlKind::Index | DdlKind::Trigger | DdlKind::Policy | DdlKind::Rule
        ) || (e.kind == DdlKind::Other && e.on.is_some());
        if attached {
            self.attached(e);
            return;
        }
        let Some(name) = &e.name else {
            match e.kind {
                DdlKind::Database => self.push(Target::ScopeRefresh),
                DdlKind::Role => self.push(Target::Namespace(NsKey::server())),
                _ => self.push(Target::Namespace(self.session.current.clone())),
            }
            return;
        };
        match e.kind {
            DdlKind::Database => self.push(Target::ScopeRefresh),
            DdlKind::Schema => self.schema(e, name),
            DdlKind::Role => self.push(Target::Namespace(NsKey::server())),
            DdlKind::Extension => {
                self.push(Target::Namespace(NsKey::database(self.db())));
                self.push(Target::Namespace(self.session.current.clone()));
            }
            DdlKind::Table | DdlKind::View | DdlKind::MaterializedView | DdlKind::Sequence => {
                self.relation_effect(e, name)
            }
            DdlKind::Index | DdlKind::Trigger | DdlKind::Policy | DdlKind::Rule => self.attached(e),
            DdlKind::Type | DdlKind::Domain => self.type_effect(name),
            DdlKind::Routine | DdlKind::Event | DdlKind::Other => {
                let create = matches!(e.action, DdlAction::Create);
                if let Some((cands, _)) = self.candidates(name, create) {
                    for ns in cands {
                        self.push(Target::Namespace(ns));
                    }
                }
            }
        }
        // A routine, a type or a domain moved to another schema: that schema too (a relation's
        // move is `relation_effect`'s).
        if let DdlAction::SetSchema { to } = &e.action
            && matches!(
                e.kind,
                DdlKind::Routine | DdlKind::Type | DdlKind::Domain | DdlKind::Other
            )
            && let Some(ns) = self.ns_for(std::slice::from_ref(to))
        {
            self.push(Target::Namespace(ns));
        }
    }

    /// `CREATE`/`DROP`/`ALTER SCHEMA` and grants on a schema.
    fn schema(&mut self, e: &DdlEffect, name: &QualifiedName) {
        match &e.action {
            DdlAction::Create | DdlAction::Drop { .. } | DdlAction::Rename { .. } => {
                self.push(Target::ScopeRefresh);
            }
            _ => {
                if let Some(ns) = self.ns_for(&name.parts) {
                    self.push(Target::Namespace(ns));
                }
            }
        }
    }

    /// A table, view, materialized view or sequence.
    fn relation_effect(&mut self, e: &DdlEffect, name: &QualifiedName) {
        let mode = match e.action {
            DdlAction::Create => Naming::Create,
            DdlAction::Drop { .. } => Naming::Definite,
            _ => Naming::Ambiguous,
        };
        let kind = relation_kind(e.kind);
        let found = self.relation(name, kind, mode);
        let names = matches!(
            e.action,
            DdlAction::Create | DdlAction::Drop { .. } | DdlAction::Rename { .. }
        );
        for (ns, key) in &found {
            if names {
                self.push(Target::NamespaceNames(ns.clone()));
            }
            match &e.action {
                DdlAction::Rename { to } => {
                    let new = self.key_in(ns, kind, self.fold(to));
                    self.push(Target::Object(new));
                }
                DdlAction::SetSchema { to } => {
                    if let Some(to_ns) = self.ns_for(std::slice::from_ref(to)) {
                        let leaf = key
                            .as_ref()
                            .map_or_else(|| Name::from(""), |k| Name::from(k.name()));
                        self.push(Target::NamespaceNames(ns.clone()));
                        self.push(Target::NamespaceNames(to_ns.clone()));
                        if leaf.is_empty() {
                            self.push(Target::Namespace(to_ns));
                        } else {
                            self.push(Target::Object(self.key_in(&to_ns, kind, leaf)));
                        }
                    }
                }
                DdlAction::Drop { .. } => {
                    if let Some(k) = key {
                        self.dependents(std::slice::from_ref(k), &[]);
                    }
                }
                _ => {}
            }
        }
    }

    /// The relation `name` names: pushes its object target and answers each `(namespace,
    /// key)` it pushed (`None`: the whole namespace, when the catalog cannot match it).
    fn relation(
        &mut self,
        name: &QualifiedName,
        kind: ObjectKind,
        mode: Naming,
    ) -> Vec<(NsKey, Option<ObjectKey>)> {
        let accept = |k: ObjectKind| {
            matches!(
                k,
                ObjectKind::Table
                    | ObjectKind::View
                    | ObjectKind::MaterializedView
                    | ObjectKind::ForeignTable
                    | ObjectKind::VirtualTable
                    | ObjectKind::Sequence
                    | ObjectKind::CompositeType
            )
        };
        let create = mode == Naming::Create;
        let mut out = Vec::new();
        for loc in self.locate(name, Some(KindGroup::Relations), &accept, create) {
            match loc {
                Located::Found(keys) => {
                    for k in keys {
                        let ns = ns_of_key(&k);
                        self.push(Target::Object(k.clone()));
                        out.push((ns, Some(k)));
                    }
                }
                // A PostgreSQL `ALTER TABLE`, rename or move may name a view, a sequence or an
                // index: one the catalog cannot match widens to its namespace (T6 review
                // carry). A create, a drop and an attached object's table name their kind.
                Located::Named(ns, _) if mode == Naming::Ambiguous && self.pg() => {
                    self.push(Target::Namespace(ns.clone()));
                    out.push((ns, None));
                }
                Located::Named(ns, leaf) => {
                    let k = self.key_in(&ns, kind, leaf);
                    self.push(Target::Object(k.clone()));
                    out.push((ns, Some(k)));
                }
            }
        }
        out
    }

    /// An index, trigger, policy or rule: its table, found from `on` or, for an object named
    /// alone (PostgreSQL `DROP INDEX i`, MySQL `DROP TRIGGER t`), from the catalog.
    fn attached(&mut self, e: &DdlEffect) {
        let changes_names = e.kind == DdlKind::Index
            && matches!(
                e.action,
                DdlAction::Create | DdlAction::Drop { .. } | DdlAction::Rename { .. }
            );
        if let Some(on) = &e.on {
            let found = self.relation(on, ObjectKind::Table, Naming::Definite);
            if changes_names && self.pg() {
                for (ns, _) in found {
                    self.push(Target::NamespaceNames(ns));
                }
            }
            return;
        }
        let Some(name) = &e.name else {
            self.push(Target::Namespace(self.session.current.clone()));
            return;
        };
        let (group, kind) = match e.kind {
            DdlKind::Index => (Some(KindGroup::Relations), ObjectKind::Index),
            DdlKind::Trigger if self.dbms != DbmsKind::Postgres => {
                (Some(KindGroup::Triggers), ObjectKind::Trigger)
            }
            _ => (None, ObjectKind::Unknown),
        };
        let group = if self.dbms == DbmsKind::Sqlite {
            Some(KindGroup::Relations)
        } else {
            group
        };
        for loc in self.parents_of(name, group, kind) {
            match loc {
                Located::Found(tables) => {
                    for t in tables {
                        if changes_names {
                            self.push(Target::NamespaceNames(ns_of_key(&t)));
                        }
                        self.push(Target::Object(t));
                    }
                }
                Located::Named(ns, _) => self.push(Target::Namespace(ns)),
            }
        }
    }

    /// The tables holding the `kind` objects `name` names (an index, a trigger), as
    /// [`Planner::locate`] answers.
    fn parents_of(
        &self,
        name: &QualifiedName,
        group: Option<KindGroup>,
        kind: ObjectKind,
    ) -> Vec<Located> {
        let Some((cands, last)) = self.candidates(name, false) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for ns in &cands {
            let hits = self.snap.zip(group).and_then(|(snap, group)| {
                let arena = snap.namespaces.get(ns)?;
                match arena.match_named(group, &last.text, last.quoted, &snap.server.ident) {
                    Lookup::Found(hits) => Some(Lookup::Found(
                        hits.iter()
                            .filter(|&&i| arena.obj(i).is_some_and(|o| o.kind() == kind))
                            .filter_map(|&i| top_level_key(arena, i))
                            .collect::<Vec<_>>(),
                    )),
                    Lookup::Missing(m) => Some(Lookup::Missing(m)),
                    Lookup::Unknown(_) => None,
                }
            });
            match hits {
                Some(Lookup::Found(keys)) if !keys.is_empty() => {
                    out.push(Located::Found(keys));
                    return out;
                }
                Some(Lookup::Found(_) | Lookup::Missing(_)) => {}
                _ => out.push(Located::Named(ns.clone(), self.fold(&last))),
            }
        }
        if out.is_empty() {
            out.push(Located::Named(cands[0].clone(), self.fold(&last)));
        }
        out
    }

    /// A type or a domain: its namespace (the object's subtype is unknown) and every table of
    /// the database with a column of that type (alg/02 step 2, `ALTER TYPE`).
    fn type_effect(&mut self, name: &QualifiedName) {
        let Some((cands, last)) = self.candidates(name, false) else {
            return;
        };
        let leaf = self.fold(&last);
        let mut types = Vec::new();
        for ns in cands {
            let schema = ns.path.last().cloned();
            self.push(Target::Namespace(ns));
            types.push((schema, leaf.clone()));
        }
        self.dependents(&[], &types);
    }

    /// The tables and sequences of the loaded catalog that depend on `tables` (partitions,
    /// owned sequences, foreign keys pointing at them) or on the types `types` (a column of
    /// that type).
    fn dependents(&mut self, tables: &[ObjectKey], types: &[(Option<Name>, Name)]) {
        let Some(snap) = self.snap else {
            return;
        };
        let db = self.db();
        let pg = self.pg();
        let mut found: Vec<ObjectKey> = Vec::new();
        for (ns, arena) in snap.namespaces.iter() {
            if pg && ns.path.first() != Some(&db) {
                continue;
            }
            for i in 0..u32::try_from(arena.len()).unwrap_or(u32::MAX) {
                let Some(o) = arena.obj(i) else {
                    break;
                };
                let on = |path: &[Name]| tables.iter().any(|k| *k.path == *path);
                let hit = match o.props() {
                    Props::Table(t) => t.partition_of.as_ref().is_some_and(|r| on(&r.path)),
                    Props::Sequence(s) => s.owned_by.as_ref().is_some_and(|(r, _)| on(&r.path)),
                    Props::ForeignKey(f) => on(&f.target.path),
                    Props::Column(c) => c.type_ref.as_ref().is_some_and(|t| {
                        types.iter().any(|(schema, name)| {
                            t.name == *name
                                && (schema.is_none() || t.schema.is_none() || t.schema == *schema)
                        })
                    }),
                    _ => false,
                };
                if hit && let Some(k) = top_level_key(arena, i) {
                    found.push(k);
                }
            }
        }
        for k in found {
            self.push(Target::Object(k));
        }
    }

    /// The key of the top-level object `leaf` of `kind` in `ns`.
    fn key_in(&self, ns: &NsKey, kind: ObjectKind, leaf: Name) -> ObjectKey {
        let mut path: SmallVec<[Name; 4]> = ns.path.iter().cloned().collect();
        path.push(leaf);
        ObjectKey {
            kind,
            path,
            server_id: None,
            overload_sig: None,
            raw_kind: None,
        }
    }
}

/// The model kind of a relation effect's kind.
fn relation_kind(k: DdlKind) -> ObjectKind {
    match k {
        DdlKind::View => ObjectKind::View,
        DdlKind::MaterializedView => ObjectKind::MaterializedView,
        DdlKind::Sequence => ObjectKind::Sequence,
        _ => ObjectKind::Table,
    }
}

/// PostgreSQL's temporary schemas (`pg_temp`, `pg_temp_N`).
fn is_pg_temp(schema: &str) -> bool {
    schema == "pg_temp" || schema.starts_with("pg_temp_")
}

/// The key of the top-level object holding arena object `idx` (itself, or its ancestor right
/// under the namespace root: an index's or a column's table).
fn top_level_key(arena: &NamespaceSnapshot, mut idx: u32) -> Option<ObjectKey> {
    loop {
        let o = arena.obj(idx)?;
        match o.parent() {
            Some(0) | None => return arena.object_key(idx),
            Some(p) => idx = p,
        }
    }
}

// ## m4-t21-refresh
/// The engine's targets for `targets` at the time of the refresh (the coordinator calls it on
/// the catalog `snap` of that moment): objects grouped per namespace, name lists and namespace
/// targets as namespace passes, [`Target::Database`] as a pass of each of its loaded
/// namespaces. A namespace that is neither loaded nor loading is dropped (nothing cached there
/// can be stale; its first expansion reads it fresh); the scope refresh always stays. Order:
/// the scope refresh, then the namespace passes, then the fragments.
pub fn lower(targets: &[Target], snap: &CatalogSnapshot) -> Vec<Target> {
    let held = |ns: &NsKey| {
        snap.namespaces.contains_key(ns)
            || matches!(
                snap.state(&ContainerKey::Namespace(ns.clone())),
                LoadState::Loading { .. }
            )
    };
    let pg = matches!(snap.server.dbms, DbmsKind::Postgres);
    let mut scope = false;
    let mut whole: BTreeSet<NsKey> = BTreeSet::new();
    let mut objects: BTreeMap<NsKey, Vec<ObjectKey>> = BTreeMap::new();
    for t in targets {
        match t {
            Target::ScopeRefresh => scope = true,
            Target::Namespace(ns) | Target::NamespaceNames(ns) => {
                whole.insert(ns.clone());
            }
            Target::Database(db) => {
                let loaded =
                    snap.namespaces
                        .keys()
                        .chain(snap.states.keys().filter_map(|k| match k {
                            ContainerKey::Namespace(ns) => Some(ns),
                            _ => None,
                        }));
                for ns in loaded {
                    // An unknown database (a session that does not know its namespace): all.
                    if !pg || db.is_empty() || ns.path.first() == Some(db) {
                        whole.insert(ns.clone());
                    }
                }
            }
            Target::Object(k) => objects.entry(ns_of_key(k)).or_default().push(k.clone()),
            Target::Objects { ns, keys } => objects
                .entry(ns.clone())
                .or_default()
                .extend(keys.iter().cloned()),
        }
    }
    let mut out = Vec::new();
    if scope {
        out.push(Target::ScopeRefresh);
    }
    out.extend(
        whole
            .iter()
            .filter(|ns| held(ns))
            .cloned()
            .map(Target::Namespace),
    );
    for (ns, mut keys) in objects {
        if whole.contains(&ns) || !held(&ns) {
            continue;
        }
        keys.sort();
        keys.dedup();
        out.push(Target::Objects { ns, keys });
    }
    out
}
