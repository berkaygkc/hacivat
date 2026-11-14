//! M3a Task 16: `raw_text`, the locale-free canonical text (I18N-11) that copy, sort keys and SQL
//! values use, and its independence from the process locale.

use std::borrow::Cow;
use std::process::Command;
use std::sync::Arc;

use hacivat_proto::DbmsKind;
use hacivat_values::valfmt::{
    BinaryDisplay, ColumnFacts, DisplaySettings, DisplayZone, Purpose, RawText, format_cell,
    raw_text,
};
use hacivat_values::{
    DateVal, DecimalVal, ErrorVal, LenUnit, LogicalType, OpaqueVal, SpecialVal, TimeVal,
    TruncatedVal, TsVal, Value,
};
use proptest::prelude::*;

fn facts_for(logical: &LogicalType, engine: DbmsKind) -> ColumnFacts<'_> {
    ColumnFacts {
        logical,
        native_name: "x",
        id_like: false,
        binary: BinaryDisplay::Detect,
        engine,
    }
}

/// The logical type a PostgreSQL column of this value would have.
fn logical_of(v: &Value) -> LogicalType {
    match v {
        Value::Int(_) | Value::UInt(_) => LogicalType::Int {
            bits: 64,
            signed: true,
        },
        Value::F32(_) => LogicalType::Float { bits: 32 },
        Value::F64(_) => LogicalType::Float { bits: 64 },
        Value::Decimal(_) => LogicalType::Decimal {
            precision: None,
            scale: None,
        },
        Value::Bytes(_) => LogicalType::Bytes { max: None },
        Value::Date(_) | Value::Special(_) => LogicalType::Date,
        Value::Time(_) => LogicalType::Time {
            precision: 6,
            tz: false,
        },
        Value::TimeTz(..) => LogicalType::Time {
            precision: 6,
            tz: true,
        },
        Value::Timestamp(_) => LogicalType::Timestamp {
            precision: 6,
            tz: false,
        },
        Value::TimestampTz { .. } => LogicalType::Timestamp {
            precision: 6,
            tz: true,
        },
        Value::Interval { .. } => LogicalType::Interval,
        _ => LogicalType::Text {
            max_chars: None,
            fixed: false,
        },
    }
}

fn raw_in(v: &Value, logical: &LogicalType, engine: DbmsKind) -> Option<String> {
    match raw_text(v.view(), &facts_for(logical, engine)) {
        RawText::Null => None,
        RawText::Text(t) => Some(t.into_owned()),
    }
}

