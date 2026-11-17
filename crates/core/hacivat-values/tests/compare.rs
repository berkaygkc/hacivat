//! M3c Task 36 (HV-GRID-07, R-M3-70): the local sort's comparator. Text compares by the
//! column's database collation mapped to ICU (never the UI locale), else by code point; exact
//! decimals never pass through a float; temporal values compare by instant; NULLs sit where
//! the engine puts them. Every assertion holds under any `LANG` (the `turkey-test` job runs
//! this file under `tr_TR.UTF-8`).

use std::cmp::Ordering;
use std::sync::Arc;

use hacivat_proto::DbmsKind;
use hacivat_values::valfmt::compare::{
    CollationUse, Resolved, Strength, comparator_for, resolve_collation,
};
use hacivat_values::valfmt::{BinaryDisplay, ColumnFacts};
use hacivat_values::{
    DecimalVal, ErrorVal, LogicalType, SpecialVal, TimeVal, TsVal, Value, ValueRef,
};

// ---- helpers ----------------------------------------------------------------------------

fn text_type() -> LogicalType {
    LogicalType::Text {
        max_chars: None,
        fixed: false,
    }
}

fn facts(logical: &LogicalType, engine: DbmsKind) -> ColumnFacts<'_> {
    ColumnFacts {
        logical,
        native_name: "",
        id_like: false,
        binary: BinaryDisplay::Detect,
        engine,
    }
}

/// `values` sorted ascending (stable) by `logical`'s comparator on `engine` with `collation`.
fn sorted_values(
    values: &[Value],
    logical: &LogicalType,
    engine: DbmsKind,
    collation: Option<&str>,
) -> Vec<Value> {
    let cmp = comparator_for(&facts(logical, engine), collation);
    let mut out = values.to_vec();
    out.sort_by(|a, b| cmp.cmp(a.view(), b.view()));
    out
}

/// Texts sorted ascending with `collation` on an engine that does not say which (every
/// engine's spelling of a collation is tried; ## m3c-t36 fix round 1: `sorted_on` names one).
fn sorted(words: &[&str], collation: Option<&str>) -> Vec<String> {
    sorted_on(DbmsKind::Unknown, words, collation)
}

/// Texts sorted ascending on `engine` with `collation`.
fn sorted_on(engine: DbmsKind, words: &[&str], collation: Option<&str>) -> Vec<String> {
    let values: Vec<Value> = words.iter().map(|w| Value::Text((*w).into())).collect();
    sorted_values(&values, &text_type(), engine, collation)
        .into_iter()
        .map(|v| match v {
            Value::Text(t) => t.to_string(),
            other => panic!("not text: {other:?}"),
        })
        .collect()
}

fn tr_collation() -> Option<&'static str> {
    Some("tr-x-icu")
}

fn dec(text: &str) -> Value {
    Value::Decimal(DecimalVal::parse(text).expect("a decimal"))
}

fn decimal_type() -> LogicalType {
    LogicalType::Decimal {
        precision: None,
        scale: None,
    }
}

/// Decimals sorted ascending (canonical text).
fn sorted_dec(texts: &[&str]) -> Vec<String> {
    let values: Vec<Value> = texts.iter().map(|t| dec(t)).collect();
    sorted_values(&values, &decimal_type(), DbmsKind::Postgres, None)
        .into_iter()
        .map(|v| match v {
            Value::Decimal(d) => {
                let mut s = String::new();
                d.write_canonical(&mut s);
                s
            }
            other => panic!("not a decimal: {other:?}"),
        })
        .collect()
}

fn texts(values: &[Value]) -> Vec<String> {
    values
        .iter()
        .map(|v| match v {
            Value::Null => "NULL".to_string(),
            Value::Text(t) => t.to_string(),
            other => format!("{other:?}"),
        })
        .collect()
}

// ---- the plan's tests ---------------------------------------------------------------------

