//! Dates, times and timestamps (HV-RES-01; Task 6): proleptic Gregorian, astronomical years
//! for BC, `24:00:00` kept.

/// Days since 1970-01-01, proleptic Gregorian.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DateVal(pub i32);

/// A time of day in nanoseconds; `86_400_000_000_000` is `24:00:00`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TimeVal {
    pub nanos_of_day: u64,
}

/// An instant (or a naive timestamp): seconds since the Unix epoch and nanoseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TsVal {
    pub secs: i64,
    pub nanos: u32,
}
