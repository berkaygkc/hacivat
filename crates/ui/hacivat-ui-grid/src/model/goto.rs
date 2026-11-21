//! Go to row (HV-GRID-14; M3b Task 37): what the user typed into the go-to field, parsed.
//!
//! Rows are 1-based **view** rows; the format locale's digits and grouping are accepted (`1.000`
//! in `tr` is row 1000); a column is named after a comma or a colon, by its 1-based index, its
//! exact name, its ASCII case-insensitive name, then the tolerant key.
//!
//! ## m3b-t01b-seams: the types and a fail-closed stub (Task 1b): [`parse_goto`] answers
//! `Err(GotoError::Empty)`, so nothing jumps. Task 37 fills it.
//!
//! ## m3c-t37
//! - **The row.** Digits (ASCII or another script's decimal digits: Arabic-Indic, Extended
//!   Arabic-Indic, Devanagari, full-width) with the locale's `group` separator anywhere
//!   between them (a space-like `group` accepts any space: `1 000`, `1 000`); the `decimal`
//!   separator or any other character is not a row ([`GotoError::NotANumber`]); `0` is not a
//!   row either (rows are 1-based).
//! - **The column** follows the first `:`, or the first `,` when `,` is not the locale's group
//!   separator (in `en` `1,000` is a thousand; in `tr` `1.000,city` is row 1000, column
//!   `city`). It is tried as a 1-based position in the result's column order, then as an exact
//!   name, an ASCII case-insensitive name, and last as the tolerant key ([`tolerant_key`]: ASCII
//!   case, the Turkish dotted and dotless `i` and the Latin accents folded), first match in
//!   column order. An empty column part names no column.
//! - **Where it lands** ([`place_row`]): among the rows the view shows, or past the fetched
//!   rows ([`GotoPlace::NotFetched`], the host fetches), or past a known total
//!   ([`GotoError::OutOfRange`] naming the last row). A hidden column is the host's check
//!   (`GridView::go_to`): parsing sees every column.

use crate::source::{ColId, ColumnSpec, ViewRow};

/// Where to go: a view row, 1-based as typed (`"1.000"` in `tr` is `Row(1000)`), maybe a
/// column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GotoTarget {
    Row(u64),
    RowCol(u64, ColId),
}

/// Why the text names no place to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GotoError {
    Empty,
    NotANumber,
    /// Past the last row (`last`: the last row when the total is known).
    OutOfRange {
        last: Option<u64>,
    },
    UnknownColumn,
    HiddenColumn(ColId),
}

// ## m3c-t37
impl GotoTarget {
    /// The 1-based view row.
    pub fn row(&self) -> u64 {
        match self {
            GotoTarget::Row(r) | GotoTarget::RowCol(r, _) => *r,
        }
    }

    /// The column, when one was named.
    pub fn col(&self) -> Option<ColId> {
        match self {
            GotoTarget::Row(_) => None,
            GotoTarget::RowCol(_, c) => Some(*c),
        }
    }
}

/// Where a row lands ([`place_row`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GotoPlace {
    /// A row the view shows (0-based view row).
    Shown(ViewRow),
    /// Past the fetched rows of a result that may have more: fetch, then go.
    NotFetched,
}

/// Where 1-based row `row` lands among the `shown` rows of a view whose total is `total` when
/// known (a filter's rows are all known: its total is `shown`).
pub fn place_row(row: u64, shown: u64, total: Option<u64>) -> Result<GotoPlace, GotoError> {
    if row == 0 {
        return Err(GotoError::NotANumber);
    }
    if row <= shown {
        return Ok(GotoPlace::Shown(ViewRow(row - 1)));
    }
    match total {
        Some(total) if row > total => Err(GotoError::OutOfRange { last: Some(total) }),
        _ => Ok(GotoPlace::NotFetched),
    }
}

/// Parses `text` against `columns` with the format locale's `decimal` and `group` separators.
pub fn parse_goto(
    text: &str,
    columns: &[ColumnSpec],
    decimal: char,
    group: char,
) -> Result<GotoTarget, GotoError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(GotoError::Empty);
    }
    let split = text
        .char_indices()
        .find(|(_, c)| *c == ':' || (*c == ',' && group != ','));
    let (row_part, col_part) = match split {
        Some((at, c)) => (&text[..at], Some(text[at + c.len_utf8()..].trim())),
        None => (text, None),
    };
    let row = parse_row(row_part.trim(), decimal, group)?;
    match col_part.filter(|c| !c.is_empty()) {
        None => Ok(GotoTarget::Row(row)),
        Some(name) => Ok(GotoTarget::RowCol(row, find_column(name, columns)?)),
    }
}

