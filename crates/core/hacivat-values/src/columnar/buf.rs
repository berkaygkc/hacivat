//! Column buffers (Task 6): one per [`BufKind`].
//!
//! A typed buffer keeps a validity bitmap (a clear bit is a NULL or an exception), its values
//! in one dense vector, and its exceptions (whole [`Value`]s the buffer cannot hold exactly)
//! in a sparse map sorted by row. A `Values` buffer holds whole values (Task 1b's
//! `Chunk::from_values`, fixtures, rare types).
//!
//! `bytes` counts what the buffers hold: their elements, the bitmap words, the `Var` offsets
//! and data, and each boxed value's text or byte payload (allocator headers are not counted).

use std::sync::Arc;

use crate::columnar::bitmap::Bitmap;
use crate::columnar::{BufKind, CellFlags, TempUnit, VarKind};
use crate::decimal::{DecimalRef, DecimalVal, Repr};
use crate::logical::LogicalType;
use crate::special::SpecialVal;
use crate::temporal::{DateVal, MICROS_PER_DAY, TimeVal, TsVal};
use crate::value::{ErrorVal, LenUnit, TruncatedVal, Value, ValueRef};

/// One column of a chunk.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ColumnBuf {
    /// Whole values (`BufKind::Values`).
    Values(ValuesBuf),
    /// Every other [`BufKind`].
    Typed(Typed),
}

/// Whole values and the sum of their payloads.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct ValuesBuf {
    values: Vec<Value>,
    payload: u64,
}

/// A typed buffer: validity, exceptions, values.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Typed {
    valid: Bitmap,
    exc: Exceptions,
    data: Data,
}

/// The exceptions of one column: rows (ascending) and their values.
#[derive(Debug, Clone, PartialEq, Default)]
struct Exceptions {
    rows: Vec<u32>,
    values: Vec<Value>,
    payload: u64,
}

/// What the `i64` of a `Temporal { unit: Micros }` column is (from its logical type).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MicrosKind {
    /// Microseconds of the day (`Time`).
    Time,
    /// Microseconds since 1970-01-01, naive (`Timestamp`).
    Timestamp,
    /// Microseconds since 1970-01-01 UTC, shown in the display zone (`TimestampTz`, no offset).
    TimestampTz,
}

/// What the `(i64, i32)` of a `Temporal2` column is (from its logical type).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PairKind {
    /// Microseconds of the day and the offset east (`TimeTz`).
    TimeTz,
    /// Microseconds since 1970-01-01 UTC and the offset east the server sent.
    TimestampTz,
}

/// What a text `Var` column holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StrKind {
    Text,
    Json,
    /// A `Var(Text)` column whose logical type is `Xml`.
    Xml,
}

/// The dense values of a typed buffer; a NULL or exception row holds a placeholder.
#[derive(Debug, Clone, PartialEq)]
enum Data {
    Bool(Bitmap),
    I64(Vec<i64>),
    U64(Vec<u64>),
    F64(Vec<f64>),
    /// `F64 { f32: true }`: kept as `f32`, so its shortest form is the server's.
    F32(Vec<f32>),
    /// `Dec64`: `i64` mantissas at the column's scale. A decimal of another scale, beyond an
    /// `i64`, or `-0` is an exception.
    Dec64 {
        values: Vec<i64>,
        scale: i16,
    },
    /// `Dec128`: up to 38 digits, the words of `Repr::Small` and each value's scale; a `Big`
    /// value is an exception.
    Dec128 {
        mants: Vec<[u64; 2]>,
        scales: Vec<i16>,
    },
    /// `Temporal { unit: Days }`: days since 1970-01-01.
    Days(Vec<i32>),
    Micros {
        values: Vec<i64>,
        kind: MicrosKind,
    },
    Pair {
        micros: Vec<i64>,
        offsets: Vec<i32>,
        kind: PairKind,
    },
    /// Text, JSON or XML: `offs` has one more entry than rows; the text is valid UTF-8.
    Str {
        offs: Vec<u32>,
        data: String,
        kind: StrKind,
    },
    Bin {
        offs: Vec<u32>,
        data: Vec<u8>,
    },
    Fixed16(Vec<[u8; 16]>),
}

