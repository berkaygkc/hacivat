//! The MySQL and MariaDB value decoders (Task 14, R-M3-31): the column mapping over synthetic
//! column definitions, both protocols' cell decoding read back through a real chunk, the
//! special values, and hostile bytes (never a panic, always an `Error` cell). No server.

use std::sync::Arc;

use hacivat_driver_mysql::decode::{
    BINARY_CHARSET, Decoded, Plan, binary_decodable, buf_kind_of, decode_cell_bin,
    decode_cell_text, decode_date_text, decode_datetime_text, decode_decimal_text, decode_time_bin,
    decode_time_text, logical_of,
};
use hacivat_values::{
    BufKind, DateVal, DecimalVal, LenUnit, LogicalType, NativeType, OriginHint, SpecialVal,
    TempUnit, TimeVal, TsVal, Value, VarKind,
};
use mysql_async::Column;
use mysql_async::Value as My;
use mysql_async::consts::{ColumnFlags, ColumnType};

/// `utf8mb4_0900_ai_ci`, what MySQL 8 reports for text columns of a utf8mb4 connection.
const UTF8MB4: u16 = 255;
const MIB: u32 = 1 << 20;

fn col(t: ColumnType) -> Column {
    Column::new(t).with_name(b"c")
}

fn flagged(t: ColumnType, flags: ColumnFlags) -> Column {
    col(t).with_flags(flags)
}

fn text_col(t: ColumnType, length: u32) -> Column {
    col(t)
        .with_character_set(UTF8MB4)
        .with_column_length(length)
}

fn bytes_col(t: ColumnType, length: u32) -> Column {
    col(t)
        .with_character_set(BINARY_CHARSET)
        .with_column_length(length)
        .with_flags(ColumnFlags::BINARY_FLAG)
}

fn decimal(length: u32, scale: u8, unsigned: bool) -> Column {
    let flags = if unsigned {
        ColumnFlags::UNSIGNED_FLAG
    } else {
        ColumnFlags::empty()
    };
    col(ColumnType::MYSQL_TYPE_NEWDECIMAL)
        .with_column_length(length)
        .with_decimals(scale)
        .with_flags(flags)
        .with_character_set(BINARY_CHARSET)
}

fn temporal(t: ColumnType, decimals: u8) -> Column {
    col(t)
        .with_decimals(decimals)
        .with_character_set(BINARY_CHARSET)
}

fn tinyint1() -> Column {
    col(ColumnType::MYSQL_TYPE_TINY).with_column_length(1)
}

struct Case {
    name: &'static str,
    column: Column,
    expect: LogicalType,
    buf: BufKind,
}

fn int(bits: u8, signed: bool) -> LogicalType {
    LogicalType::Int { bits, signed }
}

fn opaque(name: &str) -> LogicalType {
    LogicalType::Opaque {
        type_name: name.into(),
    }
}

