//! `format_cell` (Task 16): the display text of one cell (HV-GRID-10, L3).
//!
//! Numbers are grouped by the format locale through ICU4X's `DecimalFormatter` (compiled CLDR
//! data: never the process locale), built once per `(format_locale, version)` on each thread
//! and kept in a thread-local memo. Decimals reach it as their exact canonical text, never
//! through a float; integers as integers; floats as their shortest round-trip digits. ID-like
//! columns and [`DisplayMode::Raw`] skip it and show the raw text. Everything else is the raw
//! text of [`super::raw_text`]: dates stay ISO in every mode, `timestamptz` is shown in the
//! display zone unless the server sent its own offset.

use std::cell::{Cell, RefCell};
use std::fmt::Write as _;

use fixed_decimal::Decimal;
use icu_decimal::DecimalFormatter;
use icu_decimal::options::DecimalFormatterOptions;
use icu_locale::Locale;
use unicode_segmentation::UnicodeSegmentation;

use super::raw::{
    Buf, ascii, bool_text, float_canonical, float_special, is_numeric, push_escaped, push_hex,
    push_uuid, single_column, special_text, write_scalar,
};
use crate::columnar::CellFlags;
use crate::decimal::DecimalRef;
use crate::temporal::TsVal;
use crate::valfmt::{
    BinaryDisplay, ChipKind, ColumnFacts, Display, DisplayKind, DisplayMode, DisplaySettings,
    Purpose,
};
use crate::value::ValueRef;

/// The display text of `v` in column `col` under `s`, for `p` (HV-GRID-10, L3).
///
/// - Numbers (`Number`, aligned to the end): exact, grouped by the format locale unless the
///   column is ID-like or the mode is Raw; floats in their shortest round-trip form, with
///   `NaN` and the infinities as the engine spells them.
/// - `true`/`false` (`Bool`), never translated; UUIDs canonical lower-case (`Text`).
/// - Dates, times, timestamps and intervals (`Temporal`): the ISO raw text at the server's
///   precision; `timestamptz` in the display zone with its offset (`+03`), or in the offset
///   the server sent. A timestamp beyond the calendar is an `Error` cell.
/// - Specials verbatim in their column's spelling (`infinity` for a date, `Infinity` for a
///   number).
/// - Bytes (`Binary`) by the column's [`BinaryDisplay`]; `Detect`: 16 bytes are a UUID, valid
///   UTF-8 that is at least 90 % printable is text, anything else `\x` hex.
/// - JSON and XML (`Json`) and server text (`Opaque`, `Text`) verbatim. Text that is not
///   valid UTF-8 (`Lossy`, `Text`): its valid runs, and `\xNN` for every other byte (never
///   U+FFFD).
/// - `Truncated`: the prefix formatted, plus [`ChipKind::Truncated`] when its total is known
///   (the app renders the chip through Fluent). `Error`: its message. `Null`: kind `Null`, text
///   `NULL` (the grid paints its marker). `Text("")`: kind `Empty`.
///
/// Text of every kind is capped at `s.max_display_chars` graphemes; control characters are
/// kept (the grid makes them visible; copy uses [`super::raw_text`]). M3a formats every
/// [`Purpose`] alike.
pub fn format_cell(
    v: ValueRef<'_>,
    col: &ColumnFacts<'_>,
    s: &DisplaySettings,
    p: Purpose,
) -> Display {
    // M3a formats every purpose alike: the raw tooltip under a fraction cap and the grid's
    // screen-reader subtree arrive with their features (M3b).
    let _ = p;
    cell(v, col, s)
}