/// One typed push (each `ChunkBuilder::push_*` but `push_null` and `push_value`).
#[derive(Debug, Clone, Copy)]
pub(crate) enum Cell<'a> {
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    Dec64(i64),
    Decimal(&'a DecimalVal),
    Temporal(i64),
    Temporal2(i64, i32),
    /// The bytes and the cap.
    Bytes(&'a [u8], u32),
    Fixed16([u8; 16]),
}

/// How a push went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Push {
    Done,
    /// Done, but a `Var` data buffer is at its limit: the cell went to the exception map and
    /// the chunk should be sealed after this row.
    Full,
    /// The push is not this column's kind (a driver bug; the builder panics).
    Mismatch,
}

/// What a typed or exact push does with its cell.
enum Outcome {
    /// Stored in the dense buffer (valid).
    Inline,
    /// Kept whole in the exception map.
    Exception(Value),
    /// Kept whole in the exception map because the `Var` data is at its limit.
    Full(Value),
    Mismatch,
}

impl ColumnBuf {
    /// An empty buffer of `kind` with room for `rows`; `logical` tells a `Temporal` or
    /// `Var(Text)` column what its values are (`None`: naive timestamps, timestamps with an
    /// offset, text).
    pub(crate) fn new(kind: BufKind, logical: Option<&LogicalType>, rows: usize) -> Self {
        let offs = || {
            let mut offs = Vec::with_capacity(rows + 1);
            offs.push(0);
            offs
        };
        let data = match kind {
            BufKind::Values => {
                return ColumnBuf::Values(ValuesBuf {
                    values: Vec::with_capacity(rows),
                    payload: 0,
                });
            }
            BufKind::Bool => Data::Bool(Bitmap::with_capacity(rows)),
            BufKind::I64 => Data::I64(Vec::with_capacity(rows)),
            BufKind::U64 => Data::U64(Vec::with_capacity(rows)),
            BufKind::F64 { f32: false } => Data::F64(Vec::with_capacity(rows)),
            BufKind::F64 { f32: true } => Data::F32(Vec::with_capacity(rows)),
            BufKind::Dec64 { scale } => Data::Dec64 {
                values: Vec::with_capacity(rows),
                scale,
            },
            BufKind::Dec128 => Data::Dec128 {
                mants: Vec::with_capacity(rows),
                scales: Vec::with_capacity(rows),
            },
            BufKind::Temporal {
                unit: TempUnit::Days,
            } => Data::Days(Vec::with_capacity(rows)),
            BufKind::Temporal {
                unit: TempUnit::Micros,
            } => Data::Micros {
                values: Vec::with_capacity(rows),
                kind: match logical {
                    Some(LogicalType::Time { .. }) => MicrosKind::Time,
                    Some(LogicalType::Timestamp { tz: true, .. }) => MicrosKind::TimestampTz,
                    _ => MicrosKind::Timestamp,
                },
            },
            BufKind::Temporal2 => Data::Pair {
                micros: Vec::with_capacity(rows),
                offsets: Vec::with_capacity(rows),
                kind: match logical {
                    Some(LogicalType::Time { .. }) => PairKind::TimeTz,
                    _ => PairKind::TimestampTz,
                },
            },
            BufKind::Var(VarKind::Text) => Data::Str {
                offs: offs(),
                data: String::new(),
                kind: match logical {
                    Some(LogicalType::Xml) => StrKind::Xml,
                    _ => StrKind::Text,
                },
            },
            BufKind::Var(VarKind::Json) => Data::Str {
                offs: offs(),
                data: String::new(),
                kind: StrKind::Json,
            },
            BufKind::Var(VarKind::Bytes) => Data::Bin {
                offs: offs(),
                data: Vec::new(),
            },
            BufKind::Fixed16 => Data::Fixed16(Vec::with_capacity(rows)),
        };
        ColumnBuf::Typed(Typed {
            valid: Bitmap::with_capacity(rows),
            exc: Exceptions::default(),
            data,
        })
    }

