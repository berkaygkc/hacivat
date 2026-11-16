//! M3b Task 7 (R-M3-67): the PostgreSQL type matrix against byte fixtures. Arrays of every
//! M3a scalar (n-dimensional, lower bounds, NULL elements), `inet`/`cidr`/`macaddr`/`macaddr8`
//! and `bit`/`varbit` decode in binary to the text PostgreSQL prints; composites, ranges,
//! `money`, `xml` and geometry stay text but carry their own logical type. Every fixture goes
//! through the real cell path (`decode_cell`); every expectation is written from PostgreSQL's
//! wire format and its output functions (`array_out`, `network_out`, `bit_out`), never from the
//! decoder. Hostile bytes give an `Error` cell, never a panic.

use std::sync::Arc;

use hacivat_driver_pg::decode::{
    BINARY_FORMAT, TEXT_FORMAT, buf_kind_for, decode_cell, format_for, logical_for,
};
use hacivat_values::{BufKind, DateVal, LogicalType, NetKind, Value, VarKind};
use postgres_types::{Field, Kind, Type};

/// The default cell cap (1 MiB).
const CAP: u32 = 1 << 20;
/// No type modifier.
const NO_MOD: i32 = -1;

const INET: Type = Type::INET;
const CIDR: Type = Type::CIDR;
const VARBIT: Type = Type::VARBIT;
const MONEY: Type = Type::MONEY;

// ---- helpers ----------------------------------------------------------------------------

/// The text of a `Text` cell (anything else fails the test, with the value).
fn text_of(v: Value) -> String {
    match v {
        Value::Text(t) => t.to_string(),
        other => panic!("not a text cell: {other:?}"),
    }
}

fn logical_of(ty: &Type) -> LogicalType {
    logical_for(ty, NO_MOD)
}

fn decode_pg(ty: &Type, raw: &[u8]) -> Value {
    decode_cell(ty, NO_MOD, Some(raw), CAP)
}

fn be32(v: i32) -> [u8; 4] {
    v.to_be_bytes()
}

/// A binary array (`array_send`): ndim, has-null flag, element OID, each dimension's length
/// and lower bound, then each element as a length (-1: NULL) and its bytes.
fn array(elem_oid: u32, dims: &[(i32, i32)], elems: &[Option<Vec<u8>>]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend(be32(dims.len() as i32));
    b.extend(be32(i32::from(elems.iter().any(Option::is_none))));
    b.extend(elem_oid.to_be_bytes());
    for &(len, lower) in dims {
        b.extend(be32(len));
        b.extend(be32(lower));
    }
    for e in elems {
        match e {
            Some(bytes) => {
                b.extend(be32(bytes.len() as i32));
                b.extend(bytes);
            }
            None => b.extend(be32(-1)),
        }
    }
    b
}

fn int4_array_with_lower_bound(lower: i32, values: &[i32]) -> Vec<u8> {
    let elems: Vec<_> = values.iter().map(|v| Some(be32(*v).to_vec())).collect();
    array(23, &[(values.len() as i32, lower)], &elems)
}

fn int4_2d(rows: &[[Option<i32>; 2]; 2]) -> Vec<u8> {
    let elems: Vec<_> = rows
        .iter()
        .flatten()
        .map(|v| v.map(|v| be32(v).to_vec()))
        .collect();
    array(23, &[(2, 1), (2, 1)], &elems)
}

fn text_array(values: &[&str]) -> Vec<u8> {
    let elems: Vec<_> = values.iter().map(|v| Some(v.as_bytes().to_vec())).collect();
    array(25, &[(values.len() as i32, 1)], &elems)
}

/// The array type of a binary array, from the element OID in its header.
fn array_type_of(bytes: &[u8]) -> Type {
    let oid = u32::from_be_bytes(bytes[8..12].try_into().unwrap());
    let elem = Type::from_oid(oid).unwrap_or_else(|| panic!("element OID {oid}"));
    array_type_for(&elem)
}

