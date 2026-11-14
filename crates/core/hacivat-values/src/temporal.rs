//! Dates, times and timestamps (HV-RES-01; Task 6): proleptic Gregorian, astronomical years
//! for BC (1 BC is year 0, 4714 BC is year −4713), `24:00:00` kept, offsets east-positive.
//!
//! These types count from 1970-01-01; PostgreSQL counts from 2000-01-01, and the `from_pg_*`
//! constructors shift exactly. PostgreSQL's `±infinity` sentinels (`i32::MAX`/`i32::MIN` days,
//! `i64::MAX`/`i64::MIN` microseconds) are not instants: the decoder maps them to
//! [`crate::SpecialVal`] first. The `try_*` constructors answer `None` for them and for
//! anything outside the type's range; the plain ones panic, so a decoder of untrusted bytes
//! uses the `try_*` forms.

/// 2000-01-01 (PostgreSQL's epoch) in days since 1970-01-01.
pub const PG_EPOCH_DAYS: i32 = 10_957;
/// 2000-01-01 (PostgreSQL's epoch) in seconds since 1970-01-01.
pub const PG_EPOCH_SECS: i64 = 946_684_800;
/// Microseconds in a day: `24:00:00` as microseconds of the day.
pub const MICROS_PER_DAY: i64 = 86_400_000_000;

const MICROS_PER_SEC: i64 = 1_000_000;
const NANOS_PER_MICRO: u64 = 1_000;

/// Days since 1970-01-01, proleptic Gregorian.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DateVal(pub i32);

/// A time of day in nanoseconds; `86_400_000_000_000` is `24:00:00`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TimeVal {
    pub nanos_of_day: u64,
}

/// An instant (or a naive timestamp): seconds since the Unix epoch and nanoseconds
/// (`0..1_000_000_000`, counted forward from `secs`, so `secs` is the floor for times before
/// 1970).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TsVal {
    pub secs: i64,
    pub nanos: u32,
}

impl DateVal {
    /// PostgreSQL's binary `date` (days since 2000-01-01).
    ///
    /// # Panics
    /// On the `±infinity` sentinels and on a day outside `DateVal`'s range (none of
    /// PostgreSQL's finite dates); see [`DateVal::try_from_pg_days`].
    pub fn from_pg_days(days: i32) -> Self {
        Self::try_from_pg_days(days).unwrap_or_else(|| {
            panic!("PostgreSQL date {days} is ±infinity or outside the DateVal range")
        })
    }

    /// [`DateVal::from_pg_days`] without the panic: `None` for `i32::MAX`, `i32::MIN` and a
    /// day whose shift leaves `i32`.
    pub fn try_from_pg_days(days: i32) -> Option<Self> {
        if days == i32::MAX || days == i32::MIN {
            return None;
        }
        days.checked_add(PG_EPOCH_DAYS).map(DateVal)
    }

    /// The date of `year` (astronomical: 0 is 1 BC), `month` (1–12) and `day`; `None` when the
    /// day does not exist (`1900-02-29`) or lies outside `DateVal`'s range.
    pub fn from_civil(year: i32, month: u8, day: u8) -> Option<Self> {
        if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
            return None;
        }
        i32::try_from(days_from_civil(i64::from(year), month, day))
            .ok()
            .map(DateVal)
    }

    /// The (astronomical year, month 1–12, day 1–31) of this date.
    pub fn to_civil(self) -> (i32, u8, u8) {
        let (y, m, d) = civil_from_days(i64::from(self.0));
        // |days| < 2^31 keeps |year| below 5.9 million.
        (y as i32, m, d)
    }
}

impl TimeVal {
    /// `24:00:00`, the end of the day (PostgreSQL's `time` and `timetz` allow it).
    pub const END_OF_DAY: TimeVal = TimeVal {
        nanos_of_day: MICROS_PER_DAY as u64 * NANOS_PER_MICRO,
    };

    /// Microseconds of the day, `0..=86_400_000_000` (`24:00:00` included).
    ///
    /// # Panics
    /// Outside that range; see [`TimeVal::try_from_micros`].
    pub fn from_micros(micros: i64) -> Self {
        Self::try_from_micros(micros)
            .unwrap_or_else(|| panic!("{micros} µs is not a time of day (0 ..= 24:00:00)"))
    }

