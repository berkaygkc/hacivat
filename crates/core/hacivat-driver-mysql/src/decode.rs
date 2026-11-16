//! MySQL and MariaDB values to `hacivat_values` (R-M3-31; Task 14): per column type and flags
//! (`UNSIGNED`, `ENUM`, `SET`) and character set (63 is `binary`), in both protocols. Public
//! (hidden) so `tests/decode.rs` reaches the decoders.
//!
//! Each result column gets a [`Col`] from its definition, which decides its logical type, its
//! chunk buffer and how a cell is decoded:
//!
//! - **Integers** (`TINYINT` … `BIGINT`, ± `UNSIGNED`; `TINYINT(1)` stays an integer, D-065;
//!   `YEAR` an unsigned 16-bit integer): `I64`, except `BIGINT UNSIGNED` (`U64`).
//! - **`DECIMAL`** from its text (the binary protocol sends text too) through
//!   `DecimalVal::parse` and `push_decimal`, so the server's scale and a `-0.00` stay exact:
//!   `Dec64 { scale }` up to 18 digits of precision, else `Dec128`; more than 38 digits are exact
//!   `Big` exceptions. Precision is read back from the column's display length.
//! - **`FLOAT`/`DOUBLE`**: `F64 { f32 }`; text is parsed at the column's width.
//! - **Text** (`CHAR`, `VARCHAR`, `TEXT`, `ENUM`, `SET`; any character set but 63): `Var(Text)`;
//!   bytes that are not UTF-8 are `Lossy`, never replaced. **Bytes** (`BINARY`, `VARBINARY`,
//!   `BLOB`: character set 63): `Var(Bytes)`. **`JSON`**: `Var(Json)`, the server's text.
//! - **`DATE`**: `Temporal { Days }`; **`DATETIME(n)`** and **`TIMESTAMP(n)`**:
//!   `Temporal { Micros }`, naive (a `TIMESTAMP` is the server's text in the session
//!   `time_zone`; its native column type says which it is); **`TIME(n)`**: a time of day when it
//!   lies within `0 ..= 24:00:00`, else (negative, or a duration past a day: MySQL's `TIME` runs
//!   to ±838:59:59) an exact `Interval` exception.
//! - **Zero dates** (`0000-00-00`, and a zero `DATETIME`) are `Special(ZeroDate)`; a date with a
//!   zero month or day, or a zero date with a time, is `Special(PartialZeroDate)` with the text
//!   the text protocol sends (rebuilt the same way from the binary protocol's parts). Never NULL.
//!   A date that is not in the calendar but that the server stores (`ALLOW_INVALID_DATES`:
//!   `2024-02-30`) is `Opaque` with that text.
//! - **`BIT(n)`** (its `n` binary digits), **`GEOMETRY`** (the internal value, SRID then WKB, as
//!   `HEX()` prints it), **`VECTOR`** and every other type: `Value::Opaque` text in a `Values`
//!   buffer, the type name shared by the column (Task 12's choice).
//!
//! **Server bytes never panic and are never altered.** A cell whose bytes no MySQL or MariaDB
//! server sends (digits that do not parse, a month of 13, an hour of 24, a fraction of seven
//! digits, a `BIT` value longer than 8 bytes, a value of the wrong kind for its column) is a
//! `Value::Error` cell with a `mysql-*` code, and the row is kept. Text and bytes are capped by
//! the chunk builder (opaque text by [`ChunkBuilder::capped_value`]).
//!
//! The binary protocol's rows are parsed by `mysql_common`, which panics on a column type it
//! cannot read in that protocol; [`binary_decodable`] is the check the typed path runs on every
//! binary result set's columns before it reads a row.

use std::fmt::Write as _;
use std::sync::Arc;

use hacivat_values::{
    BufKind, ChunkBuilder, ColumnMeta, DateVal, DecimalVal, ErrorVal, LenUnit, LogicalType,
    NativeType, OpaqueVal, OriginHint, ResultSchema, SpecialVal, TempUnit, TruncatedVal, Value,
    VarKind,
};
use mysql_async::consts::{ColumnFlags, ColumnType};
use mysql_async::{Column, Value as My};

/// The `binary` character set (collation 63): bytes, never text.
pub const BINARY_CHARSET: u16 = 63;

const MICROS_PER_SEC: i64 = 1_000_000;
const MICROS_PER_DAY: i64 = 86_400 * MICROS_PER_SEC;
/// The most bytes of a `BIT(64)` value.
const MAX_BIT_BYTES: usize = 8;
/// The most bytes of a raw value an error message quotes.
const QUOTE_BYTES: usize = 64;