fn array_type_for(elem: &Type) -> Type {
    [
        Type::BOOL_ARRAY,
        Type::INT2_ARRAY,
        Type::INT4_ARRAY,
        Type::INT8_ARRAY,
        Type::OID_ARRAY,
        Type::FLOAT4_ARRAY,
        Type::FLOAT8_ARRAY,
        Type::NUMERIC_ARRAY,
        Type::TEXT_ARRAY,
        Type::VARCHAR_ARRAY,
        Type::BPCHAR_ARRAY,
        Type::NAME_ARRAY,
        Type::CHAR_ARRAY,
        Type::BYTEA_ARRAY,
        Type::UUID_ARRAY,
        Type::JSON_ARRAY,
        Type::JSONB_ARRAY,
        Type::DATE_ARRAY,
        Type::TIME_ARRAY,
        Type::TIMETZ_ARRAY,
        Type::TIMESTAMP_ARRAY,
        Type::TIMESTAMPTZ_ARRAY,
        Type::INTERVAL_ARRAY,
        Type::INET_ARRAY,
        Type::CIDR_ARRAY,
        Type::MACADDR_ARRAY,
        Type::MACADDR8_ARRAY,
        Type::BIT_ARRAY,
        Type::VARBIT_ARRAY,
    ]
    .into_iter()
    .find(|t| matches!(t.kind(), Kind::Array(e) if e == elem))
    .unwrap_or_else(|| panic!("no array type of {elem}"))
}

fn decode_pg_array(bytes: &[u8]) -> Value {
    decode_cell(&array_type_of(bytes), NO_MOD, Some(bytes), CAP)
}

/// `network_send`: family (2 inet, 3 inet6), bits, is-cidr, address length, the address.
fn net(family: u8, bits: u8, cidr: bool, addr: &[u8]) -> Vec<u8> {
    let mut b = vec![family, bits, u8::from(cidr), addr.len() as u8];
    b.extend(addr);
    b
}

fn inet_v4(addr: [u8; 4], bits: u8) -> Vec<u8> {
    net(2, bits, false, &addr)
}

fn cidr_v4(addr: [u8; 4], bits: u8) -> Vec<u8> {
    net(2, bits, true, &addr)
}

fn inet_v6(words: [u16; 8], bits: u8) -> Vec<u8> {
    let addr: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    net(3, bits, false, &addr)
}

/// `bit_send`: the length in bits, then the bits, most significant first, padded with zeros.
fn varbit(len: i32, byte: u8) -> Vec<u8> {
    let mut b = be32(len).to_vec();
    b.push(byte);
    b
}

fn bits(len: i32, bytes: &[u8]) -> Vec<u8> {
    let mut b = be32(len).to_vec();
    b.extend(bytes);
    b
}

fn is_error(v: &Value) -> bool {
    matches!(v, Value::Error(_))
}

// ---- arrays -----------------------------------------------------------------------------

#[test]
fn pg_array_lower_bounds_round_trip() {
    assert_eq!(
        text_of(decode_pg_array(&int4_array_with_lower_bound(0, &[1, 2, 3]))),
        "[0:2]={1,2,3}"
    );
}

#[test]
fn pg_two_dimensional_array_with_nulls() {
    assert_eq!(
        text_of(decode_pg_array(&int4_2d(&[
            [Some(1), None],
            [Some(3), Some(4)]
        ]))),
        "{{1,NULL},{3,4}}"
    );
}

#[test]
fn pg_text_array_quotes_like_pg() {
    assert_eq!(
        text_of(decode_pg_array(&text_array(&["a b", "\"", ""]))),
        r#"{"a b","\"",""}"#
    );
}

#[test]
fn array_out_quoting_rules() {
    // Quoted: empty, the word NULL in any case, a delimiter, a brace, a quote, a backslash,
    // and every character `array_isspace` takes. Not quoted: anything else, non-ASCII too.
    let cases = [
        ("plain", "plain"),
        ("NULL", "\"NULL\""),
        ("null", "\"null\""),
        ("NuLl", "\"NuLl\""),
        ("NULLS", "NULLS"),
        ("a,b", "\"a,b\""),
        ("{x}", "\"{x}\""),
        ("x}", "\"x}\""),
        ("back\\slash", "\"back\\\\slash\""),
        ("tab\there", "\"tab\there\""),
        ("line\nfeed", "\"line\nfeed\""),
        ("cr\r", "\"cr\r\""),
        ("vt\u{b}", "\"vt\u{b}\""),
        ("ff\u{c}", "\"ff\u{c}\""),
        ("şİı", "şİı"),
        ("a;b", "a;b"),
    ];
    for (elem, quoted) in cases {
        assert_eq!(
            text_of(decode_pg_array(&text_array(&[elem]))),
            format!("{{{quoted}}}"),
            "{elem:?}"
        );
    }
}