    /// A `Values` buffer of these values (`Chunk::from_values`).
    pub(crate) fn from_values(values: Vec<Value>) -> Self {
        let payload = values.iter().map(|v| payload(v) as u64).sum();
        ColumnBuf::Values(ValuesBuf { values, payload })
    }

    /// The cells pushed so far.
    pub(crate) fn len(&self) -> usize {
        match self {
            ColumnBuf::Values(b) => b.values.len(),
            ColumnBuf::Typed(t) => t.valid.len(),
        }
    }

    #[inline]
    pub(crate) fn cell(&self, row: usize) -> ValueRef<'_> {
        match self {
            ColumnBuf::Values(b) => b.values[row].view(),
            ColumnBuf::Typed(t) => t.cell(row),
        }
    }

    pub(crate) fn flags(&self, row: usize) -> CellFlags {
        match self {
            ColumnBuf::Values(b) => flags_of(&b.values[row]),
            ColumnBuf::Typed(t) if t.valid.get(row) => CellFlags::empty(),
            ColumnBuf::Typed(t) => t.exc.get(row).map_or(CellFlags::empty(), flags_of),
        }
    }

    /// The bytes the buffer holds (see the module docs).
    pub(crate) fn bytes(&self) -> u64 {
        match self {
            ColumnBuf::Values(b) => b.values.len() as u64 * size_of::<Value>() as u64 + b.payload,
            ColumnBuf::Typed(t) => t.valid.bytes() + t.exc.bytes() + t.data.bytes(),
        }
    }

    pub(crate) fn push_null(&mut self) {
        match self {
            ColumnBuf::Values(b) => b.values.push(Value::Null),
            ColumnBuf::Typed(t) => {
                t.data.push_placeholder();
                t.valid.push(false);
            }
        }
    }

    /// A typed push; `var_limit` caps a `Var` buffer's data (`u32::MAX`, so offsets fit).
    pub(crate) fn push(&mut self, cell: Cell<'_>, var_limit: u64) -> Push {
        match self {
            ColumnBuf::Values(_) => Push::Mismatch,
            ColumnBuf::Typed(t) => {
                let outcome = t.data.push(cell, var_limit);
                t.settle(outcome)
            }
        }
    }

    /// A whole value: inline when the buffer holds exactly that value, else an exception.
    pub(crate) fn push_value(&mut self, value: Value, var_limit: u64) -> Push {
        match self {
            ColumnBuf::Values(b) => {
                b.payload += payload(&value) as u64;
                b.values.push(value);
                Push::Done
            }
            ColumnBuf::Typed(_) if matches!(value, Value::Null) => {
                self.push_null();
                Push::Done
            }
            ColumnBuf::Typed(t) => {
                let outcome = t.data.push_exact(value, var_limit);
                t.settle(outcome)
            }
        }
    }

    pub(crate) fn shrink_to_fit(&mut self) {
        match self {
            ColumnBuf::Values(b) => b.values.shrink_to_fit(),
            ColumnBuf::Typed(t) => {
                t.valid.shrink_to_fit();
                t.exc.rows.shrink_to_fit();
                t.exc.values.shrink_to_fit();
                t.data.shrink_to_fit();
            }
        }
    }
}

