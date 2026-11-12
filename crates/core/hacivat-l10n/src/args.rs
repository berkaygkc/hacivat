//! Typed message references and Fluent argument values.

use std::borrow::Cow;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use fluent_bundle::types::FluentType;
use fluent_bundle::{FluentArgs, FluentValue};

/// A message ID whose arguments are typed by `A` (generated as `ids::<ns>::<CONST>`, Task 6).
pub struct MsgRef<A> {
    pub id: &'static str,
    args: PhantomData<fn(A)>,
}

impl<A> MsgRef<A> {
    pub const fn new(id: &'static str) -> Self {
        Self {
            id,
            args: PhantomData,
        }
    }
}

impl<A> Clone for MsgRef<A> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<A> Copy for MsgRef<A> {}

impl<A> fmt::Debug for MsgRef<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("MsgRef").field(&self.id).finish()
    }
}

impl<A> PartialEq for MsgRef<A> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<A> Eq for MsgRef<A> {}

impl<A> Hash for MsgRef<A> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// Converts a message's typed arguments into named Fluent arguments.
pub trait IntoFluentArgs {
    fn into_args(self) -> Vec<(&'static str, ArgValue)>;
}

/// One Fluent argument value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ArgValue {
    /// A count: selects the plural category of the UI language; digits follow the format
    /// locale.
    Count(u64),
    Text(String),
    Decimal(ExactDecimal),
    When(WhenValue),
    List(Vec<String>),
}

/// An exact decimal given as its text (`"1234.50"`), never through `f64`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExactDecimal(pub String);

/// A point in time to format with `DATETIME` (F30).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WhenValue {
    /// An instant, converted to the format time zone.
    Instant { unix_ms: i64 },
    /// A wall-clock value, formatted as given (no time-zone conversion).
    Local {
        year: i32,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    },
}

/// The arguments of a message without variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct NoArgs;

impl IntoFluentArgs for NoArgs {
    fn into_args(self) -> Vec<(&'static str, ArgValue)> {
        Vec::new()
    }
}

/// A count field of a generated args struct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Count(pub u64);

/// A text field of a generated args struct.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Text(pub String);

impl From<Count> for ArgValue {
    fn from(c: Count) -> Self {
        ArgValue::Count(c.0)
    }
}

impl From<Text> for ArgValue {
    fn from(t: Text) -> Self {
        ArgValue::Text(t.0)
    }
}

impl From<WhenValue> for ArgValue {
    fn from(w: WhenValue) -> Self {
        ArgValue::When(w)
    }
}

impl From<Vec<String>> for ArgValue {
    fn from(items: Vec<String>) -> Self {
        ArgValue::List(items)
    }
}

/// The Fluent value of an [`ArgValue::List`] (a `FluentValue::Custom`, so the `LIST` function
/// can find it).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ListValue(pub Vec<String>);

impl ArgValue {
    /// The Fluent value. Counts become numbers (they select plural variants); decimals, times
    /// and lists become custom values that the formatters (`format.rs`) downcast.
    pub(crate) fn into_fluent(self) -> FluentValue<'static> {
        match self {
            ArgValue::Count(n) => FluentValue::from(n),
            ArgValue::Text(s) => FluentValue::from(s),
            ArgValue::Decimal(d) => FluentValue::Custom(Box::new(d)),
            ArgValue::When(w) => FluentValue::Custom(Box::new(w)),
            ArgValue::List(items) => FluentValue::Custom(Box::new(ListValue(items))),
        }
    }
}

/// Fluent arguments from named values.
pub(crate) fn fluent_args(args: Vec<(&'static str, ArgValue)>) -> FluentArgs<'static> {
    args.into_iter()
        .map(|(name, value)| (name, value.into_fluent()))
        .collect()
}

// The plain-text renderings below are what a custom value shows without a formatter; Task 9's
// `hv_format` formats them for the format locale.

impl FluentType for ExactDecimal {
    fn duplicate(&self) -> Box<dyn FluentType + Send> {
        Box::new(self.clone())
    }
    fn as_string(&self, _: &intl_memoizer::IntlLangMemoizer) -> Cow<'static, str> {
        self.0.clone().into()
    }
    fn as_string_threadsafe(
        &self,
        _: &intl_memoizer::concurrent::IntlLangMemoizer,
    ) -> Cow<'static, str> {
        self.0.clone().into()
    }
}

