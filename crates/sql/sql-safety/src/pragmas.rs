//! SQLite read `PRAGMA`s the interim backend classifies as reads (Task 10,
//! `sqlite_pragma_allowlist`). Task 18's `hacivat-exec/tests/cross_crate_equalities.rs`
//! compares this list with `hacivat_guard::SQLITE_READ_PRAGMAS` (`sqlite_pragma_lists_are_equal`).
//!
//! STUB until Task 10 (fail closed): the list is empty, so no `PRAGMA` is a read.

/// The SQLite `PRAGMA` names read without an argument that are reads, lower case. STUB: none.
pub fn sqlite_read_pragmas() -> Box<dyn Iterator<Item = &'static str>> {
    Box::new(std::iter::empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_lists_no_pragma() {
        assert_eq!(sqlite_read_pragmas().count(), 0);
    }
}
