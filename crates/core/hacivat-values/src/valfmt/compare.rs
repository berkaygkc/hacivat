//! The local sort's comparisons (HV-GRID-07, R-M3-70; M3c Task 36).
//!
//! [`comparator_for`] gives one column's ascending order over its values; the app's sort job
//! (`hacivat-app::results::sort`) chains one per sort key and reverses it for a descending
//! key, NULLs included, as `ORDER BY … DESC` does.
//!
//! - **Text compares by the column's database collation, never by the UI locale.** A
//!   collation name the engine reports is resolved by [`resolve_collation`]: an ICU one
//!   (PostgreSQL `tr-x-icu`, `und-u-ks-level2`), a glibc locale (`tr_TR.utf8`), a MySQL or
//!   MariaDB UCA one (`utf8mb4_tr_0900_ai_ci`, `utf8mb4_turkish_ci`,
//!   `utf8mb4_uca1400_turkish_ai_ci`), a SQL Server one (`Turkish_CI_AS`) maps to the ICU4X
//!   collator of that language at the matching strength (accent-insensitive: primary;
//!   case-insensitive: secondary; else tertiary). A binary collation (`C`, `*_bin`, SQLite
//!   `BINARY`) **is** code-point order. Anything else (`utf8mb4_general_ci`, SQLite `NOCASE`,
//!   PostgreSQL's `default`, an unknown name) has no equivalent here: text then compares by
//!   code point, and [`CollationUse::Fallback`] says so for the header tooltip. With no
//!   collation at all (the result did not carry one) it is code point too
//!   ([`CollationUse::Unknown`]). Nothing here reads the process or UI locale.
//! - **Numbers compare as numbers.** Integers of every width exactly; exact decimals exactly
//!   (sign, then the digits; `1.50` equals `1.5`, `-0.00` equals `0`), **never through a
//!   float**, past 38 digits too; floats by IEEE order with NaN after +∞ (PostgreSQL's order)
//!   and −0 equal to 0; −∞/+∞/NaN specials around the finite numbers. A float against an
//!   exact number (a column the engine never types that way) compares through the float.
//! - **Temporal values compare by instant**: a `timestamptz` by its UTC instant whatever
//!   offset the server sent, a date as its midnight, a `timetz` by its UTC time (then its
//!   offset), an interval as PostgreSQL does (a month is 30 days, a day 24 hours); MySQL's
//!   zero dates before every date, −infinity first and +infinity last.
//! - **Enums** (PostgreSQL `CREATE TYPE … AS ENUM`, MySQL `ENUM`) compare by their labels'
//!   declared order, as both engines sort them; a label the type does not list sorts after
//!   them, by text.
//! - **Other values:** booleans `false < true`; bytes and UUIDs by their bytes; JSON, XML and
//!   server text (`Opaque`) as text; text that is not valid UTF-8 by its bytes (as the
//!   collator would see its replacement characters under a mapped collation); a truncated
//!   value by the prefix the result kept (an approximate order: the rest is not here).
//! - **A value that could not be decoded** sorts after every value (and before NULLs that sort
//!   last). Values of different kinds in one column (an untyped "Run as one" result is all
//!   text, so this is rare) order by kind: booleans, numbers, text, bytes, dates and
//!   timestamps, times, intervals, errors.
//! - **NULLs** sit where the engine sorts them ascending ([`nulls_at`]): last on PostgreSQL
//!   and its forks, Oracle and ClickHouse; first on MySQL, MariaDB, TiDB, SQL Server, SQLite
//!   and CockroachDB. `Missing` and `Default` cells sort with them.
//! - **Ties are `Equal`**: the sort is stable, so equal values keep model order (alg/22).

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

use hacivat_proto::DbmsKind;
use icu_collator::options::{CollatorOptions, Strength as IcuStrength};
use icu_collator::{CollatorBorrowed, CollatorPreferences};

use super::ColumnFacts;
use crate::decimal::DecimalRef;
use crate::logical::LogicalType;
use crate::special::SpecialVal;
use crate::value::ValueRef;

