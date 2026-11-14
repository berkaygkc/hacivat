//! `raw_text`: the locale-free canonical text of a value (I18N-11; Task 16), and the writers
//! that [`super::format_cell`] shares with it for everything that is not a grouped number.
//!
//! The spellings are PostgreSQL's text output, read from a PostgreSQL 17 server when this was
//! written: shortest round-trip floats in fixed notation for decimal exponents `-4 ..< 15`
//! (`-4 ..< 6` for `float4`) and `1e+15`/`1e-05` outside; `NaN`, `Infinity`, `-Infinity`
//! (SQLite spells the infinities `Inf`, `-Inf`); ISO dates with years of at least four digits and
//! a ` BC` suffix after the offset; fractions without trailing zeros (MySQL pads them to the
//! column's precision); offsets `+03`, `+05:30`, `+01:56:56`; intervals in the `postgres`
//! `IntervalStyle` (`1 year 2 mons -1 days +01:00:00`); bytes as `\x` and lower-case hex. No
//! function here reads the process locale, the OS region or the display zone.

use std::borrow::Cow;
use std::fmt::{self, Write as _};

use hacivat_proto::DbmsKind;

use crate::logical::LogicalType;
use crate::special::SpecialVal;
use crate::temporal::{DateVal, TimeVal, TsVal};
use crate::valfmt::ColumnFacts;
use crate::value::ValueRef;

/// A value's raw text, or SQL NULL.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RawText<'a> {
    Null,
    Text(Cow<'a, str>),
}

/// The locale-free text of `v` (`1.50`, `2026-11-13 10:00:00.123456+03`, `\x00ff`, `NaN`):
/// copy, sort keys and SQL values use it, never the display text.
///
/// Text, JSON, XML, server text (`Opaque`), error messages and specials (but MySQL's zero
/// datetime with a fraction) are borrowed. A
/// `timestamptz` without the server's offset is written in UTC (`+00`): raw text has no
/// display zone. Bytes are always hex, whatever the column's [`super::BinaryDisplay`]. Text that
/// is not valid UTF-8 (`Lossy`) keeps its valid runs and writes each other byte as `\xNN`; a
/// truncated value is its prefix's raw text; an `Error` cell is its message.
pub fn raw_text<'a>(v: ValueRef<'a>, col: &ColumnFacts<'_>) -> RawText<'a> {
    let text: Cow<'a, str> = match v {
        ValueRef::Null => return RawText::Null,
        ValueRef::Truncated(t) => return raw_text(t.prefix.view(), col),
        ValueRef::Missing => Cow::Borrowed(""),
        ValueRef::Default => Cow::Borrowed("DEFAULT"),
        ValueRef::Bool(b) => Cow::Borrowed(bool_text(b)),
        ValueRef::Text(t) | ValueRef::Json(t) | ValueRef::Xml(t) => Cow::Borrowed(t),
        ValueRef::Opaque(o) => Cow::Borrowed(&o.text),
        ValueRef::Error(e) => Cow::Borrowed(&e.message),
        ValueRef::Special(sp) => special_text(sp, col),
        ValueRef::F32(x) if !x.is_finite() => Cow::Borrowed(float_special(f64::from(x), col)),
        ValueRef::F64(x) if !x.is_finite() => Cow::Borrowed(float_special(x, col)),
        _ => {
            // One allocation: numbers and temporal values fit 40 bytes.
            let mut out = String::with_capacity(40);
            write_scalar(v, col, None, &mut out);
            Cow::Owned(out)
        }
    };
    RawText::Text(text)
}

pub(super) fn bool_text(b: bool) -> &'static str {
    if b { "true" } else { "false" }
}

