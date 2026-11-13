//! The PostgreSQL parser oracle over libpg_query (`pg_query =18.1.0`, vendored under
//! `third_party/pg_query` without protobuf regeneration; R-M3-1, P1).
//!
//! Task 1a ships the build proof: [`version`] and [`statement_count`] work and link the C
//! library on every OS. No `pg_query` type appears in this crate's public API. Messages are
//! the server's English text, never translated.

use std::fmt;
use std::sync::OnceLock;

use text_size::TextSize;

/// A libpg_query error: the server's English message and, when known, the byte offset it
/// points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgError {
    /// The message as libpg_query reports it, untranslated.
    pub message: String,
    /// The byte offset into the input the error points at, when known.
    pub cursor: Option<TextSize>,
}

impl fmt::Display for PgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PgError {}

/// Private on purpose: a `From` impl would put a `pg_query` type into the public API.
fn pg_error(e: pg_query::Error) -> PgError {
    let message = match e {
        pg_query::Error::Parse(m) => m,
        other => other.to_string(),
    };
    PgError {
        message,
        cursor: None,
    }
}

/// The crate version Hacivat pins, kept equal to the `pg_query = "=18.1.0"` workspace line.
const PG_QUERY_VERSION: &str = "18.1.0";

/// `"pg_query 18.1.0 (PostgreSQL 18.6)"`. The PostgreSQL version is the one the linked C
/// library reports (`PG_VERSION_NUM` in its parse result), read once.
pub fn version() -> &'static str {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(|| {
        let server = match pg_query::parse("", 0) {
            Ok(r) => pg_version_text(r.protobuf.version),
            Err(_) => "unknown".to_string(),
        };
        format!("pg_query {PG_QUERY_VERSION} (PostgreSQL {server})")
    })
}

/// `180006` → `"18.6"` (the PostgreSQL 10+ numbering).
fn pg_version_text(num: i32) -> String {
    format!("{}.{}", num / 10_000, num % 10_000)
}

/// How many statements PostgreSQL's own grammar finds in `sql`; a syntax error is an `Err`,
/// never a panic.
pub fn statement_count(sql: &str) -> Result<usize, PgError> {
    let parsed = pg_query::parse(sql, 0).map_err(pg_error)?;
    Ok(parsed.protobuf.stmts.len())
}
