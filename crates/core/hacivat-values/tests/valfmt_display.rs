//! M3a Task 16: `format_cell` (HV-GRID-10, L3) and the ID heuristic (R11).
//!
//! Every expected spelling of a temporal value, an interval or a float was read from
//! PostgreSQL 17 (`psql -At`) when the test was written; MySQL's padded fractions follow its
//! documented `fsp` output.

use std::sync::Arc;

use hacivat_proto::DbmsKind;
use hacivat_values::valfmt::{
    BinaryDisplay, ChipKind, ColumnFacts, Display, DisplayKind, DisplayMode, DisplaySettings,
    DisplayZone, Purpose, format_cell, formatter_builds, id_like,
};
use hacivat_values::{
    CellFlags, DateVal, DecimalVal, ErrorVal, LenUnit, LogicalType, OpaqueVal, SpecialVal, TimeVal,
    TruncatedVal, TsVal, Value,
};

// ---- helpers ----------------------------------------------------------------------------

/// A column for one value, as a PostgreSQL result would describe it.
struct Col {
    logical: LogicalType,
    native: &'static str,
    id_like: bool,
    binary: BinaryDisplay,
    engine: DbmsKind,
}

impl Col {
    fn of(v: &Value) -> Col {
        let (logical, native) = match v {
            Value::Bool(_) => (LogicalType::Bool, "bool"),
            Value::Int(_) => (int(64, true), "int8"),
            Value::UInt(_) => (int(64, false), "int8"),
            Value::F32(_) => (LogicalType::Float { bits: 32 }, "float4"),
            Value::F64(_) => (LogicalType::Float { bits: 64 }, "float8"),
            Value::Decimal(_) => (
                LogicalType::Decimal {
                    precision: None,
                    scale: None,
                },
                "numeric",
            ),
            Value::Bytes(_) => (LogicalType::Bytes { max: None }, "bytea"),
            Value::Uuid(_) => (LogicalType::Uuid, "uuid"),
            Value::Date(_) | Value::Special(_) => (LogicalType::Date, "date"),
            Value::Time(_) => (time(6, false), "time"),
            Value::TimeTz(..) => (time(6, true), "timetz"),
            Value::Timestamp(_) => (timestamp(6, false), "timestamp"),
            Value::TimestampTz { .. } => (timestamp(6, true), "timestamptz"),
            Value::Interval { .. } => (LogicalType::Interval, "interval"),
            Value::Json(_) => (LogicalType::Json { binary: true }, "jsonb"),
            Value::Xml(_) => (LogicalType::Xml, "xml"),
            Value::Opaque(o) => (
                LogicalType::Opaque {
                    type_name: o.type_name.clone(),
                },
                "_int4",
            ),
            Value::Truncated(t) => return Col::of(&t.prefix),
            _ => (text(), "text"),
        };
        Col {
            logical,
            native,
            id_like: false,
            binary: BinaryDisplay::Detect,
            engine: DbmsKind::Postgres,
        }
    }

    fn with(logical: LogicalType, engine: DbmsKind) -> Col {
        Col {
            logical,
            native: "x",
            id_like: false,
            binary: BinaryDisplay::Detect,
            engine,
        }
    }

    fn facts(&self) -> ColumnFacts<'_> {
        ColumnFacts {
            logical: &self.logical,
            native_name: self.native,
            id_like: self.id_like,
            binary: self.binary,
            engine: self.engine,
        }
    }
}

fn int(bits: u8, signed: bool) -> LogicalType {
    LogicalType::Int { bits, signed }
}

fn text() -> LogicalType {
    LogicalType::Text {
        max_chars: None,
        fixed: false,
    }
}

fn time(precision: u8, tz: bool) -> LogicalType {
    LogicalType::Time { precision, tz }
}

fn timestamp(precision: u8, tz: bool) -> LogicalType {
    LogicalType::Timestamp { precision, tz }
}

fn settings(locale: &str) -> DisplaySettings {
    DisplaySettings::with_zone(locale, DisplayZone::utc())
}

fn show(v: &Value, col: &Col, s: &DisplaySettings) -> Display {
    format_cell(v.view(), &col.facts(), s, Purpose::Cell)
}

fn fmt(v: &Value, locale: &str) -> String {
    show(v, &Col::of(v), &settings(locale)).text
}

fn fmt_in(v: &Value, col: &Col) -> String {
    show(v, col, &settings("en-US")).text
}

fn fmt_col(v: &Value, name: &str, locale: &str) -> String {
    let mut col = Col::of(v);
    col.id_like = id_like(name, &col.logical, col.native);
    show(v, &col, &settings(locale)).text
}

fn fmt_tz(v: &Value, tz: &str) -> String {
    show(v, &Col::of(v), &DisplaySettings::new("en-US", Some(tz))).text
}

fn display_of(v: &Value) -> Display {
    show(v, &Col::of(v), &settings("en-US"))
}

fn kind(v: &Value) -> DisplayKind {
    display_of(v).kind
}