/// The raw text of the values that have no borrowed form: numbers, bytes, UUIDs, temporal
/// values and lossy text. `zone_offset` is the offset a `timestamptz` without the server's
/// offset is shown in (`None`: UTC). Returns `false`, having written an English message, for a
/// timestamp beyond the calendar.
pub(super) fn write_scalar(
    v: ValueRef<'_>,
    col: &ColumnFacts<'_>,
    zone_offset: Option<&dyn Fn(TsVal) -> i32>,
    out: &mut String,
) -> bool {
    match v {
        ValueRef::Int(i) => push_i64(out, i),
        ValueRef::UInt(u) => push_u64(out, u),
        ValueRef::F32(x) => push_float(out, f64::from(x), true, col),
        ValueRef::F64(x) => push_float(out, x, single_column(x, col), col),
        ValueRef::Decimal(d) => d.write_canonical(out),
        ValueRef::Bytes(b) => push_hex(out, b, usize::MAX),
        ValueRef::Uuid(u) => push_uuid(out, &u),
        ValueRef::Lossy(b) => push_escaped(out, b),
        ValueRef::Date(d) => {
            let bc = push_date(out, d);
            push_bc(out, bc);
        }
        ValueRef::Time(t) => push_time(out, t, frac_min(col)),
        ValueRef::TimeTz(t, off) => {
            push_time(out, t, frac_min(col));
            push_offset(out, off);
        }
        ValueRef::Timestamp(ts) => return push_timestamp(out, ts, None, frac_min(col)),
        ValueRef::TimestampTz { utc, offset } => {
            let off = match (offset, zone_offset) {
                (Some(o), _) => o,
                (None, Some(zone)) => zone(normalized(utc).unwrap_or(utc)),
                (None, None) => 0,
            };
            return push_timestamp(out, utc, Some(off), frac_min(col));
        }
        ValueRef::Interval {
            months,
            days,
            micros,
        } => {
            if months == 0 && is_mysql_time(col) {
                push_mysql_time(out, days, micros, frac_min(col));
            } else {
                push_interval(out, months, days, micros);
            }
        }
        // Handled by the callers; written here only so the match is total.
        ValueRef::Null => out.push_str("NULL"),
        ValueRef::Missing => {}
        ValueRef::Default => out.push_str("DEFAULT"),
        ValueRef::Bool(b) => out.push_str(bool_text(b)),
        ValueRef::Text(t) | ValueRef::Json(t) | ValueRef::Xml(t) => out.push_str(t),
        ValueRef::Opaque(o) => out.push_str(&o.text),
        ValueRef::Error(e) => out.push_str(&e.message),
        ValueRef::Special(sp) => out.push_str(&special_text(sp, col)),
        ValueRef::Truncated(t) => return write_scalar(t.prefix.view(), col, zone_offset, out),
    }
    true
}

// ---- small buffers and digits ------------------------------------------------------------

/// A fixed stack buffer of ASCII text (formatting never allocates through it).
pub(super) struct Buf<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> Buf<N> {
    pub(super) fn new() -> Self {
        Buf {
            bytes: [0; N],
            len: 0,
        }
    }

    pub(super) fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    pub(super) fn as_str(&self) -> &str {
        ascii(self.as_bytes())
    }

    fn push(&mut self, b: u8) {
        if self.len < N {
            self.bytes[self.len] = b;
            self.len += 1;
        }
    }

    fn extend(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.push(b);
        }
    }
}

impl<const N: usize> fmt::Write for Buf<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let end = self.len + s.len();
        if end > N {
            return Err(fmt::Error);
        }
        self.bytes[self.len..end].copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

/// ASCII bytes as text (every caller writes ASCII only).
pub(super) fn ascii(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).unwrap_or_default()
}

/// The decimal digits of `v`, right-aligned in a 20-byte buffer: `(buffer, first index)`.
fn digits_u64(mut v: u64) -> ([u8; 20], usize) {
    let mut buf = [b'0'; 20];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            return (buf, i);
        }
    }
}

pub(super) fn push_u64(out: &mut String, v: u64) {
    let (buf, i) = digits_u64(v);
    out.push_str(ascii(&buf[i..]));
}

pub(super) fn push_i64(out: &mut String, v: i64) {
    if v < 0 {
        out.push('-');
    }
    push_u64(out, v.unsigned_abs());
}

/// `v` zero-padded to at least `width` digits.
fn push_padded(out: &mut String, v: u64, width: usize) {
    let (buf, i) = digits_u64(v);
    for _ in (buf.len() - i)..width {
        out.push('0');
    }
    out.push_str(ascii(&buf[i..]));
}

// ---- floats ------------------------------------------------------------------------------

/// Whether an `f64` is shown with `float4`'s spelling: it sits in a `float4` column and is
/// exactly an `f32` (a widened `float4`).
pub(super) fn single_column(x: f64, col: &ColumnFacts<'_>) -> bool {
    matches!(col.logical, LogicalType::Float { bits: 32 }) && f64::from(x as f32) == x
}