    /// [`TimeVal::from_micros`] without the panic.
    pub fn try_from_micros(micros: i64) -> Option<Self> {
        (0..=MICROS_PER_DAY).contains(&micros).then(|| TimeVal {
            nanos_of_day: micros as u64 * NANOS_PER_MICRO,
        })
    }

    /// The microseconds of the day, when the time is a whole number of them and at most
    /// `24:00:00`.
    pub fn exact_micros(self) -> Option<i64> {
        (self.nanos_of_day.is_multiple_of(NANOS_PER_MICRO) && self <= Self::END_OF_DAY)
            .then_some((self.nanos_of_day / NANOS_PER_MICRO) as i64)
    }
}

impl TsVal {
    /// PostgreSQL's binary `timestamp`/`timestamptz` (microseconds since 2000-01-01), with
    /// every finite PostgreSQL timestamp (up to 294276 AD) exact.
    ///
    /// # Panics
    /// On the `±infinity` sentinels; see [`TsVal::try_from_pg_micros`].
    pub fn from_pg_micros(micros: i64) -> Self {
        Self::try_from_pg_micros(micros)
            .unwrap_or_else(|| panic!("PostgreSQL timestamp {micros} is ±infinity"))
    }

    /// [`TsVal::from_pg_micros`] without the panic: `None` for `i64::MAX` and `i64::MIN`.
    pub fn try_from_pg_micros(micros: i64) -> Option<Self> {
        if micros == i64::MAX || micros == i64::MIN {
            return None;
        }
        let mut ts = Self::from_unix_micros(micros);
        // |micros| / 10^6 < 9.3·10^12, so the shift cannot overflow.
        ts.secs += PG_EPOCH_SECS;
        Some(ts)
    }

    /// Microseconds since 1970-01-01 (every `i64` is a value).
    pub fn from_unix_micros(micros: i64) -> Self {
        TsVal {
            secs: micros.div_euclid(MICROS_PER_SEC),
            nanos: (micros.rem_euclid(MICROS_PER_SEC) as u64 * NANOS_PER_MICRO) as u32,
        }
    }

    /// Microseconds since 1970-01-01: `None` when the instant is not a whole number of them, its
    /// `nanos` is out of range, or it lies beyond `i64` microseconds (after about 292277 AD).
    pub fn to_unix_micros(self) -> Option<i64> {
        if self.nanos >= 1_000_000_000 || u64::from(self.nanos) % NANOS_PER_MICRO != 0 {
            return None;
        }
        // Widened, because `i64::MIN` microseconds floor to seconds whose product leaves `i64`.
        let micros = i128::from(self.secs) * i128::from(MICROS_PER_SEC)
            + i128::from(u64::from(self.nanos) / NANOS_PER_MICRO);
        i64::try_from(micros).ok()
    }
}

/// PostgreSQL's binary `timetz`: microseconds of the day and the zone in seconds **west** of
/// UTC, as PostgreSQL stores it; returns the time and the offset in seconds **east** of UTC
/// (`+03` arrives as `-10_800` and is kept as `10_800`).
///
/// # Panics
/// When the time is outside `0 ..= 24:00:00` or the zone is `i32::MIN`; see
/// [`try_timetz_from_pg`].
pub fn timetz_from_pg(micros: i64, zone_west_secs: i32) -> (TimeVal, i32) {
    try_timetz_from_pg(micros, zone_west_secs).unwrap_or_else(|| {
        panic!("PostgreSQL timetz ({micros} µs, zone {zone_west_secs}) is out of range")
    })
}

/// [`timetz_from_pg`] without the panic.
pub fn try_timetz_from_pg(micros: i64, zone_west_secs: i32) -> Option<(TimeVal, i32)> {
    Some((
        TimeVal::try_from_micros(micros)?,
        zone_west_secs.checked_neg()?,
    ))
}

fn is_leap(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 of a valid proleptic Gregorian date (H. Hinnant's algorithm, public
/// domain), in `i64` so no `i32` year overflows.
fn days_from_civil(year: i64, month: u8, day: u8) -> i64 {
    let (m, d) = (i64::from(month), i64::from(day));
    let y = if m <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The inverse of [`days_from_civil`].
fn civil_from_days(days: i64) -> (i64, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let y = yoe + era * 400;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
