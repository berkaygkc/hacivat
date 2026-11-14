//! One cell value and its borrowed mirror (HV-RES-01; Task 6).
//!
//! `Value` is at most 32 bytes (`tests/m3a_types.rs`): every wide payload sits behind an `Arc`
//! or a `Box` (P8, ADR 0006), so the largest inline payload is 24 bytes (`Decimal`,
//! `TimestampTz`). Bulk bytes and text live in a chunk's `Var` buffers; a `Value` is the
//! exception path and the `Values` buffer of `Chunk::from_values`.

use std::sync::Arc;

use crate::decimal::{DecimalRef, DecimalVal};
use crate::special::SpecialVal;
use crate::temporal::{DateVal, TimeVal, TsVal};

/// One cell. `Null` is SQL NULL; `Missing` a cell the row did not carry; `Default` the
/// `DEFAULT` keyword of an edit (M6).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Missing,
    Default,
    Bool(bool),
    Int(i64),
    UInt(u64),
    F32(f32),
    F64(f64),
    Decimal(DecimalVal),
    Text(Arc<str>),
    Bytes(Arc<[u8]>),
    Uuid([u8; 16]),
    Date(DateVal),
    Time(TimeVal),
    /// A time of day and its UTC offset in seconds, east positive.
    TimeTz(TimeVal, i32),
    Timestamp(TsVal),
    TimestampTz {
        utc: TsVal,
        /// The offset the server sent, seconds east of UTC (`None`: shown in the display zone).
        offset: Option<i32>,
    },
    Interval {
        months: i32,
        days: i32,
        micros: i64,
    },
    Json(Arc<str>),
    Xml(Arc<str>),
    Special(SpecialVal),
    /// A value of a type M3a does not decode, as the server's text (R-M3-31).
    Opaque(Arc<OpaqueVal>),
    /// Text that is not valid UTF-8, kept as bytes (never replaced).
    Lossy(Arc<[u8]>),
    /// A value cut at the cell cap, with its total length.
    Truncated(Box<TruncatedVal>),
    /// A value that could not be decoded; the row is kept.
    Error(Arc<ErrorVal>),
}

/// A value of a type M3a does not decode: the type's name and the server's text.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OpaqueVal {
    pub type_name: Arc<str>,
    pub text: Arc<str>,
}

/// Why a cell could not be decoded: a stable code and an English message (never shown inside
/// a translated sentence).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ErrorVal {
    pub code: Arc<str>,
    pub message: Arc<str>,
}

/// The unit of a truncated value's total length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LenUnit {
    Bytes,
    Chars,
}

/// A value cut at the cell cap: the kept prefix (a `Text` or `Bytes` value, cut at a character
/// boundary for text) and the total length when known.
#[derive(Debug, Clone, PartialEq)]
pub struct TruncatedVal {
    pub prefix: Value,
    pub total_len: Option<u64>,
    pub unit: LenUnit,
}