/// How one result column is decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Col {
    /// Every integer an `i64` holds: signed widths, unsigned widths below 64 bits, `YEAR`.
    Int {
        bits: u8,
        signed: bool,
    },
    /// `BIGINT UNSIGNED`.
    UInt64,
    Float,
    Double,
    Decimal,
    Text {
        fixed: bool,
    },
    Bytes,
    Json,
    Date,
    /// `DATETIME` and `TIMESTAMP`.
    DateTime,
    Time,
    /// `BIT(n)`: `n` binary digits.
    Bit {
        bits: u32,
        type_name: Arc<str>,
    },
    /// The internal value as upper-case hex.
    Geometry {
        type_name: Arc<str>,
    },
    /// The server's bytes as text.
    Opaque {
        type_name: Arc<str>,
    },
}

/// The name `Value::Opaque` carries for a column type M3a does not decode.
fn type_name(t: ColumnType) -> Arc<str> {
    use ColumnType::*;
    match t {
        MYSQL_TYPE_BIT => "bit".into(),
        MYSQL_TYPE_GEOMETRY => "geometry".into(),
        MYSQL_TYPE_VECTOR => "vector".into(),
        MYSQL_TYPE_TIMESTAMP2 => "timestamp2".into(),
        MYSQL_TYPE_DATETIME2 => "datetime2".into(),
        MYSQL_TYPE_TIME2 => "time2".into(),
        MYSQL_TYPE_TYPED_ARRAY => "typed_array".into(),
        other => format!("mysql type {}", other as u8).into(),
    }
}

impl Col {
    /// The decoder of column `c`.
    pub fn for_column(c: &Column) -> Col {
        use ColumnType::*;
        let flags = c.flags();
        let signed = !flags.contains(ColumnFlags::UNSIGNED_FLAG);
        let t = c.column_type();
        match t {
            MYSQL_TYPE_TINY => Col::Int { bits: 8, signed },
            MYSQL_TYPE_SHORT => Col::Int { bits: 16, signed },
            MYSQL_TYPE_INT24 => Col::Int { bits: 24, signed },
            MYSQL_TYPE_LONG => Col::Int { bits: 32, signed },
            MYSQL_TYPE_LONGLONG if signed => Col::Int { bits: 64, signed },
            MYSQL_TYPE_LONGLONG => Col::UInt64,
            MYSQL_TYPE_YEAR => Col::Int {
                bits: 16,
                signed: false,
            },
            MYSQL_TYPE_FLOAT => Col::Float,
            MYSQL_TYPE_DOUBLE => Col::Double,
            MYSQL_TYPE_DECIMAL | MYSQL_TYPE_NEWDECIMAL => Col::Decimal,
            MYSQL_TYPE_DATE | MYSQL_TYPE_NEWDATE => Col::Date,
            MYSQL_TYPE_DATETIME | MYSQL_TYPE_TIMESTAMP => Col::DateTime,
            MYSQL_TYPE_TIME => Col::Time,
            MYSQL_TYPE_JSON => Col::Json,
            MYSQL_TYPE_BIT => Col::Bit {
                bits: c.column_length(),
                type_name: type_name(t),
            },
            MYSQL_TYPE_GEOMETRY => Col::Geometry {
                type_name: type_name(t),
            },
            MYSQL_TYPE_ENUM | MYSQL_TYPE_SET | MYSQL_TYPE_NULL => Col::Text { fixed: false },
            MYSQL_TYPE_STRING
            | MYSQL_TYPE_VAR_STRING
            | MYSQL_TYPE_VARCHAR
            | MYSQL_TYPE_TINY_BLOB
            | MYSQL_TYPE_MEDIUM_BLOB
            | MYSQL_TYPE_LONG_BLOB
            | MYSQL_TYPE_BLOB => {
                if flags.intersects(ColumnFlags::ENUM_FLAG | ColumnFlags::SET_FLAG) {
                    Col::Text { fixed: false }
                } else if c.character_set() == BINARY_CHARSET {
                    Col::Bytes
                } else {
                    Col::Text {
                        fixed: t == MYSQL_TYPE_STRING,
                    }
                }
            }
            _ => Col::Opaque {
                type_name: type_name(t),
            },
        }
    }