fn raw(v: &Value) -> String {
    match hacivat_values::valfmt::raw_text(v.view(), &Col::of(v).facts()) {
        hacivat_values::valfmt::RawText::Text(t) => t.into_owned(),
        hacivat_values::valfmt::RawText::Null => "<null>".into(),
    }
}

fn dec(s: &str) -> Value {
    Value::Decimal(DecimalVal::parse(s).expect("a decimal"))
}

fn ts(iso: &str) -> TsVal {
    let t: jiff::Timestamp = iso.parse().expect("an RFC 3339 instant");
    TsVal {
        secs: t.as_second(),
        nanos: t.subsec_nanosecond() as u32,
    }
}

fn ts_utc(iso: &str) -> Value {
    Value::TimestampTz {
        utc: ts(iso),
        offset: None,
    }
}

fn naive(date: (i32, u8, u8), hms: (i64, i64, i64), nanos: u32) -> TsVal {
    let day = DateVal::from_civil(date.0, date.1, date.2).expect("a date");
    TsVal {
        secs: i64::from(day.0) * 86_400 + hms.0 * 3_600 + hms.1 * 60 + hms.2,
        nanos,
    }
}

fn uuid_bytes() -> Arc<[u8]> {
    Arc::from(
        &[
            0x6f, 0x1e, 0x9a, 0x0c, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 0, 0, 1,
        ][..],
    )
}

fn truncated(total: u64) -> Value {
    Value::Truncated(Box::new(TruncatedVal {
        prefix: Value::Text("abc".into()),
        total_len: Some(total),
        unit: LenUnit::Bytes,
    }))
}

fn interval(months: i32, days: i32, micros: i64) -> Value {
    Value::Interval {
        months,
        days,
        micros,
    }
}

// ---- the plan's Step 1 tests -----------------------------------------------------------

#[test]
fn decimal_tr_and_en_and_raw() {
    let v = dec("1234567.50");
    assert_eq!(fmt(&v, "tr-TR"), "1.234.567,50");
    assert_eq!(fmt(&v, "en-US"), "1,234,567.50");
    assert_eq!(raw(&v), "1234567.50");
}

#[test]
fn id_column_is_never_grouped() {
    assert_eq!(
        fmt_col(&Value::Int(1_234_567), "customer_id", "tr-TR"),
        "1234567"
    );
    assert_eq!(
        fmt_col(&Value::Int(1_234_567), "amount", "tr-TR"),
        "1.234.567"
    );
}

#[test]
fn shortest_f32_and_negative_zero() {
    assert_eq!(fmt(&Value::F32(0.1), "en-US"), "0.1");
    assert_eq!(fmt(&Value::F64(-0.0), "en-US"), "-0");
}

#[test]
fn timestamptz_in_istanbul() {
    assert_eq!(
        fmt_tz(&ts_utc("2026-11-11T07:00:00Z"), "Europe/Istanbul"),
        "2026-11-11 10:00:00+03"
    );
}

#[test]
fn sixteen_byte_bytea_is_a_uuid() {
    assert_eq!(
        fmt(&Value::Bytes(uuid_bytes()), "en-US"),
        "6f1e9a0c-0000-4000-8000-000000000001"
    );
}

#[test]
fn infinity_date_is_verbatim() {
    assert_eq!(
        fmt(&Value::Special(SpecialVal::PosInfinity), "tr-TR"),
        "infinity"
    );
}

#[test]
fn json_keeps_number_spelling_and_key_order() {
    assert_eq!(
        fmt(&Value::Json("{\"b\":1.50,\"a\":1e3}".into()), "en-US"),
        "{\"b\":1.50,\"a\":1e3}"
    );
}

#[test]
fn text_null_string_is_not_the_null_marker() {
    assert_eq!(kind(&Value::Text("NULL".into())), DisplayKind::Text);
    assert_eq!(kind(&Value::Null), DisplayKind::Null);
}

#[test]
fn empty_text_is_empty_kind() {
    assert_eq!(kind(&Value::Text("".into())), DisplayKind::Empty);
}

#[test]
fn truncated_value_carries_a_chip_not_a_string() {
    let d = display_of(&truncated(5 << 20));
    assert!(matches!(d.chip, Some(ChipKind::Truncated { total, .. }) if total == 5 << 20));
    assert_eq!(
        d.chip,
        Some(ChipKind::Truncated {
            total: 5 << 20,
            unit: LenUnit::Bytes
        })
    );
    // The text is the formatted prefix alone: no number or unit is joined to it here.
    assert_eq!(d.text, "abc");
    assert_eq!(d.kind, DisplayKind::Text);
    assert_eq!(d.flags, CellFlags::TRUNCATED.bits());
}

// ---- numbers -----------------------------------------------------------------------------

