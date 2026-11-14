//! The PostgreSQL value decoders against byte fixtures (Task 12; HV-RES-01 PG mapping,
//! R-M3-31). Every fixture goes through the real cell path (`decode_cell`: the column's plan, a
//! one-row chunk built with `ChunkBuilder::with_schema`, read back), and every expectation is
//! derived from the wire format by hand, never from the decoder. Hostile bytes give a cell
//! `Error`, never a panic (Review Focus 1).

use std::sync::Arc;

use hacivat_driver_pg::decode::{
    MAX_NUMERIC_DIGITS, buf_kind_for, decode_cell, format_for, logical_for,
};
use hacivat_values::{
    BufKind, DateVal, DecimalVal, LogicalType, OpaqueVal, SpecialVal, TimeVal, TsVal, Value,
    VarKind,
};
use postgres_types::{Kind, Type};

/// The default cell cap (1 MiB, HV-RES-08).
const CAP: u32 = 1 << 20;
/// No type modifier.
const NO_MOD: i32 = -1;

fn mood() -> Type {
    Type::new(
        "mood".into(),
        91_001,
        Kind::Enum(vec!["sad".into(), "ok".into(), "happy".into()]),
        "public".into(),
    )
}

/// A type with an OID no build of PostgreSQL ships (an extension's), simple kind.
fn extension_type() -> Type {
    Type::new("hstore".into(), 91_002, Kind::Simple, "public".into())
}

/// `numeric(p, s)`'s type modifier.
fn numeric_mod(precision: i32, scale: i32) -> i32 {
    ((precision << 16) | (scale & 0x7ff)) + 4
}

/// A binary `numeric`: ndigits, weight, sign word, dscale, then the base-10000 digits.
fn numeric(weight: i16, sign: u16, dscale: u16, digits: &[i16]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&i16::try_from(digits.len()).unwrap().to_be_bytes());
    b.extend_from_slice(&weight.to_be_bytes());
    b.extend_from_slice(&sign.to_be_bytes());
    b.extend_from_slice(&dscale.to_be_bytes());
    for d in digits {
        b.extend_from_slice(&d.to_be_bytes());
    }
    b
}

/// A positive numeric with `n` base-10000 digits of 1 (its length is consistent).
fn numeric_ndigits(n: usize) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&i16::try_from(n).unwrap().to_be_bytes());
    b.extend_from_slice(&i16::try_from(n - 1).unwrap().to_be_bytes());
    b.extend_from_slice(&0u16.to_be_bytes());
    b.extend_from_slice(&0u16.to_be_bytes());
    for _ in 0..n {
        b.extend_from_slice(&1i16.to_be_bytes());
    }
    b
}

const NUMERIC_POS: u16 = 0x0000;
const NUMERIC_NEG: u16 = 0x4000;
const NUMERIC_NAN: u16 = 0xC000;
const NUMERIC_PINF: u16 = 0xD000;
const NUMERIC_NINF: u16 = 0xF000;

fn dec(s: &str) -> Value {
    Value::Decimal(DecimalVal::parse(s).unwrap())
}

fn time(h: u64, m: u64, s: u64, us: u64) -> TimeVal {
    TimeVal {
        nanos_of_day: (((h * 60 + m) * 60 + s) * 1_000_000 + us) * 1_000,
    }
}

/// 2000-01-01 00:00:00 UTC (PostgreSQL's epoch) as a `TsVal`.
const PG_EPOCH: TsVal = TsVal {
    secs: 946_684_800,
    nanos: 0,
};

struct Fixture {
    name: &'static str,
    ty: Type,
    typmod: i32,
    raw: Option<Vec<u8>>,
    expect: Value,
}

fn f(name: &'static str, ty: Type, raw: impl Into<Vec<u8>>, expect: Value) -> Fixture {
    Fixture {
        name,
        ty,
        typmod: NO_MOD,
        raw: Some(raw.into()),
        expect,
    }
}

fn fm(name: &'static str, ty: Type, typmod: i32, raw: Vec<u8>, expect: Value) -> Fixture {
    Fixture {
        name,
        ty,
        typmod,
        raw: Some(raw),
        expect,
    }
}

fn be16(v: i16) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

fn be32(v: i32) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

fn be64(v: i64) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