    /// The logical type of column `c` (this column's decoder).
    pub fn logical(&self, c: &Column) -> LogicalType {
        let fraction = c.decimals().min(6);
        match self {
            Col::Int { bits, signed } => LogicalType::Int {
                bits: *bits,
                signed: *signed,
            },
            Col::UInt64 => LogicalType::Int {
                bits: 64,
                signed: false,
            },
            Col::Float => LogicalType::Float { bits: 32 },
            Col::Double => LogicalType::Float { bits: 64 },
            Col::Decimal => LogicalType::Decimal {
                precision: decimal_precision(c),
                scale: Some(i32::from(c.decimals())),
            },
            Col::Text { fixed } => LogicalType::Text {
                max_chars: None,
                fixed: *fixed,
            },
            Col::Bytes => LogicalType::Bytes {
                max: Some(c.column_length()),
            },
            Col::Json => LogicalType::Json { binary: false },
            Col::Date => LogicalType::Date,
            Col::DateTime => LogicalType::Timestamp {
                precision: fraction,
                tz: false,
            },
            Col::Time => LogicalType::Time {
                precision: fraction,
                tz: false,
            },
            Col::Bit { type_name, .. }
            | Col::Geometry { type_name }
            | Col::Opaque { type_name } => LogicalType::Opaque {
                type_name: type_name.clone(),
            },
        }
    }

    /// The chunk buffer of column `c`.
    pub fn buf_kind(&self, c: &Column) -> BufKind {
        match self {
            Col::Int { .. } => BufKind::I64,
            Col::UInt64 => BufKind::U64,
            Col::Float => BufKind::F64 { f32: true },
            Col::Double => BufKind::F64 { f32: false },
            Col::Decimal => match decimal_precision(c) {
                Some(1..=18) => BufKind::Dec64 {
                    scale: i16::from(c.decimals()),
                },
                _ => BufKind::Dec128,
            },
            Col::Text { .. } => BufKind::Var(VarKind::Text),
            Col::Bytes => BufKind::Var(VarKind::Bytes),
            Col::Json => BufKind::Var(VarKind::Json),
            Col::Date => BufKind::Temporal {
                unit: TempUnit::Days,
            },
            Col::DateTime | Col::Time => BufKind::Temporal {
                unit: TempUnit::Micros,
            },
            Col::Bit { .. } | Col::Geometry { .. } | Col::Opaque { .. } => BufKind::Values,
        }
    }

    /// Decodes one non-NULL text-protocol cell.
    pub fn decode_text<'a>(&self, raw: &'a [u8], cap: u32) -> Decoded<'a> {
        match self {
            Col::Int { .. } => parse_ascii::<i64>(raw)
                .map_or_else(|| error("mysql-int", "an integer", raw), Decoded::Int),
            Col::UInt64 => parse_ascii::<u64>(raw).map_or_else(
                || error("mysql-int", "an unsigned integer", raw),
                Decoded::UInt,
            ),
            Col::Float => parse_ascii::<f32>(raw).map_or_else(
                || error("mysql-float", "a float", raw),
                |v| Decoded::Float(f64::from(v)),
            ),
            Col::Double => parse_ascii::<f64>(raw)
                .map_or_else(|| error("mysql-float", "a double", raw), Decoded::Float),
            Col::Decimal => decode_decimal_text(raw),
            Col::Text { .. } | Col::Bytes | Col::Json => Decoded::Var(raw),
            Col::Date => decode_date_text(raw),
            Col::DateTime => decode_datetime_text(raw),
            Col::Time => decode_time_text(raw),
            Col::Bit { bits, type_name } => match bit_text(raw, *bits) {
                Some(text) => Decoded::Exception(opaque(type_name, text)),
                None => error("mysql-bit", "a BIT value longer than 8 bytes", raw),
            },
            Col::Geometry { type_name } => Decoded::Exception(hex_opaque(type_name, raw, cap)),
            Col::Opaque { type_name } => Decoded::Exception(capped_opaque(type_name, raw, cap)),
        }
    }

    /// Decodes one non-NULL binary-protocol cell. `mysql_common` has already read it by the
    /// column type: integers as `Int`/`UInt`, floats, `Date`/`Time` parts, everything else as
    /// bytes (a decimal's text included).
    pub fn decode_bin<'a>(&self, v: &'a My, fraction: u8, cap: u32) -> Decoded<'a> {
        match (self, v) {
            (
                Col::Decimal
                | Col::Text { .. }
                | Col::Bytes
                | Col::Json
                | Col::Bit { .. }
                | Col::Geometry { .. }
                | Col::Opaque { .. },
                My::Bytes(raw),
            ) => self.decode_text(raw, cap),
            (Col::Int { .. }, My::Int(i)) => Decoded::Int(*i),
            (Col::Int { .. }, My::UInt(u)) => Decoded::UInt(*u),
            // mysql_common hands an unsigned BIGINT that fits an i64 over as `Int`.
            (Col::UInt64, My::Int(i)) => match u64::try_from(*i) {
                Ok(u) => Decoded::UInt(u),
                Err(_) => Decoded::Int(*i),
            },
            (Col::UInt64, My::UInt(u)) => Decoded::UInt(*u),
            (Col::Float | Col::Double, My::Float(f)) => Decoded::Float(f64::from(*f)),
            (Col::Float | Col::Double, My::Double(d)) => Decoded::Float(*d),
            (Col::Date, My::Date(y, m, d, h, mi, s, us)) => {
                if (*h, *mi, *s, *us) != (0, 0, 0, 0) {
                    return error("mysql-date", "a DATE value with a time", &[]);
                }
                date_parts("date", i32::from(*y), *m, *d, &|| {
                    format!("{y:04}-{m:02}-{d:02}")
                })
            }
            (Col::DateTime, My::Date(y, m, d, h, mi, s, us)) => decode_datetime_parts(
                DateTimeParts {
                    year: *y,
                    month: *m,
                    day: *d,
                    hour: *h,
                    minute: *mi,
                    second: *s,
                    micros: *us,
                },
                fraction,
            ),
            (Col::Time, My::Time(neg, days, h, m, s, us)) => {
                decode_time_bin(*neg, *days, *h, *m, *s, *us)
            }
            _ => error_text(
                "mysql-value",
                format!("a {} value in a {self:?} column", value_kind(v)),
            ),
        }
    }
}

