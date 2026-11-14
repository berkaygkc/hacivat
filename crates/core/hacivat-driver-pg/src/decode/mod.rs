//! PostgreSQL wire values to `hacivat_values` (HV-RES-01 PG mapping, R-M3-31; Task 12).
//!
//! Each result column gets a [`Col`] from its type, which decides the format it is requested in
//! and how its cells are decoded:
//!
//! - **Binary** for the R-M3-31 scalar set and enum labels: `bool`, `int2`/`int4`/`int8`,
//!   `oid`/`xid`/`cid`, `float4`/`float8`, `numeric`, the text family (`text`, `varchar`,
//!   `bpchar`, `name`, `"char"`), `bytea`, `uuid`, `json`, `jsonb`, `date`, `time`, `timetz`,
//!   `timestamp`, `timestamptz`, `interval`, and every enum (its label).
//! - **Text** for everything else (arrays, composites and `record`, ranges, network types, bit
//!   strings, geometry, `money`, `reg*`, XML, `tsvector`, extension types, unknown OIDs),
//!   stored as `Value::Opaque { type_name, text }` in a `Values` buffer (R-M3-31, P8). Each cell
//!   holds its own `OpaqueVal` (32 bytes plus the shared type name's length and the text, as the
//!   budget counts it); the type name is one `Arc<str>` shared by the column's cells. Text that
//!   is not UTF-8 (a `SQL_ASCII` database) is kept as `Lossy` bytes, and text past the cell cap
//!   as `Truncated` with a `Text` prefix, exactly as a `Var(Text)` column keeps them.
//!
//! Chunks are built with `ChunkBuilder::with_schema`, so a temporal column's buffer knows its
//! logical type. **Server bytes never panic and are never altered:** a cell whose bytes no
//! PostgreSQL server sends (a wrong length, a numeric digit of 10,000, an unknown sign word, a
//! `jsonb` version other than 1, more than [`MAX_NUMERIC_DIGITS`] numeric digits) becomes a
//! `Value::Error` cell and the row is kept; `±infinity` and `NaN` are `Value::Special`; a
//! timestamp past `i64` microseconds since 1970 (about 292277 AD) is kept exactly as an
//! exception; text is checked for UTF-8 and capped by the chunk builder in one pass.

use std::sync::Arc;

use hacivat_values::{
    BufKind, ChunkBuilder, ColumnMeta, DecimalVal, ErrorVal, LogicalType, NativeType, OriginHint,
    ResultSchema, TempUnit, Value, VarKind,
};
use postgres_types::{Kind, Type};

pub mod numeric;
pub mod scalar;
pub mod temporal;

/// The most base-10000 digits a `numeric` cell may have; more is an `Error` cell (a bound on
/// what one hostile cell costs; PostgreSQL's own limit is about 36,900).
pub const MAX_NUMERIC_DIGITS: usize = 1_000;

/// The result format code of a text column.
pub const TEXT_FORMAT: i16 = 0;
/// The result format code of a binary column.
pub const BINARY_FORMAT: i16 = 1;

/// PostgreSQL's built-in type OIDs (`pg_type.dat`; fixed since PostgreSQL 7.x).
mod oid {
    pub const BOOL: u32 = 16;
    pub const BYTEA: u32 = 17;
    pub const CHAR: u32 = 18;
    pub const NAME: u32 = 19;
    pub const INT8: u32 = 20;
    pub const INT2: u32 = 21;
    pub const INT4: u32 = 23;
    pub const TEXT: u32 = 25;
    pub const OID: u32 = 26;
    pub const XID: u32 = 28;
    pub const CID: u32 = 29;
    pub const JSON: u32 = 114;
    pub const FLOAT4: u32 = 700;
    pub const FLOAT8: u32 = 701;
    pub const BPCHAR: u32 = 1042;
    pub const VARCHAR: u32 = 1043;
    pub const DATE: u32 = 1082;
    pub const TIME: u32 = 1083;
    pub const TIMESTAMP: u32 = 1114;
    pub const TIMESTAMPTZ: u32 = 1184;
    pub const INTERVAL: u32 = 1186;
    pub const TIMETZ: u32 = 1266;
    pub const NUMERIC: u32 = 1700;
    pub const UUID: u32 = 2950;
    pub const JSONB: u32 = 3802;
}