fn pg_fixtures() -> Vec<Fixture> {
    let uuid: [u8; 16] = [
        0x6f, 0x1e, 0x9a, 0x0c, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 0, 0, 1,
    ];
    let mut timetz = be64(3_600_000_000);
    timetz.extend(be32(-10_800)); // +03 is stored as 10,800 seconds west, negated
    let mut interval = be64(1_000_001);
    interval.extend(be32(2));
    interval.extend(be32(-3));
    let mut jsonb = vec![1u8];
    jsonb.extend_from_slice(b"{\"a\": 1}");
    vec![
        f("bool true", Type::BOOL, [1], Value::Bool(true)),
        f("bool false", Type::BOOL, [0], Value::Bool(false)),
        f("int2", Type::INT2, be16(-2), Value::Int(-2)),
        f("int4", Type::INT4, be32(-123_456), Value::Int(-123_456)),
        f("int8 min", Type::INT8, be64(i64::MIN), Value::Int(i64::MIN)),
        f(
            "oid max",
            Type::OID,
            [0xff; 4],
            Value::Int(i64::from(u32::MAX)),
        ),
        f("xid", Type::XID, be32(7), Value::Int(7)),
        f("cid", Type::CID, be32(9), Value::Int(9)),
        f(
            "float4 0.1",
            Type::FLOAT4,
            0.1f32.to_be_bytes(),
            Value::F32(0.1),
        ),
        f(
            "float4 inf",
            Type::FLOAT4,
            f32::INFINITY.to_be_bytes(),
            Value::F32(f32::INFINITY),
        ),
        f(
            "float8 0.1",
            Type::FLOAT8,
            0.1f64.to_be_bytes(),
            Value::F64(0.1),
        ),
        f(
            "float8 -inf",
            Type::FLOAT8,
            f64::NEG_INFINITY.to_be_bytes(),
            Value::F64(f64::NEG_INFINITY),
        ),
        f(
            "numeric 12345.678",
            Type::NUMERIC,
            numeric(1, NUMERIC_POS, 3, &[1, 2345, 6780]),
            dec("12345.678"),
        ),
        f(
            "numeric 0.0001",
            Type::NUMERIC,
            numeric(-1, NUMERIC_POS, 4, &[1]),
            dec("0.0001"),
        ),
        f(
            "numeric keeps its scale",
            Type::NUMERIC,
            numeric(0, NUMERIC_POS, 2, &[1, 5000]),
            dec("1.50"),
        ),
        f(
            "numeric zero with scale",
            Type::NUMERIC,
            numeric(0, NUMERIC_POS, 3, &[]),
            dec("0.000"),
        ),
        fm(
            "numeric(10,2) negative",
            Type::NUMERIC,
            numeric_mod(10, 2),
            numeric(0, NUMERIC_NEG, 2, &[1, 5000]),
            dec("-1.50"),
        ),
        fm(
            "numeric(30,5)",
            Type::NUMERIC,
            numeric_mod(30, 5),
            numeric(2, NUMERIC_POS, 5, &[12, 3456, 7890, 1234, 5000]),
            dec("1234567890.12345"),
        ),
        f(
            "numeric NaN",
            Type::NUMERIC,
            numeric(0, NUMERIC_NAN, 0, &[]),
            Value::Special(SpecialVal::NaN),
        ),
        f(
            "numeric +Infinity",
            Type::NUMERIC,
            numeric(0, NUMERIC_PINF, 0, &[]),
            Value::Special(SpecialVal::PosInfinity),
        ),
        f(
            "numeric -Infinity",
            Type::NUMERIC,
            numeric(0, NUMERIC_NINF, 0, &[]),
            Value::Special(SpecialVal::NegInfinity),
        ),
        f(
            "numeric 10^40 (Big)",
            Type::NUMERIC,
            numeric(10, NUMERIC_POS, 0, &[1]),
            dec("10000000000000000000000000000000000000000"),
        ),
        f("text", Type::TEXT, "ş i̇", Value::Text("ş i̇".into())),
        fm(
            "varchar(10)",
            Type::VARCHAR,
            14,
            b"abc".to_vec(),
            Value::Text("abc".into()),
        ),
        fm(
            "bpchar(3)",
            Type::BPCHAR,
            7,
            b"ab ".to_vec(),
            Value::Text("ab ".into()),
        ),
        f(
            "name",
            Type::NAME,
            "pg_class",
            Value::Text("pg_class".into()),
        ),
        f("\"char\"", Type::CHAR, "r", Value::Text("r".into())),
        f(
            "text NULL is not the string",
            Type::TEXT,
            "NULL",
            Value::Text("NULL".into()),
        ),
        f(
            "bytea",
            Type::BYTEA,
            [0u8, 255, 10],
            Value::Bytes(Arc::from(&[0u8, 255, 10][..])),
        ),
        f("uuid", Type::UUID, uuid, Value::Uuid(uuid)),
        f(
            "json keeps its spelling",
            Type::JSON,
            "{\"b\":1.50,\"a\":1e3}",
            Value::Json("{\"b\":1.50,\"a\":1e3}".into()),
        ),
        f(
            "jsonb version 1",
            Type::JSONB,
            jsonb,
            Value::Json("{\"a\": 1}".into()),
        ),
        f(
            "date 2000-01-01",
            Type::DATE,
            be32(0),
            Value::Date(DateVal::from_civil(2000, 1, 1).unwrap()),
        ),
        f(
            "date 1999-12-31",
            Type::DATE,
            be32(-1),
            Value::Date(DateVal::from_civil(1999, 12, 31).unwrap()),
        ),
        f(
            "date 4714-11-24 BC",
            Type::DATE,
            be32(DateVal::from_civil(-4713, 11, 24).unwrap().0 - 10_957),
            Value::Date(DateVal::from_civil(-4713, 11, 24).unwrap()),
        ),
        f(
            "date infinity",
            Type::DATE,
            be32(i32::MAX),
            Value::Special(SpecialVal::PosInfinity),
        ),
        f(
            "date -infinity",
            Type::DATE,
            be32(i32::MIN),
            Value::Special(SpecialVal::NegInfinity),
        ),
        f(
            "time",
            Type::TIME,
            be64(3_723_000_001),
            Value::Time(time(1, 2, 3, 1)),
        ),
        f(
            "time 24:00:00",
            Type::TIME,
            be64(86_400_000_000),
            Value::Time(TimeVal::END_OF_DAY),
        ),
        f(
            "timetz +03",
            Type::TIMETZ,
            timetz,
            Value::TimeTz(time(1, 0, 0, 0), 10_800),
        ),
        f(
            "timestamp epoch",
            Type::TIMESTAMP,
            be64(0),
            Value::Timestamp(PG_EPOCH),
        ),
        f(
            "timestamp before 1970",
            Type::TIMESTAMP,
            be64(-946_684_800_000_001),
            Value::Timestamp(TsVal {
                secs: -1,
                nanos: 999_999_000,
            }),
        ),
        f(
            "timestamp infinity",
            Type::TIMESTAMP,
            be64(i64::MAX),
            Value::Special(SpecialVal::PosInfinity),
        ),
        f(
            "timestamp -infinity",
            Type::TIMESTAMP,
            be64(i64::MIN),
            Value::Special(SpecialVal::NegInfinity),
        ),
        f(
            "timestamp past 292277 AD stays exact",
            Type::TIMESTAMP,
            be64(9_000_000_000_000_000_000),
            Value::Timestamp(TsVal {
                secs: 9_000_000_000_000 + 946_684_800,
                nanos: 0,
            }),
        ),
        f(
            "timestamptz epoch",
            Type::TIMESTAMPTZ,
            be64(0),
            Value::TimestampTz {
                utc: PG_EPOCH,
                offset: None,
            },
        ),
        f(
            "timestamptz past 292277 AD stays exact",
            Type::TIMESTAMPTZ,
            be64(9_000_000_000_000_000_000),
            Value::TimestampTz {
                utc: TsVal {
                    secs: 9_000_000_000_000 + 946_684_800,
                    nanos: 0,
                },
                offset: None,
            },
        ),
        f(
            "interval",
            Type::INTERVAL,
            interval,
            Value::Interval {
                months: -3,
                days: 2,
                micros: 1_000_001,
            },
        ),
        f("enum label", mood(), "happy", Value::Text("happy".into())),
        f(
            "tsvector as opaque text",
            Type::TS_VECTOR,
            "'a' 'b'",
            Value::Opaque(Arc::new(OpaqueVal {
                type_name: "tsvector".into(),
                text: "'a' 'b'".into(),
            })),
        ),
        f(
            "an extension type as opaque text",
            extension_type(),
            "\"k\"=>\"v\"",
            Value::Opaque(Arc::new(OpaqueVal {
                type_name: "hstore".into(),
                text: "\"k\"=>\"v\"".into(),
            })),
        ),
        f(
            "int4[] as opaque text",
            Type::INT4_ARRAY,
            "{1,NULL,3}",
            Value::Opaque(Arc::new(OpaqueVal {
                type_name: "_int4".into(),
                text: "{1,NULL,3}".into(),
            })),
        ),
        Fixture {
            name: "NULL",
            ty: Type::INT4,
            typmod: NO_MOD,
            raw: None,
            expect: Value::Null,
        },
        Fixture {
            name: "NULL opaque",
            ty: Type::TS_VECTOR,
            typmod: NO_MOD,
            raw: None,
            expect: Value::Null,
        },
    ]
}