/// `NaN` and the infinities as the engine spells them.
pub(super) fn float_special(x: f64, col: &ColumnFacts<'_>) -> &'static str {
    let sqlite = col.engine == DbmsKind::Sqlite;
    match (x.is_nan(), x > 0.0, sqlite) {
        (true, ..) => "NaN",
        (false, true, false) => "Infinity",
        (false, false, false) => "-Infinity",
        (false, true, true) => "Inf",
        (false, false, true) => "-Inf",
    }
}

/// The canonical text of a finite float: its shortest round-trip digits, in fixed notation for
/// decimal exponents `-4 ..< 15` (`-4 ..< 6` for `float4`, `single`) and as `d.ddde±XX`
/// outside, exactly as PostgreSQL prints it.
pub(super) fn float_canonical(x: f64, single: bool) -> Buf<48> {
    let mut sci = Buf::<40>::new();
    // Rust's `{:e}` without a precision is the shortest round-trip form (`1.2345e3`).
    let _ = if single {
        write!(sci, "{:e}", x as f32)
    } else {
        write!(sci, "{x:e}")
    };
    let s = sci.as_bytes();
    let (neg, s) = match s.split_first() {
        Some((b'-', rest)) => (true, rest),
        _ => (false, s),
    };
    let e = s.iter().position(|&c| c == b'e').unwrap_or(s.len());
    // At most 17 significant digits (9 for `f32`).
    let mut digits = Buf::<24>::new();
    for &c in &s[..e] {
        if c != b'.' {
            digits.push(c);
        }
    }
    let exp = parse_exp(s.get(e + 1..).unwrap_or_default());
    let digits = digits.as_bytes();
    let limit = if single { 6 } else { 15 };
    let mut out = Buf::<48>::new();
    if neg {
        out.push(b'-');
    }
    if (-4..limit).contains(&exp) {
        if exp >= 0 {
            let int_len = exp as usize + 1;
            if digits.len() <= int_len {
                out.extend(digits);
                for _ in digits.len()..int_len {
                    out.push(b'0');
                }
            } else {
                out.extend(&digits[..int_len]);
                out.push(b'.');
                out.extend(&digits[int_len..]);
            }
        } else {
            out.extend(b"0.");
            for _ in 0..(-exp - 1) {
                out.push(b'0');
            }
            out.extend(digits);
        }
    } else {
        out.extend(&digits[..1]);
        if digits.len() > 1 {
            out.push(b'.');
            out.extend(&digits[1..]);
        }
        out.push(b'e');
        out.push(if exp < 0 { b'-' } else { b'+' });
        let (buf, i) = digits_u64(u64::from(exp.unsigned_abs()));
        if buf.len() - i < 2 {
            out.push(b'0');
        }
        out.extend(&buf[i..]);
    }
    out
}

/// The exponent of a `{:e}` rendering (`3`, `-7`).
fn parse_exp(s: &[u8]) -> i32 {
    let (neg, s) = match s.split_first() {
        Some((b'-', rest)) => (true, rest),
        _ => (false, s),
    };
    let mag = s
        .iter()
        .fold(0i32, |acc, &c| acc * 10 + i32::from(c - b'0'));
    if neg { -mag } else { mag }
}

fn push_float(out: &mut String, x: f64, single: bool, col: &ColumnFacts<'_>) {
    if x.is_finite() {
        out.push_str(float_canonical(x, single).as_str());
    } else {
        out.push_str(float_special(x, col));
    }
}

// ---- bytes and text ----------------------------------------------------------------------

const HEX: &[u8; 16] = b"0123456789abcdef";

/// `\x` and the lower-case hex of at most `max_bytes` bytes.
pub(super) fn push_hex(out: &mut String, b: &[u8], max_bytes: usize) {
    let b = &b[..b.len().min(max_bytes)];
    out.reserve(2 + 2 * b.len());
    out.push_str("\\x");
    for &byte in b {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0xf)]));
    }
}

/// The canonical lower-case, hyphenated UUID.
pub(super) fn push_uuid(out: &mut String, u: &[u8; 16]) {
    out.reserve(36);
    for (i, &byte) in u.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0xf)]));
    }
}

/// Text that is not valid UTF-8: its valid runs as they are, each other byte as `\xNN`
/// (never replaced by U+FFFD).
pub(super) fn push_escaped(out: &mut String, b: &[u8]) {
    for chunk in b.utf8_chunks() {
        out.push_str(chunk.valid());
        for &byte in chunk.invalid() {
            out.push_str("\\x");
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0xf)]));
        }
    }
}

// ---- specials ----------------------------------------------------------------------------

pub(super) fn is_numeric(logical: &LogicalType) -> bool {
    matches!(
        logical,
        LogicalType::Int { .. }
            | LogicalType::Decimal { .. }
            | LogicalType::Float { .. }
            | LogicalType::Money
    )
}