/// The verified Turkish order (alg/22 HV-GRID-07, verification/01 #39–#40): dotless ı before
/// I, dotted i before İ.
#[test]
fn turkish_collation_sorts_ilik_irmak_istek_izmir() {
    assert_eq!(
        sorted(&["İzmir", "ılık", "Irmak", "istek"], tr_collation()),
        ["ılık", "Irmak", "istek", "İzmir"]
    );
    // The same words under every Turkish spelling of the collation the engines use.
    for name in [
        "tr-x-icu",
        "tr-TR-x-icu",
        "tr_TR.utf8",
        "tr_TR.UTF-8",
        "utf8mb4_tr_0900_ai_ci",
        "utf8mb4_tr_0900_as_cs",
        "utf8mb4_turkish_ci",
        "utf8mb4_uca1400_turkish_ai_ci",
    ] {
        assert_eq!(
            sorted(&["İzmir", "ılık", "Irmak", "istek"], Some(name)),
            ["ılık", "Irmak", "istek", "İzmir"],
            "{name}"
        );
    }
}

/// Without a collation that maps (unknown, none, or one with no equivalent here) text compares
/// by code point, and the comparator says which case it is; never by the UI locale.
#[test]
fn code_point_order_without_a_mapped_collation() {
    let words = ["İzmir", "ılık", "Irmak", "istek", "Zeytin", "armut"];
    // Code points: I (U+49) < Z (U+5A) < a < i < İ (U+130) < ı (U+131).
    let expected = ["Irmak", "Zeytin", "armut", "istek", "İzmir", "ılık"];
    assert_eq!(sorted(&words, None), expected);
    for name in [
        "C",
        "POSIX",
        "ucs_basic",
        "utf8mb4_bin",
        "BINARY",
        "default",
    ] {
        assert_eq!(sorted(&words, Some(name)), expected, "{name}");
    }
    let cmp = |collation: Option<&str>| {
        comparator_for(&facts(&text_type(), DbmsKind::Unknown), collation)
            .collation()
            .clone()
    };
    assert_eq!(cmp(None), CollationUse::Unknown);
    assert_eq!(
        cmp(Some("C")),
        CollationUse::CodePoint {
            name: Arc::from("C")
        }
    );
    assert_eq!(
        cmp(Some("utf8mb4_bin")),
        CollationUse::CodePoint {
            name: Arc::from("utf8mb4_bin")
        }
    );
    // No ICU equivalent: the fallback names the collation it could not follow.
    for name in ["utf8mb4_general_ci", "NOCASE", "default", "xx_nowhere_ci"] {
        assert_eq!(
            cmp(Some(name)),
            CollationUse::Fallback {
                name: Arc::from(name)
            },
            "{name}"
        );
    }
    assert_eq!(
        cmp(Some("tr-x-icu")),
        CollationUse::Mapped {
            name: Arc::from("tr-x-icu")
        }
    );
    // The process locale plays no part: the same order whatever `LANG` says (the turkey-test
    // job runs this under tr_TR.UTF-8).
    assert_eq!(sorted(&["i", "I", "ı", "İ"], None), ["I", "i", "İ", "ı"]);
}

