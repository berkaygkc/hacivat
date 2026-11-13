//! The current database and schema of a session (HV-SESS-07; Task 7, Q27): the probe, its
//! parse, the switch statement and when to probe again.

use hacivat_driver::ExecSummary;
use hacivat_proto::DbmsKind;

/// Where a session's statements resolve names.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct SchemaCtx {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub search_path: Vec<String>,
    pub user: Option<String>,
}

/// The probe statement of `kind`. Task 1 stub: none (Task 7).
pub fn schema_probe_sql(_kind: DbmsKind) -> &'static str {
    ""
}

/// Reads the probe's answer. Task 1 stub: nothing known (Task 7).
pub fn parse_schema_probe(_kind: DbmsKind, _s: &ExecSummary) -> SchemaCtx {
    SchemaCtx::default()
}

/// The statement that makes `name` the current schema (database on MySQL). Task 1 stub: none
/// (Task 7).
pub fn switch_schema_sql(_kind: DbmsKind, _name: &str) -> String {
    String::new()
}

/// Whether `sql` may have changed the schema context. Task 1 stub: no (Task 7).
pub fn needs_schema_reprobe(_sql: &str) -> bool {
    false
}