/// A special as its column spells it: PostgreSQL's `numeric` and floats say `Infinity`, its
/// dates and timestamps `infinity`; MySQL's zero date carries the column's time and fraction.
pub(super) fn special_text<'a>(sp: &'a SpecialVal, col: &ColumnFacts<'_>) -> Cow<'a, str> {
    let numeric = is_numeric(col.logical);
    Cow::Borrowed(match sp {
        SpecialVal::NaN => "NaN",
        SpecialVal::PosInfinity if matches!(col.logical, LogicalType::Float { .. }) => {
            float_special(f64::INFINITY, col)
        }
        SpecialVal::NegInfinity if matches!(col.logical, LogicalType::Float { .. }) => {
            float_special(f64::NEG_INFINITY, col)
        }
        SpecialVal::PosInfinity if numeric => "Infinity",
        SpecialVal::NegInfinity if numeric => "-Infinity",
        SpecialVal::PosInfinity => "infinity",
        SpecialVal::NegInfinity => "-infinity",
        SpecialVal::PartialZeroDate(text) => text,
        SpecialVal::ZeroDate => match *col.logical {
            LogicalType::Timestamp { precision, .. } => {
                let mut s = String::from("0000-00-00 00:00:00");
                if precision > 0 {
                    s.push('.');
                    for _ in 0..precision.min(9) {
                        s.push('0');
                    }
                }
                return Cow::Owned(s);
            }
            _ => "0000-00-00",
        },
    })
}

// ---- dates, times, timestamps, intervals -------------------------------------------------

const NANOS_PER_SEC: u64 = 1_000_000_000;
const SECS_PER_DAY: i64 = 86_400;

fn is_mysql_family(engine: DbmsKind) -> bool {
    matches!(engine, DbmsKind::MySql | DbmsKind::MariaDb | DbmsKind::TiDb)
}

fn is_mysql_time(col: &ColumnFacts<'_>) -> bool {
    is_mysql_family(col.engine) && matches!(col.logical, LogicalType::Time { tz: false, .. })
}

/// The fewest fraction digits shown: MySQL pads to the column's precision; PostgreSQL (and
/// every other engine) shows only the digits up to the last nonzero one.
fn frac_min(col: &ColumnFacts<'_>) -> usize {
    if !is_mysql_family(col.engine) {
        return 0;
    }
    match *col.logical {
        LogicalType::Timestamp { precision, .. } | LogicalType::Time { precision, .. } => {
            usize::from(precision.min(9))
        }
        _ => 0,
    }
}

/// `.` and the fraction of `nanos`, trailing zeros dropped but at least `min` digits; nothing
/// when that leaves no digit. A digit past `min` is never hidden.
fn push_fraction(out: &mut String, nanos: u64, min: usize) {
    let mut digits = [b'0'; 9];
    let mut n = nanos % NANOS_PER_SEC;
    for slot in digits.iter_mut().rev() {
        *slot = b'0' + (n % 10) as u8;
        n /= 10;
    }
    let significant = digits.iter().rposition(|&d| d != b'0').map_or(0, |p| p + 1);
    let shown = significant.max(min);
    if shown > 0 {
        out.push('.');
        out.push_str(ascii(&digits[..shown]));
    }
}

/// `YYYY-MM-DD` (years of at least four digits); returns whether the date is BC, whose
/// ` BC` suffix the caller writes last, after any time and offset (PostgreSQL's order).
fn push_date(out: &mut String, d: DateVal) -> bool {
    let (y, m, day) = d.to_civil();
    let bc = y <= 0;
    let year = if bc { 1 - i64::from(y) } else { i64::from(y) };
    push_padded(out, year.unsigned_abs(), 4);
    out.push('-');
    push_padded(out, u64::from(m), 2);
    out.push('-');
    push_padded(out, u64::from(day), 2);
    bc
}

fn push_bc(out: &mut String, bc: bool) {
    if bc {
        out.push_str(" BC");
    }
}

/// `HH:MM:SS[.f]` of nanoseconds since midnight (`24:00:00` kept; hours are never wrapped).
fn push_clock(out: &mut String, nanos_of_day: u64, min_frac: usize) {
    let secs = nanos_of_day / NANOS_PER_SEC;
    push_padded(out, secs / 3_600, 2);
    out.push(':');
    push_padded(out, secs / 60 % 60, 2);
    out.push(':');
    push_padded(out, secs % 60, 2);
    push_fraction(out, nanos_of_day % NANOS_PER_SEC, min_frac);
}

fn push_time(out: &mut String, t: TimeVal, min_frac: usize) {
    push_clock(out, t.nanos_of_day, min_frac);
}

