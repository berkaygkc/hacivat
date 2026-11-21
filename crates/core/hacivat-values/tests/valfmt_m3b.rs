//! M3b Task 7 (R-M3-67): each engine's float spelling (ledger 177), type labels from the
//! native type (ledger 246) and the display of the newly typed text columns (`money`, arrays,
//! network addresses, bit strings). The raw channel is locale-free: every raw assertion holds
//! under any `LANG` (the `turkey-test` job runs this file under `tr_TR.UTF-8`).

use hacivat_proto::DbmsKind;
use hacivat_values::valfmt::{
    BinaryDisplay, ColumnFacts, DisplayKind, DisplaySettings, DisplayZone, Purpose, RawText,
    format_cell, raw_text, type_label,
};
use hacivat_values::{LogicalType, NativeType, NetKind, SqliteAffinity, Value, ValueRef};

// ---- helpers ----------------------------------------------------------------------------

/// MySQL's `enum_field_types` and flags used below.
const DECIMAL: u8 = 246;
const TINY: u8 = 1;
const SHORT: u8 = 2;
const INT24: u8 = 9;
const LONG: u8 = 3;
const LONGLONG: u8 = 8;
const FLOAT: u8 = 4;
const DOUBLE: u8 = 5;
const TIMESTAMP: u8 = 7;
const DATETIME: u8 = 12;
const TIME: u8 = 11;
const DATE: u8 = 10;
const YEAR: u8 = 13;
const BIT: u8 = 16;
const JSON: u8 = 245;
const BLOB: u8 = 252;
const VAR_STRING: u8 = 253;
const STRING: u8 = 254;
const GEOMETRY: u8 = 255;
const UNSIGNED: u16 = 32;
const ENUM: u16 = 256;
const SET: u16 = 2048;
const BINARY: u16 = 63;
/// `utf8mb4_0900_ai_ci`: four bytes per character.
const UTF8MB4: u16 = 255;
/// `utf8mb3_general_ci`: three bytes per character.
const UTF8MB3: u16 = 33;
/// A collation id no table here knows (a MariaDB 11 `uca1400` one, say).
const UNKNOWN_COLLATION: u16 = 2_048;

fn mysql_native(
    column_type: u8,
    flags: u16,
    charset: u16,
    length: u32,
    decimals: u8,
) -> NativeType {
    NativeType::MySql {
        column_type,
        flags,
        charset,
        length,
        decimals,
        time_zone: None,
    }
}

/// `DECIMAL(p, s)` as MySQL describes it: its display length counts a sign and a point.
fn mysql_native_decimal(precision: u32, scale: u8) -> NativeType {
    mysql_native(
        DECIMAL,
        0,
        BINARY,
        precision + 1 + u32::from(scale > 0),
        scale,
    )
}

struct Col {
    logical: LogicalType,
    engine: DbmsKind,
}

impl Col {
    fn facts(&self) -> ColumnFacts<'_> {
        ColumnFacts {
            logical: &self.logical,
            native_name: "x",
            id_like: false,
            binary: BinaryDisplay::Detect,
            engine: self.engine,
        }
    }
}

fn float_col(bits: u8, engine: DbmsKind) -> Col {
    Col {
        logical: LogicalType::Float { bits },
        engine,
    }
}

fn mysql_float_col() -> Col {
    float_col(32, DbmsKind::MySql)
}

fn settings(locale: &str) -> DisplaySettings {
    DisplaySettings::with_zone(locale, DisplayZone::utc())
}

fn settings_en() -> DisplaySettings {
    settings("en-US")
}

fn raw(v: ValueRef<'_>, col: &Col) -> String {
    match raw_text(v, &col.facts()) {
        RawText::Text(t) => t.into_owned(),
        RawText::Null => "NULL".into(),
    }
}

fn shown(v: ValueRef<'_>, col: &Col, locale: &str) -> String {
    format_cell(v, &col.facts(), &settings(locale), Purpose::Cell).text
}

// ---- floats (ledger 177, P-M3b-26) ------------------------------------------------------

