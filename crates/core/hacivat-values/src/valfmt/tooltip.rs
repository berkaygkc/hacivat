// ## m3b-t01c-core-seams (P-M3b-17)
//! `Purpose::Tooltip` (Task 38): the raw value under a fraction cap for a cell's tooltip. Its
//! own file, so Task 38 never shares `display.rs` with Task 40.
//!
//! ## m3c-t38 (M3c Task 38)
//! - **Raw.** The tooltip shows the value's raw channel ([`super::raw_text_in`]): what copy
//!   puts on the clipboard, never grouped, ASCII digits and signs, bytes as `\x` hex, a
//!   `timestamptz` without the server's offset in the display zone (so it reads as its cell
//!   does). A cell shows `1.234.567` in `tr-TR`; its tooltip says `1234567`.
//! - **A fraction cap.** At most `max_display_chars / TOOLTIP_FRACTION` graphemes (256 of the
//!   default 1,024): a tooltip is read at a glance, the value viewer shows the whole value. A cut
//!   text is marked `TRUNCATED` with [`ChipKind::Truncated`] (its whole length in characters);
//!   a value the store cut keeps its own chip (its whole size in its own unit).
//! - Kinds as the cell's: `Null` (`NULL`), `Empty`, `Number` (aligned to the end), `Bool`,
//!   `Temporal`, `Binary`, `Json`, `Error` (its message), `Text`.

use unicode_segmentation::UnicodeSegmentation;

use crate::columnar::CellFlags;
use crate::logical::LogicalType;
use crate::valfmt::{
    ChipKind, ColumnFacts, Display, DisplayKind, DisplaySettings, DisplayZone, RawText, raw_text_in,
};
use crate::value::{LenUnit, ValueRef};

/// The tooltip shows at most `max_display_chars / TOOLTIP_FRACTION` graphemes.
pub const TOOLTIP_FRACTION: u32 = 4;

/// The tooltip text of `v` in column `col` under `s` (module docs).
pub(super) fn text(v: ValueRef<'_>, col: &ColumnFacts<'_>, s: &DisplaySettings) -> Display {
    let cap = (s.max_display_chars / TOOLTIP_FRACTION).max(1) as usize;
    let mut d = Display {
        text: String::new(),
        kind: kind_of(v, col.logical),
        flags: flags_of(v).bits(),
        align_end: false,
        chip: None,
    };
    d.align_end =
        d.kind == DisplayKind::Number || (d.kind == DisplayKind::Null && is_numeric(col.logical));
    // A value the store cut names its whole size (in its own unit).
    if let ValueRef::Truncated(t) = v
        && let Some(total) = t.total_len
    {
        d.chip = Some(ChipKind::Truncated {
            total,
            unit: t.unit,
        });
    }
    let zone = DisplayZone(s.tz.clone());
    match raw_text_in(v, col, &zone) {
        RawText::Null => d.text.push_str("NULL"),
        RawText::Text(text) => {
            let cut = grapheme_cut(&text, cap);
            d.text.push_str(&text[..cut]);
            if cut < text.len() {
                d.flags |= CellFlags::TRUNCATED.bits();
                if d.chip.is_none() {
                    d.chip = Some(ChipKind::Truncated {
                        total: text.chars().count() as u64,
                        unit: LenUnit::Chars,
                    });
                }
            }
        }
    }
    d
}

/// The kind a cell of `v` has (as the cell's display).
fn kind_of(v: ValueRef<'_>, logical: &LogicalType) -> DisplayKind {
    match v {
        ValueRef::Null => DisplayKind::Null,
        ValueRef::Truncated(t) => kind_of(t.prefix.view(), logical),
        ValueRef::Text("") => DisplayKind::Empty,
        ValueRef::Text(_) if matches!(logical, LogicalType::Money) => DisplayKind::Number,
        ValueRef::Int(_)
        | ValueRef::UInt(_)
        | ValueRef::F32(_)
        | ValueRef::F64(_)
        | ValueRef::Decimal(_) => DisplayKind::Number,
        ValueRef::Bool(_) => DisplayKind::Bool,
        ValueRef::Date(_)
        | ValueRef::Time(_)
        | ValueRef::TimeTz(..)
        | ValueRef::Timestamp(_)
        | ValueRef::TimestampTz { .. }
        | ValueRef::Interval { .. } => DisplayKind::Temporal,
        ValueRef::Bytes(_) => DisplayKind::Binary,
        ValueRef::Json(_) | ValueRef::Xml(_) => DisplayKind::Json,
        ValueRef::Error(_) => DisplayKind::Error,
        _ => DisplayKind::Text,
    }
}

/// The value's own flags (as the cell's).
fn flags_of(v: ValueRef<'_>) -> CellFlags {
    match v {
        ValueRef::Truncated(t) => CellFlags::TRUNCATED | flags_of(t.prefix.view()),
        ValueRef::Lossy(_) => CellFlags::LOSSY,
        ValueRef::Error(_) => CellFlags::ERROR,
        ValueRef::Special(_) => CellFlags::SPECIAL,
        _ => CellFlags::empty(),
    }
}

fn is_numeric(logical: &LogicalType) -> bool {
    matches!(
        logical,
        LogicalType::Int { .. }
            | LogicalType::Decimal { .. }
            | LogicalType::Float { .. }
            | LogicalType::Money
    )
}

/// The byte offset after the first `cap` graphemes of `text`.
fn grapheme_cut(text: &str, cap: usize) -> usize {
    if text.len() <= cap {
        return text.len();
    }
    text.grapheme_indices(true)
        .nth(cap)
        .map_or(text.len(), |(i, _)| i)
}
