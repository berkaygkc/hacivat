//! M3a Task 6: temporal values (HV-RES-01): PostgreSQL's 2000 epoch shifted exactly to 1970,
//! proleptic Gregorian with astronomical years for BC, `24:00:00` kept, offsets east-positive.

use hacivat_values::temporal::{
    MICROS_PER_DAY, PG_EPOCH_DAYS, PG_EPOCH_SECS, timetz_from_pg, try_timetz_from_pg,
};
use hacivat_values::{DateVal, TimeVal, TsVal};
use proptest::prelude::*;

#[test]
fn pg_epoch_shift_and_bc_dates() {
    assert_eq!(DateVal::from_pg_days(0), DateVal(10_957));
    // 4714 BC = year −4713.
    assert_eq!(
        DateVal::from_pg_days(-2_451_546).to_civil(),
        (-4713, 11, 23)
    );
}

#[test]
fn time_24_00_is_kept() {
    assert_eq!(
        TimeVal::from_micros(86_400_000_000).nanos_of_day,
        86_400_000_000_000
    );
    assert_eq!(TimeVal::from_micros(MICROS_PER_DAY), TimeVal::END_OF_DAY);
    assert_eq!(TimeVal::END_OF_DAY.exact_micros(), Some(MICROS_PER_DAY));
}

#[test]
fn timetz_offset_is_east_positive() {
    assert_eq!(timetz_from_pg(0, -10_800).1, 10_800);
    let (t, east) = timetz_from_pg(3_600_000_000, 18_000);
    assert_eq!((t.nanos_of_day, east), (3_600_000_000_000, -18_000));
}

#[test]
fn known_civil_dates() {
    for (days, civil) in [
        (0, (1970, 1, 1)),
        (PG_EPOCH_DAYS, (2000, 1, 1)),
        (-1, (1969, 12, 31)),
        (11_016, (2000, 2, 29)),
        (-719_528, (0, 1, 1)),
        (-719_529, (-1, 12, 31)),
        // PostgreSQL's first date, 4714-11-24 BC (Julian day 0).
        (-2_440_588, (-4713, 11, 24)),
    ] {
        assert_eq!(DateVal(days).to_civil(), civil, "{days}");
        assert_eq!(
            DateVal::from_civil(civil.0, civil.1, civil.2),
            Some(DateVal(days))
        );
    }
}

#[test]
fn from_civil_refuses_days_that_do_not_exist() {
    assert_eq!(DateVal::from_civil(1900, 2, 29), None);
    assert_eq!(DateVal::from_civil(2023, 2, 29), None);
    assert!(DateVal::from_civil(2000, 2, 29).is_some());
    assert!(DateVal::from_civil(1600, 2, 29).is_some());
    assert_eq!(DateVal::from_civil(2026, 13, 1), None);
    assert_eq!(DateVal::from_civil(2026, 0, 1), None);
    assert_eq!(DateVal::from_civil(2026, 4, 31), None);
    assert_eq!(DateVal::from_civil(2026, 4, 0), None);
    // Outside `i32` days.
    assert_eq!(DateVal::from_civil(i32::MAX, 1, 1), None);
    assert_eq!(DateVal::from_civil(i32::MIN, 1, 1), None);
}

#[test]
fn pg_last_date_and_the_infinity_sentinels() {
    // 5874897-12-31 AD is PostgreSQL's last date.
    let last = DateVal::try_from_pg_days(2_145_031_948).unwrap();
    assert_eq!(last.to_civil(), (5_874_897, 12, 31));
    assert_eq!(DateVal::try_from_pg_days(i32::MAX), None);
    assert_eq!(DateVal::try_from_pg_days(i32::MIN), None);
    assert_eq!(
        DateVal::try_from_pg_days(i32::MAX - 1),
        None,
        "shift overflows"
    );
}

#[test]
#[should_panic(expected = "is ±infinity or outside the DateVal range")]
fn from_pg_days_panics_on_infinity() {
    DateVal::from_pg_days(i32::MAX);
}