/// How finely a mapped collation tells text apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Strength {
    /// Base letters only: accent- and case-insensitive (`*_ai_ci`).
    Primary,
    /// Accents too: case-insensitive (`*_as_ci`, `*_CI_AS`).
    Secondary,
    /// Case too (`*_cs`, ICU's default).
    Tertiary,
}

/// What a collation name resolves to ([`resolve_collation`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Resolved {
    /// The ICU4X collator of `locale` (a BCP-47 tag, `tr`, `tr-TR`, `und`,
    /// `de-u-co-phonebk`) at `strength`.
    Icu { locale: String, strength: Strength },
    /// The collation orders by code point (binary on UTF-8).
    CodePoint,
    /// No equivalent here: text falls back to code point.
    NoEquivalent,
}

/// How a column's text is compared, for the header tooltip.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CollationUse {
    /// By the ICU4X collator the column's collation `name` maps to.
    Mapped { name: Arc<str> },
    /// The column's collation `name` is code-point order: compared exactly so.
    CodePoint { name: Arc<str> },
    /// The column's collation `name` has no equivalent here: compared by code point instead.
    Fallback { name: Arc<str> },
    /// The column's collation is not known (the result did not say): compared by code point.
    Unknown,
}

impl CollationUse {
    /// The collation's name, when the result named one.
    pub fn name(&self) -> Option<&str> {
        match self {
            CollationUse::Mapped { name }
            | CollationUse::CodePoint { name }
            | CollationUse::Fallback { name } => Some(name),
            CollationUse::Unknown => None,
        }
    }
}

/// Where NULLs sort in an ascending order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NullsAt {
    First,
    Last,
}

/// Where `engine` sorts NULLs ascending by default (`ORDER BY x` without `NULLS FIRST/LAST`).
pub fn nulls_at(engine: DbmsKind) -> NullsAt {
    match engine {
        DbmsKind::Postgres
        | DbmsKind::Redshift
        | DbmsKind::Yugabyte
        | DbmsKind::Timescale
        | DbmsKind::Oracle
        | DbmsKind::ClickHouse => NullsAt::Last,
        _ => NullsAt::First,
    }
}

/// One column's ascending order (module docs). `Send + Sync`: the sort's workers share it.
pub struct Comparator {
    text: TextOrder,
    nulls: NullsAt,
    collation: CollationUse,
    /// The column holds dates or times: ±infinity specials are temporal.
    temporal: bool,
    /// An enum column's labels by declared position.
    labels: Option<HashMap<Arc<str>, usize>>,
}

/// How text compares.
enum TextOrder {
    CodePoint,
    Icu(Box<CollatorBorrowed<'static>>),
}

impl std::fmt::Debug for Comparator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Comparator")
            .field("collation", &self.collation)
            .field("nulls", &self.nulls)
            .finish_non_exhaustive()
    }
}

/// The comparator of a column with `col`'s facts whose database collation is `collation`
/// (`None`: the result did not say). Never the UI locale (R-M3-70).
pub fn comparator_for(col: &ColumnFacts<'_>, collation: Option<&str>) -> Comparator {
    let (text, collation) = match collation {
        None => (TextOrder::CodePoint, CollationUse::Unknown),
        Some(name) => {
            let named: Arc<str> = Arc::from(name);
            match resolve_collation(name) {
                Resolved::Icu { locale, strength } => match collator(&locale, strength) {
                    Some(c) => (
                        TextOrder::Icu(Box::new(c)),
                        CollationUse::Mapped { name: named },
                    ),
                    None => (TextOrder::CodePoint, CollationUse::Fallback { name: named }),
                },
                Resolved::CodePoint => (
                    TextOrder::CodePoint,
                    CollationUse::CodePoint { name: named },
                ),
                Resolved::NoEquivalent => {
                    (TextOrder::CodePoint, CollationUse::Fallback { name: named })
                }
            }
        }
    };
    Comparator {
        text,
        nulls: nulls_at(col.engine),
        collation,
        temporal: matches!(
            col.logical,
            LogicalType::Date | LogicalType::Timestamp { .. } | LogicalType::Time { .. }
        ),
        labels: match col.logical {
            LogicalType::Enum(labels) => Some(
                labels
                    .iter()
                    .enumerate()
                    .map(|(i, l)| (l.clone(), i))
                    .collect(),
            ),
            _ => None,
        },
    }
}

