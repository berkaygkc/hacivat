//! The namespaces an unqualified name is looked up in, in order (HV-MODEL-02 per-DBMS matrix;
//! HV-SESS-07 current schema).
//!
//! M4 Task 1a seam: the constructors answer an empty path until Task 9 (a lookup through an
//! empty path finds nothing and answers `Unknown`).

use crate::ids::NsKey;
use crate::name::Name;

/// An ordered list of namespaces.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchPath {
    pub entries: Vec<NsKey>,
}

impl SearchPath {
    /// PostgreSQL: `pg_temp` (relations and types), the `search_path` entries (`$user`
    /// expanded), `pg_catalog` (implicitly first when absent). Task 1a stub: empty.
    pub fn for_pg(database: &Name, search_path: &[Name], user: &Name) -> Self {
        let _ = (database, search_path, user);
        SearchPath::default()
    }

    /// MySQL / MariaDB: the current database. Task 1a stub: empty.
    pub fn for_mysql(current_database: Option<&Name>) -> Self {
        let _ = current_database;
        SearchPath::default()
    }

    /// SQLite: `temp`, `main`, then attached schemas in attach order. Task 1a stub: empty.
    pub fn for_sqlite(attached: &[Name]) -> Self {
        let _ = attached;
        SearchPath::default()
    }
}
