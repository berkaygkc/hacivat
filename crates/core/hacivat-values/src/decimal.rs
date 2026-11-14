//! Exact decimals (HV-RES-01; Task 6). Never through a binary float and never rounded.
//!
//! The mantissa is held as two `u64` words and computed with 32-bit limbs, so no 128-bit
//! integer type is involved anywhere on this path (`tests/decimal.rs` greps this file).

use std::fmt;
use std::sync::Arc;

/// An exact decimal: a 128-bit mantissa with a scale, or the canonical text when it has more
/// than 38 digits. Equality is structural, so the server's scale is kept: `1.50` and `1.5`
/// are different values, and so are `-0.00` and `0.00`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DecimalVal(pub(crate) Repr);

/// How a [`DecimalVal`] is held.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Repr {
    /// Up to 38 significant digits, sign and magnitude. `mant[0]` is the low word and
    /// `mant[1]` the high word of the magnitude, which stays below 10^38 and so never reaches
    /// bit 127; bit 63 of `mant[1]` is the sign (set: negative, which keeps `-0.00`). `scale`
    /// is the number of digits after the point (never negative from [`DecimalVal::parse`] or
    /// [`DecimalVal::from_base10000`]; a negative scale means trailing zeros before the point).
    Small { mant: [u64; 2], scale: i16 },
    /// More than 38 significant digits: the canonical text (see [`DecimalVal::write_canonical`]).
    Big(Arc<str>),
}

/// Why a decimal's text (or wire form) did not convert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecimalError {
    /// The text is empty.
    Empty,
    /// Not `[+-]digits[.digits]`, or a base-10000 digit is 10,000 or more, or a nonzero digit
    /// lies beyond the display scale (it would be dropped), or the scale is above
    /// [`MAX_SCALE`].
    Invalid,
    /// More than [`MAX_INT_DIGITS`] digits before the point or [`MAX_SCALE`] after it.
    TooLong,
}

/// The most digits before the point (PostgreSQL's `numeric` limit, the widest of the engines).
pub const MAX_INT_DIGITS: usize = 131_072;
/// The most digits after the point (PostgreSQL's `numeric` display-scale limit).
pub const MAX_SCALE: u16 = 16_383;

/// Significant digits a [`Repr::Small`] holds.
const SMALL_DIGITS: usize = 38;
/// The sign bit in `mant[1]`.
const SIGN: u64 = 1 << 63;
/// Digits laid out on the stack by [`DecimalVal::try_from_base10000`] before it allocates.
const STACK_DIGITS: usize = 128;

/// The magnitude as four 32-bit limbs, least significant first.
type Limbs = [u32; 4];

impl DecimalVal {
    /// Parses `[+-]digits[.digits]` exactly: `1.50` keeps its scale of 2, `-0.00` its sign.
    /// Leading zeros and a `+` are dropped (`007.50` is `7.50`); `.5` and `5.` are accepted
    /// (as `0.5` and `5`). No exponent, no whitespace, no `NaN` or `Infinity` (those are
    /// [`crate::SpecialVal`]s, decided by the driver). More than 38 significant digits are held
    /// as [`Repr::Big`].
    pub fn parse(text: &str) -> Result<Self, DecimalError> {
        if text.is_empty() {
            return Err(DecimalError::Empty);
        }
        let bytes = text.as_bytes();
        let (neg, body) = match bytes[0] {
            b'-' => (true, &bytes[1..]),
            b'+' => (false, &bytes[1..]),
            _ => (false, bytes),
        };
        let (int, frac) = match body.iter().position(|&b| b == b'.') {
            Some(dot) => (&body[..dot], &body[dot + 1..]),
            None => (body, &body[body.len()..]),
        };
        if int.is_empty() && frac.is_empty() {
            return Err(DecimalError::Invalid);
        }
        if !int.iter().chain(frac).all(u8::is_ascii_digit) {
            return Err(DecimalError::Invalid);
        }
        Self::from_digits(neg, int, frac)
    }

    /// PostgreSQL's binary `numeric`: `digits` in base 10,000, most significant first, the
    /// first worth 10,000^`weight`; `dscale` digits after the point are shown, zero-padded
    /// (`dscale` beyond the digits) as PostgreSQL prints them. `NaN` and `±Infinity` have their
    /// own sign words and are [`crate::SpecialVal`]s, decided by the decoder.
    ///
    /// # Panics
    /// On input PostgreSQL never sends (see [`DecimalVal::try_from_base10000`], which a decoder
    /// of untrusted bytes uses instead).
    pub fn from_base10000(neg: bool, weight: i16, dscale: u16, digits: &[u16]) -> Self {
        Self::try_from_base10000(neg, weight, dscale, digits).unwrap_or_else(|e| {
            panic!(
                "invalid base-10000 numeric ({e:?}): weight {weight}, dscale {dscale}, {} digits",
                digits.len()
            )
        })
    }