#[test]
fn mysql_f32_uses_its_own_shortest_round_trip() {
    assert_eq!(
        format_cell(
            ValueRef::F32(0.1),
            &mysql_float_col().facts(),
            &settings_en(),
            Purpose::Cell
        )
        .text,
        "0.1"
    );
}

#[test]
fn mysql_and_sqlite_floats_are_not_spelled_like_postgresql() {
    for engine in [
        DbmsKind::MySql,
        DbmsKind::MariaDb,
        DbmsKind::TiDb,
        DbmsKind::Sqlite,
    ] {
        let f8 = float_col(64, engine);
        let f4 = float_col(32, engine);
        for (x, want) in [
            (1e15, "1000000000000000"),
            (1e20, "100000000000000000000"),
            (1e21, "1e+21"),
            (1e-5, "0.00001"),
            (1e-6, "0.000001"),
            (1e-7, "1e-7"),
            (-1.5e-7, "-1.5e-7"),
            (1234.5, "1234.5"),
            (-0.0, "-0"),
            (0.1, "0.1"),
            (f64::MAX, "1.7976931348623157e+308"),
            (5e-324, "5e-324"),
        ] {
            assert_eq!(raw(ValueRef::F64(x), &f8), want, "{engine:?} {x:e}");
        }
        // An f32 at its own width: never widened (`0.10000000149011612`), never rounded to
        // MySQL's six text digits (`1234570`).
        for (x, want) in [
            (0.1f32, "0.1"),
            (1_234_567.0, "1234567"),
            (16_777_216.0, "16777216"),
            (3.4028235e38, "3.4028235e+38"),
            (1e-7, "1e-7"),
        ] {
            assert_eq!(raw(ValueRef::F32(x), &f4), want, "{engine:?} {x:e}");
        }
        // A widened f32 in an f32 column keeps the f32 spelling.
        assert_eq!(raw(ValueRef::F64(f64::from(0.1f32)), &f4), "0.1");
    }
    // PostgreSQL keeps its own spelling (M3a, read from a server).
    let pg = float_col(64, DbmsKind::Postgres);
    assert_eq!(raw(ValueRef::F64(1e15), &pg), "1e+15");
    assert_eq!(raw(ValueRef::F64(1e-5), &pg), "1e-05");
    assert_eq!(
        raw(
            ValueRef::F32(1_234_567.0),
            &float_col(32, DbmsKind::Postgres)
        ),
        "1.234567e+06"
    );
}

#[test]
fn shortest_floats_are_grouped_by_the_region_and_keep_their_exponent() {
    let my = float_col(64, DbmsKind::MySql);
    assert_eq!(
        shown(ValueRef::F64(1e15), &my, "en-US"),
        "1,000,000,000,000,000"
    );
    assert_eq!(
        shown(ValueRef::F64(1e15), &my, "tr-TR"),
        "1.000.000.000.000.000"
    );
    assert_eq!(shown(ValueRef::F64(1234.5), &my, "tr-TR"), "1.234,5");
    assert_eq!(shown(ValueRef::F64(-1.5e-7), &my, "tr-TR"), "-1,5e-7");
    assert_eq!(shown(ValueRef::F64(1e21), &my, "en-US"), "1e+21");
    let lite = float_col(64, DbmsKind::Sqlite);
    assert_eq!(shown(ValueRef::F64(f64::INFINITY), &lite, "tr-TR"), "Inf");
    assert_eq!(shown(ValueRef::F64(f64::NAN), &my, "tr-TR"), "NaN");
    // ID-like columns are never grouped.
    let mut facts = my.facts();
    facts.id_like = true;
    let d = format_cell(
        ValueRef::F64(1e15),
        &facts,
        &settings("tr-TR"),
        Purpose::Cell,
    );
    assert_eq!(d.text, "1000000000000000");
}

/// SplitMix64: a seeded generator, so every run checks the same values.
struct Seq(u64);