#[test]
fn decimals_stay_exact_with_scale_and_sign() {
    assert_eq!(fmt(&dec("-0.00"), "tr-TR"), "-0,00");
    assert_eq!(fmt(&dec("-0.00"), "en-US"), "-0.00");
    assert_eq!(fmt(&dec("1.50"), "tr-TR"), "1,50");
    assert_eq!(fmt(&dec("1.5"), "tr-TR"), "1,5");
    assert_eq!(fmt(&dec("0.000001"), "tr-TR"), "0,000001");
    assert_eq!(fmt(&dec("-110190.85"), "tr-TR"), "-110.190,85");
    // 38 digits: the widest `Small` decimal, never through a float.
    assert_eq!(
        fmt(&dec("12345678901234567890123456789.123456789"), "tr-TR"),
        "12.345.678.901.234.567.890.123.456.789,123456789"
    );
    // 45 digits: a `Big` decimal, held as text, grouped exactly.
    assert_eq!(
        fmt(
            &dec("123456789012345678901234567890123456789012345"),
            "en-US"
        ),
        "123,456,789,012,345,678,901,234,567,890,123,456,789,012,345"
    );
    let d = display_of(&dec("1.50"));
    assert_eq!((d.kind, d.align_end), (DisplayKind::Number, true));
}

#[test]
fn a_decimal_wider_than_the_formatter_falls_back_to_its_exact_text() {
    // 40,000 digits before the point (PostgreSQL allows 131,072): past the formatter's range.
    let wide = format!("1{}", "0".repeat(39_999));
    let d = show(&dec(&wide), &Col::of(&dec("1")), &settings("tr-TR"));
    assert_eq!(d.kind, DisplayKind::Number);
    assert_eq!(d.text, wide[..1_024]);
}

#[test]
fn integers_group_by_the_format_locale() {
    assert_eq!(
        fmt(&Value::Int(i64::MIN), "tr-TR"),
        "-9.223.372.036.854.775.808"
    );
    assert_eq!(
        fmt(&Value::UInt(u64::MAX), "en-US"),
        "18,446,744,073,709,551,615"
    );
    assert_eq!(fmt(&Value::Int(999), "tr-TR"), "999");
    assert_eq!(fmt(&Value::Int(12_345), "tr-TR"), "12.345");
    assert_eq!(fmt(&Value::Int(0), "tr-TR"), "0");
    // An unknown locale is the root locale: grouped with `,`, decimals with `.`.
    assert_eq!(fmt(&Value::Int(12_345), "und"), "12,345");
}

#[test]
fn raw_mode_never_groups() {
    let mut s = settings("tr-TR");
    s.mode = DisplayMode::Raw;
    s.version = 1;
    let show_raw = |v: &Value| show(v, &Col::of(v), &s).text;
    assert_eq!(show_raw(&dec("1234567.50")), "1234567.50");
    assert_eq!(show_raw(&Value::Int(1_234_567)), "1234567");
    assert_eq!(show_raw(&Value::F64(1234.5)), "1234.5");
    assert_eq!(show_raw(&Value::F32(1234567.0)), "1.234567e+06");
}

#[test]
fn floats_use_the_shortest_round_trip_spelling() {
    let f = |x: f64, l: &str| fmt(&Value::F64(x), l);
    let g = |x: f32, l: &str| fmt(&Value::F32(x), l);
    assert_eq!(f(0.1 + 0.2, "en-US"), "0.30000000000000004");
    assert_eq!(f(1e15, "en-US"), "1e+15");
    assert_eq!(f(123_456_789_012_345.0, "en-US"), "123,456,789,012,345");
    assert_eq!(f(0.0001, "en-US"), "0.0001");
    assert_eq!(f(1e-5, "en-US"), "1e-05");
    assert_eq!(f(1e300, "en-US"), "1e+300");
    assert_eq!(f(-1.5e-7, "tr-TR"), "-1,5e-07");
    assert_eq!(f(0.1, "tr-TR"), "0,1");
    assert_eq!(f(-0.0, "tr-TR"), "-0");
    assert_eq!(g(1_234_567.0, "en-US"), "1.234567e+06");
    assert_eq!(g(1_234_567.0, "tr-TR"), "1,234567e+06");
    assert_eq!(g(123_456.0, "en-US"), "123,456");
    assert_eq!(g(1e6, "en-US"), "1e+06");
    assert_eq!(g(0.1, "tr-TR"), "0,1");
    // A float4 column's value that arrived widened keeps the float4 spelling.
    let col = Col::with(LogicalType::Float { bits: 32 }, DbmsKind::Postgres);
    assert_eq!(fmt_in(&Value::F64(f64::from(0.1f32)), &col), "0.1");
    let d = display_of(&Value::F64(1.5));
    assert_eq!((d.kind, d.align_end), (DisplayKind::Number, true));
}

#[test]
fn float_specials_are_spelled_as_the_server_spells_them() {
    let pg = Col::with(LogicalType::Float { bits: 64 }, DbmsKind::Postgres);
    let lite = Col::with(LogicalType::Float { bits: 64 }, DbmsKind::Sqlite);
    for (x, pg_text, lite_text) in [
        (f64::NAN, "NaN", "NaN"),
        (f64::INFINITY, "Infinity", "Inf"),
        (f64::NEG_INFINITY, "-Infinity", "-Inf"),
    ] {
        assert_eq!(show(&Value::F64(x), &pg, &settings("tr-TR")).text, pg_text);
        assert_eq!(
            show(&Value::F64(x), &lite, &settings("tr-TR")).text,
            lite_text
        );
    }
    assert_eq!(fmt(&Value::F32(f32::INFINITY), "tr-TR"), "Infinity");
}