/// Exact decimals never pass through a float: two values a binary float cannot tell apart sort
/// apart, equal values of different scales keep their order (a stable sort sees them equal),
/// and values past 38 digits compare by their digits.
#[test]
fn decimals_sort_exactly_never_through_f64() {
    assert_eq!(
        sorted_dec(&["0.30000000000000000001", "0.3"]),
        ["0.3", "0.30000000000000000001"]
    );
    // Equal values keep model order (alg/22: `[1.50, 1.5, 10]`).
    assert_eq!(sorted_dec(&["1.50", "1.5", "10"]), ["1.50", "1.5", "10"]);
    assert_eq!(sorted_dec(&["1.5", "1.50", "10"]), ["1.5", "1.50", "10"]);
    // Signs, -0.00 = 0, and the integer part's length.
    assert_eq!(
        sorted_dec(&["-1", "0.00", "-0.00", "-10.5", "2", "-0.001"]),
        ["-10.5", "-1", "-0.001", "0.00", "-0.00", "2"]
    );
    // Past 38 digits (held as text) against small values.
    let big = "123456789012345678901234567890123456789012.5";
    let big_neg = "-123456789012345678901234567890123456789012.5";
    assert_eq!(
        sorted_dec(&[
            big,
            "99999999999999999999999999999999999999",
            big_neg,
            "0.1"
        ]),
        [
            big_neg,
            "0.1",
            "99999999999999999999999999999999999999",
            big
        ]
    );
    // Two values past 38 digits that differ only in the last fractional digit.
    let a = "1234567890123456789012345678901234567890.10";
    let b = "1234567890123456789012345678901234567890.1000000000000000000001";
    assert_eq!(sorted_dec(&[b, a]), [a, b]);
    // Integers against decimals in one column compare exactly too.
    let cmp = comparator_for(&facts(&decimal_type(), DbmsKind::Postgres), None);
    let d = DecimalVal::parse("9007199254740993.5").unwrap();
    assert_eq!(
        cmp.cmp(
            ValueRef::Int(9_007_199_254_740_993),
            ValueRef::Decimal(d.view())
        ),
        Ordering::Less
    );
    let d = DecimalVal::parse("18446744073709551615.0").unwrap();
    assert_eq!(
        cmp.cmp(ValueRef::UInt(u64::MAX), ValueRef::Decimal(d.view())),
        Ordering::Equal
    );
}

/// NULLs sit where the engine sorts them: PostgreSQL (and Oracle) last ascending; MySQL,
/// MariaDB, SQLite and SQL Server first (alg/22's cases). The comparator is ascending; a
/// descending sort reverses it, NULLs included, as `ORDER BY … DESC` does.
#[test]
fn nulls_follow_the_engine_default() {
    let values = [
        Value::Text("b".into()),
        Value::Text("A".into()),
        Value::Null,
        Value::Text("a".into()),
    ];
    assert_eq!(
        texts(&sorted_values(
            &values,
            &text_type(),
            DbmsKind::Postgres,
            None
        )),
        ["A", "a", "b", "NULL"]
    );
    for engine in [
        DbmsKind::MySql,
        DbmsKind::MariaDb,
        DbmsKind::Sqlite,
        DbmsKind::MsSql,
    ] {
        assert_eq!(
            texts(&sorted_values(&values, &text_type(), engine, None)),
            ["NULL", "A", "a", "b"],
            "{engine:?}"
        );
    }
    assert_eq!(
        texts(&sorted_values(
            &values,
            &text_type(),
            DbmsKind::Oracle,
            None
        )),
        ["A", "a", "b", "NULL"]
    );
    // A cell the row did not carry sorts with the NULLs.
    let cmp = comparator_for(&facts(&text_type(), DbmsKind::Postgres), None);
    assert_eq!(cmp.cmp(ValueRef::Missing, ValueRef::Null), Ordering::Equal);
}

// ---- more cases ---------------------------------------------------------------------------