/// The ICU4X collator of `locale` at `strength` (compiled data; `None` when the tag does not
/// parse or has no data).
fn collator(locale: &str, strength: Strength) -> Option<CollatorBorrowed<'static>> {
    let locale: icu_locale::Locale = locale.parse().ok()?;
    let mut options = CollatorOptions::default();
    options.strength = Some(match strength {
        Strength::Primary => IcuStrength::Primary,
        Strength::Secondary => IcuStrength::Secondary,
        Strength::Tertiary => IcuStrength::Tertiary,
    });
    CollatorBorrowed::try_new(CollatorPreferences::from(&locale), options).ok()
}

impl Comparator {
    /// How this column's text compares (the header tooltip names it).
    pub fn collation(&self) -> &CollationUse {
        &self.collation
    }

    /// Where NULLs sort ascending.
    pub fn nulls(&self) -> NullsAt {
        self.nulls
    }

    /// `a` against `b`, ascending (module docs).
    pub fn cmp(&self, a: ValueRef<'_>, b: ValueRef<'_>) -> Ordering {
        let (a, b) = (untruncated(a), untruncated(b));
        let (ka, kb) = (self.kind(a), self.kind(b));
        match (ka, kb) {
            (Kind::Null, Kind::Null) => return Ordering::Equal,
            (Kind::Null, _) => {
                return match self.nulls {
                    NullsAt::First => Ordering::Less,
                    NullsAt::Last => Ordering::Greater,
                };
            }
            (_, Kind::Null) => {
                return match self.nulls {
                    NullsAt::First => Ordering::Greater,
                    NullsAt::Last => Ordering::Less,
                };
            }
            _ => {}
        }
        if ka != kb {
            return ka.cmp(&kb);
        }
        match ka {
            Kind::Null | Kind::Error => Ordering::Equal,
            Kind::Bool => as_bool(a).cmp(&as_bool(b)),
            Kind::Number => cmp_numbers(number(a), number(b)),
            Kind::Text => self.cmp_text(a, b),
            Kind::Bytes => as_bytes(&a).cmp(as_bytes(&b)),
            Kind::DateTime => instant(a).cmp(&instant(b)),
            Kind::Time => time_key(a).cmp(&time_key(b)),
            Kind::Interval => interval_key(a).cmp(&interval_key(b)),
        }
    }

    fn kind(&self, v: ValueRef<'_>) -> Kind {
        match v {
            ValueRef::Null | ValueRef::Missing | ValueRef::Default => Kind::Null,
            ValueRef::Bool(_) => Kind::Bool,
            ValueRef::Int(_)
            | ValueRef::UInt(_)
            | ValueRef::F32(_)
            | ValueRef::F64(_)
            | ValueRef::Decimal(_) => Kind::Number,
            ValueRef::Special(s) => match s {
                SpecialVal::ZeroDate | SpecialVal::PartialZeroDate(_) => Kind::DateTime,
                SpecialVal::PosInfinity | SpecialVal::NegInfinity if self.temporal => {
                    Kind::DateTime
                }
                _ => Kind::Number,
            },
            ValueRef::Text(_)
            | ValueRef::Json(_)
            | ValueRef::Xml(_)
            | ValueRef::Opaque(_)
            | ValueRef::Lossy(_) => Kind::Text,
            ValueRef::Bytes(_) | ValueRef::Uuid(_) => Kind::Bytes,
            ValueRef::Date(_) | ValueRef::Timestamp(_) | ValueRef::TimestampTz { .. } => {
                Kind::DateTime
            }
            ValueRef::Time(_) | ValueRef::TimeTz(..) => Kind::Time,
            ValueRef::Interval { .. } => Kind::Interval,
            ValueRef::Error(_) => Kind::Error,
            // `untruncated` took the prefix out; a truncated value inside one is not built.
            ValueRef::Truncated(_) => Kind::Error,
        }
    }

