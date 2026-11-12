//! Typed message references and Fluent argument values.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

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
}
