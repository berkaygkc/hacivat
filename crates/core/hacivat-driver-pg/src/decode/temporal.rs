//! `date`, `time`, `timetz`, `timestamp`, `timestamptz` and `interval`, with `±infinity`
//! (Task 12).
//!
//! PostgreSQL counts from 2000-01-01; the buffers from 1970-01-01, and the `try_*` constructors
//! of `hacivat_values` shift exactly. The `±infinity` sentinels (`i32::MAX`/`i32::MIN` days,
//! `i64::MAX`/`i64::MIN` microseconds) become `SpecialVal` first, and so do PostgreSQL 17's
//! infinite `interval`s (every field at its maximum, or every field at its minimum). A
//! timestamp past `i64` microseconds since 1970 (about 292277 AD; PostgreSQL goes to 294276 AD)
//! is kept exactly as a whole value, an exception of its column. A time of day outside `0 ..= 24:00:00` is an
//! `Error` cell (the chunk builder's range check, or here for `timetz`).

use hacivat_values::{DateVal, SpecialVal, TsVal, Value, temporal::try_timetz_from_pg};

use super::{Decoded, bad_length, error};

fn special(pos: bool) -> Decoded<'static> {
    Decoded::Value(Value::Special(if pos {
        SpecialVal::PosInfinity
    } else {
        SpecialVal::NegInfinity
    }))
}

fn i64_of(raw: &[u8], what: &str) -> Result<i64, Decoded<'static>> {
    raw.try_into()
        .map(i64::from_be_bytes)
        .map_err(|_| bad_length(what, raw.len(), 8))
}

/// `date`: days since 2000-01-01.
pub fn date(raw: &[u8]) -> Decoded<'_> {
    let days = match <[u8; 4]>::try_from(raw) {
        Ok(b) => i32::from_be_bytes(b),
        Err(_) => return bad_length("date", raw.len(), 4),
    };
    match days {
        i32::MAX => special(true),
        i32::MIN => special(false),
        _ => match DateVal::try_from_pg_days(days) {
            Some(d) => Decoded::Temporal(d.0.into()),
            None => error(
                "pg-out-of-range",
                format!("the date is outside the supported range (raw value {days})"),
            ),
        },
    }
}

/// `time`: microseconds of the day (the chunk builder refuses one outside `0 ..= 24:00:00`).
pub fn time(raw: &[u8]) -> Decoded<'_> {
    i64_of(raw, "time").map_or_else(|e| e, Decoded::Temporal)
}

/// `timetz`: microseconds of the day and the zone in seconds **west** of UTC; kept with the
/// offset east-positive.
pub fn timetz(raw: &[u8]) -> Decoded<'_> {
    let Ok(b) = <[u8; 12]>::try_from(raw) else {
        return bad_length("timetz", raw.len(), 12);
    };
    let micros = i64::from_be_bytes(b[..8].try_into().unwrap_or_default());
    let zone = i32::from_be_bytes(b[8..].try_into().unwrap_or_default());
    match try_timetz_from_pg(micros, zone) {
        Some((_, east)) => Decoded::Temporal2(micros, east),
        None => error(
            "pg-out-of-range",
            format!("the timetz is outside the supported range (raw value {micros}, zone {zone})"),
        ),
    }
}

/// `timestamp` and `timestamptz`: microseconds since 2000-01-01 (UTC for `timestamptz`).
pub fn timestamp(raw: &[u8], tz: bool) -> Decoded<'_> {
    let micros = match i64_of(raw, if tz { "timestamptz" } else { "timestamp" }) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let Some(ts) = TsVal::try_from_pg_micros(micros) else {
        // Only the sentinels.
        return special(micros == i64::MAX);
    };
    match ts.to_unix_micros() {
        Some(unix) => Decoded::Temporal(unix),
        None if tz => Decoded::Value(Value::TimestampTz {
            utc: ts,
            offset: None,
        }),
        None => Decoded::Value(Value::Timestamp(ts)),
    }
}

/// `interval`: microseconds (i64), days (i32), months (i32), kept as they are. PostgreSQL 17's
/// `±infinity` (all three fields at their maximum, or all at their minimum: `INTERVAL_NOEND`,
/// `INTERVAL_NOBEGIN`) is a `SpecialVal` (fix round 1, review I2); any other combination is a
/// finite interval, however large.
pub fn interval(raw: &[u8]) -> Decoded<'_> {
    let Ok(b) = <[u8; 16]>::try_from(raw) else {
        return bad_length("interval", raw.len(), 16);
    };
    let micros = i64::from_be_bytes(b[..8].try_into().unwrap_or_default());
    let days = i32::from_be_bytes(b[8..12].try_into().unwrap_or_default());
    let months = i32::from_be_bytes(b[12..].try_into().unwrap_or_default());
    match (micros, days, months) {
        (i64::MAX, i32::MAX, i32::MAX) => return special(true),
        (i64::MIN, i32::MIN, i32::MIN) => return special(false),
        _ => {}
    }
    Decoded::Value(Value::Interval {
        months,
        days,
        micros,
    })
}