    /// [`DecimalVal::from_base10000`] without the panic: `Invalid` when a digit is 10,000 or
    /// more, `dscale` is above [`MAX_SCALE`], or a nonzero digit lies beyond `dscale` (it would
    /// be dropped). The digits are laid out on the stack when they fit 128 places, so a value
    /// of up to 38 digits (every `numeric(p ≤ 38, s)`) is built without allocating.
    pub fn try_from_base10000(
        neg: bool,
        weight: i16,
        dscale: u16,
        digits: &[u16],
    ) -> Result<Self, DecimalError> {
        if dscale > MAX_SCALE || digits.iter().any(|&g| g >= 10_000) {
            return Err(DecimalError::Invalid);
        }
        let int_len = if weight >= 0 {
            (weight as usize + 1) * 4
        } else {
            0
        };
        let frac_len = dscale as usize;
        let total = int_len + frac_len;
        let mut stack = [b'0'; STACK_DIGITS];
        let mut heap = Vec::new();
        let buf: &mut [u8] = if total <= STACK_DIGITS {
            &mut stack[..total]
        } else {
            heap.resize(total, b'0');
            &mut heap
        };
        for (i, &group) in digits.iter().enumerate() {
            let quad = [
                b'0' + (group / 1_000) as u8,
                b'0' + (group / 100 % 10) as u8,
                b'0' + (group / 10 % 10) as u8,
                b'0' + (group % 10) as u8,
            ];
            let exponent = i64::from(weight) - i as i64;
            if exponent >= 0 {
                // Integer group `i` (i ≤ weight) covers places i·4 .. i·4 + 4.
                buf[i * 4..i * 4 + 4].copy_from_slice(&quad);
            } else {
                // Fraction group j = −exponent covers places (j − 1)·4 .. j·4 after the point.
                let start = ((-exponent - 1) * 4) as usize;
                for (t, &d) in quad.iter().enumerate() {
                    let place = start + t;
                    if place < frac_len {
                        buf[int_len + place] = d;
                    } else if d != b'0' {
                        return Err(DecimalError::Invalid);
                    }
                }
            }
        }
        let (int, frac) = buf.split_at(int_len);
        Self::from_digits(neg, int, frac)
    }

    /// `mant` × 10^−`scale`, exactly (a `Dec64` buffer cell, an integer with a scale).
    pub fn from_mantissa(mant: i64, scale: i16) -> Self {
        let mag = mant.unsigned_abs();
        let sign = if mant < 0 { SIGN } else { 0 };
        DecimalVal(Repr::Small {
            mant: [mag, sign],
            scale,
        })
    }

    /// Digits after the decimal point.
    pub fn scale(&self) -> i16 {
        match &self.0 {
            Repr::Small { scale, .. } => *scale,
            // The canonical text has at most `MAX_SCALE` digits after its point.
            Repr::Big(text) => text
                .find('.')
                .map_or(0, |dot| (text.len() - dot - 1) as i16),
        }
    }

    /// Whether the sign is negative (`-0.00` included).
    pub fn is_negative(&self) -> bool {
        match &self.0 {
            Repr::Small { mant, .. } => mant[1] & SIGN != 0,
            Repr::Big(text) => text.starts_with('-'),
        }
    }

    /// Appends the canonical text, exactly: an optional `-`, the integer digits without
    /// leading zeros (`0` when there are none), then `.` and exactly `scale` digits when the
    /// scale is positive (`1.50`, `-0.00`, `0.000001`, `12345`).
    pub fn write_canonical(&self, out: &mut String) {
        // Writing into a `String` cannot fail.
        let _ = self.write_to(out);
    }

    /// How the value is held.
    pub fn repr(&self) -> &Repr {
        &self.0
    }

    /// Whether the value has more than 38 digits (held as text).
    pub fn is_big(&self) -> bool {
        matches!(self.0, Repr::Big(_))
    }