/// How one result column is requested and decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Col {
    Bool,
    Int2,
    Int4,
    Int8,
    /// `oid`, `xid`, `cid`: unsigned 32-bit.
    Oid,
    Float4,
    Float8,
    Numeric,
    /// `text`, `varchar`, `bpchar`, `name`, `"char"`: the server's bytes, UTF-8 under
    /// `client_encoding = UTF8` (any bytes in a `SQL_ASCII` database).
    Text,
    /// An enum's label (binary is the label's text).
    Enum,
    Bytea,
    Uuid,
    Json,
    /// A version byte (1), then the JSON text.
    Jsonb,
    Date,
    Time,
    TimeTz,
    Timestamp,
    TimestampTz,
    Interval,
    /// Requested as text, kept as `Value::Opaque` with this type name.
    Opaque(Arc<str>),
}

impl Col {
    /// The decoder of a column of `ty`.
    pub fn for_type(ty: &Type) -> Col {
        match ty.oid() {
            oid::BOOL => Col::Bool,
            oid::INT2 => Col::Int2,
            oid::INT4 => Col::Int4,
            oid::INT8 => Col::Int8,
            oid::OID | oid::XID | oid::CID => Col::Oid,
            oid::FLOAT4 => Col::Float4,
            oid::FLOAT8 => Col::Float8,
            oid::NUMERIC => Col::Numeric,
            oid::TEXT | oid::VARCHAR | oid::BPCHAR | oid::NAME | oid::CHAR => Col::Text,
            oid::BYTEA => Col::Bytea,
            oid::UUID => Col::Uuid,
            oid::JSON => Col::Json,
            oid::JSONB => Col::Jsonb,
            oid::DATE => Col::Date,
            oid::TIME => Col::Time,
            oid::TIMETZ => Col::TimeTz,
            oid::TIMESTAMP => Col::Timestamp,
            oid::TIMESTAMPTZ => Col::TimestampTz,
            oid::INTERVAL => Col::Interval,
            _ if matches!(ty.kind(), Kind::Enum(_)) => Col::Enum,
            _ => Col::Opaque(ty.name().into()),
        }
    }

    /// The result format code to request ([`BINARY_FORMAT`] or [`TEXT_FORMAT`]).
    pub fn format(&self) -> i16 {
        match self {
            Col::Opaque(_) => TEXT_FORMAT,
            _ => BINARY_FORMAT,
        }
    }

    /// The chunk buffer of the column, given its type modifier.
    pub fn buf_kind(&self, typmod: i32) -> BufKind {
        match self {
            Col::Bool => BufKind::Bool,
            Col::Int2 | Col::Int4 | Col::Int8 | Col::Oid => BufKind::I64,
            Col::Float4 => BufKind::F64 { f32: true },
            Col::Float8 => BufKind::F64 { f32: false },
            Col::Numeric => match numeric_mod(typmod) {
                // At most 18 digits: an `i64` mantissa at the column's scale. Values have no
                // digits after the point when the scale is negative (PostgreSQL 15+).
                Some((precision, scale)) if (1..=18).contains(&precision) => BufKind::Dec64 {
                    scale: scale.max(0) as i16,
                },
                _ => BufKind::Dec128,
            },
            Col::Text | Col::Enum => BufKind::Var(VarKind::Text),
            Col::Bytea => BufKind::Var(VarKind::Bytes),
            Col::Uuid => BufKind::Fixed16,
            Col::Json | Col::Jsonb => BufKind::Var(VarKind::Json),
            Col::Date => BufKind::Temporal {
                unit: TempUnit::Days,
            },
            Col::Time | Col::Timestamp | Col::TimestampTz => BufKind::Temporal {
                unit: TempUnit::Micros,
            },
            Col::TimeTz => BufKind::Temporal2,
            Col::Interval | Col::Opaque(_) => BufKind::Values,
        }
    }

