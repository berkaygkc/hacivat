//! M3a Task 1b: the value model's shape and the working `Chunk::from_values` (P8, Q8).

use std::sync::Arc;

use hacivat_values::valfmt::{DisplayMode, DisplaySettings, DisplayZone};
use hacivat_values::{
    CellFlags, Chunk, ColumnMeta, ErrorVal, LogicalType, NativeType, ResultSchema, Value, ValueRef,
};

fn column(name: &str, logical: LogicalType) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        logical,
        native: NativeType::Unknown,
        origin: None,
    }
}

/// `id int8, name text`.
fn schema2() -> ResultSchema {
    ResultSchema {
        columns: Arc::from(vec![
            column(
                "id",
                LogicalType::Int {
                    bits: 64,
                    signed: true,
                },
            ),
            column(
                "name",
                LogicalType::Text {
                    max_chars: None,
                    fixed: false,
                },
            ),
        ]),
    }
}

#[test]
fn value_is_at_most_32_bytes() {
    assert!(std::mem::size_of::<Value>() <= 32);
}

#[test]
fn chunk_from_values_round_trips() {
    let c = Chunk::from_values(&schema2(), vec![vec![Value::Int(1), Value::Null]]);
    assert_eq!(c.rows(), 1);
    assert!(matches!(c.cell(0, 1), ValueRef::Null));
    assert!(matches!(c.cell(0, 0), ValueRef::Int(1)));
}

#[test]
fn chunk_cells_borrow_text_and_carry_flags() {
    let error = Value::Error(Arc::new(ErrorVal {
        code: "decode".into(),
        message: "bad length".into(),
    }));
    let c = Chunk::from_values(
        &schema2(),
        vec![
            vec![Value::Int(7), Value::Text("Müşteri".into())],
            vec![Value::Int(8), error.clone()],
        ],
    );
    assert!(matches!(c.cell(0, 1), ValueRef::Text("Müşteri")));
    assert_eq!(c.cell(1, 1).to_value(), error);
    assert_eq!(c.flags(1, 1), CellFlags::ERROR);
    assert_eq!(c.flags(0, 1), CellFlags::empty());
    // Two rows of two `Value`s plus the text's 9 bytes and the error's 6 + 10, and (m3a-t06
    // review ruling M8, a named exception to this Task 1b test) the 32-byte `ErrorVal` body
    // behind the error's `Arc`.
    let cells = 4 * std::mem::size_of::<Value>() as u64;
    let error_body = std::mem::size_of::<ErrorVal>() as u64;
    assert_eq!(c.bytes(), cells + 9 + error_body + 16);
}

#[test]
#[should_panic(expected = "row 0 has 1 values; the schema has 2 columns")]
fn a_short_row_is_a_fixture_bug() {
    Chunk::from_values(&schema2(), vec![vec![Value::Int(1)]]);
}

#[test]
fn display_settings_fall_back_to_und_and_utc() {
    let s = DisplaySettings::new("not a tag!", Some("Mars/Olympus_Mons"));
    assert_eq!(s.format_locale, icu_locale::Locale::UNKNOWN);
    assert_eq!(s.tz, jiff::tz::TimeZone::UTC);
    assert_eq!(s.mode, DisplayMode::Default);
    assert_eq!(s.max_display_chars, 1_024);
    let tr = DisplaySettings::new("tr-TR", Some("Europe/Istanbul"));
    assert_eq!(tr.format_locale.to_string(), "tr-TR");
    assert_eq!(tr.tz.iana_name(), Some("Europe/Istanbul"));
}

// ## m3a-t01b-seams (fix round 1)
#[test]
fn display_settings_build_without_io_from_a_loaded_zone() {
    // The zone is loaded where the caller chooses (off the UI thread); the settings take it.
    let zone = DisplayZone::load(Some("Europe/Istanbul"));
    let s = DisplaySettings::with_zone("tr-TR", zone.clone());
    assert_eq!(s, DisplaySettings::new("tr-TR", Some("Europe/Istanbul")));
    assert_eq!(s.tz.iana_name(), Some("Europe/Istanbul"));
    assert_eq!(DisplayZone::load(None), DisplayZone::utc());
    assert_eq!(
        DisplaySettings::with_zone("en", DisplayZone::utc()).tz,
        jiff::tz::TimeZone::UTC
    );
}