// ---- the ID heuristic --------------------------------------------------------------------

#[test]
fn id_like_columns_are_integers_named_id_or_ending_in_id() {
    let i = int(64, true);
    for name in [
        "id",
        "ID",
        "customer_id",
        "Customer_ID",
        "kullanıcı_id",
        "customerId",
        "CustomerID",
    ] {
        assert!(id_like(name, &i, "int8"), "{name} is ID-like");
    }
    for name in [
        "paid", "valid", "VOID", "idx", "id_card", "uuid", "İD", "ıd", "amount",
    ] {
        assert!(!id_like(name, &i, "int8"), "{name} is not ID-like");
    }
    // Not an integer: never ID-like by name.
    assert!(!id_like("id", &text(), "text"));
    assert!(!id_like(
        "price_id",
        &LogicalType::Decimal {
            precision: Some(10),
            scale: Some(2)
        },
        "numeric"
    ));
    // An integer-valued decimal (`numeric(20,0)`) is.
    assert!(id_like(
        "order_id",
        &LogicalType::Decimal {
            precision: Some(20),
            scale: Some(0)
        },
        "numeric"
    ));
    assert!(id_like("id", &int(64, false), "bigint unsigned"));
}

#[test]
fn postgres_object_identifier_columns_are_id_like_whatever_their_name() {
    assert!(id_like("relfilenode", &int(32, false), "oid"));
    assert!(id_like("xmin", &int(32, false), "xid"));
    let opaque = |t: &str| LogicalType::Opaque {
        type_name: t.into(),
    };
    for t in ["regclass", "regtype", "regproc", "regnamespace", "regrole"] {
        assert!(id_like("c", &opaque(t), t), "{t} is ID-like");
    }
    assert!(!id_like("c", &opaque("region"), "region"));
    // The adapter computes the flag once per column; formatting reads it.
    let mut col = Col::of(&Value::UInt(4_294_967_295));
    col.native = "oid";
    col.logical = int(32, false);
    col.id_like = id_like("relid", &col.logical, col.native);
    assert_eq!(
        show(&Value::UInt(4_294_967_295), &col, &settings("tr-TR")).text,
        "4294967295"
    );
}

// ---- formatter memoisation ---------------------------------------------------------------

#[test]
fn the_formatter_is_memoised_per_locale_and_version() {
    let v = dec("1234.5");
    let col = Col::of(&v);
    let mut s = settings("tr-TR");
    s.version = 7;
    let before = formatter_builds();
    assert_eq!(show(&v, &col, &s).text, "1.234,5");
    assert_eq!(show(&v, &col, &s).text, "1.234,5");
    assert_eq!(formatter_builds(), before + 1, "one build for one key");
    // A locale change re-formats even when the version is not bumped.
    s.format_locale = "en-US".parse().expect("a locale");
    assert_eq!(show(&v, &col, &s).text, "1,234.5");
    assert_eq!(formatter_builds(), before + 2);
    // A version bump rebuilds.
    s.version = 8;
    assert_eq!(show(&v, &col, &s).text, "1,234.5");
    assert_eq!(formatter_builds(), before + 3);
    // Raw text and ID-like cells never need a formatter.
    let mut id = Col::of(&Value::Int(5));
    id.id_like = true;
    s.version = 9;
    assert_eq!(show(&Value::Int(1_234_567), &id, &s).text, "1234567");
    assert_eq!(formatter_builds(), before + 3);
}

// ---- dates and times ---------------------------------------------------------------------

#[test]
fn dates_and_times_stay_iso_at_server_precision() {
    let d = |y, m, day| Value::Date(DateVal::from_civil(y, m, day).expect("a date"));
    assert_eq!(fmt(&d(2026, 11, 13), "tr-TR"), "2026-11-13");
    assert_eq!(fmt(&d(-43, 3, 15), "tr-TR"), "0044-03-15 BC");
    assert_eq!(fmt(&d(10_000, 1, 1), "tr-TR"), "10000-01-01");
    // PostgreSQL 17: `'0044-03-15'::date - 730000` (44 AD) is `1956-07-13 BC`; the calendars agree.
    let far = DateVal(DateVal::from_civil(44, 3, 15).expect("a date").0 - 730_000);
    assert_eq!(fmt(&Value::Date(far), "en-US"), "1956-07-13 BC");
    assert_eq!(kind(&d(2026, 11, 13)), DisplayKind::Temporal);

    assert_eq!(fmt(&Value::Time(TimeVal::END_OF_DAY), "tr-TR"), "24:00:00");
    let half = TimeVal::from_micros(36_000_500_000);
    assert_eq!(
        fmt(&Value::TimeTz(half, 19_800), "tr-TR"),
        "10:00:00.5+05:30"
    );
    assert_eq!(fmt(&Value::TimeTz(half, -28_800), "tr-TR"), "10:00:00.5-08");

    // PostgreSQL trims trailing zeros of the fraction.
    let t = |nanos| Value::Timestamp(naive((2026, 11, 13), (10, 0, 0), nanos));
    assert_eq!(fmt(&t(120_000_000), "tr-TR"), "2026-11-13 10:00:00.12");
    assert_eq!(fmt(&t(0), "tr-TR"), "2026-11-13 10:00:00");
    assert_eq!(fmt(&t(123_456_000), "tr-TR"), "2026-11-13 10:00:00.123456");
    // MySQL pads to the column's fractional precision (`DATETIME(3)`).
    let my = |p| Col::with(timestamp(p, false), DbmsKind::MySql);
    assert_eq!(fmt_in(&t(120_000_000), &my(3)), "2026-11-13 10:00:00.120");
    assert_eq!(fmt_in(&t(0), &my(6)), "2026-11-13 10:00:00.000000");
    assert_eq!(fmt_in(&t(0), &my(0)), "2026-11-13 10:00:00");
    // A digit past the declared precision is never hidden.
    assert_eq!(fmt_in(&t(123_000), &my(3)), "2026-11-13 10:00:00.000123");
    let my_time = Col::with(time(2, false), DbmsKind::MariaDb);
    assert_eq!(fmt_in(&Value::Time(half), &my_time), "10:00:00.50");

    let bc = Value::TimestampTz {
        utc: naive((-43, 3, 15), (10, 0, 0), 0),
        offset: None,
    };
    assert_eq!(fmt(&bc, "en-US"), "0044-03-15 10:00:00+00 BC");
}

