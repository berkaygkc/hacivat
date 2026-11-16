//! The parse tree, scanner tokens and fingerprint of a PostgreSQL text, for `sql-safety`'s PG
//! backend (Task 9). Every call runs on the oracle's one 16 MiB parser thread behind the same
//! guard as the splitter (8 MiB cap, bracket depth ≤ 256); a prost decode-depth failure is an
//! `Err`, never a partial tree.
//!
//! [`pb`] is the only re-export of `pg_query` types (`BackendOutsideOwner`): `sql-safety`
//! walks the tree through it, and nothing outside `sql-safety` uses it.
//!
//! STUB until Task 9 (fail closed): every function answers `Err`.

use sql_text::TextRange;

use crate::error::PgError;

pub use pg_query::protobuf as pb;

/// What a scanner token is, coarsely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScanKind {
    Keyword,
    Ident,
    Literal,
    Operator,
    Param,
    Comment,
    Other,
}

fn not_yet(what: &str) -> PgError {
    PgError::own(&format!("{what} is not implemented yet (Task 9)"), None)
}

/// libpg_query's parse tree of `sql`. STUB: `Err`.
pub fn parse_tree(sql: &str) -> Result<pb::ParseResult, PgError> {
    let _ = sql;
    Err(not_yet("parse_tree"))
}

/// libpg_query's scanner tokens of `sql`. STUB: `Err`.
pub fn scan_tokens(sql: &str) -> Result<Vec<(TextRange, ScanKind)>, PgError> {
    let _ = sql;
    Err(not_yet("scan_tokens"))
}

/// libpg_query's fingerprint of `sql`. STUB: `Err`.
pub fn fingerprint(sql: &str) -> Result<u64, PgError> {
    let _ = sql;
    Err(not_yet("fingerprint"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stubs_fail_closed() {
        assert!(parse_tree("SELECT 1").is_err());
        assert!(scan_tokens("SELECT 1").is_err());
        assert!(fingerprint("SELECT 1").is_err());
    }
}