impl Typed {
    #[inline]
    fn cell(&self, row: usize) -> ValueRef<'_> {
        if !self.valid.get(row) {
            return self.exc.get(row).map_or(ValueRef::Null, Value::view);
        }
        match &self.data {
            Data::Bool(bits) => ValueRef::Bool(bits.get(row)),
            Data::I64(v) => ValueRef::Int(v[row]),
            Data::U64(v) => ValueRef::UInt(v[row]),
            Data::F64(v) => ValueRef::F64(v[row]),
            Data::F32(v) => ValueRef::F32(v[row]),
            Data::Dec64 { values, scale } => {
                ValueRef::Decimal(DecimalRef::from_mantissa(values[row], *scale))
            }
            Data::Dec128 { mants, scales } => {
                ValueRef::Decimal(DecimalRef::from_small(mants[row], scales[row]))
            }
            Data::Days(v) => ValueRef::Date(DateVal(v[row])),
            Data::Micros { values, kind } => {
                let us = values[row];
                match kind {
                    MicrosKind::Time => ValueRef::Time(time_of(us)),
                    MicrosKind::Timestamp => ValueRef::Timestamp(TsVal::from_unix_micros(us)),
                    MicrosKind::TimestampTz => ValueRef::TimestampTz {
                        utc: TsVal::from_unix_micros(us),
                        offset: None,
                    },
                }
            }
            Data::Pair {
                micros,
                offsets,
                kind,
            } => match kind {
                PairKind::TimeTz => ValueRef::TimeTz(time_of(micros[row]), offsets[row]),
                PairKind::TimestampTz => ValueRef::TimestampTz {
                    utc: TsVal::from_unix_micros(micros[row]),
                    offset: Some(offsets[row]),
                },
            },
            Data::Str { offs, data, kind } => {
                let s = &data[offs[row] as usize..offs[row + 1] as usize];
                match kind {
                    StrKind::Text => ValueRef::Text(s),
                    StrKind::Json => ValueRef::Json(s),
                    StrKind::Xml => ValueRef::Xml(s),
                }
            }
            Data::Bin { offs, data } => {
                ValueRef::Bytes(&data[offs[row] as usize..offs[row + 1] as usize])
            }
            Data::Fixed16(v) => ValueRef::Uuid(v[row]),
        }
    }

    /// Records the validity (and the exception) of the row a push just decided.
    fn settle(&mut self, outcome: Outcome) -> Push {
        let row = self.valid.len() as u32;
        let (value, push) = match outcome {
            Outcome::Inline => {
                self.valid.push(true);
                return Push::Done;
            }
            Outcome::Mismatch => return Push::Mismatch,
            Outcome::Exception(value) => (value, Push::Done),
            Outcome::Full(value) => (value, Push::Full),
        };
        self.data.push_placeholder();
        self.valid.push(false);
        self.exc.push(row, value);
        push
    }
}

impl Exceptions {
    fn get(&self, row: usize) -> Option<&Value> {
        let row = u32::try_from(row).ok()?;
        self.rows.binary_search(&row).ok().map(|i| &self.values[i])
    }

    fn push(&mut self, row: u32, value: Value) {
        self.payload += payload(&value) as u64;
        self.rows.push(row);
        self.values.push(value);
    }

    fn bytes(&self) -> u64 {
        (self.rows.len() * size_of::<u32>() + self.values.len() * size_of::<Value>()) as u64
            + self.payload
    }
}

impl Data {
    /// The placeholder of a NULL or exception row.
    fn push_placeholder(&mut self) {
        match self {
            Data::Bool(bits) => bits.push(false),
            Data::I64(v) | Data::Micros { values: v, .. } => v.push(0),
            Data::U64(v) => v.push(0),
            Data::F64(v) => v.push(0.0),
            Data::F32(v) => v.push(0.0),
            Data::Dec64 { values, .. } => values.push(0),
            Data::Dec128 { mants, scales } => {
                mants.push([0; 2]);
                scales.push(0);
            }
            Data::Days(v) => v.push(0),
            Data::Pair {
                micros, offsets, ..
            } => {
                micros.push(0);
                offsets.push(0);
            }
            Data::Str { offs, data, .. } => offs.push(data.len() as u32),
            Data::Bin { offs, data } => offs.push(data.len() as u32),
            Data::Fixed16(v) => v.push([0; 16]),
        }
    }

