//! The tri-state lookup (alg/02 HV-MODEL-02): `Missing` only when the container is loaded at a
//! level that would hold the object and in scope; `Unknown` otherwise. A view filter never makes
//! `Missing`. Only `Missing` may produce a diagnostic.

/// The answer of every catalog lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup<T> {
    Found(T),
    Missing(MissingReason),
    Unknown(UnknownReason),
}

impl<T> Lookup<T> {
    /// The found value, if any.
    pub fn found(self) -> Option<T> {
        match self {
            Lookup::Found(t) => Some(t),
            Lookup::Missing(_) | Lookup::Unknown(_) => None,
        }
    }

    /// Maps the found value.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Lookup<U> {
        match self {
            Lookup::Found(t) => Lookup::Found(f(t)),
            Lookup::Missing(r) => Lookup::Missing(r),
            Lookup::Unknown(r) => Lookup::Unknown(r),
        }
    }
}

/// Why a lookup is certain the object does not exist. A machine code; the UI maps it to a
/// Fluent id (`intro-missing-<code>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MissingReason {
    /// Loaded, in scope, at a level that would hold it, and absent.
    NotFound,
}

impl MissingReason {
    /// Every reason, in declaration order.
    pub const ALL: [MissingReason; 1] = [MissingReason::NotFound];

    /// The stable machine code.
    pub const fn code(self) -> &'static str {
        match self {
            MissingReason::NotFound => "not-found",
        }
    }
}

/// Why a lookup cannot answer. A machine code; the UI maps it to a Fluent id
/// (`intro-unknown-<code>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnknownReason {
    /// The container has not been loaded (or not at a level that would hold the object).
    NotLoaded,
    /// The container is being loaded.
    Loading,
    /// The container is outside the introspection scope.
    OutOfScope,
    /// The container's last load failed (permission, connection).
    Failed,
    /// The server's matching rule for this name is not pinned (R-M4-41).
    ServerDependent,
}

impl UnknownReason {
    /// Every reason, in declaration order.
    pub const ALL: [UnknownReason; 5] = [
        UnknownReason::NotLoaded,
        UnknownReason::Loading,
        UnknownReason::OutOfScope,
        UnknownReason::Failed,
        UnknownReason::ServerDependent,
    ];

    /// The stable machine code.
    pub const fn code(self) -> &'static str {
        match self {
            UnknownReason::NotLoaded => "not-loaded",
            UnknownReason::Loading => "loading",
            UnknownReason::OutOfScope => "out-of-scope",
            UnknownReason::Failed => "failed",
            UnknownReason::ServerDependent => "server-dependent",
        }
    }
}
