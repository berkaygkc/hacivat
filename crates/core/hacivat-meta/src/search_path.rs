//! The namespaces an unqualified name is looked up in, in order (alg/02 HV-MODEL-02 per-DBMS
//! matrix; HV-SESS-07 current schema; M4 Task 9). The caller maps the session's schema facts
//! (`hacivat_session::SchemaCtx`) into a `SearchPath`; a changed session path applies to the
//! next view (edge 8).
//!
//! - **PostgreSQL:** the session's temporary schema (`pg_temp_N`) first unless the setting
//!   names `pg_temp` (then at that place), for relations and types only (never routines or
//!   operators); `pg_catalog` first (after an implicit `pg_temp`) unless the setting names it;
//!   then the setting's entries, `$user` expanded to the session user. The server skips
//!   entries that name no existing schema; the view does the same once the schema list is
//!   loaded, and answers `Unknown` before.
//! - **MySQL/MariaDB:** the current database.
//! - **SQLite:** `temp`, `main`, then the attached schemas in attach order.

use crate::ids::NsKey;
use crate::kind::KindGroup;
use crate::name::Name;

/// One namespace of a search path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathEntry {
    pub ns: NsKey,
    /// PostgreSQL's temporary schema: searched for relations and types only.
    pub relations_and_types_only: bool,
}

/// An ordered list of namespaces, with the database a PostgreSQL `schema.name` resolves in.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchPath {
    /// PostgreSQL: the session's current database (qualified names resolve in it; a
    /// `database.schema.name` naming another one is never resolved). `None` elsewhere.
    pub database: Option<Name>,
    pub entries: Vec<PathEntry>,
}

impl SearchPath {
    /// Exactly `entries`, in order, for every kind group.
    pub fn exact(database: Option<Name>, entries: impl IntoIterator<Item = NsKey>) -> Self {
        let mut path = SearchPath {
            database,
            entries: Vec::new(),
        };
        for ns in entries {
            path.push(ns, false);
        }
        path
    }

    /// PostgreSQL: `search_path` is the setting's elements as set (`$user` and `pg_temp` as
    /// tokens, names unquoted: `SchemaCtx::search_path_setting`), `user` the session user,
    /// `temp` the session's temporary schema if it has one.
    pub fn for_pg(
        database: &Name,
        search_path: &[Name],
        user: Option<&Name>,
        temp: Option<&Name>,
    ) -> Self {
        let mut path = SearchPath {
            database: Some(database.clone()),
            entries: Vec::new(),
        };
        let schema = |name: &Name| NsKey::schema([database.clone(), name.clone()]);
        let names = |token: &str| search_path.iter().any(|n| &**n == token);
        if !names("pg_temp")
            && let Some(t) = temp
        {
            path.push(schema(t), true);
        }
        if !names("pg_catalog") {
            path.push(schema(&Name::from("pg_catalog")), false);
        }
        for entry in search_path {
            match &**entry {
                "$user" => {
                    if let Some(u) = user {
                        path.push(schema(u), false);
                    }
                }
                "pg_temp" => {
                    if let Some(t) = temp {
                        path.push(schema(t), true);
                    }
                }
                _ => path.push(schema(entry), false),
            }
        }
        path
    }

    /// MySQL / MariaDB: the current database (none: an unqualified name is never resolved).
    pub fn for_mysql(current_database: Option<&Name>) -> Self {
        SearchPath::exact(None, current_database.map(|db| NsKey::schema([db.clone()])))
    }

    /// SQLite: `temp`, `main`, then `attached` in attach order (`main` and `temp` in
    /// `attached`, as `PRAGMA database_list` reports them, are not repeated).
    pub fn for_sqlite(attached: &[Name]) -> Self {
        let mut path = SearchPath::default();
        path.push(NsKey::schema(["temp"]), false);
        path.push(NsKey::schema(["main"]), false);
        for name in attached {
            if !name.eq_ignore_ascii_case("main") && !name.eq_ignore_ascii_case("temp") {
                path.push(NsKey::schema([name.clone()]), false);
            }
        }
        path
    }

    /// The entries searched for names of `group`, in order.
    pub fn entries_for(&self, group: KindGroup) -> impl Iterator<Item = &PathEntry> + '_ {
        let temp_ok = matches!(group, KindGroup::Relations | KindGroup::Types);
        self.entries
            .iter()
            .filter(move |e| temp_ok || !e.relations_and_types_only)
    }

    /// Appends `ns` unless the path already holds it (the first place wins, as on the server).
    fn push(&mut self, ns: NsKey, relations_and_types_only: bool) {
        if !self.entries.iter().any(|e| e.ns == ns) {
            self.entries.push(PathEntry {
                ns,
                relations_and_types_only,
            });
        }
    }
}
