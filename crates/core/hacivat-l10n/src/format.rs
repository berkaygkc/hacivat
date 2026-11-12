//! Locale-aware `NUMBER`, `DATETIME` and `LIST` backed by ICU4X (I18N-05).
//!
//! The functions only shape values: `NUMBER` returns a Fluent number carrying its options (so
//! plural selection still works, including decimals and `type: "ordinal"`), `DATETIME` and `LIST`
//! return custom values. All text is produced by the bundle formatter `hv_format`, which reads the
//! format locale and time zone from the scoped [`FormatCtx`] (`with_format_ctx`). Plural rules
//! follow the bundle's (UI) language; digits, separators, dates and lists follow the format locale.
//!
//! Formatters are memoized per thread and per (format locale, options), and the memo is dropped
//! when the context's generation changes (a language or region switch).

use std::any::Any;
use std::borrow::{Borrow, Cow};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock, PoisonError};

use fixed_decimal::{Decimal, Sign, SignedRoundingMode, UnsignedRoundingMode};
use fluent_bundle::bundle::FluentBundle;
use fluent_bundle::memoizer::MemoizerKind;
use fluent_bundle::types::{
    AnyEq, FluentNumber, FluentNumberOptions, FluentNumberStyle, FluentNumberType, FluentType,
};
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use icu_datetime::fieldsets::builder::{DateFields, FieldSetBuilder};
use icu_datetime::fieldsets::enums::CompositeDateTimeFieldSet;
use icu_datetime::input::{Date, DateTime, Time};
use icu_datetime::options::{Length, TimePrecision};
use icu_datetime::preferences::CalendarAlgorithm;
use icu_datetime::{DateTimeFormatter, DateTimeFormatterPreferences};
use icu_decimal::DecimalFormatter;
use icu_decimal::options::{DecimalFormatterOptions, GroupingStrategy};
use icu_list::ListFormatter;
use icu_list::options::{ListFormatterOptions, ListLength};
use icu_locale::Locale;

use crate::args::{ExactDecimal, ListValue, WhenValue};

/// The format locale, time zone and generation the formatters read while a message renders.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct FormatCtx {
    pub format_locale: String,
    pub time_zone: String,
    pub generation: u64,
}

thread_local! {
    /// The innermost context last; `with_format_ctx` pushes and pops.
    static CTX: RefCell<Vec<FormatCtx>> = const { RefCell::new(Vec::new()) };
    static MEMO: RefCell<Memo> = RefCell::new(Memo::default());
}

/// Runs `f` with `ctx` as the current format context. Re-entrant: an inner call shadows the outer
/// context until it returns (or unwinds).
pub fn with_format_ctx<R>(ctx: &FormatCtx, f: impl FnOnce() -> R) -> R {
    struct Pop;
    impl Drop for Pop {
        fn drop(&mut self) {
            CTX.with(|c| c.borrow_mut().pop());
        }
    }
    CTX.with(|c| c.borrow_mut().push(ctx.clone()));
    let _pop = Pop;
    f()
}

/// Calls `f` with the current context. Outside `with_format_ctx` (a bug: bundles are private to
/// this crate and every render path sets the context) debug builds panic and release builds use
/// `en-US`/`UTC`.
fn with_current<R>(f: impl FnOnce(&FormatCtx) -> R) -> R {
    CTX.with(|c| {
        let stack = c.borrow();
        match stack.last() {
            Some(ctx) => f(ctx),
            None => {
                debug_assert!(false, "l10n: a value was formatted outside with_format_ctx");
                f(&FormatCtx {
                    format_locale: "en-US".into(),
                    time_zone: "UTC".into(),
                    generation: 0,
                })
            }
        }
    })
}

/// Adds `NUMBER`, `DATETIME`, `LIST` and the `hv_format` formatter to `bundle`.
pub fn install_functions<R, M>(bundle: &mut FluentBundle<R, M>)
where
    R: Borrow<FluentResource>,
    M: MemoizerKind,
{
    bundle.set_formatter(Some(hv_format::<M>));
    // `add_function` fails only when the name is taken, which means it is installed already.
    let _ = bundle.add_function("NUMBER", number_fn);
    let _ = bundle.add_function("DATETIME", datetime_fn);
    let _ = bundle.add_function("LIST", list_fn);
}

