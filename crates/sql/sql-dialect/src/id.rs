//! Dialect ids and the context a statement is lexed and split in (Task 3).

use crate::modes::LexModes;
use crate::version::ServerVersion;

/// A SQL dialect. `SqlServer`, `ClickHouse` and `Oracle` have no lexer profile in M3a
/// (`lex_profile` answers `UnsupportedDialect`).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DialectId {
    Postgres,
    MySql,
    MariaDb,
    Sqlite,
    Generic,
    PgWire(PgWireFlavor),
    SqlServer,
    ClickHouse,
    Oracle,
}

/// A server that speaks the PostgreSQL wire protocol and dialect, with its own quirks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PgWireFlavor {
    Redshift,
    Cockroach,
    Yugabyte,
    Timescale,
    Other,
}

/// The client whose script conventions a document follows (psql `\` commands, mysql
/// `DELIMITER`, sqlite3 `.` commands).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ClientFlavor {
    Psql,
    MySqlCli,
    Sqlite3,
    #[default]
    None,
}

/// Everything lexing and splitting depend on: the dialect, the server's version when known,
/// and the modes its settings imply (R-M3-39).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DialectCtx {
    pub id: DialectId,
    pub version: Option<ServerVersion>,
    pub modes: LexModes,
}
