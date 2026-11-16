//! SQL dialects (SQL-LEX-01, -04): dialect ids and contexts, server versions, the lexing modes a
//! server's settings imply, keyword tables, and the table-driven lexer (Task 4) and splitter
//! (Task 8) profiles.
//!
//! Types complete since M3a Task 1b. Bodies: Task 3 (`id`, `version`, `modes`, `keywords` and the
//! generated tables, working since Task 3), Task 4 (`lex_profiles`), Task 8 (`split_profile`).
//! Each profile module says whether it still holds Task 1b's placeholder.

pub mod id;
pub mod keywords;
pub mod lex_profiles;
pub mod modes;
pub mod split_profile;
pub mod version;

/// The keyword tables `xtask gen-sql-tables` writes from `data/keywords/*.toml` (Task 3).
mod generated {
    pub(crate) mod keywords_mariadb;
    pub(crate) mod keywords_mysql;
    pub(crate) mod keywords_pg;
    pub(crate) mod keywords_sqlite;
}

pub use id::{ClientFlavor, DialectCtx, DialectId, PgWireFlavor};
pub use keywords::{KeywordId, KwCategory, KwEntry, keyword_lookup, keyword_text, kw};
pub use lex_profiles::{LexDialect, UnsupportedDialect, lex_profile};
pub use modes::LexModes;
pub use split_profile::{SplitProfile, split_profile};
pub use version::{ServerVersion, profile_version};