    fn cmp_text(&self, a: ValueRef<'_>, b: ValueRef<'_>) -> Ordering {
        if let Some(labels) = &self.labels {
            let rank = |v: &ValueRef<'_>| {
                std::str::from_utf8(text_bytes(v))
                    .ok()
                    .and_then(|t| labels.get(t).copied())
                    .unwrap_or(usize::MAX)
            };
            let by_rank = rank(&a).cmp(&rank(&b));
            if by_rank != Ordering::Equal {
                return by_rank;
            }
        }
        match &self.text {
            // UTF-8 byte order is code-point order.
            TextOrder::CodePoint => text_bytes(&a).cmp(text_bytes(&b)),
            TextOrder::Icu(c) => c.compare(&text_str(&a), &text_str(&b)),
        }
    }
}

/// The kinds a column's values order by when they differ (module docs), NULL apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Null,
    Bool,
    Number,
    Text,
    Bytes,
    DateTime,
    Time,
    Interval,
    Error,
}

/// A truncated value as the prefix the result kept.
fn untruncated(v: ValueRef<'_>) -> ValueRef<'_> {
    match v {
        ValueRef::Truncated(t) => t.prefix.view(),
        other => other,
    }
}

fn as_bool(v: ValueRef<'_>) -> bool {
    matches!(v, ValueRef::Bool(true))
}

/// A binary value's bytes (a UUID holds its own, by value).
fn as_bytes<'b>(v: &'b ValueRef<'_>) -> &'b [u8] {
    match v {
        ValueRef::Bytes(b) => b,
        ValueRef::Uuid(u) => u,
        _ => &[],
    }
}

/// A text value's bytes (UTF-8, or the raw bytes of a `Lossy` one).
fn text_bytes<'a>(v: &ValueRef<'a>) -> &'a [u8] {
    match *v {
        ValueRef::Text(t) | ValueRef::Json(t) | ValueRef::Xml(t) => t.as_bytes(),
        ValueRef::Opaque(o) => o.text.as_bytes(),
        ValueRef::Lossy(b) => b,
        _ => &[],
    }
}

/// A text value as a string for the collator (a `Lossy` one with replacement characters).
fn text_str<'a>(v: &ValueRef<'a>) -> Cow<'a, str> {
    match *v {
        ValueRef::Text(t) | ValueRef::Json(t) | ValueRef::Xml(t) => Cow::Borrowed(t),
        ValueRef::Opaque(o) => Cow::Borrowed(&o.text),
        ValueRef::Lossy(b) => String::from_utf8_lossy(b),
        _ => Cow::Borrowed(""),
    }
}

// ---- numbers --------------------------------------------------------------------------------

/// A number's place on the line.
#[derive(Clone, Copy)]
enum Num<'a> {
    NegInf,
    Int(i128),
    Float(f64),
    Dec(DecimalRef<'a>),
    PosInf,
    NaN,
}

impl Num<'_> {
    /// −∞, the finite numbers, +∞, NaN.
    fn band(&self) -> u8 {
        match self {
            Num::NegInf => 0,
            Num::Int(_) | Num::Float(_) | Num::Dec(_) => 1,
            Num::PosInf => 2,
            Num::NaN => 3,
        }
    }
}