fn decode(f: &Fixture) -> Value {
    decode_cell(&f.ty, f.typmod, f.raw.as_deref(), CAP)
}

#[test]
fn every_scalar_decoder_against_fixtures() {
    for f in pg_fixtures() {
        assert_eq!(decode(&f), f.expect, "{}", f.name);
    }
}

#[test]
fn floats_keep_nan_and_negative_zero() {
    match decode_cell(&Type::FLOAT4, NO_MOD, Some(&(-0.0f32).to_be_bytes()), CAP) {
        Value::F32(v) => assert!(v == 0.0 && v.is_sign_negative(), "{v}"),
        other => panic!("{other:?}"),
    }
    match decode_cell(&Type::FLOAT8, NO_MOD, Some(&(-0.0f64).to_be_bytes()), CAP) {
        Value::F64(v) => assert!(v == 0.0 && v.is_sign_negative(), "{v}"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        decode_cell(&Type::FLOAT4, NO_MOD, Some(&f32::NAN.to_be_bytes()), CAP),
        Value::F32(v) if v.is_nan()
    ));
    assert!(matches!(
        decode_cell(&Type::FLOAT8, NO_MOD, Some(&f64::NAN.to_be_bytes()), CAP),
        Value::F64(v) if v.is_nan()
    ));
}

