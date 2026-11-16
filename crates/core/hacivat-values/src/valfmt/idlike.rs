//! The ID heuristic (Task 16; R11): ID-like columns stay ungrouped.
//!
//! M3a has no column origins (PK and FK membership is M6), so the name and the type decide.
//! Every comparison is ASCII (INV-3): `İD` or `ıd` are not `id`, and no locale casing runs.

use hacivat_proto::DbmsKind;

use crate::logical::LogicalType;

/// PostgreSQL's object-identifier types: `oid`, the `reg*` aliases, and the transaction and
/// command ids that share their representation. They mean that only on the PostgreSQL wire
/// family: SQLite takes any declared type name, so a SQLite column declared `oid` is a plain
/// integer.
const PG_ID_TYPES: &[&str] = &[
    "oid",
    "xid",
    "xid8",
    "cid",
    "regclass",
    "regcollation",
    "regconfig",
    "regdictionary",
    "regnamespace",
    "regoper",
    "regoperator",
    "regproc",
    "regprocedure",
    "regrole",
    "regtype",
];

/// Whether a column is ID-like: an integer column (an integer-valued `numeric(p, 0)`
/// included) named `id`, ending in `_id`, or ending in a camel-case `Id`/`ID` after a
/// lower-case letter (`customerId`, `CustomerID`), all ASCII, case-insensitive for `id` and
/// `_id`, on every engine; or, on the PostgreSQL wire family (`engine`: PostgreSQL, Redshift,
/// CockroachDB, YugabyteDB, TimescaleDB), an `oid`/`reg*`/`xid`/`cid` column whatever its name
/// (T16 review M1: the engine argument is a named addition to Task 1b's signature).
pub fn id_like(name: &str, logical: &LogicalType, native_name: &str, engine: DbmsKind) -> bool {
    let pg_wire = matches!(
        engine,
        DbmsKind::Postgres
            | DbmsKind::Redshift
            | DbmsKind::Cockroach
            | DbmsKind::Yugabyte
            | DbmsKind::Timescale
    );
    if pg_wire
        && PG_ID_TYPES
            .iter()
            .any(|t| native_name.eq_ignore_ascii_case(t))
    {
        return true;
    }
    let integer = matches!(
        logical,
        LogicalType::Int { .. } | LogicalType::Decimal { scale: Some(0), .. }
    );
    integer && named_like_an_id(name.as_bytes())
}

fn named_like_an_id(name: &[u8]) -> bool {
    let n = name.len();
    if n < 2 {
        return false;
    }
    let tail = &name[n - 2..];
    if !tail.eq_ignore_ascii_case(b"id") {
        return false;
    }
    if n == 2 {
        return true;
    }
    let before = name[n - 3];
    // `_id`, `_ID`; or camel case: a lower-case letter, then `Id` or `ID`.
    before == b'_' || (before.is_ascii_lowercase() && tail[0] == b'I')
}