#[test]
fn arrays_of_every_m3a_scalar_spell_elements_as_pg() {
    let one = |oid: u32, bytes: Vec<u8>| array(oid, &[(1, 1)], &[Some(bytes)]);
    let day_2000 = 0i32;
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (
            array(16, &[(2, 1)], &[Some(vec![1]), Some(vec![0])]),
            "{t,f}",
        ),
        (one(21, (-2i16).to_be_bytes().to_vec()), "{-2}"),
        (
            one(20, i64::MIN.to_be_bytes().to_vec()),
            "{-9223372036854775808}",
        ),
        (one(26, u32::MAX.to_be_bytes().to_vec()), "{4294967295}"),
        (one(700, 0.1f32.to_be_bytes().to_vec()), "{0.1}"),
        (
            one(700, 1_234_567f32.to_be_bytes().to_vec()),
            "{1.234567e+06}",
        ),
        (one(701, f64::NAN.to_be_bytes().to_vec()), "{NaN}"),
        (
            one(701, f64::NEG_INFINITY.to_be_bytes().to_vec()),
            "{-Infinity}",
        ),
        (one(701, 1e15f64.to_be_bytes().to_vec()), "{1e+15}"),
        // numeric 12345.678: ndigits 3, weight 1, sign +, dscale 3, digits 1 2345 6780.
        (
            one(
                1700,
                [3i16, 1, 0, 3, 1, 2345, 6780]
                    .iter()
                    .flat_map(|d| d.to_be_bytes())
                    .collect(),
            ),
            "{12345.678}",
        ),
        (
            one(
                1700,
                [0i16, 0, -16384, 0]
                    .iter()
                    .flat_map(|d| d.to_be_bytes())
                    .collect(),
            ),
            "{NaN}",
        ),
        (one(1042, b"ab ".to_vec()), "{\"ab \"}"),
        (one(19, b"pg_class".to_vec()), "{pg_class}"),
        (one(18, vec![b'r']), "{r}"),
        (one(18, vec![0]), "{\"\"}"),
        (one(17, vec![0x00, 0xff, 0x0a]), "{\"\\\\x00ff0a\"}"),
        (one(17, vec![]), "{\"\\\\x\"}"),
        (
            one(
                2950,
                vec![
                    0x6f, 0x1e, 0x9a, 0x0c, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 0, 0, 1,
                ],
            ),
            "{6f1e9a0c-0000-4000-8000-000000000001}",
        ),
        (one(114, b"{\"a\":1}".to_vec()), "{\"{\\\"a\\\":1}\"}"),
        (one(3802, [&[1u8][..], b"[1, 2]"].concat()), "{\"[1, 2]\"}"),
        (one(3802, [&[1u8][..], b"1"].concat()), "{1}"),
        (one(1082, be32(day_2000).to_vec()), "{2000-01-01}"),
        (one(1082, be32(i32::MAX).to_vec()), "{infinity}"),
        (one(1082, be32(i32::MIN).to_vec()), "{-infinity}"),
        // 4713-01-01 BC is the proleptic Gregorian year -4712; PostgreSQL counts days from
        // 2000-01-01 (10,957 days after 1970-01-01).
        (
            one(
                1082,
                be32(DateVal::from_civil(-4712, 1, 1).unwrap().0 - 10_957).to_vec(),
            ),
            "{\"4713-01-01 BC\"}",
        ),
        (
            one(1083, 86_400_000_000i64.to_be_bytes().to_vec()),
            "{24:00:00}",
        ),
        (
            one(1083, 1_500_000i64.to_be_bytes().to_vec()),
            "{00:00:01.5}",
        ),
        (
            one(
                1266,
                [
                    3_600_000_000i64.to_be_bytes().to_vec(),
                    be32(-10_800).to_vec(),
                ]
                .concat(),
            ),
            "{01:00:00+03}",
        ),
        // 1999-12-31 23:59:59.999999 is one microsecond before 2000-01-01.
        (
            one(1114, (-1i64).to_be_bytes().to_vec()),
            "{\"1999-12-31 23:59:59.999999\"}",
        ),
        (one(1114, i64::MAX.to_be_bytes().to_vec()), "{infinity}"),
        // timestamptz elements are written in UTC (`+00`): an array has no display zone.
        (
            one(1184, 0i64.to_be_bytes().to_vec()),
            "{\"2000-01-01 00:00:00+00\"}",
        ),
        (
            one(
                1186,
                [
                    7_446_000_007i64.to_be_bytes().to_vec(),
                    be32(3).to_vec(),
                    be32(14).to_vec(),
                ]
                .concat(),
            ),
            "{\"1 year 2 mons 3 days 02:04:06.000007\"}",
        ),
        (
            one(
                1186,
                [
                    0i64.to_be_bytes().to_vec(),
                    be32(0).to_vec(),
                    be32(0).to_vec(),
                ]
                .concat(),
            ),
            "{00:00:00}",
        ),
        (one(869, inet_v4([192, 168, 0, 1], 24)), "{192.168.0.1/24}"),
        (one(650, cidr_v4([10, 0, 0, 0], 8)), "{10.0.0.0/8}"),
        (one(829, vec![8, 0, 0x2b, 1, 2, 3]), "{08:00:2b:01:02:03}"),
        (one(1562, varbit(5, 0b1011_0000)), "{10110}"),
        // An empty array has no dimensions.
        (array(23, &[], &[]), "{}"),
        (array(25, &[(1, 1)], &[None]), "{NULL}"),
        // Lower bounds on every dimension once one is not 1.
        (
            array(
                23,
                &[(1, 1), (2, -1)],
                &[Some(be32(5).to_vec()), Some(be32(6).to_vec())],
            ),
            "[1:1][-1:0]={{5,6}}",
        ),
        // Three dimensions.
        (
            array(
                23,
                &[(2, 1), (1, 1), (2, 1)],
                &(1..=4).map(|v| Some(be32(v).to_vec())).collect::<Vec<_>>(),
            ),
            "{{{1,2}},{{3,4}}}",
        ),
    ];
    for (bytes, want) in cases {
        let v = decode_pg_array(&bytes);
        assert_eq!(text_of(v), want, "{want}");
    }
}