/// `+03`, `-08`, `+05:30`, `+01:56:56`: the offset in seconds east of UTC.
pub(super) fn push_offset(out: &mut String, secs_east: i32) {
    out.push(if secs_east < 0 { '-' } else { '+' });
    let a = u64::from(secs_east.unsigned_abs());
    push_padded(out, a / 3_600, 2);
    let (mm, ss) = (a / 60 % 60, a % 60);
    if mm != 0 || ss != 0 {
        out.push(':');
        push_padded(out, mm, 2);
    }
    if ss != 0 {
        out.push(':');
        push_padded(out, ss, 2);
    }
}

/// `ts` with its nanoseconds below a second (carried into the seconds), or `None` when the
/// carry leaves `i64`.
fn normalized(ts: TsVal) -> Option<TsVal> {
    let carry = i64::from(ts.nanos / NANOS_PER_SEC as u32);
    Some(TsVal {
        secs: ts.secs.checked_add(carry)?,
        nanos: ts.nanos % NANOS_PER_SEC as u32,
    })
}

/// `YYYY-MM-DD HH:MM:SS[.f][offset][ BC]`, shown at `offset` seconds east when there is one.
/// Returns `false`, having written an English message, when the instant is beyond the calendar
/// (more than about 5.8 million years from 1970: no engine sends one).
fn push_timestamp(out: &mut String, ts: TsVal, offset: Option<i32>, min_frac: usize) -> bool {
    let local = normalized(ts).and_then(|n| {
        let secs = n.secs.checked_add(i64::from(offset.unwrap_or(0)))?;
        let day = i32::try_from(secs.div_euclid(SECS_PER_DAY)).ok()?;
        Some((DateVal(day), secs.rem_euclid(SECS_PER_DAY) as u64, n.nanos))
    });
    let Some((day, sod, nanos)) = local else {
        out.push_str("the timestamp is outside the supported range (raw value ");
        push_i64(out, ts.secs);
        out.push_str(" s)");
        return false;
    };
    let bc = push_date(out, day);
    out.push(' ');
    push_clock(out, sod * NANOS_PER_SEC + u64::from(nanos), min_frac);
    if let Some(off) = offset {
        push_offset(out, off);
    }
    push_bc(out, bc);
    true
}

/// One `postgres`-style interval field (`AddPostgresIntPart`): ` +2 mons`, `-1 days`.
fn push_part(out: &mut String, value: i64, unit: &str, is_zero: &mut bool, is_before: &mut bool) {
    if value == 0 {
        return;
    }
    if !*is_zero {
        out.push(' ');
    }
    if *is_before && value > 0 {
        out.push('+');
    }
    push_i64(out, value);
    out.push(' ');
    out.push_str(unit);
    if value != 1 {
        out.push('s');
    }
    // Each nonzero field decides the `+` of the next one only, as PostgreSQL does.
    *is_before = value < 0;
    *is_zero = false;
}

/// PostgreSQL's `postgres` `IntervalStyle`: `1 year 2 mons -1 days +01:00:00`, `00:00:00`.
fn push_interval(out: &mut String, months: i32, days: i32, micros: i64) {
    let (mut is_zero, mut is_before) = (true, false);
    push_part(
        out,
        i64::from(months / 12),
        "year",
        &mut is_zero,
        &mut is_before,
    );
    push_part(
        out,
        i64::from(months % 12),
        "mon",
        &mut is_zero,
        &mut is_before,
    );
    push_part(out, i64::from(days), "day", &mut is_zero, &mut is_before);
    if !is_zero && micros == 0 {
        return;
    }
    if !is_zero {
        out.push(' ');
    }
    if micros < 0 {
        out.push('-');
    } else if is_before {
        out.push('+');
    }
    push_hms(out, u128::from(micros.unsigned_abs()), 0);
}

/// `HH:MM:SS[.f]` of a magnitude in microseconds (hours unbounded).
fn push_hms(out: &mut String, micros: u128, min_frac: usize) {
    let secs = micros / 1_000_000;
    // Hours of an `i64`/`i32` sum of microseconds fit a `u64` comfortably.
    push_padded(out, (secs / 3_600) as u64, 2);
    out.push(':');
    push_padded(out, (secs / 60 % 60) as u64, 2);
    out.push(':');
    push_padded(out, (secs % 60) as u64, 2);
    push_fraction(out, (micros % 1_000_000) as u64 * 1_000, min_frac);
}

/// MySQL's `TIME` beyond a day (`-838:59:59 ..= 838:59:59`), which the driver keeps as an
/// interval: `[-]H:MM:SS[.f]` with the hours counted through.
fn push_mysql_time(out: &mut String, days: i32, micros: i64, min_frac: usize) {
    let total = i128::from(days) * 86_400_000_000 + i128::from(micros);
    if total < 0 {
        out.push('-');
    }
    push_hms(out, total.unsigned_abs(), min_frac);
}