/// The bundle formatter: numbers and our custom values in the current format context; strings
/// take Fluent's default path.
fn hv_format<M>(value: &FluentValue<'_>, _memoizer: &M) -> Option<String> {
    match value {
        FluentValue::Number(n) => Some(with_current(|ctx| format_number(ctx, n))),
        FluentValue::Custom(c) => format_custom(custom_any(c.as_ref())),
        FluentValue::String(_) | FluentValue::None | FluentValue::Error => None,
    }
}

/// The custom value inside the box. (`c.as_any()` on the `Box` itself would resolve to the
/// blanket `AnyEq` impl of the box type, and every downcast would fail.)
fn custom_any(c: &(dyn FluentType + Send)) -> &dyn Any {
    c.as_any()
}

fn format_custom(value: &dyn Any) -> Option<String> {
    with_current(|ctx| {
        if let Some(d) = value.downcast_ref::<ExactDecimal>() {
            let dec = Decimal::try_from_str(d.0.trim()).ok()?;
            Some(format_decimal(ctx, dec, &NumOpts::exact()))
        } else if let Some(d) = value.downcast_ref::<DecimalArg>() {
            Some(format_decimal(ctx, d.value.clone(), &d.opts))
        } else if let Some(w) = value.downcast_ref::<WhenValue>() {
            Some(format_when(ctx, *w, DateKey::default()))
        } else if let Some(a) = value.downcast_ref::<DateTimeArg>() {
            Some(format_when(ctx, a.when, a.key))
        } else if let Some(l) = value.downcast_ref::<ListValue>() {
            Some(format_list(ctx, &l.0, ListKind::And))
        } else {
            value
                .downcast_ref::<ListArg>()
                .map(|l| format_list(ctx, &l.items, l.kind))
        }
    })
}

// ---------------------------------------------------------------------------------------------
// NUMBER

/// The number options we honour, resolved from Fluent's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct NumOpts {
    percent: bool,
    grouping: bool,
    min_fraction: u8,
    /// `None`: keep every digit (exact decimals without an explicit maximum).
    max_fraction: Option<u8>,
    /// Drop trailing fraction zeros beyond `min_fraction` (Intl does for floats).
    trim: bool,
}

impl NumOpts {
    /// An `ExactDecimal` printed as is: every digit, trailing zeros kept.
    fn exact() -> NumOpts {
        NumOpts {
            percent: false,
            grouping: true,
            min_fraction: 0,
            max_fraction: None,
            trim: false,
        }
    }

    /// Intl defaults for a float: at most 3 fraction digits (0 for percent).
    fn of(o: &FluentNumberOptions) -> NumOpts {
        let percent = o.style == FluentNumberStyle::Percent;
        let min = digits(o.minimum_fraction_digits).unwrap_or(0);
        let max = digits(o.maximum_fraction_digits).unwrap_or(if percent { 0 } else { 3 });
        NumOpts {
            percent,
            grouping: o.use_grouping,
            min_fraction: min,
            max_fraction: Some(max.max(min)),
            trim: true,
        }
    }
}

fn digits(v: Option<usize>) -> Option<u8> {
    v.map(|d| u8::try_from(d.min(20)).unwrap_or(20))
}

