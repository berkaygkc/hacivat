//! SQL identifiers (M3b Task 1a seams, Task 3 bodies): an identifier with its
//! quoting, qualified names (`schema.name`, `db.schema.name`), dialect folding and comparison.
//!
//! Casing is ASCII-only everywhere (INV-3): PostgreSQL folds unquoted ASCII letters to lower
//! case, MySQL, MariaDB and SQLite keep the text; locale casing is never used, so `İ` and `ı`
//! stay as written.

pub mod fold;
pub mod ident;
// ## m4-t01a-model: identifier keys (Task 3, R-M4-28/-41) and quoting (Task 7), named
// exception in M3's crate; fail-closed stubs until those tasks.
pub mod key;
pub mod qualified;
pub mod quote;

pub use fold::{PG_MAX_IDENT_BYTES, fold, ident_eq};
pub use ident::{Ident, QualifiedName};
pub use qualified::parse_qualified;