#[test]
fn an_enum_array_spells_its_labels() {
    let mood = Type::new(
        "mood".into(),
        91_001,
        Kind::Enum(vec!["sad".into(), "ok".into(), "happy".into()]),
        "public".into(),
    );
    let moods = Type::new(
        "_mood".into(),
        91_002,
        Kind::Array(mood.clone()),
        "public".into(),
    );
    let bytes = array(
        91_001,
        &[(2, 1)],
        &[Some(b"happy".to_vec()), Some(b"sad".to_vec())],
    );
    assert_eq!(format_for(&moods), BINARY_FORMAT);
    assert_eq!(
        text_of(decode_cell(&moods, NO_MOD, Some(&bytes), CAP)),
        "{happy,sad}"
    );
    assert!(matches!(
        logical_for(&moods, NO_MOD),
        LogicalType::Array(inner) if matches!(*inner, LogicalType::Enum(_))
    ));
}

#[test]
fn array_columns_are_binary_and_typed_by_their_element() {
    assert_eq!(format_for(&Type::INT4_ARRAY), BINARY_FORMAT);
    assert_eq!(
        logical_of(&Type::INT4_ARRAY),
        LogicalType::Array(Box::new(LogicalType::Int {
            bits: 32,
            signed: true
        }))
    );
    assert_eq!(
        buf_kind_for(&Type::INT4_ARRAY, NO_MOD),
        BufKind::Var(VarKind::Text)
    );
    // The column's modifier is its elements': `varchar(10)[]`, `numeric(10,2)[]`.
    assert_eq!(
        logical_for(&Type::VARCHAR_ARRAY, 14),
        LogicalType::Array(Box::new(LogicalType::Text {
            max_chars: Some(10),
            fixed: false
        }))
    );
    assert_eq!(
        logical_for(&Type::NUMERIC_ARRAY, ((10 << 16) | 2) + 4),
        LogicalType::Array(Box::new(LogicalType::Decimal {
            precision: Some(10),
            scale: Some(2)
        }))
    );
    assert_eq!(
        logical_of(&Type::INET_ARRAY),
        LogicalType::Array(Box::new(LogicalType::Network(NetKind::Inet)))
    );
    // An array of a type without a binary decoder stays text, typed by its element.
    assert_eq!(format_for(&Type::MONEY_ARRAY), TEXT_FORMAT);
    assert_eq!(
        logical_of(&Type::MONEY_ARRAY),
        LogicalType::Array(Box::new(LogicalType::Money))
    );
    assert_eq!(
        text_of(decode_pg(&Type::MONEY_ARRAY, b"{$1.00,NULL}")),
        "{$1.00,NULL}"
    );
    // `int2vector` and `oidvector` print as space-separated lists, not array literals.
    for ty in [Type::INT2_VECTOR, Type::OID_VECTOR] {
        assert_eq!(format_for(&ty), TEXT_FORMAT, "{ty}");
        assert!(matches!(
            decode_pg(&ty, b"1 2 3"),
            Value::Opaque(o) if &*o.text == "1 2 3"
        ));
    }
}

