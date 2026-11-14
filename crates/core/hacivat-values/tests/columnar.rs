//! M3a Task 6: columnar chunks (HV-RES-02 buffers): every `BufKind` with NULLs and
//! exceptions, lossless and UTF-8-safe text, exact byte accounting, bounded builder paths.

use std::sync::Arc;

use hacivat_values::temporal::MICROS_PER_DAY;
use hacivat_values::{
    BufKind, CellFlags, Chunk, ChunkBuilder, ColumnMeta, DateVal, DecimalVal, ErrorVal, LenUnit,
    LogicalType, NativeType, ResultSchema, SpecialVal, TempUnit, TimeVal, TsVal, Value, ValueRef,
    VarKind,
};
use proptest::prelude::*;

const TEXT: BufKind = BufKind::Var(VarKind::Text);

fn schema(logical: &[LogicalType]) -> ResultSchema {
    ResultSchema {
        columns: logical
            .iter()
            .enumerate()
            .map(|(i, l)| ColumnMeta {
                name: format!("c{i}").into(),
                logical: l.clone(),
                native: NativeType::Unknown,
                origin: None,
            })
            .collect(),
    }
}

fn chunk_text(s: String, cap: u32) -> Chunk {
    chunk_text_bytes(s.as_bytes(), cap)
}

fn chunk_text_bytes(bytes: &[u8], cap: u32) -> Chunk {
    let mut b = ChunkBuilder::new(&[TEXT], 1);
    b.push_bytes(0, bytes, cap);
    b.end_row();
    b.seal()
}

fn chunk_i64(rows: u32) -> Chunk {
    let mut b = ChunkBuilder::new(&[BufKind::I64], rows);
    for r in 0..rows {
        b.push_i64(0, i64::from(r));
        b.end_row();
    }
    b.seal()
}

fn error(code: &str) -> Value {
    Value::Error(Arc::new(ErrorVal {
        code: code.into(),
        message: "bad bytes".into(),
    }))
}

fn dec(s: &str) -> DecimalVal {
    DecimalVal::parse(s).unwrap()
}

/// One `BufKind` case: the plan, the logical type it is read with, a typed push of `sample`,
/// and an exception the buffer cannot hold.
struct Case {
    name: &'static str,
    kind: BufKind,
    logical: LogicalType,
    push: fn(&mut ChunkBuilder),
    sample: Value,
    exception: Value,
}

