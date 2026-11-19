//! The SQLite producer parts (M4 Task 19). Tables, indexes, views and triggers are their
//! `sqlite_schema.sql` server text, always (R-M4-24; CHECK constraints and generated-column
//! expressions exist only there, F10): `server_definitions` is on for SQLite whatever the
//! option says. Autoindexes, whose `sql` is NULL, are created by their table's constraints and
//! skipped. SQLite has no comments, grants, owners, schemas to create or sequences.

/// Whether `name` is a SQLite autoindex (`sqlite_autoindex_<table>_<n>`; the `sqlite_` prefix
/// is reserved case-insensitively).
pub fn is_autoindex(name: &str) -> bool {
    const PREFIX: &str = "sqlite_autoindex_";
    name.get(..PREFIX.len())
        .is_some_and(|h| h.eq_ignore_ascii_case(PREFIX))
}

#[cfg(test)]
mod tests {
    use super::is_autoindex;

    #[test]
    fn autoindexes_are_recognized_by_their_reserved_prefix() {
        assert!(is_autoindex("sqlite_autoindex_t_1"));
        assert!(is_autoindex("SQLITE_AUTOINDEX_t_1"));
        assert!(!is_autoindex("sqlite_autoindex"));
        assert!(!is_autoindex("t_idx"));
        assert!(!is_autoindex("ı"));
    }
}