/// What each spelling of a collation resolves to.
#[test]
fn collation_names_resolve_per_engine() {
    let icu = |name: &str| match resolve_collation(DbmsKind::Unknown, name) {
        Resolved::Icu { locale, strength } => Some((locale, strength)),
        _ => None,
    };
    assert_eq!(icu("tr-x-icu"), Some(("tr".into(), Strength::Tertiary)));
    assert_eq!(
        icu("tr-TR-x-icu"),
        Some(("tr-TR".into(), Strength::Tertiary))
    );
    assert_eq!(icu("und-x-icu"), Some(("und".into(), Strength::Tertiary)));
    assert_eq!(
        icu("tr_TR.utf8"),
        Some(("tr-TR".into(), Strength::Tertiary))
    );
    assert_eq!(icu("de_DE"), Some(("de-DE".into(), Strength::Tertiary)));
    assert_eq!(
        icu("utf8mb4_tr_0900_ai_ci"),
        Some(("tr".into(), Strength::Primary))
    );
    assert_eq!(
        icu("utf8mb4_tr_0900_as_ci"),
        Some(("tr".into(), Strength::Secondary))
    );
    assert_eq!(
        icu("utf8mb4_tr_0900_as_cs"),
        Some(("tr".into(), Strength::Tertiary))
    );
    assert_eq!(
        icu("utf8mb4_0900_ai_ci"),
        Some(("und".into(), Strength::Primary))
    );
    assert_eq!(
        icu("utf8mb4_unicode_ci"),
        Some(("und".into(), Strength::Primary))
    );
    assert_eq!(
        icu("utf8mb4_turkish_ci"),
        Some(("tr".into(), Strength::Primary))
    );
    assert_eq!(
        icu("latin5_turkish_ci"),
        Some(("tr".into(), Strength::Primary))
    );
    assert_eq!(
        icu("utf8mb4_uca1400_ai_ci"),
        Some(("und".into(), Strength::Primary))
    );
    assert_eq!(
        icu("Turkish_CI_AS"),
        Some(("tr".into(), Strength::Secondary))
    );
    assert_eq!(
        icu("Turkish_100_CS_AS"),
        Some(("tr".into(), Strength::Tertiary))
    );
    assert_eq!(
        icu("und-u-ks-level2"),
        Some(("und".into(), Strength::Secondary))
    );
    for name in [
        "C",
        "POSIX",
        "C.UTF-8",
        "C.utf8",
        "ucs_basic",
        "pg_c_utf8",
        "utf8mb4_bin",
        "utf8mb4_0900_bin",
        "binary",
        "BINARY",
        "Latin1_General_BIN2",
    ] {
        assert_eq!(
            resolve_collation(DbmsKind::Unknown, name),
            Resolved::CodePoint,
            "{name}"
        );
    }
    for name in [
        "default",
        "NOCASE",
        "RTRIM",
        "utf8mb4_general_ci",
        "",
        "xx_nowhere_ci",
        "tr",
    ] {
        assert_eq!(
            resolve_collation(DbmsKind::Unknown, name),
            Resolved::NoEquivalent,
            "{name}"
        );
    }
}

/// Case and accents follow the strength: an accent-insensitive collation sees `e` and `é` as
/// equal (the stable sort keeps their order), a case-sensitive one orders them.
#[test]
fn strength_follows_the_collation() {
    assert_eq!(
        sorted(&["é", "e", "E"], Some("utf8mb4_0900_ai_ci")),
        ["é", "e", "E"]
    );
    assert_eq!(
        sorted(&["é", "E", "e"], Some("utf8mb4_0900_as_cs")),
        ["e", "E", "é"]
    );
    assert_eq!(
        sorted(&["b", "A", "a", "B"], Some("und-x-icu")),
        ["a", "A", "b", "B"]
    );
}