fn number(v: ValueRef<'_>) -> Num<'_> {
    let float = |f: f64| {
        if f.is_nan() {
            Num::NaN
        } else if f == f64::INFINITY {
            Num::PosInf
        } else if f == f64::NEG_INFINITY {
            Num::NegInf
        } else {
            Num::Float(f)
        }
    };
    match v {
        ValueRef::Int(i) => Num::Int(i128::from(i)),
        ValueRef::UInt(u) => Num::Int(i128::from(u)),
        ValueRef::F32(f) => float(f64::from(f)),
        ValueRef::F64(f) => float(f),
        ValueRef::Decimal(d) => Num::Dec(d),
        ValueRef::Special(SpecialVal::NegInfinity) => Num::NegInf,
        ValueRef::Special(SpecialVal::PosInfinity) => Num::PosInf,
        _ => Num::NaN,
    }
}

fn cmp_numbers(a: Num<'_>, b: Num<'_>) -> Ordering {
    let band = a.band().cmp(&b.band());
    if band != Ordering::Equal || a.band() != 1 {
        return band;
    }
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => x.cmp(&y),
        (Num::Dec(x), Num::Dec(y)) => cmp_decimals(x, y),
        (Num::Int(x), Num::Dec(y)) => cmp_decimal_text(&x.to_string(), &canonical(y)),
        (Num::Dec(x), Num::Int(y)) => cmp_decimal_text(&canonical(x), &y.to_string()),
        // A float against a float, or (never typed so by an engine) an exact number.
        (x, y) => as_f64(x).partial_cmp(&as_f64(y)).unwrap_or(Ordering::Equal),
    }
}

fn as_f64(n: Num<'_>) -> f64 {
    match n {
        Num::Int(i) => i as f64,
        Num::Float(f) => f,
        Num::Dec(d) => canonical(d).parse().unwrap_or(0.0),
        Num::NegInf => f64::NEG_INFINITY,
        Num::PosInf => f64::INFINITY,
        Num::NaN => f64::NAN,
    }
}

fn canonical(d: DecimalRef<'_>) -> String {
    let mut s = String::new();
    d.write_canonical(&mut s);
    s
}

/// The sign bit of a small decimal's high word.
const SIGN: u64 = 1 << 63;

/// Two exact decimals by value: the 38-digit form by its words, else by the digits.
fn cmp_decimals(a: DecimalRef<'_>, b: DecimalRef<'_>) -> Ordering {
    if let (Some((ma, sa)), Some((mb, sb))) = (a.small(), b.small()) {
        let unpack = |m: [u64; 2]| {
            let mag = (u128::from(m[1] & !SIGN) << 64) | u128::from(m[0]);
            (m[1] & SIGN != 0 && mag != 0, mag)
        };
        let ((na, xa), (nb, xb)) = (unpack(ma), unpack(mb));
        let sign = |neg: bool, mag: u128| match (neg, mag) {
            (_, 0) => 0i8,
            (true, _) => -1,
            (false, _) => 1,
        };
        let (ga, gb) = (sign(na, xa), sign(nb, xb));
        if ga != gb || ga == 0 {
            return ga.cmp(&gb);
        }
        let mag = cmp_magnitudes(xa, sa, xb, sb);
        return if ga < 0 { mag.reverse() } else { mag };
    }
    cmp_decimal_text(&canonical(a), &canonical(b))
}

/// `xa`·10^−`sa` against `xb`·10^−`sb`, both nonzero, by scaling the one with fewer
/// fractional digits up; a scale-up past `u128` is larger than the other (below 10^38).
fn cmp_magnitudes(xa: u128, sa: i16, xb: u128, sb: i16) -> Ordering {
    let scaled = |x: u128, by: i32| -> Option<u128> {
        let p = 10u128.checked_pow(u32::try_from(by).ok()?)?;
        x.checked_mul(p)
    };
    let s = i32::from(sa.max(sb));
    match (scaled(xa, s - i32::from(sa)), scaled(xb, s - i32::from(sb))) {
        (Some(a), Some(b)) => a.cmp(&b),
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (None, None) => Ordering::Equal,
    }
}