fn mysql_columns() -> Vec<Case> {
    use ColumnType::*;
    let unsigned = ColumnFlags::UNSIGNED_FLAG;
    let case = |name, column, expect, buf| Case {
        name,
        column,
        expect,
        buf,
    };
    vec![
        case("tinyint", col(MYSQL_TYPE_TINY), int(8, true), BufKind::I64),
        case(
            "tinyint unsigned",
            flagged(MYSQL_TYPE_TINY, unsigned),
            int(8, false),
            BufKind::I64,
        ),
        case("tinyint(1)", tinyint1(), int(8, true), BufKind::I64),
        case(
            "smallint",
            col(MYSQL_TYPE_SHORT),
            int(16, true),
            BufKind::I64,
        ),
        case(
            "mediumint",
            col(MYSQL_TYPE_INT24),
            int(24, true),
            BufKind::I64,
        ),
        case("int", col(MYSQL_TYPE_LONG), int(32, true), BufKind::I64),
        case(
            "int unsigned",
            flagged(MYSQL_TYPE_LONG, unsigned),
            int(32, false),
            BufKind::I64,
        ),
        case(
            "bigint",
            col(MYSQL_TYPE_LONGLONG),
            int(64, true),
            BufKind::I64,
        ),
        case(
            "bigint unsigned",
            flagged(MYSQL_TYPE_LONGLONG, unsigned),
            int(64, false),
            BufKind::U64,
        ),
        case(
            "year",
            flagged(MYSQL_TYPE_YEAR, unsigned | ColumnFlags::ZEROFILL_FLAG),
            int(16, false),
            BufKind::I64,
        ),
        case(
            "year without flags",
            col(MYSQL_TYPE_YEAR),
            int(16, false),
            BufKind::I64,
        ),
        case(
            "decimal(10,2)",
            decimal(12, 2, false),
            LogicalType::Decimal {
                precision: Some(10),
                scale: Some(2),
            },
            BufKind::Dec64 { scale: 2 },
        ),
        case(
            "decimal(10,2) unsigned",
            decimal(11, 2, true),
            LogicalType::Decimal {
                precision: Some(10),
                scale: Some(2),
            },
            BufKind::Dec64 { scale: 2 },
        ),
        case(
            "decimal(18,0)",
            decimal(19, 0, false),
            LogicalType::Decimal {
                precision: Some(18),
                scale: Some(0),
            },
            BufKind::Dec64 { scale: 0 },
        ),
        case(
            "decimal(19,4)",
            decimal(21, 4, false),
            LogicalType::Decimal {
                precision: Some(19),
                scale: Some(4),
            },
            BufKind::Dec128,
        ),
        case(
            "decimal(65,30)",
            decimal(67, 30, false),
            LogicalType::Decimal {
                precision: Some(65),
                scale: Some(30),
            },
            BufKind::Dec128,
        ),
        case(
            "old decimal",
            col(MYSQL_TYPE_DECIMAL)
                .with_column_length(6)
                .with_decimals(2),
            LogicalType::Decimal {
                precision: Some(4),
                scale: Some(2),
            },
            BufKind::Dec64 { scale: 2 },
        ),
        case(
            "decimal with a nonsense length",
            decimal(0, 0, false),
            LogicalType::Decimal {
                precision: None,
                scale: Some(0),
            },
            BufKind::Dec128,
        ),
        case(
            "float",
            col(MYSQL_TYPE_FLOAT),
            LogicalType::Float { bits: 32 },
            BufKind::F64 { f32: true },
        ),
        case(
            "double",
            col(MYSQL_TYPE_DOUBLE),
            LogicalType::Float { bits: 64 },
            BufKind::F64 { f32: false },
        ),
        case(
            "char(10)",
            text_col(MYSQL_TYPE_STRING, 40),
            LogicalType::Text {
                max_chars: None,
                fixed: true,
            },
            BufKind::Var(VarKind::Text),
        ),
        case(
            "varchar(10)",
            text_col(MYSQL_TYPE_VAR_STRING, 40),
            LogicalType::Text {
                max_chars: None,
                fixed: false,
            },
            BufKind::Var(VarKind::Text),
        ),
        case(
            "text",
            text_col(MYSQL_TYPE_BLOB, 262_140).with_flags(ColumnFlags::BLOB_FLAG),
            LogicalType::Text {
                max_chars: None,
                fixed: false,
            },
            BufKind::Var(VarKind::Text),
        ),
        case(
            "varchar with a binary collation is still text",
            text_col(MYSQL_TYPE_VAR_STRING, 40).with_flags(ColumnFlags::BINARY_FLAG),
            LogicalType::Text {
                max_chars: None,
                fixed: false,
            },
            BufKind::Var(VarKind::Text),
        ),
        case(
            "binary(4)",
            bytes_col(MYSQL_TYPE_STRING, 4),
            LogicalType::Bytes { max: Some(4) },
            BufKind::Var(VarKind::Bytes),
        ),
        case(
            "varbinary(9)",
            bytes_col(MYSQL_TYPE_VAR_STRING, 9),
            LogicalType::Bytes { max: Some(9) },
            BufKind::Var(VarKind::Bytes),
        ),
        case(
            "longblob",
            bytes_col(MYSQL_TYPE_BLOB, u32::MAX),
            LogicalType::Bytes {
                max: Some(u32::MAX),
            },
            BufKind::Var(VarKind::Bytes),
        ),
        case(
            "enum (by flag)",
            text_col(MYSQL_TYPE_STRING, 12).with_flags(ColumnFlags::ENUM_FLAG),
            LogicalType::Text {
                max_chars: None,
                fixed: false,
            },
            BufKind::Var(VarKind::Text),
        ),
        case(
            "set (by flag)",
            text_col(MYSQL_TYPE_STRING, 20).with_flags(ColumnFlags::SET_FLAG),
            LogicalType::Text {
                max_chars: None,
                fixed: false,
            },
            BufKind::Var(VarKind::Text),
        ),
        case(
            "enum (by type)",
            text_col(MYSQL_TYPE_ENUM, 12),
            LogicalType::Text {
                max_chars: None,
                fixed: false,
            },
            BufKind::Var(VarKind::Text),
        ),
        case(
            "json",
            bytes_col(MYSQL_TYPE_JSON, u32::MAX),
            LogicalType::Json { binary: false },
            BufKind::Var(VarKind::Json),
        ),
        case(
            "date",
            temporal(MYSQL_TYPE_DATE, 0),
            LogicalType::Date,
            BufKind::Temporal {
                unit: TempUnit::Days,
            },
        ),
        case(
            "datetime(6)",
            temporal(MYSQL_TYPE_DATETIME, 6),
            LogicalType::Timestamp {
                precision: 6,
                tz: false,
            },
            BufKind::Temporal {
                unit: TempUnit::Micros,
            },
        ),
        case(
            "timestamp(3) is naive",
            temporal(MYSQL_TYPE_TIMESTAMP, 3),
            LogicalType::Timestamp {
                precision: 3,
                tz: false,
            },
            BufKind::Temporal {
                unit: TempUnit::Micros,
            },
        ),
        case(
            "time(2)",
            temporal(MYSQL_TYPE_TIME, 2),
            LogicalType::Time {
                precision: 2,
                tz: false,
            },
            BufKind::Temporal {
                unit: TempUnit::Micros,
            },
        ),
        case(
            "a fractional precision past 6 (not fixed) is 6",
            temporal(MYSQL_TYPE_DATETIME, 31),
            LogicalType::Timestamp {
                precision: 6,
                tz: false,
            },
            BufKind::Temporal {
                unit: TempUnit::Micros,
            },
        ),
        case(
            "bit(5)",
            col(MYSQL_TYPE_BIT).with_column_length(5),
            opaque("bit"),
            BufKind::Values,
        ),
        case(
            "geometry",
            bytes_col(MYSQL_TYPE_GEOMETRY, u32::MAX),
            opaque("geometry"),
            BufKind::Values,
        ),
        case(
            "vector",
            bytes_col(MYSQL_TYPE_VECTOR, 64),
            opaque("vector"),
            BufKind::Values,
        ),
        case(
            "an internal type",
            col(MYSQL_TYPE_TIME2),
            opaque("time2"),
            BufKind::Values,
        ),
        case(
            "NULL",
            col(MYSQL_TYPE_NULL),
            LogicalType::Text {
                max_chars: None,
                fixed: false,
            },
            BufKind::Var(VarKind::Text),
        ),
    ]
}