fn cases() -> Vec<Case> {
    let ts_us = 1_700_000_000_123_456;
    let text = || LogicalType::Text {
        max_chars: None,
        fixed: false,
    };
    vec![
        Case {
            name: "bool",
            kind: BufKind::Bool,
            logical: LogicalType::Bool,
            push: |b| b.push_bool(0, true),
            sample: Value::Bool(true),
            // A SQLite BOOLEAN column holding 2.
            exception: Value::Int(2),
        },
        Case {
            name: "i64",
            kind: BufKind::I64,
            logical: LogicalType::Int {
                bits: 64,
                signed: true,
            },
            push: |b| b.push_i64(0, -42),
            sample: Value::Int(-42),
            // A SQLite INTEGER column holding 'abc'.
            exception: Value::Text("abc".into()),
        },
        Case {
            name: "u64",
            kind: BufKind::U64,
            logical: LogicalType::Int {
                bits: 64,
                signed: false,
            },
            push: |b| b.push_u64(0, u64::MAX),
            sample: Value::UInt(u64::MAX),
            exception: error("decode"),
        },
        Case {
            name: "f64",
            kind: BufKind::F64 { f32: false },
            logical: LogicalType::Float { bits: 64 },
            push: |b| b.push_f64(0, 1.25),
            sample: Value::F64(1.25),
            exception: Value::Special(SpecialVal::NaN),
        },
        Case {
            name: "f32",
            kind: BufKind::F64 { f32: true },
            logical: LogicalType::Float { bits: 32 },
            push: |b| b.push_f64(0, f64::from(0.1f32)),
            sample: Value::F32(0.1),
            // Not an f32: kept as it came, never rounded.
            exception: Value::F64(0.1),
        },
        Case {
            name: "dec64",
            kind: BufKind::Dec64 { scale: 2 },
            logical: LogicalType::Decimal {
                precision: Some(14),
                scale: Some(2),
            },
            push: |b| b.push_dec64(0, 150),
            sample: Value::Decimal(dec("1.50")),
            exception: Value::Special(SpecialVal::PosInfinity),
        },
        Case {
            name: "dec128",
            kind: BufKind::Dec128,
            logical: LogicalType::Decimal {
                precision: None,
                scale: None,
            },
            push: |b| b.push_decimal(0, &dec("12345678901234567890.123456789")),
            sample: Value::Decimal(dec("12345678901234567890.123456789")),
            exception: Value::Decimal(dec(&"7".repeat(40))),
        },
        Case {
            name: "date",
            kind: BufKind::Temporal {
                unit: TempUnit::Days,
            },
            logical: LogicalType::Date,
            push: |b| b.push_temporal(0, -2_440_588),
            sample: Value::Date(DateVal(-2_440_588)),
            exception: Value::Special(SpecialVal::PosInfinity),
        },
        Case {
            name: "time",
            kind: BufKind::Temporal {
                unit: TempUnit::Micros,
            },
            logical: LogicalType::Time {
                precision: 6,
                tz: false,
            },
            push: |b| b.push_temporal(0, MICROS_PER_DAY),
            sample: Value::Time(TimeVal::END_OF_DAY),
            // A MySQL TIME beyond a day is a duration.
            exception: Value::Interval {
                months: 0,
                days: 34,
                micros: 3_599_000_000,
            },
        },
        Case {
            name: "timestamp",
            kind: BufKind::Temporal {
                unit: TempUnit::Micros,
            },
            logical: LogicalType::Timestamp {
                precision: 6,
                tz: false,
            },
            push: |b| b.push_temporal(0, 1_700_000_000_123_456),
            sample: Value::Timestamp(TsVal::from_unix_micros(ts_us)),
            exception: Value::Special(SpecialVal::ZeroDate),
        },
        Case {
            name: "timestamptz",
            kind: BufKind::Temporal {
                unit: TempUnit::Micros,
            },
            logical: LogicalType::Timestamp {
                precision: 6,
                tz: true,
            },
            push: |b| b.push_temporal(0, -1),
            sample: Value::TimestampTz {
                utc: TsVal::from_unix_micros(-1),
                offset: None,
            },
            exception: Value::Special(SpecialVal::NegInfinity),
        },
        Case {
            name: "timetz",
            kind: BufKind::Temporal2,
            logical: LogicalType::Time {
                precision: 6,
                tz: true,
            },
            push: |b| b.push_temporal2(0, 0, 10_800),
            sample: Value::TimeTz(TimeVal { nanos_of_day: 0 }, 10_800),
            exception: error("decode"),
        },
        Case {
            name: "timestamptz with offset",
            kind: BufKind::Temporal2,
            logical: LogicalType::Timestamp {
                precision: 7,
                tz: true,
            },
            push: |b| b.push_temporal2(0, 1_700_000_000_123_456, -18_000),
            sample: Value::TimestampTz {
                utc: TsVal::from_unix_micros(ts_us),
                offset: Some(-18_000),
            },
            exception: Value::Special(SpecialVal::PartialZeroDate("2026-00-13".into())),
        },
        Case {
            name: "text",
            kind: TEXT,
            logical: text(),
            push: |b| b.push_bytes(0, "Müşteri".as_bytes(), 1 << 20),
            sample: Value::Text("Müşteri".into()),
            exception: Value::Lossy(Arc::from(&[0x61, 0xc3][..])),
        },
        Case {
            name: "xml",
            kind: TEXT,
            logical: LogicalType::Xml,
            push: |b| b.push_bytes(0, b"<a/>", 1 << 20),
            sample: Value::Xml("<a/>".into()),
            exception: error("decode"),
        },
        Case {
            name: "json",
            kind: BufKind::Var(VarKind::Json),
            logical: LogicalType::Json { binary: true },
            push: |b| b.push_bytes(0, br#"{"a":1}"#, 1 << 20),
            sample: Value::Json(r#"{"a":1}"#.into()),
            exception: error("jsonb-version"),
        },
        Case {
            name: "bytes",
            kind: BufKind::Var(VarKind::Bytes),
            logical: LogicalType::Bytes { max: None },
            push: |b| b.push_bytes(0, &[0, 1, 255], 1 << 20),
            sample: Value::Bytes(Arc::from(&[0, 1, 255][..])),
            exception: error("decode"),
        },
        Case {
            name: "uuid",
            kind: BufKind::Fixed16,
            logical: LogicalType::Uuid,
            push: |b| b.push_fixed16(0, [7; 16]),
            sample: Value::Uuid([7; 16]),
            exception: Value::Bytes(Arc::from(&[7; 15][..])),
        },
        Case {
            name: "values",
            kind: BufKind::Values,
            logical: LogicalType::Interval,
            push: |b| {
                b.push_value(
                    0,
                    Value::Interval {
                        months: 1,
                        days: -2,
                        micros: 3,
                    },
                )
            },
            sample: Value::Interval {
                months: 1,
                days: -2,
                micros: 3,
            },
            exception: error("decode"),
        },
    ]
}

fn expected_flags(v: &Value) -> CellFlags {
    match v {
        Value::Lossy(_) => CellFlags::LOSSY,
        Value::Error(_) => CellFlags::ERROR,
        Value::Special(_) => CellFlags::SPECIAL,
        _ => CellFlags::empty(),
    }
}

#[test]
fn every_buf_kind_holds_nulls_and_exceptions() {
    for case in cases() {
        let s = schema(std::slice::from_ref(&case.logical));
        let mut b = ChunkBuilder::with_schema(&[case.kind], &s, 4);
        (case.push)(&mut b);
        b.end_row();
        b.push_null(0);
        b.end_row();
        b.push_value(0, case.exception.clone());
        b.end_row();
        b.push_value(0, case.sample.clone());
        b.end_row();
        let built = b.bytes();
        let c = b.seal();
        let name = case.name;
        assert_eq!(c.rows(), 4, "{name}");
        assert_eq!(c.bytes(), built, "{name}: sealing changes no count");
        assert_eq!(c.cell(0, 0), case.sample.view(), "{name}: typed push");
        assert_eq!(c.cell(1, 0), ValueRef::Null, "{name}");
        assert_eq!(c.cell(2, 0), case.exception.view(), "{name}: exception");
        assert_eq!(c.cell(3, 0), case.sample.view(), "{name}: push_value");
        assert_eq!(c.flags(0, 0), CellFlags::empty(), "{name}");
        assert_eq!(c.flags(1, 0), CellFlags::empty(), "{name}");
        assert_eq!(c.flags(2, 0), expected_flags(&case.exception), "{name}");
    }
}

#[test]
fn push_value_stores_what_the_buffer_holds_inline() {
    // Two typed pushes and two whole values make the same chunk: no exception is created.
    for case in cases() {
        let s = schema(std::slice::from_ref(&case.logical));
        let mut typed = ChunkBuilder::with_schema(&[case.kind], &s, 2);
        let mut whole = ChunkBuilder::with_schema(&[case.kind], &s, 2);
        for _ in 0..2 {
            (case.push)(&mut typed);
            typed.end_row();
            whole.push_value(0, case.sample.clone());
            whole.end_row();
        }
        assert_eq!(typed.seal(), whole.seal(), "{}", case.name);
    }
}

#[test]
fn var_text_truncates_at_a_char_boundary() {
    let c = chunk_text("ğ".repeat(600_000), 1 << 20);
    assert!(matches!(c.cell(0, 0), ValueRef::Truncated(t) if t.total_len == Some(1_200_000)));
    let ValueRef::Truncated(t) = c.cell(0, 0) else {
        unreachable!()
    };
    let Value::Text(prefix) = &t.prefix else {
        panic!("a text prefix: {:?}", t.prefix)
    };
    assert_eq!(prefix.len(), 1 << 20);
    assert_eq!(prefix.chars().count(), 524_288);
    assert_eq!(t.unit, LenUnit::Bytes);
    assert_eq!(c.flags(0, 0), CellFlags::TRUNCATED);
    // A cap inside a three-byte character moves back to its start.
    let c = chunk_text("€".repeat(10), 10);
    let ValueRef::Truncated(t) = c.cell(0, 0) else {
        panic!("truncated")
    };
    assert_eq!(t.prefix, Value::Text("€€€".into()));
    assert_eq!(t.total_len, Some(30));
}

#[test]
fn invalid_utf8_text_becomes_lossy() {
    let c = chunk_text_bytes(&[0x66, 0xff, 0x6f], 1 << 20);
    assert!(matches!(c.cell(0, 0), ValueRef::Lossy(b) if b == [0x66, 0xff, 0x6f]));
    assert_eq!(c.flags(0, 0), CellFlags::LOSSY);
}

#[test]
fn lossy_text_over_the_cap_is_truncated_and_lossy() {
    // Nine `a`, `é` (two bytes) across the cap of 10, then an invalid byte.
    let mut bytes = vec![b'a'; 9];
    bytes.extend_from_slice("é".as_bytes());
    bytes.push(0xff);
    let c = chunk_text_bytes(&bytes, 10);
    let ValueRef::Truncated(t) = c.cell(0, 0) else {
        panic!("truncated")
    };
    assert_eq!(t.prefix, Value::Lossy(Arc::from(&bytes[..9])));
    assert_eq!(t.total_len, Some(12));
    assert_eq!(c.flags(0, 0), CellFlags::TRUNCATED | CellFlags::LOSSY);
}

#[test]
fn bytes_over_the_cap_are_truncated_at_the_cap() {
    let mut b = ChunkBuilder::new(&[BufKind::Var(VarKind::Bytes)], 1);
    b.push_bytes(0, &[1, 2, 3, 4, 5], 3);
    b.end_row();
    let c = b.seal();
    let ValueRef::Truncated(t) = c.cell(0, 0) else {
        panic!("truncated")
    };
    assert_eq!(t.prefix, Value::Bytes(Arc::from(&[1, 2, 3][..])));
    assert_eq!(t.total_len, Some(5));
}

#[test]
fn bytes_accounting_is_exact() {
    let c = chunk_i64(4_096);
    assert_eq!(c.bytes(), 4_096 * 8 + 4_096 / 8);
}

#[test]
fn bytes_accounting_is_exact_for_every_layout() {
    let build = |kind: BufKind, rows: &dyn Fn(&mut ChunkBuilder)| {
        let mut b = ChunkBuilder::new(&[kind], 1_000);
        rows(&mut b);
        let before = b.bytes();
        let c = b.seal();
        assert_eq!(c.bytes(), before);
        c.bytes()
    };
    let word = 8;
    // Var: a validity word, rows + 1 offsets, the data.
    let var = build(TEXT, &|b| {
        b.push_bytes(0, b"ab", 64);
        b.end_row();
        b.push_null(0);
        b.end_row();
        b.push_bytes(0, b"cde", 64);
        b.end_row();
    });
    assert_eq!(var, word + 4 * 4 + 5);
    // Bool: validity and value bits, two words each for 100 rows.
    let bools = build(BufKind::Bool, &|b| {
        for i in 0..100 {
            b.push_bool(0, i % 2 == 0);
            b.end_row();
        }
    });
    assert_eq!(bools, 2 * 2 * word);
    // f32: four bytes a cell.
    let f32s = build(BufKind::F64 { f32: true }, &|b| {
        for _ in 0..3 {
            b.push_f64(0, 0.5);
            b.end_row();
        }
    });
    assert_eq!(f32s, word + 3 * 4);
    // Temporal2: twelve bytes a cell.
    let pairs = build(BufKind::Temporal2, &|b| {
        for _ in 0..2 {
            b.push_temporal2(0, 1, 2);
            b.end_row();
        }
    });
    assert_eq!(pairs, word + 2 * 12);
    // Dec64: a whole DecimalVal a cell.
    let decs = build(BufKind::Dec64 { scale: 2 }, &|b| {
        for _ in 0..2 {
            b.push_dec64(0, 1);
            b.end_row();
        }
    });
    assert_eq!(decs, word + 2 * size_of::<DecimalVal>() as u64);
    // An exception: its row, its value and its payload.
    let exc = build(BufKind::I64, &|b| {
        b.push_i64(0, 1);
        b.end_row();
        b.push_value(0, Value::Text("abc".into()));
        b.end_row();
    });
    assert_eq!(exc, word + 2 * 8 + 4 + size_of::<Value>() as u64 + 3);
}

#[test]
#[should_panic(expected = "column 3 outside plan of 2")]
fn out_of_plan_column_panics() {
    ChunkBuilder::new(&[BufKind::I64, BufKind::Bool], 1).push_i64(3, 1);
}

#[test]
#[should_panic(expected = "column 5 outside plan of 1")]
fn out_of_plan_column_panics_for_every_push() {
    ChunkBuilder::new(&[BufKind::Values], 1).push_value(5, Value::Null);
}

#[test]
#[should_panic(expected = "push_i64 on column 1, a Bool column")]
fn a_push_of_the_wrong_kind_panics() {
    let mut b = ChunkBuilder::new(&[BufKind::I64, BufKind::Bool], 1);
    b.push_i64(1, 1);
}

#[test]
#[should_panic(expected = "push_dec64 on column 0, a Dec128 column")]
fn dec64_needs_a_scale() {
    ChunkBuilder::new(&[BufKind::Dec128], 1).push_dec64(0, 1);
}

#[test]
#[should_panic(expected = "column 0 pushed twice in row 0")]
fn a_column_pushed_twice_in_a_row_panics() {
    let mut b = ChunkBuilder::new(&[BufKind::I64], 1);
    b.push_i64(0, 1);
    b.push_null(0);
}

#[test]
#[should_panic(expected = "column 1 has no cell in row 0")]
fn a_row_missing_a_column_panics() {
    let mut b = ChunkBuilder::new(&[BufKind::I64, BufKind::I64], 1);
    b.push_i64(0, 1);
    b.end_row();
}

#[test]
#[should_panic(expected = "seal in the middle of row 1")]
fn sealing_mid_row_panics() {
    let mut b = ChunkBuilder::new(&[BufKind::I64, BufKind::I64], 1);
    b.push_i64(0, 1);
    b.push_i64(1, 1);
    b.end_row();
    b.push_i64(1, 2);
    b.seal();
}

#[test]
#[should_panic(expected = "a plan of 1 columns for a schema of 2")]
fn a_schema_must_match_the_plan() {
    let s = schema(&[LogicalType::Bool, LogicalType::Bool]);
    ChunkBuilder::with_schema(&[BufKind::Bool], &s, 1);
}

#[test]
fn out_of_range_temporal_data_is_an_error_cell_not_a_panic() {
    let s = schema(&[
        LogicalType::Date,
        LogicalType::Time {
            precision: 6,
            tz: false,
        },
        LogicalType::Time {
            precision: 6,
            tz: true,
        },
    ]);
    let plan = [
        BufKind::Temporal {
            unit: TempUnit::Days,
        },
        BufKind::Temporal {
            unit: TempUnit::Micros,
        },
        BufKind::Temporal2,
    ];
    let mut b = ChunkBuilder::with_schema(&plan, &s, 2);
    b.push_temporal(0, i64::MAX);
    b.push_temporal(1, -1);
    b.push_temporal2(2, MICROS_PER_DAY + 1, 0);
    b.end_row();
    let c = b.seal();
    for col in 0..3 {
        assert!(matches!(c.cell(0, col), ValueRef::Error(e) if &*e.code == "out-of-range"));
        assert_eq!(c.flags(0, col), CellFlags::ERROR);
    }
}

#[test]
fn values_a_buffer_cannot_hold_exactly_stay_exact() {
    let s = schema(&[
        LogicalType::Time {
            precision: 9,
            tz: false,
        },
        LogicalType::Timestamp {
            precision: 6,
            tz: false,
        },
        LogicalType::Float { bits: 64 },
    ]);
    let plan = [
        BufKind::Temporal {
            unit: TempUnit::Micros,
        },
        BufKind::Temporal {
            unit: TempUnit::Micros,
        },
        BufKind::F64 { f32: false },
    ];
    let nanos = Value::Time(TimeVal {
        nanos_of_day: 1_500,
    });
    // PostgreSQL's last timestamp lies past i64 microseconds since 1970.
    let far = Value::Timestamp(TsVal::from_pg_micros(9_223_371_331_199_999_999));
    let mut b = ChunkBuilder::with_schema(&plan, &s, 1);
    b.push_value(0, nanos.clone());
    b.push_value(1, far.clone());
    b.push_value(2, Value::F32(0.1));
    b.end_row();
    let c = b.seal();
    assert_eq!(c.cell(0, 0), nanos.view());
    assert_eq!(c.cell(0, 1), far.view());
    assert_eq!(c.cell(0, 2), ValueRef::F32(0.1));
}

#[test]
fn f32_columns_keep_the_f32_bits() {
    let mut b = ChunkBuilder::new(&[BufKind::F64 { f32: true }], 4);
    for v in [0.1f32, -0.0, f32::INFINITY, f32::NAN] {
        b.push_f64(0, f64::from(v));
        b.end_row();
    }
    let c = b.seal();
    for (row, v) in [0.1f32, -0.0, f32::INFINITY, f32::NAN].iter().enumerate() {
        let ValueRef::F32(got) = c.cell(row as u32, 0) else {
            panic!("an f32 cell")
        };
        assert_eq!(got.to_bits(), v.to_bits(), "row {row}");
    }
    assert_eq!(format!("{:?}", c.cell(0, 0)), "F32(0.1)");
}

#[test]
fn big_decimals_are_exceptions_and_stay_exact() {
    let big = dec(&format!("-{}.5", "9".repeat(50)));
    let mut b = ChunkBuilder::new(&[BufKind::Dec64 { scale: 1 }], 2);
    b.push_decimal(0, &big);
    b.end_row();
    b.push_decimal(0, &dec("12.345"));
    b.end_row();
    let c = b.seal();
    assert_eq!(c.cell(0, 0), ValueRef::Decimal(&big));
    // Another scale than the column's is still exact.
    assert_eq!(c.cell(1, 0).to_value(), Value::Decimal(dec("12.345")));
}

#[test]
fn a_huge_capacity_hint_is_bounded() {
    let mut b = ChunkBuilder::new(&[BufKind::I64; 64], u32::MAX);
    for col in 0..64 {
        b.push_i64(col, 1);
    }
    b.end_row();
    assert!(!b.is_full());
    assert_eq!(b.seal().rows(), 1);
}

#[test]
fn builder_values_columns_match_from_values() {
    let s = schema(&[LogicalType::Interval, LogicalType::Bool]);
    let rows = vec![
        vec![
            Value::Interval {
                months: 1,
                days: 2,
                micros: 3,
            },
            Value::Null,
        ],
        vec![Value::Missing, error("x")],
    ];
    let mut b = ChunkBuilder::new(&[BufKind::Values, BufKind::Values], 2);
    for row in rows.clone() {
        for (col, v) in row.into_iter().enumerate() {
            b.push_value(col as u32, v);
        }
        b.end_row();
    }
    assert_eq!(b.seal(), Chunk::from_values(&s, rows));
}

proptest! {
    #[test]
    fn typed_chunks_read_back_what_from_values_holds(
        rows in prop::collection::vec(
            (any::<Option<i64>>(), any::<Option<String>>(), any::<Option<bool>>(), any::<Option<[u8; 16]>>()),
            0..64,
        )
    ) {
        let s = schema(&[
            LogicalType::Int { bits: 64, signed: true },
            LogicalType::Text { max_chars: None, fixed: false },
            LogicalType::Bool,
            LogicalType::Uuid,
        ]);
        let plan = [BufKind::I64, TEXT, BufKind::Bool, BufKind::Fixed16];
        let mut b = ChunkBuilder::with_schema(&plan, &s, rows.len() as u32);
        let mut values = Vec::new();
        for (i, t, x, u) in &rows {
            match i { Some(v) => b.push_i64(0, *v), None => b.push_null(0) }
            match t { Some(v) => b.push_bytes(1, v.as_bytes(), u32::MAX), None => b.push_null(1) }
            match x { Some(v) => b.push_bool(2, *v), None => b.push_null(2) }
            match u { Some(v) => b.push_fixed16(3, *v), None => b.push_null(3) }
            b.end_row();
            values.push(vec![
                i.map_or(Value::Null, Value::Int),
                t.as_deref().map_or(Value::Null, |v| Value::Text(v.into())),
                x.map_or(Value::Null, Value::Bool),
                u.map_or(Value::Null, Value::Uuid),
            ]);
        }
        let typed = b.seal();
        let whole = Chunk::from_values(&s, values);
        prop_assert_eq!(typed.rows(), whole.rows());
        for r in 0..typed.rows() {
            for c in 0..4 {
                prop_assert_eq!(typed.cell(r, c), whole.cell(r, c));
                prop_assert_eq!(typed.flags(r, c), CellFlags::empty());
            }
        }
    }

    #[test]
    fn any_bytes_in_a_text_column_are_kept_or_cut_at_a_boundary(
        bytes in prop::collection::vec(any::<u8>(), 0..64),
        cap in 0u32..48,
    ) {
        let c = chunk_text_bytes(&bytes, cap);
        let valid = std::str::from_utf8(&bytes).is_ok();
        match c.cell(0, 0) {
            ValueRef::Text(s) => prop_assert!(valid && s.as_bytes() == bytes && bytes.len() <= cap as usize),
            ValueRef::Lossy(b) => prop_assert!(!valid && b == bytes && bytes.len() <= cap as usize),
            ValueRef::Truncated(t) => {
                prop_assert!(bytes.len() > cap as usize);
                prop_assert_eq!(t.total_len, Some(bytes.len() as u64));
                let prefix: &[u8] = match &t.prefix {
                    Value::Text(s) => { prop_assert!(valid); s.as_bytes() }
                    Value::Lossy(b) => { prop_assert!(!valid); b }
                    other => return Err(TestCaseError::fail(format!("prefix {other:?}"))),
                };
                prop_assert!(bytes.starts_with(prefix));
                prop_assert!(prefix.len() <= cap as usize && cap as usize - prefix.len() <= 3);
            }
            other => return Err(TestCaseError::fail(format!("cell {other:?}"))),
        }
    }
}