    fn push(&mut self, cell: Cell<'_>, var_limit: u64) -> Outcome {
        match (self, cell) {
            (Data::Bool(bits), Cell::Bool(b)) => bits.push(b),
            (Data::I64(v), Cell::I64(x)) => v.push(x),
            (Data::U64(v), Cell::U64(x)) => v.push(x),
            (Data::F64(v), Cell::F64(x)) => v.push(x),
            (Data::F32(v), Cell::F64(x)) => {
                let narrow = x as f32;
                // Kept as f32 only when that is exact (NaN is NaN); never rounded.
                if f64::from(narrow) != x && !x.is_nan() {
                    return Outcome::Exception(Value::F64(x));
                }
                v.push(narrow);
            }
            (data @ (Data::Dec64 { .. } | Data::Dec128 { .. }), Cell::Decimal(d)) => {
                if !data.push_decimal(d.view()) {
                    return Outcome::Exception(Value::Decimal(d.clone()));
                }
            }
            (Data::Dec64 { values, .. }, Cell::Dec64(mant)) => values.push(mant),
            (Data::Days(v), Cell::Temporal(days)) => match i32::try_from(days) {
                Ok(days) => v.push(days),
                Err(_) => return Outcome::Exception(range_error("date")),
            },
            (Data::Micros { values, kind }, Cell::Temporal(us)) => {
                if *kind == MicrosKind::Time && !(0..=MICROS_PER_DAY).contains(&us) {
                    return Outcome::Exception(range_error("time of day"));
                }
                values.push(us);
            }
            (
                Data::Pair {
                    micros,
                    offsets,
                    kind,
                },
                Cell::Temporal2(us, offset),
            ) => {
                if *kind == PairKind::TimeTz && !(0..=MICROS_PER_DAY).contains(&us) {
                    return Outcome::Exception(range_error("time of day"));
                }
                micros.push(us);
                offsets.push(offset);
            }
            (Data::Str { offs, data, kind }, Cell::Bytes(bytes, cap)) => {
                return push_text(offs, data, *kind, bytes, cap as usize, var_limit);
            }
            (Data::Bin { offs, data }, Cell::Bytes(bytes, cap)) => {
                let cap = cap as usize;
                if bytes.len() > cap {
                    return Outcome::Exception(truncated(
                        Value::Bytes(bytes[..cap].into()),
                        bytes.len(),
                    ));
                }
                if !fits(data.len(), bytes.len(), var_limit) {
                    return Outcome::Full(Value::Bytes(bytes.into()));
                }
                data.extend_from_slice(bytes);
                offs.push(data.len() as u32);
            }
            (Data::Fixed16(v), Cell::Fixed16(x)) => v.push(x),
            _ => return Outcome::Mismatch,
        }
        Outcome::Inline
    }

    /// Stores `value` inline when the buffer holds exactly it (the same `ValueRef` comes
    /// back), else hands it back as an exception. Never rounds, never converts.
    fn push_exact(&mut self, value: Value, var_limit: u64) -> Outcome {
        match (self, &value) {
            (Data::Bool(bits), Value::Bool(b)) => bits.push(*b),
            (Data::I64(v), Value::Int(x)) => v.push(*x),
            (Data::U64(v), Value::UInt(x)) => v.push(*x),
            (Data::F64(v), Value::F64(x)) => v.push(*x),
            (Data::F32(v), Value::F32(x)) => v.push(*x),
            (data @ (Data::Dec64 { .. } | Data::Dec128 { .. }), Value::Decimal(d)) => {
                if !data.push_decimal(d.view()) {
                    return Outcome::Exception(value);
                }
            }
            (Data::Days(v), Value::Date(d)) => v.push(d.0),
            (
                Data::Micros {
                    values,
                    kind: MicrosKind::Time,
                },
                Value::Time(t),
            ) => match t.exact_micros() {
                Some(us) => values.push(us),
                None => return Outcome::Exception(value),
            },
            (
                Data::Micros {
                    values,
                    kind: MicrosKind::Timestamp,
                },
                Value::Timestamp(ts),
            )
            | (
                Data::Micros {
                    values,
                    kind: MicrosKind::TimestampTz,
                },
                Value::TimestampTz {
                    utc: ts,
                    offset: None,
                },
            ) => match ts.to_unix_micros() {
                Some(us) => values.push(us),
                None => return Outcome::Exception(value),
            },
            (
                Data::Pair {
                    micros,
                    offsets,
                    kind: PairKind::TimeTz,
                },
                Value::TimeTz(t, offset),
            ) => match t.exact_micros() {
                Some(us) => {
                    micros.push(us);
                    offsets.push(*offset);
                }
                None => return Outcome::Exception(value),
            },
            (
                Data::Pair {
                    micros,
                    offsets,
                    kind: PairKind::TimestampTz,
                },
                Value::TimestampTz {
                    utc,
                    offset: Some(offset),
                },
            ) => match utc.to_unix_micros() {
                Some(us) => {
                    micros.push(us);
                    offsets.push(*offset);
                }
                None => return Outcome::Exception(value),
            },
            (
                Data::Str {
                    offs,
                    data,
                    kind: StrKind::Text,
                },
                Value::Text(s),
            )
            | (
                Data::Str {
                    offs,
                    data,
                    kind: StrKind::Json,
                },
                Value::Json(s),
            )
            | (
                Data::Str {
                    offs,
                    data,
                    kind: StrKind::Xml,
                },
                Value::Xml(s),
            ) => {
                if !fits(data.len(), s.len(), var_limit) {
                    return Outcome::Full(value);
                }
                data.push_str(s);
                offs.push(data.len() as u32);
            }
            (Data::Bin { offs, data }, Value::Bytes(b)) => {
                if !fits(data.len(), b.len(), var_limit) {
                    return Outcome::Full(value);
                }
                data.extend_from_slice(b);
                offs.push(data.len() as u32);
            }
            (Data::Fixed16(v), Value::Uuid(x)) => v.push(*x),
            _ => return Outcome::Exception(value),
        }
        Outcome::Inline
    }

