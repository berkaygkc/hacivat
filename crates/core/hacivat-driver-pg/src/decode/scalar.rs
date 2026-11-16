//! Booleans, integers, floats, UUID, JSONB and the opaque text of every type M3a does not
//! decode (Task 12). The text family, `bytea` and `json` need no decoding: their bytes go to the
//! chunk builder as they are (`Decoded::Var`).

use std::sync::Arc;

use hacivat_values::{ChunkBuilder, OpaqueVal, Value, VarKind};

use super::{Decoded, bad_length, error};

/// The `N` bytes of a fixed-width value, or the cell's error.
fn fixed<const N: usize>(raw: &[u8], what: &str) -> Result<[u8; N], Decoded<'static>> {
    raw.try_into().map_err(|_| bad_length(what, raw.len(), N))
}

/// `bool`: one byte, 0 or 1.
pub fn bool(raw: &[u8]) -> Decoded<'_> {
    match fixed::<1>(raw, "bool") {
        Ok([0]) => Decoded::Bool(false),
        Ok([1]) => Decoded::Bool(true),
        Ok([b]) => error("pg-value", format!("a bool value of {b} (expected 0 or 1)")),
        Err(e) => e,
    }
}

pub fn int2(raw: &[u8]) -> Decoded<'_> {
    fixed::<2>(raw, "int2").map_or_else(|e| e, |b| Decoded::Int(i16::from_be_bytes(b).into()))
}

pub fn int4(raw: &[u8]) -> Decoded<'_> {
    fixed::<4>(raw, "int4").map_or_else(|e| e, |b| Decoded::Int(i32::from_be_bytes(b).into()))
}

pub fn int8(raw: &[u8]) -> Decoded<'_> {
    fixed::<8>(raw, "int8").map_or_else(|e| e, |b| Decoded::Int(i64::from_be_bytes(b)))
}

/// `oid`, `xid`, `cid`: an unsigned 32-bit number.
pub fn oid(raw: &[u8]) -> Decoded<'_> {
    fixed::<4>(raw, "oid").map_or_else(|e| e, |b| Decoded::Int(u32::from_be_bytes(b).into()))
}

/// `float4`, widened exactly (NaN, ±Inf and −0.0 kept; the column keeps it as `f32`, so its
/// shortest form is the server's).
pub fn float4(raw: &[u8]) -> Decoded<'_> {
    fixed::<4>(raw, "float4")
        .map_or_else(|e| e, |b| Decoded::Float(f64::from(f32::from_be_bytes(b))))
}

pub fn float8(raw: &[u8]) -> Decoded<'_> {
    fixed::<8>(raw, "float8").map_or_else(|e| e, |b| Decoded::Float(f64::from_be_bytes(b)))
}

pub fn uuid(raw: &[u8]) -> Decoded<'_> {
    fixed::<16>(raw, "uuid").map_or_else(|e| e, Decoded::Uuid)
}

/// `jsonb`: a version byte, 1 (the only one PostgreSQL has sent so far), then the JSON text.
pub fn jsonb(raw: &[u8]) -> Decoded<'_> {
    match raw.split_first() {
        Some((1, text)) => Decoded::Var(text),
        Some((v, _)) => error(
            "pg-jsonb-version",
            format!("a jsonb value of format version {v} (this decoder knows 1)"),
        ),
        None => error("pg-jsonb-version", "a jsonb value without a version".into()),
    }
}

/// The server's text of a type M3a does not decode (R-M3-31), under `cap`: `Value::Opaque`;
/// text that is not UTF-8 stays `Lossy` and text past the cap is `Truncated` (a `Text`
/// prefix), as a `Var(Text)` column keeps them.
pub fn opaque(type_name: &Arc<str>, raw: &[u8], cap: u32) -> Value {
    match ChunkBuilder::capped_value(VarKind::Text, raw, cap) {
        Value::Text(text) => Value::Opaque(Arc::new(OpaqueVal {
            type_name: type_name.clone(),
            text,
        })),
        other => other,
    }
}