/// `NUMBER($x, ...)`: validates the options and returns a Fluent number carrying them (I18N-05
/// step 3). Allowed: `minimumFractionDigits`, `maximumFractionDigits` (0–20), `useGrouping`,
/// `style` (`decimal`, `percent`) and `type` (`cardinal`, `ordinal`); anything else is ignored
/// and the default formatting stays.
fn number_fn<'a>(positional: &[FluentValue<'a>], named: &FluentArgs) -> FluentValue<'a> {
    let mut number = match positional.first() {
        Some(FluentValue::Number(n)) => n.clone(),
        Some(FluentValue::String(s)) => match FluentValue::try_number(s) {
            FluentValue::Number(n) => n,
            _ => return FluentValue::Error,
        },
        Some(FluentValue::Custom(c)) => {
            let any = custom_any(c.as_ref());
            if let Some(d) = any.downcast_ref::<ExactDecimal>() {
                match exact_to_number(&d.0) {
                    Ok(n) => n,
                    Err(Some(value)) => {
                        let mut options = FluentNumberOptions::default();
                        merge_options(&mut options, named);
                        let mut opts = NumOpts::of(&options);
                        if options.maximum_fraction_digits.is_none() {
                            opts.max_fraction = None;
                            opts.trim = false;
                        }
                        return FluentValue::Custom(Box::new(DecimalArg { value, opts }));
                    }
                    Err(None) => return FluentValue::Error,
                }
            } else {
                return FluentValue::Error;
            }
        }
        _ => return FluentValue::Error,
    };
    merge_options(&mut number.options, named);
    FluentValue::Number(number)
}

/// An exact decimal as a Fluent number when its `f64` round-trips the digits exactly (so it can
/// select plural variants); otherwise `Err(Some(decimal))`, formatted exactly but not selectable.
/// `Err(None)` when the text is not a decimal.
fn exact_to_number(text: &str) -> Result<FluentNumber, Option<Decimal>> {
    let text = text.trim();
    let exact = Decimal::try_from_str(text).map_err(|_| None)?;
    let float: f64 = text.parse().map_err(|_| Some(exact.clone()))?;
    let mut a = exact.clone();
    a.absolute.trim_end();
    let mut b = Decimal::try_from_str(&shortest(float)).map_err(|_| Some(exact.clone()))?;
    b.absolute.trim_end();
    if !float.is_finite()
        || a.absolute != b.absolute
        || (a.sign == Sign::Negative) != (b.sign == Sign::Negative)
    {
        return Err(Some(exact));
    }
    let fraction = text.split_once('.').map_or(0, |(_, f)| f.len());
    Ok(FluentNumber::new(
        float,
        FluentNumberOptions {
            minimum_fraction_digits: (fraction > 0).then_some(fraction),
            ..FluentNumberOptions::default()
        },
    ))
}

fn merge_options(o: &mut FluentNumberOptions, named: &FluentArgs) {
    for (key, value) in named.iter() {
        match (key, value) {
            ("minimumFractionDigits", FluentValue::Number(n)) if fraction_digits(n).is_some() => {
                o.minimum_fraction_digits = fraction_digits(n);
            }
            ("maximumFractionDigits", FluentValue::Number(n)) if fraction_digits(n).is_some() => {
                o.maximum_fraction_digits = fraction_digits(n);
            }
            ("useGrouping", FluentValue::String(s))
                if matches!(s.as_ref(), "true" | "false" | "auto" | "always") =>
            {
                o.use_grouping = s != "false";
            }
            ("style", FluentValue::String(s)) if matches!(s.as_ref(), "decimal" | "percent") => {
                o.style = FluentNumberStyle::from(s.as_ref());
            }
            ("type", FluentValue::String(s)) if matches!(s.as_ref(), "cardinal" | "ordinal") => {
                o.r#type = FluentNumberType::from(s.as_ref());
            }
            _ => tracing::warn!(option = key, "l10n: NUMBER ignores an unsupported option"),
        }
    }
}

fn fraction_digits(n: &FluentNumber) -> Option<usize> {
    let v = n.value;
    (v.fract() == 0.0 && (0.0..=20.0).contains(&v)).then_some(v as usize)
}

/// The shortest text that round-trips `v`, never in exponent form (`1e21` prints all digits).
fn shortest(v: f64) -> String {
    if v == 0.0 {
        // Edge 6: negative zero prints as `0`.
        "0".to_string()
    } else {
        format!("{v}")
    }
}

fn format_number(ctx: &FormatCtx, n: &FluentNumber) -> String {
    let v = n.value;
    if v.is_nan() {
        return "NaN".to_string();
    }
    if v.is_infinite() {
        return if v < 0.0 { "-∞" } else { "∞" }.to_string();
    }
    match Decimal::try_from_str(&shortest(v)) {
        Ok(dec) => format_decimal(ctx, dec, &NumOpts::of(&n.options)),
        Err(_) => shortest(v),
    }
}