#[test]
fn timestamptz_keeps_the_offset_the_server_sent() {
    let v = Value::TimestampTz {
        utc: ts("2026-11-11T07:00:00Z"),
        offset: Some(19_800),
    };
    // The server's offset wins over the display zone.
    assert_eq!(fmt_tz(&v, "Europe/Istanbul"), "2026-11-11 12:30:00+05:30");
    let lmt = Value::TimestampTz {
        utc: ts("1899-12-31T22:03:04Z"),
        offset: Some(7_016),
    };
    assert_eq!(fmt(&lmt, "en-US"), "1900-01-01 00:00:00+01:56:56");
}

#[test]
fn timestamptz_outside_the_zone_database_range_is_shown_in_utc() {
    // PostgreSQL's timestamps reach 294276 AD; the zone rules stop at 9999.
    let v = Value::TimestampTz {
        utc: naive((200_000, 1, 1), (0, 0, 0), 0),
        offset: None,
    };
    assert_eq!(fmt_tz(&v, "Europe/Istanbul"), "200000-01-01 00:00:00+00");
}

#[test]
fn a_timestamp_beyond_the_calendar_is_an_error_cell_not_a_panic() {
    let v = Value::Timestamp(TsVal {
        secs: i64::MAX,
        nanos: 0,
    });
    let d = display_of(&v);
    assert_eq!(d.kind, DisplayKind::Error);
    assert!(d.text.contains("9223372036854775807"), "{}", d.text);
    // Nanoseconds past a second carry into the seconds rather than panicking.
    let carry = Value::Timestamp(TsVal {
        secs: naive((2026, 11, 13), (10, 0, 0), 0).secs,
        nanos: 1_500_000_000,
    });
    assert_eq!(fmt(&carry, "en-US"), "2026-11-13 10:00:01.5");
}

#[test]
fn intervals_print_like_postgres() {
    for (v, want) in [
        (interval(0, 1, -3_600_000_000), "1 day -01:00:00"),
        (interval(0, -1, 3_600_000_000), "-1 days +01:00:00"),
        (interval(0, 0, 0), "00:00:00"),
        (interval(14, 0, 0), "1 year 2 mons"),
        (interval(-10, 0, 0), "-10 mons"),
        (interval(0, 0, 1_500_000), "00:00:01.5"),
        (interval(0, 0, -500_000), "-00:00:00.5"),
        (interval(0, 0, 360_000_000_000), "100:00:00"),
        (interval(0, 1, 1), "1 day 00:00:00.000001"),
        (interval(-1, -2, 0), "-1 mons -2 days"),
        (interval(0, -1, -1_000_000), "-1 days -00:00:01"),
        (interval(1, -1, 0), "1 mon -1 days"),
        (interval(-12, 0, 0), "-1 years"),
        (interval(11, 0, 0), "11 mons"),
        (interval(0, 0, -7_200_000_000), "-02:00:00"),
        (interval(0, 1, 3_600_000_000), "1 day 01:00:00"),
    ] {
        assert_eq!(fmt(&v, "tr-TR"), want);
    }
    let extreme = interval(i32::MIN, i32::MIN, i64::MIN);
    assert_eq!(kind(&extreme), DisplayKind::Temporal);
    assert_eq!(
        fmt(&extreme, "en-US"),
        "-178956970 years -8 mons -2147483648 days -2562047788:00:54.775808"
    );
}

#[test]
fn a_mysql_time_beyond_a_day_shows_its_hours() {
    let col = Col::with(time(0, false), DbmsKind::MySql);
    assert_eq!(fmt_in(&interval(0, 1, 7_200_000_000), &col), "26:00:00");
    assert_eq!(fmt_in(&interval(0, 0, -3_600_000_000), &col), "-01:00:00");
    assert_eq!(
        fmt_in(&interval(0, -34, -82_799_000_000), &col),
        "-838:59:59"
    );
}

