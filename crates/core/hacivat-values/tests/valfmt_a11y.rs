//! M3c Task 40: `format_cell(.., Purpose::A11y)`, what the grid's screen-reader subtree reads
//! for a cell (HV-GRID-16 step 2): the cell's display text with control characters spoken as
//! one space and at most 1,000 graphemes (then `…`); everything else as the cell shows it.

use hacivat_proto::DbmsKind;
use hacivat_values::valfmt::{
    BinaryDisplay, ColumnFacts, Display, DisplayKind, DisplaySettings, DisplayZone, Purpose,
    format_cell,
};
use hacivat_values::{DecimalVal, LogicalType, Value};

fn text_col() -> LogicalType {
    LogicalType::Text {
        max_chars: None,
        fixed: false,
    }
}

fn facts(logical: &LogicalType) -> ColumnFacts<'_> {
    ColumnFacts {
        logical,
        native_name: "text",
        id_like: false,
        binary: BinaryDisplay::Detect,
        engine: DbmsKind::Postgres,
    }
}

fn both(v: &Value, logical: &LogicalType) -> (Display, Display) {
    let s = DisplaySettings::with_zone("tr-TR", DisplayZone::utc());
    let f = facts(logical);
    (
        format_cell(v.view(), &f, &s, Purpose::Cell),
        format_cell(v.view(), &f, &s, Purpose::A11y),
    )
}

#[test]
fn control_characters_are_spoken_as_one_space() {
    let t = text_col();
    let (cell, spoken) = both(&Value::Text("first\r\n\tsecond\u{7}end".into()), &t);
    // The cell keeps its controls (the grid draws them); the screen reader hears spaces.
    assert_eq!(cell.text, "first\r\n\tsecond\u{7}end");
    assert_eq!(spoken.text, "first second end");
    assert_eq!(spoken.kind, DisplayKind::Text);
    assert_eq!(spoken.flags, cell.flags);
    // Leading and trailing controls say nothing.
    let (_, edges) = both(&Value::Text("\n\nmiddle\n".into()), &t);
    assert_eq!(edges.text, "middle");
}

#[test]
fn a_long_text_is_cut_at_1000_graphemes() {
    let t = text_col();
    let long = "é".repeat(1_500);
    let (cell, spoken) = both(&Value::Text(long.clone().into()), &t);
    assert_eq!(cell.text.chars().count(), 1_024, "the cell's own cap");
    assert_eq!(spoken.text.chars().count(), 1_001);
    assert!(spoken.text.ends_with('…'));
    assert_eq!(
        &spoken.text[..spoken.text.len() - '…'.len_utf8()],
        "é".repeat(1_000)
    );
    // Exactly 1,000 graphemes stay whole.
    let (_, whole) = both(&Value::Text("x".repeat(1_000).into()), &t);
    assert_eq!(whole.text, "x".repeat(1_000));
}

#[test]
fn numbers_null_and_plain_text_read_as_the_cell_shows_them() {
    let dec = LogicalType::Decimal {
        precision: None,
        scale: None,
    };
    let v = Value::Decimal(DecimalVal::parse("1234567.50").expect("decimal"));
    let (cell, spoken) = both(&v, &dec);
    assert_eq!(spoken, cell);
    assert_eq!(spoken.text, "1.234.567,50");
    let (cell, spoken) = both(&Value::Null, &text_col());
    assert_eq!(spoken, cell);
    assert_eq!(spoken.kind, DisplayKind::Null);
    let (cell, spoken) = both(&Value::Text("İstanbul".into()), &text_col());
    assert_eq!(spoken, cell);
}