#[test]
fn an_array_literal_past_the_cap_is_truncated_with_its_whole_length() {
    // 1,000 elements of 7 characters each, plus braces and commas: 8,001 bytes.
    let elems: Vec<_> = (0..1_000).map(|_| Some(b"abcdefg".to_vec())).collect();
    let bytes = array(25, &[(1_000, 1)], &elems);
    match decode_cell(&Type::TEXT_ARRAY, NO_MOD, Some(&bytes), 100) {
        Value::Truncated(t) => {
            assert_eq!(t.total_len, Some(8_001));
            // The first 100 bytes: the brace, twelve elements with their commas, three letters.
            let prefix = format!("{{{}abc", "abcdefg,".repeat(12));
            assert_eq!(t.prefix, Value::Text(prefix.into()));
        }
        other => panic!("{other:?}"),
    }
    // Text that is not UTF-8 keeps its bytes.
    let bytes = array(25, &[(1, 1)], &[Some(vec![0x61, 0xc3])]);
    assert!(matches!(
        decode_cell(&Type::TEXT_ARRAY, NO_MOD, Some(&bytes), CAP),
        Value::Lossy(b) if b[..] == [b'{', 0x61, 0xc3, b'}']
    ));
}

#[test]
fn hostile_array_header_is_an_error_cell() {
    let ok = int4_array_with_lower_bound(1, &[1, 2]);
    let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();
    // ndim 7 (PostgreSQL's MAXDIM is 6), and a negative ndim.
    let mut seven = ok.clone();
    seven[..4].copy_from_slice(&be32(7));
    cases.push(("ndim 7", seven));
    let mut negative_ndim = ok.clone();
    negative_ndim[..4].copy_from_slice(&be32(-1));
    cases.push(("negative ndim", negative_ndim));
    // A negative dimension length, and a zero one.
    let mut negative = ok.clone();
    negative[12..16].copy_from_slice(&be32(-2));
    cases.push(("negative length", negative));
    let mut zero = ok.clone();
    zero[12..16].copy_from_slice(&be32(0));
    cases.push(("zero length", zero));
    // An element length past the buffer.
    let mut past = ok.clone();
    past[20..24].copy_from_slice(&be32(1_000));
    cases.push(("element past the buffer", past));
    // A length below -1.
    let mut below = ok.clone();
    below[20..24].copy_from_slice(&be32(-2));
    cases.push(("element length -2", below));
    // Flags other than 0 and 1.
    let mut flags = ok.clone();
    flags[4..8].copy_from_slice(&be32(2));
    cases.push(("flags 2", flags));
    // An element OID that is not the column's.
    let mut oid = ok.clone();
    oid[8..12].copy_from_slice(&25u32.to_be_bytes());
    cases.push(("element OID", oid));
    // More elements than the bytes could hold (dimensions whose product overflows too).
    cases.push((
        "huge dimensions",
        array(23, &[(i32::MAX, 1), (i32::MAX, 1)], &[]),
    ));
    cases.push((
        "more elements than bytes",
        array(23, &[(3, 1)], &[Some(be32(1).to_vec())]),
    ));
    // Bytes after the last element.
    let mut trailing = ok.clone();
    trailing.push(0);
    cases.push(("trailing bytes", trailing));
    // An upper bound past i32.
    cases.push((
        "upper bound overflow",
        array(
            23,
            &[(2, i32::MAX)],
            &[Some(be32(1).to_vec()), Some(be32(2).to_vec())],
        ),
    ));
    // A header cut short, and an element of the wrong width.
    cases.push(("short header", ok[..10].to_vec()));
    cases.push(("short dimensions", ok[..15].to_vec()));
    cases.push((
        "int4 of 3 bytes",
        array(23, &[(1, 1)], &[Some(vec![0, 0, 1])]),
    ));
    for (name, bytes) in cases {
        let v = decode_cell(&Type::INT4_ARRAY, NO_MOD, Some(&bytes), CAP);
        assert!(is_error(&v), "{name}: {v:?}");
    }
    // One bad element makes the whole cell an error: never an array with a hole in it.
    let v = decode_pg_array(&array(16, &[(2, 1)], &[Some(vec![1]), Some(vec![2])]));
    assert!(is_error(&v), "{v:?}");
}