#[test]
fn invalid_utf8_text_is_lossy_never_replaced() {
    for ty in [Type::TEXT, Type::VARCHAR, Type::NAME, Type::JSON, mood()] {
        match decode_cell(&ty, NO_MOD, Some(&[0x61, 0xc3]), CAP) {
            Value::Lossy(b) => assert_eq!(&b[..], [0x61, 0xc3], "{ty}"),
            other => panic!("{ty}: {other:?}"),
        }
    }
    // An opaque column of a SQL_ASCII database keeps its bytes too.
    assert!(matches!(
        decode_cell(&Type::TS_VECTOR, NO_MOD, Some(&[0xff]), CAP),
        Value::Lossy(b) if b[..] == [0xff]
    ));
}

#[test]
fn cell_over_1_mib_is_truncated_at_a_char_boundary() {
    let big = "ş".repeat(700_000);
    for ty in [Type::TEXT, Type::TS_VECTOR] {
        match decode_cell(&ty, NO_MOD, Some(big.as_bytes()), 1 << 20) {
            Value::Truncated(t) => {
                assert_eq!(t.total_len, Some(1_400_000), "{ty}");
                match &t.prefix {
                    Value::Text(p) => {
                        assert_eq!(p.len(), 1 << 20, "{ty}: 'ş' is two bytes");
                        assert!(p.chars().all(|c| c == 'ş'), "{ty}");
                    }
                    other => panic!("{ty}: {other:?}"),
                }
            }
            other => panic!("{ty}: {other:?}"),
        }
    }
    // An odd cap cuts before the character it would split.
    match decode_cell(&Type::TEXT, NO_MOD, Some("şş".as_bytes()), 3) {
        Value::Truncated(t) => assert_eq!(t.prefix, Value::Text("ş".into())),
        other => panic!("{other:?}"),
    }
    match decode_cell(&Type::BYTEA, NO_MOD, Some(&[7u8; 10]), 4) {
        Value::Truncated(t) => {
            assert_eq!(t.total_len, Some(10));
            assert_eq!(t.prefix, Value::Bytes(Arc::from(&[7u8; 4][..])));
        }
        other => panic!("{other:?}"),
    }
}

fn is_error(v: &Value) -> bool {
    matches!(v, Value::Error(_))
}