    /// The logical type of a column of `ty` with `typmod`.
    pub fn logical(&self, ty: &Type, typmod: i32) -> LogicalType {
        let text_len = || (typmod >= VARHDRSZ).then(|| (typmod - VARHDRSZ) as u32);
        match self {
            Col::Bool => LogicalType::Bool,
            Col::Int2 => LogicalType::Int {
                bits: 16,
                signed: true,
            },
            Col::Int4 => LogicalType::Int {
                bits: 32,
                signed: true,
            },
            Col::Int8 => LogicalType::Int {
                bits: 64,
                signed: true,
            },
            Col::Oid => LogicalType::Int {
                bits: 32,
                signed: false,
            },
            Col::Float4 => LogicalType::Float { bits: 32 },
            Col::Float8 => LogicalType::Float { bits: 64 },
            Col::Numeric => match numeric_mod(typmod) {
                Some((precision, scale)) => LogicalType::Decimal {
                    precision: Some(precision),
                    scale: Some(scale),
                },
                None => LogicalType::Decimal {
                    precision: None,
                    scale: None,
                },
            },
            Col::Text => match ty.oid() {
                oid::VARCHAR => LogicalType::Text {
                    max_chars: text_len(),
                    fixed: false,
                },
                oid::BPCHAR => LogicalType::Text {
                    max_chars: text_len(),
                    fixed: true,
                },
                oid::CHAR => LogicalType::Text {
                    max_chars: Some(1),
                    fixed: true,
                },
                _ => LogicalType::Text {
                    max_chars: None,
                    fixed: false,
                },
            },
            Col::Enum => match ty.kind() {
                Kind::Enum(labels) => {
                    LogicalType::Enum(labels.iter().map(|l| Arc::from(l.as_str())).collect())
                }
                _ => LogicalType::Enum(Arc::from([])),
            },
            Col::Bytea => LogicalType::Bytes { max: None },
            Col::Uuid => LogicalType::Uuid,
            Col::Json => LogicalType::Json { binary: false },
            Col::Jsonb => LogicalType::Json { binary: true },
            Col::Date => LogicalType::Date,
            Col::Time => LogicalType::Time {
                precision: time_precision(typmod),
                tz: false,
            },
            Col::TimeTz => LogicalType::Time {
                precision: time_precision(typmod),
                tz: true,
            },
            Col::Timestamp => LogicalType::Timestamp {
                precision: time_precision(typmod),
                tz: false,
            },
            Col::TimestampTz => LogicalType::Timestamp {
                precision: time_precision(typmod),
                tz: true,
            },
            Col::Interval => LogicalType::Interval,
            Col::Opaque(type_name) => LogicalType::Opaque {
                type_name: type_name.clone(),
            },
        }
    }

    /// Decodes one non-NULL cell. `cell_cap` applies to opaque text here; the chunk builder caps
    /// `Var` cells.
    pub fn decode<'a>(&self, raw: &'a [u8], cell_cap: u32) -> Decoded<'a> {
        match self {
            Col::Bool => scalar::bool(raw),
            Col::Int2 => scalar::int2(raw),
            Col::Int4 => scalar::int4(raw),
            Col::Int8 => scalar::int8(raw),
            Col::Oid => scalar::oid(raw),
            Col::Float4 => scalar::float4(raw),
            Col::Float8 => scalar::float8(raw),
            Col::Numeric => numeric::numeric(raw),
            Col::Text | Col::Enum | Col::Bytea | Col::Json => Decoded::Var(raw),
            Col::Jsonb => scalar::jsonb(raw),
            Col::Uuid => scalar::uuid(raw),
            Col::Date => temporal::date(raw),
            Col::Time => temporal::time(raw),
            Col::TimeTz => temporal::timetz(raw),
            Col::Timestamp => temporal::timestamp(raw, false),
            Col::TimestampTz => temporal::timestamp(raw, true),
            Col::Interval => temporal::interval(raw),
            Col::Opaque(type_name) => Decoded::Value(scalar::opaque(type_name, raw, cell_cap)),
        }
    }
}