/// [`format_cell`]'s body.
fn cell(v: ValueRef<'_>, col: &ColumnFacts<'_>, s: &DisplaySettings) -> Display {
    let cap = s.max_display_chars as usize;
    let mut d = Display {
        text: String::new(),
        kind: DisplayKind::Text,
        flags: flags_of(v).bits(),
        align_end: false,
        chip: None,
    };
    match v {
        ValueRef::Null => {
            d.text.push_str("NULL");
            d.kind = DisplayKind::Null;
            d.align_end = is_numeric(col.logical);
        }
        ValueRef::Missing => {}
        ValueRef::Default => d.text.push_str("DEFAULT"),
        ValueRef::Bool(b) => {
            d.text.push_str(bool_text(b));
            d.kind = DisplayKind::Bool;
        }
        ValueRef::Int(_)
        | ValueRef::UInt(_)
        | ValueRef::F32(_)
        | ValueRef::F64(_)
        | ValueRef::Decimal(_) => {
            // One allocation: grouped numbers rarely pass 32 bytes.
            d.text.reserve(32);
            push_number(&mut d.text, v, col, s);
            d.kind = DisplayKind::Number;
            d.align_end = true;
        }
        ValueRef::Text(t) => {
            if t.is_empty() {
                d.kind = DisplayKind::Empty;
            } else {
                push_capped(&mut d.text, t, cap);
            }
        }
        ValueRef::Json(t) | ValueRef::Xml(t) => {
            push_capped(&mut d.text, t, cap);
            d.kind = DisplayKind::Json;
        }
        ValueRef::Opaque(o) => push_capped(&mut d.text, &o.text, cap),
        ValueRef::Error(e) => {
            push_capped(&mut d.text, &e.message, cap);
            d.kind = DisplayKind::Error;
        }
        ValueRef::Lossy(b) => push_escaped(&mut d.text, window(b, cap)),
        ValueRef::Uuid(u) => push_uuid(&mut d.text, &u),
        ValueRef::Bytes(b) => {
            push_bytes(&mut d.text, b, col.binary, cap);
            d.kind = DisplayKind::Binary;
        }
        ValueRef::Special(sp) => {
            d.text.push_str(&special_text(sp, col));
            if is_numeric(col.logical) {
                d.kind = DisplayKind::Number;
                d.align_end = true;
            } else {
                d.kind = DisplayKind::Temporal;
            }
        }
        ValueRef::Date(_)
        | ValueRef::Time(_)
        | ValueRef::TimeTz(..)
        | ValueRef::Timestamp(_)
        | ValueRef::TimestampTz { .. }
        | ValueRef::Interval { .. } => {
            let zone = |utc: TsVal| zone_offset(s, utc);
            d.text.reserve(40);
            let ok = write_scalar(v, col, Some(&zone), &mut d.text);
            d.kind = if ok {
                DisplayKind::Temporal
            } else {
                DisplayKind::Error
            };
        }
        ValueRef::Truncated(t) => {
            let mut inner = cell(t.prefix.view(), col, s);
            if inner.kind == DisplayKind::Empty {
                inner.kind = DisplayKind::Text;
            }
            inner.flags = d.flags;
            inner.chip = t.total_len.map(|total| ChipKind::Truncated {
                total,
                unit: t.unit,
            });
            return inner;
        }
    }
    cap_in_place(&mut d.text, cap);
    d
}

/// The `CellFlags` a value carries (the chunk's `flags_of`, on a borrowed value).
fn flags_of(v: ValueRef<'_>) -> CellFlags {
    match v {
        ValueRef::Truncated(t) => CellFlags::TRUNCATED | flags_of(t.prefix.view()),
        ValueRef::Lossy(_) => CellFlags::LOSSY,
        ValueRef::Error(_) => CellFlags::ERROR,
        ValueRef::Special(_) => CellFlags::SPECIAL,
        _ => CellFlags::empty(),
    }
}

/// The display zone's offset at `utc`; UTC beyond the zone database's years (±9999).
fn zone_offset(s: &DisplaySettings, utc: TsVal) -> i32 {
    jiff::Timestamp::new(utc.secs, utc.nanos as i32).map_or(0, |t| s.tz.to_offset(t).seconds())
}

// ---- numbers -----------------------------------------------------------------------------

