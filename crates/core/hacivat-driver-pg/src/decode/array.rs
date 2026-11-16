//! Arrays in binary (M3b Task 7, R-M3-67): one cell is the array's PostgreSQL literal, exactly
//! as `array_out` writes it, kept as `Value::Text` under `LogicalType::Array(elem)`.
//!
//! **Wire** (`array_send`): `ndim`, a has-null flag (0 or 1), the element type's OID, then each
//! dimension's length and lower bound, then every element in row-major order as a length (`-1`
//! for NULL) and its bytes in the element type's binary format.
//!
//! **Literal** (`array_out`): `{}` for no dimensions; the dimensions as `[lb:ub]` before an `=`
//! when any lower bound is not 1 (`[0:2]={1,2,3}`); nested braces per dimension; elements joined
//! by `,`; NULL as `NULL`. An element is double-quoted when it is empty, is the word `NULL` in
//! any case, or holds `"`, `\`, `{`, `}`, `,` or whitespace (space, tab, line feed, carriage
//! return, vertical tab, form feed); inside quotes `"` and `\` take a backslash.
//!
//! **Element spellings** are PostgreSQL's output functions as `hacivat_values::valfmt::raw_text`
//! writes them (read from a server for M3a's scalars): `t`/`f` for booleans (`boolout`), floats
//! and numerics with PostgreSQL's spelling, ISO dates, the `postgres` interval style, bytes as
//! `\x` hex, text and JSON as the server's bytes. One deliberate difference: a `timestamptz`
//! element is written in UTC (`+00`), because an array literal has no display zone; the
//! instant is the same, so the literal round-trips. **Copy:** a `timestamptz[]` cell therefore
//! copies with `+00` offsets, while a scalar `timestamptz` copies in the console's display zone
//! (`raw_text_in`); psql in a non-UTC session would print the array in its own zone (fix round
//! 1, review m-4). A `"char"` element is `charout`'s spelling: its empty value is `""` and a
//! byte with the high bit set is `\ooo` (PostgreSQL 15+'s form, on every server line).
//!
//! **Never silently wrong:** a header no server sends (more than 6 dimensions, a negative or zero
//! dimension, flags other than 0 or 1, another element OID than the column's, more elements
//! than the bytes could hold, an upper bound past `i32`, a length below `-1` or past the buffer,
//! bytes after the last element) or any element its own decoder refuses makes the whole cell
//! an `Error`, never an array with a hole in it.
//!
//! **Large values:** the literal is written under the cell cap: past it, only the bytes within
//! the cap are kept (cut at a character boundary) and the whole literal's length is counted, so
//! the cell is `Truncated` with its exact total. Text that is not UTF-8 (a `SQL_ASCII` database)
//! keeps its bytes (`Lossy`).

use hacivat_proto::DbmsKind;
use hacivat_values::valfmt::{BinaryDisplay, ColumnFacts, RawText, raw_text};
use hacivat_values::{
    ChunkBuilder, DateVal, LogicalType, TimeVal, TsVal, Value, ValueRef, VarKind,
};

use super::{Col, Decoded, error};

/// PostgreSQL's `MAXDIM`.
const MAX_DIMS: usize = 6;
/// `"char"`'s OID: its empty value is the byte 0, which `charout` prints as an empty string.
const CHAR_OID: u32 = 18;

/// The decoder of a binary array column: its element's decoder, OID and logical type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrayCol {
    pub elem: Col,
    pub elem_oid: u32,
    pub elem_logical: LogicalType,
}

/// A literal written under a cap: the bytes within it (and a few more, to cut at a character
/// boundary), and the whole length.
struct Capped {
    kept: Vec<u8>,
    limit: usize,
    total: u64,
}