/// `varlena`'s header size: the offset of `varchar(n)`, `bpchar(n)` and `numeric(p, s)` type
/// modifiers.
const VARHDRSZ: i32 = 4;

/// `numeric(p, s)`'s precision and scale from its type modifier (`None`: unconstrained). The
/// scale is an 11-bit two's complement field since PostgreSQL 15 (negative scales); earlier
/// servers never set its sign bit.
fn numeric_mod(typmod: i32) -> Option<(u32, i32)> {
    if typmod < VARHDRSZ {
        return None;
    }
    let v = typmod - VARHDRSZ;
    let precision = ((v >> 16) & 0xffff) as u32;
    let scale = ((v & 0x7ff) ^ 1024) - 1024;
    Some((precision, scale))
}

/// The fractional-second digits of a time or timestamp column (6 unless constrained).
fn time_precision(typmod: i32) -> u8 {
    match typmod {
        0..=6 => typmod as u8,
        _ => 6,
    }
}

/// One decoded non-NULL cell, before it goes into a chunk. Each [`Col`] produces only the
/// variants its buffer takes, plus `Value` (which every buffer takes, as an exception when it
/// is not the buffer's kind).
#[derive(Debug, Clone, PartialEq)]
pub enum Decoded<'a> {
    Bool(bool),
    Int(i64),
    /// A float (an `f32` widened, which is exact).
    Float(f64),
    Decimal(DecimalVal),
    /// Text, JSON or bytes as the server sent them: the chunk builder checks UTF-8 (else
    /// `Lossy`) and applies the cell cap (else `Truncated`).
    Var(&'a [u8]),
    Uuid([u8; 16]),
    /// Days since 1970-01-01, or microseconds (of the day, or since 1970-01-01).
    Temporal(i64),
    /// Microseconds of the day and an offset in seconds east of UTC.
    Temporal2(i64, i32),
    /// A whole value: an interval, a special, an exact exception, an opaque text, an error.
    Value(Value),
}

/// A cell that could not be decoded: the row is kept.
pub fn error(code: &'static str, message: String) -> Decoded<'static> {
    Decoded::Value(Value::Error(Arc::new(ErrorVal {
        code: code.into(),
        message: message.into(),
    })))
}

/// A fixed-width value of the wrong length.
pub(crate) fn bad_length(what: &str, got: usize, want: usize) -> Decoded<'static> {
    error(
        "pg-length",
        format!("a {what} value of {got} bytes (expected {want})"),
    )
}

/// Pushes one cell (`None`: SQL NULL) of column `col`.
pub(crate) fn push(b: &mut ChunkBuilder, col: u32, decoder: &Col, raw: Option<&[u8]>, cap: u32) {
    let Some(raw) = raw else {
        b.push_null(col);
        return;
    };
    match decoder.decode(raw, cap) {
        Decoded::Bool(v) => b.push_bool(col, v),
        Decoded::Int(v) => b.push_i64(col, v),
        Decoded::Float(v) => b.push_f64(col, v),
        Decoded::Decimal(v) => b.push_decimal(col, &v),
        Decoded::Var(bytes) => b.push_bytes(col, bytes, cap),
        Decoded::Uuid(v) => b.push_fixed16(col, v),
        Decoded::Temporal(v) => b.push_temporal(col, v),
        Decoded::Temporal2(v, offset) => b.push_temporal2(col, v, offset),
        Decoded::Value(v) => b.push_value(col, v),
    }
}

/// One column as the statement describes it.
#[derive(Debug, Clone)]
pub struct ColumnDesc<'a> {
    pub name: &'a str,
    pub ty: &'a Type,
    pub typmod: i32,
    pub table_oid: Option<u32>,
    pub attnum: Option<i16>,
}