fn raw(v: &Value) -> String {
    raw_in(v, &logical_of(v), DbmsKind::Postgres).expect("not NULL")
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

fn date(y: i32, m: u8, d: u8) -> Value {
    Value::Date(DateVal::from_civil(y, m, d).expect("a date"))
}

/// The values and raw texts that must not depend on anything but the value (and the engine's
/// spelling of its specials).
fn canonical_cases() -> Vec<(Value, &'static str)> {
    vec![
        (dec("1.50"), "1.50"),
        (dec("-0.00"), "-0.00"),
        (dec("1234567.50"), "1234567.50"),
        (dec("007.50"), "7.50"),
        (
            dec("123456789012345678901234567890123456789012345.5"),
            "123456789012345678901234567890123456789012345.5",
        ),
        (Value::Int(-42), "-42"),
        (Value::Int(i64::MIN), "-9223372036854775808"),
        (Value::UInt(u64::MAX), "18446744073709551615"),
        (Value::F32(0.1), "0.1"),
        (Value::F64(-0.0), "-0"),
        (Value::F64(1e15), "1e+15"),
        (Value::F64(1e-5), "1e-05"),
        (Value::F64(1234.5), "1234.5"),
        (Value::F64(f64::MAX), "1.7976931348623157e+308"),
        (Value::F64(5e-324), "5e-324"),
        (Value::F64(f64::NAN), "NaN"),
        (Value::F64(f64::INFINITY), "Infinity"),
        (Value::F64(f64::NEG_INFINITY), "-Infinity"),
        (Value::Bytes(Arc::from(&[0x00, 0xff][..])), "\\x00ff"),
        (Value::Bytes(Arc::from(&b""[..])), "\\x"),
        (
            Value::Uuid([
                0x6F, 0x1E, 0x9A, 0x0C, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 0, 0, 1,
            ]),
            "6f1e9a0c-0000-4000-8000-000000000001",
        ),
        (date(2026, 11, 13), "2026-11-13"),
        (date(-43, 3, 15), "0044-03-15 BC"),
        (
            Value::TimestampTz {
                utc: ts("2026-11-13T07:00:00.123456Z"),
                offset: Some(10_800),
            },
            "2026-11-13 10:00:00.123456+03",
        ),
        (
            Value::TimestampTz {
                utc: ts("2026-11-13T07:00:00.123456Z"),
                offset: None,
            },
            "2026-11-13 07:00:00.123456+00",
        ),
        (
            Value::Timestamp(ts("2026-11-13T10:00:00Z")),
            "2026-11-13 10:00:00",
        ),
        (Value::Time(TimeVal::END_OF_DAY), "24:00:00"),
        (
            Value::TimeTz(TimeVal::from_micros(36_000_500_000), 19_800),
            "10:00:00.5+05:30",
        ),
        (
            Value::Interval {
                months: 14,
                days: -1,
                micros: 3_600_000_000,
            },
            "1 year 2 mons -1 days +01:00:00",
        ),
        (Value::Bool(true), "true"),
        (Value::Bool(false), "false"),
        (Value::Special(SpecialVal::PosInfinity), "infinity"),
        (Value::Special(SpecialVal::NegInfinity), "-infinity"),
        (Value::Text("NULL".into()), "NULL"),
        (Value::Text("İzmir\tılık\n".into()), "İzmir\tılık\n"),
        (
            Value::Json("{\"b\":1.50,\"a\":1e3}".into()),
            "{\"b\":1.50,\"a\":1e3}",
        ),
        (Value::Lossy(Arc::from(&b"caf\xe9"[..])), "caf\\xe9"),
        (Value::Missing, ""),
        (Value::Default, "DEFAULT"),
    ]
}

#[test]
fn raw_forms_are_canonical() {
    for (v, want) in canonical_cases() {
        assert_eq!(raw(&v), want, "{v:?}");
    }
}

#[test]
fn raw_null_is_null_and_the_text_null_is_text() {
    let text = logical_of(&Value::Text("".into()));
    assert_eq!(raw_in(&Value::Null, &text, DbmsKind::Postgres), None);
    assert_eq!(
        raw_in(&Value::Text("NULL".into()), &text, DbmsKind::Postgres).as_deref(),
        Some("NULL")
    );
}

#[test]
fn raw_text_borrows_text_payloads() {
    let text = logical_of(&Value::Text("".into()));
    let facts = facts_for(&text, DbmsKind::Postgres);
    let opaque = Value::Opaque(Arc::new(OpaqueVal {
        type_name: "_int4".into(),
        text: "{1,2,3}".into(),
    }));
    let partial = Value::Special(SpecialVal::PartialZeroDate("2026-00-13".into()));
    for v in [
        Value::Text("abc".into()),
        Value::Json("[1]".into()),
        Value::Xml("<a/>".into()),
        opaque,
        partial,
        Value::Bool(true),
    ] {
        assert!(
            matches!(raw_text(v.view(), &facts), RawText::Text(Cow::Borrowed(_))),
            "{v:?} is borrowed"
        );
    }
}

#[test]
fn raw_bytes_are_always_hex_whatever_the_display() {
    let v = Value::Bytes(Arc::from(&[0x6f; 16][..]));
    let logical = LogicalType::Bytes { max: None };
    for binary in [
        BinaryDisplay::Detect,
        BinaryDisplay::Uuid,
        BinaryDisplay::Text,
        BinaryDisplay::Hex,
    ] {
        let facts = ColumnFacts {
            binary,
            ..facts_for(&logical, DbmsKind::Postgres)
        };
        let RawText::Text(t) = raw_text(v.view(), &facts) else {
            panic!("bytes are not NULL");
        };
        assert_eq!(t, "\\x6f6f6f6f6f6f6f6f6f6f6f6f6f6f6f6f");
    }
}

#[test]
fn raw_specials_follow_the_engine_and_column() {
    let numeric = LogicalType::Decimal {
        precision: None,
        scale: None,
    };
    let float = LogicalType::Float { bits: 64 };
    let sp = Value::Special;
    let pg = DbmsKind::Postgres;
    assert_eq!(
        raw_in(&sp(SpecialVal::NaN), &numeric, pg).as_deref(),
        Some("NaN")
    );
    assert_eq!(
        raw_in(&sp(SpecialVal::PosInfinity), &numeric, pg).as_deref(),
        Some("Infinity")
    );
    assert_eq!(
        raw_in(&sp(SpecialVal::NegInfinity), &float, pg).as_deref(),
        Some("-Infinity")
    );
    assert_eq!(
        raw_in(&Value::F64(f64::INFINITY), &float, DbmsKind::Sqlite).as_deref(),
        Some("Inf")
    );
    let dt = LogicalType::Timestamp {
        precision: 0,
        tz: false,
    };
    assert_eq!(
        raw_in(&sp(SpecialVal::ZeroDate), &dt, DbmsKind::MySql).as_deref(),
        Some("0000-00-00 00:00:00")
    );
    assert_eq!(
        raw_in(
            &sp(SpecialVal::ZeroDate),
            &LogicalType::Date,
            DbmsKind::MySql
        )
        .as_deref(),
        Some("0000-00-00")
    );
}

#[test]
fn raw_of_truncated_error_and_lossy_values() {
    let text = logical_of(&Value::Text("".into()));
    let t = Value::Truncated(Box::new(TruncatedVal {
        prefix: Value::Text("abc".into()),
        total_len: Some(5 << 20),
        unit: LenUnit::Bytes,
    }));
    assert_eq!(
        raw_in(&t, &text, DbmsKind::Postgres).as_deref(),
        Some("abc")
    );
    let e = Value::Error(Arc::new(ErrorVal {
        code: "decode".into(),
        message: "bad length".into(),
    }));
    assert_eq!(
        raw_in(&e, &text, DbmsKind::Postgres).as_deref(),
        Some("bad length")
    );
    let lossy = Value::Lossy(Arc::from(&b"\xff\xfeok\xc3"[..]));
    assert_eq!(
        raw_in(&lossy, &text, DbmsKind::Postgres).as_deref(),
        Some("\\xff\\xfeok\\xc3")
    );
}

/// The raw texts of `canonical_cases` and the `tr-TR` display of a few values: neither depends
/// on the process locale. The `turkey-test` CI job runs this whole suite under
/// `LANG=tr_TR.UTF-8`; on any other run the test also re-runs itself in a child process with a
/// Turkish locale environment and checks that the child passed.
#[test]
fn raw_text_is_locale_free_under_turkish_process_locale() {
    for (v, want) in canonical_cases() {
        assert_eq!(raw(&v), want, "{v:?}");
    }
    let s = DisplaySettings::with_zone("tr-TR", DisplayZone::utc());
    let show = |v: &Value| {
        let logical = logical_of(v);
        format_cell(
            v.view(),
            &facts_for(&logical, DbmsKind::Postgres),
            &s,
            Purpose::Cell,
        )
        .text
    };
    assert_eq!(show(&dec("1234567.50")), "1.234.567,50");
    assert_eq!(show(&Value::F64(-1.5e-7)), "-1,5e-07");
    assert_eq!(show(&Value::Int(-1_234_567)), "-1.234.567");
    assert_eq!(show(&date(2026, 11, 13)), "2026-11-13");
    assert_eq!(show(&Value::Bool(true)), "true");

    if std::env::var_os("HACIVAT_T16_TR_CHILD").is_some() {
        return;
    }
    let name = "raw_text_is_locale_free_under_turkish_process_locale";
    let out = Command::new(std::env::current_exe().expect("the test binary"))
        .args(["--exact", name, "--test-threads=1"])
        .env("LANG", "tr_TR.UTF-8")
        .env("LC_ALL", "tr_TR.UTF-8")
        .env("LC_NUMERIC", "tr_TR.UTF-8")
        .env("LC_TIME", "tr_TR.UTF-8")
        .env("HACIVAT_T16_TR_CHILD", "1")
        .output()
        .expect("the child runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "the Turkish-locale child failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("1 passed"),
        "the child ran the one test: {stdout}"
    );
}

proptest! {
    #[test]
    fn raw_floats_round_trip_exactly(bits in any::<u64>()) {
        let x = f64::from_bits(bits);
        prop_assume!(x.is_finite());
        let text = raw(&Value::F64(x));
        let back: f64 = text.parse().expect("raw float text parses");
        prop_assert_eq!(back.to_bits(), x.to_bits(), "{}", text);
    }

    #[test]
    fn raw_f32_round_trips_exactly(bits in any::<u32>()) {
        let x = f32::from_bits(bits);
        prop_assume!(x.is_finite());
        let text = raw(&Value::F32(x));
        let back: f32 = text.parse().expect("raw float text parses");
        prop_assert_eq!(back.to_bits(), x.to_bits(), "{}", text);
    }

    /// Display and raw text agree digit for digit (no float on the path): the `en-US` display
    /// without its group separators, and the `tr-TR` one with its separators swapped back, are
    /// the raw text.
    #[test]
    fn decimal_display_is_the_raw_text_regrouped(
        neg in any::<bool>(),
        int in "[0-9]{1,45}",
        frac in proptest::option::of("[0-9]{1,20}"),
    ) {
        let text = match &frac {
            Some(f) => format!("{}{int}.{f}", if neg { "-" } else { "" }),
            None => format!("{}{int}", if neg { "-" } else { "" }),
        };
        let v = dec(&text);
        let canonical = raw(&v);
        prop_assert_eq!(DecimalVal::parse(&canonical).map(Value::Decimal), Ok(v.clone()));
        let logical = logical_of(&v);
        let facts = facts_for(&logical, DbmsKind::Postgres);
        let en = DisplaySettings::with_zone("en-US", DisplayZone::utc());
        let tr = DisplaySettings::with_zone("tr-TR", DisplayZone::utc());
        let en_text = format_cell(v.view(), &facts, &en, Purpose::Cell).text;
        let tr_text = format_cell(v.view(), &facts, &tr, Purpose::Cell).text;
        prop_assert_eq!(en_text.replace(',', ""), canonical.clone());
        prop_assert_eq!(tr_text.replace('.', "").replace(',', "."), canonical);
    }
}
