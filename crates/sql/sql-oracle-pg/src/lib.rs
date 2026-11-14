//! The PostgreSQL parser oracle over libpg_query (`pg_query =18.1.0`, vendored under
//! `third_party/pg_query` without protobuf regeneration; R-M3-1, P1).
//!
//! Task 1a shipped the build proof ([`version`], [`statement_count`]); Task 9 adds
//! [`split_with_parser`], the byte error cursors of [`PgError`] and the parser thread
//! ([`on_parser_stack`]). Every libpg_query call runs on that one 16 MiB-stack thread, after a
//! check that refuses brackets nested deeper than [`MAX_ORACLE_NESTING`] (R-M3-3, R-M3-35).
//! No `pg_query` type appears in this crate's public API. Messages are the server's English
//! text, never translated.

// ## m3a-t01b-seams: the oracle's modules (Task 9 fills them; it may move `PgError`,
// `version` and `statement_count` into `error.rs` and `split.rs`, unchanged).
pub mod error;
pub mod split;
pub mod stack;

pub use split::split_with_parser;
pub use stack::{MAX_ORACLE_NESTING, on_parser_stack};

// ## m3a-t09-oracle-pg: Task 1a's items, moved into their modules.
pub use error::PgError;
pub use split::{statement_count, version};