fn format_decimal(ctx: &FormatCtx, mut dec: Decimal, o: &NumOpts) -> String {
    if o.percent {
        dec.multiply_pow10(2);
    }
    // `0.25` × 100 keeps the old leading zero (`025`); Intl never pads the integer part.
    dec.absolute.trim_start();
    if let Some(max) = o.max_fraction {
        dec.round_with_mode(
            -i16::from(max),
            SignedRoundingMode::Unsigned(UnsignedRoundingMode::HalfExpand),
        );
    }
    if o.trim {
        dec.absolute.trim_end();
    }
    dec.absolute.pad_end(-i16::from(o.min_fraction));
    with_memo(ctx, |m, locale| {
        m.ensure_decimal(locale, o.grouping);
        let Some(Some(f)) = m.decimal.get(&o.grouping) else {
            return dec.to_string();
        };
        let render = |d: &Decimal| f.format(d).to_string();
        if !o.percent {
            return render(&dec);
        }
        let negative = dec.sign == Sign::Negative;
        let mut abs = dec.clone();
        abs.sign = Sign::None;
        let body = render(&abs);
        // The locale's minus sign (with any marks) is what the signed form adds in front.
        let sign = if negative {
            let signed = render(&dec);
            signed
                .strip_suffix(body.as_str())
                .map_or_else(|| "-".to_string(), str::to_string)
        } else {
            String::new()
        };
        percent_pattern(locale, &sign, &body)
    })
}

/// CLDR's percent pattern for the format locale's language. ICU4X 2.4 ships percent data only in
/// `icu_experimental`, which is not a dependency, so the pattern is the CLDR 48 one for Turkish
/// (`%#,##0`, the canonical regression) and the common `#,##0%` otherwise.
fn percent_pattern(locale: &Locale, sign: &str, body: &str) -> String {
    match locale.id.language.as_str() {
        "tr" => format!("{sign}%{body}"),
        _ => format!("{sign}{body}%"),
    }
}

/// A decimal too precise for `f64`, with the `NUMBER` options applied: formatted exactly, not
/// usable as a plural selector.
#[derive(Debug, Clone, PartialEq)]
struct DecimalArg {
    value: Decimal,
    opts: NumOpts,
}

// ---------------------------------------------------------------------------------------------
// DATETIME

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Style {
    Full,
    Long,
    Medium,
    Short,
}

impl Style {
    fn parse(s: &str) -> Option<Style> {
        match s {
            "full" => Some(Style::Full),
            "long" => Some(Style::Long),
            "medium" => Some(Style::Medium),
            "short" => Some(Style::Short),
            _ => None,
        }
    }

    fn length(self) -> Length {
        match self {
            Style::Full | Style::Long => Length::Long,
            Style::Medium => Length::Medium,
            Style::Short => Length::Short,
        }
    }
}

/// `dateStyle` and `timeStyle`; with neither, a short date (Intl's default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
struct DateKey {
    date: Option<Style>,
    time: Option<Style>,
}

impl DateKey {
    fn field_set(self) -> Option<CompositeDateTimeFieldSet> {
        let date = match (self.date, self.time) {
            (None, None) => Some(Style::Short),
            (d, _) => d,
        };
        let mut b = FieldSetBuilder::new();
        if let Some(d) = date {
            b.date_fields = Some(if d == Style::Full {
                DateFields::YMDE
            } else {
                DateFields::YMD
            });
        }
        if let Some(t) = self.time {
            b.time_precision = Some(if t == Style::Short {
                TimePrecision::Minute
            } else {
                TimePrecision::Second
            });
        }
        b.length = date.or(self.time).map(Style::length);
        b.build_composite_datetime().ok()
    }
}

/// The value `DATETIME` returns.
#[derive(Debug, Clone, Copy, PartialEq)]
struct DateTimeArg {
    when: WhenValue,
    key: DateKey,
}