impl Capped {
    fn new(cap: u32) -> Self {
        let limit = (cap as usize).saturating_add(4);
        Capped {
            kept: Vec::with_capacity(limit.min(256)),
            limit,
            total: 0,
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        self.total += bytes.len() as u64;
        let room = self.limit.saturating_sub(self.kept.len());
        self.kept.extend_from_slice(&bytes[..bytes.len().min(room)]);
    }

    fn byte(&mut self, b: u8) {
        self.push(&[b]);
    }

    /// The cell: `Text`, `Lossy`, or `Truncated` with the whole length.
    fn finish(self, cap: u32) -> Value {
        match ChunkBuilder::capped_value(VarKind::Text, &self.kept, cap) {
            Value::Truncated(mut t) => {
                t.total_len = Some(self.total);
                Value::Truncated(t)
            }
            other => other,
        }
    }
}

/// A big-endian `i32` at `*at`, advancing it.
fn read_i32(raw: &[u8], at: &mut usize) -> Option<i32> {
    let bytes = raw.get(*at..*at + 4)?;
    *at += 4;
    Some(i32::from_be_bytes(bytes.try_into().ok()?))
}

/// The array literal of one binary array cell.
pub fn decode(raw: &[u8], col: &ArrayCol, cap: u32) -> Decoded<'static> {
    match literal(raw, col, cap) {
        Ok(v) => Decoded::Value(v),
        Err(message) => error("pg-array", message),
    }
}

fn literal(raw: &[u8], col: &ArrayCol, cap: u32) -> Result<Value, String> {
    let mut at = 0;
    let short = || format!("an array header cut short ({} bytes)", raw.len());
    let ndim = read_i32(raw, &mut at).ok_or_else(short)?;
    let flags = read_i32(raw, &mut at).ok_or_else(short)?;
    let oid = read_i32(raw, &mut at).ok_or_else(short)? as u32;
    let ndim = usize::try_from(ndim)
        .ok()
        .filter(|n| *n <= MAX_DIMS)
        .ok_or_else(|| format!("an array of {ndim} dimensions"))?;
    if flags != 0 && flags != 1 {
        return Err(format!("an array with flags {flags}"));
    }
    if oid != col.elem_oid {
        return Err(format!(
            "an array of element type {oid} in a column of element type {}",
            col.elem_oid
        ));
    }
    let mut dims = [(0usize, 0i32); MAX_DIMS];
    let mut items: usize = 1;
    for d in dims.iter_mut().take(ndim) {
        let len = read_i32(raw, &mut at).ok_or_else(short)?;
        let lower = read_i32(raw, &mut at).ok_or_else(short)?;
        let n = usize::try_from(len)
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| format!("an array dimension of length {len}"))?;
        if lower.checked_add(len - 1).is_none() {
            return Err(format!("an array dimension [{lower}] of length {len}"));
        }
        items = items
            .checked_mul(n)
            .ok_or_else(|| "an array of more elements than its bytes".to_string())?;
        *d = (n, lower);
    }
    let dims = &dims[..ndim];
    // Every element takes at least its 4-byte length.
    let rest = raw.len() - at;
    if ndim > 0 && items > rest / 4 {
        return Err(format!("an array of {items} elements in {rest} bytes"));
    }
    let mut out = Capped::new(cap);
    if ndim == 0 {
        if at != raw.len() {
            return Err("bytes after an empty array".into());
        }
        out.push(b"{}");
        return Ok(out.finish(cap));
    }
    if dims.iter().any(|&(_, lower)| lower != 1) {
        for &(len, lower) in dims {
            // Checked above: the upper bound fits an `i32`.
            let upper = i64::from(lower) + len as i64 - 1;
            out.push(format!("[{lower}:{upper}]").as_bytes());
        }
        out.byte(b'=');
    }
    let mut index = [0usize; MAX_DIMS];
    let mut open_from = 0;
    let mut element = Vec::new();
    for k in 0..items {
        for _ in open_from..ndim {
            out.byte(b'{');
        }
        let len = read_i32(raw, &mut at).ok_or_else(|| format!("array element {k} cut short"))?;
        if len == -1 {
            out.push(b"NULL");
        } else {
            let n = usize::try_from(len)
                .ok()
                .filter(|n| *n <= raw.len() - at)
                .ok_or_else(|| format!("array element {k} of length {len}"))?;
            let bytes = &raw[at..at + n];
            at += n;
            element.clear();
            write_element(&mut element, col, bytes)
                .map_err(|e| format!("array element {k}: {e}"))?;
            push_quoted(&mut out, &element);
        }
        // Close every dimension that ends here, then separate from the next element.
        let mut d = ndim;
        loop {
            if d == 0 {
                break;
            }
            d -= 1;
            index[d] += 1;
            if index[d] < dims[d].0 {
                out.byte(b',');
                open_from = d + 1;
                break;
            }
            index[d] = 0;
            out.byte(b'}');
        }
    }
    if at != raw.len() {
        return Err(format!(
            "{} bytes after the last array element",
            raw.len() - at
        ));
    }
    Ok(out.finish(cap))
}

