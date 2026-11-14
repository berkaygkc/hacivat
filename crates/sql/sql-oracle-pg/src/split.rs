//! Statement ranges from PostgreSQL's own grammar (Task 9): `hacivat-exec` resolves an
//! ambiguous PostgreSQL boundary with it (R-M3-24), on the parser stack (R-M3-35).

use sql_text::TextRange;

use crate::PgError;

/// The statement ranges libpg_query finds in `sql` (`$name` placeholders normalised to `$n`
/// first, so the parser accepts them; Task 9).
///
/// Task 1b stub (fail closed): an error, so no ambiguity is resolved by the oracle.
pub fn split_with_parser(sql: &str) -> Result<Vec<TextRange>, PgError> {
    let _ = sql;
    Err(PgError {
        message: "split_with_parser is not implemented yet".to_string(),
        cursor: None,
    })
}