#[test]
fn hostile_bytes_never_panic_in_the_new_column_kinds() {
    let noise: Vec<u8> = (0u16..96).map(|i| (i * 37 % 256) as u8).collect();
    let mut types = vec![
        Type::INET,
        Type::CIDR,
        Type::MACADDR,
        Type::MACADDR8,
        Type::BIT,
        Type::VARBIT,
        Type::MONEY,
        Type::XML,
        Type::POINT,
        Type::INT4_RANGE,
        Type::RECORD,
    ];
    let elems = [
        Type::BOOL,
        Type::INT4,
        Type::FLOAT4,
        Type::NUMERIC,
        Type::TEXT,
        Type::BYTEA,
        Type::UUID,
        Type::JSONB,
        Type::DATE,
        Type::TIME,
        Type::TIMETZ,
        Type::TIMESTAMPTZ,
        Type::INTERVAL,
        Type::INET,
        Type::VARBIT,
    ];
    types.extend(elems.iter().map(array_type_for));
    for ty in &types {
        for len in 0..=noise.len() {
            let _ = decode_cell(ty, NO_MOD, Some(&noise[..len]), 16);
        }
    }
    // A valid header over every element type, then noise.
    for elem in &elems {
        let ty = array_type_for(elem);
        for len in 0..=32 {
            let mut b = array(elem.oid(), &[(2, 1)], &[]);
            b.extend(&noise[..len]);
            let _ = decode_cell(&ty, NO_MOD, Some(&b), 16);
        }
    }
}

// ---- network and bit strings ------------------------------------------------------------

#[test]
fn inet_and_cidr_canonical_text() {
    assert_eq!(
        text_of(decode_pg(&INET, &inet_v4([10, 0, 0, 1], 32))),
        "10.0.0.1"
    );
    assert_eq!(
        text_of(decode_pg(&CIDR, &cidr_v4([10, 0, 0, 0], 8))),
        "10.0.0.0/8"
    );
}

#[test]
fn network_types_print_as_network_out() {
    let cases: Vec<(Type, Vec<u8>, &str)> = vec![
        (INET, inet_v4([192, 168, 0, 1], 24), "192.168.0.1/24"),
        (CIDR, cidr_v4([192, 168, 1, 7], 32), "192.168.1.7/32"),
        (INET, inet_v6([0, 0, 0, 0, 0, 0, 0, 1], 128), "::1"),
        (INET, inet_v6([0; 8], 128), "::"),
        (INET, inet_v6([0; 8], 0), "::/0"),
        (
            INET,
            inet_v6([0x2001, 0xdb8, 0, 0, 0, 0, 0, 1], 64),
            "2001:db8::1/64",
        ),
        (INET, inet_v6([1, 0, 0, 0, 0, 0, 0, 0], 128), "1::"),
        // The first of two equal zero runs is shortened; a single zero word is not.
        (INET, inet_v6([1, 0, 0, 1, 0, 0, 1, 1], 128), "1::1:0:0:1:1"),
        (
            INET,
            inet_v6([1, 0, 1, 1, 1, 1, 1, 1], 128),
            "1:0:1:1:1:1:1:1",
        ),
        (
            INET,
            inet_v6([0, 0, 0, 0, 0, 0xffff, 0x0102, 0x0304], 128),
            "::ffff:1.2.3.4",
        ),
        (
            INET,
            inet_v6([0, 0, 0, 0, 0, 0, 0x0102, 0x0304], 128),
            "::1.2.3.4",
        ),
        (
            INET,
            inet_v6([0xfe80, 0, 0, 0, 0xabcd, 0, 0, 0x10], 10),
            "fe80::abcd:0:0:10/10",
        ),
        (
            CIDR,
            {
                let mut b = inet_v6([0x2001, 0xdb8, 0, 0, 0, 0, 0, 0], 32);
                b[2] = 1;
                b
            },
            "2001:db8::/32",
        ),
        (
            CIDR,
            {
                let mut b = inet_v6([0x2001, 0xdb8, 1, 2, 3, 4, 5, 6], 128);
                b[2] = 1;
                b
            },
            "2001:db8:1:2:3:4:5:6/128",
        ),
        (
            Type::MACADDR,
            vec![8, 0, 0x2b, 1, 2, 3],
            "08:00:2b:01:02:03",
        ),
        (
            Type::MACADDR8,
            vec![8, 0, 0x2b, 0xff, 0xfe, 1, 2, 3],
            "08:00:2b:ff:fe:01:02:03",
        ),
    ];
    for (ty, bytes, want) in cases {
        assert_eq!(format_for(&ty), BINARY_FORMAT, "{ty}");
        assert_eq!(text_of(decode_pg(&ty, &bytes)), want, "{ty} {want}");
    }
    assert_eq!(logical_of(&INET), LogicalType::Network(NetKind::Inet));
    assert_eq!(logical_of(&CIDR), LogicalType::Network(NetKind::Cidr));
    assert_eq!(
        logical_of(&Type::MACADDR),
        LogicalType::Network(NetKind::MacAddr)
    );
    assert_eq!(
        logical_of(&Type::MACADDR8),
        LogicalType::Network(NetKind::MacAddr8)
    );
}

