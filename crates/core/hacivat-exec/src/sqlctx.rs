//! The SQL context of a console (R-M3-39; Task 23): the dialect from the engine, the server
//! version from the connection, and the modes from one probe row (PostgreSQL
//! `standard_conforming_strings`, MySQL/MariaDB `sql_mode`).

use hacivat_driver::SemVerLoose;
use hacivat_proto::DbmsKind;
use sql_dialect::{DialectCtx, DialectId};

/// The dialect of `kind` (PostgreSQL-wire flavours are `PgWire`; Task 23).
pub fn dialect_id(kind: DbmsKind) -> DialectId {
    let _ = kind;
    unimplemented!("sqlctx::dialect_id: Task 23")
}

/// The probe that reads the session's lexing modes, if `kind` has one (Task 23).
pub fn probe_sql(kind: DbmsKind) -> Option<&'static str> {
    let _ = kind;
    unimplemented!("sqlctx::probe_sql: Task 23")
}

/// The context of a console on `kind` at `version`, with the probe's answer (Task 23).
pub fn dialect_ctx(
    kind: DbmsKind,
    version: Option<&SemVerLoose>,
    probe_row: Option<&str>,
) -> DialectCtx {
    let _ = (kind, version, probe_row);
    unimplemented!("sqlctx::dialect_ctx: Task 23")
}
