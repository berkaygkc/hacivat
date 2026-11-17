//! M3c Task 38 (P-M3b-17, plan "Carried items": `valfmt/display.rs:68`): `Purpose::Tooltip`,
//! the raw value under a fraction cap. The tooltip shows what copy would put on the clipboard
//! (the raw channel: never grouped, ASCII digits; a `timestamptz` in the display zone), cut at a
//! quarter of the cell's grapheme cap. Every assertion holds under any `LANG`.

use hacivat_proto::DbmsKind;
use hacivat_values::valfmt::{
    BinaryDisplay, ChipKind, ColumnFacts, DisplayKind, DisplaySettings, DisplayZone, Purpose,
    TOOLTIP_FRACTION, format_cell,
};
use hacivat_values::{CellFlags, DecimalVal, LenUnit, LogicalType, TruncatedVal, Value};

fn facts(logical: &LogicalType) -> ColumnFacts<'_> {
    ColumnFacts {
        logical,
        native_name: "",
        id_like: false,
        binary: BinaryDisplay::Detect,
        engine: DbmsKind::Postgres,
    }
}

fn text_type() -> LogicalType {
    LogicalType::Text {
        max_chars: None,
        fixed: false,
    }
}

fn tr() -> DisplaySettings {
    DisplaySettings::with_zone("tr-TR", DisplayZone::utc())
}

#[test]
fn the_tooltip_is_the_raw_text_never_grouped() {
    let int8 = LogicalType::Int {
        bits: 64,
        signed: true,
    };
    let v = Value::Int(1_234_567);
    let cell = format_cell(v.view(), &facts(&int8), &tr(), Purpose::Cell);
    assert_eq!(
        cell.text, "1.234.567",
        "the cell groups by the format locale"
    );
    let tip = format_cell(v.view(), &facts(&int8), &tr(), Purpose::Tooltip);
    assert_eq!(tip.text, "1234567");
    assert_eq!(tip.kind, DisplayKind::Number);
    assert!(tip.align_end);
    let dec = LogicalType::Decimal {
        precision: Some(14),
        scale: Some(2),
    };
    let d = Value::Decimal(DecimalVal::parse("-1234.50").unwrap());
    let tip = format_cell(d.view(), &facts(&dec), &tr(), Purpose::Tooltip);
    assert_eq!(tip.text, "-1234.50", "exact, ASCII, never through a float");
}

#[test]
fn null_and_errors_keep_their_kind() {
    let t = text_type();
    let tip = format_cell(Value::Null.view(), &facts(&t), &tr(), Purpose::Tooltip);
    assert_eq!((tip.text.as_str(), tip.kind), ("NULL", DisplayKind::Null));
    let empty = Value::Text("".into());
    let tip = format_cell(empty.view(), &facts(&t), &tr(), Purpose::Tooltip);
    assert_eq!(tip.kind, DisplayKind::Empty);
}

#[test]
fn a_long_value_is_cut_at_a_quarter_of_the_cell_cap() {
    let t = text_type();
    let s = tr();
    let cap = (s.max_display_chars / TOOLTIP_FRACTION) as usize;
    assert_eq!(cap, 256);
    let long: String = "ğ".repeat(1_000);
    let v = Value::Text(long.as_str().into());
    let tip = format_cell(v.view(), &facts(&t), &s, Purpose::Tooltip);
    assert_eq!(tip.text.chars().count(), cap);
    assert_ne!(tip.flags & CellFlags::TRUNCATED.bits(), 0);
    assert_eq!(
        tip.chip,
        Some(ChipKind::Truncated {
            total: 1_000,
            unit: LenUnit::Chars
        })
    );
    // A short value is whole, with no chip.
    let short = Value::Text("kısa".into());
    let tip = format_cell(short.view(), &facts(&t), &s, Purpose::Tooltip);
    assert_eq!(tip.text, "kısa");
    assert_eq!(tip.chip, None);
    assert_eq!(tip.flags & CellFlags::TRUNCATED.bits(), 0);
}

#[test]
fn a_value_the_store_cut_names_its_whole_size() {
    let t = text_type();
    let v = Value::Truncated(Box::new(TruncatedVal {
        prefix: Value::Text("prefix".into()),
        total_len: Some(5 << 20),
        unit: LenUnit::Bytes,
    }));
    let tip = format_cell(v.view(), &facts(&t), &tr(), Purpose::Tooltip);
    assert_eq!(tip.text, "prefix");
    assert_ne!(tip.flags & CellFlags::TRUNCATED.bits(), 0);
    assert_eq!(
        tip.chip,
        Some(ChipKind::Truncated {
            total: 5 << 20,
            unit: LenUnit::Bytes
        })
    );
}

#[test]
fn bytes_are_hex_and_a_timestamptz_reads_in_the_display_zone() {
    let bytea = LogicalType::Bytes { max: None };
    let v = Value::Bytes(std::sync::Arc::from(&[0x00u8, 0xff, b'a'][..]));
    let tip = format_cell(v.view(), &facts(&bytea), &tr(), Purpose::Tooltip);
    assert_eq!(tip.text, "\\x00ff61");
    assert_eq!(tip.kind, DisplayKind::Binary);
    // Europe/Istanbul is +03 all year: the tooltip shows the instant as the cell does.
    let zone = DisplayZone::load(Some("Europe/Istanbul"));
    let s = DisplaySettings::with_zone("en-US", zone);
    let tz = LogicalType::Timestamp {
        precision: 0,
        tz: true,
    };
    let v = Value::TimestampTz {
        utc: hacivat_values::TsVal {
            secs: 1_763_028_000,
            nanos: 0,
        },
        offset: None,
    };
    let cell = format_cell(v.view(), &facts(&tz), &s, Purpose::Cell);
    let tip = format_cell(v.view(), &facts(&tz), &s, Purpose::Tooltip);
    assert_eq!(tip.text, cell.text);
    assert!(tip.text.ends_with("+03"), "{}", tip.text);
}
