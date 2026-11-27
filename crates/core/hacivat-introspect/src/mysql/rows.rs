// ## m4-t17-mysql
//! The rows of the MySQL pack sections, typed, and the pure text rules applied to them: the
//! UTC time markers, `EXTRA`, ENUM/SET labels and pre-quoted column defaults.
//!
//! Every section returns its columns in declared order, so a row is read by position; a row
//! whose width is not the declared one is [`IntroError::Protocol`], never guessed at. Every
//! value arrives as text (Task 5): numbers are parsed, `YES`/`NO` compared exactly, empty
//! strings in comment columns read as "no comment".

use std::sync::Arc;

use hacivat_meta::Name;
use hacivat_meta::props::GeneratedKind;
use hacivat_session::meta::Rows;
use sql_ident::quote::{QuoteCtx, string_literal};

use crate::introspector::IntroError;

/// One row of a section.
pub type Row = Vec<Option<Arc<str>>>;

/// The rows of `section`, each checked to be `width` wide.
pub fn checked<'r>(
    section: &'static str,
    rows: &'r Rows,
    width: usize,
) -> Result<&'r [Row], IntroError> {
    if rows.rows.iter().any(|r| r.len() != width) {
        return Err(IntroError::Protocol(Name::from(section)));
    }
    Ok(&rows.rows)
}

/// The text of cell `i` (`None` for SQL NULL).
pub fn cell(r: &Row, i: usize) -> Option<&str> {
    r.get(i).and_then(|c| c.as_deref())
}

/// Cell `i` as a name (`""` for SQL NULL; callers skip such rows where a name is required).
pub fn name(r: &Row, i: usize) -> Name {
    r.get(i)
        .and_then(Clone::clone)
        .unwrap_or_else(|| Name::from(""))
}

/// Cell `i` as a name, `None` for NULL or an empty string (comments, options).
pub fn opt_name(r: &Row, i: usize) -> Option<Name> {
    r.get(i).and_then(Clone::clone).filter(|s| !s.is_empty())
}

/// Cell `i` as a number (`None` for NULL or text that is not one).
pub fn number(r: &Row, i: usize) -> Option<u64> {
    cell(r, i).and_then(|s| s.trim().parse::<u64>().ok())
}

/// Cell `i` as a `u32` (saturated).
pub fn number32(r: &Row, i: usize) -> Option<u32> {
    number(r, i).map(|n| u32::try_from(n).unwrap_or(u32::MAX))
}

/// Cell `i` reads `YES` exactly.
pub fn yes(r: &Row, i: usize) -> bool {
    cell(r, i) == Some("YES")
}

/// Days from 1970-01-01 to the civil date `y-m-d` (proleptic Gregorian; H. Hinnant's
/// `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// A server time printed as `YYYY-MM-DD HH:MM:SS[.f…]` (`CAST(<datetime> AS CHAR)`, read in
/// the session's UTC) as microseconds since the Unix epoch. Anything else, including MySQL's
/// zero date, is `None`.
pub fn parse_time_micros(text: &str) -> Option<i64> {
    let t = text.trim();
    let (date, time) = t.split_once(' ')?;
    let mut d = date.split('-');
    let (y, mo, da) = (d.next()?, d.next()?, d.next()?);
    if d.next().is_some() || y.len() != 4 || mo.len() != 2 || da.len() != 2 {
        return None;
    }
    let (hms, frac) = match time.split_once('.') {
        Some((a, b)) => (a, b),
        None => (time, ""),
    };
    let mut h = hms.split(':');
    let (hh, mi, ss) = (h.next()?, h.next()?, h.next()?);
    if h.next().is_some() || hh.len() != 2 || mi.len() != 2 || ss.len() != 2 {
        return None;
    }
    let num = |s: &str| -> Option<i64> {
        if s.bytes().all(|b| b.is_ascii_digit()) {
            s.parse::<i64>().ok()
        } else {
            None
        }
    };
    let (y, mo, da, hh, mi, ss) = (num(y)?, num(mo)?, num(da)?, num(hh)?, num(mi)?, num(ss)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&da) || hh > 23 || mi > 59 || ss > 60 {
        return None;
    }
    if frac.len() > 6 || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut micros = 0i64;
    for (k, b) in frac.bytes().enumerate() {
        micros += i64::from(b - b'0') * 10i64.pow(5 - u32::try_from(k).unwrap_or(5));
    }
    let days = days_from_civil(y, mo, da);
    Some(((days * 24 + hh) * 60 + mi) * 60_000_000 + ss * 1_000_000 + micros)
}