#[test]
fn specials_are_verbatim_in_their_column_spelling() {
    let numeric = Col::with(
        LogicalType::Decimal {
            precision: None,
            scale: None,
        },
        DbmsKind::Postgres,
    );
    let date = Col::with(LogicalType::Date, DbmsKind::Postgres);
    let sp = Value::Special;
    for (v, num, day) in [
        (sp(SpecialVal::NaN), "NaN", "NaN"),
        (sp(SpecialVal::PosInfinity), "Infinity", "infinity"),
        (sp(SpecialVal::NegInfinity), "-Infinity", "-infinity"),
    ] {
        let n = show(&v, &numeric, &settings("tr-TR"));
        assert_eq!(
            (n.text.as_str(), n.kind, n.align_end),
            (num, DisplayKind::Number, true)
        );
        let d = show(&v, &date, &settings("tr-TR"));
        assert_eq!((d.text.as_str(), d.kind), (day, DisplayKind::Temporal));
        assert_eq!(d.flags, CellFlags::SPECIAL.bits());
    }
    let my_date = Col::with(LogicalType::Date, DbmsKind::MySql);
    let my_dt = Col::with(timestamp(3, false), DbmsKind::MySql);
    assert_eq!(fmt_in(&sp(SpecialVal::ZeroDate), &my_date), "0000-00-00");
    assert_eq!(
        fmt_in(&sp(SpecialVal::ZeroDate), &my_dt),
        "0000-00-00 00:00:00.000"
    );
    assert_eq!(
        fmt_in(
            &sp(SpecialVal::PartialZeroDate("2026-00-13".into())),
            &my_date
        ),
        "2026-00-13"
    );
}

// ---- bytes, text and the rest ----------------------------------------------------------

#[test]
fn bytes_are_shown_by_detection() {
    let b = |bytes: &[u8]| fmt(&Value::Bytes(Arc::from(bytes)), "en-US");
    assert_eq!(b(&[0x00, 0xff]), "\\x00ff");
    assert_eq!(b(b""), "\\x");
    assert_eq!(b(b"hello, world"), "hello, world");
    // 9 of 10 characters printable: text; 8 of 10: hex.
    assert_eq!(b(b"abcdefghi\x01"), "abcdefghi\u{1}");
    assert_eq!(b(b"abcdefgh\x01\x02"), "\\x61626364656667680102");
    assert_eq!(b("Müşteri\tadı".as_bytes()), "Müşteri\tadı");
    assert_eq!(b(b"caf\xe9 ok"), "\\x636166e9206f6b");
    assert_eq!(
        kind(&Value::Bytes(Arc::from(&b"abc"[..]))),
        DisplayKind::Binary
    );
}

#[test]
fn binary_display_overrides_detection() {
    let with = |binary, bytes: &[u8]| {
        let v = Value::Bytes(Arc::from(bytes));
        let mut col = Col::of(&v);
        col.binary = binary;
        show(&v, &col, &settings("en-US")).text
    };
    let uuid = uuid_bytes();
    assert_eq!(
        with(BinaryDisplay::Hex, &uuid),
        "\\x6f1e9a0c000040008000000000000001"
    );
    assert_eq!(
        with(BinaryDisplay::Uuid, &uuid),
        "6f1e9a0c-0000-4000-8000-000000000001"
    );
    assert_eq!(with(BinaryDisplay::Uuid, b"abc"), "\\x616263");
    assert_eq!(with(BinaryDisplay::Text, b"\x01abc"), "\u{1}abc");
    assert_eq!(with(BinaryDisplay::Text, b"caf\xe9"), "caf\\xe9");
}

#[test]
fn large_bytes_are_detected_on_a_bounded_prefix_and_capped() {
    let five_mib = 5usize << 20;
    let text = fmt(&Value::Bytes(Arc::from(vec![b'a'; five_mib])), "en-US");
    assert_eq!(text, "a".repeat(1_024));
    let hex = fmt(&Value::Bytes(Arc::from(vec![0u8; five_mib])), "en-US");
    assert!(hex.starts_with("\\x0000"));
    assert!(hex.chars().count() <= 1_024, "{}", hex.len());
    // A multi-byte character split by the detection window still reads as text.
    let mut tr = "ş".repeat(five_mib / 2).into_bytes();
    tr.insert(0, b'a');
    assert_eq!(
        fmt(&Value::Bytes(Arc::from(tr)), "en-US"),
        format!("a{}", "ş".repeat(1_023))
    );
}