fn value_kind(v: &My) -> &'static str {
    match v {
        My::NULL => "NULL",
        My::Bytes(_) => "bytes",
        My::Int(_) => "signed integer",
        My::UInt(_) => "unsigned integer",
        My::Float(_) => "float",
        My::Double(_) => "double",
        My::Date(..) => "date",
        My::Time(..) => "time",
    }
}

/// `DECIMAL(p, s)`'s precision from the column's display length: one character for the sign
/// unless `UNSIGNED`, one for the point when `s` > 0 (`None` when that leaves no digit or more
/// than 65).
fn decimal_precision(c: &Column) -> Option<u32> {
    let point = u32::from(c.decimals() > 0);
    let sign = u32::from(!c.flags().contains(ColumnFlags::UNSIGNED_FLAG));
    c.column_length()
        .checked_sub(point + sign)
        .filter(|p| (1..=65).contains(p))
}

/// Whether `mysql_common` can read a value of type `t` in the binary protocol. It panics
/// (`unimplemented!`) on the others, which only replication streams carry: a binary result set
/// with such a column is refused before any row is read.
pub fn binary_decodable(t: ColumnType) -> bool {
    !matches!(
        t,
        ColumnType::MYSQL_TYPE_TIMESTAMP2
            | ColumnType::MYSQL_TYPE_DATETIME2
            | ColumnType::MYSQL_TYPE_TIME2
            | ColumnType::MYSQL_TYPE_TYPED_ARRAY
            | ColumnType::MYSQL_TYPE_UNKNOWN
    )
}

/// One decoded non-NULL cell, before it goes into a chunk.
#[derive(Debug, Clone, PartialEq)]
pub enum Decoded<'a> {
    Int(i64),
    UInt(u64),
    /// A float (an `f32` widened, which is exact).
    Float(f64),
    Decimal(DecimalVal),
    /// Text, JSON or bytes as the server sent them: the chunk builder checks UTF-8 (else
    /// `Lossy`) and applies the cell cap (else `Truncated`).
    Var(&'a [u8]),
    /// Days since 1970-01-01, or microseconds (of the day, or since 1970-01-01).
    Temporal(i64),
    Special(SpecialVal),
    /// A whole value kept as an exception: an interval, an opaque text, an error cell.
    Exception(Value),
}

/// ASCII digits (and a sign, a point, an exponent for floats) as `T`; `None` otherwise.
fn parse_ascii<T: std::str::FromStr>(raw: &[u8]) -> Option<T> {
    std::str::from_utf8(raw).ok()?.parse().ok()
}