/// A borrowed [`Value`], as `Chunk::cell` returns it without allocating.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ValueRef<'a> {
    Null,
    Missing,
    Default,
    Bool(bool),
    Int(i64),
    UInt(u64),
    F32(f32),
    F64(f64),
    /// By value (M3a Task 6, controller ruling on Task 1b's `&'a DecimalVal`), so a `Dec64`
    /// buffer keeps bare `i64` mantissas.
    Decimal(DecimalRef<'a>),
    Text(&'a str),
    Bytes(&'a [u8]),
    Uuid([u8; 16]),
    Date(DateVal),
    Time(TimeVal),
    TimeTz(TimeVal, i32),
    Timestamp(TsVal),
    TimestampTz {
        utc: TsVal,
        offset: Option<i32>,
    },
    Interval {
        months: i32,
        days: i32,
        micros: i64,
    },
    Json(&'a str),
    Xml(&'a str),
    Special(&'a SpecialVal),
    Opaque(&'a OpaqueVal),
    Lossy(&'a [u8]),
    Truncated(&'a TruncatedVal),
    Error(&'a ErrorVal),
}

impl Value {
    /// This value, borrowed.
    pub fn view(&self) -> ValueRef<'_> {
        match self {
            Value::Null => ValueRef::Null,
            Value::Missing => ValueRef::Missing,
            Value::Default => ValueRef::Default,
            Value::Bool(v) => ValueRef::Bool(*v),
            Value::Int(v) => ValueRef::Int(*v),
            Value::UInt(v) => ValueRef::UInt(*v),
            Value::F32(v) => ValueRef::F32(*v),
            Value::F64(v) => ValueRef::F64(*v),
            Value::Decimal(v) => ValueRef::Decimal(v.view()),
            Value::Text(v) => ValueRef::Text(v),
            Value::Bytes(v) => ValueRef::Bytes(v),
            Value::Uuid(v) => ValueRef::Uuid(*v),
            Value::Date(v) => ValueRef::Date(*v),
            Value::Time(v) => ValueRef::Time(*v),
            Value::TimeTz(t, o) => ValueRef::TimeTz(*t, *o),
            Value::Timestamp(v) => ValueRef::Timestamp(*v),
            Value::TimestampTz { utc, offset } => ValueRef::TimestampTz {
                utc: *utc,
                offset: *offset,
            },
            Value::Interval {
                months,
                days,
                micros,
            } => ValueRef::Interval {
                months: *months,
                days: *days,
                micros: *micros,
            },
            Value::Json(v) => ValueRef::Json(v),
            Value::Xml(v) => ValueRef::Xml(v),
            Value::Special(v) => ValueRef::Special(v),
            Value::Opaque(v) => ValueRef::Opaque(v),
            Value::Lossy(v) => ValueRef::Lossy(v),
            Value::Truncated(v) => ValueRef::Truncated(v),
            Value::Error(v) => ValueRef::Error(v),
        }
    }
}

impl ValueRef<'_> {
    /// An owned copy (allocates for text, bytes and the boxed variants).
    pub fn to_value(&self) -> Value {
        match *self {
            ValueRef::Null => Value::Null,
            ValueRef::Missing => Value::Missing,
            ValueRef::Default => Value::Default,
            ValueRef::Bool(v) => Value::Bool(v),
            ValueRef::Int(v) => Value::Int(v),
            ValueRef::UInt(v) => Value::UInt(v),
            ValueRef::F32(v) => Value::F32(v),
            ValueRef::F64(v) => Value::F64(v),
            ValueRef::Decimal(v) => Value::Decimal(v.to_decimal()),
            ValueRef::Text(v) => Value::Text(v.into()),
            ValueRef::Bytes(v) => Value::Bytes(v.into()),
            ValueRef::Uuid(v) => Value::Uuid(v),
            ValueRef::Date(v) => Value::Date(v),
            ValueRef::Time(v) => Value::Time(v),
            ValueRef::TimeTz(t, o) => Value::TimeTz(t, o),
            ValueRef::Timestamp(v) => Value::Timestamp(v),
            ValueRef::TimestampTz { utc, offset } => Value::TimestampTz { utc, offset },
            ValueRef::Interval {
                months,
                days,
                micros,
            } => Value::Interval {
                months,
                days,
                micros,
            },
            ValueRef::Json(v) => Value::Json(v.into()),
            ValueRef::Xml(v) => Value::Xml(v.into()),
            ValueRef::Special(v) => Value::Special(v.clone()),
            ValueRef::Opaque(v) => Value::Opaque(Arc::new(v.clone())),
            ValueRef::Lossy(v) => Value::Lossy(v.into()),
            ValueRef::Truncated(v) => Value::Truncated(Box::new(v.clone())),
            ValueRef::Error(v) => Value::Error(Arc::new(v.clone())),
        }
    }
}