impl Seq {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

#[test]
fn shortest_floats_round_trip_raw_at_their_width() {
    let mut seq = Seq(7);
    for engine in [DbmsKind::MySql, DbmsKind::Sqlite] {
        let f8 = float_col(64, engine);
        let f4 = float_col(32, engine);
        for _ in 0..20_000 {
            let bits = seq.next();
            let x = f64::from_bits(bits);
            if x.is_finite() {
                let text = raw(ValueRef::F64(x), &f8);
                assert_eq!(
                    text.parse::<f64>().unwrap().to_bits(),
                    x.to_bits(),
                    "{text}"
                );
            }
            let y = f32::from_bits(bits as u32);
            if y.is_finite() {
                let text = raw(ValueRef::F32(y), &f4);
                assert_eq!(
                    text.parse::<f32>().unwrap().to_bits(),
                    y.to_bits(),
                    "{text}"
                );
            }
        }
    }
}

// ---- type labels (ledger 246) -----------------------------------------------------------

#[test]
fn mysql_type_label_from_native() {
    assert_eq!(type_label(&mysql_native_decimal(14, 2)), "decimal(14,2)");
}

#[test]
fn mysql_type_labels_name_the_declared_type() {
    let cases = [
        (
            mysql_native(LONGLONG, UNSIGNED, BINARY, 20, 0),
            "bigint unsigned",
        ),
        (mysql_native(LONGLONG, 0, BINARY, 20, 0), "bigint"),
        (mysql_native(TINY, 0, BINARY, 1, 0), "tinyint(1)"),
        (mysql_native(TINY, 0, BINARY, 4, 0), "tinyint"),
        (
            mysql_native(TINY, UNSIGNED, BINARY, 3, 0),
            "tinyint unsigned",
        ),
        (mysql_native(SHORT, 0, BINARY, 6, 0), "smallint"),
        (mysql_native(INT24, 0, BINARY, 9, 0), "mediumint"),
        (mysql_native(LONG, UNSIGNED, BINARY, 10, 0), "int unsigned"),
        (mysql_native(YEAR, UNSIGNED, BINARY, 4, 0), "year"),
        (mysql_native(FLOAT, 0, BINARY, 12, 31), "float"),
        (mysql_native(DOUBLE, 0, BINARY, 22, 31), "double"),
        // DECIMAL(10,2) UNSIGNED: no sign character in its length.
        (
            mysql_native(DECIMAL, UNSIGNED, BINARY, 11, 2),
            "decimal(10,2) unsigned",
        ),
        (mysql_native(DECIMAL, 0, BINARY, 66, 0), "decimal(65,0)"),
        (mysql_native(DATE, 0, BINARY, 10, 0), "date"),
        (mysql_native(DATETIME, 0, BINARY, 26, 6), "datetime(6)"),
        (mysql_native(DATETIME, 0, BINARY, 19, 0), "datetime"),
        (mysql_native(TIMESTAMP, 0, BINARY, 23, 3), "timestamp(3)"),
        (mysql_native(TIME, 0, BINARY, 13, 2), "time(2)"),
        (mysql_native(BIT, UNSIGNED, BINARY, 5, 0), "bit(5)"),
        (mysql_native(JSON, 0, BINARY, u32::MAX, 0), "json"),
        (mysql_native(GEOMETRY, 0, BINARY, u32::MAX, 0), "geometry"),
        (mysql_native(VAR_STRING, 0, UTF8MB4, 80, 0), "varchar(20)"),
        (mysql_native(VAR_STRING, 0, UTF8MB3, 60, 0), "varchar(20)"),
        (mysql_native(STRING, 0, UTF8MB4, 20, 0), "char(5)"),
        (mysql_native(STRING, 0, BINARY, 3, 0), "binary(3)"),
        (mysql_native(VAR_STRING, 0, BINARY, 8, 0), "varbinary(8)"),
        (mysql_native(STRING, ENUM, UTF8MB4, 24, 0), "enum"),
        (mysql_native(STRING, SET, UTF8MB4, 20, 0), "set"),
        (mysql_native(BLOB, 16, UTF8MB4, 262_140, 0), "text"),
        (mysql_native(BLOB, 16, UTF8MB4, 1_020, 0), "tinytext"),
        (mysql_native(BLOB, 16, UTF8MB4, u32::MAX, 0), "longtext"),
        (mysql_native(BLOB, 16, BINARY, 65_535, 0), "blob"),
        (mysql_native(BLOB, 16, BINARY, 16_777_215, 0), "mediumblob"),
        // An unknown collation never gives a wrong length.
        (
            mysql_native(VAR_STRING, 0, UNKNOWN_COLLATION, 80, 0),
            "varchar",
        ),
        (
            mysql_native(BLOB, 16, UNKNOWN_COLLATION, 262_140, 0),
            "text",
        ),
        // A length that is not a whole number of characters is not shown.
        (mysql_native(VAR_STRING, 0, UTF8MB4, 81, 0), "varchar"),
    ];
    for (native, want) in cases {
        assert_eq!(type_label(&native), want, "{native:?}");
    }
}

// ## m3b-t07 fix round 1 (review m-9): no English fallback; the app names the code via Fluent.
#[test]
fn a_mysql_type_code_without_a_name_has_an_empty_label() {
    for code in [17u8, 18, 19, 20, 243, 244] {
        assert_eq!(
            type_label(&mysql_native(code, 0, BINARY, 0, 0)),
            "",
            "{code}"
        );
    }
}

#[test]
fn postgresql_and_sqlite_labels_are_their_names() {
    let pg = NativeType::Pg {
        oid: 1700,
        typmod: -1,
        name: "numeric".into(),
    };
    assert_eq!(type_label(&pg), "numeric");
    let declared = NativeType::Sqlite {
        decl: Some("VARCHAR(40)".into()),
        affinity: SqliteAffinity::Text,
    };
    assert_eq!(type_label(&declared), "VARCHAR(40)");
    let bare = NativeType::Sqlite {
        decl: None,
        affinity: SqliteAffinity::Blob,
    };
    assert_eq!(type_label(&bare), "blob");
    assert_eq!(type_label(&NativeType::Unknown), "");
}

// ---- the typed text columns (R-M3-67) ---------------------------------------------------

#[test]
fn money_is_right_aligned_and_verbatim() {
    let money = Col {
        logical: LogicalType::Money,
        engine: DbmsKind::Postgres,
    };
    // The server's `lc_monetary` text: never parsed, never regrouped by the region.
    for (text, locale) in [
        ("$1,234.50", "tr-TR"),
        ("-$0.01", "en-US"),
        ("1.234,50 TL", "en-US"),
    ] {
        let d = format_cell(
            ValueRef::Text(text),
            &money.facts(),
            &settings(locale),
            Purpose::Cell,
        );
        assert_eq!(d.text, text);
        assert!(d.align_end, "{text}");
        assert_eq!(d.kind, DisplayKind::Number, "{text}");
        assert_eq!(raw(ValueRef::Text(text), &money), text);
    }
    let null = format_cell(
        ValueRef::Null,
        &money.facts(),
        &settings_en(),
        Purpose::Cell,
    );
    assert!(null.align_end);
}

#[test]
fn array_network_and_bit_text_show_and_copy_verbatim() {
    let cases = [
        (
            LogicalType::Array(Box::new(LogicalType::Int {
                bits: 32,
                signed: true,
            })),
            "[0:2]={1,2,3}",
        ),
        (LogicalType::Network(NetKind::Cidr), "10.0.0.0/8"),
        (LogicalType::BitString { len: Some(5) }, "10110"),
        (LogicalType::Geometry { srid: None }, "(1,2)"),
    ];
    for (logical, text) in cases {
        let col = Col {
            logical,
            engine: DbmsKind::Postgres,
        };
        let v = Value::Text(text.into());
        for locale in ["en-US", "tr-TR"] {
            let d = format_cell(v.view(), &col.facts(), &settings(locale), Purpose::Cell);
            assert_eq!(d.text, text);
            assert_eq!(d.kind, DisplayKind::Text);
            assert!(!d.align_end);
        }
        assert_eq!(raw(v.view(), &col), text);
    }
}