/// What `COLUMNS.EXTRA` says (ASCII-case-insensitive token tests, INV-3): MySQL writes
/// `VIRTUAL GENERATED`, `STORED GENERATED`, `DEFAULT_GENERATED on update CURRENT_TIMESTAMP`,
/// `INVISIBLE`, `auto_increment`; MariaDB `PERSISTENT GENERATED`, `on update
/// current_timestamp()`, `INVISIBLE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Extra {
    pub generated: Option<GeneratedKind>,
    pub invisible: bool,
    pub auto_increment: bool,
    /// MySQL: the default is an expression (`DEFAULT_GENERATED`).
    pub default_generated: bool,
}

pub fn parse_extra(extra: &str) -> Extra {
    let words: Vec<String> = extra
        .split_ascii_whitespace()
        .map(str::to_ascii_lowercase)
        .collect();
    let has = |w: &str| words.iter().any(|x| x == w);
    let generated = if has("generated") && (has("stored") || has("persistent")) {
        Some(GeneratedKind::Stored)
    } else if has("generated") && has("virtual") {
        Some(GeneratedKind::Virtual)
    } else {
        None
    };
    Extra {
        generated,
        invisible: has("invisible"),
        auto_increment: has("auto_increment"),
        default_generated: has("default_generated"),
    }
}

/// The labels of an `enum('a','b')` or `set(…)` `COLUMN_TYPE`, in declaration order; `None`
/// for any other type or text that does not parse. A quote inside a label is doubled (`''`);
/// with `backslash_escapes` (the session's `sql_mode` lacks `NO_BACKSLASH_ESCAPES`) a
/// backslash escapes the next character as in a MySQL string literal (`\\`, `\'`, `\n`, …;
/// `\%` and `\_` keep their backslash), and without it a backslash is a plain character.
pub fn enum_labels(column_type: &str, backslash_escapes: bool) -> Option<Vec<Name>> {
    let t = column_type.trim();
    let lower = t.get(..5).map(str::to_ascii_lowercase);
    let body = match lower.as_deref() {
        Some("enum(") => &t[5..],
        Some(_) if t.len() >= 4 && t[..4].eq_ignore_ascii_case("set(") => &t[4..],
        _ => return None,
    };
    let mut out = Vec::new();
    let mut chars = body.chars().peekable();
    loop {
        if chars.next()? != '\'' {
            return None;
        }
        let mut label = String::new();
        loop {
            match chars.next()? {
                '\'' if chars.peek() == Some(&'\'') => {
                    chars.next();
                    label.push('\'');
                }
                '\'' => break,
                '\\' if backslash_escapes => {
                    let c = chars.next()?;
                    match c {
                        '0' => label.push('\0'),
                        'b' => label.push('\u{8}'),
                        'n' => label.push('\n'),
                        'r' => label.push('\r'),
                        't' => label.push('\t'),
                        'Z' => label.push('\u{1a}'),
                        '%' | '_' => {
                            label.push('\\');
                            label.push(c);
                        }
                        other => label.push(other),
                    }
                }
                c => label.push(c),
            }
        }
        out.push(Name::from(label));
        match chars.next()? {
            ',' => continue,
            ')' => break,
            _ => return None,
        }
    }
    // Only an optional trailing space may follow (`enum('a')` is the whole type).
    if chars.any(|c| !c.is_whitespace()) {
        return None;
    }
    Some(out)
}