/// `DATETIME($t, dateStyle: .., timeStyle: ..)`: `$t` is a `WhenValue` or an ISO 8601 string; a
/// string that is not ISO 8601 prints as given (edge 9).
fn datetime_fn<'a>(positional: &[FluentValue<'a>], named: &FluentArgs) -> FluentValue<'a> {
    let when = match positional.first() {
        Some(FluentValue::Custom(c)) => {
            let any = custom_any(c.as_ref());
            any.downcast_ref::<WhenValue>()
                .copied()
                .or_else(|| any.downcast_ref::<DateTimeArg>().map(|a| a.when))
        }
        Some(FluentValue::String(s)) => match parse_iso8601(s) {
            Some(w) => Some(w),
            None => {
                tracing::warn!(value = %s, "l10n: DATETIME got a string that is not ISO 8601");
                return FluentValue::String(s.clone());
            }
        },
        _ => None,
    };
    let Some(when) = when else {
        return FluentValue::Error;
    };
    let mut key = DateKey::default();
    for (name, value) in named.iter() {
        match (name, value) {
            ("dateStyle", FluentValue::String(s)) if Style::parse(s).is_some() => {
                key.date = Style::parse(s);
            }
            ("timeStyle", FluentValue::String(s)) if Style::parse(s).is_some() => {
                key.time = Style::parse(s);
            }
            _ => tracing::warn!(
                option = name,
                "l10n: DATETIME ignores an unsupported option"
            ),
        }
    }
    FluentValue::Custom(Box::new(DateTimeArg { when, key }))
}