#[test]
fn mapping_table_over_synthetic_columns() {
    for c in mysql_columns() {
        assert_eq!(logical_of(&c.column), c.expect, "{}", c.name);
        assert_eq!(buf_kind_of(&c.column), c.buf, "{}", c.name);
    }
}

#[test]
fn tinyint_1_stays_an_integer() {
    assert_eq!(
        logical_of(&tinyint1()),
        LogicalType::Int {
            bits: 8,
            signed: true
        }
    );
    assert_eq!(
        decode_cell_text(&tinyint1(), Some(b"1"), MIB),
        Value::Int(1)
    );
    assert_eq!(
        decode_cell_bin(&tinyint1(), &My::Int(1), MIB),
        Value::Int(1)
    );
}

#[test]
fn mysql_zero_date_is_special_not_null() {
    assert!(matches!(
        decode_date_text("0000-00-00"),
        Decoded::Special(SpecialVal::ZeroDate)
    ));
    let date = temporal(ColumnType::MYSQL_TYPE_DATE, 0);
    let zero = Value::Special(SpecialVal::ZeroDate);
    assert_eq!(decode_cell_text(&date, Some(b"0000-00-00"), MIB), zero);
    assert_eq!(
        decode_cell_bin(&date, &My::Date(0, 0, 0, 0, 0, 0, 0), MIB),
        zero
    );
    let dt = temporal(ColumnType::MYSQL_TYPE_DATETIME, 0);
    assert_eq!(
        decode_cell_text(&dt, Some(b"0000-00-00 00:00:00"), MIB),
        zero
    );
    assert_eq!(
        decode_cell_bin(&dt, &My::Date(0, 0, 0, 0, 0, 0, 0), MIB),
        zero
    );
}

#[test]
fn partial_zero_dates_are_specials_with_the_servers_text() {
    let partial = |s: &str| Value::Special(SpecialVal::PartialZeroDate(s.into()));
    assert!(matches!(
        decode_date_text("2024-00-15"),
        Decoded::Special(SpecialVal::PartialZeroDate(t)) if &*t == "2024-00-15"
    ));
    let date = temporal(ColumnType::MYSQL_TYPE_DATE, 0);
    assert_eq!(
        decode_cell_text(&date, Some(b"2024-05-00"), MIB),
        partial("2024-05-00")
    );
    // The binary protocol's parts give the same text the text protocol sends.
    assert_eq!(
        decode_cell_bin(&date, &My::Date(2024, 5, 0, 0, 0, 0, 0), MIB),
        partial("2024-05-00")
    );
    let dt3 = temporal(ColumnType::MYSQL_TYPE_DATETIME, 3);
    assert_eq!(
        decode_cell_bin(&dt3, &My::Date(2024, 0, 0, 10, 5, 7, 123_000), MIB),
        partial("2024-00-00 10:05:07.123")
    );
    assert_eq!(
        decode_cell_text(&dt3, Some(b"0000-00-00 10:05:07.123"), MIB),
        partial("0000-00-00 10:05:07.123")
    );
    // A zero date with a time is a partial zero, not the zero date.
    let dt = temporal(ColumnType::MYSQL_TYPE_DATETIME, 0);
    assert_eq!(
        decode_cell_bin(&dt, &My::Date(0, 0, 0, 12, 0, 0, 0), MIB),
        partial("0000-00-00 12:00:00")
    );
    assert!(matches!(
        decode_datetime_text("0000-00-00 00:00:01"),
        Decoded::Special(SpecialVal::PartialZeroDate(_))
    ));
}

