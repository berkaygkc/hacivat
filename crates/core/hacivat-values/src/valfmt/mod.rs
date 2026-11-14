//! Cell display formatting, raw text and the ID heuristic (HV-GRID-10, L3, I18N-11; Task 16).
//!
//! The display channel follows the format locale (OS region) and the display zone; the raw
//! channel ([`raw_text`]) is locale-free and feeds copy, sort keys and SQL. No `hacivat-l10n`:
//! numbers go through ICU4X directly. A number joined to a unit is never built here: a
//! truncated value carries a [`ChipKind`] that the app renders through Fluent.
//!
//! Types and [`DisplaySettings::new`] (minimal) are complete since Task 1b; Task 16 fills
//! [`format_cell`] (the display channel), [`raw_text`] (the raw channel) and [`id_like`] (R11).
//!
//! - **Grouping** goes through ICU4X's `DecimalFormatter` with compiled CLDR data, memoised per
//!   `(format_locale, version)` on each thread: a change of either rebuilds it, and nothing reads
//!   the process locale (`tr-TR` is `1.234.567,50` under any `LANG`). Digits are always Latin,
//!   whatever the locale's numbering system (`ar-EG` is `1,234,567.50`). The punctuation is the
//!   locale's CLDR punctuation, not plain ASCII: `sv` negatives use U+2212 and group with a
//!   no-break space, `fr` groups with a narrow no-break space (U+202F), and `es` groups only
//!   from five integer digits (`1234`, `12.345`). Display text is for reading and is never
//!   parsed back; copy, sort keys and SQL take the raw channel, which is ASCII.
//! - **Decimals are exact:** their canonical text goes to the formatter digit for digit; no
//!   decimal ever passes through a float, and `1.50`, `1.5` and `-0.00` stay distinct.
//! - **The two channels share one spelling** for everything but grouped numbers: dates stay ISO
//!   in every mode, specials verbatim. The display channel adds grouping, the display zone, the
//!   [`BinaryDisplay`] choice and the grapheme cap; the raw one is what copy uses.
//!   [`raw_text_in`] is the raw channel with a `timestamptz` in the display zone, so a copied
//!   instant reads as the cell shows it.

mod display;
mod idlike;
mod raw;

pub use display::format_cell;
#[doc(hidden)]
pub use display::formatter_builds;
pub use idlike::id_like;
pub use raw::{RawText, raw_text, raw_text_in};

use hacivat_proto::DbmsKind;

use crate::logical::LogicalType;
use crate::value::LenUnit;

/// Raw: server text as is (no grouping); Default: L3's locale-formatted numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DisplayMode {
    Raw,
    #[default]
    Default,
}

/// How a binary value is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BinaryDisplay {
    /// 16 bytes as a UUID; mostly printable UTF-8 as text; else hex.
    #[default]
    Detect,
    Hex,
    Uuid,
    Text,
}

/// The settings every display call reads; `version` changes whenever any of them does (the
/// app's display caches key on it).
#[derive(Debug, Clone, PartialEq)]
pub struct DisplaySettings {
    pub mode: DisplayMode,
    pub format_locale: icu_locale::Locale,
    pub tz: jiff::tz::TimeZone,
    /// Graphemes shown in a cell (1,024).
    pub max_display_chars: u32,
    pub version: u32,
}

impl DisplaySettings {
    /// Settings for the OS region `region_bcp47` (a BCP-47 tag such as `tr-TR`) and the IANA
    /// zone `tz_iana`, so the app builds them without ICU or jiff (P6). A tag that does not
    /// parse is `und`; a zone that is unknown (or `None`) is UTC. Version 0, mode Default,
    /// 1,024 characters.
    ///
    /// **Does file I/O**: the zone is read from the system's time-zone database
    /// ([`DisplayZone::load`]). Never call it on the UI thread; there, load the zone on the core
    /// runtime (a blocking task) and build with [`DisplaySettings::with_zone`].
    pub fn new(region_bcp47: &str, tz_iana: Option<&str>) -> Self {
        Self::with_zone(region_bcp47, DisplayZone::load(tz_iana))
    }

    /// [`DisplaySettings::new`] with a zone already loaded: no I/O, safe on the UI thread.
    pub fn with_zone(region_bcp47: &str, zone: DisplayZone) -> Self {
        let format_locale = region_bcp47
            .parse::<icu_locale::Locale>()
            .unwrap_or(icu_locale::Locale::UNKNOWN);
        DisplaySettings {
            mode: DisplayMode::Default,
            format_locale,
            tz: zone.0,
            max_display_chars: 1_024,
            version: 0,
        }
    }
}

/// A display time zone, loaded (fix round 1): what [`DisplaySettings::with_zone`] takes, so
/// the I/O of reading the time-zone database happens where the caller chooses. `Send`, so a
/// blocking task can load it and hand it to the UI thread.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayZone(jiff::tz::TimeZone);

impl DisplayZone {
    /// The zone `tz_iana` from the system's time-zone database; unknown (or `None`) is UTC.
    /// **Does file I/O** on its first use of the database: call it off the UI thread.
    pub fn load(tz_iana: Option<&str>) -> Self {
        DisplayZone(
            tz_iana
                .and_then(|z| jiff::tz::TimeZone::get(z).ok())
                .unwrap_or(jiff::tz::TimeZone::UTC),
        )
    }

    /// UTC, without I/O.
    pub fn utc() -> Self {
        DisplayZone(jiff::tz::TimeZone::UTC)
    }
}

/// What the text is for: a cell, its tooltip, or the screen reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Purpose {
    Cell,
    Tooltip,
    A11y,
}

/// What a column contributes to formatting one of its cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnFacts<'a> {
    pub logical: &'a LogicalType,
    /// The engine's type name (`numeric(14,2)`), data.
    pub native_name: &'a str,
    /// ID-like columns are never grouped ([`id_like`]).
    pub id_like: bool,
    pub binary: BinaryDisplay,
    pub engine: DbmsKind,
}

/// The kind of a formatted cell; the grid styles and aligns by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DisplayKind {
    Text,
    Number,
    /// SQL NULL: the grid paints `NULL_MARKER` (D-065).
    Null,
    /// The empty string.
    Empty,
    Bool,
    Temporal,
    Binary,
    Json,
    Error,
}

/// A chip after the cell's text; data, which the app renders through Fluent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChipKind {
    Truncated { total: u64, unit: LenUnit },
}

/// One formatted cell.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Display {
    pub text: String,
    pub kind: DisplayKind,
    /// `CellFlags` bits of the value.
    pub flags: u8,
    pub align_end: bool,
    pub chip: Option<ChipKind>,
}