/// `YYYY-MM-DD`, optionally `THH:MM[:SS[.fff]]`, optionally `Z` or `±HH:MM`. With a zone it is an
/// instant; without, a wall-clock value.
fn parse_iso8601(s: &str) -> Option<WhenValue> {
    let s = s.trim();
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let part = s.get(r)?;
        part.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    if s.len() < 10 || s.as_bytes()[4] != b'-' || s.as_bytes()[7] != b'-' {
        return None;
    }
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let mut rest = &s[10..];
    let (mut hour, mut minute, mut second) = (0, 0, 0);
    if let Some(t) = rest.strip_prefix(['T', ' ']) {
        let tn = |r: std::ops::Range<usize>| -> Option<i64> {
            let part = t.get(r)?;
            part.bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| part.parse().ok())?
        };
        hour = tn(0..2)?;
        (t.as_bytes().get(2) == Some(&b':')).then_some(())?;
        minute = tn(3..5)?;
        rest = &t[5..];
        if let Some(sec) = rest.strip_prefix(':') {
            second = sec.get(0..2).and_then(|p| p.parse().ok())?;
            rest = &sec[2..];
            if let Some(frac) = rest.strip_prefix('.') {
                let n = frac.bytes().take_while(u8::is_ascii_digit).count();
                rest = &frac[n..];
            }
        }
    }
    let valid = (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && hour < 24
        && minute < 60
        && second < 60;
    if !valid {
        return None;
    }
    let offset_secs = match rest {
        "" => {
            return Some(WhenValue::Local {
                year: i32::try_from(year).ok()?,
                month: u8::try_from(month).ok()?,
                day: u8::try_from(day).ok()?,
                hour: u8::try_from(hour).ok()?,
                minute: u8::try_from(minute).ok()?,
                second: u8::try_from(second).ok()?,
            });
        }
        "Z" | "z" => 0,
        o if o.len() == 6 && o.as_bytes()[3] == b':' => {
            let sign = match o.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let h: i64 = o.get(1..3)?.parse().ok()?;
            let m: i64 = o.get(4..6)?.parse().ok()?;
            sign * (h * 3600 + m * 60)
        }
        _ => return None,
    };
    let secs = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second
        - offset_secs;
    Some(WhenValue::Instant {
        unix_ms: secs * 1000,
    })
}

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The proleptic Gregorian date of `days` since 1970-01-01 (H. Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// The wall-clock fields of `when`: an instant is converted to the context's time zone (F30); a
/// local value is taken as given.
fn wall_clock(ctx: &FormatCtx, when: WhenValue) -> Option<(i32, u8, u8, u8, u8, u8)> {
    Some(match when {
        WhenValue::Local {
            year,
            month,
            day,
            hour,
            minute,
            second,
        } => (year, month, day, hour, minute, second),
        WhenValue::Instant { unix_ms } => {
            let utc = unix_ms.div_euclid(1000);
            let local = utc + i64::from(utc_offset_seconds(ctx, utc));
            let (y, mo, d) = civil_from_days(local.div_euclid(86_400));
            let sod = local.rem_euclid(86_400);
            (
                i32::try_from(y).ok()?,
                u8::try_from(mo).ok()?,
                u8::try_from(d).ok()?,
                u8::try_from(sod / 3600).ok()?,
                u8::try_from(sod % 3600 / 60).ok()?,
                u8::try_from(sod % 60).ok()?,
            )
        }
    })
}

/// The UTC offset of `ctx.time_zone` at `unix_secs` from the tz database, daylight saving time
/// included (jiff: the OS zoneinfo, or its bundled copy where the OS has none, as on Windows).
/// Unknown zones format in UTC (edge 10). Zones are cached per thread until the generation
/// changes.
fn utc_offset_seconds(ctx: &FormatCtx, unix_secs: i64) -> i32 {
    thread_local! {
        static ZONES: RefCell<(u64, HashMap<String, Option<jiff::tz::TimeZone>>)> =
            RefCell::new((0, HashMap::new()));
    }
    let Ok(at) = jiff::Timestamp::from_second(unix_secs) else {
        return 0;
    };
    ZONES.with(|zones| {
        let mut zones = zones.borrow_mut();
        if zones.0 != ctx.generation {
            *zones = (ctx.generation, HashMap::new());
        }
        let tz = zones
            .1
            .entry(ctx.time_zone.clone())
            .or_insert_with(|| jiff::tz::TimeZone::get(&ctx.time_zone).ok());
        match tz {
            Some(tz) => tz.to_offset(at).seconds(),
            None => {
                warn_once(&ctx.time_zone, "l10n: unknown time zone, formatting in UTC");
                0
            }
        }
    })
}

fn warn_once(key: &str, message: &'static str) {
    static SEEN: OnceLock<Mutex<HashSet<(String, &'static str)>>> = OnceLock::new();
    let first = SEEN
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert((key.to_string(), message));
    if first {
        tracing::warn!(zone = key, "{message}");
    }
}

fn format_when(ctx: &FormatCtx, when: WhenValue, key: DateKey) -> String {
    let dt = wall_clock(ctx, when).and_then(|(y, mo, d, h, mi, s)| {
        Some(DateTime {
            date: Date::try_new_iso(y, mo, d).ok()?,
            time: Time::try_new(h, mi, s, 0).ok()?,
        })
    });
    let Some(dt) = dt else {
        return when.iso8601();
    };
    with_memo(ctx, |m, locale| match m.datetime(locale, key) {
        Some(f) => f.format(&dt).to_string(),
        None => when.iso8601(),
    })
}

// ---------------------------------------------------------------------------------------------
// LIST

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ListKind {
    And,
    Or,
    Unit,
}

/// The value `LIST` returns.
#[derive(Debug, Clone, PartialEq)]
struct ListArg {
    items: Vec<String>,
    kind: ListKind,
}

/// `LIST($items, type: "and" | "or" | "unit")`.
fn list_fn<'a>(positional: &[FluentValue<'a>], named: &FluentArgs) -> FluentValue<'a> {
    let items = match positional.first() {
        Some(FluentValue::Custom(c)) => {
            let any = custom_any(c.as_ref());
            match any.downcast_ref::<ListValue>() {
                Some(l) => l.0.clone(),
                None => match any.downcast_ref::<ListArg>() {
                    Some(l) => l.items.clone(),
                    None => return FluentValue::Error,
                },
            }
        }
        Some(FluentValue::String(s)) => vec![s.to_string()],
        _ => return FluentValue::Error,
    };
    let mut kind = ListKind::And;
    for (name, value) in named.iter() {
        match (name, value) {
            ("type", FluentValue::String(s)) if s == "and" => kind = ListKind::And,
            ("type", FluentValue::String(s)) if s == "or" => kind = ListKind::Or,
            ("type", FluentValue::String(s)) if s == "unit" => kind = ListKind::Unit,
            _ => tracing::warn!(option = name, "l10n: LIST ignores an unsupported option"),
        }
    }
    FluentValue::Custom(Box::new(ListArg { items, kind }))
}