    /// Stores a decimal inline when the buffer holds it exactly: a `Dec64` one at the
    /// column's scale whose mantissa is an `i64` (not `-0`), any `Dec128` one of up to 38
    /// digits. `false`: the caller keeps it as an exception.
    fn push_decimal(&mut self, d: DecimalRef<'_>) -> bool {
        match self {
            Data::Dec64 { values, scale } => match d.mantissa_i64() {
                Some((mant, s)) if s == *scale => {
                    values.push(mant);
                    true
                }
                _ => false,
            },
            Data::Dec128 { mants, scales } => match d.small() {
                Some((mant, s)) => {
                    mants.push(mant);
                    scales.push(s);
                    true
                }
                None => false,
            },
            _ => false,
        }
    }

    fn bytes(&self) -> u64 {
        fn elems<T>(v: &[T]) -> u64 {
            size_of_val(v) as u64
        }
        match self {
            Data::Bool(bits) => bits.bytes(),
            Data::I64(v) | Data::Micros { values: v, .. } => elems(v),
            Data::U64(v) => elems(v),
            Data::F64(v) => elems(v),
            Data::F32(v) => elems(v),
            Data::Dec64 { values, .. } => elems(values),
            Data::Dec128 { mants, scales } => elems(mants) + elems(scales),
            Data::Days(v) => elems(v),
            Data::Pair {
                micros, offsets, ..
            } => elems(micros) + elems(offsets),
            Data::Str { offs, data, .. } => elems(offs) + data.len() as u64,
            Data::Bin { offs, data } => elems(offs) + data.len() as u64,
            Data::Fixed16(v) => elems(v),
        }
    }

    fn shrink_to_fit(&mut self) {
        match self {
            Data::Bool(bits) => bits.shrink_to_fit(),
            Data::I64(v) | Data::Micros { values: v, .. } => v.shrink_to_fit(),
            Data::U64(v) => v.shrink_to_fit(),
            Data::F64(v) => v.shrink_to_fit(),
            Data::F32(v) => v.shrink_to_fit(),
            Data::Dec64 { values, .. } => values.shrink_to_fit(),
            Data::Dec128 { mants, scales } => {
                mants.shrink_to_fit();
                scales.shrink_to_fit();
            }
            Data::Days(v) => v.shrink_to_fit(),
            Data::Pair {
                micros, offsets, ..
            } => {
                micros.shrink_to_fit();
                offsets.shrink_to_fit();
            }
            Data::Str { offs, data, .. } => {
                offs.shrink_to_fit();
                data.shrink_to_fit();
            }
            Data::Bin { offs, data } => {
                offs.shrink_to_fit();
                data.shrink_to_fit();
            }
            Data::Fixed16(v) => v.shrink_to_fit(),
        }
    }
}