/// A number, grouped by the format locale unless the column is ID-like or the mode is Raw.
fn push_number(out: &mut String, v: ValueRef<'_>, col: &ColumnFacts<'_>, s: &DisplaySettings) {
    let localized = s.mode == DisplayMode::Default && !col.id_like;
    let pushed = localized
        && match v {
            ValueRef::Int(i) => with_formatter(s, |f| push_formatted(out, f, &Decimal::from(i))),
            ValueRef::UInt(u) => with_formatter(s, |f| push_formatted(out, f, &Decimal::from(u))),
            ValueRef::F32(x) => push_float(out, f64::from(x), true, col, s),
            ValueRef::F64(x) => push_float(out, x, single_column(x, col), col, s),
            ValueRef::Decimal(d) => push_decimal(out, d, s),
            _ => false,
        };
    if !pushed {
        write_scalar(v, col, None, out);
    }
}

fn push_formatted(out: &mut String, f: &DecimalFormatter, d: &Decimal) {
    // Writing into a `String` cannot fail.
    let _ = write!(out, "{}", f.format(d));
}

/// A decimal through its exact canonical text (stack-built for up to 64 bytes); one wider than
/// the formatter's range (about 32,000 digits) is written as that text, ungrouped.
fn push_decimal(out: &mut String, d: DecimalRef<'_>, s: &DisplaySettings) -> bool {
    let mut stack = Buf::<64>::new();
    let heap;
    let canonical: &[u8] = if write!(stack, "{d}").is_ok() {
        stack.as_bytes()
    } else {
        let mut text = String::new();
        d.write_canonical(&mut text);
        heap = text;
        heap.as_bytes()
    };
    match Decimal::try_from_utf8(canonical) {
        Ok(fixed) => with_formatter(s, |f| push_formatted(out, f, &fixed)),
        Err(_) => {
            out.push_str(ascii(canonical));
            true
        }
    }
}

/// A finite float as its shortest round-trip digits in the locale's symbols; an exponent, when
/// the canonical form has one, stays `e±XX` after the localized mantissa (`1,5e-07`).
fn push_float(
    out: &mut String,
    x: f64,
    single: bool,
    col: &ColumnFacts<'_>,
    s: &DisplaySettings,
) -> bool {
    if !x.is_finite() {
        out.push_str(float_special(x, col));
        return true;
    }
    let canonical = float_canonical(x, single);
    let text = canonical.as_bytes();
    let e = text.iter().position(|&c| c == b'e').unwrap_or(text.len());
    let Ok(mantissa) = Decimal::try_from_utf8(&text[..e]) else {
        return false;
    };
    with_formatter(s, |f| {
        push_formatted(out, f, &mantissa);
        out.push_str(ascii(&text[e..]));
    })
}

/// The formatter for `s`'s locale and version, built on a miss.
struct Memo {
    locale: Locale,
    version: u32,
    formatter: Option<DecimalFormatter>,
}

thread_local! {
    static MEMO: RefCell<Option<Memo>> = const { RefCell::new(None) };
    static BUILDS: Cell<u64> = const { Cell::new(0) };
}

fn build(locale: &Locale) -> Option<DecimalFormatter> {
    let _ = BUILDS.try_with(|b| b.set(b.get() + 1));
    DecimalFormatter::try_new(locale.into(), DecimalFormatterOptions::default()).ok()
}

/// Runs `f` with the memoised formatter for `(s.format_locale, s.version)`; `false` (and `f`
/// not run) when the locale has no formatter, so the caller writes the raw text.
fn with_formatter(s: &DisplaySettings, f: impl FnOnce(&DecimalFormatter)) -> bool {
    let mut f = Some(f);
    let hit = MEMO.try_with(|memo| {
        let mut memo = memo.borrow_mut();
        let fresh = memo
            .as_ref()
            .is_some_and(|m| m.version == s.version && m.locale == s.format_locale);
        if !fresh {
            *memo = Some(Memo {
                locale: s.format_locale.clone(),
                version: s.version,
                formatter: build(&s.format_locale),
            });
        }
        match memo.as_ref().and_then(|m| m.formatter.as_ref()) {
            Some(formatter) => {
                if let Some(f) = f.take() {
                    f(formatter);
                }
                true
            }
            None => false,
        }
    });
    match hit {
        Ok(done) => done,
        // The thread is tearing down its locals: build one for this call.
        Err(_) => match (build(&s.format_locale), f.take()) {
            (Some(formatter), Some(f)) => {
                f(&formatter);
                true
            }
            _ => false,
        },
    }
}