/// The column default as SQL text a DDL producer writes after `DEFAULT` (the T7/T19 carry:
/// defaults arrive pre-quoted), keeping each flavor's own spelling (F9):
///
/// - MariaDB (10.2.7+) already reports SQL text: a literal quoted (`'eu'`), an expression as
///   written (`current_timestamp()`), `NULL` for an explicit `DEFAULT NULL`: kept verbatim.
/// - MySQL reports a literal unquoted (`eu`) and an expression default as its text with
///   `DEFAULT_GENERATED` in `EXTRA`. A literal is quoted here (`'eu'`, as `SHOW CREATE TABLE`
///   prints every literal default, numbers included), a `BIT` literal (`b'101'`) kept; an
///   expression is kept when it is a `CURRENT_TIMESTAMP` form or already parenthesized, and
///   parenthesized otherwise (`DEFAULT (rand())`). NULL (no default) is `None`.
pub fn default_sql(
    mariadb: bool,
    raw: Option<&str>,
    data_type: &str,
    extra: &Extra,
    quote: &QuoteCtx,
) -> Option<Name> {
    let raw = raw?;
    if mariadb {
        return Some(Name::from(raw));
    }
    if extra.default_generated {
        let t = raw.trim();
        let ts = ["current_timestamp", "now(", "localtime", "localtimestamp"]
            .iter()
            .any(|p| t.len() >= p.len() && t[..p.len()].eq_ignore_ascii_case(p));
        return Some(if ts || t.starts_with('(') {
            Name::from(t)
        } else {
            Name::from(format!("({t})"))
        });
    }
    if data_type.eq_ignore_ascii_case("bit")
        && raw.len() >= 2
        && raw[..2].eq_ignore_ascii_case("b'")
    {
        return Some(Name::from(raw));
    }
    string_literal(sql_dialect::DialectId::MySql, raw, quote)
        .ok()
        .map(Name::from)
}

/// The view hash marker: the first 16 hex digits of `SHA2(…, 256)` (`None` for text that is
/// not hex).
pub fn hash_mark(hex: &str) -> Option<u64> {
    let h = hex.get(..16)?;
    u64::from_str_radix(h, 16).ok()
}

// ---------------------------------------------------------------------------------------------
// Typed rows, one struct per section (declared column order).

/// `Tables`, and the first three columns of `TableMarks`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRow {
    pub name: Name,
    pub table_type: Name,
    pub engine: Option<Name>,
    pub collation: Option<Name>,
    pub options: Option<Name>,
    pub comment: Option<Name>,
    /// `CREATE_TIME` text (NULL for views and some engines).
    pub create_time: Option<Name>,
    pub rows: Option<u64>,
}

pub fn tables(rows: &Rows) -> Result<Vec<TableRow>, IntroError> {
    Ok(checked("Tables", rows, 8)?
        .iter()
        .map(|r| TableRow {
            name: name(r, 0),
            table_type: name(r, 1),
            engine: opt_name(r, 2),
            collation: opt_name(r, 3),
            options: opt_name(r, 4),
            comment: opt_name(r, 5),
            create_time: opt_name(r, 6),
            rows: number(r, 7),
        })
        .collect())
}

pub fn table_marks(rows: &Rows) -> Result<Vec<TableRow>, IntroError> {
    Ok(checked("TableMarks", rows, 3)?
        .iter()
        .map(|r| TableRow {
            name: name(r, 0),
            table_type: name(r, 1),
            engine: None,
            collation: None,
            options: None,
            comment: None,
            create_time: opt_name(r, 2),
            rows: None,
        })
        .collect())
}

/// `Views`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewRow {
    pub name: Name,
    pub check_option: Option<Name>,
    pub updatable: Option<bool>,
    pub security_type: Option<Name>,
    pub definer: Option<Name>,
    pub algorithm: Option<Name>,
    /// The definition columns' hash (`SHA2`, first 16 hex digits).
    pub hash: Option<u64>,
    /// `VIEW_DEFINITION` is readable by this login (`def_visible = 'YES'`; review I-1).
    pub def_visible: bool,
}

pub fn views(rows: &Rows) -> Result<Vec<ViewRow>, IntroError> {
    Ok(checked("Views", rows, 8)?
        .iter()
        .map(|r| ViewRow {
            name: name(r, 0),
            check_option: opt_name(r, 1),
            updatable: cell(r, 2).map(|s| s == "YES"),
            security_type: opt_name(r, 3),
            definer: opt_name(r, 4),
            algorithm: opt_name(r, 5),
            hash: cell(r, 6).and_then(hash_mark),
            def_visible: cell(r, 7) == Some("YES"),
        })
        .collect())
}