/// A cell no server sends: an `Error` cell quoting (at most 64 bytes of) the raw value.
fn error(code: &'static str, what: &str, raw: &[u8]) -> Decoded<'static> {
    let quoted = String::from_utf8_lossy(&raw[..raw.len().min(QUOTE_BYTES)]);
    let message = if raw.is_empty() {
        format!("not {what}")
    } else {
        format!("not {what}: {quoted:?}")
    };
    error_text(code, message)
}

fn error_text(code: &'static str, message: String) -> Decoded<'static> {
    Decoded::Exception(Value::Error(Arc::new(ErrorVal {
        code: code.into(),
        message: message.into(),
    })))
}

fn opaque(type_name: &Arc<str>, text: String) -> Value {
    Value::Opaque(Arc::new(OpaqueVal {
        type_name: type_name.clone(),
        text: text.into(),
    }))
}

/// The server's bytes as an opaque value's text: `Text` → `Opaque`; past the cap `Truncated`
/// with a `Text` prefix, bytes that are not UTF-8 `Lossy`, as a `Var(Text)` column keeps them.
fn capped_opaque(type_name: &Arc<str>, raw: &[u8], cap: u32) -> Value {
    match ChunkBuilder::capped_value(VarKind::Text, raw, cap) {
        Value::Text(text) => Value::Opaque(Arc::new(OpaqueVal {
            type_name: type_name.clone(),
            text,
        })),
        other => other,
    }
}

/// `raw` as upper-case hex (`HEX()`'s form); past the cap, the hex of the bytes that fit, with
/// the full text's length.
fn hex_opaque(type_name: &Arc<str>, raw: &[u8], cap: u32) -> Value {
    let total = raw.len().saturating_mul(2);
    let keep = raw.len().min(cap as usize / 2);
    let mut text = String::with_capacity(keep * 2);
    for b in &raw[..keep] {
        let _ = write!(text, "{b:02X}");
    }
    if keep == raw.len() {
        return opaque(type_name, text);
    }
    Value::Truncated(Box::new(TruncatedVal {
        prefix: Value::Text(text.into()),
        total_len: Some(total as u64),
        unit: LenUnit::Bytes,
    }))
}

/// A `BIT(bits)` value's binary digits, the last `bits` of the big-endian bytes (all of them
/// when `bits` is 0 or wider than the value); `None` past 8 bytes.
fn bit_text(raw: &[u8], bits: u32) -> Option<String> {
    if raw.len() > MAX_BIT_BYTES {
        return None;
    }
    let all: String = raw.iter().map(|b| format!("{b:08b}")).collect();
    let keep = match bits as usize {
        0 => all.len(),
        n => n.min(all.len()),
    };
    Some(all[all.len() - keep..].to_owned())
}

/// A decimal's text (both protocols send `DECIMAL` as text), exactly: its scale and sign kept,
/// more than 38 digits as `Big`.
pub fn decode_decimal_text(raw: impl AsRef<[u8]>) -> Decoded<'static> {
    let raw = raw.as_ref();
    match std::str::from_utf8(raw).ok().map(DecimalVal::parse) {
        Some(Ok(d)) => Decoded::Decimal(d),
        _ => error("mysql-decimal", "a decimal", raw),
    }
}

/// `n` ASCII digits at the start of `s` as a number, and the rest.
fn digits(s: &[u8], n: usize) -> Option<(u32, &[u8])> {
    if s.len() < n || !s[..n].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let v = s[..n]
        .iter()
        .fold(0_u32, |acc, d| acc * 10 + u32::from(d - b'0'));
    Some((v, &s[n..]))
}

fn expect(s: &[u8], c: u8) -> Option<&[u8]> {
    s.strip_prefix(&[c])
}

/// A fraction of one to six digits after a `.`, as microseconds (`.5` is 500,000); none is 0.
fn fraction_micros(s: &[u8]) -> Option<u32> {
    if s.is_empty() {
        return Some(0);
    }
    let f = expect(s, b'.')?;
    if f.is_empty() || f.len() > 6 || !f.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let (v, _) = digits(f, f.len())?;
    Some(v * 10_u32.pow(6 - f.len() as u32))
}

/// `YYYY-MM-DD` (the text protocol's form) as year, month and day.
fn date_fields(s: &[u8]) -> Option<(u32, u32, u32, &[u8])> {
    let (y, s) = digits(s, 4)?;
    let (m, s) = digits(expect(s, b'-')?, 2)?;
    let (d, s) = digits(expect(s, b'-')?, 2)?;
    Some((y, m, d, s))
}

/// A date's parts: the zero date, a partial-zero date (with `text`), a calendar date, an
/// out-of-calendar date the server stores (`Opaque`), or an error for a month or day no server
/// sends.
fn date_parts(
    kind: &str,
    year: i32,
    month: u8,
    day: u8,
    text: &dyn Fn() -> String,
) -> Decoded<'static> {
    if (year, month, day) == (0, 0, 0) {
        return Decoded::Special(SpecialVal::ZeroDate);
    }
    if month == 0 || day == 0 {
        return Decoded::Special(SpecialVal::PartialZeroDate(text().into()));
    }
    if month > 12 || day > 31 {
        return error_text("mysql-date", format!("not a date: {:?}", text()));
    }
    match DateVal::from_civil(year, month, day) {
        Some(d) => Decoded::Temporal(i64::from(d.0)),
        None => Decoded::Exception(opaque(&kind.into(), text())),
    }
}