#[test]
fn pg_timestamps_shift_by_the_2000_epoch() {
    assert_eq!(
        TsVal::from_pg_micros(0),
        TsVal {
            secs: PG_EPOCH_SECS,
            nanos: 0
        }
    );
    // Before 2000 the seconds floor and the nanoseconds count forward.
    assert_eq!(
        TsVal::from_pg_micros(-1),
        TsVal {
            secs: PG_EPOCH_SECS - 1,
            nanos: 999_999_000
        }
    );
    assert_eq!(
        TsVal::from_pg_micros(1_500_001),
        TsVal {
            secs: PG_EPOCH_SECS + 1,
            nanos: 500_001_000
        }
    );
}

#[test]
fn pg_last_timestamp_is_exact_in_year_294276() {
    // 294276-12-31 23:59:59.999999, one microsecond before PostgreSQL's END_TIMESTAMP.
    let ts = TsVal::from_pg_micros(9_223_371_331_199_999_999);
    let days = ts.secs.div_euclid(86_400);
    assert_eq!(DateVal(days as i32).to_civil(), (294_276, 12, 31));
    assert_eq!(ts.secs.rem_euclid(86_400), 86_399);
    assert_eq!(ts.nanos, 999_999_000);
    // Past i64 microseconds since 1970: no Unix-micros form, the value stays exact.
    assert_eq!(ts.to_unix_micros(), None);
    assert_eq!(TsVal::try_from_pg_micros(i64::MAX), None);
    assert_eq!(TsVal::try_from_pg_micros(i64::MIN), None);
}

#[test]
fn unix_micros_round_trip_and_refuse_sub_micro() {
    for us in [0, 1, -1, i64::MIN, i64::MAX, -86_400_000_001] {
        assert_eq!(
            TsVal::from_unix_micros(us).to_unix_micros(),
            Some(us),
            "{us}"
        );
    }
    assert_eq!(TsVal { secs: 0, nanos: 1 }.to_unix_micros(), None);
    assert_eq!(
        TsVal {
            secs: 0,
            nanos: 1_000_000_000
        }
        .to_unix_micros(),
        None
    );
}

#[test]
fn times_outside_the_day_are_refused() {
    assert_eq!(TimeVal::try_from_micros(-1), None);
    assert_eq!(TimeVal::try_from_micros(MICROS_PER_DAY + 1), None);
    assert_eq!(try_timetz_from_pg(0, i32::MIN), None);
    assert_eq!(try_timetz_from_pg(-5, 0), None);
    assert_eq!(
        TimeVal {
            nanos_of_day: 1_500
        }
        .exact_micros(),
        None
    );
}

#[test]
#[should_panic(expected = "is not a time of day")]
fn from_micros_panics_past_the_end_of_day() {
    TimeVal::from_micros(MICROS_PER_DAY + 1);
}

proptest! {
    #[test]
    fn civil_round_trips_over_the_whole_range(days in any::<i32>()) {
        let (y, m, d) = DateVal(days).to_civil();
        prop_assert!((1..=12).contains(&m) && (1..=31).contains(&d));
        prop_assert_eq!(DateVal::from_civil(y, m, d), Some(DateVal(days)));
    }

    #[test]
    fn consecutive_days_are_consecutive_dates(days in -10_000_000i32..10_000_000) {
        let (y0, m0, d0) = DateVal(days).to_civil();
        let (y1, m1, d1) = DateVal(days + 1).to_civil();
        let next_day = (y1, m1, d1) == (y0, m0, d0 + 1);
        let next_month = d1 == 1 && ((y1, m1) == (y0, m0 + 1) || (y1, m1, m0) == (y0 + 1, 1, 12));
        prop_assert!(next_day || next_month, "{days}: {:?} then {:?}", (y0, m0, d0), (y1, m1, d1));
    }

    #[test]
    fn pg_micros_agree_with_unix_micros(us in -9_000_000_000_000_000_000i64..9_000_000_000_000_000_000) {
        let ts = TsVal::from_pg_micros(us);
        prop_assert_eq!(ts.nanos % 1_000, 0);
        prop_assert!(ts.nanos < 1_000_000_000);
        if let Some(unix) = us.checked_add(PG_EPOCH_SECS * 1_000_000) {
            prop_assert_eq!(ts.to_unix_micros(), Some(unix));
        }
    }
}