/// Two decimals in canonical text (`-`? digits (`.` digits)?), exactly.
fn cmp_decimal_text(a: &str, b: &str) -> Ordering {
    let (na, ia, fa) = split_decimal(a);
    let (nb, ib, fb) = split_decimal(b);
    let sign = |neg: bool, int: &str, frac: &str| {
        if int.is_empty() && frac.is_empty() {
            0i8
        } else if neg {
            -1
        } else {
            1
        }
    };
    let (ga, gb) = (sign(na, ia, fa), sign(nb, ib, fb));
    if ga != gb || ga == 0 {
        return ga.cmp(&gb);
    }
    let mag = ia
        .len()
        .cmp(&ib.len())
        .then_with(|| ia.cmp(ib))
        .then_with(|| fa.cmp(fb));
    if ga < 0 { mag.reverse() } else { mag }
}

/// A canonical decimal's sign, integer digits without leading zeros and fractional digits
/// without trailing zeros (zero is two empty strings).
fn split_decimal(s: &str) -> (bool, &str, &str) {
    let (neg, body) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let (int, frac) = body.split_once('.').unwrap_or((body, ""));
    (neg, int.trim_start_matches('0'), frac.trim_end_matches('0'))
}

// ---- temporal values --------------------------------------------------------------------------

/// A date or timestamp's instant, in nanoseconds since the epoch; specials around them.
fn instant(v: ValueRef<'_>) -> (u8, i128) {
    const NANOS: i128 = 1_000_000_000;
    match v {
        ValueRef::Special(SpecialVal::NegInfinity) => (0, 0),
        ValueRef::Special(SpecialVal::ZeroDate) => (1, 0),
        ValueRef::Special(SpecialVal::PartialZeroDate(_)) => (2, 0),
        ValueRef::Date(d) => (3, i128::from(d.0) * 86_400 * NANOS),
        ValueRef::Timestamp(t) | ValueRef::TimestampTz { utc: t, .. } => {
            (3, i128::from(t.secs) * NANOS + i128::from(t.nanos))
        }
        _ => (4, 0),
    }
}

/// A time of day at UTC (a `timetz` less its offset), then the offset.
fn time_key(v: ValueRef<'_>) -> (i128, i32) {
    match v {
        ValueRef::Time(t) => (i128::from(t.nanos_of_day), 0),
        ValueRef::TimeTz(t, offset) => (
            i128::from(t.nanos_of_day) - i128::from(offset) * 1_000_000_000,
            offset,
        ),
        _ => (0, 0),
    }
}

/// An interval's length as PostgreSQL compares it: a month is 30 days, a day 24 hours.
fn interval_key(v: ValueRef<'_>) -> i128 {
    match v {
        ValueRef::Interval {
            months,
            days,
            micros,
        } => (i128::from(months) * 30 + i128::from(days)) * 86_400_000_000 + i128::from(micros),
        _ => 0,
    }
}

// ---- collation names ------------------------------------------------------------------------

/// What the collation `name` (as the engine spells it) resolves to (module docs).
pub fn resolve_collation(name: &str) -> Resolved {
    let name = name.trim();
    if name.is_empty() {
        return Resolved::NoEquivalent;
    }
    let lower = name.to_ascii_lowercase();
    // Binary collations: code-point order (UTF-8 bytes; Latin-1 bytes are its code points).
    if matches!(
        lower.as_str(),
        "c" | "posix"
            | "c.utf-8"
            | "c.utf8"
            | "ucs_basic"
            | "pg_c_utf8"
            | "pg_unicode_fast"
            | "binary"
    ) || lower.ends_with("_bin")
        || lower.ends_with("_bin2")
    {
        return Resolved::CodePoint;
    }
    // PostgreSQL ICU collations: `tr-x-icu`, `tr-TR-x-icu`, `und-u-ks-level2`.
    if let Some(tag) = lower.strip_suffix("-x-icu") {
        return icu_tag(tag);
    }
    if lower.contains("-u-") {
        return icu_tag(&lower);
    }
    // glibc locales: `tr_TR.utf8`, `de_DE`, `en_US.UTF-8@euro`.
    if let Some(r) = glibc(name) {
        return r;
    }
    // SQL Server: `Turkish_CI_AS`, `Turkish_100_CS_AS`, `Latin1_General_CI_AI`.
    if let Some(r) = mssql(name) {
        return r;
    }
    mysql(&lower).unwrap_or(Resolved::NoEquivalent)
}

