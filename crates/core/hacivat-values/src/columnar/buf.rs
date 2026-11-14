//! Column buffers (Task 6 adds one per [`super::BufKind`]; Task 1b has `Values`).

use crate::columnar::CellFlags;
use crate::value::{Value, ValueRef};

/// One column of a chunk.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ColumnBuf {
    /// Whole values (`BufKind::Values`).
    Values(Vec<Value>),
}

impl ColumnBuf {
    pub(crate) fn cell(&self, row: usize) -> ValueRef<'_> {
        match self {
            ColumnBuf::Values(v) => v[row].view(),
        }
    }

    pub(crate) fn flags(&self, row: usize) -> CellFlags {
        match self {
            ColumnBuf::Values(v) => flags_of(&v[row]),
        }
    }

    /// The buffer's bytes: one `Value` per cell plus each value's heap payload.
    pub(crate) fn bytes(&self) -> u64 {
        match self {
            ColumnBuf::Values(v) => v
                .iter()
                .map(|value| (std::mem::size_of::<Value>() + payload(value)) as u64)
                .sum(),
        }
    }
}

/// The flags a whole value implies.
pub(crate) fn flags_of(v: &Value) -> CellFlags {
    match v {
        Value::Truncated(_) => CellFlags::TRUNCATED,
        Value::Lossy(_) => CellFlags::LOSSY,
        Value::Error(_) => CellFlags::ERROR,
        Value::Special(_) => CellFlags::SPECIAL,
        _ => CellFlags::empty(),
    }
}

/// The heap bytes a value points at (its text or bytes; the boxed variants' contents).
fn payload(v: &Value) -> usize {
    match v {
        Value::Text(s) | Value::Json(s) | Value::Xml(s) => s.len(),
        Value::Bytes(b) | Value::Lossy(b) => b.len(),
        Value::Opaque(o) => o.type_name.len() + o.text.len(),
        Value::Error(e) => e.code.len() + e.message.len(),
        Value::Truncated(t) => std::mem::size_of::<Value>() + payload(&t.prefix),
        _ => 0,
    }
}