impl<'a> From<&'a tokio_postgres::Column> for ColumnDesc<'a> {
    fn from(c: &'a tokio_postgres::Column) -> Self {
        ColumnDesc {
            name: c.name(),
            ty: c.type_(),
            typmod: c.type_modifier(),
            table_oid: c.table_oid(),
            attnum: c.column_id(),
        }
    }
}

/// The decode plan of one result set: its schema, each column's decoder, buffer and format.
#[derive(Debug, Clone)]
pub struct Plan {
    pub schema: Arc<ResultSchema>,
    pub cols: Vec<Col>,
    pub kinds: Vec<BufKind>,
    pub formats: Vec<i16>,
}

impl Plan {
    pub fn new<'a>(columns: impl IntoIterator<Item = ColumnDesc<'a>>) -> Plan {
        let mut cols = Vec::new();
        let mut kinds = Vec::new();
        let mut formats = Vec::new();
        let mut meta = Vec::new();
        for c in columns {
            let col = Col::for_type(c.ty);
            kinds.push(col.buf_kind(c.typmod));
            formats.push(col.format());
            meta.push(ColumnMeta {
                name: c.name.into(),
                logical: col.logical(c.ty, c.typmod),
                native: NativeType::Pg {
                    oid: c.ty.oid(),
                    typmod: c.typmod,
                    name: c.ty.name().into(),
                },
                origin: match (c.table_oid, c.attnum) {
                    (Some(table_oid), Some(attnum)) => Some(OriginHint::Pg { table_oid, attnum }),
                    _ => None,
                },
            });
            cols.push(col);
        }
        Plan {
            schema: Arc::new(ResultSchema {
                columns: meta.into(),
            }),
            cols,
            kinds,
            formats,
        }
    }

    /// The plan of a prepared statement's columns.
    pub fn for_columns(columns: &[tokio_postgres::Column]) -> Plan {
        Plan::new(columns.iter().map(ColumnDesc::from))
    }

    /// A chunk builder for this result set (`with_schema`, so temporal columns are typed).
    pub fn builder(&self, capacity_rows: u32) -> ChunkBuilder {
        ChunkBuilder::with_schema(&self.kinds, &self.schema, capacity_rows)
    }

    /// Pushes the cells of one row (`None`: SQL NULL) and ends it. `cells` yields one cell per
    /// column.
    pub fn push_row<'r>(
        &self,
        b: &mut ChunkBuilder,
        cells: impl IntoIterator<Item = Option<&'r [u8]>>,
        cell_cap: u32,
    ) {
        for ((i, decoder), raw) in self.cols.iter().enumerate().zip(cells) {
            push(b, i as u32, decoder, raw, cell_cap);
        }
        b.end_row();
    }
}

/// The result format code a column of `ty` is requested in.
pub fn format_for(ty: &Type) -> i16 {
    Col::for_type(ty).format()
}

/// The logical type of a column of `ty` with `typmod`.
pub fn logical_for(ty: &Type, typmod: i32) -> LogicalType {
    Col::for_type(ty).logical(ty, typmod)
}

/// The chunk buffer of a column of `ty` with `typmod`.
pub fn buf_kind_for(ty: &Type, typmod: i32) -> BufKind {
    Col::for_type(ty).buf_kind(typmod)
}

/// One cell of a column of `ty` through the real path (its plan, a one-row chunk built with
/// `with_schema`), read back from the chunk: what the result store holds for these bytes.
pub fn decode_cell(ty: &Type, typmod: i32, raw: Option<&[u8]>, cell_cap: u32) -> Value {
    let plan = Plan::new([ColumnDesc {
        name: "c",
        ty,
        typmod,
        table_oid: None,
        attnum: None,
    }]);
    let mut b = plan.builder(1);
    plan.push_row(&mut b, [raw], cell_cap);
    b.seal().cell(0, 0).to_value()
}