/// A text, JSON or XML cell: valid UTF-8 inline (cut at a character boundary past `cap`),
/// anything else `Lossy` (its bytes kept, never replaced).
fn push_text(
    offs: &mut Vec<u32>,
    data: &mut String,
    kind: StrKind,
    bytes: &[u8],
    cap: usize,
    var_limit: u64,
) -> Outcome {
    let whole = |s: &str| -> Value {
        match kind {
            StrKind::Text => Value::Text(s.into()),
            StrKind::Json => Value::Json(s.into()),
            StrKind::Xml => Value::Xml(s.into()),
        }
    };
    match std::str::from_utf8(bytes) {
        Ok(s) if s.len() > cap => {
            Outcome::Exception(truncated(whole(&s[..floor_char_boundary(s, cap)]), s.len()))
        }
        Ok(s) if !fits(data.len(), s.len(), var_limit) => Outcome::Full(whole(s)),
        Ok(s) => {
            data.push_str(s);
            offs.push(data.len() as u32);
            Outcome::Inline
        }
        Err(_) if bytes.len() > cap => Outcome::Exception(truncated(
            Value::Lossy(bytes[..floor_utf8_lead(bytes, cap)].into()),
            bytes.len(),
        )),
        Err(_) => Outcome::Exception(Value::Lossy(bytes.into())),
    }
}

/// Whether `add` more bytes keep a `Var` buffer of `len` bytes within `limit`.
fn fits(len: usize, add: usize, limit: u64) -> bool {
    len as u64 + add as u64 <= limit
}

/// The largest character boundary of `s` at or below `cap`.
fn floor_char_boundary(s: &str, cap: usize) -> usize {
    let mut i = cap.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// A cut of bytes that are not valid UTF-8 at or below `cap`, moved back (at most three
/// bytes) so it does not split a sequence that starts before it.
fn floor_utf8_lead(bytes: &[u8], cap: usize) -> usize {
    let mut i = cap.min(bytes.len());
    let floor = i.saturating_sub(3);
    while i > floor && i < bytes.len() && bytes[i] & 0xC0 == 0x80 {
        i -= 1;
    }
    i
}

fn truncated(prefix: Value, total: usize) -> Value {
    Value::Truncated(Box::new(TruncatedVal {
        prefix,
        total_len: Some(total as u64),
        unit: LenUnit::Bytes,
    }))
}

/// A cell whose value lies outside what its buffer can represent.
fn range_error(what: &str) -> Value {
    Value::Error(Arc::new(ErrorVal {
        code: "out-of-range".into(),
        message: format!("the {what} is outside the supported range").into(),
    }))
}

fn time_of(micros: i64) -> TimeVal {
    TimeVal {
        nanos_of_day: micros as u64 * 1_000,
    }
}

/// The flags a whole value implies (a truncated value also carries its prefix's).
pub(crate) fn flags_of(v: &Value) -> CellFlags {
    match v {
        Value::Truncated(t) => CellFlags::TRUNCATED | flags_of(&t.prefix),
        Value::Lossy(_) => CellFlags::LOSSY,
        Value::Error(_) => CellFlags::ERROR,
        Value::Special(_) => CellFlags::SPECIAL,
        _ => CellFlags::empty(),
    }
}

/// The heap bytes a value points at: its text or bytes, a big decimal's text, a truncated
/// value's box and prefix.
fn payload(v: &Value) -> usize {
    match v {
        Value::Text(s) | Value::Json(s) | Value::Xml(s) => s.len(),
        Value::Bytes(b) | Value::Lossy(b) => b.len(),
        Value::Decimal(d) => match d.repr() {
            Repr::Big(text) => text.len(),
            Repr::Small { .. } => 0,
        },
        Value::Special(SpecialVal::PartialZeroDate(s)) => s.len(),
        Value::Opaque(o) => o.type_name.len() + o.text.len(),
        Value::Error(e) => e.code.len() + e.message.len(),
        Value::Truncated(t) => size_of::<TruncatedVal>() + payload(&t.prefix),
        _ => 0,
    }
}