/// An ICU locale tag with its `-u-ks-levelN` strength (tertiary when absent); the language,
/// script, region and collation type (`-u-co-…`) are kept.
fn icu_tag(tag: &str) -> Resolved {
    let Ok(locale) = tag.parse::<icu_locale::Locale>() else {
        return Resolved::NoEquivalent;
    };
    let mut strength = Strength::Tertiary;
    let mut co = None;
    let mut parts = tag.split('-').skip_while(|p| *p != "u").skip(1).peekable();
    while let Some(key) = parts.next() {
        if key.len() != 2 {
            continue;
        }
        let value = parts.peek().copied().unwrap_or("");
        match (key, value) {
            ("ks", "level1") => strength = Strength::Primary,
            ("ks", "level2") => strength = Strength::Secondary,
            ("co", v) if !v.is_empty() => co = Some(v.to_string()),
            _ => {}
        }
    }
    let mut out = locale.id.to_string();
    if let Some(co) = co {
        out.push_str("-u-co-");
        out.push_str(&co);
    }
    Resolved::Icu {
        locale: out,
        strength,
    }
}

/// A glibc locale name: a two- or three-letter language, `_` and a two-letter region, then
/// an optional `.encoding` and `@modifier`.
fn glibc(name: &str) -> Option<Resolved> {
    let base = name.split(['.', '@']).next()?;
    let (lang, region) = base.split_once('_')?;
    let lang_ok = (2..=3).contains(&lang.len()) && lang.bytes().all(|b| b.is_ascii_lowercase());
    let region_ok = region.len() == 2 && region.bytes().all(|b| b.is_ascii_uppercase());
    if !lang_ok || !region_ok {
        return None;
    }
    Some(Resolved::Icu {
        locale: format!("{lang}-{region}"),
        strength: Strength::Tertiary,
    })
}

/// A SQL Server Windows collation: `<Language>[_<version>]_<CI|CS>_<AI|AS>[_…]`.
fn mssql(name: &str) -> Option<Resolved> {
    let parts: Vec<&str> = name.split('_').collect();
    let case_at = parts.iter().position(|p| *p == "CI" || *p == "CS")?;
    let accent = parts.get(case_at + 1)?;
    if *accent != "AI" && *accent != "AS" {
        return None;
    }
    if parts.first() == Some(&"SQL") {
        // The legacy `SQL_*` collations sort non-Unicode data by code-page tables.
        return Some(Resolved::NoEquivalent);
    }
    let language: Vec<&str> = parts[..case_at]
        .iter()
        .copied()
        .filter(|p| !p.bytes().all(|b| b.is_ascii_digit()))
        .collect();
    let locale = match language.join("_").as_str() {
        "Latin1_General" => "und",
        other => language_named(&other.to_ascii_lowercase())?,
    };
    let strength = match (parts[case_at], *accent) {
        ("CI", "AI") => Strength::Primary,
        ("CI", _) => Strength::Secondary,
        _ => Strength::Tertiary,
    };
    Some(Resolved::Icu {
        locale: locale.to_string(),
        strength,
    })
}

