//! Engine-native column types and origin hints (HV-RES-01; Task 6).

use std::sync::Arc;

/// SQLite's type affinity of a declared column type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SqliteAffinity {
    Integer,
    Text,
    Blob,
    Real,
    Numeric,
}

/// The column type as the engine reports it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NativeType {
    Pg {
        oid: u32,
        typmod: i32,
        name: Arc<str>,
    },
    MySql {
        column_type: u8,
        flags: u16,
        charset: u16,
        length: u32,
        decimals: u8,
        // ## m3b-t07-types (ledger 194): on a `TIMESTAMP` column, the session `time_zone` as
        // the server reported it **at connect** (`SYSTEM`, `+03:00`, `Europe/Istanbul`); it
        // may be stale after a `SET time_zone` in the session (read at connect only; fix
        // round 1, review m-1). Metadata for display only: a consumer never converts values
        // with it. `None` for every other column type, or when it was not read.
        time_zone: Option<Arc<str>>,
    },
    Sqlite {
        decl: Option<Arc<str>>,
        affinity: SqliteAffinity,
    },
    Unknown,
}

/// Where a column's values came from, when the engine says (captured now, used in M6).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OriginHint {
    Pg {
        table_oid: u32,
        attnum: i16,
    },
    MySql {
        schema: Arc<str>,
        table: Arc<str>,
        column: Arc<str>,
    },
    Sqlite {
        database: Arc<str>,
        table: Arc<str>,
        column: Arc<str>,
    },
}