#[test]
fn numeric_with_1001_digits_is_an_error_cell_not_a_panic() {
    assert_eq!(MAX_NUMERIC_DIGITS, 1_000);
    let v = decode_cell(&Type::NUMERIC, NO_MOD, Some(&numeric_ndigits(1_001)), CAP);
    assert!(is_error(&v), "{v:?}");
    // 1,000 digits is still a value (held exactly as text).
    let v = decode_cell(&Type::NUMERIC, NO_MOD, Some(&numeric_ndigits(1_000)), CAP);
    assert!(
        matches!(&v, Value::Decimal(d) if d.is_big()),
        "{:?}",
        std::mem::discriminant(&v)
    );
}

/// Bytes no PostgreSQL server sends, per type.
fn hostile_lengths() -> Vec<(Type, Vec<u8>)> {
    let mut v: Vec<(Type, Vec<u8>)> = Vec::new();
    for (ty, ok) in [
        (Type::BOOL, 1usize),
        (Type::INT2, 2),
        (Type::INT4, 4),
        (Type::INT8, 8),
        (Type::OID, 4),
        (Type::FLOAT4, 4),
        (Type::FLOAT8, 8),
        (Type::UUID, 16),
        (Type::DATE, 4),
        (Type::TIME, 8),
        (Type::TIMETZ, 12),
        (Type::TIMESTAMP, 8),
        (Type::TIMESTAMPTZ, 8),
        (Type::INTERVAL, 16),
    ] {
        for len in [0, ok - 1, ok + 1, ok * 3] {
            v.push((ty.clone(), vec![0; len]));
        }
    }
    // bool is 0 or 1.
    v.push((Type::BOOL, vec![2]));
    // numeric: too short for its header, a length that disagrees with ndigits, a negative
    // ndigits, a digit of 10,000, a negative digit, an unknown sign word, a display scale
    // above 16,383, a nonzero digit beyond the display scale, a special with digits.
    v.push((Type::NUMERIC, vec![0; 7]));
    let mut short = numeric(0, NUMERIC_POS, 0, &[1, 2]);
    short.truncate(short.len() - 1);
    v.push((Type::NUMERIC, short));
    let mut long = numeric(0, NUMERIC_POS, 0, &[1]);
    long.extend([0, 0]);
    v.push((Type::NUMERIC, long));
    let mut negative = numeric(0, NUMERIC_POS, 0, &[]);
    negative[..2].copy_from_slice(&(-1i16).to_be_bytes());
    v.push((Type::NUMERIC, negative));
    v.push((Type::NUMERIC, numeric(0, NUMERIC_POS, 0, &[10_000])));
    v.push((Type::NUMERIC, numeric(0, NUMERIC_POS, 0, &[-1])));
    v.push((Type::NUMERIC, numeric(0, 0x1234, 0, &[1])));
    v.push((Type::NUMERIC, numeric(0, NUMERIC_POS, 16_384, &[1])));
    v.push((Type::NUMERIC, numeric(-1, NUMERIC_POS, 2, &[1234])));
    v.push((Type::NUMERIC, numeric(0, NUMERIC_NAN, 0, &[1])));
    // jsonb: no version byte, or one this decoder does not know.
    v.push((Type::JSONB, vec![]));
    let mut v2 = vec![2u8];
    v2.extend_from_slice(b"{}");
    v.push((Type::JSONB, v2));
    // A time of day past 24:00:00 or before midnight; a zone no offset can hold.
    v.push((Type::TIME, be64(86_400_000_001)));
    v.push((Type::TIME, be64(-1)));
    let mut zone = be64(0);
    zone.extend(be32(i32::MIN));
    v.push((Type::TIMETZ, zone));
    let mut late = be64(86_400_000_001);
    late.extend(be32(0));
    v.push((Type::TIMETZ, late));
    v
}

#[test]
fn malformed_lengths_are_error_cells() {
    for (ty, b) in hostile_lengths() {
        let v = decode_cell(&ty, NO_MOD, Some(&b), CAP);
        assert!(is_error(&v), "{ty} {b:?}: {v:?}");
    }
}