/// A text-protocol `DATE` (`YYYY-MM-DD`).
pub fn decode_date_text(raw: impl AsRef<[u8]>) -> Decoded<'static> {
    let raw = raw.as_ref();
    match date_fields(raw) {
        Some((y, m, d, [])) => {
            let text = || String::from_utf8_lossy(raw).into_owned();
            date_parts("date", y as i32, m as u8, d as u8, &text)
        }
        _ => error("mysql-date", "a date", raw),
    }
}

/// The parts of a `DATETIME` or `TIMESTAMP` value, as the binary protocol sends them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTimeParts {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub micros: u32,
}

impl DateTimeParts {
    /// The text protocol's form of this value with `fraction` digits.
    fn text(&self, fraction: u8) -> String {
        let mut s = format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        );
        if fraction > 0 {
            let digits = u32::from(fraction.min(6));
            let shown = self.micros / 10_u32.pow(6 - digits);
            let _ = write!(s, ".{shown:0width$}", width = digits as usize);
        }
        s
    }
}

/// A `DATETIME`/`TIMESTAMP` from its parts (`fraction` digits shown in a special's text).
pub fn decode_datetime_parts(p: DateTimeParts, fraction: u8) -> Decoded<'static> {
    let text = || p.text(fraction);
    if p.hour > 23 || p.minute > 59 || p.second > 59 || p.micros > 999_999 {
        return error_text("mysql-date", format!("not a date and time: {:?}", text()));
    }
    let time_is_zero = (p.hour, p.minute, p.second, p.micros) == (0, 0, 0, 0);
    if (p.year, p.month, p.day) == (0, 0, 0) && !time_is_zero {
        return Decoded::Special(SpecialVal::PartialZeroDate(text().into()));
    }
    match date_parts("datetime", i32::from(p.year), p.month, p.day, &text) {
        Decoded::Temporal(days) => {
            let secs = i64::from(p.hour) * 3_600 + i64::from(p.minute) * 60 + i64::from(p.second);
            // |days| < 2^31, so the product stays far inside an i64.
            Decoded::Temporal(days * MICROS_PER_DAY + secs * MICROS_PER_SEC + i64::from(p.micros))
        }
        other => other,
    }
}

/// A text-protocol `DATETIME`/`TIMESTAMP` (`YYYY-MM-DD HH:MM:SS[.ffffff]`); a special keeps the
/// server's text as sent.
pub fn decode_datetime_text(raw: impl AsRef<[u8]>) -> Decoded<'static> {
    let raw = raw.as_ref();
    let parsed = (|| {
        let (y, m, d, s) = date_fields(raw)?;
        let (h, s) = digits(expect(s, b' ')?, 2)?;
        let (mi, s) = digits(expect(s, b':')?, 2)?;
        let (sec, s) = digits(expect(s, b':')?, 2)?;
        let us = fraction_micros(s)?;
        Some((y, m, d, h, mi, sec, us))
    })();
    let Some((y, m, d, h, mi, sec, us)) = parsed else {
        return error("mysql-date", "a date and time", raw);
    };
    if h > 23 || mi > 59 || sec > 59 {
        return error("mysql-date", "a date and time", raw);
    }
    let text = || String::from_utf8_lossy(raw).into_owned();
    if (y, m, d) == (0, 0, 0) {
        return if (h, mi, sec, us) == (0, 0, 0, 0) {
            Decoded::Special(SpecialVal::ZeroDate)
        } else {
            Decoded::Special(SpecialVal::PartialZeroDate(text().into()))
        };
    }
    match date_parts("datetime", y as i32, m as u8, d as u8, &text) {
        Decoded::Temporal(days) => {
            let secs = i64::from(h) * 3_600 + i64::from(mi) * 60 + i64::from(sec);
            Decoded::Temporal(days * MICROS_PER_DAY + secs * MICROS_PER_SEC + i64::from(us))
        }
        other => other,
    }
}