/// The value of `c` as a decimal digit in a script the format locale may use.
fn digit(c: char) -> Option<u64> {
    const ZEROS: [u32; 5] = [0x30, 0x660, 0x6F0, 0x966, 0xFF10];
    let c = c as u32;
    ZEROS
        .iter()
        .find(|z| (**z..**z + 10).contains(&c))
        .map(|z| u64::from(c - z))
}

/// Whether `c` separates digit groups when the locale groups with `group`.
fn is_group(c: char, group: char) -> bool {
    c == group || (group.is_whitespace() && c.is_whitespace())
}

/// A 1-based row: digits, maybe grouped.
fn parse_row(s: &str, decimal: char, group: char) -> Result<u64, GotoError> {
    if s.is_empty() {
        return Err(GotoError::Empty);
    }
    let mut row: u64 = 0;
    let mut digits = 0usize;
    let mut last_group = false;
    for c in s.chars() {
        if let Some(d) = digit(c) {
            row = row
                .checked_mul(10)
                .and_then(|r| r.checked_add(d))
                .ok_or(GotoError::OutOfRange { last: None })?;
            digits += 1;
            last_group = false;
        } else if is_group(c, group) && c != decimal && digits > 0 && !last_group {
            last_group = true;
        } else {
            return Err(GotoError::NotANumber);
        }
    }
    if digits == 0 || last_group || row == 0 {
        return Err(GotoError::NotANumber);
    }
    Ok(row)
}

/// The column `name` names: its 1-based position, exact name, ASCII case-insensitive name, then
/// tolerant key, the first in column order at each step.
fn find_column(name: &str, columns: &[ColumnSpec]) -> Result<ColId, GotoError> {
    if name.chars().all(|c| digit(c).is_some()) {
        let mut ix: u64 = 0;
        for c in name.chars() {
            ix = ix.saturating_mul(10).saturating_add(digit(c).unwrap_or(0));
        }
        return usize::try_from(ix)
            .ok()
            .and_then(|ix| ix.checked_sub(1))
            .and_then(|ix| columns.get(ix))
            .map(|c| c.id)
            .ok_or(GotoError::UnknownColumn);
    }
    let by = |f: &dyn Fn(&str) -> bool| columns.iter().find(|c| f(c.name.as_ref())).map(|c| c.id);
    let key = tolerant_key(name);
    by(&|n| n == name)
        .or_else(|| by(&|n| n.eq_ignore_ascii_case(name)))
        .or_else(|| by(&|n| tolerant_key(n) == key))
        .ok_or(GotoError::UnknownColumn)
}

/// The letters that fold to each ASCII base letter in a tolerant key (the Turkish letters and
/// the Latin-1 / Latin Extended-A accents, both cases).
const FOLDS: &[(char, &str)] = &[
    ('a', "àáâãäåāăąÀÁÂÃÄÅĀĂĄ"),
    ('c', "çćĉċčÇĆĈĊČ"),
    ('d', "ďđĎĐ"),
    ('e', "èéêëēĕėęěÈÉÊËĒĔĖĘĚ"),
    ('g', "ĝğġģĜĞĠĢ"),
    ('h', "ĥħĤĦ"),
    ('i', "ıìíîïĩīĭįİÌÍÎÏĨĪĬĮ"),
    ('j', "ĵĴ"),
    ('k', "ķĶ"),
    ('l', "ĺļľŀłĹĻĽĿŁ"),
    ('n', "ñńņňÑŃŅŇ"),
    ('o', "òóôõöøōŏőÒÓÔÕÖØŌŎŐ"),
    ('r', "ŕŗřŔŖŘ"),
    ('s', "śŝşšŚŜŞŠ"),
    ('t', "ţťŧŢŤŦ"),
    ('u', "ùúûüũūŭůűųÙÚÛÜŨŪŬŮŰŲ"),
    ('w', "ŵŴ"),
    ('y', "ýÿŷÝŸŶ"),
    ('z', "źżžŹŻŽ"),
];

/// A column name's tolerant key: ASCII letters lower-cased, the Turkish dotted and dotless `i`
/// and the Latin accents folded to their base letter, combining accents (U+0300–U+036F)
/// dropped. Only ASCII casing (the workspace rule); other letters stay as written.
pub fn tolerant_key(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if ('\u{300}'..='\u{36F}').contains(&c) {
            continue;
        }
        match FOLDS.iter().find(|(_, set)| set.contains(c)) {
            Some((base, _)) => out.push(*base),
            None => out.push(c.to_ascii_lowercase()),
        }
    }
    out
}
