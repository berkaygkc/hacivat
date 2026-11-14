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
    /// The context is folded into the 64-bit seed of the text's hash by chained one-shot
    /// hashes: the modes seed the dialect tag's XXH3-64, that seeds the delimiter's, and that
    /// seeds the text's XXH3-128. Each part is hashed on its own, so no two tuples read alike;
    /// nothing is allocated or copied.
    pub fn of(ctx: &DialectCtx, delimiter: Option<&str>, text: &str) -> StmtKey {
        let seed = XxHash3_64::oneshot_with_seed(
            u64::from(modes_bits(ctx.modes)),
            dialect_tag(ctx.id).as_bytes(),
        );
        let seed = XxHash3_64::oneshot_with_seed(seed, delimiter.unwrap_or(";").as_bytes());
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