/// `ViewMarks`: `(view, definition hash, definition visible)`.
pub fn view_marks(rows: &Rows) -> Result<Vec<(Name, Option<u64>, bool)>, IntroError> {
    Ok(checked("ViewMarks", rows, 3)?
        .iter()
        .map(|r| {
            (
                name(r, 0),
                cell(r, 1).and_then(hash_mark),
                cell(r, 2) == Some("YES"),
            )
        })
        .collect())
}

/// One `ViewColumnMarks` row: a view's column as `COLUMNS` reports it (visible without SHOW
/// VIEW; review I-1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewCol {
    pub view: Name,
    pub name: Name,
    pub ordinal: u32,
    pub column_type: Name,
    pub nullable: bool,
    pub default: Option<Name>,
    pub collation: Option<Name>,
}

/// `ViewColumnMarks`.
pub fn view_cols(rows: &Rows) -> Result<Vec<ViewCol>, IntroError> {
    Ok(checked("ViewColumnMarks", rows, 7)?
        .iter()
        .map(|r| ViewCol {
            view: name(r, 0),
            name: name(r, 1),
            ordinal: number32(r, 2).unwrap_or(0),
            column_type: name(r, 3),
            nullable: yes(r, 4),
            default: r.get(5).and_then(Clone::clone),
            collation: opt_name(r, 6),
        })
        .collect())
}

/// `ViewUsage`: `(view, table schema, table)`.
pub fn view_usage(rows: &Rows) -> Result<Vec<(Name, Name, Name)>, IntroError> {
    Ok(checked("ViewUsage", rows, 3)?
        .iter()
        .map(|r| (name(r, 0), name(r, 1), name(r, 2)))
        .collect())
}

/// `Columns`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnRow {
    pub table: Name,
    pub name: Name,
    pub ordinal: u32,
    pub default: Option<Name>,
    pub nullable: bool,
    pub data_type: Name,
    pub column_type: Name,
    pub charset: Option<Name>,
    pub collation: Option<Name>,
    pub extra: Option<Name>,
    pub comment: Option<Name>,
    pub generation_expression: Option<Name>,
}

pub fn columns(rows: &Rows) -> Result<Vec<ColumnRow>, IntroError> {
    Ok(checked("Columns", rows, 12)?
        .iter()
        .map(|r| ColumnRow {
            table: name(r, 0),
            name: name(r, 1),
            ordinal: number32(r, 2).unwrap_or(0),
            // Not `opt_name`: an empty string is a real default (`DEFAULT ''`).
            default: r.get(3).and_then(Clone::clone),
            nullable: yes(r, 4),
            data_type: name(r, 5),
            column_type: name(r, 6),
            charset: opt_name(r, 7),
            collation: opt_name(r, 8),
            extra: opt_name(r, 9),
            comment: opt_name(r, 10),
            generation_expression: opt_name(r, 11),
        })
        .collect())
}

/// `Routines`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineRow {
    pub specific_name: Name,
    pub name: Name,
    pub routine_type: Name,
    pub returns: Option<Name>,
    pub deterministic: bool,
    pub data_access: Option<Name>,
    pub security_type: Option<Name>,
    pub definer: Option<Name>,
    pub comment: Option<Name>,
    pub last_altered: Option<Name>,
}

pub fn routines(rows: &Rows) -> Result<Vec<RoutineRow>, IntroError> {
    Ok(checked("Routines", rows, 10)?
        .iter()
        .map(|r| RoutineRow {
            specific_name: name(r, 0),
            name: name(r, 1),
            routine_type: name(r, 2),
            returns: opt_name(r, 3),
            deterministic: yes(r, 4),
            data_access: opt_name(r, 5),
            security_type: opt_name(r, 6),
            definer: opt_name(r, 7),
            comment: opt_name(r, 8),
            last_altered: opt_name(r, 9),
        })
        .collect())
}

/// `RoutineMarks`: `(specific name, routine type, last altered)`.
pub fn routine_marks(rows: &Rows) -> Result<Vec<(Name, Name, Option<Name>)>, IntroError> {
    Ok(checked("RoutineMarks", rows, 3)?
        .iter()
        .map(|r| (name(r, 0), name(r, 1), opt_name(r, 2)))
        .collect())
}

/// `Parameters` (and `SystemParameters`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamRow {
    pub specific_name: Name,
    pub routine_type: Name,
    pub ordinal: u32,
    pub mode: Option<Name>,
    pub name: Option<Name>,
    pub data_type: Name,
}