#[test]
fn hostile_bytes_never_panic_in_any_column_kind() {
    // Every decoder over every short prefix of a noisy buffer: a value or an Error, never a
    // panic, and every cell reads back.
    let noise: Vec<u8> = (0u16..64).map(|i| (i * 37 % 256) as u8).collect();
    let types = [
        Type::BOOL,
        Type::INT2,
        Type::INT4,
        Type::INT8,
        Type::OID,
        Type::FLOAT4,
        Type::FLOAT8,
        Type::NUMERIC,
        Type::TEXT,
        Type::BYTEA,
        Type::UUID,
        Type::JSON,
        Type::JSONB,
        Type::DATE,
        Type::TIME,
        Type::TIMETZ,
        Type::TIMESTAMP,
        Type::TIMESTAMPTZ,
        Type::INTERVAL,
        mood(),
        Type::TS_VECTOR,
    ];
    for ty in &types {
        for typmod in [NO_MOD, numeric_mod(10, 2), numeric_mod(40, 0)] {
            for len in 0..=noise.len() {
                let _ = decode_cell(ty, typmod, Some(&noise[..len]), 16);
            }
        }
    }
}

#[test]
fn unknown_oid_is_requested_as_text_and_opaque() {
    assert_eq!(format_for(&Type::TS_VECTOR), 0);
    assert_eq!(format_for(&Type::INT8), 1);
    // The binary set of R-M3-31, and enums.
    for ty in [
        Type::BOOL,
        Type::INT2,
        Type::INT4,
        Type::OID,
        Type::XID,
        Type::CID,
        Type::FLOAT4,
        Type::FLOAT8,
        Type::NUMERIC,
        Type::TEXT,
        Type::VARCHAR,
        Type::BPCHAR,
        Type::NAME,
        Type::CHAR,
        Type::BYTEA,
        Type::UUID,
        Type::JSON,
        Type::JSONB,
        Type::DATE,
        Type::TIME,
        Type::TIMETZ,
        Type::TIMESTAMP,
        Type::TIMESTAMPTZ,
        Type::INTERVAL,
        mood(),
    ] {
        assert_eq!(format_for(&ty), 1, "{ty}");
    }
    // Text for everything else: arrays, network, bit strings, money, reg*, geometry, ranges,
    // XML, records, extension types.
    for ty in [
        Type::INT4_ARRAY,
        Type::TEXT_ARRAY,
        Type::INET,
        Type::CIDR,
        Type::MACADDR,
        Type::BIT,
        Type::VARBIT,
        Type::MONEY,
        Type::REGCLASS,
        Type::REGPROC,
        Type::POINT,
        Type::INT4_RANGE,
        Type::XML,
        Type::RECORD,
        extension_type(),
    ] {
        assert_eq!(format_for(&ty), 0, "{ty}");
        assert!(
            matches!(logical_for(&ty, NO_MOD), LogicalType::Opaque { ref type_name } if **type_name == *ty.name()),
            "{ty}"
        );
        assert_eq!(buf_kind_for(&ty, NO_MOD), BufKind::Values, "{ty}");
    }
}