/// A MySQL or MariaDB collation: `<charset>_<rest>`, lower case.
fn mysql(lower: &str) -> Option<Resolved> {
    let (_charset, rest) = lower.split_once('_')?;
    let words: Vec<&str> = rest.split('_').collect();
    // The strength suffix: `ai_ci`, `as_ci`, `as_cs` (MySQL 8, MariaDB uca1400), or the legacy
    // `ci` / `cs`.
    let (body, strength): (&[&str], Strength) = match words.as_slice() {
        [body @ .., "ai", "ci"] => (body, Strength::Primary),
        [body @ .., "as", "ci"] => (body, Strength::Secondary),
        [body @ .., "as", "cs"] | [body @ .., "as", "cs", "ks"] => (body, Strength::Tertiary),
        [body @ .., "ci"] => (body, Strength::Primary),
        [body @ .., "cs"] => (body, Strength::Tertiary),
        _ => return None,
    };
    let locale = match body {
        // MySQL 8's UCA 9.0.0 family: `utf8mb4_0900_ai_ci`, `utf8mb4_tr_0900_ai_ci`.
        [lang @ .., "0900"] => mysql_language_code(&lang.join("_"))?,
        // MariaDB's UCA 14.0.0 family: `utf8mb4_uca1400_ai_ci`, `…_uca1400_turkish_ai_ci`.
        ["uca1400", lang @ ..] => {
            if lang.is_empty() {
                "und"
            } else {
                language_named(&lang.join("_"))?
            }
        }
        // The UCA 4.0.0 / 5.2.0 family: `utf8mb4_unicode_ci`, `utf8mb4_unicode_520_ci`.
        ["unicode"] | ["unicode", "520"] => "und",
        // `utf8mb4_turkish_ci`, `latin5_turkish_ci`, `utf8mb4_german2_ci`.
        [lang] => language_named(lang)?,
        _ => return None,
    };
    Some(Resolved::Icu {
        locale: locale.to_string(),
        strength,
    })
}

/// MySQL 8's language codes in `_0900_` collations (empty: the root order).
fn mysql_language_code(code: &str) -> Option<&'static str> {
    Some(match code {
        "" => "und",
        "de_pb" => "de-u-co-phonebk",
        "es_trad" => "es-u-co-trad",
        "sr_latn" => "sr-Latn",
        "zh" => "zh",
        "ja" => "ja",
        "ru" => "ru",
        "tr" => "tr",
        "is" => "is",
        "lv" => "lv",
        "ro" => "ro",
        "sl" => "sl",
        "pl" => "pl",
        "et" => "et",
        "es" => "es",
        "sv" => "sv",
        "cs" => "cs",
        "da" => "da",
        "lt" => "lt",
        "sk" => "sk",
        "la" => "la",
        "eo" => "eo",
        "hu" => "hu",
        "hr" => "hr",
        "vi" => "vi",
        "bs" => "bs",
        "bg" => "bg",
        "gl" => "gl",
        "mn" => "mn",
        "nb" => "nb",
        "nn" => "nn",
        _ => return None,
    })
}

/// The legacy language names of MySQL/MariaDB and SQL Server collations.
fn language_named(name: &str) -> Option<&'static str> {
    Some(match name {
        "turkish" => "tr",
        "german2" | "german_phonebook" => "de-u-co-phonebk",
        "german" | "german1" => "de",
        "spanish" | "modern_spanish" => "es",
        "spanish2" | "traditional_spanish" => "es-u-co-trad",
        "swedish" | "finnish_swedish" => "sv",
        "danish" | "danish_norwegian" => "da",
        "norwegian" => "nb",
        "polish" => "pl",
        "czech" => "cs",
        "slovak" => "sk",
        "slovenian" => "sl",
        "romanian" => "ro",
        "hungarian" => "hu",
        "estonian" => "et",
        "latvian" => "lv",
        "lithuanian" => "lt",
        "icelandic" => "is",
        "croatian" => "hr",
        "vietnamese" => "vi",
        "persian" => "fa",
        "esperanto" => "eo",
        "sinhala" => "si",
        "roman" | "latin" => "la",
        "greek" => "el",
        "russian" | "cyrillic_general" => "ru",
        "ukrainian" => "uk",
        "french" => "fr",
        "italian" => "it",
        "dutch" => "nl",
        "portuguese" => "pt",
        "japanese" => "ja",
        "korean" | "korean_wansung" => "ko",
        "chinese" | "chinese_prc" => "zh",
        "arabic" => "ar",
        "hebrew" => "he",
        "thai" => "th",
        _ => return None,
    })
}
