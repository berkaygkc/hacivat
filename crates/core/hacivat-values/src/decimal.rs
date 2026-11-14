//! Exact decimals (HV-RES-01; Task 6). Never through a binary float.

use std::sync::Arc;

/// An exact decimal: a 128-bit mantissa with a scale, or the canonical text when it has more
/// than 38 digits.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DecimalVal(pub(crate) Repr);

/// How a [`DecimalVal`] is held.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Repr {
    /// Up to 38 digits: the 128-bit mantissa as two `u64` words (Task 6 fixes the word order
    /// and how the sign is held) and `scale`, the digits after the point.
    Small { mant: [u64; 2], scale: i16 },
    /// More than 38 digits: the canonical text.
    Big(Arc<str>),
}

/// Why a decimal's text did not parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecimalError {
    Empty,
    Invalid,
    TooLong,
}

impl DecimalVal {
    /// Parses decimal text exactly (Task 6). Task 1b stub: always `Invalid`.
    pub fn parse(text: &str) -> Result<Self, DecimalError> {
        let _ = text;
        Err(DecimalError::Invalid)
    }

    /// PostgreSQL's base-10000 `numeric` wire form (Task 6).
    pub fn from_base10000(neg: bool, weight: i16, dscale: u16, digits: &[u16]) -> Self {
        let _ = (neg, weight, dscale, digits);
        unimplemented!("DecimalVal::from_base10000: Task 6")
    }

    /// Digits after the decimal point (Task 6).
    pub fn scale(&self) -> i16 {
        unimplemented!("DecimalVal::scale: Task 6")
    }

    /// Appends the canonical text (`1.50`, `-0.00`), exactly (Task 6).
    pub fn write_canonical(&self, out: &mut String) {
        let _ = out;
        unimplemented!("DecimalVal::write_canonical: Task 6")
    }

    /// How the value is held.
    pub fn repr(&self) -> &Repr {
        &self.0
    }

    /// Whether the value has more than 38 digits (held as text).
    pub fn is_big(&self) -> bool {
        matches!(self.0, Repr::Big(_))
    }
}
