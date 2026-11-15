//! The SQL context of a console (R-M3-39; Task 23): the dialect from the engine, the server
//! version from the connection, and the modes from one probe row (PostgreSQL
//! `standard_conforming_strings`, MySQL/MariaDB `sql_mode`).
//!
//! The probe runs when a console binds and again after any unit whose first keyword is `SET`
//! (the app's call: [`needs_reprobe`]). Every comparison here is ASCII and locale-free.

use hacivat_driver::SemVerLoose;
use hacivat_proto::DbmsKind;
use sql_dialect::{DialectCtx, DialectId, LexModes, PgWireFlavor, ServerVersion};

/// The dialect of `kind` (PostgreSQL-wire flavours are `PgWire`; Task 23). TiDB speaks the
/// MySQL dialect; engines without a SQL dialect of their own lex as `Generic`.
pub fn dialect_id(kind: DbmsKind) -> DialectId {
    match kind {
        DbmsKind::Postgres => DialectId::Postgres,
        DbmsKind::Redshift => DialectId::PgWire(PgWireFlavor::Redshift),
        DbmsKind::Cockroach => DialectId::PgWire(PgWireFlavor::Cockroach),
        DbmsKind::Yugabyte => DialectId::PgWire(PgWireFlavor::Yugabyte),
        DbmsKind::Timescale => DialectId::PgWire(PgWireFlavor::Timescale),
        DbmsKind::MySql | DbmsKind::TiDb => DialectId::MySql,
        DbmsKind::MariaDb => DialectId::MariaDb,
        DbmsKind::Sqlite => DialectId::Sqlite,
        DbmsKind::MsSql | DbmsKind::AzureSql => DialectId::SqlServer,
        DbmsKind::ClickHouse => DialectId::ClickHouse,
        DbmsKind::Oracle => DialectId::Oracle,
        DbmsKind::Redis
        | DbmsKind::Valkey
        | DbmsKind::MongoDb
        | DbmsKind::DocumentDb
        | DbmsKind::Unknown => DialectId::Generic,
    }
}

/// The probe that reads the session's lexing modes, if `kind` has one (Task 23): PostgreSQL
/// wire servers report `standard_conforming_strings`, the MySQL family its `sql_mode`.
pub fn probe_sql(kind: DbmsKind) -> Option<&'static str> {
    match dialect_id(kind) {
        DialectId::Postgres | DialectId::PgWire(_) => Some("SHOW standard_conforming_strings"),
        DialectId::MySql | DialectId::MariaDb => Some("SELECT @@SESSION.sql_mode"),
        _ => None,
    }
}

/// The context of a console on `kind` at `version`, with the probe's answer (Task 23).
///
/// Without an answer (no probe yet, or it failed) the dialect's defaults stand. A PostgreSQL
/// answer other than `on`/`off` keeps the default (`on`, the server default since 9.1).
pub fn dialect_ctx(
    kind: DbmsKind,
    version: Option<&SemVerLoose>,
    probe_row: Option<&str>,
) -> DialectCtx {
    let id = dialect_id(kind);
    let mut modes = LexModes::default_for(id);
    if let Some(row) = probe_row {
        let row = row.trim();
        match id {
            DialectId::Postgres | DialectId::PgWire(_) => {
                if row.eq_ignore_ascii_case("off") {
                    modes.standard_conforming_strings = false;
                } else if row.eq_ignore_ascii_case("on") {
                    modes.standard_conforming_strings = true;
                }
            }
            DialectId::MySql | DialectId::MariaDb => {
                modes = LexModes::from_mysql_sql_mode(row);
            }
            _ => {}
        }
    }
    DialectCtx {
        id,
        version: version.map(server_version),
        modes,
    }
}

/// Whether the unit `sql` (a strict range: it starts at its first keyword) may have changed the
/// session's lexing modes, so the console probes again: `SET` (R-M3-39), `RESET` and `DISCARD`
/// (which put settings back), and `ROLLBACK`/`ABORT` (which undo a `SET` made inside the
/// transaction; always reprobed, as tracking which transactions set something costs more than
/// one probe row).
pub fn needs_reprobe(sql: &str) -> bool {
    let word = first_word(sql);
    ["SET", "RESET", "DISCARD", "ROLLBACK", "ABORT"]
        .iter()
        .any(|w| word.eq_ignore_ascii_case(w))
}

/// The leading ASCII word of `sql` (empty when it starts with anything else).
pub(crate) fn first_word(sql: &str) -> &str {
    let end = sql
        .bytes()
        .position(|b| !(b.is_ascii_alphabetic() || b == b'_'))
        .unwrap_or(sql.len());
    &sql[..end]
}

fn server_version(v: &SemVerLoose) -> ServerVersion {
    let part = |n: u32| u16::try_from(n).unwrap_or(u16::MAX);
    ServerVersion::new(part(v.major), part(v.minor), part(v.patch))
}