fn format_list(ctx: &FormatCtx, items: &[String], kind: ListKind) -> String {
    with_memo(ctx, |m, locale| {
        m.ensure_list(locale, kind);
        match m.list.get(&kind) {
            Some(Some(f)) => f.format(items.iter().map(String::as_str)).to_string(),
            _ => items.join(", "),
        }
    })
}

// ---------------------------------------------------------------------------------------------
// Custom value plumbing

macro_rules! fluent_type {
    ($t:ty) => {
        impl FluentType for $t {
            fn duplicate(&self) -> Box<dyn FluentType + Send> {
                Box::new(self.clone())
            }
            fn as_string(&self, _: &intl_memoizer::IntlLangMemoizer) -> Cow<'static, str> {
                format_custom(self.as_any()).unwrap_or_default().into()
            }
            fn as_string_threadsafe(
                &self,
                _: &intl_memoizer::concurrent::IntlLangMemoizer,
            ) -> Cow<'static, str> {
                format_custom(self.as_any()).unwrap_or_default().into()
            }
        }
    };
}

fluent_type!(DecimalArg);
fluent_type!(DateTimeArg);
fluent_type!(ListArg);

// ---------------------------------------------------------------------------------------------
// Memoized formatters

/// Formatters of one format locale; `None` records a locale/options pair ICU4X cannot serve.
#[derive(Default)]
struct LocaleFormatters {
    locale: Option<Locale>,
    decimal: HashMap<bool, Option<DecimalFormatter>>,
    datetime: HashMap<DateKey, Option<DateTimeFormatter<CompositeDateTimeFieldSet>>>,
    list: HashMap<ListKind, Option<ListFormatter>>,
}

#[derive(Default)]
struct Memo {
    generation: Option<u64>,
    by_locale: HashMap<String, LocaleFormatters>,
}

/// Runs `f` with the memoized formatters of the context's format locale and the parsed locale
/// (`en-US` when the tag does not parse). The memo is dropped when the generation changes.
fn with_memo<R>(ctx: &FormatCtx, f: impl FnOnce(&mut LocaleFormatters, &Locale) -> R) -> R {
    MEMO.with(|memo| {
        let mut memo = memo.borrow_mut();
        if memo.generation != Some(ctx.generation) {
            memo.by_locale.clear();
            memo.generation = Some(ctx.generation);
        }
        if !memo.by_locale.contains_key(ctx.format_locale.as_str()) {
            let locale =
                Locale::try_from_str(&ctx.format_locale).unwrap_or(icu_locale::locale!("en-US"));
            memo.by_locale.insert(
                ctx.format_locale.clone(),
                LocaleFormatters {
                    locale: Some(locale),
                    ..LocaleFormatters::default()
                },
            );
        }
        let entry = memo
            .by_locale
            .get_mut(ctx.format_locale.as_str())
            .unwrap_or_else(|| unreachable!("inserted above"));
        let locale = entry.locale.take().unwrap_or(icu_locale::locale!("en-US"));
        let out = f(entry, &locale);
        entry.locale = Some(locale);
        out
    })
}

impl LocaleFormatters {
    fn ensure_decimal(&mut self, locale: &Locale, grouping: bool) {
        self.decimal.entry(grouping).or_insert_with(|| {
            let strategy = if grouping {
                GroupingStrategy::Auto
            } else {
                GroupingStrategy::Never
            };
            DecimalFormatter::try_new(locale.into(), DecimalFormatterOptions::from(strategy)).ok()
        });
    }