#[test]
fn dates_and_timestamps_decode_exactly() {
    let date = temporal(ColumnType::MYSQL_TYPE_DATE, 0);
    let d = DateVal::from_civil(2024, 2, 29).unwrap();
    assert_eq!(
        decode_cell_text(&date, Some(b"2024-02-29"), MIB),
        Value::Date(d)
    );
    assert_eq!(
        decode_cell_bin(&date, &My::Date(2024, 2, 29, 0, 0, 0, 0), MIB),
        Value::Date(d)
    );
    // Year 0 is a real year (MySQL's proleptic calendar; astronomical numbering).
    assert_eq!(
        decode_cell_text(&date, Some(b"0000-01-01"), MIB),
        Value::Date(DateVal::from_civil(0, 1, 1).unwrap())
    );
    let dt6 = temporal(ColumnType::MYSQL_TYPE_DATETIME, 6);
    let ts = TsVal {
        secs: 1_709_164_800 + 13 * 3600 + 14 * 60 + 15,
        nanos: 123_456_000,
    };
    assert_eq!(
        decode_cell_text(&dt6, Some(b"2024-02-29 13:14:15.123456"), MIB),
        Value::Timestamp(ts)
    );
    assert_eq!(
        decode_cell_bin(&dt6, &My::Date(2024, 2, 29, 13, 14, 15, 123_456), MIB),
        Value::Timestamp(ts)
    );
    // Fewer fraction digits are tenths, hundredths...
    assert_eq!(
        decode_cell_text(&dt6, Some(b"1970-01-01 00:00:00.5"), MIB),
        Value::Timestamp(TsVal {
            secs: 0,
            nanos: 500_000_000
        })
    );
    // Before 1970.
    assert_eq!(
        decode_cell_text(&dt6, Some(b"1969-12-31 23:59:59.999999"), MIB),
        Value::Timestamp(TsVal {
            secs: -1,
            nanos: 999_999_000
        })
    );
    // A TIMESTAMP is the server's session-zone text, naive.
    let tstamp = temporal(ColumnType::MYSQL_TYPE_TIMESTAMP, 0);
    assert_eq!(
        decode_cell_text(&tstamp, Some(b"2038-01-19 03:14:07"), MIB),
        Value::Timestamp(TsVal {
            secs: 2_147_483_647,
            nanos: 0
        })
    );
}