#[test]
fn logical_types_and_buffers_follow_the_type_modifier() {
    assert_eq!(
        logical_for(&Type::NUMERIC, numeric_mod(10, 2)),
        LogicalType::Decimal {
            precision: Some(10),
            scale: Some(2)
        }
    );
    assert_eq!(
        buf_kind_for(&Type::NUMERIC, numeric_mod(10, 2)),
        BufKind::Dec64 { scale: 2 }
    );
    assert_eq!(
        buf_kind_for(&Type::NUMERIC, numeric_mod(18, 0)),
        BufKind::Dec64 { scale: 0 }
    );
    assert_eq!(
        buf_kind_for(&Type::NUMERIC, numeric_mod(19, 2)),
        BufKind::Dec128
    );
    assert_eq!(
        logical_for(&Type::NUMERIC, NO_MOD),
        LogicalType::Decimal {
            precision: None,
            scale: None
        }
    );
    assert_eq!(buf_kind_for(&Type::NUMERIC, NO_MOD), BufKind::Dec128);
    // PostgreSQL 15+: a negative scale rounds before the point; values have no digits after it.
    assert_eq!(
        logical_for(&Type::NUMERIC, numeric_mod(5, -2)),
        LogicalType::Decimal {
            precision: Some(5),
            scale: Some(-2)
        }
    );
    assert_eq!(
        buf_kind_for(&Type::NUMERIC, numeric_mod(5, -2)),
        BufKind::Dec64 { scale: 0 }
    );
    assert_eq!(
        logical_for(&Type::VARCHAR, 24),
        LogicalType::Text {
            max_chars: Some(20),
            fixed: false
        }
    );
    assert_eq!(
        logical_for(&Type::BPCHAR, 7),
        LogicalType::Text {
            max_chars: Some(3),
            fixed: true
        }
    );
    assert_eq!(
        logical_for(&Type::TEXT, NO_MOD),
        LogicalType::Text {
            max_chars: None,
            fixed: false
        }
    );
    assert_eq!(
        logical_for(&Type::TIMESTAMPTZ, 3),
        LogicalType::Timestamp {
            precision: 3,
            tz: true
        }
    );
    assert_eq!(
        logical_for(&Type::TIME, NO_MOD),
        LogicalType::Time {
            precision: 6,
            tz: false
        }
    );
    assert_eq!(
        logical_for(&Type::FLOAT4, NO_MOD),
        LogicalType::Float { bits: 32 }
    );
    assert_eq!(
        buf_kind_for(&Type::FLOAT4, NO_MOD),
        BufKind::F64 { f32: true }
    );
    assert_eq!(
        logical_for(&Type::OID, NO_MOD),
        LogicalType::Int {
            bits: 32,
            signed: false
        }
    );
    assert_eq!(
        logical_for(&Type::JSONB, NO_MOD),
        LogicalType::Json { binary: true }
    );
    assert_eq!(
        buf_kind_for(&Type::JSONB, NO_MOD),
        BufKind::Var(VarKind::Json)
    );
    match logical_for(&mood(), NO_MOD) {
        LogicalType::Enum(labels) => {
            assert_eq!(
                labels.iter().map(|l| &**l).collect::<Vec<_>>(),
                ["sad", "ok", "happy"]
            )
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(buf_kind_for(&Type::INTERVAL, NO_MOD), BufKind::Values);
}

/// A binary `interval`: microseconds, days, months.
fn interval_bytes(micros: i64, days: i32, months: i32) -> Vec<u8> {
    let mut b = be64(micros);
    b.extend(be32(days));
    b.extend(be32(months));
    b
}

#[test]
fn infinite_intervals_are_special() {
    // PostgreSQL 17+: `infinity` is every field at its maximum (INTERVAL_NOEND), `-infinity`
    // every field at its minimum (fix round 1, review I2).
    let cases = [
        (
            interval_bytes(i64::MAX, i32::MAX, i32::MAX),
            Value::Special(SpecialVal::PosInfinity),
        ),
        (
            interval_bytes(i64::MIN, i32::MIN, i32::MIN),
            Value::Special(SpecialVal::NegInfinity),
        ),
        // Anything short of all three is a finite interval, however large.
        (
            interval_bytes(i64::MAX, i32::MAX, 0),
            Value::Interval {
                months: 0,
                days: i32::MAX,
                micros: i64::MAX,
            },
        ),
        (
            interval_bytes(i64::MIN, i32::MIN, i32::MAX),
            Value::Interval {
                months: i32::MAX,
                days: i32::MIN,
                micros: i64::MIN,
            },
        ),
    ];
    for (raw, want) in cases {
        assert_eq!(decode_cell(&Type::INTERVAL, NO_MOD, Some(&raw), CAP), want);
    }
}

#[test]
fn an_empty_char_is_its_nul_byte() {
    // `"char"` `''` (`pg_attribute.attidentity` on most columns) is the byte 0 in binary
    // (`charsend`); it is kept as sent, a U+0000 character the grid shows as a control (review
    // M1, kept by the controller's ruling; ADR 0002).
    assert_eq!(
        decode_cell(&Type::CHAR, NO_MOD, Some(&[0u8]), CAP),
        Value::Text("\0".into())
    );
}

#[test]
fn values_outside_a_dec64_column_stay_exact() {
    // numeric(10,2) holds 2-scale mantissas inline; a special or a big value is an exact
    // exception, never rounded.
    let m = numeric_mod(10, 2);
    assert_eq!(
        decode_cell(
            &Type::NUMERIC,
            m,
            Some(&numeric(0, NUMERIC_NAN, 0, &[])),
            CAP
        ),
        Value::Special(SpecialVal::NaN)
    );
    assert_eq!(
        decode_cell(
            &Type::NUMERIC,
            m,
            Some(&numeric(0, NUMERIC_POS, 3, &[1, 5000])),
            CAP
        ),
        dec("1.500")
    );
    // -0.00 (never sent by PostgreSQL, but kept as sent).
    assert_eq!(
        decode_cell(
            &Type::NUMERIC,
            m,
            Some(&numeric(0, NUMERIC_NEG, 2, &[])),
            CAP
        ),
        dec("-0.00")
    );
}