    fn datetime(
        &mut self,
        locale: &Locale,
        key: DateKey,
    ) -> Option<&DateTimeFormatter<CompositeDateTimeFieldSet>> {
        self.datetime
            .entry(key)
            .or_insert_with(|| {
                let mut prefs = DateTimeFormatterPreferences::from(locale);
                // Gregorian unless the locale asks for a calendar explicitly (`-u-ca-`), so an
                // OS region never leaks another calendar into chrome dates.
                if prefs.calendar_algorithm.is_none() {
                    prefs.calendar_algorithm = Some(CalendarAlgorithm::Gregory);
                }
                DateTimeFormatter::try_new(prefs, key.field_set()?).ok()
            })
            .as_ref()
    }

    fn ensure_list(&mut self, locale: &Locale, kind: ListKind) {
        self.list.entry(kind).or_insert_with(|| {
            let prefs = locale.into();
            let options = ListFormatterOptions::default().with_length(ListLength::Wide);
            match kind {
                ListKind::And => ListFormatter::try_new_and(prefs, options),
                ListKind::Or => ListFormatter::try_new_or(prefs, options),
                ListKind::Unit => ListFormatter::try_new_unit(prefs, options),
            }
            .ok()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_8601_strings_parse_to_instants_or_local_values() {
        assert_eq!(
            parse_iso8601("2026-11-11T14:05:09Z"),
            Some(WhenValue::Instant {
                unix_ms: 1_794_405_909_000
            })
        );
        assert_eq!(
            parse_iso8601("2026-11-11T17:05:09+03:00"),
            Some(WhenValue::Instant {
                unix_ms: 1_794_405_909_000
            })
        );
        assert_eq!(
            parse_iso8601("2026-11-11 14:05"),
            Some(WhenValue::Local {
                year: 2026,
                month: 11,
                day: 11,
                hour: 14,
                minute: 5,
                second: 0
            })
        );
        assert_eq!(
            parse_iso8601("2026-11-11"),
            Some(WhenValue::Local {
                year: 2026,
                month: 11,
                day: 11,
                hour: 0,
                minute: 0,
                second: 0
            })
        );
        for bad in [
            "yesterday",
            "2026-13-01",
            "2026-11-11T25:00",
            "2026-11-11T14:05+3",
        ] {
            assert_eq!(parse_iso8601(bad), None, "{bad}");
        }
    }

    #[test]
    fn civil_dates_round_trip() {
        for days in [-1, 0, 20_403, 719_468, -719_468] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
    }

    #[test]
    fn offsets_follow_the_tz_database() {
        let at = |zone: &str| FormatCtx {
            format_locale: "en-US".into(),
            time_zone: zone.into(),
            generation: 1,
        };
        // 2026-11-11T14:05:09Z, 2026-07-01T16:00Z and 2026-01-15T16:00Z.
        let (nov, jul, jan) = (1_794_405_909, 1_782_921_600, 1_768_492_800);
        assert_eq!(utc_offset_seconds(&at("Europe/Istanbul"), nov), 3 * 3600);
        assert_eq!(utc_offset_seconds(&at("America/New_York"), jul), -4 * 3600);
        assert_eq!(utc_offset_seconds(&at("America/New_York"), jan), -5 * 3600);
        assert_eq!(utc_offset_seconds(&at("UTC"), nov), 0);
        assert_eq!(utc_offset_seconds(&at("No/Such_Zone"), nov), 0);
    }

    #[test]
    fn exact_decimals_become_numbers_only_when_f64_is_exact() {
        let n = exact_to_number("1234.50").expect("exact in f64");
        assert_eq!(
            (n.value, n.options.minimum_fraction_digits),
            (1234.5, Some(2))
        );
        assert!(matches!(
            exact_to_number("12345678901234567890.1"),
            Err(Some(_))
        ));
        assert!(matches!(exact_to_number("abc"), Err(None)));
    }

    #[test]
    fn memo_is_dropped_when_the_generation_changes() {
        let ctx = |generation| FormatCtx {
            format_locale: "tr-TR".into(),
            time_zone: "UTC".into(),
            generation,
        };
        with_memo(&ctx(1), |m, l| m.ensure_decimal(l, true));
        MEMO.with(|m| assert_eq!(m.borrow().by_locale["tr-TR"].decimal.len(), 1));
        with_memo(&ctx(2), |m, _| assert!(m.decimal.is_empty()));
    }
}
