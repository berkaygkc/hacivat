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
//!   loaded, and answers `Unknown` before. `pg_catalog` and a temporary schema the caller
//!   names exist by definition: they are never skipped (T9 fix round 1, I3).
//! - **MySQL/MariaDB:** the current database.
//! - **SQLite:** `temp`, `main`, then the attached schemas in attach order.
//!
//! **What the session does not know stays unknown** (T9 fix round 1, I2): `$user` with an
//! unknown session user, and an empty setting (`SchemaCtx::search_path_setting` is empty when
//! the probe gave nothing), become an [`PathEntry::unknown`] entry; a lookup that reaches it
//! answers `Unknown(NotLoaded)`. A mapper that knows nothing at all passes
//! `SearchPath::default()` (no entry: every unqualified lookup is `Unknown`).

use crate::ids::NsKey;
use crate::kind::KindGroup;
use crate::name::Name;

/// One namespace of a search path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathEntry {
    /// The namespace; `None` for an entry the session could not name (`$user` with an unknown
    /// user, an unknown setting): a lookup reaching it is `Unknown(NotLoaded)`.
    pub ns: Option<NsKey>,
    /// PostgreSQL's temporary schema: searched for relations and types only.
    pub relations_and_types_only: bool,
    /// The namespace exists whatever the schema list says (`pg_catalog`; a temporary schema
    /// the session reported): never skipped as absent; unlisted, it is `Unknown(NotLoaded)`.
    pub must_exist: bool,
}

impl PathEntry {
    /// An entry the session could not name.
    pub fn unknown() -> Self {
        PathEntry {
            ns: None,
            relations_and_types_only: false,
            must_exist: false,
        }
    }
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
            path.push(ns, false, false);
        }
        path
    }

    /// PostgreSQL: `search_path` is the setting's elements as set (`$user` and `pg_temp` as
    /// tokens, names unquoted: `SchemaCtx::search_path_setting`; empty means unknown), `user`
    /// the session user (`None`: unknown), `temp` the session's temporary schema if it has one.
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
            path.push(schema(t), true, true);
        }
        if !names("pg_catalog") {
            path.push(schema(&Name::from("pg_catalog")), false, true);
        }
        if search_path.is_empty() {
            path.entries.push(PathEntry::unknown());
        }
        for entry in search_path {
            match &**entry {
                "$user" => match user {
                    Some(u) => path.push(schema(u), false, false),
                    None => path.entries.push(PathEntry::unknown()),
                },
                "pg_temp" => {
                    if let Some(t) = temp {
                        path.push(schema(t), true, true);
                    }
                }
                "pg_catalog" => path.push(schema(entry), false, true),
                _ => path.push(schema(entry), false, false),
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
        path.push(NsKey::schema(["temp"]), false, false);
        path.push(NsKey::schema(["main"]), false, false);
        for name in attached {
            if !name.eq_ignore_ascii_case("main") && !name.eq_ignore_ascii_case("temp") {
                path.push(NsKey::schema([name.clone()]), false, false);
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
    fn push(&mut self, ns: NsKey, relations_and_types_only: bool, must_exist: bool) {
        if !self.entries.iter().any(|e| e.ns.as_ref() == Some(&ns)) {
            self.entries.push(PathEntry {
                ns: Some(ns),
                relations_and_types_only,
                must_exist,
            });
        }
    }
}