#[test]
fn a_calendar_date_the_server_allows_is_kept_as_its_text() {
    // ALLOW_INVALID_DATES stores 2024-02-30; it is data, never altered.
    let date = temporal(ColumnType::MYSQL_TYPE_DATE, 0);
    match decode_cell_text(&date, Some(b"2024-02-30"), MIB) {
        Value::Opaque(o) => {
            assert_eq!(&*o.type_name, "date");
            assert_eq!(&*o.text, "2024-02-30");
        }
        other => panic!("{other:?}"),
    }
    match decode_cell_bin(&date, &My::Date(2023, 4, 31, 0, 0, 0, 0), MIB) {
        Value::Opaque(o) => assert_eq!(&*o.text, "2023-04-31"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn time_over_24_hours_is_an_interval_exception() {
    assert!(matches!(
        decode_time_bin(false, 1, 2, 0, 0, 0),
        Decoded::Exception(Value::Interval { .. })
    ));
    let time = temporal(ColumnType::MYSQL_TYPE_TIME, 0);
    let interval = |micros: i64| Value::Interval {
        months: 0,
        days: 0,
        micros,
    };
    let h = 3_600_000_000_i64;
    assert_eq!(
        decode_cell_bin(&time, &My::Time(false, 1, 2, 0, 0, 0), MIB),
        interval(26 * h)
    );
    assert_eq!(
        decode_cell_text(&time, Some(b"838:59:59"), MIB),
        interval(838 * h + 59 * 60_000_000 + 59_000_000)
    );
    // Negative times are durations too.
    assert_eq!(
        decode_cell_text(&time, Some(b"-01:30:00"), MIB),
        interval(-(h + 30 * 60_000_000))
    );
    assert_eq!(
        decode_cell_bin(&time, &My::Time(true, 0, 1, 30, 0, 0), MIB),
        interval(-(h + 30 * 60_000_000))
    );
    assert_eq!(
        decode_cell_text(&time, Some(b"-838:59:59.000000"), MIB),
        interval(-(838 * h + 59 * 60_000_000 + 59_000_000))
    );
}

#[test]
fn time_within_a_day_is_a_time_of_day() {
    let time = temporal(ColumnType::MYSQL_TYPE_TIME, 6);
    let t = |micros: i64| Value::Time(TimeVal::from_micros(micros));
    assert_eq!(
        decode_cell_text(&time, Some(b"13:14:15.000123"), MIB),
        t(((13 * 60 + 14) * 60 + 15) * 1_000_000 + 123)
    );
    assert_eq!(
        decode_cell_bin(&time, &My::Time(false, 0, 13, 14, 15, 123), MIB),
        t(((13 * 60 + 14) * 60 + 15) * 1_000_000 + 123)
    );
    assert_eq!(decode_cell_text(&time, Some(b"00:00:00"), MIB), t(0));
    // 24:00:00 is the end of the day, inside the 0-24 h range.
    assert_eq!(
        decode_cell_text(&time, Some(b"24:00:00"), MIB),
        Value::Time(TimeVal::END_OF_DAY)
    );
    assert_eq!(
        decode_cell_bin(&time, &My::Time(false, 1, 0, 0, 0, 0), MIB),
        Value::Time(TimeVal::END_OF_DAY)
    );
    assert!(matches!(
        decode_time_text("00:00:01"),
        Decoded::Temporal(1_000_000)
    ));
}

#[test]
fn decimal_65_30_is_big() {
    assert!(matches!(
        decode_decimal_text(format!("{}.{}", "9".repeat(35), "1".repeat(30))),
        Decoded::Decimal(d) if d.is_big()
    ));
    let c = decimal(67, 30, false);
    let text = format!("-{}.{}", "9".repeat(35), "1".repeat(30));
    match decode_cell_text(&c, Some(text.as_bytes()), MIB) {
        Value::Decimal(d) => {
            assert!(d.is_big());
            let mut s = String::new();
            d.write_canonical(&mut s);
            assert_eq!(s, text);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn decimals_keep_their_scale_and_sign() {
    let c = decimal(12, 2, false);
    let parsed = |s: &str| Value::Decimal(DecimalVal::parse(s).unwrap());
    assert_eq!(
        decode_cell_text(&c, Some(b"1234.50"), MIB),
        parsed("1234.50")
    );
    // The binary protocol sends a decimal as the same text.
    assert_eq!(
        decode_cell_bin(&c, &My::Bytes(b"-0.00".to_vec()), MIB),
        parsed("-0.00")
    );
    match decode_cell_text(&c, Some(b"-0.00"), MIB) {
        Value::Decimal(d) => assert!(d.is_negative() && d.scale() == 2),
        other => panic!("{other:?}"),
    }
    // A wider decimal in a Dec128 column.
    let wide = decimal(40, 10, false);
    assert_eq!(
        decode_cell_text(&wide, Some(b"123456789012345678901234567.0123456789"), MIB),
        parsed("123456789012345678901234567.0123456789")
    );
}

#[test]
fn integers_in_both_protocols_including_the_unsigned_extremes() {
    let big_u = flagged(ColumnType::MYSQL_TYPE_LONGLONG, ColumnFlags::UNSIGNED_FLAG);
    assert_eq!(
        decode_cell_text(&big_u, Some(b"18446744073709551615"), MIB),
        Value::UInt(u64::MAX)
    );
    assert_eq!(
        decode_cell_bin(&big_u, &My::UInt(u64::MAX), MIB),
        Value::UInt(u64::MAX)
    );
    // mysql_common hands a small unsigned BIGINT over as `Int`.
    assert_eq!(decode_cell_bin(&big_u, &My::Int(7), MIB), Value::UInt(7));
    let big = col(ColumnType::MYSQL_TYPE_LONGLONG);
    assert_eq!(
        decode_cell_text(&big, Some(b"-9223372036854775808"), MIB),
        Value::Int(i64::MIN)
    );
    assert_eq!(
        decode_cell_bin(&big, &My::Int(i64::MIN), MIB),
        Value::Int(i64::MIN)
    );
    let uint = flagged(ColumnType::MYSQL_TYPE_LONG, ColumnFlags::UNSIGNED_FLAG);
    assert_eq!(
        decode_cell_text(&uint, Some(b"4294967295"), MIB),
        Value::Int(4_294_967_295)
    );
    let year = flagged(ColumnType::MYSQL_TYPE_YEAR, ColumnFlags::UNSIGNED_FLAG);
    assert_eq!(
        decode_cell_text(&year, Some(b"2024"), MIB),
        Value::Int(2024)
    );
    assert_eq!(decode_cell_bin(&year, &My::Int(0), MIB), Value::Int(0));
}

#[test]
fn floats_keep_their_exact_value() {
    let f = col(ColumnType::MYSQL_TYPE_FLOAT);
    assert_eq!(
        decode_cell_bin(&f, &My::Float(0.1), MIB),
        Value::F32(0.1_f32)
    );
    assert_eq!(decode_cell_text(&f, Some(b"0.1"), MIB), Value::F32(0.1_f32));
    let d = col(ColumnType::MYSQL_TYPE_DOUBLE);
    assert_eq!(
        decode_cell_text(&d, Some(b"1.7976931348623157e308"), MIB),
        Value::F64(f64::MAX)
    );
    match decode_cell_text(&d, Some(b"-0"), MIB) {
        Value::F64(v) => assert!(v == 0.0 && v.is_sign_negative()),
        other => panic!("{other:?}"),
    }
}

#[test]
fn text_and_bytes_are_kept_byte_for_byte() {
    let t = text_col(ColumnType::MYSQL_TYPE_VAR_STRING, 40);
    assert_eq!(
        decode_cell_text(&t, Some("İstanbul ğüşıöç".as_bytes()), MIB),
        Value::Text("İstanbul ğüşıöç".into())
    );
    // Not UTF-8 (a latin1 result after SET NAMES latin1): kept as bytes, never replaced.
    assert_eq!(
        decode_cell_text(&t, Some(&[0x66, 0xFC, 0x72]), MIB),
        Value::Lossy(Arc::from(&[0x66_u8, 0xFC, 0x72][..]))
    );
    let b = bytes_col(ColumnType::MYSQL_TYPE_BLOB, u32::MAX);
    assert_eq!(
        decode_cell_bin(&b, &My::Bytes(vec![0, 0xFF, 0x10]), MIB),
        Value::Bytes(Arc::from(&[0_u8, 0xFF, 0x10][..]))
    );
    let j = bytes_col(ColumnType::MYSQL_TYPE_JSON, u32::MAX);
    assert_eq!(
        decode_cell_text(&j, Some(br#"{"a": [1, 2]}"#), MIB),
        Value::Json(r#"{"a": [1, 2]}"#.into())
    );
    let e = text_col(ColumnType::MYSQL_TYPE_STRING, 8).with_flags(ColumnFlags::ENUM_FLAG);
    assert_eq!(
        decode_cell_text(&e, Some(b"medium"), MIB),
        Value::Text("medium".into())
    );
}

#[test]
fn a_cell_over_the_cap_is_truncated_at_a_char_boundary() {
    let t = text_col(ColumnType::MYSQL_TYPE_BLOB, u32::MAX);
    let s = "ş".repeat(600_000); // 1.2 MB of two-byte characters
    match decode_cell_text(&t, Some(s.as_bytes()), MIB) {
        Value::Truncated(tr) => {
            assert_eq!(tr.total_len, Some(s.len() as u64));
            assert_eq!(tr.unit, LenUnit::Bytes);
            match &tr.prefix {
                Value::Text(p) => assert_eq!(p.len(), MIB as usize),
                other => panic!("{other:?}"),
            }
        }
        other => panic!("{other:?}"),
    }
    let b = bytes_col(ColumnType::MYSQL_TYPE_BLOB, u32::MAX);
    match decode_cell_bin(&b, &My::Bytes(vec![7; 5]), 3) {
        Value::Truncated(tr) => {
            assert_eq!(tr.total_len, Some(5));
            assert_eq!(tr.prefix, Value::Bytes(Arc::from(&[7_u8, 7, 7][..])));
        }
        other => panic!("{other:?}"),
    }
}

fn opaque_text(v: &Value) -> (String, String) {
    match v {
        Value::Opaque(o) => (o.type_name.to_string(), o.text.to_string()),
        other => panic!("{other:?}"),
    }
}

#[test]
fn bit_geometry_and_unknown_types_are_opaque_text() {
    let bit5 = col(ColumnType::MYSQL_TYPE_BIT).with_column_length(5);
    assert_eq!(
        opaque_text(&decode_cell_bin(&bit5, &My::Bytes(vec![0b10110]), MIB)),
        ("bit".into(), "10110".into())
    );
    let bit64 = col(ColumnType::MYSQL_TYPE_BIT).with_column_length(64);
    assert_eq!(
        opaque_text(&decode_cell_text(
            &bit64,
            Some(&[0x80, 0, 0, 0, 0, 0, 0, 1]),
            MIB
        ))
        .1,
        format!("1{}1", "0".repeat(62))
    );
    // A POINT(1 1) with SRID 0: the internal value (SRID, then WKB) as MySQL's HEX() prints it.
    let mut point = vec![0, 0, 0, 0, 1, 1, 0, 0, 0];
    point.extend_from_slice(&1.0_f64.to_le_bytes());
    point.extend_from_slice(&1.0_f64.to_le_bytes());
    let geom = bytes_col(ColumnType::MYSQL_TYPE_GEOMETRY, u32::MAX);
    assert_eq!(
        opaque_text(&decode_cell_bin(&geom, &My::Bytes(point), MIB)),
        (
            "geometry".into(),
            "000000000101000000000000000000F03F000000000000F03F".into()
        )
    );
    // Past the cap the hex text is truncated, its total length counted.
    match decode_cell_bin(&geom, &My::Bytes(vec![0xAB; 10]), 4) {
        Value::Truncated(tr) => {
            assert_eq!(tr.total_len, Some(20));
            assert_eq!(tr.prefix, Value::Text("ABAB".into()));
        }
        other => panic!("{other:?}"),
    }
    let vector = bytes_col(ColumnType::MYSQL_TYPE_VECTOR, 8);
    assert_eq!(
        opaque_text(&decode_cell_text(&vector, Some(b"[1,2]"), MIB)),
        ("vector".into(), "[1,2]".into())
    );
}

#[test]
fn null_is_null_in_every_column_kind() {
    for c in mysql_columns() {
        assert_eq!(
            decode_cell_text(&c.column, None, MIB),
            Value::Null,
            "{}",
            c.name
        );
        assert_eq!(
            decode_cell_bin(&c.column, &My::NULL, MIB),
            Value::Null,
            "{}",
            c.name
        );
    }
}

fn error_code(v: &Value) -> String {
    match v {
        Value::Error(e) => e.code.to_string(),
        other => panic!("not an error cell: {other:?}"),
    }
}

#[test]
fn bytes_no_server_sends_are_error_cells() {
    let int = col(ColumnType::MYSQL_TYPE_LONG);
    assert_eq!(
        error_code(&decode_cell_text(&int, Some(b"12a"), MIB)),
        "mysql-int"
    );
    let date = temporal(ColumnType::MYSQL_TYPE_DATE, 0);
    for bad in [
        &b"2024-13-01"[..],
        b"2024-01-32",
        b"2024-1-01",
        b"20240101",
        b"",
    ] {
        assert_eq!(
            error_code(&decode_cell_text(&date, Some(bad), MIB)),
            "mysql-date",
            "{bad:?}"
        );
    }
    assert_eq!(
        error_code(&decode_cell_bin(
            &date,
            &My::Date(2024, 1, 1, 10, 0, 0, 0),
            MIB
        )),
        "mysql-date"
    );
    let dt = temporal(ColumnType::MYSQL_TYPE_DATETIME, 6);
    for bad in [
        &b"2024-01-01 24:00:00"[..],
        b"2024-01-01 10:60:00",
        b"2024-01-01 10:00:00.1234567",
        b"2024-01-01T10:00:00",
    ] {
        assert_eq!(
            error_code(&decode_cell_text(&dt, Some(bad), MIB)),
            "mysql-date",
            "{bad:?}"
        );
    }
    assert_eq!(
        error_code(&decode_cell_bin(
            &dt,
            &My::Date(2024, 1, 1, 10, 0, 0, 1_000_000),
            MIB
        )),
        "mysql-date"
    );
    let time = temporal(ColumnType::MYSQL_TYPE_TIME, 0);
    for bad in [&b"10:60:00"[..], b"1:2:3", b"--1:00:00", b"10:00"] {
        assert_eq!(
            error_code(&decode_cell_text(&time, Some(bad), MIB)),
            "mysql-time",
            "{bad:?}"
        );
    }
    // Days beyond what i64 microseconds hold: an error, never an overflow.
    assert_eq!(
        error_code(&decode_cell_bin(
            &time,
            &My::Time(false, u32::MAX, 23, 59, 59, 999_999),
            MIB
        )),
        "mysql-time"
    );
    assert_eq!(
        error_code(&decode_cell_text(&decimal(12, 2, false), Some(b"1e5"), MIB)),
        "mysql-decimal"
    );
    assert_eq!(
        error_code(&decode_cell_text(
            &col(ColumnType::MYSQL_TYPE_DOUBLE),
            Some(b"x"),
            MIB
        )),
        "mysql-float"
    );
    // A BIT value is at most 8 bytes.
    let bit = col(ColumnType::MYSQL_TYPE_BIT).with_column_length(64);
    assert_eq!(
        error_code(&decode_cell_bin(&bit, &My::Bytes(vec![0; 9]), MIB)),
        "mysql-bit"
    );
    // A value of the wrong kind for its column (no real server sends one).
    assert_eq!(
        error_code(&decode_cell_bin(
            &int,
            &My::Date(2024, 1, 1, 0, 0, 0, 0),
            MIB
        )),
        "mysql-value"
    );
    assert_eq!(
        error_code(&decode_cell_bin(&date, &My::Int(3), MIB)),
        "mysql-value"
    );
}

/// A small deterministic generator (no dependency): xorshift64*.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn bytes(&mut self, max: usize) -> Vec<u8> {
        let len = (self.next() as usize) % (max + 1);
        (0..len).map(|_| self.next() as u8).collect()
    }
}

#[test]
fn hostile_bytes_never_panic_in_any_column_kind() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let columns = mysql_columns();
    for _ in 0..4_000 {
        for c in &columns {
            let raw = rng.bytes(40);
            let _ = decode_cell_text(&c.column, Some(&raw), 16);
            let bin = match rng.next() % 6 {
                0 => My::Bytes(raw),
                1 => My::Int(rng.next() as i64),
                2 => My::UInt(rng.next()),
                3 => My::Double(f64::from_bits(rng.next())),
                4 => {
                    let n = rng.next();
                    My::Date(
                        n as u16,
                        (n >> 16) as u8,
                        (n >> 24) as u8,
                        (n >> 32) as u8,
                        (n >> 40) as u8,
                        (n >> 48) as u8,
                        rng.next() as u32,
                    )
                }
                _ => {
                    let n = rng.next();
                    My::Time(
                        n & 1 == 0,
                        (n >> 8) as u32,
                        (n >> 40) as u8,
                        (n >> 48) as u8,
                        (n >> 56) as u8,
                        rng.next() as u32,
                    )
                }
            };
            let _ = decode_cell_bin(&c.column, &bin, 16);
        }
    }
}

#[test]
fn only_the_types_mysql_common_decodes_pass_the_binary_check() {
    use ColumnType::*;
    for t in [
        MYSQL_TYPE_TIMESTAMP2,
        MYSQL_TYPE_DATETIME2,
        MYSQL_TYPE_TIME2,
        MYSQL_TYPE_TYPED_ARRAY,
        MYSQL_TYPE_UNKNOWN,
    ] {
        assert!(!binary_decodable(t), "{t:?}");
    }
    for c in mysql_columns() {
        let t = c.column.column_type();
        if t != MYSQL_TYPE_TIME2 {
            assert!(binary_decodable(t), "{}", c.name);
        }
    }
}

#[test]
fn the_schema_keeps_names_native_types_and_origins() {
    let columns = [
        Column::new(ColumnType::MYSQL_TYPE_LONG)
            .with_schema(b"hv_test")
            .with_table(b"o")
            .with_org_table(b"orders")
            .with_name(b"order id")
            .with_org_name(b"id")
            .with_flags(ColumnFlags::NOT_NULL_FLAG | ColumnFlags::PRI_KEY_FLAG)
            .with_column_length(11)
            .with_character_set(BINARY_CHARSET),
        // A computed column has no origin.
        Column::new(ColumnType::MYSQL_TYPE_LONGLONG).with_name(b"COUNT(*)"),
        // Duplicate names keep their order.
        Column::new(ColumnType::MYSQL_TYPE_LONGLONG).with_name(b"COUNT(*)"),
    ];
    let plan = Plan::for_columns(&columns);
    let cols = &plan.schema.columns;
    assert_eq!(cols.len(), 3);
    assert_eq!(&*cols[0].name, "order id");
    assert_eq!(
        cols[0].origin,
        Some(OriginHint::MySql {
            schema: "hv_test".into(),
            table: "orders".into(),
            column: "id".into(),
        })
    );
    assert_eq!(
        cols[0].native,
        NativeType::MySql {
            column_type: ColumnType::MYSQL_TYPE_LONG as u8,
            flags: (ColumnFlags::NOT_NULL_FLAG | ColumnFlags::PRI_KEY_FLAG).bits(),
            charset: BINARY_CHARSET,
            length: 11,
            decimals: 0,
        }
    );
    assert_eq!(cols[1].origin, None);
    assert_eq!(&*cols[2].name, "COUNT(*)");
}