#[test]
fn text_is_capped_at_max_display_chars_graphemes_and_keeps_controls() {
    let t = |s: String| fmt(&Value::Text(s.into()), "en-US");
    assert_eq!(t("a".repeat(5_000)), "a".repeat(1_024));
    assert_eq!(t("e\u{301}".repeat(2_000)), "e\u{301}".repeat(1_024));
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    assert_eq!(t(family.repeat(1_500)), family.repeat(1_024));
    assert_eq!(t("İıŞş".repeat(300)), "İıŞş".repeat(256));
    // Controls are kept: the grid draws them, copy keeps them.
    assert_eq!(t("a\tb\nc\u{202E}d\0".into()), "a\tb\nc\u{202E}d\0");
    let mut s = settings("en-US");
    s.max_display_chars = 3;
    let col = Col::of(&Value::Text("".into()));
    assert_eq!(show(&Value::Text("Ağaç".into()), &col, &s).text, "Ağa");
    assert_eq!(show(&Value::Json("[1,2,3]".into()), &col, &s).text, "[1,");
    assert_eq!(show(&dec("1234567.5"), &Col::of(&dec("1")), &s).text, "1,2");
}

#[test]
fn lossy_text_is_shown_with_escapes_and_flagged() {
    let v = Value::Lossy(Arc::from(&b"caf\xe9 \xff\xfe!"[..]));
    let d = display_of(&v);
    assert_eq!(d.text, "caf\\xe9 \\xff\\xfe!");
    assert_eq!(d.kind, DisplayKind::Text);
    assert_eq!(d.flags, CellFlags::LOSSY.bits());
    // A truncated lossy prefix carries both flags.
    let t = Value::Truncated(Box::new(TruncatedVal {
        prefix: v,
        total_len: None,
        unit: LenUnit::Bytes,
    }));
    let d = show(&t, &Col::of(&Value::Text("".into())), &settings("en-US"));
    assert_eq!(d.flags, (CellFlags::TRUNCATED | CellFlags::LOSSY).bits());
    // An unknown total has no chip: no length is invented.
    assert_eq!(d.chip, None);
}

#[test]
fn error_cells_carry_their_message() {
    let v = Value::Error(Arc::new(ErrorVal {
        code: "pg-out-of-range".into(),
        message: "the date is outside the supported range (raw value 2147483646)".into(),
    }));
    let d = display_of(&v);
    assert_eq!(d.kind, DisplayKind::Error);
    assert_eq!(
        d.text,
        "the date is outside the supported range (raw value 2147483646)"
    );
    assert_eq!(d.flags, CellFlags::ERROR.bits());
    assert!(!d.align_end);
}

#[test]
fn null_bool_uuid_opaque_and_xml() {
    let null = show(&Value::Null, &Col::of(&dec("1")), &settings("tr-TR"));
    assert_eq!((null.kind, null.align_end), (DisplayKind::Null, true));
    assert_eq!(null.text, "NULL");
    assert!(
        !show(
            &Value::Null,
            &Col::of(&Value::Text("".into())),
            &settings("tr-TR")
        )
        .align_end
    );
    assert_eq!(fmt(&Value::Bool(true), "tr-TR"), "true");
    assert_eq!(fmt(&Value::Bool(false), "tr-TR"), "false");
    assert_eq!(kind(&Value::Bool(true)), DisplayKind::Bool);
    let uuid = Value::Uuid([
        0xAB, 0xCD, 0xEF, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0x01, 0x23, 0x45, 0x67,
        0x89,
    ]);
    assert_eq!(fmt(&uuid, "tr-TR"), "abcdef01-2345-6789-abcd-ef0123456789");
    let opaque = Value::Opaque(Arc::new(OpaqueVal {
        type_name: "_int4".into(),
        text: "{1,2,3}".into(),
    }));
    assert_eq!(fmt(&opaque, "tr-TR"), "{1,2,3}");
    assert_eq!(kind(&opaque), DisplayKind::Text);
    assert_eq!(
        fmt(&Value::Xml("<a b=\"1\"/>".into()), "tr-TR"),
        "<a b=\"1\"/>"
    );
    assert_eq!(kind(&Value::Xml("<a/>".into())), DisplayKind::Json);
    assert_eq!(kind(&Value::Json("{}".into())), DisplayKind::Json);
}

#[test]
fn every_purpose_formats_alike_in_m3a() {
    let v = dec("1234567.50");
    let col = Col::of(&v);
    let s = settings("tr-TR");
    let cell = format_cell(v.view(), &col.facts(), &s, Purpose::Cell);
    for p in [Purpose::Tooltip, Purpose::A11y] {
        assert_eq!(format_cell(v.view(), &col.facts(), &s, p), cell);
    }
}

// ---- hostile payloads ------------------------------------------------------------------

/// Every temporal, interval and float payload a hostile or buggy decoder could produce, in a
/// PostgreSQL and a MySQL column, under a real zone: never a panic; a temporal cell is
/// `Temporal` or (beyond the calendar) `Error`.
fn every_cell_is_formatted(v: &Value, logical: LogicalType) {
    let s = DisplaySettings::new("tr-TR", Some("Europe/Istanbul"));
    for engine in [DbmsKind::Postgres, DbmsKind::MySql] {
        let col = Col::with(logical.clone(), engine);
        let d = show(v, &col, &s);
        assert!(d.text.chars().count() <= 1_024);
        let _ = hacivat_values::valfmt::raw_text(v.view(), &col.facts());
        if !matches!(v, Value::F64(_)) {
            assert!(
                matches!(d.kind, DisplayKind::Temporal | DisplayKind::Error),
                "{d:?}"
            );
        }
    }
}

