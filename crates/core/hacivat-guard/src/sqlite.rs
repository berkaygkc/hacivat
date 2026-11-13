//! SQLite's read-only layers: connection hardening (`PRAGMA query_only`, `SQLITE_DBCONFIG_*`,
//! limits) and a default-deny authorizer (B21). Task 11 applies them.

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

/// The hardening of a read-only SQLite connection (B21): `PRAGMA query_only`,
/// `SQLITE_DBCONFIG_DEFENSIVE` on, `SQLITE_DBCONFIG_ENABLE_ATTACH_CREATE`/`_WRITE` off, no
/// attached databases, values up to 100 MB, statements up to 1 MiB (the same bound as
/// [`crate::MAX_STATEMENT_BYTES`]) and SQLite's default maximum of bound variables.
pub fn sqlite_hardening() -> SqliteHardening {
    SqliteHardening {
        query_only: true,
        defensive: true,
        attach_create: false,
        attach_write: false,
        limit_attached: 0,
        limit_length: 100_000_000,
        limit_sql_length: 1 << 20,
        limit_variable_number: 32_766,
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

/// Functions a read-only connection never calls: they load code, touch files outside the
/// database (`readfile`, `writefile`, `edit` from the CLI's fileio) or swap tokenizer pointers.
const DENIED_FUNCTIONS: [&str; 5] = [
    "load_extension",
    "readfile",
    "writefile",
    "edit",
    "fts3_tokenizer",
];

/// Pragmas a read-only connection may read without an argument (Q16). With an argument, only
/// the [`SQLITE_INTROSPECTION_PRAGMAS`] are allowed; for every other one an argument is how it
/// is set, so it is denied.
pub const SQLITE_READ_PRAGMAS: [&str; 20] = [
    "table_info",
    "table_xinfo",
    "index_list",
    "index_info",
    "index_xinfo",
    "foreign_key_list",
    "database_list",
    "collation_list",
    "function_list",
    "pragma_list",
    "schema_version",
    "user_version",
    "page_count",
    "page_size",
    "encoding",
    "compile_options",
    "query_only",
    "journal_mode",
    "trusted_schema",
    "foreign_keys",
];

/// Introspection pragmas whose argument is a table or index name (controller ruling on Q16).
/// None of them has a setter: SQLite gives the authorizer the same `(name, argument)` for
/// `PRAGMA table_info(t)`, `PRAGMA table_info = t`, `PRAGMA main.table_info(t)`,
/// `pragma_table_info('t')` and `pragma_table_info('t', 'main')` (the schema arrives
/// separately), and every one of these only reads the schema.
pub const SQLITE_INTROSPECTION_PRAGMAS: [&str; 6] = [
    "table_info",
    "table_xinfo",
    "index_list",
    "index_info",
    "index_xinfo",
    "foreign_key_list",
];

/// The read-only authorizer's answer to `a`. It denies by default and allows only reads
/// (`Read`, `Select`, `Recursive`), functions outside [`DENIED_FUNCTIONS`], the pragmas of
/// [`SQLITE_READ_PRAGMAS`] without an argument, and the [`SQLITE_INTROSPECTION_PRAGMAS`] with
/// a name argument. Names are compared with ASCII case folding (INV-3) and must match whole: a
/// name with a schema prefix or padding is denied (SQLite passes the bare name). The match
/// names every action, so a new one cannot be allowed by accident.
pub fn sqlite_authorize(a: &SqliteAction<'_>) -> Authorization {
    let allow = match a {
        SqliteAction::Read { .. } | SqliteAction::Select | SqliteAction::Recursive => true,
        SqliteAction::Function { name } => !DENIED_FUNCTIONS
            .iter()
            .any(|f| name.eq_ignore_ascii_case(f)),
        SqliteAction::Pragma { name, arg: None } => SQLITE_READ_PRAGMAS
            .iter()
            .any(|p| name.eq_ignore_ascii_case(p)),
        SqliteAction::Pragma { name, arg: Some(_) } => SQLITE_INTROSPECTION_PRAGMAS
            .iter()
            .any(|p| name.eq_ignore_ascii_case(p)),
        SqliteAction::Attach { .. }
        | SqliteAction::Detach
        | SqliteAction::Insert { .. }
        | SqliteAction::Update { .. }
        | SqliteAction::Delete { .. }
        | SqliteAction::Create
        | SqliteAction::Drop
        | SqliteAction::Alter
        | SqliteAction::Transaction
        | SqliteAction::Savepoint
        | SqliteAction::Analyze
        | SqliteAction::Other(_) => false,
    };
    if allow {
        Authorization::Allow
    } else {
        Authorization::Deny
    }
}