/// `element`, quoted as `array_out` quotes it.
fn push_quoted(out: &mut Capped, element: &[u8]) {
    let needs = element.is_empty()
        || element.eq_ignore_ascii_case(b"NULL")
        || element.iter().any(|b| {
            matches!(
                b,
                b'"' | b'\\' | b'{' | b'}' | b',' | b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c
            )
        });
    if !needs {
        out.push(element);
        return;
    }
    out.byte(b'"');
    for &b in element {
        if b == b'"' || b == b'\\' {
            out.byte(b'\\');
        }
        out.byte(b);
    }
    out.byte(b'"');
}

/// One element's text (unquoted), as the element type's output function writes it.
fn write_element(out: &mut Vec<u8>, col: &ArrayCol, raw: &[u8]) -> Result<(), String> {
    let decoded = col.elem.decode(raw, u32::MAX);
    let facts = ColumnFacts {
        logical: &col.elem_logical,
        native_name: "",
        id_like: false,
        binary: BinaryDisplay::Hex,
        engine: DbmsKind::Postgres,
    };
    let owned;
    let value: ValueRef<'_> = match decoded {
        Decoded::Bool(b) => {
            out.push(if b { b't' } else { b'f' });
            return Ok(());
        }
        Decoded::Var(bytes) => match col.elem {
            Col::Bytea => ValueRef::Bytes(bytes),
            // `"char"`'s empty value.
            Col::Text if col.elem_oid == CHAR_OID && bytes == [0] => return Ok(()),
            // A `"char"` byte with the high bit set: `charout`'s `\ooo` (PostgreSQL 15+; 14
            // printed the raw byte, which is not text in a UTF-8 literal). Fix round 1, m-3.
            Col::Text if col.elem_oid == CHAR_OID && bytes.len() == 1 && bytes[0] >= 0x80 => {
                let b = bytes[0];
                out.extend_from_slice(&[
                    b'\\',
                    b'0' + (b >> 6),
                    b'0' + ((b >> 3) & 7),
                    b'0' + (b & 7),
                ]);
                return Ok(());
            }
            // Text, enum labels and JSON: the server's bytes as they are.
            _ => {
                out.extend_from_slice(bytes);
                return Ok(());
            }
        },
        Decoded::Int(i) => ValueRef::Int(i),
        Decoded::Float(x) if col.elem == Col::Float4 => ValueRef::F32(x as f32),
        Decoded::Float(x) => ValueRef::F64(x),
        Decoded::Decimal(d) => {
            owned = Value::Decimal(d);
            owned.view()
        }
        Decoded::Uuid(u) => ValueRef::Uuid(u),
        Decoded::Temporal(v) => match col.elem {
            Col::Date => ValueRef::Date(DateVal(
                i32::try_from(v).map_err(|_| format!("a date of {v} days"))?,
            )),
            Col::Time => ValueRef::Time(
                TimeVal::try_from_micros(v).ok_or_else(|| format!("a time of {v} µs"))?,
            ),
            Col::TimestampTz => ValueRef::TimestampTz {
                utc: TsVal::from_unix_micros(v),
                offset: None,
            },
            _ => ValueRef::Timestamp(TsVal::from_unix_micros(v)),
        },
        Decoded::Temporal2(v, offset) => ValueRef::TimeTz(
            TimeVal::try_from_micros(v).ok_or_else(|| format!("a time of {v} µs"))?,
            offset,
        ),
        Decoded::Value(Value::Error(e)) => return Err(e.message.to_string()),
        Decoded::Value(v) => {
            owned = v;
            owned.view()
        }
    };
    // PostgreSQL's timestamps (±294,277 years) always lie within the calendar `raw_text`
    // writes, so its out-of-range message never reaches a literal.
    match raw_text(value, &facts) {
        RawText::Text(t) => out.extend_from_slice(t.as_bytes()),
        RawText::Null => out.extend_from_slice(b"NULL"),
    }
    Ok(())
}
