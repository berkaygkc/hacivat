//! SQLite's read-only layers: connection hardening (`PRAGMA query_only`, `SQLITE_DBCONFIG_*`,
//! limits) and a default-deny authorizer (B21). Task 5 fills the bodies; Task 11 applies them.

/// The settings a read-only SQLite connection is opened with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SqliteHardening {
    pub query_only: bool,
    pub defensive: bool,
    pub attach_create: bool,
    pub attach_write: bool,
    pub limit_attached: i32,
    pub limit_length: i32,
    pub limit_sql_length: i32,
    pub limit_variable_number: i32,
}

/// The hardening of a read-only SQLite connection. Task 1 stub: the strictest settings (every
/// switch closed, every limit 0) until Task 5 sets the working values.
pub fn sqlite_hardening() -> SqliteHardening {
    SqliteHardening {
        query_only: true,
        defensive: true,
        attach_create: false,
        attach_write: false,
        limit_attached: 0,
        limit_length: 0,
        limit_sql_length: 0,
        limit_variable_number: 0,
    }
}

/// One action SQLite's authorizer asks about (`SQLITE_*` action codes, as Task 11's adapter
/// maps them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SqliteAction<'a> {
    Read {
        table: &'a str,
        column: &'a str,
    },
    Select,
    Function {
        name: &'a str,
    },
    Pragma {
        name: &'a str,
        arg: Option<&'a str>,
    },
    Attach {
        file: &'a str,
    },
    Detach,
    Insert {
        table: &'a str,
    },
    Update {
        table: &'a str,
    },
    Delete {
        table: &'a str,
    },
    Create,
    Drop,
    Alter,
    Transaction,
    Savepoint,
    Analyze,
    Recursive,
    /// Any other action code.
    Other(i32),
}

/// The authorizer's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Authorization {
    Allow,
    Deny,
    Ignore,
}

/// The read-only authorizer's answer to `a`. Task 1 stub: denies everything until Task 5.
pub fn sqlite_authorize(_a: &SqliteAction<'_>) -> Authorization {
    Authorization::Deny
}