pub fn params(rows: &Rows) -> Result<Vec<ParamRow>, IntroError> {
    Ok(checked("Parameters", rows, 6)?
        .iter()
        .map(|r| ParamRow {
            specific_name: name(r, 0),
            routine_type: name(r, 1),
            ordinal: number32(r, 2).unwrap_or(0),
            mode: opt_name(r, 3),
            name: opt_name(r, 4),
            data_type: name(r, 5),
        })
        .collect())
}

/// `Triggers`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerRow {
    pub name: Name,
    pub table: Name,
    pub timing: Option<Name>,
    pub event: Option<Name>,
    pub order: Option<u32>,
    pub created: Option<Name>,
    pub definer: Option<Name>,
}

pub fn triggers(rows: &Rows) -> Result<Vec<TriggerRow>, IntroError> {
    Ok(checked("Triggers", rows, 7)?
        .iter()
        .map(|r| TriggerRow {
            name: name(r, 0),
            table: name(r, 1),
            timing: opt_name(r, 2),
            event: opt_name(r, 3),
            order: number32(r, 4),
            created: opt_name(r, 5),
            definer: opt_name(r, 6),
        })
        .collect())
}

/// `TriggerMarks`: `(trigger, table, created)`.
pub fn trigger_marks(rows: &Rows) -> Result<Vec<TriggerRow>, IntroError> {
    Ok(checked("TriggerMarks", rows, 3)?
        .iter()
        .map(|r| TriggerRow {
            name: name(r, 0),
            table: name(r, 1),
            timing: None,
            event: None,
            order: None,
            created: opt_name(r, 2),
            definer: None,
        })
        .collect())
}

/// `Events`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRow {
    pub name: Name,
    pub comment: Option<Name>,
    pub definer: Option<Name>,
    pub last_altered: Option<Name>,
}

pub fn events(rows: &Rows) -> Result<Vec<EventRow>, IntroError> {
    Ok(checked("Events", rows, 10)?
        .iter()
        .map(|r| EventRow {
            name: name(r, 0),
            comment: opt_name(r, 7),
            definer: opt_name(r, 8),
            last_altered: opt_name(r, 9),
        })
        .collect())
}

/// `EventMarks`: `(event, last altered)`.
pub fn event_marks(rows: &Rows) -> Result<Vec<EventRow>, IntroError> {
    Ok(checked("EventMarks", rows, 2)?
        .iter()
        .map(|r| EventRow {
            name: name(r, 0),
            comment: None,
            definer: None,
            last_altered: opt_name(r, 1),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_parse_as_utc_micros() {
        assert_eq!(parse_time_micros("1970-01-01 00:00:00"), Some(0));
        assert_eq!(parse_time_micros("1970-01-01 00:00:01.5"), Some(1_500_000));
        assert_eq!(
            parse_time_micros("2026-11-20 10:11:12.000001"),
            Some(1_795_169_472_000_001)
        );
        assert_eq!(
            parse_time_micros("2000-02-29 00:00:00"),
            Some(951_782_400_000_000)
        );
        for bad in [
            "",
            "0000-00-00 00:00:00",
            "2026-11-20",
            "2026-11-20T10:11:12",
            "2026-11-20 10:11:12.1234567",
            "2026-13-01 00:00:00",
            "２０２６-11-20 10:11:12",
        ] {
            assert_eq!(parse_time_micros(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn extra_parses_generated_invisible_and_expression_defaults() {
        let e = parse_extra("VIRTUAL GENERATED");
        assert_eq!(e.generated, Some(GeneratedKind::Virtual));
        let e = parse_extra("STORED GENERATED INVISIBLE");
        assert_eq!(
            (e.generated, e.invisible),
            (Some(GeneratedKind::Stored), true)
        );
        assert!(parse_extra("auto_increment").auto_increment);
        let e = parse_extra("DEFAULT_GENERATED on update CURRENT_TIMESTAMP");
        assert!(e.default_generated && e.generated.is_none());
        assert_eq!(
            parse_extra("PERSISTENT GENERATED").generated,
            Some(GeneratedKind::Stored)
        );
        assert_eq!(parse_extra(""), Extra::default());
    }
}