/// A signed duration in microseconds: a time of day within `0 ..= 24:00:00`, else an exact
/// interval.
fn time_value(negative: bool, micros: i64) -> Decoded<'static> {
    if !negative && micros <= MICROS_PER_DAY {
        return Decoded::Temporal(micros);
    }
    if negative && micros == 0 {
        return Decoded::Temporal(0);
    }
    Decoded::Exception(Value::Interval {
        months: 0,
        days: 0,
        micros: if negative { -micros } else { micros },
    })
}

/// A binary-protocol `TIME`: its sign, days, hours, minutes, seconds and microseconds.
pub fn decode_time_bin(
    negative: bool,
    days: u32,
    hours: u8,
    minutes: u8,
    seconds: u8,
    micros: u32,
) -> Decoded<'static> {
    let bad = || {
        error_text(
            "mysql-time",
            format!("not a time: {days} days {hours}:{minutes}:{seconds}.{micros}"),
        )
    };
    if hours > 23 || minutes > 59 || seconds > 59 || micros > 999_999 {
        return bad();
    }
    let clock = (i64::from(hours) * 3_600 + i64::from(minutes) * 60 + i64::from(seconds))
        * MICROS_PER_SEC
        + i64::from(micros);
    match i64::from(days)
        .checked_mul(MICROS_PER_DAY)
        .and_then(|d| d.checked_add(clock))
    {
        Some(total) => time_value(negative, total),
        None => bad(),
    }
}

/// A text-protocol `TIME` (`[-]H…H:MM:SS[.ffffff]`, one to three hour digits).
pub fn decode_time_text(raw: impl AsRef<[u8]>) -> Decoded<'static> {
    let raw = raw.as_ref();
    let parsed = (|| {
        let (negative, s) = match raw.strip_prefix(b"-") {
            Some(rest) => (true, rest),
            None => (false, raw),
        };
        let hour_digits = s.iter().take_while(|c| c.is_ascii_digit()).count();
        if !(1..=3).contains(&hour_digits) {
            return None;
        }
        let (h, s) = digits(s, hour_digits)?;
        let (m, s) = digits(expect(s, b':')?, 2)?;
        let (sec, s) = digits(expect(s, b':')?, 2)?;
        let us = fraction_micros(s)?;
        (m <= 59 && sec <= 59).then_some((negative, h, m, sec, us))
    })();
    let Some((negative, h, m, sec, us)) = parsed else {
        return error("mysql-time", "a time", raw);
    };
    let total = (i64::from(h) * 3_600 + i64::from(m) * 60 + i64::from(sec)) * MICROS_PER_SEC
        + i64::from(us);
    time_value(negative, total)
}

/// Pushes one decoded cell of column `col` (`unsigned64`: the column's buffer is `U64`).
fn push_decoded(b: &mut ChunkBuilder, col: u32, unsigned64: bool, d: Decoded<'_>, cap: u32) {
    match d {
        Decoded::Int(v) if unsigned64 => match u64::try_from(v) {
            Ok(u) => b.push_u64(col, u),
            Err(_) => b.push_value(col, Value::Int(v)),
        },
        Decoded::Int(v) => b.push_i64(col, v),
        Decoded::UInt(v) if unsigned64 => b.push_u64(col, v),
        Decoded::UInt(v) => match i64::try_from(v) {
            Ok(i) => b.push_i64(col, i),
            Err(_) => b.push_value(col, Value::UInt(v)),
        },
        Decoded::Float(v) => b.push_f64(col, v),
        Decoded::Decimal(v) => b.push_decimal(col, &v),
        Decoded::Var(bytes) => b.push_bytes(col, bytes, cap),
        Decoded::Temporal(v) => b.push_temporal(col, v),
        Decoded::Special(s) => b.push_value(col, Value::Special(s)),
        Decoded::Exception(v) => b.push_value(col, v),
    }
}

/// Which protocol a result set's rows arrive in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Wire {
    /// `COM_STMT_EXECUTE`: values typed by `mysql_common`.
    Binary,
    /// `COM_QUERY`: every value as text.
    Text,
}