impl FluentType for WhenValue {
    fn duplicate(&self) -> Box<dyn FluentType + Send> {
        Box::new(*self)
    }
    fn as_string(&self, _: &intl_memoizer::IntlLangMemoizer) -> Cow<'static, str> {
        self.iso8601().into()
    }
    fn as_string_threadsafe(
        &self,
        _: &intl_memoizer::concurrent::IntlLangMemoizer,
    ) -> Cow<'static, str> {
        self.iso8601().into()
    }
}

impl FluentType for ListValue {
    fn duplicate(&self) -> Box<dyn FluentType + Send> {
        Box::new(self.clone())
    }
    fn as_string(&self, _: &intl_memoizer::IntlLangMemoizer) -> Cow<'static, str> {
        self.0.join(", ").into()
    }
    fn as_string_threadsafe(
        &self,
        _: &intl_memoizer::concurrent::IntlLangMemoizer,
    ) -> Cow<'static, str> {
        self.0.join(", ").into()
    }
}

impl WhenValue {
    /// ISO 8601: `2026-11-11T14:05:09Z` for an instant (UTC), without the `Z` for a local value.
    pub fn iso8601(&self) -> String {
        match *self {
            WhenValue::Instant { unix_ms } => {
                let secs = unix_ms.div_euclid(1000);
                let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
                let s = secs.rem_euclid(86_400);
                format!(
                    "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
                    s / 3600,
                    s % 3600 / 60,
                    s % 60
                )
            }
            WhenValue::Local {
                year,
                month,
                day,
                hour,
                minute,
                second,
            } => format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}"),
        }
    }
}

/// The proleptic Gregorian date of `days` since 1970-01-01 (H. Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (
        y,
        u32::try_from(m).unwrap_or(1),
        u32::try_from(d).unwrap_or(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msg_ref_is_const_and_compares_by_id() {
        const A: MsgRef<NoArgs> = MsgRef::new("hv-a");
        assert_eq!(A.id, "hv-a");
        assert_eq!(A, MsgRef::<NoArgs>::new("hv-a"));
        assert_eq!(NoArgs.into_args(), Vec::new());
        assert_eq!(ArgValue::from(Count(2)), ArgValue::Count(2));
        assert_eq!(ArgValue::from(Text("x".into())), ArgValue::Text("x".into()));
    }

    #[test]
    fn when_values_print_as_iso_8601_without_a_formatter() {
        let instant = WhenValue::Instant {
            unix_ms: 1_794_405_909_000,
        };
        assert_eq!(instant.iso8601(), "2026-11-11T14:05:09Z");
        assert_eq!(
            WhenValue::Instant { unix_ms: -1 }.iso8601(),
            "1969-12-31T23:59:59Z"
        );
        let local = WhenValue::Local {
            year: 2026,
            month: 11,
            day: 11,
            hour: 14,
            minute: 5,
            second: 0,
        };
        assert_eq!(local.iso8601(), "2026-11-11T14:05:00");
    }

    #[test]
    fn values_become_fluent_values() {
        assert_eq!(ArgValue::Count(3).into_fluent(), FluentValue::from(3u64));
        assert_eq!(
            ArgValue::Text("a".into()).into_fluent(),
            FluentValue::from("a")
        );
        assert_eq!(
            ArgValue::List(vec!["a".into(), "b".into()]).into_fluent(),
            FluentValue::Custom(Box::new(ListValue(vec!["a".into(), "b".into()])))
        );
        assert_eq!(
            ArgValue::Decimal(ExactDecimal("1234.50".into())).into_fluent(),
            FluentValue::Custom(Box::new(ExactDecimal("1234.50".into())))
        );
    }
}
