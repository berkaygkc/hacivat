//! The current database and schema of a session (HV-SESS-07; Task 7, Q27): the probe, its
//! parse, the switch statement and when to probe again.
//!
//! The actor probes only after a user statement that [`needs_schema_reprobe`], after a user
//! commit or rollback, and after `SwitchSchema`; never at spawn, never after its own control
//! statements, and never after a reset (the reset's environment plan restores the last schema).

use hacivat_driver::ExecSummary;
use hacivat_proto::DbmsKind;

use crate::session::env::{quote_my_ident, quote_pg_ident};

/// Where a session's statements resolve names.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct SchemaCtx {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub search_path: Vec<String>,
    pub user: Option<String>,
}

/// The engine families whose SQL the session layer writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Family {
    Postgres,
    MySql,
    Sqlite,
    /// An engine without a driver yet: no probe, no schema switch.
    Other,
}

pub(crate) fn family(kind: DbmsKind) -> Family {
    match kind {
        DbmsKind::Postgres => Family::Postgres,
        DbmsKind::MySql | DbmsKind::MariaDb => Family::MySql,
        DbmsKind::Sqlite => Family::Sqlite,
        _ => Family::Other,
    }
}

/// The probe statement of `kind` (empty for an engine without one).
pub fn schema_probe_sql(kind: DbmsKind) -> &'static str {
    match family(kind) {
        Family::Postgres => {
            "SELECT current_database(), current_schema(), \
             array_to_string(current_schemas(true), ','), current_user"
        }
        Family::MySql => "SELECT DATABASE(), CURRENT_USER()",
        Family::Sqlite => "PRAGMA database_list",
        Family::Other => "",
    }
}

/// Reads the probe's answer; nothing known (the default) when it has no row.
pub fn parse_schema_probe(kind: DbmsKind, s: &ExecSummary) -> SchemaCtx {
    fn cell(row: &[Option<String>], i: usize) -> Option<String> {
        row.get(i).cloned().flatten()
    }
    if family(kind) == Family::Sqlite {
        // `seq, name, file` per attached database, `main` first.
        let names: Vec<String> = s.rows.iter().filter_map(|r| cell(r, 1)).collect();
        return SchemaCtx {
            schema: names.first().cloned(),
            search_path: names,
            ..SchemaCtx::default()
        };
    }
    let Some(row) = s.rows.first() else {
        return SchemaCtx::default();
    };
    match family(kind) {
        Family::Postgres => SchemaCtx {
            database: cell(row, 0),
            schema: cell(row, 1),
            search_path: cell(row, 2)
                .map(|p| {
                    p.split(',')
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default(),
            user: cell(row, 3),
        },
        // A MySQL database is its schema.
        Family::MySql => {
            let database = cell(row, 0);
            SchemaCtx {
                schema: database.clone(),
                search_path: database.iter().cloned().collect(),
                database,
                user: cell(row, 1),
            }
        }
        Family::Sqlite | Family::Other => SchemaCtx::default(),
    }
}

/// The statement that makes `name` the current schema (database on MySQL), with `name` quoted
/// exactly (never case-folded). Empty, so refused, for an empty name, a name with NUL (it would
/// end the server's C string early) and an engine without a current schema (SQLite).
pub fn switch_schema_sql(kind: DbmsKind, name: &str) -> String {
    if name.is_empty() || name.contains('\0') {
        return String::new();
    }
    match family(kind) {
        Family::Postgres => format!("SET search_path TO {}", quote_pg_ident(name)),
        Family::MySql => format!("USE {}", quote_my_ident(name)),
        Family::Sqlite | Family::Other => String::new(),
    }
}

/// The first keywords after which the schema context may have changed.
const REPROBE_KEYWORDS: [&str; 10] = [
    "SET", "RESET", "USE", "BEGIN", "START", "COMMIT", "END", "ROLLBACK", "ABORT", "DISCARD",
];

/// Whether `sql` may have changed the schema context: its first keyword (ASCII-folded, after
/// leading whitespace and comments) is a session or transaction control word.
pub fn needs_schema_reprobe(sql: &str) -> bool {
    REPROBE_KEYWORDS.contains(&first_keyword(sql).as_str())
}

/// The first keyword of `sql`, ASCII upper-cased, after whitespace and `--`, `#` and `/* */`
/// comments (not nested; an unterminated comment leaves no keyword). Empty when `sql` does not
/// start with a word.
pub(crate) fn first_keyword(sql: &str) -> String {
    let mut rest = sql;
    loop {
        rest = rest.trim_start();
        if let Some(r) = rest.strip_prefix("--").or_else(|| rest.strip_prefix('#')) {
            rest = r.find('\n').map_or("", |i| &r[i + 1..]);
        } else if let Some(r) = rest.strip_prefix("/*") {
            match r.find("*/") {
                Some(i) => rest = &r[i + 2..],
                None => return String::new(),
            }
        } else {
            break;
        }
    }
    rest.chars()
        .take_while(char::is_ascii_alphabetic)
        .collect::<String>()
        .to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_keyword_skips_comments_and_folds_ascii() {
        assert_eq!(first_keyword("  -- a\n /* b */ select 1"), "SELECT");
        assert_eq!(first_keyword("rollback;"), "ROLLBACK");
        assert_eq!(first_keyword("(select 1)"), "");
        assert_eq!(first_keyword("-- only a comment"), "");
    }
}