#[test]
fn hostile_network_bytes_are_error_cells() {
    let cases: Vec<(Type, Vec<u8>)> = vec![
        (INET, vec![]),
        (INET, vec![2, 32, 0]),
        // An unknown family.
        (INET, net(4, 32, false, &[1, 2, 3, 4])),
        // Bits past the family's width.
        (INET, net(2, 33, false, &[1, 2, 3, 4])),
        (INET, net(3, 129, false, &[0; 16])),
        // An address length that is not the family's, or not the bytes that follow.
        (INET, net(2, 32, false, &[1, 2, 3])),
        (INET, net(3, 128, false, &[0; 4])),
        (INET, {
            let mut b = inet_v4([1, 2, 3, 4], 32);
            b.push(0);
            b
        }),
        (INET, {
            let mut b = inet_v4([1, 2, 3, 4], 32);
            b[3] = 16;
            b
        }),
        // A cidr flag on an inet column, and none on a cidr one.
        (INET, cidr_v4([10, 0, 0, 0], 8)),
        (CIDR, inet_v4([10, 0, 0, 0], 8)),
        (INET, net(2, 32, true, &[1, 2, 3, 4])),
        (Type::MACADDR, vec![1, 2, 3, 4, 5]),
        (Type::MACADDR, vec![1, 2, 3, 4, 5, 6, 7]),
        (Type::MACADDR8, vec![1, 2, 3, 4, 5, 6]),
    ];
    for (ty, bytes) in cases {
        let v = decode_pg(&ty, &bytes);
        assert!(is_error(&v), "{ty} {bytes:?}: {v:?}");
    }
}

#[test]
#[allow(
    clippy::unusual_byte_groupings,
    reason = "the plan's literal: the five value bits, then the three pad bits"
)]
fn varbit_keeps_its_length() {
    assert_eq!(
        text_of(decode_pg(&VARBIT, &varbit(5, 0b10110_000))),
        "10110"
    );
}

#[test]
fn bit_strings_print_every_bit() {
    assert_eq!(text_of(decode_pg(&Type::BIT, &bits(0, &[]))), "");
    assert_eq!(
        text_of(decode_pg(&Type::BIT, &bits(8, &[0b1000_0001]))),
        "10000001"
    );
    assert_eq!(
        text_of(decode_pg(&Type::VARBIT, &bits(12, &[0xff, 0b0101_0000]))),
        "111111110101"
    );
    // The pad bits after the length are not part of the value.
    assert_eq!(text_of(decode_pg(&Type::BIT, &bits(1, &[0xff]))), "1");
    assert_eq!(format_for(&Type::BIT), BINARY_FORMAT);
    assert_eq!(
        logical_for(&Type::BIT, 5),
        LogicalType::BitString { len: Some(5) }
    );
    assert_eq!(
        logical_for(&Type::VARBIT, NO_MOD),
        LogicalType::BitString { len: None }
    );
}

#[test]
fn hostile_bit_strings_are_error_cells() {
    for bytes in [
        vec![0, 0, 0],
        bits(-1, &[]),
        bits(9, &[0xff]),
        bits(8, &[0xff, 0]),
        bits(i32::MAX, &[0xff]),
    ] {
        let v = decode_pg(&VARBIT, &bytes);
        assert!(is_error(&v), "{bytes:?}: {v:?}");
    }
}

// ---- typed text columns -----------------------------------------------------------------

#[test]
fn money_is_typed_text() {
    assert!(matches!(logical_of(&MONEY), LogicalType::Money));
}

