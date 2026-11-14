//! Content-addressed statement keys (R-M3-4, R-M3-42; Task 8).

use sql_dialect::{DialectCtx, DialectId, LexModes, PgWireFlavor};
use twox_hash::{XxHash3_64, XxHash3_128};

/// The XXH3-128 (`twox-hash`) of a statement's dialect, modes, delimiter and text; no
/// interner. Equal statement texts under the same dialect, modes and delimiter share a key
/// wherever they stand; the key is build-stable but only ever kept in memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StmtKey(pub u128);

impl StmtKey {
    /// The key of `text` split under `ctx` with the client `delimiter` (`None`: `;`).
    ///
    /// The context (dialect, modes, delimiter; each part length-prefixed, so no two tuples
    /// read alike) is folded into the hash's 64-bit seed (XXH3-64 of those bytes), and the text
    /// is hashed once with it: no allocation, no copy of the statement.
    pub fn of(ctx: &DialectCtx, delimiter: Option<&str>, text: &str) -> StmtKey {
        let mut context = Vec::with_capacity(48);
        for part in [
            dialect_tag(ctx.id).as_bytes(),
            &[modes_bits(ctx.modes)],
            delimiter.unwrap_or(";").as_bytes(),
        ] {
            context.extend_from_slice(&(part.len() as u64).to_le_bytes());
            context.extend_from_slice(part);
        }
        let seed = XxHash3_64::oneshot(&context);
        StmtKey(XxHash3_128::oneshot_with_seed(seed, text.as_bytes()))
    }
}

/// A stable name for each dialect (the enum's discriminants are not part of the key).
fn dialect_tag(id: DialectId) -> &'static str {
    match id {
        DialectId::Postgres => "postgres",
        DialectId::MySql => "mysql",
        DialectId::MariaDb => "mariadb",
        DialectId::Sqlite => "sqlite",
        DialectId::Generic => "generic",
        DialectId::PgWire(PgWireFlavor::Redshift) => "pgwire-redshift",
        DialectId::PgWire(PgWireFlavor::Cockroach) => "pgwire-cockroach",
        DialectId::PgWire(PgWireFlavor::Yugabyte) => "pgwire-yugabyte",
        DialectId::PgWire(PgWireFlavor::Timescale) => "pgwire-timescale",
        DialectId::PgWire(PgWireFlavor::Other) => "pgwire-other",
        DialectId::SqlServer => "sqlserver",
        DialectId::ClickHouse => "clickhouse",
        DialectId::Oracle => "oracle",
        _ => "other",
    }
}

fn modes_bits(m: LexModes) -> u8 {
    u8::from(m.standard_conforming_strings)
        | u8::from(m.ansi_quotes) << 1
        | u8::from(m.no_backslash_escapes) << 2
        | u8::from(m.pipes_as_concat) << 3
        | u8::from(m.high_not_precedence) << 4
        | u8::from(m.mariadb_oracle_mode) << 5
}