/// The decode plan of one result set: its schema, each column's decoder and buffer.
#[derive(Debug, Clone)]
pub struct Plan {
    pub schema: Arc<ResultSchema>,
    pub cols: Vec<Col>,
    pub kinds: Vec<BufKind>,
    /// Each column's fractional digits (`DATETIME(n)`, `TIME(n)`).
    pub fractions: Vec<u8>,
}

impl Plan {
    /// The plan of a result set's columns, in server order.
    pub fn for_columns(columns: &[Column]) -> Plan {
        let mut cols = Vec::with_capacity(columns.len());
        let mut kinds = Vec::with_capacity(columns.len());
        let mut fractions = Vec::with_capacity(columns.len());
        let mut meta = Vec::with_capacity(columns.len());
        for c in columns {
            let col = Col::for_column(c);
            kinds.push(col.buf_kind(c));
            fractions.push(c.decimals().min(6));
            let org_table = c.org_table_str();
            let org_name = c.org_name_str();
            meta.push(ColumnMeta {
                name: c.name_str().as_ref().into(),
                logical: col.logical(c),
                native: NativeType::MySql {
                    column_type: c.column_type() as u8,
                    flags: c.flags().bits(),
                    charset: c.character_set(),
                    length: c.column_length(),
                    decimals: c.decimals(),
                },
                origin: (!org_table.is_empty() && !org_name.is_empty()).then(|| {
                    OriginHint::MySql {
                        schema: c.schema_str().as_ref().into(),
                        table: org_table.as_ref().into(),
                        column: org_name.as_ref().into(),
                    }
                }),
            });
            cols.push(col);
        }
        Plan {
            schema: Arc::new(ResultSchema {
                columns: meta.into(),
            }),
            cols,
            kinds,
            fractions,
        }
    }

    /// The number of columns.
    pub fn width(&self) -> usize {
        self.cols.len()
    }

    /// A chunk builder for this result set (`with_schema`, so temporal columns are typed).
    pub fn builder(&self, capacity_rows: u32) -> ChunkBuilder {
        ChunkBuilder::with_schema(&self.kinds, &self.schema, capacity_rows)
    }

    /// Pushes the cells of one row (`None` or `Value::NULL`: SQL NULL) and ends it.
    ///
    /// # Panics
    /// When `values` does not hold one value per column (the caller checks it first).
    pub fn push_row(&self, b: &mut ChunkBuilder, values: &[Option<My>], wire: Wire, cap: u32) {
        assert_eq!(values.len(), self.width(), "one value per column");
        for (i, (col, v)) in self.cols.iter().zip(values).enumerate() {
            let i = i as u32;
            let Some(v) = v.as_ref().filter(|v| !matches!(v, My::NULL)) else {
                b.push_null(i);
                continue;
            };
            let fraction = self.fractions[i as usize];
            let decoded = match (wire, v) {
                (Wire::Text, My::Bytes(raw)) => col.decode_text(raw, cap),
                (Wire::Text, other) => error_text(
                    "mysql-value",
                    format!("a {} value in a text row", value_kind(other)),
                ),
                (Wire::Binary, v) => col.decode_bin(v, fraction, cap),
            };
            push_decoded(b, i, matches!(col, Col::UInt64), decoded, cap);
        }
        b.end_row();
    }
}

/// The logical type of column `c`.
pub fn logical_of(c: &Column) -> LogicalType {
    Col::for_column(c).logical(c)
}

/// The chunk buffer of column `c`.
pub fn buf_kind_of(c: &Column) -> BufKind {
    Col::for_column(c).buf_kind(c)
}

/// One cell through the real path (a one-row chunk built with `with_schema`), read back from
/// the chunk: what the result store holds for these bytes.
fn read_back(c: &Column, value: Option<My>, wire: Wire, cap: u32) -> Value {
    let plan = Plan::for_columns(std::slice::from_ref(c));
    let mut b = plan.builder(1);
    plan.push_row(&mut b, &[value], wire, cap);
    b.seal().cell(0, 0).to_value()
}

/// A text-protocol cell of column `c` (`None`: NULL), as the store holds it.
pub fn decode_cell_text(c: &Column, raw: Option<&[u8]>, cap: u32) -> Value {
    read_back(c, raw.map(|r| My::Bytes(r.to_vec())), Wire::Text, cap)
}

/// A binary-protocol cell of column `c`, as the store holds it.
pub fn decode_cell_bin(c: &Column, v: &My, cap: u32) -> Value {
    read_back(c, Some(v.clone()), Wire::Binary, cap)
}
