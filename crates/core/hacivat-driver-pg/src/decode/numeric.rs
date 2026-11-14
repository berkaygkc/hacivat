//! `numeric` to `DecimalVal`, `NaN` and `±Infinity` to `SpecialVal` (Task 12).
//!
//! The binary form: `ndigits` (i16), `weight` (i16, the first digit is worth
//! 10,000^weight), the sign word (u16), `dscale` (u16, the digits shown after the point), then
//! `ndigits` base-10,000 digits (i16 each). Exact: never through a float, the server's scale
//! kept. More than 38 significant digits are `DecimalVal`'s `Big` (its canonical text); more
//! than [`MAX_NUMERIC_DIGITS`] base-10,000 digits, a length that disagrees with `ndigits`, a
//! digit of 10,000 or more, an unknown sign word, a display scale above 16,383, a nonzero digit
//! beyond the display scale, or a special with digits is an `Error` cell.

use hacivat_values::{DecimalVal, SpecialVal, Value};

use super::{Decoded, MAX_NUMERIC_DIGITS, error};

const POS: u16 = 0x0000;
const NEG: u16 = 0x4000;
const NAN: u16 = 0xC000;
const PINF: u16 = 0xD000;
const NINF: u16 = 0xF000;

fn invalid(what: String) -> Decoded<'static> {
    error("pg-numeric", what)
}

pub fn numeric(raw: &[u8]) -> Decoded<'static> {
    let Some(header) = raw.get(..8) else {
        return invalid(format!(
            "a numeric value of {} bytes (the header is 8)",
            raw.len()
        ));
    };
    let word = |i: usize| u16::from_be_bytes([header[i], header[i + 1]]);
    let ndigits = word(0) as i16;
    let weight = word(2) as i16;
    let sign = word(4);
    let dscale = word(6);
    let Ok(ndigits) = usize::try_from(ndigits) else {
        return invalid(format!("a numeric value with {ndigits} digits"));
    };
    if ndigits > MAX_NUMERIC_DIGITS {
        return error(
            "pg-numeric-digits",
            format!(
                "a numeric value of {ndigits} base-10000 digits (at most {MAX_NUMERIC_DIGITS} are decoded)"
            ),
        );
    }
    let body = &raw[8..];
    if body.len() != ndigits * 2 {
        return invalid(format!(
            "a numeric value of {ndigits} digits in {} bytes",
            body.len()
        ));
    }
    let special = match sign {
        POS | NEG => None,
        NAN => Some(SpecialVal::NaN),
        PINF => Some(SpecialVal::PosInfinity),
        NINF => Some(SpecialVal::NegInfinity),
        _ => return invalid(format!("a numeric value with the sign word {sign:#06x}")),
    };
    if let Some(special) = special {
        if ndigits != 0 {
            return invalid(format!("a special numeric value with {ndigits} digits"));
        }
        return Decoded::Value(Value::Special(special));
    }
    // Laid out on the stack: at most MAX_NUMERIC_DIGITS of them. A negative i16 digit reads as
    // 32,768 or more, which `try_from_base10000` refuses like any digit of 10,000 or more.
    let mut digits = [0u16; MAX_NUMERIC_DIGITS];
    let (pairs, _) = body.as_chunks::<2>();
    for (d, pair) in digits.iter_mut().zip(pairs) {
        *d = u16::from_be_bytes(*pair);
    }
    match DecimalVal::try_from_base10000(sign == NEG, weight, dscale, &digits[..ndigits]) {
        Ok(d) => Decoded::Decimal(d),
        Err(e) => invalid(format!(
            "a numeric value that is not a decimal ({e:?}): weight {weight}, dscale {dscale}, \
             {ndigits} digits"
        )),
    }
}