/// How many `DecimalFormatter`s this thread has built: one per `(format_locale, version)` key
/// it met in a row (the memo's test seam).
#[doc(hidden)]
pub fn formatter_builds() -> u64 {
    BUILDS.try_with(Cell::get).unwrap_or(0)
}

// ---- bytes and capping -------------------------------------------------------------------

/// The bytes a capped display can show: a cell never shows more than `cap` graphemes, and no
/// grapheme takes fewer than one byte nor any escape more than four characters.
fn window(b: &[u8], cap: usize) -> &[u8] {
    &b[..b.len().min(cap.saturating_mul(4).saturating_add(4))]
}

/// Bytes by display type; detection reads only the [`window`] a cell can show.
fn push_bytes(out: &mut String, b: &[u8], binary: BinaryDisplay, cap: usize) {
    let hex_bytes = cap.saturating_sub(2) / 2;
    let uuid = || <&[u8; 16]>::try_from(b).ok();
    match binary {
        BinaryDisplay::Hex => push_hex(out, b, hex_bytes),
        BinaryDisplay::Uuid => match uuid() {
            Some(u) => push_uuid(out, u),
            None => push_hex(out, b, hex_bytes),
        },
        BinaryDisplay::Text => match utf8_window(b, cap) {
            Some(text) => push_capped(out, text, cap),
            None => push_escaped(out, window(b, cap)),
        },
        BinaryDisplay::Detect => {
            if let Some(u) = uuid() {
                return push_uuid(out, u);
            }
            match utf8_window(b, cap).filter(|t| !t.is_empty() && mostly_printable(t)) {
                Some(text) => push_capped(out, text, cap),
                None => push_hex(out, b, hex_bytes),
            }
        }
    }
}

/// The [`window`] of `b` as text when it is valid UTF-8; a character the window cuts at its
/// end counts as valid when the bytes go on past the window.
fn utf8_window(b: &[u8], cap: usize) -> Option<&str> {
    let w = window(b, cap);
    match std::str::from_utf8(w) {
        Ok(text) => Some(text),
        Err(e) if e.error_len().is_none() && w.len() < b.len() => {
            std::str::from_utf8(&w[..e.valid_up_to()]).ok()
        }
        Err(_) => None,
    }
}

/// At least 90 % of the characters are printable (tab, line feed and carriage return count).
fn mostly_printable(text: &str) -> bool {
    let (mut total, mut printable) = (0usize, 0usize);
    for c in text.chars() {
        total += 1;
        if !c.is_control() || matches!(c, '\t' | '\n' | '\r') {
            printable += 1;
        }
    }
    printable * 10 >= total * 9
}

/// Appends the first `cap` graphemes of `text` (without copying the rest).
fn push_capped(out: &mut String, text: &str, cap: usize) {
    out.push_str(&text[..grapheme_cut(text, cap)]);
}

/// Cuts `text` after its first `cap` graphemes.
fn cap_in_place(text: &mut String, cap: usize) {
    let cut = grapheme_cut(text, cap);
    text.truncate(cut);
}

/// The byte offset after the first `cap` graphemes; a text of at most `cap` bytes has at most
/// `cap` graphemes, so the common case reads nothing.
fn grapheme_cut(text: &str, cap: usize) -> usize {
    if text.len() <= cap {
        return text.len();
    }
    text.grapheme_indices(true)
        .nth(cap)
        .map_or(text.len(), |(i, _)| i)
}
