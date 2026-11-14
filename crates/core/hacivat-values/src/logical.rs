//! Logical column types and result schemas (HV-RES-01; Task 6).

use std::sync::Arc;

use crate::native::{NativeType, OriginHint};

/// What a column holds, independent of the engine (HV-RES-01). M3a decodes the R-M3-31 set;
/// the rest is requested as text and stored as [`crate::Value::Opaque`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LogicalType {
    Bool,
    Int {
        bits: u8,
        signed: bool,
    },
    Decimal {
        precision: Option<u32>,
        scale: Option<i32>,
    },
    Float {
        bits: u8,
    },
    Money,
    Text {
        max_chars: Option<u32>,
        fixed: bool,
    },
    Bytes {
        max: Option<u32>,
    },
    Uuid,
    Date,
    Time {
        precision: u8,
        tz: bool,
    },
    Timestamp {
        precision: u8,
        tz: bool,
    },
    Interval,
    Json {
        binary: bool,
    },
    Xml,
    Array(Box<LogicalType>),
    Composite(Arc<[(Arc<str>, LogicalType)]>),
    Enum(Arc<[Arc<str>]>),
    Range(Box<LogicalType>),
    Network(NetKind),
    BitString {
        len: Option<u32>,
    },
    Geometry {
        srid: Option<i32>,
    },
    Document,
    Opaque {
        type_name: Arc<str>,
    },
}

/// A network address type (PostgreSQL `inet`, `cidr`, `macaddr`, `macaddr8`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NetKind {
    Inet,
    Cidr,
    MacAddr,
    MacAddr8,
}

/// One result column: its name (data, never translated), logical and native types, and where
/// it came from when the engine says (captured in M3a, used in M6).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ColumnMeta {
    pub name: Arc<str>,
    pub logical: LogicalType,
    pub native: NativeType,
    pub origin: Option<OriginHint>,
}

/// The columns of one result set, in server order (duplicate names keep their order).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResultSchema {
    pub columns: Arc<[ColumnMeta]>,
}
