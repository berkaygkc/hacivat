//! SQLite read `PRAGMA`s the interim backend classifies as reads (Task 10,
//! `sqlite_pragma_allowlist`). Task 18's `hacivat-exec/tests/cross_crate_equalities.rs`
//! compares this list with `hacivat_guard::SQLITE_READ_PRAGMAS` (`sqlite_pragma_lists_are_equal`).
//!
//! The rule matches the read-only authorizer's (`hacivat-guard`, Q16): every name on
//! [`SQLITE_READ_PRAGMAS`] is a read **without an argument** (`PRAGMA user_version`); with an
//! argument (`PRAGMA user_version(7)`, `PRAGMA journal_mode = WAL`) an argument is how most of
//! them are set, so only the [`SQLITE_INTROSPECTION_PRAGMAS`] take one (a table or index name).
//! Names match whole and ASCII case-insensitively (INV-3); everything else is not a read.

/// The pragmas read without an argument, lower case. Equal to the guard's list.
const SQLITE_READ_PRAGMAS: [&str; 20] = [
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

/// The introspection pragmas whose one argument is a table or index name; none has a setter.
/// Equal to the guard's `SQLITE_INTROSPECTION_PRAGMAS`.
pub(crate) const SQLITE_INTROSPECTION_PRAGMAS: [&str; 6] = [
    "table_info",
    "table_xinfo",
    "index_list",
    "index_info",
    "index_xinfo",
    "foreign_key_list",
];

/// The SQLite `PRAGMA` names that are reads when given without an argument, lower case.
pub fn sqlite_read_pragmas() -> Box<dyn Iterator<Item = &'static str>> {
    Box::new(SQLITE_READ_PRAGMAS.into_iter())
}

/// Whether `PRAGMA name` (no argument) is a read.
pub(crate) fn pragma_read_without_arg(name: &str) -> bool {
    SQLITE_READ_PRAGMAS
        .iter()
        .any(|p| name.eq_ignore_ascii_case(p))
}

/// Whether `PRAGMA name(<table or index>)` is a read.
pub(crate) fn pragma_read_with_name_arg(name: &str) -> bool {
    SQLITE_INTROSPECTION_PRAGMAS
        .iter()
        .any(|p| name.eq_ignore_ascii_case(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lists_are_lower_case_unique_and_nested() {
        let mut all: Vec<&str> = sqlite_read_pragmas().collect();
        assert_eq!(all.len(), 20);
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), 20);
        for p in &all {
            assert_eq!(p.to_ascii_lowercase(), *p);
        }
        for p in SQLITE_INTROSPECTION_PRAGMAS {
            assert!(pragma_read_without_arg(p), "{p}");
        }
    }

    #[test]
    fn names_match_whole_and_ascii_only() {
        assert!(pragma_read_without_arg("USER_VERSION"));
        assert!(!pragma_read_without_arg("user_version "));
        assert!(!pragma_read_without_arg("writable_schema"));
        // U+0130 never folds to `i` (INV-3).
        assert!(!pragma_read_with_name_arg("table_\u{130}nfo"));
        assert!(!pragma_read_with_name_arg("user_version"));
    }
}