#[test]
fn text_format_columns_carry_their_own_logical_type() {
    let pair = Type::new(
        "pair".into(),
        91_010,
        Kind::Composite(vec![
            Field::new("a".into(), Type::INT4),
            Field::new("b".into(), Type::TEXT),
        ]),
        "public".into(),
    );
    let cases: Vec<(Type, LogicalType, &[u8], Value)> = vec![
        (
            MONEY,
            LogicalType::Money,
            b"$1,234.50",
            Value::Text("$1,234.50".into()),
        ),
        (
            Type::XML,
            LogicalType::Xml,
            b"<a>1</a>",
            Value::Xml("<a>1</a>".into()),
        ),
        (
            Type::POINT,
            LogicalType::Geometry { srid: None },
            b"(1,2)",
            Value::Text("(1,2)".into()),
        ),
        (
            Type::BOX,
            LogicalType::Geometry { srid: None },
            b"(1,1),(0,0)",
            Value::Text("(1,1),(0,0)".into()),
        ),
        (
            Type::INT4_RANGE,
            LogicalType::Range(Box::new(LogicalType::Int {
                bits: 32,
                signed: true,
            })),
            b"[1,3)",
            Value::Text("[1,3)".into()),
        ),
        (
            Type::RECORD,
            LogicalType::Composite(Arc::from([])),
            b"(1,a)",
            Value::Text("(1,a)".into()),
        ),
        // `record[]` is a pseudo-type, not `Kind::Array`: typed by OID.
        (
            Type::RECORD_ARRAY,
            LogicalType::Array(Box::new(LogicalType::Composite(Arc::from([])))),
            b"{\"(1,a)\"}",
            Value::Text("{\"(1,a)\"}".into()),
        ),
        (
            pair.clone(),
            LogicalType::Composite(Arc::from([
                (
                    Arc::from("a"),
                    LogicalType::Int {
                        bits: 32,
                        signed: true,
                    },
                ),
                (
                    Arc::from("b"),
                    LogicalType::Text {
                        max_chars: None,
                        fixed: false,
                    },
                ),
            ])),
            b"(1,\"a b\")",
            Value::Text("(1,\"a b\")".into()),
        ),
    ];
    for (ty, logical, raw, value) in cases {
        assert_eq!(format_for(&ty), TEXT_FORMAT, "{ty}");
        assert_eq!(logical_for(&ty, NO_MOD), logical, "{ty}");
        assert_eq!(
            buf_kind_for(&ty, NO_MOD),
            BufKind::Var(VarKind::Text),
            "{ty}"
        );
        assert_eq!(decode_pg(&ty, raw), value, "{ty}");
    }
    // A multirange is a range of its element; tsvector and extension types stay opaque.
    let multi = Type::new(
        "int4multirange".into(),
        4_451,
        Kind::Multirange(Type::INT4),
        "pg_catalog".into(),
    );
    assert_eq!(
        logical_for(&multi, NO_MOD),
        LogicalType::Range(Box::new(LogicalType::Int {
            bits: 32,
            signed: true
        }))
    );
    assert_eq!(
        logical_of(&Type::TS_VECTOR),
        LogicalType::Opaque {
            type_name: "tsvector".into()
        }
    );
    // Typed text past the cap is truncated, and text that is not UTF-8 is kept as bytes.
    assert!(matches!(
        decode_cell(&MONEY, NO_MOD, Some(b"$1,234.50"), 4),
        Value::Truncated(t) if t.total_len == Some(9)
    ));
    assert!(matches!(
        decode_cell(&Type::POINT, NO_MOD, Some(&[0xff]), CAP),
        Value::Lossy(_)
    ));
}

#[test]
fn a_composite_nested_past_the_depth_limit_is_opaque_inside() {
    let mut ty = Type::INT4;
    for i in 0..40 {
        ty = Type::new(
            format!("nest{i}"),
            92_000 + i,
            Kind::Composite(vec![Field::new("x".into(), ty)]),
            "public".into(),
        );
    }
    // Every level up to the limit is a composite; below it the field is opaque. Never a stack
    // overflow, whatever the nesting.
    let mut logical = logical_of(&ty);
    let mut depth = 0;
    while let LogicalType::Composite(fields) = logical {
        logical = fields[0].1.clone();
        depth += 1;
    }
    assert!((1..40).contains(&depth), "{depth}");
    assert!(matches!(logical, LogicalType::Opaque { .. }), "{logical:?}");
}