    /// The value of `int` and `frac`, ASCII digits already checked: [`Repr::Small`] up to 38
    /// significant digits, else [`Repr::Big`] with the canonical text.
    fn from_digits(neg: bool, int: &[u8], frac: &[u8]) -> Result<Self, DecimalError> {
        let int = &int[int.iter().take_while(|&&b| b == b'0').count()..];
        if int.len() > MAX_INT_DIGITS || frac.len() > MAX_SCALE as usize {
            return Err(DecimalError::TooLong);
        }
        let significant = if int.is_empty() {
            frac.len() - frac.iter().take_while(|&&b| b == b'0').count()
        } else {
            int.len() + frac.len()
        };
        if significant <= SMALL_DIGITS {
            let mut limbs: Limbs = [0; 4];
            for &d in int.iter().chain(frac) {
                // Never overflows: at most 38 significant digits, below 10^38 < 2^127.
                mul_add(&mut limbs, 10, u32::from(d - b'0'));
            }
            let [lo, hi] = pack(&limbs);
            let sign = if neg { SIGN } else { 0 };
            return Ok(DecimalVal(Repr::Small {
                mant: [lo, hi | sign],
                scale: frac.len() as i16,
            }));
        }
        let mut text = String::with_capacity(int.len() + frac.len() + 3);
        if neg {
            text.push('-');
        }
        text.push_str(if int.is_empty() { "0" } else { ascii(int) });
        if !frac.is_empty() {
            text.push('.');
            text.push_str(ascii(frac));
        }
        Ok(DecimalVal(Repr::Big(text.into())))
    }

    fn write_to<W: fmt::Write>(&self, out: &mut W) -> fmt::Result {
        let (mant, scale) = match &self.0 {
            Repr::Big(text) => return out.write_str(text),
            Repr::Small { mant, scale } => (*mant, *scale),
        };
        if mant[1] & SIGN != 0 {
            out.write_char('-')?;
        }
        let mut limbs = unpack([mant[0], mant[1] & !SIGN]);
        // The magnitude's decimal digits, filled from the right, nine at a time.
        let mut buf = [b'0'; 45];
        let mut start = buf.len();
        while limbs != [0; 4] {
            let mut chunk = div_rem(&mut limbs, 1_000_000_000);
            let low = start.saturating_sub(9);
            for slot in buf[low..start].iter_mut().rev() {
                *slot = b'0' + (chunk % 10) as u8;
                chunk /= 10;
            }
            start = low;
        }
        while start < buf.len() && buf[start] == b'0' {
            start += 1;
        }
        let digits = &buf[start..];
        if scale <= 0 {
            if digits.is_empty() {
                return out.write_char('0');
            }
            out.write_str(ascii(digits))?;
            for _ in 0..scale.unsigned_abs() {
                out.write_char('0')?;
            }
            return Ok(());
        }
        let scale = scale as usize;
        if digits.len() > scale {
            let (int, frac) = digits.split_at(digits.len() - scale);
            out.write_str(ascii(int))?;
            out.write_char('.')?;
            out.write_str(ascii(frac))
        } else {
            out.write_str("0.")?;
            for _ in digits.len()..scale {
                out.write_char('0')?;
            }
            out.write_str(ascii(digits))
        }
    }
}

impl fmt::Display for DecimalVal {
    /// The canonical text ([`DecimalVal::write_canonical`]).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_to(f)
    }
}

/// ASCII digits as text (checked by the caller; never fails).
fn ascii(digits: &[u8]) -> &str {
    std::str::from_utf8(digits).expect("ASCII digits")
}

/// `limbs` × `m` + `add`; the caller guarantees the result fits 128 bits.
fn mul_add(limbs: &mut Limbs, m: u32, add: u32) {
    let mut carry = u64::from(add);
    for limb in limbs.iter_mut() {
        let t = u64::from(*limb) * u64::from(m) + carry;
        *limb = t as u32;
        carry = t >> 32;
    }
    debug_assert_eq!(carry, 0, "decimal mantissa overflow");
}

/// Divides `limbs` by `d` in place and returns the remainder.
fn div_rem(limbs: &mut Limbs, d: u32) -> u32 {
    let d = u64::from(d);
    let mut rem = 0u64;
    for limb in limbs.iter_mut().rev() {
        let t = (rem << 32) | u64::from(*limb);
        *limb = (t / d) as u32;
        rem = t % d;
    }
    rem as u32
}

fn pack(l: &Limbs) -> [u64; 2] {
    [
        u64::from(l[0]) | u64::from(l[1]) << 32,
        u64::from(l[2]) | u64::from(l[3]) << 32,
    ]
}

fn unpack(w: [u64; 2]) -> Limbs {
    [
        w[0] as u32,
        (w[0] >> 32) as u32,
        w[1] as u32,
        (w[1] >> 32) as u32,
    ]
}