/// Floats: NaN after +infinity (PostgreSQL's order), -0 equal to 0; specials around the
/// numbers; integers of both signs and widths compare as numbers.
#[test]
fn numbers_compare_as_numbers() {
    let float = LogicalType::Float { bits: 64 };
    let nan = SpecialVal::NaN;
    let values = [
        Value::F64(1.5),
        Value::F64(f64::NAN),
        Value::F64(f64::INFINITY),
        Value::F64(-0.0),
        Value::F64(f64::NEG_INFINITY),
        Value::F64(0.0),
        Value::F64(-2.0),
    ];
    let out = sorted_values(&values, &float, DbmsKind::Postgres, None);
    let shown: Vec<String> = out
        .iter()
        .map(|v| match v {
            Value::F64(f) => format!("{f}"),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(shown, ["-inf", "-2", "-0", "0", "1.5", "inf", "NaN"]);
    let cmp = comparator_for(&facts(&decimal_type(), DbmsKind::Postgres), None);
    let one = DecimalVal::parse("1").unwrap();
    assert_eq!(
        cmp.cmp(
            ValueRef::Special(&nan),
            ValueRef::Special(&SpecialVal::PosInfinity)
        ),
        Ordering::Greater
    );
    assert_eq!(
        cmp.cmp(
            ValueRef::Special(&SpecialVal::NegInfinity),
            ValueRef::Decimal(one.view())
        ),
        Ordering::Less
    );
    assert_eq!(
        cmp.cmp(
            ValueRef::Special(&SpecialVal::PosInfinity),
            ValueRef::Decimal(one.view())
        ),
        Ordering::Greater
    );
    let int = LogicalType::Int {
        bits: 64,
        signed: false,
    };
    let cmp = comparator_for(&facts(&int, DbmsKind::MySql), None);
    assert_eq!(
        cmp.cmp(ValueRef::UInt(u64::MAX), ValueRef::Int(-1)),
        Ordering::Greater
    );
    assert_eq!(
        cmp.cmp(ValueRef::Int(5), ValueRef::UInt(5)),
        Ordering::Equal
    );
    assert_eq!(
        cmp.cmp(ValueRef::Bool(false), ValueRef::Bool(true)),
        Ordering::Less
    );
}

/// Temporal values compare by instant: a `timestamptz` by its UTC instant whatever offset the
/// server sent; a `timetz` by its UTC time; MySQL zero dates before every date.
#[test]
fn temporal_values_compare_by_instant() {
    let tz = LogicalType::Timestamp {
        precision: 6,
        tz: true,
    };
    let cmp = comparator_for(&facts(&tz, DbmsKind::Postgres), None);
    // 10:00 UTC shown at +03:00 and 09:00 UTC shown at -05:00 (later on the wall clock).
    let a = ValueRef::TimestampTz {
        utc: TsVal {
            secs: 36_000,
            nanos: 0,
        },
        offset: Some(3 * 3600),
    };
    let b = ValueRef::TimestampTz {
        utc: TsVal {
            secs: 32_400,
            nanos: 0,
        },
        offset: Some(-5 * 3600),
    };
    assert_eq!(cmp.cmp(a, b), Ordering::Greater);
    let time = LogicalType::Time {
        precision: 6,
        tz: true,
    };
    let cmp = comparator_for(&facts(&time, DbmsKind::Postgres), None);
    // 12:00+03 is 09:00 UTC; 10:00+00 is 10:00 UTC.
    let noon = TimeVal {
        nanos_of_day: 12 * 3_600_000_000_000,
    };
    let ten = TimeVal {
        nanos_of_day: 10 * 3_600_000_000_000,
    };
    assert_eq!(
        cmp.cmp(ValueRef::TimeTz(noon, 3 * 3600), ValueRef::TimeTz(ten, 0)),
        Ordering::Less
    );
    let date = LogicalType::Date;
    let cmp = comparator_for(&facts(&date, DbmsKind::MySql), None);
    assert_eq!(
        cmp.cmp(
            ValueRef::Special(&SpecialVal::ZeroDate),
            ValueRef::Date(hacivat_values::DateVal(-1_000_000))
        ),
        Ordering::Less
    );
    // Intervals as PostgreSQL compares them: a month is 30 days, a day 24 hours.
    let cmp = comparator_for(&facts(&LogicalType::Interval, DbmsKind::Postgres), None);
    let month = ValueRef::Interval {
        months: 1,
        days: 0,
        micros: 0,
    };
    let days = ValueRef::Interval {
        months: 0,
        days: 30,
        micros: 0,
    };
    assert_eq!(cmp.cmp(month, days), Ordering::Equal);
}

/// Bytes and UUIDs by their bytes; a value that could not be decoded sorts after every value
/// (before the NULLs that sort last); a truncated text by the prefix the result kept.
#[test]
fn bytes_errors_and_truncated_values() {
    let bytes = LogicalType::Bytes { max: None };
    let cmp = comparator_for(&facts(&bytes, DbmsKind::Postgres), None);
    assert_eq!(
        cmp.cmp(ValueRef::Bytes(b"\x01\xff"), ValueRef::Bytes(b"\x02")),
        Ordering::Less
    );
    assert_eq!(
        cmp.cmp(ValueRef::Bytes(b"ab"), ValueRef::Bytes(b"a")),
        Ordering::Greater
    );
    let err = ErrorVal {
        code: "decode".into(),
        message: "bad".into(),
    };
    let cmp = comparator_for(&facts(&text_type(), DbmsKind::Postgres), None);
    assert_eq!(
        cmp.cmp(ValueRef::Error(&err), ValueRef::Text("zzz")),
        Ordering::Greater
    );
    assert_eq!(
        cmp.cmp(ValueRef::Error(&err), ValueRef::Null),
        Ordering::Less
    );
    let truncated = hacivat_values::TruncatedVal {
        prefix: Value::Text("abc".into()),
        total_len: Some(10_000),
        unit: hacivat_values::LenUnit::Chars,
    };
    assert_eq!(
        cmp.cmp(ValueRef::Truncated(&truncated), ValueRef::Text("abd")),
        Ordering::Less
    );
}

/// Enums sort by their labels' declared order (PostgreSQL and MySQL do), not alphabetically;
/// a label the type does not list sorts after them.
#[test]
fn enums_sort_by_declared_order() {
    let labels: Arc<[Arc<str>]> =
        Arc::from([Arc::from("small"), Arc::from("medium"), Arc::from("large")]);
    let logical = LogicalType::Enum(labels);
    let values: Vec<Value> = ["large", "zzz", "small", "medium"]
        .iter()
        .map(|t| Value::Text((*t).into()))
        .collect();
    assert_eq!(
        texts(&sorted_values(&values, &logical, DbmsKind::Postgres, None)),
        ["small", "medium", "large", "zzz"]
    );
}

/// The comparator travels to the sort's worker threads.
#[test]
fn the_comparator_is_send_and_sync() {
    fn check<T: Send + Sync>(_: &T) {}
    let cmp = comparator_for(&facts(&text_type(), DbmsKind::Postgres), tr_collation());
    check(&cmp);
}

// ---- ## m3c-t36 fix round 1 ---------------------------------------------------------------

/// I1: PostgreSQL 17's `interval` ±infinity sort around the finite intervals, as PG does:
/// `-infinity, 1 day, infinity` ascending.
#[test]
fn interval_infinity_sorts_around_the_finite_intervals() {
    let day = Value::Interval {
        months: 0,
        days: 1,
        micros: 0,
    };
    let values = [
        day.clone(),
        Value::Special(SpecialVal::PosInfinity),
        Value::Special(SpecialVal::NegInfinity),
    ];
    let out = sorted_values(&values, &LogicalType::Interval, DbmsKind::Postgres, None);
    assert_eq!(
        out,
        [
            Value::Special(SpecialVal::NegInfinity),
            day,
            Value::Special(SpecialVal::PosInfinity),
        ]
    );
}

/// m1: an integer against a float compares exactly (no rounding through `f64` past 2^53), so
/// the order is transitive; an exact decimal against a float too.
#[test]
fn integers_and_floats_compare_exactly() {
    let real = LogicalType::Float { bits: 64 };
    let cmp = comparator_for(&facts(&real, DbmsKind::Sqlite), None);
    let big = 1i64 << 53;
    let (a, f, b) = (
        ValueRef::Int(big + 1),
        ValueRef::F64(big as f64),
        ValueRef::Int(big),
    );
    assert_eq!(cmp.cmp(a, f), Ordering::Greater);
    assert_eq!(cmp.cmp(f, b), Ordering::Equal);
    assert_eq!(cmp.cmp(a, b), Ordering::Greater);
    assert_eq!(
        cmp.cmp(ValueRef::F64(0.5), ValueRef::Int(0)),
        Ordering::Greater
    );
    assert_eq!(
        cmp.cmp(ValueRef::F64(0.5), ValueRef::Int(1)),
        Ordering::Less
    );
    assert_eq!(
        cmp.cmp(ValueRef::F64(-0.5), ValueRef::Int(0)),
        Ordering::Less
    );
    assert_eq!(
        cmp.cmp(ValueRef::F64(-0.5), ValueRef::Int(-1)),
        Ordering::Greater
    );
    assert_eq!(
        cmp.cmp(ValueRef::F64(1e30), ValueRef::UInt(u64::MAX)),
        Ordering::Greater
    );
    assert_eq!(
        cmp.cmp(ValueRef::F64(-1e30), ValueRef::Int(i64::MIN)),
        Ordering::Less
    );
    assert_eq!(
        cmp.cmp(ValueRef::Int(3), ValueRef::F64(3.0)),
        Ordering::Equal
    );
    // 0.1 as a binary float is 0.1000000000000000055…: above the exact decimal 0.1.
    let tenth = DecimalVal::parse("0.1").unwrap();
    assert_eq!(
        cmp.cmp(ValueRef::Decimal(tenth.view()), ValueRef::F64(0.1)),
        Ordering::Less
    );
    let half = DecimalVal::parse("0.50").unwrap();
    assert_eq!(
        cmp.cmp(ValueRef::Decimal(half.view()), ValueRef::F64(0.5)),
        Ordering::Equal
    );
    // A column of mixed integers and floats (SQLite) sorts the same whatever the input order.
    let mixed = [
        Value::Int(big + 1),
        Value::F64(-0.25),
        Value::Int(-1),
        Value::Int(big),
        Value::F64(big as f64 + 4.0),
    ];
    let one = sorted_values(&mixed, &real, DbmsKind::Sqlite, None);
    let mut reversed = mixed.to_vec();
    reversed.reverse();
    let two = sorted_values(&reversed, &real, DbmsKind::Sqlite, None);
    assert_eq!(one, two);
    assert_eq!(
        one,
        [
            Value::Int(-1),
            Value::F64(-0.25),
            Value::Int(big),
            Value::Int(big + 1),
            Value::F64(big as f64 + 4.0),
        ]
    );
}

/// m1: a SQLite `BOOLEAN` column holding other integers sorts them as numbers (SQLite does:
/// `-1, 0, 1, 2`); elsewhere booleans keep their own kind.
#[test]
fn sqlite_booleans_sort_as_numbers() {
    let values = [
        Value::Int(2),
        Value::Bool(true),
        Value::Int(-1),
        Value::Bool(false),
    ];
    assert_eq!(
        sorted_values(&values, &LogicalType::Bool, DbmsKind::Sqlite, None),
        [
            Value::Int(-1),
            Value::Bool(false),
            Value::Bool(true),
            Value::Int(2)
        ]
    );
}

/// m7: two `timetz` values at the same UTC instant order as PostgreSQL's `timetz_cmp`: by the
/// zone in seconds west, ascending (the eastern offset first).
#[test]
fn timetz_ties_follow_postgres() {
    let time = LogicalType::Time {
        precision: 6,
        tz: true,
    };
    let cmp = comparator_for(&facts(&time, DbmsKind::Postgres), None);
    let h = |n: u64| TimeVal {
        nanos_of_day: n * 3_600_000_000_000,
    };
    // 12:00+03 and 09:00+00 are both 09:00 UTC.
    assert_eq!(
        cmp.cmp(ValueRef::TimeTz(h(12), 3 * 3600), ValueRef::TimeTz(h(9), 0)),
        Ordering::Less
    );
}

/// m7: MySQL's partial zero dates compare by their parts: `2020-00-15` after 2019 and before
/// `2020-01-00`, which is before `2020-01-01`; the zero date first.
#[test]
fn mysql_partial_zero_dates_compare_by_their_parts() {
    let partial = |t: &str| Value::Special(SpecialVal::PartialZeroDate(t.into()));
    let values = [
        Value::Date(hacivat_values::DateVal(18_262)), // 2020-01-01
        partial("2020-01-00"),
        partial("2020-00-15"),
        Value::Special(SpecialVal::ZeroDate),
        Value::Date(hacivat_values::DateVal(18_261)), // 2019-12-31
    ];
    assert_eq!(
        sorted_values(&values, &LogicalType::Date, DbmsKind::MySql, None),
        [
            Value::Special(SpecialVal::ZeroDate),
            Value::Date(hacivat_values::DateVal(18_261)),
            partial("2020-00-15"),
            partial("2020-01-00"),
            Value::Date(hacivat_values::DateVal(18_262)),
        ]
    );
}

/// m7: SQL Server's `*_BIN2` orders by UTF-16 code unit: a supplementary character (a
/// surrogate pair, 0xD83D…) before U+FFFD, the reverse of code-point order.
#[test]
fn sql_server_bin2_is_utf16_order() {
    let words = ["\u{FFFD}", "\u{1F600}"];
    assert_eq!(
        sorted_on(DbmsKind::MsSql, &words, Some("Latin1_General_100_BIN2")),
        ["\u{1F600}", "\u{FFFD}"]
    );
    assert_eq!(
        sorted_on(DbmsKind::Postgres, &words, Some("C")),
        ["\u{FFFD}", "\u{1F600}"]
    );
}

/// m7: a collation name resolves with the engine known: a PostgreSQL collation named like a
/// MySQL one is not taken for MySQL's rules, and the other way round.
#[test]
fn collation_names_resolve_with_the_engine() {
    let tr = Resolved::Icu {
        locale: "tr".into(),
        strength: Strength::Primary,
    };
    assert_eq!(resolve_collation(DbmsKind::MySql, "utf8mb4_turkish_ci"), tr);
    assert_eq!(
        resolve_collation(DbmsKind::MariaDb, "utf8mb4_turkish_ci"),
        tr
    );
    assert_eq!(
        resolve_collation(DbmsKind::Postgres, "x_turkish_ci"),
        Resolved::NoEquivalent
    );
    assert_eq!(
        resolve_collation(DbmsKind::Postgres, "tr_TR.utf8"),
        Resolved::Icu {
            locale: "tr-TR".into(),
            strength: Strength::Tertiary
        }
    );
    assert_eq!(
        resolve_collation(DbmsKind::MySql, "tr_TR.utf8"),
        Resolved::NoEquivalent
    );
    assert_eq!(
        resolve_collation(DbmsKind::Postgres, "utf8mb4_bin"),
        Resolved::NoEquivalent
    );
    assert_eq!(
        resolve_collation(DbmsKind::Sqlite, "BINARY"),
        Resolved::CodePoint
    );
    assert_eq!(
        resolve_collation(DbmsKind::Sqlite, "utf8mb4_turkish_ci"),
        Resolved::NoEquivalent
    );
    assert_eq!(
        resolve_collation(DbmsKind::MsSql, "Latin1_General_100_BIN2"),
        Resolved::Utf16CodeUnits
    );
    // NO PAD collations (MariaDB `nopad`) map like their PAD SPACE twins.
    assert_eq!(
        resolve_collation(DbmsKind::MariaDb, "utf8mb4_uca1400_turkish_nopad_ai_ci"),
        tr
    );
}

/// m7: a PAD SPACE collation (MySQL's legacy and `uca1400` ones without `nopad`, SQL Server)
/// ignores trailing spaces; a NO PAD one (MySQL 8's `_0900_`, PostgreSQL) does not.
#[test]
fn trailing_spaces_follow_the_pad_attribute() {
    let cmp =
        |engine: DbmsKind, name: &str| comparator_for(&facts(&text_type(), engine), Some(name));
    let (a, a_space) = (ValueRef::Text("a"), ValueRef::Text("a  "));
    assert_eq!(
        cmp(DbmsKind::MySql, "utf8mb4_unicode_ci").cmp(a, a_space),
        Ordering::Equal
    );
    assert_eq!(
        cmp(DbmsKind::MySql, "utf8mb4_bin").cmp(a, a_space),
        Ordering::Equal
    );
    assert_eq!(
        cmp(DbmsKind::MySql, "utf8mb4_0900_ai_ci").cmp(a, a_space),
        Ordering::Less
    );
    assert_eq!(
        cmp(DbmsKind::MySql, "utf8mb4_0900_bin").cmp(a, a_space),
        Ordering::Less
    );
    assert_eq!(
        cmp(DbmsKind::MariaDb, "utf8mb4_uca1400_nopad_ai_ci").cmp(a, a_space),
        Ordering::Less
    );
    assert_eq!(cmp(DbmsKind::Postgres, "C").cmp(a, a_space), Ordering::Less);
    assert_eq!(
        cmp(DbmsKind::MsSql, "Latin1_General_100_BIN2").cmp(a, a_space),
        Ordering::Equal
    );
}