proptest::proptest! {
    #[test]
    fn hostile_temporal_payloads_never_panic(
        secs in proptest::prelude::any::<i64>(),
        nanos in proptest::prelude::any::<u32>(),
        offset in proptest::option::of(proptest::prelude::any::<i32>()),
        tod in proptest::prelude::any::<u64>(),
        day in proptest::prelude::any::<i32>(),
        (months, days, micros) in proptest::prelude::any::<(i32, i32, i64)>(),
        float_bits in proptest::prelude::any::<u64>(),
        precision in 0u8..=12,
    ) {
        let ts = TsVal { secs, nanos };
        every_cell_is_formatted(&Value::Timestamp(ts), timestamp(precision, false));
        every_cell_is_formatted(&Value::TimestampTz { utc: ts, offset }, timestamp(precision, true));
        every_cell_is_formatted(&Value::Date(DateVal(day)), LogicalType::Date);
        let t = TimeVal { nanos_of_day: tod };
        every_cell_is_formatted(&Value::Time(t), time(precision, false));
        every_cell_is_formatted(&Value::TimeTz(t, offset.unwrap_or(0)), time(precision, true));
        every_cell_is_formatted(&interval(months, days, micros), time(precision, false));
        every_cell_is_formatted(&interval(months, days, micros), LogicalType::Interval);
        every_cell_is_formatted(&Value::F64(f64::from_bits(float_bits)), LogicalType::Float { bits: 32 });
    }
}

// ---- perf --------------------------------------------------------------------------------

/// `hacivat_testkit::perf_enabled`, inlined (the crate has no testkit dev-dependency).
fn perf_enabled() -> bool {
    std::env::var_os("HACIVAT_PERF").is_some_and(|v| v == "1")
}

/// The B1-real mix (int8, text, numeric(14,2), timestamptz) in `tr-TR` and Europe/Istanbul,
/// cache misses only: raw text p50 under 1 µs, localized display p50 under 5 µs per cell.
#[test]
#[ignore = "perf"]
fn raw_under_1_us_localized_under_5_us() {
    use std::hint::black_box;
    use std::time::Instant;

    if !perf_enabled() {
        return;
    }
    let s = DisplaySettings::new("tr-TR", Some("Europe/Istanbul"));
    let base = ts("2026-11-11T07:00:00Z").secs;
    let n = 4_096i64;
    let words = [
        "Müşteri",
        "İstanbul",
        "ılık",
        "Çağrı",
        "5d41402abc4b2a76",
        "Ağaç",
    ];
    let values: Vec<Value> = (0..n)
        .flat_map(|i| {
            [
                Value::Int(1_000_000 + i * 7_919),
                Value::Text(words[i as usize % words.len()].into()),
                Value::Decimal(DecimalVal::from_mantissa(11_019_085 + i * 37, 2)),
                Value::TimestampTz {
                    utc: TsVal {
                        secs: base + i * 3_601,
                        nanos: (i as u32 % 1_000) * 1_000,
                    },
                    offset: None,
                },
            ]
        })
        .collect();
    let cols: Vec<Col> = values.iter().take(4).map(Col::of).collect();
    let facts: Vec<ColumnFacts<'_>> = cols.iter().map(Col::facts).collect();
    let per_cell = |f: &dyn Fn(&Value, &ColumnFacts<'_>)| {
        let mut samples = Vec::new();
        for _ in 0..25 {
            let start = Instant::now();
            for (k, v) in values.iter().enumerate() {
                f(v, &facts[k % 4]);
            }
            samples.push(start.elapsed().as_nanos() as f64 / values.len() as f64);
        }
        samples.sort_by(f64::total_cmp);
        samples[samples.len() / 2]
    };
    let raw_ns = per_cell(&|v, c| {
        black_box(hacivat_values::valfmt::raw_text(v.view(), c));
    });
    let shown_ns = per_cell(&|v, c| {
        black_box(format_cell(v.view(), c, &s, Purpose::Cell));
    });
    let dec_col = &facts[2];
    let dec_ns = {
        let decimals: Vec<&Value> = values.iter().skip(2).step_by(4).collect();
        let mut samples = Vec::new();
        for _ in 0..25 {
            let start = Instant::now();
            for v in &decimals {
                black_box(format_cell(v.view(), dec_col, &s, Purpose::Cell));
            }
            samples.push(start.elapsed().as_nanos() as f64 / decimals.len() as f64);
        }
        samples.sort_by(f64::total_cmp);
        samples[samples.len() / 2]
    };
    println!(
        "valfmt p50 per cell (B1 mix, tr-TR): raw_text {raw_ns:.0} ns, format_cell {shown_ns:.0} ns, numeric(14,2) format_cell {dec_ns:.0} ns"
    );
    assert!(raw_ns < 1_000.0, "raw_text p50 {raw_ns:.0} ns, budget 1 µs");
    assert!(
        shown_ns < 5_000.0,
        "format_cell p50 {shown_ns:.0} ns, budget 5 µs"
    );
    assert!(
        dec_ns < 5_000.0,
        "decimal format_cell p50 {dec_ns:.0} ns, budget 5 µs"
    );
}
